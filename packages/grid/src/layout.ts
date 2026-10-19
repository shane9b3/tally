/**
 * Axis geometry: index ↔ pixel mapping for 1,048,576 rows / 16,384 columns
 * without per-index arrays, plus the scaled-scrolling rule.
 *
 * An axis is stored as the sparse runs from `SheetLayout.colRuns/rowRuns`
 * (sizes that differ from the default). The runs and the default-sized gaps
 * between them form "segments" of constant size; each segment keeps the pixel
 * offset of its first index (a prefix sum over segments), so every query is a
 * binary search over segments: O(log runs). Hidden indices have size 0.
 *
 * All pixel values are CSS pixels at zoom 100%; the renderer multiplies by
 * the zoom factor.
 */
import type { DimRunPx, SheetLayout } from "@tally/bridge";
import { MAX_COLS, MAX_ROWS } from "./types";

/** A run in normalized form (hidden runs have size 0). */
interface Run {
  start: number;
  end: number;
  size: number;
  hidden: boolean;
}

export class AxisModel {
  /** Number of indices on this axis. */
  readonly count: number;
  /** Size of indices without a run. */
  readonly defaultSize: number;
  private runs: Run[] = [];
  private segStart: number[] = [];
  private segEnd: number[] = [];
  private segSize: number[] = [];
  private segPx: number[] = [];
  private totalPx = 0;

  constructor(count: number, defaultSize: number, runs: readonly DimRunPx[] = []) {
    this.count = Math.max(1, Math.floor(count));
    this.defaultSize = Math.max(0, defaultSize);
    this.runs = normalizeRuns(runs, this.count);
    this.rebuild();
  }

  /** Builds the row and column axes of a sheet layout. */
  static fromLayout(layout: SheetLayout, rows = MAX_ROWS, cols = MAX_COLS): { rows: AxisModel; cols: AxisModel } {
    return {
      rows: new AxisModel(rows, layout.defaultRowHeightPx, layout.rowRuns),
      cols: new AxisModel(cols, layout.defaultColWidthPx, layout.colRuns),
    };
  }

  /** Total size of the axis in pixels. */
  total(): number {
    return this.totalPx;
  }

  /** Size of index `i` in pixels (0 if hidden). */
  sizeOf(i: number): number {
    const k = this.segmentOfIndex(i);
    return this.segSize[k] ?? this.defaultSize;
  }

  /** Pixel offset of the start of index `i`; `indexToPx(count)` is the total. */
  indexToPx(i: number): number {
    if (i <= 0) return 0;
    if (i >= this.count) return this.totalPx;
    const k = this.segmentOfIndex(i);
    return (this.segPx[k] ?? 0) + (i - (this.segStart[k] ?? 0)) * (this.segSize[k] ?? 0);
  }

  /**
   * The index whose pixel span contains `px` (clamped to the axis). Hidden
   * indices are never returned unless every index is hidden.
   */
  pxToIndex(px: number): number {
    const n = this.segPx.length;
    if (px >= this.totalPx) return this.lastVisibleFrom(n - 1);
    if (px < 0) px = 0;
    // Last segment whose start offset is <= px. Zero-sized segments share
    // their offset with the next segment, so this lands on a sized one.
    let lo = 0;
    let hi = n - 1;
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      if ((this.segPx[mid] ?? 0) <= px) lo = mid;
      else hi = mid - 1;
    }
    const size = this.segSize[lo] ?? 0;
    if (size <= 0) return this.lastVisibleFrom(lo);
    const start = this.segStart[lo] ?? 0;
    const end = this.segEnd[lo] ?? start;
    return Math.min(end, start + Math.floor((px - (this.segPx[lo] ?? 0)) / size));
  }

  /** Whether index `i` is hidden (size 0). */
  isHidden(i: number): boolean {
    return this.sizeOf(i) <= 0;
  }

  /**
   * Sets indices `start..=end` to `px` (or hides them). Rebuilds the
   * segments: O(runs).
   */
  setSize(start: number, end: number, px: number, hidden = false): void {
    start = Math.max(0, Math.floor(start));
    end = Math.min(this.count - 1, Math.floor(end));
    if (end < start) return;
    const out: Run[] = [];
    for (const r of this.runs) {
      if (r.end < start || r.start > end) {
        out.push(r);
        continue;
      }
      if (r.start < start) out.push({ ...r, end: start - 1 });
      if (r.end > end) out.push({ ...r, start: end + 1 });
    }
    const size = hidden ? 0 : Math.max(0, px);
    if (hidden || size !== this.defaultSize) out.push({ start, end, size, hidden });
    out.sort((a, b) => a.start - b.start);
    this.runs = mergeRuns(out);
    this.rebuild();
  }

  /** The runs in `DimRunPx` form (sizes of hidden runs are reported as 0). */
  getRuns(): DimRunPx[] {
    return this.runs.map((r) => ({ start: r.start, end: r.end, size: r.size, hidden: r.hidden }));
  }

  /** Next index after `i` that is not hidden, or -1. */
  nextVisible(i: number, step: 1 | -1): number {
    let j = i + step;
    while (j >= 0 && j < this.count) {
      const k = this.segmentOfIndex(j);
      if ((this.segSize[k] ?? 0) > 0) return j;
      // Skip the whole hidden segment.
      j = step > 0 ? (this.segEnd[k] ?? j) + 1 : (this.segStart[k] ?? j) - 1;
    }
    return -1;
  }

  private lastVisibleFrom(seg: number): number {
    for (let k = seg; k >= 0; k--) if ((this.segSize[k] ?? 0) > 0) return this.segEnd[k] ?? 0;
    return 0;
  }

  private segmentOfIndex(i: number): number {
    let lo = 0;
    let hi = this.segStart.length - 1;
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      if ((this.segStart[mid] ?? 0) <= i) lo = mid;
      else hi = mid - 1;
    }
    return lo;
  }

  private rebuild(): void {
    const start: number[] = [];
    const end: number[] = [];
    const size: number[] = [];
    const pxs: number[] = [];
    let pos = 0;
    let px = 0;
    const push = (s: number, e: number, sz: number): void => {
      start.push(s);
      end.push(e);
      size.push(sz);
      pxs.push(px);
      px += (e - s + 1) * sz;
    };
    for (const r of this.runs) {
      if (r.start > pos) push(pos, r.start - 1, this.defaultSize);
      push(r.start, r.end, r.size);
      pos = r.end + 1;
    }
    if (pos < this.count) push(pos, this.count - 1, this.defaultSize);
    this.segStart = start;
    this.segEnd = end;
    this.segSize = size;
    this.segPx = pxs;
    this.totalPx = px;
  }
}

