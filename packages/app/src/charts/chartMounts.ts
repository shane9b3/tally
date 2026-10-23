/**
 * Live ECharts instances on the grid's object layer (W5-5): one per chart
 * that intersects the viewport, created in `grid.objectLayer.elementFor(id)`
 * when it scrolls in and disposed when it leaves (at most
 * {@link MAX_LIVE_CHARTS}). Pictures get an `<img>` in the same way.
 *
 * `sync()` is idempotent and cheap when nothing changed: the option is only
 * re-applied when the chart DTO, its data or the theme object changed, and
 * `resize()` only runs when the node's size changed (zoom, drag-resize).
 */
import type { ChartDataDto, ChartDto, ImageDto } from "@tally/bridge";
import type { Grid, GridGeometryInfo } from "@tally/grid";
import { echarts, type ECharts } from "./echarts";
import { objectId } from "./chartStore";
import { nodeVisible } from "./nodeVisible";
import { toEchartsOption, type ChartTheme } from "./toEcharts";

/** Live instances kept at once. */
export const MAX_LIVE_CHARTS = 20;

export { nodeVisible };

/** Disposes a chart without losing the grid's own children (selection handles) in its container, which ECharts empties. */
function disposeKeeping(chart: ECharts): void {
  const dom = chart.getDom();
  const kept = [...dom.childNodes].filter((n) => n instanceof HTMLElement && n.dataset.handle !== undefined);
  chart.dispose();
  for (const k of kept) if (!k.parentNode) dom.appendChild(k);
}

interface Mount {
  chart: ECharts;
  dto: ChartDto;
  data: ChartDataDto | undefined;
  theme: ChartTheme;
  w: number;
  h: number;
}

declare global {
  interface Window {
    /** e2e hook (dev builds): live ECharts instances by chart id. */
    __tallyCharts?: Record<number, ECharts>;
  }
}

export class ChartMounts {
  private readonly mounts = new Map<number, Mount>();
  private readonly images = new Map<number, HTMLImageElement>();
  private ro: ResizeObserver | null = null;

  /** `exposeHooks`: register instances on `window.__tallyCharts` (the primary pane only). */
  constructor(private readonly exposeHooks = true) {}

  /** Places/updates every visible chart and picture; disposes what left. */
  sync(grid: Grid, charts: readonly ChartDto[], data: Readonly<Record<number, ChartDataDto>>, images: readonly ImageDto[], imageData: Readonly<Record<number, string>>, theme: ChartTheme): void {
    const geo = grid.getGeometry();
    const layer = grid.objectLayer;
    const wanted = new Set<number>();
    for (const dto of charts) {
      const id = objectId("chart", dto.id);
      const node = layer.elementFor(id);
      const rect = grid.objectRect(id);
      if (!node || !rect || !nodeVisible(rect, geo)) continue;
      if (!this.mounts.has(dto.id) && this.mounts.size >= MAX_LIVE_CHARTS) continue;
      wanted.add(dto.id);
      this.mountChart(node, dto, data[dto.id], theme, rect.w, rect.h);
    }
    for (const [id, m] of this.mounts) {
      if (!wanted.has(id)) {
        disposeKeeping(m.chart);
        this.mounts.delete(id);
        if (this.exposeHooks) expose(id, null);
      }
    }
    const wantedImages = new Set<number>();
    for (const im of images) {
      const id = objectId("image", im.id);
      const node = layer.elementFor(id);
      if (!node) continue;
      wantedImages.add(im.id);
      let img = this.images.get(im.id);
      if (!img) {
        img = node.ownerDocument.createElement("img");
        img.alt = im.name || "Picture";
        img.draggable = false;
        Object.assign(img.style, { width: "100%", height: "100%", objectFit: "fill", display: "block", pointerEvents: "none", userSelect: "none" });
        node.appendChild(img);
        this.images.set(im.id, img);
      }
      const url = imageData[im.id] ?? (im.data ? `data:${im.mime};base64,${im.data}` : "");
      if (url !== "" && img.src !== url) img.src = url;
      node.dataset.placeholder = url === "" ? "true" : "false";
      if (url === "") node.title = im.name ? `${im.name} (picture data not loaded)` : "Picture";
    }
    for (const [id, img] of this.images) {
      if (!wantedImages.has(id)) {
        img.remove();
        this.images.delete(id);
      }
    }
  }

