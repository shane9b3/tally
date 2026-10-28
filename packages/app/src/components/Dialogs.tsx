/**
 * Small modal dialogs: Row height / Column width, Insert N rows/columns,
 * Custom number format, Sort…, Find & Replace, Paste Special, Define Name,
 * Manage Names and a generic confirmation.
 */
import { tsvDims, type NameInfo, type PasteOperation, type PasteSelection, type SortKey } from "@tally/bridge";
import { useEffect, useId, useRef, useState, type ReactNode } from "react";
import { useController } from "../context";
import { useW56Store } from "../data/store";
import { t } from "../i18n";
import { a1, ALL_KINDS, DEFAULT_PASTE_SPECIAL, isValidName, PASTE_KINDS, pasteChoiceValid, pasteKinds, pasteShiftAllowed, pasteWhat, searchIn, type PasteKind, type PasteShift, type PasteSpecialChoice } from "../dataOps";
import { useDataStore } from "../stores/dataStore";
import { NUMBER_FORMATS } from "../formats";
import { formatNumberLite } from "../numfmtLite";
import { useDocStore } from "../stores/docStore";
import { selectionRanges, useSelectionStore } from "../stores/selectionStore";
import { useUiStore, type DialogState } from "../stores/uiStore";
import styles from "./Dialogs.module.css";
import fs from "../data/FillSheetDialog.module.css";
import { V3Dialog, isV3Dialog } from "./V3Dialogs";
import { FunctionWizard } from "./FunctionWizard";
import { FOCUSABLE, trapTab } from "./trapTab";
import { arrowNav, useReturnFocus, useTitleId } from "./dialogA11y";
import { loadFillSheetDialog } from "../lazyChunks";
import { LazySlot, lazyPart } from "../lazyPart";

export function Dialogs() {
  const dialog = useUiStore((s) => s.dialog);
  useW56Store((s) => s.i18nRev);
  if (!dialog) return null;
  return <DialogFor dialog={dialog} />;
}

function DialogFor({ dialog }: { dialog: DialogState }) {
  if (isV3Dialog(dialog)) return <V3Dialog dialog={dialog} />;
  switch (dialog.kind) {
    case "resize":
      return <ResizeDialog d={dialog} />;
    case "insert":
      return <InsertDialog d={dialog} />;
    case "numberFormat":
      return <NumberFormatDialog d={dialog} />;
    case "sort":
      return <SortDialog d={dialog} />;
    case "findReplace":
      return <FindReplaceDialog d={dialog} />;
    case "pasteSpecial":
      return <PasteSpecialDialog />;
    case "defineName":
      return <DefineNameDialog d={dialog} />;
    case "manageNames":
      return <ManageNamesDialog />;
    case "confirm":
      return <ConfirmDialog d={dialog} />;
    case "functionWizard":
      return <FunctionWizard />;
    case "fillSheet":
      return (
        <LazySlot onFail={close}>
          <FillSheetDialog />
        </LazySlot>
      );
  }
}

// Edit ▸ Fill ▸ Sheets… loads on first use, in its own slot (see `lazyChunks.ts`).
const FillSheetDialog = lazyPart(() => loadFillSheetDialog().then((m) => m.FillSheetDialog));

export { trapTab };

export function close(): void {
  useUiStore.getState().openDialog(null);
}

export interface ShellProps {
  title: string;
  children: ReactNode;
  onSubmit: () => void;
  ok?: string;
  valid?: boolean;
  wide?: boolean;
  danger?: boolean;
  testId?: string;
  /** Extra buttons left of Cancel. */
  extra?: ReactNode;
  /** Keep the dialog open after submitting. */
  keepOpen?: boolean;
  cancelLabel?: string;
  /** A third, secondary action (AOO's "Current selection" next to "Extend selection"): closes the dialog, then runs. */
  alt?: { label: string; onClick: () => void };
  /** Focus the primary button on open (Enter confirms), as AOO's Yes / No boxes do. */
  focusOk?: boolean;
  /** Runs when the dialog is cancelled (Cancel, Escape, a click outside). */
  onCancel?: () => void;
  /** Extra class on the card (larger dialogs). */
  cardClass?: string;
  /** Hide the primary button (dialogs with their own actions). */
  noOk?: boolean;
}

