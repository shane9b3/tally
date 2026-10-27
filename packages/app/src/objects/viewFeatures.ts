/**
 * View features of W5-5: the split window (`SplitView` + `setSplit`), Page
 * Break Preview (`page_breaks` lines, draggable manual breaks →
 * `setPageBreak`), zoom presets (optimal / page width / whole page, §4 item
 * 50) and full screen.
 */
import { isApiError, type Op, type PrintPageDto, type RangeDto, type SplitDto } from "@tally/bridge";
import type { AppController } from "../controller";
import { clampZoom } from "../formats";
import { useDocStore } from "../stores/docStore";
import { useUiStore } from "../stores/uiStore";
import { useV3Store } from "../stores/v3Store";
import { autoPageBreaks, defaultPageSetup } from "../v03";
import { DEFAULT_PRINT_OPTIONS } from "../print/printStore";
import type { W5Shell } from "./shell";
import { clampRatio, type SplitAxis } from "@tally/grid";
import { useW5Store } from "./w5Store";
import { clientV4 } from "./shell";
import { t } from "../i18n";

/** Delay before a divider drag is persisted with `setSplit`. */
export const SPLIT_PERSIST_MS = 300;

/** The width/height in px (zoom 100%) of a range from the layout. */
export function rangeSizePx(ctl: Pick<AppController, "layoutSize">, range: RangeDto): { width: number; height: number } {
  let width = 0;
  let height = 0;
  for (let c = range.c0; c <= range.c1; c++) width += ctl.layoutSize("col", c);
  for (let r = range.r0; r <= range.r1; r++) height += ctl.layoutSize("row", r);
  return { width, height };
}

/** The zoom that fits a content size in a viewport (clamped to the app's zoom range). */
export function zoomToFit(content: { width: number; height: number }, viewport: { width: number; height: number }, mode: "both" | "width"): number {
  const zw = viewport.width / Math.max(1, content.width);
  const zh = viewport.height / Math.max(1, content.height);
  return clampZoom(mode === "width" ? zw : Math.min(zw, zh));
}

export class ViewFeatures {
  private persistTimer: ReturnType<typeof setTimeout> | null = null;

  constructor(private readonly ctl: AppController) {}

  private get shell(): W5Shell {
    return this.ctl.shell as unknown as W5Shell;
  }

  private toast(kind: "info" | "error", message: string): void {
    useUiStore.getState().pushToast(kind, message);
  }

  // -- split window ---------------------------------------------------------------

  /** View ▸ Split Window: toggles a split at the active cell (rows above it; columns when it is in row 1). */
  toggleSplit(): void {
    if (useW5Store.getState().split) {
      this.removeSplit();
      return;
    }
    const grid = this.ctl.grid;
    const a = this.ctl.selection.active;
    let axis: SplitAxis = "horizontal";
    let ratio = 0.5;
    if (grid) {
      const g = grid.getGeometry();
      if (a.r === 0 && a.c > 0) {
        axis = "vertical";
        ratio = g.colLeft(a.c) / Math.max(1, g.width);
      } else if (a.r > 0) ratio = g.rowTop(a.r) / Math.max(1, g.height);
    }
    this.setSplit(axis, clampRatio(ratio));
  }

  setSplit(axis: SplitAxis, ratio: number): void {
    useW5Store.getState().setSplit({ axis, ratio });
    this.persistSplit();
  }

  /**
   * Shows the split the document stores for the sheet (`view_options().split`,
   * px at zoom 100 %), without persisting it again: a vertical divider when
   * only `xPx` is set, else a horizontal one.
   */
  restoreSplit(split: SplitDto | null): void {
    if (!split || (split.xPx <= 0 && split.yPx <= 0)) {
      if (useW5Store.getState().split) useW5Store.getState().setSplit(null);
      return;
    }
    const axis: SplitAxis = split.yPx > 0 ? "horizontal" : "vertical";
    const host = this.hostSize();
    const zoom = useUiStore.getState().zoom || 1;
    const extent = axis === "vertical" ? host.width : host.height;
    const px = (axis === "vertical" ? split.xPx : split.yPx) * zoom;
    const ratio = extent > 0 ? clampRatio(px / extent) : 0.5;
    useW5Store.getState().setSplit({ axis, ratio });
  }

