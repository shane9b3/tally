import type { FunctionInfo } from "@tally/bridge";
import { describe, expect, it } from "vitest";
import {
  applyCompletion,
  applyPoint,
  assistFor,
  assistKey,
  formulaHighlights,
  isPointMode,
  REF_COLORS_DARK,
  REF_COLORS_LIGHT,
  signatureAt,
  splitSignature,
} from "../src/formulaEdit";

const FNS: FunctionInfo[] = [
  { name: "SUM", category: "Mathematical", signature: "SUM(number1, [number2], …)", description: "Adds numbers.", params: [], example: "" },
  { name: "SUMIF", category: "Mathematical", signature: "SUMIF(range, criteria, [sum_range])", description: "Adds if.", params: [], example: "" },
  { name: "SUMPRODUCT", category: "Mathematical", signature: "SUMPRODUCT(array1, [array2], …)", description: "Products.", params: [], example: "" },
  { name: "IF", category: "Logical", signature: "IF(test, [then], [else])", description: "Chooses.", params: [], example: "" },
];

describe("formulaHighlights", () => {
  it("colours each distinct reference and outlines those on this sheet", () => {
    const h = formulaHighlights("=A1+B2:C3*$A$1+Other.D4", "Sheet1", "light");
    expect(h.spans.map((s) => [s.start, s.end, s.color])).toEqual([
      [1, 3, REF_COLORS_LIGHT[0]],
      [4, 9, REF_COLORS_LIGHT[1]],
      [10, 14, REF_COLORS_LIGHT[0]],
      [15, 23, REF_COLORS_LIGHT[2]],
    ]);
    expect(h.refs).toEqual([
      { range: { r0: 0, c0: 0, r1: 0, c1: 0 }, color: REF_COLORS_LIGHT[0] },
      { range: { r0: 1, c0: 1, r1: 2, c1: 2 }, color: REF_COLORS_LIGHT[1] },
    ]);
    expect(formulaHighlights("=A1", "Sheet1", "dark").spans[0]?.color).toBe(REF_COLORS_DARK[0]);
    expect(formulaHighlights("A1+B1", "Sheet1", "light").spans).toEqual([]);
  });
});

describe("point mode", () => {
  it("inserts, then replaces the picked reference while the caret stays at its end", () => {
    const r = (r0: number, c0: number, r1 = r0, c1 = c0) => ({ r0, c0, r1, c1 });
    expect(isPointMode("=A1+", 4, null)).toBe(true);
    expect(isPointMode("=A1+2", 5, null)).toBe(false);
    const first = applyPoint("=SUM(", 5, null, r(0, 1));
    expect(first).toEqual({ text: "=SUM(B1", caret: 7, span: { start: 5, end: 7 } });
    const second = applyPoint(first!.text, first!.caret, first!.span, r(0, 1, 4, 1));
    expect(second?.text).toBe("=SUM(B1:B5");
    expect(applyPoint("=1+2", 4, null, r(0, 0))).toBeNull();
  });
});

describe("autocomplete", () => {
  it("lists functions by prefix, shortest first, and completes with NAME(", () => {
    const a = assistFor("=su", 3, FNS, null);
    expect(a?.items.map((f) => f.name)).toEqual(["SUM", "SUMIF", "SUMPRODUCT"]);
    expect(applyCompletion("=su", a!.span, "SUM")).toEqual({ text: "=SUM(", caret: 5 });
    expect(applyCompletion("=su(A1)", { start: 1, end: 3 }, "SUM")).toEqual({ text: "=SUM(A1)", caret: 5 });
    expect(assistFor("=x", 2, FNS, null)).toBeNull();
    expect(assistFor("=su", 3, FNS, assistKey(a!))).toBeNull();
    expect(assistFor("=A1+i", 5, FNS, null)?.items[0]?.name).toBe("IF");
  });

  it("shows the signature with the current argument", () => {
    const s = signatureAt("=SUMIF(A1:A5,", 13, FNS);
    expect(s?.info.name).toBe("SUMIF");
    expect(s?.argIndex).toBe(1);
    expect(splitSignature("SUMIF(range, criteria, [sum_range])", 1)).toEqual(["SUMIF(range, ", "criteria", ", [sum_range])"]);
    expect(splitSignature("SUM(number1, [number2], …)", 5)).toEqual(["SUM(number1, ", "[number2]", ", …)"]);
    expect(signatureAt("=SUM(1)", 7, FNS)).toBeNull();
  });
});
