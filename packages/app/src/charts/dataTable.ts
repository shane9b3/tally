/**
 * The Chart Data Table model (T18; AOO 4.1 View ▸ Data Table,
 * `chart2/source/controller/dialogs/DataBrowserModel.cxx` and
 * `DataBrowser.cxx`): the columns a chart's own data shows (a category
 * column for category charts, then per series its role columns: X-Values,
 * Y-Values, Bubble Sizes, or the stock roles), and the toolbar edits as
 * pure functions over a `ChartDataTableDto` (each returns a new table).
 *
 * Rows and series follow AOO: Insert Row adds an empty row after the
 * cursor row (`InternalData::insertRow`), Insert Series an empty series
 * after the cursor's series (first when the cursor is on the categories;
 * an XY series copies the X values of its neighbour, as AOO shares the X
 * sequence), Delete Row needs a second row, Move Row Down / Move Series
 * Right swap with the next one. Series keep their `source` (the chart
 * series whose formatting they carry) through moves.
 */
import type { ChartDataTableDto, ChartDto, DataTableSeriesDto } from "@tally/bridge";
import { stockRoles } from "./toEcharts";

/** What a column holds (AOO's UI role names). */
export type DataRole = "categories" | "x" | "y" | "size" | "open" | "low" | "high" | "close";

/** One column of the grid: the categories (`series` -1) or a role of a series. */
export interface DataColumn {
  series: number;
  role: DataRole;
}

/** The cursor: a data row and a column index into `dataColumns`. */
export interface DataCursor {
  row: number;
  col: number;
}

const isXy = (kind: ChartDto["kind"]): boolean => kind === "scatter" || kind === "bubble";

/** AOO shows the category column for category diagrams only (not XY / bubble). */
export function showsCategories(chart: Pick<ChartDto, "kind">): boolean {
  return !isXy(chart.kind);
}

/** The grid's columns for a chart and its table, in AOO's order. */
export function dataColumns(chart: Pick<ChartDto, "kind" | "stock" | "series">, t: ChartDataTableDto): DataColumn[] {
  const out: DataColumn[] = [];
  if (showsCategories(chart)) out.push({ series: -1, role: "categories" });
  const stock = chart.kind === "stock" ? stockRoles(chart.stock) : null;
  t.series.forEach((s, i) => {
    if (s.x) out.push({ series: i, role: "x" });
    const r = stock?.[i];
    out.push({ series: i, role: r === undefined || r === "volume" ? "y" : r });
    if (s.sizes) out.push({ series: i, role: "size" });
  });
  return out;
}

/** Number of data rows. */
export function rowCount(t: ChartDataTableDto): number {
  return t.categories.length;
}

/** The value of a cell: text for categories, a number or `null` otherwise. */
export function cellValue(t: ChartDataTableDto, col: DataColumn, row: number): string | number | null {
  if (col.series < 0) return t.categories[row] ?? "";
  const s = t.series[col.series];
  if (!s) return null;
  const v = col.role === "x" ? s.x : col.role === "size" ? s.sizes : s.values;
  return v?.[row] ?? null;
}

/** A number for display, with the locale's decimal separator. */
export function formatCell(v: string | number | null, decimalSep = "."): string {
  if (v === null) return "";
  if (typeof v === "string") return v;
  const s = String(v);
  return decimalSep === "." ? s : s.replace(".", decimalSep);
}

/**
 * A number typed into a value cell (`1234.5`, `-3`, `1e3`, `12%`; group
 * separators allowed), `null` for an empty cell, `undefined` when it is
 * not a number (AOO keeps the cell in edit and says "Numbers are
 * required").
 */
export function parseCell(text: string, decimalSep = "."): number | null | undefined {
  let t = text.trim();
  if (t === "") return null;
  const group = decimalSep === "," ? /[.\s  ]/g : /[,\s  ]/g;
  t = t.replace(group, "");
  if (decimalSep !== ".") t = t.replace(decimalSep, ".");
  if (!/^[-+]?(\d+\.?\d*|\.\d+)(e[-+]?\d+)?%?$/i.test(t)) return undefined;
  const n = Number(t.replace(/%$/, ""));
  if (!Number.isFinite(n)) return undefined;
  return t.endsWith("%") ? n / 100 : n;
}

const clone = (t: ChartDataTableDto): ChartDataTableDto => ({
  categories: [...t.categories],
  series: t.series.map((s) => ({ ...s, values: [...s.values], x: s.x ? [...s.x] : s.x, sizes: s.sizes ? [...s.sizes] : s.sizes })),
});

const columnsOf = (s: DataTableSeriesDto): (number | null)[][] => [s.values, ...(s.x ? [s.x] : []), ...(s.sizes ? [s.sizes] : [])];

/** Sets one cell (`value`: text for categories, number or `null` for values). */
export function setCell(t: ChartDataTableDto, col: DataColumn, row: number, value: string | number | null): ChartDataTableDto {
  const n = clone(t);
  if (col.series < 0) {
    n.categories[row] = typeof value === "string" ? value : value === null ? "" : String(value);
    return n;
  }
  const s = n.series[col.series];
  if (!s) return t;
  const v = col.role === "x" ? s.x : col.role === "size" ? s.sizes : s.values;
  if (!v) return t;
  v[row] = typeof value === "number" ? value : null;
  return n;
}

/** Renames a series (the series header's name field). */
export function setSeriesName(t: ChartDataTableDto, series: number, name: string): ChartDataTableDto {
  const n = clone(t);
  const s = n.series[series];
  if (s) s.name = name;
  return n;
}

