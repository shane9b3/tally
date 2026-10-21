/**
 * Printing (W5-5, over the W5-3 engine pipeline): the Print Preview
 * (`print_layout` → `prepare_print_job` with the charts' SVGs →
 * `print_job_page_png`), Export PDF… (`wb_export_pdf`) and Print (⌘P,
 * `wb_print`). Until W5-3 lands every engine call answers `unsupported`,
 * which is shown as a toast.
 */
import { CANCELLED, isApiError, type ChartSvgDto, type PrintOptionsDto } from "@tally/bridge";
import type { AppController } from "../controller";
import type { ChartFeatures } from "../charts/chartFeatures";
import { useDocStore } from "../stores/docStore";
import { useUiStore } from "../stores/uiStore";
import { dpiFor, stepPage, stepPreviewZoom, THUMB_DPI, thumbWindow } from "./printReducer";
import { usePrintStore } from "./printStore";
import { clientV4 } from "../objects/shell";

export const PRINT_SOON = "Printing and PDF export are coming with the print pipeline (W5-3).";

export class PrintFeatures {
  private pending = new Set<string>();

  constructor(
    private readonly ctl: AppController,
    private readonly charts: ChartFeatures,
  ) {}

  private toast(kind: "info" | "error", message: string): void {
    useUiStore.getState().pushToast(kind, message);
  }

  /** Reports an engine/shell failure: `unsupported` as the W5-3 notice, `cancelled` silently. */
  private report(e: unknown): void {
    if (isApiError(e) && e.code === CANCELLED) return;
    if (isApiError(e) && e.code === "unsupported") {
      this.toast("info", PRINT_SOON);
      return;
    }
    this.ctl.notifyError(e);
  }

  // -- preview ------------------------------------------------------------------

  /** File ▸ Print Preview. */
  async openPreview(options: PrintOptionsDto = usePrintStore.getState().options): Promise<void> {
    const st = usePrintStore.getState();
    st.setOpen(true);
    st.setOptions(options);
    await this.prepare(options);
  }

  closePreview(): void {
    usePrintStore.getState().setOpen(false);
    this.ctl.focusGrid();
  }

  /** Builds (or rebuilds) the job for the current document version; renders the current page. */
  async prepare(options: PrintOptionsDto = usePrintStore.getState().options): Promise<number | null> {
    const st = usePrintStore.getState();
    try {
      const doc = this.ctl.requireDoc();
      const version = useDocStore.getState().info?.version ?? 0;
      st.setStatus("loading");
      const svgs = await this.chartSvgs(options);
      const job = await clientV4(this.ctl).preparePrintJob(doc, options, svgs);
      usePrintStore.getState().setJob(job.job, job.layout, version);
      usePrintStore.getState().setStatus("ready");
      if (job.layout.warnings.length > 0) this.toast("info", job.layout.warnings[0] ?? "");
      void this.renderCurrent();
      return job.job;
    } catch (e) {
      // Without a job the preview can still show the layout (page count) when available.
      try {
        const layout = await clientV4(this.ctl).printLayout(this.ctl.requireDoc(), options);
        usePrintStore.getState().setJob(null, layout, -1);
      } catch {
        // Nothing to show.
      }
      usePrintStore.getState().setStatus("error", isApiError(e) ? (e.code === "unsupported" ? PRINT_SOON : e.message) : String(e));
      return null;
    }
  }

  /** The chart SVGs of every printed sheet. */
  private async chartSvgs(options: PrintOptionsDto): Promise<ChartSvgDto[]> {
    const info = this.ctl.info;
    if (!info) return [];
    const sheets = options.sheets.type === "active" ? [this.ctl.sheet] : options.sheets.type === "all" ? info.sheets.map((s) => s.id) : options.sheets.ids;
    const out: ChartSvgDto[] = [];
    for (const sheet of sheets) out.push(...(await this.charts.chartSvgsOf(sheet).catch((): ChartSvgDto[] => [])));
    return out;
  }

