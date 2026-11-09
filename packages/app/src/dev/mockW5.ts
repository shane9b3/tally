/**
 * The W5-5 side model of the browser mock (`WebMockClient`): charts and
 * pictures per sheet (the `addChart` … `moveResizeImage` ops), `chart_data`
 * computed from the mock's cells (series derived from the wizard source
 * the way the engine does, trend lines, error bars, means),
 * `suggest_chart`, a print layout paginated with the v0.3 page-break
 * estimate, page PNGs drawn on a canvas, and detective traces parsed from
 * formulas. Pure functions over plain JSON (snapshotted for undo/redo by
 * the mock's `DocExt` mechanism); nothing here ships in the desktop bundle.
 */
import type { CellView, ChartDataDto, ChartDataTableDto, ChartDto, ChartKind, ChartSeriesDto, ChartSvgDto, ImageDto, Op, PageBreaksDto, PageSetupDto, PrintLayoutDto, PrintOptionsDto, PrintPageDto, RangeDto, SeriesDataDto, SheetLayout, SplitDto, TraceDto, TrendDataDto } from "@tally/bridge";
import { absoluteRef, parseAbsoluteRef } from "../dataOps";
import { currentRegion } from "../glance";
import { autoPageBreaks, defaultPageSetup, paperMm } from "../v03";
import { AOO_PALETTE, DEFAULT_AXIS } from "../charts/toEcharts";
import { floorUnit, supportsDateAxis } from "../charts/chartAxes";
import { isDateCode } from "../numfmtLite";
import { hfShown, pageBoxes } from "../print/hfLayout";

export interface W5Ext {
  charts: Record<number, ChartDto[]>;
  images: Record<number, ImageDto[]>;
  split: Record<number, SplitDto | null>;
  nextId: number;
}

export function emptyW5(): W5Ext {
  return { charts: {}, images: {}, split: {}, nextId: 1 };
}

/** Reads cells of a sheet range (the mock's `cells`). */
export type CellLookup = (sheet: number, range: RangeDto) => Promise<CellView[]>;
/** Sheet id by (lower-case) name. */
export type SheetResolver = (name: string) => number | undefined;

export type W5Outcome = { kind: "pass" } | { kind: "handled"; sheet: number; created: number | null; objects: boolean; chartsChanged: number[] } | { kind: "error"; code: string; message: string };

/** Whether a chart reads no cells (the engine's `has_own_data`: no range anywhere). */
export function isOwnData(c: ChartDto): boolean {
  if (c.source?.range || c.categories) return false;
  return c.series.every((s) => !s.values && !s.xValues && !s.sizes && s.name?.type !== "ref" && s.errorBars?.kind.type !== "range");
}

/** The own data the engine hands out (`data_table_of`): the stored table re-shaped to the series. */
function ownTable(c: ChartDto, prev: ChartDataTableDto | null | undefined): ChartDataTableDto {
  const rows = prev?.categories.length ?? 0;
  const empty = (): (number | null)[] => Array.from({ length: rows }, () => null);
  return {
    categories: prev ? [...prev.categories] : [],
    series: c.series.map((s, i) => {
      const old = prev?.series[i];
      const kind = s.kind ?? c.kind;
      const xy = kind === "scatter" || kind === "bubble";
      return {
        source: i,
        name: s.name?.type === "text" ? s.name.text : "",
        values: old ? [...old.values] : empty(),
        x: xy || old?.x ? (old?.x ? [...old.x] : empty()) : null,
        sizes: kind === "bubble" || old?.sizes ? (old?.sizes ? [...old.sizes] : empty()) : null,
      };
    }),
  };
}

const blankSeries = (): ChartSeriesDto => ({ name: null, values: "", xValues: null, sizes: null, kind: null, secondaryAxis: false, color: null, marker: null, labels: { value: false, percent: false, category: false, seriesName: false, position: "auto" }, trend: null, errorBars: null, meanLine: false });

/** `apply_data_table`: the series follow the table (formatting from `source`), the table is stored. */
function applyTable(c: ChartDto, t: ChartDataTableDto): void {
  const old = c.series;
  c.series = t.series.map((ts) => ({ ...(ts.source != null && old[ts.source] ? structuredClone(old[ts.source]!) : blankSeries()), values: "", xValues: null, sizes: null, name: ts.name ? { type: "text", text: ts.name } : null }));
  c.dataTable = { categories: [...t.categories], series: t.series.map((s, i) => ({ ...structuredClone(s), source: i })) };
}

