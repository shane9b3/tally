/**
 * `toEchartsOption(chart, data, theme)`: a pure mapping from a `ChartDto`
 * (what the user set up) plus its resolved `ChartDataDto` (what the engine
 * read from the cells) to an ECharts option (W5-5, docs/wave5-plan.md).
 *
 * Conventions (AOO 4.1 semantics, rendered 2D):
 * - column: category x, value y, `bar` series; bar: the axes swapped.
 * - stacking: `stack: "total"`; percent stacking is normalised here per
 *   category (|values| sum to 100).
 * - line: `line` series; points-only (no lines) becomes `scatter` on the
 *   category axis; area is `line` with `areaStyle`.
 * - pie: one `pie` per series as concentric rings (donut: `hole` × outer
 *   radius); exploded pies select every slice with an offset.
 * - scatter: value x; lines/smooth turn it into a `line` series over
 *   `[x, y]` points; bubble: `symbolSize ∝ sqrt(size)`, max 60 px.
 * - radar: indicators from the categories; filled = `areaStyle`.
 * - stock (series order as in the AOO data table): HLC = Low, High, Close;
 *   OHLC = Open, Low, High, Close; the volume variants put Volume first,
 *   drawn as bars on a second y axis. Candlesticks take ECharts'
 *   `[open, close, low, high]`; HLC sets open = close.
 * - combo: `ChartSeriesDto.kind` overrides the series type (a line on a
 *   column chart); `secondaryAxis` plots on y axis 1.
 * - trend line: an extra dashed `line` series over `TrendDataDto.points`
 *   (x is the 1-based category index on category charts, the x value on
 *   scatter charts) with the equation and R² as its end label.
 * - error bars: a `custom` series drawing I-shapes from `errors`; the mean
 *   value is a `markLine`.
 * - data labels: value / percent / category / series name, joined with
 *   spaces, at the requested position; formatted values come from
 *   `SeriesDataDto.labels`.
 * - axis labels are formatted with `numfmtLite` when a number format is
 *   set; colours come from the engine (`SeriesDataDto.color`, AOO palette).
 * - dark theme: text, axis and grid colours from the theme tokens.
 */
import type { ChartAxisDto, ChartDataDto, ChartDto, ChartKind, ChartSeriesDto, LabelPos, SeriesDataDto } from "@tally/bridge";
import type { ComposeOption } from "echarts/core";
import type { BarSeriesOption, CandlestickSeriesOption, CustomSeriesOption, LineSeriesOption, PieSeriesOption, RadarSeriesOption, ScatterSeriesOption } from "echarts/charts";
import type { DatasetComponentOption, GridComponentOption, LegendComponentOption, TitleComponentOption, TooltipComponentOption } from "echarts/components";
import { formatNumberLite } from "../numfmtLite";

export type EChartsOption = ComposeOption<
  | BarSeriesOption
  | LineSeriesOption
  | PieSeriesOption
  | ScatterSeriesOption
  | RadarSeriesOption
  | CandlestickSeriesOption
  | CustomSeriesOption
  | TitleComponentOption
  | LegendComponentOption
  | GridComponentOption
  | TooltipComponentOption
  | DatasetComponentOption
>;

/** What the option builder needs from the UI theme. */
export interface ChartTheme {
  dark: boolean;
  /** Text ink. */
  text: string;
  textMuted: string;
  /** Axis and grid lines. */
  gridLine: string;
  /** The chart background when the DTO sets none. */
  bg: string;
  fontFamily: string;
}

export const LIGHT_CHART_THEME: ChartTheme = {
  dark: false,
  text: "#111827",
  textMuted: "#6b7280",
  gridLine: "#e5e7eb",
  bg: "#ffffff",
  fontFamily: '"Inter Variable", "Inter", system-ui, sans-serif',
};

/** The AOO 4.1 default palette (`SeriesDataDto.color` normally carries the engine's pick). */
export const AOO_PALETTE = [0x004586, 0xff420e, 0xffd320, 0x579d1c, 0x7e0021, 0x83caff, 0x314004, 0xaecf00, 0x4b1f6f, 0xff950e, 0xc5000b, 0x0084d1] as const;

/** Largest bubble diameter in px. */
export const MAX_BUBBLE_PX = 60;

type AnySeries = BarSeriesOption | LineSeriesOption | PieSeriesOption | ScatterSeriesOption | RadarSeriesOption | CandlestickSeriesOption | CustomSeriesOption;

