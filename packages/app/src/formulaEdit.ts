/**
 * Formula-editing UX as pure functions over the edit text and caret:
 * reference colours (editor spans + grid outlines), point-mode insertion,
 * function autocomplete and the signature tooltip. The controller and the
 * components (FormulaBar, FormulaAssist, GridHost) share these.
 */
import type { FunctionInfo, RangeDto } from "@tally/bridge";
import {
  callAt,
  formatRef,
  groupedReferences,
  identifierAt,
  isFormulaText,
  pointInsertion,
  type Span,
} from "@tally/formula-ts";
import type { RefHighlight, TextHighlight } from "@tally/grid";
import type { ResolvedTheme } from "./theme/theme";

/** Reference colours, in order of first appearance (cycled). */
export const REF_COLORS_LIGHT = ["#2563eb", "#dc2626", "#7c3aed", "#059669", "#d97706", "#db2777", "#0891b2", "#65a30d"];
/** Lighter variants that read well on the dark editor background. */
export const REF_COLORS_DARK = ["#60a5fa", "#f87171", "#a78bfa", "#34d399", "#fbbf24", "#f472b6", "#22d3ee", "#a3e635"];

export function refColor(group: number, theme: ResolvedTheme): string {
  const p = theme === "dark" ? REF_COLORS_DARK : REF_COLORS_LIGHT;
  return p[group % p.length] ?? p[0]!;
}

export interface FormulaHighlights {
  /** Coloured spans of the text. */
  spans: TextHighlight[];
  /** Outlines on the grid (references to the shown sheet only). */
  refs: RefHighlight[];
}

const NONE: FormulaHighlights = { spans: [], refs: [] };

/**
 * Colours every reference of a formula; the same reference (ignoring `$`)
 * gets the same colour. Outlines are only produced for references to
 * `sheetName` (or unqualified ones).
 */
export function formulaHighlights(text: string, sheetName: string | null, theme: ResolvedTheme): FormulaHighlights {
  if (!isFormulaText(text)) return NONE;
  const refs = groupedReferences(text);
  if (refs.length === 0) return NONE;
  const spans: TextHighlight[] = [];
  const outlines: RefHighlight[] = [];
  const seen = new Set<number>();
  for (const r of refs) {
    const color = refColor(r.group, theme);
    spans.push({ start: r.start, end: r.end, color });
    const here = r.sheet === null || (sheetName !== null && r.sheet.toLowerCase() === sheetName.toLowerCase());
    if (here && !seen.has(r.group)) {
      seen.add(r.group);
      outlines.push({ range: { ...r.range }, color });
    }
  }
  return { spans, refs: outlines };
}

/**
 * Where a picked reference goes: the previous pick's span when the caret
 * is still at its end, else the point-mode insertion point at the caret.
 */
export function pointTarget(text: string, caret: number, pointSpan: Span | null): Span | null {
  if (pointSpan && caret === pointSpan.end && pointSpan.end <= text.length) return pointSpan;
  return pointInsertion(text, caret);
}

/** Whether clicks/arrows pick references for this text and caret. */
export function isPointMode(text: string, caret: number, pointSpan: Span | null): boolean {
  return isFormulaText(text) && pointTarget(text, caret, pointSpan) !== null;
}

/** `A1` / `A1:B3` for a picked range. */
export function refText(range: RangeDto): string {
  return formatRef(range);
}

/** Inserts or replaces a picked reference. Returns the new text, caret and the reference's span. */
export function applyPoint(
  text: string,
  caret: number,
  pointSpan: Span | null,
  range: RangeDto,
): { text: string; caret: number; span: Span } | null {
  const at = pointTarget(text, caret, pointSpan);
  if (!at) return null;
  const ref = refText(range);
  const next = text.slice(0, at.start) + ref + text.slice(at.end);
  const end = at.start + ref.length;
  return { text: next, caret: end, span: { start: at.start, end } };
}

/** Autocomplete candidates for the identifier at the caret. */
export interface AssistList {
  span: Span;
  prefix: string;
  items: FunctionInfo[];
}

/** Most candidates shown. */
export const ASSIST_MAX = 8;

/** Functions whose name starts with the identifier being typed (or `null`). */
export function assistFor(text: string, caret: number, functions: readonly FunctionInfo[], dismissed: string | null): AssistList | null {
  const id = identifierAt(text, caret);
  if (!id) return null;
  const prefix = id.text.toUpperCase();
  if (dismissed !== null && dismissed === `${id.start}:${prefix}`) return null;
  const items = functions
    .filter((f) => f.name.toUpperCase().startsWith(prefix))
    .sort((a, b) => a.name.length - b.name.length || a.name.localeCompare(b.name))
    .slice(0, ASSIST_MAX);
  if (items.length === 0) return null;
  // Typing the full name of the only match: nothing left to complete.
  if (items.length === 1 && items[0]?.name.toUpperCase() === prefix && text[caret] === "(") return null;
  return { span: { start: id.start, end: id.end }, prefix, items };
}

/** The dismissal key of an assist list (Esc hides it until the identifier changes). */
export function assistKey(a: AssistList): string {
  return `${a.span.start}:${a.prefix}`;
}

/** Replaces the identifier with `NAME(` (keeping an existing `(`). */
export function applyCompletion(text: string, span: Span, name: string): { text: string; caret: number } {
  const hasParen = text[span.end] === "(";
  const insert = hasParen ? name : `${name}(`;
  const next = text.slice(0, span.start) + insert + text.slice(span.end);
  return { text: next, caret: span.start + insert.length + (hasParen ? 1 : 0) };
}

/** The function whose argument list holds the caret, with its info. */
export interface SignatureHint {
  info: FunctionInfo;
  argIndex: number;
}

export function signatureAt(text: string, caret: number, functions: readonly FunctionInfo[]): SignatureHint | null {
  const call = callAt(text, caret);
  if (!call) return null;
  const info = functions.find((f) => f.name.toUpperCase() === call.name);
  return info ? { info, argIndex: call.argIndex } : null;
}

/**
 * Splits a signature like `SUM(number1, [number2], …)` into the text
 * before, the current argument and after (for bolding the argument).
 */
export function splitSignature(signature: string, argIndex: number): [string, string, string] {
  const open = signature.indexOf("(");
  const close = signature.lastIndexOf(")");
  if (open < 0 || close < open) return [signature, "", ""];
  const inner = signature.slice(open + 1, close);
  const args: { start: number; end: number }[] = [];
  let depth = 0;
  let start = 0;
  for (let i = 0; i <= inner.length; i++) {
    const ch = inner[i];
    if (ch === "[" || ch === "(") depth++;
    else if (ch === "]" || ch === ")") depth--;
    if (i === inner.length || ((ch === "," || ch === ";") && depth === 0)) {
      args.push({ start, end: i });
      start = i + 1;
    }
  }
  // Variadic tails ("…") keep highlighting the last real argument.
  let k = Math.min(argIndex, args.length - 1);
  while (k > 0 && /^\s*(…|\.\.\.)\s*$/.test(inner.slice(args[k]!.start, args[k]!.end))) k--;
  const a = args[k];
  if (!a) return [signature, "", ""];
  const lead = inner.slice(a.start, a.end).match(/^\s*/)?.[0].length ?? 0;
  const s0 = open + 1 + a.start + lead;
  const s1 = open + 1 + a.end;
  return [signature.slice(0, s0), signature.slice(s0, s1), signature.slice(s1)];
}
