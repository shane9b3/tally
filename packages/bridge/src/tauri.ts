/**
 * `TauriClient`: the {@link EngineClient} used inside the desktop app. Each
 * method invokes the Tauri command of the same name in docs/api.md (camelCase
 * JS arguments; Tauri maps them to the snake_case Rust parameters) and
 * rejects with an {@link ApiError} `{ code, message }` on failure.
 *
 * It also implements the shell-only commands (`wb_open_dialog`, the dialog
 * half of `wb_save`, `recent_files`, `list_templates`), which have no
 * `DocumentService` method.
 *
 * The Tauri transport is injected (`invoke` from `@tauri-apps/api/core`,
 * `listen` from `@tauri-apps/api/event`) so this package keeps no runtime
 * dependency on Tauri and tests can pass fakes:
 *
 *   import { invoke } from "@tauri-apps/api/core";
 *   import { listen } from "@tauri-apps/api/event";
 *   const client = new TauriClient({ invoke, listen });
 *
 * Owned by WP7.
 */
import type { ApiError } from "./generated/ApiError";
import type { ApplyResult } from "./generated/ApplyResult";
import type { CellBlock } from "./generated/CellBlock";
import type { CellEditInfo } from "./generated/CellEditInfo";
import type { CellPos } from "./generated/CellPos";
import type { ClipboardOut } from "./generated/ClipboardOut";
import type { Direction } from "./generated/Direction";
import type { DocChangedEvent } from "./generated/DocChangedEvent";
import type { DocId } from "./generated/DocId";
import type { FileFormat } from "./generated/FileFormat";
import type { FunctionInfo } from "./generated/FunctionInfo";
import type { MenuActionEvent } from "./generated/MenuActionEvent";
import type { RangeDto } from "./generated/RangeDto";
import type { SelectionDto } from "./generated/SelectionDto";
import type { SelectionStats } from "./generated/SelectionStats";
import type { SheetLayout } from "./generated/SheetLayout";
import type { StyleDto } from "./generated/StyleDto";
import type { Suggestion } from "./generated/Suggestion";
import type { Transaction } from "./generated/Transaction";
import type { WorkbookInfo } from "./generated/WorkbookInfo";
import { isApiError, type EngineClient, type SheetId, type Unsubscribe } from "./client";
import type { ShellClient, TemplateInfo } from "./shell";

export { CANCELLED, type TemplateInfo } from "./shell";

/** Arguments of a Tauri command (JSON-serializable, camelCase keys). */
export type InvokeArgs = Record<string, unknown>;

/** Signature of `invoke` from `@tauri-apps/api/core`. */
export type InvokeFn = <T>(cmd: string, args?: InvokeArgs) => Promise<T>;

/** The part of a Tauri event that we read. */
export interface TauriEvent<T> {
  payload: T;
}

/** Signature of `listen` from `@tauri-apps/api/event`. */
export type ListenFn = <T>(event: string, handler: (event: TauriEvent<T>) => void) => Promise<() => void>;

/** The injected Tauri API. */
export interface TauriTransport {
  invoke: InvokeFn;
  listen: ListenFn;
}

/** Command names, exactly as in docs/api.md. */
export const COMMANDS = {
  newWorkbook: "wb_new",
  openDialog: "wb_open_dialog",
  open: "wb_open",
  save: "wb_save",
  close: "wb_close",
  sheetLayout: "sheet_layout",
  cells: "get_cells",
  styles: "get_styles",
  cellEditInfo: "cell_edit_info",
  apply: "apply",
  undo: "undo",
  redo: "redo",
  selectionStats: "selection_stats",
  copy: "clipboard_copy",
  navigateEdge: "navigate_edge",
  resolveNameBox: "resolve_name_box",
  listFunctions: "list_functions",
  assistSuggest: "assist_suggest",
  recentFiles: "recent_files",
  listTemplates: "list_templates",
} as const;

/** Event names, exactly as in docs/api.md. */
export const EVENTS = { docChanged: "doc-changed", menuAction: "menu-action" } as const;

/** Normalises any rejection into an {@link ApiError}. */
export function toApiError(e: unknown): ApiError {
  if (isApiError(e)) return { code: e.code, message: e.message };
  if (typeof e === "string") return { code: "internal", message: e };
  if (e instanceof Error) return { code: "internal", message: e.message };
  return { code: "internal", message: String(e) };
}

export class TauriClient implements EngineClient, ShellClient {
  constructor(private readonly tauri: TauriTransport) {}

  private call<T>(cmd: string, args?: InvokeArgs): Promise<T> {
    return this.tauri.invoke<T>(cmd, args).catch((e: unknown) => {
      throw toApiError(e);
    });
  }

