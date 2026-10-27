import type { CellView } from "@tally/bridge";
import { describe, expect, it } from "vitest";
import { pasteBlocks, planSeries, toSourceValue, type SourceValue } from "../src/dev/mockFill";
import { extensionOf, fillDownEnd, fillRange } from "../src/fill";

const num = (n: number, date = false): SourceValue => ({ kind: "number", num: n, date });
const txt = (t: string): SourceValue => ({ kind: "text", text: t });
const run = (vals: SourceValue[], offsets: number[]): (string | null)[] => {
  const g = planSeries(vals);
  return g ? offsets.map(g) : offsets.map(() => null);
};

describe("planSeries", () => {
  it("numbers: one cell steps by 1 (backwards when filling up)", () => {
    expect(run([num(1)], [1, 2, 3])).toEqual(["2", "3", "4"]);
    expect(run([num(5)], [-1, -2])).toEqual(["4", "3"]);
  });

  it("numbers: two or more use the difference", () => {
    expect(run([num(2), num(4)], [2, 3])).toEqual(["6", "8"]);
    expect(run([num(0.1), num(0.2)], [2])).toEqual(["0.3"]);
    expect(run([num(10), num(7), num(4)], [3, -1])).toEqual(["1", "13"]);
    expect(planSeries([num(1), num(2), num(4)])).toBeNull();
  });

  it("date-formatted numbers: days and months", () => {
    expect(run([num(46314, true)], [1, 2])).toEqual(["46315", "46316"]);
    // 2026-01-31, 2026-02-28 do not share a day → day step (28).
    // 2026-01-15 → 2026-02-15: month step.
    const jan15 = 46037; // 2026-01-15
    const feb15 = 46068; // 2026-02-15
    expect(run([num(jan15, true), num(feb15, true)], [2])).toEqual(["46096"]); // 2026-03-15
  });

  it("weekday and month names keep their case", () => {
    expect(run([txt("Monday")], [1, 6])).toEqual(["Tuesday", "Sunday"]);
    expect(run([txt("JAN")], [1, 12])).toEqual(["FEB", "JAN"]);
    expect(run([txt("mar"), txt("may")], [2])).toEqual(["jul"]);
    expect(run([txt("Dec")], [-1])).toEqual(["Nov"]);
  });

  it("text dates, numbered text and ordinals", () => {
    expect(run([txt("10/19/2026")], [1, 13])).toEqual(["10/20/2026", "11/01/2026"]);
    expect(run([txt("2026-12-31")], [1])).toEqual(["2027-01-01"]);
    expect(run([txt("Item 1")], [1, 2])).toEqual(["Item 2", "Item 3"]);
    expect(run([txt("Q1"), txt("Q3")], [2])).toEqual(["Q5"]);
    expect(run([txt("007")], [1])).toEqual(["008"]);
    expect(run([txt("1st")], [1, 2, 10])).toEqual(["2nd", "3rd", "11th"]);
  });

  it("ordinals and numbered text match the engine (T16)", () => {
    expect(run([txt("1st")], [3, 10, 11, 12, 20, 21, 22, 100, 110])).toEqual(["4th", "11th", "12th", "13th", "21st", "22nd", "23rd", "101st", "111th"]);
    // Filling up counts down.
    expect(run([txt("Item 5")], [-1, -2])).toEqual(["Item 4", "Item 3"]);
    expect(run([txt("3rd")], [-1, -2])).toEqual(["2nd", "1st"]);
    expect(run([txt("Item 10")], [-1])).toEqual(["Item 9"]);
    expect(run([txt("1th")], [1])).toEqual(["2th"]);
  });

  it("uses the Options sort lists after the built-in names, abbreviated first (L6)", () => {
    const lists = [["North", "East", "South", "West"]];
    const g = planSeries([txt("South")], lists);
    expect(g ? [1, 2, -2].map(g) : null).toEqual(["West", "North", "North"]);
    expect(planSeries([txt("South")])).toBeNull();
    expect(run([txt("May")], [1])).toEqual(["Jun"]);
  });

  it("copies everything else", () => {
    expect(planSeries([txt("Paid")])).toBeNull();
    expect(planSeries([txt("A1B")])).toBeNull();
    expect(planSeries([num(1), txt("x")])).toBeNull();
    expect(planSeries([{ kind: "other" }])).toBeNull();
    expect(planSeries([num(1), { kind: "empty" }])).toBeNull();
  });
});

