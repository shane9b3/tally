import { describe, expect, it } from "vitest";
import { isApiError, MockClient, type Op, type SelectionDto, type Transaction } from "../src/index";

const tx = (...ops: Op[]): Transaction => ({ ops, label: null, selectionBefore: null, selectionAfter: null });
const set = (sheet: number, r: number, c: number, input: string): Op => ({ type: "setCellInput", sheet, r, c, input });
const sel = (sheet: number, r0: number, c0: number, r1: number, c1: number): SelectionDto => ({
  sheet,
  ranges: [{ r0, c0, r1, c1 }],
  active: { r: r0, c: c0 },
});

async function setup() {
  const client = new MockClient();
  const info = await client.newWorkbook();
  const sheet = info.activeSheet;
  return { client, doc: info.doc, sheet };
}

async function text(client: MockClient, doc: number, sheet: number, r: number, c: number) {
  const [block] = await client.cells(doc, sheet, [{ r0: r, c0: c, r1: r, c1: c }]);
  return block?.cells[0]?.text ?? "";
}

describe("MockClient", () => {
  it("creates a workbook with one sheet", async () => {
    const client = new MockClient();
    const info = await client.newWorkbook();
    expect(info.sheets).toHaveLength(1);
    expect(info.sheets[0]?.name).toBe("Sheet1");
    expect(info.dirty).toBe(false);
  });

  it("evaluates the supported formulas", async () => {
    const { client, doc, sheet } = await setup();
    const res = await client.apply(
      doc,
      tx(set(sheet, 0, 0, "2"), set(sheet, 0, 1, "3"), set(sheet, 0, 2, "=A1*B1"), set(sheet, 0, 3, "=a1+b1")),
    );
    expect(res.errors).toEqual([]);
    expect(await text(client, doc, sheet, 0, 2)).toBe("6");
    expect(await text(client, doc, sheet, 0, 3)).toBe("5");
    await client.apply(doc, tx(set(sheet, 1, 0, "=SUM(A1:D1)"), set(sheet, 1, 1, "=A1/0"), set(sheet, 1, 2, "=FOO()")));
    expect(await text(client, doc, sheet, 1, 0)).toBe("16");
    expect(await text(client, doc, sheet, 1, 1)).toBe("#DIV/0!");
    expect(await text(client, doc, sheet, 1, 2)).toBe("#NAME?");
    // Dependents are fresh after an edit.
    await client.apply(doc, tx(set(sheet, 0, 0, "10")));
    expect(await text(client, doc, sheet, 0, 2)).toBe("30");
  });

  it("detects cycles and text in arithmetic", async () => {
    const { client, doc, sheet } = await setup();
    await client.apply(doc, tx(set(sheet, 0, 0, "=B1+1"), set(sheet, 0, 1, "=A1+1"), set(sheet, 1, 0, "x"), set(sheet, 1, 1, "=A2*2")));
    expect(await text(client, doc, sheet, 0, 0)).toBe("Err:522");
    expect(await text(client, doc, sheet, 1, 1)).toBe("#VALUE!");
  });

  it("reports cell views with kinds, flags and edit info", async () => {
    const { client, doc, sheet } = await setup();
    await client.apply(doc, tx(set(sheet, 0, 0, "1.5"), set(sheet, 0, 1, "hello"), set(sheet, 0, 2, "true"), set(sheet, 0, 3, "=A1*A1")));
    const [block] = await client.cells(doc, sheet, [{ r0: 0, c0: 0, r1: 10, c1: 10 }]);
    expect(block?.cells.map((c) => c.kind)).toEqual(["number", "text", "bool", "number"]);
    expect(block?.cells[3]?.flags).toBe(1);
    expect(block?.cells[3]?.num).toBe(2.25);
    expect(await client.cellEditInfo(doc, sheet, 0, 3)).toEqual({ input: "=A1*A1", hint: "A1 × A1" });
  });

  it("applies bold and italic as deduplicated styles", async () => {
    const { client, doc, sheet } = await setup();
    await client.apply(doc, tx(set(sheet, 0, 0, "x")));
    const ranges = [{ r0: 0, c0: 0, r1: 0, c1: 1 }];
    const patch = { bold: true, italic: null, underline: null, strike: null, fontFamily: null, fontSizePt: null, color: null, fill: null, hAlign: null, vAlign: null, wrap: null, numberFormat: null };
    await client.apply(doc, tx({ type: "applyStyle", sheet, ranges, patch }));
    const [block] = await client.cells(doc, sheet, ranges);
    expect(block?.cells).toHaveLength(2); // styled empty cell B1 is included
    const ids = [...new Set(block?.cells.map((c) => c.style))];
    expect(ids).toHaveLength(1);
    const [style] = await client.styles(doc, ids);
    expect(style?.bold).toBe(true);
    expect(style?.italic).toBe(false);
    await client.apply(doc, tx({ type: "applyStyle", sheet, ranges, patch: { ...patch, bold: null, italic: true } }));
    const [after] = await client.cells(doc, sheet, ranges);
    const [both] = await client.styles(doc, [after?.cells[0]?.style ?? -1]);
    expect(both?.bold && both.italic).toBe(true);
  });

  it("adds, renames, moves and deletes sheets", async () => {
    const { client, doc, sheet } = await setup();
    const added = await client.apply(doc, tx({ type: "addSheet", name: null, index: null }));
    expect(added.sheetsChanged).toBe(true);
    expect(added.info?.sheets.map((s) => s.name)).toEqual(["Sheet1", "Sheet2"]);
    const second = added.info?.sheets[1]?.id ?? -1;
    const renamed = await client.apply(doc, tx({ type: "renameSheet", sheet: second, name: "Data" }));
    expect(renamed.info?.sheets.map((s) => s.name)).toEqual(["Sheet1", "Data"]);
    const dup = await client.apply(doc, tx({ type: "renameSheet", sheet, name: "data" }));
    expect(dup.errors[0]?.code).toBe("invalid_argument");
    const moved = await client.apply(doc, tx({ type: "moveSheet", sheet: second, to: 0 }));
    expect(moved.info?.sheets.map((s) => s.name)).toEqual(["Data", "Sheet1"]);
    const deleted = await client.apply(doc, tx({ type: "deleteSheet", sheet: second }));
    expect(deleted.info?.sheets.map((s) => s.name)).toEqual(["Sheet1"]);
    const last = await client.apply(doc, tx({ type: "deleteSheet", sheet }));
    expect(last.errors).toHaveLength(1);
  });

  it("undoes and redoes transactions", async () => {
    const { client, doc, sheet } = await setup();
    await client.apply(doc, { ...tx(set(sheet, 0, 0, "1")), label: "Type" });
    const r2 = await client.apply(doc, { ...tx(set(sheet, 0, 0, "2")), label: "Type again" });
    expect(r2.undo).toEqual({ canUndo: true, canRedo: false, undoLabel: "Type again", redoLabel: null });
    const u = await client.undo(doc);
    expect(u.undo.canRedo).toBe(true);
    expect(await text(client, doc, sheet, 0, 0)).toBe("1");
    await client.undo(doc);
    expect(await text(client, doc, sheet, 0, 0)).toBe("");
    await expect(client.undo(doc)).rejects.toMatchObject({ code: "invalid_argument" });
    await client.redo(doc);
    await client.redo(doc);
    expect(await text(client, doc, sheet, 0, 0)).toBe("2");
    // A new edit clears the redo stack.
    await client.undo(doc);
    const r = await client.apply(doc, tx(set(sheet, 5, 5, "z")));
    expect(r.undo.canRedo).toBe(false);
  });

  it("computes selection stats", async () => {
    const { client, doc, sheet } = await setup();
    await client.apply(doc, tx(set(sheet, 0, 0, "2"), set(sheet, 0, 1, "3"), set(sheet, 0, 2, "=A1*B1"), set(sheet, 0, 3, "note")));
    const stats = await client.selectionStats(doc, sel(sheet, 0, 0, 0, 3));
    expect(stats).toEqual({ sum: 11, count: 4, countNumbers: 3, average: 11 / 3, min: 2, max: 6, formattedSum: "11" });
    const empty = await client.selectionStats(doc, sel(sheet, 9, 9, 9, 9));
    expect(empty.sum).toBeNull();
    expect(empty.count).toBe(0);
  });

  it("returns a default layout and records column widths", async () => {
    const { client, doc, sheet } = await setup();
    let layout = await client.sheetLayout(doc, sheet);
    expect(layout.defaultColWidthPx).toBeGreaterThan(0);
    expect(layout.used).toBeNull();
    const res = await client.apply(doc, tx({ type: "setColWidth", sheet, start: 2, end: 4, px: 150 }, set(sheet, 3, 1, "x")));
    expect(res.layoutChanged).toEqual([sheet]);
    layout = await client.sheetLayout(doc, sheet);
    expect(layout.colRuns).toEqual([{ start: 2, end: 4, size: 150, hidden: false }]);
    expect(layout.used).toEqual({ r0: 3, c0: 1, r1: 3, c1: 1 });
  });

  it("navigates edges, resolves the name box, copies and pastes", async () => {
    const { client, doc, sheet } = await setup();
    await client.apply(doc, tx(set(sheet, 0, 0, "1"), set(sheet, 1, 0, "2"), set(sheet, 2, 0, "3"), set(sheet, 9, 0, "4")));
    expect(await client.navigateEdge(doc, sheet, { r: 0, c: 0 }, "down")).toEqual({ r: 2, c: 0 });
    expect(await client.navigateEdge(doc, sheet, { r: 2, c: 0 }, "down")).toEqual({ r: 9, c: 0 });
    expect(await client.navigateEdge(doc, sheet, { r: 0, c: 0 }, "up")).toEqual({ r: 0, c: 0 });
    expect(await client.navigateEdge(doc, sheet, { r: 0, c: 0 }, "right")).toEqual({ r: 0, c: 16383 });
    expect((await client.resolveNameBox(doc, "b2:a1")).ranges).toEqual([{ r0: 0, c0: 0, r1: 1, c1: 1 }]);
    expect((await client.resolveNameBox(doc, "Sheet1.C3")).active).toEqual({ r: 2, c: 2 });
    await expect(client.resolveNameBox(doc, "nope")).rejects.toSatisfy(isApiError);
    const clip = await client.copy(doc, sel(sheet, 0, 0, 1, 0));
    expect(clip.tsv).toBe("1\n2");
    await client.apply(doc, tx({ type: "paste", sheet, target: { r0: 0, c0: 5, r1: 0, c1: 5 }, payload: { type: "internal", token: clip.token } }));
    await client.apply(doc, tx({ type: "paste", sheet, target: { r0: 0, c0: 6, r1: 0, c1: 6 }, payload: { type: "tsv", text: "7\t8\n" } }));
    expect(await text(client, doc, sheet, 1, 5)).toBe("2");
    expect(await text(client, doc, sheet, 0, 7)).toBe("8");
  });

  it("rejects unknown documents and unsupported ops with ApiError", async () => {
    const { client, doc, sheet } = await setup();
    await expect(client.sheetLayout(999, sheet)).rejects.toMatchObject({ code: "not_found" });
    const res = await client.apply(doc, tx({ type: "insertRows", sheet, at: 0, count: 1 }));
    expect(res.errors[0]).toMatchObject({ opIndex: 0, code: "unsupported" });
    expect(await client.assistSuggest(doc, "sum", sel(sheet, 0, 0, 0, 0))).toEqual([]);
    expect((await client.listFunctions()).map((f) => f.name)).toContain("SUM");
    await client.close(doc);
    await expect(client.close(doc)).rejects.toSatisfy(isApiError);
  });

  it("delivers events to subscribers until unsubscribed", async () => {
    const client = new MockClient();
    const seen: string[] = [];
    const off = client.onMenuAction((e) => seen.push(e.id));
    client.emitMenuAction("edit.undo");
    off();
    client.emitMenuAction("edit.redo");
    expect(seen).toEqual(["edit.undo"]);
  });
});
