/**
 * Pure builders for the v0.2 engine ops (sort, AutoFilter, borders, names,
 * freeze, paste special, find) so the UI logic can be unit-tested without
 * an engine. The controller calls these and applies the result.
 */
import { t } from "./i18n";
import type {
  AutoFilterInfo,
  BorderLineDto,
  BordersPatch,
  CellPos,
  ColumnFilter,
  FilterOp,
  FindMode,
  FindQuery,
  Op,
  PasteOperation,
  PasteOptions,
  PasteSelection,
  RangeDto,
  SortKey,
} from "@tally/bridge";
import { colToLetters, type FilterButtons } from "@tally/grid";

// -- borders ------------------------------------------------------------------

export type BorderPreset = "all" | "outer" | "inner" | "top" | "bottom" | "left" | "right" | "none";
export type LineWeight = "thin" | "medium" | "thick";

export const BORDER_PRESETS: readonly { id: BorderPreset; label: string }[] = [
  { id: "all", get label() { return t("cells.allBorders", "All borders"); } },
  { id: "outer", get label() { return t("lbl.border.outer", "Outer borders"); } },
  { id: "inner", get label() { return t("lbl.border.inner", "Inner borders"); } },
  { id: "top", get label() { return t("lbl.border.top", "Top border"); } },
  { id: "bottom", get label() { return t("lbl.border.bottom", "Bottom border"); } },
  { id: "left", get label() { return t("lbl.border.left", "Left border"); } },
  { id: "right", get label() { return t("lbl.border.right", "Right border"); } },
  { id: "none", get label() { return t("cells.noBorder", "No borders"); } },
];

/** Line width in px for each weight (Calc: 0.75 pt, 1.75 pt, 2.5 pt). */
export const LINE_WIDTH: Record<LineWeight, number> = { thin: 1, medium: 2, thick: 3 };

/** Default border colour: near-black. */
export const DEFAULT_BORDER_COLOR = 0x1f2937ff;

export function borderLine(weight: LineWeight, color: number = DEFAULT_BORDER_COLOR): BorderLineDto {
  return { style: "solid", widthPx: LINE_WIDTH[weight], color };
}

/**
 * The `StylePatch.borders` for a preset: `line` on the chosen edges.
 * "none" removes all six edge kinds; "outer" leaves inner lines alone,
 * "inner" leaves the outline alone.
 */
export function bordersPatch(preset: BorderPreset, line: BorderLineDto): BordersPatch {
  const set = { line };
  const off = { line: null };
  const p: BordersPatch = { top: null, bottom: null, left: null, right: null, innerH: null, innerV: null };
  switch (preset) {
    case "all":
      return { top: set, bottom: set, left: set, right: set, innerH: set, innerV: set };
    case "outer":
      return { ...p, top: set, bottom: set, left: set, right: set };
    case "inner":
      return { ...p, innerH: set, innerV: set };
    case "none":
      return { top: off, bottom: off, left: off, right: off, innerH: off, innerV: off };
    default:
      return { ...p, [preset]: set };
  }
}

// -- references and names ---------------------------------------------------

/** A sheet name as CalcA1 writes it: quoted when it is not a plain identifier. */
export function quoteSheet(name: string): string {
  return /^[A-Za-z_][A-Za-z0-9_]*$/.test(name) ? name : `'${name.replace(/'/g, "''")}'`;
}

/** `A1` / `A1:B5` (relative, for display). */
export function a1(range: RangeDto): string {
  const tl = `${colToLetters(range.c0)}${range.r0 + 1}`;
  return range.r0 === range.r1 && range.c0 === range.c1 ? tl : `${tl}:${colToLetters(range.c1)}${range.r1 + 1}`;
}

/** An absolute CalcA1 reference for `refersTo`: `$Sheet1.$A$1:$B$5`. */
export function absoluteRef(sheetName: string, range: RangeDto): string {
  const cell = (r: number, c: number) => `$${colToLetters(c)}$${r + 1}`;
  const head = `$${quoteSheet(sheetName)}.${cell(range.r0, range.c0)}`;
  return range.r0 === range.r1 && range.c0 === range.c1 ? head : `${head}:${cell(range.r1, range.c1)}`;
}

