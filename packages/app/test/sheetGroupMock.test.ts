/** Sheet groups and Fill Sheet in the browser mock (mirrors the engine's group.rs). */
import type { Op, Transaction } from "@tally/bridge";
import { describe, expect, it } from "vitest";
import { WebMockClient } from "../src/dev/webMock";

const tx = (group: number[] | null, ...ops: Op[]): Transaction => ({ ops, label: "Edit", selectionBefore: null, selectionAfter: null, group });
const set = (sheet: number, r: number, c: number, input: string): Op => ({ type: "setCellInput", sheet, r, c, input });

async function setup() {
  const client = new WebMockClient();
  const info = await client.newWorkbook();
  const res = await client.apply(info.doc, tx(null, { type: "addSheet", name: null, index: null }));
  const [a, b] = (res.info?.sheets ?? []).map((s) => s.id) as [number, number];
  return { client, doc: info.doc, a, b };
}

async function text(client: WebMockClient, doc: number, sheet: number, r: number, c: number) {
  const [block] = await client.cells(doc, sheet, [{ r0: r, c0: c, r1: r, c1: c }]);
  return block?.cells.find((v) => v.r === r && v.c === c)?.text ?? "";
}

describe("WebMockClient sheet groups", () => {
  it("types, inserts rows and undoes on every grouped sheet in one step", async () => {
    const { client, doc, a, b } = await setup();
    await client.apply(doc, tx([a, b], set(a, 0, 0, "7")));
    expect(await text(client, doc, b, 0, 0)).toBe("7");
    await client.apply(doc, tx([a, b], { type: "insertRows", sheet: a, at: 0, count: 1 }));
    expect(await text(client, doc, a, 1, 0)).toBe("7");
    expect(await text(client, doc, b, 1, 0)).toBe("7");
    await client.undo(doc);
    expect(await text(client, doc, a, 0, 0)).toBe("7");
    expect(await text(client, doc, b, 0, 0)).toBe("7");
    await client.undo(doc);
    expect(await text(client, doc, a, 0, 0)).toBe("");
    expect(await text(client, doc, b, 0, 0)).toBe("");
  });

  it("refuses the whole transaction when a grouped sheet is protected", async () => {
    const { client, doc, a, b } = await setup();
    await client.apply(doc, tx(null, { type: "protectSheet", sheet: b, password: null, options: null }));
    const res = await client.apply(doc, tx([a, b], set(a, 0, 0, "x"), set(a, 0, 1, "y")));
    expect(res.errors.map((e) => [e.opIndex, e.code])).toEqual([
      [0, "protected"],
      [1, "protected"],
    ]);
    expect(await text(client, doc, a, 0, 0)).toBe("");
    expect(res.changed).toEqual([]);
  });

  it("fills sheets, copying notes and refusing protected targets", async () => {
    const { client, doc, a, b } = await setup();
    await client.apply(doc, tx(null, set(a, 0, 0, "5"), { type: "setNote", sheet: a, r: 0, c: 0, note: { r: 0, c: 0, text: "n", author: null, created: null, shown: false } }));
    const fill: Op = { type: "fillSheets", sheet: a, ranges: [{ r0: 0, c0: 0, r1: 0, c1: 0 }], to: [b], options: { text: true, numbers: true, dateTime: true, formulas: true, notes: true, formats: true, operation: "none", skipEmpty: false } };
    await client.apply(doc, tx(null, fill));
    expect(await text(client, doc, b, 0, 0)).toBe("5");
    expect((await client.note(doc, b, 0, 0))?.text).toBe("n");
    await client.undo(doc);
    expect(await text(client, doc, b, 0, 0)).toBe("");
    expect(await client.note(doc, b, 0, 0)).toBeNull();
    await client.apply(doc, tx(null, { type: "protectSheet", sheet: b, password: null, options: null }));
    const res = await client.apply(doc, tx(null, fill));
    expect(res.errors[0]?.code).toBe("protected");
  });

  it("leaves hidden sheets (but group[0]) out and refuses unknown ids at op 0, like group_sheets", async () => {
    const { client, doc, a, b } = await setup();
    await client.apply(doc, tx(null, { type: "setSheetHidden", sheet: b, hidden: true }));
    const res = await client.apply(doc, tx([a, b, a], set(a, 0, 0, "x")));
    expect(res.errors).toEqual([]);
    expect(await text(client, doc, a, 0, 0)).toBe("x");
    expect(await text(client, doc, b, 0, 0)).toBe("");
    const bad = await client.apply(doc, tx([a, 999], set(a, 1, 0, "y"), set(a, 2, 0, "z")));
    expect(bad.errors).toEqual([expect.objectContaining({ opIndex: 0, code: "not_found" })]);
    expect(await text(client, doc, a, 1, 0)).toBe("");
  });

  it("keeps the target's notes when filling with an operation", async () => {
    const { client, doc, a, b } = await setup();
    const note = { r: 0, c: 0, text: "t", author: null, created: null, shown: false };
    await client.apply(doc, tx(null, set(a, 0, 0, "1"), set(b, 0, 0, "abc"), { type: "setNote", sheet: b, r: 0, c: 0, note }));
    const options = { text: true, numbers: true, dateTime: true, formulas: true, notes: true, formats: true, operation: "add" as const, skipEmpty: false };
    await client.apply(doc, tx(null, { type: "fillSheets", sheet: a, ranges: [{ r0: 0, c0: 0, r1: 0, c1: 0 }], to: [b], options }));
    expect((await client.note(doc, b, 0, 0))?.text).toBe("t");
    expect(await text(client, doc, b, 0, 0)).toBe("abc");
  });
});
