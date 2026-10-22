/**
 * Font pickers (toolbar font box and size box, Format Cells ▸ Font): the
 * pure parts. The list sections (recently used, fonts in this document, all
 * installed), type-ahead filtering, the typefaces a family offers mapped to
 * the style model's bold/italic, the selection's font (blank when mixed),
 * font-size parsing and the `StylePatch` a pick produces.
 *
 * The style model stores bold and italic only, so the typefaces offered
 * are Regular, Italic, Bold and Bold Italic; other weights or widths of a
 * family (Light, Medium, Condensed…) are reported so the dialog can say so.
 */
import type { FontFamilyInfo, RangeDto, StyleDto, StylePatch } from "@tally/bridge";
import { EMPTY_PATCH } from "../formats";

/** The application font: cells without a family (and no document font) use it. */
export const APP_FONT = "Inter";

/** Longest family name sent (the engine cuts at 128 too). */
export const MAX_FONT_NAME = 128;

/** Font sizes offered (AOO's list); any value 1–409 in half points can be typed. */
export const FONT_SIZE_LIST: readonly number[] = [6, 7, 8, 9, 10, 10.5, 11, 12, 13, 14, 15, 16, 18, 20, 22, 24, 26, 28, 32, 36, 40, 44, 48, 54, 60, 66, 72, 80, 88, 96];

export const MIN_FONT_SIZE = 1;
export const MAX_FONT_SIZE = 409;

/** Most recently used fonts kept. */
export const MAX_RECENT_FONTS = 8;

export type FontSection = "recent" | "document" | "all";

/** A row of the font list: a section header or a family. */
export type FontRow = { kind: "header"; section: FontSection } | { kind: "font"; family: string; section: FontSection };

/** A typed family name as stored: trimmed, control characters removed, ≤ 128 characters. */
export function cleanFontName(name: string): string {
  // eslint-disable-next-line no-control-regex
  return name.replace(/[\u0000-\u001f\u007f]/g, "").trim().slice(0, MAX_FONT_NAME).trim();
}

/** Family names compare ignoring case (as the engine and the PDF writer match them). */
export const sameFont = (a: string, b: string): boolean => a.toLowerCase() === b.toLowerCase();
const same = sameFont;

/** Whether `name` is an installed family (ignoring case). */
export function isInstalled(name: string, families: readonly FontFamilyInfo[]): boolean {
  return families.some((f) => same(f.family, name));
}

/** The installed family named `name` (ignoring case). */
export function findFamily(name: string, families: readonly FontFamilyInfo[]): FontFamilyInfo | undefined {
  return families.find((f) => same(f.family, name));
}

function uniq(names: readonly string[]): string[] {
  const out: string[] = [];
  for (const n of names) if (n && !out.some((o) => same(o, n))) out.push(n);
  return out;
}

/**
 * The rows of the font list. Without a query: recently used fonts, the
 * fonts used in this document, then every installed family, each under a
 * header (a family can appear in several sections). With a query: one flat
 * list of the distinct names containing it (ignoring case), names starting
 * with it first.
 */
export function fontRows(query: string, families: readonly FontFamilyInfo[], recent: readonly string[], documentFonts: readonly string[]): FontRow[] {
  const q = query.trim().toLowerCase();
  if (q === "") {
    const rows: FontRow[] = [];
    const section = (s: FontSection, names: readonly string[]) => {
      if (names.length === 0) return;
      rows.push({ kind: "header", section: s });
      for (const family of names) rows.push({ kind: "font", family, section: s });
    };
    section("recent", uniq(recent));
    section("document", uniq(documentFonts));
    section(
      "all",
      families.map((f) => f.family),
    );
    return rows;
  }
  const all = uniq([...recent, ...documentFonts, ...families.map((f) => f.family)]);
  const starts = all.filter((n) => n.toLowerCase().startsWith(q));
  const contains = all.filter((n) => !n.toLowerCase().startsWith(q) && n.toLowerCase().includes(q));
  return [...starts, ...contains].map((family) => ({ kind: "font", family, section: "all" }));
}

/** The index of the row to highlight while typing `query`: the first family starting with it, else -1. */
export function typeAheadIndex(rows: readonly FontRow[], query: string): number {
  const q = query.trim().toLowerCase();
  if (q === "") return -1;
  return rows.findIndex((r) => r.kind === "font" && r.family.toLowerCase().startsWith(q));
}

/** The next selectable row from `from` in direction `dir` (headers are skipped); stays put at the ends. */
export function stepRow(rows: readonly FontRow[], from: number, dir: 1 | -1): number {
  for (let i = from + dir; i >= 0 && i < rows.length; i += dir) if (rows[i]?.kind === "font") return i;
  return from;
}

/** Families used in the document: the document font and the families of the given (loaded) styles. */
export function documentFonts(documentFont: string, styles: readonly (StyleDto | undefined)[]): string[] {
  const names = [documentFont, ...styles.map((s) => s?.fontFamily ?? "")].map(cleanFontName).filter((n) => n !== "");
  return uniq(names).sort((a, b) => a.localeCompare(b, undefined, { sensitivity: "base" }));
}

/** Adds `name` at the front of the recent list (moved if present), at most {@link MAX_RECENT_FONTS}. */
export function withRecent(recent: readonly string[], name: string): string[] {
  const n = cleanFontName(name);
  if (n === "") return [...recent];
  return [n, ...recent.filter((r) => !same(r, n))].slice(0, MAX_RECENT_FONTS);
}

// ---------------------------------------------------------------------------
// Typefaces
// ---------------------------------------------------------------------------

export type Typeface = "regular" | "italic" | "bold" | "boldItalic";

export const TYPEFACES: readonly Typeface[] = ["regular", "italic", "bold", "boldItalic"];

