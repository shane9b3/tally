/**
 * `TauriClient`: the {@link EngineClientV3} used inside the desktop app. Each
 * method invokes the Tauri command of the same name in docs/api.md (camelCase
 * JS arguments; Tauri maps them to the snake_case Rust parameters) and
 * rejects with an {@link ApiError} `{ code, message }` on failure.
 *
 * It also implements the shell-only commands (`wb_open_dialog`, the dialog
 * half of `wb_save`, `recent_files`, `list_templates`), the v0.3 shell
 * commands `open_url` (http/https/mailto only, validated again in Rust)
 * and `user_name`, and the wave-5 shell commands `wb_export_pdf` and
 * `wb_print`, which have no `DocumentService` method. The wave-5 queries
 * (`EngineClientV4`) invoke `apps/desktop/src-tauri/src/commands_v4.rs`;
 * `tauriClientV4.test.ts` checks that every method has a registered command.
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
import type { AutoFilterInfo } from "./generated/AutoFilterInfo";
import type { FindQuery } from "./generated/FindQuery";
import type { FindResult } from "./generated/FindResult";
import type { NameInfo } from "./generated/NameInfo";
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
import type { ConditionalFormatDto } from "./generated/ConditionalFormatDto";
import type { DocPropertiesDto } from "./generated/DocPropertiesDto";
import type { GoalSeekRequest } from "./generated/GoalSeekRequest";
import type { GoalSeekResult } from "./generated/GoalSeekResult";
import type { LocaleInfo } from "./generated/LocaleInfo";
import type { NoteDto } from "./generated/NoteDto";
import type { PageSetupDto } from "./generated/PageSetupDto";
import type { ProtectionInfo } from "./generated/ProtectionInfo";
import type { ValidationDto } from "./generated/ValidationDto";
import type { ValidationInfo } from "./generated/ValidationInfo";
import type { AutocompleteDto } from "./generated/AutocompleteDto";
import type { CalcSettingsDto } from "./generated/CalcSettingsDto";
import type { CellStyleDto } from "./generated/CellStyleDto";
import type { ChartDataDto } from "./generated/ChartDataDto";
import type { ChartDto } from "./generated/ChartDto";
import type { ChartKind } from "./generated/ChartKind";
import type { ChartSvgDto } from "./generated/ChartSvgDto";
import type { DatabaseRangeDto } from "./generated/DatabaseRangeDto";
import type { DocumentStatsDto } from "./generated/DocumentStatsDto";
import type { ImageDto } from "./generated/ImageDto";
import type { NavigatorDto } from "./generated/NavigatorDto";
import type { PageBreaksDto } from "./generated/PageBreaksDto";
import type { PivotDto } from "./generated/PivotDto";
import type { PivotFieldItemsDto } from "./generated/PivotFieldItemsDto";
import type { PivotHit } from "./generated/PivotHit";
import type { PivotSource } from "./generated/PivotSource";
import type { PrintJobInfo } from "./generated/PrintJobInfo";
import type { PrintLayoutDto } from "./generated/PrintLayoutDto";
import type { PrintOptionsDto } from "./generated/PrintOptionsDto";
import type { ScenarioDto } from "./generated/ScenarioDto";
import type { SolverRequest } from "./generated/SolverRequest";
import type { SolverResult } from "./generated/SolverResult";
import type { TraceDto } from "./generated/TraceDto";
import type { ViewOptionsDto } from "./generated/ViewOptionsDto";
import type { CellsOptions } from "./generated/CellsOptions";
import { isApiError, type EngineClientV4, type SheetId, type Unsubscribe } from "./client";
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
  names: "names",
  filterInfo: "filter_info",
  find: "find",
  detectHeader: "detect_header",
  recalcAll: "recalc_all",
  conditionalFormats: "conditional_formats",
  validations: "validations",
  validationAt: "validation_at",
  note: "note",
  notes: "notes",
  protectionInfo: "protection_info",
  pageSetup: "page_setup",
  documentProperties: "document_properties",
  goalSeek: "goal_seek",
  listLocales: "list_locales",
  setLocale: "set_locale",
  resolveNameBoxIn: "resolve_name_box_in",
  openUrl: "open_url",
  userName: "user_name",
  // v0.4–v0.6 (wave 5): apps/desktop/src-tauri/src/commands_v4.rs
  charts: "charts",
  chartData: "chart_data",
  suggestChart: "suggest_chart",
  images: "images",
  pivots: "pivots",
  pivotFieldItems: "pivot_field_items",
  pivotAt: "pivot_at",
  autocomplete: "autocomplete",
  cellStyles: "cell_styles",
  scenarios: "scenarios",
  databaseRanges: "database_ranges",
  navigator: "navigator",
  documentStatistics: "document_statistics",
  printLayout: "print_layout",
  pageBreaks: "page_breaks",
  preparePrintJob: "prepare_print_job",
  printJobPagePng: "print_job_page_png",
  printJobPdf: "print_job_pdf",
  calcSettings: "calc_settings",
  solver: "solver",
  precedents: "precedents",
  dependents: "dependents",
  imageData: "image_data",
  viewOptions: "view_options",
  invalidCells: "invalid_cells",
  customSortLists: "custom_sort_lists",
  cellsWith: "cells_with",
  previewFormula: "preview_formula",
  exportPdf: "wb_export_pdf",
  print: "wb_print",
  // W5-6 shell: prefs.rs, autosave.rs, templates.rs, l10n.rs
  getPrefs: "get_prefs",
  setPrefs: "set_prefs",
  autosave: "autosave_snapshot",
  listRecovery: "list_recovery",
  recoverSnapshot: "recover_snapshot",
  discardRecovery: "discard_recovery",
  saveAsTemplate: "save_as_template",
  listUserTemplates: "list_user_templates",
  newFromUserTemplate: "new_from_user_template",
  setUiLanguage: "set_ui_language",
  clearRecent: "clear_recent",
  // Unsaved-changes guard: apps/desktop/src-tauri/src/closing.rs
  setDocumentEdited: "set_document_edited",
  confirmClose: "confirm_close",
} as const;

/**
 * The `EngineClientV4` methods added in wave 5, each with its command
 * (`COMMANDS` key = method name). Tests check this list against the
 * interface and against `commands_v4.rs`.
 */
