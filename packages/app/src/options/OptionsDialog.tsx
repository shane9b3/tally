/**
 * Tools ▸ Options (⌘,): Calculate (document calc settings), View (per-sheet
 * view options), Input (AutoInput, AutoCorrect, Enter moves, Delete key),
 * Formula syntax, Sort lists (the document's custom lists), Load/Save (incl.
 * "Open documents in": tabs or separate windows), Language and Online Update
 * (AOO's page name: "Check for updates automatically", the installed version). Document settings go through ops (`setCalcSettings`,
 * `setViewOptions`, `setCustomSortLists`); the rest are app prefs.
 */
import type { CalcSettingsDto, ViewOptionsDto } from "@tally/bridge";
import { useState } from "react";
import dlg from "../components/Dialogs.module.css";
import { useController } from "../context";
import { t } from "../i18n";
import { AUTOCORRECT_OPTIONS } from "../autocorrect/autocorrect";
import { BUILTIN_SORT_LISTS, parseSortList } from "../data/builders";
import { useW56Store, type OptionsTab, type W56Dialog } from "../data/store";
import { Check, Field, Tabs, W56Shell } from "../data/W56Shell";
import w56 from "../data/W56.module.css";
import { isTwoDigitYearStart, isUndoSteps, TWO_DIGIT_YEAR_MAX, TWO_DIGIT_YEAR_MIN, UI_LANGUAGES, UNDO_STEPS_MAX, UNDO_STEPS_MIN, type AppPrefs } from "./prefs";
import { useV3Store } from "../stores/v3Store";
import { ViewTab } from "./ViewTab";
import { UpdateTab } from "./UpdateTab";

const TABS: readonly { id: OptionsTab; key: string; label: string }[] = [
  { id: "calculate", key: "options.calculate", label: "Calculate" },
  { id: "view", key: "options.view", label: "View" },
  { id: "input", key: "od.tabInput", label: "Input" },
  { id: "formula", key: "options.formulaSyntax", label: "Formula syntax" },
  { id: "sortLists", key: "od.tabSortLists", label: "Sort lists" },
  { id: "loadSave", key: "od.tabLoadSave", label: "Load / Save" },
  { id: "memory", key: "od.tabMemory", label: "Memory" },
  { id: "language", key: "options.language", label: "Language" },
  { id: "update", key: "options.onlineUpdate", label: "Online Update" },
];