/** Applies a W5-5 op to the side model (async: `addChart` derives series from the cells). */
export async function applyW5Op(x: W5Ext, op: Op, cells: CellLookup, sheets: SheetResolver): Promise<W5Outcome> {
  const err = (code: string, message: string): W5Outcome => ({ kind: "error", code, message });
  switch (op.type) {
    case "addChart": {
      const list = x.charts[op.sheet] ?? [];
      if (list.length >= 1000) return err("invalid_argument", "too many charts");
      const id = x.nextId++;
      const chart = { ...structuredClone(op.chart), id, name: `Object ${id}` };
      if (chart.series.length === 0 && chart.source) {
        const derived = await deriveSeries(chart.source, cells, sheets);
        if (!derived) return err("invalid_argument", `the data range "${chart.source.range}" is not a range`);
        chart.series = derived.series;
        if (chart.categories === null) chart.categories = derived.categories;
      }
      if (isOwnData(chart)) {
        if (op.chart.dataTable) applyTable(chart, op.chart.dataTable);
        else chart.dataTable = ownTable(chart, null);
      } else chart.dataTable = null;
      list.push(chart);
      x.charts[op.sheet] = list;
      return { kind: "handled", sheet: op.sheet, created: id, objects: true, chartsChanged: [] };
    }
    case "updateChart": {
      const list = x.charts[op.sheet] ?? [];
      const i = list.findIndex((c) => c.id === op.id);
      if (i < 0) return err("not_found", `chart ${op.id} not found`);
      const chart = { ...structuredClone(op.chart), id: op.id, name: list[i]?.name ?? "" };
      if (chart.series.length === 0 && chart.source) {
        const derived = await deriveSeries(chart.source, cells, sheets);
        if (derived) {
          chart.series = derived.series;
          if (chart.categories === null) chart.categories = derived.categories;
        }
      }
      // The own data survives an edit (the editor never sends it).
      chart.dataTable = isOwnData(chart) ? ownTable(chart, list[i]?.dataTable) : null;
      list[i] = chart;
      return { kind: "handled", sheet: op.sheet, created: null, objects: true, chartsChanged: [op.id] };
    }
    case "setChartDataTable": {
      const c = (x.charts[op.sheet] ?? []).find((ch) => ch.id === op.id);
      if (!c) return err("not_found", `chart ${op.id} not found`);
      if (!isOwnData(c)) return err("invalid_argument", "the chart takes its data from cells: edit the cells instead");
      const rows = op.table.categories.length;
      const cols = op.table.series.flatMap((s) => [s.values, ...(s.x ? [s.x] : []), ...(s.sizes ? [s.sizes] : [])]);
      if (cols.some((v) => v.length !== rows)) return err("invalid_argument", "every column of the chart data table needs one cell per row");
      applyTable(c, op.table);
      return { kind: "handled", sheet: op.sheet, created: null, objects: true, chartsChanged: [op.id] };
    }
    case "moveResizeChart": {
      const c = (x.charts[op.sheet] ?? []).find((ch) => ch.id === op.id);
      if (!c) return err("not_found", `chart ${op.id} not found`);
      c.anchor = { ...op.anchor };
      return { kind: "handled", sheet: op.sheet, created: null, objects: true, chartsChanged: [] };
    }
    case "deleteChart": {
      const list = x.charts[op.sheet] ?? [];
      if (!list.some((c) => c.id === op.id)) return err("not_found", `chart ${op.id} not found`);
      x.charts[op.sheet] = list.filter((c) => c.id !== op.id);
      return { kind: "handled", sheet: op.sheet, created: null, objects: true, chartsChanged: [] };
    }
    case "moveChartToSheet": {
      const list = x.charts[op.sheet] ?? [];
      const c = list.find((ch) => ch.id === op.id);
      if (!c) return err("not_found", `chart ${op.id} not found`);
      x.charts[op.sheet] = list.filter((ch) => ch.id !== op.id);
      x.charts[op.to] = [...(x.charts[op.to] ?? []), c];
      return { kind: "handled", sheet: op.sheet, created: null, objects: true, chartsChanged: [] };
    }
    case "addImage": {
      const id = x.nextId++;
      const data = op.image.data;
      if (data.length > (64 * 1024 * 1024 * 4) / 3) return err("invalid_argument", "the picture is too large");
      x.images[op.sheet] = [...(x.images[op.sheet] ?? []), { ...op.image, id, name: op.image.name || `Picture ${id}` }];
      return { kind: "handled", sheet: op.sheet, created: id, objects: true, chartsChanged: [] };
    }
    case "deleteImage": {
      const list = x.images[op.sheet] ?? [];
      if (!list.some((i) => i.id === op.id)) return err("not_found", `picture ${op.id} not found`);
      x.images[op.sheet] = list.filter((i) => i.id !== op.id);
      return { kind: "handled", sheet: op.sheet, created: null, objects: true, chartsChanged: [] };
    }
    case "moveResizeImage": {
      const im = (x.images[op.sheet] ?? []).find((i) => i.id === op.id);
      if (!im) return err("not_found", `picture ${op.id} not found`);
      im.anchor = { ...op.anchor };
      return { kind: "handled", sheet: op.sheet, created: null, objects: true, chartsChanged: [] };
    }
    case "setSplit":
      x.split[op.sheet] = op.split ? { ...op.split } : null;
      return { kind: "handled", sheet: op.sheet, created: null, objects: false, chartsChanged: [] };
    default:
      return { kind: "pass" };
  }
}