/** `0xRRGGBBAA` → CSS colour. */
export function cssColor(rgba: number): string {
  const n = rgba >>> 0;
  const r = (n >>> 24) & 255;
  const g = (n >>> 16) & 255;
  const b = (n >>> 8) & 255;
  const a = n & 255;
  const hex = `#${[r, g, b].map((x) => x.toString(16).padStart(2, "0")).join("")}`;
  return a === 255 || a === 0 ? hex : `rgba(${r}, ${g}, ${b}, ${(a / 255).toFixed(3)})`;
}

/** The palette colour of series `i` (`0xRRGGBB` → CSS). */
export function paletteColor(i: number): string {
  const rgb = AOO_PALETTE[i % AOO_PALETTE.length] ?? 0x004586;
  return `#${rgb.toString(16).padStart(6, "0")}`;
}

/** Series colour: the engine's resolved colour (0 = unset) else the palette. */
function seriesColor(s: SeriesDataDto | undefined, i: number): string {
  return s && s.color !== 0 ? cssColor(s.color) : paletteColor(i);
}

/** Percent-stacks the series: each category's |values| sum to 100. */
export function percentStack(values: (number | null)[][]): (number | null)[][] {
  const n = Math.max(0, ...values.map((v) => v.length));
  const totals: number[] = [];
  for (let i = 0; i < n; i++) {
    let t = 0;
    for (const v of values) t += Math.abs(v[i] ?? 0);
    totals.push(t);
  }
  return values.map((v) => v.map((y, i) => (y === null ? null : totals[i] ? (Math.abs(y) / (totals[i] ?? 1)) * 100 * Math.sign(y || 1) : 0)));
}

/** Percent stacking per value axis: the primary and secondary series each sum to 100 on their own. */
export function percentStackByAxis(values: (number | null)[][], axis: readonly number[]): (number | null)[][] {
  const out: (number | null)[][] = values.map(() => []);
  for (const a of new Set(axis)) {
    const idx = values.map((_, i) => i).filter((i) => axis[i] === a);
    const stacked = percentStack(idx.map((i) => values[i] ?? []));
    idx.forEach((i, k) => (out[i] = stacked[k] ?? []));
  }
  return out;
}

type LabelPosition = "inside" | "top" | "bottom" | "left" | "right";

/** Pie label placement for an AOO placement. */
function pieLabelPosition(pos: LabelPos): "inside" | "outside" {
  return pos === "inside" || pos === "center" ? "inside" : "outside";
}

/** ECharts label position for an AOO placement on a cartesian chart. */
function labelPosition(pos: LabelPos, _kind: ChartKind, horizontal: boolean): LabelPosition {
  switch (pos) {
    case "inside":
      return "inside";
    case "center":
      return "inside";
    case "outside":
    case "above":
      return horizontal ? "right" : "top";
    case "below":
      return horizontal ? "left" : "bottom";
    case "auto":
    default:
      return horizontal ? "right" : "top";
  }
}

interface LabelParams {
  dataIndex: number;
  seriesName?: string;
  name?: string;
  percent?: number;
  value?: unknown;
}

/** A data-label formatter for a series, or `undefined` when labels are off. */
function labelFormatter(spec: ChartSeriesDto, data: SeriesDataDto | undefined, categories: readonly string[]): ((p: LabelParams) => string) | undefined {
  const l = spec.labels;
  if (!l.value && !l.percent && !l.category && !l.seriesName) return undefined;
  return (p) => {
    const parts: string[] = [];
    if (l.seriesName) parts.push(p.seriesName ?? data?.name ?? "");
    if (l.category) parts.push(categories[p.dataIndex] ?? p.name ?? "");
    if (l.value) {
      const text = data?.labels[p.dataIndex];
      if (text !== undefined && text !== "") parts.push(text);
      else {
        const v = data?.values[p.dataIndex];
        if (v !== null && v !== undefined) parts.push(String(v));
      }
    }
    if (l.percent && typeof p.percent === "number") parts.push(`${Math.round(p.percent * 10) / 10}%`);
    return parts.filter((s) => s !== "").join(" ");
  };
}

/** Axis label formatter from a number format code. */
function axisFormatter(fmt: string | null): ((v: number) => string) | undefined {
  if (!fmt || fmt.trim() === "" || /^general$/i.test(fmt.trim())) return undefined;
  return (v: number) => formatNumberLite(v, fmt) ?? String(v);
}

