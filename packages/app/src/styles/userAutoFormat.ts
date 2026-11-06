/**
 * User-defined AutoFormats (AOO's `autotbl.fmt`, Add / Rename / Delete in the
 * AutoFormat dialog).
 *
 * AOO's `ScAutoFormatData` holds 16 cell formats, a 4 × 4 grid: the first row,
 * the "odd" and "even" inner rows and the last row, crossed with the same four
 * column roles. `ScDocument::GetAutoFormatData` fills it by sampling the
 * selection (rows 0, 1, 2 and last; columns likewise), and applying maps every
 * target cell to a slot with `GetIdx`: row 0 / last row, the inner rows
 * alternating odd / even. A format is stored here the same way, with the font,
 * colours, borders, alignment and number format of every slot.
 *
 * The built-in presets stay coarse (header / body / total) and are applied by
 * the engine (`Op::ApplyAutoFormat`); user formats are applied as ordinary
 * `applyStyle` ops built by [`userAutoFormatOps`].
 */
import type { AutoFormatOptions, BorderLineDto, HAlign, Op, RangeDto, StyleDto, StylePatch, VAlign } from "@tally/bridge";
import { t } from "../i18n";
import { AUTOFORMAT_PRESETS, presetLabel } from "./autoformat";

/** How many user formats are kept (a sane cap on untrusted prefs). */
export const MAX_USER_AUTOFORMATS = 100;
/** Longest format name (characters). */
export const MAX_AUTOFORMAT_NAME = 64;
/** AutoFormat needs at least 3 × 3 cells (AOO: `cellsh3.cxx`). */
export const MIN_AUTOFORMAT_SIDE = 3;

export interface AutoFormatBorders {
  top: BorderLineDto | null;
  bottom: BorderLineDto | null;
  left: BorderLineDto | null;
  right: BorderLineDto | null;
}

/** One of the 16 cell formats. */
export interface AutoFormatCell {
  bold: boolean;
  italic: boolean;
  underline: boolean;
  strike: boolean;
  fontFamily: string | null;
  fontSizePt: number | null;
  /** Text colour (RGBA), or `null` for the automatic colour. */
  color: number | null;
  /** Fill (RGBA), or `null` for none. */
  fill: number | null;
  hAlign: HAlign;
  vAlign: VAlign;
  numberFormat: string;
  borders: AutoFormatBorders;
}

export interface UserAutoFormat {
  name: string;
  /** 16 cells, row-major: `cells[rowRole * 4 + colRole]`. */
  cells: AutoFormatCell[];
}

const NO_BORDERS: AutoFormatBorders = { top: null, bottom: null, left: null, right: null };

export function emptyAutoFormatCell(): AutoFormatCell {
  return { bold: false, italic: false, underline: false, strike: false, fontFamily: null, fontSizePt: null, color: null, fill: null, hAlign: "general", vAlign: "bottom", numberFormat: "General", borders: { ...NO_BORDERS } };
}

/** The slot role (0 first, 1 odd inner, 2 even inner, 3 last) of index `i` among `n` (AOO's `GetIdx`). */
export function roleOf(i: number, n: number): 0 | 1 | 2 | 3 {
  if (i === 0) return 0;
  if (i === n - 1) return 3;
  return i % 2 === 1 ? 1 : 2;
}

/** The offsets sampled for roles 0..3 of an `n`-long side (AOO samples 0, 1, 2 and last). */
export function sampleOffsets(n: number): [number, number, number, number] {
  return [0, 1, Math.min(2, Math.max(1, n - 2)), n - 1];
}

const edge = (l: BorderLineDto | null | undefined): BorderLineDto | null => (l ? { style: l.style, widthPx: l.widthPx, color: l.color } : null);

function cellFromStyle(s: StyleDto): AutoFormatCell {
  return {
    bold: s.bold,
    italic: s.italic,
    underline: s.underline,
    strike: s.strike,
    fontFamily: s.fontFamily,
    fontSizePt: s.fontSizePt,
    color: s.color,
    fill: s.fill,
    hAlign: s.hAlign,
    vAlign: s.vAlign,
    numberFormat: s.numberFormat || "General",
    borders: { top: edge(s.borders.top), bottom: edge(s.borders.bottom), left: edge(s.borders.left), right: edge(s.borders.right) },
  };
}

