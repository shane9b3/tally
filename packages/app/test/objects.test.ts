// @vitest-environment happy-dom
/**
 * W5-5 objects: drag/resize maths, arrow nudges, anchor px placement at
 * zoom 1.25 with scroll (chart layer), viewport culling, and a drag on the
 * object layer emitting exactly one `moveResizeChart` op.
 */
import type { ChartDto } from "@tally/bridge";
import { AxisModel, anchorToPx, pxToAnchor, type AnchorAxes } from "@tally/grid";
import { describe, expect, it, vi } from "vitest";
import { nodeVisible } from "../src/charts/chartMounts";
import { objectId, objectsFor, parseObjectId, useChartStore } from "../src/charts/chartStore";
import { defaultChart } from "../src/charts/chartFeatures";
import { ObjectFeatures } from "../src/objects/objectFeatures";
import { anchorChanged, dragRect, nudgeAnchor, roundAnchor } from "../src/objects/objectDrag";

const axes = (zoom = 1): AnchorAxes => ({ rows: new AxisModel(1000, 24), cols: new AxisModel(100, 100), zoom });

describe("drag maths", () => {
  const start = { x: 100, y: 50, w: 200, h: 100 };
  it("moves the body and resizes from each handle", () => {
    expect(dragRect(start, "body", 10, -5)).toEqual({ x: 110, y: 45, w: 200, h: 100 });
    expect(dragRect(start, "se", 10, 20)).toEqual({ x: 100, y: 50, w: 210, h: 120 });
    expect(dragRect(start, "nw", 10, 20)).toEqual({ x: 110, y: 70, w: 190, h: 80 });
    expect(dragRect(start, "e", -30, 99)).toEqual({ x: 100, y: 50, w: 170, h: 100 });
    expect(dragRect(start, "n", 99, -10)).toEqual({ x: 100, y: 40, w: 200, h: 110 });
  });
  it("never goes below the minimum size or above the sheet origin", () => {
    expect(dragRect(start, "se", -500, -500)).toEqual({ x: 100, y: 50, w: 24, h: 24 });
    expect(dragRect(start, "nw", 500, 500)).toEqual({ x: 276, y: 126, w: 24, h: 24 });
    expect(dragRect(start, "body", -500, -500)).toEqual({ x: 0, y: 0, w: 200, h: 100 });
  });
  it("nudges by 1 mm (10 mm with Shift) and keeps the size", () => {
    const a = { r: 2, c: 1, dxMm: 0, dyMm: 0, widthMm: 160, heightMm: 90 };
    const right = nudgeAnchor(a, "ArrowRight", false, axes())!;
    expect(right.c).toBe(1);
    expect(right.dxMm).toBeCloseTo(1, 5);
    expect(right.widthMm).toBe(160);
    const far = nudgeAnchor(a, "ArrowDown", true, axes())!;
    expect(far.r).toBe(3);
    expect(far.dyMm).toBeCloseTo(10 - 24 / (96 / 25.4), 5);
    expect(nudgeAnchor(a, "Enter", false, axes())).toBeNull();
    expect(nudgeAnchor({ ...a, dxMm: 0.5 }, "ArrowLeft", false, axes())?.c).toBe(0);
  });
  it("rounds anchors and detects changes", () => {
    expect(roundAnchor({ r: 1, c: 1, dxMm: 1.23456, dyMm: 0, widthMm: 160.004, heightMm: 90 }).dxMm).toBe(1.23);
    const a = { r: 1, c: 1, dxMm: 1, dyMm: 1, widthMm: 160, heightMm: 90 };
    expect(anchorChanged(a, { ...a, dxMm: 1.004 })).toBe(false);
    expect(anchorChanged(a, { ...a, r: 2 })).toBe(true);
  });
});

describe("chart layer placement", () => {
  it("anchor mm → px at zoom 1.25 with scroll, and back", () => {
    const ax = axes(1.25);
    const a = { r: 4, c: 2, dxMm: 5, dyMm: 2, widthMm: 160, heightMm: 90 };
    const px = anchorToPx(a, ax);
    const k = (96 / 25.4) * 1.25;
    expect(px.x).toBeCloseTo(200 * 1.25 + 5 * k, 6);
    expect(px.y).toBeCloseTo(96 * 1.25 + 2 * k, 6);
    expect(px.w).toBeCloseTo(160 * k, 6);
    const back = pxToAnchor(px, ax);
    expect(back.r).toBe(4);
    expect(back.c).toBe(2);
    expect(back.dxMm).toBeCloseTo(5, 6);
    expect(back.widthMm).toBeCloseTo(160, 6);
    // A rect scrolled past the layer's origin keeps its content coordinates (scroll is applied by the layer transform).
    const geo = { colLeft: (c: number) => 44 + c * 100 - 150, rowTop: (r: number) => 24 + r * 24 - 40, headerW: 44, headerH: 24, gutterW: 0, gutterH: 0, scrollX: 150, scrollY: 40, frozen: { rows: 0, cols: 0 }, zoom: 1, width: 800, height: 600 };
    expect(nodeVisible({ x: 100, y: 10, w: 300, h: 200 }, geo)).toBe(true);
    expect(nodeVisible({ x: 1000, y: 10, w: 300, h: 200 }, geo)).toBe(false);
    expect(nodeVisible({ x: 0, y: 0, w: 100, h: 30 }, geo)).toBe(false);
  });

  it("builds layer objects from charts and pictures with stable ids", () => {
    const c = { ...defaultChart("Sheet1", { r0: 0, c0: 0, r1: 5, c1: 2 }), id: 7, title: "Sales" };
    const objs = objectsFor([c], [{ id: 3, name: "logo.png", anchor: c.anchor, mime: "image/png", data: "" }]);
    expect(objs.map((o) => o.id)).toEqual(["chart:7", "image:3"]);
    expect(objs[0]?.label).toBe("Sales");
    expect(parseObjectId(objectId("image", 3))).toEqual({ kind: "image", id: 3 });
    expect(parseObjectId("bogus")).toBeNull();
    useChartStore.getState().setObjects(1, [c], []);
    useChartStore.getState().select("chart:7");
    useChartStore.getState().setObjects(1, [], []);
    expect(useChartStore.getState().selected).toBeNull();
  });
});