export const V4_METHODS = [
  "charts",
  "chartData",
  "suggestChart",
  "images",
  "pivots",
  "pivotFieldItems",
  "pivotAt",
  "autocomplete",
  "cellStyles",
  "scenarios",
  "databaseRanges",
  "navigator",
  "documentStatistics",
  "printLayout",
  "pageBreaks",
  "preparePrintJob",
  "printJobPagePng",
  "printJobPdf",
  "calcSettings",
  "solver",
  "precedents",
  "dependents",
  "imageData",
  "viewOptions",
  "invalidCells",
  "customSortLists",
  "cellsWith",
  "previewFormula",
] as const satisfies readonly (keyof EngineClientV4 & keyof typeof COMMANDS)[];

/** The W5-6 shell commands (prefs, autosave, templates, UI language); `w56shell.test.ts` covers them. */
export const W56_SHELL_COMMANDS = ["getPrefs", "setPrefs", "autosave", "listRecovery", "recoverSnapshot", "discardRecovery", "saveAsTemplate", "listUserTemplates", "newFromUserTemplate", "setUiLanguage", "clearRecent"] as const satisfies readonly (keyof typeof COMMANDS)[];

/** The unsaved-changes guard's shell commands (closing.rs); `unsaved.test.ts` covers them. */
export const CLOSE_SHELL_COMMANDS = ["setDocumentEdited", "confirmClose"] as const satisfies readonly (keyof typeof COMMANDS)[];

/** Event names, exactly as in docs/api.md. */
export const EVENTS = { docChanged: "doc-changed", menuAction: "menu-action", closeRequested: "close-requested" } as const;

/** Normalises any rejection into an {@link ApiError}. */
export function toApiError(e: unknown): ApiError {
  if (isApiError(e)) return { code: e.code, message: e.message };
  if (typeof e === "string") return { code: "internal", message: e };
  if (e instanceof Error) return { code: "internal", message: e.message };
  return { code: "internal", message: String(e) };
}

