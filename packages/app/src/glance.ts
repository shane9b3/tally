/**
 * "At a glance": a quick summary of the data region around the active
 * cell, computed client-side from fetched cells (pure; the GlancePanel
 * does the fetching).
 *
 * - Region: the "current region" (the block of non-empty cells connected to
 *   the active cell, like Ctrl+*), else the used range.
 * - Header: the first row when all its values are text.
 * - KPIs: body row count; the Sum of each numeric (non-date) column,
 *   formatted with that column's number format; the counts per value of the
 *   lowest-cardinality text column (2–12 repeating values).
 * - Chart: a value column (one named like Total/Amount, else the first
 *   numeric one) summed per category value, top 8.
 */
import type { CellPos, CellView, RangeDto } from "@tally/bridge";
import { colToLetters } from "@tally/grid";
import { formatNumberLite, isDateCode } from "./numfmtLite";

/** Cells fetched at most (the panel says "Sampled" beyond). */
export const GLANCE_CELL_CAP = 20_000;
/** Bars in the chart. */
export const GLANCE_TOP = 8;

export type Lookup = (r: number, c: number) => CellView | undefined;

function filled(v: CellView | undefined): v is CellView {
  return v !== undefined && v.kind !== "empty" && v.text !== "";
}

/**
 * The block of non-empty cells around `active` (grown while any cell on the
 * border just outside it is non-empty), within `bounds`. `null` if the
 * active cell and its neighbours are empty.
 */
export function currentRegion(get: Lookup, active: CellPos, bounds: RangeDto): RangeDto | null {
  let r: RangeDto = { r0: active.r, c0: active.c, r1: active.r, c1: active.c };
  const any = (r0: number, c0: number, r1: number, c1: number): boolean => {
    for (let i = Math.max(r0, bounds.r0); i <= Math.min(r1, bounds.r1); i++) {
      for (let j = Math.max(c0, bounds.c0); j <= Math.min(c1, bounds.c1); j++) if (filled(get(i, j))) return true;
    }
    return false;
  };
  for (let guard = 0; guard < 10_000; guard++) {
    let grew = false;
    if (r.r0 > bounds.r0 && any(r.r0 - 1, r.c0 - 1, r.r0 - 1, r.c1 + 1)) {
      r = { ...r, r0: r.r0 - 1 };
      grew = true;
    }
    if (r.r1 < bounds.r1 && any(r.r1 + 1, r.c0 - 1, r.r1 + 1, r.c1 + 1)) {
      r = { ...r, r1: r.r1 + 1 };
      grew = true;
    }
    if (r.c0 > bounds.c0 && any(r.r0 - 1, r.c0 - 1, r.r1 + 1, r.c0 - 1)) {
      r = { ...r, c0: r.c0 - 1 };
      grew = true;
    }
    if (r.c1 < bounds.c1 && any(r.r0 - 1, r.c1 + 1, r.r1 + 1, r.c1 + 1)) {
      r = { ...r, c1: r.c1 + 1 };
      grew = true;
    }
    if (!grew) break;
  }
  if (r.r0 === r.r1 && r.c0 === r.c1 && !filled(get(r.r0, r.c0))) return null;
  return r;
}

export interface GlanceSum {
  c: number;
  header: string;
  sum: number;
  text: string;
}

export interface GlanceCount {
  value: string;
  count: number;
}

export interface GlanceBar {
  label: string;
  value: number;
  text: string;
}

export interface GlanceResult {
  region: RangeDto;
  /** Body rows (excluding the header). */
  rows: number;
  hasHeader: boolean;
  sums: GlanceSum[];
  category: { c: number; header: string; counts: GlanceCount[] } | null;
  chart: { valueHeader: string; categoryHeader: string; bars: GlanceBar[] } | null;
}

function fmt(v: number, code: string): string {
  return formatNumberLite(v, code) ?? String(Number(v.toPrecision(12)));
}

/**
 * Analyses `region`. `formatOf(c)` is the number format of column `c`'s
 * body (e.g. of its first numeric cell).
 */