/** Charts of `sheet` whose series ranges intersect `ranges` (for `chartsChanged`). */
export function chartsTouched(x: W5Ext, sheet: number, ranges: readonly RangeDto[], sheets: SheetResolver): number[] {
  const out: number[] = [];
  for (const c of x.charts[sheet] ?? []) {
    const refs = [c.categories, ...c.series.flatMap((s) => [s.values, s.xValues, s.sizes, s.name?.type === "ref" ? s.name.ref : null])];
    const hit = refs.some((ref) => {
      if (!ref) return false;
      const p = parseAbsoluteRef(ref);
      if (!p) return false;
      const sid = sheets(p.sheet.toLowerCase());
      if (sid !== sheet) return false;
      return ranges.some((r) => r.r0 <= p.range.r1 && p.range.r0 <= r.r1 && r.c0 <= p.range.c1 && p.range.c0 <= r.c1);
    });
    if (hit) out.push(c.id);
  }
  return out;
}

// ---------------------------------------------------------------------------
// Series derivation and data
// ---------------------------------------------------------------------------

/** Derives series from a wizard source range (AOO: labels row/column, series in columns or rows). */
export async function deriveSeries(source: ChartDto["source"] & object, cells: CellLookup, sheets: SheetResolver): Promise<{ series: ChartSeriesDto[]; categories: string | null } | null> {
  const p = parseAbsoluteRef(source.range);
  if (!p) return null;
  void cells;
  void sheets;
  const { r0, c0, r1, c1 } = p.range;
  const sheet = p.sheet;
  const ref = (rg: RangeDto) => absoluteRef(sheet, rg);
  const series: ChartSeriesDto[] = [];
  const blank = (values: string, name: ChartSeriesDto["name"]): ChartSeriesDto => ({ name, values, xValues: null, sizes: null, kind: null, secondaryAxis: false, color: null, marker: null, labels: { value: false, percent: false, category: false, seriesName: false, position: "auto" }, trend: null, errorBars: null, meanLine: false });
  let categories: string | null = null;
  if (!source.seriesInRows) {
    const top = source.firstRowLabel ? r0 + 1 : r0;
    const left = source.firstColLabel ? c0 + 1 : c0;
    if (top > r1) return { series: [], categories: null };
    if (source.firstColLabel) categories = ref({ r0: top, c0, r1, c1: c0 });
    for (let c = left; c <= c1; c++) series.push(blank(ref({ r0: top, c0: c, r1, c1: c }), source.firstRowLabel ? { type: "ref", ref: ref({ r0, c0: c, r1: r0, c1: c }) } : null));
  } else {
    const top = source.firstRowLabel ? r0 + 1 : r0;
    const left = source.firstColLabel ? c0 + 1 : c0;
    if (left > c1) return { series: [], categories: null };
    if (source.firstRowLabel) categories = ref({ r0, c0: left, r1: r0, c1 });
    for (let r = top; r <= r1; r++) series.push(blank(ref({ r0: r, c0: left, r1: r, c1 }), source.firstColLabel ? { type: "ref", ref: ref({ r0: r, c0, r1: r, c1: c0 }) } : null));
  }
  return { series, categories };
}

async function readRef(ref: string | null, cells: CellLookup, sheets: SheetResolver, problems: string[]): Promise<CellView[] | null> {
  if (!ref) return null;
  const p = parseAbsoluteRef(ref);
  const sid = p ? sheets(p.sheet.toLowerCase()) : undefined;
  if (!p || sid === undefined) {
    problems.push(`"${ref}" is not a range`);
    return null;
  }
  const n = (p.range.r1 - p.range.r0 + 1) * (p.range.c1 - p.range.c0 + 1);
  if (n > 32_000) p.range.r1 = p.range.r0 + Math.min(31_999, p.range.r1 - p.range.r0);
  const list = await cells(sid, p.range);
  const byKey = new Map(list.map((c) => [`${c.r},${c.c}`, c]));
  const out: CellView[] = [];
  for (let r = p.range.r0; r <= p.range.r1; r++) for (let c = p.range.c0; c <= p.range.c1; c++) out.push(byKey.get(`${r},${c}`) ?? { r, c, text: "", kind: "empty", num: null, style: 0, color: null, flags: 0 });
  return out;
}

const nums = (cells: CellView[] | null): (number | null)[] => (cells ?? []).map((c) => (c.kind === "number" && typeof c.num === "number" ? c.num : c.kind === "number" ? Number(c.text.replace(/[^0-9.-]/g, "")) || null : null));

