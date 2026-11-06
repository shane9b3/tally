/**
 * Format Cells (⌘1, §4 item 29): the dialog's state (one object per tab)
 * seeded from the active cell's resolved `StyleDto`, and the `StylePatch`
 * that writes back only what changed. Borders include the diagonals; the
 * Protection tab becomes `Op::SetCellProtection`.
 */
import type { BorderEdgePatch, BorderLineDto, BorderStyleDto, BordersPatch, HAlign, StyleDto, StylePatch, UnderlineStyle, VAlign } from "@tally/bridge";
import { EMPTY_PATCH } from "../formats";
import { APP_FONT, sameFont, typefaceFields, type SelectionFont, type Typeface } from "../fonts/fontModel";

/** Font fields a mixed selection leaves indeterminate (blank) in the dialog. */
export type MixedField = "fontFamily" | "fontSizePt" | "typeface";
export type MixedFields = Partial<Record<MixedField, true>>;

/** The indeterminate fields of a selection's font. */
export function mixedOf(sel: SelectionFont | undefined): MixedFields {
  const m: MixedFields = {};
  if (!sel) return m;
  if (sel.family === null) m.fontFamily = true;
  if (sel.sizePt === null) m.fontSizePt = true;
  if (sel.bold === null || sel.italic === null) m.typeface = true;
  return m;
}

export type BorderEdge = "top" | "bottom" | "left" | "right" | "innerH" | "innerV" | "diagonalDown" | "diagonalUp";

export const BORDER_EDGES: readonly { edge: BorderEdge; label: string }[] = [
  { edge: "top", label: "Top" },
  { edge: "bottom", label: "Bottom" },
  { edge: "left", label: "Left" },
  { edge: "right", label: "Right" },
  { edge: "innerH", label: "Inner horizontal" },
  { edge: "innerV", label: "Inner vertical" },
  { edge: "diagonalDown", label: "Diagonal ↘" },
  { edge: "diagonalUp", label: "Diagonal ↗" },
];

export const BORDER_STYLES: readonly { style: BorderStyleDto; label: string }[] = [
  { style: "solid", label: "Solid" },
  { style: "dashed", label: "Dashed" },
  { style: "dotted", label: "Dotted" },
  { style: "double", label: "Double" },
];

export const UNDERLINE_STYLES: readonly { style: UnderlineStyle; label: string }[] = [
  { style: "single", label: "Single" },
  { style: "double", label: "Double" },
  { style: "singleAccounting", label: "Single (accounting)" },
  { style: "doubleAccounting", label: "Double (accounting)" },
  { style: "dotted", label: "Dotted" },
  { style: "dashed", label: "Dashed" },
  { style: "wave", label: "Wave" },
  { style: "bold", label: "Bold" },
];

export const H_ALIGNS: readonly { align: HAlign; label: string }[] = [
  { align: "general", label: "Default" },
  { align: "left", label: "Left" },
  { align: "center", label: "Centre" },
  { align: "right", label: "Right" },
  { align: "justify", label: "Justified" },
];

export const V_ALIGNS: readonly { align: VAlign; label: string }[] = [
  { align: "bottom", label: "Bottom" },
  { align: "middle", label: "Middle" },
  { align: "top", label: "Top" },
];

export interface FormatCellsState {
  numberFormat: string;
  fontFamily: string;
  fontSizePt: number;
  bold: boolean;
  italic: boolean;
  underline: boolean;
  underlineStyle: UnderlineStyle;
  strike: boolean;
  color: number | null;
  hAlign: HAlign;
  vAlign: VAlign;
  wrap: boolean;
  indent: number;
  rotationDeg: number;
  stacked: boolean;
  shrinkToFit: boolean;
  /** Edge → line (null = remove; absent = unchanged). */
  borders: Partial<Record<BorderEdge, BorderLineDto | null>>;
  borderColor: number;
  borderStyle: BorderStyleDto;
  borderWidth: number;
  fill: number | null;
  locked: boolean;
  hiddenFormula: boolean;
  /** Cell protection "Hide all". */
  hidden: boolean;
  /** Cell protection "Hide when printing". */
  hidePrint: boolean;
  /** Font fields the selection mixed when the dialog opened. */
  mixed: MixedFields;
  /** Font fields still indeterminate (mixed and not set in the dialog). */
  indeterminate: MixedFields;
}

