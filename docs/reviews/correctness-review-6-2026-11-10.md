# Review 6: correctness of G7 (7feec0b), checked against AOO 4.1

Method: I extracted 7feec0b and its parent into scratch and built `tally-numfmt` probes (`corr/src/crates/tally-numfmt/examples/{pc,pcraw,xl}.rs`). I ran 525k (locale, value, code) cases through the Rust engine and through the TS preview (natnum.ts / numfmtNative.ts / numfmtLite.ts run under node 24 type-stripping) and diffed the two. For the Western codes I also diffed the new build against the parent build. The AOO sources are AOO41X raw files in `review6/aoo/`.

## Findings

### 1. Medium: en-US and en-GB have a Jewish "other calendar" in AOO, so the documented "en-US `E` = `26`" is not AOO's output
- Code: `crates/tally-numfmt/src/calendar.rs:111-128` (`other_calendar` / `has_calendar`: no entry for primary language 0x09). The TS mirror is `packages/app/src/natnum.ts:384-396`. The claim is in `docs/parity-deviations.md` G7 row ("en-US `G` = `AD`, `E` = `26`") and in `docs/numfmt-natnum.md`.
- AOO: `i18npool/source/localedata/data/en_US.xml:367` declares `<Calendar unoid="jewish" default="false">`: months Nissan…Adar B, Saturday "Shabbat", eras BCE/CE. `en_GB.xml:246` has `<LC_CALENDAR ref="en_US"/>`. `zformat.cxx` `ImpIsOtherCalendar` returns true for E/EE/R/RR/AAA/AAAA. `SwitchToOtherCalendar` then loads the first non-Gregorian entry of `getAllCalendars(en-US)`, which is `jewish`. `calendar_jewish.cxx` `getDisplayString` forces NatNum2; NatNum2 is invalid for en, so the digits stay ASCII. SHORT_YEAR is `year % 1000`.
- Reproduction (en-US, value 46314 = 2026-10-19, which is 8 Heshvan 5787):

  | code | Tally | AOO |
  |---|---|---|
  | `G E` | `AD 26` | `CE 787` |
  | `GG EE` | `AD 2026` | `CE 5787` |
  | `RR` | `Anno Domini2026` | `CE5787` |
  | `AAAA` on a Saturday | `Saturday` | `Shabbat` |
  | `[~jewish]D MMMM YYYY` | `19 October 2026` | `8 Heshvan 5787` |

  en-GB gives the same (`E` → `26`). `G` with no E/R/AAA in the section is correctly `AD`.
- Knock-on effects in the file filters:
  - XLSX: `[$-1080409]d mmmm yyyy` imports as `[$-409][~jewish]…`. `excel.rs:69` deliberately keeps en for Jewish, but the formatter then ignores the `[~jewish]` and shows the Gregorian date.
  - ODS: `xmlnumfe.cxx` `lcl_GetDefaultCalendar(en-US)` writes `number:calendar="jewish"` on E/EE/R/RR/AAA elements. Tally writes no calendar.
- Fix: add en (primary 0x09) → `Jewish` to `other_calendar` / `has_calendar`, in both Rust and TS. Give the Jewish calendar per-language names: the en table from en_US.xml (months Nissan…Adar B, days Sun…"Shabbat", eras BCE/CE). The forced-NatNum2 path already leaves the digits ASCII for en. Then correct the G7 row and add cases to `natnum_cases.json` (`G E` → `CE 787`). Alternatively keep Gregorian, but then document it as a deliberate deviation and remove the "en-US `E` = `26` as AOO" claim.

### 2. Medium (regression): XLSX export drops a `[DBNumN]` that has no language tag
- Code: `crates/tally-numfmt/src/excel.rs:138` writes `[DBNum]` only `if let (Some(nn), Some(l)) = (s.natnum, lang)`. `excel.rs:252` now also swallows `Token::DbNum(_)`. Before G7, DbNum fell through to `print_token` and was kept.
- Reproduction (probe `xl`, old vs new build):

  | `to_excel_code` input | parent 7feec0b^ | 7feec0b |
  |---|---|---|
  | `[DBNum1]0` | `[DBNum1]0` | `0` |
  | `[DBNum1]General` | `[DBNum1]General` | `General` |
  | `[DBNum1]General;[DBNum1]-General` | – | `General;-General` |

  So an XLSX → XLSX round trip of `[DBNum1]General`, a form Japanese Excel writes, loses the modifier. In a ja-JP document Tally shows 一二三四 and the exported file shows 1234.
- Fix: when `nn.dbnum` is set, emit `[DBNum{n}]` whatever `lang` is (Excel resolves it with its own locale). Only the NatNum→DBNum or numeral-shape mapping needs a language. A NatNum without a tag could also fall back to the document language, as the ODS writer does with `doc_lang`.

### 3. Medium: the Format Cells preview hangs on large serials with a Jewish date, and shows `NaN` or invented years where the engine shows `###`
- Code:
  - `packages/app/src/numfmtNative.ts:216-245` (`formatDateNative`) has no range check. The engine's `date_section` returns `Err` and prints `###`.
  - `packages/app/src/natnum.ts:466-468` (`hebrewDate`) starts from `abs/366` and steps `year++` about `abs·5.6e-6` times.
