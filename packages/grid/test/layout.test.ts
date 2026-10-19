import { describe, expect, it } from "vitest";
import { AxisModel, MAX_SCROLL_EXTENT, ScrollAxisMap } from "../src/layout";
import { MAX_COLS, MAX_ROWS } from "../src/types";

describe("AxisModel", () => {
  it("maps a default-only axis", () => {
    const a = new AxisModel(100, 24);
    expect(a.total()).toBe(2400);
    expect(a.indexToPx(0)).toBe(0);
    expect(a.indexToPx(10)).toBe(240);
    expect(a.pxToIndex(0)).toBe(0);
    expect(a.pxToIndex(23.9)).toBe(0);
    expect(a.pxToIndex(24)).toBe(1);
    expect(a.pxToIndex(-50)).toBe(0);
    expect(a.pxToIndex(1e9)).toBe(99);
    expect(a.sizeOf(5)).toBe(24);
  });

  it("builds prefix sums over runs", () => {
    const a = new AxisModel(10, 10, [
      { start: 2, end: 3, size: 50, hidden: false },
      { start: 7, end: 7, size: 5, hidden: false },
    ]);
    // 0:0 1:10 2:20 3:70 4:120 5:130 6:140 7:150 8:155 9:165 end:175
    expect([0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10].map((i) => a.indexToPx(i))).toEqual([0, 10, 20, 70, 120, 130, 140, 150, 155, 165, 175]);
    expect(a.total()).toBe(175);
    expect(a.pxToIndex(69)).toBe(2);
    expect(a.pxToIndex(70)).toBe(3);
    expect(a.pxToIndex(152)).toBe(7);
    expect(a.pxToIndex(155)).toBe(8);
    expect(a.sizeOf(3)).toBe(50);
    expect(a.sizeOf(7)).toBe(5);
  });

  it("gives hidden indices size 0 and never returns them from pxToIndex", () => {
    const a = new AxisModel(10, 10, [{ start: 3, end: 5, size: 10, hidden: true }]);
    expect(a.sizeOf(4)).toBe(0);
    expect(a.isHidden(4)).toBe(true);
    expect(a.indexToPx(3)).toBe(30);
    expect(a.indexToPx(6)).toBe(30);
    expect(a.pxToIndex(29)).toBe(2);
    expect(a.pxToIndex(30)).toBe(6);
    expect(a.total()).toBe(70);
    expect(a.nextVisible(2, 1)).toBe(6);
    expect(a.nextVisible(6, -1)).toBe(2);
  });

  it("handles hidden indices at the end", () => {
    const a = new AxisModel(5, 10, [{ start: 3, end: 4, size: 0, hidden: true }]);
    expect(a.total()).toBe(30);
    expect(a.pxToIndex(1000)).toBe(2);
    expect(a.pxToIndex(29)).toBe(2);
  });

  it("setSize splits and merges runs", () => {
    const a = new AxisModel(20, 10);
    a.setSize(5, 9, 30);
    expect(a.getRuns()).toEqual([{ start: 5, end: 9, size: 30, hidden: false }]);
    a.setSize(7, 7, 10); // back to default in the middle
    expect(a.getRuns()).toEqual([
      { start: 5, end: 6, size: 30, hidden: false },
      { start: 8, end: 9, size: 30, hidden: false },
    ]);
    a.setSize(7, 7, 30);
    expect(a.getRuns()).toEqual([{ start: 5, end: 9, size: 30, hidden: false }]);
    expect(a.total()).toBe(15 * 10 + 5 * 30);
    a.setSize(0, 19, 0, true);
    expect(a.total()).toBe(0);
  });

  it("maps 1,048,576 rows without per-row arrays", () => {
    const a = new AxisModel(MAX_ROWS, 24, [
      { start: 1000, end: 1999, size: 0, hidden: true },
      { start: 500_000, end: 500_000, size: 100, hidden: false },
    ]);
    const total = (MAX_ROWS - 1000 - 1) * 24 + 100;
    expect(a.total()).toBe(total);
    expect(a.indexToPx(MAX_ROWS)).toBe(total);
    expect(a.pxToIndex(total - 1)).toBe(MAX_ROWS - 1);
    expect(a.indexToPx(MAX_ROWS - 1)).toBe(total - 24);
    expect(a.pxToIndex(999 * 24)).toBe(999);
    expect(a.pxToIndex(1000 * 24)).toBe(2000);
    expect(a.indexToPx(500_001) - a.indexToPx(500_000)).toBe(100);
    // Round trip at random rows.
    for (const r of [0, 1, 999, 2000, 123_456, 500_000, 500_001, 1_048_575]) {
      expect(a.pxToIndex(a.indexToPx(r))).toBe(r);
      expect(a.pxToIndex(a.indexToPx(r) + a.sizeOf(r) - 0.5)).toBe(r);
    }
    expect(new AxisModel(MAX_COLS, 100).total()).toBe(1_638_400);
  });

  it("queries are fast with many runs", () => {
    const runs = Array.from({ length: 10_000 }, (_, i) => ({ start: i * 100, end: i * 100 + 10, size: 40, hidden: false }));
    const a = new AxisModel(MAX_ROWS, 24, runs);
    const t0 = performance.now();
    let s = 0;
    for (let i = 0; i < 100_000; i++) s += a.pxToIndex((i * 251) % a.total());
    expect(s).toBeGreaterThan(0);
    expect(performance.now() - t0).toBeLessThan(500);
  });
});

describe("ScrollAxisMap (scaled scrolling)", () => {
  it("is the identity below the cap", () => {
    const m = new ScrollAxisMap(5_000_000, 800);
    expect(m.scaled).toBe(false);
    expect(m.extent).toBe(5_000_000);
    expect(m.toContent(1234)).toBe(1234);
    expect(m.toScroll(1234)).toBe(1234);
    expect(m.toContent(1e9)).toBe(5_000_000 - 800);
  });

  it("scales 1M rows × 24 px proportionally into the cap", () => {
    const content = MAX_ROWS * 24; // 25,165,824
    const view = 800;
    const m = new ScrollAxisMap(content, view);
    expect(m.scaled).toBe(true);
    expect(m.extent).toBe(MAX_SCROLL_EXTENT);
    const maxScroll = MAX_SCROLL_EXTENT - view;
    expect(m.toContent(0)).toBe(0);
    expect(m.toContent(maxScroll)).toBeCloseTo(content - view, 6);
    expect(m.toContent(maxScroll / 2)).toBeCloseTo((content - view) / 2, 6);
    expect(m.ratio).toBeCloseTo((content - view) / maxScroll, 12);
    // Inverse.
    for (const y of [0, 24, 1_000_000, 12_345_678, content - view]) expect(m.toContent(m.toScroll(y))).toBeCloseTo(y, 6);
    // The last row is reachable.
    expect(Math.floor(m.toContent(maxScroll) / 24)).toBeGreaterThanOrEqual(MAX_ROWS - Math.ceil(view / 24));
  });

  it("clamps", () => {
    const m = new ScrollAxisMap(30_000_000, 1000);
    expect(m.toContent(-5)).toBe(0);
    expect(m.toContent(1e12)).toBe(m.maxContent);
    expect(m.toScroll(-5)).toBe(0);
  });
});
