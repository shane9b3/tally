/**
 * Print Preview state (W5-5): the prepared job, its layout, the page shown,
 * zoom and the rendered page/thumbnail PNGs. Driven by `PrintFeatures`.
 */
import type { PrintLayoutDto, PrintOptionsDto } from "@tally/bridge";
import { create } from "zustand";
import { clampPage, clampPreviewZoom } from "./printReducer";

export type PrintStatus = "idle" | "loading" | "ready" | "error";

export interface PrintState {
  open: boolean;
  status: PrintStatus;
  /** The job handle (`prepare_print_job`), or `null` before it exists. */
  job: number | null;
  /** The document version the job was prepared at. */
  jobVersion: number;
  layout: PrintLayoutDto | null;
  options: PrintOptionsDto;
  page: number;
  zoom: number;
  /** Page PNG data URLs by `index:dpi`. */
  pages: Record<string, string>;
  /** Thumbnail data URLs by page index. */
  thumbs: Record<number, string>;
  error: string | null;
  setOpen(open: boolean): void;
  setStatus(status: PrintStatus, error?: string | null): void;
  setJob(job: number | null, layout: PrintLayoutDto | null, version: number): void;
  setOptions(o: PrintOptionsDto): void;
  setPage(page: number): void;
  setZoom(z: number): void;
  setPagePng(key: string, url: string): void;
  setThumb(index: number, url: string): void;
  reset(): void;
}

export const DEFAULT_PRINT_OPTIONS: PrintOptionsDto = { sheets: { type: "active" }, selection: null, suppressEmptyPages: true, pageFrom: null, pageTo: null };

const initial = { open: false, status: "idle" as PrintStatus, job: null, jobVersion: -1, layout: null, options: DEFAULT_PRINT_OPTIONS, page: 0, zoom: 1, pages: {}, thumbs: {}, error: null };

export const usePrintStore = create<PrintState>()((set) => ({
  ...initial,
  setOpen: (open) => set(open ? { open } : { ...initial, open: false }),
  setStatus: (status, error = null) => set({ status, error }),
  setJob: (job, layout, jobVersion) => set((s) => ({ job, layout, jobVersion, pages: {}, thumbs: {}, page: clampPage(s.page, layout?.pages.length ?? 0) })),
  setOptions: (options) => set({ options }),
  setPage: (page) => set((s) => ({ page: clampPage(page, s.layout?.pages.length ?? 0) })),
  setZoom: (zoom) => set({ zoom: clampPreviewZoom(zoom) }),
  setPagePng: (key, url) => set((s) => ({ pages: { ...s.pages, [key]: url } })),
  setThumb: (index, url) => set((s) => ({ thumbs: { ...s.thumbs, [index]: url } })),
  reset: () => set({ ...initial }),
}));
