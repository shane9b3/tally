/**
 * W5-5: the ECharts option builder. One snapshot per chart kind × variant
 * (plus combo, trend, error bars, multi-ring donut, bubble and the four
 * stock variants), and property checks: percent stacks sum to 100, a bar
 * chart swaps the axes, the palette order, dark-theme colours.
 */
import type { ChartDataDto, ChartDto, ChartSeriesDto } from "@tally/bridge";
import { describe, expect, it } from "vitest";
import { AOO_PALETTE, axisCrossFixes, cssColor, DEFAULT_AXIS, LIGHT_CHART_THEME, paletteColor, percentStack, percentStackByAxis, stockRoles, toEchartsOption, type EChartsOption } from "../src/charts/toEcharts";
import { addUnits, applyAxisCrossFixes, axisRangeInvalid, DAY_MS, dateScale, floorUnit, isoToSerial, serialToIso } from "../src/charts/chartAxes";

function series(values: string, extra: Partial<ChartSeriesDto> = {}): ChartSeriesDto {
  return { name: null, values, xValues: null, sizes: null, kind: null, secondaryAxis: false, color: null, marker: null, labels: { value: false, percent: false, category: false, seriesName: false, position: "auto" }, trend: null, errorBars: null, meanLine: false, ...extra };
}

function chart(p: Partial<ChartDto> = {}): ChartDto {
  return {
    id: 1,
    name: "Object 1",
    anchor: { r: 0, c: 4, dxMm: 0, dyMm: 0, widthMm: 160, heightMm: 90 },
    kind: "column",
    stacking: "none",
    lines: true,
    markers: false,
    smooth: false,
    hole: null,
    exploded: false,
    filled: false,
    stock: null,
    threeD: false,
    source: null,
    categories: "$Sheet1.$A$2:$A$4",
    series: [series("$Sheet1.$B$2:$B$4"), series("$Sheet1.$C$2:$C$4")],
    title: "Sales",
    subtitle: null,
    legend: "right",
    xAxis: { ...DEFAULT_AXIS, majorGrid: false },
    yAxis: { ...DEFAULT_AXIS },
    y2Axis: null,
    background: null,
    plotBackground: null,
    includeHidden: false,
    ownData: false,
    ...p,
  };
}

function data(p: Partial<ChartDataDto> = {}): ChartDataDto {
  return {
    chart: { sheet: 1, id: 1 },
    version: 3,
    categories: ["Jan", "Feb", "Mar"],
    series: [
      { name: "North", color: 0x004586ff, values: [10, 20, 30], x: null, sizes: null, labels: ["10", "20", "30"], numberFormat: "#,##0", trend: null, errors: null, mean: null },
      { name: "South", color: 0xff420eff, values: [5, null, 15], x: null, sizes: null, labels: ["5", "", "15"], numberFormat: null, trend: null, errors: null, mean: null },
    ],
    truncated: false,
    problems: [],
    ...p,
  };
}

const build = (c: Partial<ChartDto> = {}, d: Partial<ChartDataDto> = {}): EChartsOption => toEchartsOption(chart(c), data(d), LIGHT_CHART_THEME);
const seriesOf = (o: EChartsOption) => (Array.isArray(o.series) ? o.series : o.series ? [o.series] : []) as Record<string, unknown>[];
const axes = (o: EChartsOption) => ({ x: (Array.isArray(o.xAxis) ? o.xAxis[0] : o.xAxis) as Record<string, unknown> | undefined, y: (Array.isArray(o.yAxis) ? o.yAxis[0] : o.yAxis) as Record<string, unknown> | undefined });

