/**
 * v0.3 helpers without React or engine access: conditional-format rule
 * builders and summaries, "Highlight cells" presets, data validation
 * builders, link (URL) checks, page setup (paper sizes, automatic page
 * breaks, header/footer placeholders), protection defaults and the UI
 * locales. The controller (features.ts) and the dialogs use them; tests
 * cover them directly.
 */
import type {
  CellPos,
  CfColorStop,
  CfDatePeriod,
  CfRuleDto,
  CfRuleKind,
  CfStyleDto,
  CfTextOp,
  CfThreshold,
  CfValueKind,
  CmpOp,
  DimRunPx,
  IconSetKind,
  PageSetupDto,
  Paper,
  ProtectionOptions,
  RangeDto,
  ValidationDto,
  ValidationErrorStyle,
  ValidationKind,
} from "@tally/bridge";
import { parseA1Range } from "./a1";
import { a1 } from "./dataOps";

// ---------------------------------------------------------------------------
// Links
// ---------------------------------------------------------------------------

/** Longest URL `open_url` accepts (bytes; the Rust check is authoritative). */
export const MAX_URL = 2048;

/**
 * Whether `open_url` would open this URL: `http(s)://host…` or
 * `mailto:address`, no whitespace, control characters, quotes, `\`, `<`,
 * `>` or backticks, at most {@link MAX_URL} bytes. Mirrors
 * `apps/desktop/src-tauri/src/links.rs::check_url`.
 */
