import { describe, expect, it, vi } from "vitest";
import { MockClient, type EngineClient, type RangeDto } from "@tally/bridge";
import { EngineClientSource } from "../src/engineSource";
import type { DirtySet } from "../src/types";

const flush = () => new Promise((r) => setTimeout(r, 0));

async function setup() {
  const client = new MockClient();
  const info = await client.newWorkbook(null);
  const sheet = info.activeSheet;
  await client.apply(info.doc, {
    ops: [{ type: "setRangeInputs", sheet, r: 0, c: 0, rows: [["1", "2", "=A1+B1"], ["x", "", "TRUE"]] }],
    label: null,
    selectionBefore: null,
    selectionAfter: null,
  });
  const source = new EngineClientSource(client, info.doc, sheet);
  const events: DirtySet[] = [];
  source.subscribe((d) => events.push(d));
  return { client, doc: info.doc, sheet, source, events };
}

describe("EngineClientSource", () => {
  it("fetches tiles, then serves cells synchronously", async () => {
    const { source, events } = await setup();
    expect(source.getCell(0, 2)).toBeUndefined();
    source.ensureRanges([{ r0: 0, c0: 0, r1: 63, c1: 31 }]);
    await flush();
    expect(source.getCell(0, 2)?.text).toBe("3");
    expect(source.getCell(0, 2)?.kind).toBe("number");
    expect(source.getCell(1, 0)?.text).toBe("x");
    expect(source.getCell(1, 1)).toBeUndefined();
    expect(events.some((e) => e !== "all" && e.some((r) => r.r0 === 0 && r.c0 === 0))).toBe(true);
    expect(source.getStyle(0)?.id).toBe(0);
  });

  it("is idempotent for fresh tiles and batches ≤ 8 tiles per call", async () => {
    const { client, doc, sheet } = await setup();
    const spy = vi.spyOn(client, "cells");
    const source = new EngineClientSource(client, doc, sheet);
    const ranges: RangeDto[] = [];
    for (let i = 0; i < 10; i++) ranges.push({ r0: i * 64, c0: 0, r1: i * 64 + 63, c1: 31 });
    source.ensureRanges(ranges);
    expect(spy).toHaveBeenCalledTimes(2);
    expect(spy.mock.calls[0]?.[2].length).toBe(8);
    await flush();
    source.ensureRanges(ranges);
    expect(spy).toHaveBeenCalledTimes(2);
  });

  it("applies ApplyResult.changed: stale tiles keep old data until refetched", async () => {
    const { client, doc, sheet, source, events } = await setup();
    source.ensureRanges([{ r0: 0, c0: 0, r1: 63, c1: 31 }]);
    await flush();
    const res = await client.apply(doc, {
      ops: [{ type: "setCellInput", sheet, r: 0, c: 0, input: "10" }],
      label: null,
      selectionBefore: null,
      selectionAfter: null,
    });
    events.length = 0;
    source.applyResult(res);
    expect(events.length).toBeGreaterThan(0);
    expect(source.getCell(0, 2)?.text).toBe("3"); // old value still shown
    source.ensureRanges([{ r0: 0, c0: 0, r1: 63, c1: 31 }]);
    await flush();
    expect(source.getCell(0, 0)?.text).toBe("10");
  });

  it("drops responses for tiles that changed while in flight", async () => {
    const { client, doc, sheet } = await setup();
    let release: (() => void) | null = null;
    const slow: EngineClient = Object.create(client) as EngineClient;
    slow.cells = (d, s, r) =>
      new Promise((resolve) => {
        release = () => void client.cells(d, s, r).then(resolve);
      });
    const source = new EngineClientSource(slow, doc, sheet, { listenToDocChanged: false });
    source.ensureRanges([{ r0: 0, c0: 0, r1: 63, c1: 31 }]);
    source.applyResult({
      version: 1,
      changed: [{ sheet, ranges: [{ r0: 0, c0: 0, r1: 0, c1: 0 }] }],
      layoutChanged: [],
      sheetsChanged: false,
      errors: [],
      undo: { canUndo: false, canRedo: false, undoLabel: null, redoLabel: null },
      selection: null,
      info: null,
    });
    (release as (() => void) | null)?.();
    await flush();
    await flush();
    expect(source.getCell(0, 0)).toBeUndefined();
  });

  it("refetches the layout when it changes", async () => {
    const { client, doc, sheet, source, events } = await setup();
    await flush();
    expect(source.getLayout(sheet).colRuns).toEqual([]);
    const res = await client.apply(doc, {
      ops: [{ type: "setColWidth", sheet, start: 2, end: 3, px: 150 }],
      label: null,
      selectionBefore: null,
      selectionAfter: null,
    });
    events.length = 0;
    source.applyResult(res);
    await flush();
    await flush();
    expect(source.getLayout(sheet).colRuns).toEqual([{ start: 2, end: 3, size: 150, hidden: false }]);
    expect(events).toContain("all");
  });

  it("evicts beyond capacity and tells the grid", async () => {
    const { client, doc, sheet } = await setup();
    const source = new EngineClientSource(client, doc, sheet, { capacity: 4 });
    const events: DirtySet[] = [];
    source.subscribe((d) => events.push(d));
    for (let i = 0; i < 8; i++) {
      source.ensureRanges([{ r0: i * 64, c0: 0, r1: i * 64 + 63, c1: 31 }]);
      await flush();
    }
    expect(source.getCell(0, 2)).toBeUndefined();
    expect(events.some((e) => e !== "all" && e.some((r) => r.r0 === 0 && r.c0 === 0 && r.r1 === 63))).toBe(true);
  });
});
