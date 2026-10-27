/**
 * Pure builders for the data-tool dialogs (W5-6B): each turns dialog state
 * into the `Op` (or DTO) the engine takes, plus small parsers and label
 * tables. No React, no engine access; tests cover them directly.
 */
import { t } from "../i18n";
import type {
  AggFn,
  CellPos,
  DateUnit,
  DeleteContentsFlags,
  NameLabelFlags,
  Op,
  RangeDto,
  SeriesKind,
  SheetCell,
  SheetRange,
  SortKey,
  SortOptions,
  StdCondition,
  StdConnector,
  StdFilterOp,
  StdFilterOptions,
  SubtotalGroup,
  SubtotalOptions,
} from "@tally/bridge";
import { parseA1Range } from "../a1";
import { a1 } from "../dataOps";

// ---------------------------------------------------------------------------
// Standard / advanced filter
// ---------------------------------------------------------------------------

/** The 16 standard-filter conditions (AOO order). */
export const STD_FILTER_OPS: readonly { op: StdFilterOp; label: string; numeric?: boolean }[] = [
  { op: "eq", label: "=" },
  { op: "lt", label: "<" },
  { op: "gt", label: ">" },
  { op: "le", label: "<=" },
  { op: "ge", label: ">=" },
  { op: "ne", label: "<>" },
  { op: "largest", get label() { return t("lbl.stdFilter.largest", "Largest"); }, numeric: true },
  { op: "smallest", get label() { return t("lbl.stdFilter.smallest", "Smallest"); }, numeric: true },
  { op: "largestPct", get label() { return t("lbl.stdFilter.largestPct", "Largest %"); }, numeric: true },
  { op: "smallestPct", get label() { return t("lbl.stdFilter.smallestPct", "Smallest %"); }, numeric: true },
  { op: "contains", get label() { return t("lbl.stdFilter.contains", "Contains"); } },
  { op: "notContains", get label() { return t("lbl.stdFilter.notContains", "Does not contain"); } },
  { op: "beginsWith", get label() { return t("lbl.stdFilter.beginsWith", "Begins with"); } },
  { op: "notBeginsWith", get label() { return t("lbl.stdFilter.notBeginsWith", "Does not begin with"); } },
  { op: "endsWith", get label() { return t("lbl.stdFilter.endsWith", "Ends with"); } },
  { op: "notEndsWith", get label() { return t("lbl.stdFilter.notEndsWith", "Does not end with"); } },
];

/** One row of the Standard Filter dialog; `col === null` leaves the row unused. */
export interface FilterRow {
  connector: StdConnector;
  col: number | null;
  op: StdFilterOp;
  value: string;
}

export const DEFAULT_FILTER_OPTIONS: StdFilterOptions = { caseSensitive: false, regex: false, hasHeader: true, noDuplicates: false, copyTo: null, keepCriteria: true };

export function emptyFilterRows(firstCol: number): FilterRow[] {
  return [
    { connector: "and", col: firstCol, op: "eq", value: "" },
    { connector: "and", col: null, op: "eq", value: "" },
    { connector: "and", col: null, op: "eq", value: "" },
    { connector: "and", col: null, op: "eq", value: "" },
  ];
}

/** The used rows as conditions (rows after the first unused one are ignored, as in AOO). */
export function filterConditions(rows: readonly FilterRow[]): StdCondition[] {
  const out: StdCondition[] = [];
  for (const r of rows) {
    if (r.col === null) break;
    out.push({ connector: r.connector, col: r.col, op: r.op, value: r.value });
  }
  return out;
}

/** Why the filter cannot run (`null` = fine). */
export function filterError(conds: readonly StdCondition[], options: StdFilterOptions): string | null {
  if (conds.length === 0) return t("lbl.err.firstField", "Choose a field for the first condition.");
  for (const c of conds) {
    const meta = STD_FILTER_OPS.find((o) => o.op === c.op);
    if (meta?.numeric && !(Number(c.value) > 0)) return t("lbl.err.positive", "“{name}” needs a positive number.", { name: meta.label });
    if (options.regex) {
      try {
        new RegExp(c.value);
      } catch {
        return t("lbl.err.regex", "“{value}” is not a valid regular expression.", { value: c.value });
      }
    }
  }
  return null;
}

