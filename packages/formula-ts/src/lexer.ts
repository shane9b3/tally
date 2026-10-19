/**
 * Formula tokenizer: a faithful port of crates/tally-formula/src/lexer.rs.
 *
 * Never fails: unknown characters become `Invalid` tokens. Whitespace is
 * kept, so tokens cover the whole input. Unlike the Rust lexer (UTF-8 byte
 * spans), spans here are UTF-16 code-unit offsets into the JS string. Both
 * pass the shared fixtures in crates/tally-formula/tests/fixtures/tokens.
 */
import { cpAt, cpLen, eqIgnoreAsciiCase, isAsciiDigit, isIdentChar, isIdentStart, isSheetChar, isWhitespace } from "./chars";
import { parseA1, scanA1Ref, scanBracket, scanQuoted, type RawRef } from "./refsyntax";

export type TokenKind =
  | "Prefix"
  | "Whitespace"
  | "Number"
  | "String"
  | "Ident"
  | "Func"
  | "CellRef"
  | "RangeRef"
  | "OfRef"
  | "Error"
  | "Plus"
  | "Minus"
  | "Star"
  | "Slash"
  | "Caret"
  | "Amp"
  | "Eq"
  | "Ne"
  | "Lt"
  | "Le"
  | "Gt"
  | "Ge"
  | "Percent"
  | "Colon"
  | "Tilde"
  | "Bang"
  | "LParen"
  | "RParen"
  | "LBrace"
  | "RBrace"
  | "Comma"
  | "Semicolon"
  | "Pipe"
  | "Invalid";

/**
 * `calc-a1`: the UI dialect (`Sheet1.A1`); `open-formula`: ODF storage
 * (`[.A1]`); `excel-a1`: OOXML / Excel syntax (`Sheet1!A1`, 3D
 * `Jan:Dec!B2`, dotted sheet names `Sales.EU!A1`, `Sheet1!#REF!`).
 */
export type Dialect = "calc-a1" | "open-formula" | "excel-a1";

export interface Token {
  kind: TokenKind;
  /** `src.slice(start, end)`. */
  text: string;
  /** UTF-16 code-unit offsets. */
  start: number;
  end: number;
}

/** Tokens the parser ignores. */
export function isTrivia(kind: TokenKind): boolean {
  return kind === "Whitespace" || kind === "Prefix";
}

const ERROR_NAMES = ["#NULL!", "#DIV/0!", "#VALUE!", "#REF!", "#NAME?", "#NUM!", "#N/A"] as const;

/** Formula prefix: its length and whether it switches to OpenFormula. */
export function detectPrefix(src: string): [number, boolean] | null {
  if (src.startsWith("of:=")) return [4, true];
  if (src.startsWith("oooc:=")) return [6, true];
  if (src.startsWith("=")) return [1, false];
  return null;
}

/** Whether the formula uses OpenFormula syntax (dialect, or an `of:=` / `oooc:=` prefix). */
export function usesOpenFormula(src: string, dialect: Dialect = "calc-a1"): boolean {
  return dialect === "open-formula" || (detectPrefix(src)?.[1] ?? false);
}

/** Tokenizes `src`. */
export function tokenize(src: string, dialect: Dialect = "calc-a1"): Token[] {
  const out: Token[] = [];
  let of = dialect === "open-formula";
  let i = 0;
  const prefix = detectPrefix(src);
  if (prefix) {
    of ||= prefix[1];
    out.push({ kind: "Prefix", text: src.slice(0, prefix[0]), start: 0, end: prefix[0] });
    i = prefix[0];
  }
  const excel = dialect === "excel-a1" && !of;
  for (let c = cpAt(src, i); c !== undefined; c = cpAt(src, i)) {
    let r: [TokenKind, number] | null = null;
    if (excel) {
      const d3 = scanExcel3d(src, i);
      if (d3 !== null) r = ["RangeRef", d3];
      else r = scanExcelQualified(src, i);
    }
    const [kind, e] = r ?? lexOne(src, i, c, of);
    // Defensive: always make progress.
    const end = e <= i ? i + cpLen(c) : e;
    out.push({ kind, text: src.slice(i, end), start: i, end });
    i = end;
  }
  return out;
}

