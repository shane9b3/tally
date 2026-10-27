/**
 * `ShellClient`: what the UI may ask of the app shell rather than the
 * engine: native file dialogs, the recent-files list and the template list.
 * These map to the shell-only Tauri commands in docs/api.md
 * (`wb_open_dialog`, the dialog half of `wb_save`, `recent_files`,
 * `list_templates`) and have no `DocumentService` method.
 *
 * Implementations: `TauriClient` (./tauri.ts) and `MockClient` (./mock.ts,
 * in-memory stand-ins for the browser build and e2e).
 *
 * Templates themselves are created with `EngineClient.newWorkbook(id)`.
 */
import type { CsvExportOptions } from "./generated/CsvExportOptions";
import type { DocId } from "./generated/DocId";
import type { FileFormat } from "./generated/FileFormat";
import type { WorkbookInfo } from "./generated/WorkbookInfo";

/** A template offered by `listTemplates` (pass `id` to `newWorkbook`). */
export interface TemplateInfo {
  /** Stable id passed to `newWorkbook(template)`, e.g. "invoices". */
  id: string;
  /** Display name, e.g. "Invoice tracker". */
  title: string;
  /** One-line description. */
  description: string;
}

/**
 * Rejection code when the user dismisses a native dialog
 * (`ApiError::CANCELLED` in Rust). The UI ignores it.
 */
export const CANCELLED = "cancelled";

/**
 * Rejection code of `saveWithDialog` when the user picked a CSV file and
 * the shell waits for the Text Export options (`ApiError::CSV_OPTIONS`):
 * the UI shows the dialog, then calls `saveCsv` (the message is the file
 * name).
 */
export const CSV_OPTIONS = "csv_options";

/** The templates the engine builds (`tally_engine::service::templates::CATALOG`). */
export const BUILTIN_TEMPLATES: readonly TemplateInfo[] = [
  { id: "invoices", title: "Invoice tracker", description: "Customers, amounts and payment status" },
  { id: "budget", title: "Monthly budget", description: "Planned versus actual spending" },
  { id: "stock", title: "Stock count", description: "Items on hand and reorder levels" },
];

/** One face of an installed font family (`list_fonts`, metadata only). */
export interface FontFaceInfo {
  /** 100–900 (400 regular, 700 bold). */
  weight: number;
  /** Italic or oblique. */
  italic: boolean;
  /** Width class 1–9 (5 normal). */
  stretch: number;
}

/** An installed font family and its faces (`list_fonts`). */
export interface FontFamilyInfo {
  family: string;
  faces: FontFaceInfo[];
  /** Every face is monospaced. */
  monospaced: boolean;
}

const FOUR_FACES: readonly FontFaceInfo[] = [
  { weight: 400, italic: false, stretch: 5 },
  { weight: 400, italic: true, stretch: 5 },
  { weight: 700, italic: false, stretch: 5 },
  { weight: 700, italic: true, stretch: 5 },
];

/**
 * The font list of the browser build and e2e (the desktop app lists the
 * installed fonts): common web-safe families plus Inter, the app font.
 */
export const FALLBACK_FONTS: readonly FontFamilyInfo[] = [
  "Arial",
  "Courier New",
  "Georgia",
  "Helvetica",
  "Inter",
  "Tahoma",
  "Times New Roman",
  "Trebuchet MS",
  "Verdana",
].map((family) => ({ family, faces: FOUR_FACES.map((f) => ({ ...f })), monospaced: family === "Courier New" }));

/** Shell services. Every method rejects with an `ApiError` on failure. */
export interface ShellClient {
  /** Native Open dialog, then open; `null` when the user cancels. */
  openDialog(): Promise<WorkbookInfo | null>;
  /**
   * Saves `doc`, showing the Save dialog when `saveAs` is set or the
   * document has no file yet. `format` defaults to the file's extension
   * (else ODS). Rejects with code {@link CANCELLED} if the dialog is dismissed.
   */
  saveWithDialog(doc: DocId, saveAs: boolean, format: FileFormat | null, csv?: CsvExportOptions | null): Promise<WorkbookInfo>;
  /**
   * Finishes a save rejected with {@link CSV_OPTIONS}: writes the picked
   * CSV file with `csv` (`wb_save_csv`); `null` drops it (the options
   * dialog was cancelled) and resolves `null`.
   */
  saveCsv(doc: DocId, csv: CsvExportOptions | null): Promise<WorkbookInfo | null>;
  /** Recently opened or saved files, most recent first. */
  recentFiles(): Promise<string[]>;
  /** Templates for `newWorkbook(id)`. */
  listTemplates(): Promise<TemplateInfo[]>;
  /**
   * Wave 5 (W5-3): writes the PDF of print job `job` (from
   * `preparePrintJob`) to a file chosen in a native Save dialog
   * (`wb_export_pdf`). Rejects with {@link CANCELLED} when dismissed and
   * with `unsupported` until the print pipeline lands.
   */
  exportPdf?(doc: DocId, job: number): Promise<void>;
  /** Wave 5 (W5-3): sends print job `job` to the OS print dialog (`wb_print`). */
  print?(doc: DocId, job: number): Promise<void>;
  /**
   * The installed font families, sorted by name (`list_fonts`); `refresh`
   * rescans the font directories. The browser build returns
   * {@link FALLBACK_FONTS}.
   */
  listFonts?(refresh?: boolean): Promise<FontFamilyInfo[]>;
}
