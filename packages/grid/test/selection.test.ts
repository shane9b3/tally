import { describe, expect, it } from "vitest";
import {
  NO_MODS,
  cellSelection,
  extendTo,
  extentCorner,
  reduceKey,
  selectCols,
  selectRows,
  stepWithin,
  tabEnterStep,
  type TabReturn,
  type KeyAction,
  type Limits,
  type Mods,
} from "../src/selection";
import type { Selection } from "../src/types";

const lim: Limits = { rows: 1_048_576, cols: 16_384, pageRows: 20, pageCols: 8, used: { r0: 0, c0: 0, r1: 99, c1: 9 } };
const mods = (m: Partial<Mods> = {}): Mods => ({ ...NO_MODS, ...m });
const at = (r: number, c: number): Selection => cellSelection({ r, c });

function sel(a: KeyAction): Selection {
  if (a.type !== "select") throw new Error(`expected select, got ${a.type}`);
  return a.selection;
}

describe("reduceKey: movement", () => {
  it("arrows move and clamp", () => {
    expect(sel(reduceKey(at(5, 5), "ArrowDown", mods(), lim)).active).toEqual({ r: 6, c: 5 });
    expect(sel(reduceKey(at(5, 5), "ArrowUp", mods(), lim)).active).toEqual({ r: 4, c: 5 });
    expect(sel(reduceKey(at(5, 5), "ArrowLeft", mods(), lim)).active).toEqual({ r: 5, c: 4 });
    expect(sel(reduceKey(at(5, 5), "ArrowRight", mods(), lim)).active).toEqual({ r: 5, c: 6 });
    expect(sel(reduceKey(at(0, 0), "ArrowUp", mods(), lim)).active).toEqual({ r: 0, c: 0 });
    expect(sel(reduceKey(at(0, 0), "ArrowLeft", mods(), lim)).active).toEqual({ r: 0, c: 0 });
    expect(sel(reduceKey(at(1_048_575, 16_383), "ArrowDown", mods(), lim)).active).toEqual({ r: 1_048_575, c: 16_383 });
  });

  it("moving collapses a range", () => {
    const s = extendTo(at(2, 2), { r: 5, c: 5 });
    const n = sel(reduceKey(s, "ArrowDown", mods(), lim));
    expect(n.range).toEqual({ r0: 3, c0: 2, r1: 3, c1: 2 });
  });

  it("Shift+arrows extend from the anchor, keeping the active cell", () => {
    let s = at(5, 5);
    s = sel(reduceKey(s, "ArrowDown", mods({ shift: true }), lim));
    s = sel(reduceKey(s, "ArrowRight", mods({ shift: true }), lim));
    expect(s.range).toEqual({ r0: 5, c0: 5, r1: 6, c1: 6 });
    expect(s.active).toEqual({ r: 5, c: 5 });
    s = sel(reduceKey(s, "ArrowUp", mods({ shift: true }), lim));
    s = sel(reduceKey(s, "ArrowUp", mods({ shift: true }), lim));
    expect(s.range).toEqual({ r0: 4, c0: 5, r1: 5, c1: 6 });
    expect(extentCorner(s)).toEqual({ r: 4, c: 6 });
  });

  it("Home/End and Ctrl+Home/End", () => {
    expect(sel(reduceKey(at(7, 7), "Home", mods(), lim)).active).toEqual({ r: 7, c: 0 });
    expect(sel(reduceKey(at(7, 7), "Home", mods({ mod: true }), lim)).active).toEqual({ r: 0, c: 0 });
    expect(sel(reduceKey(at(7, 3), "End", mods(), lim)).active).toEqual({ r: 7, c: 9 });
    expect(sel(reduceKey(at(7, 3), "End", mods({ mod: true }), lim)).active).toEqual({ r: 99, c: 9 });
    const ext = sel(reduceKey(at(7, 7), "Home", mods({ shift: true, mod: true }), lim));
    expect(ext.range).toEqual({ r0: 0, c0: 0, r1: 7, c1: 7 });
  });

  it("PageUp/PageDown move by a page; Alt pages sideways", () => {
    expect(sel(reduceKey(at(5, 1), "PageDown", mods(), lim)).active).toEqual({ r: 25, c: 1 });
    expect(sel(reduceKey(at(5, 1), "PageUp", mods(), lim)).active).toEqual({ r: 0, c: 1 });
    expect(sel(reduceKey(at(5, 1), "PageDown", mods({ alt: true }), lim)).active).toEqual({ r: 5, c: 9 });
    expect(reduceKey(at(5, 1), "PageDown", mods({ mod: true }), lim).type).toBe("none");
  });
});

