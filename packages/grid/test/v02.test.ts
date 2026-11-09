// @vitest-environment happy-dom
import { DEFAULT_THEME } from "@tally/tokens";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fillDirection, fillTarget } from "../src/fillHandle";
import { Geometry } from "../src/geometry";
import { Grid } from "../src/grid";
import { AxisModel } from "../src/layout";
import { luminance, textColor, wrapLines } from "../src/renderer";
import { addRange, allRanges, cellSelection, extendTo, sameSelection } from "../src/selection";
import { SyntheticSource } from "../src/synthetic";
import type { GridProps } from "../src/types";

describe("fill handle math", () => {
  const src = { r0: 2, c0: 1, r1: 3, c1: 2 };
  it("extends along the dominant axis and never inside the source", () => {
    expect(fillTarget(src, { r: 3, c: 2 })).toBeNull();
    expect(fillTarget(src, { r: 9, c: 3 })).toEqual({ target: { ...src, r1: 9 }, dir: "down" });
    expect(fillTarget(src, { r: 0, c: 2 })).toEqual({ target: { ...src, r0: 0 }, dir: "up" });
    expect(fillTarget(src, { r: 4, c: 7 })).toEqual({ target: { ...src, c1: 7 }, dir: "right" });
    expect(fillTarget(src, { r: 3, c: 0 })).toEqual({ target: { ...src, c0: 0 }, dir: "left" });
    expect(fillDirection(src, { ...src, r1: 5 })).toBe("down");
    expect(fillDirection(src, src)).toBeNull();
  });
});

describe("multi-range selection", () => {
  it("adds ranges, keeps them while extending, compares them", () => {
    const a = extendTo(cellSelection({ r: 0, c: 0 }), { r: 1, c: 0 });
    const b = addRange(a, { r: 0, c: 2 });
    expect(allRanges(b)).toEqual([
      { r0: 0, c0: 0, r1: 1, c1: 0 },
      { r0: 0, c0: 2, r1: 0, c1: 2 },
    ]);
    const c = extendTo(b, { r: 2, c: 2 });
    expect(c.others).toEqual([{ r0: 0, c0: 0, r1: 1, c1: 0 }]);
    expect(c.range).toEqual({ r0: 0, c0: 2, r1: 2, c1: 2 });
    expect(sameSelection(b, { ...b, others: [] })).toBe(false);
    expect(sameSelection(b, addRange(a, { r: 0, c: 2 }))).toBe(true);
  });
});

describe("row gutters", () => {
  it("hit-tests row header boundaries", () => {
    const geo = new Geometry(new AxisModel(100, 24), new AxisModel(100, 100), 1, 44, 24, 0, 0, 800, 600);
    expect(geo.hit(10, 24 + 24 - 1)).toEqual({ area: "rowGutter", r: 0 });
    expect(geo.hit(10, 24 + 24 + 1)).toEqual({ area: "rowGutter", r: 0 });
    expect(geo.hit(10, 24 + 36)).toEqual({ area: "rowHeader", r: 1 });
  });
});

describe("text helpers", () => {
  it("wraps words and splits long ones", () => {
    const m = (s: string) => s.length * 10;
    expect(wrapLines("one two three", 70, m)).toEqual(["one two", "three"]);
    expect(wrapLines("abcdefghij", 40, m)).toEqual(["abcd", "efgh", "ij"]);
    expect(wrapLines("a\nb", 100, m)).toEqual(["a", "b"]);
  });

  it("keeps text legible on fills", () => {
    expect(luminance(0xffffffff)).toBeCloseTo(1);
    expect(luminance(0x000000ff)).toBeCloseTo(0);
    const cell = { r: 0, c: 0, text: "x", kind: "text" as const, num: null, style: 1, color: null, flags: 0 };
    const style = (fill: number | null, color: number | null = null) =>
      ({ fill, color }) as unknown as Parameters<typeof textColor>[1];
    const dark = { ...DEFAULT_THEME, colors: { ...DEFAULT_THEME.colors, text: "#eeeeee" } };
    expect(textColor(cell, style(0xdcfce7ff), dark)).toBe("#111827");
    expect(textColor(cell, style(0x14532dff), DEFAULT_THEME)).toBe("#f3f4f6");
    expect(textColor(cell, style(null), dark)).toBe("#eeeeee");
    expect(textColor(cell, style(0xdcfce7ff, 0x166534ff), dark)).toBe("rgb(22,101,52)");
    // A transparent "no fill" keeps the theme text.
    expect(textColor(cell, style(0xffffff00), dark)).toBe("#eeeeee");
  });
});