function lexOne(src: string, i: number, c: number, of: boolean): [TokenKind, number] {
  if (isWhitespace(c)) {
    let j = i;
    for (let w = cpAt(src, j); w !== undefined && isWhitespace(w); w = cpAt(src, j)) j += cpLen(w);
    return ["Whitespace", j];
  }
  if (c === 0x22) return lexString(src, i);
  if (c === 0x23) {
    for (const name of ERROR_NAMES) {
      if (eqIgnoreAsciiCase(src.slice(i, i + name.length), name)) return ["Error", i + name.length];
    }
    return ["Invalid", i + 1];
  }
  if (c === 0x5b && of) {
    const end = scanBracket(src, i);
    return end === null ? ["Invalid", src.length] : ["OfRef", end];
  }
  if (isIdentStart(c)) {
    const end = scanErrLiteral(src, i);
    if (end !== null) return ["Error", end];
  }
  if (!of && (c === 0x24 || c === 0x27 || isIdentStart(c) || isAsciiDigit(c))) {
    const r = scanA1Ref(src, i);
    if (r) return [r[1] === "cell" ? "CellRef" : "RangeRef", r[0]];
  }
  if (c === 0x27) {
    // A quoted name that is not a sheet qualifier.
    const q = scanQuoted(src, i);
    return ["Invalid", q ? q[0] : src.length];
  }
  if (isAsciiDigit(c) || (c === 0x2e && isAsciiDigit(src.charCodeAt(i + 1)))) return ["Number", scanNumber(src, i)];
  if (isIdentStart(c)) {
    let j = i + cpLen(c);
    for (let w = cpAt(src, j); w !== undefined && isIdentChar(w); w = cpAt(src, j)) j += cpLen(w);
    let k = j;
    for (let w = cpAt(src, k); w !== undefined && isWhitespace(w); w = cpAt(src, k)) k += cpLen(w);
    return [src.charCodeAt(k) === 0x28 ? "Func" : "Ident", j];
  }
  const next = src.charCodeAt(i + 1);
  switch (String.fromCodePoint(c)) {
    case "+":
      return ["Plus", i + 1];
    case "-":
      return ["Minus", i + 1];
    case "*":
      return ["Star", i + 1];
    case "/":
      return ["Slash", i + 1];
    case "^":
      return ["Caret", i + 1];
    case "&":
      return ["Amp", i + 1];
    case "=":
      return ["Eq", i + 1];
    case "<":
      if (next === 0x3e) return ["Ne", i + 2];
      if (next === 0x3d) return ["Le", i + 2];
      return ["Lt", i + 1];
    case ">":
      return next === 0x3d ? ["Ge", i + 2] : ["Gt", i + 1];
    case "%":
      return ["Percent", i + 1];
    case ":":
      return ["Colon", i + 1];
    case "~":
      return ["Tilde", i + 1];
    case "!":
      return ["Bang", i + 1];
    case "(":
      return ["LParen", i + 1];
    case ")":
      return ["RParen", i + 1];
    case "{":
      return ["LBrace", i + 1];
    case "}":
      return ["RBrace", i + 1];
    case ",":
      return ["Comma", i + 1];
    case ";":
      return ["Semicolon", i + 1];
    case "|":
      return ["Pipe", i + 1];
    default:
      return ["Invalid", i + cpLen(c)];
  }
}

function lexString(src: string, i: number): [TokenKind, number] {
  let j = i + 1;
  for (;;) {
    const q = src.indexOf('"', j);
    if (q < 0) return ["Invalid", src.length];
    const after = q + 1;
    if (src.charCodeAt(after) === 0x22) j = after + 1;
    else return ["String", after];
  }
}