describe("reduceKey: Tab and Enter", () => {
  it("move right/left/down/up from a single cell", () => {
    expect(sel(reduceKey(at(3, 3), "Tab", mods(), lim)).active).toEqual({ r: 3, c: 4 });
    expect(sel(reduceKey(at(3, 3), "Tab", mods({ shift: true }), lim)).active).toEqual({ r: 3, c: 2 });
    expect(sel(reduceKey(at(3, 3), "Enter", mods(), lim)).active).toEqual({ r: 4, c: 3 });
    expect(sel(reduceKey(at(3, 3), "Enter", mods({ shift: true }), lim)).active).toEqual({ r: 2, c: 3 });
  });

  it("walk inside a multi-cell selection and wrap", () => {
    const s0 = extendTo(at(0, 0), { r: 1, c: 1 }); // A1:B2
    const walk = (key: string, n: number, m = mods()): string[] => {
      let s = s0;
      const out: string[] = [];
      for (let i = 0; i < n; i++) {
        s = sel(reduceKey(s, key, m, lim));
        out.push(`${s.active.r},${s.active.c}`);
        expect(s.range).toEqual(s0.range);
      }
      return out;
    };
    expect(walk("Tab", 4)).toEqual(["0,1", "1,0", "1,1", "0,0"]);
    expect(walk("Enter", 4)).toEqual(["1,0", "0,1", "1,1", "0,0"]);
    expect(walk("Tab", 2, mods({ shift: true }))).toEqual(["1,1", "1,0"]);
    expect(stepWithin(s0, "up", lim).active).toEqual({ r: 1, c: 1 });
  });

  it("leaves ⌃Tab / ⌘Tab (± Shift) to the app: no action", () => {
    for (const m of [mods({ mod: true }), mods({ mod: true, shift: true })]) expect(reduceKey(at(3, 3), "Tab", m, lim)).toEqual({ type: "none" });
  });
});

describe("reduceKey: other actions", () => {
  it("Ctrl/Cmd+arrows ask for an edge jump", () => {
    expect(reduceKey(at(1, 1), "ArrowDown", mods({ mod: true }), lim)).toEqual({ type: "navigateEdge", dir: "down", extend: false });
    expect(reduceKey(at(1, 1), "ArrowLeft", mods({ mod: true, shift: true }), lim)).toEqual({ type: "navigateEdge", dir: "left", extend: true });
  });

  it("F2 edits, Delete/Backspace clear, Escape and letters do nothing", () => {
    expect(reduceKey(at(1, 1), "F2", mods(), lim).type).toBe("startEdit");
    // ⌘F2 is the app's Function Wizard, not an edit.
    expect(reduceKey(at(1, 1), "F2", mods({ mod: true }), lim).type).toBe("none");
    expect(reduceKey(at(1, 1), "Delete", mods(), lim).type).toBe("delete");
    expect(reduceKey(at(1, 1), "Backspace", mods(), lim).type).toBe("delete");
    expect(reduceKey(at(1, 1), "Escape", mods(), lim).type).toBe("none");
    expect(reduceKey(at(1, 1), "x", mods(), lim).type).toBe("none");
    expect(reduceKey(at(1, 1), "a", mods(), lim).type).toBe("none");
  });

  it("Ctrl/Cmd+A selects the used range, then everything", () => {
    const first = sel(reduceKey(at(5, 5), "a", mods({ mod: true }), lim));
    expect(first.range).toEqual({ r0: 0, c0: 0, r1: 99, c1: 9 });
    const second = sel(reduceKey(first, "a", mods({ mod: true }), lim));
    expect(second.range).toEqual({ r0: 0, c0: 0, r1: lim.rows - 1, c1: lim.cols - 1 });
    const empty = sel(reduceKey(at(5, 5), "a", mods({ mod: true }), { ...lim, used: null }));
    expect(empty.range).toEqual({ r0: 0, c0: 0, r1: lim.rows - 1, c1: lim.cols - 1 });
  });
});

