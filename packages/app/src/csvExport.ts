/**
 * CSV export (AOO's "Keep current format" warning and Text Export dialog):
 * the prompts the save flow awaits, and the session's last Text Export
 * choices. The shell rejects a Save that picked a CSV file with code
 * `csv_options`; the controller then asks here and finishes with
 * `ShellClient.saveCsv` (controller.ts `runSave`).
 */
import type { CsvCharset, CsvExportOptions } from "@tally/bridge";
import { create } from "zustand";

/** AOO's defaults: comma, double quote, as shown, UTF-8 (Tally's charset default). */
export const DEFAULT_CSV_OPTIONS: CsvExportOptions = { fieldDelimiter: ",", textDelimiter: '"', quoteAllText: false, asShown: true, charset: "utf8" };

/** The field delimiters offered by name (AOO's list); anything else is "other". */
export const FIELD_DELIMITERS = [",", ";", ":", "\t", " "] as const;

export const CHARSETS: readonly CsvCharset[] = ["utf8", "utf16", "windows1252", "iso88591"];

let last: CsvExportOptions = { ...DEFAULT_CSV_OPTIONS };

/** The Text Export choices last confirmed this session (defaults at first). */
export function lastCsvOptions(): CsvExportOptions {
  return { ...last };
}

export function rememberCsvOptions(o: CsvExportOptions): void {
  last = { ...o };
}

/** `null` when valid, else what is wrong (one ASCII character each, not CR/LF, different). */
export function csvOptionsProblem(o: CsvExportOptions): "field" | "text" | "same" | null {
  const one = (s: string) => s.length === 1 && s.charCodeAt(0) < 128 && s !== "\r" && s !== "\n";
  if (!one(o.fieldDelimiter)) return "field";
  if (o.textDelimiter !== "" && !one(o.textDelimiter)) return "text";
  if (o.textDelimiter === o.fieldDelimiter) return "same";
  return null;
}

export type KeepFormatChoice = "keep" | "odf" | "cancel";

export interface CsvPromptState {
  /** The "Keep current format" warning (file name, sheet count, active sheet). */
  keep: { file: string; multiSheet: boolean; sheet: string } | null;
  /** The Text Export dialog. */
  options: { file: string; multiSheet: boolean; sheet: string; initial: CsvExportOptions } | null;
  askKeep(file: string, multiSheet: boolean, sheet: string): Promise<KeepFormatChoice>;
  answerKeep(choice: KeepFormatChoice): void;
  askOptions(file: string, multiSheet: boolean, sheet: string): Promise<CsvExportOptions | null>;
  answerOptions(o: CsvExportOptions | null): void;
}

let keepResolver: ((c: KeepFormatChoice) => void) | null = null;
let optionsResolver: ((o: CsvExportOptions | null) => void) | null = null;

export const useCsvPromptStore = create<CsvPromptState>()((set) => ({
  keep: null,
  options: null,
  askKeep: (file, multiSheet, sheet) =>
    new Promise<KeepFormatChoice>((resolve) => {
      keepResolver?.("cancel");
      keepResolver = resolve;
      set({ keep: { file, multiSheet, sheet } });
    }),
  answerKeep: (choice) => {
    const r = keepResolver;
    keepResolver = null;
    set({ keep: null });
    r?.(choice);
  },
  askOptions: (file, multiSheet, sheet) =>
    new Promise<CsvExportOptions | null>((resolve) => {
      optionsResolver?.(null);
      optionsResolver = resolve;
      set({ options: { file, multiSheet, sheet, initial: lastCsvOptions() } });
    }),
  answerOptions: (o) => {
    const r = optionsResolver;
    optionsResolver = null;
    if (o) rememberCsvOptions(o);
    set({ options: null });
    r?.(o);
  },
}));
