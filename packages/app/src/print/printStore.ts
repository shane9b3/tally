/**
 * Print Preview state (W5-5): the prepared job, its layout, the page shown,
 * zoom and the rendered page/thumbnail PNGs. Driven by `PrintFeatures`.
 */
import type { PrintLayoutDto, PrintOptionsDto } from "@tally/bridge";
import { create } from "zustand";
import { clampPage, clampPreviewZoom, DEFAULT_PRINT_SCOPE, type PrintScope } from "./printReducer";

export type PrintStatus = "idle" | "loading" | "ready" | "error";

export interface PrintState {
  open: boolean;
  status: PrintStatus;
  /** The job handle (`prepare_print_job`), or `null` before it exists. */
  job: number | null;
  /** The document version the job was prepared at. */
  jobVersion: number;
  /** The document and options the job was prepared with (JSON), so another document or range re-prepares. */
  jobKey: string;
  layout: PrintLayoutDto | null;
  options: PrintOptionsDto;
  page: number;
  zoom: number;
  /** Page PNG data URLs by `index:dpi`. */
  pages: Record<string, string>;
  /** Thumbnail data URLs by page index. */
  thumbs: Record<number, string>;
  error: string | null;
  /** The print range chosen this session (kept when the preview closes). */
  scope: PrintScope;
  /** The page list ("1-3,5"), blank for every page. */
  pageSpec: string;
  /** The document the range was chosen for (another document starts at the default). */
  scopeDoc: number | null;
  /** The range step of File ▸ Print… / Export as PDF…, or `null`. */
  rangeDialog: "print" | "pdf" | null;
  setOpen(open: boolean): void;
  setStatus(status: PrintStatus, error?: string | null): void;
  setJob(job: number | null, layout: PrintLayoutDto | null, version: number, key?: string): void;
  setRange(doc: number | null, scope: PrintScope, pageSpec: string): void;
  setRangeDialog(d: "print" | "pdf" | null): void;
  setOptions(o: PrintOptionsDto): void;
  setPage(page: number): void;
  setZoom(z: number): void;
  setPagePng(key: string, url: string): void;
  setThumb(index: number, url: string): void;
  reset(): void;
}

export const DEFAULT_PRINT_OPTIONS: PrintOptionsDto = { sheets: { type: "active" }, selection: null, suppressEmptyPages: true, pageFrom: null, pageTo: null };

const initial = { open: false, status: "idle" as PrintStatus, job: null, jobVersion: -1, jobKey: "", layout: null, options: DEFAULT_PRINT_OPTIONS, page: 0, zoom: 1, pages: {}, thumbs: {}, error: null };
const initialRange = { scope: DEFAULT_PRINT_SCOPE, pageSpec: "", scopeDoc: null, rangeDialog: null };

export const usePrintStore = create<PrintState>()((set) => ({
  ...initial,
  ...initialRange,
  // Closing drops the job but keeps the chosen range for this session.
  setOpen: (open) => set(open ? { open } : { ...initial, open: false }),
  setStatus: (status, error = null) => set({ status, error }),
  setJob: (job, layout, jobVersion, jobKey = "") => set((s) => ({ job, layout, jobVersion, jobKey, pages: {}, thumbs: {}, page: clampPage(s.page, layout?.pages.length ?? 0) })),
  setRange: (scopeDoc, scope, pageSpec) => set({ scopeDoc, scope, pageSpec }),
  setRangeDialog: (rangeDialog) => set({ rangeDialog }),
  setOptions: (options) => set({ options }),
  setPage: (page) => set((s) => ({ page: clampPage(page, s.layout?.pages.length ?? 0) })),
  setZoom: (zoom) => set({ zoom: clampPreviewZoom(zoom) }),
  setPagePng: (key, url) => set((s) => ({ pages: { ...s.pages, [key]: url } })),
  setThumb: (index, url) => set((s) => ({ thumbs: { ...s.thumbs, [index]: url } })),
  reset: () => set({ ...initial, ...initialRange }),
}));