describe("a drag on the object layer", () => {
  it("emits exactly one moveResizeChart op on pointer-up", async () => {
    const ops: unknown[][] = [];
    const chart: ChartDto = { ...defaultChart("Sheet1", { r0: 0, c0: 0, r1: 5, c1: 2 }), id: 1 };
    useChartStore.getState().setObjects(1, [chart], []);
    const node = document.createElement("div");
    const start = anchorToPx(chart.anchor, axes());
    Object.assign(node.style, { left: `${start.x}px`, top: `${start.y}px`, width: `${start.w}px`, height: `${start.h}px` });
    document.body.appendChild(node);
    const grid = {
      objectLayer: { elementFor: () => node, list: () => objectsFor([chart], []), setObjects: () => undefined, select: () => undefined },
      objectRect: () => ({ x: start.x, y: start.y, w: start.w, h: start.h }),
      anchorAxes: () => axes(),
    };
    const ctl = {
      grid,
      sheet: 1,
      info: { doc: 1 },
      selection: { active: { r: 0, c: 0 } },
      apply: async (label: string, list: unknown[]) => {
        ops.push([label, ...list]);
        return { errors: [] };
      },
      focusGrid: () => undefined,
      notifyError: () => undefined,
    };
    const f = new ObjectFeatures(ctl as never);
    const ev = (type: string, x: number, y: number) => new PointerEvent(type, { bubbles: true, clientX: x, clientY: y, button: 0, pointerId: 1 });
    f.onObjectPointerDown("chart:1", "body", ev("pointerdown", 10, 10));
    node.dispatchEvent(ev("pointermove", 30, 10));
    node.dispatchEvent(ev("pointermove", 60, 34));
    expect(parseFloat(node.style.left)).toBeCloseTo(start.x + 50, 3);
    node.dispatchEvent(ev("pointerup", 60, 34));
    await new Promise((r) => setTimeout(r, 0));
    expect(ops).toHaveLength(1);
    const op = ops[0]?.[1] as { type: string; anchor: { c: number; r: number; dxMm: number } };
    expect(ops[0]?.[0]).toBe("Move chart");
    expect(op.type).toBe("moveResizeChart");
    expect(op.anchor.c).toBe(3);
    expect(op.anchor.r).toBe(1);
    // A click without movement sends nothing.
    f.onObjectPointerDown("chart:1", "body", ev("pointerdown", 10, 10));
    node.dispatchEvent(ev("pointerup", 10, 10));
    await new Promise((r) => setTimeout(r, 0));
    expect(ops).toHaveLength(1);
    node.remove();
  });
});

describe("chart data refresh budget (review 4 C3)", () => {
  it("fetches at most 20 charts, 4 at a time, and coalesces chartsChanged", async () => {
    let inFlight = 0;
    let maxInFlight = 0;
    let calls = 0;
    const client = {
      chartData: async (_doc: number, sheet: number, id: number) => {
        calls++;
        inFlight++;
        maxInFlight = Math.max(maxInFlight, inFlight);
        await new Promise((r) => setTimeout(r, 1));
        inFlight--;
        return { chart: { sheet, id }, version: 1, categories: [], series: [], truncated: false, problems: [] };
      },
    };
    const ctl = { info: { doc: 1 }, sheet: 1, client, grid: null };
    useChartStore.getState().setObjects(1, [], []);
    const f = new ObjectFeatures(ctl as never);
    const ids = Array.from({ length: 1000 }, (_, i) => i + 1);
    // Three bursts inside the debounce window become one flush of at most 20 fetches.
    const p = [f.scheduleRefresh(ids), f.scheduleRefresh(ids), f.scheduleRefresh(ids)];
    await Promise.all(p);
    expect(calls).toBe(20);
    expect(maxInFlight).toBeLessThanOrEqual(4);
    vi.restoreAllMocks();
  });
});
