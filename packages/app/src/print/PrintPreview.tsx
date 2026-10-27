/**
 * Print Preview (W5-5): a full-window view of the print job's pages (page
 * thumbnails on the left, the current page rasterized by
 * `print_job_page_png`), zoom, page navigation, the print range (sheets and
 * pages, so the preview shows what prints), Page Setup…, Print and Export
 * PDF. Esc closes it.
 *
 * Also the range step of File ▸ Print… and Export as PDF… (AOO's Print ▸
 * Range and PDF Options ▸ Range): All sheets / Selected sheets / Selected
 * cells, and All pages / Pages "1-3, 5".
 */
import { useEffect, useRef, useState } from "react";
import { useController } from "../context";
import { t } from "../i18n";
import { useReturnFocus, useTitleId } from "../components/dialogA11y";
import { trapTab } from "../components/trapTab";
import { arrowNav } from "../components/dialogA11y";
import { dpiFor, fitWidthZoom, fitZoom, paperPx, parsePageRanges, PRINT_SCOPES, type PrintScope } from "./printReducer";
import { usePrintStore } from "./printStore";
import styles from "./PrintPreview.module.css";
import dlg from "../components/Dialogs.module.css";

export function PrintPreview({ inset = false }: { inset?: boolean }) {
  const open = usePrintStore((s) => s.open);
  const rangeDialog = usePrintStore((s) => s.rangeDialog);
  return (
    <>
      {open ? <Preview inset={inset} /> : null}
      {rangeDialog ? <RangeDialog kind={rangeDialog} /> : null}
    </>
  );
}

function scopeLabel(scope: PrintScope): string {
  switch (scope) {
    case "all":
      return t("print.scope.all", "All sheets");
    case "selected":
      return t("print.scope.selected", "Selected sheets");
    case "selection":
      return t("print.scope.selection", "Selected cells");
  }
}

/** The preview bar's range: sheets (a menu) and a page list (blank = all pages). */
function RangeBar() {
  const ctl = useController();
  const scope = usePrintStore((s) => s.scope);
  const spec = usePrintStore((s) => s.pageSpec);
  const [draft, setDraft] = useState(spec);
  const [bad, setBad] = useState(false);
  useEffect(() => setDraft(spec), [spec]);
  const print = ctl.w5.print;
  const commit = () => {
    if (draft.trim() === spec) return setBad(false);
    const ok = parsePageRanges(draft).ok;
    setBad(!ok);
    if (ok) void print.changeRange(scope, draft);
  };
  return (
    <>
      <select className={styles.select} aria-label={t("print.range", "Range")} data-testid="print-scope" value={scope} onChange={(e) => void print.changeRange(e.target.value as PrintScope, spec)}>
        {PRINT_SCOPES.map((s) => (
          <option key={s} value={s}>
            {scopeLabel(s)}
          </option>
        ))}
      </select>
      <input
        className={bad ? `${styles.pages} ${styles.pagesBad}` : styles.pages}
        aria-label={t("printPreview.pagesLabel", "Pages to print")}
        aria-invalid={bad || undefined}
        title={bad ? t("print.pagesInvalid", "Enter page numbers or ranges, such as 1-3, 5.") : undefined}
        placeholder={t("print.pagesAll", "All pages")}
        data-testid="print-pages"
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            commit();
          }
        }}
      />
    </>
  );
}

