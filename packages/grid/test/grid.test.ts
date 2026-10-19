// @vitest-environment happy-dom
import { DEFAULT_THEME } from "@tally/tokens";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Grid } from "../src/grid";
import { cellSelection } from "../src/selection";
import { SyntheticSource } from "../src/synthetic";
import type { GridProps } from "../src/types";

/** A 2D context stub that records fillText calls. */
function stubCanvas(texts: string[]) {
  const ctx = new Proxy(
    {
      fillText: (t: string) => texts.push(t),
      measureText: (t: string) => ({ width: t.length * 7 }),
    } as Record<string, unknown>,
    {
      get: (target, prop: string) => (prop in target ? target[prop] : () => undefined),
      set: (target, prop: string, v) => {
        target[prop] = v;
        return true;
      },
    },
  );
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockImplementation((() => ctx) as any);
}

let container: HTMLDivElement;
let texts: string[];

beforeEach(() => {
  texts = [];
  stubCanvas(texts);
  container = document.createElement("div");
  document.body.appendChild(container);
  // happy-dom has no layout: give the scroller a size.
  Object.defineProperty(HTMLElement.prototype, "clientWidth", { configurable: true, get: () => 800 });
  Object.defineProperty(HTMLElement.prototype, "clientHeight", { configurable: true, get: () => 600 });
});

afterEach(() => {
  container.remove();
  vi.restoreAllMocks();
});

function make(extra: Partial<GridProps> = {}) {
  const source = new SyntheticSource(1_048_576, 100);
  const props: GridProps = {
    source,
    sheet: 1,
    theme: DEFAULT_THEME,
    zoom: 1,
    selection: cellSelection({ r: 0, c: 0 }),
    edit: null,
    ...extra,
  };
  const grid = new Grid(container, props);
  grid.flush();
  const ta = container.querySelector("textarea");
  if (!ta) throw new Error("no textarea");
  return { grid, source, ta };
}

const key = (ta: HTMLTextAreaElement, k: string, init: KeyboardEventInit = {}) =>
  ta.dispatchEvent(new KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true, ...init }));

describe("Grid (DOM smoke test)", () => {
  it("draws headers and visible cells only", () => {
    make();
    expect(texts).toContain("A");
    expect(texts).toContain("1");
    expect(texts).toContain("Item");
    expect(texts).toContain("Item 1");
    expect(texts.some((t) => t.startsWith("Item 1000"))).toBe(false);
    expect(texts.length).toBeLessThan(2000);
  });

  it("moves the selection with the keyboard and emits it", () => {
    const onSelectionChange = vi.fn();
    const { grid, ta } = make({ onSelectionChange });
    key(ta, "ArrowDown");
    key(ta, "ArrowRight", { shiftKey: true });
    expect(onSelectionChange).toHaveBeenCalledTimes(2);
    expect(grid.getSelection().range).toEqual({ r0: 1, c0: 0, r1: 1, c1: 1 });
    expect(grid.getSelection().active).toEqual({ r: 1, c: 0 });
  });

  it("starts editing on typing and commits on Enter, then moves down", () => {
    const onStartEdit = vi.fn();
    const onCommitEdit = vi.fn();
    const { grid, ta } = make({ onStartEdit, onCommitEdit });
    ta.value = "x";
    ta.dispatchEvent(new Event("input", { bubbles: true }));
    expect(onStartEdit).toHaveBeenCalledWith({ r: 0, c: 0 }, "x");
    expect(ta.style.opacity).toBe("1");
    key(ta, "Enter");
    expect(onCommitEdit).toHaveBeenCalledWith("down");
    expect(grid.getSelection().active).toEqual({ r: 1, c: 0 });
    expect(ta.value).toBe("");
    expect(ta.style.opacity).toBe("0");
  });

  it("F2 asks the app for the edit text; Escape cancels", () => {
    const onStartEdit = vi.fn();
    const onCancelEdit = vi.fn();
    const { grid, ta } = make({ onStartEdit, onCancelEdit });
    key(ta, "F2");
    expect(onStartEdit).toHaveBeenCalledWith({ r: 0, c: 0 }, undefined);
    grid.update({ edit: { cell: { r: 0, c: 0 }, text: "Item", caret: 4 } });
    expect(ta.value).toBe("Item");
    key(ta, "Escape");
    expect(onCancelEdit).toHaveBeenCalled();
  });

  it("Delete asks to clear; Ctrl+arrow awaits the edge", async () => {
    const onDeleteContents = vi.fn();
    const onNavigateEdge = vi.fn(() => Promise.resolve({ r: 500, c: 0 }));
    const { grid, ta } = make({ onDeleteContents, onNavigateEdge });
    key(ta, "Delete");
    expect(onDeleteContents).toHaveBeenCalledWith({ r0: 0, c0: 0, r1: 0, c1: 0 });
    key(ta, "ArrowDown", { ctrlKey: true });
    await Promise.resolve();
    await Promise.resolve();
    expect(grid.getSelection().active).toEqual({ r: 500, c: 0 });
    // Scrolled so row 500 is visible.
    const vp = grid.getViewport();
    expect(vp.r0).toBeLessThanOrEqual(500);
    expect(vp.r1).toBeGreaterThanOrEqual(500);
  });

  it("scrolls to the last row of 1,048,576", () => {
    const { grid } = make();
    grid.scrollCellIntoView({ r: 1_048_575, c: 0 });
    grid.flush();
    expect(grid.getViewport().r1).toBe(1_048_575);
    expect(texts).toContain("1048576");
  });

  it("emits clipboard events outside the editor", () => {
    const onCopy = vi.fn();
    const onPaste = vi.fn();
    const { ta } = make({ onCopy, onPaste });
    const copy = new Event("copy", { bubbles: true, cancelable: true });
    ta.dispatchEvent(copy);
    expect(onCopy).toHaveBeenCalled();
    expect(copy.defaultPrevented).toBe(true);
    ta.dispatchEvent(new Event("paste", { bubbles: true, cancelable: true }));
    expect(onPaste).toHaveBeenCalled();
  });

  it("destroy removes the DOM", () => {
    const { grid } = make();
    grid.destroy();
    expect(container.querySelector(".tally-grid")).toBeNull();
  });
});
