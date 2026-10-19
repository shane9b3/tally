/**
 * Formula-editor helpers built on the lexer: where point mode may insert a
 * reference, the identifier being typed (function autocomplete), the
 * function call the caret is in (signature tooltip) and reference grouping
 * for colouring. All offsets are UTF-16 indices into the edited text.
 */
import { tokenize, type Dialect, type Token, type TokenKind } from "./lexer";
import { referencesIn, type RefInfo } from "./refs";

/** A half-open span `[start, end)` of the text. */
export interface Span {
  start: number;
  end: number;
}

/** Tokens after which a reference may start (an operand is expected). */
const OPERAND_EXPECTED: ReadonlySet<TokenKind> = new Set<TokenKind>([
  "Prefix",
  "Plus",
  "Minus",
  "Star",
  "Slash",
  "Caret",
  "Amp",
  "Eq",
  "Ne",
  "Lt",
  "Le",
  "Gt",
  "Ge",
  "Colon",
  "Tilde",
  "Bang",
  "LParen",
  "Comma",
  "Semicolon",
  "LBrace",
  "Pipe",
]);

/** True for text the editor treats as a formula (`=...`). */
export function isFormulaText(src: string): boolean {
  return src.startsWith("=");
}

function lastSignificantBefore(tokens: readonly Token[], index: number): Token | undefined {
  for (let i = index - 1; i >= 0; i--) {
    const t = tokens[i];
    if (t && t.kind !== "Whitespace") return t;
  }
  return undefined;
}

/**
 * Point mode: where a picked reference goes for the caret at `caret`, or
 * `null` when point mode is off there.
 *
 * - After an operator, `(`, `,`, `;` or the leading `=` (whitespace
 *   allowed): an empty span at the caret (insert).
 * - Directly after a reference that itself follows such a token: that
 *   reference's span (replace), so repeated picks replace the reference.
 */
export function pointInsertion(src: string, caret: number, dialect: Dialect = "calc-a1"): Span | null {
  if (!isFormulaText(src) || caret < 1 || caret > src.length) return null;
  const tokens = tokenize(src, dialect);
  // The token that ends exactly at, or contains, the caret.
  let i = 0;
  while (i < tokens.length && (tokens[i]?.end ?? 0) < caret) i++;
  const at = tokens[i];
  if (at && at.start < caret && at.end > caret) {
    // Inside a token: only whitespace allows a pick.
    if (at.kind !== "Whitespace") return null;
    const prev = lastSignificantBefore(tokens, i);
    return prev && OPERAND_EXPECTED.has(prev.kind) ? { start: caret, end: caret } : null;
  }
  // `at` ends at the caret (or is the first token starting at the caret).
  const endsHere = at && at.end === caret ? at : undefined;
  const idx = endsHere ? i + 1 : i;
  if (endsHere && (endsHere.kind === "CellRef" || endsHere.kind === "RangeRef")) {
    const before = lastSignificantBefore(tokens, i);
    return before && OPERAND_EXPECTED.has(before.kind) ? { start: endsHere.start, end: endsHere.end } : null;
  }
  const prev = lastSignificantBefore(tokens, idx);
  if (!prev || !OPERAND_EXPECTED.has(prev.kind)) return null;
  // A unary-looking minus right after an operand is still binary; fine either way.
  return { start: caret, end: caret };
}

/** The identifier ending at the caret (for function autocomplete), or `null`. */
export function identifierAt(src: string, caret: number, dialect: Dialect = "calc-a1"): (Span & { text: string }) | null {
  if (!isFormulaText(src)) return null;
  const tokens = tokenize(src, dialect);
  for (let i = 0; i < tokens.length; i++) {
    const t = tokens[i];
    if (!t || t.end !== caret || t.start >= caret) continue;
    const plainRef = t.kind === "CellRef" && /^[A-Za-z]+[0-9]+$/.test(t.text);
    if (t.kind !== "Ident" && !plainRef) return null;
    if (!/^[A-Za-z_]/.test(t.text)) return null;
    const prev = lastSignificantBefore(tokens, i);
    if (prev && !OPERAND_EXPECTED.has(prev.kind)) return null;
    // Sheet-qualified names (`Sheet1.A`) are not function names.
    if (t.text.includes(".")) return null;
    return { start: t.start, end: t.end, text: t.text };
  }
  return null;
}

/** The function call whose parentheses contain the caret. */
export interface CallContext {
  /** Function name as typed (upper-cased). */
  name: string;
  /** 0-based index of the argument the caret is in. */
  argIndex: number;
  /** Span of the function name. */
  nameSpan: Span;
}

/** The innermost function call around the caret, or `null`. */
export function callAt(src: string, caret: number, dialect: Dialect = "calc-a1"): CallContext | null {
  if (!isFormulaText(src)) return null;
  type Frame = CallContext | "paren" | "brace";
  const stack: Frame[] = [];
  let pendingFunc: Token | null = null;
  for (const t of tokenize(src, dialect)) {
    if (t.start >= caret) break;
    switch (t.kind) {
      case "Func":
        pendingFunc = t;
        continue;
      case "Whitespace":
        continue;
      case "LParen":
        stack.push(
          pendingFunc
            ? { name: pendingFunc.text.toUpperCase(), argIndex: 0, nameSpan: { start: pendingFunc.start, end: pendingFunc.end } }
            : "paren",
        );
        break;
      case "LBrace":
        stack.push("brace");
        break;
      case "RParen":
      case "RBrace":
        stack.pop();
        break;
      case "Comma":
      case "Semicolon": {
        const top = stack[stack.length - 1];
        if (top && typeof top === "object") top.argIndex++;
        break;
      }
      default:
        break;
    }
    pendingFunc = null;
  }
  for (let i = stack.length - 1; i >= 0; i--) {
    const f = stack[i];
    if (f && typeof f === "object") return f;
  }
  return null;
}

/** A reference with the index of its colour group (same reference text → same group). */
export interface GroupedRef extends RefInfo {
  group: number;
}

/**
 * The references of a formula, each with a colour group: distinct
 * references (ignoring `$` and case) get consecutive groups in order of
 * first appearance.
 */
export function groupedReferences(src: string, dialect: Dialect = "calc-a1"): GroupedRef[] {
  if (!isFormulaText(src) && !src.startsWith("of:=")) return [];
  const groups = new Map<string, number>();
  return referencesIn(src, dialect).map((r) => {
    const key = `${r.sheet ?? ""}|${r.sheet2 ?? ""}|${r.range.r0},${r.range.c0},${r.range.r1},${r.range.c1}`.toLowerCase();
    let g = groups.get(key);
    if (g === undefined) {
      g = groups.size;
      groups.set(key, g);
    }
    return { ...r, group: g };
  });
}
