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

/** The templates the engine builds (`tally_engine::service::templates::CATALOG`). */
export const BUILTIN_TEMPLATES: readonly TemplateInfo[] = [
  { id: "invoices", title: "Invoice tracker", description: "Customers, amounts and payment status" },
  { id: "budget", title: "Monthly budget", description: "Planned versus actual spending" },
  { id: "stock", title: "Stock count", description: "Items on hand and reorder levels" },
];

/** Shell services. Every method rejects with an `ApiError` on failure. */
export interface ShellClient {
  /** Native Open dialog, then open; `null` when the user cancels. */
  openDialog(): Promise<WorkbookInfo | null>;
  /**
   * Saves `doc`, showing the Save dialog when `saveAs` is set or the
   * document has no file yet. `format` defaults to the file's extension
   * (else ODS). Rejects with code {@link CANCELLED} if the dialog is dismissed.
   */
  saveWithDialog(doc: DocId, saveAs: boolean, format: FileFormat | null): Promise<WorkbookInfo>;
  /** Recently opened or saved files, most recent first. */
  recentFiles(): Promise<string[]>;
  /** Templates for `newWorkbook(id)`. */
  listTemplates(): Promise<TemplateInfo[]>;
}