export class TauriClient implements EngineClientV4, ShellClient {
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

  // -- v0.2 -------------------------------------------------------------------

  names(doc: DocId): Promise<NameInfo[]> {
    return this.call(COMMANDS.names, { doc });
  }

  filterInfo(doc: DocId, sheet: SheetId): Promise<AutoFilterInfo> {
    return this.call(COMMANDS.filterInfo, { doc, sheet });
  }

  find(doc: DocId, query: FindQuery): Promise<FindResult> {
    return this.call(COMMANDS.find, { doc, query });
  }

  detectHeader(doc: DocId, sheet: SheetId, range: RangeDto): Promise<boolean> {
    return this.call(COMMANDS.detectHeader, { doc, sheet, range });
  }

  recalcAll(doc: DocId): Promise<ApplyResult> {
    return this.call(COMMANDS.recalcAll, { doc });
  }

  // -- v0.3 -------------------------------------------------------------------

  conditionalFormats(doc: DocId, sheet: SheetId): Promise<ConditionalFormatDto[]> {
    return this.call(COMMANDS.conditionalFormats, { doc, sheet });
  }

  validations(doc: DocId, sheet: SheetId): Promise<ValidationDto[]> {
    return this.call(COMMANDS.validations, { doc, sheet });
  }

  validationAt(doc: DocId, sheet: SheetId, r: number, c: number): Promise<ValidationInfo | null> {
    return this.call(COMMANDS.validationAt, { doc, sheet, r, c });
  }

  note(doc: DocId, sheet: SheetId, r: number, c: number): Promise<NoteDto | null> {
    return this.call(COMMANDS.note, { doc, sheet, r, c });
  }

  notes(doc: DocId, sheet: SheetId): Promise<NoteDto[]> {
    return this.call(COMMANDS.notes, { doc, sheet });
  }

  protectionInfo(doc: DocId, sheet: SheetId): Promise<ProtectionInfo> {
    return this.call(COMMANDS.protectionInfo, { doc, sheet });
  }

  pageSetup(doc: DocId, sheet: SheetId): Promise<PageSetupDto> {
    return this.call(COMMANDS.pageSetup, { doc, sheet });
  }

  documentProperties(doc: DocId): Promise<DocPropertiesDto> {
    return this.call(COMMANDS.documentProperties, { doc });
  }

  goalSeek(doc: DocId, req: GoalSeekRequest): Promise<GoalSeekResult> {
    return this.call(COMMANDS.goalSeek, { doc, req });
  }

  listLocales(): Promise<LocaleInfo[]> {
    return this.call(COMMANDS.listLocales);
  }

  setLocale(doc: DocId, locale: string): Promise<ApplyResult> {
    return this.call(COMMANDS.setLocale, { doc, locale });
  }

  resolveNameBoxIn(doc: DocId, sheet: SheetId, text: string): Promise<SelectionDto> {
    return this.call(COMMANDS.resolveNameBoxIn, { doc, sheet, text });
  }

  // -- v0.3 shell ----------------------------------------------------------------

  /** Opens an http(s)/mailto link with the OS handler (anything else is rejected in Rust). */
  openUrl(url: string): Promise<void> {
    return this.call(COMMANDS.openUrl, { url });
  }

  /** The OS account name (note authors), or `null`. */
  userName(): Promise<string | null> {
    return this.call(COMMANDS.userName);
  }

  // -- v0.4–v0.6 (wave 5) ---------------------------------------------------------

  charts(doc: DocId, sheet: SheetId): Promise<ChartDto[]> {
    return this.call(COMMANDS.charts, { doc, sheet });
  }

  chartData(doc: DocId, sheet: SheetId, id: number): Promise<ChartDataDto> {
    return this.call(COMMANDS.chartData, { doc, sheet, id });
  }

  suggestChart(doc: DocId, sheet: SheetId, sel: RangeDto, kind: ChartKind): Promise<ChartDto> {
    return this.call(COMMANDS.suggestChart, { doc, sheet, sel, kind });
  }

  images(doc: DocId, sheet: SheetId): Promise<ImageDto[]> {
    return this.call(COMMANDS.images, { doc, sheet });
  }