  /** The live instance of a chart, if mounted. */
  instance(id: number): ECharts | undefined {
    return this.mounts.get(id)?.chart;
  }

  /** Re-applies the option of one chart (after a theme change or data refresh). */
  private mountChart(node: HTMLDivElement, dto: ChartDto, data: ChartDataDto | undefined, theme: ChartTheme, w: number, h: number): void {
    let m = this.mounts.get(dto.id);
    if (!m) {
      // ECharts clears its container; keep what the grid put there (selection handles can exist before this lazy layer loads).
      const kept = [...node.childNodes];
      const chart = echarts.init(node, undefined, { renderer: "canvas", width: Math.max(1, w), height: Math.max(1, h) });
      for (const k of kept) if (!k.parentNode) node.appendChild(k);
      m = { chart, dto, data, theme, w, h };
      this.mounts.set(dto.id, m);
      m.chart.setOption(toEchartsOption(dto, data ?? emptyData(dto), theme), true);
      this.observe(node);
      if (this.exposeHooks) expose(dto.id, chart);
      return;
    }
    if (Math.abs(m.w - w) >= 0.5 || Math.abs(m.h - h) >= 0.5) {
      m.w = w;
      m.h = h;
      m.chart.resize({ width: Math.max(1, w), height: Math.max(1, h) });
    }
    if (m.dto !== dto || m.data !== data || m.theme !== theme) {
      m.dto = dto;
      m.data = data;
      m.theme = theme;
      m.chart.setOption(toEchartsOption(dto, data ?? emptyData(dto), theme), true);
    }
  }

  private observe(node: HTMLElement): void {
    if (typeof ResizeObserver === "undefined") return;
    this.ro ??= new ResizeObserver((entries) => {
      for (const e of entries) {
        const el = e.target as HTMLElement;
        const id = /^chart:(\d+)$/.exec(el.dataset.objectId ?? "")?.[1];
        const m = id ? this.mounts.get(Number(id)) : undefined;
        if (!m) continue;
        const w = el.clientWidth;
        const h = el.clientHeight;
        if (w > 0 && h > 0 && (Math.abs(m.w - w) >= 0.5 || Math.abs(m.h - h) >= 0.5)) {
          m.w = w;
          m.h = h;
          m.chart.resize({ width: w, height: h });
        }
      }
    });
    this.ro.observe(node);
  }

  /** Disposes every instance and picture (e.g. the second pane after the split is removed). */
  clear(): void {
    for (const [id, m] of this.mounts) {
      disposeKeeping(m.chart);
      if (this.exposeHooks) expose(id, null);
    }
    this.mounts.clear();
    for (const img of this.images.values()) img.remove();
    this.images.clear();
  }

  dispose(): void {
    this.clear();
    this.ro?.disconnect();
    this.ro = null;
    if (this.exposeHooks) expose(null, null);
  }
}

/** e2e hook (dev builds only; compiled out of the desktop bundle): the live instance of a chart, `null` to drop it, or everything when `id` is null. */
function expose(id: number | null, inst: ECharts | null): void {
  if (import.meta.env.DEV) {
    if (id === null) {
      window.__tallyCharts = {};
      return;
    }
    window.__tallyCharts ??= {};
    if (inst) window.__tallyCharts[id] = inst;
    else delete window.__tallyCharts[id];
  }
}

/** Placeholder data while `chart_data` is in flight (or unsupported). */
export function emptyData(dto: ChartDto): ChartDataDto {
  return { chart: { sheet: 0, id: dto.id }, version: 0, categories: [], series: [], truncated: false, problems: [] };
}
