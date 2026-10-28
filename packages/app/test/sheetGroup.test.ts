/**
 * Multi-sheet selection, UI side (chunk B): the pure helpers (sheetGroup.ts),
 * the docStore actions and pruning, `apply` attaching the group, the
 * gestures in the controller, the print range, the command guards and the
 * keys.
 */
import type { ApplyResult, DocId, Transaction } from "@tally/bridge";
import { beforeEach, describe, expect, it } from "vitest";
import { buildCommands, GROUP_DISABLED_COMMANDS, guardGroup } from "../src/commands";
import { AppController } from "../src/controller";
import { WebMockClient } from "../src/dev/webMock";
import { resolveKey } from "../src/keymap";
import { printOptionsFor } from "../src/print/printReducer";
import type { Command } from "../src/registry";
import { extendStep, moveBlockSteps, nextActiveAfterDelete, normalizeGroup, rangeBetween, stepVisible, toggle, txGroup } from "../src/sheetGroup";
import { useDocStore } from "../src/stores/docStore";
import { useUiStore } from "../src/stores/uiStore";
import { useDataStore } from "../src/stores/dataStore";
import { useW56Store } from "../src/data/store";

const tick = () => new Promise((r) => setTimeout(r, 0));

/** Sheets 1–5, 3 hidden. */
const SHEETS = [1, 2, 3, 4, 5].map((id) => ({ id, hidden: id === 3 }));

describe("sheetGroup helpers", () => {
  it("normalizes: tab order, visible only, active added, one sheet is no group", () => {
    expect(normalizeGroup(SHEETS, [5, 1], 2)).toEqual([1, 2, 5]);
    expect(normalizeGroup(SHEETS, [3, 9], 2)).toEqual([]);
    expect(normalizeGroup(SHEETS, [], 2)).toEqual([]);
    expect(normalizeGroup(SHEETS, [2], 2)).toEqual([]);
  });

  it("toggles a sheet but never the active or a hidden one", () => {
    expect(toggle(SHEETS, [], 1, 4)).toEqual([1, 4]);
    expect(toggle(SHEETS, [1, 4], 1, 4)).toEqual([]);
    expect(toggle(SHEETS, [1, 4], 1, 1)).toEqual([1, 4]);
    expect(toggle(SHEETS, [1, 4], 1, 3)).toEqual([1, 4]);
    expect(toggle(SHEETS, [1, 2, 4], 4, 2)).toEqual([1, 4]);
  });

  it("selects a range from the active sheet, skipping hidden sheets", () => {
    expect(rangeBetween(SHEETS, 2, 5)).toEqual([2, 4, 5]);
    expect(rangeBetween(SHEETS, 4, 1)).toEqual([1, 2, 4]);
    expect(rangeBetween(SHEETS, 2, 2)).toEqual([]);
    expect(rangeBetween(SHEETS, 2, 3)).toEqual([]);
  });

  it("steps over hidden sheets; extends without wrapping", () => {
    expect(stepVisible(SHEETS, 2, 1, false)).toBe(4);
    expect(stepVisible(SHEETS, 5, 1, false)).toBeNull();
    expect(stepVisible(SHEETS, 5, 1, true)).toBe(1);
    expect(extendStep(SHEETS, [], 2, 1)).toEqual({ active: 4, group: [2, 4] });
    expect(extendStep(SHEETS, [2, 4], 4, 1)).toEqual({ active: 5, group: [2, 4, 5] });
    expect(extendStep(SHEETS, [], 5, 1)).toBeNull();
    expect(extendStep(SHEETS, [], 1, -1)).toBeNull();
  });

  it("puts the active sheet first in the transaction group", () => {
    expect(txGroup([1, 2, 4], 2)).toEqual([2, 1, 4]);
    expect(txGroup([], 2)).toBeNull();
    expect(txGroup([1, 4], 2)).toBeNull();
  });

  it("picks the next visible survivor after a delete or hide", () => {
    expect(nextActiveAfterDelete(SHEETS, [1, 2], 2)).toBe(4);
    expect(nextActiveAfterDelete(SHEETS, [4, 5], 4)).toBe(2);
    expect(nextActiveAfterDelete(SHEETS, [4], 1)).toBe(1);
    expect(nextActiveAfterDelete(SHEETS, [1, 2, 4, 5], 1)).toBeNull();
  });

  it("moves a block of sheets in tab order", () => {
    const apply = (order: number[], steps: { sheet: number; to: number }[]) => {
      const cur = [...order];
      for (const s of steps) {
        cur.splice(cur.indexOf(s.sheet), 1);
        cur.splice(s.to, 0, s.sheet);
      }
      return cur;
    };
    const order = [1, 2, 3, 4, 5];
    expect(apply(order, moveBlockSteps(order, [2, 4], 5))).toEqual([1, 3, 5, 2, 4]);
    expect(apply(order, moveBlockSteps(order, [4, 2], 0))).toEqual([2, 4, 1, 3, 5]);
    expect(apply(order, moveBlockSteps(order, [5], 1))).toEqual([1, 5, 2, 3, 4]);
    expect(moveBlockSteps(order, [2, 3], 2)).toEqual([]);
  });
});