/** A cartesian value axis. */
function valueAxis(a: ChartAxisDto, theme: ChartTheme, fallbackFormat: string | null): Record<string, unknown> {
  const fmt = axisFormatter(a.numberFormat ?? fallbackFormat);
  return {
    type: a.log ? "log" : "value",
    show: a.visible,
    name: a.title ?? undefined,
    nameLocation: "middle",
    nameGap: 28,
    nameTextStyle: { color: theme.textMuted },
    min: a.min ?? undefined,
    max: a.max ?? undefined,
    interval: a.major ?? undefined,
    inverse: a.reverse,
    minorTick: a.minorCount ? { show: a.minorGrid, splitNumber: a.minorCount } : undefined,
    minorSplitLine: { show: a.minorGrid, lineStyle: { color: theme.gridLine, opacity: 0.6 } },
    splitLine: { show: a.majorGrid, lineStyle: { color: theme.gridLine } },
    axisLine: { lineStyle: { color: theme.gridLine } },
    axisLabel: { color: theme.textMuted, ...(fmt ? { formatter: fmt } : {}) },
  };
}

/** A cartesian category axis. */
function categoryAxis(a: ChartAxisDto, categories: readonly string[], theme: ChartTheme): Record<string, unknown> {
  return {
    type: "category",
    show: a.visible,
    name: a.title ?? undefined,
    nameLocation: "middle",
    nameGap: 28,
    nameTextStyle: { color: theme.textMuted },
    data: [...categories],
    inverse: a.reverse,
    boundaryGap: true,
    splitLine: { show: a.majorGrid, lineStyle: { color: theme.gridLine } },
    axisLine: { lineStyle: { color: theme.gridLine } },
    axisTick: { alignWithLabel: true },
    axisLabel: { color: theme.textMuted },
  };
}

function legend(chart: ChartDto, theme: ChartTheme): LegendComponentOption {
  const pos = chart.legend;
  if (pos === "none") return { show: false };
  const vertical = pos === "left" || pos === "right";
  return {
    show: true,
    type: "scroll",
    orient: vertical ? "vertical" : "horizontal",
    left: pos === "left" ? 8 : pos === "right" ? undefined : "center",
    right: pos === "right" ? 8 : undefined,
    top: pos === "top" ? (chart.title ? 30 : 8) : pos === "bottom" ? undefined : "middle",
    bottom: pos === "bottom" ? 8 : undefined,
    textStyle: { color: theme.text },
  };
}

function title(chart: ChartDto, theme: ChartTheme): TitleComponentOption {
  return {
    show: !!chart.title || !!chart.subtitle,
    text: chart.title ?? "",
    subtext: chart.subtitle ?? "",
    left: "center",
    top: 6,
    textStyle: { color: theme.text, fontSize: 14, fontWeight: 600 },
    subtextStyle: { color: theme.textMuted },
  };
}

/** Plot-area margins leaving room for the title and the legend. */
function gridFor(chart: ChartDto): GridComponentOption {
  const top = (chart.title ? 34 : 14) + (chart.subtitle ? 16 : 0) + (chart.legend === "top" ? 26 : 0);
  return {
    left: chart.legend === "left" ? 110 : 48,
    right: chart.legend === "right" ? 110 : chart.y2Axis || chart.series.some((s) => s.secondaryAxis) ? 48 : 24,
    top,
    bottom: chart.legend === "bottom" ? 46 : 32,
    containLabel: true,
    show: chart.plotBackground !== null,
    backgroundColor: chart.plotBackground !== null ? cssColor(chart.plotBackground) : undefined,
    borderWidth: 0,
  };
}

/** The series kind actually drawn for series `i` (combo override). */
export function effectiveKind(chart: ChartDto, spec: ChartSeriesDto | undefined): ChartKind {
  return spec?.kind ?? chart.kind;
}

