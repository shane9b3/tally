/**
 * The `EngineClient` contract: everything the UI may ask of the engine.
 *
 * It mirrors the Rust `DocumentService` trait (crates/tally-api/src/service.rs)
 * method for method, camelCased and Promise-returning, plus event
 * subscriptions. Implementations: `TauriClient` (WP7, wraps the Tauri
 * commands listed in docs/api.md) and `MockClient` (./mock.ts).
 *
 * Components never call Tauri `invoke` directly; they receive an
 * `EngineClient`.
 *
 * This file is a frozen WP0 contract (see docs/contributing.md).
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

/** Sheet ids are stable numbers, independent of tab order. */
export type SheetId = number;

/** Removes a previously registered listener. */
export type Unsubscribe = () => void;

/**
 * Engine access for the UI.
 *
 * Every method returning a Promise rejects with an {@link ApiError}
 * (`{ code, message }`, not an `Error` instance) on failure; narrow with
 * {@link isApiError}.
 */
export interface EngineClient {
  /** Creates a workbook, optionally from a bundled template id. */
  newWorkbook(template?: string | null): Promise<WorkbookInfo>;
  /** Opens a file by path. */
  open(path: string): Promise<WorkbookInfo>;
  /** Saves to `path` (or the document's current path when `null`). */
  save(doc: DocId, path: string | null, format: FileFormat): Promise<WorkbookInfo>;
  /** Closes a document. */
  close(doc: DocId): Promise<void>;
  /** Row/column geometry of a sheet. */
  sheetLayout(doc: DocId, sheet: SheetId): Promise<SheetLayout>;
  /** Cells in the given ranges, one block per range. */
  cells(doc: DocId, sheet: SheetId, ranges: RangeDto[]): Promise<CellBlock[]>;
  /** Resolves style ids from `CellView.style`. */
  styles(doc: DocId, ids: number[]): Promise<StyleDto[]>;
  /** Formula-bar text and plain-English hint for a cell. */
  cellEditInfo(doc: DocId, sheet: SheetId, r: number, c: number): Promise<CellEditInfo>;
  /** Applies a transaction as one undo step. */
  apply(doc: DocId, tx: Transaction): Promise<ApplyResult>;
  /** Undoes the last transaction. */
  undo(doc: DocId): Promise<ApplyResult>;
  /** Redoes the last undone transaction. */
  redo(doc: DocId): Promise<ApplyResult>;
  /** Status bar statistics. */
  selectionStats(doc: DocId, sel: SelectionDto): Promise<SelectionStats>;
  /** Copies the selection; write the result to the system clipboard. */
  copy(doc: DocId, sel: SelectionDto): Promise<ClipboardOut>;
  /** Ctrl+arrow target. */
  navigateEdge(doc: DocId, sheet: SheetId, from: CellPos, dir: Direction): Promise<CellPos>;
  /** Resolves name box text such as `B12`, `A1:C5` or `Sheet2.A1`. */
  resolveNameBox(doc: DocId, text: string): Promise<SelectionDto>;
  /** Every registered function. */
  listFunctions(): Promise<FunctionInfo[]>;
  /** ⌘K suggestions. */
  assistSuggest(doc: DocId, query: string, sel: SelectionDto): Promise<Suggestion[]>;

  /** `doc-changed` event: changes the UI did not initiate (volatile recalc, async load). */
  onDocChanged(cb: (event: DocChangedEvent) => void): Unsubscribe;
  /** `menu-action` event: a native menu item was chosen. */
  onMenuAction(cb: (event: MenuActionEvent) => void): Unsubscribe;
}

/** Narrows an unknown rejection value to {@link ApiError}. */
export function isApiError(e: unknown): e is ApiError {
  return (
    typeof e === "object" &&
    e !== null &&
    typeof (e as { code?: unknown }).code === "string" &&
    typeof (e as { message?: unknown }).message === "string"
  );
}
