/**
 * Chart images without a DOM (W5-5): `renderChartSvg` draws a chart with
 * ECharts' SVG renderer in SSR mode and returns the SVG document, used for
 * the print job (`ChartSvgDto`) and "Copy as image"; `chartPngDataUrl`
 * rasterizes a live instance for the clipboard.
 */
import type { ChartDataDto, ChartDto, ChartSvgDto } from "@tally/bridge";
import { PX_PER_MM } from "@tally/grid";
import { echarts, type ECharts } from "./echarts";
import { applyAxisCrossFixes } from "./chartAxes";
import { axisCrossFixes, LIGHT_CHART_THEME, toEchartsOption, type ChartTheme } from "./toEcharts";

/** Largest SVG the engine accepts (`limits::MAX_SVG_BYTES`). */
export const MAX_SVG_BYTES = 4 * 1024 * 1024;

/** Pixel size of a chart anchor at 96 dpi. */
export function chartPixelSize(chart: ChartDto): { width: number; height: number } {
  return { width: Math.max(16, Math.round(chart.anchor.widthMm * PX_PER_MM)), height: Math.max(16, Math.round(chart.anchor.heightMm * PX_PER_MM)) };
}

/** The SVG document of a chart at its anchor size (print uses the light theme). */
export function renderChartSvg(chart: ChartDto, data: ChartDataDto, theme: ChartTheme = LIGHT_CHART_THEME, size = chartPixelSize(chart)): string {
  const inst = echarts.init(null, undefined, { renderer: "svg", ssr: true, width: size.width, height: size.height });
  try {
    inst.setOption(toEchartsOption(chart, data, theme), true);
    applyAxisCrossFixes(inst, axisCrossFixes(chart, data, theme));
    return inst.renderToSVGString();
  } finally {
    inst.dispose();
  }
}

/** The `ChartSvgDto`s of a sheet's charts for `prepare_print_job` (oversized SVGs are skipped). */
export function chartSvgsFor(sheet: number, charts: readonly ChartDto[], data: Readonly<Record<number, ChartDataDto>>): ChartSvgDto[] {
  const out: ChartSvgDto[] = [];
  for (const chart of charts) {
    const d = data[chart.id];
    if (!d) continue;
    const svg = renderChartSvg(chart, d);
    if (new TextEncoder().encode(svg).length > MAX_SVG_BYTES) continue;
    out.push({ sheet, id: chart.id, svg });
  }
  return out;
}

/** A PNG data URL of a live chart (2× for crisp clipboard images). */
export function chartPngDataUrl(inst: ECharts, background = "#ffffff"): string {
  return inst.getDataURL({ type: "png", pixelRatio: 2, backgroundColor: background });
}

/** Writes a PNG data URL to the system clipboard (`image/png`); false when not possible here. */
export async function copyPngToClipboard(dataUrl: string): Promise<boolean> {
  const clip = typeof navigator === "undefined" ? undefined : navigator.clipboard;
  if (!clip || typeof ClipboardItem === "undefined" || typeof clip.write !== "function") return false;
  try {
    const blob = await (await fetch(dataUrl)).blob();
    await clip.write([new ClipboardItem({ "image/png": blob })]);
    return true;
  } catch {
    return false;
  }
}
