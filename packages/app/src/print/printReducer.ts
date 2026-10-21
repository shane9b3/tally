/**
 * Pure helpers behind the Print Preview (W5-5): page stepping, zoom steps,
 * fit-to-window zoom and the raster dpi for a zoom level.
 */

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