  pivots(doc: DocId): Promise<PivotDto[]> {
    return this.call(COMMANDS.pivots, { doc });
  }

  pivotFieldItems(doc: DocId, source: PivotSource): Promise<PivotFieldItemsDto[]> {
    return this.call(COMMANDS.pivotFieldItems, { doc, source });
  }

  pivotAt(doc: DocId, sheet: SheetId, r: number, c: number): Promise<PivotHit | null> {
    return this.call(COMMANDS.pivotAt, { doc, sheet, r, c });
  }

  autocomplete(doc: DocId, sheet: SheetId, r: number, c: number, prefix: string): Promise<AutocompleteDto> {
    return this.call(COMMANDS.autocomplete, { doc, sheet, r, c, prefix });
  }

  cellStyles(doc: DocId): Promise<CellStyleDto[]> {
    return this.call(COMMANDS.cellStyles, { doc });
  }

  scenarios(doc: DocId, sheet: SheetId): Promise<ScenarioDto[]> {
    return this.call(COMMANDS.scenarios, { doc, sheet });
  }

  databaseRanges(doc: DocId): Promise<DatabaseRangeDto[]> {
    return this.call(COMMANDS.databaseRanges, { doc });
  }

  navigator(doc: DocId): Promise<NavigatorDto> {
    return this.call(COMMANDS.navigator, { doc });
  }

  documentStatistics(doc: DocId): Promise<DocumentStatsDto> {
    return this.call(COMMANDS.documentStatistics, { doc });
  }

  printLayout(doc: DocId, opts: PrintOptionsDto): Promise<PrintLayoutDto> {
    return this.call(COMMANDS.printLayout, { doc, opts });
  }

  pageBreaks(doc: DocId, sheet: SheetId): Promise<PageBreaksDto> {
    return this.call(COMMANDS.pageBreaks, { doc, sheet });
  }

  preparePrintJob(doc: DocId, opts: PrintOptionsDto, charts: ChartSvgDto[]): Promise<PrintJobInfo> {
    return this.call(COMMANDS.preparePrintJob, { doc, opts, charts });
  }

  printJobPagePng(job: number, page: number, dpi: number): Promise<string> {
    return this.call(COMMANDS.printJobPagePng, { job, page, dpi });
  }

  /** The PDF bytes (Tauri serializes `Vec<u8>` as a number array). */
  printJobPdf(job: number): Promise<Uint8Array> {
    return this.call<ArrayLike<number> | ArrayBuffer>(COMMANDS.printJobPdf, { job }).then((bytes) =>
      bytes instanceof ArrayBuffer ? new Uint8Array(bytes) : Uint8Array.from(bytes),
    );
  }

  calcSettings(doc: DocId): Promise<CalcSettingsDto> {
    return this.call(COMMANDS.calcSettings, { doc });
  }

  solver(doc: DocId, req: SolverRequest): Promise<SolverResult> {
    return this.call(COMMANDS.solver, { doc, req });
  }

  precedents(doc: DocId, sheet: SheetId, r: number, c: number): Promise<TraceDto[]> {
    return this.call(COMMANDS.precedents, { doc, sheet, r, c });
  }

  dependents(doc: DocId, sheet: SheetId, r: number, c: number): Promise<TraceDto[]> {
    return this.call(COMMANDS.dependents, { doc, sheet, r, c });
  }

  imageData(doc: DocId, sheet: SheetId, id: number): Promise<string> {
    return this.call(COMMANDS.imageData, { doc, sheet, id });
  }

  viewOptions(doc: DocId, sheet: SheetId): Promise<ViewOptionsDto> {
    return this.call(COMMANDS.viewOptions, { doc, sheet });
  }

  invalidCells(doc: DocId, sheet: SheetId, range: RangeDto | null): Promise<CellPos[]> {
    return this.call(COMMANDS.invalidCells, { doc, sheet, range });
  }

  customSortLists(doc: DocId): Promise<string[][]> {
    return this.call(COMMANDS.customSortLists, { doc });
  }

  cellsWith(doc: DocId, sheet: SheetId, ranges: RangeDto[], opts: CellsOptions): Promise<CellBlock[]> {
    return this.call(COMMANDS.cellsWith, { doc, sheet, ranges, opts });
  }

