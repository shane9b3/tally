/**
 * Format Cells ▸ Numbers (AOO SvxNumberFormatTabPage): Category and Format
 * lists, Language (and the currency list in the Currency category), the
 * Options (decimal places, leading zeros, negative numbers red, thousands
 * separator), the Format code field and a preview of the cell's value.
 * The model is `numberFormat.ts`; the code is the one source of truth and
 * the options are read back from it on every render.
 */
import { useMemo, useState } from "react";
import dlg from "../components/Dialogs.module.css";
import { SimpleList } from "../components/FontPickers";
import { cx } from "../context";
import { useW56Store } from "../data/store";
import { Check, Field } from "../data/W56Shell";
import w56 from "../data/W56.module.css";
import { t } from "../i18n";
import { useV3Store } from "../stores/v3Store";
import css from "./NumbersTab.module.css";
import {
  CURRENCIES,
  MAX_DECIMALS,
  MAX_LEADING,
  NF_CATEGORIES,
  NF_LANGUAGES,
  analyzeFormat,
  builtinCodes,
  categoryDefault,
  currencyById,
  generateFormat,
  isPredefined,
  languageByLcid,
  languageOf,
  DEFAULT_NUMVALUE,
  listEntry,
  optionsEnabled,
  predefinedFormats,
  previewColor,
  previewText,
  withLanguage,
  type NfCategory,
  type NfOptions,
} from "./numberFormat";

export interface NumbersTabProps {
  code: string;
  onChange(code: string): void;
}

/** The dialog's own state besides the code (AOO keeps the same in SvxNumberFormatShell). */
export interface NumbersUiState {
  category: NfCategory;
  /** Hex locale id, `null` for Default. */
  language: string | null;
  currencyId: string;
}

/** The initial UI state for a cell's code (category, language and currency of the format). */
export function initialNumbersState(code: string, docTag: string): NumbersUiState {
  const a = analyzeFormat(code);
  const doc = languageOf(docTag);
  return { category: a.category, language: a.language, currencyId: a.currency?.id ?? doc.currency };
}

/** The format list of a category: predefined codes, then the user codes of that type. */
export function formatList(ui: NumbersUiState, code: string, docTag: string, userCodes: readonly string[]): string[] {
  const doc = languageOf(docTag);
  const currency = currencyById(ui.currencyId);
  const users = [...userCodes, code].filter((c, i, xs) => c.trim() !== "" && xs.indexOf(c) === i && !isPredefined(c, ui.language, doc, currency));
  if (ui.category === "user") return users;
  const pre = predefinedFormats(ui.category, ui.language, doc, currency);
  const own = ui.category === "all" ? users : users.filter((c) => analyzeFormat(c).category === ui.category);
  return [...pre, ...own.filter((c) => !pre.some((p) => p.toLowerCase() === c.toLowerCase()))];
}

/** The code after picking a category: unchanged when it already is one (or All / User-defined), else the category's standard format. */
export function codeForCategory(cat: NfCategory, ui: NumbersUiState, code: string, docTag: string): string {
  if (cat === "all" || cat === "user" || analyzeFormat(code).category === cat) return code;
  return categoryDefault(cat, ui.language, languageOf(docTag), currencyById(ui.currencyId)) ?? code;
}

/** The code after an option change (GenerateFormat), or `null` where the options do not apply. */
export function codeForOptions(ui: NumbersUiState, code: string, options: NfOptions): string | null {
  const a = analyzeFormat(code);
  const cat = ui.category === "all" || ui.category === "user" ? a.category : ui.category;
  const currency = cat === "currency" && a.currency ? a.currency : currencyById(ui.currencyId);
  return generateFormat(cat, options, { language: a.language, currency, exponent: a.exponent ?? undefined });
}

/**
 * The code after a language change: the format at the same place in the new
 * language's list (AOO GetFormatForLanguageIfBuiltIn), else the code with
 * the new `[$-LCID]`. In the Currency category the language picks its
 * currency.
 */
