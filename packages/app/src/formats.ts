import { t } from "./i18n";
import type { StylePatch } from "@tally/bridge";

/** Number format codes used by the toolbar (OOo format-code syntax). */
export const CURRENCY_FORMAT = "[$$-409]#,##0.00;[RED]-[$$-409]#,##0.00";
export const PERCENT_FORMAT = "0.00%";

/** The number-format dropdown (OOo codes). */
export interface NumberFormatPreset {
  id: string;
  label: string;
  code: string;
  /** Sample shown in the menu. */
  sample: string;
}

export const NUMBER_FORMATS: readonly NumberFormatPreset[] = [
  { id: "general", get label() { return t("fmt.general", "General"); }, code: "General", sample: "1234.5" },
  { id: "number", get label() { return t("fmt.number", "Number"); }, code: "0.00", sample: "1234.50" },
  { id: "thousands", get label() { return t("lbl.fmt.thousands", "Number with separators"); }, code: "#,##0.00", sample: "1,234.50" },
  { id: "percent", get label() { return t("fmt.percent", "Percent"); }, code: PERCENT_FORMAT, sample: "12.50%" },
  { id: "currency", get label() { return t("fmt.currency", "Currency"); }, code: CURRENCY_FORMAT, sample: "$1,234.50" },
  { id: "date", get label() { return t("fmt.date", "Date"); }, code: "MM/DD/YYYY", sample: "10/19/2026" },
  { id: "time", get label() { return t("fmt.time", "Time"); }, code: "HH:MM:SS", sample: "14:30:00" },
  { id: "scientific", get label() { return t("fmt.scientific", "Scientific"); }, code: "0.00E+00", sample: "1.23E+03" },
  { id: "text", get label() { return t("fmt.text", "Text"); }, code: "@", sample: "1234.5" },
];

/** The preset whose code matches `code` (case-insensitive), if any. */
export function presetFor(code: string | undefined): NumberFormatPreset | undefined {
  // A language tag (`[$-407]General`, `[$-407]0.00`) does not change the preset.
  const c = (code ?? "General").replace(/\[\$-[0-9A-Fa-f]+\]/g, "").trim().toLowerCase() || "general";
  return NUMBER_FORMATS.find((p) => p.code.toLowerCase() === c);
}

/** Zoom presets of the status bar menu. */
export const ZOOM_PRESETS = [0.5, 0.75, 1, 1.25, 1.5, 2] as const;
/** Steps for zoom in/out. */
export const ZOOM_STEPS = [0.2, 0.25, 0.33, 0.5, 0.67, 0.75, 0.9, 1, 1.1, 1.25, 1.5, 1.75, 2, 2.5, 3, 4] as const;
/** AOO's MINZOOM / MAXZOOM (sc/inc/global.hxx): 20–400 %. */
export const MIN_ZOOM = 0.2;
export const MAX_ZOOM = 4;

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