export function standardFilterOp(sheet: number, range: RangeDto, conditions: StdCondition[], options: StdFilterOptions): Op {
  return { type: "standardFilter", sheet, range: { ...range }, conditions, options: { ...options, copyTo: options.copyTo ? { ...options.copyTo } : null } };
}

export function advancedFilterOp(sheet: number, range: RangeDto, criteria: SheetRange, options: StdFilterOptions): Op {
  return { type: "advancedFilter", sheet, range: { ...range }, criteria: { sheet: criteria.sheet, range: { ...criteria.range } }, options: { ...options } };
}

// ---------------------------------------------------------------------------
// Subtotals
// ---------------------------------------------------------------------------

/** The 11 subtotal functions (AOO's list) with their SUBTOTAL function numbers. */
export const SUBTOTAL_FNS: readonly { fn: AggFn; label: string; n: number }[] = [
  { fn: "sum", get label() { return t("pivot.sum", "Sum"); }, n: 9 },
  { fn: "count", get label() { return t("pivot.count", "Count"); }, n: 3 },
  { fn: "average", get label() { return t("pivot.average", "Average"); }, n: 1 },
  { fn: "max", get label() { return t("pivot.max", "Max"); }, n: 4 },
  { fn: "min", get label() { return t("pivot.min", "Min"); }, n: 5 },
  { fn: "product", get label() { return t("pivot.product", "Product"); }, n: 6 },
  { fn: "countNums", get label() { return t("lbl.subtotal.countNums", "Count (numbers only)"); }, n: 2 },
  { fn: "stDev", get label() { return t("lbl.subtotal.stDev", "StDev (sample)"); }, n: 7 },
  { fn: "stDevP", get label() { return t("lbl.subtotal.stDevP", "StDevP (population)"); }, n: 8 },
  { fn: "var", get label() { return t("lbl.subtotal.var", "Var (sample)"); }, n: 10 },
  { fn: "varP", get label() { return t("lbl.subtotal.varP", "VarP (population)"); }, n: 11 },
];

/** One of the three group tabs; `groupBy === null` means unused. */
export interface SubtotalTab {
  groupBy: number | null;
  columns: number[];
  function: AggFn;
}

export const DEFAULT_SUBTOTAL_OPTIONS: SubtotalOptions = { pageBreaks: false, caseSensitive: false, presort: true, ascending: true, includeFormats: true };

export function emptySubtotalTabs(): SubtotalTab[] {
  return [0, 1, 2].map(() => ({ groupBy: null, columns: [], function: "sum" }));
}

export function subtotalGroups(tabs: readonly SubtotalTab[]): SubtotalGroup[] {
  return tabs.filter((t): t is SubtotalTab & { groupBy: number } => t.groupBy !== null && t.columns.length > 0).map((t) => ({ groupBy: t.groupBy, columns: [...t.columns].sort((a, b) => a - b), function: t.function }));
}

export function subtotalsOp(sheet: number, range: RangeDto, groups: SubtotalGroup[], options: SubtotalOptions): Op {
  return { type: "subtotals", sheet, range: { ...range }, groups, options: { ...options } };
}

export function removeSubtotalsOp(sheet: number, range: RangeDto): Op {
  return { type: "removeSubtotals", sheet, range: { ...range } };
}

// ---------------------------------------------------------------------------
// Consolidate
// ---------------------------------------------------------------------------

export interface ConsolidateState {
  function: AggFn;
  sources: string[];
  dest: string;
  byRowLabels: boolean;
  byColLabels: boolean;
  link: boolean;
}

