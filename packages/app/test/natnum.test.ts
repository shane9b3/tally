import { describe, expect, it } from "vitest";
import cases from "../../../crates/tally-numfmt/tests/data/natnum_cases.json";
import { transliterate, LANG_JA, LANG_KO, LANG_ZH_CN, LANG_HE, LANG_TH, calendarFields } from "../src/natnum";
import { formatNumberLite } from "../src/numfmtLite";
import { languageOf, previewText } from "../src/styles/numberFormat";

const SEP = { decimal: ".", group: "," };

describe("natnum transliteration (port of tally_numfmt::natnum)", () => {
  it("spells and transliterates as AOO", () => {
    expect(transliterate("1234", LANG_JA, 1, SEP)).toBe("一二三四");
    expect(transliterate("1234", LANG_JA, 4, SEP)).toBe("一千二百三十四");
    expect(transliterate("110", LANG_JA, 4, SEP)).toBe("百十");
    expect(transliterate("1001", LANG_ZH_CN, 4, SEP)).toBe("一千〇一");
    expect(transliterate("1234", LANG_ZH_CN, 7, SEP)).toBe("千二百三十四");
    expect(transliterate("1234", LANG_KO, 10, SEP)).toBe("일천이백삼십사");
    expect(transliterate("5786", LANG_HE, 2, SEP)).toBe("ה׳ תשפ״ו");
    expect(transliterate("1,234", LANG_TH, 1, SEP)).toBe("๑,๒๓๔");
    expect(transliterate("123", 0x409, 1, SEP)).toBe("123");
  });

  it("computes calendar fields", () => {
    expect(calendarFields("gengou", 2019, 5, 1, 18017, 3)).toMatchObject({ era: 5, year: 1 });
    expect(calendarFields("ROC", 2026, 1, 1, 0, 0)).toMatchObject({ era: 1, year: 115 });
    expect(calendarFields("hijri", 2001, 3, 26, 11407, 1)).toMatchObject({ era: 1, year: 1422, month: 1, day: 1 });
    expect(calendarFields("jewish", 2026, 9, 12, 20708, 6)).toMatchObject({ year: 5787, month: 7, day: 1 });
  });
});

const LCID: Record<string, string> = { "en-US": "409", "ja-JP": "411", "de-DE": "407", "fr-FR": "40C", "es-ES": "C0A", "en-GB": "809" };
const SEPS: Record<string, { decimal: string; group: string }> = { "de-DE": { decimal: ",", group: "." }, "fr-FR": { decimal: ",", group: " " }, "es-ES": { decimal: ",", group: "." } };

describe("the Format Cells preview matches the engine (shared table)", () => {
  for (const c of cases as { value: number; code: string; locale: string; want: string }[]) {
    it(`${c.code} [${c.locale}] ${c.value}`, () => {
      const sep = SEPS[c.locale] ?? { decimal: ".", group: "," };
      expect(formatNumberLite(c.value, c.code, { ...sep, lcid: LCID[c.locale] })).toBe(c.want);
    });
  }

  it("passes the document language through previewText", () => {
    expect(previewText(1234, "[NatNum1]0", languageOf("ja-JP"))).toBe("一二三四");
    expect(previewText(1234, "[NatNum1]0", languageOf("en-US"))).toBe("1234");
    expect(previewText(46314, "[$-411]GGGE\"年\"", languageOf("en-US"))).toBe("令和8年");
  });

  it("shows ### for dates outside the engine's range, quickly (review 6 H1)", () => {
    // 1e20 used to loop forever in hebrewDate; others gave NaN or invented years.
    const codes = [
      "[$-40D]EE",
      "[$-40D][~jewish]D",
      "[$-40D][~jewish]YYYY-MM-DD",
      "[~jewish]D MMMM YYYY",
      "G E",
      "[$-401]EE",
      "[$-401][~hijri]D/M/YYYY",
      "[$-411]GGGE",
      "[$-404]GE",
      "[$-41E]GG EE",
      "[$-412]GGG E",
      "[NatNum1][$-411]YYYY",
    ];
    const t0 = performance.now();
    for (const code of codes) {
      for (const v of [1e15, 1e20, 1e308, -1e308, 1e8 + 1, 2958466, -693594]) expect(formatNumberLite(v, code), `${code} ${v}`).toBe("###");
      for (const v of [Infinity, -Infinity, NaN]) expect(formatNumberLite(v, code), `${code} ${v}`).toBe("#NUM!");
    }
    expect(performance.now() - t0).toBeLessThan(1000);
    // The range ends are still shown.
    expect(formatNumberLite(2958465, "[~jewish]D MMMM YYYY")).toBe("28 Heshvan 13760");
    expect(formatNumberLite(-693593, "[~jewish]D MMMM YYYY")).toBe("18 Tevet 3761");
  });

  it("computes the Jewish year directly (no long search)", () => {
    // 9999-12-31 and 0001-01-01 (unix days).
    expect(calendarFields("jewish", 9999, 12, 31, 2932896, 5)).toMatchObject({ year: 13760, month: 8, day: 28 });
    expect(calendarFields("jewish", 1, 1, 1, -719162, 1)).toMatchObject({ year: 3761, month: 10, day: 18 });
  });

  it("keeps R next to digits as text and plain codes on the fast path", () => {
    expect(formatNumberLite(1234.5, "R #,##0.00")).toBe("R 1,234.50");
    expect(formatNumberLite(46314, "YYYY-MM-DD")).toBe("2026-10-19");
    expect(formatNumberLite(5, "General")).toBe("5");
  });
});