export function codeForLanguage(ui: NumbersUiState, code: string, language: string | null, docTag: string): { code: string; currencyId: string } {
  const doc = languageOf(docTag);
  if (ui.category === "currency") {
    const lang = NF_LANGUAGES.find((l) => l.lcid === language) ?? doc;
    const currency = currencyById(lang.currency);
    const a = analyzeFormat(code);
    const opts = a.category === "currency" ? a.options : { decimals: currency.decimals, leading: 1, thousands: true, red: true };
    return { code: generateFormat("currency", opts, { language: null, currency }) ?? code, currencyId: currency.id };
  }
  const cat = analyzeFormat(code).category;
  const currency = currencyById(ui.currencyId);
  const codes = (lcid: string | null): string[] => {
    const pre = lcid === null ? "" : `[$-${lcid}]`;
    const l = languageByLcid(lcid) ?? doc;
    return cat === "date" || cat === "time" ? builtinCodes(cat, l).map((c) => pre + c) : predefinedFormats(cat, lcid, doc, currency);
  };
  const i = codes(ui.language).findIndex((c) => c.toLowerCase() === code.trim().toLowerCase());
  const after = codes(language);
  const next = i >= 0 && after[i] !== undefined ? after[i]! : withLanguage(code, language);
  return { code: next, currencyId: ui.currencyId };
}