/** `Sheet1.A1:B5` or `A1:B5` (current sheet) as a `SheetRange`. */
export function parseSheetRange(text: string, sheetByName: (name: string) => number | null, currentSheet: number): SheetRange | null {
  const t = text.trim().replace(/^=/, "").replace(/\$/g, "");
  const m = /^(?:(?:'([^']+)'|([^.!':]+))[.!])?([A-Za-z]{1,3}\d{1,7}(?::[A-Za-z]{1,3}\d{1,7})?)$/.exec(t);
  if (!m) return null;
  const name = m[1] ?? m[2] ?? null;
  const range = parseA1Range(m[3] ?? "");
  if (!range) return null;
  if (name === null) return { sheet: currentSheet, range };
  const sheet = sheetByName(name);
  return sheet === null ? null : { sheet, range };
}

export function parseSheetCell(text: string, sheetByName: (name: string) => number | null, currentSheet: number): SheetCell | null {
  const sr = parseSheetRange(text, sheetByName, currentSheet);
  return sr ? { sheet: sr.sheet, r: sr.range.r0, c: sr.range.c0 } : null;
}

export function consolidateOp(st: ConsolidateState, sheetByName: (name: string) => number | null, currentSheet: number): Op | string {
  const sources = st.sources.map((s) => parseSheetRange(s, sheetByName, currentSheet));
  if (sources.length === 0) return t("lbl.err.addSource", "Add at least one source range.");
  const bad = st.sources.find((_, i) => sources[i] === null);
  if (bad !== undefined) return t("lbl.err.notRange", "“{value}” is not a range.", { value: bad });
  const dest = parseSheetCell(st.dest, sheetByName, currentSheet);
  if (!dest) return t("lbl.err.destCell", "Choose a destination cell.");
  return { type: "consolidate", function: st.function, sources: sources as SheetRange[], dest, byRowLabels: st.byRowLabels, byColLabels: st.byColLabels, link: st.link };
}

// ---------------------------------------------------------------------------
// Remove duplicates, names from labels, database range
// ---------------------------------------------------------------------------

export function removeDuplicatesOp(sheet: number, range: RangeDto, cols: number[], hasHeader: boolean): Op {
  return { type: "removeDuplicates", sheet, range: { ...range }, cols: [...cols].sort((a, b) => a - b), hasHeader };
}

export function createNamesOp(sheet: number, range: RangeDto, from: NameLabelFlags): Op {
  return { type: "createNamesFromLabels", sheet, range: { ...range }, from: { ...from } };
}

/** Database range names: like defined names but spaces are allowed in AOO; we keep the stricter rule. */
export function isValidDbRangeName(name: string): boolean {
  return /^[A-Za-z_][A-Za-z0-9_.]*$/.test(name) && !parseA1Range(name);
}

export function defineDatabaseRangeOp(name: string, sheet: number, range: RangeDto, hasHeader: boolean): Op {
  return { type: "defineDatabaseRange", name, sheet, range: { ...range }, hasHeader };
}

// ---------------------------------------------------------------------------
// Fill series
// ---------------------------------------------------------------------------

export interface FillSeriesState {
  direction: "down" | "right" | "up" | "left";
  kind: SeriesKind;
  unit: DateUnit;
  start: string;
  end: string;
  increment: string;
}

export const DEFAULT_FILL_SERIES: FillSeriesState = { direction: "down", kind: "linear", unit: "day", start: "", end: "", increment: "1" };

export function fillSeriesOp(sheet: number, range: RangeDto, st: FillSeriesState): Op {
  const opt = (s: string): string | null => (s.trim() === "" ? null : s.trim());
  return { type: "fillSeries", sheet, range: { ...range }, direction: st.direction, kind: st.kind, unit: st.kind === "date" ? st.unit : null, start: opt(st.start), end: opt(st.end), increment: st.kind === "autoFill" ? null : opt(st.increment) };
}

