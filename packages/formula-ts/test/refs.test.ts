import { describe, expect, it } from "vitest";
import { colLetters, formatPickedRef, formatRef, MAX_COLS, MAX_ROWS, parseRef, referencesIn, tokenize } from "../src/index";

describe("referencesIn", () => {
  it("finds cells and ranges with spans", () => {
    const refs = referencesIn("=A1+SUM(B2:C3)");
    expect(refs.map((r) => [r.text, r.kind, r.start, r.end])).toEqual([
      ["A1", "cell", 1, 3],
      ["B2:C3", "range", 8, 13],
    ]);
    expect(refs[1]?.range).toEqual({ r0: 1, c0: 1, r1: 2, c1: 2 });
    expect(refs[0]?.sheet).toBeNull();
  });

  it("reads absolute markers", () => {
    const [r] = referencesIn("=$A1:B$2");
    expect(r?.absolute).toEqual({ c0: true, r0: false, c1: false, r1: true });
  });

  it("reads sheet qualifiers", () => {
    const [a, b] = referencesIn("=Sheet2.A1+'My Sheet'!B2:C3");
    expect(a?.sheet).toBe("Sheet2");
    expect(a?.range).toEqual({ r0: 0, c0: 0, r1: 0, c1: 0 });
    expect(b?.sheet).toBe("My Sheet");
    expect(b?.kind).toBe("range");
    expect(b?.text).toBe("'My Sheet'!B2:C3");
  });

  it("whole columns and rows", () => {
    const [cols, rows] = referencesIn("=SUM(A:B)+SUM(1:3)");
    expect(cols?.kind).toBe("cols");
    expect(cols?.range).toEqual({ r0: 0, c0: 0, r1: MAX_ROWS - 1, c1: 1 });
    expect(rows?.kind).toBe("rows");
    expect(rows?.range).toEqual({ r0: 0, c0: 0, r1: 2, c1: MAX_COLS - 1 });
  });

  it("ignores references inside strings and function names", () => {
    expect(referencesIn('="A1"&LOG10(2)').map((r) => r.text)).toEqual([]);
  });

  it("reads OpenFormula references", () => {
    const [r] = referencesIn("of:=SUM([.A1:.B2])");
    expect(r?.text).toBe("[.A1:.B2]");
    expect(r?.dialect).toBe("open-formula");
    expect(r?.range).toEqual({ r0: 0, c0: 0, r1: 1, c1: 1 });
    expect(referencesIn("of:=[.#REF!]")).toEqual([]);
  });

  it("uses UTF-16 spans after non-BMP characters", () => {
    const src = '="😀"&B7';
    const [r] = referencesIn(src);
    expect(r?.start).toBe(6);
    expect(src.slice(r?.start, r?.end)).toBe("B7");
  });

  it("parseRef rejects non-references", () => {
    expect(parseRef("A1")?.kind).toBe("cell");
    expect(parseRef("SUM")).toBeNull();
    expect(parseRef("XFE1")).toBeNull();
  });
});

describe("formatting", () => {
  it("colLetters", () => {
    expect([0, 25, 26, 701, 16383].map(colLetters)).toEqual(["A", "Z", "AA", "ZZ", "XFD"]);
  });

  it("formatRef", () => {
    expect(formatRef({ r0: 0, c0: 0, r1: 0, c1: 0 })).toBe("A1");
    expect(formatRef({ r0: 1, c0: 1, r1: 4, c1: 2 })).toBe("B2:C5");
    expect(formatRef({ r0: 0, c0: 2, r1: MAX_ROWS - 1, c1: 2 })).toBe("C:C");
    expect(formatRef({ r0: 2, c0: 0, r1: 3, c1: MAX_COLS - 1 })).toBe("3:4");
    expect(formatRef({ r0: 0, c0: 0, r1: 0, c1: 0 }, { sheet: "Sheet2" })).toBe("Sheet2.A1");
    expect(formatRef({ r0: 0, c0: 0, r1: 0, c1: 0 }, { sheet: "My 'Q' Sheet" })).toBe("'My ''Q'' Sheet'.A1");
    expect(formatRef({ r0: 0, c0: 0, r1: 0, c1: 0 }, { sheet: "A1" })).toBe("'A1'.A1");
  });

  it("formatted references lex back as one reference token", () => {
    const text = formatRef({ r0: 0, c0: 0, r1: 9, c1: 3 }, { sheet: "Q1 sales" });
    const toks = tokenize(`=${text}`).filter((t) => t.kind !== "Prefix");
    expect(toks.map((t) => t.kind)).toEqual(["RangeRef"]);
  });
});