  private subscribe<T>(event: string, cb: (payload: T) => void): Unsubscribe {
    let off: (() => void) | null = null;
    let cancelled = false;
    this.tauri.listen<T>(event, (e) => cb(e.payload)).then(
      (unlisten) => {
        if (cancelled) unlisten();
        else off = unlisten;
      },
      (e: unknown) => console.warn(`TauriClient: cannot listen to ${event}`, e),
    );
    return () => {
      cancelled = true;
      off?.();
      off = null;
    };
  }

  // -- documents --------------------------------------------------------------

  newWorkbook(template?: string | null): Promise<WorkbookInfo> {
    return this.call(COMMANDS.newWorkbook, { template: template ?? null });
  }

  open(path: string): Promise<WorkbookInfo> {
    return this.call(COMMANDS.open, { path });
  }

  /**
   * `wb_save`. The webview never chooses arbitrary paths: `path === null`
   * saves to the document's file (asking only if it has none); any other
   * value shows the Save dialog (`saveAs: true`), and the shell uses the
   * user's choice.
   */
  save(doc: DocId, path: string | null, format: FileFormat): Promise<WorkbookInfo> {
    return this.call(COMMANDS.save, { doc, saveAs: path !== null, format });
  }

  close(doc: DocId): Promise<void> {
    return this.call(COMMANDS.close, { doc });
  }

  // -- shell-only ------------------------------------------------------------

  /** Native Open dialog, then open; `null` when cancelled. */
  openDialog(): Promise<WorkbookInfo | null> {
    return this.call(COMMANDS.openDialog);
  }

  /** `wb_save` with explicit dialog control; rejects with code `cancelled` if dismissed. */
  saveWithDialog(doc: DocId, saveAs: boolean, format: FileFormat | null): Promise<WorkbookInfo> {
    return this.call(COMMANDS.save, { doc, saveAs, format });
  }

  recentFiles(): Promise<string[]> {
    return this.call(COMMANDS.recentFiles);
  }

  listTemplates(): Promise<TemplateInfo[]> {
    return this.call(COMMANDS.listTemplates);
  }

  // -- queries ----------------------------------------------------------------

  sheetLayout(doc: DocId, sheet: SheetId): Promise<SheetLayout> {
    return this.call(COMMANDS.sheetLayout, { doc, sheet });
  }

  cells(doc: DocId, sheet: SheetId, ranges: RangeDto[]): Promise<CellBlock[]> {
    return this.call(COMMANDS.cells, { doc, sheet, ranges });
  }

  styles(doc: DocId, ids: number[]): Promise<StyleDto[]> {
    return this.call(COMMANDS.styles, { doc, ids });
  }

  cellEditInfo(doc: DocId, sheet: SheetId, r: number, c: number): Promise<CellEditInfo> {
    return this.call(COMMANDS.cellEditInfo, { doc, sheet, r, c });
  }

  selectionStats(doc: DocId, sel: SelectionDto): Promise<SelectionStats> {
    return this.call(COMMANDS.selectionStats, { doc, sel });
  }

  copy(doc: DocId, sel: SelectionDto): Promise<ClipboardOut> {
    return this.call(COMMANDS.copy, { doc, sel });
  }

  navigateEdge(doc: DocId, sheet: SheetId, from: CellPos, dir: Direction): Promise<CellPos> {
    return this.call(COMMANDS.navigateEdge, { doc, sheet, from, dir });
  }

  resolveNameBox(doc: DocId, text: string): Promise<SelectionDto> {
    return this.call(COMMANDS.resolveNameBox, { doc, text });
  }

  listFunctions(): Promise<FunctionInfo[]> {
    return this.call(COMMANDS.listFunctions);
  }

  assistSuggest(doc: DocId, query: string, sel: SelectionDto): Promise<Suggestion[]> {
    return this.call(COMMANDS.assistSuggest, { doc, query, sel });
  }

  // -- mutations --------------------------------------------------------------

  apply(doc: DocId, tx: Transaction): Promise<ApplyResult> {
    return this.call(COMMANDS.apply, { doc, tx });
  }

  undo(doc: DocId): Promise<ApplyResult> {
    return this.call(COMMANDS.undo, { doc });
  }

  redo(doc: DocId): Promise<ApplyResult> {
    return this.call(COMMANDS.redo, { doc });
  }

  // -- events -----------------------------------------------------------------

  onDocChanged(cb: (event: DocChangedEvent) => void): Unsubscribe {
    return this.subscribe(EVENTS.docChanged, cb);
  }

  onMenuAction(cb: (event: MenuActionEvent) => void): Unsubscribe {
    return this.subscribe(EVENTS.menuAction, cb);
  }
}
