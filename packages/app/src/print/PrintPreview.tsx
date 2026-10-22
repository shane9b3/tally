/**
 * Print Preview (W5-5): a full-window view of the print job's pages (page
 * thumbnails on the left, the current page rasterized by
 * `print_job_page_png`), zoom, page navigation, Page Setup…, Print and
 * Export PDF. Esc closes it.
 */
import { useEffect, useRef } from "react";
import { useController } from "../context";
import { t } from "../i18n";
import { dpiFor, fitWidthZoom, fitZoom, pageLabel, paperPx } from "./printReducer";
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
  const print = ctl.w5.print;
  const count = layout?.pages.length ?? 0;
  const cur = layout?.pages[page];
  const dpr = typeof devicePixelRatio === "number" ? devicePixelRatio : 1;
  const png = pages[`${page}:${dpiFor(zoom, dpr)}`];
  const size = cur ? paperPx(cur.paperMm, zoom) : null;

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        print.closePreview();
      } else if (e.key === "ArrowRight" || e.key === "PageDown") print.stepPage(1);
      else if (e.key === "ArrowLeft" || e.key === "PageUp") print.stepPage(-1);
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [print]);

  const viewport = () => {
    const el = stage.current;
    return el ? { width: el.clientWidth, height: el.clientHeight } : { width: 800, height: 600 };
  };

  return (
    <div className={styles.overlay} role="dialog" aria-label={t("print.preview", "Print preview")} data-testid="print-preview">
      <div className={inset ? `${styles.bar} ${styles.inset}` : styles.bar} data-tauri-drag-region>
        <span className={styles.barTitle}>{t("print.preview", "Print preview")}</span>
        <button type="button" className={styles.icon} aria-label="Previous page" disabled={page <= 0} onClick={() => print.stepPage(-1)}>
          ‹
        </button>
        <span className={styles.pageLabel} data-testid="print-page-label">
          {status === "loading" && !layout ? "Preparing…" : pageLabel(page, count)}
        </span>
        <button type="button" className={styles.icon} aria-label="Next page" disabled={page >= count - 1} onClick={() => print.stepPage(1)}>
          ›
        </button>
        <span className={styles.spacer} />
        <button type="button" className={styles.icon} aria-label="Zoom out" onClick={() => print.zoomBy(-1)}>
          −
        </button>
        <span className={styles.zoom} data-testid="print-zoom">
          {Math.round(zoom * 100)}%
        </span>
        <button type="button" className={styles.icon} aria-label="Zoom in" onClick={() => print.zoomBy(1)}>
          +
        </button>
        <button type="button" className={styles.btn} disabled={!cur} onClick={() => cur && print.setZoom(fitWidthZoom(cur.paperMm, viewport()))}>
          Fit width
        </button>
        <button type="button" className={styles.btn} disabled={!cur} onClick={() => cur && print.setZoom(fitZoom(cur.paperMm, viewport()))}>
          Whole page
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
        <button type="button" className={styles.btn} aria-label="Close print preview" data-testid="print-close" onClick={() => print.closePreview()}>
          Close
        </button>
      </div>
      <div className={styles.main}>
        <nav className={styles.thumbs} aria-label="Pages" data-testid="print-thumbs">
          {layout?.pages.map((p) => (
            <button key={p.index} type="button" className={p.index === page ? styles.thumbOn : styles.thumb} aria-current={p.index === page ? "page" : undefined} onClick={() => print.goToPage(p.index)}>
              {thumbs[p.index] ? <img src={thumbs[p.index]} alt={`Page ${p.number}`} /> : <span className={styles.thumbBlank} />}
              <span>{p.number}</span>
            </button>
          ))}
        </nav>
        <div ref={stage} className={styles.stage} data-testid="print-stage">
          {cur && size ? (
            png ? (
              <img className={styles.paper} src={png} width={size.width} height={size.height} alt={`Page ${cur.number}`} data-testid="print-page" />
            ) : (
              <div className={styles.paper} style={{ width: size.width, height: size.height }} data-testid="print-page-pending">
                {error ? <div className={styles.empty}>{error}</div> : null}
              </div>
            )
          ) : (
            <div className={styles.empty}>
              {status === "loading" ? "Preparing the pages…" : error ? <div className={styles.warn}>{error}</div> : "Nothing to print on this sheet."}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
