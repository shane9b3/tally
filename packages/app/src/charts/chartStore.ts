/**
 * Objects of the active sheet (W5-5): the charts and pictures the grid's
 * object layer places, each chart's resolved data, the selected object and
 * the Chart Editor panel state. Refreshed by `ChartFeatures` (charts.ts)
 * from `charts()` / `images()` / `chart_data()` on sheet changes and on
 * `ApplyResult.objectsChanged` / `chartsChanged`.
 */
import type { ChartDataDto, ChartDto, ImageDto } from "@tally/bridge";
import type { GridObject } from "@tally/grid";
import { create } from "zustand";

/** Object ids on the grid layer: `chart:<id>` and `image:<id>`. */
export type ObjectKind = "chart" | "image";

export function objectId(kind: ObjectKind, id: number): string {
  return `${kind}:${id}`;
}

/** Parses a layer object id. */
export function parseObjectId(id: string): { kind: ObjectKind; id: number } | null {
  const m = /^(chart|image):(\d+)$/.exec(id);
  return m ? { kind: m[1] as ObjectKind, id: Number(m[2]) } : null;
}

export interface ChartsState {
  /** The sheet the lists belong to. */
  sheet: number;
  charts: ChartDto[];
  images: ImageDto[];
  /** Resolved data by chart id. */
  data: Record<number, ChartDataDto>;
  /** Picture bytes as data URLs by image id (pictures inserted this session). */
  imageData: Record<number, string>;
  /** The selected object (layer id), or `null`. */
  selected: string | null;
  /** The Chart Editor side panel. */
  editorOpen: boolean;
  /** A copied chart (⌘C on a selected chart). */
  clipboard: ChartDto | null;
  /** The chart whose Data Table dialog is open (View ▸ Data Table), or `null`. */
  dataTable: number | null;
  /** Bumped on scroll/zoom so the chart layer re-culls (throttled per frame). */
  viewportTick: number;
  setObjects(sheet: number, charts: ChartDto[], images: ImageDto[]): void;
  setData(id: number, data: ChartDataDto): void;
  setImageData(id: number, url: string): void;
  select(id: string | null): void;
  setEditorOpen(open: boolean): void;
  setClipboard(chart: ChartDto | null): void;
  setDataTable(id: number | null): void;
  bumpViewport(): void;
  reset(): void;
}

const initial = { sheet: 0, charts: [] as ChartDto[], images: [] as ImageDto[], data: {} as Record<number, ChartDataDto>, imageData: {} as Record<number, string>, selected: null, editorOpen: false, viewportTick: 0, dataTable: null as number | null };

export const useChartStore = create<ChartsState>()((set) => ({
  ...initial,
  clipboard: null,
  setObjects: (sheet, charts, images) =>
    set((s) => {
      const ids = new Set(charts.map((c) => c.id));
      const data: Record<number, ChartDataDto> = {};
      if (s.sheet === sheet) for (const [k, v] of Object.entries(s.data)) if (ids.has(Number(k))) data[Number(k)] = v;
      const keep = s.selected !== null && objectsFor(charts, images).some((o) => o.id === s.selected);
      return { sheet, charts, images, data, selected: keep ? s.selected : null, editorOpen: keep ? s.editorOpen : false };
    }),
  setData: (id, data) => set((s) => ({ data: { ...s.data, [id]: data } })),
  setImageData: (id, url) => set((s) => ({ imageData: { ...s.imageData, [id]: url } })),
  select: (selected) => set((s) => ({ selected, editorOpen: selected === null ? false : s.editorOpen })),
  setEditorOpen: (editorOpen) => set({ editorOpen }),
  setClipboard: (clipboard) => set({ clipboard }),
  setDataTable: (dataTable) => set({ dataTable }),
  bumpViewport: () => set((s) => ({ viewportTick: s.viewportTick + 1 })),
  reset: () => set({ ...initial }),
}));

/** The grid-layer objects for a chart and picture list (charts first, in id order). */
export function objectsFor(charts: readonly ChartDto[], images: readonly ImageDto[]): GridObject[] {
  const out: GridObject[] = charts.map((c) => ({ id: objectId("chart", c.id), kind: "chart", anchor: { ...c.anchor }, label: c.title ?? `Chart ${c.id}` }));
  for (const im of images) out.push({ id: objectId("image", im.id), kind: "image", anchor: { ...im.anchor }, label: im.name || `Picture ${im.id}` });
  return out;
}

/** The selected chart, if a chart is selected. */
export function selectedChart(s: Pick<ChartsState, "selected" | "charts">): ChartDto | null {
  const p = s.selected === null ? null : parseObjectId(s.selected);
  if (!p || p.kind !== "chart") return null;
  return s.charts.find((c) => c.id === p.id) ?? null;
}