describe("toEchartsOption snapshots", () => {
  const cases: [string, Partial<ChartDto>, Partial<ChartDataDto>?][] = [
    ["column", {}],
    ["column stacked", { stacking: "stacked" }],
    ["column percent", { stacking: "percent" }],
    ["bar", { kind: "bar" }],
    ["bar stacked", { kind: "bar", stacking: "stacked" }],
    ["line points and lines", { kind: "line", markers: true }],
    ["line points only", { kind: "line", lines: false, markers: true }],
    ["line smooth", { kind: "line", smooth: true }],
    ["line stacked", { kind: "line", stacking: "stacked" }],
    ["area", { kind: "area" }],
    ["area stacked", { kind: "area", stacking: "stacked" }],
    ["area percent", { kind: "area", stacking: "percent" }],
    ["pie", { kind: "pie", series: [series("$Sheet1.$B$2:$B$4")] }, { series: [data().series[0]!] }],
    ["pie exploded", { kind: "pie", exploded: true, series: [series("$Sheet1.$B$2:$B$4")] }, { series: [data().series[0]!] }],
    ["donut", { kind: "pie", hole: 0.5, series: [series("$Sheet1.$B$2:$B$4")] }, { series: [data().series[0]!] }],
    ["donut multi ring", { kind: "pie", hole: 0.4 }],
    ["scatter points", { kind: "scatter", lines: false, markers: true }, { series: data().series.map((s) => ({ ...s, x: [1, 2, 3] })) }],
    ["scatter lines", { kind: "scatter", lines: true, markers: true }, { series: data().series.map((s) => ({ ...s, x: [1, 2, 3] })) }],
    ["scatter smooth", { kind: "scatter", lines: true, smooth: true }, { series: data().series.map((s) => ({ ...s, x: [1, 2, 3] })) }],
    ["bubble", { kind: "bubble" }, { series: data().series.map((s) => ({ ...s, x: [1, 2, 3], sizes: [4, 9, 16] })) }],
    ["radar", { kind: "radar", markers: true }],
    ["radar filled", { kind: "radar", filled: true }],
    ["stock hlc", { kind: "stock", stock: "hlc", series: [series("l"), series("h"), series("c")] }, stockData(3)],
    ["stock ohlc", { kind: "stock", stock: "ohlc", series: [series("o"), series("l"), series("h"), series("c")] }, stockData(4)],
    ["stock volume hlc", { kind: "stock", stock: "volumeHlc", series: [series("v"), series("l"), series("h"), series("c")] }, stockData(4, true)],
    ["stock volume ohlc", { kind: "stock", stock: "volumeOhlc", series: [series("v"), series("o"), series("l"), series("h"), series("c")] }, stockData(5, true)],
    ["combo column + line on secondary axis", { series: [series("b"), series("c", { kind: "line", secondaryAxis: true })], y2Axis: { ...DEFAULT_AXIS, title: "Margin" } }],
    ["trend line with equation", { series: [series("b", { trend: { kind: "linear", showEquation: true, showR2: true, name: null } }), series("c")] }, { series: [{ ...data().series[0]!, trend: { points: [[1, 10], [3, 30]], equation: "f(x) = 10x + 0", r2: 1 } }, data().series[1]!] }],
    ["error bars and mean line", { series: [series("b", { errorBars: { kind: { type: "stdDev", k: 1 }, direction: "both" }, meanLine: true }), series("c")] }, { series: [{ ...data().series[0]!, errors: [[2, 2], [3, 3], [4, 4]], mean: 20 }, data().series[1]!] }],
    ["data labels", { series: [series("b", { labels: { value: true, percent: false, category: true, seriesName: false, position: "above" } }), series("c")] }],
    ["axes options", { xAxis: { ...DEFAULT_AXIS, title: "Month", reverse: true }, yAxis: { ...DEFAULT_AXIS, title: "Units", min: 0, max: 50, major: 10, minorCount: 2, minorGrid: true, log: false, numberFormat: "#,##0.0" } }],
    ["legend bottom, titles, backgrounds", { legend: "bottom", subtitle: "FY26", background: 0xf0f0f0ff, plotBackground: 0xffffffff }],
    ["no legend", { legend: "none", title: null }],
    ["empty data with problems", {}, { series: [], categories: [], problems: ["#REF!"] }],
  ];
  for (const [name, c, d] of cases) {
    it(name, () => {
      expect(build(c, d)).toMatchSnapshot();
    });
  }
});

