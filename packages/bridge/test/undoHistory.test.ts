/** MockClient: undo / redo history lists, n steps at once, and the undo depth setting (AOO parity, U17). */
import { describe, expect, it } from "vitest";
import { MockClient, mergeResults, type Op, type Transaction } from "../src/index";

const set = (sheet: number, r: number, c: number, input: string): Op => ({ type: "setCellInput", sheet, r, c, input });
const tx = (op: Op, label: string): Transaction => ({ ops: [op], label, selectionBefore: null, selectionAfter: null });

async function setup(steps: number) {
  const client = new MockClient();
  const info = await client.newWorkbook();
  const sheet = info.activeSheet;
  for (let i = 0; i < steps; i++) await client.apply(info.doc, tx(set(sheet, i, 0, String(i)), `step ${i}`));
  return { client, doc: info.doc, sheet };
}

async function text(client: MockClient, doc: number, sheet: number, r: number) {
  const [block] = await client.cells(doc, sheet, [{ r0: r, c0: 0, r1: r, c1: 0 }]);
  return block?.cells[0]?.text ?? "";
}

describe("undo history", () => {
  it("lists the labels next first and undoes / redoes n steps as one result", async () => {
    const { client, doc, sheet } = await setup(4);
    expect(await client.undoHistory(doc)).toEqual({ undo: ["step 3", "step 2", "step 1", "step 0"], redo: [] });
    const u = await client.undoSteps(doc, 3);
    expect(u.undo).toMatchObject({ undoLabel: "step 0", redoLabel: "step 1" });
    // Every cell the steps touched is reported, not only the last step's.
    const rows = u.changed.flatMap((c) => c.ranges.map((r) => r.r0));
    expect(rows.length).toBeGreaterThan(0);
    expect(await text(client, doc, sheet, 0)).toBe("0");
    expect(await text(client, doc, sheet, 1)).toBe("");
    expect(await client.undoHistory(doc)).toEqual({ undo: ["step 0"], redo: ["step 1", "step 2", "step 3"] });
    await client.redoSteps(doc, 2);
    expect(await text(client, doc, sheet, 2)).toBe("2");
    expect(await text(client, doc, sheet, 3)).toBe("");
    // More than there are: clamped; zero: nothing.
    await client.redoSteps(doc, 99);
    expect(await text(client, doc, sheet, 3)).toBe("3");
    const z = await client.undoSteps(doc, 0);
    expect(z.undo.canRedo).toBe(false);
    expect(await text(client, doc, sheet, 3)).toBe("3");
  });

  it("limits and trims the steps with the undo depth like AOO", async () => {
    const { client, doc } = await setup(10);
    await client.undoSteps(doc, 3);
    await client.setUndoDepth(6);
    // Alternately the farthest redo step and the oldest undo step go.
    expect(await client.undoHistory(doc)).toEqual({ undo: ["step 6", "step 5", "step 4", "step 3", "step 2"], redo: ["step 7"] });
    await expect(client.setUndoDepth(0)).rejects.toMatchObject({ code: "invalid_argument" });
    await expect(client.setUndoDepth(1001)).rejects.toMatchObject({ code: "invalid_argument" });
    await client.setUndoDepth(2);
    const info = await client.newWorkbook();
    for (let i = 0; i < 5; i++) await client.apply(info.doc, tx(set(info.activeSheet, 0, 0, String(i)), `t${i}`));
    expect((await client.undoHistory(info.doc)).undo).toEqual(["t4", "t3"]);
  });

  it("merges results", () => {
    const base = { version: 1, changed: [], layoutChanged: [], sheetsChanged: false, errors: [], undo: { canUndo: true, canRedo: false, undoLabel: null, redoLabel: null }, selection: null, info: null };
    const a = { ...base, changed: [{ sheet: 1, ranges: [{ r0: 0, c0: 0, r1: 0, c1: 0 }] }], layoutChanged: [1] };
    const b = { ...base, version: 2, changed: [{ sheet: 1, ranges: [{ r0: 5, c0: 0, r1: 5, c1: 0 }] }], layoutChanged: [1], objectsChanged: [3] };
    const m = mergeResults([a, b]);
    expect(m.version).toBe(2);
    expect(m.changed).toHaveLength(2);
    expect(m.layoutChanged).toEqual([1]);
    expect(m.objectsChanged).toEqual([3]);
  });
});
