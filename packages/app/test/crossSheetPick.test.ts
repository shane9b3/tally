/**
 * Picking references on other sheets while a formula is edited (AOO's
 * reference input with ScTabView::SetTabNo): the edit stays open, picks
 * name the sheet, Shift+tab extends to 3D, and the commit (or Escape) goes
 * back to the edited cell's sheet.
 */
import type { CellPos } from "@tally/bridge";
import { cellSelection } from "@tally/grid";
import { describe, expect, it } from "vitest";
import { AppController } from "../src/controller";
import { WebMockClient } from "../src/dev/webMock";
import { formulaHighlights, refOnSheet } from "../src/formulaEdit";
import { useDocStore } from "../src/stores/docStore";
import { useEditStore } from "../src/stores/editStore";
import { useSelectionStore } from "../src/stores/selectionStore";
import { buildCommands } from "../src/commands";
import type { ApplyResult, DocId, Transaction } from "@tally/bridge";

const tick = () => new Promise((r) => setTimeout(r, 0));
const one = (c: CellPos) => ({ r0: c.r, c0: c.c, r1: c.r, c1: c.c });

/** A blank workbook with three sheets (Sheet1..Sheet3), Sheet1 shown. */
async function setup() {
  const client = new WebMockClient();
  const ctl = new AppController(client, client, { nativeMenu: false });
  await ctl.start(null);
  await ctl.addSheet();
  await ctl.addSheet();
  const ids = (ctl.info?.sheets ?? []).map((s) => s.id);
  ctl.showSheet(ids[0]!, false);
  await tick();
  return { client, ctl, ids: ids as [number, number, number] };
}

async function input(ctl: AppController, client: WebMockClient, sheet: number, cell: CellPos) {
  return (await client.cellEditInfo(ctl.info!.doc, sheet, cell.r, cell.c)).input;
}

/** Types `text` into `cell` of the shown sheet (an in-cell edit). */
function type(ctl: AppController, cell: CellPos, text: string) {
  useSelectionStore.getState().setSelection(cellSelection(cell));
  ctl.onGridStartEdit(cell, text);
}

function append(ctl: AppController, s: string) {
  const e = useEditStore.getState().edit!;
  ctl.onEditChange(e.text + s, e.text.length + s.length);
}