export function NumbersTab({ code, onChange }: NumbersTabProps) {
  const docTag = useV3Store((s) => s.locale);
  const cellStyles = useW56Store((s) => s.cellStyles);
  useW56Store((s) => s.i18nRev);
  const [ui, setUi] = useState<NumbersUiState>(() => initialNumbersState(code, docTag));
  const doc = languageOf(docTag);
  const a = analyzeFormat(code);
  const userCodes = useMemo(() => cellStyles.map((s) => s.style.numberFormat).filter((c) => c && !/^general$/i.test(c)), [cellStyles]);
  const list = formatList(ui, code, docTag, userCodes);
  const selected = list.find((c) => c.toLowerCase() === code.trim().toLowerCase()) ?? null;
  const effective = ui.category === "all" || ui.category === "user" ? a.category : ui.category;
  const enabled = optionsEnabled(effective);
  const setOptions = (o: Partial<NfOptions>) => {
    const next = codeForOptions(ui, code, { ...a.options, ...o });
    if (next !== null) onChange(next);
  };
  const editCode = (text: string) => {
    onChange(text);
    const b = analyzeFormat(text);
    setUi((u) => ({
      category: u.category === "all" || u.category === "user" ? u.category : b.category,
      language: b.category === "currency" ? u.language : b.language,
      currencyId: b.currency?.id ?? u.currencyId,
    }));
  };
  const items = list.map((c) => {
    const e = listEntry(c, doc);
    return { value: c, label: e.general ? t("fmt.general", "General") : e.label, hint: c, style: e.color ? { color: e.color } : undefined };
  });
  const langLabel = (key: string, label: string) => t(key, label);
  return (
    <div className={css.tab} data-testid="numbers-tab">
      <div className={css.lists}>
        <div className={css.col}>
          <span className={css.label}>
            {t("fmt.category", "Category")}
          </span>
          <SimpleList
            items={NF_CATEGORIES.map((c) => ({ value: c.id, label: t(c.key, c.label) }))}
            value={ui.category}
            onPick={(cat) => {
              setUi({ ...ui, category: cat });
              const next = codeForCategory(cat, ui, code, docTag);
              if (next !== code) onChange(next);
            }}
            label={t("fmt.category", "Category")}
            testId="numbers-category"
            maxRows={11}
          />
          {ui.category === "currency" ? (
            <select
              className={cx(dlg.select, css.currency)}
              aria-label={t("fmt.currencySymbol", "Currency format")}
              value={ui.currencyId}
              onChange={(e) => {
                const currency = currencyById(e.target.value);
                setUi({ ...ui, currencyId: currency.id });
                const opts = a.category === "currency" ? a.options : { decimals: currency.decimals, leading: 1, thousands: true, red: true };
                onChange(generateFormat("currency", opts, { language: null, currency }) ?? code);
              }}
              data-testid="numbers-currency"
            >
              {[currencyById(doc.currency), ...CURRENCIES.filter((c) => c.id !== doc.currency)].map((c) => (
                <option key={c.id} value={c.id}>
                  {`${c.symbol} ${langLabel(c.key, c.label)}`}
                </option>
              ))}
            </select>
          ) : null}
        </div>
        <div className={css.col}>
          <span className={css.label}>{t("fmt.format", "Format")}</span>
          {list.length > 0 ? (
            <SimpleList items={items} value={selected} onPick={(c) => onChange(c)} label={t("fmt.format", "Format")} testId="numbers-format" maxRows={11} />
          ) : (
            <p className={dlg.note} data-testid="numbers-format-empty">
              {t("fmt.noUserFormats", "No user-defined formats in this document.")}
            </p>
          )}
        </div>
        <div className={css.col}>
          <Field label={t("fmt.language", "Language")}>
            <select
              className={dlg.select}
              value={ui.language ?? ""}
              onChange={(e) => {
                const language = e.target.value === "" ? null : e.target.value;
                const next = codeForLanguage(ui, code, language, docTag);
                setUi({ ...ui, language, currencyId: next.currencyId });
                if (next.code !== code) onChange(next.code);
              }}
              data-testid="numbers-language"
            >
              <option value="">{t("fmt.languageDefault", "Default - {language}", { language: langLabel(doc.key, doc.label) })}</option>
              {NF_LANGUAGES.map((l) => (
                <option key={l.lcid} value={l.lcid}>
                  {langLabel(l.key, l.label)}
                </option>
              ))}
              {ui.language !== null && !NF_LANGUAGES.some((l) => l.lcid === ui.language) ? <option value={ui.language}>{`[$-${ui.language}]`}</option> : null}
            </select>
          </Field>
        </div>
      </div>
      <fieldset className={css.options}>
        <legend>{t("fmt.options", "Options")}</legend>
        <Field label={t("fmt.decimalPlaces", "Decimal places")}>
          <input className={dlg.input} type="number" min={0} max={MAX_DECIMALS} value={enabled.decimals ? a.options.decimals : 0} disabled={!enabled.decimals} onChange={(e) => setOptions({ decimals: clamp(e.target.value, MAX_DECIMALS) })} data-testid="numbers-decimals" />
        </Field>
        <Field label={t("fmt.leadingZeroes", "Leading zeroes")}>
          <input className={dlg.input} type="number" min={0} max={MAX_LEADING} value={enabled.leading ? a.options.leading : 0} disabled={!enabled.leading} onChange={(e) => setOptions({ leading: clamp(e.target.value, MAX_LEADING) })} data-testid="numbers-leading" />
        </Field>
        <Check label={t("fmt.negativeRed", "Negative numbers red")} checked={enabled.red && a.options.red} disabled={!enabled.red} onChange={(v) => setOptions({ red: v })} testId="numbers-red" />
        <Check label={t("fmt.thousandsSeparator", "Thousands separator")} checked={enabled.thousands && a.options.thousands} disabled={!enabled.thousands} onChange={(v) => setOptions({ thousands: v })} testId="numbers-thousands" />
      </fieldset>
      <Field label={t("fmt.formatCode", "Format code")} className={dlg.gap}>
        <input className={cx(dlg.input, dlg.mono)} value={code} spellCheck={false} onChange={(e) => editCode(e.target.value)} data-testid="cells-format-code" />
      </Field>
    </div>
  );
}

function clamp(text: string, max: number): number {
  const n = Math.floor(Number(text));
  return Number.isFinite(n) ? Math.max(0, Math.min(max, n)) : 0;
}

/** The Numbers tab preview: the cell's value (or AOO's sample) with the code, and its colour. */
export function numbersPreview(code: string, docTag: string, value: number | null | undefined, text: string | undefined): { text: string; color?: string } {
  const doc = languageOf(docTag);
  // AOO previews a text cell's string (SvxNumberFormatShell::GetPreviewString_Impl).
  if ((value === null || value === undefined) && text && text.trim() !== "") return { text };
  const v = value ?? DEFAULT_NUMVALUE;
  return { text: previewText(v, code, doc) ?? String(v), color: previewColor(v, code) };
}
