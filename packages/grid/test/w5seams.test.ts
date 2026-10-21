// @vitest-environment happy-dom
/**
 * Wave-5 grid seams (W5-0b): the object overlay layer (anchor maths, hit
 * fall-through, selection handles), the ARIA mirror, range-pick mode,
 * outline gutter geometry, the page watermark stub, getGeometry /
 * onViewportChange and the split view.
 */
import { DEFAULT_THEME } from "@tally/tokens";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { describeCell, describeSelection } from "../src/aria";
import { Grid } from "../src/grid";
import { AxisModel } from "../src/layout";
import { anchorRange, anchorToPx, ObjectLayer, PX_PER_MM, pxToAnchor } from "../src/objectLayer";
import { outlineHit, outlineLevels } from "../src/outline";
import { cellSelection, extendTo } from "../src/selection";
import { SplitView } from "../src/split";
import { SyntheticSource } from "../src/synthetic";
import type { GridProps, RangeDto, Viewport } from "../src/types";

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
});

afterEach(() => {
  container.remove();
  vi.restoreAllMocks();
});

function make(extra: Partial<GridProps> = {}, host: HTMLElement = container) {
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
  const grid = new Grid(host, props);
  grid.flush();
  const ta = host.querySelector("textarea");
  if (!ta) throw new Error("no textarea");
  return { grid, ta, source, props };
}

function pointer(el: Element, type: string, x: number, y: number, init: PointerEventInit = {}) {
  el.dispatchEvent(new PointerEvent(type, { bubbles: true, cancelable: true, clientX: x, clientY: y, button: 0, pointerId: 1, ...init }));
}

const axes = { rows: new AxisModel(100, 24), cols: new AxisModel(50, 100), zoom: 1 };

describe("object layer anchors", () => {
  it("anchorToPx and pxToAnchor round-trip at zoom 1 and 1.25", () => {
    for (const zoom of [1, 1.25]) {
      const ax = { ...axes, zoom };
      const a = { r: 3, c: 2, dxMm: 2.5, dyMm: 1, widthMm: 160, heightMm: 90 };
      const px = anchorToPx(a, ax);
      expect(px.x).toBeCloseTo((200 + 2.5 * PX_PER_MM) * zoom, 6);
      expect(px.y).toBeCloseTo((72 + 1 * PX_PER_MM) * zoom, 6);
      expect(px.w).toBeCloseTo(160 * PX_PER_MM * zoom, 6);
      expect(px.h).toBeCloseTo(90 * PX_PER_MM * zoom, 6);
      const back = pxToAnchor(px, ax);
      expect(back.r).toBe(3);
      expect(back.c).toBe(2);
      expect(back.dxMm).toBeCloseTo(2.5, 6);
      expect(back.dyMm).toBeCloseTo(1, 6);
      expect(back.widthMm).toBeCloseTo(160, 6);
      expect(back.heightMm).toBeCloseTo(90, 6);
    }
    expect(anchorRange({ r: 0, c: 0, dxMm: 0, dyMm: 0, widthMm: 160, heightMm: 90 }, axes)).toEqual({ r0: 0, c0: 0, r1: 14, c1: 6 });
  });

  it("clamps negative rects to the sheet origin", () => {
    const a = pxToAnchor({ x: -50, y: -10, w: 10, h: 10 }, axes);
    expect(a).toMatchObject({ r: 0, c: 0, dxMm: 0, dyMm: 0 });
  });
});

