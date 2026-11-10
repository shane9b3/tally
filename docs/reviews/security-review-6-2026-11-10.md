# Security review 6: G7 number-format transliteration and calendars (commit 7feec0b)

Scope: `crates/tally-numfmt` (natnum.rs, calendar.rs, format.rs, parse_format.rs, locale.rs, excel.rs), the ODS/XLSX number-style read/write changes, and `packages/app/src/natnum.ts` / `numfmtNative.ts` / `numfmtLite.ts`. Probes ran on `git archive 7feec0b` in `scratchpad/review6/src` (release and debug builds), and the TS port ran under node 24 (`--experimental-strip-types`) in `scratchpad/review6/ts`. Probe source: `scratchpad/review6/src/crates/tally-numfmt/examples/probe6.rs` and `scratchpad/review6/ts/probe.ts`.

## Summary

| # | Sev | Where | What |
|---|---|---|---|
| H1 | High | `packages/app/src/natnum.ts:466-472` (`hebrewDate`), reached through `numfmtNative.ts:216-247` | The UI hangs (infinite loop) when a Jewish-calendar format meets a large value. The TS port has no date-range check. |
| H2 | High | `crates/tally-numfmt/src/calendar.rs:344`, `format.rs:1505` | Every Jewish-calendar cell takes about 50 µs because the year search starts about 7,500 years too early. A `[~jewish]` tag repeated about 450 times takes 22 ms per cell. |
| H3 | High | `crates/tally-numfmt/src/parse_format.rs:569-579` (new) and `:280`/`keyword` (older), through `tally-functions/src/categories/text/text_fn.rs:39-41` | `TEXT()` has no format length cap, and the new `RR` → literal `splice` is O(n²). One formula can hang recalc for about 24 minutes. |

Checked with no finding:
- Fuzz run, release: 60,000 random modifier/era/calendar codes × 22 values (NaN, ±inf, ±1e308, 5e-324, serial bounds 2958465/2958466/-693593/-693594, ±1e8), going through `format_value_with`, `format_value_in_locale` (both epochs, text values), `from_excel_code` and `to_excel_code`, and round-tripping each. Result: 0 panics. Slowest call: 0.53 ms.
- The same fuzz in a debug build (overflow checks on), 8,000 × 22: 0 panics.
- NatNum 0..19 × 8 languages × {`0`, `General`, `#,##0.00`, `# ?/?`} × extreme values, release and debug: no panic and nothing over 2 ms. A 4,000-digit `0` pad with NatNum4/7 and the NatNum0 reverse mapping on 4,000 CJK literals each run under 0.1 ms.
- Nothing found in these areas:
  - Hebrew numbering: the 19-digit cap keeps `u64` from overflowing.
  - `number_maker`: recursion depth is bounded by the 2/6/7-entry `exp` tables.
  - Hijri: the 64-step guard holds.
  - Gengou/ROC/Buddhist: closed form.
  - Name tables: all indexed with `get()` or `saturating_sub`.
  - `lcid_value` BCP-47 parsing: ASCII-only checks, no slicing.
  - `[$-…]` `rfind('-')` slicing: always on an ASCII boundary.
  - `from_excel_code`: expands only the first extended id per section, and its output tags are ≤ 0xFFFF, so nothing re-expands.
  - ODS `number:calendar`: capped at 32 bytes and escaped on write (`emit_dated`, `escape_attr`).
  - `from_xml_attributes("")`: returns `None`.

---

## H1 (High): TS calendar port loops forever on large serials (UI main-thread hang from file data)

**Where:** `packages/app/src/natnum.ts:466-472`:
```ts
function hebrewDate(abs: number): [number, number, number] {
  let year = Math.trunc((abs - 1373429) / 366);
  while (abs >= hebAbs(7, 1, year + 1)) year++;
  ...
```
`formatDateNative` (`numfmtNative.ts:216-247`) passes `unix = days - 25569` straight into `calendarFields`, with no range check. The engine's `unix_days_from_serial` does have one (|serial| ≤ 1e8 and year 1..9999), so Rust returns an error for these values while TS keeps going. Two problems in the loop:
- The start estimate is low by about `abs·(1/365.2468 − 1/366) + 7,500` years, so the loop runs millions of times.
- Above about 2^53 / 366 the step `year + 1 === year`, so the loop never ends.

