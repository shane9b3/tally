# Tally correctness review: engine, functions, formula, numfmt (main @ 01b6af6)

## Method

- **CLI.** I copied the built CLI to `$S/tcli` and ran `tcli eval --at Z99 [--cells ...] '<formula>'`. Here `$S` = `/private/tmp/claude-501/-Volumes-FILES-Projects-OSX-Office/5227e701-ee6d-4d46-9584-0099831db42f/scratchpad`. Note that `--at` defaults to A1, so a formula that reads A1 is circular unless you pass `--at`.
- **Scratch tests.** These ran against a `git archive HEAD` snapshot at `$S/snap`, so the concurrent edits in the working tree are not included and the repo was not touched:
  - `$S/snap/crates/tally-engine/tests/review.rs`: 20 scenario tests r01–r28 that drive `EngineService` (apply, undo, redo, copy, paste, cells, stats, navigate, `to_data`/`from_data`).
  - `$S/snap/crates/tally-numfmt/tests/review.rs`: `general_string` with column widths.
  - Run them with `CARGO_TARGET_DIR=$S/target cargo test -p tally-engine --test review -- --nocapture --test-threads=1`.
- **Volume.** About 400 individual checks. Every finding below has a reproduction. Where the AOO behaviour comes only from my memory of the AOO source and not from running AOO, the finding says "AOO-side unconfirmed".

---

## HIGH (silent wrong results or data loss)

### H1. Formula-bar text for times and durations drops whole days, so re-entering the text changes the value
`crates/tally-engine/src/service/view.rs:276-290` (`edit_text`): a format with no date part always edits as `HH:MM:SS`, which wraps hours modulo 24.

```
set A1 "36:00:00"  -> value 1.5, shown "36:00:00", edit text "12:00:00"
re-enter edit text -> Number(0.5)                                   (r20)
"25:00" -> 1.0417, edit "01:00:00";  "24:00" -> 1.0, edit "00:00:00"     (r23)
1.5 with format HH:MM -> shown "12:00", edit "12:00:00"                 (r10)
-0.25 with HH:MM:SS -> edit "-06:00:00" -> re-entered becomes Text("-06:00:00")  (r20)
"1:30:15.5" -> edit "01:30:16" (the 0.5 s is lost on re-entry)
```

- **Expected.** The edit text must re-enter to the same value. LibreOffice and AOO (`SvNumberFormatter::GetInputLineString`) switch to `[HH]:MM:SS` for time values that are below 0 or at least 1. The `HH:MM:SS.00` precision was not checked against AOO.
- **Fix.**
  - When `!has_date` and the value is `n < 0 || n >= 1`, use `[HH]:MM:SS`.
  - Add `.00` when there are fractional seconds.
  - Make sure `recognize_input` accepts `-HH:MM:SS` as a negative time. It currently returns text: `"-1:00"` gives Text.

### H2. Criteria do not recognise locale-formatted numbers or dates, so COUNTIF/SUMIF match nothing
`crates/tally-functions/src/criteria.rs:287`: the operand is turned into a number only through `parse_unambiguous`, which accepts plain and ISO input only.

```
cells A1=1/5/2026,A2=10/19/2026,A3=12/1/2026
=COUNTIF(A1:A3,">10/1/2026")  => 0        (expected 2)
=COUNTIF(A1:A3,"10/19/2026")  => 0        (expected 1)
=COUNTIF(A1:A3,">2026-10-01") => 2        (ISO works)
cells 500,1500,2500  =COUNTIF(A1:A3,">1,000") => 0   (expected 2)
cells 0.2,0.6        =COUNTIF(A1:A2,">50%")   => 0   (expected 1)
cells 1,10           =SUMIF(A1:A2,">$5")      => 0   (expected 10)
```

- **AOO.** `ScCountIf`, `ScSumIf` and the other criteria functions call `pFormatter->IsNumberFormat(queryString, nIndex, fVal)`, which uses the locale recognizer. Excel behaves the same way.
- **Impact.** `">"&TEXT(...)` and `">1/1/2026"` are very common patterns, and the wrong result is silent.
- **Fix.** Parse the operand with `tally_numfmt::recognize_input_with(rest, &EN_US, ctx)`. Use it if the result is numeric, then fall back to `parse_unambiguous`.

