# Native numbers and calendars in number formats (G7)

Status: implemented 2026-11-09 (`crates/tally-numfmt/src/natnum.rs`,
`calendar.rs`, `format.rs::date_section_cal`; preview port
`packages/app/src/natnum.ts` + `numfmtNative.ts`).

Sources (AOO41X branch): `svl/source/numbers/zformat.cxx`
(`SvNumberNatNum::MapDBNumToNatNum` / `MapNatNumToDBNum`, the `[NatNum` /
`[DBNum` / `[$-` prefix scan of the constructor, `ImpTransliterate`,
`ImpIntToString`, `ImpGetDateOutput` / `ImpGetDateTimeOutput` /
`ImpGetTimeOutput`, `ImpIsOtherCalendar`, `SwitchToOtherCalendar`,
`ImpFallBackToGregorianCalendar`, `ImpAppendEraG`, `ImpGetLanguageType`),
`svl/source/numbers/zforscan.cxx` (keywords `E EE R RR G GG GGG AAA AAAA`,
`[~calendar]`), `i18npool/source/nativenumber/nativenumbersupplier.cxx` and
`data/numberchar.h`, `i18npool/source/calendar/calendar_gregorian.cxx`
(eras tables, `mapFromGregorian`, `getDisplayString`, `NatNumForCalendar`),
`calendar_hijri.cxx`, `calendar_jewish.cxx`, `calendarImpl.cxx`
(`loadCalendar`), `i18npool/source/localedata/data/{ja_JP,zh_TW,zh_CN,ko_KR,th_TH,ar_EG,ar_SA,he_IL}.xml`,
`xmloff/source/style/xmlnumfe.cxx` / `xmlnumfi.cxx` (ODF). For Excel's
extended locale ids: LibreOffice `SvNumberformat::ImpObtainCalendarAndNumerals`
and `LocaleType`.

## NatNum modes