export function analyzeGlance(get: Lookup, region: RangeDto, formatOf: (c: number) => string): GlanceResult {
  const headerCells: (CellView | undefined)[] = [];
  for (let c = region.c0; c <= region.c1; c++) headerCells.push(get(region.r0, c));
  const headerVals = headerCells.filter(filled);
  const hasHeader = region.r1 > region.r0 && headerVals.length > 0 && headerVals.every((v) => v.kind === "text");
  const body0 = hasHeader ? region.r0 + 1 : region.r0;
  const rows = region.r1 - body0 + 1;
  const headerOf = (c: number): string => {
    const h = hasHeader ? get(region.r0, c) : undefined;
    return h && filled(h) ? h.text : `Column ${colToLetters(c)}`;
  };

  interface Col {
    c: number;
    nums: number;
    texts: number;
    sum: number;
    values: Map<string, number>;
    order: string[];
  }
  const cols: Col[] = [];
  for (let c = region.c0; c <= region.c1; c++) {
    const col: Col = { c, nums: 0, texts: 0, sum: 0, values: new Map(), order: [] };
    for (let r = body0; r <= region.r1; r++) {
      const v = get(r, c);
      if (!filled(v)) continue;
      if (v.kind === "number" && v.num !== null) {
        col.nums++;
        col.sum += v.num;
      } else if (v.kind === "text") {
        col.texts++;
        const key = v.text.trim();
        if (!col.values.has(key)) col.order.push(key);
        col.values.set(key, (col.values.get(key) ?? 0) + 1);
      }
    }
    cols.push(col);
  }

  const numeric = cols.filter((col) => col.nums > 0 && col.nums >= 0.6 * (col.nums + col.texts) && !isDateCode(formatOf(col.c)));
  const sums: GlanceSum[] = numeric.map((col) => ({ c: col.c, header: headerOf(col.c), sum: col.sum, text: fmt(col.sum, formatOf(col.c)) }));

  // The category: a mostly-text column with 2–12 distinct values that repeat;
  // the one with the fewest distinct values per row wins (first on ties).
  let cat: Col | undefined;
  let best = Infinity;
  for (const col of cols) {
    const ratio = col.values.size / Math.max(1, col.texts);
    if (col.texts < 0.6 * Math.max(1, col.nums + col.texts) || col.values.size < 2 || col.values.size > 12 || ratio > 0.6) continue;
    if (ratio < best) {
      best = ratio;
      cat = col;
    }
  }
  const category = cat
    ? {
        c: cat.c,
        header: headerOf(cat.c),
        counts: [...cat.values.entries()].map(([value, count]) => ({ value, count })).sort((a, b) => b.count - a.count),
      }
    : null;

  let chart: GlanceResult["chart"] = null;
  if (cat && numeric.length > 0) {
    const valueCol = numeric.find((col) => /^(total|amount|sum|revenue|sales|value|actual)\b/i.test(headerOf(col.c))) ?? numeric[0]!;
    const by = new Map<string, number>();
    for (let r = body0; r <= region.r1; r++) {
      const k = get(r, cat.c);
      const v = get(r, valueCol.c);
      if (!filled(k) || !v || v.kind !== "number" || v.num === null) continue;
      by.set(k.text.trim(), (by.get(k.text.trim()) ?? 0) + v.num);
    }
    const code = formatOf(valueCol.c);
    const bars = [...by.entries()]
      .map(([label, value]) => ({ label, value, text: fmt(value, code) }))
      .sort((a, b) => b.value - a.value)
      .slice(0, GLANCE_TOP);
    if (bars.length > 0) chart = { valueHeader: headerOf(valueCol.c), categoryHeader: headerOf(cat.c), bars };
  }

  return { region, rows, hasHeader, sums, category, chart };
}

/** The rows to fetch for a region so at most `cap` cells are read. */
export function cappedRange(used: RangeDto, cap: number = GLANCE_CELL_CAP): { range: RangeDto; sampled: boolean } {
  const width = used.c1 - used.c0 + 1;
  const maxRows = Math.max(1, Math.floor(cap / Math.max(1, width)));
  if (used.r1 - used.r0 + 1 <= maxRows) return { range: used, sampled: false };
  return { range: { ...used, r1: used.r0 + maxRows - 1 }, sampled: true };
}
