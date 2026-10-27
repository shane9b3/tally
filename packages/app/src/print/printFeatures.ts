/**
 * Printing (W5-5, over the W5-3 engine pipeline): the Print Preview
 * (`print_layout` → `prepare_print_job` with the charts' SVGs →
 * `print_job_page_png`), Export PDF… (`wb_export_pdf`) and Print (⌘P,
 * `wb_print`). Until W5-3 lands every engine call answers `unsupported`,
 * which is shown as a toast.
 *
 * All three cover the same print range (AOO's File ▸ Print ▸ Range): the
 * selected sheets (the active one; the default), all sheets or the selected
 * cells, plus an optional page list. The options always name the sheet the
 * user is on (`activeSheet`): the engine's own active sheet only follows
 * sheet switches that went through an edit. The range is remembered for the
 * session and starts at the default for another document. From the preview,
 * Print and Export PDF use the preview's job, so they print what it shows.
 */
import { CANCELLED, isApiError, type ChartSvgDto, type PrintOptionsDto } from "@tally/bridge";
import type { AppController } from "../controller";
import type { ChartFeatures } from "../charts/chartFeatures";
import { useDocStore } from "../stores/docStore";
import { t } from "../i18n";
import { useUiStore } from "../stores/uiStore";
import { DEFAULT_PRINT_SCOPE, dpiFor, parsePageRanges, printOptionsFor, stepPage, stepPreviewZoom, THUMB_DPI, thumbWindow, type PrintScope } from "./printReducer";
import { selectionRanges, useSelectionStore } from "../stores/selectionStore";
import { usePrintStore } from "./printStore";
import { clientV4 } from "../objects/shell";

