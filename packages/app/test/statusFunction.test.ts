/** The status bar's selection function and selection mode (parity U19). */
import type { SelectionStats } from "@tally/bridge";
import { describe, expect, it } from "vitest";
import { DEFAULT_PREFS, parsePrefs, serializePrefs } from "../src/options/prefs";
import { STATUS_FUNCTIONS, nextSelectionMode, selectionModeLabel, selectionModeOf, statusFunctionMenuLabel, statusFunctionText } from "../src/statusFunction";

const NONE: SelectionStats = { sum: null, count: 0, countNumbers: 0, average: null, min: null, max: null, formattedSum: null, formattedAverage: null, formattedMin: null, formattedMax: null };

const stats = (p: Partial<SelectionStats>): SelectionStats => ({ ...NONE, ...p });

describe("status bar function", () => {
  it("lists AOO's menu in AOO's order", () => {
    expect(STATUS_FUNCTIONS.map(statusFunctionMenuLabel)).toEqual(["Average", "CountA", "Count", "Maximum", "Minimum", "Sum", "None"]);
  });

  it("writes Name=value with the formatted value of the chosen function", () => {
    const s = stats({ sum: 11, count: 4, countNumbers: 3, average: 11 / 3, min: 2, max: 6, formattedSum: "11.00", formattedAverage: "3.67", formattedMin: "2.00", formattedMax: "6.00" });
    expect(statusFunctionText("sum", s)).toBe("Sum=11.00");
    expect(statusFunctionText("average", s)).toBe("Average=3.67");
    expect(statusFunctionText("max", s)).toBe("Max=6.00");
    expect(statusFunctionText("min", s)).toBe("Min=2.00");
    expect(statusFunctionText("count", s)).toBe("Count=3");
    expect(statusFunctionText("countA", s)).toBe("CountA=4");
    expect(statusFunctionText("none", s)).toBe("");
  });

  it("writes a result of 0 as 0 whatever the format", () => {
    const s = stats({ sum: 0, count: 2, countNumbers: 2, average: 0, min: -1, max: 1, formattedSum: "$0.00", formattedAverage: "$0.00", formattedMin: "-$1.00", formattedMax: "$1.00" });
    expect(statusFunctionText("sum", s)).toBe("Sum=0");
    expect(statusFunctionText("average", s)).toBe("Average=0");
    expect(statusFunctionText("min", s)).toBe("Min=-$1.00");
  });

  it("without numbers: Sum is 0, the counts are 0, the others have no value", () => {
    const s = stats({ count: 2 });
    expect(statusFunctionText("sum", s)).toBe("Sum=0");
    expect(statusFunctionText("count", s)).toBe("Count=0");
    expect(statusFunctionText("countA", s)).toBe("CountA=2");
    expect(statusFunctionText("average", s)).toBe("Average=");
    expect(statusFunctionText("max", s)).toBe("Max=");
    expect(statusFunctionText("min", s)).toBe("Min=");
  });

  it("with an error in the selection: the error's text, the counts stay", () => {
    const s = stats({ count: 5, countNumbers: 3, formattedSum: "#DIV/0!" });
    expect(statusFunctionText("sum", s)).toBe("Sum=#DIV/0!");
    expect(statusFunctionText("average", s)).toBe("Average=#DIV/0!");
    expect(statusFunctionText("count", s)).toBe("Count=3");
    expect(statusFunctionText("countA", s)).toBe("CountA=5");
  });

  it("is an app preference, Sum by default", () => {
    expect(DEFAULT_PREFS.statusFunction).toBe("sum");
    expect(parsePrefs({}).statusFunction).toBe("sum");
    expect(parsePrefs({ statusFunction: "bogus" }).statusFunction).toBe("sum");
    expect(parsePrefs(JSON.parse(serializePrefs({ ...DEFAULT_PREFS, statusFunction: "countA" }))).statusFunction).toBe("countA");
  });
});

describe("status bar selection mode", () => {
  it("steps STD → EXT → ADD → STD on a click", () => {
    expect(nextSelectionMode("std")).toBe("ext");
    expect(nextSelectionMode("ext")).toBe("add");
    expect(nextSelectionMode("add")).toBe("std");
  });

  it("reads the F8 / ⇧F8 flags", () => {
    expect(selectionModeOf(false, false)).toBe("std");
    expect(selectionModeOf(true, false)).toBe("ext");
    expect(selectionModeOf(false, true)).toBe("add");
    expect(["std", "ext", "add"].map((m) => selectionModeLabel(m as "std" | "ext" | "add"))).toEqual(["STD", "EXT", "ADD"]);
  });
});