export function Shell({ title, children, onSubmit, ok, valid = true, wide, danger, testId, extra, keepOpen, cancelLabel, alt, onCancel, focusOk, cardClass, noOk }: ShellProps) {
  const ctl = useController();
  const formRef = useRef<HTMLFormElement>(null);
  const titleId = useTitleId();
  useReturnFocus();
  const cancel = () => {
    close();
    onCancel?.();
    ctl.focusGrid();
  };
  // Modal: focus starts inside the dialog (the first field, else the first button).
  useEffect(() => {
    const form = formRef.current;
    if (!form || form.contains(document.activeElement)) return;
    const okBtn = focusOk ? form.querySelector<HTMLElement>('[data-testid="dialog-ok"]') : null;
    (okBtn ?? form.querySelector<HTMLElement>(FOCUSABLE))?.focus();
  }, []);
  return (
    <div className={styles.overlay} onMouseDown={(e) => e.target === e.currentTarget && cancel()}>
      <form
        ref={formRef}
        className={[styles.card, wide ? styles.wide : "", cardClass ?? ""].filter(Boolean).join(" ")}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        data-testid={testId}
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            e.preventDefault();
            e.stopPropagation();
            cancel();
            return;
          }
          if (arrowNav(e)) return;
          if (formRef.current) trapTab(formRef.current, e, document.activeElement);
        }}
        onSubmit={(e) => {
          e.preventDefault();
          if (!valid) return;
          if (!keepOpen) close();
          onSubmit();
          if (!keepOpen) ctl.focusGrid();
        }}
      >
        <h2 id={titleId} className={styles.title}>
          {title}
        </h2>
        {children}
        <div className={styles.actions}>
          {extra ? <div className={styles.extra}>{extra}</div> : null}
          {alt ? (
            <button
              type="button"
              className={styles.secondary}
              onClick={() => {
                close();
                alt.onClick();
                ctl.focusGrid();
              }}
              data-testid="dialog-alt"
            >
              {alt.label}
            </button>
          ) : null}
          <button type="button" className={styles.secondary} onClick={cancel} data-testid="dialog-cancel">
            {cancelLabel ?? t("dialog.cancel", "Cancel")}
          </button>
          {noOk ? null : (
            <button type="submit" className={danger ? `${styles.primary} ${styles.dangerBtn}` : styles.primary} disabled={!valid} data-testid="dialog-ok">
              {ok ?? t("dialog.apply", "Apply")}
            </button>
          )}
        </div>
      </form>
    </div>
  );
}

export function useAutoFocus() {
  const ref = useRef<HTMLInputElement>(null);
  useEffect(() => {
    ref.current?.focus();
    ref.current?.select();
  }, []);
  return ref;
}

function ResizeDialog({ d }: { d: Extract<DialogState, { kind: "resize" }> }) {
  const ctl = useController();
  const [px, setPx] = useState(String(d.px));
  const n = Number(px);
  const valid = Number.isFinite(n) && n >= 2 && n <= 2000;
  const ref = useAutoFocus();
  const what = d.axis === "row" ? t("dlg.rowHeight", "Row height") : t("dlg.columnWidth", "Column width");
  return (
    <Shell title={what} valid={valid} onSubmit={() => ctl.resize(d.axis, d.start, d.end, Math.round(n))}>
      <label className={styles.field}>
        <span>{t("dlg.inPixels", "{what} in pixels", { what })}</span>
        <input ref={ref} className={styles.input} inputMode="numeric" value={px} onChange={(e) => setPx(e.target.value)} />
      </label>
      <p className={styles.note}>
        {d.end > d.start
          ? d.axis === "row"
            ? t("dlg.appliesRows", "Applies to {n} rows.", { n: d.end - d.start + 1 })
            : t("dlg.appliesCols", "Applies to {n} columns.", { n: d.end - d.start + 1 })
          : d.axis === "row"
            ? t("dlg.defaultRow", "Default: 24 px")
            : t("dlg.defaultCol", "Default: 100 px")}
      </p>
    </Shell>
  );
}

