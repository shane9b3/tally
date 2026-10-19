/**
 * Cell and range references in formula text, for the editor (coloured
 * references, point mode, range outlines on the grid).
 */
import { tokenize, usesOpenFormula, type Dialect } from "./lexer";
import { MAX_COLS, MAX_ROWS, parseA1Ref, parseOfRef, sheetNeedsQuotes, type Part, type RawRef } from "./refsyntax";

export { MAX_COLS, MAX_ROWS };

/** 0-based inclusive, normalised (r0 ≤ r1, c0 ≤ c1). */
export interface RefRange {
  r0: number;
  c0: number;
  r1: number;
  c1: number;
}

export interface RefInfo {
  /** UTF-16 span of the token in the source. */
  start: number;
  end: number;
  text: string;
  /** `A1` / `A1:B2` / `A:C` / `1:3`. */
  kind: "cell" | "range" | "cols" | "rows";
  /** Unquoted, unescaped sheet name of the first endpoint, or `null`. */
  sheet: string | null;
  /** Sheet of the second endpoint if it names one, else `null`. */
  sheet2: string | null;
  /** Whole columns span every row; whole rows span every column. */
  range: RefRange;
  absolute: { c0: boolean; r0: boolean; c1: boolean; r1: boolean };
  dialect: Dialect;
}

export type ParsedRef = Omit<RefInfo, "start" | "end" | "text" | "dialect">;

/** Row/column bounds and absolute flags of one endpoint part. */
function bounds(p: Part): { r0: number; r1: number; c0: number; c1: number; absR: boolean; absC: boolean } {
  switch (p.type) {
    case "cell":
      return { r0: p.row, r1: p.row, c0: p.col, c1: p.col, absR: p.absRow, absC: p.absCol };
    case "col":
      return { r0: 0, r1: MAX_ROWS - 1, c0: p.col, c1: p.col, absR: false, absC: p.abs };
    case "row":
      return { r0: p.row, r1: p.row, c0: 0, c1: MAX_COLS - 1, absR: p.abs, absC: false };
  }
}

function fromRaw(raw: RawRef): ParsedRef | null {
  const a = raw.start.part;
  const b = raw.end ? raw.end.part : a;
  if (!a || !b) return null; // #REF!
  if (raw.start.sheet?.deleted || raw.end?.sheet?.deleted) return null;
  const x = bounds(a);
  const y = bounds(b);
  const kind: ParsedRef["kind"] = a.type === "col" ? "cols" : a.type === "row" ? "rows" : raw.end ? "range" : "cell";
  return {
    kind,
    sheet: raw.start.sheet?.name ?? null,
    sheet2: raw.end?.sheet?.name ?? null,
    range: {
      r0: Math.min(x.r0, y.r0),
      c0: Math.min(x.c0, y.c0),
      r1: Math.max(x.r1, y.r1),
      c1: Math.max(x.c1, y.c1),
    },
    absolute: { c0: x.absC, r0: x.absR, c1: y.absC, r1: y.absR },
  };
}

/** Parses the text of a CalcA1 `CellRef` / `RangeRef` token (`$Sheet1.A1:B2`). */
export function parseRef(text: string): ParsedRef | null {
  const raw = parseA1Ref(text);
  return raw ? fromRaw(raw) : null;
}

/** Every cell/range reference in `src`, in order (`#REF!` references are skipped). */
export function referencesIn(src: string, dialect: Dialect = "calc-a1"): RefInfo[] {
  const of = usesOpenFormula(src, dialect);
  const out: RefInfo[] = [];
  for (const t of tokenize(src, dialect)) {
    let parsed: ParsedRef | null = null;
    if (t.kind === "CellRef" || t.kind === "RangeRef") parsed = parseRef(t.text);
    else if (t.kind === "OfRef") {
      const raw = parseOfRef(t.text.slice(1, -1));
      parsed = raw ? fromRaw(raw) : null;
    }
    if (parsed) out.push({ start: t.start, end: t.end, text: t.text, ...parsed, dialect: of ? "open-formula" : "calc-a1" });
  }
  return out;
}

/** 0-based column index to letters (0 → `A`, 16383 → `XFD`). */
export function colLetters(c: number): string {
  let n = c + 1;
  let s = "";
  while (n > 0) {
    const rem = (n - 1) % 26;
    s = String.fromCharCode(65 + rem) + s;
    n = Math.floor((n - 1) / 26);
  }
  return s;
}

/** A sheet name as written in CalcA1, quoted and escaped when needed. */
export function quoteSheet(name: string): string {
  return sheetNeedsQuotes(name) ? `'${name.replace(/'/g, "''")}'` : name;
}

/**
 * CalcA1 text for a range: `A1`, `A1:B5`, `A:C` (whole columns), `1:3`
 * (whole rows), prefixed by `Sheet.` when `sheet` is given.
 */
export function formatRef(range: RefRange, opts: { sheet?: string | null } = {}): string {
  const prefix = opts.sheet ? `${quoteSheet(opts.sheet)}.` : "";
  const { r0, c0, r1, c1 } = range;
  let body: string;
  if (r0 === 0 && r1 === MAX_ROWS - 1 && !(c0 === 0 && c1 === MAX_COLS - 1)) body = `${colLetters(c0)}:${colLetters(c1)}`;
  else if (c0 === 0 && c1 === MAX_COLS - 1) body = `${r0 + 1}:${r1 + 1}`;
  else {
    const a = `${colLetters(c0)}${r0 + 1}`;
    body = r0 === r1 && c0 === c1 ? a : `${a}:${colLetters(c1)}${r1 + 1}`;
  }
  return prefix + body;
}
