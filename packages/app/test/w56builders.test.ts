/**
 * W5-6B: the data-tool DTO builders, the text-to-columns preview and the
 * client-side outline model.
 */
import { describe, expect, it } from "vitest";
import {
  BUILTIN_SORT_LISTS,
  consolidateOp,
  copySheetName,
  DEFAULT_FILTER_OPTIONS,
  deleteContentsOp,
  emptyFilterRows,
  emptySubtotalTabs,
  fillSeriesOp,
  filterConditions,
  filterError,
  groupAxisFor,
  hiddenContentCount,
  isValidDbRangeName,
  mergedContent,
  multipleOperationsOps,
  parseSheetRange,
  parseSortList,
  previewSeries,
  removeDuplicatesOp,
  sortWithOptionsOp,
  standardFilterOp,
  STD_FILTER_OPS,
  subtotalGroups,
  SUBTOTAL_FNS,
  subtotalsOp,
} from "../src/data/builders";
import { DEFAULT_SEPARATORS, delimiterString, guessSeparator, previewColumns, previewSplit, splitLine, toCsvOptions } from "../src/data/textToColumns";
import { addGroup, applyOutlineOp, autoOutlineGroups, emptyOutlineModel, hiddenByOutline, removeGroup, showLevel } from "../src/data/outlineModel";

const sheets = (name: string) => (name.toLowerCase() === "sheet2" ? 2 : name.toLowerCase() === "sheet1" ? 1 : null);

describe("standard filter", () => {
  it("lists the 16 AOO operators and stops at the first unused row", () => {
    expect(STD_FILTER_OPS.map((o) => o.op)).toEqual(["eq", "lt", "gt", "le", "ge", "ne", "largest", "smallest", "largestPct", "smallestPct", "contains", "notContains", "beginsWith", "notBeginsWith", "endsWith", "notEndsWith"]);
    const rows = emptyFilterRows(2);
    rows[0] = { connector: "and", col: 2, op: "gt", value: "50" };
    rows[1] = { connector: "or", col: 3, op: "contains", value: "x" };
    rows[3] = { connector: "and", col: 4, op: "eq", value: "ignored" };
    const conds = filterConditions(rows);
    expect(conds).toEqual([
      { connector: "and", col: 2, op: "gt", value: "50" },
      { connector: "or", col: 3, op: "contains", value: "x" },
    ]);
    expect(filterError(conds, DEFAULT_FILTER_OPTIONS)).toBeNull();
    expect(filterError([], DEFAULT_FILTER_OPTIONS)).toMatch(/first condition/);
    expect(filterError([{ connector: "and", col: 1, op: "largest", value: "x" }], DEFAULT_FILTER_OPTIONS)).toMatch(/positive number/);
    expect(filterError([{ connector: "and", col: 1, op: "eq", value: "(" }], { ...DEFAULT_FILTER_OPTIONS, regex: true })).toMatch(/regular expression/);
    const op = standardFilterOp(1, { r0: 0, c0: 0, r1: 9, c1: 3 }, conds, { ...DEFAULT_FILTER_OPTIONS, copyTo: { sheet: 2, r: 0, c: 0 } });
    expect(op).toMatchObject({ type: "standardFilter", sheet: 1, options: { copyTo: { sheet: 2, r: 0, c: 0 }, noDuplicates: false } });
  });
});

describe("subtotals and consolidate", () => {
  it("builds at most three groups with the SUBTOTAL function numbers", () => {
    expect(SUBTOTAL_FNS.map((f) => f.n)).toEqual([9, 3, 1, 4, 5, 6, 2, 7, 8, 10, 11]);
    const tabs = emptySubtotalTabs();
    tabs[0] = { groupBy: 0, columns: [3, 1], function: "sum" };
    tabs[2] = { groupBy: 2, columns: [], function: "max" };
    expect(subtotalGroups(tabs)).toEqual([{ groupBy: 0, columns: [1, 3], function: "sum" }]);
    expect(subtotalsOp(1, { r0: 0, c0: 0, r1: 5, c1: 3 }, subtotalGroups(tabs), { pageBreaks: false, caseSensitive: false, presort: true, ascending: true, includeFormats: true })).toMatchObject({ type: "subtotals", groups: [{ groupBy: 0 }] });
  });
  it("parses sheet ranges and refuses bad consolidate input", () => {
    expect(parseSheetRange("Sheet2.A1:B5", sheets, 1)).toEqual({ sheet: 2, range: { r0: 0, c0: 0, r1: 4, c1: 1 } });
    expect(parseSheetRange("$A$1:$B$5", sheets, 1)).toEqual({ sheet: 1, range: { r0: 0, c0: 0, r1: 4, c1: 1 } });
    expect(parseSheetRange("'Sheet 3'.A1", sheets, 1)).toBeNull();
    expect(parseSheetRange("nope", sheets, 1)).toBeNull();
    expect(consolidateOp({ function: "sum", sources: [], dest: "A1", byRowLabels: false, byColLabels: false, link: false }, sheets, 1)).toMatch(/at least one/);
    expect(consolidateOp({ function: "sum", sources: ["A1:B2", "zzz"], dest: "A1", byRowLabels: false, byColLabels: false, link: false }, sheets, 1)).toMatch(/zzz/);
    const op = consolidateOp({ function: "average", sources: ["A1:B2", "Sheet2.A1:B2"], dest: "Sheet2.D1", byRowLabels: true, byColLabels: false, link: true }, sheets, 1);
    expect(op).toMatchObject({ type: "consolidate", function: "average", dest: { sheet: 2, r: 0, c: 3 }, link: true, sources: [{ sheet: 1 }, { sheet: 2 }] });
  });
});