### H3. MOD returns wrong or negative remainders once the quotient reaches about 15 significant digits
`crates/tally-functions/src/categories/mathematical/basic.rs:72` computes `x - y*approx_floor(x/y)`. `approx_floor` first rounds the quotient to 15 digits, which can round it up.

```
=MOD(999999999999999,2)    => -1   (expected 1)
=MOD(1234567890123456,10)  => -4   (expected 6)
=MOD(1E16,7)               => 0    (expected 4)
=MOD(1E20,3)               => 0    (expected 1)
```

- **AOO** (`interpr1.cxx` `ScMod`): when the divisor is integral, AOO uses `fmod(x, y)` and adds `y` if the signs differ. Only non-integral divisors take the approxFloor/approxSub path. AOO-side unconfirmed by running it, but the code path is well known.
- **Fix.** Port that branch. With it, `MOD(1;0.1)=0` still holds and the large-integer cases become exact.

### H4. Saving silently drops formatting on long style runs
`crates/tally-engine/src/convert.rs:141` (`to_data`): any style run of `LONG_STYLE_RUN` (10,000) rows or more is clipped to `used.r1`. A run that starts below the used area keeps only its first row.

```
italic C100:C20100, data only in A1 -> to_data exports 1 styled cell in column C
after from_data, C5001 has StyleId(0)        (r12)
```

- **Impact.** Formatting is lost on every save. This also covers a whole formatted column: rows past the data lose the format after reload, where AOO keeps the column default style.
- **Fix.** Carry style runs in `tally-model` (for example `(col, r0, r1, style)`, or a column default style) and let the ODS writer emit them as repeated rows or columns, instead of expanding them into cells.

---

## MEDIUM (visible wrong errors, or AOO deviations that will matter)

### M1. Static cycle detection reports Err:522 for formulas that AOO evaluates fine
`crates/tally-engine/src/recalc.rs:176-255`: every SCC with a static self-edge, or with two or more members, gets Err:522 without being evaluated.

```
--at A5  =ROW(A5)-1          => Err:522   (AOO: 4)
--at B2  =COLUMN(B2)         => Err:522   (AOO: 2)
--at A1  =IF(TRUE(),5,A1)    => Err:522   (AOO: 5)
--at A1  =IF(FALSE(),A1,5)   => Err:522   (AOO: 5)
--at A1  =IFERROR(1,A1)      => Err:522   (AOO: 1)
ISBLANK(H1) in H1 => Err:522 (AOO checks the cell type without interpreting it, so FALSE; less certain)
```

- **AOO.** Circularity is detected at interpretation time (`ScFormulaCell::Interpret`, through the bRunning/recursion helper). A reference that is never actually read, such as an untaken IF branch or a reference-only argument like ROW or COLUMN, is not circular. `=ROW(A2)-1` typed in A2 is a common row-numbering idiom.
- **Fix.** Use Tarjan only for ordering. Evaluate the members of a cyclic SCC with on-demand evaluation, and give Err:522 only to cells that actually re-enter `CellState::Evaluating`. The machinery already exists in `formula_value`. Alternatively, leave reference-only parameters out of the precedent edges.

### M2. Elementwise ops on mismatched matrix sizes follow LibreOffice/Excel (max size, #N/A fill), not AOO 4.1
`crates/tally-engine/src/eval.rs:403` (`pick`, whose doc comment says "OOo matrix element pick") and line 461 (`ra.max(rb)`).

```
cells A1:A3=1,2,3 B1:B2=10,20   =SUMPRODUCT(A1:A3*B1:B2) => #N/A
=SUMPRODUCT({1;2;3}*{1;2}) => #N/A
```

- **AOO 4.1** (`interpr5.cxx` `lcl_GetMinExtent`, used by `lcl_MatrixCalculation`): the result takes the smaller extent unless one operand is a vector being replicated. That gives 50 here. AOO-side unconfirmed by running AOO.
- **Action.** Decide on the target and document it; the current comment attributes the behaviour to OOo.

### M3. Formulas typed into Text-formatted (`@`) cells are evaluated
`crates/tally-engine/src/workbook.rs:468-480`: the `=` check runs before the text-format check, which contradicts the doc comment on `set_input` ("cells formatted `@` keep the input as text").