  previewFormula(doc: DocId, sheet: SheetId, r: number, c: number, text: string): Promise<string | null> {
    return this.call(COMMANDS.previewFormula, { doc, sheet, r, c, text });
  }

  // -- wave-5 shell ----------------------------------------------------------------

  /** `wb_export_pdf`: Save dialog + the job's PDF (`unsupported` until W5-3). */
  exportPdf(doc: DocId, job: number): Promise<void> {
    return this.call(COMMANDS.exportPdf, { doc, job });
  }

  /** `wb_print`: the OS print dialog for the job (`unsupported` until W5-3). */
  print(doc: DocId, job: number): Promise<void> {
    return this.call(COMMANDS.print, { doc, job });
  }

  // -- W5-6 shell: prefs, autosave and recovery, templates, UI language --------------

  /** `get_prefs`: the prefs.json text (`{}` when there is none). */
  getPrefs(): Promise<string> {
    return this.call(COMMANDS.getPrefs);
  }

  /** `set_prefs`: writes prefs.json (≤ 64 KiB, validated in Rust). */
  setPrefs(json: string): Promise<void> {
    return this.call(COMMANDS.setPrefs, { json });
  }

  /** `autosave_snapshot`: an ODS recovery snapshot of `doc` in the app data dir. */
  autosave(doc: DocId): Promise<void> {
    return this.call(COMMANDS.autosave, { doc });
  }

  /** `list_recovery`: snapshots left by a crashed session. */
  listRecovery(): Promise<{ path: string; title: string; savedAt: string }[]> {
    return this.call(COMMANDS.listRecovery);
  }

  /** `recover_snapshot`: opens a snapshot as an untitled, dirty workbook and deletes it. */
  recoverSnapshot(path: string): Promise<WorkbookInfo> {
    return this.call(COMMANDS.recoverSnapshot, { path });
  }

  discardRecovery(path: string): Promise<void> {
    return this.call(COMMANDS.discardRecovery, { path });
  }

  /** `save_as_template`: writes `<name>.ots` into the user templates folder. */
  saveAsTemplate(doc: DocId, name: string): Promise<void> {
    return this.call(COMMANDS.saveAsTemplate, { doc, name });
  }

  listUserTemplates(): Promise<TemplateInfo[]> {
    return this.call(COMMANDS.listUserTemplates);
  }

  /** `new_from_user_template`: opens a user template as an untitled workbook. */
  newFromUserTemplate(id: string): Promise<WorkbookInfo> {
    return this.call(COMMANDS.newFromUserTemplate, { id });
  }

  /** `set_ui_language`: stores the language and rebuilds the native menu. */
  setUiLanguage(lang: string): Promise<void> {
    return this.call(COMMANDS.setUiLanguage, { lang });
  }

  clearRecent(): Promise<void> {
    return this.call(COMMANDS.clearRecent);
  }

  // -- unsaved-changes guard (closing.rs) -------------------------------------

  /** `set_document_edited`: the dirty state for the close/quit guard and the macOS "edited" dot. */
  setDocumentEdited(edited: boolean): Promise<void> {
    return this.call(COMMANDS.setDocumentEdited, { edited });
  }

  /** `confirm_close`: the save prompt resolved; close the window (`quit: false`) or quit. */
  confirmClose(quit: boolean): Promise<void> {
    return this.call(COMMANDS.confirmClose, { quit });
  }

  /** `close-requested { kind }`: a window close or quit is waiting for the save prompt. */
  onCloseRequested(cb: (kind: "window" | "quit") => void): Unsubscribe {
    return this.subscribe<{ kind: "window" | "quit" }>(EVENTS.closeRequested, (e) => cb(e.kind));
  }

  // -- events -----------------------------------------------------------------

  onDocChanged(cb: (event: DocChangedEvent) => void): Unsubscribe {
    return this.subscribe(EVENTS.docChanged, cb);
  }

  onMenuAction(cb: (event: MenuActionEvent) => void): Unsubscribe {
    return this.subscribe(EVENTS.menuAction, cb);
  }
}