/** Least-squares trend on transformed axes (R² as RSQ would compute it on the transformed data). */
export function trend(kind: "linear" | "logarithmic" | "exponential" | "power", xs: number[], ys: number[]): TrendDataDto | null {
  const pts: [number, number][] = [];
  for (let i = 0; i < Math.min(xs.length, ys.length); i++) {
    const x = xs[i];
    const y = ys[i];
    if (x === undefined || y === undefined || !Number.isFinite(x) || !Number.isFinite(y)) continue;
    const tx = kind === "logarithmic" || kind === "power" ? (x > 0 ? Math.log(x) : NaN) : x;
    const ty = kind === "exponential" || kind === "power" ? (y > 0 ? Math.log(y) : NaN) : y;
    if (Number.isFinite(tx) && Number.isFinite(ty)) pts.push([tx, ty]);
  }
  if (pts.length < 2) return null;
  const n = pts.length;
  const sx = pts.reduce((a, p) => a + p[0], 0);
  const sy = pts.reduce((a, p) => a + p[1], 0);
  const sxx = pts.reduce((a, p) => a + p[0] * p[0], 0);
  const sxy = pts.reduce((a, p) => a + p[0] * p[1], 0);
  const syy = pts.reduce((a, p) => a + p[1] * p[1], 0);
  const den = n * sxx - sx * sx;
  if (den === 0) return null;
  const b = (n * sxy - sx * sy) / den;
  const a = (sy - b * sx) / n;
  const r2den = (n * sxx - sx * sx) * (n * syy - sy * sy);
  const r2 = r2den === 0 ? 1 : (n * sxy - sx * sy) ** 2 / r2den;
  const f = (x: number): number => {
    switch (kind) {
      case "linear":
        return a + b * x;
      case "logarithmic":
        return a + b * Math.log(x);
      case "exponential":
        return Math.exp(a) * Math.exp(b * x);
      case "power":
        return Math.exp(a) * x ** b;
    }
  };
  const valid = xs.filter((x) => Number.isFinite(x) && (kind === "linear" || kind === "exponential" || x > 0));
  const x0 = Math.min(...valid);
  const x1 = Math.max(...valid);
  const points: [number, number][] = [];
  const steps = kind === "linear" ? 1 : 20;
  for (let i = 0; i <= steps; i++) {
    const x = x0 + ((x1 - x0) * i) / steps;
    const y = f(x);
    if (Number.isFinite(y)) points.push([round(x), round(y)]);
  }
  const fmt = (v: number) => String(round(v));
  const equation =
    kind === "linear"
      ? `f(x) = ${fmt(b)}x ${a < 0 ? "-" : "+"} ${fmt(Math.abs(a))}`
      : kind === "logarithmic"
        ? `f(x) = ${fmt(b)} ln(x) ${a < 0 ? "-" : "+"} ${fmt(Math.abs(a))}`
        : kind === "exponential"
          ? `f(x) = ${fmt(Math.exp(a))} exp(${fmt(b)}x)`
          : `f(x) = ${fmt(Math.exp(a))} x^${fmt(b)}`;
  return { points, equation, r2: round(r2, 6) };
}

function round(v: number, digits = 4): number {
  const k = 10 ** digits;
  return Math.round(v * k) / k;
}

function stats(values: (number | null)[]): { mean: number; sd: number; se: number; variance: number; n: number } {
  const v = values.filter((x): x is number => x !== null && Number.isFinite(x));
  const n = v.length;
  const mean = n > 0 ? v.reduce((a, b) => a + b, 0) / n : 0;
  const variance = n > 1 ? v.reduce((a, b) => a + (b - mean) ** 2, 0) / (n - 1) : 0;
  const sd = Math.sqrt(variance);
  return { mean, sd, se: n > 0 ? sd / Math.sqrt(n) : 0, variance, n };
}

/** `chart_data` for a chart from the mock's cells. */
/** The engine's `date_axis_of`: AOO's automatic/explicit date axis over the category cells (days since 1970, floored to the resolution). */
export async function dateAxisFor(chart: ChartDto, catCells: CellView[] | null, formatOf: (style: number) => Promise<string | null>): Promise<ChartDataDto["dateAxis"]> {
  const type = chart.xAxis.axisType;
  if (type === "text" || !supportsDateAxis(chart.kind) || !catCells || catCells.length === 0) return null;
  const days: (number | null)[] = [];
  let firstFormat: string | null = null;
  for (const c of catCells) {
    if (c.kind === "empty" || (c.kind === "text" && c.text === "")) {
      days.push(null);
      continue;
    }
    if (c.kind !== "number" || c.num === null) return null;
    const code = await formatOf(c.style);
    const date = code !== null && isDateCode(code);
    if (type === "auto" && !date) return null;
    if (firstFormat === null && code !== null && !/^general$/i.test(code.trim())) firstFormat = code;
    days.push(Math.floor(c.num) - 25_569);
  }
  const sorted = days.filter((d): d is number => d !== null).sort((a, b) => a - b);
  if (sorted.length === 0) return null;
  let res = chart.xAxis.timeResolution;
  if (res === null) {
    res = "years";
    for (let i = 1; i < sorted.length; i++) {
      const a = new Date(sorted[i - 1]! * 86_400_000);
      const b = new Date(sorted[i]! * 86_400_000);
      if (res === "years" && a.getUTCFullYear() === b.getUTCFullYear()) res = "months";
      if (res === "months" && a.getUTCFullYear() === b.getUTCFullYear() && a.getUTCMonth() === b.getUTCMonth()) {
        res = "days";
        break;
      }
    }
  }
  const r = res;
  return { days: days.map((d) => (d === null ? null : floorUnit(d, r))), resolution: r, numberFormat: chart.xAxis.numberFormat ?? firstFormat };
}