```
format A1 "@", input "=1+1" -> Number(2.0)      (r28; "007" and dates do stay text)
```

- **AOO.** A Text-formatted cell stores `=1+1` as a string.
- **Fix.** Move the `is_text_format()` check above the formula branch.

### M4. Text ordering uses lower-cased code points, not collation
`crates/tally-engine/src/eval.rs:640`. Criteria ordering (`criteria.rs`, `ordering_holds`) has the same problem.

```
="é"<"f"  => FALSE   (AOO, through ScGlobal::GetCollator(): TRUE)
```

- **Impact.** Comparisons and `<`/`>` criteria on accented text, and any sort or lookup built on `compare`.
- **Fix.** Use a collation key, or at least Unicode NFD plus folding, and document the remaining gap.

### M5. The default `case_sensitive=false` is documented as AOO's default, but AOO defaults to case-sensitive
`crates/tally-types/src/settings.rs:94`, plus the comment in `criteria.rs`.

```
="A"="a" => TRUE
```

- **AOO.** `ScDocOptions::ResetDocOptions` sets `bIsIgnoreCase = sal_False`, and the ODF default for `table:case-sensitive` is true. The ODS reader already treats a missing attribute as true. AOO-side unconfirmed by running AOO.
- **Action.** Either change the default for new documents or correct the docs that claim parity.

### M6. A row inserted directly below a formatted block does not inherit its format
`crates/tally-engine/src/storage.rs:284-296` (`StyleRuns::insert_rows`): the `e < at` branch leaves a run that ends at `at-1` unchanged.

```
bold A1:C1, insert 1 row at row 2 -> A2 StyleId(0)        (r24)
```

- **AOO.** `ScAttrArray::InsertRow` copies the attributes of the row above, so the new row would be bold. Inserting inside a run already behaves correctly.
- **Fix.** Extend a run whose `e == at-1`, and do the same in `DimRuns::insert` if row heights should follow.

### M7. `selection_stats` ignores error cells and booleans
`crates/tally-engine/src/service/view.rs:392`.

```
A1=1,A2=x,A3==1/0,A4=TRUE,A5=2 -> sum 3, count 5, count_numbers 2      (r09)
```

- **AOO.** `lcl_UpdateSubTotal` sets `bError` for an error formula cell (every function except Count), so the status bar shows an error. TRUE is the number 1, so it is included. AOO-side unconfirmed by running AOO.

### M8. Smaller deviations, unconfirmed against a running AOO
- **MATCH on a 2-D range** gives `#N/A`. AOO `ScMatch` calls `PushIllegalParameter()`, which is Err:504. The conformance file expects `#N/A` with no AOO citation.
- **Whole-column references are written as `[.A:.A]`** (`of:=SUM([.A:.A])`). AOO 4.1 has no entire-column references, so such files may not load correctly there. This is an interop risk.

---

## LOW (cosmetic, docs, or uncertain)

- **PROPER("3rd") gives "3Rd"**, and the conformance file expects "2Nd Place". AOO and LibreOffice `ScProper` look at `isLetterNumeric(prev)`, which gives "3rd" and "2nd Place". Needs an oracle check.
- **SUBSTITUTE("aaaa","aa","b",2) gives "aab".** AOO's loop advances `nPos++` after a non-replaced match, so it counts overlapping occurrences and gives "aba". Needs an oracle check.
- **VALUE and DATEVALUE error codes.**
  - `VALUE("TRUE")` gives `#VALUE!`. AOO's `IsNumberFormat` recognises booleans and returns 1.
  - `VALUE("")` and `DATEVALUE("14:30")` give `#VALUE!`. AOO calls `PushIllegalArgument`, which is Err:502.
  - AOO-side unconfirmed.
- **AND("TRUE") gives TRUE.** In AOO `ScAnd`, a string argument gives errNoValue (`#VALUE!`). AOO-side unconfirmed.
- **TRUNC(2.9) gives Err:511.** This is documented in `TRUNC.toml`. ODFF 1.2 and Excel make Count optional; I am not sure about AOO's `MustHaveParamCount`, so this needs an oracle check.
- **LEN("😀") is 1.** AOO counts UTF-16 units and gives 2, and MID and LEFT offsets differ the same way on astral characters.
- **Unknown sheet references never re-resolve.**
  - `=Nosuch.A1` stays `#REF!` after a sheet "Nosuch" is added (r05), but after save and reload the same file resolves.
  - `[$Gone.A1]` is re-saved as `[Gone.A1]`, so the `$` is lost (r11).