/** Trend-line and error-bar companion series for cartesian charts. */
function statistics(chart: ChartDto, data: ChartDataDto, i: number, color: string, horizontal: boolean, isValueX: boolean): AnySeries[] {
  const spec = chart.series[i];
  const d = data.series[i];
  if (!spec || !d) return [];
  const out: AnySeries[] = [];
  if (spec.trend && d.trend) {
    const pts = d.trend.points.map(([x, y]) => (isValueX ? [x, y] : [x - 1, y]));
    const eq = [spec.trend.showEquation ? d.trend.equation : "", spec.trend.showR2 ? `R² = ${Math.round(d.trend.r2 * 10000) / 10000}` : ""].filter((s) => s !== "").join("\n");
    out.push({
      type: "line",
      name: spec.trend.name ?? `Trend (${d.name})`,
      data: horizontal ? pts.map(([x, y]) => [y, x]) : pts,
      showSymbol: false,
      smooth: false,
      lineStyle: { type: "dashed", width: 1.5, color },
      itemStyle: { color },
      endLabel: eq ? { show: true, formatter: () => eq, color, fontSize: 11 } : undefined,
      yAxisIndex: spec.secondaryAxis ? 1 : 0,
      tooltip: { show: false },
      silent: true,
      z: 3,
    } satisfies LineSeriesOption);
  }
  if (spec.errorBars && d.errors) {
    const dir = spec.errorBars.direction;
    const items = d.values.map((y, j) => {
      const e = d.errors?.[j];
      if (y === null || !e) return null;
      const x = isValueX ? (d.x?.[j] ?? j) : j;
      if (x === null) return null;
      return [x, y, dir === "plus" ? 0 : e[0], dir === "minus" ? 0 : e[1]];
    });
    out.push({
      type: "custom",
      name: `Error bars (${d.name})`,
      data: items.filter((it): it is number[] => it !== null),
      yAxisIndex: spec.secondaryAxis ? 1 : 0,
      tooltip: { show: false },
      silent: true,
      z: 4,
      renderItem: (_params, api) => {
        const x = api.value(0) as number;
        const y = api.value(1) as number;
        const minus = api.value(2) as number;
        const plus = api.value(3) as number;
        const lo = api.coord(horizontal ? [y - minus, x] : [x, y - minus]);
        const hi = api.coord(horizontal ? [y + plus, x] : [x, y + plus]);
        const cap = 4;
        const style = { stroke: color, fill: "none", lineWidth: 1.25 };
        const line = (x1: number, y1: number, x2: number, y2: number) => ({ type: "line" as const, shape: { x1, y1, x2, y2 }, style });
        const children = horizontal
          ? [line(lo[0] ?? 0, lo[1] ?? 0, hi[0] ?? 0, hi[1] ?? 0), line(lo[0] ?? 0, (lo[1] ?? 0) - cap, lo[0] ?? 0, (lo[1] ?? 0) + cap), line(hi[0] ?? 0, (hi[1] ?? 0) - cap, hi[0] ?? 0, (hi[1] ?? 0) + cap)]
          : [line(lo[0] ?? 0, lo[1] ?? 0, hi[0] ?? 0, hi[1] ?? 0), line((lo[0] ?? 0) - cap, lo[1] ?? 0, (lo[0] ?? 0) + cap, lo[1] ?? 0), line((hi[0] ?? 0) - cap, hi[1] ?? 0, (hi[0] ?? 0) + cap, hi[1] ?? 0)];
        return { type: "group", children };
      },
    } satisfies CustomSeriesOption);
  }
  return out;
}

function meanLine(spec: ChartSeriesDto | undefined, d: SeriesDataDto | undefined, color: string, horizontal: boolean): LineSeriesOption["markLine"] | undefined {
  if (!spec?.meanLine || !d || d.mean === null) return undefined;
  return {
    silent: true,
    symbol: "none",
    lineStyle: { type: "dotted", color, width: 1.25 },
    label: { formatter: () => `Mean ${d.labels.length > 0 ? Math.round(d.mean! * 100) / 100 : d.mean}`, color },
    data: [horizontal ? { xAxis: d.mean } : { yAxis: d.mean }],
  };
}

function symbolFor(spec: ChartSeriesDto | undefined, markers: boolean): string {
  const m = spec?.marker ?? "auto";
  if (m === "none" || (!markers && m === "auto")) return "none";
  switch (m) {
    case "square":
      return "rect";
    case "diamond":
      return "diamond";
    case "triangle":
      return "triangle";
    case "cross":
    case "plus":
      return "pin";
    case "circle":
    case "auto":
    default:
      return "circle";
  }
}

