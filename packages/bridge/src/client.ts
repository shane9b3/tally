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

/**
 * v0.2 additions to {@link EngineClient} (named ranges, AutoFilter, find,
 * sort header detection, hard recalc). Kept as a separate interface so
 * existing `EngineClient` implementations keep compiling; `MockClient`
 * implements it, `TauriClient` should too (Tauri commands `names`,
 * `filter_info`, `find`, `detect_header`, `recalc_all`, see docs/api.md).
 */
export interface EngineClientV2 extends EngineClient {
  /** Every defined name (workbook scope first, then by sheet), sorted. */
  names(doc: DocId): Promise<NameInfo[]>;
  /** The sheet's AutoFilter with each column's distinct values. */
  filterInfo(doc: DocId, sheet: SheetId): Promise<AutoFilterInfo>;
  /** Find (at most 10,000 hits). */
  find(doc: DocId, query: FindQuery): Promise<FindResult>;
  /** `true` if the first row of `range` looks like column labels. */
  detectHeader(doc: DocId, sheet: SheetId, range: RangeDto): Promise<boolean>;
  /** Hard recalculation (⇧⌘F9). */
  recalcAll(doc: DocId): Promise<ApplyResult>;
}

/**
 * v0.3 additions (conditional formatting, validation, notes, protection,
 * page setup, document properties, goal seek, locales). The mutations are
 * `Op`s applied through `apply`; these are the queries. Tauri commands:
 * `conditional_formats`, `validations`, `validation_at`, `note`, `notes`,
 * `protection_info`, `page_setup`, `document_properties`, `goal_seek`,
 * `list_locales`, `set_locale`, `resolve_name_box_in` (see docs/api.md).
 */
export interface EngineClientV3 extends EngineClientV2 {
  /** The sheet's conditional formats in priority order. */
  conditionalFormats(doc: DocId, sheet: SheetId): Promise<ConditionalFormatDto[]>;
  /** The sheet's data validations. */
  validations(doc: DocId, sheet: SheetId): Promise<ValidationDto[]>;
  /** The validation of one cell with its list items resolved, or `null`. */
  validationAt(doc: DocId, sheet: SheetId, r: number, c: number): Promise<ValidationInfo | null>;
  /** The note of a cell, or `null`. */
  note(doc: DocId, sheet: SheetId, r: number, c: number): Promise<NoteDto | null>;
  /** Every note of a sheet, row-major. */
  notes(doc: DocId, sheet: SheetId): Promise<NoteDto[]>;
  /** Protection state of a sheet and of the document structure. */
  protectionInfo(doc: DocId, sheet: SheetId): Promise<ProtectionInfo>;
  /** A sheet's page setup (with its manual breaks). */
  pageSetup(doc: DocId, sheet: SheetId): Promise<PageSetupDto>;
  /** Document properties. */
  documentProperties(doc: DocId): Promise<DocPropertiesDto>;
  /** Goal seek in a sandbox; apply `value` with `setCellInput` to accept. */
  goalSeek(doc: DocId, req: GoalSeekRequest): Promise<GoalSeekResult>;
  /** Supported locales. */
  listLocales(): Promise<LocaleInfo[]>;
  /** Sets the document locale (formatting and input recognition). */
  setLocale(doc: DocId, locale: string): Promise<ApplyResult>;
  /** Name box resolution relative to the sheet the UI shows. */
  resolveNameBoxIn(doc: DocId, sheet: SheetId, text: string): Promise<SelectionDto>;
}

/**
 * v0.4–v0.6 additions (wave 5): charts and pictures, DataPilot, data
 * tools, named cell styles, scenarios, the Navigator, printing and PDF,
 * calculation settings, the solver and the detective. The mutations are
 * `Op`s applied through `apply`; these are the queries. The engine's
 * defaults answer `unsupported` until each work package lands. Tauri
 * commands: `charts`, `chart_data`, `suggest_chart`, `images`, `pivots`,
 * `pivot_field_items`, `pivot_at`, `autocomplete`, `cell_styles`,
 * `scenarios`, `database_ranges`, `navigator`, `document_statistics`,
 * `print_layout`, `page_breaks`, `prepare_print_job`,
 * `print_job_page_png`, `print_job_pdf`, `calc_settings`, `solver`,
 * `precedents`, `dependents` (see docs/api.md, "v0.4–v0.6").
 */
