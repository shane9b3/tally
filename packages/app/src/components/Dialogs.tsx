/** Small modal dialogs: Row height / Column width, Insert N rows/columns, Custom number format. */
import { useEffect, useRef, useState, type ReactNode } from "react";
import { useController } from "../context";
import { NUMBER_FORMATS } from "../formats";
import { formatNumberLite } from "../numfmtLite";
import { useDocStore } from "../stores/docStore";
import { useSelectionStore } from "../stores/selectionStore";
import { useUiStore, type DialogState } from "../stores/uiStore";
import styles from "./Dialogs.module.css";

export function Dialogs() {
  const dialog = useUiStore((s) => s.dialog);
  if (!dialog) return null;
  return <DialogFor dialog={dialog} />;
}

function DialogFor({ dialog }: { dialog: DialogState }) {
  switch (dialog.kind) {
    case "resize":
      return <ResizeDialog d={dialog} />;
    case "insert":
      return <InsertDialog d={dialog} />;
    case "numberFormat":
      return <NumberFormatDialog d={dialog} />;
  }
}

function close(): void {
  useUiStore.getState().openDialog(null);
}

interface ShellProps {
  title: string;
  children: ReactNode;
  onSubmit: () => void;
  ok?: string;
  valid?: boolean;
  wide?: boolean;
}

function Shell({ title, children, onSubmit, ok = "Apply", valid = true, wide }: ShellProps) {
  const ctl = useController();
  const cancel = () => {
    close();
    ctl.focusGrid();
  };
  return (
    <div className={styles.overlay} onMouseDown={(e) => e.target === e.currentTarget && cancel()}>
      <form
        className={wide ? `${styles.card} ${styles.wide}` : styles.card}
        role="dialog"
        aria-label={title}
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            e.preventDefault();
            e.stopPropagation();
            cancel();
          }
        }}
        onSubmit={(e) => {
          e.preventDefault();
          if (!valid) return;
          close();
          onSubmit();
          ctl.focusGrid();
        }}
      >
        <h2 className={styles.title}>{title}</h2>
        {children}
        <div className={styles.actions}>
          <button type="button" className={styles.secondary} onClick={cancel}>
            Cancel
          </button>
          <button type="submit" className={styles.primary} disabled={!valid}>
            {ok}
          </button>
        </div>
      </form>
    </div>
  );
}

function useAutoFocus() {
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