/** A preview of a linear/growth/date series (what the first cells will hold). */
export function previewSeries(st: FillSeriesState, count: number, first: string): string[] {
  const n = Math.max(0, Math.min(count, 8));
  const start = Number(st.start.trim() === "" ? first : st.start);
  const inc = Number(st.increment === "" ? "1" : st.increment);
  if (!Number.isFinite(start) || !Number.isFinite(inc)) return [];
  const end = st.end.trim() === "" ? null : Number(st.end);
  const out: string[] = [];
  let v = start;
  for (let i = 0; i < n; i++) {
    if (end !== null && Number.isFinite(end) && (inc >= 0 ? v > end : v < end)) break;
    out.push(String(Number(v.toPrecision(12))));
    v = st.kind === "growth" ? v * inc : v + inc;
  }
  return out;
}

// ---------------------------------------------------------------------------
// Delete contents, insert/delete cells, sort options
// ---------------------------------------------------------------------------

export const DELETE_ALL: DeleteContentsFlags = { text: true, numbers: true, dates: true, formulas: true, notes: true, formats: false, objects: false };
/** AOO's dialog defaults: everything but formats and objects. */
export const DEFAULT_DELETE: DeleteContentsFlags = { text: true, numbers: true, dates: true, formulas: true, notes: true, formats: false, objects: false };

export const DELETE_FLAG_LABELS: readonly { key: keyof DeleteContentsFlags; label: string }[] = [
  { key: "text", label: "Text" },
  { key: "numbers", label: "Numbers" },
  { key: "dates", label: "Date & time" },
  { key: "formulas", label: "Formulas" },
  { key: "notes", label: "Notes" },
  { key: "formats", label: "Formats" },
  { key: "objects", label: "Objects" },
];

export function deleteContentsOp(sheet: number, ranges: readonly RangeDto[], what: DeleteContentsFlags): Op {
  return { type: "deleteContents", sheet, ranges: ranges.map((r) => ({ ...r })), what: { ...what } };
}

export function insertCellsOp(sheet: number, range: RangeDto, shift: "down" | "right"): Op {
  return { type: "insertCells", sheet, range: { ...range }, shift };
}

export function deleteCellsOp(sheet: number, range: RangeDto, shift: "up" | "left"): Op {
  return { type: "deleteCells", sheet, range: { ...range }, shift };
}

export interface SortOptionsState {
  keys: SortKey[];
  hasHeader: boolean;
  caseSensitive: boolean;
  leftToRight: boolean;
  copyTo: string;
  customList: number | null;
  includeFormats: boolean;
  natural: boolean;
}

export function sortWithOptionsOp(sheet: number, range: RangeDto, st: SortOptionsState, lists: readonly string[][], parseCopyTo: (s: string) => SheetCell | null): Op | string {
  const copyTo = st.copyTo.trim() === "" ? null : parseCopyTo(st.copyTo);
  if (st.copyTo.trim() !== "" && !copyTo) return t("lbl.err.notCell", "“{value}” is not a cell.", { value: st.copyTo });
  if (st.keys.length === 0) return t("lbl.err.sortKey", "Choose a sort key.");
  const list = st.customList === null ? null : (lists[st.customList] ?? null);
  const options: SortOptions = { leftToRight: st.leftToRight, copyTo, customOrder: list ? [...list] : null, includeFormats: st.includeFormats, natural: st.natural };
  return { type: "sortRange", sheet, range: { ...range }, keys: st.keys.map((k) => ({ ...k })), hasHeader: st.hasHeader, caseSensitive: st.caseSensitive, options };
}

/** AOO's built-in sort lists (Tools ▸ Options ▸ Sort Lists). */
export const BUILTIN_SORT_LISTS: readonly string[][] = [
  ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"],
  ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"],
  ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"],
  ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"],
];

/** Parses the Sort Lists editor text (one entry per line, blanks dropped). */
export function parseSortList(text: string): string[] {
  return text.split(/\r?\n/).map((s) => s.trim()).filter((s) => s !== "");
}

