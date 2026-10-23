/**
 * Small modal dialogs: Row height / Column width, Insert N rows/columns,
 * Custom number format, Sort…, Find & Replace, Paste Special, Define Name,
 * Manage Names and a generic confirmation.
 */
import type { NameInfo, PasteOperation, SortKey } from "@tally/bridge";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { useController } from "../context";
import { a1, DEFAULT_PASTE_SPECIAL, isValidName, type PasteSpecialChoice } from "../dataOps";
import { useDataStore } from "../stores/dataStore";
import { NUMBER_FORMATS } from "../formats";
import { formatNumberLite } from "../numfmtLite";
import { useDocStore } from "../stores/docStore";
import { useSelectionStore } from "../stores/selectionStore";
import { useUiStore, type DialogState } from "../stores/uiStore";
import styles from "./Dialogs.module.css";
import { V3Dialog, isV3Dialog } from "./V3Dialogs";
import { FunctionWizard } from "./FunctionWizard";
import { FOCUSABLE, trapTab } from "./trapTab";
import { arrowNav, useReturnFocus, useTitleId } from "./dialogA11y";

export function Dialogs() {
  const dialog = useUiStore((s) => s.dialog);
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
  }
}

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
  /** Extra class on the card (larger dialogs). */
  cardClass?: string;
  /** Hide the primary button (dialogs with their own actions). */
  noOk?: boolean;
}

