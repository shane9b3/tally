/**
 * The dialog shell and small controls shared by the W5-6 dialogs: a modal
 * card (focus-trapped like `Dialogs.Shell`) that hides itself while the
 * user picks a range in the grid, `RangeField` (text + pick button),
 * `Check`, `Field`, `Tabs` and `Segmented`.
 */
import type { RangeDto } from "@tally/bridge";
import { useEffect, useRef, type ReactNode } from "react";
import { parseA1Range } from "../a1";
import { hexToRgba, rgbaToHex } from "../palette";
import { cx, useController } from "../context";
import { trapTab } from "../components/Dialogs";
import { a1 } from "../dataOps";
import { t } from "../i18n";
import dlg from "../components/Dialogs.module.css";
import styles from "./W56.module.css";
import { useW56Store } from "./store";

const FOCUSABLE = 'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

export function closeW56(): void {
  useW56Store.getState().openDialog(null);
}

export interface W56ShellProps {
  title: string;
  children: ReactNode;
  onSubmit: () => void | Promise<void>;
  ok?: string;
  valid?: boolean;
  size?: "normal" | "wide" | "xwide";
  danger?: boolean;
  testId?: string;
  extra?: ReactNode;
  keepOpen?: boolean;
  cancelLabel?: string;
  noOk?: boolean;
  /** Called when the dialog closes for any reason. */
  onClose?: () => void;
}

export function W56Shell({ title, children, onSubmit, ok, valid = true, size = "normal", danger, testId, extra, keepOpen, cancelLabel, noOk, onClose }: W56ShellProps) {
  const ctl = useController();
  const picking = useW56Store((s) => s.picking);
  const formRef = useRef<HTMLFormElement>(null);
  const cancel = () => {
    closeW56();
    onClose?.();
    ctl.focusGrid();
  };
  useEffect(() => {
    const form = formRef.current;
    if (!form || picking || form.contains(document.activeElement)) return;
    form.querySelector<HTMLElement>(FOCUSABLE)?.focus();
  }, [picking]);
  return (
    <div className={cx(dlg.overlay, picking && styles.pickingOverlay)} data-picking={picking ? "true" : undefined} onMouseDown={(e) => e.target === e.currentTarget && !picking && cancel()}>
      <form
        ref={formRef}
        className={cx(dlg.card, size === "wide" && dlg.wide, size === "xwide" && styles.xwide, picking && styles.pickingCard)}
        role="dialog"
        aria-modal="true"
        aria-label={title}
        data-testid={testId}
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            e.preventDefault();
            e.stopPropagation();
            cancel();
            return;
          }
          if (formRef.current) trapTab(formRef.current, e, document.activeElement);
        }}
        onSubmit={(e) => {
          e.preventDefault();
          if (!valid) return;
          if (!keepOpen) {
            closeW56();
            onClose?.();
          }
          void onSubmit();
          if (!keepOpen) ctl.focusGrid();
        }}
      >
        <h2 className={dlg.title}>{title}</h2>
        {picking ? <p className={styles.pickHint}>{t("wizard.select", "Select")}: drag over the cells, then press Enter (Esc cancels).</p> : null}
        {children}
        <div className={dlg.actions}>
          {extra ? <div className={dlg.extra}>{extra}</div> : null}
          <button type="button" className={dlg.secondary} onClick={cancel} data-testid="dialog-cancel">
            {cancelLabel ?? t("dialog.cancel", "Cancel")}
          </button>
          {noOk ? null : (
            <button type="submit" className={danger ? `${dlg.primary} ${dlg.dangerBtn}` : dlg.primary} disabled={!valid} data-testid="dialog-ok">
              {ok ?? t("dialog.ok", "OK")}
            </button>
          )}
        </div>
      </form>
    </div>
  );
}

export function Field({ label, children, className }: { label: string; children: ReactNode; className?: string }) {
  return (
    <label className={cx(dlg.field, className)}>
      <span>{label}</span>
      {children}
    </label>
  );
}

export function Check({ label, checked, onChange, testId, disabled }: { label: string; checked: boolean; onChange: (v: boolean) => void; testId?: string; disabled?: boolean }) {
  return (
    <label className={dlg.checkRow}>
      <input type="checkbox" checked={checked} disabled={disabled} onChange={(e) => onChange(e.target.checked)} data-testid={testId} />
      <span>{label}</span>
    </label>
  );
}

