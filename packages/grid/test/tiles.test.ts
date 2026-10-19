import { describe, expect, it } from "vitest";
import type { RangeDto } from "@tally/bridge";
import { MAX_TILES_PER_BATCH, TILE_COLS, TILE_ROWS, TileTracker, tileRange } from "../src/tiles";
import { MAX_COLS, MAX_ROWS } from "../src/types";

function tracker(opts = {}) {
  const calls: RangeDto[][] = [];
  const t = new TileTracker({ ensureRanges: (r) => calls.push(r) }, MAX_ROWS, MAX_COLS, opts);
  t.reset(1);
  return { t, calls, all: () => calls.flat() };
}

const key = (r: RangeDto): string => `${r.r0 / TILE_ROWS}:${r.c0 / TILE_COLS}`;

describe("TileTracker", () => {
  it("uses 64 × 32 tiles clamped to the sheet", () => {
    expect(tileRange(0, 0, MAX_ROWS, MAX_COLS)).toEqual({ r0: 0, c0: 0, r1: 63, c1: 31 });
    expect(tileRange(2, 3, MAX_ROWS, MAX_COLS)).toEqual({ r0: 128, c0: 96, r1: 191, c1: 127 });
    expect(tileRange(0, 0, 10, 5)).toEqual({ r0: 0, c0: 0, r1: 9, c1: 4 });
  });

  it("requests visible tiles first, then the one-tile prefetch ring", () => {
    const { t, all } = tracker();
    // A viewport inside tile (2, 1) only.
    t.ensureVisible({ r0: 130, c0: 40, r1: 170, c1: 50 });
    const keys = all().map(key);
    expect(keys[0]).toBe("2:1");
    expect(new Set(keys)).toEqual(new Set(["2:1", "1:0", "1:1", "1:2", "2:0", "2:2", "3:0", "3:1", "3:2"]));
    expect(keys.length).toBe(9);
  });

  it("batches at most 8 tiles per ensureRanges call", () => {
    const { t, calls, all } = tracker();
    // 3 × 3 visible tiles + ring of 16 = 25 tiles.
    t.ensureVisible({ r0: 64, c0: 32, r1: 64 * 4 - 1, c1: 32 * 4 - 1 });
    expect(all().length).toBe(25);
    expect(calls.length).toBe(4);
    for (const c of calls) expect(c.length).toBeLessThanOrEqual(MAX_TILES_PER_BATCH);
    // The 9 visible tiles come before any ring tile.
    const visible = new Set(["1:1", "1:2", "1:3", "2:1", "2:2", "2:3", "3:1", "3:2", "3:3"]);
    expect(all().slice(0, 9).every((r) => visible.has(key(r)))).toBe(true);
  });

  it("clamps the ring at the sheet edges", () => {
    const { t, all } = tracker();
    t.ensureVisible({ r0: 0, c0: 0, r1: 40, c1: 10 });
    expect(new Set(all().map(key))).toEqual(new Set(["0:0", "0:1", "1:0", "1:1"]));
    const end = tracker();
    end.t.ensureVisible({ r0: MAX_ROWS - 30, c0: MAX_COLS - 5, r1: MAX_ROWS - 1, c1: MAX_COLS - 1 });
    expect(end.all().length).toBe(4);
  });

  it("does not ask again for tiles it already requested", () => {
    const { t, calls } = tracker();
    const view = { r0: 0, c0: 0, r1: 40, c1: 10 };
    t.ensureVisible(view);
    const n = calls.length;
    t.ensureVisible(view);
    t.ensureVisible({ r0: 10, c0: 2, r1: 50, c1: 12 });
    expect(calls.length).toBe(n);
    // Scrolling down one tile asks only for the new ring row.
    t.ensureVisible({ r0: 64, c0: 0, r1: 100, c1: 10 });
    expect(calls.slice(n).flat().map(key).sort()).toEqual(["2:0", "2:1"]);
  });

  it("invalidates tiles intersecting dirty ranges", () => {
    const { t, calls } = tracker();
    const view = { r0: 0, c0: 0, r1: 40, c1: 10 };
    t.ensureVisible(view);
    const n = calls.length;
    t.invalidate([{ r0: 5, c0: 5, r1: 5, c1: 5 }]);
    expect(t.isRequested(0, 0)).toBe(false);
    expect(t.isRequested(1, 1)).toBe(true);
    t.ensureVisible(view);
    expect(calls.slice(n).flat().map(key)).toEqual(["0:0"]);
    // A whole-column change hits every tile in that column.
    t.invalidate([{ r0: 0, c0: 40, r1: MAX_ROWS - 1, c1: 40 }]);
    expect(t.isRequested(0, 1)).toBe(false);
    expect(t.isRequested(1, 1)).toBe(false);
    expect(t.isRequested(0, 0)).toBe(true);
    t.invalidate("all");
    expect(t.size).toBe(0);
  });

  it("reset forgets everything (sheet switch)", () => {
    const { t, calls } = tracker();
    t.ensureVisible({ r0: 0, c0: 0, r1: 10, c1: 10 });
    const n = calls.length;
    t.reset(2);
    t.ensureVisible({ r0: 0, c0: 0, r1: 10, c1: 10 });
    expect(calls.length).toBeGreaterThan(n);
  });

  it("evicts least recently used tiles beyond capacity", () => {
    const { t } = tracker({ capacity: 20 });
    for (let i = 0; i < 50; i++) t.ensureVisible({ r0: i * 64 * 3, c0: 0, r1: i * 64 * 3 + 10, c1: 10 });
    expect(t.size).toBeLessThanOrEqual(20);
    expect(t.isRequested(0, 0)).toBe(false);
  });
});
