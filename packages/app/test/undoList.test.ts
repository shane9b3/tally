/** Toolbar Undo / Redo lists and the undo steps option (AOO parity, U17). */
import { describe, expect, it } from "vitest";
import { undoListFooter, undoListKey, undoListLabel, undoListTitle } from "../src/undoList";
import { DEFAULT_PREFS, isUndoSteps, parsePrefs } from "../src/options/prefs";
import { WebMockClient } from "../src/dev/webMock";

describe("undo list keys", () => {
  it("moves the marked count within 1..len and picks or closes", () => {
    expect(undoListKey("ArrowDown", 1, 5)).toEqual({ count: 2 });
    expect(undoListKey("ArrowDown", 5, 5)).toEqual({ count: 5 });
    expect(undoListKey("ArrowUp", 1, 5)).toEqual({ count: 1 });
    expect(undoListKey("ArrowUp", 3, 5)).toEqual({ count: 2 });
    expect(undoListKey("PageDown", 1, 25)).toEqual({ count: 11 });
    expect(undoListKey("PageUp", 4, 25)).toEqual({ count: 1 });
    expect(undoListKey("Home", 4, 25)).toEqual({ count: 1 });
    expect(undoListKey("End", 4, 25)).toEqual({ count: 25 });
    expect(undoListKey("Enter", 4, 25)).toBe("pick");
    expect(undoListKey(" ", 4, 25)).toBe("pick");
    expect(undoListKey("Tab", 4, 25)).toBe("close");
    expect(undoListKey("x", 4, 25)).toBeNull();
  });

  it("labels the footer, the list and unnamed steps", () => {
    expect(undoListFooter("undo", 3)).toBe("Actions to undo: 3");
    expect(undoListFooter("redo", 1)).toBe("Actions to redo: 1");
    expect(undoListTitle("undo")).toBe("Undo history");
    expect(undoListLabel("")).toBe("(unnamed action)");
    expect(undoListLabel("Typing")).toBe("Typing");
  });
});

describe("undo steps option", () => {
  it("defaults to 100 and accepts integers 1–1000", () => {
    expect(DEFAULT_PREFS.undoSteps).toBe(100);
    expect(parsePrefs({}).undoSteps).toBe(100);
    expect(parsePrefs({ undoSteps: 250 }).undoSteps).toBe(250);
    expect(parsePrefs({ undoSteps: 0 }).undoSteps).toBe(100);
    expect(parsePrefs({ undoSteps: 1001 }).undoSteps).toBe(100);
    expect(parsePrefs({ undoSteps: 2.5 }).undoSteps).toBe(100);
    expect(isUndoSteps(1)).toBe(true);
    expect(isUndoSteps(1000)).toBe(true);
  });
});

describe("WebMockClient undo steps", () => {
  it("keeps its side model in step when undoing n steps and trimming", async () => {
    const c = new WebMockClient();
    const info = await c.newWorkbook(null);
    const doc = info.doc;
    const apply = (ops: Parameters<WebMockClient["apply"]>[1]["ops"], label: string) => c.apply(doc, { ops, label, selectionBefore: null, selectionAfter: null });
    await apply([{ type: "setCellInput", sheet: 1, r: 0, c: 0, input: "1" }], "a");
    await apply([{ type: "insertRows", sheet: 1, at: 0, count: 2 }], "b");
    await apply([{ type: "setCellInput", sheet: 1, r: 0, c: 0, input: "x" }], "c");
    expect((await c.undoHistory(doc)).undo).toEqual(["c", "b", "a"]);
    await c.undoSteps(doc, 2);
    const [block] = await c.cells(doc, 1, [{ r0: 0, c0: 0, r1: 3, c1: 0 }]);
    expect(block!.cells.map((v) => `${v.r}:${v.text}`)).toEqual(["0:1"]);
    await c.setUndoDepth(1);
    expect(await c.undoHistory(doc)).toEqual({ undo: [], redo: ["b"] });
    await c.redoSteps(doc, 1);
    const [after] = await c.cells(doc, 1, [{ r0: 0, c0: 0, r1: 3, c1: 0 }]);
    expect(after!.cells.map((v) => `${v.r}:${v.text}`)).toEqual(["2:1"]);
  });
});