- **Invalid formulas are saved as `of:=` followed by UI-dialect text**, for example `of:=SUM(A1:B2 C1)`. An invalid formula loaded from ODS shows OpenFormula syntax in the formula bar (r26). It round-trips within Tally.
- **Cosmetic formula and edit text.**
  - The printer keeps reversed or degenerate ranges as written (`=B2:A1`, `=H1:H1` after a delete); AOO normalises them.
  - The edit text for AM/PM time formats is 24-hour (`12:00 am` edits as `00:00:00`).
- **The inline-array column separator is `;` while the en-US argument separator is `,`**, so `={1,2}` and `=SUM({1,2;3,4})` give Err:504. This matches the dialect docs but will surprise users.
- **Sheet references are stored as absolute sheet ids.** Pasting `=Two.A1` onto another sheet keeps `Two` (r07). In AOO, a sheet reference without `$` is relative to the tab and may shift. Unconfirmed.

---

## Verified correct (one line each)

**Dependency tracking and recalc**
- SUM range dependents recalc on edit, undo and redo, including after inserting rows inside the range and editing the new cell (r01).
- Deleting rows: a reference into deleted rows becomes `=#REF!`, `SUM(H2:H3)` with both rows deleted becomes `SUM(#REF!)`, and a partial delete shrinks the range. Undo restores the formulas and values, and later edits propagate (r02, r14).
- A formula moved by an insert keeps the correct relative reference and recalcs (`B12=A10+A1` becomes `B14=A12+A1`) (r21).
- Cross-sheet references are rewritten on insert, and the dependency index is rebuilt (`Two.A1=SUM(Sheet1.A1:A5)`); later edits propagate (r21).
- Deleting a column referenced from another sheet gives `#REF!`, and undo restores it; edits propagate (r24).
- Delete sheet: dependent formulas become `#REF!`; undo restores the formulas intact, and an edit on the restored sheet propagates (r06).
- Rename sheet updates the printed formulas (r21).
- A 6-op transaction (inputs, insert, delete, clear) undoes back to an identical `to_data()` (r21).
- Real cycles (3-cycle through a range, self-reference) give Err:522, and the cells recover once the cycle is broken (r03).
- A volatile diamond `=B1-C1` built over `RAND()` gives 0, and `NOW()-NOW()` gives 0, because `now` is fixed per recalc (r04).
- `HashDepIndex::remove` handles the swap_remove back-index correctly (checked by reading the code).

**Operators and parser**
- `-2^2` is 4, `2^3^2` is 64, `-(2^2)` is -4, `=(-2)^2` is 4, `=2^-2` is 0.25, `=10%^2` is 0.01, and `=1%%` is 0.0001.
- `0.1+0.2&""` is "0.3", `1/3&""` is 15 digits, and `1E20&""` is "1E+20".
- `""=0` is FALSE. An empty cell equals both `0` and `""`. Text sorts after numbers, `"1"=1` is FALSE, and `TRUE=1` is TRUE.
- `0.1+0.7=0.8` and `1-0.9-0.1=0` both behave as expected because of approx_add and approx_equal.
- Implicit intersection works: `=A1:A3` at C2 gives 2, at C9 gives `#VALUE!`, and `SUM(A1:A3>1)` with no array context intersects.
- `SUMPRODUCT((A1:A3>1)*B1:B3)` with text gives 5, and a mismatched SUMPRODUCT gives `#VALUE!`.
- `A1:B2!B1:C3` is the intersection; a disjoint `!` gives `#NULL!`.
- About 90 parser edge cases give sensible errors or canonical text: `=A1=` is Err:511, `=SUM(A1:B2 C1)` is Err:509, `=1E+400` is `#NUM!`, `=--1` is 1, `='Sheet 1'!A1` prints as `'Sheet 1'.A1`, `;` and `,` are both accepted, the printer keeps the parentheses as written and doubles `""` in strings, and `XFE1` and `A1048577` give `#NAME?` (r22, r27).
- `to_data`/`from_data` round-trip is exact for 24 formula kinds: `of:=` prefix, `[.A1]`, `['My Sheet'.A1]`, `[.$A$1]`, `[.1:.2]`, inline arrays, `oooc:=`, and sheet names resolved on load (r11, r26).