/** File ▸ Print… / Export as PDF…: which sheets and pages, then the OS dialog. */
function RangeDialog({ kind }: { kind: "print" | "pdf" }) {
  const ctl = useController();
  const print = ctl.w5.print;
  const start = useRef(print.range()).current;
  const [scope, setScope] = useState<PrintScope>(start.scope);
  const [some, setSome] = useState(start.pageSpec !== "");
  const [spec, setSpec] = useState(start.pageSpec);
  const form = useRef<HTMLFormElement>(null);
  const pagesInput = useRef<HTMLInputElement>(null);
  const titleId = useTitleId();
  useReturnFocus();
  const valid = !some || (spec.trim() !== "" && parsePageRanges(spec).ok);
  useEffect(() => {
    form.current?.querySelector<HTMLInputElement>("input[type=radio]:checked")?.focus();
  }, []);
  const title = kind === "print" ? t("print.title", "Print") : t("print.exportPdf", "Export as PDF");
  return (
    <div className={dlg.overlay} onMouseDown={(e) => e.target === e.currentTarget && print.cancelRange()}>
      <form
        ref={form}
        className={dlg.card}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        data-testid="print-range-dialog"
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            e.preventDefault();
            e.stopPropagation();
            print.cancelRange();
            return;
          }
          if (arrowNav(e)) return;
          if (form.current) trapTab(form.current, e, document.activeElement);
        }}
        onSubmit={(e) => {
          e.preventDefault();
          if (valid) void print.confirmRange(kind, scope, some ? spec : "");
        }}
      >
        <h2 id={titleId} className={dlg.title}>
          {title}
        </h2>
        <fieldset className={styles.group} role="radiogroup">
          <legend className={styles.legend}>{t("print.range", "Range")}</legend>
          {PRINT_SCOPES.map((s) => (
            <label key={s} className={styles.radio}>
              <input type="radio" name="print-scope" value={s} checked={scope === s} onChange={() => setScope(s)} data-testid={`print-scope-${s}`} />
              {scopeLabel(s)}
            </label>
          ))}
        </fieldset>
        <fieldset className={styles.group} role="radiogroup">
          <legend className={styles.legend}>{t("print.pages", "Pages")}</legend>
          <label className={styles.radio}>
            <input type="radio" name="print-pages" checked={!some} onChange={() => setSome(false)} data-testid="print-pages-all" />
            {t("print.pagesAll", "All pages")}
          </label>
          <label className={styles.radio}>
            <input
              type="radio"
              name="print-pages"
              checked={some}
              onChange={() => {
                setSome(true);
                setTimeout(() => pagesInput.current?.focus());
              }}
              data-testid="print-pages-some"
            />
            {t("print.pagesSome", "Pages")}
          </label>
          <input
            ref={pagesInput}
            className={`${dlg.input} ${styles.pagesField}`}
            aria-label={t("printPreview.pagesLabel", "Pages to print")}
            aria-invalid={(some && spec.trim() !== "" && !valid) || undefined}
            placeholder={t("print.pagesExample", "e.g. 1-3, 5")}
            data-testid="print-pages-spec"
            value={spec}
            onChange={(e) => {
              setSpec(e.target.value);
              setSome(true);
            }}
          />
          {some && spec.trim() !== "" && !valid ? <p className={dlg.error}>{t("print.pagesInvalid", "Enter page numbers or ranges, such as 1-3, 5.")}</p> : null}
        </fieldset>
        <div className={dlg.actions}>
          <button type="button" className={dlg.secondary} onClick={() => print.cancelRange()} data-testid="dialog-cancel">
            {t("dialog.cancel", "Cancel")}
          </button>
          <button type="submit" className={dlg.primary} disabled={!valid} data-testid="dialog-ok">
            {kind === "print" ? t("cmd.file.print", "Print…") : t("print.export", "Export…")}
          </button>
        </div>
      </form>
    </div>
  );
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
      // The range controls keep their own arrow keys.
      if ((a instanceof HTMLInputElement || a instanceof HTMLSelectElement) && e.key !== "Escape") return;
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
        <RangeBar />
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
              <img className={styles.paper} src={png} width={size.width} height={size.height} alt={t("printPreview.pageN", "Page {n}", { n: cur.number })} data-testid="print-page" data-sheet={cur.sheet} />
            ) : (
              <div className={styles.paper} style={{ width: size.width, height: size.height }} data-testid="print-page-pending" data-sheet={cur.sheet}>
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
