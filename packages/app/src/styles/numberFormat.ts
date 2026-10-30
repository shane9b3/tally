/**
 * Format Cells ▸ Numbers (AOO SvxNumberFormatTabPage + SvxNumberFormatShell,
 * svx/cui): the category list, the predefined formats per category and
 * language, the Language and currency choices, and the option controls.
 *
 * - {@link generateFormat} mirrors SvNumberFormatter::GenerateFormat
 *   (zforlist.cxx): the code built from decimals, leading zeros, thousands
 *   separator and negative-red for the Number, Percent, Currency and
 *   Scientific categories;
 * - {@link analyzeFormat} is the way back (SvNumberFormatter::
 *   GetFormatSpecialInfo plus the category of the format type): code →
 *   category, options, language and currency;
 * - {@link predefinedFormats} lists the built-in codes of a category for a
 *   language (AOO's standard formats of the locale data, en-US first).
 *
 * Codes are kept in Tally's stored spelling (English keywords, `.` decimal
 * and `,` grouping, as the engine and the files hold them). A language other
 * than Default is part of the code, `[$-407]` (AOO keeps it in the format;
 * ODS writes it as number:language / number:country, XLSX as the tag); a
 * currency carries its own locale, `[$€-407]`.
 */
import { formatNumberLite, splitSections, type LiteLocale } from "../numfmtLite";

export type NfCategory = "all" | "user" | "number" | "percent" | "currency" | "date" | "time" | "scientific" | "fraction" | "boolean" | "text";

/** The Category list, in AOO's order, with the i18n key and English label. */
export const NF_CATEGORIES: readonly { id: NfCategory; key: string; label: string }[] = [
  { id: "all", key: "fmt.all", label: "All" },
  { id: "user", key: "fmt.userDefined", label: "User-defined" },
  { id: "number", key: "fmt.number", label: "Number" },
  { id: "percent", key: "fmt.percent", label: "Percent" },
  { id: "currency", key: "fmt.currency", label: "Currency" },
  { id: "date", key: "fmt.date", label: "Date" },
  { id: "time", key: "fmt.time", label: "Time" },
  { id: "scientific", key: "fmt.scientific", label: "Scientific" },
  { id: "fraction", key: "fmt.fraction", label: "Fraction" },
  { id: "boolean", key: "fmt.boolean", label: "Boolean Value" },
  { id: "text", key: "fmt.text", label: "Text" },
];

/** A language of the Language list (the locales Tally has data for). */
export interface NfLanguage {
  tag: string;
  /** Windows locale id in hex, as in `[$-407]`. */
  lcid: string;
  key: string;
  label: string;
  decimal: string;
  group: string;
  trueName: string;
  falseName: string;
  order: "MDY" | "DMY" | "YMD";
  dateSep: string;
  /** Tally's short date (`tally_numfmt::Locale::short_date`). */
  shortDate: string;
  /** The locale's currency ({@link CURRENCIES} id). */
  currency: string;
}

export const NF_LANGUAGES: readonly NfLanguage[] = [
  { tag: "en-US", lcid: "409", key: "fmt.lang.enUS", label: "English (USA)", decimal: ".", group: ",", trueName: "TRUE", falseName: "FALSE", order: "MDY", dateSep: "/", shortDate: "MM/DD/YYYY", currency: "USD-409" },
  { tag: "en-GB", lcid: "809", key: "fmt.lang.enGB", label: "English (UK)", decimal: ".", group: ",", trueName: "TRUE", falseName: "FALSE", order: "DMY", dateSep: "/", shortDate: "DD/MM/YYYY", currency: "GBP-809" },
  { tag: "de-DE", lcid: "407", key: "fmt.lang.deDE", label: "German (Germany)", decimal: ",", group: ".", trueName: "WAHR", falseName: "FALSCH", order: "DMY", dateSep: ".", shortDate: "DD.MM.YYYY", currency: "EUR-407" },
  { tag: "fr-FR", lcid: "40C", key: "fmt.lang.frFR", label: "French (France)", decimal: ",", group: " ", trueName: "VRAI", falseName: "FAUX", order: "DMY", dateSep: "/", shortDate: "DD/MM/YYYY", currency: "EUR-40C" },
  { tag: "es-ES", lcid: "C0A", key: "fmt.lang.esES", label: "Spanish (Spain)", decimal: ",", group: ".", trueName: "VERDADERO", falseName: "FALSO", order: "DMY", dateSep: "/", shortDate: "DD/MM/YYYY", currency: "EUR-C0A" },
  { tag: "ja-JP", lcid: "411", key: "fmt.lang.jaJP", label: "Japanese", decimal: ".", group: ",", trueName: "TRUE", falseName: "FALSE", order: "YMD", dateSep: "/", shortDate: "YYYY/MM/DD", currency: "JPY-411" },
];