**Measured** with `formatNumberLite(v, "[$-40D]EE")`:

| v | Time |
|---|---|
| 1e9 | 2.4 ms |
| 1e11 | 103 ms |
| 1e12 | 1.2 s |
| 1e13 | 13.1 s |
| 1e20 | did not finish in 45 s (infinite) |
| 1e300 | did not finish in 45 s (infinite) |

`"[$-40D][~jewish]YYYY-MM-DD"` with 1e20 or 1e308 also did not finish. The other calendars return garbage such as `令和NaN` but finish.

**How a file reaches it:** `formatNumberLite` uses the native path for these codes. It is called with values and codes from the document in three places:
- **Chart value axes:** `charts/toEcharts.ts:196-199` `axisFormatter`. The axis format falls back to the source cells' number format (`tally-engine/src/charts.rs:2020`), and ECharts calls the formatter for every tick.
- **Date-axis labels:** `chartAxes.ts:121-125`.
- **Glance panel and Format Cells preview:** `glance.ts:105` and `styles/numberFormat.ts:478`, which use the selected cell's value.

**Trigger:** an ODS or XLSX file with a cell holding `1E+20`, styled `[$-40D]EE` (XLSX: `[$-40D]ee` or `[$-8040D]…`), and a chart over that cell. The WebView freezes as soon as the chart renders, and also when the user opens Format Cells on the cell.

**Fix:**
1. In `formatDateNative`, when `hasDate` is set, return `null` (the callers fall back) unless the serial is finite, |serial| ≤ 1e8, and the Gregorian year is in 1..9999. This is the same range as `unix_days_from_serial`.
2. In `hebrewDate`, start from `Math.trunc((abs + 1373429) / 366)` (see H2), and add a hard iteration guard to both `while` loops.
3. Add natnum.test.ts cases: 1e20, 1e308, -1e308, Infinity and NaN with `[$-40D]EE`, `[$-40D][~jewish]D` and `[$-401]EE` must return within milliseconds.

## H2 (High): Jewish calendar costs 50 µs per cell, and repeated `[~jewish]` multiplies it (render DoS)

**Where:** `crates/tally-numfmt/src/calendar.rs:344`:
```rust
let mut year = (d + HEBREW_EPOCH) / 366;   // HEBREW_EPOCH = -1_373_429
```
The estimate should be `(d - HEBREW_EPOCH) / 366`. As written it starts about 2·1,373,429/366 ≈ 7,500 years below the answer. Each step of the `while` calls `hebrew_to_absolute`, which runs up to 13 months × `days_in_hebrew_year`, which calls `hebrew_elapsed_days` twice.

On top of that, `date_section_cal` recomputes `at(c)` for every `Token::Calendar` (`format.rs:1505-1517`) and memoizes nothing. A 4,096-character code can hold about 450 `[~jewish]` tags.

**Measured** (release, `format_value_with`):
- `[$-40D][~jewish]D` and `[$-40D]EE`: 48-52 µs per cell at every serial in range. For comparison, `[$-401]EE` takes 0.22 µs.
- `[$-40D]` + 450 × `[~jewish]` + `D` (4,058 characters): 21.8-22.3 ms per cell.

The render budget in `security.md` §8 is 20,000 cells in under 100 ms (`long_number_formats_render_100ms_release`, about 5 µs per cell). Against that budget:
- The plain code takes about 1 s per viewport.
- The 450-tag code takes about 440 s per viewport.

Each viewport fetch holds the document lock, and exports such as CSV/HTML/PDF format every cell. One style applied to a column makes the document unusable.

**Trigger:** an ODS data style `<number:day number:calendar="jewish"/>` repeated with alternating calendars (each change inserts a `[~…]`), or an XLSX numFmt `[$-40D][~jewish][~jewish]…d` capped at 4,096 characters, applied to `A1:Z50000`.