describe("references picked on other sheets", () => {
  it("keeps the edit, names the sheet and commits into the original cell", async () => {
    const { ctl, client, ids } = await setup();
    const [s1, s2] = ids;
    await client.apply(ctl.info!.doc, { ops: [{ type: "setCellInput", sheet: s2, r: 1, c: 1, input: "5" }, { type: "setCellInput", sheet: s1, r: 2, c: 2, input: "7" }], label: "x", selectionBefore: null, selectionAfter: null });
    type(ctl, { r: 0, c: 0 }, "=");
    ctl.clickTab(s2);
    expect(ctl.sheet).toBe(s2);
    expect(useEditStore.getState().edit?.text).toBe("=");
    expect(useEditStore.getState().sheet).toBe(s1);
    expect(ctl.editingElsewhere).toBe(true);
    ctl.onPointRange(one({ r: 1, c: 1 }), "start");
    expect(useEditStore.getState().edit?.text).toBe("=Sheet2.B2");
    append(ctl, "+");
    ctl.clickTab(s1);
    expect(ctl.sheet).toBe(s1);
    expect(ctl.editingElsewhere).toBe(false);
    ctl.onPointRange(one({ r: 2, c: 2 }), "start");
    expect(useEditStore.getState().edit?.text).toBe("=Sheet2.B2+C3");
    ctl.clickTab(s2);
    expect(await ctl.commitEdit("down")).toBe(true);
    expect(ctl.sheet).toBe(s1);
    expect(useEditStore.getState().edit).toBeNull();
    expect(useSelectionStore.getState().selection.active).toEqual({ r: 1, c: 0 });
    expect(await input(ctl, client, s1, { r: 0, c: 0 })).toBe("=Sheet2.B2+C3");
    const [block] = await client.cells(ctl.info!.doc, s1, [one({ r: 0, c: 0 })]);
    expect(block?.cells[0]?.text).toBe("12");
    expect(await input(ctl, client, s2, { r: 0, c: 0 })).toBe("");
    // One undo step.
    await ctl.undo();
    expect(await input(ctl, client, s1, { r: 0, c: 0 })).toBe("");
  });

  it("Escape on another sheet returns without changing the cell", async () => {
    const { ctl, client, ids } = await setup();
    const [s1, , s3] = ids;
    type(ctl, { r: 4, c: 3 }, "=");
    ctl.clickTab(s3);
    ctl.onPointRange(one({ r: 0, c: 0 }), "start");
    ctl.cancelEdit();
    expect(ctl.sheet).toBe(s1);
    expect(useSelectionStore.getState().selection.active).toEqual({ r: 4, c: 3 });
    expect(useEditStore.getState().edit).toBeNull();
    expect(await input(ctl, client, s1, { r: 4, c: 3 })).toBe("");
  });

  it("Shift+click on a tab extends the last pick to a 3D reference", async () => {
    const { ctl, ids } = await setup();
    const [, s2, s3] = ids;
    type(ctl, { r: 5, c: 0 }, "=SUM(");
    ctl.onPointRange(one({ r: 0, c: 0 }), "start");
    expect(useEditStore.getState().edit?.text).toBe("=SUM(A1");
    ctl.clickTab(s3, { range: true });
    expect(ctl.sheet).toBe(s3);
    expect(useEditStore.getState().edit?.text).toBe("=SUM(Sheet1.A1:Sheet3.A1");
    // Further picks on the shown sheet replace it with a plain reference there.
    ctl.onPointRange(one({ r: 1, c: 1 }), "start");
    expect(useEditStore.getState().edit?.text).toBe("=SUM(Sheet3.B2");
    // ⌘-click is ignored (no group change), the view moves.
    ctl.clickTab(s2, { toggle: true });
    expect(ctl.sheet).toBe(s2);
    expect(useDocStore.getState().groupSheets).toEqual([]);
  });

  it("typing : then picking on another sheet builds a 3D range", async () => {
    const { ctl, ids } = await setup();
    const [, , s3] = ids;
    type(ctl, { r: 5, c: 0 }, "=SUM(");
    ctl.onPointRange(one({ r: 0, c: 0 }), "start");
    append(ctl, ":");
    ctl.clickTab(s3);
    ctl.onPointRange(one({ r: 0, c: 0 }), "start");
    expect(useEditStore.getState().edit?.text).toBe("=SUM(A1:Sheet3.A1");
  });

  it("Ctrl+PgDn steps the view while picking; plain text commits on a tab click", async () => {
    const { ctl, client, ids } = await setup();
    const [s1, s2] = ids;
    type(ctl, { r: 0, c: 0 }, "=");
    const key = { key: "PageDown", metaKey: false, ctrlKey: true, shiftKey: false, altKey: false };
    expect(ctl.handleEditorKey(key)).toBe(true);
    expect(ctl.sheet).toBe(s2);
    expect(useEditStore.getState().edit?.text).toBe("=");
    ctl.cancelEdit();
    expect(ctl.sheet).toBe(s1);

    type(ctl, { r: 0, c: 1 }, "hello");
    ctl.clickTab(s2);
    expect(useEditStore.getState().edit).toBeNull();
    await tick();
    await tick();
    expect(ctl.sheet).toBe(s2);
    expect(await input(ctl, client, s1, { r: 0, c: 1 })).toBe("hello");
  });

  it("keeps a sheet group for the commit", async () => {
    const { ctl, client, ids } = await setup();
    const [s1, s2, s3] = ids;
    useDocStore.getState().setGroup([s1, s2]);
    type(ctl, { r: 0, c: 0 }, "=");
    ctl.clickTab(s3);
    ctl.onPointRange(one({ r: 0, c: 0 }), "start");
    expect(useDocStore.getState().groupSheets).toEqual([s1, s2]);
    await ctl.commitEdit("none");
    expect(ctl.sheet).toBe(s1);
    expect(useDocStore.getState().groupSheets).toEqual([s1, s2]);
    expect(await input(ctl, client, s1, { r: 0, c: 0 })).toBe("=Sheet3.A1");
    expect(await input(ctl, client, s2, { r: 0, c: 0 })).toBe("=Sheet3.A1");
  });
});

describe("reference outlines across sheets", () => {
  const order = ["Sheet1", "Sheet2", "Sheet3"];
  it("unqualified references belong to the edited cell's sheet", () => {
    const r = { sheet: null, sheet2: null };
    expect(refOnSheet(r, "Sheet1", { editSheet: "Sheet1" })).toBe(true);
    expect(refOnSheet(r, "Sheet2", { editSheet: "Sheet1", order })).toBe(false);
    expect(refOnSheet(r, "Sheet2")).toBe(true);
  });
  it("qualified and 3D references", () => {
    expect(refOnSheet({ sheet: "sheet2", sheet2: null }, "Sheet2", { editSheet: "Sheet1" })).toBe(true);
    expect(refOnSheet({ sheet: "Sheet1", sheet2: "Sheet3" }, "Sheet2", { editSheet: "Sheet1", order })).toBe(true);
    expect(refOnSheet({ sheet: null, sheet2: "Sheet3" }, "Sheet2", { editSheet: "Sheet1", order })).toBe(true);
    expect(refOnSheet({ sheet: "Sheet2", sheet2: "Sheet3" }, "Sheet1", { editSheet: "Sheet1", order })).toBe(false);
  });
  it("draws only the shown sheet's references", () => {
    const hl = formulaHighlights("=A1+Sheet2.B2", "Sheet2", "light", { editSheet: "Sheet1", order });
    expect(hl.spans).toHaveLength(2);
    expect(hl.refs.map((r) => r.range)).toEqual([{ r0: 1, c0: 1, r1: 1, c1: 1 }]);
  });
});