/** Insert Row: an empty row after `afterRow` (-1: first). */
export function insertRow(t: ChartDataTableDto, afterRow: number): ChartDataTableDto {
  const n = clone(t);
  const at = Math.max(0, Math.min(afterRow + 1, rowCount(t)));
  n.categories.splice(at, 0, "");
  for (const s of n.series) for (const c of columnsOf(s)) c.splice(at, 0, null);
  return n;
}

/** Delete Row (AOO keeps at least one row). */
export function deleteRow(t: ChartDataTableDto, row: number): ChartDataTableDto {
  if (!mayDeleteRow(t, row)) return t;
  const n = clone(t);
  n.categories.splice(row, 1);
  for (const s of n.series) for (const c of columnsOf(s)) c.splice(row, 1);
  return n;
}

/** Move Row Down: swaps `row` with the next row. */
export function moveRowDown(t: ChartDataTableDto, row: number): ChartDataTableDto {
  if (!mayMoveRowDown(t, row)) return t;
  const n = clone(t);
  const swap = (a: unknown[]): void => {
    const x = a[row];
    a[row] = a[row + 1];
    a[row + 1] = x;
  };
  swap(n.categories);
  for (const s of n.series) for (const c of columnsOf(s)) swap(c);
  return n;
}

/** Insert Series: an empty series after `afterSeries` (-1: first) shaped for the chart kind. */
export function insertSeries(t: ChartDataTableDto, chart: Pick<ChartDto, "kind" | "series">, afterSeries: number): ChartDataTableDto {
  const n = clone(t);
  const rows = rowCount(t);
  const at = Math.max(0, Math.min(afterSeries + 1, t.series.length));
  const near = t.series[Math.max(0, Math.min(afterSeries, t.series.length - 1))];
  const kind = chart.kind;
  const empty = (): (number | null)[] => Array.from({ length: rows }, () => null);
  const s: DataTableSeriesDto = {
    source: null,
    name: "",
    values: empty(),
    // AOO shares the X values of an XY chart's series with the new one.
    x: isXy(kind) || near?.x ? (near?.x ? [...near.x] : empty()) : null,
    sizes: kind === "bubble" || near?.sizes ? empty() : null,
  };
  n.series.splice(at, 0, s);
  return n;
}

/** Delete Series. */
export function deleteSeries(t: ChartDataTableDto, series: number): ChartDataTableDto {
  if (series < 0 || series >= t.series.length) return t;
  const n = clone(t);
  n.series.splice(series, 1);
  return n;
}

/** Move Series Right: swaps the series with the next one. */
export function moveSeriesRight(t: ChartDataTableDto, series: number): ChartDataTableDto {
  if (series < 0 || series >= t.series.length - 1) return t;
  const n = clone(t);
  const a = n.series[series]!;
  n.series[series] = n.series[series + 1]!;
  n.series[series + 1] = a;
  return n;
}

/** `DataBrowser::MayDeleteRow`: a second row exists. */
export function mayDeleteRow(t: ChartDataTableDto, row: number): boolean {
  return row >= 0 && row < rowCount(t) && rowCount(t) > 1;
}

/** `DataBrowser::MaySwapRows`: not the last row. */
export function mayMoveRowDown(t: ChartDataTableDto, row: number): boolean {
  return row >= 0 && row < rowCount(t) - 1;
}

/**
 * Whether series can be inserted or deleted: a stock chart's series are
 * its roles (fixed by the variant), so the dialog leaves them alone.
 */
export function seriesEditable(chart: Pick<ChartDto, "kind">): boolean {
  return chart.kind !== "stock";
}

/** The first column of series `i` (for moving the cursor with the series). */
export function firstColumnOf(cols: readonly DataColumn[], series: number): number {
  const i = cols.findIndex((c) => c.series === series);
  return i < 0 ? 0 : i;
}

/** Series groups for the name header row: `[series, first column, span]`, the categories as -1. */
export function headerGroups(cols: readonly DataColumn[]): { series: number; start: number; span: number }[] {
  const out: { series: number; start: number; span: number }[] = [];
  cols.forEach((c, i) => {
    const last = out[out.length - 1];
    if (last && last.series === c.series) last.span++;
    else out.push({ series: c.series, start: i, span: 1 });
  });
  return out;
}

/**
 * The table as the dialog shows it: the engine sends X and size columns
 * only where a series has them; AOO shows them for every XY / bubble
 * series, so empty ones are added here.
 */
export function withRoleColumns(chart: Pick<ChartDto, "kind" | "series">, t: ChartDataTableDto): ChartDataTableDto {
  const rows = rowCount(t);
  const empty = (): (number | null)[] => Array.from({ length: rows }, () => null);
  return {
    categories: t.categories,
    series: t.series.map((s) => {
      const kind = (s.source != null ? chart.series[s.source]?.kind : null) ?? chart.kind;
      return { ...s, x: s.x ?? (isXy(kind) ? empty() : null), sizes: s.sizes ?? (kind === "bubble" ? empty() : null) };
    }),
  };
}

/**
 * Undoes [`withRoleColumns`] for the result: an X or size column that the
 * series (by `source`) did not have in `base` and that is still empty is
 * left out again, so the chart keeps index X values (and its trend line).
 */
export function withoutAddedColumns(t: ChartDataTableDto, base: ChartDataTableDto): ChartDataTableDto {
  const blank = (v: (number | null)[] | null | undefined): boolean => !!v && v.every((x) => x === null);
  return {
    categories: t.categories,
    series: t.series.map((s) => {
      const was = s.source != null ? base.series[s.source] : undefined;
      return { ...s, x: !was?.x && blank(s.x) ? null : s.x, sizes: !was?.sizes && blank(s.sizes) ? null : s.sizes };
    }),
  };
}

/** Whether two tables hold the same data. */
export function sameTable(a: ChartDataTableDto, b: ChartDataTableDto): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}
