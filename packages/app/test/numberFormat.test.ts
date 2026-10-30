/**
 * Format Cells ▸ Numbers (G14): GenerateFormat parity with AOO's
 * SvNumberFormatter::GenerateFormat, the code → options read-back, the
 * predefined lists per language and the dialog state transitions.
 */
import { describe, expect, it } from "vitest";
import { formatNumberLite } from "../src/numfmtLite";
import {
  CURRENCIES,
  NF_LANGUAGES,
  analyzeFormat,
  categoryDefault,
  currencyById,
  currencyFormats,
  generateFormat,
  languageOf,
  optionsEnabled,
  predefinedFormats,
  previewColor,
  previewText,
  listEntry,
  withLanguage,
  type NfCategory,
  type NfOptions,
} from "../src/styles/numberFormat";
import { codeForCategory, codeForLanguage, codeForOptions, formatList, initialNumbersState, numbersPreview } from "../src/styles/NumbersTab";

const USD = currencyById("USD-409");
const EUR_DE = currencyById("EUR-407");
const gen = (cat: NfCategory, decimals: number, leading: number, thousands: boolean, red: boolean, language: string | null = null, currency = USD) =>
  generateFormat(cat, { decimals, leading, thousands, red }, { language, currency });

describe("generateFormat (SvNumberFormatter::GenerateFormat)", () => {
  // [category, decimals, leading zeros, thousands, red, expected]
  const table: [NfCategory, number, number, boolean, boolean, string][] = [
    ["number", 0, 1, false, false, "0"],
    ["number", 2, 1, false, false, "0.00"],
    ["number", 0, 1, true, false, "#,##0"],
    ["number", 2, 1, true, false, "#,##0.00"],
    ["number", 2, 1, true, true, "#,##0.00;[RED]-#,##0.00"],
    ["number", 2, 0, false, false, "#.00"],
    ["number", 2, 0, true, false, "#,###.00"],
    ["number", 0, 2, false, false, "00"],
    ["number", 0, 2, true, false, "#,#00"],
    ["number", 0, 3, true, false, "#,000"],
    ["number", 1, 4, true, false, "0,000.0"],
    ["number", 0, 7, true, false, "0,000,000"],
    ["number", 3, 5, false, true, "00000.000;[RED]-00000.000"],
    ["percent", 2, 1, false, false, "0.00%"],
    ["percent", 0, 1, false, true, "0%;[RED]-0%"],
    ["percent", 1, 1, true, false, "#,##0.0%"],
    ["scientific", 2, 1, false, false, "0.00E+00"],
    ["scientific", 3, 2, true, true, "00.000E+00;[RED]-00.000E+00"],
    ["currency", 2, 1, true, true, "[$$-409]#,##0.00;[RED]-[$$-409]#,##0.00"],
    ["currency", 2, 1, true, false, "[$$-409]#,##0.00;-[$$-409]#,##0.00"],
    ["currency", 0, 1, true, false, "[$$-409]#,##0;-[$$-409]#,##0"],
    ["currency", 2, 1, false, true, "[$$-409]0.00;[RED]-[$$-409]0.00"],
  ];
  it.each(table)("%s dec=%i lead=%i thousands=%s red=%s → %s", (cat, d, l, th, red, want) => {
    expect(gen(cat, d, l, th, red)).toBe(want);
  });

  it("completes each locale's currency formats", () => {
    expect(gen("currency", 2, 1, true, true, null, EUR_DE)).toBe("#,##0.00 [$€-407];[RED]-#,##0.00 [$€-407]");
    expect(gen("currency", 2, 1, true, false, null, currencyById("EUR-40C"))).toBe("#,##0.00 [$€-40C];-#,##0.00 [$€-40C]");
    expect(gen("currency", 2, 1, true, true, null, currencyById("GBP-809"))).toBe("[$£-809]#,##0.00;[RED]-[$£-809]#,##0.00");
    expect(gen("currency", 0, 1, true, true, null, currencyById("JPY-411"))).toBe("[$￥-411]#,##0;[RED]-[$￥-411]#,##0");
    expect(gen("currency", 2, 1, true, true, null, currencyById("CHF-807"))).toBe("[$SFr.-807] #,##0.00;[RED][$SFr.-807] -#,##0.00");
    expect(gen("currency", 2, 1, true, false, null, currencyById("CAD-1009"))).toBe("[$$-1009]#,##0.00;-[$$-1009]#,##0.00");
    expect(gen("currency", 2, 1, true, false, null, currencyById("AUD-C09"))).toBe("[$$-C09]#,##0.00;-[$$-C09]#,##0.00");
  });

  it("puts a non-default language in the code as [$-LCID]", () => {
    // AOO's de GenerateFormat gives `#.##0,00`: the German spelling of the stored `#,##0.00`.
    expect(gen("number", 2, 1, true, false, "407")).toBe("[$-407]#,##0.00");
    expect(gen("number", 2, 1, true, true, "407")).toBe("[$-407]#,##0.00;[RED]-#,##0.00");
    expect(gen("percent", 2, 1, false, false, "40C")).toBe("[$-40C]0.00%");
    // The German preview uses German separators.
    const de = languageOf("de-DE");
    expect(previewText(1234.5, "[$-407]#,##0.00", languageOf("en-US"))).toBe("1.234,50");
    expect(previewText(1234.5, "#,##0.00", de)).toBe("1.234,50");
    expect(previewText(1234.5, "[$-409]#,##0.00", de)).toBe("1,234.50");
  });

  it("keeps the scientific exponent and has no code for the other categories", () => {
    expect(generateFormat("scientific", { decimals: 1, leading: 1, thousands: false, red: false }, { language: null, currency: USD, exponent: "E+000" })).toBe("0.0E+000");
    for (const c of ["date", "time", "fraction", "boolean", "text", "all", "user"] as NfCategory[]) expect(gen(c, 2, 1, true, true)).toBeNull();
  });

  it("enables the options as AOO 4.1's UpdateOptions_Impl (Number, Percent, Currency)", () => {
    expect(optionsEnabled("number")).toEqual({ decimals: true, leading: true, thousands: true, red: true });
    expect(optionsEnabled("currency").thousands).toBe(true);
    expect(optionsEnabled("percent").red).toBe(true);
    for (const c of ["scientific", "date", "time", "fraction", "boolean", "text"] as NfCategory[]) expect(Object.values(optionsEnabled(c)).some(Boolean)).toBe(false);
  });
});