/**
 * A currency of the currency list: symbol, locale and AOO's positive and
 * negative currency formats of that locale (`$` stands for the symbol tag,
 * `1` for the number: en-US `$1` / `-$1`, de-DE `1 $` / `-1 $`).
 */
export interface CurrencyEntry {
  id: string;
  iso: string;
  symbol: string;
  lcid: string;
  key: string;
  label: string;
  pos: string;
  neg: string;
  decimals: number;
}

export const CURRENCIES: readonly CurrencyEntry[] = [
  { id: "USD-409", iso: "USD", symbol: "$", lcid: "409", key: "fmt.lang.enUS", label: "English (USA)", pos: "$1", neg: "-$1", decimals: 2 },
  { id: "EUR-407", iso: "EUR", symbol: "€", lcid: "407", key: "fmt.lang.deDE", label: "German (Germany)", pos: "1 $", neg: "-1 $", decimals: 2 },
  { id: "EUR-40C", iso: "EUR", symbol: "€", lcid: "40C", key: "fmt.lang.frFR", label: "French (France)", pos: "1 $", neg: "-1 $", decimals: 2 },
  { id: "EUR-C0A", iso: "EUR", symbol: "€", lcid: "C0A", key: "fmt.lang.esES", label: "Spanish (Spain)", pos: "1 $", neg: "-1 $", decimals: 2 },
  { id: "GBP-809", iso: "GBP", symbol: "£", lcid: "809", key: "fmt.lang.enGB", label: "English (UK)", pos: "$1", neg: "-$1", decimals: 2 },
  { id: "JPY-411", iso: "JPY", symbol: "￥", lcid: "411", key: "fmt.lang.jaJP", label: "Japanese", pos: "$1", neg: "-$1", decimals: 0 },
  { id: "CHF-807", iso: "CHF", symbol: "SFr.", lcid: "807", key: "fmt.lang.deCH", label: "German (Switzerland)", pos: "$ 1", neg: "$ -1", decimals: 2 },
  { id: "CAD-1009", iso: "CAD", symbol: "$", lcid: "1009", key: "fmt.lang.enCA", label: "English (Canada)", pos: "$1", neg: "-$1", decimals: 2 },
  { id: "AUD-C09", iso: "AUD", symbol: "$", lcid: "C09", key: "fmt.lang.enAU", label: "English (Australia)", pos: "$1", neg: "-$1", decimals: 2 },
];

/** The option controls (AOO: Decimal places, Leading zeroes, Negative numbers red, Thousands separator). */
export interface NfOptions {
  decimals: number;
  leading: number;
  thousands: boolean;
  red: boolean;
}

/** AOO's spin field limits. */
export const MAX_DECIMALS = 20;
export const MAX_LEADING = 20;

/** The doc locale (or en-US) for a tag. */
export function languageOf(tag: string | null | undefined): NfLanguage {
  const t = (tag ?? "").toLowerCase();
  return NF_LANGUAGES.find((l) => l.tag.toLowerCase() === t) ?? NF_LANGUAGES.find((l) => l.tag.split("-")[0] === t.split("-")[0]) ?? NF_LANGUAGES[0]!;
}

