/**
 * Screen geometry of the grid: where cells and headers are on the canvas and
 * what is under a point. Pure apart from the axis models; coordinates are
 * CSS pixels relative to the canvas' top-left corner.
 */
import type { CellPos, RangeDto } from "@tally/bridge";
import type { AxisModel } from "./layout";

/** Default column-header height at zoom 100%. */
export const HEADER_HEIGHT = 24;
/** Minimum row-header width at zoom 100%. */
export const MIN_ROW_HEADER_WIDTH = 44;
/** Half-width of the column resize hit zone around a header boundary. */
export const RESIZE_GUTTER = 4;

export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export type Hit =
  | { area: "corner" }
  | { area: "colHeader"; c: number }
  | { area: "rowHeader"; r: number }
  | { area: "colGutter"; c: number }
  | { area: "rowGutter"; r: number }
  | { area: "cell"; r: number; c: number };

/** Frozen rows/columns at the top/left (0 = none). */
export interface Frozen {
  rows: number;
  cols: number;
}

export const NO_FROZEN: Frozen = { rows: 0, cols: 0 };

/**
 * One scroll pane: the visible index spans and the screen rect it owns.
 * Without frozen panes there is one; with frozen rows and columns, four
 * (top-left frozen both ways, top, left, and the scrolling body).
 */
export interface Pane {
  r0: number;
  r1: number;
  c0: number;
  c1: number;
  /** Smallest/largest column whose pixels can appear in this pane. */
  cMin: number;
  cMax: number;
  /** Smallest/largest row whose pixels can appear in this pane. */
  rMin: number;
  rMax: number;
  clip: Rect;
  frozenRows: boolean;
  frozenCols: boolean;
}

export class Geometry {
  /** Frozen pane extents (screen px). */
  readonly frozenW: number;
  readonly frozenH: number;
  readonly frozen: Frozen;

  constructor(
    readonly rows: AxisModel,
    readonly cols: AxisModel,
    /** Zoom factor (1 = 100%). */
    readonly zoom: number,
    /** Row-header width (screen px). */
    readonly headerW: number,
    /** Column-header height (screen px). */
    readonly headerH: number,
    /** Content offset of the cell area (screen px, i.e. already zoomed). */
    readonly scrollX: number,
    readonly scrollY: number,
    /** Canvas size (CSS px). */
    readonly width: number,
    readonly height: number,
    frozen: Frozen = NO_FROZEN,
    /** Merged areas of the sheet. */
    readonly merges: readonly RangeDto[] = [],
    /** Outline gutter left of the row header (included in `headerW`), screen px. */
    readonly gutterW: number = 0,
    /** Outline gutter above the column header (included in `headerH`), screen px. */
    readonly gutterH: number = 0,
  ) {
    const fr = Math.max(0, Math.min(rows.count - 1, Math.floor(frozen.rows)));
    const fc = Math.max(0, Math.min(cols.count - 1, Math.floor(frozen.cols)));
    this.frozen = { rows: fr, cols: fc };
    this.frozenW = cols.indexToPx(fc) * zoom;
    this.frozenH = rows.indexToPx(fr) * zoom;
  }

  /** Width/height of the cell area. */
  get cellsW(): number {
    return Math.max(0, this.width - this.headerW);
  }

  get cellsH(): number {
    return Math.max(0, this.height - this.headerH);
  }

  /** Screen x of the left edge of column `c` (may be off-canvas). Frozen columns do not scroll. */
  colX(c: number): number {
    const x = this.headerW + this.cols.indexToPx(c) * this.zoom;
    return c < this.frozen.cols ? x : x - this.scrollX;
  }

  /** Screen y of the top edge of row `r` (may be off-canvas). Frozen rows do not scroll. */
  rowY(r: number): number {
    const y = this.headerH + this.rows.indexToPx(r) * this.zoom;
    return r < this.frozen.rows ? y : y - this.scrollY;
  }

  /** Screen x of the right edge of column `c` (use instead of `colX(c + 1)` across a frozen edge). */
  colRight(c: number): number {
    return this.colX(c) + this.colW(c);
  }

  /** Screen y of the bottom edge of row `r`. */
  rowBottom(r: number): number {
    return this.rowY(r) + this.rowH(r);
  }

  colW(c: number): number {
    return this.cols.sizeOf(c) * this.zoom;
  }

  rowH(r: number): number {
    return this.rows.sizeOf(r) * this.zoom;
  }

  cellRect(r: number, c: number): Rect {
    return { x: this.colX(c), y: this.rowY(r), w: this.colW(c), h: this.rowH(r) };
  }