  /** The size of the whole grid area (both panes of a split), CSS px. */
  private hostSize(): { width: number; height: number } {
    const el = typeof document === "undefined" ? null : document.querySelector<HTMLElement>('[data-testid="grid"]');
    if (el && el.clientWidth > 0) return { width: el.clientWidth, height: el.clientHeight };
    const g = this.ctl.grid?.getGeometry();
    return { width: g?.width ?? 0, height: g?.height ?? 0 };
  }

  /** The divider was dragged (persisted after a short delay). */

  onSplitRatio = (ratio: number): void => {
    const s = useW5Store.getState().split;
    if (!s || s.ratio === ratio) return;
    useW5Store.getState().setSplit({ ...s, ratio });
    if (this.persistTimer) clearTimeout(this.persistTimer);
    this.persistTimer = setTimeout(() => this.persistSplit(), SPLIT_PERSIST_MS);
  };

  /** Window ▸ Remove Split. */
  removeSplit(): void {
    if (!useW5Store.getState().split) return;
    useW5Store.getState().setSplit(null);
    this.persistSplit();
  }

  /** `setSplit { xPx, yPx }` at zoom 100 % (quietly ignored while the engine answers `unsupported`). */
  private persistSplit(): void {
    const info = this.ctl.info;
    const grid = this.ctl.grid;
    if (!info) return;
    const s = useW5Store.getState().split;
    const g = grid ? this.hostSize() : null;
    const zoom = useUiStore.getState().zoom || 1;
    const split = s && g ? { xPx: s.axis === "vertical" ? Math.round((s.ratio * g.width) / zoom) : 0, yPx: s.axis === "horizontal" ? Math.round((s.ratio * g.height) / zoom) : 0 } : null;
    const op: Op = { type: "setSplit", sheet: this.ctl.sheet, split };
    void this.ctl.client
      .apply(info.doc, { ops: [op], label: split ? "Split window" : "Remove split", selectionBefore: null, selectionAfter: null })
      .then((res) => {
        if (res.errors.length === 0) this.ctl.handleResult(res);
      })
      .catch((e: unknown) => {
        if (!isApiError(e) || e.code !== "unsupported") this.ctl.notifyError(e);
      });
  }

  // -- page break preview -----------------------------------------------------------

  /** View ▸ Page Break Preview (toggle). */
  togglePageBreakPreview(): void {
    const on = !useW5Store.getState().pageBreakPreview;
    useW5Store.getState().setPageBreakPreview(on);
    if (on) void this.refreshPageBreaks();
    else useW5Store.getState().setPageBreaks(null);
  }

  /** Re-reads `page_breaks` (falls back to the v0.3 estimate while unsupported). */
  async refreshPageBreaks(): Promise<void> {
    const info = this.ctl.info;
    if (!info || !useW5Store.getState().pageBreakPreview) return;
    const sheet = this.ctl.sheet;
    try {
      const b = await clientV4(this.ctl).pageBreaks(info.doc, sheet);
      if (sheet === this.ctl.sheet) useW5Store.getState().setPageBreaks(b);
    } catch {
      if (sheet === this.ctl.sheet) {
        useW5Store.getState().setPageBreaks(null);
        // Estimate from the page setup (features.ts computes into the v3 store).
        if (!useV3Store.getState().pageBreaksOn) this.ctl.v3.setPageBreaksOn(true);
        else this.ctl.v3.computePageBreaks();
      }
    }
  }