function stockData(n: number, volume = false): Partial<ChartDataDto> {
  const mk = (name: string, values: number[]): ChartDataDto["series"][number] => ({ name, color: 0, values, x: null, sizes: null, labels: values.map(String), numberFormat: null, trend: null, errors: null, mean: null });
  const all = [mk("Volume", [100, 120, 90]), mk("Open", [10, 12, 11]), mk("Low", [9, 11, 10]), mk("High", [13, 14, 12]), mk("Close", [12, 11, 11.5])];
  const pick = volume ? (n === 4 ? [all[0]!, all[2]!, all[3]!, all[4]!] : all) : n === 3 ? [all[2]!, all[3]!, all[4]!] : all.slice(1);
  return { series: pick };
}

describe("toEchartsOption properties", () => {
  it("percent stacks sum to 100 per category", () => {
    const stacked = percentStack([
      [10, 20, null],
      [30, 20, 5],
    ]);
    expect(stacked[0]).toEqual([25, 50, null]);
    expect(stacked[1]).toEqual([75, 50, 100]);
    const o = build({ stacking: "percent" });
    const s = seriesOf(o);
    const sums = [0, 1, 2].map((i) => s.reduce((a, x) => a + (((x.data as (number | null)[])[i] ?? 0) as number), 0));
    expect(sums.map((v) => Math.round(v))).toEqual([100, 100, 100]);
    expect(s.every((x) => x.stack === "total")).toBe(true);
  });

  it("a bar chart swaps the axes; a column chart does not", () => {
    const col = axes(build());
    expect(col.x?.type).toBe("category");
    expect(col.y?.type).toBe("value");
    const bar = axes(build({ kind: "bar" }));
    expect(bar.x?.type).toBe("value");
    expect(bar.y?.type).toBe("category");
    expect(bar.y?.data).toEqual(["Jan", "Feb", "Mar"]);
  });

  it("uses the engine's colours, else the AOO palette in order", () => {
    expect(AOO_PALETTE[0]).toBe(0x004586);
    expect(paletteColor(1)).toBe("#ff420e");
    expect(paletteColor(12)).toBe(paletteColor(0));
    expect(cssColor(0x004586ff)).toBe("#004586");
    expect(cssColor(0xff000080)).toBe("rgba(255, 0, 0, 0.502)");
    const s = seriesOf(build({}, { series: data().series.map((x) => ({ ...x, color: 0 })) }));
    expect((s[0]?.itemStyle as { color: string }).color).toBe("#004586");
    expect((s[1]?.itemStyle as { color: string }).color).toBe("#ff420e");
  });

  it("points-only line charts become scatter on the category axis; areas get an areaStyle", () => {
    expect(seriesOf(build({ kind: "line", lines: false, markers: true }))[0]?.type).toBe("scatter");
    expect(seriesOf(build({ kind: "line" }))[0]?.type).toBe("line");
    expect(seriesOf(build({ kind: "area" }))[0]?.areaStyle).toBeTruthy();
  });

  it("donuts with several series become concentric rings", () => {
    const s = seriesOf(build({ kind: "pie", hole: 0.5 }));
    expect(s).toHaveLength(2);
    expect(s[0]?.radius).toEqual(["35%", "50.5%"]);
    expect(s[1]?.radius).toEqual(["52.5%", "68%"]);
    const exploded = seriesOf(build({ kind: "pie", exploded: true }));
    expect(exploded[0]?.selectedMode).toBe("multiple");
  });

  it("bubble sizes scale with the square root, capped at 60 px", () => {
    const s = seriesOf(build({ kind: "bubble" }, { series: [{ ...data().series[0]!, x: [1, 2, 3], sizes: [1, 4, 16] }] }));
    const f = s[0]?.symbolSize as (v: number[]) => number;
    expect(f([1, 1, 16])).toBe(60);
    expect(f([1, 1, 4])).toBe(30);
    expect(f([1, 1, 1])).toBe(15);
  });

  it("stock variants map to the AOO series order and candlestick [open, close, low, high]", () => {
    expect(stockRoles("hlc")).toEqual(["low", "high", "close"]);
    expect(stockRoles("volumeOhlc")).toEqual(["volume", "open", "low", "high", "close"]);
    const o = build({ kind: "stock", stock: "hlc", series: [series("l"), series("h"), series("c")] }, stockData(3));
    const candle = seriesOf(o).find((s) => s.type === "candlestick");
    expect((candle?.data as number[][])[0]).toEqual([12, 12, 9, 13]);
    const withVolume = seriesOf(build({ kind: "stock", stock: "volumeOhlc", series: [series("v"), series("o"), series("l"), series("h"), series("c")] }, stockData(5, true)));
    expect(withVolume[0]?.type).toBe("bar");
    expect(withVolume[0]?.yAxisIndex).toBe(1);
    expect((withVolume[1]?.data as number[][])[0]).toEqual([10, 12, 9, 13]);
  });

  it("combo overrides, trend lines and error bars add companion series", () => {
    const combo = seriesOf(build({ series: [series("b"), series("c", { kind: "line", secondaryAxis: true })] }));
    expect(combo.map((s) => s.type)).toEqual(["bar", "line"]);
    expect(combo[1]?.yAxisIndex).toBe(1);
    const trend = seriesOf(build({ series: [series("b", { trend: { kind: "linear", showEquation: true, showR2: false, name: "Fit" } }), series("c")] }, { series: [{ ...data().series[0]!, trend: { points: [[1, 10], [3, 30]], equation: "f(x) = 10x", r2: 1 } }, data().series[1]!] }));
    expect(trend.map((s) => s.name)).toEqual(["North", "Fit", "South"]);
    expect((trend[1]?.data as number[][])[0]).toEqual([0, 10]);
    const bars = seriesOf(build({ series: [series("b", { errorBars: { kind: { type: "percent", percent: 10 }, direction: "plus" } }), series("c")] }, { series: [{ ...data().series[0]!, errors: [[1, 1], [2, 2], [3, 3]] }, data().series[1]!] }));
    expect(bars[1]?.type).toBe("custom");
    expect((bars[1]?.data as number[][])[0]).toEqual([0, 10, 0, 1]);
  });

  it("formats axis labels with the number format and applies dark theme colours", () => {
    const o = build({ yAxis: { ...DEFAULT_AXIS, numberFormat: "#,##0.00" } });
    const fmt = (axes(o).y?.axisLabel as { formatter: (v: number) => string }).formatter;
    expect(fmt(1234.5)).toBe("1,234.50");
    const dark = toEchartsOption(chart(), data(), { ...LIGHT_CHART_THEME, dark: true, text: "#eee", textMuted: "#aaa", gridLine: "#333" });
    expect((dark.textStyle as { color: string }).color).toBe("#eee");
    expect(((Array.isArray(dark.yAxis) ? dark.yAxis[0] : dark.yAxis) as { splitLine: { lineStyle: { color: string } } }).splitLine.lineStyle.color).toBe("#333");
  });

  it("data labels join the requested parts", () => {
    const s = seriesOf(build({ series: [series("b", { labels: { value: true, percent: false, category: true, seriesName: true, position: "inside" } }), series("c")] }));
    const label = s[0]?.label as { show: boolean; position: string; formatter: (p: { dataIndex: number; seriesName: string }) => string };
    expect(label.show).toBe(true);
    expect(label.position).toBe("inside");
    expect(label.formatter({ dataIndex: 1, seriesName: "North" })).toBe("North Feb 20");
  });
});