**Fix:**
1. `let mut year = (d - HEBREW_EPOCH) / 366;`. Verified in the scratch copy:
   - Identical `(year, month, day)` to the current code for every R.D. day 1..3,652,099 (Gregorian 0001-01-01..9999-12-31).
   - Time drops to 0.46-0.57 µs per cell (about 100× faster).
   - The 450-tag code drops to 104-148 µs per cell.
2. In `date_section_cal`, memoize `Fields` per `CalendarId`, for example `[Option<Fields>; 7]`, so repeated `[~x]` tags cost nothing. Optionally also apply only the first `[~cal]` per run of identical tags.
3. Add a release timing test next to `long_number_formats_render_100ms_release` using a 4,096-character `[~jewish]`-repeating code.

## H3 (High): `TEXT()` format codes are uncapped, and the new `RR` rewrite is quadratic

**Where:**
- `parse_format.rs:569-579` (new in 7feec0b). In a section with digits and only `R`/`RR` date tokens, each `EraAndYear(2)` is replaced by two literals through `tokens.splice(i..=i, …)`. That shifts the whole tail every time, so the rewrite is O(tokens²).
- `parse_format.rs:280` `run_len` with `keyword()` (`:411-437`), older but on the same path. Run-capped tokens (`E`, `G`, `Q`, `NN…`, `AAA…`, and now `R`) rescan the whole run on every call but consume only 1-4 characters, which is also O(run²).
- `tally-functions/src/categories/text/text_fn.rs:39-41` parses the formula's format argument with no `MAX_NUMBER_FORMAT_LEN` check. Its only bound is `MAX_TEXT_BYTES` (4 MiB). Every other way in (`SetNumberFormat`, styles, readers, conditional formats) caps codes at 4,096 characters, where both effects are harmless (2.4 ms).

**Measured** (release, `parse_format`):

| Input | Size | Time |
|---|---|---|
| `"RR "`×n + `"0"` | 65 K chars | 0.34 s |
| `"RR "`×n + `"0"` | 131 K chars | 1.35 s |
| `"RR "`×n + `"0"` | 262 K chars | 5.56 s |
| `"EE "`×n + `"0"` (control) | 262 K chars | 2.5 ms |
| `"R"`×n + `" 0"` (both effects) | 64 K chars | 0.68 s |
| `"R"`×n + `" 0"` (both effects) | 262 K chars | 11.5 s |

Extrapolated to the 4 MiB TEXT limit: about 24 minutes for `RR ` alone and about 49 minutes for a solid `R` run. The older `E`/`G` runs take about 20 minutes (4.9 s at 262 K).

**Trigger:**
- `=TEXT(1;REPT("RR ";1398101)&"0")`. The formula is 33 characters; the REPT result is 4,194,303 bytes, under the cap.
- In a file this cell is recalculated on open. The 20 s open watchdog is only checked between formulas, nothing interrupts a single call, and the UI has no cancel (`security.md` §8 residual risk). So the document hangs on open for tens of minutes. Any edit that recalculates the cell does the same.

**Fix:**
1. In `text_fn.rs`, and anywhere else that parses a code from a formula value, return `Err:502` when `code.chars().count() > tally_model::MAX_NUMBER_FORMAT_LEN`. AOO and Excel limit format codes to about 255 characters, so this costs no compatibility.
2. Make the parser linear anyway:
   - Build a new `Vec<Token>` in one pass instead of calling `splice` in a loop.
   - Make `run_len` stop at 256 (`count_u8` saturates there anyway), or compute run lengths once per section.
3. Add a regression test: `TEXT` with a 4 MiB `RR ` / `E` code returns within 100 ms (Err:502).

---

## Notes (not findings)

- AOO's `calendar_jewish.cxx` has the same `(d + HebrewEpoch)/366` start, so the H2 fix changes no output. It was verified equal across the full supported range.
- `analyze`'s `RR` rewrite and `run_len` are trivial at 4,096 characters, so H3 only matters through uncapped entry points. Of those, the review found only TEXT. FIXED builds its code internally.
- The TS tokenizer is also O(n²) on long codes (`s.slice(i)` per character, `[...out].reverse()` per `.`). Every TS input is capped at 4,096 characters (chart `MAX_TEXT` and the format cap), which keeps this to milliseconds, so it is not reported.