  /** A break line was dragged: the manual break moves (an automatic one becomes manual). */
  onPageBreakDrag = (axis: "row" | "col", from: number, to: number, manual: boolean): void => {
    const sheet = this.ctl.sheet;
    const ops: Op[] = [];
    if (manual) ops.push({ type: "setPageBreak", sheet, axis, at: from, present: false });
    ops.push({ type: "setPageBreak", sheet, axis, at: to, present: true });
    void this.ctl.apply(t("msg.undo.movePageBreak", "Move page break"), ops).then(() => this.refreshPageBreaks());
  };

  // -- zoom presets -----------------------------------------------------------------

  private viewport(): { width: number; height: number } | null {
    const g = this.ctl.grid?.getGeometry();
    if (!g) return null;
    return { width: Math.max(1, g.width - g.headerW - 4), height: Math.max(1, g.height - g.headerH - 4) };
  }

  /** View ▸ Optimal Zoom: the selection (or the used area) fills the window. */
  zoomOptimal(): void {
    const vp = this.viewport();
    if (!vp) return;
    const sel = this.ctl.selection.range;
    const single = sel.r0 === sel.r1 && sel.c0 === sel.c1;
    const range = single ? (useDocStore.getState().source?.getLayout(this.ctl.sheet).used ?? sel) : sel;
    const z = zoomToFit(rangeSizePx(this.ctl, range), vp, "both");
    useUiStore.getState().setZoom(z);
    this.ctl.grid?.scrollTo({ r: range.r0, c: range.c0 });
  }

  /** View ▸ Zoom to Page Width / Whole Page: from the first printed page's rows and columns. */
  async zoomToPage(mode: "width" | "both"): Promise<void> {
    const vp = this.viewport();
    if (!vp) return;
    const page = await this.firstPage();
    if (!page) {
      this.toast("info", "Nothing to print on this sheet.");
      return;
    }
    const size = rangeSizePx(this.ctl, { r0: page.rows[0], c0: page.cols[0], r1: page.rows[1], c1: page.cols[1] });
    useUiStore.getState().setZoom(zoomToFit(size, vp, mode));
    this.ctl.grid?.scrollTo({ r: page.rows[0], c: page.cols[0] });
  }

  /** The first page from `print_layout`, else from the v0.3 page-break estimate. */
  private async firstPage(): Promise<Pick<PrintPageDto, "rows" | "cols"> | null> {
    const info = this.ctl.info;
    if (!info) return null;
    try {
      const layout = await clientV4(this.ctl).printLayout(info.doc, DEFAULT_PRINT_OPTIONS);
      const p = layout.pages.find((pg) => pg.sheet === this.ctl.sheet) ?? layout.pages[0];
      if (p) return p;
    } catch {
      // Estimate below.
    }
    const layout = useDocStore.getState().source?.getLayout(this.ctl.sheet);
    const setup = useV3Store.getState().pageSetup ?? defaultPageSetup();
    const area = setup.printRange ?? layout?.used ?? null;
    if (!layout || !area) return null;
    const auto = autoPageBreaks(setup, area, { runs: layout.rowRuns, dflt: layout.defaultRowHeightPx }, { runs: layout.colRuns, dflt: layout.defaultColWidthPx });
    const r1 = (auto.rows[0] ?? area.r1 + 1) - 1;
    const c1 = (auto.cols[0] ?? area.c1 + 1) - 1;
    return { rows: [area.r0, Math.max(area.r0, r1)], cols: [area.c0, Math.max(area.c0, c1)] };
  }

  // -- full screen ----------------------------------------------------------------------

  /** View ▸ Full Screen (desktop: the window; browser: the document). */
  async toggleFullScreen(): Promise<void> {
    try {
      if (this.shell.toggleFullScreen) {
        await this.shell.toggleFullScreen();
        return;
      }
      if (typeof document === "undefined") return;
      if (document.fullscreenElement) await document.exitFullscreen();
      else await document.documentElement.requestFullscreen();
    } catch (e) {
      this.ctl.notifyError(e);
    }
  }
}