/** WebMockClient that records every transaction. */
class TxClient extends WebMockClient {
  readonly txs: Transaction[] = [];
  override async apply(doc: DocId, tx: Transaction): Promise<ApplyResult> {
    this.txs.push(structuredClone(tx));
    return super.apply(doc, tx);
  }
}

async function controller(sheets = 3) {
  const client = new TxClient();
  const ctl = new AppController(client, client, { nativeMenu: false });
  await ctl.start();
  for (let i = 1; i < sheets; i++) await ctl.addSheet();
  await tick();
  const ids = ctl.info!.sheets.map((s) => s.id);
  ctl.showSheet(ids[0]!, false);
  client.txs.length = 0;
  return { ctl, client, ids };
}

describe("docStore group and the controller gestures", () => {
  beforeEach(() => useDocStore.getState().reset());

  it("⌘-click toggles, Shift-click selects a run, a plain click collapses", async () => {
    const { ctl, ids } = await controller(4);
    const [a, b, c, d] = ids as [number, number, number, number];
    ctl.clickTab(c, { toggle: true });
    expect(useDocStore.getState().groupSheets).toEqual([a, c]);
    ctl.clickTab(a, { toggle: true });
    expect(useDocStore.getState().groupSheets).toEqual([a, c]);
    ctl.clickTab(d, { range: true });
    expect(useDocStore.getState().groupSheets).toEqual([a, b, c, d]);
    // A click inside the group makes it active and keeps the group.
    ctl.clickTab(c);
    expect(ctl.sheet).toBe(c);
    expect(useDocStore.getState().groupSheets).toEqual([a, b, c, d]);
    ctl.deselectAllSheets();
    ctl.clickTab(a, { toggle: true });
    expect(useDocStore.getState().groupSheets).toEqual([a, c]);
    ctl.clickTab(b);
    expect(ctl.sheet).toBe(b);
    expect(useDocStore.getState().groupSheets).toEqual([]);
  });

  it("⌃PgDn collapses; ⇧⌃PgDn extends", async () => {
    const { ctl, ids } = await controller(3);
    const [a, b, c] = ids as [number, number, number];
    ctl.selectStep(1);
    expect(ctl.sheet).toBe(b);
    expect(useDocStore.getState().groupSheets).toEqual([a, b]);
    ctl.selectStep(1);
    expect(useDocStore.getState().groupSheets).toEqual([a, b, c]);
    ctl.stepSheet(-1);
    expect(ctl.sheet).toBe(b);
    expect(useDocStore.getState().groupSheets).toEqual([]);
  });

  it("apply sends the group, the active sheet first", async () => {
    const { ctl, client, ids } = await controller(3);
    const [a, b, c] = ids as [number, number, number];
    await ctl.apply("Typing", [{ type: "setCellInput", sheet: a, r: 0, c: 0, input: "1" }]);
    expect(client.txs.at(-1)?.group ?? null).toBeNull();
    ctl.clickTab(c, { toggle: true });
    ctl.clickTab(c);
    useDocStore.getState().toggleSheet(b);
    await ctl.apply("Typing", [{ type: "setCellInput", sheet: c, r: 0, c: 0, input: "1" }]);
    expect(client.txs.at(-1)?.group).toEqual([c, a, b]);
  });

  it("prunes the group when sheets go away; deletes and hides the whole group", async () => {
    const { ctl, client, ids } = await controller(4);
    const [a, b, c, d] = ids as [number, number, number, number];
    useDocStore.getState().setGroup([a, b, c]);
    await ctl.deleteSelectedSheets();
    const tx = client.txs.at(-1)!;
    expect(tx.ops).toEqual([a, b, c].map((sheet) => ({ type: "deleteSheet", sheet })));
    // Sheet ops aren't group ops: explicit per-sheet ops, no group.
    expect(tx.group ?? null).toBeNull();
    expect(ctl.info!.sheets.map((s) => s.id)).toEqual([d]);
    expect(ctl.sheet).toBe(d);
    expect(useDocStore.getState().groupSheets).toEqual([]);
  });

  it("hides the group and shows the next sheet", async () => {
    const { ctl, client, ids } = await controller(3);
    const [a, b, c] = ids as [number, number, number];
    useDocStore.getState().setGroup([a, b]);
    await ctl.w56.hideSheet();
    expect(client.txs.at(-1)!.ops).toEqual([a, b].map((sheet) => ({ type: "setSheetHidden", sheet, hidden: true })));
    expect(ctl.sheet).toBe(c);
    expect(useDocStore.getState().groupSheets).toEqual([]);
  });

  it("colours every selected tab and moves the group as a block", async () => {
    const { ctl, client, ids } = await controller(4);
    const [a, b, c, d] = ids as [number, number, number, number];
    useDocStore.getState().setGroup([a, c]);
    await ctl.w56.setTabColor(a, 0xff0000ff);
    expect(client.txs.at(-1)!.ops.map((o) => ("sheet" in o ? o.sheet : null))).toEqual([a, c]);
    await ctl.dropSheets(a, 4);
    expect(ctl.info!.sheets.map((s) => s.id)).toEqual([b, d, a, c]);
  });
});