/** The language with this hex locale id, if Tally has data for it. */
export function languageByLcid(lcid: string | null): NfLanguage | undefined {
  if (lcid === null) return undefined;
  const v = Number.parseInt(lcid, 16);
  if (v === 0x40a) return NF_LANGUAGES.find((l) => l.tag === "es-ES");
  return NF_LANGUAGES.find((l) => Number.parseInt(l.lcid, 16) === v);
}

export function currencyById(id: string): CurrencyEntry {
  return CURRENCIES.find((c) => c.id === id) ?? CURRENCIES[0]!;
}

/** The currency of a `[$sym-LCID]` tag (`null` for symbols not in the list). */
export function currencyByTag(symbol: string, lcid: string | null): CurrencyEntry | null {
  const v = lcid === null ? null : Number.parseInt(lcid, 16);
  // AOO's ja-JP yen is full width (`￥`); the half-width `¥` is the same currency.
  const sym = symbol === "¥" ? "￥" : symbol;
  return CURRENCIES.find((c) => c.symbol === sym && (v === null || Number.parseInt(c.lcid, 16) === v)) ?? null;
}

/** `[$€-407]`. */
export function currencyTag(c: CurrencyEntry): string {
  return `[$${c.symbol}-${c.lcid}]`;
}

/**
 * Whether the option controls apply (AOO 4.1 UpdateOptions_Impl: Number,
 * Percent and Currency only; Scientific and Fraction keep them disabled).
 */
export function optionsEnabled(cat: NfCategory): { decimals: boolean; leading: boolean; thousands: boolean; red: boolean } {
  const on = cat === "number" || cat === "percent" || cat === "currency";
  return { decimals: on, leading: on, thousands: on, red: on };
}

// ---------------------------------------------------------------------------
// GenerateFormat
// ---------------------------------------------------------------------------

/**
 * The integer part of SvNumberFormatter::GenerateFormat for groups of three:
 * no leading zeros is `#` (`#,###` with the separator); otherwise the zeros,
 * with the separator every third digit, padded with `#` to the first group
 * when the separator is on (`#,##0`, `#,#00`, `0,000`).
 */
function integerPart(leading: number, thousands: boolean): string {
  if (leading === 0) return thousands ? "#,###" : "#";
  let s = "";
  for (let i = 0; i < leading; i++) {
    if (thousands && i > 0 && i % 3 === 0) s = "," + s;
    s = "0" + s;
  }
  if (thousands && leading < 4) {
    for (let i = leading; i < 4; i++) {
      if (i === 3) s = "," + s;
      s = "#" + s;
    }
  }
  return s;
}

/** The number body: integer part and `.` with `decimals` zeros. */
export function numberBody(o: Pick<NfOptions, "decimals" | "leading" | "thousands">): string {
  const dec = Math.max(0, Math.min(MAX_DECIMALS, Math.floor(o.decimals)));
  const lead = Math.max(0, Math.min(MAX_LEADING, Math.floor(o.leading)));
  return integerPart(lead, o.thousands) + (dec > 0 ? "." + "0".repeat(dec) : "");
}

/** A currency format string: the template with `$` → tag, `1` → body. */
function currencyString(template: string, tag: string, body: string): string {
  return [...template].map((ch) => (ch === "$" ? tag : ch === "1" ? body : ch)).join("");
}

export interface GenerateContext {
  /** The format's language as a hex locale id; `null` is Default. */
  language: string | null;
  /** The currency of the Currency category. */
  currency: CurrencyEntry;
  /** Scientific: the exponent part of the current code (`E+00`). */
  exponent?: string;
}

/**
 * SvNumberFormatter::GenerateFormat: the code for `cat` with the options.
 * Number `#,##0.00`, red `#,##0.00;[RED]-#,##0.00`; Percent adds `%`;
 * Currency completes the locale's positive and negative currency formats
 * (`[$$-409]#,##0.00;[RED]-[$$-409]#,##0.00`, `#,##0.00 [$€-407];-#,##0.00
 * [$€-407]`), with `[RED]` before the negative part. A language other than
 * Default is the `[$-LCID]` prefix. Scientific (which AOO 4.1 generates
 * as a plain number, its dialog keeping the options disabled) keeps the
 * exponent of the current code. Other categories have no code (`null`).
 */
