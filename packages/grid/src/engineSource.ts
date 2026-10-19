/**
 * `EngineClientSource`: a {@link GridDataSource} backed by an
 * `EngineClient` (MockClient now, TauriClient in the app).
 *
 * - Cells are cached per tile (64 × 32, the grid's tile geometry), fetched
 *   with `client.cells` (one call per `ensureRanges`, ≤ 8 tiles), evicted
 *   LRU beyond `capacity` tiles.
 * - Changed ranges from `ApplyResult.changed` (pass results of
 *   apply/undo/redo to {@link applyResult}; `doc-changed` events are
 *   subscribed automatically) mark tiles stale: their old cells stay visible
 *   until the refetch lands, so edits do not flicker. A response for a tile
 *   that changed again while in flight is dropped.
 * - Styles are fetched by id on first sight (`client.styles`) and cached;
 *   style ids are immutable.
 * - The layout comes from `client.sheetLayout`; it is refetched when
 *   `layoutChanged` names the sheet or cells changed (the used range moves).
 */
import type { ApplyResult, CellView, DocId, EngineClient, RangeDto, SheetLayout, StyleDto } from "@tally/bridge";
import { intersects, MAX_TILES_PER_BATCH, TILE_COLS, TILE_ROWS } from "./tiles";
import { MAX_COLS, MAX_ROWS, type DirtySet, type GridDataSource } from "./types";

export interface EngineClientSourceOptions {
  /** Tiles kept in memory (default 512). */
  capacity?: number;
  /** Subscribe to `client.onDocChanged` (default true). */
  listenToDocChanged?: boolean;
  /** Fetch errors (default: console.warn). */
  onError?: (e: unknown) => void;
}

interface Tile {
  tr: number;
  tc: number;
  /** Local index (row * TILE_COLS + col) → cell. */
  cells: Map<number, CellView>;
  ready: boolean;
  stale: boolean;
  /** Bumped whenever the tile's contents are known to have changed. */
  seq: number;
  /** `seq` at which the in-flight request was made, or -1. */
  inflight: number;
}

const tileId = (tr: number, tc: number): number => tr * 1024 + tc;

function defaultLayout(sheet: number): SheetLayout {
  return {
    sheet,
    used: null,
    defaultColWidthPx: 100,
    defaultRowHeightPx: 24,
    colRuns: [],
    rowRuns: [],
    frozen: null,
    merges: [],
  };
}

export class EngineClientSource implements GridDataSource {
  private readonly tiles = new Map<number, Tile>();
  private readonly styles = new Map<number, StyleDto>();
  private readonly pendingStyles = new Set<number>();
  private readonly requestedStyles = new Set<number>();
  private readonly listeners = new Set<(d: DirtySet) => void>();
  private readonly capacity: number;
  private readonly onError: (e: unknown) => void;
  private layout: SheetLayout;
  private layoutSeq = 0;
  private layoutQueued = false;
  private styleQueued = false;
  private disposed = false;
  private readonly offDoc: (() => void) | null;
  /** Number of `client.cells` calls made (tests, perf). */
  fetches = 0;

  constructor(
    private readonly client: EngineClient,
    private readonly doc: DocId,
    private sheet: number,
    opts: EngineClientSourceOptions = {},
  ) {
    this.capacity = opts.capacity ?? 512;
    this.onError = opts.onError ?? ((e) => console.warn("@tally/grid EngineClientSource:", e));
    this.layout = defaultLayout(sheet);
    this.offDoc =
      opts.listenToDocChanged === false
        ? null
        : client.onDocChanged((e) => {
            if (e.doc === this.doc) this.applyResult(e.result);
          });
    void this.refreshLayout();
    this.queueStyle(0);
  }

  get sheetId(): number {
    return this.sheet;
  }

  /** Switches to another sheet of the same document (drops the cache). */
  setSheet(sheet: number): void {
    if (sheet === this.sheet) return;
    this.sheet = sheet;
    this.tiles.clear();
    this.layout = defaultLayout(sheet);
    void this.refreshLayout();
    this.emit("all");
  }

  dispose(): void {
    this.disposed = true;
    this.offDoc?.();
    this.listeners.clear();
    this.tiles.clear();
  }

  // -- GridDataSource ---------------------------------------------------------

  getLayout(_sheet: number): SheetLayout {
    return this.layout;
  }

  getCell(r: number, c: number): CellView | undefined {
    const tr = Math.floor(r / TILE_ROWS);
    const tc = Math.floor(c / TILE_COLS);
    const tile = this.tiles.get(tileId(tr, tc));
    if (!tile) return undefined;
    return tile.cells.get((r - tr * TILE_ROWS) * TILE_COLS + (c - tc * TILE_COLS));
  }

  getStyle(id: number): StyleDto | undefined {
    const s = this.styles.get(id);
    if (!s) this.queueStyle(id);
    return s;
  }

  ensureRanges(ranges: RangeDto[]): void {
    if (this.disposed) return;
    const want: Tile[] = [];
    for (const rg of ranges) {
      const tr0 = Math.max(0, Math.floor(rg.r0 / TILE_ROWS));
      const tr1 = Math.floor(Math.min(rg.r1, MAX_ROWS - 1) / TILE_ROWS);
      const tc0 = Math.max(0, Math.floor(rg.c0 / TILE_COLS));
      const tc1 = Math.floor(Math.min(rg.c1, MAX_COLS - 1) / TILE_COLS);
      for (let tr = tr0; tr <= tr1; tr++) {
        for (let tc = tc0; tc <= tc1; tc++) {
          const id = tileId(tr, tc);
          let tile = this.tiles.get(id);
          if (tile) {
            this.tiles.delete(id); // LRU touch
          } else {
            tile = { tr, tc, cells: new Map(), ready: false, stale: false, seq: 0, inflight: -1 };
          }
          this.tiles.set(id, tile);
          const fresh = tile.ready && !tile.stale;
          if (!fresh && tile.inflight !== tile.seq) want.push(tile);
        }
      }
    }
    for (let i = 0; i < want.length; i += MAX_TILES_PER_BATCH) this.fetch(want.slice(i, i + MAX_TILES_PER_BATCH));
    this.evict();
  }