`[NatNumN]` transliterates the formatted text in the section's NatNum
language: the section's own `[$-LCID]`, else the format's language (the
code's first tag, else the document locale). `isValidNatNum` decides; an
invalid mode leaves the text unchanged (`[NatNum1]0` in en-US shows `5`).

| N | Text | Languages |
|---|---|---|
| 0 | native digits → ASCII (Tally: digits and separators only, no multiplier parsing) | all |
| 1 | digit by digit, lower: 一二三 / ๑๒๓ / ١٢٣ / ۱۲۳ / Hebrew numbering | zh, ja, ko, he, ar, th, hi, or, mr, bn, pa, gu, ta, te, kn, ml, lo, bo, my, km, mn, ne, dz, fa |
| 2 | digit by digit, upper / financial: 壹贰叁 / 壱弐参; Hebrew with geresh | zh, ja, ko, he |
| 3 | full-width ０１２ (decimal and grouping separators full width too) | all |
| 4 / 5 / 6 | spelled with units (一千二百三十四), lower / upper / full-width digits | zh, ja, ko |
| 7 / 8 | the same, short (zero and one omitted: 千二百三十四) | zh, ja, ko |
| 9 / 10 / 11 | Hangul digits / Hangul spelled long / short | ko |

The tables of `numberchar.h` are copied verbatim (AOO's Oriya and
Malayalam code points and Tamil's ASCII zero included; LibreOffice has the
same). Japanese spelled forms omit 1 only before 十 and 百
(`NUMBER_OMIT_ONE_67`), Korean and Japanese never write 〇 inside a number.
The decimal, grouping and minus signs before a digit map per number set
(Japanese `・` and `−`, Chinese `．` and `、`, Arabic `٫` / `٬`).

Where AOO transliterates (zformat.cxx), and so Tally:

- number sections: the whole output, literals and sign included
  (`ImpGetNumberOutput`); scientific sections and literal-only sections are
  not transliterated (AOO does not either); General (`ImpGetOutputStandard`);
- fractions: integer, numerator and denominator;
- dates: each calendar field through `getDisplayString` with
  `NatNumForCalendar` — in zh / ja / ko, NatNum1 becomes 4 (ja) or 7 (zh,
  ko) for days, months and years below 100 (十九日), years from 100 stay
  digit by digit (二〇二六年), NatNum4+ give nothing except Korean
  (NatNum4 → 9 or 11); month and day names are not transliterated;
- times, week numbers, elapsed values and fractional seconds: digit
  strings (`ImpIntToString`, zero-padded first: 一四:三〇);
- text sections and `@`: never.

Tally takes the decimal and grouping separators from the locale the text
was formatted with; AOO takes them from the NatNum language's locale data.
They differ only for a NatNum language that is not the format's language
(and Arabic, whose own separators Tally has no locale for: `123.45` with
`[NatNum1][$-401]` shows `١٢٣٫٤٥`, as AOO's Arabic-formatted `١٢٣٫٤٥`).

## DBNum

`[DBNumN]` (N 1..9) is kept as written and mapped per language at format
time (`MapDBNumToNatNum`; dates use the date column):

| | zh | ja | ko | date (all) |
|---|---|---|---|---|
| DBNum1 | 4 | 1 | 1 | 1 |
| DBNum2 | 5 | 4 | 2 | 2 |
| DBNum3 | 6 | 5 | 3 | 3 |
| DBNum4 | – | 7 | 9 | ko 9 |

Other languages get NatNum 0 (nothing). ODS stores the mapped NatNum
(AOO writes no DBNum), so `[DBNum2][$-411]0` reads back as
`[NatNum4][$-411]0`.

## Language tags and Excel locale ids

`[$-LCID]` is hex; like AOO's 16-bit `LanguageType` only the low 16 bits
are the language. BCP 47 tags with a known language and country are also
accepted (`[$-ja-JP-x-gannen]`, `[$-de-DE]`): Excel writes them.

Excel's extended ids `[$-NNCCLLLL]` (numeral shape `NN`, calendar `CC`,
language `LLLL`) are expanded on XLSX import (`excel::from_excel_code`)
to AOO's spelling, as LibreOffice does:

- numeral shape `02`–`13` → `[NatNum1]` in the reference language when the
  locale's language does not have those digits (`02` Arabic-Indic, `03`
  Persian, `04` Devanagari, `05` Bengali, … `0D` Thai, `0E` Lao, `0F`
  Tibetan, `10` Myanmar, `12` Khmer, `13` Mongolian); `1B`–`26` →
  `[NatNum1..3]` in ja (`1B`–`1D`), zh-CN (`1E`–`20`), zh-TW (`21`–`23`),
  ko (`24`–`26`); `27` → `[NatNum9]` ko;
- calendar `03` → gengou (ja), `04` → ROC (zh-TW, Tally addition: LibreOffice
  ignores it), `06` / `17` → `[~hijri]` (ar), `07` → `[~buddhist]` (th),
  `08` → `[~jewish]`; the language becomes the calendar's when the
  locale's has no such calendar (`[$-1070409]` → `[$-41E][~buddhist]`);
  `01`/`02` (Gregorian) and others are dropped.

`[$-D07041E]d/m/yyyy` → `[NatNum1][$-41E][~buddhist]D/M/YYYY`. Export
(`to_excel_code`) reverses it: a zh / ja / ko NatNum with a DBNum
equivalent is written `[DBNumN]`, other NatNum1–3 as a numeral-shape byte,
`[~calendar]` as a calendar byte (`[$-D07041E]`), era codes as Excel's
`g` / `e` in East-Asian sections (`R` → `ee`, `RR` → `gggee`); a NatNum
Excel cannot show (no language, NatNum4+ outside zh/ja/ko) is dropped.

`[$-ja-JP-x-gannen]` keeps its tag (format language ja) and shows the
first gengou year as 元 without a NatNum (Excel's gannen setting; AOO
shows 元 only with NatNum1/2). ODS has no place for it: it is written as
`number:language="ja"`.

## Calendars