// ---------------------------------------------------------------------------
// Multiple operations (writes MULTIPLE.OPERATIONS formulas)
// ---------------------------------------------------------------------------

export interface MultiOpsState {
  formulas: string;
  rowInput: string;
  colInput: string;
}

function cellRef(p: CellPos, absolute = true): string {
  const s = a1({ r0: p.r, c0: p.c, r1: p.r, c1: p.c });
  return absolute ? s.replace(/^([A-Z]+)(\d+)$/, "$$$1$$$2") : s;
}

/**
 * The ops of Data ▸ Multiple Operations over `range` (the selection):
 * one-variable tables fill the column right of (or the row below) the
 * variable values; two-variable tables fill the body. The formulas are
 * `=MULTIPLE.OPERATIONS(formula; colInput; colValue; rowInput; rowValue)`
 * as AOO writes them.
 */
export function multipleOperationsOps(sheet: number, range: RangeDto, st: MultiOpsState, parseCell: (s: string) => CellPos | null): Op[] | string {
  const formula = parseA1Range(st.formulas.trim().replace(/\$/g, ""));
  if (!formula) return t("lbl.err.formulaCells", "Choose the formula cell(s).");
  const rowInput = st.rowInput.trim() === "" ? null : parseCell(st.rowInput);
  const colInput = st.colInput.trim() === "" ? null : parseCell(st.colInput);
  if (st.rowInput.trim() !== "" && !rowInput) return t("lbl.err.notCell", "“{value}” is not a cell.", { value: st.rowInput });
  if (st.colInput.trim() !== "" && !colInput) return t("lbl.err.notCell", "“{value}” is not a cell.", { value: st.colInput });
  if (!rowInput && !colInput) return t("lbl.err.inputCell", "Choose a row or column input cell.");
  const f = cellRef({ r: formula.r0, c: formula.c0 });
  const rel = (p: CellPos) => cellRef(p, false);
  const ops: Op[] = [];
  if (rowInput && colInput) {
    // Two variables: the top-left cell holds the formula; first row = row values, first column = column values.
    for (let r = range.r0 + 1; r <= range.r1; r++) {
      for (let c = range.c0 + 1; c <= range.c1; c++) {
        const input = `=MULTIPLE.OPERATIONS(${f};${cellRef(colInput)};${rel({ r, c: range.c0 })};${cellRef(rowInput)};${rel({ r: range.r0, c })})`;
        ops.push({ type: "setCellInput", sheet, r, c, input });
      }
    }
    return ops;
  }
  if (colInput) {
    // Values down the first column; results in the next column(s), one per formula cell.
    const nF = formula.c1 - formula.c0 + 1;
    for (let r = range.r0; r <= range.r1; r++) {
      for (let k = 0; k < nF && range.c0 + 1 + k <= range.c1; k++) {
        const fk = cellRef({ r: formula.r0, c: formula.c0 + k });
        ops.push({ type: "setCellInput", sheet, r, c: range.c0 + 1 + k, input: `=MULTIPLE.OPERATIONS(${fk};${cellRef(colInput)};${rel({ r, c: range.c0 })})` });
      }
    }
    return ops;
  }
  const nF = formula.r1 - formula.r0 + 1;
  for (let c = range.c0; c <= range.c1; c++) {
    for (let k = 0; k < nF && range.r0 + 1 + k <= range.r1; k++) {
      const fk = cellRef({ r: formula.r0 + k, c: formula.c0 });
      ops.push({ type: "setCellInput", sheet, r: range.r0 + 1 + k, c, input: `=MULTIPLE.OPERATIONS(${fk};${cellRef(rowInput as CellPos)};${rel({ r: range.r0, c })})` });
    }
  }
  return ops;
}

// ---------------------------------------------------------------------------
// Scenarios, sheets
// ---------------------------------------------------------------------------