export async function chartDataFor(sheet: number, chart: ChartDto, version: number, cells: CellLookup, sheets: SheetResolver, formatOf: (style: number) => Promise<string | null> = async () => null): Promise<ChartDataDto> {
  const problems: string[] = [];
  const own = chart.dataTable;
  if (own) {
    // Own data: the table is the data.
    const cats = own.categories.some((c) => c.trim() !== "") ? [...own.categories] : own.categories.map((_, j) => String(j + 1));
    const series: SeriesDataDto[] = own.series.map((s, i) => ({
      name: s.name || `Series ${i + 1}`,
      color: chart.series[i]?.color ?? (((AOO_PALETTE[i % AOO_PALETTE.length] ?? 0) << 8) | 0xff) >>> 0,
      values: [...s.values],
      x: s.x ? [...s.x] : null,
      sizes: s.sizes ? [...s.sizes] : null,
      labels: s.values.map((v) => (v === null ? "" : String(v))),
      numberFormat: null,
      trend: null,
      errors: null,
      mean: null,
    }));
    return { chart: { sheet, id: chart.id }, version, categories: cats, dateAxis: null, series, truncated: false, problems };
  }
  const catCells = await readRef(chart.categories, cells, sheets, problems);
  let categories = (catCells ?? []).map((c) => c.text);
  const series: SeriesDataDto[] = [];
  for (const [i, s] of chart.series.entries()) {
    const vCells = await readRef(s.values, cells, sheets, problems);
    const values = nums(vCells);
    const xCells = s.xValues ? await readRef(s.xValues, cells, sheets, problems) : null;
    const sCells = s.sizes ? await readRef(s.sizes, cells, sheets, problems) : null;
    let name = `Series ${i + 1}`;
    if (s.name?.type === "text") name = s.name.text;
    else if (s.name?.type === "ref") {
      const nc = await readRef(s.name.ref, cells, sheets, problems);
      name = nc?.[0]?.text || name;
    }
    const x = xCells ? nums(xCells) : null;
    const xs = x ? x.map((v) => v ?? NaN) : values.map((_, j) => j + 1);
    const st = stats(values);
    let errors: [number, number][] | null = null;
    if (s.errorBars) {
      const k = s.errorBars.kind;
      let plusCells: CellView[] | null = null;
      let minusCells: CellView[] | null = null;
      if (k.type === "range") {
        plusCells = await readRef(k.plus, cells, sheets, problems);
        minusCells = await readRef(k.minus, cells, sheets, problems);
      }
      errors = values.map((v, j) => {
        const y = v ?? 0;
        switch (k.type) {
          case "constant":
            return [k.minus, k.plus];
          case "percent":
            return [Math.abs(y) * (k.percent / 100), Math.abs(y) * (k.percent / 100)];
          case "stdDev":
            return [st.sd * k.k, st.sd * k.k];
          case "variance":
            return [st.variance, st.variance];
          case "stdError":
            return [st.se, st.se];
          case "errorMargin":
            return [Math.abs(st.mean) * (k.percent / 100), Math.abs(st.mean) * (k.percent / 100)];
          case "range":
            return [nums(minusCells)[j] ?? 0, nums(plusCells)[j] ?? 0];
        }
      });
    }
    series.push({
      name,
      color: s.color ?? (((AOO_PALETTE[i % AOO_PALETTE.length] ?? 0) << 8) | 0xff) >>> 0,
      values,
      x,
      sizes: sCells ? nums(sCells) : null,
      labels: (vCells ?? []).map((c) => c.text),
      numberFormat: null,
      trend: s.trend ? trend(s.trend.kind, xs, values.map((v) => v ?? NaN)) : null,
      errors,
      mean: s.meanLine && st.n > 0 ? round(st.mean) : null,
    });
  }
  if (categories.length === 0 && series.length > 0) categories = (series[0]?.values ?? []).map((_, j) => String(j + 1));
  const dateAxis = await dateAxisFor(chart, catCells, formatOf);
  return { chart: { sheet, id: chart.id }, version, categories, dateAxis, series: series.slice(0, 255), truncated: series.length > 255, problems };
}