describe("analyzeFormat (code → category and options)", () => {
  const opts = (decimals: number, leading: number, thousands: boolean, red: boolean): NfOptions => ({ decimals, leading, thousands, red });
  const cases: [string, string, NfOptions | null][] = [
    ["General", "number", opts(0, 1, false, false)],
    ["0", "number", opts(0, 1, false, false)],
    ["0.00", "number", opts(2, 1, false, false)],
    ["#,##0", "number", opts(0, 1, true, false)],
    ["#,##0.00", "number", opts(2, 1, true, false)],
    ["#,###.00", "number", opts(2, 0, true, false)],
    ["#,##0.00;[RED]-#,##0.00", "number", opts(2, 1, true, true)],
    ["0,000.000", "number", opts(3, 4, true, false)],
    ['0.00" kg"', "number", opts(2, 1, false, false)],
    ["#,##0,", "number", opts(0, 1, true, false)],
    ["0%", "percent", opts(0, 1, false, false)],
    ["0.00%;[RED]-0.00%", "percent", opts(2, 1, false, true)],
    ["[$$-409]#,##0.00;[RED]-[$$-409]#,##0.00", "currency", opts(2, 1, true, true)],
    ["#,##0.00 [$€-407];-#,##0.00 [$€-407]", "currency", opts(2, 1, true, false)],
    ["$#,##0.00;[RED]-$#,##0.00", "currency", opts(2, 1, true, true)],
    ["#,##0.00 [$USD-409]", "currency", opts(2, 1, true, false)],
    ["0.00E+00", "scientific", opts(2, 1, false, false)],
    ["##0.0E+0", "scientific", opts(1, 1, false, false)],
    ["# ?/?", "fraction", null],
    ["# ??/??", "fraction", null],
    ["MM/DD/YYYY", "date", null],
    ["DD.MM.YYYY", "date", null],
    ["NNNNMMMM D, YYYY", "date", null],
    ["MMMM", "date", null],
    ["QQ YY", "date", null],
    ["MM/DD/YYYY HH:MM", "date", null],
    ["HH:MM:SS", "time", null],
    ["HH:MM AM/PM", "time", null],
    ["[HH]:MM:SS", "time", null],
    ["MM:SS.00", "time", null],
    ["BOOLEAN", "boolean", null],
    ["@", "text", null],
  ];
  it.each(cases)("%s → %s", (code, cat, o) => {
    const a = analyzeFormat(code);
    expect(a.category).toBe(cat);
    if (o) expect(a.options).toEqual(o);
  });

  it("reads back what generateFormat writes, for every predefined code", () => {
    const doc = languageOf("en-US");
    for (const cat of ["number", "percent", "currency", "scientific"] as NfCategory[]) {
      for (const code of predefinedFormats(cat, null, doc, USD)) {
        const a = analyzeFormat(code);
        expect(a.category, code).toBe(cat);
        const again = generateFormat(cat, a.options, { language: a.language, currency: a.currency ?? USD, exponent: a.exponent ?? undefined });
        // `General`, `.--` and the ISO-code currency are not GenerateFormat output.
        if (!/General|--|USD/.test(code)) expect(again, code).toBe(code);
      }
    }
    for (const cat of ["date", "time", "fraction", "boolean", "text"] as NfCategory[]) {
      // The Time list ends with the date-times (AOO FillEListWithDateTime_Impl).
      for (const l of NF_LANGUAGES) for (const code of predefinedFormats(cat, l.lcid, doc, USD)) expect(analyzeFormat(code).category, code).toBe(cat === "time" && /Y/.test(code) ? "date" : cat);
    }
  });

  it("reads the language and the currency", () => {
    expect(analyzeFormat("[$-407]DD.MM.YYYY")).toMatchObject({ category: "date", language: "407" });
    expect(analyzeFormat("[$-0C0A]0.00")).toMatchObject({ language: "C0A" });
    expect(analyzeFormat("#,##0.00 [$€-407];[RED]-#,##0.00 [$€-407]").currency?.id).toBe("EUR-407");
    expect(analyzeFormat("[$£-809]#,##0.00").currency?.id).toBe("GBP-809");
    expect(analyzeFormat("[$₹-4009]#,##0.00")).toMatchObject({ category: "currency", currency: null, currencySymbol: "₹" });
  });
});