describe("Shift+click on another sheet's cell (AOO UpdateRef across tabs)", () => {
  it("extends the earlier pick to a 3D reference", async () => {
    const { ctl, ids } = await setup();
    const [, , s3] = ids;
    type(ctl, { r: 5, c: 0 }, "=SUM(");
    ctl.onPointRange(one({ r: 0, c: 0 }), "start");
    ctl.onPointRange(one({ r: 0, c: 0 }), "end");
    ctl.clickTab(s3);
    ctl.onPointRange(one({ r: 1, c: 1 }), "start", true);
    expect(useEditStore.getState().edit?.text).toBe("=SUM(Sheet1.A1:Sheet3.B2");
    ctl.onPointRange({ r0: 1, c0: 1, r1: 2, c1: 1 }, "end");
    expect(useEditStore.getState().edit?.text).toBe("=SUM(Sheet1.A1:Sheet3.B3");
    // A plain click afterwards picks on the shown sheet only.
    ctl.onPointRange(one({ r: 3, c: 3 }), "start");
    expect(useEditStore.getState().edit?.text).toBe("=SUM(Sheet3.D4");
  });
});

/** Starts `=` on Sheet1.A1 (or `cell`) and shows Sheet2 for picking. */
function pickingOn(ctl: AppController, ids: number[], cell: CellPos = { r: 0, c: 0 }) {
  type(ctl, cell, "=");
  ctl.clickTab(ids[1]!);
  expect(ctl.editingElsewhere).toBe(true);
}

describe("nothing acts on the shown sheet while picking there (review)", () => {
  it("⌘⇧↩ from another sheet ignores the shown sheet's array block", async () => {
    const { ctl, client, ids } = await setup();
    const [s1, s2] = ids;
    pickingOn(ctl, ids);
    ctl.onPointRange(one({ r: 9, c: 9 }), "start");
    // The shown sheet's active cell sits in an array block A1:C5.
    useEditStore.getState().setActive({ sheet: s2, cell: { r: 0, c: 0 }, input: "{=1}", hint: null, arrayRange: { r0: 0, c0: 0, r1: 4, c1: 2 } });
    const before = client.recorded.length;
    await ctl.commitArray();
    const ops = client.recorded.slice(before);
    expect(ops).toHaveLength(1);
    expect(ops[0]).toMatchObject({ type: "setArrayFormula", sheet: s1, range: { r0: 0, c0: 0, r1: 0, c1: 0 } });
    expect(ctl.sheet).toBe(s1);
  });

  it("commands are disabled, apply refuses and Add up / Bold / a palette function do nothing", async () => {
    const { ctl, client, ids } = await setup();
    const [s1, s2] = ids;
    ctl.registry.registerAll(buildCommands(ctl, { renameActiveSheet() {}, showAbout() {}, customNumberFormat() {}, resize() {} }));
    await client.apply(ctl.info!.doc, { ops: [{ type: "setCellInput", sheet: s2, r: 0, c: 0, input: "4" }, { type: "setCellInput", sheet: s2, r: 1, c: 0, input: "5" }], label: "x", selectionBefore: null, selectionAfter: null });
    expect(ctl.registry.isEnabled("format.bold")).toBe(true);
    pickingOn(ctl, ids, { r: 2, c: 0 });
    expect(ctl.registry.isEnabled("format.bold")).toBe(false);
    expect(ctl.registry.reasonFor("format.bold")).toBe("Finish editing the cell first");
    expect(ctl.registry.isEnabled("edit.undo")).toBe(true);
    expect(ctl.registry.run("format.bold")).toBe(false);
    const before = client.recorded.length;
    await ctl.addUp();
    expect(await ctl.apply("Bold", [{ type: "setCellInput", sheet: s2, r: 2, c: 0, input: "x" }])).toBeNull();
    expect(client.recorded.length).toBe(before);
    expect(await input(ctl, client, s2, { r: 2, c: 0 })).toBe("");
    // The palette's function item starts an edit: refused, the formula stays.
    ctl.startEdit({ r: 2, c: 0 }, "=SUM(", "grid");
    expect(useEditStore.getState().edit?.text).toBe("=");
    expect(useEditStore.getState().sheet).toBe(s1);
    ctl.cancelEdit();
    expect(ctl.registry.isEnabled("format.bold")).toBe(true);
  });

  it("deleting the edited cell's sheet cancels the edit", async () => {
    const { ctl, client, ids } = await setup();
    const [s1, s2] = ids;
    pickingOn(ctl, ids);
    const res = await client.apply(ctl.info!.doc, { ops: [{ type: "deleteSheet", sheet: s1 }], label: "x", selectionBefore: null, selectionAfter: null });
    ctl.handleResult(res);
    expect(useEditStore.getState().edit).toBeNull();
    expect(ctl.editingElsewhere).toBe(false);
    expect(ctl.info!.sheets.some((s) => s.id === ctl.sheet)).toBe(true);
    expect(ctl.sheet).toBe(s2);
  });

  it("deleting the shown sheet goes back home with the edit open; a rename keeps picking", async () => {
    const { ctl, client, ids } = await setup();
    const [s1, s2, s3] = ids;
    pickingOn(ctl, ids, { r: 4, c: 1 });
    let res = await client.apply(ctl.info!.doc, { ops: [{ type: "renameSheet", sheet: s3, name: "Other" }], label: "x", selectionBefore: null, selectionAfter: null });
    ctl.handleResult(res);
    expect(ctl.sheet).toBe(s2);
    expect(useEditStore.getState().edit?.text).toBe("=");
    res = await client.apply(ctl.info!.doc, { ops: [{ type: "deleteSheet", sheet: s2 }], label: "x", selectionBefore: null, selectionAfter: null });
    ctl.handleResult(res);
    expect(ctl.sheet).toBe(s1);
    expect(useEditStore.getState().edit?.text).toBe("=");
    expect(useSelectionStore.getState().selection.active).toEqual({ r: 4, c: 1 });
  });

  it("Enter from another sheet steps within the edited cell's multi-cell selection", async () => {
    const { ctl, ids } = await setup();
    const [s1, s2] = ids;
    useSelectionStore.getState().setSelection({ range: { r0: 0, c0: 0, r1: 1, c1: 1 }, active: { r: 1, c: 0 }, anchor: { r: 0, c: 0 } });
    ctl.onGridStartEdit({ r: 1, c: 0 }, "=");
    ctl.clickTab(s2);
    ctl.onPointRange(one({ r: 0, c: 0 }), "start");
    await ctl.commitEdit("down");
    expect(ctl.sheet).toBe(s1);
    const sel = useSelectionStore.getState().selection;
    expect(sel.range).toEqual({ r0: 0, c0: 0, r1: 1, c1: 1 });
    expect(sel.active).toEqual({ r: 0, c: 1 });
  });
});

