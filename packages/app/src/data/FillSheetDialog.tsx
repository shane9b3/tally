/**
 * Edit ▸ Fill ▸ Sheets… (AOO's ScInsertContentsDlg in fill mode): copies
 * the selection of the active sheet to the same cells of the other
 * selected sheets, choosing what to copy (Paste all, or Text, Numbers,
 * Date & time, Formulas, Notes, Formats), an operation with the target
 * numbers and Skip empty cells. Link, Transpose and Shift cells are not
 * offered, as in AOO. OK sends one `fillSheets` op (one undo step).
 *
 * Opened through `useUiStore().openDialog({ kind: "fillSheet" })`; loaded
 * lazily from `Dialogs.tsx` (see `lazyChunks.ts`).
 */
import type { FillSheetOptions, Op, PasteOperation, RangeDto } from "@tally/bridge";
import { useId, useState } from "react";
import { Shell } from "../components/Dialogs";
import dlg from "../components/Dialogs.module.css";
import { useController } from "../context";
import { a1 } from "../dataOps";
import { t } from "../i18n";
import { useDocStore } from "../stores/docStore";
import { selectionRanges, useSelectionStore } from "../stores/selectionStore";
import { Check } from "./W56Shell";
import { useW56Store } from "./store";
import styles from "./FillSheetDialog.module.css";

type Kind = "text" | "numbers" | "dateTime" | "formulas" | "notes" | "formats";

/** The content kinds in AOO's order, with the Paste Special labels. */
const KINDS = (): { key: Kind; label: string }[] => [
  { key: "text", label: t("paste.text", "Text") },
  { key: "numbers", label: t("paste.numbers", "Numbers") },
  { key: "dateTime", label: t("paste.dates", "Date and time") },
  { key: "formulas", label: t("paste.formulas", "Formulas") },
  { key: "notes", label: t("paste.notes", "Notes") },
  { key: "formats", label: t("paste.formats", "Formats") },
];

const OPERATIONS = (): { op: PasteOperation; label: string }[] => [
  { op: "none", label: t("paste.none", "None") },
  { op: "add", label: t("paste.addOp", "Add") },
  { op: "subtract", label: t("paste.subtract", "Subtract") },
  { op: "multiply", label: t("paste.multiply", "Multiply") },
  { op: "divide", label: t("paste.divide", "Divide") },
];

/** AOO's defaults: Paste all, no operation, empty cells copied. */
export const DEFAULT_FILL_SHEET: FillSheetOptions = { text: true, numbers: true, dateTime: true, formulas: true, notes: true, formats: true, operation: "none", skipEmpty: false };

/** The `fillSheets` op for the active sheet's selection, or null with no other sheet selected. */
export function fillSheetsOp(sheet: number, group: readonly number[], ranges: RangeDto[], options: FillSheetOptions): Op | null {
  const to = group.filter((id) => id !== sheet);
  if (to.length === 0 || ranges.length === 0) return null;
  return { type: "fillSheets", sheet, ranges, to, options };
}

export function FillSheetDialog() {
  useW56Store((s) => s.i18nRev);
  const ctl = useController();
  const group = useDocStore((s) => s.groupSheets);
  const sheets = useDocStore((s) => s.info?.sheets ?? []);
  const sheet = ctl.sheet;
  const [all, setAll] = useState(true);
  const [o, setO] = useState<FillSheetOptions>(DEFAULT_FILL_SHEET);
  const selId = useId();
  const opId = useId();
  const optId = useId();
  const ranges = selectionRanges(useSelectionStore.getState().selection);
  const to = group.filter((id) => id !== sheet);
  const names = to.map((id) => sheets.find((s) => s.id === id)?.name ?? String(id)).join(", ");
  const flags: FillSheetOptions = all ? { ...o, text: true, numbers: true, dateTime: true, formulas: true, notes: true, formats: true } : o;
  const any = KINDS().some((k) => flags[k.key]);
  return (
    <Shell
      title={t("fillSheet.title", "Fill Sheet")}
      ok={t("dialog.ok", "OK")}
      testId="fill-sheet-dialog"
      valid={any && to.length > 0}
      onSubmit={() => {
        const op = fillSheetsOp(sheet, group, ranges, flags);
        if (op) void ctl.apply(t("msg.undo.fillSheet", "Fill sheets"), [op]);
      }}
    >
      <p className={dlg.lead}>
        {to.length > 0
          ? t("fillSheet.lead", "Copies {ranges} to the same cells of {sheets}.", { ranges: ranges.map(a1).join(", "), sheets: names })
          : t("fillSheet.needGroup", "Select two or more sheets first (⌘-click their tabs).")}
      </p>
      <div className={styles.cols}>
        <fieldset className={styles.group} aria-labelledby={selId}>
          <legend id={selId} className={styles.legend}>
            {t("dd.selection", "Selection")}
          </legend>
          <Check label={t("paste.all", "Paste all")} checked={all} onChange={setAll} testId="fillsheet-all" />
          {KINDS().map((k) => (
            <Check key={k.key} label={k.label} checked={flags[k.key]} disabled={all} onChange={(v) => setO({ ...o, [k.key]: v })} testId={`fillsheet-${k.key}`} />
          ))}
        </fieldset>
        <fieldset className={styles.group} role="radiogroup" aria-labelledby={opId}>
          <legend id={opId} className={styles.legend}>
            {t("fillSheet.operations", "Operations")}
          </legend>
          {OPERATIONS().map((x) => (
            <label key={x.op} className={`${dlg.checkRow} ${styles.radioRow}`}>
              <input type="radio" name={opId} checked={o.operation === x.op} onChange={() => setO({ ...o, operation: x.op })} data-testid={`fillsheet-op-${x.op}`} />
              <span>{x.label}</span>
            </label>
          ))}
        </fieldset>
      </div>
      <fieldset className={styles.group} aria-labelledby={optId}>
        <legend id={optId} className={styles.legend}>
          {t("dialog.options", "Options")}
        </legend>
        <Check label={t("paste.skipEmpty", "Skip empty cells")} checked={o.skipEmpty} onChange={(v) => setO({ ...o, skipEmpty: v })} testId="fillsheet-skip-empty" />
      </fieldset>
    </Shell>
  );
}