/** Fully transparent: clears a fill / resets the font colour to Automatic. */
export const NO_COLOR = 0xffffff00;

export const DEFAULT_BORDER_COLOR = 0x1f2937ff;

/** The dialog state for the active cell's style. */
export function stateFromStyle(s: StyleDto | undefined, mixed: MixedFields = {}): FormatCellsState {
  return {
    mixed: { ...mixed },
    indeterminate: { ...mixed },
    numberFormat: s?.numberFormat ?? "General",
    fontFamily: s?.fontFamily ?? "",
    fontSizePt: s?.fontSizePt ?? 11,
    bold: s?.bold ?? false,
    italic: s?.italic ?? false,
    underline: s?.underline ?? false,
    underlineStyle: s?.underlineStyle ?? "single",
    strike: s?.strike ?? false,
    color: s?.color ?? null,
    hAlign: s?.hAlign ?? "general",
    vAlign: s?.vAlign ?? "bottom",
    wrap: s?.wrap ?? false,
    indent: s?.indent ?? 0,
    rotationDeg: s?.rotationDeg ?? 0,
    stacked: s?.stacked ?? false,
    shrinkToFit: s?.shrinkToFit ?? false,
    borders: {},
    borderColor: DEFAULT_BORDER_COLOR,
    borderStyle: "solid",
    borderWidth: 1,
    fill: s?.fill ?? null,
    locked: s?.protection?.locked ?? true,
    hiddenFormula: s?.protection?.hiddenFormula ?? false,
    hidden: s?.protection?.hidden ?? false,
    hidePrint: s?.protection?.hidePrint ?? false,
  };
}

/** The Cell Protection tab's four check boxes (AOO `ScProtectionAttr`). */
export interface ProtectionState {
  locked: boolean;
  hiddenFormula: boolean;
  hidden: boolean;
  hidePrint: boolean;
}

/** The boxes over a selection: `null` = the selected cells differ (an indeterminate box). */
export type TriProtection = { [K in keyof ProtectionState]: boolean | null };

export const PROTECTION_KEYS: readonly (keyof ProtectionState)[] = ["locked", "hiddenFormula", "hidden", "hidePrint"];

/** The protection attributes of a style (the default: protected, nothing hidden). */
export function protectionOf(s: StyleDto | undefined): ProtectionState {
  const st = stateFromStyle(s);
  return { locked: st.locked, hiddenFormula: st.hiddenFormula, hidden: st.hidden, hidePrint: st.hidePrint };
}

/**
 * The boxes for the selected cells' styles: a flag the styles agree on is
 * definite, one they differ on is `null`. `undefined` styles (not loaded)
 * are skipped; none at all gives the default.
 */
export function selectionProtection(styles: Iterable<StyleDto | undefined>): TriProtection {
  let out: TriProtection | undefined;
  for (const s of styles) {
    if (!s) continue;
    const p = protectionOf(s);
    if (!out) {
      out = { ...p };
      continue;
    }
    for (const k of PROTECTION_KEYS) if (out[k] !== null && out[k] !== p[k]) out[k] = null;
  }
  return out ?? protectionOf(undefined);
}

/**
 * Which boxes are enabled (AOO `ScTabPageProtection::UpdateButtons`):
 * a checked "Hide all" disables "Protected" and "Hide formula" (it implies
 * both); their values are kept. An indeterminate "Hide all" leaves them
 * enabled. "Hide when printing" is always available.
 */
/**
 * The styles of the selected cells from the loaded cells (as
 * `selectionFont`): the active cell's, then each selected cell inside the
 * used range, at most `limit` cells; cells outside the used range are
 * empty, so one default style stands for them.
 */