describe("a refused commit keeps the sheet", () => {
  it("plain text refused by the engine: the tab click does not switch", async () => {
    class Refusing extends WebMockClient {
      override async apply(doc: DocId, tx: Transaction): Promise<ApplyResult> {
        const res = await super.apply(doc, tx);
        if (tx.ops[0]?.type === "setCellInput" && (tx.ops[0] as { input: string }).input === "no") return { ...res, errors: [{ opIndex: 0, code: "validation", message: "Stop" }] };
        return res;
      }
    }
    const client = new Refusing();
    const ctl = new AppController(client, client, { nativeMenu: false });
    await ctl.start(null);
    await ctl.addSheet();
    const [s1, s2] = (ctl.info?.sheets ?? []).map((s) => s.id) as [number, number];
    ctl.showSheet(s1, false);
    type(ctl, { r: 0, c: 0 }, "no");
    ctl.clickTab(s2);
    await tick();
    await tick();
    expect(ctl.sheet).toBe(s1);
    type(ctl, { r: 0, c: 0 }, "yes");
    ctl.clickTab(s2);
    await tick();
    await tick();
    expect(ctl.sheet).toBe(s2);
  });
});

describe("browser mock: sheet-qualified operands", () => {
  it("evaluates Unicode and quoted sheet names", async () => {
    const { ctl, client, ids } = await setup();
    const [s1, s2, s3] = ids;
    const doc = ctl.info!.doc;
    const tx = (ops: Transaction["ops"]) => client.apply(doc, { ops, label: "x", selectionBefore: null, selectionAfter: null });
    await tx([{ type: "renameSheet", sheet: s2, name: "Données" }, { type: "renameSheet", sheet: s3, name: "My Sheet" }]);
    await tx([
      { type: "setCellInput", sheet: s2, r: 0, c: 0, input: "2" },
      { type: "setCellInput", sheet: s3, r: 0, c: 0, input: "3" },
      { type: "setCellInput", sheet: s1, r: 0, c: 0, input: "=Données.A1+'My Sheet'.A1" },
    ]);
    const [block] = await client.cells(doc, s1, [one({ r: 0, c: 0 })]);
    expect(block?.cells[0]?.text).toBe("5");
  });
});