export function OptionsDialog({ d }: { d: Extract<W56Dialog, { kind: "options" }> }) {
  const ctl = useController();
  useW56Store((s) => s.i18nRev);
  const prefs0 = useW56Store((s) => s.prefs);
  const [tab, setTab] = useState<OptionsTab>(d.tab);
  const [calc, setCalc] = useState<CalcSettingsDto>(d.calc);
  const [view, setView] = useState<ViewOptionsDto>(d.view);
  // View ▸ Page breaks (a session toggle; the document keeps AOO's `ShowPageBreaks` as read).
  const pageBreaks0 = useV3Store((s) => s.pageBreaksOn);
  const [pageBreaks, setPageBreaks] = useState(pageBreaks0);
  const [prefs, setPrefs] = useState<AppPrefs>(prefs0);
  const [lists, setLists] = useState<string[][]>(d.lists);
  const [listIndex, setListIndex] = useState<number | null>(d.lists.length > 0 ? 0 : null);
  const [draft, setDraft] = useState<string>(d.lists[0]?.join("\n") ?? "");
  const iter = calc.iterations;
  const decimals = calc.standardDecimals ?? null;
  // AOO's "Year (two digits)" field: shown as typed, kept while it is a valid year.
  const [yearText, setYearText] = useState(String(prefs0.twoDigitYearStart));
  const yearValid = isTwoDigitYearStart(Number(yearText));
  // AOO Memory ▸ Undo "Number of steps" (ED_UNDO, 1–1000): shown as typed, kept while valid.
  const [undoText, setUndoText] = useState(String(prefs0.undoSteps));
  const undoValid = isUndoSteps(Number(undoText));
  const calcValid =
    (iter === null || (Number.isInteger(iter.steps) && iter.steps >= 1 && iter.steps <= 32_767 && Number.isFinite(iter.minChange) && iter.minChange >= 0)) &&
    (decimals === null || (Number.isInteger(decimals) && decimals >= 0 && decimals <= 20)) &&
    yearValid &&
    undoValid;
  const submit = async () => {
    if (JSON.stringify(calc) !== JSON.stringify(d.calc)) await ctl.w56.setCalcSettings(calc);
    if (JSON.stringify(view) !== JSON.stringify(d.view)) await ctl.w56.setViewOptions(view);
    if (pageBreaks !== pageBreaks0) ctl.v3.setPageBreaksOn(pageBreaks);
    if (JSON.stringify(lists) !== JSON.stringify(d.lists)) await ctl.w56.setCustomLists(lists);
    if (JSON.stringify(prefs) !== JSON.stringify(prefs0)) await ctl.w56.savePrefs(prefs);
  };
  const commitDraft = (i: number | null, text: string) => {
    if (i === null) return;
    const entries = parseSortList(text);
    setLists(lists.map((l, j) => (j === i ? entries : l)));
  };
  return (
    <W56Shell title={t("options.title", "Options")} ok={t("dialog.ok", "OK")} size="xwide" testId="options-dialog" valid={calcValid} onSubmit={submit}>
      <Tabs label={t("options.title", "Options")} tabs={TABS.map((x) => ({ id: x.id, label: t(x.key, x.label) }))} value={tab} onChange={setTab} />
      {tab === "calculate" ? (
        <div>
          <Check label={t("options.iterations", "Iterations")} checked={iter !== null} onChange={(v) => setCalc({ ...calc, iterations: v ? { steps: 100, minChange: 0.001 } : null })} testId="opt-iterations" />
          <div className={w56.grid2}>
            <Field label={t("options.steps", "Steps")}>
              <input className={dlg.input} type="number" min={1} max={32767} disabled={iter === null} value={iter?.steps ?? 100} onChange={(e) => setCalc({ ...calc, iterations: { steps: Number(e.target.value), minChange: iter?.minChange ?? 0.001 } })} data-testid="opt-steps" />
            </Field>
            <Field label={t("options.minChange", "Minimum change")}>
              <input className={dlg.input} type="number" min={0} step="any" disabled={iter === null} value={iter?.minChange ?? 0.001} onChange={(e) => setCalc({ ...calc, iterations: { steps: iter?.steps ?? 100, minChange: Number(e.target.value) } })} data-testid="opt-min-change" />
            </Field>
            <Field label={t("options.dateEpoch", "Date")}>
              <select className={dlg.select} value={calc.dateEpoch} onChange={(e) => setCalc({ ...calc, dateEpoch: e.target.value as CalcSettingsDto["dateEpoch"] })} data-testid="opt-date-epoch">
                <option value="d1899">{t("od.epochDefault", "12/30/1899 (default)")}</option>
                <option value="d1900">01/01/1900 (StarCalc 1.0)</option>
                <option value="d1904">01/01/1904</option>
              </select>
            </Field>
            <Field label={t("od.searchCriteria", "Search criteria")}>
              <select className={dlg.select} value={calc.criteriaMode} onChange={(e) => setCalc({ ...calc, criteriaMode: e.target.value as CalcSettingsDto["criteriaMode"] })} data-testid="opt-criteria">
                <option value="wildcards">{t("options.wildcards", "Enable wildcards in formulas")}</option>
                <option value="regex">{t("options.regex", "Enable regular expressions in formulas")}</option>
                <option value="literal">{t("od.noWildcards", "No wildcards or regular expressions")}</option>
              </select>
            </Field>
            <Field label={t("od.textToNumber", "Conversion from text to number")}>
              <select className={dlg.select} value={calc.textToNumber} onChange={(e) => setCalc({ ...calc, textToNumber: e.target.value as CalcSettingsDto["textToNumber"] })} data-testid="opt-text-to-number">
                <option value="generateError">{t("od.generateError", "Generate #VALUE! error")}</option>
                <option value="treatAsZero">{t("od.treatAsZero", "Treat as zero")}</option>
                <option value="convertUnambiguous">{t("od.convertUnambiguous", "Convert only if unambiguous")}</option>
                <option value="convertLocale">{t("od.convertLocale", "Convert also locale dependent")}</option>
              </select>
            </Field>
          </div>
          <div className={dlg.optionGrid}>
            <Check label={t("options.caseSensitive", "Case sensitive")} checked={calc.caseSensitive} onChange={(v) => setCalc({ ...calc, caseSensitive: v })} testId="opt-case" />
            <Check label={t("options.precisionAsShown", "Precision as shown")} checked={calc.precisionAsShown} onChange={(v) => setCalc({ ...calc, precisionAsShown: v })} testId="opt-precision" />
            <Check label={t("options.wholeCellMatch", "Search criteria = and <> must apply to whole cells")} checked={calc.wholeCellMatch} onChange={(v) => setCalc({ ...calc, wholeCellMatch: v })} testId="opt-whole-cell" />
          </div>
          {/* AOO Calculate page: BTN_GENERAL_PREC / ED_PREC (0–20; off = unlimited). */}
          <div className={w56.grid2}>
            <Check label={t("od.limitDecimals", "Limit decimals for general number format")} checked={decimals !== null} onChange={(v) => setCalc({ ...calc, standardDecimals: v ? 0 : null })} testId="opt-limit-decimals" />
            <Field label={t("od.decimalPlaces", "Decimal places")}>
              <input className={dlg.input} type="number" min={0} max={20} step={1} disabled={decimals === null} value={decimals ?? 0} onChange={(e) => setCalc({ ...calc, standardDecimals: Number(e.target.value) })} data-testid="opt-decimal-places" />
            </Field>
          </div>
          {/* An app setting, not saved with the document (LibreOffice's CPU threading settings; AOO 4.1 has none). */}
          <div className={w56.section}>{t("od.cpuThreading", "CPU threading settings")}</div>
          <Check label={t("od.multithreading", "Enable multi-threaded calculation")} checked={prefs.calcMultithreading} onChange={(v) => setPrefs({ ...prefs, calcMultithreading: v })} testId="opt-multithreading" />
        </div>
      ) : null}
      {tab === "view" ? (
        <ViewTab view={view} setView={setView} vp={prefs.view} setVp={(v) => setPrefs({ ...prefs, view: { ...prefs.view, ...v } })} pageBreaks={pageBreaks} setPageBreaks={setPageBreaks} />
      ) : null}
      {tab === "input" ? (
        <div>
          <Check label={t("options.autoInput", "AutoInput")} checked={prefs.autoInput} onChange={(v) => setPrefs({ ...prefs, autoInput: v })} testId="opt-autoinput" />
          <Field label={t("od.enterMoves", "Press Enter to move selection")} className={dlg.gap}>
            <select className={dlg.select} value={prefs.enterMoves} onChange={(e) => setPrefs({ ...prefs, enterMoves: e.target.value as AppPrefs["enterMoves"] })} data-testid="opt-enter-moves">
              <option value="down">{t("od.down", "Down")}</option>
              <option value="right">{t("od.right", "Right")}</option>
              <option value="up">{t("od.up", "Up")}</option>
              <option value="left">{t("od.left", "Left")}</option>
            </select>
          </Field>
          <Check label={t("od.deleteKeyDialog", "Delete key opens the Delete Contents dialog (⌫ always clears at once)")} checked={prefs.deleteKeyDialog} onChange={(v) => setPrefs({ ...prefs, deleteKeyDialog: v })} testId="opt-delete-dialog" />
          <Check label={t("od.pasteOverwriteWarning", "Show overwrite warning when pasting data")} checked={prefs.pasteOverwriteWarning} onChange={(v) => setPrefs({ ...prefs, pasteOverwriteWarning: v })} testId="opt-paste-warning" />
          {/* AOO Tools ▸ Options ▸ General ▸ Year (two digits): FT_INTERPRET, NF_YEARVALUE (1583–9857), FT_TOYEAR. */}
          <div className={w56.section}>{t("od.twoDigitYear", "Year (two digits)")}</div>
          <div className={w56.row}>
            <label htmlFor="opt-year-start">{t("od.yearsBetween", "Interpret as years between")}</label>
            <input
              id="opt-year-start"
              className={dlg.input}
              style={{ width: 90 }}
              type="number"
              min={TWO_DIGIT_YEAR_MIN}
              max={TWO_DIGIT_YEAR_MAX}
              step={1}
              value={yearText}
              aria-invalid={!yearValid}
              onChange={(e) => {
                setYearText(e.target.value);
                const y = Number(e.target.value);
                if (isTwoDigitYearStart(y)) setPrefs({ ...prefs, twoDigitYearStart: y });
              }}
              data-testid="opt-year-start"
            />
            <span data-testid="opt-year-end">{t("od.yearsAnd", "and {year}", { year: yearValid ? String(Number(yearText) + 99) : "????" })}</span>
          </div>
          <p className={dlg.note}>{t("od.twoDigitYearNote", "Applies to documents created or opened from now on; a document saved with its own setting keeps it.")}</p>
          <div className={w56.section}>{t("options.autoCorrect", "AutoCorrect")}</div>
          {AUTOCORRECT_OPTIONS.map((o) => (
            <Check key={o.key} label={`${t(`od.ac.${o.key}`, o.label)} (${o.example})`} checked={prefs.autoCorrect[o.key]} onChange={(v) => setPrefs({ ...prefs, autoCorrect: { ...prefs.autoCorrect, [o.key]: v } })} testId={`opt-ac-${o.key}`} />
          ))}
        </div>
      ) : null}
      {tab === "formula" ? (
        <div>
          <Field label={t("options.functionSeparator", "Function")}>
            <select className={dlg.select} value={prefs.argumentSeparator} onChange={(e) => setPrefs({ ...prefs, argumentSeparator: e.target.value as ";" | "," })} data-testid="opt-separator">
              <option value=";">; (OpenOffice Calc A1)</option>
              <option value=",">, (Excel A1)</option>
            </select>
          </Field>
          <Check label={t("od.excelA1", "Show references in Excel A1 form (Sheet1!A1)")} checked={prefs.excelA1Display} onChange={(v) => setPrefs({ ...prefs, excelA1Display: v })} />
          <p className={dlg.note}>{t("od.formulaNote", "Display preference for the Function Wizard and the formula bar; the engine stores formulas in the OpenOffice dialect.")}</p>
        </div>
      ) : null}
      {tab === "sortLists" ? (
        <div style={{ display: "grid", gridTemplateColumns: "200px 1fr", gap: 12 }}>
          <div>
            <div className={w56.list} role="listbox" aria-label={t("od.lists", "Lists")} style={{ maxHeight: 200 }}>
              {BUILTIN_SORT_LISTS.map((l, i) => (
                <div key={`b${i}`} className={w56.listItem} title={t("od.builtIn", "Built in")}>
                  <span className={w56.grow}>{l.slice(0, 3).join(", ")}, …</span>
                </div>
              ))}
              {lists.map((l, i) => (
                <button key={i} type="button" role="option" aria-selected={listIndex === i} className={listIndex === i ? w56.listItemOn : w56.listItem} onClick={() => (commitDraft(listIndex, draft), setListIndex(i), setDraft(l.join("\n")))} data-testid={`sortlist-${i}`}>
                  <span className={w56.grow}>{l.length > 0 ? l.slice(0, 3).join(", ") + (l.length > 3 ? ", …" : "") : t("dp.empty", "(empty)")}</span>
                </button>
              ))}
            </div>
            <div className={w56.row}>
              <button type="button" className={dlg.secondary} onClick={() => (commitDraft(listIndex, draft), setLists([...lists, []]), setListIndex(lists.length), setDraft(""))} data-testid="sortlist-new">
                {t("od.new", "New")}
              </button>
              <button type="button" className={dlg.secondary} disabled={listIndex === null} onClick={() => (setLists(lists.filter((_, i) => i !== listIndex)), setListIndex(null), setDraft(""))} data-testid="sortlist-delete">
                {t("dialog.delete", "Delete")}
              </button>
            </div>
          </div>
          <Field label={t("od.entries", "Entries (one per line)")}>
            <textarea className={w56.textarea} style={{ minHeight: 180 }} disabled={listIndex === null} value={draft} onChange={(e) => setDraft(e.target.value)} onBlur={() => commitDraft(listIndex, draft)} data-testid="sortlist-entries" />
          </Field>
        </div>
      ) : null}
      {tab === "loadSave" ? (
        <div className={w56.grid2}>
          <Field label={t("od.defaultFormat", "Default file format")}>
            <select className={dlg.select} value={prefs.defaultFormat} onChange={(e) => setPrefs({ ...prefs, defaultFormat: e.target.value as AppPrefs["defaultFormat"] })} data-testid="opt-default-format">
              <option value="ods">{t("od.formatOds", "OpenDocument Spreadsheet (.ods)")}</option>
              <option value="xlsx">{t("od.formatXlsx", "Excel Workbook (.xlsx)")}</option>
            </select>
          </Field>
          <Field label={t("od.autosave", "Save AutoRecovery information every (minutes, 0 = off)")}>
            <input className={dlg.input} type="number" min={0} max={120} value={prefs.autosaveMinutes} onChange={(e) => setPrefs({ ...prefs, autosaveMinutes: Math.max(0, Math.min(120, Number(e.target.value) || 0)) })} data-testid="opt-autosave" />
          </Field>
          <Field label={t("options.openDocumentsIn", "Open documents in")}>
            <select className={dlg.select} value={prefs.openDocumentsIn} onChange={(e) => setPrefs({ ...prefs, openDocumentsIn: e.target.value as AppPrefs["openDocumentsIn"] })} data-testid="opt-open-documents-in">
              <option value="tabs">{t("options.openInTabs", "Tabs in one window")}</option>
              <option value="windows">{t("options.openInWindows", "Separate windows")}</option>
            </select>
          </Field>
          <p className={dlg.note} style={{ gridColumn: "1 / -1" }}>
            {t("options.openDocumentsInNote", "Applies to documents opened from now on; Window ▸ Merge All Windows combines open windows.")}
          </p>
        </div>
      ) : null}
      {tab === "memory" ? (
        <div>
          {/* AOO Tools ▸ Options ▸ OpenOffice ▸ Memory: GB_UNDO / FT_UNDO / ED_UNDO (cui/source/options/optmemory.src). */}
          <div className={w56.section}>{t("od.undo", "Undo")}</div>
          <div className={w56.row}>
            <label htmlFor="opt-undo-steps">{t("od.undoSteps", "Number of steps")}</label>
            <input
              id="opt-undo-steps"
              className={dlg.input}
              style={{ width: 90 }}
              type="number"
              min={UNDO_STEPS_MIN}
              max={UNDO_STEPS_MAX}
              step={1}
              value={undoText}
              aria-invalid={!undoValid}
              onChange={(e) => {
                setUndoText(e.target.value);
                const n = Number(e.target.value);
                if (isUndoSteps(n)) setPrefs({ ...prefs, undoSteps: n });
              }}
              data-testid="opt-undo-steps"
            />
          </div>
          <p className={dlg.note}>{t("od.undoStepsNote", "Applies to every open document at once; a lower number drops the oldest steps.")}</p>
        </div>
      ) : null}
      {tab === "update" ? <UpdateTab checked={prefs.checkForUpdates} onChange={(v) => setPrefs({ ...prefs, checkForUpdates: v })} /> : null}
      {tab === "language" ? (
        <div className={w56.grid2}>
          <Field label={t("od.userInterface", "User interface")}>
            <select className={dlg.select} value={prefs.uiLanguage} onChange={(e) => setPrefs({ ...prefs, uiLanguage: e.target.value as AppPrefs["uiLanguage"] })} data-testid="opt-ui-language">
              {UI_LANGUAGES.map((l) => (
                <option key={l.id} value={l.id}>
                  {l.label}
                </option>
              ))}
            </select>
          </Field>
          <Field label={t("od.localeNew", "Locale for new documents")}>
            <select className={dlg.select} value={prefs.defaultLocale} onChange={(e) => setPrefs({ ...prefs, defaultLocale: e.target.value })} data-testid="opt-default-locale">
              {["en-US", "en-GB", "de-DE", "fr-FR", "es-ES", "ja-JP"].map((tag) => (
                <option key={tag} value={tag}>
                  {tag}
                </option>
              ))}
            </select>
          </Field>
          <p className={dlg.note} style={{ gridColumn: "1 / -1" }}>{t("od.languageNote", "Changing the interface language re-labels menus, dialogs and panels at once. Function names follow the document locale (View ▸ Locale) and are translated for display only; files always store the English names.")}</p>
        </div>
      ) : null}
    </W56Shell>
  );
}
