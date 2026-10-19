/**
 * Reference syntax scanning, ported from crates/tally-formula/src/refsyntax.rs
 * and `parse_a1` / `letters_to_col` from crates/tally-types/src/addr.rs.
 */
import { cpAt, cpLen, isAsciiAlpha, isAsciiDigit, isIdentChar, isIdentStart, isSheetChar } from "./chars";

export const MAX_ROWS = 1_048_576;
export const MAX_COLS = 16_384;

/** The coordinate part of a reference endpoint (0-based). */
export type Part =
  | { type: "cell"; row: number; col: number; absRow: boolean; absCol: boolean }
  | { type: "col"; col: number; abs: boolean }
  | { type: "row"; row: number; abs: boolean };

export interface RawSheet {
  name: string;
  abs: boolean;
  /** Written as `#REF!` (OpenFormula only). */
  deleted: boolean;
}

/** One endpoint; `part === null` means `#REF!`. */
export interface RawEndpoint {
  sheet: RawSheet | null;
  part: Part | null;
}

export interface RawRef {
  start: RawEndpoint;
  end: RawEndpoint | null;
}

const DOLLAR = 0x24;
const QUOTE = 0x27;

/** `A` → 0, `XFD` → 16383; `null` outside the grid or if not 1–3 ASCII letters. */
export function lettersToCol(s: string): number | null {
  if (s.length === 0 || s.length > 3) return null;
  let n = 0;
  for (let i = 0; i < s.length; i++) {
    const cp = s.charCodeAt(i);
    if (!isAsciiAlpha(cp)) return null;
    n = n * 26 + ((cp & ~32) - 0x41) + 1;
  }
  const col = n - 1;
  return col < MAX_COLS ? col : null;
}

/** Parses a whole-string A1 cell (`A1`, `$A$1`); `null` if malformed or outside the grid. */
export function parseA1(s: string): { row: number; col: number; absRow: boolean; absCol: boolean } | null {
  let i = 0;
  const absCol = s.charCodeAt(0) === DOLLAR;
  if (absCol) i++;
  const cs = i;
  while (i < s.length && isAsciiAlpha(s.charCodeAt(i))) i++;
  const col = lettersToCol(s.slice(cs, i));
  if (col === null) return null;
  const absRow = s.charCodeAt(i) === DOLLAR;
  if (absRow) i++;
  const digits = s.slice(i);
  if (digits.length === 0 || digits.length > 7 || !/^[0-9]+$/.test(digits)) return null;
  const row1 = Number(digits);
  if (row1 === 0 || row1 > MAX_ROWS) return null;
  return { row: row1 - 1, col, absRow, absCol };
}

function parseRow(digits: string): number | null {
  if (digits.length > 7) return null;
  const n = Number(digits);
  return n >= 1 && n <= MAX_ROWS ? n - 1 : null;
}

/** Scans a `'quoted'` name at `i` (which must be `'`): index after the closing quote and the unescaped name. */
export function scanQuoted(s: string, i: number): [number, string] | null {
  if (s.charCodeAt(i) !== QUOTE) return null;
  let j = i + 1;
  let name = "";
  for (;;) {
    const q = s.indexOf("'", j);
    if (q < 0) return null;
    name += s.slice(j, q);
    const after = q + 1;
    if (s.charCodeAt(after) === QUOTE) {
      name += "'";
      j = after + 1;
    } else {
      return [after, name];
    }
  }
}

/** Scans a coordinate part at `i`; the next character must not be an identifier char, `$` or `(`. */
export function scanPart(s: string, i: number): [number, Part] | null {
  let j = i;
  const absCol = s.charCodeAt(j) === DOLLAR;
  if (absCol) j++;
  const ls = j;
  while (j < s.length && isAsciiAlpha(s.charCodeAt(j))) j++;
  const letters = s.slice(ls, j);
  const absRow = s.charCodeAt(j) === DOLLAR;
  if (absRow) j++;
  const ds = j;
  while (j < s.length && isAsciiDigit(s.charCodeAt(j))) j++;
  const digits = s.slice(ds, j);
  const next = cpAt(s, j);
  if (next !== undefined && (isIdentChar(next) || next === DOLLAR || next === 0x28)) return null;
  let part: Part;
  if (letters !== "" && digits !== "") {
    const row = parseRow(digits);
    const col = lettersToCol(letters);
    if (row === null || col === null) return null;
    part = { type: "cell", row, col, absRow, absCol };
  } else if (letters !== "" && !absRow) {
    const col = lettersToCol(letters);
    if (col === null) return null;
    part = { type: "col", col, abs: absCol };
  } else if (letters === "" && digits !== "" && !(absCol && absRow)) {
    const row = parseRow(digits);
    if (row === null) return null;
    part = { type: "row", row, abs: absCol || absRow };
  } else {
    return null;
  }
  return [j, part];
}

