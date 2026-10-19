/**
 * Character classes and helpers matching the Rust lexer
 * (crates/tally-formula/src/refsyntax.rs, Rust `char` predicates).
 *
 * Positions are UTF-16 code-unit indices; scanning steps by code point.
 */

/** The code point at `i`, or `undefined` past the end. */
export function cpAt(s: string, i: number): number | undefined {
  return i < s.length ? s.codePointAt(i) : undefined;
}

/** UTF-16 length of a code point. */
export function cpLen(cp: number): number {
  return cp > 0xffff ? 2 : 1;
}

/** Rust `char::is_whitespace` (Unicode White_Space; not U+FEFF). */
export function isWhitespace(cp: number): boolean {
  return (
    (cp >= 0x09 && cp <= 0x0d) ||
    cp === 0x20 ||
    cp === 0x85 ||
    cp === 0xa0 ||
    cp === 0x1680 ||
    (cp >= 0x2000 && cp <= 0x200a) ||
    cp === 0x2028 ||
    cp === 0x2029 ||
    cp === 0x202f ||
    cp === 0x205f ||
    cp === 0x3000
  );
}

const ALPHABETIC = /^\p{Alphabetic}$/u;
const ALPHANUMERIC = /^[\p{Alphabetic}\p{Nd}\p{Nl}\p{No}]$/u;

export function isAsciiAlpha(cp: number | undefined): boolean {
  return cp !== undefined && ((cp >= 0x41 && cp <= 0x5a) || (cp >= 0x61 && cp <= 0x7a));
}

export function isAsciiDigit(cp: number | undefined): boolean {
  return cp !== undefined && cp >= 0x30 && cp <= 0x39;
}

/** First character of an identifier or unquoted sheet name. */
export function isIdentStart(cp: number): boolean {
  if (cp < 0x80) return isAsciiAlpha(cp) || cp === 0x5f;
  return ALPHABETIC.test(String.fromCodePoint(cp));
}

/** A character that may continue an identifier (names, function names). */
export function isIdentChar(cp: number): boolean {
  if (cp < 0x80) return isAsciiAlpha(cp) || isAsciiDigit(cp) || cp === 0x5f || cp === 0x2e;
  return ALPHANUMERIC.test(String.fromCodePoint(cp));
}

/** A character that may continue an unquoted sheet name. */
export function isSheetChar(cp: number): boolean {
  return cp !== 0x2e && isIdentChar(cp);
}

/** ASCII case-insensitive equality (non-ASCII must match exactly). */
export function eqIgnoreAsciiCase(a: string, b: string): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) {
    let x = a.charCodeAt(i);
    let y = b.charCodeAt(i);
    if (x >= 0x61 && x <= 0x7a) x -= 32;
    if (y >= 0x61 && y <= 0x7a) y -= 32;
    if (x !== y) return false;
  }
  return true;
}
