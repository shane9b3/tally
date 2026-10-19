/**
 * Viewport tile tracking (docs/architecture-plan.md §4.1).
 *
 * The sheet is cut into tiles of 64 rows × 32 columns keyed by
 * `(sheet, tileRow, tileCol)`. The grid asks the data source for missing
 * visible tiles plus a one-tile prefetch ring around them, at most 8 tiles
 * per `ensureRanges` call (visible tiles first). The tracker remembers what
 * it already asked for (with the invalidation version it asked at), so a
 * steady viewport issues no requests; dirty notifications from the source
 * forget the intersecting tiles so they are asked for again.
 *
 * The data itself lives in the data source (see engineSource.ts, which uses
 * the same tile geometry); `ensureRanges` must be idempotent.
 */
import type { RangeDto } from "@tally/bridge";
import type { DirtySet } from "./types";

export const TILE_ROWS = 64;
export const TILE_COLS = 32;
/** Most tiles per `ensureRanges` call (one IPC invoke). */
export const MAX_TILES_PER_BATCH = 8;

export interface TileRequester {
  ensureRanges(ranges: RangeDto[]): void;
}

export interface TileTrackerOptions {
  tileRows?: number;
  tileCols?: number;
  maxBatch?: number;
  /** Forget the least recently touched tiles beyond this many. */
  capacity?: number;
  /** Prefetch ring width in tiles (default 1). */
  ring?: number;
}

interface Entry {
  tr: number;
  tc: number;
  version: number;
}

export function tileKey(sheet: number, tr: number, tc: number): string {
  return `${sheet}:${tr}:${tc}`;
}

/** The cell range of a tile, clamped to the sheet. */
export function tileRange(tr: number, tc: number, rows: number, cols: number, tileRows = TILE_ROWS, tileCols = TILE_COLS): RangeDto {
  const r0 = tr * tileRows;
  const c0 = tc * tileCols;
  return { r0, c0, r1: Math.min(rows - 1, r0 + tileRows - 1), c1: Math.min(cols - 1, c0 + tileCols - 1) };
}

export function intersects(a: RangeDto, b: RangeDto): boolean {
  return a.r0 <= b.r1 && b.r0 <= a.r1 && a.c0 <= b.c1 && b.c0 <= a.c1;
}

export class TileTracker {
  private readonly tileRows: number;
  private readonly tileCols: number;
  private readonly maxBatch: number;
  private readonly capacity: number;
  private readonly ring: number;
  private readonly entries = new Map<string, Entry>();
  private sheet = 0;
  /** Bumped by every invalidation; entries remember the version they were requested at. */
  private version = 0;

  constructor(
    private readonly requester: TileRequester,
    private rows: number,
    private cols: number,
    opts: TileTrackerOptions = {},
  ) {
    this.tileRows = opts.tileRows ?? TILE_ROWS;
    this.tileCols = opts.tileCols ?? TILE_COLS;
    this.maxBatch = opts.maxBatch ?? MAX_TILES_PER_BATCH;
    this.capacity = opts.capacity ?? 1024;
    this.ring = opts.ring ?? 1;
  }

  /** Switches sheet (or source): forgets everything. */
  reset(sheet: number, rows = this.rows, cols = this.cols): void {
    this.sheet = sheet;
    this.rows = rows;
    this.cols = cols;
    this.entries.clear();
    this.version++;
  }

  /** Number of tiles currently remembered as requested. */
  get size(): number {
    return this.entries.size;
  }

  isRequested(tr: number, tc: number): boolean {
    return this.entries.has(tileKey(this.sheet, tr, tc));
  }

  /**
   * Requests the missing tiles of `view` and of the prefetch ring around it.
   * Returns the ranges requested, in request order.
   */
  ensureVisible(view: RangeDto): RangeDto[] {
    const tr0 = Math.max(0, Math.floor(view.r0 / this.tileRows));
    const tr1 = Math.floor(Math.min(view.r1, this.rows - 1) / this.tileRows);
    const tc0 = Math.max(0, Math.floor(view.c0 / this.tileCols));
    const tc1 = Math.floor(Math.min(view.c1, this.cols - 1) / this.tileCols);
    const maxTr = Math.floor((this.rows - 1) / this.tileRows);
    const maxTc = Math.floor((this.cols - 1) / this.tileCols);

    const missing: RangeDto[] = [];
    const visit = (tr: number, tc: number): void => {
      const key = tileKey(this.sheet, tr, tc);
      const e = this.entries.get(key);
      if (e) {
        // Touch for LRU order.
        this.entries.delete(key);
        this.entries.set(key, e);
        return;
      }
      this.entries.set(key, { tr, tc, version: this.version });
      missing.push(tileRange(tr, tc, this.rows, this.cols, this.tileRows, this.tileCols));
    };
    for (let tr = tr0; tr <= tr1; tr++) for (let tc = tc0; tc <= tc1; tc++) visit(tr, tc);
    const rr0 = Math.max(0, tr0 - this.ring);
    const rr1 = Math.min(maxTr, tr1 + this.ring);
    const rc0 = Math.max(0, tc0 - this.ring);
    const rc1 = Math.min(maxTc, tc1 + this.ring);
    for (let tr = rr0; tr <= rr1; tr++) {
      for (let tc = rc0; tc <= rc1; tc++) {
        if (tr >= tr0 && tr <= tr1 && tc >= tc0 && tc <= tc1) continue;
        visit(tr, tc);
      }
    }
    for (let i = 0; i < missing.length; i += this.maxBatch) {
      this.requester.ensureRanges(missing.slice(i, i + this.maxBatch));
    }
    this.evict();
    return missing;
  }

  /** Forgets tiles touched by `dirty` so the next `ensureVisible` asks again. */
  invalidate(dirty: DirtySet): void {
    this.version++;
    if (dirty === "all") {
      this.entries.clear();
      return;
    }
    if (dirty.length === 0) return;
    for (const [key, e] of this.entries) {
      const tr = tileRange(e.tr, e.tc, this.rows, this.cols, this.tileRows, this.tileCols);
      if (dirty.some((d) => intersects(d, tr))) this.entries.delete(key);
    }
  }

  private evict(): void {
    const over = this.entries.size - this.capacity;
    if (over <= 0) return;
    let n = 0;
    for (const key of this.entries.keys()) {
      if (n++ >= over) break;
      this.entries.delete(key);
    }
  }
}
