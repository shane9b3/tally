/**
 * Format Cells (⌘1, §4 item 29): the dialog's state (one object per tab)
 * seeded from the active cell's resolved `StyleDto`, and the `StylePatch`
 * that writes back only what changed. Borders include the diagonals; the
 * Protection tab becomes `Op::SetCellProtection`.
 */
import type { BorderEdgePatch, BorderLineDto, BorderStyleDto, BordersPatch, HAlign, StyleDto, StylePatch, UnderlineStyle, VAlign } from "@tally/bridge";
import { EMPTY_PATCH } from "../formats";

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

export const FONT_FAMILIES: readonly string[] = ["", "Inter", "Arial", "Helvetica", "Times New Roman", "Courier New", "Georgia", "Verdana", "Liberation Sans", "Liberation Serif", "DejaVu Sans"];

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
}

export const DEFAULT_BORDER_COLOR = 0x1f2937ff;

/** The dialog state for the active cell's style. */
export function stateFromStyle(s: StyleDto | undefined): FormatCellsState {
  return {
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
    locked: true,
    hiddenFormula: false,
  };
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

/** The `StylePatch` of the fields that differ from `base` (every field when `base` is undefined). */
export function patchFromState(st: FormatCellsState, base: StyleDto | undefined): StylePatch {
  const b = stateFromStyle(base);
  const diff = <T>(cur: T, was: T): T | null => (base === undefined || JSON.stringify(cur) !== JSON.stringify(was) ? cur : null);
  const patch: StylePatch = {
    ...EMPTY_PATCH,
    bold: diff(st.bold, b.bold),
    italic: diff(st.italic, b.italic),
    underline: diff(st.underline, b.underline),
    strike: diff(st.strike, b.strike),
    fontFamily: st.fontFamily === "" ? (b.fontFamily === "" ? null : "") : diff(st.fontFamily, b.fontFamily),
    fontSizePt: diff(st.fontSizePt, b.fontSizePt),
    color: diff(st.color, b.color),
    fill: diff(st.fill, b.fill),
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

/** Whether the patch changes anything at all. */
export function patchIsEmpty(p: StylePatch): boolean {
  return Object.entries(p).every(([k, v]) => k === "borders" ? v === null || v === undefined : v === null || v === undefined);
}

/** Problems with the entered values. */
export function formatCellsError(st: FormatCellsState): string | null {
  if (!Number.isFinite(st.fontSizePt) || st.fontSizePt < 1 || st.fontSizePt > 409) return "Font size must be between 1 and 409 pt.";
  if (!Number.isFinite(st.rotationDeg) || st.rotationDeg < -90 || st.rotationDeg > 90) return "Rotation must be between −90° and 90°.";
  if (!Number.isFinite(st.indent) || st.indent < 0) return "Indent must be 0 or more.";
  return null;
}