/** Why `rows × cols` cells cannot become an AutoFormat, or `null`. */
export function sampleError(rows: number, cols: number): string | null {
  return rows < MIN_AUTOFORMAT_SIDE || cols < MIN_AUTOFORMAT_SIDE ? t("stylesUi.afErr.small", "Select at least 3 rows and 3 columns for an AutoFormat.") : null;
}

/**
 * Builds a format from a selection of `rows × cols` cells; `styleAt(dr, dc)` is
 * the resolved style of the cell at that offset (only the 16 sampled are read).
 */
export function buildAutoFormat(name: string, rows: number, cols: number, styleAt: (dr: number, dc: number) => StyleDto): UserAutoFormat {
  if (sampleError(rows, cols)) throw new Error(sampleError(rows, cols) ?? "");
  const ro = sampleOffsets(rows);
  const co = sampleOffsets(cols);
  const cells: AutoFormatCell[] = [];
  for (const dr of ro) for (const dc of co) cells.push(cellFromStyle(styleAt(dr, dc)));
  return { name: name.trim(), cells };
}

// ---------------------------------------------------------------------------
// Names and the list
// ---------------------------------------------------------------------------

/** The names of the built-in presets (cannot be reused, renamed or deleted). */
export const isBuiltinAutoFormat = (name: string): boolean => {
  const n = name.trim().toLowerCase();
  return AUTOFORMAT_PRESETS.some((p) => p.name.toLowerCase() === n || presetLabel(p.name).toLowerCase() === n);
};

/** Why `name` is not usable for a format (`ignore`: the format being renamed), or `null`. */
export function autoFormatNameError(name: string, existing: readonly UserAutoFormat[], ignore?: string): string | null {
  const n = name.trim();
  if (n === "") return t("stylesUi.afErr.empty", "Enter a name.");
  if ([...n].length > MAX_AUTOFORMAT_NAME) return t("stylesUi.afErr.long", "The name is limited to {max} characters.", { max: MAX_AUTOFORMAT_NAME });
  // eslint-disable-next-line no-control-regex
  if (/[\u0000-\u001f\u007f]/.test(n)) return t("stylesUi.afErr.chars", "The name has invalid characters.");
  if (isBuiltinAutoFormat(n)) return t("stylesUi.afErr.used", "This name is already in use.");
  const lower = n.toLowerCase();
  if (existing.some((f) => f.name.toLowerCase() === lower && f.name !== ignore)) return t("stylesUi.afErr.used", "This name is already in use.");
  return null;
}

/** Adds a format; throws when the name is bad or the cap is reached. */
export function addAutoFormat(list: readonly UserAutoFormat[], f: UserAutoFormat): UserAutoFormat[] {
  const err = autoFormatNameError(f.name, list);
  if (err) throw new Error(err);
  if (list.length >= MAX_USER_AUTOFORMATS) throw new Error(t("stylesUi.afErr.limit", "At most {max} AutoFormats can be saved.", { max: MAX_USER_AUTOFORMATS }));
  return [...list, { ...f, name: f.name.trim() }];
}

/** Renames a user format (built-ins cannot be renamed). */
export function renameAutoFormat(list: readonly UserAutoFormat[], from: string, to: string): UserAutoFormat[] {
  if (!list.some((f) => f.name === from)) throw new Error(t("stylesUi.afErr.own", "Only your own AutoFormats can be renamed or deleted."));
  const err = autoFormatNameError(to, list, from);
  if (err) throw new Error(err);
  return list.map((f) => (f.name === from ? { ...f, name: to.trim() } : f));
}

/** Deletes a user format (built-ins, including "Default", cannot be deleted). */
export function deleteAutoFormat(list: readonly UserAutoFormat[], name: string): UserAutoFormat[] {
  if (!list.some((f) => f.name === name)) throw new Error(t("stylesUi.afErr.own", "Only your own AutoFormats can be renamed or deleted."));
  return list.filter((f) => f.name !== name);
}