export function selectionStyles(
  source: { getCell(r: number, c: number): { style: number } | undefined; getStyle(id: number): StyleDto | undefined },
  ranges: readonly { r0: number; c0: number; r1: number; c1: number }[],
  active: { r: number; c: number },
  used: { r0: number; c0: number; r1: number; c1: number } | null,
  limit = 10_000,
): (StyleDto | undefined)[] {
  const ids = new Set<number>([source.getCell(active.r, active.c)?.style ?? 0]);
  let budget = limit;
  for (const rg of ranges) {
    const r0 = used ? Math.max(rg.r0, used.r0) : 0;
    const r1 = used ? Math.min(rg.r1, used.r1) : -1;
    const c0 = used ? Math.max(rg.c0, used.c0) : 0;
    const c1 = used ? Math.min(rg.c1, used.c1) : -1;
    if (!used || r0 > rg.r0 || r1 < rg.r1 || c0 > rg.c0 || c1 < rg.c1) ids.add(0);
    for (let r = r0; r <= r1 && budget > 0; r++) {
      for (let c = c0; c <= c1 && budget > 0; c++, budget--) ids.add(source.getCell(r, c)?.style ?? 0);
    }
  }
  return [...ids].map((id) => source.getStyle(id));
}

export function protectionEnabled(p: TriProtection): Record<keyof ProtectionState, boolean> {
  return { hidden: true, locked: p.hidden !== true, hiddenFormula: p.hidden !== true, hidePrint: true };
}

/**
 * The `setCellProtection` fields: a definite value for each box the user
 * changed, `null` ("leave each cell as it is") for the others; `null`
 * when nothing changed.
 */
export function protectionChange(before: TriProtection, after: TriProtection): TriProtection | null {
  const out: TriProtection = { locked: null, hiddenFormula: null, hidden: null, hidePrint: null };
  let any = false;
  for (const k of PROTECTION_KEYS) {
    if (after[k] !== null && after[k] !== before[k]) {
      out[k] = after[k];
      any = true;
    }
  }
  return any ? out : null;
}

function edge(line: BorderLineDto | null | undefined): BorderEdgePatch | null {
  return line === undefined ? null : { line: line ? { ...line } : null };
}

/** The borders patch for the edges the user touched (`null` when none). */
export function bordersPatchOf(st: FormatCellsState): BordersPatch | null {
  const b = st.borders;
  if (Object.keys(b).length === 0) return null;
  return { top: edge(b.top), bottom: edge(b.bottom), left: edge(b.left), right: edge(b.right), innerH: edge(b.innerH), innerV: edge(b.innerV), diagonalDown: edge(b.diagonalDown), diagonalUp: edge(b.diagonalUp) };
}

/** The line the Borders tab currently draws with. */
export function currentLine(st: FormatCellsState): BorderLineDto {
  return { style: st.borderStyle, widthPx: st.borderWidth, color: st.borderColor };
}

/** Toggles an edge: sets the current line, or removes the border when it already has one. */
export function toggleEdge(st: FormatCellsState, e: BorderEdge): FormatCellsState {
  const cur = st.borders[e];
  const next = { ...st.borders };
  if (cur) next[e] = null;
  else next[e] = currentLine(st);
  return { ...st, borders: next };
}

export function presetEdges(st: FormatCellsState, preset: "none" | "outer" | "all"): FormatCellsState {
  const line = currentLine(st);
  const b: FormatCellsState["borders"] = {};
  const outer: BorderEdge[] = ["top", "bottom", "left", "right"];
  const inner: BorderEdge[] = ["innerH", "innerV"];
  const diagonals: BorderEdge[] = ["diagonalDown", "diagonalUp"];
  if (preset === "none") for (const e of [...outer, ...inner, ...diagonals]) b[e] = null;
  else {
    for (const e of outer) b[e] = line;
    for (const e of inner) b[e] = preset === "all" ? line : null;
  }
  return { ...st, borders: b };
}

/**
 * The `StylePatch` of the fields that differ from `base` (every field when
 * `base` is undefined). An empty font field means the document font
 * (`docFont`): the patch then clears the cells' own family ("").
 */