function normalizeRuns(runs: readonly DimRunPx[], count: number): Run[] {
  const out: Run[] = [];
  for (const r of [...runs].sort((a, b) => a.start - b.start)) {
    const start = Math.max(0, Math.floor(r.start));
    const end = Math.min(count - 1, Math.floor(r.end));
    const prev = out[out.length - 1];
    const s = prev ? Math.max(start, prev.end + 1) : start;
    if (end < s) continue;
    out.push({ start: s, end, size: r.hidden ? 0 : Math.max(0, r.size), hidden: r.hidden });
  }
  return mergeRuns(out);
}

function mergeRuns(runs: Run[]): Run[] {
  const out: Run[] = [];
  for (const r of runs) {
    const prev = out[out.length - 1];
    if (prev && prev.end + 1 === r.start && prev.size === r.size && prev.hidden === r.hidden) prev.end = r.end;
    else out.push({ ...r });
  }
  return out;
}

// ---------------------------------------------------------------------------
// Scaled scrolling
// ---------------------------------------------------------------------------

/**
 * Largest spacer we give the native scroll container. Browsers cap element
 * sizes (WebKit/Firefox ≈ 16.7M–17.9M px; Chromium ≈ 33.5M), and the default
 * 1,048,576 rows × 24 px is 25.2M px, so beyond this we scale.
 */
export const MAX_SCROLL_EXTENT = 10_000_000;

/**
 * Maps between the native scroll position and the content offset on one axis.
 *
 * Rule: let `content` be the content size and `view` the viewport size (both
 * in screen px). If `content <= MAX_SCROLL_EXTENT` the spacer is `content`
 * and the mapping is the identity. Otherwise the spacer is
 * `MAX_SCROLL_EXTENT` and
 *
 *     contentOffset = scrollPos × (content − view) / (MAX_SCROLL_EXTENT − view)
 *
 * so scrollPos 0 shows the first pixel and the maximum scrollPos shows the
 * last one; the scroll thumb position is proportional. The grid keeps the
 * exact content offset itself (keyboard moves and wheel deltas are applied
 * in content px, then the scroll position is derived), so the rounding of
 * `scrollTop` to device pixels never loses precision.
 */
export class ScrollAxisMap {
  readonly content: number;
  readonly view: number;
  /** Size to give the spacer element. */
  readonly extent: number;
  /** Content px per scroll px (1 when not scaled). */
  readonly ratio: number;

  constructor(content: number, view: number, maxExtent = MAX_SCROLL_EXTENT) {
    this.content = Math.max(0, content);
    this.view = Math.max(0, view);
    if (this.content <= maxExtent || this.view >= maxExtent) {
      this.extent = this.content;
      this.ratio = 1;
    } else {
      this.extent = maxExtent;
      const maxScroll = maxExtent - this.view;
      const maxContent = Math.max(0, this.content - this.view);
      this.ratio = maxScroll > 0 ? maxContent / maxScroll : 1;
    }
  }

  get scaled(): boolean {
    return this.ratio !== 1;
  }

  /** Largest valid content offset. */
  get maxContent(): number {
    return Math.max(0, this.content - this.view);
  }

  /** Native scroll position → content offset (clamped). */
  toContent(scrollPos: number): number {
    return clamp(scrollPos * this.ratio, 0, this.maxContent);
  }

  /** Content offset → native scroll position. */
  toScroll(contentPos: number): number {
    return clamp(contentPos, 0, this.maxContent) / this.ratio;
  }
}

export function clamp(v: number, lo: number, hi: number): number {
  return v < lo ? lo : v > hi ? hi : v;
}
