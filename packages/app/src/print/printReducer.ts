/**
 * Pure helpers behind the Print Preview (W5-5): page stepping, zoom steps,
 * fit-to-window zoom, the raster dpi for a zoom level, and the print range
 * (sheet scope and the "1-3,5" page list) turned into engine options.
 */
import type { PrintOptionsDto, RangeDto } from "@tally/bridge";

/** Zoom steps of the preview (1 = 96 dpi, the page at its paper size). */
export const PREVIEW_ZOOMS = [0.25, 0.33, 0.5, 0.67, 0.75, 1, 1.25, 1.5, 2, 3] as const;
export const MIN_PREVIEW_ZOOM = 0.25;
export const MAX_PREVIEW_ZOOM = 3;
/** Raster dpi of a thumbnail. */
export const THUMB_DPI = 24;
/** The engine's dpi limit (`limits::MAX_PRINT_DPI`). */
export const MAX_DPI = 600;
/** Thumbnails rendered around the current page. */
export const THUMB_WINDOW = 12;

export function clampPage(page: number, count: number): number {
  if (count <= 0) return 0;
  return Math.min(count - 1, Math.max(0, Math.trunc(page)));
}

export function stepPage(page: number, count: number, delta: number): number {
  return clampPage(page + delta, count);
}

export function clampPreviewZoom(z: number): number {
  if (!Number.isFinite(z)) return 1;
  return Math.min(MAX_PREVIEW_ZOOM, Math.max(MIN_PREVIEW_ZOOM, Math.round(z * 100) / 100));
}

export function stepPreviewZoom(z: number, dir: 1 | -1): number {
  if (dir > 0) return PREVIEW_ZOOMS.find((s) => s > z + 1e-9) ?? MAX_PREVIEW_ZOOM;
  return [...PREVIEW_ZOOMS].reverse().find((s) => s < z - 1e-9) ?? MIN_PREVIEW_ZOOM;
}

/** CSS px of a paper size at zoom 1 (96 dpi). */
export function paperPx(paperMm: readonly [number, number], zoom = 1): { width: number; height: number } {
  const k = (96 / 25.4) * zoom;
  return { width: Math.round(paperMm[0] * k), height: Math.round(paperMm[1] * k) };
}

/** The zoom that fits a page in a viewport (with `pad` px around it), "whole page". */
export function fitZoom(paperMm: readonly [number, number], viewport: { width: number; height: number }, pad = 24): number {
  const { width, height } = paperPx(paperMm);
  const z = Math.min((viewport.width - 2 * pad) / width, (viewport.height - 2 * pad) / height);
  return clampPreviewZoom(z);
}

/** The zoom that fits a page's width in a viewport. */
export function fitWidthZoom(paperMm: readonly [number, number], viewport: { width: number }, pad = 24): number {
  const { width } = paperPx(paperMm);
  return clampPreviewZoom((viewport.width - 2 * pad) / width);
}

/** The raster dpi for a zoom level on a display (capped at the engine's limit). */
export function dpiFor(zoom: number, dpr = 1): number {
  return Math.max(1, Math.min(MAX_DPI, Math.round(96 * zoom * Math.max(1, dpr))));
}

/** Page indices whose thumbnails to render: a window around `page`. */
export function thumbWindow(page: number, count: number, size = THUMB_WINDOW): number[] {
  const half = Math.floor(size / 2);
  const start = Math.max(0, Math.min(page - half, count - size));
  const out: number[] = [];
  for (let i = start; i < Math.min(count, start + size); i++) out.push(i);
  return out;
}

/** "Page 3 of 12", or "No pages". */
export function pageLabel(page: number, count: number): string {
  return count === 0 ? "No pages" : `Page ${page + 1} of ${count}`;
}

// -- print range ----------------------------------------------------------------

/**
 * What a print, the preview and Export PDF cover (AOO's File ▸ Print ▸
 * Range): every sheet, the selected sheets, or the selected cells. Tally
 * has no multi-sheet selection yet, so "selected" is the active sheet.
 */
export type PrintScope = "all" | "selected" | "selection";

export const PRINT_SCOPES: readonly PrintScope[] = ["all", "selected", "selection"];

/** The scope a fresh document starts with. */
export const DEFAULT_PRINT_SCOPE: PrintScope = "selected";

/** Highest page number a print may address (`limits::MAX_PRINT_PAGES`). */
export const MAX_PRINT_PAGE = 10_000;
/** Most ranges in a page list (`limits::MAX_PRINT_PAGE_RANGES`). */
export const MAX_PAGE_RANGES = 256;

export type PageRanges = [number, number][];

export type PageRangeResult =
  | { ok: true; ranges: PageRanges | null }
  | { ok: false; error: "syntax" | "zero" | "reversed" | "tooMany" | "tooLarge" | "outOfRange"; part: string };

/**
 * Parses a page list like AOO's "Pages" field: numbers and `a-b` ranges
 * separated by commas or semicolons ("1-3, 5"), an open end ("4-") running
 * to the last page; empty items are skipped. Blank means every page
 * (`ranges: null`). With `count`
 * (the job's pages), a range wholly past the end is an error and an open or
 * overlong end is clipped to it.
 */
export function parsePageRanges(spec: string, count?: number): PageRangeResult {
  const text = spec.trim();
  if (text === "") return { ok: true, ranges: null };
  // Empty items ("1, ", "1,,2") are skipped, as in AOO.
  const parts = text
    .split(/[,;]/)
    .map((p) => p.trim())
    .filter((p) => p !== "");
  if (parts.length === 0) return { ok: true, ranges: null };
  if (parts.length > MAX_PAGE_RANGES) return { ok: false, error: "tooMany", part: text };
  const ranges: PageRanges = [];
  for (const part of parts) {
    const m = /^(\d+)(?:\s*-\s*(\d*))?$/.exec(part);
    if (!m) return { ok: false, error: "syntax", part };
    const a = Number(m[1]);
    const open = m[2] === "";
    const b = m[2] === undefined ? a : open ? (count ?? MAX_PRINT_PAGE) : Number(m[2]);
    if (a === 0 || b === 0) return { ok: false, error: "zero", part };
    if (a > MAX_PRINT_PAGE || b > MAX_PRINT_PAGE) return { ok: false, error: "tooLarge", part };
    if (b < a && !open) return { ok: false, error: "reversed", part };
    if (count !== undefined && a > count) return { ok: false, error: "outOfRange", part };
    ranges.push([a, count !== undefined ? Math.min(Math.max(a, b), count) : Math.max(a, b)]);
  }
  return { ok: true, ranges };
}

/**
 * The engine options for a scope on the user's sheet and selection (every
 * range of a multi-selection, each printed as its own print range, in order).
 */
export function printOptionsFor(scope: PrintScope, ranges: PageRanges | null, sheet: number, selection: readonly RangeDto[] | null): PrintOptionsDto {
  return {
    sheets: { type: scope === "all" ? "all" : "active" },
    selection: scope === "selection" && selection ? selection.map((r) => ({ ...r })) : null,
    suppressEmptyPages: true,
    pageFrom: null,
    pageTo: null,
    activeSheet: sheet,
    pageRanges: ranges ? ranges.map(([a, b]): [number, number] => [a, b]) : null,
  };
}