export function generateFormat(cat: NfCategory, o: NfOptions, ctx: GenerateContext): string | null {
  const lang = ctx.language === null ? "" : `[$-${ctx.language.toUpperCase()}]`;
  if (cat === "currency") {
    const c = ctx.currency;
    const body = numberBody(o);
    const tag = currencyTag(c);
    return `${currencyString(c.pos, tag, body)};${o.red ? "[RED]" : ""}${currencyString(c.neg, tag, body)}`;
  }
  let s: string;
  if (cat === "number") s = numberBody(o);
  else if (cat === "percent") s = numberBody(o) + "%";
  else if (cat === "scientific") s = numberBody({ ...o, thousands: false }) + (ctx.exponent ?? "E+00");
  else return null;
  return lang + (o.red ? `${s};[RED]-${s}` : s);
}

// ---------------------------------------------------------------------------
// Code → category and options
// ---------------------------------------------------------------------------

export interface NfAnalysis {
  category: Exclude<NfCategory, "all" | "user">;
  options: NfOptions;
  /** `[$-407]`: the format's language (hex, upper case), `null` for Default. */
  language: string | null;
  /** The currency of a `[$sym-LCID]` tag, if it is one of {@link CURRENCIES}. */
  currency: CurrencyEntry | null;
  /** The currency symbol of the code (`$`, `[$€-407]` → `€`), if any. */
  currencySymbol: string | null;
  /** Scientific: the exponent part (`E+00`). */
  exponent: string | null;
}

/** Removes quoted text, escapes, `_x` / `*x` and bracket modifiers. */
function bare(section: string): string {
  return section
    .replace(/"[^"]*"?/g, " ")
    .replace(/\\./g, " ")
    .replace(/[_*]./g, " ")
    .replace(/\[[^\]]*\]/g, (m) => (/^\[[hms]+\]$/i.test(m) ? m.slice(1, -1) : " "));
}

/**
 * The category, options, language and currency of a code (AOO: the
 * format's type and GetFormatSpecialInfo — thousands separator, red
 * negatives from the second section's colour, decimals and leading zeros
 * of the first section).
 */
