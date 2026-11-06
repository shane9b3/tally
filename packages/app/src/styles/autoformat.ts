/**
 * AutoFormat (§4 item 27): AOO's 17 presets as preview colours. The engine
 * applies the real formats (`Op::ApplyAutoFormat`); the dialog draws a 5×5
 * preview from these header / body / total colours and the options decide
 * which parts the engine applies.
 */
import type { AutoFormatOptions } from "@tally/bridge";
import { t } from "../i18n";

export interface AutoFormatPreset {
  name: string;
  /** Header row and first column: fill, text. */
  header: [string, string];
  /** Body cells (alternating rows for the 3D-ish ones). */
  body: [string, string];
  /** Total row and column. */
  total: [string, string];
  /** Border colour of the preview. */
  border: string;
  /** Number format sample of the body (currency presets). */
  currency?: boolean;
  bold?: boolean;
}

/** The 17 AOO presets, in AOO's list order. */
export const AUTOFORMAT_PRESETS: readonly AutoFormatPreset[] = [
  { name: "Default", header: ["#d1d5db", "#111827"], body: ["#ffffff", "#111827"], total: ["#e5e7eb", "#111827"], border: "#9ca3af", bold: true },
  { name: "3D", header: ["#1f2937", "#ffffff"], body: ["#f3f4f6", "#111827"], total: ["#9ca3af", "#111827"], border: "#4b5563", bold: true },
  { name: "Black 1", header: ["#000000", "#ffffff"], body: ["#ffffff", "#000000"], total: ["#000000", "#ffffff"], border: "#000000", bold: true },
  { name: "Black 2", header: ["#ffffff", "#000000"], body: ["#ffffff", "#000000"], total: ["#e5e7eb", "#000000"], border: "#000000", bold: true },
  { name: "Blue", header: ["#1d4ed8", "#ffffff"], body: ["#dbeafe", "#111827"], total: ["#93c5fd", "#111827"], border: "#1d4ed8", bold: true },
  { name: "Brown", header: ["#92400e", "#ffffff"], body: ["#fef3c7", "#111827"], total: ["#fcd34d", "#111827"], border: "#92400e", bold: true },
  { name: "Currency", header: ["#e5e7eb", "#111827"], body: ["#ffffff", "#111827"], total: ["#d1d5db", "#111827"], border: "#6b7280", currency: true, bold: true },
  { name: "Currency 3D", header: ["#374151", "#ffffff"], body: ["#f9fafb", "#111827"], total: ["#9ca3af", "#111827"], border: "#374151", currency: true, bold: true },
  { name: "Currency Gray", header: ["#9ca3af", "#111827"], body: ["#f3f4f6", "#111827"], total: ["#d1d5db", "#111827"], border: "#6b7280", currency: true, bold: true },
  { name: "Currency Lavender", header: ["#7c3aed", "#ffffff"], body: ["#ede9fe", "#111827"], total: ["#c4b5fd", "#111827"], border: "#7c3aed", currency: true, bold: true },
  { name: "Currency Turquoise", header: ["#0e7490", "#ffffff"], body: ["#cffafe", "#111827"], total: ["#67e8f9", "#111827"], border: "#0e7490", currency: true, bold: true },
  { name: "Gray", header: ["#6b7280", "#ffffff"], body: ["#f3f4f6", "#111827"], total: ["#d1d5db", "#111827"], border: "#6b7280", bold: true },
  { name: "Green", header: ["#15803d", "#ffffff"], body: ["#dcfce7", "#111827"], total: ["#86efac", "#111827"], border: "#15803d", bold: true },
  { name: "Lavender", header: ["#6d28d9", "#ffffff"], body: ["#ede9fe", "#111827"], total: ["#c4b5fd", "#111827"], border: "#6d28d9", bold: true },
  { name: "Red", header: ["#b91c1c", "#ffffff"], body: ["#fee2e2", "#111827"], total: ["#fca5a5", "#111827"], border: "#b91c1c", bold: true },
  { name: "Turquoise", header: ["#0f766e", "#ffffff"], body: ["#ccfbf1", "#111827"], total: ["#5eead4", "#111827"], border: "#0f766e", bold: true },
  { name: "Yellow", header: ["#a16207", "#ffffff"], body: ["#fef9c3", "#111827"], total: ["#fde047", "#111827"], border: "#a16207", bold: true },
];

export const DEFAULT_AUTOFORMAT_OPTIONS: AutoFormatOptions = { numberFormat: true, font: true, alignment: true, borders: true, pattern: true, autofit: true };

export const AUTOFORMAT_OPTION_LABELS: readonly { key: keyof AutoFormatOptions; label: string }[] = [
  { key: "numberFormat", label: "Number format" },
  { key: "font", label: "Font" },
  { key: "alignment", label: "Alignment" },
  { key: "borders", label: "Borders" },
  { key: "pattern", label: "Pattern" },
  { key: "autofit", label: "AutoFit width and height" },
];

/** The preview's sample grid (AOO's: Jan Feb Mar Total rows North Mid South Total). */
export const PREVIEW_ROWS: readonly (readonly string[])[] = [
  ["", "Jan", "Feb", "Mar", "Total"],
  ["North", "6", "7", "8", "21"],
  ["Mid", "11", "12", "13", "36"],
  ["South", "16", "17", "18", "51"],
  ["Total", "33", "36", "39", "108"],
];

/** The colours of a preview cell (header band, total band or body) honouring the pattern option. */
export function previewCell(preset: AutoFormatPreset, r: number, c: number, options: AutoFormatOptions): { bg: string; fg: string; bold: boolean } {
  const isHeader = r === 0 || c === 0;
  const isTotal = r === PREVIEW_ROWS.length - 1 || c === (PREVIEW_ROWS[0]?.length ?? 1) - 1;
  const band = isHeader ? preset.header : isTotal ? preset.total : preset.body;
  return { bg: options.pattern ? band[0] : "transparent", fg: options.pattern ? band[1] : "inherit", bold: options.font && (isHeader || isTotal) && (preset.bold ?? false) };
}

/** Preview text of a body cell (currency presets show money). */
export function previewText(preset: AutoFormatPreset, r: number, c: number, options: AutoFormatOptions): string {
  const raw = PREVIEW_ROWS[r]?.[c] ?? "";
  if (preset.currency && options.numberFormat && r > 0 && c > 0) return `$${raw}.00`;
  return raw;
}

const PRESET_KEYS: Readonly<Record<string, string>> = {"Default": "stylesUi.preset.default", "3D": "stylesUi.preset.threeD", "Black 1": "stylesUi.preset.black1", "Black 2": "stylesUi.preset.black2", "Blue": "stylesUi.preset.blue", "Brown": "stylesUi.preset.brown", "Currency": "stylesUi.preset.currency", "Currency 3D": "stylesUi.preset.currency3d", "Currency Gray": "stylesUi.preset.currencyGray", "Currency Lavender": "stylesUi.preset.currencyLavender", "Currency Turquoise": "stylesUi.preset.currencyTurquoise", "Gray": "stylesUi.preset.gray", "Green": "stylesUi.preset.green", "Lavender": "stylesUi.preset.lavender", "Red": "stylesUi.preset.red", "Turquoise": "stylesUi.preset.turquoise", "Yellow": "stylesUi.preset.yellow"};

/** Option and preset labels live in the model files in English; they are translated when drawn. */
export const presetLabel = (name: string): string => t(PRESET_KEYS[name] ?? "", name);
