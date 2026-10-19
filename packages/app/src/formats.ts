import type { StylePatch } from "@tally/bridge";

/** Number format codes used by the toolbar (OOo format-code syntax). */
export const CURRENCY_FORMAT = "[$$-409]#,##0.00;[RED]-[$$-409]#,##0.00";
export const PERCENT_FORMAT = "0.00%";

/** Zoom presets of the status bar menu. */
export const ZOOM_PRESETS = [0.5, 0.75, 1, 1.25, 1.5, 2] as const;
/** Steps for zoom in/out. */
export const ZOOM_STEPS = [0.5, 0.67, 0.75, 0.9, 1, 1.1, 1.25, 1.5, 1.75, 2] as const;
export const MIN_ZOOM = 0.5;
export const MAX_ZOOM = 2;

export function clampZoom(z: number): number {
  return Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, Math.round(z * 100) / 100));
}

export function nextZoom(z: number, dir: 1 | -1): number {
  if (dir > 0) return ZOOM_STEPS.find((s) => s > z + 1e-9) ?? MAX_ZOOM;
  return [...ZOOM_STEPS].reverse().find((s) => s < z - 1e-9) ?? MIN_ZOOM;
}

/** A patch that changes nothing (spread and override one field). */
export const EMPTY_PATCH: StylePatch = {
  bold: null,
  italic: null,
  underline: null,
  strike: null,
  fontFamily: null,
  fontSizePt: null,
  color: null,
  fill: null,
  hAlign: null,
  vAlign: null,
  wrap: null,
  numberFormat: null,
};
