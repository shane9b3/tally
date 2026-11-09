import type { Op } from "@tally/bridge";
import { describe, expect, it } from "vitest";
import {
  absoluteRef,
  arrayFormulaOp,
  bordersPatch,
  borderLine,
  columnFilterFrom,
  fillDownRight,
  filterButtonsFor,
  findQuery,
  freezeAt,
  friendlyError,
  limitDetail,
  isValidName,
  parseAbsoluteRef,
  pasteOptions,
  quoteSheet,
  sortOp,
  stepHit,
  stripArrayBraces,
  suggestName,
  DEFAULT_FIND,
} from "../src/dataOps";
import { overlayRuns, WebMockClient } from "../src/dev/webMock";

describe("op builders", () => {
  const line = borderLine("medium", 0x112233ff);

  it("borders presets map to edges", () => {
    expect(line).toEqual({ style: "solid", widthPx: 2, color: 0x112233ff });
    expect(bordersPatch("all", line)).toEqual({ top: { line }, bottom: { line }, left: { line }, right: { line }, innerH: { line }, innerV: { line } });
    expect(bordersPatch("outer", line)).toMatchObject({ top: { line }, innerH: null, innerV: null });
    expect(bordersPatch("inner", line)).toMatchObject({ top: null, innerH: { line }, innerV: { line } });
    expect(bordersPatch("left", line)).toEqual({ top: null, bottom: null, left: { line }, right: null, innerH: null, innerV: null });
    expect(bordersPatch("none", line).top).toEqual({ line: null });
    expect(bordersPatch("none", line).innerV).toEqual({ line: null });
  });

  it("absolute references and names", () => {
    expect(absoluteRef("Sheet1", { r0: 0, c0: 0, r1: 4, c1: 1 })).toBe("$Sheet1.$A$1:$B$5");
    expect(absoluteRef("My sheet", { r0: 2, c0: 2, r1: 2, c1: 2 })).toBe("$'My sheet'.$C$3");
    expect(quoteSheet("It's")).toBe("'It''s'");
    expect(parseAbsoluteRef("$'My sheet'.$C$3")).toEqual({ sheet: "My sheet", range: { r0: 2, c0: 2, r1: 2, c1: 2 } });
    expect(parseAbsoluteRef("$Sheet1.$A$1:$B$5")).toEqual({ sheet: "Sheet1", range: { r0: 0, c0: 0, r1: 4, c1: 1 } });
    expect(parseAbsoluteRef("0.07")).toBeNull();
    expect(isValidName("TaxRate")).toBe(true);
    expect(isValidName("_x.y")).toBe(true);
    expect(isValidName("A1")).toBe(false);
    expect(isValidName("AB12")).toBe(false);
    expect(isValidName("R1C1")).toBe(false);
    expect(isValidName("true")).toBe(false);
    expect(isValidName("1abc")).toBe(false);
    expect(suggestName("Unit price")).toBe("Unit_price");
  });

  it("freeze at the active cell", () => {
    expect(freezeAt({ r: 1, c: 2 }, "both")).toEqual({ rows: 1, cols: 2 });
    expect(freezeAt({ r: 1, c: 2 }, "rows")).toEqual({ rows: 1, cols: 0 });
    expect(freezeAt({ r: 1, c: 2 }, "cols")).toEqual({ rows: 0, cols: 2 });
  });

  it("sort op keeps at most three keys", () => {
    const op = sortOp(1, {
      range: { r0: 0, c0: 0, r1: 14, c1: 6 },
      keys: [
        { col: 5, ascending: false },
        { col: 1, ascending: true },
        { col: 2, ascending: true },
        { col: 3, ascending: true },
      ],
      hasHeader: true,
    });
    expect(op).toMatchObject({ type: "sortRange", sheet: 1, hasHeader: true, caseSensitive: false });
    expect((op as Extract<Op, { type: "sortRange" }>).keys).toHaveLength(3);
  });

  it("column filters from the popover state", () => {
    const all = ["Due", "Overdue", "Paid"];
    expect(columnFilterFrom(all, new Set(all), { op: null, value: "" })).toBeNull();
    expect(columnFilterFrom(all, new Set(["Paid"]), { op: null, value: "" })).toEqual({ values: ["Paid"], condition: null });
    expect(columnFilterFrom(all, new Set(all), { op: "contains", value: " due " })).toEqual({ values: null, condition: { op: "contains", value: "due" } });
    expect(columnFilterFrom(all, new Set(all), { op: "gt", value: "" })).toBeNull();
    expect(columnFilterFrom(all, new Set(all), { op: "notEmpty", value: "x" })).toEqual({ values: null, condition: { op: "notEmpty", value: "" } });
  });

  it("filter buttons from AutoFilterInfo", () => {
    expect(filterButtonsFor(null)).toBeNull();
    expect(filterButtonsFor({ range: null, columns: [] })).toBeNull();
    const info = {
      range: { r0: 0, c0: 0, r1: 14, c1: 6 },
      columns: [
        { col: 0, unique: [], counts: [], active: null },
        { col: 6, unique: ["Paid"], counts: [8], active: { values: ["Paid"], condition: null } },
      ],
    };
    expect(filterButtonsFor(info)).toEqual({ row: 0, c0: 0, c1: 6, active: [6] });
  });

  it("array formulas", () => {
    expect(stripArrayBraces("{=SUM(A1:A3*B1:B3)}")).toBe("=SUM(A1:A3*B1:B3)");
    expect(stripArrayBraces("=A1")).toBe("=A1");
    const range = { r0: 0, c0: 0, r1: 2, c1: 1 };
    expect(arrayFormulaOp(1, range, "{=A1:A3*2}")).toEqual({ type: "setArrayFormula", sheet: 1, range, input: "=A1:A3*2" });
    expect(arrayFormulaOp(1, range, "hello")).toBeNull();
  });

  it("fill down / right plans", () => {
    expect(fillDownRight({ r0: 1, c0: 0, r1: 5, c1: 1 }, "down")).toEqual({ source: { r0: 1, c0: 0, r1: 1, c1: 1 }, target: { r0: 1, c0: 0, r1: 5, c1: 1 } });
    expect(fillDownRight({ r0: 3, c0: 2, r1: 3, c1: 2 }, "down")).toEqual({ source: { r0: 2, c0: 2, r1: 2, c1: 2 }, target: { r0: 2, c0: 2, r1: 3, c1: 2 } });
    expect(fillDownRight({ r0: 0, c0: 0, r1: 0, c1: 0 }, "down")).toBeNull();
    expect(fillDownRight({ r0: 0, c0: 1, r1: 0, c1: 4 }, "right")).toEqual({ source: { r0: 0, c0: 1, r1: 0, c1: 1 }, target: { r0: 0, c0: 1, r1: 0, c1: 4 } });
    // Up / Left copy the last row / column (AOO FILL_TO_TOP / FILL_TO_LEFT); one row / column does nothing.
    expect(fillDownRight({ r0: 1, c0: 0, r1: 5, c1: 1 }, "up")).toEqual({ source: { r0: 5, c0: 0, r1: 5, c1: 1 }, target: { r0: 1, c0: 0, r1: 5, c1: 1 } });
    expect(fillDownRight({ r0: 2, c0: 1, r1: 3, c1: 4 }, "left")).toEqual({ source: { r0: 2, c0: 4, r1: 3, c1: 4 }, target: { r0: 2, c0: 1, r1: 3, c1: 4 } });
    expect(fillDownRight({ r0: 3, c0: 2, r1: 3, c1: 2 }, "up")).toBeNull();
    expect(fillDownRight({ r0: 3, c0: 2, r1: 4, c1: 2 }, "left")).toBeNull();
  });

  it("find steps wrap; paste options; friendly errors", () => {
    expect(stepHit(-1, 3, 1)).toBe(0);
    expect(stepHit(-1, 3, -1)).toBe(2);
    expect(stepHit(2, 3, 1)).toBe(0);
    expect(stepHit(0, 3, -1)).toBe(2);
    expect(stepHit(0, 0, 1)).toBe(-1);
    expect(findQuery("Paid", { ...DEFAULT_FIND, allSheets: true }, 1, null)).toMatchObject({ text: "Paid", sheet: null });
    expect(pasteOptions({ what: "values", transpose: true, skipEmpty: false, operation: "add" })).toEqual({
      valuesOnly: true,
      formatsOnly: false,
      transpose: true,
      skipEmpty: false,
      operation: "add",
    });
    expect(friendlyError("array_block", "x")).toMatch(/array/);
    expect(friendlyError("other", "x")).toBe("x");
    expect(
      friendlyError("bad_file", "limit exceeded: the file is too large or complex to open safely (zip entry uncompressed bytes over 2 GiB)"),
    ).toMatch(/too large or too complex.*limit: zip entry uncompressed bytes over 2 GiB/);
    expect(friendlyError("bad_file", "not a zip archive")).toBe("not a zip archive");
    expect(limitDetail("limit exceeded: the file is too large or complex to open safely (file size over 512 MiB)")).toBe(
      "file size over 512 MiB",
    );
    expect(limitDetail("limit exceeded: cells (limit 1000000)")).toBe("cells over 1000000");
    expect(limitDetail("limit exceeded: cells (limit: 1000000)")).toBe("cells over 1000000");
    expect(limitDetail("bad header")).toBeNull();
  });
});