| Calendar | Locale (AOO localedata) | Years | Eras |
|---|---|---|---|
| gregorian | all | proleptic Gregorian (Tally has no Julian switch) | locale's (`BC`/`AD`, 紀元前/西暦 …) |
| gengou | ja | era year, eras from 1868-01-01, 1912-07-30, 1926-12-25, 1989-01-08, **2019-05-01** | 明治 大正 昭和 平成 **令和** (G: M T S H **R**) |
| ROC | zh-TW | 1912 = 1, before: 民國前 n | 中華民國 / 民國 |
| buddhist | th | Gregorian + 543 | พุทธศักราช / พ.ศ. |
| hanja | ko | Gregorian | 西曆紀元 / 西紀 |
| hijri | ar (not ar-OM) | AOO's astronomical new-moon computation (`NewMoon`, reference 1 Muharram 1422 = 2001-03-26) | هجري |
| jewish | he | Dershowitz–Reingold; numbers always Hebrew (NatNum2) | לסה"נ |

- `E`, `EE`, `R`, `RR`, `AAA`, `AAAA` switch the whole section to the
  language's other calendar (`ImpIsOtherCalendar`) unless a `[~calendar]`
  comes first; `YY`, `YYYY`, `DDD`, `DDDD` stay Gregorian then
  (`SwitchToGregorianCalendar`). `[~name]` switches at its position, only
  to a calendar the format language has (`loadCalendar` fails otherwise,
  so `[~buddhist]` in en-US does nothing). A gengou date before Meiji
  falls back to Gregorian (`ImpFallBackToGregorianCalendar`).
- Field text (`getDisplayString`): `E` short year — all digits in era
  calendars, else two digits from 100 on (en-US `E` = `26`); `EE` / `R`
  long year (gengou two digits `08`); `RR` long era + long year (Buddhist
  era 0: short year + short era); `G` the gengou letter or the short era;
  `GG` short, `GGG` long era. Year 1 of an era is 元 with NatNum1/2.
- Names: the calendar's own day and month names (gengou and hanja weekdays
  日 … 土曜日, ROC Chinese, Buddhist Thai, Hijri and Hebrew month names);
  Gregorian names and the gengou months come from the Tally locale.

### Deliberate differences from AOO 4.1

- Reiwa: AOO41X has the 2019-05-01 era boundary but the era names are the
  typo 苓 / 苓和 and `G` shows `?`; Tally uses 令 / 令和 and `R`, as ICU and
  LibreOffice do.
- Gengou month names: AOO's ja_JP gengou calendar (and its ja Gregorian
  calendar) has English month names; Tally keeps the ja-JP locale's `10月`.
- Gregorian `YY` / `YYYY` keep Tally's two / four digits (AOO: `5` for a
  year below 10 with `YY`, no padding with `YYYY`).
- NatNum0 converts native digits and separators only, not spelled numbers
  with multipliers.
- The Thai `t` modifier of Thai-locale codes (`NF_KEY_THAI_T`) is not
  recognised (only typed in a Thai document or imported from Thai Excel).
- Excel `B1` / `B2` year codes are not translated.

## Files

- ODS: `number:transliteration-format` / `-language` / `-country` /
  `-style` on the data style ↔ `[NatNumN]` (+ the tag when the language
  differs from `number:language`); `number:calendar` on date elements ↔
  `[~calendar]` (a change inserts it before the element, as
  `SvXMLNumFormatContext::UpdateCalendar`), a year after an era reads as
  `E` / `EE`; on writing E / EE / R / RR put the language's other calendar
  on every date element and AAA / AAAA on their weekday element
  (`lcl_GetDefaultCalendar`). See docs/file-formats.md.
- XLSX: see above.
- The Format Cells preview (`numfmtLite`) uses the TypeScript port for
  codes with these modifiers; the table
  `crates/tally-numfmt/tests/data/natnum_cases.json` is run by both
  `tests/natnum_cases.rs` and `packages/app/test/natnum.test.ts`.