/** Builds the ECharts option for a chart. Pure; `animation` is off so output is stable. */
export function toEchartsOption(chart: ChartDto, data: ChartDataDto, theme: ChartTheme = LIGHT_CHART_THEME): EChartsOption {
  const n = Math.max(0, ...data.series.map((s) => s.values.length));
  const categories = data.categories.length > 0 ? data.categories : Array.from({ length: n }, (_, i) => String(i + 1));
  const base: EChartsOption = {
    animation: false,
    backgroundColor: chart.background !== null ? cssColor(chart.background) : theme.bg,
    textStyle: { fontFamily: theme.fontFamily, color: theme.text },
    title: title(chart, theme),
    legend: legend(chart, theme),
    tooltip: { trigger: chart.kind === "pie" || chart.kind === "scatter" || chart.kind === "bubble" ? "item" : "axis", confine: true },
  };
  if (data.problems.length > 0 && data.series.length === 0) {
    return { ...base, graphic: [{ type: "text", left: "center", top: "middle", style: { text: data.problems.join("\n"), fill: theme.textMuted, fontSize: 12 } }] };
  }
  switch (chart.kind) {
    case "pie":
      return { ...base, series: pieSeries(chart, data, categories) };
    case "radar":
      return { ...base, ...radarOption(chart, data, categories, theme) };
    case "scatter":
    case "bubble":
      return { ...base, ...scatterOption(chart, data, theme) };
    case "stock":
      return { ...base, ...stockOption(chart, data, categories, theme) };
    case "column":
    case "bar":
    case "line":
    case "area":
    default:
      return { ...base, ...cartesianOption(chart, data, categories, theme) };
  }
}

function cartesianOption(chart: ChartDto, data: ChartDataDto, categories: readonly string[], theme: ChartTheme): EChartsOption {
  const horizontal = chart.kind === "bar";
  const stacked = chart.stacking !== "none";
  const values = chart.stacking === "percent" ? percentStackByAxis(data.series.map((s) => s.values), data.series.map((_, i) => (chart.series[i]?.secondaryAxis ? 1 : 0))) : data.series.map((s) => s.values);
  const series: AnySeries[] = [];
  const secondary = chart.series.some((s) => s.secondaryAxis);
  data.series.forEach((d, i) => {
    const spec = chart.series[i];
    const kind = effectiveKind(chart, spec);
    const color = seriesColor(d, i);
    const fmt = labelFormatter(spec ?? EMPTY_SPEC, d, categories);
    const label = fmt ? { show: true, position: labelPosition(spec?.labels.position ?? "auto", kind, horizontal), formatter: fmt, color: theme.text } : { show: false };
    const common = {
      name: d.name,
      data: values[i] ?? [],
      itemStyle: { color },
      label,
      yAxisIndex: spec?.secondaryAxis ? 1 : 0,
      stack: stacked && kind !== "scatter" ? "total" : undefined,
      markLine: meanLine(spec, d, color, horizontal),
    };
    if (kind === "column" || kind === "bar") {
      series.push({ ...common, type: "bar", barMaxWidth: 48 } satisfies BarSeriesOption);
    } else if (kind === "line" && !chart.lines && chart.markers) {
      series.push({ ...common, type: "scatter", symbolSize: 8, symbol: symbolFor(spec, true) } satisfies ScatterSeriesOption);
    } else {
      const area = kind === "area";
      series.push({
        ...common,
        type: "line",
        smooth: chart.smooth,
        showSymbol: area ? false : chart.markers,
        symbol: symbolFor(spec, area ? false : chart.markers),
        symbolSize: 7,
        lineStyle: { color, width: 2 },
        areaStyle: area ? { color, opacity: stacked ? 0.85 : 0.5 } : undefined,
        connectNulls: false,
      } satisfies LineSeriesOption);
    }
    series.push(...statistics(chart, data, i, color, horizontal, false));
  });
  const fallbackFmt = data.series.find((s) => s.numberFormat)?.numberFormat ?? null;
  const cat = categoryAxis(chart.xAxis, categories, theme);
  const val = valueAxis(chart.yAxis, theme, chart.stacking === "percent" ? "0\"%\"" : fallbackFmt);
  const y2 = secondary ? [valueAxis(chart.y2Axis ?? DEFAULT_AXIS, theme, fallbackFmt)] : [];
  return {
    grid: gridFor(chart),
    xAxis: horizontal ? [val, ...y2.map((a) => ({ ...a, position: "top" }))] : [cat],
    yAxis: horizontal ? [cat] : [val, ...y2.map((a) => ({ ...a, position: "right" }))],
    series,
  } as EChartsOption;
}