/**
 * Valid defined-name syntax (the engine re-checks): starts with a letter or
 * `_`, then letters, digits, `_` or `.`; not a cell reference (`AB12`),
 * not R1C1-like and not TRUE/FALSE.
 */
export function isValidName(name: string): boolean {
  if (!/^[A-Za-z_][A-Za-z0-9_.]*$/.test(name)) return false;
  if (/^[A-Za-z]{1,3}[0-9]+$/.test(name)) return false;
  if (/^[Rr][0-9]*[Cc][0-9]*$/.test(name)) return false;
  return !/^(true|false)$/i.test(name);
}

/** A suggested name from a header text: `Unit price` → `Unit_price`. */
export function suggestName(header: string): string {
  const s = header.trim().replace(/[^A-Za-z0-9_.]+/g, "_").replace(/^_+|_+$/g, "");
  const n = /^[A-Za-z_]/.test(s) ? s : `_${s}`;
  return isValidName(n) ? n : "";
}

// -- freeze -------------------------------------------------------------------

/** Freeze at the active cell: the rows above and columns left of it stay. */
export function freezeAt(active: CellPos, what: "both" | "rows" | "cols"): { rows: number; cols: number } {
  return { rows: what === "cols" ? 0 : active.r, cols: what === "rows" ? 0 : active.c };
}

// -- sort -----------------------------------------------------------------------

export interface SortSpec {
  range: RangeDto;
  keys: SortKey[];
  hasHeader: boolean;
  caseSensitive?: boolean;
}

export function sortOp(sheet: number, spec: SortSpec): Op {
  return {
    type: "sortRange",
    sheet,
    range: { ...spec.range },
    keys: spec.keys.slice(0, 3).map((k) => ({ ...k })),
    hasHeader: spec.hasHeader,
    caseSensitive: spec.caseSensitive ?? false,
  };
}

// -- AutoFilter -------------------------------------------------------------------

/** The grid's ▾ buttons for an AutoFilter (or `null` without one). */
export function filterButtonsFor(info: AutoFilterInfo | null): FilterButtons | null {
  const r = info?.range;
  if (!info || !r) return null;
  return { row: r.r0, c0: r.c0, c1: r.c1, active: info.columns.filter((c) => c.active !== null).map((c) => c.col) };
}

/** Condition operators of the filter popover (label, op, takes a value). */
export const FILTER_OPS: readonly { op: FilterOp; label: string; value: boolean }[] = [
  { op: "eq", get label() { return t("filter.op.eq", "equals"); }, value: true },
  { op: "ne", get label() { return t("filter.op.ne", "does not equal"); }, value: true },
  { op: "gt", get label() { return t("filter.op.gt", "greater than"); }, value: true },
  { op: "ge", get label() { return t("filter.op.ge", "at least"); }, value: true },
  { op: "lt", get label() { return t("filter.op.lt", "less than"); }, value: true },
  { op: "le", get label() { return t("filter.op.le", "at most"); }, value: true },
  { op: "contains", get label() { return t("filter.op.contains", "contains"); }, value: true },
  { op: "beginsWith", get label() { return t("filter.op.beginsWith", "begins with"); }, value: true },
  { op: "endsWith", get label() { return t("filter.op.endsWith", "ends with"); }, value: true },
  { op: "empty", get label() { return t("filter.op.empty", "is empty"); }, value: false },
  { op: "notEmpty", get label() { return t("filter.op.notEmpty", "is not empty"); }, value: false },
  // AOO's Largest / Smallest / Largest % / Smallest % (N in the value; "Top 10" is largest 10).
  { op: "topValues", get label() { return t("filter.op.topValues", "largest"); }, value: true },
  { op: "bottomValues", get label() { return t("filter.op.bottomValues", "smallest"); }, value: true },
  { op: "topPercent", get label() { return t("filter.op.topPercent", "largest %"); }, value: true },
  { op: "bottomPercent", get label() { return t("filter.op.bottomPercent", "smallest %"); }, value: true },
];

/** AOO's AutoFilter "Top 10": the 10 largest values of the column (ties included). */
export const TOP_10_FILTER: ColumnFilter = { values: null, condition: { op: "topValues", value: "10" } };