describe("toSourceValue", () => {
  const cell = (p: Partial<CellView>): CellView => ({ r: 0, c: 0, text: "", kind: "empty", num: null, style: 0, color: null, flags: 0, ...p });
  it("treats formulas as patterns and detects date formats", () => {
    expect(toSourceValue(cell({ kind: "number", num: 3, text: "3", flags: 1 }), "General")).toEqual({ kind: "other" });
    expect(toSourceValue(cell({ kind: "number", num: 3, text: "3" }), "MM/DD/YYYY")).toEqual({ kind: "number", num: 3, date: true });
    expect(toSourceValue(undefined, "General")).toEqual({ kind: "empty" });
  });
});

describe("pasteBlocks / extensionOf", () => {
  const src = { r0: 2, c0: 0, r1: 3, c1: 1 };
  it("tiles down with a partial last block", () => {
    const ext = extensionOf(src, { ...src, r1: 8 }, "down");
    expect(ext).toEqual({ r0: 4, c0: 0, r1: 8, c1: 1 });
    expect(pasteBlocks(src, ext, "down", 2, 5)).toEqual([
      { src, target: { r0: 4, c0: 0, r1: 5, c1: 1 } },
      { src, target: { r0: 6, c0: 0, r1: 7, c1: 1 } },
      { src: { ...src, r1: 2 }, target: { r0: 8, c0: 0, r1: 8, c1: 1 } },
    ]);
  });
  it("tiles up from the source, the partial block taking the last rows", () => {
    const ext = extensionOf(src, { ...src, r0: 0 }, "up");
    expect(ext).toEqual({ r0: 0, c0: 0, r1: 1, c1: 1 });
    expect(pasteBlocks({ r0: 2, c0: 0, r1: 4, c1: 0 }, { r0: 0, c0: 0, r1: 1, c1: 0 }, "up", 3, 2)).toEqual([
      { src: { r0: 3, c0: 0, r1: 4, c1: 0 }, target: { r0: 0, c0: 0, r1: 1, c1: 0 } },
    ]);
  });
});

describe("fillDownEnd", () => {
  it("follows the adjacent column's data and stops at data below", () => {
    const filled = new Set(["0,0", "1,0", "2,0", "3,0", "4,0", "0,1"]);
    const get = (r: number, c: number) =>
      filled.has(`${r},${c}`) ? ({ r, c, text: "x", kind: "text", num: null, style: 0, color: null, flags: 0 } as CellView) : undefined;
    expect(fillDownEnd({ r0: 0, c0: 1, r1: 0, c1: 1 }, get, 1000)).toBe(4);
    filled.add("3,1");
    expect(fillDownEnd({ r0: 0, c0: 1, r1: 0, c1: 1 }, get, 1000)).toBe(2);
    expect(fillDownEnd({ r0: 0, c0: 5, r1: 0, c1: 5 }, get, 1000)).toBeNull();
  });
});

describe("fillRange (engine op)", () => {
  it("sends one fillRange op with mode auto", () => {
    const source = { r0: 0, c0: 0, r1: 1, c1: 0 };
    const target = { r0: 0, c0: 0, r1: 9, c1: 0 };
    expect(fillRange({ sheet: 3 }, source, target)).toEqual([{ type: "fillRange", sheet: 3, source, target, mode: "auto" }]);
    // ⌘D / ⌘R copy (AOO FillSimple).
    expect(fillRange({ sheet: 3 }, source, target, "copy")).toEqual([{ type: "fillRange", sheet: 3, source, target, mode: "copy" }]);
    expect(fillRange({ sheet: 3 }, source, source)).toEqual([]);
  });
});