/** `suggest_chart`: the region around the selection, labels heuristic, anchored right of it (16 × 9 cm). */
export async function suggestChartFor(sheetName: string, sel: RangeDto, kind: ChartKind, used: RangeDto | null, cells: CellLookup, sheet: number): Promise<ChartDto> {
  let region = sel;
  const single = sel.r0 === sel.r1 && sel.c0 === sel.c1;
  if (single && used) {
    const list = await cells(sheet, used);
    const map = new Map(list.map((c) => [`${c.r},${c.c}`, c]));
    region = currentRegion((r, c) => map.get(`${r},${c}`), { r: sel.r0, c: sel.c0 }, used) ?? sel;
  }
  const list = await cells(sheet, region);
  const at = (r: number, c: number) => list.find((v) => v.r === r && v.c === c);
  const isText = (v: CellView | undefined) => !!v && v.kind === "text" && v.text !== "";
  const rows = region.r1 - region.r0 + 1;
  const cols = region.c1 - region.c0 + 1;
  const seriesInRows = rows < cols;
  const firstRowLabel = Array.from({ length: cols }, (_, i) => at(region.r0, region.c0 + i)).some(isText);
  const firstColLabel = Array.from({ length: rows }, (_, i) => at(region.r0 + i, region.c0)).some(isText);
  return {
    id: 0,
    name: "",
    anchor: { r: region.r0, c: region.c1 + 1, dxMm: 3, dyMm: 0, widthMm: 160, heightMm: 90 },
    kind,
    stacking: "none",
    lines: true,
    markers: kind === "scatter",
    smooth: false,
    hole: null,
    exploded: false,
    filled: false,
    stock: kind === "stock" ? "hlc" : null,
    threeD: false,
    source: { range: absoluteRef(sheetName, region), seriesInRows, firstRowLabel, firstColLabel },
    categories: null,
    series: [],
    title: null,
    subtitle: null,
    legend: "right",
    xAxis: { ...DEFAULT_AXIS, majorGrid: false },
    yAxis: { ...DEFAULT_AXIS },
    y2Axis: null,
    background: null,
    plotBackground: null,
    includeHidden: false,
  };
}

// ---------------------------------------------------------------------------
// Print
// ---------------------------------------------------------------------------

/** Paginates a sheet with the v0.3 estimate: pages top-to-bottom, then right (AOO order). */
export function printPagesFor(sheet: number, setup: PageSetupDto, layout: SheetLayout, firstIndex: number, firstNumber: number): PrintPageDto[] {
  const area = setup.printRange ?? layout.used;
  if (!area) return [];
  const auto = autoPageBreaks(setup, area, { runs: layout.rowRuns, dflt: layout.defaultRowHeightPx }, { runs: layout.colRuns, dflt: layout.defaultColWidthPx });
  const rowEdges = [area.r0, ...new Set([...auto.rows, ...setup.rowBreaks].filter((at) => at > area.r0 && at <= area.r1).sort((a, b) => a - b)), area.r1 + 1];
  const colEdges = [area.c0, ...new Set([...auto.cols, ...setup.colBreaks].filter((at) => at > area.c0 && at <= area.c1).sort((a, b) => a - b)), area.c1 + 1];
  const paper = paperMm(setup);
  const pages: PrintPageDto[] = [];
  const m = setup.margins;
  const boxes = pageBoxes(setup, paper);
  const bodyW = paper[0] - m.leftMm - m.rightMm;
  const bodyH = boxes.bodyHeight;
  const pct = setup.scale.type === "percent" ? setup.scale.percent : 100;
  const head = hfShown(setup, "header");
  const foot = hfShown(setup, "footer");
  const order: [number, number][] = [];
  if (setup.pageOrder === "leftToRight") {
    for (let ri = 0; ri < rowEdges.length - 1; ri++) for (let ci = 0; ci < colEdges.length - 1; ci++) order.push([ri, ci]);
  } else {
    for (let ci = 0; ci < colEdges.length - 1; ci++) for (let ri = 0; ri < rowEdges.length - 1; ri++) order.push([ri, ci]);
  }
  for (const [ri, ci] of order) {
    const idx = firstIndex + pages.length;
    const r0 = rowEdges[ri] ?? area.r0;
    const r1 = (rowEdges[ri + 1] ?? area.r1 + 1) - 1;
    const c0 = colEdges[ci] ?? area.c0;
    const c1 = (colEdges[ci + 1] ?? area.c1 + 1) - 1;
    const resolve = (s: string) => s.replace(/&\[PAGE\]/g, String(firstNumber + pages.length)).replace(/&\[PAGES\]/g, "?").replace(/&\[SHEET\]/g, "Sheet");
    pages.push({
      index: idx,
      number: firstNumber + pages.length,
      sheet,
      rows: [r0, r1],
      cols: [c0, c1],
      repeatRows: setup.repeatRows,
      repeatCols: setup.repeatCols,
      scalePct: pct,
      paperMm: [paper[0], paper[1]],
      originMm: [m.leftMm + (setup.centerH ? Math.max(0, bodyW) * 0 : 0), boxes.bodyTop + (setup.centerV ? Math.max(0, bodyH) * 0 : 0)],
      header: head ? [resolve(head.left), resolve(head.center), resolve(head.right)] : null,
      footer: foot ? [resolve(foot.left), resolve(foot.center), resolve(foot.right)] : null,
    });
  }
  return pages;
}