function InsertDialog({ d }: { d: Extract<DialogState, { kind: "insert" }> }) {
  const ctl = useController();
  const [count, setCount] = useState("1");
  const [where, setWhere] = useState<"before" | "after">(d.where);
  const n = Number(count);
  const valid = Number.isInteger(n) && n >= 1 && n <= 10_000;
  const ref = useAutoFocus();
  const rows = d.axis === "row";
  return (
    <Shell
      title={rows ? t("dlg.insertRows", "Insert rows") : t("dlg.insertColumns", "Insert columns")}
      ok={t("dialog.insert", "Insert")}
      valid={valid}
      onSubmit={() => void (rows ? ctl.insertRows(where, n) : ctl.insertCols(where, n))}
    >
      <label className={styles.field}>
        <span>{t("dlg.howMany", "How many")}</span>
        <input ref={ref} className={styles.input} inputMode="numeric" value={count} onChange={(e) => setCount(e.target.value)} />
      </label>
      <div className={styles.segmented} role="radiogroup">
        {(["before", "after"] as const).map((w) => (
          <button
            key={w}
            type="button"
            role="radio"
            aria-checked={where === w}
            className={where === w ? styles.segOn : styles.seg}
            onClick={() => setWhere(w)}
          >
            {rows ? (w === "before" ? t("dlg.above", "Above") : t("dlg.below", "Below")) : w === "before" ? t("dlg.left", "Left") : t("dlg.right", "Right")}
          </button>
        ))}
      </div>
    </Shell>
  );
}

function NumberFormatDialog({ d }: { d: Extract<DialogState, { kind: "numberFormat" }> }) {
  const ctl = useController();
  const [code, setCode] = useState(d.code);
  const ref = useAutoFocus();
  // Preview with the active cell's number (if any) plus fixed samples.
  const active = useSelectionStore((s) => s.selection.active);
  const source = useDocStore((s) => s.source);
  const cell = source?.getCell(active.r, active.c);
  const value = cell?.kind === "number" && cell.num !== null ? cell.num : 1234.5;
  const samples = [value, -value, 0];
  const preview = samples.map((v) => formatNumberLite(v, code));
  return (
    <Shell title={t("dlg.customNumberFormat", "Custom number format")} wide onSubmit={() => void ctl.setNumberFormat(code.trim() || "General", t("toolbar.numberFormat", "Number format"))}>
      <label className={styles.field}>
        <span>{t("fmt.formatCode", "Format code")}</span>
        <input
          ref={ref}
          className={`${styles.input} ${styles.mono}`}
          value={code}
          spellCheck={false}
          onChange={(e) => setCode(e.target.value)}
          data-testid="numfmt-code"
        />
      </label>
      <div className={styles.preview} data-testid="numfmt-preview">
        {preview[0] === null ? (
          <span className={styles.note}>{t("dlg.noPreview", "No preview for this code; the engine will format it when applied.")}</span>
        ) : (
          samples.map((v, i) => (
            <div key={i} className={styles.previewRow}>
              <span className={styles.previewIn}>{Number(v.toPrecision(10))}</span>
              <span className={styles.previewOut}>{preview[i]}</span>
            </div>
          ))
        )}
      </div>
      <div className={styles.presets}>
        {NUMBER_FORMATS.map((f) => (
          <button key={f.id} type="button" className={styles.preset} title={f.code} onClick={() => setCode(f.code)}>
            {f.label}
          </button>
        ))}
      </div>
    </Shell>
  );
}

// -- Sort… ---------------------------------------------------------------------------