describe("percent stacking per axis (review 4 M45)", () => {
  it("normalises primary and secondary series separately", () => {
    const out = percentStackByAxis([[50, 50], [50, 150], [1000, 10]], [0, 0, 1]);
    expect(out[0]).toEqual([50, 25]);
    expect(out[1]).toEqual([50, 75]);
    expect(out[2]).toEqual([100, 100]);
  });
});

describe("date axis and crossing (T18)", () => {
  const day = (y: number, m: number, d: number) => Date.UTC(y, m - 1, d) / DAY_MS;
  // 5, 6, 8 and 9 January 2026: the 7th is missing.
  const days = [day(2026, 1, 5), day(2026, 1, 6), day(2026, 1, 8), day(2026, 1, 9)];
  const dateData = (p: Partial<ChartDataDto> = {}): Partial<ChartDataDto> => ({
    categories: ["01/05/26", "01/06/26", "01/08/26", "01/09/26"],
    dateAxis: { days, resolution: "days", numberFormat: "MM/DD/YY" },
    series: [{ ...data().series[0]!, values: [10, 20, 30, 40], labels: ["10", "20", "30", "40"] }],
    ...p,
  });
  const line = { kind: "line" as const, series: [series("$Sheet1.$B$2:$B$5")] };

  it("snapshot: line over dates with a gap", () => {
    expect(build(line, dateData())).toMatchSnapshot();
  });

  it("places points by date, leaving the missing day empty", () => {
    const o = build(line, dateData());
    const x = axes(o).x as { type: string; min: number; max: number; axisLabel: { customValues: number[]; formatter: (v: number) => string } };
    expect(x.type).toBe("time");
    expect(seriesOf(o)[0]?.data).toEqual(days.map((d, i) => [d * DAY_MS, (i + 1) * 10]));
    expect(x.min).toBe(days[0]! * DAY_MS);
    expect(x.max).toBe(days[3]! * DAY_MS);
    // Daily labels in the category format, one per day including the gap.
    expect(x.axisLabel.customValues.map((v) => x.axisLabel.formatter(v))).toEqual(["01/05/26", "01/06/26", "01/07/26", "01/08/26", "01/09/26"]);
    expect(o.useUTC).toBe(true);
  });

  it("columns sit in the middle of their day and the axis runs one day further", () => {
    const o = build({ series: [series("b")] }, dateData());
    const x = axes(o).x as { max: number };
    expect(x.max).toBe((days[3]! + 1) * DAY_MS);
    expect((seriesOf(o)[0]?.data as number[][])[0]).toEqual([(days[0]! + 0.5) * DAY_MS, 10]);
  });

  it("text type keeps the category axis", () => {
    const o = build({ ...line, xAxis: { ...DEFAULT_AXIS, majorGrid: false, axisType: "text" } }, dateData());
    expect(axes(o).x?.type).toBe("category");
  });

  it("a fixed major interval of 2 months on a year of monthly dates", () => {
    const monthly = Array.from({ length: 12 }, (_, i) => day(2025, i + 1, 1));
    const c = chart({ ...line, xAxis: { ...DEFAULT_AXIS, majorTime: { number: 2, unit: "months" } } });
    const s = dateScale(c, { days: monthly, resolution: "months", numberFormat: "MMM YY" })!;
    expect(s.ticks.map((t) => s.label(t))).toEqual(["Jan 25", "Mar 25", "May 25", "Jul 25", "Sep 25", "Nov 25"]);
    expect(s.major).toEqual({ number: 2, unit: "months" });
  });

  it("automatic interval: weekly for two months of daily dates (AOO rounds 3-6 days to 7)", () => {
    const daily = Array.from({ length: 60 }, (_, i) => day(2026, 1, 1) + i);
    const s = dateScale(chart({ ...line, legend: "none" }), { days: daily, resolution: "days", numberFormat: "MM/DD" })!;
    // 160 mm leave room for 11 labels: 59 days / 11 = 5 days, raised to a week.
    expect(s.ticks.length).toBe(9);
    expect(s.major).toEqual({ number: 7, unit: "days" });
  });

  it("month arithmetic keeps the day, clamped (AOO DateHelper)", () => {
    expect(addUnits(day(2026, 1, 31), "months", 1)).toBe(day(2026, 2, 28));
    expect(addUnits(day(2024, 2, 29), "years", 1)).toBe(day(2025, 2, 28));
    expect(floorUnit(day(2026, 7, 19), "years")).toBe(day(2026, 1, 1));
  });

  const crossCases: [string, Partial<ChartDto>][] = [
    ["x crosses at start", { xAxis: { ...DEFAULT_AXIS, majorGrid: false, crosses: { type: "start" } } }],
    ["x crosses at end", { xAxis: { ...DEFAULT_AXIS, majorGrid: false, crosses: { type: "end" } } }],
    ["x crosses at value 50", { xAxis: { ...DEFAULT_AXIS, majorGrid: false, crosses: { type: "value", value: 50 } } }],
    ["y crosses at category 2, labels outside start", { yAxis: { ...DEFAULT_AXIS, crosses: { type: "value", value: 2 }, labelPosition: "outsideStart" } }],
    ["x labels near axis other side", { xAxis: { ...DEFAULT_AXIS, majorGrid: false, labelPosition: "nearAxisOtherSide" } }],
  ];
  for (const [name, c] of crossCases) {
    it(`snapshot: ${name}`, () => {
      expect({ option: build(c), fixes: axisCrossFixes(chart(c), data()) }).toMatchSnapshot();
    });
  }

  it("crossing at a value moves the axis after layout; start/end need no layout", () => {
    expect(axisCrossFixes(chart(), data())).toEqual([]);
    const start = build({ xAxis: { ...DEFAULT_AXIS, crosses: { type: "start" } } });
    expect(axes(start).x).toMatchObject({ position: "bottom", axisLine: { onZero: false } });
    const end = build({ xAxis: { ...DEFAULT_AXIS, crosses: { type: "end" } } });
    expect(axes(end).x).toMatchObject({ position: "top" });
    const fixes = axisCrossFixes(chart({ xAxis: { ...DEFAULT_AXIS, crosses: { type: "value", value: 50 } } }), data());
    expect(fixes).toEqual([{ axis: "xAxis", other: "yAxis", value: 50, mode: "move", position: "bottom", color: LIGHT_CHART_THEME.gridLine }]);
  });

  it("applyAxisCrossFixes offsets the axis to the crossing pixel", () => {
    const calls: Record<string, unknown>[] = [];
    const inst = {
      getModel: () => ({ getComponent: () => ({ coordinateSystem: { getRect: () => ({ x: 40, y: 20, width: 300, height: 200 }) } }) }),
      convertToPixel: (_f: Record<string, number>, v: number) => 220 - v * 2,
      setOption: (o: Record<string, unknown>) => void calls.push(o),
    };
    applyAxisCrossFixes(inst, [{ axis: "xAxis", other: "yAxis", value: 50, mode: "move", position: "bottom", color: "#000" }]);
    // Value 50 is at y = 120; the bottom edge is 220: offset -100.
    expect(calls[0]).toEqual({ xAxis: [{ offset: -100 }] });
  });
});