describe("print range with a group", () => {
  it('"selected" prints the selected sheets', () => {
    expect(printOptionsFor("selected", null, 2, null, [1, 2, 4]).sheets).toEqual({ type: "some", ids: [1, 2, 4] });
    expect(printOptionsFor("selected", null, 2, null, []).sheets).toEqual({ type: "active" });
    expect(printOptionsFor("all", null, 2, null, [1, 2]).sheets).toEqual({ type: "all" });
    expect(printOptionsFor("selection", null, 2, null, [1, 2]).sheets).toEqual({ type: "active" });
  });
});

describe("commands with several sheets selected", () => {
  const ui = { renameActiveSheet() {}, showAbout() {}, customNumberFormat() {}, resize() {} };

  it("lists every guarded command", () => {
    expect([...GROUP_DISABLED_COMMANDS].sort()).toEqual(
      [
        "chart.byColumn",
        "data.advancedFilter",
        "data.clearFilters",
        "data.consolidate",
        "data.createNamesFromLabels",
        "data.multipleOperations",
        "data.refreshFilter",
        "data.removeDuplicates",
        "data.resetFilter",
        "format.autoFormat",
        "format.colorByValue",
        "insert.hyperlink",
        "insert.removeHyperlink",
        "tools.goalSeek",
        "tools.solver",
        "data.dataPilot",
        "data.defineDatabaseRange",
        "data.defineName",
        "data.filter",
        "data.sort",
        "data.sortAsc",
        "data.sortDesc",
        "data.sortOptions",
        "data.standardFilter",
        "data.subtotals",
        "data.textToColumns",
        "filter.byColumn",
        "insert.chart",
        "insert.image",
        "sheet.insert",
        "sheet.rename",
        "tools.scenarios",
      ].sort(),
    );
    const ids = new Set(buildCommands({} as AppController, ui).map((c) => c.id));
    expect(GROUP_DISABLED_COMMANDS.filter((id) => !ids.has(id))).toEqual([]);
  });

  it("disables the guarded commands while grouped, with the reason", () => {
    let grouped = false;
    const cmds: Command[] = [
      { id: "data.sortAsc", title: "Sort", kind: "Sort", run: () => {} },
      { id: "data.recalc", title: "Recalc", kind: "Data", enabled: () => false, reason: "No", run: () => {} },
      { id: "data.filter", title: "Filter", kind: "Filter", enabled: () => true, reason: "own", run: () => {} },
    ];
    const [sort, clear, filter] = guardGroup(cmds, () => grouped) as [Command, Command, Command];
    expect(sort.enabled?.()).toBe(true);
    expect(filter.reason).toBe("own");
    grouped = true;
    expect(sort.enabled?.()).toBe(false);
    expect(sort.reason).toBe("Not available with several sheets selected");
    expect(filter.enabled?.()).toBe(false);
    expect(clear).toBe(cmds[1]);
  });

  it("enables Fill Sheets only when grouped and opens the fillSheet dialog", async () => {
    const { ctl, ids } = await controller(2);
    ctl.registry.registerAll(buildCommands(ctl, ui));
    expect(ctl.registry.isEnabled("edit.fillSheet")).toBe(false);
    expect(ctl.registry.isEnabled("data.sortAsc")).toBe(true);
    useDocStore.getState().setGroup(ids);
    expect(ctl.registry.isEnabled("edit.fillSheet")).toBe(true);
    expect(ctl.registry.isEnabled("data.sortAsc")).toBe(false);
    expect(ctl.registry.isEnabled("sheet.delete")).toBe(false);
    expect(ctl.registry.run("edit.fillSheet")).toBe(true);
    expect(useUiStore.getState().dialog).toEqual({ kind: "fillSheet" });
    useUiStore.getState().openDialog(null);
    ctl.registry.run("sheet.deselectAll");
    expect(useDocStore.getState().groupSheets).toEqual([]);
    ctl.registry.run("sheet.selectAll");
    expect(useDocStore.getState().groupSheets).toEqual(ids);
  });
});

