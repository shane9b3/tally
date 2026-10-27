/**
 * Browser demo engine: typed input with a leading `+` or `-` becomes a
 * formula when it is not a number (AOO `ScViewFunc::EnterData`; the real
 * engine's `Workbook::set_typed_input`). `+B1+B2` is stored `=+B1+B2`;
 * `-5`, `+1e3`, `-5%`, `- 5`, `--5`, a lone sign, and input that does not
 * compile or uses an unknown name or function (`-abc`) are left alone.
 * Only `setCellInput` (one typed cell) uses this; paste, CSV and Fill
 * Selection do not.
 */
import { tokenize, type TokenKind } from "@tally/formula-ts";

/**
 * Signed numbers, percentages, exponents, currency and times. A signed date
 * is not a number (AOO's scanner rejects it), so `+12/31/2024` is a formula.
 */
const NUMBER = /^[+-][ \u00a0]*(\$?(\d[\d,]*(\.\d*)?|\.\d+)(e[+-]?\d+)?%?|\d+:\d{2}(:\d{2}(\.\d+)?)?)$/i;

const BINARY: ReadonlySet<TokenKind> = new Set<TokenKind>(["Plus", "Minus", "Star", "Slash", "Caret", "Amp", "Eq", "Ne", "Lt", "Le", "Gt", "Ge", "Colon", "Tilde", "Bang", "Comma", "Semicolon", "LParen"]);

export interface SignedContext {
  /** A defined name (case-insensitive). */
  isName(name: string): boolean;
  /** A known function (upper case). */
  isFunction(name: string): boolean;
}

/** The formula `input` is stored as (`=` + input), or `null` to store it as typed. */
export function signedFormulaInput(input: string, ctx: SignedContext): string | null {
  const sign = input[0];
  if (sign !== "+" && sign !== "-") return null;
  const rest = input.slice(1).replace(/^[+-]+/, "");
  if (rest === "" || NUMBER.test(sign + rest)) return null;
  let depth = 0;
  let last: TokenKind | null = null;
  for (const t of tokenize(input)) {
    switch (t.kind) {
      case "Invalid":
        return null;
      case "Ident":
        if (!/^(true|false)$/i.test(t.text) && !ctx.isName(t.text)) return null;
        break;
      case "Func":
        if (!ctx.isFunction(t.text.toUpperCase())) return null;
        break;
      case "LParen":
        depth++;
        break;
      case "RParen":
        if (--depth < 0) return null;
        break;
      default:
        break;
    }
    if (t.kind !== "Whitespace") last = t.kind;
  }
  if (depth !== 0 || last === null || BINARY.has(last)) return null;
  return `=${input}`;
}