- Reproduction (`node ts/run.ts`, en-US):

  | value | code | TS preview | engine |
  |---|---|---|---|
  | `1e15` | `[$-40D]EE` | does not return; killed after 30 s | `###` |
  | `1e15` | `[$-411]GGGE` | `令和NaN` | `###` |
  | `100000000` | `[$-411]GGGE"年"M"月"D"日"` | `令和273672年9月11日` | `###` |
  | `100000000` | `[~hijri]D/M/YYYY` | `11/9/275690` | `###` |

  The preview runs on the selected cell's value on the UI thread, so the first case freezes the dialog.
- Fix: in `formatDateNative`, return `"###"` (or `null`) when the serial falls outside the engine's date range: Rust accepts −693593 … 2958465 (D1899), i.e. year 1 to 9999. Do this before `calendarFields` runs.

### 4. Low: the TS preview prints a doubled sign for negative General with a NatNum
- Code: `packages/app/src/numfmtLite.ts:263-264`. `general(value)` already contains the `-`, and the next line prepends another.
- Reproduction (the preview diff found 1,700 such rows; examples below). `[NatNum1]General` in a ja document behaves the same way.

  | value | code | TS preview | engine (and AOO) |
  |---|---|---|---|
  | `-1` | `[NatNum1][$-411]General` | `-−一` | `−一` |
  | `-1` | `[NatNum3]General` | `-－１` | `－１` |

- Fix: add the sign only when the body is not General, or format `Math.abs(value)`.

### 5. Low: fractions transliterate the sign and the literal text; AOO transliterates only the integer, the numerator and the denominator
- Code: `crates/tally-numfmt/src/format.rs:452-461` runs `transliterate_out` on the whole output of any Number section with a digit token, fractions included.
- AOO: `zformat.cxx:2429` `ImpTransliterate(sStr)` (integer part) and `:2438-2439` `ImpIntToString` (numerator and denominator). The sign is added afterwards as ASCII (`OutString.Insert('-',0)`, fraction block line ~2482), and literal strings are copied untouched.
- Reproduction (en-US):

  | value | code | Tally | AOO |
  |---|---|---|---|
  | `-1.5` | `[NatNum1][$-411]# ?/?` | `−一 一/二` | `-一 一/二` |
  | `1.5` | `[NatNum1][$-411]# ?/? "円1"` | `一 一/二 円一` | `一 一/二 円1` |

- Fix: for `lay.fraction` sections, transliterate the three digit groups separately instead of calling `transliterate_out`. The TS port needs the same change.

### 6. Low: two General + NatNum cases differ from `ImpGetOutputStandard`
- Code: `format.rs:470-478` and `format.rs:430-440`.
- AOO, `zformat.cxx:1805-1863`:
  - `ImpGetOutputStdToPrecision` transliterates with `NumFor[0]`'s NatNum even when the General keyword sits in another section (`ImpGetNumberOutput`, NF_KEY_GENERAL).
  - Values with |x| > 1e15 take the `rtl_math_StringFormat_E` branch, which is never transliterated.
- Reproduction:

  | value | code | Tally | AOO |
  |---|---|---|---|
  | `-5` | `[NatNum1][$-411]General;General` | `5` | `五` |
  | `1e16` | `[NatNum1][$-411]General` | `一E+一六` | `1E+016`, untransliterated |

- Fix: in General sections, use the first section's NatNum when the current one has none, and skip transliteration when the General text came out in E notation.

### 7. Low (regression): a lowercase `r` used as a literal comes out upper-case
- Code: `crates/tally-numfmt/src/parse_format.rs:575` replaces the token with `Token::Literal('R')` whatever case was typed.
- Reproduction (parent vs new, all locales): `r0` with 1 → `r1` before, `R1` now. The code also prints back as `R0`. `RR #` and `R 0.00` are unaffected, because they are upper-case already.
- Fix: keep the original characters. Store the source char in `EraAndYear` or re-scan the original slice.

### 8. Low: Western era names differ from AOO's locale data (now reachable through `G`)
- Code: `crates/tally-numfmt/src/locale.rs:125-126` and `183-184` (en: `GGG` = `Anno Domini`), `289-290` (fr), `347-348` (es).
- AOO localedata `<Eras>`:

  | locale | abbreviated (G / GG) | full (GGG) |
  |---|---|---|
  | en_US | `AD` | `AD` |
  | fr_FR | `apr. J.-C.` | `ap. J.-C.` |
  | es_ES | `d.C.` | `d.C.` |
  | de_DE | matches Tally | matches Tally |

- Reproduction:

  | locale | value | code | Tally | AOO |
  |---|---|---|---|---|
  | en-US | 46314 | `GGG` | `Anno Domini` | `AD` |
  | fr-FR | 46314 | `G GG GGG` | `ap. J.-C. ap. J.-C. après Jésus-Christ` | `apr. J.-C. apr. J.-C. ap. J.-C.` |

- Fix: copy the strings from localedata, or record the difference as a deviation.