describe("text to columns", () => {
  it("splits with quotes, merged delimiters and fixed widths", () => {
    const st = { ...DEFAULT_SEPARATORS, tab: false, semicolon: false, comma: true };
    expect(splitLine('a,"b,c",d', st)).toEqual(["a", "b,c", "d"]);
    expect(splitLine("a,,b", { ...st, mergeDelimiters: true })).toEqual(["a", "b"]);
    expect(splitLine('x,"say ""hi"""', st)).toEqual(["x", 'say "hi"']);
    expect(splitLine("abcdefgh", { ...st, fixedWidth: true, widths: [3, 5] })).toEqual(["abc", "de", "fgh"]);
    expect(delimiterString({ ...st, tab: true, other: true, otherChar: "|" })).toBe("\t,|");
    expect(previewSplit(["a;b", "c"], DEFAULT_SEPARATORS)).toEqual([
      ["a", "b"],
      ["c", ""],
    ]);
    expect(previewColumns(["a;b;c", "d"], DEFAULT_SEPARATORS)).toBe(3);
    expect(guessSeparator(["a,b,c", "d,e,f"])).toMatchObject({ comma: true, semicolon: false });
    expect(guessSeparator(["a\tb", "c\td"])).toMatchObject({ tab: true });
    const opts = toCsvOptions({ ...st, columnTypes: ["text", "standard"] });
    expect(opts).toMatchObject({ delimiters: ",", quote: '"', fixedWidths: null, columnTypes: ["text", "standard"] });
    expect(toCsvOptions({ ...st, fixedWidth: true, widths: [5, 2] }).fixedWidths).toEqual([2, 5]);
  });
});

describe("duplicates, names, fill series, delete contents", () => {
  it("builds the ops", () => {
    expect(removeDuplicatesOp(1, { r0: 0, c0: 0, r1: 3, c1: 2 }, [2, 0], true)).toMatchObject({ type: "removeDuplicates", cols: [0, 2], hasHeader: true });
    expect(isValidDbRangeName("Sales_2027")).toBe(true);
    expect(isValidDbRangeName("A1")).toBe(false);
    expect(isValidDbRangeName("1x")).toBe(false);
    const series = fillSeriesOp(1, { r0: 0, c0: 0, r1: 4, c1: 0 }, { direction: "down", kind: "linear", unit: "day", start: "1", end: "", increment: "2" });
    expect(series).toMatchObject({ type: "fillSeries", direction: "down", kind: "linear", unit: null, start: "1", end: null, increment: "2" });
    expect(fillSeriesOp(1, { r0: 0, c0: 0, r1: 4, c1: 0 }, { direction: "right", kind: "date", unit: "month", start: "", end: "", increment: "1" })).toMatchObject({ unit: "month", start: null });
    expect(previewSeries({ direction: "down", kind: "linear", unit: "day", start: "1", end: "", increment: "2" }, 5, "")).toEqual(["1", "3", "5", "7", "9"]);
    expect(previewSeries({ direction: "down", kind: "growth", unit: "day", start: "", end: "20", increment: "2" }, 5, "2")).toEqual(["2", "4", "8", "16"]);
    expect(deleteContentsOp(1, [{ r0: 0, c0: 0, r1: 1, c1: 1 }], { text: true, numbers: false, dates: false, formulas: true, notes: false, formats: false, objects: false })).toMatchObject({ type: "deleteContents", what: { text: true, numbers: false } });
  });
});