/** Scans a CalcA1 sheet qualifier (`$Sheet1.`, `'My Sheet'!`) at `i`; returns the index after the separator. */
function scanSheet(s: string, i: number): [number, RawSheet] | null {
  let j = i;
  const abs = s.charCodeAt(j) === DOLLAR;
  if (abs) j++;
  let end: number;
  let name: string;
  let quoted: boolean;
  if (s.charCodeAt(j) === QUOTE) {
    const q = scanQuoted(s, j);
    if (!q) return null;
    [end, name] = q;
    quoted = true;
  } else {
    const first = cpAt(s, j);
    if (first === undefined || !isIdentStart(first)) return null;
    let e = j + cpLen(first);
    for (let c = cpAt(s, e); c !== undefined && isSheetChar(c); c = cpAt(s, e)) e += cpLen(c);
    end = e;
    name = s.slice(j, e);
    quoted = false;
  }
  const sep = s.charCodeAt(end);
  // `A1!B1` is an intersection, not a sheet named "A1".
  if (sep === 0x2e || (sep === 0x21 && (quoted || parseA1(name) === null))) {
    return [end + 1, { name, abs, deleted: false }];
  }
  return null;
}

/** Scans one CalcA1 endpoint: an optional sheet qualifier plus a part. */
export function scanEndpoint(s: string, i: number): [number, RawEndpoint] | null {
  const sh = scanSheet(s, i);
  if (sh) {
    const p = scanPart(s, sh[0]);
    if (p) return [p[0], { sheet: sh[1], part: p[1] }];
  }
  const p = scanPart(s, i);
  return p ? [p[0], { sheet: null, part: p[1] }] : null;
}

function compatible(a: Part | null, b: Part | null): boolean {
  return a !== null && b !== null && a.type === b.type;
}

/** Scans a CalcA1 reference token at `i`: a cell or an adjacent range. */
export function scanA1Ref(s: string, i: number): [number, "cell" | "range", RawRef] | null {
  const e = scanEndpoint(s, i);
  if (!e) return null;
  const [e1, ep1] = e;
  if (s.charCodeAt(e1) === 0x3a) {
    const f = scanEndpoint(s, e1 + 1);
    if (f && compatible(ep1.part, f[1].part)) return [f[0], "range", { start: ep1, end: f[1] }];
  }
  return ep1.part?.type === "cell" ? [e1, "cell", { start: ep1, end: null }] : null;
}

/** Parses the full text of a CalcA1 `CellRef` / `RangeRef` token. */
export function parseA1Ref(text: string): RawRef | null {
  const r = scanA1Ref(text, 0);
  return r && r[0] === text.length ? r[2] : null;
}

/** Scans an OpenFormula `[...]` reference at `i`, honouring quoted sheet names; index after `]`. */
export function scanBracket(s: string, i: number): number | null {
  if (s.charCodeAt(i) !== 0x5b) return null;
  let j = i + 1;
  while (j < s.length) {
    const b = s.charCodeAt(j);
    if (b === 0x5d) return j + 1;
    if (b === QUOTE) {
      const q = scanQuoted(s, j);
      if (!q) return null;
      j = q[0];
    } else j++;
  }
  return null;
}

function ofEndpoint(s: string, i: number): [number, RawEndpoint] | null {
  let j = i;
  const abs = s.charCodeAt(j) === DOLLAR;
  if (abs) j++;
  let sheet: RawSheet | null = null;
  if (s.charCodeAt(j) === 0x2e) {
    if (abs) return null;
    j++;
  } else {
    let e: number;
    let name: string;
    let deleted = false;
    if (s.charCodeAt(j) === QUOTE) {
      const q = scanQuoted(s, j);
      if (!q) return null;
      [e, name] = q;
    } else if (s.startsWith("#REF!", j)) {
      e = j + 5;
      name = "";
      deleted = true;
    } else {
      const m = /[.:\]']/.exec(s.slice(j));
      if (!m || m.index === 0) return null;
      e = j + m.index;
      name = s.slice(j, e);
    }
    if (s.charCodeAt(e) !== 0x2e) return null;
    j = e + 1;
    sheet = { name, abs, deleted };
  }
  if (s.startsWith("#REF!", j)) return [j + 5, { sheet, part: null }];
  const p = scanPart(s, j);
  return p ? [p[0], { sheet, part: p[1] }] : null;
}

/** Parses the inside of an OpenFormula reference (no brackets): `.A1`, `$Sheet1.A1:.B2`, `.A:.A`. */
export function parseOfRef(inner: string): RawRef | null {
  const a = ofEndpoint(inner, 0);
  if (!a) return null;
  const [e1, start] = a;
  if (e1 === inner.length) return { start, end: null };
  if (inner.charCodeAt(e1) !== 0x3a) return null;
  const b = ofEndpoint(inner, e1 + 1);
  if (!b || b[0] !== inner.length) return null;
  const end = b[1];
  if (start.part && end.part && !compatible(start.part, end.part)) return null;
  return { start, end };
}

/** Whether a sheet name must be quoted when printed. */
export function sheetNeedsQuotes(name: string): boolean {
  const first = cpAt(name, 0);
  if (first === undefined || !isIdentStart(first)) return true;
  for (let i = cpLen(first); i < name.length; ) {
    const c = cpAt(name, i) ?? 0;
    if (!isSheetChar(c)) return true;
    i += cpLen(c);
  }
  return parseA1(name) !== null;
}
