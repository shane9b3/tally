/**
 * W5-6A: the DataPilot layout model and the mock's pivot emulation
 * (`createPivot` writes the AOO tabular layout; `pivot_at`, drill-down).
 */
import type { PivotFieldItemsDto } from "@tally/bridge";
import { describe, expect, it } from "vitest";
import { WebMockClient } from "../src/dev/webMock";
import { addField, availableFields, dataCaption, emptyLayout, fromPivotDto, layoutError, layoutOptionsEnabled, moveField, pivotAtCell, pivotHeaderRanges, removeField, toPivotDto, updateDataField, updateField } from "../src/pivot/pivotModel";

const FIELDS: PivotFieldItemsDto[] = [
  { sourceCol: 0, name: "Region", numeric: false, date: false, items: [{ text: "East", count: 2 }, { text: "West", count: 2 }] },
  { sourceCol: 1, name: "Quarter", numeric: false, date: false, items: [{ text: "Q1", count: 2 }, { text: "Q2", count: 2 }] },
  { sourceCol: 2, name: "Revenue", numeric: true, date: false, items: [] },
];

describe("pivot field layout options", () => {
  it("apply to row fields with an inner level only (AOO pvlaydlg.cxx:353)", () => {
    let st = emptyLayout({ type: "range", sheet: 1, range: { r0: 0, c0: 0, r1: 4, c1: 2 } }, { r0: 0, c0: 0, r1: 4, c1: 2 }, 1);
    st = addField(st, FIELDS[0]!, "row");
    st = addField(st, FIELDS[1]!, "col");
    st = addField(st, FIELDS[2]!, "data");
    // The only (innermost) row field, one data field; a column field.
    expect(layoutOptionsEnabled(st, "row", 0)).toBe(false);
    expect(layoutOptionsEnabled(st, "col", 1)).toBe(false);
    st = moveField(st, 1, "col", "row");
    expect(layoutOptionsEnabled(st, "row", 0)).toBe(true);
    expect(layoutOptionsEnabled(st, "row", 1)).toBe(false);
    // Several data fields: the Data field may follow the last row field.
    st = addField(st, FIELDS[2]!, "data");
    expect(layoutOptionsEnabled(st, "row", 1)).toBe(true);
  });
});

describe("pivot layout model", () => {
  it("places fields, moves them between areas and builds the DTO", () => {
    let st = emptyLayout({ type: "range", sheet: 1, range: { r0: 0, c0: 0, r1: 4, c1: 2 } }, { r0: 0, c0: 0, r1: 4, c1: 2 }, 1);
    expect(st.target).toEqual({ r: 0, c: 4 });
    expect(layoutError(st)).toMatch(/data field/);
    st = addField(st, FIELDS[0]!, "row");
    st = addField(st, FIELDS[1]!, "col");
    st = addField(st, FIELDS[2]!, "data");
    expect(availableFields(st, FIELDS).map((f) => f.name)).toEqual(["Revenue"]);
    expect(st.data[0]?.function).toBe("sum");
    expect(dataCaption(st.data[0]!)).toBe("Sum - Revenue");
    st = moveField(st, 1, "col", "page");
    expect(st.col).toEqual([]);
    expect(st.page.map((f) => f.name)).toEqual(["Quarter"]);
    st = updateField(st, "page", 1, { pageItem: "Q1" });
    st = updateDataField(st, 0, { function: "average", showAs: { kind: "percentOfTotal", baseField: null, baseItem: null } });
    expect(layoutError(st)).toBeNull();
    const dto = toPivotDto(st, 1, 0, "");
    expect(dto).toMatchObject({ targetSheet: 1, rowFields: [{ sourceCol: 0, subtotals: { type: "auto" } }], pageFields: [{ sourceCol: 1, pageItem: "Q1" }], dataFields: [{ sourceCol: 2, function: "average", showAs: { kind: "percentOfTotal" } }], rowTotals: true });
    const back = fromPivotDto({ ...dto, id: 3, output: { r0: 0, c0: 4, r1: 5, c1: 6 } }, FIELDS);
    expect(back.row[0]?.name).toBe("Region");
    expect(back.data[0]?.name).toBe("Revenue");
    st = removeField(st, 0, null);
    expect(st.row).toEqual([]);
    expect(removeField(st, 2, 0).data).toEqual([]);
    expect(pivotAtCell([{ ...dto, id: 3, output: { r0: 0, c0: 4, r1: 5, c1: 6 } }], 1, { r: 2, c: 5 })?.id).toBe(3);
    expect(pivotAtCell([{ ...dto, id: 3, output: { r0: 0, c0: 4, r1: 5, c1: 6 } }], 2, { r: 2, c: 5 })).toBeNull();
    expect(pivotHeaderRanges({ ...dto, id: 3, output: { r0: 0, c0: 4, r1: 5, c1: 6 } })[0]).toEqual({ r0: 3, c0: 4, r1: 3, c1: 6 });
  });
});

async function setup() {
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
  const texts = async (range: { r0: number; c0: number; r1: number; c1: number }) => {
    const [b] = await c.cells(doc, sheet, [range]);
    const out: string[][] = [];
    for (let r = range.r0; r <= range.r1; r++) out.push(Array.from({ length: range.c1 - range.c0 + 1 }, (_, i) => b?.cells.find((v) => v.r === r && v.c === range.c0 + i)?.text ?? ""));
    return out;
  };
  return { c, doc, sheet, tx, texts };
}