export interface EngineClientV4 extends EngineClientV3 {
  /** The charts of a sheet, in id order. */
  charts(doc: DocId, sheet: SheetId): Promise<ChartDto[]>;
  /** The resolved data of a chart (cached values, no recalc). */
  chartData(doc: DocId, sheet: SheetId, id: number): Promise<ChartDataDto>;
  /** A chart proposal for a selection (nothing is added). */
  suggestChart(doc: DocId, sheet: SheetId, sel: RangeDto, kind: ChartKind): Promise<ChartDto>;
  /** The pictures of a sheet (without their bytes). */
  images(doc: DocId, sheet: SheetId): Promise<ImageDto[]>;
  /** Every pivot table of the document. */
  pivots(doc: DocId): Promise<PivotDto[]>;
  /** The fields and distinct items of a pivot source. */
  pivotFieldItems(doc: DocId, source: PivotSource): Promise<PivotFieldItemsDto[]>;
  /** The pivot output a cell belongs to, or `null`. */
  pivotAt(doc: DocId, sheet: SheetId, r: number, c: number): Promise<PivotHit | null>;
  /** AutoInput completions for a prefix at a cell. */
  autocomplete(doc: DocId, sheet: SheetId, r: number, c: number, prefix: string): Promise<AutocompleteDto>;
  /** Named cell styles, `Default` first. */
  cellStyles(doc: DocId): Promise<CellStyleDto[]>;
  /** The scenarios of a (base) sheet. */
  scenarios(doc: DocId, sheet: SheetId): Promise<ScenarioDto[]>;
  /** Every named database range. */
  databaseRanges(doc: DocId): Promise<DatabaseRangeDto[]>;
  /** The Navigator's entries. */
  navigator(doc: DocId): Promise<NavigatorDto>;
  /** Document statistics. */
  documentStatistics(doc: DocId): Promise<DocumentStatsDto>;
  /** Pagination of a print (no rendering). */
  printLayout(doc: DocId, opts: PrintOptionsDto): Promise<PrintLayoutDto>;
  /** Automatic and manual page breaks of a sheet (Page Break Preview). */
  pageBreaks(doc: DocId, sheet: SheetId): Promise<PageBreaksDto>;
  /** Builds a print job (charts as the given SVGs); the engine keeps 4. */
  preparePrintJob(doc: DocId, opts: PrintOptionsDto, charts: ChartSvgDto[]): Promise<PrintJobInfo>;
  /** One page of a job rasterized to PNG, base64. */
  printJobPagePng(job: number, page: number, dpi: number): Promise<string>;
  /**
   * The PDF of a job. The desktop shell normally writes or prints it itself
   * (`wb_export_pdf`, `wb_print`); the bytes are exposed for tests and the
   * browser build.
   */
  printJobPdf(job: number): Promise<Uint8Array>;
  /** The document's calculation settings. */
  calcSettings(doc: DocId): Promise<CalcSettingsDto>;
  /** The linear solver, run in a sandbox (nothing is changed). */
  solver(doc: DocId, req: SolverRequest): Promise<SolverResult>;
  /** Detective: the cells a formula reads. */
  precedents(doc: DocId, sheet: SheetId, r: number, c: number): Promise<TraceDto[]>;
  /** Detective: the formulas that read a cell. */
  dependents(doc: DocId, sheet: SheetId, r: number, c: number): Promise<TraceDto[]>;
  /** A picture's bytes, base64 (pictures from files carry no `data` in `images`). */
  imageData(doc: DocId, sheet: SheetId, id: number): Promise<string>;
  /** A sheet's view options with the persisted split (`null` when not split). */
  viewOptions(doc: DocId, sheet: SheetId): Promise<ViewOptionsDto>;
  /** Mark Invalid Data: cells failing their validation (≤ 10,000, row-major). */
  invalidCells(doc: DocId, sheet: SheetId, range: RangeDto | null): Promise<CellPos[]>;
  /** Tools ▸ Options ▸ Sort Lists. */
  customSortLists(doc: DocId): Promise<string[][]>;
  /** `cells` rendered with view options (formulas, value highlighting, zero values). */
  cellsWith(doc: DocId, sheet: SheetId, ranges: RangeDto[], opts: CellsOptions): Promise<CellBlock[]>;
  /** Function Wizard: a formula evaluated at a cell without committing it, formatted; `null` if it does not parse. */
  previewFormula(doc: DocId, sheet: SheetId, r: number, c: number, text: string): Promise<string | null>;
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
