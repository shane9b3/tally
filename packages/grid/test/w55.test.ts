// @vitest-environment happy-dom
/**
 * W5-5 grid additions: `anchorAxes()` / `objectRect()`, Page Break Preview
 * break-line dragging (`onPageBreakDrag`), the "Mark invalid data" circles
 * and the SplitGrid wrapper's view.
 */
import { DEFAULT_THEME } from "@tally/tokens";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Grid } from "../src/grid";
import { anchorToPx } from "../src/objectLayer";
import { cellSelection } from "../src/selection";
import { SyntheticSource } from "../src/synthetic";
import type { GridProps } from "../src/types";

interface Call {
  name: string;
  args: unknown[];
}
let calls: Call[];

function stubCanvas() {
  const state: Record<string, unknown> = {};
  const ctx = new Proxy(state, {
    get: (target, prop: string) => {
      if (prop === "measureText") return (t: string) => ({ width: t.length * 7 });
      if (prop in target) return target[prop];
      return (...args: unknown[]) => {
        calls.push({ name: prop, args });
      };
    },
    set: (target, prop: string, v) => {
      target[prop] = v;
      return true;
    },
  });
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockImplementation((() => ctx) as any);
}

let container: HTMLDivElement;

beforeEach(() => {
  calls = [];
  stubCanvas();
  container = document.createElement("div");
  document.body.appendChild(container);
  Object.defineProperty(HTMLElement.prototype, "clientWidth", { configurable: true, get: () => 800 });
  Object.defineProperty(HTMLElement.prototype, "clientHeight", { configurable: true, get: () => 600 });
  Object.defineProperty(HTMLElement.prototype, "getBoundingClientRect", { configurable: true, value: () => ({ left: 0, top: 0, right: 800, bottom: 600, width: 800, height: 600, x: 0, y: 0, toJSON: () => ({}) }) });
});

afterEach(() => {
  container.remove();
  vi.restoreAllMocks();
});

function make(extra: Partial<GridProps> = {}) {
  const source = new SyntheticSource(1_048_576, 100);
  const props: GridProps = { source, sheet: 1, theme: DEFAULT_THEME, zoom: 1, selection: cellSelection({ r: 0, c: 0 }), edit: null, ...extra };
  const grid = new Grid(container, props);
  grid.flush();
  return { grid, props };
}

function pointer(el: Element, type: string, x: number, y: number, init: PointerEventInit = {}) {
  el.dispatchEvent(new PointerEvent(type, { bubbles: true, cancelable: true, clientX: x, clientY: y, button: 0, pointerId: 1, ...init }));
}

describe("object anchors", () => {
  it("anchorAxes places objects like the layer and objectRect reads the node box", () => {
    const { grid } = make({ objects: [{ id: "chart:1", kind: "chart", anchor: { r: 2, c: 1, dxMm: 0, dyMm: 0, widthMm: 50, heightMm: 20 } }] });
    const px = anchorToPx({ r: 2, c: 1, dxMm: 0, dyMm: 0, widthMm: 50, heightMm: 20 }, grid.anchorAxes());
    const rect = grid.objectRect("chart:1")!;
    expect(rect.x).toBeCloseTo(px.x, 3);
    expect(rect.y).toBeCloseTo(px.y, 3);
    expect(rect.w).toBeCloseTo(px.w, 3);
    expect(grid.objectRect("nope")).toBeNull();
    expect(grid.contentOffset()).toEqual({ x: 0, y: 0 });
    grid.destroy();
  });
});

describe("page break preview", () => {
  it("dragging a break line reports its new row", () => {
    const drags: unknown[][] = [];
    const { grid } = make({
      pageBreakPreview: true,
      pageBreaks: { rows: [{ at: 5, manual: false }], cols: [{ at: 3, manual: true }] },
      onPageBreakDrag: (axis, from, to, manual) => drags.push([axis, from, to, manual]),
    });
    const geo = grid.getGeometry();
    const scroller = container.querySelector<HTMLElement>(".tally-grid > div[style*='overflow: auto']") ?? (container.querySelector(".tally-grid")!.children[2] as HTMLElement);
    const y = geo.rowTop(5);
    pointer(scroller, "pointerdown", geo.headerW + 150, y + 1);
    pointer(scroller, "pointermove", geo.headerW + 150, geo.rowTop(8) + 3);
    grid.flush();
    expect(calls.some((c) => c.name === "setLineDash" && JSON.stringify(c.args) === "[[7,3]]")).toBe(true);
    pointer(scroller, "pointerup", geo.headerW + 150, geo.rowTop(8) + 3);
    expect(drags).toEqual([["row", 5, 8, false]]);
    // A manual column break dragged past the middle of a column lands left of the next one.
    pointer(scroller, "pointerdown", geo.colLeft(3) - 2, geo.headerH + 50);
    pointer(scroller, "pointerup", geo.colLeft(6) + 70, geo.headerH + 50);
    expect(drags[1]).toEqual(["col", 3, 7, true]);
    // Off a line: nothing.
    pointer(scroller, "pointerdown", geo.headerW + 150, geo.rowTop(10) + 12);
    pointer(scroller, "pointerup", geo.headerW + 150, geo.rowTop(12));
    expect(drags).toHaveLength(2);
    grid.destroy();
  });

  it("does nothing without the preview mode or the callback", () => {
    const drags: unknown[] = [];
    const { grid } = make({ pageBreaks: { rows: [{ at: 5, manual: false }], cols: [] }, onPageBreakDrag: (...a) => drags.push(a) });
    const geo = grid.getGeometry();
    const scroller = container.querySelector(".tally-grid")!.children[2] as HTMLElement;
    pointer(scroller, "pointerdown", geo.headerW + 150, geo.rowTop(5));
    pointer(scroller, "pointerup", geo.headerW + 150, geo.rowTop(8));
    expect(drags).toEqual([]);
    grid.destroy();
  });
});

describe("invalid data circles", () => {
  it("draws an ellipse per listed visible cell", () => {
    const { grid } = make({ invalidCells: [{ r: 2, c: 1 }, { r: 500, c: 1 }] });
    const ellipses = calls.filter((c) => c.name === "ellipse");
    expect(ellipses).toHaveLength(1);
    const geo = grid.getGeometry();
    expect(ellipses[0]?.args[0]).toBeCloseTo(geo.colLeft(1) + 50, 1);
    grid.update({ invalidCells: null });
    calls = [];
    grid.flush();
    expect(calls.filter((c) => c.name === "ellipse")).toHaveLength(0);
    grid.destroy();
  });
});