// ---------------------------------------------------------------------------
// Applying
// ---------------------------------------------------------------------------

function patchOf(c: AutoFormatCell, o: AutoFormatOptions): StylePatch {
  const p: StylePatch = { bold: null, italic: null, underline: null, strike: null, fontFamily: null, fontSizePt: null, color: null, fill: null, hAlign: null, vAlign: null, wrap: null, numberFormat: null };
  if (o.font) {
    p.bold = c.bold;
    p.italic = c.italic;
    p.underline = c.underline;
    p.strike = c.strike;
    p.fontFamily = c.fontFamily;
    p.fontSizePt = c.fontSizePt;
    if (c.color === null) p.clearColor = true;
    else p.color = c.color;
  }
  if (o.pattern) {
    if (c.fill === null) p.clearFill = true;
    else p.fill = c.fill;
  }
  if (o.alignment) {
    p.hAlign = c.hAlign;
    p.vAlign = c.vAlign;
  }
  if (o.numberFormat) p.numberFormat = c.numberFormat;
  if (o.borders) {
    p.borders = { top: { line: c.borders.top }, bottom: { line: c.borders.bottom }, left: { line: c.borders.left }, right: { line: c.borders.right }, innerH: null, innerV: null };
  }
  return p;
}

/**
 * The op applying `f` to `range`, honouring the options: one `applyAutoFormatCustom`
 * carrying the 16 slot patches (the engine lays them over the range and, with the
 * autofit option, fits the column widths and row heights in the same undo step).
 */
export function userAutoFormatOps(sheet: number, range: RangeDto, f: UserAutoFormat, o: AutoFormatOptions): Op[] {
  const err = sampleError(range.r1 - range.r0 + 1, range.c1 - range.c0 + 1);
  if (err) throw new Error(err);
  if (f.cells.length !== 16) throw new Error(t("stylesUi.afErr.damaged", "The AutoFormat is damaged."));
  return [{ type: "applyAutoFormatCustom", sheet, range: { ...range }, cells: f.cells.map((c) => patchOf(c, o)), autofit: o.autofit }];
}

// ---------------------------------------------------------------------------
// Preview
// ---------------------------------------------------------------------------

const hex = (rgba: number): string => `#${((rgba >>> 8) & 0xffffff).toString(16).padStart(6, "0")}`;

/** The look of cell (r, c) of a `rows × cols` preview grid for a user format. */
export function userPreviewCell(f: UserAutoFormat, r: number, c: number, rows: number, cols: number, o: AutoFormatOptions): { bg: string; fg: string; bold: boolean; italic: boolean; border: string; align: "general" | "left" | "right" | "center" } {
  const cell = f.cells[roleOf(r, rows) * 4 + roleOf(c, cols)] ?? emptyAutoFormatCell();
  const line = cell.borders.top ?? cell.borders.bottom ?? cell.borders.left ?? cell.borders.right;
  return {
    bg: o.pattern && cell.fill !== null ? hex(cell.fill) : "transparent",
    fg: o.font && cell.color !== null ? hex(cell.color) : "inherit",
    bold: o.font && cell.bold,
    italic: o.font && cell.italic,
    border: o.borders && line ? hex(line.color) : "transparent",
    align: o.alignment ? (cell.hAlign === "right" ? "right" : cell.hAlign === "center" ? "center" : cell.hAlign === "general" ? "general" : "left") : "general",
  };
}

/** Preview text of a body cell: numbers show decimals / money when the slot's number format has them. */
export function userPreviewText(f: UserAutoFormat, raw: string, r: number, c: number, rows: number, cols: number, o: AutoFormatOptions): string {
  const cell = f.cells[roleOf(r, rows) * 4 + roleOf(c, cols)];
  if (!cell || !o.numberFormat || r === 0 || c === 0 || raw === "") return raw;
  if (cell.numberFormat === "General") return raw;
  return /[$€£¥]/.test(cell.numberFormat) ? `$${raw}.00` : /0\.0/.test(cell.numberFormat) ? `${raw}.00` : raw;
}