describe("object layer in the grid", () => {
  it("places nodes with the scroll, lets clicks outside objects fall through and shows handles on select", () => {
    const selects: (string | null)[] = [];
    const downs: string[] = [];
    const { grid } = make({
      objects: [{ id: "chart-1", kind: "chart", anchor: { r: 2, c: 1, dxMm: 0, dyMm: 0, widthMm: 50, heightMm: 30 }, label: "Sales" }],
      onObjectSelect: (id) => selects.push(id),
      onObjectPointerDown: (id, part) => downs.push(`${id}:${part}`),
    });
    const layer = grid.objectLayer;
    const node = layer.elementFor("chart-1");
    expect(node).toBeDefined();
    const g0 = grid.getGeometry();
    const col1 = g0.colLeft(1) - g0.headerW;
    expect(node?.style.left).toBe(`${col1}px`);
    expect(node?.style.top).toBe("48px");
    expect(node?.getAttribute("aria-label")).toBe("Sales");
    // The layer itself never takes pointer events; the node does.
    expect(layer.el.style.pointerEvents).toBe("none");
    expect(node?.style.pointerEvents).toBe("auto");
    const geo = grid.getGeometry();
    expect(layer.el.style.left).toBe(`${geo.headerW}px`);
    // Hit test in canvas px: inside the chart vs. a cell next to it.
    expect(layer.hitTest(geo.headerW + col1 + 10, geo.headerH + 60)).toBe("chart-1");
    expect(layer.hitTest(geo.headerW + 10, geo.headerH + 10)).toBeNull();
    // Select: 8 handles, aria-selected, event.
    pointer(node as Element, "pointerdown", 0, 0);
    expect(layer.selected).toBe("chart-1");
    expect(node?.querySelectorAll("[data-handle]").length).toBe(8);
    expect(selects).toEqual(["chart-1"]);
    expect(downs).toEqual(["chart-1:body"]);
    const handle = node?.querySelector('[data-handle="se"]') as Element;
    pointer(handle, "pointerdown", 0, 0);
    expect(downs).toEqual(["chart-1:body", "chart-1:se"]);
    // A click on the scroller (a cell) deselects.
    const scroller = container.querySelector(".tally-grid > div[style*='overflow: auto']") ?? container.querySelectorAll(".tally-grid > div")[0];
    pointer(scroller as Element, "pointerdown", 300, 300);
    expect(layer.selected).toBeNull();
    expect(selects).toEqual(["chart-1", null]);
    // Scrolling moves the inner container, not the nodes.
    grid.scrollTo({ r: 10, c: 0 });
    grid.flush();
    expect(layer.inner.style.transform).toBe("translate(0px, -240px)");
    expect(node?.style.top).toBe("48px");
    grid.update({ objects: [] });
    expect(layer.elementFor("chart-1")).toBeUndefined();
    grid.destroy();
  });
});

describe("ARIA mirror", () => {
  it("describes cells and selections", () => {
    expect(describeCell("B2", "42", null)).toBe("B2: 42");
    expect(describeCell("B2", "", null)).toBe("B2: empty");
    expect(describeCell("C1", "6", "=A1*B1")).toBe("C1: 6, formula =A1*B1");
    const sel = extendTo(cellSelection({ r: 0, c: 0 }), { r: 2, c: 2 });
    expect(describeSelection(sel, "x", null)).toBe("A1: x, selected A1 to C3 (3 rows, 3 columns)");
  });

  it("mirrors the grid: role, counts, active descendant, live announcements", () => {
    const { grid, ta } = make({ aria: { sheetName: "Budget" } });
    const root = container.querySelector(".tally-grid") as HTMLElement;
    expect(root.getAttribute("role")).toBe("grid");
    expect(root.getAttribute("aria-rowcount")).toBe("1048576");
    expect(root.getAttribute("aria-colcount")).toBe("16384");
    expect(root.getAttribute("aria-label")).toBe("Sheet Budget");
    const id = ta.getAttribute("aria-activedescendant");
    expect(id).toBeTruthy();
    const cell = root.querySelector(`#${id}`) as HTMLElement;
    expect(cell.getAttribute("role")).toBe("gridcell");
    expect(cell.parentElement?.getAttribute("role")).toBe("row");
    // Move with the keyboard: the description and the live region follow.
    ta.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
    ta.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }));
    expect(cell.getAttribute("aria-rowindex")).toBe("2");
    expect(cell.getAttribute("aria-colindex")).toBe("2");
    expect(cell.textContent?.startsWith("B2: ")).toBe(true);
    expect(grid.ariaMirror.announcement.startsWith("B2: ")).toBe(true);
    expect(grid.ariaMirror.live.getAttribute("aria-live")).toBe("polite");
    // The app supplies the formula of the active cell.
    grid.update({ aria: { sheetName: "Budget", activeFormula: "=A1*2" } });
    expect(grid.ariaMirror.announcement).toContain("formula =A1*2");
    grid.destroy();
    expect(root.getAttribute("role")).toBeNull();
  });
});