/**
 * The column filter from the popover state: `values` only when some (not
 * all) values are checked; a condition when an operator is chosen (and has
 * a value if it needs one). `null` clears the column's filter.
 */
export function columnFilterFrom(
  all: readonly string[],
  checked: ReadonlySet<string>,
  cond: { op: FilterOp | null; value: string },
): ColumnFilter | null {
  const allChecked = all.every((v) => checked.has(v));
  const values = allChecked ? null : all.filter((v) => checked.has(v));
  const needs = FILTER_OPS.find((o) => o.op === cond.op)?.value ?? true;
  const condition = cond.op && (!needs || cond.value.trim() !== "") ? { op: cond.op, value: needs ? cond.value.trim() : "" } : null;
  if (values === null && condition === null) return null;
  return { values, condition };
}

export function columnFilterOp(sheet: number, col: number, filter: ColumnFilter | null): Op {
  return { type: "setColumnFilter", sheet, col, filter };
}

// -- paste special ----------------------------------------------------------------

/**
 * A Paste Special choice (AOO's ScInsertContentsDlg). `what` is "Paste all",
 * one of Tally's quick presets ("values": text, numbers and dates, formula
 * results as values; "formats") or "custom" with `kinds`.
 */
export interface PasteSpecialChoice {
  what: "all" | "values" | "formats" | "custom";
  /** The Selection check boxes, for `what: "custom"`. */
  kinds?: PasteSelection;
  transpose: boolean;
  skipEmpty: boolean;
  operation: PasteOperation;
  /** Paste references to the source cells (AOO "Link"). */
  link?: boolean;
  /** Insert cells first (AOO "Shift cells"). */
  shift?: PasteShift;
}

export type PasteShift = "none" | "down" | "right";

export type PasteKind = keyof PasteSelection;

/** The Selection check boxes in AOO's order. */
export const PASTE_KINDS: readonly PasteKind[] = ["text", "numbers", "dateTime", "formulas", "notes", "formats"];

export const ALL_KINDS: PasteSelection = { text: true, numbers: true, dateTime: true, formulas: true, notes: true, formats: true };
const VALUES_KINDS: PasteSelection = { text: true, numbers: true, dateTime: true, formulas: false, notes: false, formats: false };
const FORMATS_KINDS: PasteSelection = { text: false, numbers: false, dateTime: false, formulas: false, notes: false, formats: true };

export const DEFAULT_PASTE_SPECIAL: PasteSpecialChoice = { what: "all", transpose: false, skipEmpty: false, operation: "none" };

const sameKinds = (a: PasteSelection, b: PasteSelection): boolean => PASTE_KINDS.every((k) => a[k] === b[k]);

/** The check boxes a choice stands for. */
export function pasteKinds(choice: PasteSpecialChoice): PasteSelection {
  switch (choice.what) {
    case "all":
      return { ...ALL_KINDS };
    case "values":
      return { ...VALUES_KINDS };
    case "formats":
      return { ...FORMATS_KINDS };
    case "custom":
      return { ...(choice.kinds ?? ALL_KINDS) };
  }
}

/** The choice for the dialog's "Paste all" box and Selection boxes (a preset when they match one). */
export function pasteWhat(all: boolean, kinds: PasteSelection): Pick<PasteSpecialChoice, "what" | "kinds"> {
  if (all) return { what: "all" };
  if (sameKinds(kinds, VALUES_KINDS)) return { what: "values" };
  if (sameKinds(kinds, FORMATS_KINDS)) return { what: "formats" };
  return { what: "custom", kinds: { ...kinds } };
}

/** Whether the choice pastes cell contents (text, numbers, dates or formulas): operations apply to them only. */
export function pastesCells(choice: PasteSpecialChoice): boolean {
  const k = pasteKinds(choice);
  return k.text || k.numbers || k.dateTime || k.formulas;
}

/** Whether OK can paste anything (some kind, or Link with a cell kind). */
export function pasteChoiceValid(choice: PasteSpecialChoice): boolean {
  const k = pasteKinds(choice);
  return PASTE_KINDS.some((x) => k[x]);
}

/** Whether the "pasting over data" question applies (AOO: not with an operation, a shift or formats/notes only). */
export function pasteAsksOverwrite(choice: PasteSpecialChoice): boolean {
  return pastesCells(choice) && choice.operation === "none" && (choice.shift ?? "none") === "none";
}

