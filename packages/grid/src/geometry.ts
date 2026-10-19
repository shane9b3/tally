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

export class Geometry {
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
  ) {}

  /** Width/height of the cell area. */
  get cellsW(): number {
    return Math.max(0, this.width - this.headerW);
  }

  get cellsH(): number {
    return Math.max(0, this.height - this.headerH);
  }

  /** Screen x of the left edge of column `c` (may be off-canvas). */
  colX(c: number): number {
    return this.headerW + this.cols.indexToPx(c) * this.zoom - this.scrollX;
  }

  /** Screen y of the top edge of row `r` (may be off-canvas). */
  rowY(r: number): number {
    return this.headerH + this.rows.indexToPx(r) * this.zoom - this.scrollY;
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

  /** Bounding rect of a range, clamped to a margin around the canvas. */
  rangeRect(range: RangeDto): Rect {
    const m = 16;
    const x0 = clampN(this.colX(range.c0), -m, this.width + m);
    const x1 = clampN(this.colX(range.c1 + 1), -m, this.width + m);
    const y0 = clampN(this.rowY(range.r0), -m, this.height + m);
    const y1 = clampN(this.rowY(range.r1 + 1), -m, this.height + m);
    return { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
  }

  /** Column under screen x (in the cell area). */
  colAt(x: number): number {
    return this.cols.pxToIndex((x - this.headerW + this.scrollX) / this.zoom);
  }

  /** Row under screen y (in the cell area). */
  rowAt(y: number): number {
    return this.rows.pxToIndex((y - this.headerH + this.scrollY) / this.zoom);
  }

  /** Visible cells (inclusive), including partially visible ones. */
  visibleRange(): RangeDto {
    const r0 = this.rowAt(this.headerH);
    const c0 = this.colAt(this.headerW);
    const r1 = this.rowAt(Math.max(this.headerH, this.height - 1));
    const c1 = this.colAt(Math.max(this.headerW, this.width - 1));
    return { r0, c0, r1, c1 };
  }

  /** Number of fully visible rows (for PageUp/PageDown). */
  pageRows(): number {
    const defaultH = this.rows.defaultSize * this.zoom;
    return Math.max(1, Math.floor(this.cellsH / Math.max(1, defaultH)) - 1);
  }

  pageCols(): number {
    const defaultW = this.cols.defaultSize * this.zoom;
    return Math.max(1, Math.floor(this.cellsW / Math.max(1, defaultW)));
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
    const cx = this.cols.indexToPx(cell.c) * this.zoom;
    const cw = this.colW(cell.c);
    const cy = this.rows.indexToPx(cell.r) * this.zoom;
    const ch = this.rowH(cell.r);
    if (cx < sx) sx = cx;
    else if (cx + cw > sx + this.cellsW) sx = Math.min(cx, cx + cw - this.cellsW);
    if (cy < sy) sy = cy;
    else if (cy + ch > sy + this.cellsH) sy = Math.min(cy, cy + ch - this.cellsH);
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