export function Shell({ title, children, onSubmit, ok = "Apply", valid = true, wide, danger, testId, extra, keepOpen, cancelLabel = "Cancel", cardClass, noOk }: ShellProps) {
  const ctl = useController();
  const formRef = useRef<HTMLFormElement>(null);
  const titleId = useTitleId();
  useReturnFocus();
  const cancel = () => {
    close();
    ctl.focusGrid();
  };
  // Modal: focus starts inside the dialog (the first field, else the first button).
  useEffect(() => {
    const form = formRef.current;
    if (!form || form.contains(document.activeElement)) return;
    form.querySelector<HTMLElement>(FOCUSABLE)?.focus();
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
          <button type="button" className={styles.secondary} onClick={cancel} data-testid="dialog-cancel">
            {cancelLabel}
          </button>
          {noOk ? null : (
            <button type="submit" className={danger ? `${styles.primary} ${styles.dangerBtn}` : styles.primary} disabled={!valid} data-testid="dialog-ok">
              {ok}
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
  const what = d.axis === "row" ? "Row height" : "Column width";
  return (
    <Shell title={what} valid={valid} onSubmit={() => ctl.resize(d.axis, d.start, d.end, Math.round(n))}>
      <label className={styles.field}>
        <span>{what} in pixels</span>
        <input ref={ref} className={styles.input} inputMode="numeric" value={px} onChange={(e) => setPx(e.target.value)} />
      </label>
      <p className={styles.note}>
        {d.end > d.start ? `Applies to ${d.end - d.start + 1} ${d.axis === "row" ? "rows" : "columns"}.` : "Default: "}
        {d.end > d.start ? "" : d.axis === "row" ? "24 px" : "100 px"}
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
      title={rows ? "Insert rows" : "Insert columns"}
      ok="Insert"
      valid={valid}
      onSubmit={() => void (rows ? ctl.insertRows(where, n) : ctl.insertCols(where, n))}
    >
      <label className={styles.field}>
        <span>How many</span>
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
            {rows ? (w === "before" ? "Above" : "Below") : w === "before" ? "Left" : "Right"}
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
    <Shell title="Custom number format" wide onSubmit={() => void ctl.setNumberFormat(code.trim() || "General", "Number format")}>
      <label className={styles.field}>
        <span>Format code</span>
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
          <span className={styles.note}>No preview for this code; the engine will format it when applied.</span>
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
  const label = (c: number) => (hasHeader ? (d.labels[c - d.range.c0] ?? "") : `Column ${a1({ r0: 0, c0: c, r1: 0, c1: c }).replace(/\d+$/, "")}`);
  const setKey = (i: number, k: SortKey | null) => setKeys((ks) => ks.map((x, j) => (j === i ? k : x)));
  const chosen = keys.filter((k): k is SortKey => k !== null);
  const range = hasHeader ? `${a1(d.range)} (header row kept)` : a1(d.range);
  return (
    <Shell title="Sort" ok="Sort" wide testId="sort-dialog" valid={chosen.length > 0} onSubmit={() => void ctl.sortWith({ range: d.range, keys: chosen, hasHeader, caseSensitive })}>
      <p className={styles.lead}>Sorts {range}. Whole rows move together.</p>
      {keys.map((k, i) => (
        <div key={i} className={styles.sortRow}>
          <span className={styles.sortLabel}>{i === 0 ? "Sort by" : "Then by"}</span>
          <select
            className={styles.select}
            aria-label={i === 0 ? "Sort by" : `Then by ${i}`}
            value={k ? String(k.col) : ""}
            disabled={i > 0 && keys[i - 1] === null}
            onChange={(e) => setKey(i, e.target.value === "" ? null : { col: Number(e.target.value), ascending: k?.ascending ?? true })}
            data-testid={`sort-key-${i}`}
          >
            {i > 0 ? <option value="">(none)</option> : null}
            {cols.map((c) => (
              <option key={c} value={c}>
                {label(c)}
              </option>
            ))}
          </select>
          <div className={styles.segmentedInline} role="radiogroup" aria-label="Order">
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
        <span>Range contains column labels</span>
      </label>
      <label className={styles.checkRow}>
        <input type="checkbox" checked={caseSensitive} onChange={(e) => setCase(e.target.checked)} />
        <span>Case sensitive</span>
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
  return (
    <Shell
      title="Find & replace"
      ok="Replace all"
      wide
      keepOpen
      cancelLabel="Close"
      testId="find-replace-dialog"
      valid={text !== ""}
      onSubmit={() => void ctl.replaceAll(text, replace)}
      extra={
        <button
          type="button"
          className={styles.secondary}
          disabled={text === ""}
          onClick={() => {
            patch({ text, searched: null, index: -1 });
            void ctl.findStep(1);
          }}
        >
          Find next
        </button>
      }
    >
      <label className={styles.field}>
        <span>Find</span>
        <input ref={ref} className={styles.input} value={text} spellCheck={false} onChange={(e) => setText(e.target.value)} data-testid="replace-find" />
      </label>
      <label className={`${styles.field} ${styles.gap}`}>
        <span>Replace with</span>
        <input className={styles.input} value={replace} spellCheck={false} onChange={(e) => setReplace(e.target.value)} data-testid="replace-with" />
      </label>
      <div className={styles.optionGrid}>
        <label className={styles.checkRow}>
          <input type="checkbox" checked={options.matchCase} onChange={(e) => setOpt({ matchCase: e.target.checked })} />
          <span>Match case</span>
        </label>
        <label className={styles.checkRow}>
          <input type="checkbox" checked={options.wholeCell} onChange={(e) => setOpt({ wholeCell: e.target.checked })} />
          <span>Whole cell</span>
        </label>
        <label className={styles.checkRow}>
          <input type="checkbox" checked={options.allSheets} onChange={(e) => setOpt({ allSheets: e.target.checked })} />
          <span>All sheets</span>
        </label>
        <label className={styles.checkRow}>
          <span>Match</span>
          <select className={styles.selectSmall} value={options.mode} onChange={(e) => setOpt({ mode: e.target.value as typeof options.mode })}>
            <option value="literal">text</option>
            <option value="wildcards">wildcards (* ?)</option>
            <option value="regex">regular expression</option>
          </select>
        </label>
      </div>
      {options.mode === "regex" ? <p className={styles.note}>In the replacement, $1 is the first group and &amp; the whole match.</p> : null}
    </Shell>
  );
}

// -- Paste Special ---------------------------------------------------------------------

const OPERATIONS: { op: PasteOperation; label: string }[] = [
  { op: "none", label: "None" },
  { op: "add", label: "Add" },
  { op: "subtract", label: "Subtract" },
  { op: "multiply", label: "Multiply" },
  { op: "divide", label: "Divide" },
];

function PasteSpecialDialog() {
  const ctl = useController();
  const [c, setC] = useState<PasteSpecialChoice>(DEFAULT_PASTE_SPECIAL);
  const set = (p: Partial<PasteSpecialChoice>) => setC((x) => ({ ...x, ...p }));
  return (
    <Shell title="Paste special" ok="Paste" testId="paste-special-dialog" onSubmit={() => void ctl.pasteSpecial(c)}>
      <div className={styles.segmented} role="radiogroup" aria-label="Paste">
        {(
          [
            ["all", "Everything"],
            ["values", "Values"],
            ["formats", "Formats"],
          ] as const
        ).map(([w, label]) => (
          <button key={w} type="button" role="radio" aria-checked={c.what === w} className={c.what === w ? styles.segOn : styles.seg} onClick={() => set({ what: w })} data-testid={`paste-${w}`}>
            {label}
          </button>
        ))}
      </div>
      <label className={styles.checkRow}>
        <input type="checkbox" checked={c.transpose} onChange={(e) => set({ transpose: e.target.checked })} data-testid="paste-transpose" />
        <span>Transpose (rows become columns)</span>
      </label>
      <label className={styles.checkRow}>
        <input type="checkbox" checked={c.skipEmpty} onChange={(e) => set({ skipEmpty: e.target.checked })} />
        <span>Skip empty cells</span>
      </label>
      <label className={`${styles.field} ${styles.gap}`}>
        <span>Operation with the cells pasted onto</span>
        <select className={styles.select} value={c.operation} disabled={c.what === "formats"} onChange={(e) => set({ operation: e.target.value as PasteOperation })} data-testid="paste-operation">
          {OPERATIONS.map((o) => (
            <option key={o.op} value={o.op}>
              {o.label}
            </option>
          ))}
        </select>
      </label>
    </Shell>
  );
}

// -- Names -----------------------------------------------------------------------------

function nameError(name: string, scope: number | null, names: readonly NameInfo[], replacing: NameInfo | null): string | null {
  if (name === "") return null;
  if (!isValidName(name)) return "Names start with a letter or _, use letters, digits, _ and ., and can't look like a cell (A1).";
  const clash = names.find((n) => n.name.toLowerCase() === name.toLowerCase() && n.scope === scope);
  if (clash && clash !== replacing && !(replacing && replacing.name === clash.name && replacing.scope === clash.scope)) return `“${clash.name}” already exists here; saving replaces it.`;
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
      title={d.replacing ? "Edit name" : "Define name"}
      ok={d.replacing ? "Save" : "Define"}
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
        <span>Name</span>
        <input ref={ref} className={styles.input} value={name} spellCheck={false} placeholder="e.g. TaxRate" onChange={(e) => setName(e.target.value)} data-testid="name-name" />
      </label>
      {err ? <p className={hard ? styles.error : styles.note}>{err}</p> : null}
      <label className={`${styles.field} ${styles.gap}`}>
        <span>Scope</span>
        <select className={styles.select} value={scope === null ? "" : String(scope)} onChange={(e) => setScope(e.target.value === "" ? null : Number(e.target.value))} data-testid="name-scope">
          <option value="">Workbook</option>
          {sheets.map((s) => (
            <option key={s.id} value={s.id}>
              {s.name} only
            </option>
          ))}
        </select>
      </label>
      <label className={`${styles.field} ${styles.gap}`}>
        <span>Refers to</span>
        <input className={`${styles.input} ${styles.mono}`} value={refersTo} spellCheck={false} onChange={(e) => setRefersTo(e.target.value)} data-testid="name-refers" />
      </label>
      <p className={styles.note}>A range like $Sheet1.$A$1:$B$5, or a constant like 0.07. Use the name in formulas: =SUM({name.trim() || "Name"}).</p>
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
      title="Names"
      ok="Done"
      wide
      testId="manage-names-dialog"
      onSubmit={() => undefined}
      extra={
        <button type="button" className={styles.secondary} onClick={ctl.openDefineName}>
          New…
        </button>
      }
    >
      {names.length === 0 ? (
        <p className={styles.note}>No names yet. Select a range and choose Data ▸ Define Name…, or type a name in the name box.</p>
      ) : (
        <div className={styles.table} role="table" aria-label="Defined names">
          {names.map((n) => (
            <div key={`${n.scope ?? "wb"}:${n.name}`} className={styles.tableRow} role="row" data-name={n.name}>
              <span className={styles.nameCell} role="cell">
                {n.name}
              </span>
              <span className={`${styles.refCell} ${styles.mono}`} role="cell" title={n.refersTo}>
                {n.refersTo}
              </span>
              <span className={styles.scopeCell} role="cell">
                {n.sheetName ?? "Workbook"}
              </span>
              <button type="button" className={styles.rowBtn} onClick={() => void ctl.goTo(n.name).then(() => close())} title="Select the range">
                Go
              </button>
              <button type="button" className={styles.rowBtn} onClick={() => edit(n)}>
                Edit
              </button>
              <button type="button" className={`${styles.rowBtn} ${styles.dangerText}`} onClick={() => void ctl.deleteName(n)} data-testid={`delete-name-${n.name}`}>
                Delete
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
    <Shell title={d.title} ok={d.ok} danger={d.danger} testId="confirm-dialog" onSubmit={d.onConfirm}>
      <p className={styles.lead}>{d.message}</p>
    </Shell>
  );
}