const EMPTY_SPEC: ChartSeriesDto = {
  name: null,
  values: "",
  xValues: null,
  sizes: null,
  kind: null,
  secondaryAxis: false,
  color: null,
  marker: null,
  labels: { value: false, percent: false, category: false, seriesName: false, position: "auto" },
  trend: null,
  errorBars: null,
  meanLine: false,
};

export const DEFAULT_AXIS: ChartAxisDto = { visible: true, title: null, min: null, max: null, major: null, minorCount: null, log: false, reverse: false, majorGrid: true, minorGrid: false, numberFormat: null };

function pieSeries(chart: ChartDto, data: ChartDataDto, categories: readonly string[]): PieSeriesOption[] {
  const rings = Math.max(1, data.series.length);
  const outer = 70;
  const hole = chart.hole ?? 0;
  const inner = Math.max(0, Math.min(0.95, hole)) * outer;
  const band = (outer - inner) / rings;
  const top = chart.title ? 36 : 10;
  return data.series.map((d, i) => {
    const spec = chart.series[i];
    const fmt = labelFormatter(spec ?? EMPTY_SPEC, d, categories);
    const r0 = inner + band * i;
    const r1 = inner + band * (i + 1) - (rings > 1 ? 2 : 0);
    return {
      type: "pie",
      name: d.name,
      radius: [`${r0}%`, `${r1}%`],
      center: ["50%", `${50 + top / 4}%`],
      selectedMode: chart.exploded ? "multiple" : false,
      selectedOffset: 12,
      label: fmt ? { show: true, position: pieLabelPosition(spec?.labels.position ?? "auto"), formatter: fmt } : { show: false },
      labelLine: { show: !!fmt && pieLabelPosition(spec?.labels.position ?? "auto") === "outside" },
      data: d.values.map((v, j) => ({
        name: categories[j] ?? String(j + 1),
        value: v ?? 0,
        selected: chart.exploded,
        itemStyle: { color: paletteColor(j) },
      })),
    } satisfies PieSeriesOption;
  });
}

function radarOption(chart: ChartDto, data: ChartDataDto, categories: readonly string[], theme: ChartTheme): EChartsOption {
  let max = 0;
  for (const s of data.series) for (const v of s.values) if (v !== null && Math.abs(v) > max) max = Math.abs(v);
  const axisMax = chart.yAxis.max ?? (max > 0 ? max : 1);
  return {
    radar: {
      indicator: categories.map((name) => ({ name, max: axisMax, min: chart.yAxis.min ?? undefined })),
      center: ["50%", chart.title ? "56%" : "50%"],
      radius: "62%",
      axisName: { color: theme.textMuted },
      splitLine: { lineStyle: { color: theme.gridLine } },
      splitArea: { show: false },
      axisLine: { lineStyle: { color: theme.gridLine } },
    },
    series: [
      {
        type: "radar",
        symbol: chart.markers ? "circle" : "none",
        symbolSize: 6,
        data: data.series.map((d, i) => {
          const color = seriesColor(d, i);
          return {
            name: d.name,
            value: d.values.map((v) => v ?? 0),
            itemStyle: { color },
            lineStyle: { color, width: 2 },
            areaStyle: chart.filled ? { color, opacity: 0.35 } : undefined,
          };
        }),
      } satisfies RadarSeriesOption,
    ],
  };
}