describe("mouse helpers", () => {
  it("header selections span the sheet", () => {
    expect(selectCols(4, 2, lim).range).toEqual({ r0: 0, c0: 2, r1: lim.rows - 1, c1: 4 });
    expect(selectRows(9, 9, lim).range).toEqual({ r0: 9, c0: 0, r1: 9, c1: lim.cols - 1 });
  });

  it("shift-click extends from the anchor in any direction", () => {
    expect(extendTo(at(5, 5), { r: 2, c: 8 }).range).toEqual({ r0: 2, c0: 5, r1: 5, c1: 8 });
    expect(extendTo(at(5, 5), { r: 2, c: 8 }).active).toEqual({ r: 5, c: 5 });
  });
});

describe("tabEnterStep: Enter returns to the column a Tab run started in", () => {
  /** Presses keys from `start`, the way the grid does, and returns the cells visited. */
  function press(start: Selection, keys: Array<"Tab" | "Enter" | "ShiftTab" | "ShiftEnter" | { click: [number, number] }>): Array<[number, number]> {
    let cur = start;
    let run: TabReturn | null = null;
    const seen: Array<[number, number]> = [];
    for (const k of keys) {
      if (typeof k === "object") {
        cur = at(k.click[0], k.click[1]); // a click does not touch the run: it is stale by position
      } else {
        const key = k.endsWith("Tab") ? "Tab" : "Enter";
        const shift = k.startsWith("Shift");
        const next = sel(reduceKey(cur, key, mods({ shift }), lim));
        const step = tabEnterStep(cur, key, shift, next, run, lim);
        cur = step.selection;
        run = step.run;
      }
      seen.push([cur.active.r, cur.active.c]);
    }
    return seen;
  }

  it("Tab, Tab, Enter goes to the start column on the next row", () => {
    expect(press(at(3, 1), ["Tab", "Tab", "Enter"])).toEqual([[3, 2], [3, 3], [4, 1]]);
  });

  it("each row of a table starts in the same column", () => {
    expect(press(at(0, 0), ["Tab", "Tab", "Enter", "Tab", "Tab", "Enter"]).at(-1)).toEqual([2, 0]);
  });

  it("Enter with no Tab before it goes straight down", () => {
    expect(press(at(3, 4), ["Enter", "Enter"])).toEqual([[4, 4], [5, 4]]);
  });

  it("Shift+Tab inside a run keeps the start column", () => {
    expect(press(at(3, 1), ["Tab", "Tab", "ShiftTab", "Enter"]).at(-1)).toEqual([4, 1]);
  });

  it("clicking another cell ends the run", () => {
    expect(press(at(3, 1), ["Tab", "Tab", { click: [8, 6] }, "Enter"]).at(-1)).toEqual([9, 6]);
  });

  it("Shift+Enter goes up and ends the run", () => {
    expect(press(at(3, 1), ["Tab", "ShiftEnter", "Enter"])).toEqual([[3, 2], [2, 2], [3, 2]]);
  });

  it("inside a multi-cell selection Tab and Enter keep walking the range", () => {
    const range: Selection = { range: { r0: 0, c0: 0, r1: 1, c1: 1 }, active: { r: 0, c: 0 }, anchor: { r: 0, c: 0 } };
    const next = stepWithin(range, "right", lim);
    const step = tabEnterStep(range, "Tab", false, next, null, lim);
    expect(step.selection).toEqual(next);
    expect(step.run).toBeNull();
  });
});