function SortDialog({ d }: { d: Extract<DialogState, { kind: "sort" }> }) {
  const ctl = useController();
  const [hasHeader, setHasHeader] = useState(d.hasHeader);
  const [keys, setKeys] = useState<(SortKey | null)[]>(() => [d.keys[0] ?? { col: d.range.c0, ascending: true }, null, null]);
  const [caseSensitive, setCase] = useState(false);
  const cols: number[] = [];
  for (let c = d.range.c0; c <= d.range.c1; c++) cols.push(c);
  const label = (c: number) => (hasHeader ? (d.labels[c - d.range.c0] ?? "") : t("dlg.columnLetter", "Column {col}", { col: a1({ r0: 0, c0: c, r1: 0, c1: c }).replace(/\d+$/, "") }));
  const setKey = (i: number, k: SortKey | null) => setKeys((ks) => ks.map((x, j) => (j === i ? k : x)));
  const chosen = keys.filter((k): k is SortKey => k !== null);
  const range = hasHeader ? t("dlg.headerKept", "{range} (header row kept)", { range: a1(d.range) }) : a1(d.range);
  return (
    <Shell title={t("sort.title", "Sort")} ok={t("sort.title", "Sort")} wide testId="sort-dialog" valid={chosen.length > 0} onSubmit={() => void ctl.sortWith({ range: d.range, keys: chosen, hasHeader, caseSensitive })}>
      <p className={styles.lead}>{t("dlg.sortLead", "Sorts {range}. Whole rows move together.", { range })}</p>
      {keys.map((k, i) => (
        <div key={i} className={styles.sortRow}>
          <span className={styles.sortLabel}>{i === 0 ? t("dlg.sortBy", "Sort by") : t("sort.thenBy", "Then by")}</span>
          <select
            className={styles.select}
            aria-label={i === 0 ? t("dlg.sortBy", "Sort by") : t("dlg.thenByN", "Then by {n}", { n: i })}
            value={k ? String(k.col) : ""}
            disabled={i > 0 && keys[i - 1] === null}
            onChange={(e) => setKey(i, e.target.value === "" ? null : { col: Number(e.target.value), ascending: k?.ascending ?? true })}
            data-testid={`sort-key-${i}`}
          >
            {i > 0 ? <option value="">{t("dlg.noneParen", "(none)")}</option> : null}
            {cols.map((c) => (
              <option key={c} value={c}>
                {label(c)}
              </option>
            ))}
          </select>
          <div className={styles.segmentedInline} role="radiogroup" aria-label={t("dlg.order", "Order")}>
            {[true, false].map((asc) => (
              <button
                key={String(asc)}
                type="button"
                role="radio"
                aria-checked={k !== null && k.ascending === asc}
                disabled={k === null}
                className={k !== null && k.ascending === asc ? styles.segOn : styles.seg}
                onClick={() => k && setKey(i, { ...k, ascending: asc })}
              >
                {asc ? "A → Z" : "Z → A"}
              </button>
            ))}
          </div>
        </div>
      ))}
      <label className={styles.checkRow}>
        <input type="checkbox" checked={hasHeader} onChange={(e) => setHasHeader(e.target.checked)} data-testid="sort-header" />
        <span>{t("sort.hasHeader", "Range contains column labels")}</span>
      </label>
      <label className={styles.checkRow}>
        <input type="checkbox" checked={caseSensitive} onChange={(e) => setCase(e.target.checked)} />
        <span>{t("sort.caseSensitive", "Case sensitive")}</span>
      </label>
    </Shell>
  );
}

// -- Find & Replace -------------------------------------------------------------------