export const SCENARIO_COLORS: readonly { label: string; rgba: number }[] = [
  { get label() { return t("lbl.scenarioColor.lightBlue", "Light blue"); }, rgba: 0xbfdbfeff },
  { get label() { return t("lbl.scenarioColor.lightGreen", "Light green"); }, rgba: 0xbbf7d0ff },
  { get label() { return t("lbl.scenarioColor.lightYellow", "Light yellow"); }, rgba: 0xfef08aff },
  { get label() { return t("lbl.scenarioColor.lightRed", "Light red"); }, rgba: 0xfecacaff },
  { get label() { return t("lbl.scenarioColor.lavender", "Lavender"); }, rgba: 0xddd6feff },
  { get label() { return t("lbl.scenarioColor.grey", "Grey"); }, rgba: 0xe5e7ebff },
];

/** The tab colours offered by Sheet ▸ Tab Colour (AOO's standard palette row). */
export const TAB_COLORS: readonly { id: string; label: string; rgba: number }[] = [
  { id: "red", get label() { return t("lbl.tabColor.red", "Red"); }, rgba: 0xef4444ff },
  { id: "orange", get label() { return t("lbl.tabColor.orange", "Orange"); }, rgba: 0xf97316ff },
  { id: "yellow", get label() { return t("lbl.tabColor.yellow", "Yellow"); }, rgba: 0xeab308ff },
  { id: "green", get label() { return t("lbl.tabColor.green", "Green"); }, rgba: 0x22c55eff },
  { id: "teal", get label() { return t("lbl.tabColor.teal", "Teal"); }, rgba: 0x14b8a6ff },
  { id: "blue", get label() { return t("lbl.tabColor.blue", "Blue"); }, rgba: 0x3b82f6ff },
  { id: "purple", get label() { return t("lbl.tabColor.purple", "Purple"); }, rgba: 0xa855f7ff },
  { id: "pink", get label() { return t("lbl.tabColor.pink", "Pink"); }, rgba: 0xec4899ff },
  { id: "grey", get label() { return t("lbl.tabColor.grey", "Grey"); }, rgba: 0x6b7280ff },
];

/** `Name_2`, `Name_3`… — the first copy name not taken. */
export function copySheetName(name: string, taken: readonly string[]): string {
  const lower = new Set(taken.map((n) => n.toLowerCase()));
  const base = name.replace(/_\d+$/, "");
  for (let i = 2; i < 10_000; i++) {
    const candidate = `${base}_${i}`;
    if (!lower.has(candidate.toLowerCase())) return candidate;
  }
  return `${base}_copy`;
}

/** Outline helpers: the deepest level present, 0 without groups. */
export function maxOutlineLevel(groups: readonly { level: number }[]): number {
  return groups.reduce((m, g) => Math.max(m, g.level), 0);
}

/** Rows/columns of `range` to group: whole rows when the selection spans every column, else by the longer side. */
export function groupAxisFor(range: RangeDto, wholeRows: boolean, wholeCols: boolean): "rows" | "cols" | null {
  if (wholeRows && !wholeCols) return "rows";
  if (wholeCols && !wholeRows) return "cols";
  if (wholeRows && wholeCols) return null;
  return range.r1 - range.r0 >= range.c1 - range.c0 ? "rows" : "cols";
}

/** The merge prompt (§51): how many hidden (non-top-left) cells hold content. */
export function hiddenContentCount(range: RangeDto, filled: readonly CellPos[]): number {
  return filled.filter((p) => !(p.r === range.r0 && p.c === range.c0)).length;
}

/** Moves the hidden cells' texts into the top-left cell (one line each), as AOO's "move contents" does. */
export function mergedContent(texts: readonly { pos: CellPos; input: string }[], range: RangeDto): string {
  const ordered = [...texts].filter((t) => t.input !== "").sort((a, b) => a.pos.r - b.pos.r || a.pos.c - b.pos.c);
  void range;
  return ordered.map((t) => t.input).join("\n");
}