// -- DOM: pointer interactions ------------------------------------------------

let container: HTMLDivElement;

beforeEach(() => {
  const ctx = new Proxy({ measureText: (t: string) => ({ width: t.length * 7 }) } as Record<string, unknown>, {
    get: (target, prop: string) => (prop in target ? target[prop] : () => undefined),
    set: (target, prop: string, v) => {
      target[prop] = v;
      return true;
    },
  });
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockImplementation((() => ctx) as any);
  container = document.createElement("div");
  document.body.appendChild(container);
  Object.defineProperty(HTMLElement.prototype, "clientWidth", { configurable: true, get: () => 800 });
  Object.defineProperty(HTMLElement.prototype, "clientHeight", { configurable: true, get: () => 600 });
});

afterEach(() => {
  container.remove();
  vi.restoreAllMocks();
});

function make(extra: Partial<GridProps> = {}) {
  const props: GridProps = {
    source: new SyntheticSource(1000, 50),
    sheet: 1,
    theme: DEFAULT_THEME,
    zoom: 1,
    selection: cellSelection({ r: 0, c: 0 }),
    edit: null,
    ...extra,
  };
  const grid = new Grid(container, props);
  grid.flush();
  const root = container.querySelector(".tally-grid") as HTMLElement;
  const scroller = root.children[2] as HTMLElement;
  const ta = container.querySelector("textarea") as HTMLTextAreaElement;
  return { grid, scroller, ta };
}

/** Centre of a cell at default sizes (row header 44 px, column header 24 px; col A is 120 px in SyntheticSource). */
const at = (r: number, c: number) => ({ clientX: c === 0 ? 44 + 60 : 164 + (c - 1) * 100 + 50, clientY: 24 + r * 24 + 12 });

function pointer(el: HTMLElement, type: string, p: { clientX: number; clientY: number }, init: PointerEventInit = {}) {
  el.dispatchEvent(new PointerEvent(type, { bubbles: true, cancelable: true, button: 0, pointerId: 1, ...p, ...init }));
}