  /** The job for the current document (re-prepared when the document changed). */
  private async ensureJob(): Promise<number | null> {
    const st = usePrintStore.getState();
    const version = useDocStore.getState().info?.version ?? 0;
    if (st.job !== null && st.jobVersion === version) return st.job;
    return this.prepare(st.options);
  }

  /** Renders the current page at the current zoom (cached) and the thumbnails around it. */
  async renderCurrent(): Promise<void> {
    const st = usePrintStore.getState();
    if (st.job === null || !st.layout) return;
    const dpr = typeof devicePixelRatio === "number" ? devicePixelRatio : 1;
    await this.renderPage(st.page, dpiFor(st.zoom, dpr));
    for (const i of thumbWindow(st.page, st.layout.pages.length)) void this.renderThumb(i);
  }

  async renderPage(index: number, dpi: number): Promise<void> {
    const st = usePrintStore.getState();
    const key = `${index}:${dpi}`;
    if (st.job === null || st.pages[key] || this.pending.has(key)) return;
    this.pending.add(key);
    try {
      const png = await clientV4(this.ctl).printJobPagePng(st.job, index, dpi);
      if (usePrintStore.getState().job === st.job) usePrintStore.getState().setPagePng(key, `data:image/png;base64,${png}`);
    } catch (e) {
      if (usePrintStore.getState().status !== "error") usePrintStore.getState().setStatus("error", isApiError(e) ? e.message : String(e));
    } finally {
      this.pending.delete(key);
    }
  }

  async renderThumb(index: number): Promise<void> {
    const st = usePrintStore.getState();
    const key = `thumb:${index}`;
    if (st.job === null || st.thumbs[index] || this.pending.has(key)) return;
    this.pending.add(key);
    try {
      const png = await clientV4(this.ctl).printJobPagePng(st.job, index, THUMB_DPI);
      if (usePrintStore.getState().job === st.job) usePrintStore.getState().setThumb(index, `data:image/png;base64,${png}`);
    } catch {
      // Thumbnails are optional.
    } finally {
      this.pending.delete(key);
    }
  }

  goToPage(page: number): void {
    usePrintStore.getState().setPage(page);
    void this.renderCurrent();
  }

  stepPage(delta: number): void {
    const st = usePrintStore.getState();
    this.goToPage(stepPage(st.page, st.layout?.pages.length ?? 0, delta));
  }

  setZoom(zoom: number): void {
    usePrintStore.getState().setZoom(zoom);
    void this.renderCurrent();
  }

  zoomBy(dir: 1 | -1): void {
    this.setZoom(stepPreviewZoom(usePrintStore.getState().zoom, dir));
  }

  // -- export and print ---------------------------------------------------------

  /** File ▸ Export ▸ PDF… / Share ▸ Export as PDF…: save dialog, then the PDF bytes. */
  async exportPdf(): Promise<void> {
    try {
      const doc = this.ctl.requireDoc();
      const shell = this.ctl.shell;
      if (!shell.exportPdf) {
        this.toast("info", "Exporting a PDF needs the desktop app.");
        return;
      }
      const job = await this.ensureJob();
      if (job === null) {
        const err = usePrintStore.getState().error;
        if (err) this.toast(err === PRINT_SOON ? "info" : "error", err);
        return;
      }
      await shell.exportPdf(doc, job);
      this.toast("info", "Exported the PDF.");
    } catch (e) {
      this.report(e);
    }
  }

  /** ⌘P / File ▸ Print…: the OS print dialog over the job's PDF. */
  async print(): Promise<void> {
    try {
      const doc = this.ctl.requireDoc();
      const shell = this.ctl.shell;
      if (!shell.print) {
        this.toast("info", "Printing needs the desktop app.");
        return;
      }
      const job = await this.ensureJob();
      if (job === null) {
        const err = usePrintStore.getState().error;
        if (err) this.toast(err === PRINT_SOON ? "info" : "error", err);
        return;
      }
      await shell.print(doc, job);
    } catch (e) {
      this.report(e);
    }
  }
}