export function isOpenableUrl(raw: string): boolean {
  const url = raw.trim();
  if (url === "" || new TextEncoder().encode(url).length > MAX_URL) return false;
  // eslint-disable-next-line no-control-regex
  if (/[\s"\\<>`\u0000-\u001f\u007f-\u009f]/.test(url)) return false;
  const lower = url.toLowerCase();
  if (lower.startsWith("mailto:")) {
    const rest = url.slice(7);
    return rest !== "" && !rest.startsWith("/");
  }
  const m = /^https?:\/\/(.*)$/.exec(lower);
  if (!m) return false;
  const host = (m[1] ?? "").split(/[/?#]/)[0]?.split("@").pop() ?? "";
  return host !== "" && !host.startsWith(":");
}

/** A link inside the workbook (`#Sheet2.A1`, `#Name`). */
export function isInternalLink(url: string): boolean {
  return url.trim().startsWith("#");
}

/** Schemes `setLink` refuses (the engine refuses them too). */
export function isForbiddenLink(url: string): boolean {
  return /^\s*(javascript|data|vbscript):/i.test(url);
}

/**
 * What the Hyperlink dialog stores for the typed address: `#…` stays,
 * `name@host` becomes `mailto:`, `www.x` or `x.tld/…` gets `https://`;
 * anything with a scheme stays as typed.
 */
export function normalizeLink(input: string): string {
  const t = input.trim();
  if (t === "" || t.startsWith("#")) return t;
  if (/^[a-z][a-z0-9+.-]*:/i.test(t)) return t;
  if (/^[^\s@/]+@[^\s@/]+\.[^\s@/]+$/.test(t)) return `mailto:${t}`;
  if (/^(www\.)?[a-z0-9-]+(\.[a-z0-9-]+)+(:\d+)?([/?#].*)?$/i.test(t)) return `https://${t}`;
  return t;
}

/** Problems with a link the Hyperlink dialog shows (`null` when it can be saved). */
export function linkError(url: string): string | null {
  const t = url.trim();
  if (t === "") return null;
  if (isForbiddenLink(t)) return "Script and data links are not allowed.";
  if (new TextEncoder().encode(t).length > MAX_URL) return "That link is too long.";
  if (isInternalLink(t)) return t.length > 1 ? null : "Type a cell or name after #, e.g. #Sheet2.A1.";
  if (/^file:/i.test(t)) return null;
  if (!isOpenableUrl(t)) return "Use a web address (https://…), an e-mail address or #Sheet.A1.";
  return null;
}

// ---------------------------------------------------------------------------
// Conditional formatting
// ---------------------------------------------------------------------------

/** A style with nothing set. */
export function emptyCfStyle(): CfStyleDto {
  return { bold: null, italic: null, underline: null, strike: null, color: null, fill: null, borders: null, numberFormat: null, display: null };
}

/** A chip (pill) style. */
export function chipCfStyle(bg: number, fg: number): CfStyleDto {
  return { ...emptyCfStyle(), display: { type: "chip", bg, fg } };
}

/** The common highlight styles (Excel's "Light red fill with dark red text" …). */
export const CF_STYLE_PRESETS: readonly { id: string; label: string; style: CfStyleDto }[] = [
  { id: "red", label: "Light red fill, dark red text", style: { ...emptyCfStyle(), fill: 0xfee2e2ff, color: 0x991b1bff } },
  { id: "amber", label: "Yellow fill, dark yellow text", style: { ...emptyCfStyle(), fill: 0xfef3c7ff, color: 0x92400eff } },
  { id: "green", label: "Green fill, dark green text", style: { ...emptyCfStyle(), fill: 0xdcfce7ff, color: 0x166534ff } },
  { id: "blue", label: "Blue fill, dark blue text", style: { ...emptyCfStyle(), fill: 0xdbeafeff, color: 0x1e40afff } },
  { id: "redText", label: "Red text", style: { ...emptyCfStyle(), color: 0xdc2626ff } },
  { id: "bold", label: "Bold", style: { ...emptyCfStyle(), bold: true } },
  { id: "chipGreen", label: "Green chip", style: chipCfStyle(0xdcfce7ff, 0x166534ff) },
  { id: "chipRed", label: "Red chip", style: chipCfStyle(0xfee2e2ff, 0x991b1bff) },
];

/** Default style of new rules. */
export const DEFAULT_CF_STYLE: CfStyleDto = CF_STYLE_PRESETS[0]!.style;

/** A threshold. */
export function threshold(kind: CfValueKind, value = ""): CfThreshold {
  return { kind, value };
}

/** Colour scale presets (low → high). */
export const COLOR_SCALES: readonly { id: string; label: string; colors: number[] }[] = [
  { id: "rwg", label: "Red – white – green", colors: [0xf8696bff, 0xffffffff, 0x63be7bff] },
  { id: "gwr", label: "Green – white – red", colors: [0x63be7bff, 0xffffffff, 0xf8696bff] },
  { id: "wg", label: "White – green", colors: [0xffffffff, 0x63be7bff] },
  { id: "wb", label: "White – blue", colors: [0xffffffff, 0x5a8ac6ff] },
  { id: "ryg", label: "Red – yellow – green", colors: [0xf8696bff, 0xffeb84ff, 0x63be7bff] },
];

/** Colour scale stops for 2 or 3 colours (min, [50th percentile], max). */
export function colorScaleStops(colors: readonly number[]): CfColorStop[] {
  if (colors.length >= 3) {
    return [
      { threshold: threshold("min"), color: colors[0]! },
      { threshold: threshold("percentile", "50"), color: colors[1]! },
      { threshold: threshold("max"), color: colors[2]! },
    ];
  }
  return [
    { threshold: threshold("min"), color: colors[0] ?? 0xffffffff },
    { threshold: threshold("max"), color: colors[1] ?? 0x63be7bff },
  ];
}

/** Icon sets offered in the editor. */
export const ICON_SETS: readonly { set: IconSetKind; label: string }[] = [
  { set: "threeArrows", label: "3 arrows" },
  { set: "threeArrowsGray", label: "3 grey arrows" },
  { set: "threeTrafficLights", label: "3 traffic lights" },
  { set: "threeTrafficLightsRimmed", label: "3 traffic lights (rimmed)" },
  { set: "threeFlags", label: "3 flags" },
  { set: "threeSigns", label: "3 signs" },
  { set: "threeSymbols", label: "3 symbols (circled)" },
  { set: "threeSymbolsUncircled", label: "3 symbols" },
  { set: "threeStars", label: "3 stars" },
  { set: "threeTriangles", label: "3 triangles" },
  { set: "fourArrows", label: "4 arrows" },
  { set: "fourArrowsGray", label: "4 grey arrows" },
  { set: "fourRedToBlack", label: "4 red to black" },
  { set: "fourRating", label: "4 ratings" },
  { set: "fourTrafficLights", label: "4 traffic lights" },
  { set: "fiveArrows", label: "5 arrows" },
  { set: "fiveArrowsGray", label: "5 grey arrows" },
  { set: "fiveRating", label: "5 ratings" },
  { set: "fiveQuarters", label: "5 quarters" },
  { set: "fiveBoxes", label: "5 boxes" },
];

/** Icons in a set. */
export function iconSetSize(set: IconSetKind): number {
  return set.startsWith("three") ? 3 : set.startsWith("four") ? 4 : 5;
}

/** Percent thresholds splitting the range evenly (the first is ignored by the engine). */
export function iconThresholds(set: IconSetKind): CfThreshold[] {
  const n = iconSetSize(set);
  return Array.from({ length: n }, (_, i) => threshold("percent", String(Math.round((i * 100) / n))));
}

/** Rule types of the editor, in menu order. */
export type CfRuleType = CfRuleKind["type"];

export const CF_RULE_TYPES: readonly { type: CfRuleType; label: string }[] = [
  { type: "cellValue", label: "Cell value" },
  { type: "formula", label: "Formula is" },
  { type: "colorScale", label: "Colour scale" },
  { type: "dataBar", label: "Data bar" },
  { type: "iconSet", label: "Icon set" },
  { type: "top", label: "Top / bottom" },
  { type: "average", label: "Above / below average" },
  { type: "duplicate", label: "Duplicate values" },
  { type: "unique", label: "Unique values" },
  { type: "text", label: "Text" },
  { type: "blank", label: "Blank cells" },
  { type: "notBlank", label: "Non-blank cells" },
  { type: "error", label: "Errors" },
  { type: "notError", label: "No errors" },
  { type: "dateIs", label: "Date is" },
];

export const CMP_OPS: readonly { op: CmpOp; label: string; symbol: string; operands: 1 | 2 }[] = [
  { op: "gt", label: "greater than", symbol: ">", operands: 1 },
  { op: "ge", label: "greater than or equal to", symbol: "≥", operands: 1 },
  { op: "lt", label: "less than", symbol: "<", operands: 1 },
  { op: "le", label: "less than or equal to", symbol: "≤", operands: 1 },
  { op: "eq", label: "equal to", symbol: "=", operands: 1 },
  { op: "ne", label: "not equal to", symbol: "≠", operands: 1 },
  { op: "between", label: "between", symbol: "between", operands: 2 },
  { op: "notBetween", label: "not between", symbol: "not between", operands: 2 },
];

export const TEXT_OPS: readonly { op: CfTextOp; label: string }[] = [
  { op: "contains", label: "contains" },
  { op: "notContains", label: "does not contain" },
  { op: "beginsWith", label: "begins with" },
  { op: "endsWith", label: "ends with" },
];

export const DATE_PERIODS: readonly { period: CfDatePeriod; label: string }[] = [
  { period: "yesterday", label: "yesterday" },
  { period: "today", label: "today" },
  { period: "tomorrow", label: "tomorrow" },
  { period: "last7Days", label: "in the last 7 days" },
  { period: "lastWeek", label: "last week" },
  { period: "thisWeek", label: "this week" },
  { period: "nextWeek", label: "next week" },
  { period: "lastMonth", label: "last month" },
  { period: "thisMonth", label: "this month" },
  { period: "nextMonth", label: "next month" },
  { period: "lastYear", label: "last year" },
  { period: "thisYear", label: "this year" },
  { period: "nextYear", label: "next year" },
];

/** A rule kind with sensible defaults for the editor. */
export function defaultKind(type: CfRuleType): CfRuleKind {
  switch (type) {
    case "cellValue":
      return { type, op: "gt", formulas: ["0"] };
    case "formula":
      return { type, formula: "" };
    case "colorScale":
      return { type, stops: colorScaleStops(COLOR_SCALES[0]!.colors) };
    case "dataBar":
      return { type, min: threshold("autoMin"), max: threshold("autoMax"), color: 0x638ec6ff, negativeColor: 0xff0000ff, showValue: true, gradient: true };
    case "iconSet":
      return { type, set: "threeArrows", thresholds: iconThresholds("threeArrows"), showValue: true, reverse: false };
    case "top":
      return { type, rank: 10, percent: false, bottom: false };
    case "average":
      return { type, above: true, equal: false };
    case "text":
      return { type, op: "contains", text: "" };
    case "dateIs":
      return { type, period: "today" };
    default:
      return { type } as CfRuleKind;
  }
}

/** A rule with a default kind and style. */
export function defaultRule(type: CfRuleType = "cellValue"): CfRuleDto {
  return { kind: defaultKind(type), style: { ...DEFAULT_CF_STYLE }, stopIfTrue: false };
}

/** Whether a rule kind uses the style (colour scales, data bars and icon sets do not). */
export function usesStyle(kind: CfRuleKind): boolean {
  return kind.type !== "colorScale" && kind.type !== "dataBar" && kind.type !== "iconSet";
}

/** A text operand for a cell-value rule: `"Paid"` (quotes doubled). */
export function quoteText(s: string): string {
  return `"${s.replace(/"/g, '""')}"`;
}

/** The operand as the user sees it: numbers and formulas as typed, `"text"` unquoted. */
export function operandText(f: string): string {
  const m = /^"((?:[^"]|"")*)"$/.exec(f);
  return m ? (m[1] ?? "").replace(/""/g, '"') : f;
}

/** An operand from what the user typed: a number or a reference/formula stays, other text is quoted. */
export function operandFrom(input: string): string {
  const t = input.trim().replace(/^=/, "");
  if (t === "") return "0";
  if (/^-?\d+(\.\d+)?(e-?\d+)?%?$/i.test(t)) return t;
  if (/^"(?:[^"]|"")*"$/.test(t)) return t;
  if (/^\$?[A-Za-z]{1,3}\$?\d+$/.test(t) || /[()+*/<>^&]/.test(t)) return t;
  return quoteText(t);
}

/** One line describing a rule (manager list). */
export function ruleSummary(rule: CfRuleDto): string {
  const k = rule.kind;
  switch (k.type) {
    case "cellValue": {
      const op = CMP_OPS.find((o) => o.op === k.op);
      const vals = k.formulas.map(operandText);
      if (op?.operands === 2) return `Cell value ${op.label} ${vals[0] ?? ""} and ${vals[1] ?? ""}`;
      return `Cell value ${op?.symbol ?? k.op} ${vals[0] ?? ""}`;
    }
    case "formula":
      return `Formula: =${k.formula.replace(/^=/, "")}`;
    case "colorScale":
      return `Colour scale (${k.stops.length} colours)`;
    case "dataBar":
      return "Data bar";
    case "iconSet":
      return `Icon set: ${ICON_SETS.find((s) => s.set === k.set)?.label ?? k.set}`;
    case "top":
      return `${k.bottom ? "Bottom" : "Top"} ${k.rank}${k.percent ? "%" : ""}`;
    case "average":
      return `${k.above ? "Above" : "Below"} average${k.equal ? " (or equal)" : ""}`;
    case "duplicate":
      return "Duplicate values";
    case "unique":
      return "Unique values";
    case "text":
      return `Text ${TEXT_OPS.find((o) => o.op === k.op)?.label ?? k.op} “${k.text}”`;
    case "blank":
      return "Blank cells";
    case "notBlank":
      return "Non-blank cells";
    case "error":
      return "Cells with errors";
    case "notError":
      return "Cells without errors";
    case "dateIs":
      return `Date is ${DATE_PERIODS.find((p) => p.period === k.period)?.label ?? k.period}`;
  }
}

/** `F2:F15, H2` for a list of ranges. */
export function rangesText(ranges: readonly RangeDto[]): string {
  return ranges.map(a1).join(", ");
}

/** Parses `F2:F15, H2` (or `;`-separated); `null` if any part is not a range. */
export function parseRanges(text: string): RangeDto[] | null {
  const parts = text
    .split(/[,;]/)
    .map((p) => p.trim())
    .filter((p) => p !== "");
  if (parts.length === 0) return null;
  const out: RangeDto[] = [];
  for (const p of parts) {
    const r = parseA1Range(p.replace(/\$/g, ""));
    if (!r) return null;
    out.push(r);
  }
  return out;
}

/** Problems with a rule the editor shows (`null` when it can be saved). */
export function ruleError(rule: CfRuleDto): string | null {
  const k = rule.kind;
  switch (k.type) {
    case "cellValue":
      return k.formulas.some((f) => f.trim() === "") ? "Enter a value." : null;
    case "formula":
      return k.formula.trim().replace(/^=/, "") === "" ? "Enter a formula, e.g. $G2=\"Overdue\"." : null;
    case "top":
      return !Number.isInteger(k.rank) || k.rank < 1 || k.rank > 1000 ? "Rank is 1 to 1000." : null;
    case "text":
      return k.text === "" ? "Enter the text." : null;
    case "colorScale":
      return k.stops.length < 2 || k.stops.length > 3 ? "Use 2 or 3 colours." : null;
    default:
      return null;
  }
}

/** "Highlight cells" presets of the toolbar menu. */
export type HighlightPreset = "gt" | "lt" | "between" | "eq" | "textContains" | "duplicates" | "top10" | "colorScale" | "dataBar" | "iconSet";

export const HIGHLIGHT_PRESETS: readonly { id: HighlightPreset; label: string; values: 0 | 1 | 2; separator?: boolean }[] = [
  { id: "gt", label: "Greater than…", values: 1 },
  { id: "lt", label: "Less than…", values: 1 },
  { id: "between", label: "Between…", values: 2 },
  { id: "eq", label: "Equal to…", values: 1 },
  { id: "textContains", label: "Text that contains…", values: 1 },
  { id: "duplicates", label: "Duplicate values", values: 0 },
  { id: "top10", label: "Top 10 items", values: 0, separator: true },
  { id: "colorScale", label: "Colour scale", values: 0, separator: true },
  { id: "dataBar", label: "Data bars", values: 0 },
  { id: "iconSet", label: "Icon set (arrows)", values: 0 },
];

/** The rule for a preset with the values the user typed. */
export function presetRule(preset: HighlightPreset, values: readonly string[], style: CfStyleDto = DEFAULT_CF_STYLE): CfRuleDto {
  const v = (i: number) => operandFrom(values[i] ?? "");
  const rule = (kind: CfRuleKind): CfRuleDto => ({ kind, style: usesStyle(kind) ? { ...style } : emptyCfStyle(), stopIfTrue: false });
  switch (preset) {
    case "gt":
      return rule({ type: "cellValue", op: "gt", formulas: [v(0)] });
    case "lt":
      return rule({ type: "cellValue", op: "lt", formulas: [v(0)] });
    case "between":
      return rule({ type: "cellValue", op: "between", formulas: [v(0), v(1)] });
    case "eq":
      return rule({ type: "cellValue", op: "eq", formulas: [v(0)] });
    case "textContains":
      return rule({ type: "text", op: "contains", text: (values[0] ?? "").trim() });
    case "duplicates":
      return rule({ type: "duplicate" });
    case "top10":
      return rule({ type: "top", rank: 10, percent: false, bottom: false });
    case "colorScale":
      return rule(defaultKind("colorScale"));
    case "dataBar":
      return rule(defaultKind("dataBar"));
    case "iconSet":
      return rule(defaultKind("iconSet"));
  }
}

/**
 * "Colour by value": one chip rule per distinct value (`cellValue = "v"`),
 * in order of appearance.
 */
export function colorByValueRules(values: readonly string[], colors: ReadonlyMap<string, { bg: number; fg: number }>): CfRuleDto[] {
  return values.map((value) => {
    const chip = colors.get(value) ?? { bg: 0xf3f4f6ff, fg: 0x374151ff };
    return { kind: { type: "cellValue", op: "eq", formulas: [quoteText(value)] }, style: chipCfStyle(chip.bg, chip.fg), stopIfTrue: false };
  });
}

/** Whether a conditional format is a "Colour by value" one (only chip `= "text"` rules). */
export function isColorByValue(rules: readonly CfRuleDto[]): boolean {
  return (
    rules.length > 0 &&
    rules.every((r) => r.kind.type === "cellValue" && r.kind.op === "eq" && r.kind.formulas.length === 1 && /^".*"$/.test(r.kind.formulas[0] ?? "") && r.style.display?.type === "chip")
  );
}

/** Moves item `i` by `delta` (priority reordering); returns a new array. */
export function moveItem<T>(list: readonly T[], i: number, delta: -1 | 1): T[] {
  const j = i + delta;
  if (i < 0 || i >= list.length || j < 0 || j >= list.length) return [...list];
  const out = [...list];
  const [x] = out.splice(i, 1);
  out.splice(j, 0, x as T);
  return out;
}

// ---------------------------------------------------------------------------
// Data validation
// ---------------------------------------------------------------------------

export type ValidationType = ValidationKind["type"];

export const VALIDATION_TYPES: readonly { type: ValidationType; label: string }[] = [
  { type: "any", label: "Any value" },
  { type: "list", label: "List" },
  { type: "whole", label: "Whole number" },
  { type: "decimal", label: "Decimal" },
  { type: "date", label: "Date" },
  { type: "time", label: "Time" },
  { type: "textLength", label: "Text length" },
  { type: "custom", label: "Custom formula" },
];

/** A validation kind with defaults for the dialog. */
export function defaultValidationKind(type: ValidationType): ValidationKind {
  switch (type) {
    case "any":
      return { type };
    case "list":
      return { type, source: { type: "values", items: [] } };
    case "custom":
      return { type, formula: "" };
    default:
      return { type, op: "between", formulas: ["1", "10"] };
  }
}

/** A new validation for `ranges` (list, Stop alert). */
export function defaultValidation(ranges: readonly RangeDto[]): ValidationDto {
  return {
    id: 0,
    ranges: ranges.map((r) => ({ ...r })),
    kind: defaultValidationKind("list"),
    allowBlank: true,
    showDropdown: true,
    inputHelp: null,
    error: { style: "stop", title: "", text: "" },
  };
}

/** List items from the inline text: one per line, or comma-separated. */
export function listItems(text: string): string[] {
  const parts = text.includes("\n") ? text.split(/\r?\n/) : text.split(",");
  return parts.map((s) => s.trim()).filter((s) => s !== "");
}

/** Problems with a validation the dialog shows (`null` when it can be applied). */
export function validationError(v: ValidationDto): string | null {
  const k = v.kind;
  if (k.type === "list") {
    if (k.source.type === "values" && k.source.items.length === 0) return "Enter the list items.";
    if (k.source.type === "range" && k.source.reference.trim() === "") return "Enter the source range, e.g. $Lists.$A$1:$A$5.";
  }
  if (k.type === "custom" && k.formula.trim().replace(/^=/, "") === "") return "Enter a formula.";
  if ("formulas" in k) {
    const n = CMP_OPS.find((o) => o.op === k.op)?.operands ?? 1;
    if (k.formulas.slice(0, n).some((f) => f.trim() === "")) return "Enter the limits.";
  }
  if (v.error && v.error.title.length > 256) return "The title is too long.";
  return null;
}

export const ALERT_STYLES: readonly { style: ValidationErrorStyle; label: string; hint: string }[] = [
  { style: "stop", label: "Stop", hint: "Invalid values are rejected." },
  { style: "warning", label: "Warning", hint: "Invalid values can be kept or undone." },
  { style: "info", label: "Information", hint: "Invalid values are accepted with a message." },
];

// ---------------------------------------------------------------------------
// Page setup
// ---------------------------------------------------------------------------

/** Paper sizes in millimetres (portrait width × height). */
export const PAPERS: readonly { type: Exclude<Paper["type"], "custom">; label: string; mm: [number, number] }[] = [
  { type: "a4", label: "A4", mm: [210, 297] },
  { type: "a3", label: "A3", mm: [297, 420] },
  { type: "a5", label: "A5", mm: [148, 210] },
  { type: "b5", label: "B5", mm: [176, 250] },
  { type: "letter", label: "Letter", mm: [215.9, 279.4] },
  { type: "legal", label: "Legal", mm: [215.9, 355.6] },
  { type: "executive", label: "Executive", mm: [184.2, 266.7] },
  { type: "tabloid", label: "Tabloid", mm: [279.4, 431.8] },
];

/** Paper width × height in mm, oriented. */
export function paperMm(setup: Pick<PageSetupDto, "paper" | "orientation">): [number, number] {
  const p = setup.paper;
  const mm: [number, number] = p.type === "custom" ? [p.widthMm, p.heightMm] : (PAPERS.find((x) => x.type === p.type)?.mm ?? [210, 297]);
  return setup.orientation === "landscape" ? [mm[1], mm[0]] : mm;
}

/** CSS px per mm (96 dpi). */
export const PX_PER_MM = 96 / 25.4;

/** A4 portrait, 20 mm margins, 100 %. */
export function defaultPageSetup(): PageSetupDto {
  return {
    orientation: "portrait",
    paper: { type: "a4" },
    margins: { topMm: 20, bottomMm: 20, leftMm: 20, rightMm: 20, headerMm: 8, footerMm: 8 },
    scale: { type: "percent", percent: 100 },
    printRange: null,
    repeatRows: null,
    repeatCols: null,
    header: null,
    footer: null,
    printGridlines: false,
    printHeaders: false,
    centerH: false,
    centerV: false,
    pageOrder: "topToBottom",
    firstPageNumber: null,
    rowBreaks: [],
    colBreaks: [],
  };
}

/** Size (px) of index `i` from layout runs. */
function sizeAt(runs: readonly DimRunPx[], i: number, dflt: number): number {
  for (const r of runs) if (i >= r.start && i <= r.end) return r.hidden || r.filtered ? 0 : r.size;
  return dflt;
}

/**
 * Automatic page breaks of the print area (the print range or the used
 * area): rows and columns are packed onto pages of the printable size
 * (paper minus margins, divided by the scale), starting a new page at every
 * manual break. `fitTo` scaling is approximated as one page per requested
 * page across the area. Returns the indices a new page starts at (> start).
 */
export function autoPageBreaks(
  setup: PageSetupDto,
  area: RangeDto,
  rows: { runs: readonly DimRunPx[]; dflt: number },
  cols: { runs: readonly DimRunPx[]; dflt: number },
  limit = 5000,
): { rows: number[]; cols: number[] } {
  const [wMm, hMm] = paperMm(setup);
  const m = setup.margins;
  const availW = Math.max(10, (wMm - m.leftMm - m.rightMm) * PX_PER_MM);
  const availH = Math.max(10, (hMm - m.topMm - m.bottomMm) * PX_PER_MM);
  const pack = (start: number, end: number, runs: readonly DimRunPx[], dflt: number, avail: number, manual: readonly number[], fit: number | null): number[] => {
    let total = 0;
    if (fit !== null) for (let i = start; i <= end && i - start < 200_000; i++) total += sizeAt(runs, i, dflt);
    const page = fit !== null && fit > 0 ? Math.max(avail, total / fit) : avail;
    const out: number[] = [];
    let used = 0;
    const manualSet = new Set(manual);
    for (let i = start; i <= end && out.length < limit; i++) {
      const s = sizeAt(runs, i, dflt);
      if (i > start && (manualSet.has(i) || (used + s > page && used > 0))) {
        out.push(i);
        used = 0;
      }
      used += s;
    }
    return out;
  };
  const scale = setup.scale.type === "percent" ? Math.max(10, Math.min(400, setup.scale.percent)) / 100 : 1;
  const fitW = setup.scale.type === "fitTo" ? setup.scale.width : null;
  const fitH = setup.scale.type === "fitTo" ? setup.scale.height : null;
  return {
    rows: pack(area.r0, area.r1, rows.runs, rows.dflt, availH / scale, setup.rowBreaks, fitH && fitH > 0 ? fitH : null),
    cols: pack(area.c0, area.c1, cols.runs, cols.dflt, availW / scale, setup.colBreaks, fitW && fitW > 0 ? fitW : null),
  };
}

/** Header/footer placeholders and their buttons. */
export const HF_PLACEHOLDERS: readonly { token: string; label: string }[] = [
  { token: "&[PAGE]", label: "Page" },
  { token: "&[PAGES]", label: "Pages" },
  { token: "&[SHEET]", label: "Sheet" },
  { token: "&[FILE]", label: "File" },
  { token: "&[DATE]", label: "Date" },
  { token: "&[TIME]", label: "Time" },
];

/** Inserts `token` at the caret (replacing the selection). */
export function insertToken(text: string, start: number, end: number, token: string): { text: string; caret: number } {
  const s = Math.max(0, Math.min(start, text.length));
  const e = Math.max(s, Math.min(end, text.length));
  return { text: text.slice(0, s) + token + text.slice(e), caret: s + token.length };
}

/** Header/footer preview with sample values. */
export function previewHeaderFooter(text: string, ctx: { sheet: string; file: string; page?: number; pages?: number; now?: Date }): string {
  const now = ctx.now ?? new Date();
  return text
    .replace(/&\[PAGES\]/g, String(ctx.pages ?? 1))
    .replace(/&\[PAGE\]/g, String(ctx.page ?? 1))
    .replace(/&\[SHEET\]/g, ctx.sheet)
    .replace(/&\[FILE\]/g, ctx.file)
    .replace(/&\[DATE\]/g, now.toLocaleDateString())
    .replace(/&\[TIME\]/g, now.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" }));
}

/** `1:2` / `A:B` text of a repeat span. */
export function spanText(span: readonly [number, number] | null, axis: "row" | "col"): string {
  if (!span) return "";
  if (axis === "row") return `${span[0] + 1}:${span[1] + 1}`;
  const col = (c: number) => a1({ r0: 0, c0: c, r1: 0, c1: c }).replace(/\d+$/, "");
  return `${col(span[0])}:${col(span[1])}`;
}

/** Parses `1:2` / `3` (rows) or `A:B` / `C` (columns); `undefined` if invalid, `null` if empty. */
export function parseSpan(text: string, axis: "row" | "col"): [number, number] | null | undefined {
  const t = text.trim().replace(/\$/g, "");
  if (t === "") return null;
  const parts = t.split(":").map((x) => x.trim());
  if (parts.length > 2) return undefined;
  const one = (p: string): number | null => {
    if (axis === "row") return /^\d+$/.test(p) && Number(p) >= 1 ? Number(p) - 1 : null;
    const r = parseA1Range(`${p}1`);
    return r ? r.c0 : null;
  };
  const a = one(parts[0] ?? "");
  const b = one(parts[1] ?? parts[0] ?? "");
  if (a === null || b === null) return undefined;
  return [Math.min(a, b), Math.max(a, b)];
}

// ---------------------------------------------------------------------------
// Protection, goal seek, locales
// ---------------------------------------------------------------------------

/** What users may do on a protected sheet by default (select cells only). */
export const DEFAULT_PROTECTION: ProtectionOptions = {
  selectLocked: true,
  selectUnlocked: true,
  insertRows: false,
  insertCols: false,
  deleteRows: false,
  deleteCols: false,
  formatCells: false,
  sort: false,
  autoFilter: false,
};

export const PROTECTION_LABELS: readonly { key: keyof ProtectionOptions; label: string }[] = [
  { key: "selectLocked", label: "Select locked cells" },
  { key: "selectUnlocked", label: "Select unlocked cells" },
  { key: "formatCells", label: "Format cells" },
  { key: "insertRows", label: "Insert rows" },
  { key: "insertCols", label: "Insert columns" },
  { key: "deleteRows", label: "Delete rows" },
  { key: "deleteCols", label: "Delete columns" },
  { key: "sort", label: "Sort" },
  { key: "autoFilter", label: "Use AutoFilter" },
];

/** A single cell from `B2` / `$B$2`, or `null`. */
export function parseCell(text: string): CellPos | null {
  const r = parseA1Range(text.trim().replace(/\$/g, ""));
  if (!r || r.r0 !== r.r1 || r.c0 !== r.c1) return null;
  return { r: r.r0, c: r.c0 };
}

/** `B2` for a cell. */
export function cellText(p: CellPos): string {
  return a1({ r0: p.r, c0: p.c, r1: p.r, c1: p.c });
}

/** A number typed in the UI (`1,234.5`, `-3`, `12%`), or `null`. */
export function parseNumber(text: string): number | null {
  const t = text.trim().replace(/,/g, "");
  if (!/^[-+]?(\d+\.?\d*|\.\d+)(e[-+]?\d+)?%?$/i.test(t)) return null;
  const n = Number(t.replace(/%$/, ""));
  if (!Number.isFinite(n)) return null;
  return t.endsWith("%") ? n / 100 : n;
}

/** The locales of View ▸ Locale (command ids `view.locale.<tag without ->`). */
export const UI_LOCALES: readonly { tag: string; label: string; command: string }[] = [
  { tag: "en-US", label: "English (US)", command: "view.locale.enUS" },
  { tag: "en-GB", label: "English (UK)", command: "view.locale.enGB" },
  { tag: "de-DE", label: "Deutsch (Deutschland)", command: "view.locale.deDE" },
  { tag: "fr-FR", label: "Français (France)", command: "view.locale.frFR" },
  { tag: "es-ES", label: "Español (España)", command: "view.locale.esES" },
  { tag: "ja-JP", label: "日本語 (日本)", command: "view.locale.jaJP" },
];
