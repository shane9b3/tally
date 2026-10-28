import { existsSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { expandGroup, GROUP_OPS, groupSheets, MockClient, retarget, type FillSheetOptions, type Op, type Transaction } from "../src/index";

const tx = (group: number[] | null, ...ops: Op[]): Transaction => ({ ops, label: "Edit", selectionBefore: null, selectionAfter: null, group });
const set = (sheet: number, r: number, c: number, input: string): Op => ({ type: "setCellInput", sheet, r, c, input });

async function setup(sheets = 3) {
  const client = new MockClient();
  const info = await client.newWorkbook();
  let ids = info.sheets.map((s) => s.id);
  for (let i = 1; i < sheets; i++) {
    const res = await client.apply(info.doc, tx(null, { type: "addSheet", name: null, index: null }));
    ids = res.info?.sheets.map((s) => s.id) ?? ids;
  }
  return { client, doc: info.doc, ids };
}

async function text(client: MockClient, doc: number, sheet: number, r: number, c: number) {
  const [block] = await client.cells(doc, sheet, [{ r0: r, c0: c, r1: r, c1: c }]);
  return block?.cells[0]?.text ?? "";
}

describe("expandGroup", () => {
  it("repeats whitelisted ops on group[0] for each sheet, keeping the original index", () => {
    const ops = expandGroup(tx([2, 5], set(2, 0, 0, "x"), { type: "renameSheet", sheet: 2, name: "A" }, set(5, 1, 1, "y")));
    expect(ops.map((o) => [o.index, (o.op as { sheet: number }).sheet])).toEqual([
      [0, 2],
      [0, 5],
      [1, 2],
      [2, 5],
    ]);
  });

  it("passes ops through without a group or with a single sheet", () => {
    expect(expandGroup(tx(null, set(1, 0, 0, "x")))).toHaveLength(1);
    expect(expandGroup(tx([1], set(1, 0, 0, "x")))).toHaveLength(1);
  });

  it("retargets replaceAll's query sheet too", () => {
    const op: Op = { type: "replaceAll", sheet: 1, replace: "b", query: { text: "a", sheet: 1, inFormulas: false, matchCase: false, wholeCell: false, mode: "plain", from: null, inNotes: false, selection: null } as never };
    const out = retarget(op, 4) as Extract<Op, { type: "replaceAll" }>;
    expect(out.sheet).toBe(4);
    expect(out.query.sheet).toBe(4);
  });

  it("matches the engine's GROUP_OPS", () => {
    const path = fileURLToPath(new URL("../../../crates/tally-engine/src/group.rs", import.meta.url));
    if (!existsSync(path)) return; // engine chunk not landed yet
    const src = readFileSync(path, "utf8");
    const block = /pub const GROUP_OPS[^=]*=\s*&\[([\s\S]*?)\];/.exec(src)?.[1] ?? "";
    const rust = [...block.matchAll(/"([A-Za-z]+)"/g)].map((m) => m[1]!);
    const camel = rust.map((n) => n.charAt(0).toLowerCase() + n.slice(1));
    expect(camel.length).toBeGreaterThan(0);
    expect([...camel].sort()).toEqual([...GROUP_OPS].sort());
  });
});

describe("MockClient sheet groups", () => {
  it("applies a grouped input to every sheet in one undo step", async () => {
    const { client, doc, ids } = await setup();
    const [a, b, c] = ids as [number, number, number];
    await client.apply(doc, tx([a, c], set(a, 0, 0, "42")));
    expect(await text(client, doc, a, 0, 0)).toBe("42");
    expect(await text(client, doc, c, 0, 0)).toBe("42");
    expect(await text(client, doc, b, 0, 0)).toBe("");
    await client.undo(doc);
    expect(await text(client, doc, a, 0, 0)).toBe("");
    expect(await text(client, doc, c, 0, 0)).toBe("");
  });

  it("is all-or-nothing and keeps the original op index", async () => {
    const { client, doc, ids } = await setup(2);
    const [a, b] = ids as [number, number];
    const res = await client.apply(doc, tx([a, b], set(a, 0, 0, "1"), set(a, -1, 0, "bad")));
    expect(res.errors.map((e) => e.opIndex)).toEqual([1, 1]);
    expect(await text(client, doc, a, 0, 0)).toBe("");
    expect(await text(client, doc, b, 0, 0)).toBe("");
    expect(res.changed).toEqual([]);
  });

  it("fills sheets with values, formats, operations and skip-empty", async () => {
    const { client, doc, ids } = await setup(2);
    const [a, b] = ids as [number, number];
    await client.apply(doc, tx(null, set(a, 0, 0, "10"), set(a, 0, 1, "hi"), set(b, 0, 0, "5"), set(b, 0, 2, "keep"), { type: "applyStyle", sheet: a, ranges: [{ r0: 0, c0: 0, r1: 0, c1: 0 }], patch: { bold: true } as never }));
    const all: FillSheetOptions = { text: true, numbers: true, dateTime: true, formulas: true, notes: true, formats: true, operation: "none", skipEmpty: true };
    const fill = (options: FillSheetOptions): Op => ({ type: "fillSheets", sheet: a, ranges: [{ r0: 0, c0: 0, r1: 0, c1: 2 }], to: [b], options });
    await client.apply(doc, tx(null, fill(all)));
    expect(await text(client, doc, b, 0, 0)).toBe("10");
    expect(await text(client, doc, b, 0, 1)).toBe("hi");
    expect(await text(client, doc, b, 0, 2)).toBe("keep");
    const [block] = await client.cells(doc, b, [{ r0: 0, c0: 0, r1: 0, c1: 0 }]);
    expect(block?.cells[0]?.style).not.toBe(0);
    await client.apply(doc, tx(null, fill({ ...all, operation: "add" })));
    expect(await text(client, doc, b, 0, 0)).toBe("20");
    await client.apply(doc, tx(null, fill({ ...all, skipEmpty: false, text: false })));
    expect(await text(client, doc, b, 0, 1)).toBe("hi");
    expect(await text(client, doc, b, 0, 2)).toBe("keep");
    await client.apply(doc, tx(null, fill({ ...all, skipEmpty: false })));
    expect(await text(client, doc, b, 0, 2)).toBe("");
  });
});

describe("groupSheets (engine group_sheets)", () => {
  const sheets: Record<number, { hidden: boolean }> = { 1: { hidden: false }, 2: { hidden: true }, 3: { hidden: false } };
  const look = (id: number) => sheets[id] ?? null;
  it("drops duplicates and hidden sheets except group[0]; a group of one is no group", () => {
    expect(groupSheets(tx([1, 3, 1, 2]), look)).toEqual([1, 3]);
    expect(groupSheets(tx([2, 1]), look)).toEqual([2, 1]);
    expect(groupSheets(tx([1]), look)).toBeNull();
    expect(groupSheets(tx(null), look)).toBeNull();
    // Normalized to one sheet: still a group transaction, nothing repeats.
    expect(groupSheets(tx([1, 2]), look)).toEqual([1]);
    expect(expandGroup(tx([1, 2], set(1, 0, 0, "x")), [1])).toHaveLength(1);
  });
  it("refuses an unknown id with not_found", () => {
    expect(() => groupSheets(tx([1, 9]), look)).toThrow();
    try {
      groupSheets(tx([1, 9]), look);
    } catch (e) {
      expect((e as { code: string }).code).toBe("not_found");
    }
  });
  it("MockClient reports an unknown group id at op 0 and changes nothing", async () => {
    const { client, doc, ids } = await setup(2);
    const [a] = ids as [number, number];
    const res = await client.apply(doc, tx([a, 999], set(a, 0, 0, "x"), set(a, 0, 1, "y")));
    expect(res.errors).toEqual([expect.objectContaining({ opIndex: 0, code: "not_found" })]);
    expect(await text(client, doc, a, 0, 0)).toBe("");
  });
});

describe("Fill Sheet with an operation (engine paste_arithmetic parity)", () => {
  const all: FillSheetOptions = { text: true, numbers: true, dateTime: true, formulas: true, notes: true, formats: false, operation: "add", skipEmpty: false };
  // [source input, target input, operation, expected target input] as documented by clipboard.rs paste_arithmetic.
  const cases: [string, string, FillSheetOptions["operation"], string][] = [
    ["5", "abc", "add", "abc"], // text targets are kept
    ["5", "=2*3", "add", "=(2*3)+5"], // formula targets are wrapped
    ["5", "", "subtract", "-5"], // empty targets count as 0
    ["2", "4", "divide", "2"],
    ["0", "4", "divide", "#DIV/0!"],
    ["3", "4", "multiply", "12"],
    ["=2+3", "1", "add", "6"], // the source's value, not its formula
    ["x", "1", "add", "x"], // a non-number source is written as a constant
    ["1", "TRUE", "add", "2"], // booleans count as 1/0
    ["", "7", "add", "7"], // an empty source leaves the target
  ];
  it.each(cases)("source %j onto %j with %s gives %j", async (src, dst, operation, want) => {
    const { client, doc, ids } = await setup(2);
    const [a, b] = ids as [number, number];
    const ops: Op[] = [];
    if (src !== "") ops.push(set(a, 0, 0, src));
    if (dst !== "") ops.push(set(b, 0, 0, dst));
    await client.apply(doc, tx(null, ...ops));
    await client.apply(doc, tx(null, { type: "fillSheets", sheet: a, ranges: [{ r0: 0, c0: 0, r1: 0, c1: 0 }], to: [b], options: { ...all, operation } }));
    expect((await client.cellEditInfo(doc, b, 0, 0)).input).toBe(want);
  });
  it("keeps the target's notes unless replacing", async () => {
    const { client, doc, ids } = await setup(2);
    const [a, b] = ids as [number, number];
    const note = { r: 0, c: 0, text: "t", author: null, created: null, shown: false };
    await client.apply(doc, tx(null, set(a, 0, 0, "1"), { type: "setNote", sheet: b, r: 0, c: 0, note }));
    const fill = (o: Partial<FillSheetOptions>): Op => ({ type: "fillSheets", sheet: a, ranges: [{ r0: 0, c0: 0, r1: 0, c1: 0 }], to: [b], options: { ...all, ...o } });
    await client.apply(doc, tx(null, fill({ operation: "add" })));
    expect((await client.note(doc, b, 0, 0))?.text).toBe("t");
    await client.apply(doc, tx(null, fill({ operation: "none", skipEmpty: true })));
    expect((await client.note(doc, b, 0, 0))?.text).toBe("t");
    await client.apply(doc, tx(null, fill({ operation: "none", skipEmpty: false })));
    expect(await client.note(doc, b, 0, 0)).toBeNull();
  });
});
