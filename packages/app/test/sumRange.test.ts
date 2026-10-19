import type { CellView } from "@tally/bridge";
import { describe, expect, it } from "vitest";
import { detectSumRange, sumFormula, sumScanRanges } from "../src/sumRange";

function sheet(cells: Record<string, number | string>) {
  return (r: number, c: number): CellView | undefined => {
    const v = cells[`${r},${c}`];
    if (v === undefined) return undefined;
    const num = typeof v === "number";
    return { r, c, text: String(v), kind: num ? "number" : "text", num: num ? v : null, style: 0, color: null, flags: 0 };
  };
}

describe("detectSumRange", () => {
  it("prefers the numeric block above", () => {
    const get = sheet({ "0,0": "Qty", "1,0": 1, "2,0": 2, "3,0": 3, "4,1": 9 });
    expect(detectSumRange({ r: 4, c: 0 }, get)).toEqual({ r0: 1, c0: 0, r1: 3, c1: 0 });
    expect(sumFormula(detectSumRange({ r: 4, c: 0 }, get))).toBe("=SUM(A2:A4)");
  });
  it("falls back to the block on the left", () => {
    const get = sheet({ "5,0": "Total", "5,1": 10, "5,2": 20 });
    expect(detectSumRange({ r: 5, c: 3 }, get)).toEqual({ r0: 5, c0: 1, r1: 5, c1: 2 });
  });
  it("stops at a gap and returns null when nothing is numeric", () => {
    const get = sheet({ "0,0": 1, "2,0": 5 });
    expect(detectSumRange({ r: 3, c: 0 }, get)).toEqual({ r0: 2, c0: 0, r1: 2, c1: 0 });
    expect(detectSumRange({ r: 0, c: 0 }, sheet({}))).toBeNull();
    expect(sumFormula(null)).toBe("=SUM()");
  });
  it("fetches only valid ranges", () => {
    expect(sumScanRanges({ r: 0, c: 0 })).toEqual([]);
    expect(sumScanRanges({ r: 3, c: 2 })).toEqual([
      { r0: 0, c0: 2, r1: 2, c1: 2 },
      { r0: 3, c0: 0, r1: 3, c1: 1 },
    ]);
  });
});