export function patchFromState(st: FormatCellsState, base: StyleDto | undefined, docFont: string = APP_FONT): StylePatch {
  const b = stateFromStyle(base);
  const diff = <T>(cur: T, was: T): T | null => (base === undefined || JSON.stringify(cur) !== JSON.stringify(was) ? cur : null);
  const face = (cur: boolean, was: boolean) => (st.indeterminate.typeface ? null : st.mixed.typeface ? cur : diff(cur, was));
  const patch: StylePatch = {
    ...EMPTY_PATCH,
    bold: face(st.bold, b.bold),
    italic: face(st.italic, b.italic),
    underline: diff(st.underline, b.underline),
    strike: diff(st.strike, b.strike),
    fontFamily: familyPatch(st, b, base === undefined, docFont),
    fontSizePt: st.indeterminate.fontSizePt ? null : st.mixed.fontSizePt ? st.fontSizePt : diff(st.fontSizePt, b.fontSizePt),
    // StylePatch has no "clear": an alpha-0 colour/fill is the engine-visible "Automatic" / "No fill".
    color: st.color === null && b.color !== null ? NO_COLOR : diff(st.color, b.color),
    fill: st.fill === null && b.fill !== null ? NO_COLOR : diff(st.fill, b.fill),
    hAlign: diff(st.hAlign, b.hAlign),
    vAlign: diff(st.vAlign, b.vAlign),
    wrap: diff(st.wrap, b.wrap),
    numberFormat: diff(st.numberFormat.trim() === "" ? "General" : st.numberFormat.trim(), b.numberFormat),
  };
  const borders = bordersPatchOf(st);
  if (borders) patch.borders = borders;
  const rot = diff(st.rotationDeg, b.rotationDeg);
  if (rot !== null) patch.rotationDeg = Math.max(-90, Math.min(90, Math.round(rot)));
  const stacked = diff(st.stacked, b.stacked);
  if (stacked !== null) patch.stacked = stacked;
  const shrink = diff(st.shrinkToFit, b.shrinkToFit);
  if (shrink !== null) patch.shrinkToFit = shrink;
  const indent = diff(st.indent, b.indent);
  if (indent !== null) patch.indent = Math.max(0, Math.min(250, Math.round(indent)));
  if (st.underline && (base === undefined || st.underlineStyle !== b.underlineStyle || !b.underline)) patch.underlineStyle = st.underlineStyle;
  return patch;
}

/** The family a field means: "" is the document font (`docFont`, the app font when the document has none). */
export function shownFamily(f: string, docFont: string = APP_FONT): string {
  return f.trim() === "" ? docFont : f.trim();
}

/** The family field of the patch: unchanged → null (unless the selection mixed families). */
function familyPatch(st: FormatCellsState, b: FormatCellsState, all: boolean, docFont: string): string | null {
  if (st.indeterminate.fontFamily) return null;
  const cur = st.fontFamily.trim();
  if (all) return cur === "" ? null : cur;
  if ( !st.mixed.fontFamily && sameFont(shownFamily(cur, docFont), shownFamily(b.fontFamily, docFont))) return null;
  return cur;
}

/** A named style's family from the font field: empty or the document font → `null` (the style follows the document font). */
export function styleFamily(field: string, docFont: string): string | null {
  const f = field.trim();
  return f === "" || sameFont(f, docFont) ? null : f;
}

/** Sets the typeface (bold + italic) and clears its indeterminate state. */
export function withTypeface(st: FormatCellsState, tf: Typeface): FormatCellsState {
  const { typeface: _done, ...rest } = st.indeterminate;
  return { ...st, ...typefaceFields(tf), indeterminate: rest };
}

/** Sets the family and clears its indeterminate state. */
export function withFamily(st: FormatCellsState, name: string): FormatCellsState {
  const { fontFamily: _done, ...rest } = st.indeterminate;
  return { ...st, fontFamily: name, indeterminate: rest };
}

/** Sets the size and clears its indeterminate state. */
export function withSize(st: FormatCellsState, pt: number): FormatCellsState {
  const { fontSizePt: _done, ...rest } = st.indeterminate;
  return { ...st, fontSizePt: pt, indeterminate: rest };
}

/** Whether the patch changes anything at all. */
export function patchIsEmpty(p: StylePatch): boolean {
  return Object.entries(p).every(([k, v]) => k === "borders" ? v === null || v === undefined : v === null || v === undefined);
}

/** Problems with the entered values. */
export function formatCellsError(st: FormatCellsState): string | null {
  if (!st.indeterminate.fontSizePt && (!Number.isFinite(st.fontSizePt) || st.fontSizePt < 1 || st.fontSizePt > 409)) return "Font size must be between 1 and 409 pt.";
  if (!Number.isFinite(st.rotationDeg) || st.rotationDeg < -90 || st.rotationDeg > 90) return "Rotation must be between −90° and 90°.";
  if (!Number.isFinite(st.indent) || st.indent < 0) return "Indent must be 0 or more.";
  return null;
}