### 9. Low: AM/PM in the Hanja calendar should be Japanese
- AOO: `calendar_gregorian.cxx:194-205` (`Calendar_hanja::getDisplayName` AM_PM returns the ja locale's 午前 / 午後). `zformat.cxx` gets AM/PM from `rCal.getDisplayName(AM_PM, …)`.
- Reproduction (en-US): value 46314.75, code `[$-412][~hanja]AM/PM HH` gives Tally `PM 06`; AOO gives `午後 06`. The same applies under implicit Hanja (`[$-412]AAAA AM/PM`).
- Fix: in `date_section_cal`, use 午前 / 午後 for `AmPm` when `cur == Hanja`.

## Verified correct (no finding)

- **numberchar.h tables.** `NumberChar`, `DecimalChar`, `MinusChar` and `SeparatorChar` match byte for byte (scripted compare). So do the `MultiplierChar_{2,6,7}` / exponent tables, natnum4–11, natnum1/2, `getLanguageNumber` (zh TW/HK/MO) and `isValidNatNum`, including the Hebrew NatNum2 fall-through.
- **Spelled numbers, traced by hand against `AsciiToNative` / `AsciiToNative_numberMaker`:**

  | input | lang / mode | output |
  |---|---|---|
  | 1001 | zh NatNum4 | 一千〇一 |
  | 10010 | zh NatNum4 | 一万〇一十 |
  | 100000000 | zh NatNum4 | 一亿 |
  | 10^16 | zh NatNum4 | 一万兆 |
  | 10 | zh NatNum4 | 一十 |
  | 1001 | zh NatNum7 | 千一 |
  | 110 | zh NatNum7 | 百十 |
  | 1001 | ja NatNum4 | 一千一 |
  | 10010 | ja NatNum4 | 一万十 |
  | 10^16 | ja NatNum4 | 一京 |
  | 110 | ja NatNum4 | 百十 (omit 1 only before 百 / 十) |
  | -12.5 | zh NatNum4 | －一十二．五 |
  | 1234567 | he | א רלד תקסז |
  | -5 | he | ה (AOO drops the minus too: `neg` is unused) |

- **DBNum ↔ NatNum.** Both tables match `zformat.cxx:189-318`, including the date column. Dates take the date mapping only when `has_date` is set, as `SetNatNumDate(eScannedType & DATE)` does.
- **`NatNumForCalendar`.** Year ≥ 100 and quarters are "short"; ja 1→4 and 2→5; zh / ko 1→7 and 2→8; ko 4→9 or 11; NatNum4 in ja/zh gives 0; DBNum4 ko dates (→9) give 0 (ASCII), as in AOO.
- **元.** Shown only for gengou SHORT_YEAR / LONG_YEAR = 1 with the section's mode 1 or 2. So `[NatNum3]` gives `令和１` and `[DBNum2]` on a date gives 元.
- **Gengou era table.** Matches `gengou_eraArray`: 1868-01-01, 1912-07-30, 1926-12-25, 1989-01-08, 2019-05-01. Boundary dates check out (S64 / H1, 大正01, 明治1, pre-Meiji `西暦1867` fallback through the Dummy era). LONG_YEAR is `%02d` only in gengou; SHORT_YEAR shows all digits for era-array calendars.
- **ROC and Buddhist years.** `mapFromGregorian`: ROC 1911 is 民國前1, ROC 9999 is 8088, Buddhist is Gregorian + 543. RR with Buddhist era 0 uses the swapped order.
- **Hijri.** `NewMoon` / `getHijri` are an exact port (C truncation preserved); 2001-03-26 is 1/1/1422.
- **Jewish.** The Dershowitz–Reingold port matches. SHORT_YEAR is %1000 and NatNum2 is forced. 2026-10-19 is ח׳ חשון תשפ״ז.
- **xmlnumfe / xmlnumfi.**
  - `convertTo/FromXmlAttributes` match.
  - Writing: E/EE/R/RR put the default calendar on every element; AAA/AAAA put it on their element only.
  - Reading: `UpdateCalendar` puts `[~cal]` on change, and Y after G becomes E/EE, which loses information exactly as AOO does.
- **XLSX extended ids round-trip** (`[$-D07041E]`, `[$-2000401]`, `[$-30411]`, `[$-1E000804]`, `[$-27000412]`, `[$-40404]`).
- **Western fast path.** The parent and new builds were diffed over 6,264 plain cases (6 locales × 58 codes × 18 values). The only changes are `E0` (now Gregorian SHORT_YEAR), `r0` (finding 7) and `[DBNum0]`, which is now a parse error as in AOO (`'1'..'9'` only). The calendar path gives the same output as the plain path for 30 Western date/time codes, with BCP-47 tags and with `[NatNum0]` / `[NatNum3]`.
- **Rust vs TS.** Apart from findings 3 and 4, the remaining diffs are the Lite path's existing behaviour for codes with no G7 modifier (padding of `# ?/?`, `NNNN` / `Q` / `WW` / `MMMMM` unsupported, `[$-411]` names in English, negative times). Everything else in the native path agrees across 497k cases.