function scatterOption(chart: ChartDto, data: ChartDataDto, theme: ChartTheme): EChartsOption {
  const bubble = chart.kind === "bubble";
  let maxSize = 0;
  if (bubble) for (const s of data.series) for (const v of s.sizes ?? []) if (v !== null && v > maxSize) maxSize = v;
  const series: AnySeries[] = [];
  data.series.forEach((d, i) => {
    const spec = chart.series[i];
    const color = seriesColor(d, i);
    const fmt = labelFormatter(spec ?? EMPTY_SPEC, d, []);
    const label = fmt ? { show: true, position: "top" as const, formatter: fmt, color: theme.text } : { show: false };
    const points = d.values.map((y, j) => {
      const x = d.x?.[j] ?? j + 1;
      if (y === null || x === null) return null;
      return bubble ? [x, y, d.sizes?.[j] ?? 0] : [x, y];
    });
    const pts = points.filter((p): p is number[] => p !== null);
    const common = { name: d.name, itemStyle: { color }, label, yAxisIndex: spec?.secondaryAxis ? 1 : 0, markLine: meanLine(spec, d, color, false) };
    if (bubble) {
      series.push({ ...common, type: "scatter", data: pts, symbolSize: (v: number[]) => (maxSize > 0 ? Math.max(4, Math.sqrt(Math.max(0, v[2] ?? 0) / maxSize) * MAX_BUBBLE_PX) : 10), itemStyle: { color, opacity: 0.75 } } satisfies ScatterSeriesOption);
    } else if (chart.lines) {
      series.push({ ...common, type: "line", data: pts, smooth: chart.smooth, showSymbol: chart.markers, symbol: symbolFor(spec, chart.markers), symbolSize: 7, lineStyle: { color, width: 2 } } satisfies LineSeriesOption);
    } else {
      series.push({ ...common, type: "scatter", data: pts, symbol: symbolFor(spec, true), symbolSize: 8 } satisfies ScatterSeriesOption);
    }
    series.push(...statistics(chart, data, i, color, false, true));
  });
  const secondary = chart.series.some((s) => s.secondaryAxis);
  const fmt = data.series.find((s) => s.numberFormat)?.numberFormat ?? null;
  return {
    grid: gridFor(chart),
    xAxis: [{ ...valueAxis(chart.xAxis, theme, null), scale: true }],
    yAxis: [valueAxis(chart.yAxis, theme, fmt), ...(secondary ? [{ ...valueAxis(chart.y2Axis ?? DEFAULT_AXIS, theme, fmt), position: "right" }] : [])],
    series,
  } as EChartsOption;
}

/** Series roles by stock variant, in AOO data-table order. */
export function stockRoles(variant: ChartDto["stock"]): ("volume" | "open" | "low" | "high" | "close")[] {
  switch (variant) {
    case "ohlc":
      return ["open", "low", "high", "close"];
    case "volumeHlc":
      return ["volume", "low", "high", "close"];
    case "volumeOhlc":
      return ["volume", "open", "low", "high", "close"];
    case "hlc":
    default:
      return ["low", "high", "close"];
  }
}

function stockOption(chart: ChartDto, data: ChartDataDto, categories: readonly string[], theme: ChartTheme): EChartsOption {
  const roles = stockRoles(chart.stock);
  const by = (role: string): SeriesDataDto | undefined => {
    const i = roles.indexOf(role as (typeof roles)[number]);
    return i >= 0 ? data.series[i] : undefined;
  };
  const n = categories.length;
  const low = by("low");
  const high = by("high");
  const close = by("close");
  const open = by("open");
  const volume = by("volume");
  const candles = Array.from({ length: n }, (_, j) => {
    const c = close?.values[j] ?? null;
    const o = open ? (open.values[j] ?? null) : c;
    const l = low?.values[j] ?? null;
    const h = high?.values[j] ?? null;
    return o === null || c === null || l === null || h === null ? [null, null, null, null] : [o, c, l, h];
  });
  const up = "#579d1c";
  const down = "#c5000b";
  const series: AnySeries[] = [
    {
      type: "candlestick",
      name: close?.name ?? "Stock",
      data: candles,
      itemStyle: { color: up, color0: down, borderColor: up, borderColor0: down },
      barMaxWidth: 14,
      yAxisIndex: 0,
    } satisfies CandlestickSeriesOption,
  ];
  if (volume) {
    series.unshift({
      type: "bar",
      name: volume.name,
      data: volume.values,
      yAxisIndex: 1,
      itemStyle: { color: seriesColor(volume, 0), opacity: 0.6 },
      barMaxWidth: 14,
    } satisfies BarSeriesOption);
  }
  const fmt = close?.numberFormat ?? null;
  return {
    grid: gridFor(chart),
    xAxis: [categoryAxis(chart.xAxis, categories, theme)],
    yAxis: [
      { ...valueAxis(chart.yAxis, theme, fmt), scale: true },
      ...(volume ? [{ ...valueAxis(chart.y2Axis ?? { ...DEFAULT_AXIS, majorGrid: false }, theme, null), position: "right", max: (v: { max: number }) => v.max * 3 }] : []),
    ],
    series,
  } as EChartsOption;
}
