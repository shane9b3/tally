/**
 * Enter/Tab after typing must leave the cursor on the next cell. The real
 * engine echoes the transaction's selection in `ApplyResult.selection`; a
 * plain edit must not snap the cursor back to the edited cell when that
 * result arrives (it did, because the grid moves after the commit is sent).
 */
import type { ApplyResult, DocId, Transaction } from "@tally/bridge";
import { cellSelection } from "@tally/grid";
import { describe, expect, it } from "vitest";
import { AppController } from "../src/controller";
import { WebMockClient } from "../src/dev/webMock";
import { useSelectionStore } from "../src/stores/selectionStore";

const tick = () => new Promise((r) => setTimeout(r, 0));

/** WebMockClient that, like the Rust engine, returns `selectionAfter`. */
class EchoingClient extends WebMockClient {
  override async apply(doc: DocId, tx: Transaction): Promise<ApplyResult> {
    const res = await super.apply(doc, tx);
    return { ...res, selection: tx.selectionAfter ?? null };
  }
}

async function controller() {
  const client = new EchoingClient();
  const ctl = new AppController(client, client, { nativeMenu: false });
  await ctl.start("invoices");
  await tick();
  return ctl;
}

/** What the grid does on Enter/Tab: emit the commit, then move the selection. */
function gridCommit(ctl: AppController, cell: { r: number; c: number }, text: string, move: "down" | "right") {
  useSelectionStore.getState().setSelection(cellSelection(cell));
  ctl.onGridStartEdit(cell, text);
  ctl.onGridCommitEdit(move);
  const next = move === "down" ? { r: cell.r + 1, c: cell.c } : { r: cell.r, c: cell.c + 1 };
  useSelectionStore.getState().setSelection(cellSelection(next));
  return next;
}

describe("commit moves the cursor", () => {
  it("Enter goes down and stays there after the engine replies", async () => {
    const ctl = await controller();
    const next = gridCommit(ctl, { r: 19, c: 0 }, "42", "down");
    await tick();
    await tick();
    expect(useSelectionStore.getState().selection.active).toEqual(next);
  });

  it("Tab goes right and stays there after the engine replies", async () => {
    const ctl = await controller();
    const next = gridCommit(ctl, { r: 19, c: 0 }, "42", "right");
    await tick();
    await tick();
    expect(useSelectionStore.getState().selection.active).toEqual(next);
  });

  it("undo still puts the cursor back on the edited cell", async () => {
    const ctl = await controller();
    gridCommit(ctl, { r: 19, c: 0 }, "42", "down");
    await tick();
    await tick();
    await ctl.undo();
    await tick();
    expect(useSelectionStore.getState().selection.active).toEqual({ r: 19, c: 0 });
  });
});
