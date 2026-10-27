/**
 * Print Preview (W5-5): a full-window view of the print job's pages (page
 * thumbnails on the left, the current page rasterized by
 * `print_job_page_png`), zoom, page navigation, Page Setup…, Print and
 * Export PDF. Esc closes it.
 */
import { useEffect, useRef } from "react";
import { useController } from "../context";
import { t } from "../i18n";
import { useReturnFocus, useTitleId } from "../components/dialogA11y";
import { trapTab } from "../components/trapTab";
import { dpiFor, fitWidthZoom, fitZoom, paperPx } from "./printReducer";
import { usePrintStore } from "./printStore";
import styles from "./PrintPreview.module.css";

export function PrintPreview({ inset = false }: { inset?: boolean }) {
  const open = usePrintStore((s) => s.open);
  if (!open) return null;
  return <Preview inset={inset} />;
}

function Preview({ inset }: { inset: boolean }) {
  const ctl = useController();
  const status = usePrintStore((s) => s.status);
  const layout = usePrintStore((s) => s.layout);
  const page = usePrintStore((s) => s.page);
  const zoom = usePrintStore((s) => s.zoom);
  const pages = usePrintStore((s) => s.pages);
  const thumbs = usePrintStore((s) => s.thumbs);
  const error = usePrintStore((s) => s.error);
  const stage = useRef<HTMLDivElement>(null);
  const root = useRef<HTMLDivElement>(null);
  const titleId = useTitleId();
  useReturnFocus();
  const print = ctl.w5.print;
  const count = layout?.pages.length ?? 0;
  const cur = layout?.pages[page];
  const dpr = typeof devicePixelRatio === "number" ? devicePixelRatio : 1;
  const png = pages[`${page}:${dpiFor(zoom, dpr)}`];
  const size = cur ? paperPx(cur.paperMm, zoom) : null;

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // A dialog opened from the preview (Page Setup…) owns its keys.
      const a = document.activeElement;
      if (a && !root.current?.contains(a) && a.closest("[role=dialog], [role=alertdialog]")) return;
      if (e.key === "Escape") {
        e.stopPropagation();
        print.closePreview();
      } else if (e.key === "ArrowRight" || e.key === "PageDown") print.stepPage(1);
      else if (e.key === "ArrowLeft" || e.key === "PageUp") print.stepPage(-1);
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [print]);

  // Modal: the focus moves to the toolbar's Close button and stays inside.
  useEffect(() => {
    if (!root.current?.contains(document.activeElement)) root.current?.querySelector<HTMLElement>("[data-testid=print-close]")?.focus();
  }, []);

  const viewport = () => {
    const el = stage.current;
    return el ? { width: el.clientWidth, height: el.clientHeight } : { width: 800, height: 600 };
  };

  return (
    <div
      ref={root}
      className={styles.overlay}
      role="dialog"
      aria-modal="true"
      aria-labelledby={titleId}
      data-testid="print-preview"
      onKeyDown={(e) => {
        if (root.current) trapTab(root.current, e, document.activeElement);
      }}
    >
      <div className={inset ? `${styles.bar} ${styles.inset}` : styles.bar} data-tauri-drag-region>
        <span id={titleId} className={styles.barTitle}>{t("printPreview.title", "Print preview")}</span>
        <button type="button" className={styles.icon} aria-label={t("printPreview.prev", "Previous page")} disabled={page <= 0} onClick={() => print.stepPage(-1)}>
          ‹
        </button>
        <span className={styles.pageLabel} data-testid="print-page-label">
          {status === "loading" && !layout ? t("printPreview.preparing", "Preparing…") : count === 0 ? t("printPreview.noPages", "No pages") : t("print.pageOf", "Page {n} of {total}", { n: page + 1, total: count })}
        </span>
        <button type="button" className={styles.icon} aria-label={t("printPreview.next", "Next page")} disabled={page >= count - 1} onClick={() => print.stepPage(1)}>
          ›
        </button>
        <span className={styles.spacer} />
        <button type="button" className={styles.icon} aria-label={t("cmd.view.zoomOut", "Zoom out")} onClick={() => print.zoomBy(-1)}>
          −
        </button>
        <span className={styles.zoom} data-testid="print-zoom">
          {Math.round(zoom * 100)}%
        </span>
        <button type="button" className={styles.icon} aria-label={t("cmd.view.zoomIn", "Zoom in")} onClick={() => print.zoomBy(1)}>
          +
        </button>
        <button type="button" className={styles.btn} disabled={!cur} onClick={() => cur && print.setZoom(fitWidthZoom(cur.paperMm, viewport()))}>
          {t("printPreview.fitWidth", "Fit width")}
        </button>
        <button type="button" className={styles.btn} disabled={!cur} onClick={() => cur && print.setZoom(fitZoom(cur.paperMm, viewport()))}>
          {t("printPreview.wholePage", "Whole page")}
        </button>
        <span className={styles.spacer} />
        <button type="button" className={styles.btn} onClick={() => void ctl.v3.openPageSetup()}>
          {t("cmd.file.pageSetup", "Page setup…")}
        </button>
        <button type="button" className={styles.btn} data-testid="print-export-pdf" onClick={() => void print.exportPdf()}>
          {t("cmd.file.exportPdf", "Export as PDF…")}
        </button>
        <button type="button" className={styles.primary} data-testid="print-print" onClick={() => void print.print()}>
          {t("cmd.file.print", "Print…")}
        </button>
        <button type="button" className={styles.btn} aria-label={t("printPreview.close", "Close print preview")} data-testid="print-close" onClick={() => print.closePreview()}>
          {t("dialog.close", "Close")}
        </button>
      </div>
      <div className={styles.main}>
        <nav className={styles.thumbs} aria-label={t("print.pages", "Pages")} data-testid="print-thumbs">
          {layout?.pages.map((p) => (
            <button key={p.index} type="button" className={p.index === page ? styles.thumbOn : styles.thumb} aria-current={p.index === page ? "page" : undefined} onClick={() => print.goToPage(p.index)}>
              {thumbs[p.index] ? <img src={thumbs[p.index]} alt={t("printPreview.pageN", "Page {n}", { n: p.number })} /> : <span className={styles.thumbBlank} />}
              <span>{p.number}</span>
            </button>
          ))}
        </nav>
        <div ref={stage} className={styles.stage} tabIndex={0} role="region" aria-label={t("print.page", "Page")} data-testid="print-stage">
          {cur && size ? (
            png ? (
              <img className={styles.paper} src={png} width={size.width} height={size.height} alt={t("printPreview.pageN", "Page {n}", { n: cur.number })} data-testid="print-page" />
            ) : (
              <div className={styles.paper} style={{ width: size.width, height: size.height }} data-testid="print-page-pending">
                {error ? <div className={styles.empty}>{error}</div> : null}
              </div>
            )
          ) : (
            <div className={styles.empty}>
              {status === "loading" ? t("printPreview.preparingPages", "Preparing the pages…") : error ? <div className={styles.warn}>{error}</div> : t("printPreview.nothing", "Nothing to print on this sheet.")}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
