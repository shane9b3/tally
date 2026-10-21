/**
 * Text to Columns: the dialog's separator options and a client-side split
 * preview of the selected column's texts (the engine does the real split
 * with the `tally-io-csv` tokenizer; this mirrors its rules closely enough
 * for the live preview grid).
 */
import type { CsvColumnType, CsvImportOptionsDto } from "@tally/bridge";

export interface SeparatorState {
  tab: boolean;
  comma: boolean;
  semicolon: boolean;
  space: boolean;
  other: boolean;
  otherChar: string;
  mergeDelimiters: boolean;
  quote: '"' | "'";
  trim: boolean;
  detectNumbers: boolean;
  fixedWidth: boolean;
  /** Column start offsets for fixed width (0 is implicit). */
  widths: number[];
  columnTypes: CsvColumnType[];
}

export const DEFAULT_SEPARATORS: SeparatorState = { tab: true, comma: false, semicolon: true, space: false, other: false, otherChar: "", mergeDelimiters: false, quote: '"', trim: true, detectNumbers: true, fixedWidth: false, widths: [], columnTypes: [] };

export const COLUMN_TYPES: readonly { type: CsvColumnType; label: string }[] = [
  { type: "standard", label: "Standard" },
  { type: "text", label: "Text" },
  { type: "dateDmy", label: "Date (DMY)" },
  { type: "dateMdy", label: "Date (MDY)" },
  { type: "dateYmd", label: "Date (YMD)" },
  { type: "hide", label: "Hide" },
];

/** The separator characters as the engine wants them (`";\t"`). */
export function delimiterString(s: SeparatorState): string {
  let out = "";
  if (s.tab) out += "\t";
  if (s.comma) out += ",";
  if (s.semicolon) out += ";";
  if (s.space) out += " ";
  if (s.other && s.otherChar !== "") out += s.otherChar[0] ?? "";
  return out;
}

export function toCsvOptions(s: SeparatorState): CsvImportOptionsDto {
  return {
    delimiters: s.fixedWidth ? "" : delimiterString(s),
    mergeDelimiters: s.mergeDelimiters,
    quote: s.quote,
    trim: s.trim,
    detectNumbers: s.detectNumbers,
    fixedWidths: s.fixedWidth ? [...s.widths].filter((w) => w > 0).sort((a, b) => a - b) : null,
    columnTypes: [...s.columnTypes],
  };
}

/** Splits one line by the separator options (quotes protect separators; `""` is a literal quote). */
export function splitLine(line: string, s: SeparatorState): string[] {
  if (s.fixedWidth) {
    const starts = [0, ...s.widths.filter((w) => w > 0).sort((a, b) => a - b)];
    const out: string[] = [];
    for (let i = 0; i < starts.length; i++) {
      const a = starts[i] ?? 0;
      const b = starts[i + 1] ?? line.length;
      out.push(s.trim ? line.slice(a, b).trim() : line.slice(a, b));
    }
    return out;
  }
  const seps = delimiterString(s);
  if (seps === "") return [s.trim ? line.trim() : line];
  const out: string[] = [];
  let cur = "";
  let quoted = false;
  let lastWasSep = false;
  for (let i = 0; i < line.length; i++) {
    const ch = line[i] ?? "";
    if (ch === s.quote) {
      if (quoted && line[i + 1] === s.quote) {
        cur += ch;
        i++;
      } else quoted = !quoted;
      lastWasSep = false;
      continue;
    }
    if (!quoted && seps.includes(ch)) {
      if (s.mergeDelimiters && lastWasSep) continue;
      out.push(cur);
      cur = "";
      lastWasSep = true;
      continue;
    }
    cur += ch;
    lastWasSep = false;
  }
  out.push(cur);
  return s.trim ? out.map((f) => f.trim()) : out;
}

/** The preview grid: one row per text, padded to the widest row. */
export function previewSplit(texts: readonly string[], s: SeparatorState, maxRows = 12): string[][] {
  const rows = texts.slice(0, maxRows).map((t) => splitLine(t, s));
  const width = Math.max(1, ...rows.map((r) => r.length));
  return rows.map((r) => [...r, ...Array.from({ length: width - r.length }, () => "")]);
}

/** Column count of the split (for the per-column type row). */
export function previewColumns(texts: readonly string[], s: SeparatorState): number {
  return Math.max(1, ...texts.map((t) => splitLine(t, s).length));
}

/** Heuristic: which single separator splits the sample best (most columns, consistent). */
export function guessSeparator(texts: readonly string[]): Partial<SeparatorState> {
  const candidates: { key: keyof SeparatorState; ch: string }[] = [
    { key: "tab", ch: "\t" },
    { key: "semicolon", ch: ";" },
    { key: "comma", ch: "," },
    { key: "space", ch: " " },
  ];
  let best: { key: keyof SeparatorState; score: number } | null = null;
  for (const c of candidates) {
    const counts = texts.filter((t) => t !== "").map((t) => t.split(c.ch).length - 1);
    if (counts.length === 0) continue;
    const min = Math.min(...counts);
    const max = Math.max(...counts);
    const score = min > 0 && min === max ? min * 2 : min;
    if (score > 0 && (!best || score > best.score)) best = { key: c.key, score };
  }
  if (!best) return {};
  return { tab: best.key === "tab", comma: best.key === "comma", semicolon: best.key === "semicolon", space: best.key === "space" };
}