  subscribe(fn: (dirty: DirtySet) => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  // -- Engine results ---------------------------------------------------------

  /** Feeds an apply/undo/redo result (or a `doc-changed` payload) to the cache. */
  applyResult(result: ApplyResult): void {
    if (this.disposed) return;
    const dirty: RangeDto[] = [];
    for (const sr of result.changed) {
      if (sr.sheet !== this.sheet) continue;
      for (const range of sr.ranges) {
        dirty.push(range);
        for (const tile of this.tiles.values()) {
          if (intersects(range, this.tileRange(tile))) {
            tile.stale = true;
            tile.seq++;
          }
        }
      }
    }
    if (result.layoutChanged.includes(this.sheet) || dirty.length > 0) this.queueLayout();
    if (dirty.length > 0) this.emit(dirty);
  }

  /** Refetches the sheet layout; notifies `"all"` if it changed. */
  async refreshLayout(): Promise<void> {
    const seq = ++this.layoutSeq;
    const sheet = this.sheet;
    try {
      const layout = await this.client.sheetLayout(this.doc, sheet);
      if (this.disposed || seq !== this.layoutSeq || sheet !== this.sheet) return;
      if (JSON.stringify(layout) === JSON.stringify(this.layout)) return;
      this.layout = layout;
      this.emit("all");
    } catch (e) {
      this.onError(e);
    }
  }

  // -- Internals --------------------------------------------------------------

  private tileRange(t: { tr: number; tc: number }): RangeDto {
    const r0 = t.tr * TILE_ROWS;
    const c0 = t.tc * TILE_COLS;
    return { r0, c0, r1: Math.min(MAX_ROWS - 1, r0 + TILE_ROWS - 1), c1: Math.min(MAX_COLS - 1, c0 + TILE_COLS - 1) };
  }

  private fetch(batch: Tile[]): void {
    const sheet = this.sheet;
    const seqs = batch.map((t) => t.seq);
    for (const t of batch) t.inflight = t.seq;
    this.fetches++;
    this.client.cells(this.doc, sheet, batch.map((t) => this.tileRange(t))).then(
      (blocks) => {
        if (this.disposed || sheet !== this.sheet) return;
        const dirty: RangeDto[] = [];
        batch.forEach((tile, i) => {
          const block = blocks[i];
          if (tile.inflight === seqs[i]) tile.inflight = -1;
          if (!block || tile.seq !== seqs[i]) return; // changed while in flight: refetch later
          const cells = new Map<number, CellView>();
          for (const cell of block.cells) {
            cells.set((cell.r - tile.tr * TILE_ROWS) * TILE_COLS + (cell.c - tile.tc * TILE_COLS), cell);
            if (!this.styles.has(cell.style)) this.queueStyle(cell.style);
            // Synthesized conditional-format styles (≥ 0x40000000) resolve like any other id.
            if (cell.cfStyle != null && !this.styles.has(cell.cfStyle)) this.queueStyle(cell.cfStyle);
          }
          tile.cells = cells;
          tile.ready = true;
          tile.stale = false;
          dirty.push(block.range);
        });
        if (dirty.length > 0) this.emit(dirty);
      },
      (e: unknown) => {
        batch.forEach((tile, i) => {
          if (tile.inflight === seqs[i]) tile.inflight = -1;
        });
        this.onError(e);
      },
    );
  }

  private evict(): void {
    if (this.tiles.size <= this.capacity) return;
    const evicted: RangeDto[] = [];
    for (const [id, tile] of this.tiles) {
      if (this.tiles.size <= this.capacity) break;
      if (tile.inflight !== -1) continue;
      this.tiles.delete(id);
      evicted.push(this.tileRange(tile));
    }
    // Let the grid forget them, so it asks again if they come back into view.
    if (evicted.length > 0) this.emit(evicted);
  }

  private queueStyle(id: number): void {
    if (this.styles.has(id) || this.requestedStyles.has(id)) return;
    this.pendingStyles.add(id);
    this.requestedStyles.add(id);
    if (this.styleQueued) return;
    this.styleQueued = true;
    queueMicrotask(() => {
      this.styleQueued = false;
      const ids = [...this.pendingStyles];
      this.pendingStyles.clear();
      if (ids.length === 0 || this.disposed) return;
      this.client.styles(this.doc, ids).then(
        (list) => {
          if (this.disposed) return;
          for (const s of list) this.styles.set(s.id, s);
          this.emit("all");
        },
        (e: unknown) => {
          for (const id of ids) this.requestedStyles.delete(id);
          this.onError(e);
        },
      );
    });
  }

  private queueLayout(): void {
    if (this.layoutQueued) return;
    this.layoutQueued = true;
    queueMicrotask(() => {
      this.layoutQueued = false;
      void this.refreshLayout();
    });
  }

  private emit(d: DirtySet): void {
    for (const fn of this.listeners) fn(d);
  }
}