  /** The merged area containing (r, c), if any. */
  mergeAt(r: number, c: number): RangeDto | undefined {
    for (const m of this.merges) if (r >= m.r0 && r <= m.r1 && c >= m.c0 && c <= m.c1) return m;
    return undefined;
  }

  /** The cell's rect, or its whole merged area's. */
  cellBox(r: number, c: number): Rect {
    const m = this.mergeAt(r, c);
    return m ? this.rangeRect(m, false) : this.cellRect(r, c);
  }

  /**
   * Bounding rect of a range, clamped to a margin around the canvas. Parts
   * of the scrolling area that are scrolled under a frozen pane are cut off
   * (a range entirely under it has zero size).
   */
  rangeRect(range: RangeDto, clampToCanvas = true): Rect {
    const m = 16;
    const fx = this.headerW + this.frozenW;
    const fy = this.headerH + this.frozenH;
    const fc = this.frozen.cols;
    const fr = this.frozen.rows;
    let x0 = this.colX(range.c0);
    let x1 = this.colRight(range.c1);
    let y0 = this.rowY(range.r0);
    let y1 = this.rowBottom(range.r1);
    if (fc > 0) {
      if (range.c0 >= fc) x0 = Math.max(x0, fx);
      if (range.c1 >= fc) x1 = Math.max(x1, fx);
    }
    if (fr > 0) {
      if (range.r0 >= fr) y0 = Math.max(y0, fy);
      if (range.r1 >= fr) y1 = Math.max(y1, fy);
    }
    if (clampToCanvas) {
      x0 = clampN(x0, -m, this.width + m);
      x1 = clampN(x1, -m, this.width + m);
      y0 = clampN(y0, -m, this.height + m);
      y1 = clampN(y1, -m, this.height + m);
    }
    return { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
  }

  /** Column under screen x (in the cell area). */
  colAt(x: number): number {
    const fc = this.frozen.cols;
    if (fc > 0 && x < this.headerW + this.frozenW) {
      return Math.min(fc - 1, this.cols.pxToIndex((x - this.headerW) / this.zoom));
    }
    const c = this.cols.pxToIndex((x - this.headerW + this.scrollX) / this.zoom);
    return fc > 0 ? Math.max(fc, c) : c;
  }

  /** Row under screen y (in the cell area). */
  rowAt(y: number): number {
    const fr = this.frozen.rows;
    if (fr > 0 && y < this.headerH + this.frozenH) {
      return Math.min(fr - 1, this.rows.pxToIndex((y - this.headerH) / this.zoom));
    }
    const r = this.rows.pxToIndex((y - this.headerH + this.scrollY) / this.zoom);
    return fr > 0 ? Math.max(fr, r) : r;
  }

  /** Visible cells of the scrolling pane (inclusive), including partially visible ones. */
  visibleRange(): RangeDto {
    const r0 = this.rowAt(Math.min(this.height - 1, this.headerH + this.frozenH));
    const c0 = this.colAt(Math.min(this.width - 1, this.headerW + this.frozenW));
    const r1 = this.rowAt(Math.max(this.headerH, this.height - 1));
    const c1 = this.colAt(Math.max(this.headerW, this.width - 1));
    return { r0, c0, r1: Math.max(r0, r1), c1: Math.max(c0, c1) };
  }

  /** Visible row spans: the frozen rows (if any), then the scrolling ones. */
  rowSpans(): { r0: number; r1: number; y0: number; y1: number; frozen: boolean; min: number; max: number }[] {
    const out = [];
    const fr = this.frozen.rows;
    const fy = Math.min(this.height, this.headerH + this.frozenH);
    if (fr > 0 && this.frozenH > 0) {
      out.push({ r0: 0, r1: this.rowAt(Math.max(this.headerH, fy - 1)), y0: this.headerH, y1: fy, frozen: true, min: 0, max: fr - 1 });
    }
    if (fy < this.height) {
      const v = this.visibleRange();
      out.push({ r0: v.r0, r1: v.r1, y0: fy, y1: this.height, frozen: false, min: fr, max: this.rows.count - 1 });
    }
    return out;
  }

  /** Visible column spans: the frozen columns (if any), then the scrolling ones. */
  colSpans(): { c0: number; c1: number; x0: number; x1: number; frozen: boolean; min: number; max: number }[] {
    const out = [];
    const fc = this.frozen.cols;
    const fx = Math.min(this.width, this.headerW + this.frozenW);
    if (fc > 0 && this.frozenW > 0) {
      out.push({ c0: 0, c1: this.colAt(Math.max(this.headerW, fx - 1)), x0: this.headerW, x1: fx, frozen: true, min: 0, max: fc - 1 });
    }
    if (fx < this.width) {
      const v = this.visibleRange();
      out.push({ c0: v.c0, c1: v.c1, x0: fx, x1: this.width, frozen: false, min: fc, max: this.cols.count - 1 });
    }
    return out;
  }

  /** The panes to draw, frozen ones first. */
  panes(): Pane[] {
    const out: Pane[] = [];
    for (const rs of this.rowSpans()) {
      for (const cs of this.colSpans()) {
        out.push({
          r0: rs.r0,
          r1: rs.r1,
          c0: cs.c0,
          c1: cs.c1,
          rMin: rs.min,
          rMax: rs.max,
          cMin: cs.min,
          cMax: cs.max,
          clip: { x: cs.x0, y: rs.y0, w: cs.x1 - cs.x0, h: rs.y1 - rs.y0 },
          frozenRows: rs.frozen,
          frozenCols: cs.frozen,
        });
      }
    }
    return out;
  }

  /** Number of fully visible rows (for PageUp/PageDown). */
  pageRows(): number {
    const defaultH = this.rows.defaultSize * this.zoom;
    return Math.max(1, Math.floor((this.cellsH - this.frozenH) / Math.max(1, defaultH)) - 1);
  }

  pageCols(): number {
    const defaultW = this.cols.defaultSize * this.zoom;
    return Math.max(1, Math.floor((this.cellsW - this.frozenW) / Math.max(1, defaultW)));
  }

  hit(x: number, y: number): Hit {
    const inColHeader = y < this.headerH;
    const inRowHeader = x < this.headerW;
    if (inColHeader && inRowHeader) return { area: "corner" };
    if (inColHeader) {
      const c = this.colAt(x);
      const left = this.colX(c);
      const right = left + this.colW(c);
      if (right - x <= RESIZE_GUTTER) return { area: "colGutter", c };
      if (x - left <= RESIZE_GUTTER) {
        const prev = this.cols.nextVisible(c, -1);
        if (prev >= 0) return { area: "colGutter", c: prev };
      }
      return { area: "colHeader", c };
    }
    if (inRowHeader) {
      const r = this.rowAt(y);
      const top = this.rowY(r);
      const bottom = top + this.rowH(r);
      // Row gutters are thinner than column gutters (rows are short).
      const g = Math.min(RESIZE_GUTTER, 3);
      if (bottom - y <= g) return { area: "rowGutter", r };
      if (y - top <= g) {
        const prev = this.rows.nextVisible(r, -1);
        if (prev >= 0) return { area: "rowGutter", r: prev };
      }
      return { area: "rowHeader", r };
    }
    return { area: "cell", r: this.rowAt(y), c: this.colAt(x) };
  }

  /** Scroll offsets that bring `cell` fully into view (or `null` if it already is). */
  scrollToReveal(cell: CellPos): { x: number; y: number } | null {
    let { scrollX: sx, scrollY: sy } = this;
    if (cell.c >= this.frozen.cols) {
      const cx = this.cols.indexToPx(cell.c) * this.zoom;
      const cw = this.colW(cell.c);
      if (cx - sx < this.frozenW) sx = cx - this.frozenW;
      else if (cx + cw - sx > this.cellsW) sx = Math.min(cx - this.frozenW, cx + cw - this.cellsW);
    }
    if (cell.r >= this.frozen.rows) {
      const cy = this.rows.indexToPx(cell.r) * this.zoom;
      const ch = this.rowH(cell.r);
      if (cy - sy < this.frozenH) sy = cy - this.frozenH;
      else if (cy + ch - sy > this.cellsH) sy = Math.min(cy - this.frozenH, cy + ch - this.cellsH);
    }
    sx = Math.max(0, sx);
    sy = Math.max(0, sy);
    if (sx === this.scrollX && sy === this.scrollY) return null;
    return { x: sx, y: sy };
  }
}

function clampN(v: number, lo: number, hi: number): number {
  return v < lo ? lo : v > hi ? hi : v;
}

/**
 * Snaps a coordinate for a crisp line of `widthCss` CSS px at `dpr`: odd
 * device-pixel widths are centred on a half device pixel, even widths on a
 * whole one. Use with `stroke()`.
 */
export function snapStroke(v: number, widthCss: number, dpr: number): number {
  const w = Math.max(1, Math.round(widthCss * dpr));
  const dev = Math.round(v * dpr);
  return (w % 2 === 1 ? dev + 0.5 : dev) / dpr;
}

/** Snaps a coordinate to the device-pixel grid (for `fillRect`). */
export function snapFill(v: number, dpr: number): number {
  return Math.round(v * dpr) / dpr;
}
