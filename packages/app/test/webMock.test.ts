import { describe, expect, it } from "vitest";
import { WebMockClient } from "../src/dev/webMock";
import { fromSelectionDto, toSelectionDto } from "../src/stores/selectionStore";

async function setup() {
  const c = new WebMockClient();
  const info = await c.newWorkbook(null);
  const doc = info.doc;
  const tx = (ops: Parameters<WebMockClient["apply"]>[1]["ops"]) => c.apply(doc, { ops, label: null, selectionBefore: null, selectionAfter: null });
  await tx([{ type: "setRangeInputs", sheet: 1, r: 0, c: 0, rows: [["a", "b"], ["c", "d"], ["e", "f"]] }]);
  await tx([{ type: "applyStyle", sheet: 1, ranges: [{ r0: 1, c0: 0, r1: 1, c1: 0 }], patch: { bold: true, italic: null, underline: null, strike: null, fontFamily: null, fontSizePt: null, color: null, fill: null, hAlign: null, vAlign: null, wrap: null, numberFormat: null } }]);
  const col = async (c0: number) => (await c.cells(doc, 1, [{ r0: 0, c0, r1: 5, c1: c0 }]))[0]!.cells.map((v) => `${v.r}:${v.text}`);
  return { c, doc, tx, col };
}

describe("WebMockClient structural ops", () => {
  it("inserts rows (moving styles) as one undo step", async () => {
    const { c, doc, tx, col } = await setup();
    const res = await tx([{ type: "insertRows", sheet: 1, at: 1, count: 2 }]);
    expect(res.errors).toEqual([]);
    expect(await col(0)).toEqual(["0:a", "3:c", "4:e"]);
    const [b] = await c.cells(doc, 1, [{ r0: 3, c0: 0, r1: 3, c1: 0 }]);
    const [style] = await c.styles(doc, [b!.cells[0]!.style]);
    expect(style?.bold).toBe(true);
    await c.undo(doc);
    expect(await col(0)).toEqual(["0:a", "1:c", "2:e"]);
  });

  it("deletes rows and columns", async () => {
    const { tx, col } = await setup();
    await tx([{ type: "deleteRows", sheet: 1, at: 0, count: 1 }]);
    expect(await col(0)).toEqual(["0:c", "1:e"]);
    await tx([{ type: "deleteCols", sheet: 1, at: 0, count: 1 }]);
    expect(await col(0)).toEqual(["0:d", "1:f"]);
    await tx([{ type: "insertCols", sheet: 1, at: 0, count: 1 }]);
    expect(await col(1)).toEqual(["0:d", "1:f"]);
  });
});

describe("multi-range selection DTO", () => {
  it("round-trips every range with the current one last", () => {
    const sel = { range: { r0: 0, c0: 2, r1: 0, c1: 2 }, active: { r: 0, c: 2 }, anchor: { r: 0, c: 2 }, others: [{ r0: 0, c0: 0, r1: 1, c1: 0 }] };
    const dto = toSelectionDto(sel, 1);
    expect(dto.ranges).toEqual([{ r0: 0, c0: 0, r1: 1, c1: 0 }, { r0: 0, c0: 2, r1: 0, c1: 2 }]);
    expect(fromSelectionDto(dto)).toEqual(sel);
  });
});