// ---------------------------------------------------------------------------
// Prefs (untrusted JSON)
// ---------------------------------------------------------------------------

const H_VALUES: readonly string[] = ["general", "left", "center", "right", "justify"];
const V_VALUES: readonly string[] = ["bottom", "middle", "top"];
const LINE_STYLES: readonly string[] = ["solid", "dashed", "dotted", "double"];

const rec = (v: unknown): Record<string, unknown> | null => (typeof v === "object" && v !== null && !Array.isArray(v) ? (v as Record<string, unknown>) : null);
const rgba = (v: unknown): number | null => (typeof v === "number" && Number.isInteger(v) && v >= 0 && v <= 0xffffffff ? v : null);

function parseLine(v: unknown): BorderLineDto | null {
  const o = rec(v);
  if (!o || typeof o.style !== "string" || !LINE_STYLES.includes(o.style)) return null;
  const w = typeof o.widthPx === "number" && Number.isFinite(o.widthPx) ? Math.max(0, Math.min(8, o.widthPx)) : 1;
  return { style: o.style as BorderLineDto["style"], widthPx: w, color: rgba(o.color) ?? 0x000000ff };
}

function parseCell(v: unknown): AutoFormatCell {
  const o = rec(v) ?? {};
  const b = rec(o.borders) ?? {};
  const size = typeof o.fontSizePt === "number" && Number.isFinite(o.fontSizePt) && o.fontSizePt >= 1 && o.fontSizePt <= 409 ? o.fontSizePt : null;
  return {
    bold: o.bold === true,
    italic: o.italic === true,
    underline: o.underline === true,
    strike: o.strike === true,
    fontFamily: typeof o.fontFamily === "string" && o.fontFamily.length > 0 && o.fontFamily.length <= 100 ? o.fontFamily : null,
    fontSizePt: size,
    color: rgba(o.color),
    fill: rgba(o.fill),
    hAlign: typeof o.hAlign === "string" && H_VALUES.includes(o.hAlign) ? (o.hAlign as HAlign) : "general",
    vAlign: typeof o.vAlign === "string" && V_VALUES.includes(o.vAlign) ? (o.vAlign as VAlign) : "bottom",
    numberFormat: typeof o.numberFormat === "string" && o.numberFormat.length > 0 && o.numberFormat.length <= 255 ? o.numberFormat : "General",
    borders: { top: parseLine(b.top), bottom: parseLine(b.bottom), left: parseLine(b.left), right: parseLine(b.right) },
  };
}

/** Validates the stored user formats: bad entries are dropped, the list is capped, names are unique. */
export function parseUserAutoFormats(raw: unknown): UserAutoFormat[] {
  if (!Array.isArray(raw)) return [];
  const out: UserAutoFormat[] = [];
  for (const item of raw) {
    if (out.length >= MAX_USER_AUTOFORMATS) break;
    const o = rec(item);
    if (!o || typeof o.name !== "string" || !Array.isArray(o.cells) || o.cells.length !== 16) continue;
    if (autoFormatNameError(o.name, out) !== null) continue;
    out.push({ name: o.name.trim(), cells: o.cells.map(parseCell) });
  }
  return out;
}

/** The compact stored form: defaults are left out (the parser puts them back). */
export function serializeUserAutoFormats(list: readonly UserAutoFormat[]): unknown[] {
  const d = emptyAutoFormatCell();
  return list.slice(0, MAX_USER_AUTOFORMATS).map((f) => ({
    name: f.name,
    cells: f.cells.map((c) => {
      const out: Record<string, unknown> = {};
      for (const k of ["bold", "italic", "underline", "strike", "fontFamily", "fontSizePt", "color", "fill", "hAlign", "vAlign", "numberFormat"] as const) {
        if (c[k] !== d[k]) out[k] = c[k];
      }
      const borders: Record<string, BorderLineDto> = {};
      for (const k of ["top", "bottom", "left", "right"] as const) if (c.borders[k]) borders[k] = c.borders[k]!;
      if (Object.keys(borders).length > 0) out.borders = borders;
      return out;
    }),
  }));
}