/** Paginates `sheets` in order; the selection applies on the `active` one (the user's sheet), as in the engine. */
export function printLayoutFor(sheets: { id: number; active?: boolean; setup: PageSetupDto; layout: SheetLayout }[], opts: PrintOptionsDto): PrintLayoutDto {
  let pages: PrintPageDto[] = [];
  const onSheet = sheets.some((s) => s.active) ? sheets.filter((s) => s.active) : sheets.slice(0, 1);
  for (const s of sheets) {
    // Each selected range is its own print range; whole columns / rows end at the data (as the engine).
    const used = s.layout.used;
    const clip = (r: RangeDto): RangeDto | null => {
      if (!used) return null;
      const out = { ...r, r1: r.r1 >= 1_048_575 ? Math.min(r.r1, used.r1) : r.r1, c1: r.c1 >= 16_383 ? Math.min(r.c1, used.c1) : r.c1 };
      return out.r1 >= out.r0 && out.c1 >= out.c0 ? out : null;
    };
    const areas = opts.selection && onSheet.includes(s) ? opts.selection.map(clip) : [undefined];
    for (const area of areas) {
      if (area === null) continue;
      const setup = area ? { ...s.setup, printRange: area } : s.setup;
      pages.push(...printPagesFor(s.id, setup, s.layout, pages.length, s.setup.firstPageNumber ?? 1));
    }
  }
  const warnings: string[] = [];
  const ranges = opts.pageRanges ?? null;
  if (opts.pageFrom !== null || opts.pageTo !== null || ranges !== null) {
    const from = opts.pageFrom ?? 1;
    const to = opts.pageTo ?? Number.MAX_SAFE_INTEGER;
    const keep = (pos: number) => pos >= from && pos <= to && (ranges === null || ranges.some(([a, b]) => pos >= Math.min(a, b) && pos <= Math.max(a, b)));
    pages = pages.filter((p) => keep(p.index + 1)).map((p, i) => ({ ...p, index: i }));
    if (pages.length === 0) warnings.push("The page range selects no pages.");
  }
  if (pages.length > 10_000) warnings.push("The print has more than 10,000 pages; only the first are shown.");
  return { pages: pages.slice(0, 10_000), warnings };
}

export function pageBreaksFor(setup: PageSetupDto, layout: SheetLayout): PageBreaksDto {
  const area = setup.printRange ?? layout.used;
  if (!area) return { area: null, rowBreaks: [], colBreaks: [] };
  const auto = autoPageBreaks(setup, area, { runs: layout.rowRuns, dflt: layout.defaultRowHeightPx }, { runs: layout.colRuns, dflt: layout.defaultColWidthPx });
  const merge = (autoAt: number[], manual: number[]) => {
    const m = new Set(manual);
    return [...new Set([...autoAt, ...manual])].sort((a, b) => a - b).map((at) => ({ at, manual: m.has(at) }));
  };
  return { area, rowBreaks: merge(auto.rows, setup.rowBreaks), colBreaks: merge(auto.cols, setup.colBreaks) };
}

/** A 1×1 white PNG (node tests, no canvas). */
export const BLANK_PNG = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+ip1sAAAAASUVORK5CYII=";