describe("mock DataPilot", () => {
  it("scans the source fields, writes the tabular layout and answers pivot_at", async () => {
    const { c, doc, sheet, tx, texts } = await setup();
    const source = { type: "range" as const, sheet, range: { r0: 0, c0: 0, r1: 4, c1: 2 } };
    const fields = await c.pivotFieldItems(doc, source);
    expect(fields.map((f) => [f.name, f.numeric, f.items.length])).toEqual([["Region", false, 2], ["Quarter", false, 2], ["Revenue", true, 4]]);
    let layout = emptyLayout(source, source.range, sheet);
    layout = addField(layout, fields[0]!, "row");
    layout = addField(layout, fields[1]!, "col");
    layout = addField(layout, fields[2]!, "data");
    layout = { ...layout, showFilterButton: false };
    const res = await tx([{ type: "createPivot", pivot: toPivotDto(layout, sheet), overwrite: false }]);
    expect(res.errors).toEqual([]);
    expect(res.pivotsChanged).toBe(true);
    const pivots = await c.pivots(doc);
    expect(pivots).toHaveLength(1);
    expect(pivots[0]?.output).toEqual({ r0: 0, c0: 4, r1: 4, c1: 7 });
    expect(await texts({ r0: 0, c0: 4, r1: 4, c1: 7 })).toEqual([
      ["Quarter", "Q1", "Q2", "Total Result"],
      ["Region", "", "", ""],
      ["East", "10", "20", "30"],
      ["West", "5", "7", "12"],
      ["Total Result", "15", "27", "42"],
    ]);
    expect(await c.pivotAt(doc, sheet, 2, 5)).toMatchObject({ pivot: 1, role: "data" });
    expect(await c.pivotAt(doc, sheet, 2, 4)).toMatchObject({ role: "rowItem" });
    expect(await c.pivotAt(doc, sheet, 4, 5)).toMatchObject({ role: "total" });
    expect(await c.pivotAt(doc, sheet, 0, 0)).toBeNull();
    // A non-empty target is refused unless overwrite.
    const clash = await tx([{ type: "createPivot", pivot: { ...toPivotDto(layout, sheet), target: { r: 0, c: 5 } }, overwrite: false }]);
    expect(clash.errors[0]?.code).toBe("target_not_empty");
    // Refresh after an edit and drill down.
    await tx([{ type: "setCellInput", sheet, r: 1, c: 2, input: "100" }]);
    await tx([{ type: "refreshPivot", id: 1 }]);
    expect((await texts({ r0: 2, c0: 5, r1: 2, c1: 5 }))[0]).toEqual(["100"]);
    const drill = await tx([{ type: "pivotDrillDown", id: 1, r: 2, c: 5 }]);
    expect(drill.sheetsChanged).toBe(true);
    const added = drill.info?.sheets.find((s) => s.id !== sheet);
    expect(added).toBeDefined();
    const [b] = await c.cells(doc, added!.id, [{ r0: 0, c0: 0, r1: 3, c1: 2 }]);
    expect(b?.cells.filter((v) => v.c === 0).map((v) => v.text)).toEqual(["Region", "East", "East"]);
    // Undo (all the way back) restores the output and the list.
    for (let i = 0; i < 12; i++) {
      const u = await c.undo(doc);
      if (!u.undo.canUndo) break;
    }
    expect(await c.pivots(doc)).toEqual([]);
    expect((await texts({ r0: 0, c0: 4, r1: 0, c1: 4 }))[0]).toEqual([""]);
    const nav = await c.navigator(doc);
    expect(nav.sheets.length).toBeGreaterThanOrEqual(1);
  });
});

describe("pivot layout round trip (review 4 H14)", () => {
  it("keeps the field grouping when a layout is edited and sent back", () => {
    let st = emptyLayout({ type: "range", sheet: 1, range: { r0: 0, c0: 0, r1: 4, c1: 2 } }, { r0: 0, c0: 0, r1: 4, c1: 2 }, 1);
    st = addField(st, FIELDS[2]!, "data");
    st = addField(st, FIELDS[0]!, "row");
    const group = { type: "numeric", start: 0, end: 100, step: 10 } as never;
    const dto = toPivotDto(st, 1);
    dto.rowFields[0]!.group = group;
    const back = toPivotDto(fromPivotDto(dto, FIELDS), 1);
    expect(back.rowFields[0]!.group).toEqual(group);
    const moved = toPivotDto(moveField(fromPivotDto(dto, FIELDS), 0, "row", "col"), 1);
    expect(moved.colFields[0]!.group).toEqual(group);
  });
});

describe("mock DataPilot refresh over occupied cells", () => {
  it("refuses with target_not_empty unless overwrite is set", async () => {
    const { c, doc, sheet, tx } = await setup();
    const source = { type: "range" as const, sheet, range: { r0: 0, c0: 0, r1: 6, c1: 2 } };
    const fields = await c.pivotFieldItems(doc, source);
    let layout = emptyLayout(source, source.range, sheet);
    layout = addField(layout, fields[0]!, "row");
    layout = addField(layout, fields[1]!, "col");
    layout = addField(layout, fields[2]!, "data");
    expect((await tx([{ type: "createPivot", pivot: toPivotDto(layout, sheet), overwrite: false }])).errors).toEqual([]);
    const out = (await c.pivots(doc))[0]!.output!;
    // A new source row makes the output grow by one row, over an occupied cell.
    await tx([{ type: "setRangeInputs", sheet, r: 5, c: 0, rows: [["North", "Q1", "3"], ["South", "Q1", "4"]] }]);
    await tx([{ type: "setRangeInputs", sheet, r: out.r1 + 1, c: out.c0, rows: [["keep", "keep", "keep", "keep", "keep"], ["keep", "keep", "keep", "keep", "keep"]] }]);
    const refused = await tx([{ type: "refreshPivot", id: 1, overwrite: false }]);
    expect(refused.errors[0]?.code).toBe("target_not_empty");
    const ok = await tx([{ type: "refreshPivot", id: 1, overwrite: true }]);
    expect(ok.errors).toEqual([]);
  });
});