**Copy, paste and input**
- Internal paste across sheets re-anchors relative references and keeps `$A$1`. References pushed off the grid become `#REF!`, and tiling works (r07).
- TSV paste recognises `1,234`, `10/19/2026`, `5%`, `$5`, `(5)`, `1.5%`, `'x`, `=A1*2`, `TRUE`, ` 42 `, `1e5` and `12:00 am` (r08).
- `recognize_input` handled about 60 ambiguous inputs plausibly: `"1/2"` and `"3/4"` become dates, `"1,23"` stays text, `"2/30/2026"` stays text, `"1 1/2"` is 1.5, `"--5"` stays text (r23).

**View queries**
- `navigate_edge` matches Ctrl+arrow at run ends, gaps, empty-string formula cells and the sheet edge (r09).
- `cells().num` is set only for General numbers (r24).
- `selection_stats.formatted_sum` uses the active cell's format (r24).

**Functions**
- ROUND family: `ROUND(2.5)`=3, `ROUND(-2.5)`=-3, `ROUND(1.005,2)`=1.01, `ROUND(0.285,2)`=0.29, `ROUND(-15,-1)`=-20, `ROUNDUP(1.1-1,1)`=0.1, `ROUND(x,-21)` is Err:502.
- `INT(-1.5)`=-2, `MOD(-1,3)`=2, `MOD(1,-3)`=-2, `MOD(x,0)` is `#DIV/0!`.
- SUMIF resizes the sum range, COUNTIF handles `"<>"`/`"="`/`"<b"`, `AVERAGEIF` with no match is `#DIV/0!`, and `TRUE` counts as 1 in ranges.
- MATCH type 1/-1 with duplicates returns the last match, and MATCH type 0 is case-insensitive.
- INDEX edge cases: `0,0` and a single row work, out-of-range is `#REF!`, and fractional indexes truncate. HLOOKUP works, and VLOOKUP index > columns gives Err:502.
- DATE normalisation: `DATE(2026,13,1)`, `DATE(2026,2,30)`, `DATE(26,1,1)` and `DATE(2026,0,0)` give the right serials. `DATE(-1,…)` is Err:502, `WEEKDAY` types 1–3 work and 11+ give Err:502, and serials 0 and -1 give the right weekdays.
- VALUE on `"1,234"`, `"$5"`, `"50%"`, `"10/19/2026"` and `"(5)"`; DATEVALUE on ISO and en-US dates.
- Text functions: LEFT and MID with count 0, FIND and SEARCH with start greater than the length give `#VALUE!`, `FIND("",s,len)`, SUBSTITUTE with an occurrence past the end, `PROPER("o'neil")`, TRIM collapsing internal spaces, and `UPPER("straße")`.
- AND and OR over text-only references give `#VALUE!`. IFERROR and IFNA propagate correctly, IS* functions work on references and errors, `NA(1)` is Err:504, and RANDBETWEEN is inclusive and gives Err:502 when the bounds round inward and cross.
- Unverified as AOO-exact: VLOOKUP and MATCH in sorted mode on unsorted data return `#N/A` (VLOOKUP(3,…), MATCH(3,…)). This is consistent with a binary search; document it as "undefined on unsorted data".

**Number formats**
- About 75 TEXT() format/value pairs match LibreOffice/AOO output. These include sections and conditions, `#,##0,` and `,,` scaling, `0.0E+0`, `[h]:mm` (also negative), `[mm]:ss`, `[ss]`, AM/PM and a/p, fractions, literals, `@`, NNN/NNNN, Q, WW, MMMMM, `[$€-407]`, and `0.##` printing `3.`.
- Dates before 1900 and the 1899-12-30 epoch: serial 60 is 1900-02-28, 61 is 03-01, and -1 is 1899-12-29.
- General width fitting: `0.0001` stays fixed while `1E-05` and `1E+15` switch to scientific, `1/3` at width 5 is `0.333`, and a value that does not fit becomes `###`.