export function analyzeFormat(code: string): NfAnalysis {
  const trimmed = code.trim();
  const sections = splitSections(trimmed === "" ? "General" : trimmed);
  const first = sections[0] ?? "";
  const lang = /\[\$-([0-9A-Fa-f]+)\]/.exec(trimmed);
  const cur = /\[\$([^\]-]+)(?:-([0-9A-Fa-f]+))?\]/.exec(first);
  const plain = bare(first);
  const dollar = /\$/.test(plain);
  const currencySymbol = cur ? cur[1]! : dollar ? "$" : null;
  const red = sections.length > 1 && /\[RED\]/i.test(sections[1] ?? "");
  const options: NfOptions = { decimals: 0, leading: 1, thousands: false, red };
  const base = {
    language: lang ? lang[1]!.toUpperCase().replace(/^0+(?=.)/, "") : null,
    currency: cur ? currencyByTag(cur[1]!, cur[2] ?? null) : dollar ? CURRENCIES[0]! : null,
    currencySymbol,
    exponent: null as string | null,
  };
  const noOpts = { ...options, leading: 0, red: false };
  if (/^\s*(general|standard)\s*$/i.test(plain)) return { ...base, category: "number", options };
  if (/BOOLEAN/i.test(plain)) return { ...base, category: "boolean", options: noOpts };
  const digits = /[0#?]/.test(plain);
  if (plain.includes("@") && !digits) return { ...base, category: "text", options: noOpts };
  const sci = /([0#?][0#?,]*(?:\.[0#?]*)?)([Ee][+-][0#?]+)/.exec(plain);
  if (!sci) {
    const words = plain.replace(/AM\/PM|A\/P/gi, "H").replace(/General|Standard/gi, "");
    if (/[YDNQWGE]/i.test(words) || (/M/i.test(words) && !/[HS]/i.test(words))) return { ...base, category: "date", options: noOpts };
    if (/[HS]/i.test(words) || /M/i.test(words)) return { ...base, category: "time", options: noOpts };
  }
  if (/[0#?]\s*\/\s*[0-9#?]/.test(plain)) {
    const m = /([0#?]+)\s+[0#?]+\s*\//.exec(plain);
    return { ...base, category: "fraction", options: { ...noOpts, leading: m ? (m[1]!.match(/0/g) ?? []).length : 0 } };
  }
  const num = sci ? sci[1]! : (/[0#?][0#?,]*(?:\.[0#?]*)?|\.[0#?]+/.exec(plain)?.[0] ?? "");
  const [int = "", dec = ""] = num.split(".");
  const intDigits = int.replace(/,+$/, "");
  options.decimals = (dec.match(/[0#?]/g) ?? []).length;
  options.leading = (intDigits.match(/0/g) ?? []).length;
  options.thousands = !sci && /[0#?],[0#?]/.test(intDigits);
  if (sci) return { ...base, category: "scientific", options, exponent: sci[2]!.toUpperCase().replace(/#|\?/g, "0") };
  if (currencySymbol !== null) return { ...base, category: "currency", options };
  if (plain.includes("%")) return { ...base, category: "percent", options };
  return { ...base, category: "number", options };
}

// ---------------------------------------------------------------------------
// Predefined formats
// ---------------------------------------------------------------------------

/**
 * The DATE (formatindex 18-38), TIME (39-45) and DATE_TIME (46-47) codes of
 * AOO 4.1's locale data (i18npool/source/localedata/data/*.xml), in
 * Tally's spelling (German `TT.MM.JJJJ` is `DD.MM.YYYY`), one entry per
 * formatindex (the list drops duplicates; a language change maps by index).
 */
const LOCALE_DATES: Readonly<Record<string, { date: readonly string[]; time: readonly string[]; dateTime: readonly string[] }>> = {
  "en-US": {
    date: ["M/D/YY", "NNNNMMMM DD, YYYY", "MM/DD/YY", "MM/DD/YYYY", "MMM D, YY", "MMM D, YYYY", "D. MMM. YYYY", "MMMM D, YYYY", "D. MMMM YYYY", "NN, MMM D, YY", "NN DD/MMM YY", "NN, MMMM D, YYYY", "NNNNMMMM D, YYYY", "MM-DD", "YY-MM-DD", "YYYY-MM-DD", "MM/YY", "MMM DD", "MMMM", "QQ YY", "WW"],
    time: ["HH:MM", "HH:MM:SS", "HH:MM AM/PM", "HH:MM:SS AM/PM", "[HH]:MM:SS", "MM:SS.00", "[HH]:MM:SS.00"],
    dateTime: ["MM/DD/YY HH:MM AM/PM", "MM/DD/YYYY HH:MM:SS"],
  },
  "en-GB": {
    date: ["DD/MM/YY", "DD MMMM YYYY", "DD/MM/YY", "DD/MM/YYYY", "D MMM YY", "D MMM YYYY", "D. MMM. YYYY", "D MMMM YYYY", "D. MMMM YYYY", "NN D MMM YY", "NN DD/MMM YY", "NN D MMMM YYYY", "NNNND MMMM YYYY", "MM-DD", "YY-MM-DD", "YYYY-MM-DD", "MM/YY", "DD/MMM", "MMMM", "QQ YY", "WW"],
    time: ["HH:MM", "HH:MM:SS", "HH:MM AM/PM", "HH:MM:SS AM/PM", "[HH]:MM:SS", "MM:SS.00", "[HH]:MM:SS.00"],
    dateTime: ["DD/MM/YY HH:MM", "DD/MM/YYYY HH:MM:SS"],
  },
  "de-DE": {
    date: ["DD.MM.YY", "NNNND. MMMM YYYY", "DD.MM.YY", "DD.MM.YYYY", "D. MMM YY", "D. MMM YYYY", "D. MMM. YYYY", "D. MMMM YYYY", "D. MMMM YYYY", "NN, D. MMM YY", "NN DD.MMM YY", "NN, D. MMMM YYYY", "NNNND. MMMM YYYY", "MM-DD", "YY-MM-DD", "YYYY-MM-DD", "MM.YY", "DD.MMM", "MMMM", "QQ YY", "WW"],
    time: ["HH:MM", "HH:MM:SS", "HH:MM AM/PM", "HH:MM:SS AM/PM", "[HH]:MM:SS", "MM:SS.00", "[HH]:MM:SS.00"],
    dateTime: ["DD.MM.YY HH:MM", "DD.MM.YYYY HH:MM:SS"],
  },
  "fr-FR": {
    date: ["DD/MM/YYYY", "NNNND MMMM YYYY", "DD/MM/YY", "DD/MM/YYYY", "D MMM YY", "D MMM YYYY", "D. MMM YYYY", "D MMMM YYYY", "D. MMMM YYYY", "NN D MMM YY", "NN DD/MMM YY", "NN D MMMM YYYY", "NNNND MMMM YYYY", "MM-DD", "YY-MM-DD", "YYYY-MM-DD", "MM/YY", "DD/MMM", "MMMM", "QQ YY", "WW"],
    time: ["HH:MM", "HH:MM:SS", "HH:MM AM/PM", "HH:MM:SS AM/PM", "[HH]:MM:SS", "MM:SS.00", "[HH]:MM:SS.00"],
    dateTime: ["DD/MM/YY HH:MM", "DD/MM/YYYY HH:MM:SS"],
  },
  "es-ES": {
    date: ["D/MM/YY", "NNNND \"de\" MMMM \"de\" YYYY", "DD/MM/YY", "DD/MM/YYYY", "D \"de\" MMM \"de\" YY", "D \"de\" MMM \"de\" YYYY", "D. MMM. YYYY", "D \"de\" MMMM \"de\" YYYY", "D. MMMM YYYY", "NN D \"de\" MMM \"de\" YY", "NN DD/MMM YY", "NN D \"de\" MMMM \"de\" YYYY", "NNNND \"de\" MMMM \"de\" YYYY", "MM-DD", "YY-MM-DD", "YYYY-MM-DD", "MM/YY", "DD/MMM", "MMMM", "QQ YY", "WW"],
    time: ["HH:MM", "HH:MM:SS", "HH:MM AM/PM", "HH:MM:SS AM/PM", "[HH]:MM:SS", "MM:SS.00", "[HH]:MM:SS.00"],
    dateTime: ["DD/MM/YY HH:MM", "DD/MM/YYYY HH:MM:SS"],
  },
  "ja-JP": {
    date: ["YY/M/D", "YYYY年MM月DD日（AAAA）", "YY/MM/DD", "YYYY/MM/DD", "YY年M月D日", "YYYY年M月D日", "GGGE年M月D日", "YYYY年M月D日", "GGGE年M月D日（AAAA）", "YY年M月D日（AAA）", "GGGE年M月D日（AAA）", "YYYY年M月D日（AAA）", "YYYY年M月D日（AAAA）", "MM.DD", "GE.M.D", "GE.MM.DD", "YY/MM", "M月D日", "M月", "YY年 QQ", "WW 1"],
    time: ["H:MM", "HH:MM:SS", "AM/PM H:MM", "AM/PM H:MM:SS", "[HH]:MM:SS", "MM:SS.00", "[HH]:MM:SS.00"],
    dateTime: ["YY/MM/DD HH:MM", "YYYY/M/D H:MM"],
  },
};

const dedupe = (xs: readonly string[]): string[] => xs.filter((x, i) => xs.indexOf(x) === i);

/**
 * The built-in date or time codes of a language in formatindex order, with
 * duplicates (AOO: the DATE or TIME formats, then the DATE_TIME ones,
 * FillEListWithDateTime_Impl).
 */
export function builtinCodes(cat: "date" | "time", l: NfLanguage): string[] {
  const d = localeDates(l);
  return [...(cat === "date" ? d.date : d.time), ...d.dateTime];
}

function localeDates(l: NfLanguage): { date: readonly string[]; time: readonly string[]; dateTime: readonly string[] } {
  return LOCALE_DATES[l.tag] ?? LOCALE_DATES["en-US"]!;
}

/** AOO's standard currency formats with the chosen currency (CURRENCY usage, with `.--` and the ISO code). */
export function currencyFormats(c: CurrencyEntry): string[] {
  const tag = currencyTag(c);
  const gen = (decimals: number, red: boolean) => generateFormat("currency", { decimals, leading: 1, thousands: true, red }, { language: null, currency: c });
  const dash = `${currencyString(c.pos, tag, "#,##0.--")};[RED]${currencyString(c.neg, tag, "#,##0.--")}`;
  const iso = `#,##0${c.decimals > 0 ? "." + "0".repeat(c.decimals) : ""} [$${c.iso}-${c.lcid}]`;
  const out = [gen(0, false), gen(c.decimals, false), gen(0, true), gen(c.decimals, true), iso, dash].filter((x): x is string => x !== null);
  return [...new Set(out)];
}

/**
 * The predefined codes of a category for a language (`language`: hex locale
 * id, or `null` for Default, which is `docLocale`). A language other than
 * Default prefixes `[$-LCID]` (Currency carries the currency's own locale).
 */
export function predefinedFormats(cat: NfCategory, language: string | null, docLocale: NfLanguage, currency: CurrencyEntry): string[] {
  const l = languageByLcid(language) ?? docLocale;
  const pre = language === null ? "" : `[$-${language.toUpperCase()}]`;
  const tagged = (xs: readonly string[]) => xs.map((x) => pre + x);
  switch (cat) {
    case "number":
      return tagged(["General", "0", "0.00", "#,##0", "#,##0.00", "#,###.00"]);
    case "percent":
      return tagged(["0%", "0.00%"]);
    case "currency":
      return currencyFormats(currency);
    case "date":
    case "time": {
      const xs = builtinCodes(cat, l);
      return tagged(dedupe(cat === "date" && !xs.includes(l.shortDate) ? [l.shortDate, ...xs] : xs));
    }
    case "scientific":
      return tagged(["0.00E+000", "0.00E+00"]);
    case "fraction":
      return tagged(["# ?/?", "# ??/??"]);
    case "boolean":
      return tagged(["BOOLEAN"]);
    case "text":
      return tagged(["@"]);
    case "all":
      return (["number", "percent", "currency", "date", "time", "scientific", "fraction", "boolean", "text"] as const).flatMap((c) => predefinedFormats(c, language, docLocale, currency));
    case "user":
      return [];
  }
}

/** The format selected when a category is picked (AOO: the category's standard format). */
export function categoryDefault(cat: NfCategory, language: string | null, docLocale: NfLanguage, currency: CurrencyEntry): string | null {
  const pre = language === null ? "" : `[$-${language.toUpperCase()}]`;
  const l = languageByLcid(language) ?? docLocale;
  switch (cat) {
    case "number":
      return pre + "General";
    case "percent":
      return pre + "0%";
    case "currency":
      return generateFormat("currency", { decimals: currency.decimals, leading: 1, thousands: true, red: true }, { language: null, currency });
    case "date":
      return pre + l.shortDate;
    case "time":
      return pre + "HH:MM:SS";
    case "scientific":
      return pre + "0.00E+000";
    case "fraction":
      return pre + "# ?/?";
    case "boolean":
      return pre + "BOOLEAN";
    case "text":
      return pre + "@";
    default:
      return null;
  }
}

/** Whether `code` is one of the predefined codes (any category, the language and currency given). */
export function isPredefined(code: string, language: string | null, docLocale: NfLanguage, currency: CurrencyEntry): boolean {
  return predefinedFormats("all", language, docLocale, currency).some((c) => c.toLowerCase() === code.trim().toLowerCase());
}

/**
 * Replaces the language of a code (`[$-407]` prefix): `null` removes it.
 * A currency code keeps its own tag.
 */
export function withLanguage(code: string, language: string | null): string {
  const stripped = code.replace(/\[\$-[0-9A-Fa-f]+\]/g, "");
  if (language === null) return stripped === "" ? "General" : stripped;
  return `[$-${language.toUpperCase()}]${stripped === "" ? "General" : stripped}`;
}

// ---------------------------------------------------------------------------
// Preview
// ---------------------------------------------------------------------------

/** The format list's sample values (AOO svx/flagsdef.hxx SVX_NUMVAL_*). */
export const NUMVAL_STANDARD = -1234.12345678901234;
export const NUMVAL_CURRENCY = -1234;
export const NUMVAL_PERCENT = -0.1295;
export const NUMVAL_DATE = 36525.5678935185;
export const NUMVAL_BOOLEAN = 1;
/** The preview value when the cell has none (SvxNumberFormatShell::DEFAULT_NUMVALUE). */
export const DEFAULT_NUMVALUE = 1234.56789;

/** The language whose separators and names a code is shown with. */
export function displayLanguage(code: string, docLocale: NfLanguage): NfLanguage {
  const a = /\[\$-([0-9A-Fa-f]+)\]/.exec(code);
  return (a ? languageByLcid(a[1]!) : undefined) ?? docLocale;
}

/**
 * The separators of a code's `[$-LCID]` language for {@link formatNumberLite}
 * (`undefined`: no tag, or a language without data; the caller's default
 * applies). Cell formats only: TEXT() codes keep the document's.
 */
export function liteLocaleOf(code: string): LiteLocale | undefined {
  const a = /\[\$-([0-9A-Fa-f]+)\]/.exec(code);
  const l = a ? languageByLcid(a[1]!) : undefined;
  return l ? { decimal: l.decimal, group: l.group, trueName: l.trueName, falseName: l.falseName } : undefined;
}

/** `v` formatted with `code` in its language, or `null` if the preview formatter cannot. */
export function previewText(v: number, code: string, docLocale: NfLanguage): string | null {
  const l = displayLanguage(code, docLocale);
  const loc: LiteLocale = { decimal: l.decimal, group: l.group, trueName: l.trueName, falseName: l.falseName };
  return formatNumberLite(v, code.trim() === "" ? "General" : code, loc);
}

/**
 * The colour of the section `v` is shown with (`[RED]` …), as CSS. Red is
 * the theme's danger red (AOO's light red #ff0000 fails WCAG contrast on
 * the dialog's background).
 */
export function previewColor(v: number, code: string): string | undefined {
  const sections = splitSections(code);
  const s = v < 0 && sections.length > 1 ? sections[1]! : v === 0 && sections.length > 2 ? sections[2]! : sections[0]!;
  const m = /\[(RED|BLUE|GREEN|BLACK|WHITE|CYAN|MAGENTA|YELLOW|BROWN|GREY|GRAY)\]/i.exec(s);
  if (!m) return undefined;
  const name = m[1]!.toUpperCase();
  return ({ RED: "var(--tally-danger)", BLUE: "#0000ff", GREEN: "#00ff00", BLACK: "#000000", WHITE: "#ffffff", CYAN: "#00ffff", MAGENTA: "#ff00ff", YELLOW: "#ffff00", BROWN: "#800000", GREY: "#808080", GRAY: "#808080" } as Record<string, string>)[name];
}

/**
 * A format list entry as AOO draws it (FillFormatListBox_Impl /
 * GetExpColorString): the entry's sample value in the format, with its
 * colour; `General` by name and text formats as their code.
 */
export function listEntry(code: string, docLocale: NfLanguage): { label: string; color?: string; general: boolean } {
  const a = analyzeFormat(code);
  if (/^(\[\$-[0-9A-Fa-f]+\])?\s*(general|standard)\s*$/i.test(code.trim())) return { label: code, general: true };
  if (a.category === "text") return { label: code, general: false };
  const v = a.category === "currency" ? NUMVAL_CURRENCY : a.category === "percent" ? NUMVAL_PERCENT : a.category === "date" || a.category === "time" ? NUMVAL_DATE : a.category === "boolean" ? NUMVAL_BOOLEAN : NUMVAL_STANDARD;
  return { label: previewText(v, code, docLocale) ?? code, color: previewColor(v, code), general: false };
}
