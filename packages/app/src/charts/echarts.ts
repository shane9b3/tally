/**
 * Tree-shaken ECharts registration (W5-5). Everything the chart layer, the
 * editor and the print pipeline need, and nothing else: the seven series
 * types Tally renders (bar covers column; custom draws error bars), the
 * components the option builder emits, and both renderers (canvas on the
 * grid, SVG for `chart_svg` / the print job).
 *
 * Import `echarts` from here; never from the `echarts` root (that pulls in
 * every chart and map). `echarts` is pinned to 6.1.0 in package.json.
 */
import { BarChart, CandlestickChart, CustomChart, LineChart, PieChart, RadarChart, ScatterChart } from "echarts/charts";
import { DatasetComponent, GraphicComponent, GridComponent, LegendComponent, MarkLineComponent, RadarComponent, TitleComponent, TooltipComponent } from "echarts/components";
import * as core from "echarts/core";
import { CanvasRenderer, SVGRenderer } from "echarts/renderers";

core.use([
  BarChart,
  LineChart,
  PieChart,
  ScatterChart,
  RadarChart,
  CandlestickChart,
  CustomChart,
  TitleComponent,
  LegendComponent,
  GridComponent,
  TooltipComponent,
  MarkLineComponent,
  RadarComponent,
  DatasetComponent,
  GraphicComponent,
  CanvasRenderer,
  SVGRenderer,
]);

export const echarts = core;
export type ECharts = core.ECharts;
export type { EChartsOption } from "./toEcharts";