/**
 * Which Shift cells directions the dialog offers (AOO cellsh1.cxx:1296-1316:
 * none that would push the block from the cursor past the sheet's last
 * column or row; the size is the copied block's, untransposed).
 */
export function pasteShiftAllowed(at: { r: number; c: number }, block: { rows: number; cols: number } | null, maxRows = 1_048_576, maxCols = 16_384): { down: boolean; right: boolean } {
  if (!block) return { down: true, right: true };
  return { down: at.r + block.rows - 1 < maxRows - 1, right: at.c + block.cols - 1 < maxCols - 1 };
}

export function pasteOptions(choice: PasteSpecialChoice): PasteOptions {
  const out: PasteOptions = {
    valuesOnly: choice.what === "values",
    formatsOnly: choice.what === "formats",
    transpose: choice.transpose,
    skipEmpty: choice.skipEmpty,
    operation: pastesCells(choice) ? choice.operation : "none",
  };
  if (choice.what === "custom") out.selection = pasteKinds(choice);
  if (choice.link) out.link = true;
  if (choice.shift && choice.shift !== "none") out.shift = choice.shift;
  return out;
}

// -- find -------------------------------------------------------------------------

export interface FindOptions {
  matchCase: boolean;
  wholeCell: boolean;
  inFormulas: boolean;
  /** AOO's Search in "Notes": search note (comment) text instead of the cells. */
  inNotes?: boolean;
  mode: FindMode;
  /** Search every sheet instead of the active one. */
  allSheets: boolean;
  /**
   * "Current selection only": the sheet and ranges selected when it was
   * turned on (stepping through hits moves the selection, so it is kept here).
   */
  scope?: { sheet: number; ranges: RangeDto[] } | null;
}

export const DEFAULT_FIND: FindOptions = { matchCase: false, wholeCell: false, inFormulas: false, inNotes: false, mode: "literal", allSheets: false, scope: null };

/** AOO's Find & Replace "Search in" choice of `opts`. */
export function searchIn(opts: FindOptions): "formulas" | "values" | "notes" {
  return opts.inNotes ? "notes" : opts.inFormulas ? "formulas" : "values";
}

export function findQuery(text: string, opts: FindOptions, sheet: number, from: CellPos | null): FindQuery {
  const scope = opts.scope ?? null;
  return {
    text,
    // Current selection only searches the selection's sheet (as AOO, not all sheets).
    sheet: scope ? scope.sheet : opts.allSheets ? null : sheet,
    inFormulas: opts.inFormulas && !opts.inNotes,
    matchCase: opts.matchCase,
    wholeCell: opts.wholeCell,
    mode: opts.mode,
    from,
    inNotes: opts.inNotes ? true : null,
    selection: scope ? scope.ranges.map((r) => ({ ...r })) : null,
  };
}

/** Next/previous hit index, wrapping (−1 when there are no hits). */
export function stepHit(index: number, count: number, dir: 1 | -1): number {
  if (count === 0) return -1;
  if (index < 0) return dir > 0 ? 0 : count - 1;
  return (index + dir + count) % count;
}

// -- array formulas -------------------------------------------------------------

/** `{=SUM(A1:A3*B1:B3)}` → `=SUM(A1:A3*B1:B3)`; plain text is returned as is. */
export function stripArrayBraces(text: string): string {
  const t = text.trim();
  return t.startsWith("{=") && t.endsWith("}") ? t.slice(1, -1) : t;
}

/** The `setArrayFormula` op for an editor text over `range`, or `null` if it is not a formula. */
export function arrayFormulaOp(sheet: number, range: RangeDto, text: string): Op | null {
  const input = stripArrayBraces(text);
  if (!input.startsWith("=") || input.length < 2) return null;
  return { type: "setArrayFormula", sheet, range: { ...range }, input };
}

// -- fill ---------------------------------------------------------------------------

/**
 * Edit ▸ Fill ▸ Down / Right / Up / Left (AOO `ScDocFunc::FillSimple`,
 * docfunc.cxx:3996-4015): the source is the first row (Down), first column
 * (Right), last row (Up) or last column (Left) of the selection, copied
 * over the rest. ⌘D / ⌘R on a single row / column copy the one before it
 * (Excel); Up / Left there do nothing, as in AOO (`nCount` is 0). `null`
 * when there is nothing to fill.
 */