describe("formatPickedRef (point mode across sheets)", () => {
  const a1 = { r0: 0, c0: 0, r1: 0, c1: 0 };
  const b2c4 = { r0: 1, c0: 1, r1: 3, c1: 2 };
  it("writes the formula's own sheet without a prefix", () => {
    expect(formatPickedRef(a1)).toBe("A1");
    expect(formatPickedRef(b2c4, { sheet: null })).toBe("B2:C4");
  });
  it("names another sheet, relative, quoting when needed (Calc A1)", () => {
    expect(formatPickedRef(a1, { sheet: "Sheet2" })).toBe("Sheet2.A1");
    expect(formatPickedRef(b2c4, { sheet: "Sheet2" })).toBe("Sheet2.B2:C4");
    expect(formatPickedRef(a1, { sheet: "My Sheet" })).toBe("'My Sheet'.A1");
    expect(formatPickedRef(a1, { sheet: "O'Brien 2" })).toBe("'O''Brien 2'.A1");
    expect(formatPickedRef({ r0: 0, c0: 2, r1: MAX_ROWS - 1, c1: 2 }, { sheet: "Data" })).toBe("Data.C:C");
  });
  it("writes a 3D reference with both sheets", () => {
    expect(formatPickedRef(a1, { sheet: "Sheet1", sheet2: "Sheet3" })).toBe("Sheet1.A1:Sheet3.A1");
    expect(formatPickedRef(b2c4, { sheet: "Sheet1", sheet2: "My Sheet" })).toBe("Sheet1.B2:'My Sheet'.C4");
    // The same sheet twice is a plain sheet reference.
    expect(formatPickedRef(a1, { sheet: "Sheet2", sheet2: "sheet2" })).toBe("Sheet2.A1");
  });
  it("Excel A1 syntax", () => {
    expect(formatPickedRef(a1, { sheet: "Sheet2", dialect: "excel-a1" })).toBe("Sheet2!A1");
    expect(formatPickedRef(b2c4, { sheet: "My Sheet", dialect: "excel-a1" })).toBe("'My Sheet'!B2:C4");
    expect(formatPickedRef(a1, { sheet: "Sheet1", sheet2: "Sheet3", dialect: "excel-a1" })).toBe("Sheet1:Sheet3!A1");
    expect(formatPickedRef(a1, { dialect: "excel-a1" })).toBe("A1");
    // R1C1-looking names are quoted in Excel syntax (printer.rs excel_needs_quotes), not in Calc.
    expect(formatPickedRef(a1, { sheet: "R1C1", dialect: "excel-a1" })).toBe("'R1C1'!A1");
    expect(formatPickedRef(a1, { sheet: "rc", dialect: "excel-a1" })).toBe("'rc'!A1");
    expect(formatPickedRef(a1, { sheet: "C", sheet2: "Sheet3", dialect: "excel-a1" })).toBe("'C:Sheet3'!A1");
    expect(formatPickedRef(a1, { sheet: "Rx", dialect: "excel-a1" })).toBe("Rx!A1");
    expect(formatPickedRef(a1, { sheet: "R1C1" })).toBe("R1C1.A1");
  });
  it("lexes as one reference", () => {
    for (const [text, sheet, sheet2] of [
      ["=SUM(Sheet1.A1:Sheet3.A1)", "Sheet1", "Sheet3"],
      ["=SUM('My Sheet'.B2:C4)", "My Sheet", null],
      ["=SUM(A1:Sheet3.A1)", null, "Sheet3"],
    ] as const) {
      const refs = referencesIn(text);
      expect(refs).toHaveLength(1);
      expect(refs[0]?.sheet).toBe(sheet);
      expect(refs[0]?.sheet2).toBe(sheet2);
    }
    for (const text of ["=SUM(Sheet1:Sheet3!A1)", "=SUM('My Sheet'!B2:C4)"]) {
      const kinds = tokenize(text, "excel-a1").map((t) => t.kind);
      expect(kinds).toEqual(["Prefix", "Func", "LParen", "RangeRef", "RParen"]);
    }
  });
});