describe("predefined formats", () => {
  it("lists AOO's standard formats per category and language", () => {
    const us = languageOf("en-US");
    expect(predefinedFormats("number", null, us, USD)).toEqual(["General", "0", "0.00", "#,##0", "#,##0.00", "#,###.00"]);
    expect(predefinedFormats("percent", null, us, USD)).toEqual(["0%", "0.00%"]);
    expect(predefinedFormats("scientific", null, us, USD)).toEqual(["0.00E+000", "0.00E+00"]);
    expect(predefinedFormats("fraction", null, us, USD)).toEqual(["# ?/?", "# ??/??"]);
    // AOO en_US.xml formatindex 18-38, then 46-47.
    expect(predefinedFormats("date", null, us, USD)).toEqual(["M/D/YY", "NNNNMMMM DD, YYYY", "MM/DD/YY", "MM/DD/YYYY", "MMM D, YY", "MMM D, YYYY", "D. MMM. YYYY", "MMMM D, YYYY", "D. MMMM YYYY", "NN, MMM D, YY", "NN DD/MMM YY", "NN, MMMM D, YYYY", "NNNNMMMM D, YYYY", "MM-DD", "YY-MM-DD", "YYYY-MM-DD", "MM/YY", "MMM DD", "MMMM", "QQ YY", "WW", "MM/DD/YY HH:MM AM/PM", "MM/DD/YYYY HH:MM:SS"]);
    expect(predefinedFormats("time", null, us, USD)).toEqual(["HH:MM", "HH:MM:SS", "HH:MM AM/PM", "HH:MM:SS AM/PM", "[HH]:MM:SS", "MM:SS.00", "[HH]:MM:SS.00", "MM/DD/YY HH:MM AM/PM", "MM/DD/YYYY HH:MM:SS"]);
    // de_DE.xml `TT.MM.JJJJ` in Tally's spelling.
    expect(predefinedFormats("date", "407", us, USD).slice(0, 3)).toEqual(["[$-407]DD.MM.YY", "[$-407]NNNND. MMMM YYYY", "[$-407]DD.MM.YYYY"]);
    expect(predefinedFormats("date", null, languageOf("fr-FR"), USD)[0]).toBe("DD/MM/YYYY");
    expect(predefinedFormats("time", null, languageOf("fr-FR"), USD)[2]).toBe("HH:MM AM/PM");
    expect(predefinedFormats("date", null, languageOf("ja-JP"), USD)).toContain("YYYY/MM/DD");
    expect(currencyFormats(USD)).toEqual([
      "[$$-409]#,##0;-[$$-409]#,##0",
      "[$$-409]#,##0.00;-[$$-409]#,##0.00",
      "[$$-409]#,##0;[RED]-[$$-409]#,##0",
      "[$$-409]#,##0.00;[RED]-[$$-409]#,##0.00",
      "#,##0.00 [$USD-409]",
      "[$$-409]#,##0.--;[RED]-[$$-409]#,##0.--",
    ]);
    expect(currencyFormats(currencyById("JPY-411"))).toHaveLength(4);
    for (const l of NF_LANGUAGES) {
      expect(categoryDefault("date", null, l, USD)).toBe(l.shortDate);
      expect(CURRENCIES.some((c) => c.id === l.currency), l.tag).toBe(true);
    }
    expect(categoryDefault("currency", null, us, USD)).toBe("[$$-409]#,##0.00;[RED]-[$$-409]#,##0.00");
    expect(categoryDefault("scientific", null, us, USD)).toBe("0.00E+000");
  });

  it("previews the list entries as AOO (SVX_NUMVAL_* samples, red negatives)", () => {
    const us = languageOf("en-US");
    expect(listEntry("#,##0.00", us)).toEqual({ label: "-1,234.12", color: undefined, general: false });
    expect(listEntry("[$$-409]#,##0.00;[RED]-[$$-409]#,##0.00", us)).toEqual({ label: "-$1,234.00", color: "var(--tally-danger)", general: false });
    expect(listEntry("0.00%", us).label).toBe("-12.95%");
    expect(listEntry("MM/DD/YYYY", us).label).toBe("12/31/1999");
    expect(listEntry("HH:MM:SS", us).label).toBe("13:37:46");
    expect(listEntry("General", us).general).toBe(true);
    expect(listEntry("@", us).label).toBe("@");
    expect(previewText(-1234.56789, "#,##0.00", us)).toBe("-1,234.57");
    expect(previewText(-1234.56789, "#,###.00", us)).toBe("-1,234.57");
    expect(previewText(-1234.56789, "0.00E+00", us)).toBe("-1.23E+03");
    expect(previewText(-1234.56789, "# ?/?", us)).toBe("-1234 4/7");
    expect(previewText(1.75, "# ??/??", us)).toBe("1 3/4");
    expect(previewText(1, "BOOLEAN", languageOf("de-DE"))).toBe("WAHR");
    expect(previewText(-1234.56789, "[$$-409]#,##0.--;[RED]-[$$-409]#,##0.--", us)).toBe("-$1,235.--");
    expect(previewText(1.5, "[HH]:MM:SS", us)).toBe("36:00:00");
    expect(previewColor(-1, "#,##0.00;[RED]-#,##0.00")).toBe("var(--tally-danger)");
    expect(previewColor(1, "#,##0.00;[RED]-#,##0.00")).toBeUndefined();
    expect(formatNumberLite(1234.5, "#,##0.00", { decimal: ",", group: "." })).toBe("1.234,50");
  });

  it("replaces a code's language", () => {
    expect(withLanguage("[$-407]DD.MM.YYYY", null)).toBe("DD.MM.YYYY");
    expect(withLanguage("0.00", "40C")).toBe("[$-40C]0.00");
    expect(withLanguage("[$-407]0.00", "411")).toBe("[$-411]0.00");
  });
});