export function Tabs<T extends string>({ tabs, value, onChange, label }: { tabs: readonly { id: T; label: string }[]; value: T; onChange: (t: T) => void; label: string }) {
  return (
    <div className={styles.tabs} role="tablist" aria-label={label}>
      {tabs.map((tab) => (
        <button key={tab.id} type="button" role="tab" aria-selected={value === tab.id} className={value === tab.id ? styles.tabOn : styles.tab} onClick={() => onChange(tab.id)} data-testid={`tab-${tab.id}`}>
          {tab.label}
        </button>
      ))}
    </div>
  );
}

export function Segmented<T extends string>({ options, value, onChange, label }: { options: readonly { id: T; label: string }[]; value: T; onChange: (v: T) => void; label: string }) {
  return (
    <div className={dlg.segmentedInline} role="radiogroup" aria-label={label}>
      {options.map((o) => (
        <button key={o.id} type="button" role="radio" aria-checked={value === o.id} className={value === o.id ? dlg.segOn : dlg.seg} onClick={() => onChange(o.id)}>
          {o.label}
        </button>
      ))}
    </div>
  );
}

/** Text + "pick" button; `sheetPrefix` allows `Sheet.A1:B2` (validated by the caller). */
export function RangeField({ label, value, onChange, testId, placeholder, allowSheet }: { label: string; value: string; onChange: (v: string) => void; testId?: string; placeholder?: string; allowSheet?: boolean }) {
  const ctl = useController();
  const valid = value.trim() === "" || parseA1Range(value.trim().replace(/^[^.!]*[.!]/, "").replace(/\$/g, "")) !== null;
  const pick = async () => {
    const r = await ctl.w56.pickRange();
    if (r) onChange(a1(r));
    // Back to the dialog's field.
    setTimeout(() => document.querySelector<HTMLInputElement>(`[data-testid="${testId ?? ""}"]`)?.focus(), 0);
  };
  return (
    <label className={dlg.field}>
      <span>{label}</span>
      <div className={styles.rangeRow}>
        <input className={cx(dlg.input, dlg.mono, !valid && styles.invalid)} value={value} placeholder={placeholder ?? (allowSheet ? "Sheet1.A1:B5" : "A1:B5")} spellCheck={false} onChange={(e) => onChange(e.target.value)} data-testid={testId} />
        <button type="button" className={styles.pickBtn} title="Pick in the sheet" aria-label="Pick in the sheet" onClick={() => void pick()} data-testid={testId ? `${testId}-pick` : undefined}>
          ⌖
        </button>
      </div>
    </label>
  );
}

/** Parses a RangeField value (`A1:B5`, `$A$1`, optionally `Sheet.A1`) to a range on the current sheet. */
export function parseRangeText(text: string): RangeDto | null {
  return parseA1Range(text.trim().replace(/^[^.!]*[.!]/, "").replace(/\$/g, ""));
}

/** An inline colour field: a native colour input plus an optional "none" toggle. */
export function ColorField({ value, onChange, allowNone, testId }: { value: number | null; onChange: (rgba: number | null) => void; allowNone?: boolean; testId?: string }) {
  const hex = value === null ? "#000000" : rgbaToHex(value);
  return (
    <div className={styles.rangeRow}>
      <input type="color" className={styles.pickBtn} style={{ width: 44, padding: 2 }} value={hex} aria-label="Colour" onChange={(e) => onChange(hexToRgba(e.target.value))} data-testid={testId} />
      <input className={cx(dlg.input, dlg.mono)} value={value === null ? "" : hex} placeholder={allowNone ? "none" : "#rrggbb"} spellCheck={false} onChange={(e) => {
        const v = e.target.value.trim();
        if (v === "" && allowNone) onChange(null);
        else {
          const rgba = hexToRgba(v);
          if (rgba !== null) onChange(rgba);
        }
      }} />
      {allowNone ? (
        <button type="button" className={styles.pickBtn} title="No colour" aria-label="No colour" onClick={() => onChange(null)}>
          ✕
        </button>
      ) : null}
    </div>
  );
}
