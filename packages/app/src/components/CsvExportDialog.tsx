/**
 * CSV export prompts (csvExport.ts): AOO's "Keep current format" warning
 * (CSV keeps only the active sheet and no formatting) and its Text Export
 * dialog (field delimiter, text delimiter, "Quote all text cells", "Save
 * cell content as shown", character set). Built like SavePrompt: shared
 * dialog styles, focus trap, Escape cancels.
 */
import type { CsvCharset, CsvExportOptions } from "@tally/bridge";
import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { cx } from "../context";
import { CHARSETS, csvOptionsProblem, FIELD_DELIMITERS, useCsvPromptStore } from "../csvExport";
import { t } from "../i18n";
import { trapTab } from "./trapTab";
import dlg from "./Dialogs.module.css";
import styles from "./SavePrompt.module.css";

export function CsvExportPrompts() {
  const keep = useCsvPromptStore((s) => s.keep);
  const options = useCsvPromptStore((s) => s.options);
  return (
    <>
      {keep ? <KeepFormat file={keep.file} multiSheet={keep.multiSheet} sheet={keep.sheet} /> : null}
      {options ? <TextExport key={options.file} file={options.file} multiSheet={options.multiSheet} sheet={options.sheet} initial={options.initial} /> : null}
    </>
  );
}

function onKeys(form: HTMLFormElement | null, e: KeyboardEvent, cancel: () => void): void {
  e.stopPropagation();
  if (e.key === "Escape") {
    e.preventDefault();
    cancel();
    return;
  }
  if (form) trapTab(form, e.nativeEvent, document.activeElement);
}

/** "Only the active sheet “X” is saved." when the document has several sheets. */
function ActiveSheetNote({ multiSheet, sheet }: { multiSheet: boolean; sheet: string }) {
  if (!multiSheet) return null;
  return (
    <p className={dlg.note} data-testid="csv-active-sheet">
      {t("csv.activeSheetOnly", "Only the active sheet “{sheet}” is saved.", { sheet })}
    </p>
  );
}

function KeepFormat({ file, multiSheet, sheet }: { file: string; multiSheet: boolean; sheet: string }) {
  const answer = useCsvPromptStore((s) => s.answerKeep);
  const formRef = useRef<HTMLFormElement>(null);
  const keepRef = useRef<HTMLButtonElement>(null);
  useEffect(() => keepRef.current?.focus(), []);
  return (
    <div className={cx(dlg.overlay, styles.overlay)}>
      <form
        ref={formRef}
        className={cx(dlg.card, dlg.wide)}
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="csv-keep-title"
        aria-describedby="csv-keep-detail"
        data-testid="csv-keep-format"
        onKeyDown={(e) => onKeys(formRef.current, e, () => answer("cancel"))}
        onSubmit={(e) => {
          e.preventDefault();
          answer("keep");
        }}
      >
        <h2 id="csv-keep-title" className={cx(dlg.title, styles.title)}>
          {t("csv.keepTitle", "Keep the current format?")}
        </h2>
        <p id="csv-keep-detail" className={dlg.lead}>
          {t("csv.keepDetail", "“{file}” may lose formatting or content that cannot be saved in the Text CSV format. Use the default ODF format to be sure that the document is saved correctly.", { file })}
        </p>
        <ActiveSheetNote multiSheet={multiSheet} sheet={sheet} />
        <div className={dlg.actions}>
          <div className={dlg.extra}>
            <button type="button" className={dlg.secondary} onClick={() => answer("cancel")} data-testid="csv-keep-cancel">
              {t("dialog.cancel", "Cancel")}
            </button>
          </div>
          <button type="button" className={dlg.secondary} onClick={() => answer("odf")} data-testid="csv-keep-odf">
            {t("csv.useOdf", "Use ODF Format!")}
          </button>
          <button ref={keepRef} type="submit" className={dlg.primary} data-testid="csv-keep-keep">
            {t("csv.keepCurrent", "Keep Current Format")}
          </button>
        </div>
      </form>
    </div>
  );
}

function charsetLabel(c: CsvCharset): string {
  switch (c) {
    case "utf8":
      return t("csv.charset.utf8", "Unicode (UTF-8)");
    case "utf16":
      return t("csv.charset.utf16", "Unicode (UTF-16)");
    case "windows1252":
      return t("csv.charset.windows1252", "Western Europe (Windows-1252/WinLatin 1)");
    case "iso88591":
      return t("csv.charset.iso88591", "Western Europe (ISO-8859-1)");
  }
}

function delimiterLabel(d: string): string {
  switch (d) {
    case "\t":
      return t("csv.tab", "Tab");
    case " ":
      return t("csv.space", "Space");
    default:
      return d;
  }
}

