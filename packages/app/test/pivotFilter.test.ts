/**
 * T17: the DataPilot "Filter" button and its source filter (AOO
 * `ScPivotFilterDlg`): the dialog model, the button cells and the
 * browser mock's emulation (the filter limits the source rows).
 */
import type { PivotDto } from "@tally/bridge";
import { describe, expect, it } from "vitest";
import { WebMockClient } from "../src/dev/webMock";
import { addField, emptyLayout, toPivotDto } from "../src/pivot/pivotModel";
import { PIVOT_FILTER_OPS, filterButtonCell, pivotFilterButtons, pivotFilterDto, pivotFilterError, pivotFilterOptions, pivotFilterRows, pivotWithButtonAt, withSourceFilter } from "../src/pivot/pivotFilter";

describe("pivot filter model", () => {
  it("offers AOO's six conditions and three rows", () => {
    expect(PIVOT_FILTER_OPS.map((o) => o.op)).toEqual(["eq", "lt", "gt", "le", "ge", "ne"]);
    const rows = pivotFilterRows(null);
    expect(rows).toHaveLength(3);
    expect(rows.every((r) => r.col === null)).toBe(true);
    expect(pivotFilterOptions(undefined)).toEqual({ caseSensitive: false, regex: false, noDuplicates: false });
  });

  it("builds the filter from the used rows and reads it back", () => {
    const rows = pivotFilterRows(null);
    rows[0] = { connector: "or", col: 0, op: "eq", value: "East" };
    rows[1] = { connector: "or", col: 2, op: "gt", value: "5" };
    // Rows after the first unused one are ignored (AOO `GetOutputItem`).
    rows[2] = { connector: "and", col: null, op: "eq", value: "" };
    const f = pivotFilterDto(rows, { caseSensitive: true, regex: false, noDuplicates: true });
    expect(f).toEqual({
      conditions: [
        { connector: "and", col: 0, op: "eq", value: "East" },
        { connector: "or", col: 2, op: "gt", value: "5" },
      ],
      caseSensitive: true,
      regex: false,
      noDuplicates: true,
    });
    expect(pivotFilterRows(f).slice(0, 2).map((r) => [r.col, r.op, r.value])).toEqual([
      [0, "eq", "East"],
      [2, "gt", "5"],
    ]);
    expect(pivotFilterOptions(f)).toEqual({ caseSensitive: true, regex: false, noDuplicates: true });
    // "- none -" in the first row: no filter.
    expect(pivotFilterDto(pivotFilterRows(null), pivotFilterOptions(null))).toBeNull();
  });

  it("checks regular expressions only with the regex option", () => {
    const bad = { conditions: [{ connector: "and" as const, col: 0, op: "eq" as const, value: "(" }], caseSensitive: false, regex: true, noDuplicates: false };
    expect(pivotFilterError(bad)).toMatch(/regular expression/);
    expect(pivotFilterError({ ...bad, regex: false })).toBeNull();
    expect(pivotFilterError(null)).toBeNull();
  });

  it("finds the button cells of the pivots on a sheet", () => {
    const base = { output: { r0: 2, c0: 4, r1: 9, c1: 6 }, targetSheet: 1, showFilterButton: true, id: 7 } as PivotDto;
    expect(filterButtonCell(base)).toEqual({ r: 2, c: 4 });
    expect(filterButtonCell({ ...base, showFilterButton: false })).toBeNull();
    expect(filterButtonCell({ ...base, output: null })).toBeNull();
    expect(pivotFilterButtons([base, { ...base, id: 8, targetSheet: 2 }], 1)).toEqual([{ r: 2, c: 4 }]);
    expect(pivotWithButtonAt([base], 1, { r: 2, c: 4 })?.id).toBe(7);
    expect(pivotWithButtonAt([base], 1, { r: 3, c: 4 })).toBeNull();
    expect(withSourceFilter(base, null).sourceFilter).toBeNull();
  });
});

describe("mock DataPilot source filter", () => {
  it("writes the Filter button and filters the source rows", async () => {
    const c = new WebMockClient();
    const info = await c.newWorkbook(null);
    const doc = info.doc;
    const sheet = info.activeSheet;
    const tx = (ops: Parameters<WebMockClient["apply"]>[1]["ops"]) => c.apply(doc, { ops, label: null, selectionBefore: null, selectionAfter: null });
    await tx([
      {
        type: "setRangeInputs",
        sheet,
        r: 0,
        c: 0,
        rows: [
          ["Region", "Quarter", "Revenue"],
          ["East", "Q1", "10"],
          ["East", "Q2", "20"],
          ["West", "Q1", "5"],
          ["West", "Q2", "7"],
        ],
      },
    ]);
    const source = { type: "range" as const, sheet, range: { r0: 0, c0: 0, r1: 4, c1: 2 } };
    const fields = await c.pivotFieldItems(doc, source);
    let layout = emptyLayout(source, source.range, sheet);
    layout = addField(layout, fields[0]!, "row");
    layout = addField(layout, fields[2]!, "data");
    layout = { ...layout, showFilterButton: true };
    expect((await tx([{ type: "createPivot", pivot: toPivotDto(layout, sheet), overwrite: false }])).errors).toEqual([]);
    const p = (await c.pivots(doc))[0]!;
    const cell = async (r: number, col: number) => (await c.cells(doc, sheet, [{ r0: r, c0: col, r1: r, c1: col }]))[0]?.cells[0]?.text ?? "";
    expect(await cell(0, 4)).toBe("Filter");
    expect(await cell(1, 4)).toBe("");
    expect(await c.pivotAt(doc, sheet, 0, 4)).toMatchObject({ role: "filterButton" });
    const filter = { conditions: [{ connector: "and" as const, col: 2, op: "ge" as const, value: "7" }], caseSensitive: false, regex: false, noDuplicates: false };
    expect((await tx([{ type: "updatePivot", id: p.id, pivot: withSourceFilter(p, filter), overwrite: false }])).errors).toEqual([]);
    // Revenue >= 7: East 10 + 20, West 7.
    expect(await cell(3, 4)).toBe("East");
    expect(await cell(3, 5)).toBe("30");
    expect(await cell(4, 5)).toBe("7");
    expect((await c.pivots(doc))[0]?.sourceFilter).toEqual(filter);
  });
});