function FindReplaceDialog({ d }: { d: Extract<DialogState, { kind: "findReplace" }> }) {
  const ctl = useController();
  const options = useDataStore((s) => s.find.options);
  const patch = useDataStore.getState().patchFind;
  const [text, setText] = useState(d.text);
  const [replace, setReplace] = useState("");
  const ref = useAutoFocus();
  const setOpt = (o: Partial<typeof options>) => patch({ options: { ...options, ...o }, searched: null });
  const scopeNow = () => ({ sheet: ctl.sheet, ranges: selectionRanges(useSelectionStore.getState().selection) });
  // As AOO: "Current selection only" starts checked when several cells are selected.
  useEffect(() => {
    const ranges = selectionRanges(useSelectionStore.getState().selection);
    const multi = ranges.length > 1 || ranges.some((r) => r.r1 > r.r0 || r.c1 > r.c0);
    const o = useDataStore.getState().find.options;
    patch({ options: { ...o, scope: multi ? scopeNow() : null }, searched: null });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  return (
    <Shell
      title={t("dlg.findReplace", "Find & replace")}
      ok={t("dlg.replaceAll", "Replace all")}
      wide
      keepOpen
      cancelLabel={t("dialog.close", "Close")}
      testId="find-replace-dialog"
      valid={text !== ""}
      onSubmit={() => void ctl.replaceAll(text, replace)}
      extra={
        <>
          <button
            type="button"
            className={styles.secondary}
            disabled={text === ""}
            onClick={() => {
              patch({ text, searched: null, index: -1 });
              void ctl.findStep(1);
            }}
          >
            {t("cmd.edit.findNext", "Find next")}
          </button>
          <button type="button" className={styles.secondary} disabled={text === ""} onClick={() => void ctl.replaceOne(text, replace)} data-testid="replace-one">
            {t("dlg.replace", "Replace")}
          </button>
        </>
      }
    >
      <label className={styles.field}>
        <span>{t("dlg.find", "Find")}</span>
        <input ref={ref} className={styles.input} value={text} spellCheck={false} onChange={(e) => setText(e.target.value)} data-testid="replace-find" />
      </label>
      <label className={`${styles.field} ${styles.gap}`}>
        <span>{t("find.replaceWith", "Replace with")}</span>
        <input className={styles.input} value={replace} spellCheck={false} onChange={(e) => setReplace(e.target.value)} data-testid="replace-with" />
      </label>
      <div className={styles.optionGrid}>
        <label className={styles.checkRow}>
          <input type="checkbox" checked={options.matchCase} onChange={(e) => setOpt({ matchCase: e.target.checked })} />
          <span>{t("find.matchCase", "Match case")}</span>
        </label>
        <label className={styles.checkRow}>
          <input type="checkbox" checked={options.wholeCell} onChange={(e) => setOpt({ wholeCell: e.target.checked })} />
          <span>{t("findbar.wholeCell", "Whole cell")}</span>
        </label>
        <label className={styles.checkRow}>
          <input type="checkbox" checked={options.allSheets} disabled={!!options.scope} onChange={(e) => setOpt({ allSheets: e.target.checked })} />
          <span>{t("find.allSheets", "All sheets")}</span>
        </label>
        <label className={styles.checkRow}>
          <input type="checkbox" checked={!!options.scope} onChange={(e) => setOpt({ scope: e.target.checked ? scopeNow() : null })} data-testid="find-selection-only" />
          <span>{t("find.selectionOnly", "Current selection only")}</span>
        </label>
        <label className={styles.checkRow}>
          <span>{t("find.searchIn", "Search in")}</span>
          <select
            className={styles.selectSmall}
            value={searchIn(options)}
            onChange={(e) => setOpt({ inFormulas: e.target.value === "formulas", inNotes: e.target.value === "notes" })}
            data-testid="find-search-in"
          >
            <option value="formulas">{t("find.inFormulas", "Formulas")}</option>
            <option value="values">{t("find.inValues", "Values")}</option>
            <option value="notes">{t("find.inNotes", "Notes")}</option>
          </select>
        </label>
        <label className={styles.checkRow}>
          <span>{t("dlg.match", "Match")}</span>
          <select className={styles.selectSmall} value={options.mode} onChange={(e) => setOpt({ mode: e.target.value as typeof options.mode })}>
            <option value="literal">{t("dlg.modeText", "text")}</option>
            <option value="wildcards">{t("dlg.modeWildcards", "wildcards (* ?)")}</option>
            <option value="regex">{t("dlg.modeRegex", "regular expression")}</option>
          </select>
        </label>
      </div>
      {options.mode === "regex" ? <p className={styles.note}>{t("dlg.regexNote", "In the replacement, $1 is the first group and & the whole match.")}</p> : null}
    </Shell>
  );
}

// -- Paste Special ---------------------------------------------------------------------

const operations = (): { op: PasteOperation; label: string }[] => [
  { op: "none", label: t("paste.none", "None") },
  { op: "add", label: t("paste.addOp", "Add") },
  { op: "subtract", label: t("paste.subtract", "Subtract") },
  { op: "multiply", label: t("paste.multiply", "Multiply") },
  { op: "divide", label: t("paste.divide", "Divide") },
];

const kindLabels = (): Record<PasteKind, string> => ({
  text: t("paste.text", "Text"),
  numbers: t("paste.numbers", "Numbers"),
  dateTime: t("paste.dates", "Date and time"),
  formulas: t("paste.formulas", "Formulas"),
  notes: t("paste.notes", "Notes"),
  formats: t("paste.formats", "Formats"),
});

const shifts = (): { id: PasteShift; label: string }[] => [
  { id: "none", label: t("paste.dontShift", "Don't shift") },
  { id: "down", label: t("paste.down", "Down") },
  { id: "right", label: t("paste.right", "Right") },
];

/** The dialog's choices, kept for the session (AOO's static `nPreviousChecks` etc., inscodlg.cxx:40-44). */
interface PasteDialogState {
  all: boolean;
  kinds: PasteSelection;
  o: Omit<PasteSpecialChoice, "what" | "kinds">;
}

/** AOO's first-open state: Paste all off; Text, Numbers, Date & time on; no operation; nothing else. */
const FIRST_PASTE_DIALOG: PasteDialogState = {
  all: false,
  kinds: { text: true, numbers: true, dateTime: true, formulas: false, notes: false, formats: false },
  o: { transpose: false, skipEmpty: false, operation: "none", link: false, shift: "none" },
};
let lastPasteDialog: PasteDialogState = FIRST_PASTE_DIALOG;

/**
 * Paste Special (AOO's ScInsertContentsDlg): Selection (Paste all, or Text,
 * Numbers, Date and time, Formulas, Notes, Formats), Operations, Options
 * (Skip empty cells, Transpose, Link) and Shift cells, plus Tally's quick
 * presets (Everything / Values / Formats). The choices persist for the
 * session. Objects are not offered: Tally's clipboard carries no drawing
 * objects. Link needs a copy made in this document; text from outside
 * Tally only takes Formulas (off: `=…` stays text), Skip empty cells,
 * Transpose and Shift cells, so the rest is greyed out. Shift directions
 * that would push the block off the sheet are greyed out.
 */
function PasteSpecialDialog() {
  const ctl = useController();
  const clip = useDataStore((s) => s.clip);
  const active = useSelectionStore((s) => s.selection.active);
  const [all, setAll] = useState(lastPasteDialog.all);
  const [kinds, setKinds] = useState<PasteSelection>(lastPasteDialog.kinds);
  const [o, setO] = useState(lastPasteDialog.o);
  const [source, setSource] = useState<{ text: string | null; token: string | null } | null>(null);
  useEffect(() => {
    let live = true;
    void ctl.readClipboard().then((src) => {
      if (live) setSource(src);
    });
    return () => {
      live = false;
    };
  }, [ctl]);
  const selId = useId();
  const opId = useId();
  const optId = useId();
  const shiftId = useId();
  const external = source !== null && source.token === null && source.text !== null;
  const own = clip !== null && (source === null || source.token === clip.token);
  const canLink = own && !external;
  const block = own && clip ? { rows: clip.range.r1 - clip.range.r0 + 1, cols: clip.range.c1 - clip.range.c0 + 1 } : external && source?.text ? tsvDims(source.text) : null;
  const shiftOk = pasteShiftAllowed(active, block);
  const shift = o.shift === "down" && !shiftOk.down ? "none" : o.shift === "right" && !shiftOk.right ? "none" : o.shift;
  const choice: PasteSpecialChoice = { ...pasteWhat(all, kinds), ...o, link: canLink && o.link === true, shift, operation: external ? "none" : o.operation };
  const shown = all ? ALL_KINDS : kinds;
  const preset = (w: "all" | "values" | "formats") => {
    setAll(w === "all");
    setKinds(pasteKinds({ ...DEFAULT_PASTE_SPECIAL, what: w }));
  };
  const labels = kindLabels();
  return (
    <Shell
      title={t("dlg.pasteSpecial", "Paste special")}
      ok={t("cmd.edit.paste", "Paste")}
      wide
      testId="paste-special-dialog"
      valid={pasteChoiceValid(choice)}
      onSubmit={() => {
        lastPasteDialog = { all, kinds, o };
        void ctl.pasteSpecial(choice, source ?? undefined);
      }}
    >
      <div className={styles.segmented} role="radiogroup" aria-label={t("dlg.pastePresets", "Presets")}>
        {(
          [
            ["all", t("dlg.pasteEverything", "Everything")],
            ["values", t("find.values", "Values")],
            ["formats", t("paste.formats", "Formats")],
          ] as const
        ).map(([w, label]) => (
          <button key={w} type="button" role="radio" aria-checked={choice.what === w} className={choice.what === w ? styles.segOn : styles.seg} onClick={() => preset(w)} data-testid={`paste-${w}`}>
            {label}
          </button>
        ))}
      </div>
      <div className={fs.cols}>
        <fieldset className={fs.group} aria-labelledby={selId}>
          <legend id={selId} className={fs.legend}>
            {t("dd.selection", "Selection")}
          </legend>
          <label className={styles.checkRow}>
            <input type="checkbox" checked={all} onChange={(e) => setAll(e.target.checked)} data-testid="paste-kind-all" />
            <span>{t("paste.all", "Paste all")}</span>
          </label>
          {PASTE_KINDS.map((k) => (
            <label key={k} className={styles.checkRow}>
              <input type="checkbox" checked={shown[k]} disabled={all || (external && k !== "formulas")} onChange={(e) => setKinds({ ...kinds, [k]: e.target.checked })} data-testid={`paste-kind-${k}`} />
              <span>{labels[k]}</span>
            </label>
          ))}
        </fieldset>
        <fieldset className={fs.group} role="radiogroup" aria-labelledby={opId} disabled={external}>
          <legend id={opId} className={fs.legend}>
            {t("fillSheet.operations", "Operations")}
          </legend>
          {operations().map((x) => (
            <label key={x.op} className={`${styles.checkRow} ${fs.radioRow}`}>
              <input type="radio" name={opId} checked={choice.operation === x.op} onChange={() => setO({ ...o, operation: x.op })} data-testid={`paste-op-${x.op}`} />
              <span>{x.label}</span>
            </label>
          ))}
        </fieldset>
      </div>
      <div className={fs.cols}>
        <fieldset className={fs.group} aria-labelledby={optId}>
          <legend id={optId} className={fs.legend}>
            {t("dialog.options", "Options")}
          </legend>
          <label className={styles.checkRow}>
            <input type="checkbox" checked={o.skipEmpty} onChange={(e) => setO({ ...o, skipEmpty: e.target.checked })} data-testid="paste-skip-empty" />
            <span>{t("paste.skipEmpty", "Skip empty cells")}</span>
          </label>
          <label className={styles.checkRow}>
            <input type="checkbox" checked={o.transpose} onChange={(e) => setO({ ...o, transpose: e.target.checked })} data-testid="paste-transpose" />
            <span>{t("paste.transpose", "Transpose")}</span>
          </label>
          <label className={styles.checkRow} title={canLink ? undefined : t("paste.linkNeedsCopy", "Link needs cells copied in this document.")}>
            <input type="checkbox" checked={choice.link === true} disabled={!canLink} onChange={(e) => setO({ ...o, link: e.target.checked })} data-testid="paste-link" />
            <span>{t("paste.link", "Link")}</span>
          </label>
        </fieldset>
        <fieldset className={fs.group} role="radiogroup" aria-labelledby={shiftId}>
          <legend id={shiftId} className={fs.legend}>
            {t("paste.shiftCells", "Shift cells")}
          </legend>
          {shifts().map((x) => (
            <label key={x.id} className={`${styles.checkRow} ${fs.radioRow}`}>
              <input
                type="radio"
                name={shiftId}
                checked={(shift ?? "none") === x.id}
                disabled={(x.id === "down" && !shiftOk.down) || (x.id === "right" && !shiftOk.right)}
                onChange={() => setO({ ...o, shift: x.id })}
                data-testid={`paste-shift-${x.id}`}
              />
              <span>{x.label}</span>
            </label>
          ))}
        </fieldset>
      </div>
    </Shell>
  );
}

// -- Names -----------------------------------------------------------------------------

function nameError(name: string, scope: number | null, names: readonly NameInfo[], replacing: NameInfo | null): string | null {
  if (name === "") return null;
  if (!isValidName(name)) return t("dlg.nameInvalid", "Names start with a letter or _, use letters, digits, _ and ., and can't look like a cell (A1).");
  const clash = names.find((n) => n.name.toLowerCase() === name.toLowerCase() && n.scope === scope);
  if (clash && clash !== replacing && !(replacing && replacing.name === clash.name && replacing.scope === clash.scope)) return t("dlg.nameExists", "“{name}” already exists here; saving replaces it.", { name: clash.name });
  return null;
}

function DefineNameDialog({ d }: { d: Extract<DialogState, { kind: "defineName" }> }) {
  const ctl = useController();
  const names = useDataStore((s) => s.names);
  const sheets = useDocStore((s) => s.info?.sheets ?? []);
  const [name, setName] = useState(d.name);
  const [scope, setScope] = useState<number | null>(d.scope);
  const [refersTo, setRefersTo] = useState(d.refersTo);
  const ref = useAutoFocus();
  const err = nameError(name.trim(), scope, names, d.replacing);
  const hard = name.trim() !== "" && !isValidName(name.trim());
  return (
    <Shell
      title={d.replacing ? t("dlg.editName", "Edit name") : t("dlg.defineName", "Define name")}
      ok={d.replacing ? t("dialog.save", "Save") : t("dlg.define", "Define")}
      wide
      testId="define-name-dialog"
      valid={name.trim() !== "" && !hard && refersTo.trim() !== ""}
      onSubmit={() => {
        void ctl.defineName(name.trim(), scope, refersTo, d.replacing).then(() => {
          if (d.replacing) useUiStore.getState().openDialog({ kind: "manageNames" });
        });
      }}
    >
      <label className={styles.field}>
        <span>{t("dialog.name", "Name")}</span>
        <input ref={ref} className={styles.input} value={name} spellCheck={false} placeholder={t("dlg.nameExample", "e.g. TaxRate")} onChange={(e) => setName(e.target.value)} data-testid="name-name" />
      </label>
      {err ? <p className={hard ? styles.error : styles.note}>{err}</p> : null}
      <label className={`${styles.field} ${styles.gap}`}>
        <span>{t("names.scope", "Scope")}</span>
        <select className={styles.select} value={scope === null ? "" : String(scope)} onChange={(e) => setScope(e.target.value === "" ? null : Number(e.target.value))} data-testid="name-scope">
          <option value="">{t("dlg.workbook", "Workbook")}</option>
          {sheets.map((s) => (
            <option key={s.id} value={s.id}>
              {t("dlg.sheetOnly", "{name} only", { name: s.name })}
            </option>
          ))}
        </select>
      </label>
      <label className={`${styles.field} ${styles.gap}`}>
        <span>{t("dlg.refersTo", "Refers to")}</span>
        <input className={`${styles.input} ${styles.mono}`} value={refersTo} spellCheck={false} onChange={(e) => setRefersTo(e.target.value)} data-testid="name-refers" />
      </label>
      <p className={styles.note}>{t("dlg.refersNote", "A range like $Sheet1.$A$1:$B$5, or a constant like 0.07. Use the name in formulas: =SUM({name}).", { name: name.trim() || t("dialog.name", "Name") })}</p>
    </Shell>
  );
}

function ManageNamesDialog() {
  const ctl = useController();
  const names = useDataStore((s) => s.names);
  useEffect(() => {
    void ctl.refreshNames();
  }, [ctl]);
  const edit = (n: NameInfo) => useUiStore.getState().openDialog({ kind: "defineName", name: n.name, refersTo: n.refersTo, scope: n.scope, replacing: n });
  return (
    <Shell
      title={t("dlg.names", "Names")}
      ok={t("dlg.done", "Done")}
      wide
      testId="manage-names-dialog"
      onSubmit={() => undefined}
      extra={
        <button type="button" className={styles.secondary} onClick={ctl.openDefineName}>
          {t("dlg.new", "New…")}
        </button>
      }
    >
      {names.length === 0 ? (
        <p className={styles.note}>{t("dlg.noNames", "No names yet. Select a range and choose Data ▸ Define Name…, or type a name in the name box.")}</p>
      ) : (
        <div className={styles.table} role="table" aria-label={t("dlg.definedNames", "Defined names")}>
          {names.map((n) => (
            <div key={`${n.scope ?? "wb"}:${n.name}`} className={styles.tableRow} role="row" data-name={n.name}>
              <span className={styles.nameCell} role="cell">
                {n.name}
              </span>
              <span className={`${styles.refCell} ${styles.mono}`} role="cell" title={n.refersTo}>
                {n.refersTo}
              </span>
              <span className={styles.scopeCell} role="cell">
                {n.sheetName ?? t("dlg.workbook", "Workbook")}
              </span>
              <button type="button" className={styles.rowBtn} onClick={() => void ctl.goTo(n.name).then(() => close())} title={t("dlg.selectRange", "Select the range")}>
                {t("dlg.go", "Go")}
              </button>
              <button type="button" className={styles.rowBtn} onClick={() => edit(n)}>
                {t("dialog.edit", "Edit")}
              </button>
              <button type="button" className={`${styles.rowBtn} ${styles.dangerText}`} onClick={() => void ctl.deleteName(n)} data-testid={`delete-name-${n.name}`}>
                {t("dialog.delete", "Delete")}
              </button>
            </div>
          ))}
        </div>
      )}
    </Shell>
  );
}

// -- Confirm -----------------------------------------------------------------------------

function ConfirmDialog({ d }: { d: Extract<DialogState, { kind: "confirm" }> }) {
  return (
    <Shell
      title={d.title}
      ok={d.ok}
      danger={d.danger}
      testId="confirm-dialog"
      onSubmit={d.onConfirm}
      {...(d.cancel ? { cancelLabel: d.cancel } : {})}
      {...(d.alt && d.onAlt ? { alt: { label: d.alt, onClick: d.onAlt } } : {})}
      {...(d.focusOk ? { focusOk: true } : {})}
      {...(d.onCancel ? { onCancel: d.onCancel } : {})}
    >
      <p className={styles.lead}>{d.message}</p>
    </Shell>
  );
}