/** Rasterizes a page on a canvas (browser): paper, cell texts, header/footer, charts (as the SVGs given). Base64 PNG. */
export function renderPagePng(page: PrintPageDto, cells: CellView[], layout: SheetLayout, dpi: number, charts: ChartSvgDto[], chartBoxes: { id: number; x: number; y: number; w: number; h: number }[]): string {
  if (typeof document === "undefined") return BLANK_PNG;
  const k = dpi / 25.4;
  const w = Math.max(1, Math.round(page.paperMm[0] * k));
  const h = Math.max(1, Math.round(page.paperMm[1] * k));
  const canvas = document.createElement("canvas");
  canvas.width = w;
  canvas.height = h;
  const ctx = canvas.getContext("2d");
  if (!ctx) return BLANK_PNG;
  ctx.fillStyle = "#fff";
  ctx.fillRect(0, 0, w, h);
  const pxPerMm = k;
  const scale = (page.scalePct / 100) * (dpi / 96);
  const ox = page.originMm[0] * pxPerMm;
  const oy = page.originMm[1] * pxPerMm;
  const size = (axis: "row" | "col", i: number): number => {
    const runs = axis === "row" ? layout.rowRuns : layout.colRuns;
    const run = runs.find((r) => i >= r.start && i <= r.end);
    if (run?.hidden) return 0;
    return (run ? run.size : axis === "row" ? layout.defaultRowHeightPx : layout.defaultColWidthPx) * scale;
  };
  ctx.font = `${Math.round(13 * scale)}px Inter, system-ui, sans-serif`;
  ctx.fillStyle = "#111";
  ctx.strokeStyle = "#ddd";
  ctx.lineWidth = 1;
  let y = oy;
  for (let r = page.rows[0]; r <= page.rows[1]; r++) {
    const rh = size("row", r);
    if (rh === 0) continue;
    let x = ox;
    for (let c = page.cols[0]; c <= page.cols[1]; c++) {
      const cw = size("col", c);
      if (cw === 0) continue;
      const cell = cells.find((v) => v.r === r && v.c === c);
      if (cell && cell.text !== "") {
        ctx.save();
        ctx.beginPath();
        ctx.rect(x, y, cw, rh);
        ctx.clip();
        ctx.textBaseline = "middle";
        const right = cell.kind === "number";
        ctx.textAlign = right ? "right" : "left";
        ctx.fillText(cell.text, right ? x + cw - 3 * scale : x + 3 * scale, y + rh / 2);
        ctx.restore();
      }
      x += cw;
    }
    y += rh;
  }
  const hf = (parts: [string, string, string] | null, yy: number) => {
    if (!parts) return;
    ctx.fillStyle = "#555";
    ctx.textBaseline = "middle";
    ctx.textAlign = "left";
    ctx.fillText(parts[0], ox, yy);
    ctx.textAlign = "center";
    ctx.fillText(parts[1], w / 2, yy);
    ctx.textAlign = "right";
    ctx.fillText(parts[2], w - ox, yy);
  };
  hf(page.header, oy / 2);
  hf(page.footer, h - oy / 2);
  for (const box of chartBoxes) {
    const svg = charts.find((c) => c.id === box.id);
    ctx.strokeStyle = "#999";
    ctx.strokeRect(box.x * pxPerMm, box.y * pxPerMm, box.w * pxPerMm, box.h * pxPerMm);
    if (svg) {
      ctx.fillStyle = "#777";
      ctx.textAlign = "center";
      ctx.fillText(`[chart ${box.id}]`, (box.x + box.w / 2) * pxPerMm, (box.y + box.h / 2) * pxPerMm);
    }
  }
  return canvas.toDataURL("image/png").replace(/^data:image\/png;base64,/, "");
}

// ---------------------------------------------------------------------------
// Detective
// ---------------------------------------------------------------------------

const REF_RE = /(?:(?:'((?:[^']|'')+)'|([A-Za-z_][A-Za-z0-9_]*))\.)?\$?([A-Za-z]{1,3})\$?(\d+)(?::\$?([A-Za-z]{1,3})\$?(\d+))?/g;

/** The ranges a formula text references (same-sheet unless a sheet prefix is given). */
export function refsIn(formula: string, sheet: number, sheets: SheetResolver): { sheet: number; range: RangeDto }[] {
  if (!formula.startsWith("=")) return [];
  const col = (s: string) => [...s.toUpperCase()].reduce((n, ch) => n * 26 + ch.charCodeAt(0) - 64, 0) - 1;
  const out: { sheet: number; range: RangeDto }[] = [];
  const text = formula.replace(/"[^"]*"/g, (m) => " ".repeat(m.length));
  for (const m of text.matchAll(REF_RE)) {
    const name = m[1] !== undefined ? m[1].replace(/''/g, "'") : m[2];
    const sid = name === undefined ? sheet : sheets(name.toLowerCase());
    if (sid === undefined) continue;
    // Skip function names such as LOG10( (letters followed by digits then a parenthesis).
    const end = (m.index ?? 0) + m[0].length;
    if (text[end] === "(") continue;
    const c0 = col(m[3] ?? "A");
    const r0 = Number(m[4]) - 1;
    const c1 = m[5] ? col(m[5]) : c0;
    const r1 = m[6] ? Number(m[6]) - 1 : r0;
    out.push({ sheet: sid, range: { r0: Math.min(r0, r1), c0: Math.min(c0, c1), r1: Math.max(r0, r1), c1: Math.max(c0, c1) } });
  }
  return out;
}

export function precedentsOf(formula: string, sheet: number, sheets: SheetResolver, isError: (sheet: number, rg: RangeDto) => boolean): TraceDto[] {
  return refsIn(formula, sheet, sheets).map((r) => ({ sheet: r.sheet, range: r.range, error: isError(r.sheet, r.range) }));
}

/** Formulas (as `[cell, input]`) of `sheet` that read `cell`. */
export function dependentsOf(cell: { r: number; c: number }, formulas: readonly { r: number; c: number; input: string; error: boolean }[], sheet: number, sheets: SheetResolver): TraceDto[] {
  const out: TraceDto[] = [];
  for (const f of formulas) {
    const hit = refsIn(f.input, sheet, sheets).some((ref) => ref.sheet === sheet && cell.r >= ref.range.r0 && cell.r <= ref.range.r1 && cell.c >= ref.range.c0 && cell.c <= ref.range.c1);
    if (hit) out.push({ sheet, range: { r0: f.r, c0: f.c, r1: f.r, c1: f.c }, error: f.error });
  }
  return out;
}