export function fillDownRight(sel: RangeDto, dir: "down" | "right" | "up" | "left"): { source: RangeDto; target: RangeDto } | null {
  switch (dir) {
    case "down":
      if (sel.r1 === sel.r0) return sel.r0 > 0 ? { source: { ...sel, r0: sel.r0 - 1, r1: sel.r0 - 1 }, target: { ...sel, r0: sel.r0 - 1 } } : null;
      return { source: { ...sel, r1: sel.r0 }, target: { ...sel } };
    case "right":
      if (sel.c1 === sel.c0) return sel.c0 > 0 ? { source: { ...sel, c0: sel.c0 - 1, c1: sel.c0 - 1 }, target: { ...sel, c0: sel.c0 - 1 } } : null;
      return { source: { ...sel, c1: sel.c0 }, target: { ...sel } };
    case "up":
      return sel.r1 === sel.r0 ? null : { source: { ...sel, r0: sel.r1 }, target: { ...sel } };
    case "left":
      return sel.c1 === sel.c0 ? null : { source: { ...sel, c0: sel.c1 }, target: { ...sel } };
  }
}

// -- friendly errors ------------------------------------------------------------------

/**
 * The detail of a reader's limit refusal, or `null` if `message` is not one.
 * Current readers say "limit exceeded: … (<what> over <cap>)"; older ones
 * (XLS, dBase, HTML) say "limit exceeded: <what> (limit <cap>)", which
 * becomes "<what> over <cap>" too.
 */
export function limitDetail(message: string): string | null {
  const m = /^\s*limit exceeded:\s*(.*?)\s*$/s.exec(message);
  if (!m) return null;
  const body = m[1] ?? "";
  const over = /\(([^()]*\sover\s[^()]*)\)$/.exec(body);
  if (over) return (over[1] ?? "").trim();
  const old = /^(.*?)\s*\(limit:?\s*([^()]*)\)$/.exec(body);
  if (old) return `${(old[1] ?? "").trim()} over ${(old[2] ?? "").trim()}`;
  return body;
}

/** User-facing text for engine refusals (OpError codes). */
export function friendlyError(code: string, message: string): string {
  switch (code) {
    case "array_block":
      return t("lbl.err.arrayBlock", "You can't change part of an array formula. Select the whole block, edit it, and press ⌃⇧↩.");
    case "merged":
      return t("lbl.err.merged", "That would split a merged cell. Unmerge it first (Format ▸ Unmerge Cells).");
    case "bad_file": {
      const what = limitDetail(message);
      if (what === null) return message;
      return t(
        "error.fileTooLarge",
        "This file is too large or too complex to open safely. Tally refuses it to protect your computer (limit: {what}).",
        { what },
      );
    }
    default:
      return message;
  }
}

/** Parses an absolute CalcA1 reference (`$Sheet1.$A$1:$B$5`, `$'My sheet'.$C$3`) into a sheet name and range. */
export function parseAbsoluteRef(text: string): { sheet: string; range: RangeDto } | null {
  const m = /^\$?(?:'((?:[^']|'')+)'|([A-Za-z_][A-Za-z0-9_ ]*))\.\$?([A-Za-z]{1,3})\$?(\d+)(?::\$?([A-Za-z]{1,3})\$?(\d+))?$/.exec(text.trim());
  if (!m) return null;
  const sheet = m[1] !== undefined ? m[1].replace(/''/g, "'") : (m[2] ?? "");
  const col = (s: string) => [...s.toUpperCase()].reduce((n, ch) => n * 26 + ch.charCodeAt(0) - 64, 0) - 1;
  const c0 = col(m[3] ?? "A");
  const r0 = Number(m[4]) - 1;
  const c1 = m[5] ? col(m[5]) : c0;
  const r1 = m[6] ? Number(m[6]) - 1 : r0;
  return { sheet, range: { r0: Math.min(r0, r1), c0: Math.min(c0, c1), r1: Math.max(r0, r1), c1: Math.max(c0, c1) } };
}