describe("Numbers tab state", () => {
  it("opens on the category, language and currency of the cell's code", () => {
    expect(initialNumbersState("MM/DD/YYYY", "en-US")).toEqual({ category: "date", language: null, currencyId: "USD-409" });
    expect(initialNumbersState("#,##0.00 [$€-407];-#,##0.00 [$€-407]", "en-US")).toEqual({ category: "currency", language: null, currencyId: "EUR-407" });
    expect(initialNumbersState("[$-407]0.00", "en-US")).toEqual({ category: "number", language: "407", currencyId: "USD-409" });
    expect(initialNumbersState("General", "de-DE").currencyId).toBe("EUR-407");
  });

  it("lists the category's formats with the cell's own code and selects it", () => {
    const ui = initialNumbersState("MM/DD/YYYY", "en-US");
    const list = formatList(ui, "MM/DD/YYYY", "en-US", []);
    expect(list[3]).toBe("MM/DD/YYYY");
    const custom = formatList(initialNumbersState('0.0" kg"', "en-US"), '0.0" kg"', "en-US", []);
    expect(custom.at(-1)).toBe('0.0" kg"');
    expect(formatList({ category: "user", language: null, currencyId: "USD-409" }, "0.00", "en-US", ["0.000", "#,##0.00", "0.000"])).toEqual(["0.000"]);
    expect(formatList({ category: "all", language: null, currencyId: "USD-409" }, "General", "en-US", []).length).toBeGreaterThan(40);
  });

  it("picks a category's standard format unless the code already is one", () => {
    const ui = initialNumbersState("General", "en-US");
    expect(codeForCategory("currency", ui, "General", "en-US")).toBe("[$$-409]#,##0.00;[RED]-[$$-409]#,##0.00");
    expect(codeForCategory("date", ui, "General", "de-DE")).toBe("DD.MM.YYYY");
    expect(codeForCategory("number", ui, "0.000", "en-US")).toBe("0.000");
    expect(codeForCategory("all", ui, "0.000", "en-US")).toBe("0.000");
    expect(codeForCategory("text", ui, "0.000", "en-US")).toBe("@");
  });

  it("regenerates the code from the options (two-way)", () => {
    const ui = { category: "currency" as const, language: null, currencyId: "USD-409" };
    let code = "[$$-409]#,##0;-[$$-409]#,##0";
    code = codeForOptions(ui, code, { ...analyzeFormat(code).options, decimals: 2 })!;
    expect(code).toBe("[$$-409]#,##0.00;-[$$-409]#,##0.00");
    code = codeForOptions(ui, code, { ...analyzeFormat(code).options, red: true })!;
    expect(code).toBe("[$$-409]#,##0.00;[RED]-[$$-409]#,##0.00");
    // The code's own currency wins over the list's.
    expect(codeForOptions(ui, "#,##0.00 [$€-407];-#,##0.00 [$€-407]", { decimals: 1, leading: 1, thousands: true, red: false })).toBe("#,##0.0 [$€-407];-#,##0.0 [$€-407]");
    // The language survives an option change.
    expect(codeForOptions({ category: "number", language: "407", currencyId: "USD-409" }, "[$-407]0.00", { decimals: 3, leading: 1, thousands: true, red: false })).toBe("[$-407]#,##0.000");
    expect(codeForOptions({ category: "date", language: null, currencyId: "USD-409" }, "MM/DD/YYYY", { decimals: 3, leading: 1, thousands: true, red: false })).toBeNull();
    // All: the options follow the code's type.
    expect(codeForOptions({ category: "all", language: null, currencyId: "USD-409" }, "0.00%", { decimals: 1, leading: 1, thousands: false, red: false })).toBe("0.0%");
  });

  it("switches the language: same place in the list, or the new tag", () => {
    const ui = { category: "date" as const, language: null, currencyId: "USD-409" };
    expect(codeForLanguage(ui, "MM/DD/YYYY", "407", "en-US").code).toBe("[$-407]DD.MM.YYYY");
    expect(codeForLanguage({ ...ui, language: "407" }, "[$-407]DD.MM.YYYY", null, "en-US").code).toBe("MM/DD/YYYY");
    expect(codeForLanguage({ ...ui, category: "number" }, '0.0" kg"', "40C", "en-US").code).toBe('[$-40C]0.0" kg"');
    const cur = codeForLanguage({ category: "currency", language: null, currencyId: "USD-409" }, "[$$-409]#,##0.00;-[$$-409]#,##0.00", "407", "en-US");
    expect(cur).toEqual({ code: "#,##0.00 [$€-407];-#,##0.00 [$€-407]", currencyId: "EUR-407" });
  });

  it("previews the cell's value, or AOO's sample", () => {
    expect(numbersPreview("[$$-409]#,##0.00;[RED]-[$$-409]#,##0.00", "en-US", -5, "-5")).toEqual({ text: "-$5.00", color: "var(--tally-danger)" });
    // SvxNumberFormatShell::DEFAULT_NUMVALUE 1234.56789 = 1903-05-18.
    expect(numbersPreview("MM/DD/YYYY", "en-US", null, "")).toEqual({ text: "05/18/1903", color: undefined });
    expect(numbersPreview("#,##0.00", "en-US", null, "")).toEqual({ text: "1,234.57", color: undefined });
    expect(numbersPreview("@", "en-US", null, "hello").text).toBe("hello");
  });
});

describe("review fixes (G14)", () => {
  it("treats a language-tagged code as its preset in the format bar", async () => {
    const { presetFor } = await import("../src/formats");
    expect(presetFor("[$-407]General")?.id).toBe("general");
    expect(presetFor("[$-407]0.00")?.id).toBe("number");
  });

  it("formats cells (glance, chart axes, the web mock) in the code's language", async () => {
    const { liteLocaleOf } = await import("../src/styles/numberFormat");
    expect(formatNumberLite(1234.5, "[$-407]#,##0.00", liteLocaleOf("[$-407]#,##0.00"))).toBe("1.234,50");
    expect(liteLocaleOf("#,##0.00")).toBeUndefined();
    expect(liteLocaleOf("[$-416]0.00")).toBeUndefined();
  });

  it("matches the half- and full-width yen", async () => {
    const { currencyByTag } = await import("../src/styles/numberFormat");
    expect(currencyByTag("¥", "411")?.id).toBe("JPY-411");
    expect(currencyByTag("￥", "411")?.id).toBe("JPY-411");
  });
});
