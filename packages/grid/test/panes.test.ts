import { describe, expect, it } from "vitest";
import { Geometry } from "../src/geometry";
import { AxisModel } from "../src/layout";
import { filterButtonRect } from "../src/renderer";
import { cellSelection, expandToMerges, extendTo, snapToMerges } from "../src/selection";

const rows = () => new AxisModel(1000, 24);
const cols = () => new AxisModel(100, 100);
/** 2 frozen rows, 1 frozen column, scrolled by 1000 px right and 240 px down. */
const frozenGeo = () => new Geometry(rows(), cols(), 1, 44, 24, 1000, 240, 800, 600, { rows: 2, cols: 1 });

describe("frozen panes", () => {
  it("frozen rows and columns do not scroll; the rest does", () => {
    const g = frozenGeo();
    expect(g.frozenW).toBe(100);
    expect(g.frozenH).toBe(48);
    expect(g.colX(0)).toBe(44);
    expect(g.rowY(1)).toBe(24 + 24);
    expect(g.colX(1)).toBe(44 + 100 - 1000);
    expect(g.rowY(2)).toBe(24 + 48 - 240);
    // The first scrolling cell visible right of / below the dividers.
    expect(g.colAt(44 + 100)).toBe(11);
    expect(g.rowAt(24 + 48)).toBe(12);
    expect(g.colAt(60)).toBe(0);
    expect(g.rowAt(30)).toBe(0);
  });

  it("returns four panes, frozen ones first", () => {
    const panes = frozenGeo().panes();
    expect(panes).toHaveLength(4);
    expect(panes[0]).toMatchObject({ r0: 0, r1: 1, c0: 0, c1: 0, frozenRows: true, frozenCols: true });
    expect(panes[0]?.clip).toEqual({ x: 44, y: 24, w: 100, h: 48 });
    expect(panes[3]).toMatchObject({ r0: 12, c0: 11, frozenRows: false, frozenCols: false });
    expect(panes[3]?.clip).toEqual({ x: 144, y: 72, w: 656, h: 528 });
    // Without freezing there is a single pane.
    expect(new Geometry(rows(), cols(), 1, 44, 24, 0, 0, 800, 600).panes()).toHaveLength(1);
  });

  it("cuts ranges scrolled under the frozen panes", () => {
    const g = frozenGeo();
    // Rows 3..5 are scrolled above the visible body: zero height.
    expect(g.rangeRect({ r0: 3, c0: 12, r1: 5, c1: 12 }).h).toBe(0);
    // A range crossing the frozen row edge starts in the frozen area and continues in the body.
    const r = g.rangeRect({ r0: 0, c0: 0, r1: 12, c1: 0 });
    expect(r.y).toBe(24);
    expect(r.y + r.h).toBe(g.rowBottom(12));
  });

  it("reveals cells below the frozen rows, not under them", () => {
    const g = frozenGeo();
    const next = g.scrollToReveal({ r: 5, c: 3 });
    // Row 5 must start at the frozen edge (y = 48 in content px).
    expect(next?.y).toBe(5 * 24 - 48);
    expect(next?.x).toBe(3 * 100 - 100);
    // Frozen cells never scroll.
    expect(g.scrollToReveal({ r: 0, c: 0 })).toBeNull();
  });
});

describe("merged cells", () => {
  const merges = [{ r0: 0, c0: 0, r1: 0, c1: 2 }];
  const lim = { rows: 100, cols: 100 };

  it("expands ranges to whole merged areas", () => {
    expect(expandToMerges({ r0: 0, c0: 1, r1: 1, c1: 1 }, merges)).toEqual({ r0: 0, c0: 0, r1: 1, c1: 2 });
    expect(expandToMerges({ r0: 3, c0: 3, r1: 3, c1: 3 }, merges)).toEqual({ r0: 3, c0: 3, r1: 3, c1: 3 });
  });

  it("snaps a click inside a merge to its anchor and whole area", () => {
    const s = snapToMerges(cellSelection({ r: 0, c: 2 }), null, merges, lim);
    expect(s.active).toEqual({ r: 0, c: 0 });
    expect(s.range).toEqual({ r0: 0, c0: 0, r1: 0, c1: 2 });
  });

  it("an arrow key from the anchor jumps past the merge", () => {
    const at = snapToMerges(cellSelection({ r: 0, c: 0 }), null, merges, lim);
    const right = snapToMerges(cellSelection({ r: 0, c: 1 }), at, merges, lim);
    expect(right.active).toEqual({ r: 0, c: 3 });
    expect(right.range).toEqual({ r0: 0, c0: 3, r1: 0, c1: 3 });
    const down = snapToMerges(cellSelection({ r: 1, c: 0 }), at, merges, lim);
    expect(down.active).toEqual({ r: 1, c: 0 });
  });

  it("extending a selection into a merge covers all of it", () => {
    const s = snapToMerges(extendTo(cellSelection({ r: 2, c: 1 }), { r: 0, c: 1 }), null, merges, lim);
    expect(s.range).toEqual({ r0: 0, c0: 0, r1: 2, c1: 2 });
    expect(s.anchor).toEqual({ r: 2, c: 1 });
  });

  it("cellBox covers the merged area", () => {
    const g = new Geometry(rows(), cols(), 1, 44, 24, 0, 0, 800, 600, undefined, merges);
    expect(g.cellBox(0, 0)).toEqual({ x: 44, y: 24, w: 300, h: 24 });
    expect(g.cellBox(1, 0)).toEqual({ x: 44, y: 48, w: 100, h: 24 });
  });
});

describe("filter buttons", () => {
  it("sit at the right edge of the header cell", () => {
    const g = new Geometry(rows(), cols(), 1, 44, 24, 0, 0, 800, 600);
    const b = filterButtonRect(g, 0, 1);
    expect(b.w).toBe(16);
    expect(b.x + b.w).toBeLessThanOrEqual(g.colRight(1));
    expect(b.x).toBeGreaterThan(g.colX(1) + 50);
    expect(b.y).toBe(24 + 4);
  });
});