describe("Grid pointer interactions", () => {
  it("Cmd/Ctrl+click adds a range", () => {
    const onSelectionChange = vi.fn();
    const { grid, scroller } = make({ onSelectionChange });
    pointer(scroller, "pointerdown", at(2, 2), { metaKey: true });
    pointer(scroller, "pointerup", at(2, 2));
    expect(grid.getSelection().others).toEqual([{ r0: 0, c0: 0, r1: 0, c1: 0 }]);
    expect(grid.getSelection().range).toEqual({ r0: 2, c0: 2, r1: 2, c1: 2 });
  });

  it("dragging the fill handle emits onFill and selects the target", () => {
    const onFill = vi.fn();
    const { grid, scroller } = make({ onFill });
    // The handle sits at the bottom-right corner of A1: (164, 48).
    pointer(scroller, "pointerdown", { clientX: 163, clientY: 47 });
    pointer(scroller, "pointermove", at(4, 0));
    pointer(scroller, "pointerup", at(4, 0));
    expect(onFill).toHaveBeenCalledWith({ r0: 0, c0: 0, r1: 0, c1: 0 }, { r0: 0, c0: 0, r1: 4, c1: 0 }, false);
    expect(grid.getSelection().range).toEqual({ r0: 0, c0: 0, r1: 4, c1: 0 });
  });

  it.each([{ metaKey: true }, { ctrlKey: true }])("the copy modifier held on release is passed to onFill (AOO KEY_MOD1): %o", (mod) => {
    const onFill = vi.fn();
    const { scroller } = make({ onFill });
    pointer(scroller, "pointerdown", { clientX: 163, clientY: 47 });
    pointer(scroller, "pointermove", at(4, 0));
    pointer(scroller, "pointerup", at(4, 0), mod);
    expect(onFill).toHaveBeenCalledWith({ r0: 0, c0: 0, r1: 0, c1: 0 }, { r0: 0, c0: 0, r1: 4, c1: 0 }, true);
  });

  it("double-clicking the fill handle asks the app to fill down", () => {
    const onFillHandleDoubleClick = vi.fn();
    const { scroller } = make({ onFillHandleDoubleClick });
    scroller.dispatchEvent(new MouseEvent("dblclick", { bubbles: true, clientX: 163, clientY: 47 }));
    expect(onFillHandleDoubleClick).toHaveBeenCalledWith({ r0: 0, c0: 0, r1: 0, c1: 0 });
  });

  it("dragging a row gutter resizes the row; double-clicking a column gutter auto-fits", () => {
    const onResize = vi.fn();
    const onAutoFit = vi.fn();
    const { scroller } = make({ onResize, onAutoFit });
    pointer(scroller, "pointerdown", { clientX: 20, clientY: 48 });
    pointer(scroller, "pointermove", { clientX: 20, clientY: 64 });
    pointer(scroller, "pointerup", { clientX: 20, clientY: 64 });
    expect(onResize).toHaveBeenCalledWith("row", 0, 0, 40);
    scroller.dispatchEvent(new MouseEvent("dblclick", { bubbles: true, clientX: 163, clientY: 10 }));
    expect(onAutoFit).toHaveBeenCalledWith("col", 0, 0);
  });

  it("point mode: clicks and arrows pick references instead of committing", () => {
    const onPointRange = vi.fn();
    const onCommitEdit = vi.fn();
    const { grid, scroller, ta } = make({ onPointRange, onCommitEdit, selection: cellSelection({ r: 0, c: 1 }) });
    grid.update({ edit: { cell: { r: 0, c: 1 }, text: "=", caret: 1 }, pointMode: true });
    pointer(scroller, "pointerdown", at(3, 0));
    pointer(scroller, "pointermove", at(4, 0));
    pointer(scroller, "pointerup", at(4, 0));
    expect(onCommitEdit).not.toHaveBeenCalled();
    expect(onPointRange).toHaveBeenNthCalledWith(1, { r0: 3, c0: 0, r1: 3, c1: 0 }, "start");
    expect(onPointRange).toHaveBeenLastCalledWith({ r0: 3, c0: 0, r1: 4, c1: 0 }, "end");
    // Arrow keys continue from the last pick, in "enter" mode only (F2 opened "edit" mode here).
    ta.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true, cancelable: true }));
    expect(onPointRange).toHaveBeenCalledTimes(3);
  });

  it("point mode: a column or row header picks the whole column / row", () => {
    const onPointRange = vi.fn();
    const onCommitEdit = vi.fn();
    const { grid, scroller } = make({ onPointRange, onCommitEdit });
    grid.update({ edit: { cell: { r: 0, c: 0 }, text: "=", caret: 1 }, pointMode: true });
    pointer(scroller, "pointerdown", { clientX: 164 + 50, clientY: 10 });
    expect(onCommitEdit).not.toHaveBeenCalled();
    const col = onPointRange.mock.calls.at(-1)?.[0];
    expect([col.c0, col.c1, col.r0]).toEqual([1, 1, 0]);
    expect(col.r1).toBeGreaterThan(1000);
    pointer(scroller, "pointerdown", { clientX: 20, clientY: 24 + 2 * 24 + 12 });
    const row = onPointRange.mock.calls.at(-1)?.[0];
    expect([row.r0, row.r1, row.c0]).toEqual([2, 2, 0]);
    expect(onCommitEdit).not.toHaveBeenCalled();
  });

  it("editElsewhere: the editor is hidden, other clicks and the context menu do nothing, a commit leaves the selection", () => {
    const onCommitEdit = vi.fn();
    const onSelectionChange = vi.fn();
    const onContextMenu = vi.fn();
    const { grid, scroller, ta } = make({ onCommitEdit, onSelectionChange, onContextMenu, selection: cellSelection({ r: 3, c: 2 }) });
    grid.update({ edit: { cell: { r: 3, c: 2 }, text: "=SUM(A1)", caret: 8 }, pointMode: true, editElsewhere: true });
    grid.flush();
    expect(ta.style.opacity).toBe("0");
    pointer(scroller, "pointerdown", { clientX: 5, clientY: 5 });
    pointer(scroller, "pointerdown", at(1, 1), { button: 2 });
    scroller.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, ...at(1, 1) }));
    expect(onContextMenu).not.toHaveBeenCalled();
    expect(onCommitEdit).not.toHaveBeenCalled();
    ta.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
    expect(onCommitEdit).toHaveBeenCalledWith("down");
    expect(onSelectionChange).not.toHaveBeenCalled();
  });

  it("onEditorKeyDown can swallow editor keys", () => {
    const onCommitEdit = vi.fn();
    const onEditorKeyDown = vi.fn(() => true);
    const { grid, ta } = make({ onCommitEdit, onEditorKeyDown });
    grid.update({ edit: { cell: { r: 0, c: 0 }, text: "=SU", caret: 3 } });
    ta.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
    expect(onEditorKeyDown).toHaveBeenCalled();
    expect(onCommitEdit).not.toHaveBeenCalled();
  });

  it("⌃Tab / ⌘Tab in the editor neither commits nor is prevented (the native menu gets it)", () => {
    const onCommitEdit = vi.fn();
    const { grid, ta } = make({ onCommitEdit });
    grid.update({ edit: { cell: { r: 0, c: 0 }, text: "12", caret: 2 } });
    for (const init of [{ ctrlKey: true }, { ctrlKey: true, shiftKey: true }, { metaKey: true }]) {
      const e = new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true, ...init });
      ta.dispatchEvent(e);
      expect(e.defaultPrevented).toBe(false);
    }
    expect(onCommitEdit).not.toHaveBeenCalled();
    // A plain Tab still commits and moves right.
    const plain = new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true });
    ta.dispatchEvent(plain);
    expect(plain.defaultPrevented).toBe(true);
    expect(onCommitEdit).toHaveBeenCalledTimes(1);
  });

  it("⌃Tab on the idle grid is not prevented either", () => {
    const { ta } = make();
    const e = new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true, ctrlKey: true });
    ta.dispatchEvent(e);
    expect(e.defaultPrevented).toBe(false);
  });

  it("colours the editor text through the mirror", () => {
    const { grid, ta } = make();
    grid.update({ edit: { cell: { r: 0, c: 0 }, text: "=A1+B2", caret: 6 }, editHighlights: [{ start: 1, end: 3, color: "#ff0000" }] });
    grid.flush();
    const mirror = ta.previousElementSibling as HTMLElement;
    expect(mirror.style.display).toBe("block");
    expect(mirror.querySelector("span")?.textContent).toBe("A1");
    expect(ta.style.color).toBe("transparent");
    grid.update({ editHighlights: [] });
    expect(mirror.style.display).toBe("none");
  });

  it("a programmatic edit (seq bump) is applied even if it echoes earlier text", () => {
    const { grid, ta } = make();
    grid.update({ edit: { cell: { r: 0, c: 0 }, text: "", caret: 0 } });
    ta.value = "a";
    ta.dispatchEvent(new Event("input", { bubbles: true }));
    ta.value = "ab";
    ta.dispatchEvent(new Event("input", { bubbles: true }));
    grid.update({ edit: { cell: { r: 0, c: 0 }, text: "a", caret: 1 } });
    expect(ta.value).toBe("ab"); // stale echo ignored
    grid.update({ edit: { cell: { r: 0, c: 0 }, text: "a", caret: 1, seq: 1 } });
    expect(ta.value).toBe("a"); // editor undo applied
  });
});