/** `Err:NNN` (prefix matched ASCII case-insensitively). */
function scanErrLiteral(src: string, i: number): number | null {
  if (src.length - i < 5 || !eqIgnoreAsciiCase(src.slice(i, i + 4), "err:")) return null;
  let end = i + 4;
  while (isAsciiDigit(src.charCodeAt(end))) end++;
  if (end === i + 4) return null;
  const c = cpAt(src, end);
  return c !== undefined && isIdentChar(c) ? null : end;
}

function scanNumber(s: string, i: number): number {
  let j = i;
  while (isAsciiDigit(s.charCodeAt(j))) j++;
  if (s.charCodeAt(j) === 0x2e) {
    j++;
    while (isAsciiDigit(s.charCodeAt(j))) j++;
  }
  const e = s.charCodeAt(j);
  if (e === 0x65 || e === 0x45) {
    let k = j + 1;
    const sign = s.charCodeAt(k);
    if (sign === 0x2b || sign === 0x2d) k++;
    if (isAsciiDigit(s.charCodeAt(k))) {
      while (isAsciiDigit(s.charCodeAt(k))) k++;
      j = k;
    }
  }
  return j;
}

// -- ExcelA1 extras (mirror lexer.rs scan_excel_3d / scan_excel_qualified) --

/** An unquoted Excel sheet name at `i` (ident start, then sheet chars or `.`); its end. */
function scanExcelSheetName(src: string, i: number): number | null {
  const first = cpAt(src, i);
  if (first === undefined || !isIdentStart(first)) return null;
  let j = i + cpLen(first);
  for (let c = cpAt(src, j); c !== undefined && (isSheetChar(c) || c === 0x2e); c = cpAt(src, j)) j += cpLen(c);
  return j;
}

/** True when a scanned reference part carries no sheet qualifier. */
function unqualified(part: RawRef): boolean {
  return part.start.sheet === null && (part.end === null || part.end.sheet === null);
}

/**
 * A sheet-qualified token the CalcA1 scanner does not cover: a dotted
 * sheet name (`Sales.EU!A1`) or a deleted reference (`Sheet1!#REF!`).
 */
function scanExcelQualified(src: string, i: number): [TokenKind, number] | null {
  let after: number;
  let dotted = false;
  if (src.charCodeAt(i) === 0x27) {
    const q = scanQuoted(src, i);
    if (!q) return null;
    after = q[0];
  } else {
    const end = scanExcelSheetName(src, i);
    if (end === null) return null;
    after = end;
    dotted = src.slice(i, end).includes(".");
  }
  if (src.charCodeAt(after) !== 0x21) return null;
  const start = after + 1;
  if (eqIgnoreAsciiCase(src.slice(start, start + 5), "#REF!")) return ["Error", start + 5];
  if (!dotted) return null;
  const r = scanA1Ref(src, start);
  if (!r || !unqualified(r[2])) return null;
  return [r[1] === "cell" ? "CellRef" : "RangeRef", r[0]];
}

/** A 3D reference `First:Last!A1` or `'First:Last'!A1:B2`; its end. */
function scanExcel3d(src: string, i: number): number | null {
  let afterNames: number;
  if (src.charCodeAt(i) === 0x27) {
    const q = scanQuoted(src, i);
    if (!q) return null;
    const k = q[1].indexOf(":");
    if (k <= 0 || k === q[1].length - 1 || q[1].indexOf(":", k + 1) >= 0) return null;
    afterNames = q[0];
  } else {
    const e1 = scanExcelSheetName(src, i);
    if (e1 === null || src.charCodeAt(e1) !== 0x3a) return null;
    const e2 = scanExcelSheetName(src, e1 + 1);
    if (e2 === null) return null;
    if (parseA1(src.slice(i, e1)) !== null || parseA1(src.slice(e1 + 1, e2)) !== null) return null;
    afterNames = e2;
  }
  if (src.charCodeAt(afterNames) !== 0x21) return null;
  const r = scanA1Ref(src, afterNames + 1);
  if (!r || !unqualified(r[2])) return null;
  return r[0];
}