export const TYPEFACE_LABELS: Readonly<Record<Typeface, string>> = { regular: "Regular", italic: "Italic", bold: "Bold", boldItalic: "Bold Italic" };

export function typefaceOf(bold: boolean, italic: boolean): Typeface {
  return bold ? (italic ? "boldItalic" : "bold") : italic ? "italic" : "regular";
}

export function typefaceFields(t: Typeface): { bold: boolean; italic: boolean } {
  return { bold: t === "bold" || t === "boldItalic", italic: t === "italic" || t === "boldItalic" };
}

/** A face is drawn bold from semibold (600) up, as the PDF writer and CSS pick faces. */
const isBoldWeight = (w: number) => w >= 600;

export interface FamilyTypefaces {
  /** The four typefaces, each with whether the family has a face for it (else it is synthesised). */
  faces: { typeface: Typeface; real: boolean }[];
  /** The family has weights or widths the style model cannot store (Light, Medium, Condensed…). */
  extra: boolean;
}

/** The typefaces of `info` mapped to bold/italic; without info (not installed) all four count as real. */
export function familyTypefaces(info: FontFamilyInfo | undefined): FamilyTypefaces {
  if (!info || info.faces.length === 0) return { faces: TYPEFACES.map((typeface) => ({ typeface, real: true })), extra: false };
  const have = new Set(info.faces.filter((f) => f.stretch === 5 || info.faces.every((g) => g.stretch !== 5)).map((f) => typefaceOf(isBoldWeight(f.weight), f.italic)));
  const extra = info.faces.some((f) => (f.weight !== 400 && f.weight !== 700) || f.stretch !== 5);
  return { faces: TYPEFACES.map((typeface) => ({ typeface, real: have.has(typeface) })), extra };
}

// ---------------------------------------------------------------------------
// Sizes
// ---------------------------------------------------------------------------

/** A typed size ("10.5", "10,5", "12 pt") in points, rounded to half points; `null` when not 1–409. */
export function parseFontSize(text: string): number | null {
  const s = text.trim().replace(/\s*pt$/i, "").replace(",", ".");
  if (!/^\d+(\.\d*)?$|^\.\d+$/.test(s)) return null;
  const v = Math.round(Number(s) * 2) / 2;
  return Number.isFinite(v) && v >= MIN_FONT_SIZE && v <= MAX_FONT_SIZE ? v : null;
}

/** A size as shown ("10", "10.5"). */
export function formatFontSize(pt: number): string {
  return String(Math.round(pt * 10) / 10);
}

// ---------------------------------------------------------------------------
// The selection's font and the patches
// ---------------------------------------------------------------------------

/** The font of a selection; `null` fields are mixed. */
export interface SelectionFont {
  family: string | null;
  sizePt: number | null;
  bold: boolean | null;
  italic: boolean | null;
}

interface CellSource {
  getCell(r: number, c: number): { style: number } | undefined;
  getStyle(id: number): StyleDto | undefined;
}

/** Most cells looked at for the selection's font (the rest are assumed alike). */
export const MAX_SCANNED_CELLS = 10_000;

/**
 * The selection's font from the loaded cells: the active cell first, then
 * every selected cell inside the used range (cells outside it are empty, so
 * one default cell stands for them), at most {@link MAX_SCANNED_CELLS}.
 * A style not loaded yet is skipped. `undefined` when nothing is known.
 */
export function selectionFont(source: CellSource, ranges: readonly RangeDto[], active: { r: number; c: number }, used: RangeDto | null, appFont: string = APP_FONT, limit = MAX_SCANNED_CELLS): SelectionFont | undefined {
  let out: SelectionFont | undefined;
  const add = (styleId: number) => {
    const s = source.getStyle(styleId);
    if (!s) return;
    const family = s.fontFamily && cleanFontName(s.fontFamily) !== "" ? cleanFontName(s.fontFamily) : appFont;
    if (!out) {
      out = { family, sizePt: s.fontSizePt, bold: s.bold, italic: s.italic };
      return;
    }
    if (out.family !== null && !same(out.family, family)) out.family = null;
    if (out.sizePt !== null && out.sizePt !== s.fontSizePt) out.sizePt = null;
    if (out.bold !== null && out.bold !== s.bold) out.bold = null;
    if (out.italic !== null && out.italic !== s.italic) out.italic = null;
  };
  add(source.getCell(active.r, active.c)?.style ?? 0);
  let budget = limit;
  let outside = false;
  for (const rg of ranges) {
    const r0 = used ? Math.max(rg.r0, used.r0) : 0;
    const r1 = used ? Math.min(rg.r1, used.r1) : -1;
    const c0 = used ? Math.max(rg.c0, used.c0) : 0;
    const c1 = used ? Math.min(rg.c1, used.c1) : -1;
    if (!used || r0 > rg.r0 || r1 < rg.r1 || c0 > rg.c0 || c1 < rg.c1) outside = true;
    for (let r = r0; r <= r1 && budget > 0; r++) {
      for (let c = c0; c <= c1 && budget > 0; c++, budget--) add(source.getCell(r, c)?.style ?? 0);
    }
  }
  if (outside) add(0);
  return out;
}

/**
 * The patch for picking `name` in a font box: the family, or "" (no family
 * of its own: the document font) when `name` is the document font.
 */
export function fontFamilyPatch(name: string, documentFont: string): StylePatch {
  const n = cleanFontName(name);
  return { ...EMPTY_PATCH, fontFamily: n === "" || same(n, documentFont) ? "" : n };
}

/** The patch for a font size. */
export function fontSizePatch(pt: number): StylePatch {
  return { ...EMPTY_PATCH, fontSizePt: pt };
}