/** What a prepared job was built from: the document (a window can show another one later) and the options. */
function jobKey(doc: number, options: PrintOptionsDto): string {
  return JSON.stringify([doc, options]);
}

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

  // -- print range --------------------------------------------------------------

  /** The range chosen for the current document (the default for another one). */
  range(): { scope: PrintScope; pageSpec: string } {
    const st = usePrintStore.getState();
    const doc = this.ctl.info?.doc ?? null;
    if (st.scopeDoc !== doc) return { scope: DEFAULT_PRINT_SCOPE, pageSpec: "" };
    return { scope: st.scope, pageSpec: st.pageSpec };
  }

  /** Remembers a range for this session (an invalid page list is not stored). */
  setRange(scope: PrintScope, pageSpec: string): boolean {
    if (!parsePageRanges(pageSpec).ok) return false;
    usePrintStore.getState().setRange(this.ctl.info?.doc ?? null, scope, pageSpec.trim());
    return true;
  }

  /** The engine options of a range on the user's sheet and current selection. */
  optionsFor(scope: PrintScope, pageSpec: string): PrintOptionsDto {
    const parsed = parsePageRanges(pageSpec);
    const sel = selectionRanges(useSelectionStore.getState().selection);
    return printOptionsFor(scope, parsed.ok ? parsed.ranges : null, this.ctl.sheet, sel);
  }

  // -- preview ------------------------------------------------------------------

  /** File ▸ Print Preview: the remembered range, opened at the active sheet's first page. */
  async openPreview(options?: PrintOptionsDto): Promise<void> {
    if (!options) {
      // The bar shows the range of this document.
      const r = this.range();
      this.setRange(r.scope, r.pageSpec);
      options = this.optionsFor(r.scope, r.pageSpec);
    }
    const st = usePrintStore.getState();
    st.setOpen(true);
    st.setOptions(options);
    await this.prepare(options);
    this.showActiveSheet();
  }

  /** The preview bar's range: remembered, then the preview is rebuilt for it. */
  async changeRange(scope: PrintScope, pageSpec: string): Promise<boolean> {
    if (!this.setRange(scope, pageSpec)) return false;
    const options = this.optionsFor(scope, pageSpec);
    usePrintStore.getState().setOptions(options);
    usePrintStore.getState().setPage(0);
    await this.prepare(options);
    this.showActiveSheet();
    return true;
  }

  /** Goes to the first page of the user's sheet (AOO opens the preview there). */
  private showActiveSheet(): void {
    const layout = usePrintStore.getState().layout;
    const at = layout?.pages.findIndex((p) => p.sheet === this.ctl.sheet) ?? -1;
    this.goToPage(at >= 0 ? at : 0);
  }

  closePreview(): void {
    usePrintStore.getState().setOpen(false);
    usePrintStore.getState().setRangeDialog(null);
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
      usePrintStore.getState().setJob(job.job, job.layout, version, jobKey(doc, options));
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
    const sheets = options.sheets.type === "active" ? [options.activeSheet ?? this.ctl.sheet] : options.sheets.type === "all" ? info.sheets.filter((s) => !s.hidden).map((s) => s.id) : options.sheets.ids;
    const out: ChartSvgDto[] = [];
    let failed = false;
    for (const sheet of sheets) {
      // A chart that cannot be drawn must not stop the print: go on without it, but say so.
      out.push(
        ...(await this.charts.chartSvgsOf(sheet).catch((): ChartSvgDto[] => {
          failed = true;
          return [];
        })),
      );
    }
    if (failed) useUiStore.getState().pushToast("warning", t("print.chartsFailed", "Some charts couldn't be drawn; printing without them."));
    return out;
  }

  /** The job for `options` (re-prepared when the document or the options changed). */
  private async ensureJob(options: PrintOptionsDto): Promise<number | null> {
    const st = usePrintStore.getState();
    const version = useDocStore.getState().info?.version ?? 0;
    const doc = this.ctl.info?.doc ?? null;
    if (st.job !== null && doc !== null && st.jobVersion === version && st.jobKey === jobKey(doc, options)) return st.job;
    st.setOptions(options);
    return this.prepare(options);
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

  /**
   * File ▸ Export ▸ PDF… / Share ▸ Export as PDF…: the range step first
   * (AOO's PDF Options ▸ Range); from the preview, its range directly.
   */
  async exportPdf(): Promise<void> {
    const st = usePrintStore.getState();
    if (st.open) return this.exportPdfWith(st.options);
    st.setRangeDialog("pdf");
  }

  /** ⌘P / File ▸ Print…: the range step first (AOO's Print ▸ Range); from the preview, its range directly. */
  async print(): Promise<void> {
    const st = usePrintStore.getState();
    if (st.open) return this.printWith(st.options);
    st.setRangeDialog("print");
  }

  /** The range step's OK: remembers the range, then prints or exports it. */
  async confirmRange(kind: "print" | "pdf", scope: PrintScope, pageSpec: string): Promise<void> {
    usePrintStore.getState().setRangeDialog(null);
    if (!this.setRange(scope, pageSpec)) return;
    const options = this.optionsFor(scope, pageSpec);
    await (kind === "print" ? this.printWith(options) : this.exportPdfWith(options));
  }

  cancelRange(): void {
    usePrintStore.getState().setRangeDialog(null);
    this.ctl.focusGrid();
  }

  /** Save dialog, then the PDF bytes of the job for `options`. */
  async exportPdfWith(options: PrintOptionsDto): Promise<void> {
    try {
      const doc = this.ctl.requireDoc();
      const shell = this.ctl.shell;
      if (!shell.exportPdf) {
        this.toast("info", "Exporting a PDF needs the desktop app.");
        return;
      }
      const job = await this.ensureJob(options);
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

  /** The OS print dialog over the PDF of the job for `options`. */
  async printWith(options: PrintOptionsDto): Promise<void> {
    try {
      const doc = this.ctl.requireDoc();
      const shell = this.ctl.shell;
      if (!shell.print) {
        this.toast("info", "Printing needs the desktop app.");
        return;
      }
      const job = await this.ensureJob(options);
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
