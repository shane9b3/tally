/**
 * Window ▸ Split after undo / redo: `setSplit` is an undo step, so undoing
 * or redoing it changes the split the engine stores; the shown split must
 * follow (it was only restored on sheet show / load).
 */
import type { ApplyResult, SplitDto, ViewOptionsDto } from "@tally/bridge";
import { beforeEach, describe, expect, it } from "vitest";
import type { AppController } from "../src/controller";
import { W56Features } from "../src/data/w56";
import { DEFAULT_VIEW, useW56Store } from "../src/data/store";

function result(layoutChanged: number[] = []): ApplyResult {
  return { version: 1, changed: [], layoutChanged, sheetsChanged: false, errors: [], undo: { canUndo: true, canRedo: false, undoLabel: null, redoLabel: null } } as unknown as ApplyResult;
}

function setup() {
  let engineSplit: SplitDto | null = null;
  const restored: (SplitDto | null)[] = [];
  const ctl = {
    info: { doc: 1 },
    sheet: 0,
    client: {
      viewOptions: (): Promise<ViewOptionsDto> => Promise.resolve({ ...DEFAULT_VIEW, split: engineSplit ? { ...engineSplit } : null }),
    },
    w5: { view: { restoreSplit: (s: SplitDto | null) => restored.push(s), applyDocZoom: () => {} } },
    notifyError: (e: unknown) => {
      throw e;
    },
  };
  const w56 = new W56Features(ctl as unknown as AppController);
  return {
    w56,
    restored,
    setEngine: (s: SplitDto | null) => {
      engineSplit = s;
    },
  };
}

const flush = () => new Promise((r) => setTimeout(r, 0));

describe("split restore after undo / redo", () => {
  beforeEach(() => useW56Store.getState().setView({ ...DEFAULT_VIEW, split: null }));

  it("undo and redo of a split show the engine's split again", async () => {
    const { w56, restored, setEngine } = setup();
    // The split was made (persisted with its op): the store learns it, nothing is restored.
    setEngine({ xPx: 0, yPx: 200 });
    w56.afterResult(result([0]), [{ type: "setSplit", sheet: 0, split: { xPx: 0, yPx: 200 } }]);
    await flush();
    expect(restored).toEqual([]);
    expect(useW56Store.getState().view.split).toEqual({ xPx: 0, yPx: 200 });
    // Undo: the engine has no split any more.
    setEngine(null);
    w56.afterResult(result([0]), null);
    await flush();
    expect(restored).toEqual([null]);
    // Redo: the split comes back.
    setEngine({ xPx: 0, yPx: 200 });
    w56.afterResult(result([0]), null);
    await flush();
    expect(restored).toEqual([null, { xPx: 0, yPx: 200 }]);
  });

  it("an undo that does not touch the split leaves the shown split alone", async () => {
    const { w56, restored, setEngine } = setup();
    setEngine({ xPx: 120, yPx: 0 });
    w56.afterResult(result([0]), [{ type: "setSplit", sheet: 0, split: { xPx: 120, yPx: 0 } }]);
    await flush();
    w56.afterResult(result(), null);
    await flush();
    expect(restored).toEqual([]);
  });
});