describe("range pick", () => {
  it("pickRange resolves with the dragged range and reports onPointRange without an edit", async () => {
    const picks: string[] = [];
    const { grid } = make({ onPointRange: (r, phase) => picks.push(`${phase}:${r.r0},${r.c0}-${r.r1},${r.c1}`) });
    const scroller = container.querySelector(".tally-grid")?.children[2] as HTMLElement;
    const p = grid.pickRange();
    expect(grid.rangePicking).toBe(true);
    const geo = grid.getGeometry();
    pointer(scroller, "pointerdown", geo.headerW + 150, geo.headerH + 30);
    pointer(scroller, "pointermove", geo.headerW + 250, geo.headerH + 60);
    pointer(scroller, "pointerup", geo.headerW + 250, geo.headerH + 60);
    const range = await p;
    expect(range).toEqual({ r0: 1, c0: 1, r1: 2, c1: 2 });
    expect(picks[0]).toBe("start:1,1-1,1");
    expect(picks[picks.length - 1]).toBe("end:1,1-2,2");
    expect(grid.rangePicking).toBe(false);
    // The selection itself did not move.
    expect(grid.getSelection().active).toEqual({ r: 0, c: 0 });
    grid.destroy();
  });

  it("the rangePick prop picks with the keyboard; Escape cancels a pending pick", async () => {
    const picks: string[] = [];
    const { grid, ta } = make({ rangePick: true, onPointRange: (r, phase) => picks.push(`${phase}:${r.r0},${r.c0}`) });
    ta.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
    expect(picks).toEqual(["move:1,0"]);
    ta.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    expect(picks).toEqual(["move:1,0", "end:1,0"]);
    const p = grid.pickRange();
    ta.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    expect(await p).toBeNull();
    grid.destroy();
  });
});

describe("outline gutter", () => {
  it("reserves a gutter per level and reports clicks on level and group buttons", () => {
    expect(outlineLevels([])).toBe(0);
    expect(outlineLevels([{ start: 1, end: 5, level: 1, collapsed: false }, { start: 2, end: 3, level: 2, collapsed: true }])).toBe(3);
    const toggles: string[] = [];
    const levels: string[] = [];
    const outline = { rows: [{ start: 2, end: 5, level: 1, collapsed: false }], cols: [], levelPx: 13 };
    const { grid } = make({ outline, onOutlineToggle: (axis, g) => toggles.push(`${axis}:${g.start}-${g.end}`), onOutlineLevel: (axis, l) => levels.push(`${axis}:${l}`) });
    const geo = grid.getGeometry();
    expect(geo.gutterW).toBe(26);
    expect(geo.gutterH).toBe(0);
    expect(geo.headerW).toBeGreaterThan(26 + 40);
    expect(geo.colLeft(0)).toBe(geo.headerW);
    // Hit test: level 2 button in the corner, the group box at row 3 (index 2).
    const g = (grid as unknown as { geo: Parameters<typeof outlineHit>[0] }).geo;
    expect(outlineHit(g, outline, 15, 5)).toEqual({ kind: "level", axis: "rows", level: 2 });
    expect(outlineHit(g, outline, 5, geo.headerH + 2 * 24 + 3)).toEqual({ kind: "group", axis: "rows", group: outline.rows[0] });
    expect(outlineHit(g, outline, 5, geo.headerH + 8 * 24 + 3)).toBeNull();
    expect(outlineHit(g, outline, geo.headerW + 5, geo.headerH + 5)).toBeNull();
    const scroller = container.querySelector(".tally-grid")?.children[2] as HTMLElement;
    pointer(scroller, "pointerdown", 15, 5);
    pointer(scroller, "pointerdown", 5, geo.headerH + 2 * 24 + 3);
    expect(levels).toEqual(["rows:2"]);
    expect(toggles).toEqual(["rows:2-5"]);
    // The row numbers are drawn right of the gutter.
    const rowOne = calls.find((c) => c.name === "fillText" && c.args[0] === "1");
    expect(rowOne && (rowOne.args[1] as number)).toBeGreaterThan(26);
    grid.update({ outline: null });
    expect(grid.getGeometry().gutterW).toBe(0);
    grid.destroy();
  });
});

