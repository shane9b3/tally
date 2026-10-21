/**
 * Chart commands (W5-5): Insert ▸ Chart (`suggest_chart` → `addChart`), the
 * editor's `updateChart` transactions, copy/paste of a chart inside the
 * document, "Copy as image" and the SVG export used by the print job. The
 * object layer's events (selection, drag, delete) are in
 * `objects/objectFeatures.ts`.
 */
import type { ApplyResult, ChartDataDto, ChartDto, ChartKind, ChartSvgDto, RangeDto } from "@tally/bridge";
import type { AppController } from "../controller";
import { absoluteRef } from "../dataOps";
import { useUiStore } from "../stores/uiStore";
import type { ObjectFeatures } from "../objects/objectFeatures";
import { chartPngDataUrl, chartSvgsFor, copyPngToClipboard, renderChartSvg } from "./chartSvgs";
import { objectId, selectedChart, useChartStore } from "./chartStore";
import type { ChartMounts } from "./chartMounts";
import { DEFAULT_AXIS } from "./toEcharts";
import { clientV4 } from "../objects/shell";

/** AOO's default chart size. */
export const DEFAULT_CHART_MM = { width: 160, height: 90 } as const;

/** A column chart over `range` (the fallback when `suggest_chart` is unavailable). */
export function defaultChart(sheetName: string, range: RangeDto, kind: ChartKind = "column"): ChartDto {
  const anchor = { r: range.r0, c: range.c1 + 1, dxMm: 3, dyMm: 0, widthMm: DEFAULT_CHART_MM.width, heightMm: DEFAULT_CHART_MM.height };
  return {
    id: 0,
    name: "",
    anchor,
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
    source: { range: absoluteRef(sheetName, range), seriesInRows: range.r1 - range.r0 < range.c1 - range.c0, firstRowLabel: true, firstColLabel: true },
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

export class ChartFeatures {
  /** The live instances (set by the chart layer when it mounts). */
  mounts: ChartMounts | null = null;

  constructor(
    private readonly ctl: AppController,
    private readonly objects: ObjectFeatures,
  ) {}

  private toast(kind: "info" | "error", message: string): void {
    useUiStore.getState().pushToast(kind, message);
  }

  /** The selected chart, or `null`. */
  get selected(): ChartDto | null {
    return selectedChart(useChartStore.getState());
  }

  /**
   * Toolbar Chart / Insert ▸ Chart: the engine's proposal for the data
   * region around the selection (a column chart), else a default column
   * chart over the selection; adds it, selects it and opens the editor.
   */
  async insertChart(kind: ChartKind = "column"): Promise<void> {
    try {
      const doc = this.ctl.requireDoc();
      const sheet = this.ctl.sheet;
      const sel = { ...this.ctl.selection.range };
      let chart: ChartDto;
      try {
        chart = await clientV4(this.ctl).suggestChart(doc, sheet, sel, kind);
      } catch {
        const region = (await this.ctl.dataRegion().catch(() => null)) ?? sel;
        chart = defaultChart(this.ctl.sheetName(), region, kind);
      }
      await this.addChart(chart, "Insert chart");
    } catch (e) {
      this.ctl.notifyError(e);
    }
  }

  /** Adds a chart DTO (id/name ignored by the engine), then selects it and opens the editor. */
  async addChart(chart: ChartDto, label = "Insert chart", openEditor = true): Promise<number | null> {
    const res = await this.ctl.apply(label, [{ type: "addChart", sheet: this.ctl.sheet, chart }]);
    if (!res || res.errors.length > 0) return null;
    const id = res.created?.[0]?.id;
    await this.objects.refreshObjects();
    if (id === undefined) return null;
    const oid = objectId("chart", id);
    useChartStore.getState().select(oid);
    if (openEditor) useChartStore.getState().setEditorOpen(true);
    // The layer node exists after the objects prop reaches the grid.
    requestAnimationFrame(() => this.ctl.grid?.objectLayer.select(oid, false));
    return id;
  }

  /** One "Edit chart" transaction (the editor panel). */
  async updateChart(chart: ChartDto): Promise<ApplyResult | null> {
    const res = await this.ctl.apply("Edit chart", [{ type: "updateChart", sheet: this.ctl.sheet, id: chart.id, chart }]);
    if (res) useChartStore.getState().select(objectId("chart", chart.id));
    return res;
  }

  /** Format ▸ Chart ▸ Edit Chart… (needs a selected chart). */
  openEditor(): void {
    if (!this.selected) {
      this.toast("info", "Select a chart first (click it).");
      return;
    }
    useChartStore.getState().setEditorOpen(true);
  }

  closeEditor(): void {
    useChartStore.getState().setEditorOpen(false);
    this.ctl.focusGrid();
  }

  /** chart.delete. */
  async deleteSelected(): Promise<void> {
    const c = this.selected;
    if (!c) {
      this.toast("info", "Select a chart first (click it).");
      return;
    }
    await this.objects.deleteObject(objectId("chart", c.id));
  }

  /** ⌘C on a selected chart: keeps the DTO for paste and puts a PNG on the system clipboard. True when a chart was copied. */
  async copySelected(): Promise<boolean> {
    const c = this.selected;
    if (!c) return false;
    useChartStore.getState().setClipboard(structuredClone(c));
    await this.copyAsImage(false);
    return true;
  }

  /** ⌘X on a selected chart. */
  async cutSelected(): Promise<boolean> {
    const c = this.selected;
    if (!c) return false;
    await this.copySelected();
    await this.objects.deleteObject(objectId("chart", c.id));
    return true;
  }

  /** ⌘V with a copied chart: adds a copy anchored at the active cell. True when pasted. */
  async pasteChart(): Promise<boolean> {
    const clip = useChartStore.getState().clipboard;
    if (!clip) return false;
    const a = this.ctl.selection.active;
    const copy: ChartDto = { ...structuredClone(clip), id: 0, anchor: { ...clip.anchor, r: a.r, c: a.c, dxMm: 2, dyMm: 2 } };
    await this.addChart(copy, "Paste chart", false);
    return true;
  }

  /** "Copy as image": the selected chart as PNG on the system clipboard. */
  async copyAsImage(announce = true): Promise<void> {
    const c = this.selected;
    if (!c) return;
    const inst = this.mounts?.instance(c.id);
    let ok = false;
    if (inst) ok = await copyPngToClipboard(chartPngDataUrl(inst));
    if (!ok && announce) {
      const data = useChartStore.getState().data[c.id];
      if (data && typeof navigator !== "undefined" && navigator.clipboard?.writeText) {
        await navigator.clipboard.writeText(renderChartSvg(c, data)).catch(() => undefined);
        this.toast("info", "Copied the chart as SVG text (image clipboard not available here).");
        return;
      }
      this.toast("info", "The clipboard is not available here.");
    } else if (announce) this.toast("info", "Copied the chart as an image.");
  }

  /** The SVG of one chart of the active sheet (empty when its data is unknown). */
  chartSvg(id: number): string | null {
    const st = useChartStore.getState();
    const chart = st.charts.find((c) => c.id === id);
    const data = st.data[id];
    return chart && data ? renderChartSvg(chart, data) : null;
  }

  /** The `ChartSvgDto`s of the active sheet (for `prepare_print_job`). */
  chartSvgsOfActiveSheet(): ChartSvgDto[] {
    const st = useChartStore.getState();
    return chartSvgsFor(st.sheet, st.charts, st.data);
  }

  /** The `ChartSvgDto`s of any sheet (fetched; used when printing every sheet). */
  async chartSvgsOf(sheet: number): Promise<ChartSvgDto[]> {
    const doc = this.ctl.requireDoc();
    const st = useChartStore.getState();
    if (sheet === st.sheet) return this.chartSvgsOfActiveSheet();
    const charts = await clientV4(this.ctl).charts(doc, sheet).catch((): ChartDto[] => []);
    const data: Record<number, ChartDataDto> = {};
    await Promise.all(
      charts.map(async (c) => {
        try {
          data[c.id] = await clientV4(this.ctl).chartData(doc, sheet, c.id);
        } catch {
          // Skipped.
        }
      }),
    );
    return chartSvgsFor(sheet, charts, data);
  }

  /** The editor's "pick" button: range-pick mode on the grid; resolves to reference text or `null`. */
  async pickRange(): Promise<string | null> {
    const grid = this.ctl.grid;
    if (!grid) return null;
    this.toast("info", "Select the range on the sheet (Esc cancels).");
    const range = await grid.pickRange();
    this.ctl.focusGrid();
    return range ? absoluteRef(this.ctl.sheetName(), range) : null;
  }
}