describe("WebMockClient v0.2 emulation", () => {
  async function invoices() {
    const c = new WebMockClient();
    const info = await c.newWorkbook("invoices");
    const doc = info.doc;
    const sheet = info.activeSheet;
    const tx = (ops: Op[]) => c.apply(doc, { ops, label: null, selectionBefore: null, selectionAfter: null });
    return { c, doc, sheet, tx };
  }

  it("filters rows and reports them as hidden + filtered", async () => {
    const { c, doc, sheet, tx } = await invoices();
    await tx([{ type: "setAutoFilter", sheet, range: { r0: 0, c0: 0, r1: 14, c1: 6 } }]);
    let info = await c.filterInfo(doc, sheet);
    const status = info.columns.find((x) => x.col === 6);
    expect(status?.unique).toEqual(["Due", "Overdue", "Paid"]);
    expect(status?.counts).toEqual([4, 2, 8]);
    const res = await tx([{ type: "setColumnFilter", sheet, col: 6, filter: { values: ["Paid"], condition: null } }]);
    expect(res.layoutChanged).toContain(sheet);
    const layout = await c.sheetLayout(doc, sheet);
    const hidden = layout.rowRuns.filter((r) => r.hidden).reduce((n, r) => n + r.end - r.start + 1, 0);
    expect(hidden).toBe(6);
    expect(layout.rowRuns.every((r) => !r.hidden || r.filtered === true)).toBe(true);
    info = await c.filterInfo(doc, sheet);
    expect(info.columns.find((x) => x.col === 6)?.active).toEqual({ values: ["Paid"], condition: null });
    await c.undo(doc);
    expect((await c.filterInfo(doc, sheet)).columns.find((x) => x.col === 6)?.active).toBeNull();
  });

  it("merges keep the top-left value and undo restores both", async () => {
    const { c, doc, sheet, tx } = await invoices();
    await tx([{ type: "mergeCells", sheet, range: { r0: 0, c0: 0, r1: 0, c1: 1 } }]);
    expect((await c.sheetLayout(doc, sheet)).merges).toEqual([{ r0: 0, c0: 0, r1: 0, c1: 1 }]);
    const [b] = await c.cells(doc, sheet, [{ r0: 0, c0: 0, r1: 0, c1: 1 }]);
    expect(b?.cells.map((v) => v.text).filter((t) => t !== "")).toEqual(["Date"]);
    const err = await tx([{ type: "mergeCells", sheet, range: { r0: 0, c0: 1, r1: 0, c1: 2 } }]);
    expect(err.errors[0]?.code).toBe("merged");
    await c.undo(doc);
    expect((await c.sheetLayout(doc, sheet)).merges).toEqual([]);
    const [a] = await c.cells(doc, sheet, [{ r0: 0, c0: 1, r1: 0, c1: 1 }]);
    expect(a?.cells[0]?.text).toBe("Customer");
  });

  it("sorts rows, finds text and records ops", async () => {
    const { c, doc, sheet, tx } = await invoices();
    await tx([{ type: "sortRange", sheet, range: { r0: 0, c0: 0, r1: 14, c1: 6 }, keys: [{ col: 3, ascending: false }], hasHeader: true, caseSensitive: false }]);
    const [col] = await c.cells(doc, sheet, [{ r0: 1, c0: 3, r1: 3, c1: 3 }]);
    expect(col?.cells.map((v) => v.text)).toEqual(["50", "40", "30"]);
    const found = await c.find(doc, { text: "Paid", sheet, inFormulas: false, matchCase: false, wholeCell: true, mode: "literal", from: null });
    expect(found.hits).toHaveLength(8);
    expect(await c.detectHeader(doc, sheet, { r0: 0, c0: 0, r1: 14, c1: 6 })).toBe(true);
    await tx([{ type: "defineName", name: "Totals", scope: null, refersTo: "$Invoices.$F$2:$F$15" }]);
    expect((await c.names(doc)).map((n) => n.name)).toEqual(["Totals"]);
    expect(c.recorded.map((o) => o.type)).toEqual(["sortRange", "defineName"]);
  });

  it("array blocks show {=…} and refuse partial edits", async () => {
    const { c, doc, sheet, tx } = await invoices();
    await tx([{ type: "setArrayFormula", sheet, range: { r0: 20, c0: 0, r1: 21, c1: 1 }, input: "=D2*2" }]);
    const info = await c.cellEditInfo(doc, sheet, 21, 1);
    expect(info).toMatchObject({ input: "{=D2*2}", array: true, arrayRange: { r0: 20, c0: 0, r1: 21, c1: 1 } });
    const res = await tx([{ type: "setCellInput", sheet, r: 20, c: 1, input: "5" }]);
    expect(res.errors[0]?.code).toBe("array_block");
  });

  it("borders land per edge (outer box)", async () => {
    const { c, doc, sheet, tx } = await invoices();
    const line = borderLine("thin");
    await tx([{ type: "applyStyle", sheet, ranges: [{ r0: 1, c0: 1, r1: 2, c1: 2 }], patch: { bold: null, italic: null, underline: null, strike: null, fontFamily: null, fontSizePt: null, color: null, fill: null, hAlign: null, vAlign: null, wrap: null, numberFormat: null, borders: bordersPatch("outer", line) } }]);
    const [b] = await c.cells(doc, sheet, [{ r0: 1, c0: 1, r1: 1, c1: 1 }]);
    const [st] = await c.styles(doc, [b?.cells[0]?.style ?? 0]);
    expect(st?.borders).toEqual({ top: line, bottom: null, left: line, right: null });
  });

  it("overlays hidden rows on size runs", () => {
    expect(overlayRuns([{ start: 0, end: 9, size: 30, hidden: false }], 24, new Set([2, 3]), new Set([5]))).toEqual([
      { start: 0, end: 1, size: 30, hidden: false },
      { start: 2, end: 3, size: 30, hidden: true, filtered: null },
      { start: 4, end: 4, size: 30, hidden: false },
      { start: 5, end: 5, size: 30, hidden: true, filtered: true },
      { start: 6, end: 9, size: 30, hidden: false },
    ]);
  });
});