describe("keys", () => {
  const key = (k: string, shift: boolean) => resolveKey({ key: k, metaKey: false, ctrlKey: true, shiftKey: shift, altKey: false }, { typing: false, nativeMenu: false });
  it("⇧⌃PgDn / ⇧⌃PgUp extend the selection; ⌃PgDn / ⌃PgUp switch", () => {
    expect(key("PageDown", true)).toBe("sheet.selectNext");
    expect(key("PageUp", true)).toBe("sheet.selectPrev");
    expect(key("PageDown", false)).toBe("sheet.next");
  });
});

describe("which transactions carry the group (review fixes)", () => {
  beforeEach(() => useDocStore.getState().reset());
  const ui = { renameActiveSheet() {}, showAbout() {}, customNumberFormat() {}, resize() {} };
  const set = (sheet: number, r: number, c: number, input: string) => ({ type: "setCellInput" as const, sheet, r, c, input });
  const text = async (client: TxClient, ctl: AppController, sheet: number, r: number, c: number) => {
    const [block] = await client.cells(ctl.info!.doc, sheet, [{ r0: r, c0: c, r1: r, c1: c }]);
    return block?.cells.find((v) => v.r === r && v.c === c)?.text ?? "";
  };

  it("only group ops on the active sheet; never protection, sheet ops or opt-outs", async () => {
    const { ctl, client, ids } = await controller(3);
    const [a, b] = ids as [number, number];
    useDocStore.getState().setGroup([a, b]);
    await ctl.apply("x", [set(a, 0, 0, "1")]);
    expect(client.txs.at(-1)!.group).toEqual([a, b]);
    await ctl.apply("x", [set(a, 0, 0, "1")], undefined, false, { group: false });
    expect(client.txs.at(-1)!.group ?? null).toBeNull();
    await ctl.apply("x", [set(b, 0, 0, "1")]);
    expect(client.txs.at(-1)!.group ?? null).toBeNull();
    await ctl.apply("x", [{ type: "setTabColor", sheet: a, color: null }]);
    expect(client.txs.at(-1)!.group ?? null).toBeNull();
    await ctl.apply("x", [{ type: "protectSheet", sheet: a, password: null, options: null }]);
    expect(client.txs.at(-1)!.group ?? null).toBeNull();
    await ctl.apply("x", [{ type: "unprotectSheet", sheet: a, password: null }]);
    expect(client.txs.at(-1)!.group ?? null).toBeNull();
  });

  it("H1: merge asks when any selected sheet hides content and moves each sheet's own text, without the group", async () => {
    const { ctl, client, ids } = await controller(2);
    const [a, b] = ids as [number, number];
    await ctl.apply("seed", [set(a, 0, 0, "a1"), set(b, 0, 0, "b1"), set(b, 0, 1, "b2")], undefined, false, { group: false });
    useDocStore.getState().setGroup([a, b]);
    const range = { r0: 0, c0: 0, r1: 0, c1: 1 };
    const { useSelectionStore } = await import("../src/stores/selectionStore");
    useSelectionStore.getState().setSelection({ range, active: { r: 0, c: 0 }, anchor: { r: 0, c: 0 } });
    await ctl.mergeCells();
    const d = useW56Store.getState().dialog;
    expect(d?.kind).toBe("mergePrompt");
    client.txs.length = 0;
    await ctl.w56.mergeMoving(range, new Map([[a, [{ r: 0, c: 0, text: "a1" }]], [b, [{ r: 0, c: 0, text: "b1" }, { r: 0, c: 1, text: "b2" }]]]) as never);
    const tx = client.txs.at(-1)!;
    expect(tx.group ?? null).toBeNull();
    expect(tx.ops.map((o) => [o.type, (o as { sheet: number }).sheet])).toEqual([
      ["setRangeInputs", a],
      ["mergeCells", a],
      ["setRangeInputs", b],
      ["mergeCells", b],
    ]);
    expect(await text(client, ctl, a, 0, 0)).toBe("a1");
    expect(await text(client, ctl, b, 0, 0)).toMatch(/^b1\sb2$/);
    useW56Store.getState().openDialog(null);
  });

  it("H2/M1: existing CF and validity rules aren't edited by id while grouped; new ones are group edits", async () => {
    const { ctl, client, ids } = await controller(2);
    const [a, b] = ids as [number, number];
    useDocStore.getState().setGroup([a, b]);
    client.txs.length = 0;
    expect(await ctl.v3.updateConditionalFormat(1, [], [])).toBeNull();
    expect(await ctl.v3.deleteConditionalFormat(1)).toBeNull();
    expect(await ctl.v3.reorderConditionalFormats([])).toBeNull();
    await ctl.v3.clearValidation(1);
    expect(client.txs).toEqual([]);
    await ctl.v3.addConditionalFormat([{ r0: 0, c0: 0, r1: 1, c1: 1 }], [], "CF");
    expect(client.txs.at(-1)!.group).toEqual([a, b]);
  });

  it("M2/M3, L4: hyperlink, solver, goal seek and the rest are off while grouped", async () => {
    const { ctl, ids } = await controller(2);
    ctl.registry.registerAll(buildCommands(ctl, ui));
    for (const id of ["insert.hyperlink", "tools.solver", "tools.goalSeek", "format.autoFormat", "data.removeDuplicates", "data.createNamesFromLabels", "data.multipleOperations", "data.resetFilter"]) expect(ctl.registry.isEnabled(id), id).toBe(true);
    useDocStore.getState().setGroup(ids);
    for (const id of ["insert.hyperlink", "tools.solver", "tools.goalSeek", "format.autoFormat", "data.removeDuplicates", "data.createNamesFromLabels", "data.multipleOperations", "data.resetFilter"]) expect(ctl.registry.isEnabled(id), id).toBe(false);
  });

  it("L1: Delete keeps a visible sheet when the others are hidden", async () => {
    const { ctl, ids } = await controller(3);
    ctl.registry.registerAll(buildCommands(ctl, ui));
    const [a, b, c] = ids as [number, number, number];
    await ctl.apply("hide", [{ type: "setSheetHidden", sheet: c, hidden: true }]);
    useDocStore.getState().setGroup([a, b]);
    expect(ctl.registry.isEnabled("sheet.delete")).toBe(false);
    expect(await ctl.deleteSelectedSheets()).toBeNull();
  });

  it("L2: Replace (one cell) goes without the group; Find searches the selected sheets", async () => {
    const { ctl, client, ids } = await controller(3);
    const [a, b, c] = ids as [number, number, number];
    await ctl.apply("seed", [set(a, 0, 0, "x"), set(b, 0, 0, "x"), set(c, 0, 0, "x")], undefined, false, { group: false });
    useDocStore.getState().setGroup([a, b]);
    client.txs.length = 0;
    await ctl.replaceOne("x", "y");
    const replace = client.txs.find((tx) => tx.ops.some((o) => o.type === "replaceAll"))!;
    expect(replace.group ?? null).toBeNull();
    expect(await text(client, ctl, b, 0, 0)).toBe("x");
    useDataStore.getState().patchFind({ text: "x", searched: null, hits: [], index: -1 });
    await ctl.findStep(1);
    const hits = useDataStore.getState().find.hits;
    expect(hits.length).toBeGreaterThan(0);
    expect(hits.every((h) => h.sheet === a || h.sheet === b)).toBe(true);
  });

  it("a cut pasted from another sheet is a move without the group", async () => {
    const { ctl, client, ids } = await controller(2);
    const [a, b] = ids as [number, number];
    useDocStore.getState().setGroup([a, b]);
    client.txs.length = 0;
    useDataStore.getState().setClip({ sheet: b, range: { r0: 0, c0: 0, r1: 0, c1: 0 }, token: "tok", cut: true });
    await (ctl as unknown as { pasteNow: (t: string, tok: string, v: boolean, r: object) => Promise<unknown> }).pasteNow("", "tok", false, { r0: 2, c0: 2, r1: 2, c1: 2 });
    const tx = client.txs.at(-1)!;
    expect(tx.ops.map((o) => o.type)).toEqual(["paste", "clear"]);
    expect(tx.group ?? null).toBeNull();
  });

  it("L4: Move sheet left/right moves the group as a block; L6: copies become the selection", async () => {
    const { ctl, ids } = await controller(4);
    const [a, b, c, d] = ids as [number, number, number, number];
    useDocStore.getState().setGroup([a, c]);
    await ctl.moveSheetBy(1);
    expect(ctl.info!.sheets.map((s) => s.id)).toEqual([b, a, c, d]);
    await ctl.moveSheetBy(1);
    expect(ctl.info!.sheets.map((s) => s.id)).toEqual([b, d, a, c]);
    await ctl.moveSheetBy(1);
    expect(ctl.info!.sheets.map((s) => s.id)).toEqual([b, d, a, c]);
    await ctl.moveSheetBy(-1);
    expect(ctl.info!.sheets.map((s) => s.id)).toEqual([b, a, c, d]);
    await ctl.w56.copySheets([a, c], null);
    const order = ctl.info!.sheets.map((s) => s.id);
    expect(order).toHaveLength(6);
    const copies = order.filter((id) => !ids.includes(id));
    expect(useDocStore.getState().groupSheets).toEqual(copies);
    expect(ctl.sheet).toBe(copies[0]);
  });

  it("L5: Fill Sheets is off with a multiple selection", async () => {
    const { ctl, ids } = await controller(2);
    ctl.registry.registerAll(buildCommands(ctl, ui));
    useDocStore.getState().setGroup(ids);
    const { useSelectionStore } = await import("../src/stores/selectionStore");
    const sel = useSelectionStore.getState().selection;
    expect(ctl.registry.isEnabled("edit.fillSheet")).toBe(true);
    useSelectionStore.getState().setSelection({ ...sel, others: [{ r0: 5, c0: 5, r1: 6, c1: 6 }] });
    expect(ctl.ranges.length).toBe(2);
    expect(ctl.registry.isEnabled("edit.fillSheet")).toBe(false);
    expect(ctl.registry.get("edit.fillSheet")?.reason).toBe("This function cannot be used with multiple selections");
  });
});