describe("page break preview and traces", () => {
  it("draws Page N watermarks per page and trace arrows", () => {
    const { grid } = make({
      pageBreaks: { rows: [{ at: 10, manual: false }], cols: [{ at: 4, manual: true }] },
      printRange: { r0: 0, c0: 0, r1: 19, c1: 7 },
      pageBreakPreview: true,
      traces: [{ from: { r: 0, c: 0 }, to: { r: 2, c: 2 }, kind: "precedent" }],
    });
    const pages = calls.filter((c) => c.name === "fillText" && String(c.args[0]).startsWith("Page ")).map((c) => c.args[0]);
    expect(pages).toEqual(["Page 1", "Page 2", "Page 3", "Page 4"]);
    expect(calls.some((c) => c.name === "arc")).toBe(true);
    grid.destroy();
  });
});

describe("geometry and viewport events", () => {
  it("getGeometry matches the canvas geometry and onViewportChange fires on scroll", () => {
    const vps: Viewport[] = [];
    const { grid } = make({ onViewportChange: (vp) => vps.push(vp) });
    const g = grid.getGeometry();
    expect(g.headerH).toBe(24);
    expect(g.rowTop(2)).toBe(24 + 48);
    expect(g.colLeft(0)).toBe(g.headerW);
    expect(g.colLeft(1)).toBeGreaterThan(g.headerW);
    expect(g.zoom).toBe(1);
    expect(g.frozen).toEqual({ rows: 0, cols: 0 });
    const before = vps.length;
    expect(before).toBeGreaterThan(0);
    grid.scrollTo({ r: 20, c: 0 });
    expect(vps.length).toBe(before + 1);
    expect(vps[vps.length - 1]?.scrollY).toBe(480);
    expect(grid.getGeometry().rowTop(20)).toBe(24);
    grid.destroy();
  });
});

describe("split view", () => {
  it("creates two grids over one source with a draggable divider", () => {
    const source = new SyntheticSource(1000, 50);
    const props: GridProps = { source, sheet: 1, theme: DEFAULT_THEME, zoom: 1, selection: cellSelection({ r: 0, c: 0 }), edit: null };
    const split = new SplitView(container, props, { axis: "horizontal", ratio: 0.3 });
    expect(container.querySelectorAll(".tally-grid").length).toBe(2);
    expect(split.ratio).toBe(0.3);
    expect(split.divider.getAttribute("role")).toBe("separator");
    expect(split.root.style.flexDirection).toBe("column");
    split.setRatio(2);
    expect(split.ratio).toBe(0.9);
    split.divider.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowUp", bubbles: true }));
    expect(split.ratio).toBeCloseTo(0.85, 6);
    split.setAxis("vertical");
    expect(split.root.style.flexDirection).toBe("row");
    const sel = cellSelection({ r: 3, c: 3 });
    split.update({ selection: sel });
    expect(split.primary.getSelection().active).toEqual({ r: 3, c: 3 });
    expect(split.secondary.getSelection().active).toEqual({ r: 3, c: 3 });
    split.destroy();
    expect(container.querySelectorAll(".tally-grid").length).toBe(0);
  });
});

describe("view toggles (gridlines, headers)", () => {
  it("hiding the headers drops the header strips from the geometry; gridlines can be turned off", () => {
    const { grid } = make();
    const on = grid.getGeometry();
    expect(on.headerW).toBeGreaterThan(0);
    expect(on.headerH).toBeGreaterThan(0);
    grid.update({ showHeaders: false });
    grid.flush();
    const off = grid.getGeometry();
    expect(off.headerW).toBe(0);
    expect(off.headerH).toBe(0);
    expect(off.colLeft(0)).toBe(0);
    // Gridlines off: the frame draws fewer 1 px rects.
    const thin = () => calls.filter((c) => c.name === "fillRect" && ((c.args[2] as number) <= 1 || (c.args[3] as number) <= 1)).length;
    calls.length = 0;
    grid.update({ showGridlines: true, zoom: 1.0001 });
    grid.flush();
    const withLines = thin();
    calls.length = 0;
    grid.update({ showGridlines: false, zoom: 1 });
    grid.flush();
    expect(thin()).toBeLessThan(withLines);
    // Headers back, with the outline gutter kept left of them.
    grid.update({ showHeaders: true, outline: { rows: [{ start: 1, end: 3, level: 1, collapsed: false }], cols: [], levelPx: 13 } });
    grid.flush();
    expect(grid.getGeometry().headerW).toBeGreaterThan(26);
    grid.destroy();
  });
});