describe("axis editor helpers (review L4/L5)", () => {
  it("serialToIso: none, dates in years 1-9999, null outside (shown as a number)", () => {
    expect(serialToIso(null)).toBe("");
    expect(serialToIso(46027)).toBe("2026-01-05");
    expect(serialToIso(46027.75)).toBe("2026-01-05");
    expect(serialToIso(1)).toBe("1899-12-31");
    expect(serialToIso(-693_593)).toBe("0001-01-01");
    expect(serialToIso(-693_594)).toBeNull();
    expect(serialToIso(2_958_465)).toBe("9999-12-31");
    expect(serialToIso(2_958_466)).toBeNull();
    // Values that used to throw (RangeError) or give "+010113-09".
    for (const v of [1e9, 3e6, -1e9, Number.NaN, Number.POSITIVE_INFINITY]) expect(serialToIso(v)).toBeNull();
  });

  it("isoToSerial round trips and refuses junk", () => {
    for (const v of [-693_593, 1, 46027, 2_958_465]) expect(isoToSerial(serialToIso(v)!)).toBe(v);
    expect(isoToSerial("")).toBeNull();
    expect(isoToSerial("+010113-09-01")).toBeNull();
  });

  it("axisRangeInvalid: min at or above max", () => {
    expect(axisRangeInvalid({ min: null, max: 5 })).toBe(false);
    expect(axisRangeInvalid({ min: 1, max: 5 })).toBe(false);
    expect(axisRangeInvalid({ min: 5, max: 5 })).toBe(true);
    expect(axisRangeInvalid({ min: 6, max: 5 })).toBe(true);
  });
});