function TextExport({ file, multiSheet, sheet, initial }: { file: string; multiSheet: boolean; sheet: string; initial: CsvExportOptions }) {
  const answer = useCsvPromptStore((s) => s.answerOptions);
  const formRef = useRef<HTMLFormElement>(null);
  const firstRef = useRef<HTMLSelectElement>(null);
  const [o, setO] = useState<CsvExportOptions>(initial);
  const named = (FIELD_DELIMITERS as readonly string[]).includes(o.fieldDelimiter);
  const [other, setOther] = useState(named ? "" : o.fieldDelimiter);
  const [useOther, setUseOther] = useState(!named);
  useEffect(() => firstRef.current?.focus(), []);
  const result: CsvExportOptions = { ...o, fieldDelimiter: useOther ? other : o.fieldDelimiter };
  const problem = csvOptionsProblem(result);
  const set = (p: Partial<CsvExportOptions>) => setO((x) => ({ ...x, ...p }));
  return (
    <div className={cx(dlg.overlay, styles.overlay)}>
      <form
        ref={formRef}
        className={cx(dlg.card, dlg.wide)}
        role="dialog"
        aria-modal="true"
        aria-labelledby="csv-export-title"
        data-testid="csv-export-dialog"
        onKeyDown={(e) => onKeys(formRef.current, e, () => answer(null))}
        onSubmit={(e) => {
          e.preventDefault();
          if (!problem) answer(result);
        }}
      >
        <h2 id="csv-export-title" className={dlg.title}>
          {t("csv.title", "Export Text File")}
        </h2>
        <p className={dlg.lead}>{file}</p>
        <ActiveSheetNote multiSheet={multiSheet} sheet={sheet} />
        <label className={dlg.field}>
          <span>{t("csv.charset", "Character set")}</span>
          <select ref={firstRef} className={dlg.select} value={o.charset} onChange={(e) => set({ charset: e.target.value as CsvCharset })} data-testid="csv-charset">
            {CHARSETS.map((c) => (
              <option key={c} value={c}>
                {charsetLabel(c)}
              </option>
            ))}
          </select>
        </label>
        <label className={cx(dlg.field, dlg.gap)}>
          <span>{t("csv.fieldDelimiter", "Field delimiter")}</span>
          <select
            className={dlg.select}
            value={useOther ? "other" : o.fieldDelimiter}
            onChange={(e) => {
              if (e.target.value === "other") setUseOther(true);
              else {
                setUseOther(false);
                set({ fieldDelimiter: e.target.value });
              }
            }}
            data-testid="csv-field-delimiter"
          >
            {FIELD_DELIMITERS.map((d) => (
              <option key={d} value={d}>
                {delimiterLabel(d)}
              </option>
            ))}
            <option value="other">{t("csv.other", "Other")}</option>
          </select>
        </label>
        {useOther ? (
          <label className={cx(dlg.field, dlg.gap)}>
            <span>{t("csv.otherDelimiter", "Other delimiter")}</span>
            <input className={dlg.input} value={other} maxLength={1} spellCheck={false} onChange={(e) => setOther(e.target.value)} data-testid="csv-other-delimiter" />
          </label>
        ) : null}
        <label className={cx(dlg.field, dlg.gap)}>
          <span>{t("csv.textDelimiter", "Text delimiter")}</span>
          <select className={dlg.select} value={o.textDelimiter} onChange={(e) => set({ textDelimiter: e.target.value })} data-testid="csv-text-delimiter">
            <option value={'"'}>"</option>
            <option value="'">'</option>
            <option value="">{t("csv.none", "None")}</option>
          </select>
        </label>
        <div className={dlg.optionGrid}>
          <label className={dlg.checkRow}>
            <input type="checkbox" checked={o.asShown} onChange={(e) => set({ asShown: e.target.checked })} data-testid="csv-as-shown" />
            <span>{t("csv.asShown", "Save cell content as shown")}</span>
          </label>
          <label className={dlg.checkRow}>
            <input type="checkbox" checked={o.quoteAllText} disabled={o.textDelimiter === ""} onChange={(e) => set({ quoteAllText: e.target.checked })} data-testid="csv-quote-all" />
            <span>{t("csv.quoteAllText", "Quote all text cells")}</span>
          </label>
        </div>
        {problem ? (
          <p className={dlg.error} role="alert">
            {problem === "same" ? t("csv.sameDelimiters", "The field and text delimiters must differ.") : t("csv.badDelimiter", "A delimiter must be a single character.")}
          </p>
        ) : null}
        <div className={dlg.actions}>
          <button type="button" className={dlg.secondary} onClick={() => answer(null)} data-testid="csv-export-cancel">
            {t("dialog.cancel", "Cancel")}
          </button>
          <button type="submit" className={dlg.primary} disabled={problem !== null} data-testid="csv-export-ok">
            {t("dialog.ok", "OK")}
          </button>
        </div>
      </form>
    </div>
  );
}