describe("sort options, multiple operations, sheets", () => {
  it("sort with options refuses a bad copy-to cell and uses a custom list", () => {
    const base = { keys: [{ col: 0, ascending: true }], hasHeader: true, caseSensitive: false, leftToRight: false, copyTo: "", customList: 2, includeFormats: true, natural: false };
    const op = sortWithOptionsOp(1, { r0: 0, c0: 0, r1: 5, c1: 2 }, base, BUILTIN_SORT_LISTS, () => null);
    expect(op).toMatchObject({ type: "sortRange", options: { customOrder: ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"], leftToRight: false, copyTo: null } });
    expect(sortWithOptionsOp(1, { r0: 0, c0: 0, r1: 5, c1: 2 }, { ...base, copyTo: "zz" }, [], () => null)).toMatch(/not a cell/);
    expect(parseSortList(" Q1\nQ2\n\n Q3 ")).toEqual(["Q1", "Q2", "Q3"]);
  });
  it("writes MULTIPLE.OPERATIONS formulas for one- and two-variable tables", () => {
    const cell = (s: string) => (s === "B1" ? { r: 0, c: 1 } : s === "B2" ? { r: 1, c: 1 } : null);
    const one = multipleOperationsOps(1, { r0: 3, c0: 0, r1: 5, c1: 0 }, { formulas: "B4", rowInput: "", colInput: "B1" }, cell);
    expect(one).toEqual([
      { type: "setCellInput", sheet: 1, r: 3, c: 1, input: "=MULTIPLE.OPERATIONS($B$4;$B$1;A4)" },
      { type: "setCellInput", sheet: 1, r: 4, c: 1, input: "=MULTIPLE.OPERATIONS($B$4;$B$1;A5)" },
      { type: "setCellInput", sheet: 1, r: 5, c: 1, input: "=MULTIPLE.OPERATIONS($B$4;$B$1;A6)" },
    ]);
    const two = multipleOperationsOps(1, { r0: 3, c0: 0, r1: 4, c1: 2 }, { formulas: "B4", rowInput: "B1", colInput: "B2" }, cell);
    expect(two).toEqual([
      { type: "setCellInput", sheet: 1, r: 4, c: 1, input: "=MULTIPLE.OPERATIONS($B$4;$B$2;A5;$B$1;B4)" },
      { type: "setCellInput", sheet: 1, r: 4, c: 2, input: "=MULTIPLE.OPERATIONS($B$4;$B$2;A5;$B$1;C4)" },
    ]);
    expect(multipleOperationsOps(1, { r0: 0, c0: 0, r1: 1, c1: 1 }, { formulas: "", rowInput: "", colInput: "" }, cell)).toMatch(/formula cell/);
    expect(multipleOperationsOps(1, { r0: 0, c0: 0, r1: 1, c1: 1 }, { formulas: "B4", rowInput: "", colInput: "" }, cell)).toMatch(/input cell/);
  });
  it("names sheet copies and picks the group axis", () => {
    expect(copySheetName("Sheet1", ["Sheet1", "sheet1_2"])).toBe("Sheet1_3");
    expect(copySheetName("Data_2", ["Data_2"])).toBe("Data_3");
    expect(groupAxisFor({ r0: 1, c0: 0, r1: 5, c1: 16383 }, true, false)).toBe("rows");
    expect(groupAxisFor({ r0: 0, c0: 2, r1: 1048575, c1: 3 }, false, true)).toBe("cols");
    expect(groupAxisFor({ r0: 1, c0: 1, r1: 5, c1: 2 }, false, false)).toBe("rows");
    expect(groupAxisFor({ r0: 1, c0: 1, r1: 1, c1: 4 }, false, false)).toBe("cols");
    expect(groupAxisFor({ r0: 0, c0: 0, r1: 1048575, c1: 16383 }, true, true)).toBeNull();
  });
  it("counts hidden content for the merge prompt and joins it", () => {
    const range = { r0: 0, c0: 0, r1: 1, c1: 1 };
    expect(hiddenContentCount(range, [{ r: 0, c: 0 }, { r: 1, c: 1 }, { r: 0, c: 1 }])).toBe(2);
    expect(mergedContent([{ pos: { r: 1, c: 1 }, input: "c" }, { pos: { r: 0, c: 0 }, input: "a" }, { pos: { r: 0, c: 1 }, input: "" }], range)).toBe("a\nc");
  });
});

describe("outline model", () => {
  it("nests groups, removes them and hides collapsed rows", () => {
    let rows = addGroup([], 1, 10);
    rows = addGroup(rows, 2, 4);
    expect(rows).toEqual([
      { start: 1, end: 10, level: 1, collapsed: false },
      { start: 2, end: 4, level: 2, collapsed: false },
    ]);
    const outer = addGroup(rows, 0, 12);
    expect(outer.map((g) => g.level)).toEqual([1, 2, 3]);
    const collapsed = applyOutlineOp({ rows, cols: [] }, { type: "setOutlineCollapsed", sheet: 1, axis: "rows", start: 2, end: 4, collapsed: true });
    expect(hiddenByOutline(collapsed.rows)).toEqual([2, 3, 4]);
    expect(showLevel(rows, 1).map((g) => g.collapsed)).toEqual([true, true]);
    expect(showLevel(rows, 2).map((g) => g.collapsed)).toEqual([false, true]);
    expect(removeGroup(rows, 1, 10)).toEqual([{ start: 2, end: 4, level: 1, collapsed: false }]);
    expect(applyOutlineOp({ rows, cols: [] }, { type: "clearOutline", sheet: 1, axis: null })).toEqual(emptyOutlineModel());
    expect(applyOutlineOp({ rows, cols: [] }, { type: "ungroup", sheet: 1, axis: "cols", start: 0, end: 1 }).rows).toEqual(rows);
  });
  it("auto-outlines data blocks ended by formula rows", () => {
    const formulas = new Set(["3,0", "3,1", "7,0", "7,1"]);
    const m = autoOutlineGroups((r, c) => formulas.has(`${r},${c}`), { r0: 0, c0: 0, r1: 7, c1: 1 });
    expect(m.rows).toEqual([
      { start: 0, end: 2, level: 1, collapsed: false },
      { start: 4, end: 6, level: 1, collapsed: false },
    ]);
  });
});
