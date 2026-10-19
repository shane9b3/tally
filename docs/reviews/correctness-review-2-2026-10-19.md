# Tally correctness review 2: Wave 2 (201e669 → f6193fa)

## Method

- **Paths.** `$S` = `/private/tmp/claude-501/-Volumes-FILES-Projects-OSX-Office/5227e701-ee6d-4d46-9584-0099831db42f/scratchpad`.
- **Worktree.** I worked in a detached worktree at `$S/corr2-wt` (`git worktree add $S/corr2-wt f6193fa`) and built with `CARGO_TARGET_DIR=$S/corr2-target`. The repo itself was not modified. Remove the worktree with `git worktree remove $S/corr2-wt`.
- **CLI.** `$S/c2/ev.sh <file>` runs `tally-cli eval --at Z999 [--cells …] '<formula>'` once per line. The batches are in `$S/c2/t1.txt` … `t12.txt`, about 420 formulas.
- **Scratch tests.** These exist only in the worktree:
  - `crates/tally-engine/tests/zz_review.rs`, with these tests: names, names2, arrays, arrays2, moves, merges, sorting, filters, subtotal_hidden, fills, findrep, paste_and_borders, cell_format, xlsx_rt and xlsx_rt2. Run them with `cargo test -p tally-engine --features io --test zz_review -- --nocapture`.
  - `crates/tally-io-xlsx/src/formula.rs` and `src/numfmt.rs`, each with a `mod zz_review` (ExcelA1 round trips, number-format translation).
  - `crates/tally-functions/tests/zz_review.rs` (OOXML name clashes).
  - `$S/c2/ts/*.ts`, which runs the UI `planSeries` and `analyzeGlance` with `node` type stripping.
- **AOO oracle.** The local AOO source in `$S/openoffice-src/main` (`sc/source/core/tool/interpr*.cxx`, `sc/source/core/data/{table3,table4,column,dociter}.cxx`, `scaddins/source/{analysis,datefunc}`, `officecfg`). Every AOO claim below cites the line I read. I did not run AOO.
- **Volume.** About 520 checks: about 420 formula evaluations and about 100 engine, IO and UI observations. Every finding below was reproduced.

---

## HIGH (silently wrong results or data loss)

### H1. SUBTOTAL never excludes filtered or hidden rows, even though AutoFilter now ships
- **Where.** `crates/tally-engine/src/recalc.rs` and `eval.rs` never implement `EvalContext::hidden_row`, so the default `false` in `tally-functions/src/context.rs:165` applies. `subtotal.rs:36` is therefore a no-op.
- **Repro** (`zz_review::filters`, `subtotal_hidden`).
  - Data: A2:A5 = 1, 2, 3, 2. AutoFilter on A1:A5 with values = {"2"}, which hides rows 2 and 4.
  - `=SUBTOTAL(9;A2:A5)` and `=SUBTOTAL(109;A2:A5)` both give **8**. The expected value is 4.
  - The result is still 8 after `recalc_all` and for a freshly typed formula.
  - With row 2 hidden manually, `SUBTOTAL(109;A1:A4)` gives **10**. The expected value is 8.
- **AOO.** `dociter.cxx:345` and `:1077`: `if ( !bSubTotal || !pDoc->pTab[nTab]->RowFiltered(nRow) )`. So SUBTOTAL 1–11 skip filtered rows.
- **Docs.** `docs/phase-a.md` lists this as a stub ("the engine only has to implement it"). v0.2 shipped AutoFilter without it, and filter-plus-SUBTOTAL is the canonical use case.
- **Fix.**
  - Implement `hidden_row` from `Sheet::row_hidden`, with separate filtered and user-hidden queries: 1–11 skip filtered rows, and 101–111 also skip hidden rows.
  - Make every SUBTOTAL cell dirty when row visibility changes (filter, hide/unhide, undo).

### H2. XLSX: CONVERT, WEEKNUM and DURATION resolve to the wrong function, on import and on a Tally→XLSX→Tally round trip
- **Where.** `crates/tally-io-xlsx/src/formula.rs:61` (`XlsxResolver::function`) calls `registry().lookup_any(name)`. `lookup_any` (`tally-functions/src/registry.rs:591`) tries canonical UI names before OOXML names.
- **Clashes.** `zz_review::ooxml_clashes` lists three:
  - Excel `WEEKNUM` resolves to the built-in WEEKNUM instead of WEEKNUM_ADD.
  - Excel `CONVERT` resolves to the built-in currency CONVERT instead of CONVERT_ADD.
  - Excel `DURATION` resolves to the built-in (PDURATION) instead of DURATION_ADD.
- **Repro** (`zz_review::xlsx_rt`). Write a workbook to XLSX, read it back and recalculate:

  | Before | After |
  |---|---|
  | `=CONVERT_ADD(1;"m";"ft")` = 3.2808 | `of:=ORG.OPENOFFICE.CONVERT(1;"m";"ft")` = **#N/A** |
  | `=DURATION_ADD(...)` = 5.9938 | `of:=PDURATION(...)` = **Err:504** |
  | `=WEEKNUM_ADD(A1;1)` | `of:=ISOWEEKNUM([.A1];1)`, i.e. AOO's built-in, different semantics |

- **Excel-authored files.** `CONVERT(1,"m","ft")` and `WEEKNUM(A1)` break the same way. `WEEKNUM(A1)` with one argument becomes Err:511.
- **Export.** The built-in `=WEEKNUM(A1;2)` is written as `_xlfn.ISOWEEKNUM(A1,2)`, but Excel's ISOWEEKNUM takes one argument.
- **Fix.**
  - In the XLSX resolver (ExcelA1 dialect), try `lookup_ooxml` first and fall back to `lookup_any`.
  - Add the three clashes to the round-trip tests.
  - Export the built-in WEEKNUM as something Excel can evaluate. Excel has no exact equivalent, so add a warning.

### H3. FREQUENCY with unsorted bins returns the counts in sorted-bin order instead of the caller's bin order
- **Where.** `crates/tally-functions/src/categories/array/frequency.rs:29-35`.
- **Repro.** `=INDEX(FREQUENCY({1;2;2;3;5};{4;2});1;1)` gives 3 and `;2;1` gives 1. AOO gives 1 and 3.
- **AOO.** `interpr5.cxx:1832`: `pResMat->PutDouble(nCount, aBinIndexOrder[j])` writes each count back at the bin's original index. Excel does the same.
- **Why tests miss it.** The conformance case "unsorted bins are sorted" (`tests/conformance/array/FREQUENCY.toml:8`) and the unit test use symmetric data (1..5 or 1..6 with bins 4 and 2), whose result is identical either way.
- **Fix.** Sort `(bin, original_index)` pairs and scatter the counts back. Add an asymmetric vector.

### H4. Paste Special with Transpose does not transpose references
- **Where.** `crates/tally-engine/src/clipboard.rs:357-421`. The cell position is transposed (`place`), but the formula is re-anchored by the plain offset.
- **Repro** (`zz_review::paste_and_borders`).
  - Setup: A1 = 2 and B1 = `=A1*5`. Copy A1:B2 and paste with transpose at G1.
  - Result: G2 = `=F2*5`, which evaluates to 0. AOO gives G2 = `=G1*5` = 10.
- **AOO.** `ScDocument::TransposeClip` calls `ScFormulaCell::TransposeReference`, so relative references are mirrored along with the cell.
- **Fix.** Transpose each relative offset (dr, dc) → (dc, dr) before re-anchoring. Absolute parts stay as they are.

### H5. The reference union operator (`~`, Excel `(A1,B1)`) is unsupported, and COUNT over a union silently returns 0
- **Where.** `crates/tally-engine/src/eval.rs:452`: `BinOp::Union => return EvalValue::error(ErrorKind::ParamList)`.
- **Repro.** With A1:A3 = 1, 2, 3:
  - `=SUM(A1:A2~A3)`, `=AREAS(A1:A2~A3)`, `=INDEX(A1:A2~A3;1;1;2)` and `=SUM((A1;A3))` all give Err:504. AOO gives 6, 2, 3 and 4.
  - `=COUNT(A1~A3)` gives **0**, silently.
- **Scope.** The parser, the printer and XLSX already translate unions (`SUM((A1,B1))` → `of:=SUM(([.A1]~[.B1]))`, `zz_review::roundtrips`), so imported Excel formulas evaluate wrongly. The features inventory (`docs/calc-features-inventory.md:685`) lists `~` as a Calc feature.
- **Fix.** Add a reference-list value: `EvalValue::Ranges`, iterated by the range-taking functions, plus AREAS and INDEX area selection. Until then, COUNT and COUNTA must propagate the error instead of returning 0.

### H6. WEEKNUM uses Excel's week system instead of AOO's ISO-style rule (minimum four days)
- **Where.** `crates/tally-functions/src/categories/datetime/days.rs:136-151`.
- **Repro.**
  - `=WEEKNUM(DATE(1995;1;1);2)` gives 1. The AOO help example gives 52.
  - `=WEEKNUM(DATE(2021;1;1);2)` gives 1. AOO gives 53.
  - `=WEEKNUM(DATE(2026;1;1);1)` gives 1. AOO gives 53, because the Sunday-start week containing 1 January has only three days in 2026.
  - `=WEEKNUM(d;3)` gives Err:502. AOO treats every mode other than 1 as Monday.
- **AOO.** `interpr2.cxx:225-236`: `aDate.GetWeekOfYear( nFlag == 1 ? SUNDAY : MONDAY )`, which uses the default `nMinimumNumberOfDaysInWeek = 4` from `tools/date.hxx`. This is also why the ODFF name is ISOWEEKNUM (`core_resource.src:332`).
- **Docs and tests.** The doc comment and `tests/conformance/datetime/WEEKNUM.toml` encode Excel semantics, which is what WEEKNUM_ADD implements.
- **Fix.** Port GetWeekOfYear: week 1 is the first week that has at least four days in the new year, starting on Sunday or Monday. Map mode ≠ 1 to Monday. Fix the vectors.

### H7. D-functions: a criteria header that matches no field is ignored, so every record matches
- **Where.** `crates/tally-functions/src/categories/database/common.rs:126-134` (`continue` when the header is unknown). The module doc at `common.rs:20-21` claims "as in AOO's query builder".
- **Repro.** Database A1:C5 has the fields Name, Age and Val. Criteria E1:E2 = `Bogus` / `1`. Then `=DSUM(A1:C5;"Val";E1:E2)` gives **42**, the sum of every record.
- **AOO.**
  - `table3.cxx:1652-1672` (`CreateExcelQuery`): a criteria header not found among the database headers sets `bValid = sal_False`.
  - `interpr1.cxx:6656` and `:6697` (`DBIterator`): an invalid query param means `SetError(errIllegalParameter)`, which shows as **Err:504**.
- **Fix.** Return `Err:504` when any criteria header is unmatched, and correct the doc comment.

---

## MEDIUM (wrong errors, or notable AOO deviations)

### M1. D-functions: a blank criteria row matches every record, but AOO ignores it
- **Repro.** Criteria E1:E3 = `Age` / `30` / (blank). `=DSUM(A1:C5;"Val";E1:E3)` gives 42. AOO gives 15.
- **AOO.** `table3.cxx:1690-1715`: only non-empty cells create entries, and an empty row adds nothing to the OR list. (The current behaviour is Excel's.)
- **Where and fix.** `common.rs:122-140`. Drop alternatives that have no conditions, unless every row is empty.

### M2. Sort: the reference-adjustment rule is a hybrid that is neither AOO's nor "references follow"
- **Where.** `crates/tally-engine/src/sort.rs:8-14` and `:215`. A reference "entirely inside the sorted rows" keeps its relative offset; every other reference keeps its absolute target.
- **AOO 4.1.** `column.cxx:919-983` (`ScColumn::SwapRow`): cells are cloned at the new position (`CloneWithoutNote`), with the comment "hier kein UpdateReference … mitsortiert werden nur noch relative Referenzen". Every relative reference keeps its offset, whether it points inside or outside the range. Absolute references stay. References from outside the range are not updated.
- **Repro** (`zz_review::sorting`). Header in A1, E1 = 100. Rows 2–4 hold A = 3, 1, 2, B = `=A{r}*10+E1` (literal E1), C = `=A{r-1}`. Sort A2:C4 ascending.
  - Tally: row 4 (the old row 2) has C4 = `=A1` (kept on the header), and B keeps `+E1`.
  - AOO: C4 = `=A3`. The B cells that moved up from rows 3 and 4 become `=A2*10+E0` (#REF!) and `=A3*10+E2`.
- **Assessment.** Tally's result is friendlier, but it is not AOO's, and the doc says "(as in AOO)".
- **Fix.** Either port AOO's rule (relative offsets for every relative reference) or document the deviation.

### M3. INDIRECT with a1 = FALSE always gives #REF!
- **Where.** `crates/tally-engine/src/recalc.rs:930-933` (`if !a1 { return None; }`).
- **Repro.** `=INDIRECT("R2C1";0)` and `=SUM(INDIRECT("R1C1:R3C1";0))` both give #REF!. AOO gives 2 and 6.
- **AOO.** `ScIndirect` switches to `CONV_XL_R1C1` when the second argument is 0.

### M4. ADDRESS output deviates from AOO
- **Where.** `crates/tally-functions/src/categories/spreadsheet/reference.rs:37-91`.
- **Sheet prefix.**
  - `=ADDRESS(1;1;1;1;"Sheet1")` gives `$Sheet1.$A$1`. AOO gives `Sheet1.$A$1`.
  - `=ADDRESS(1;1;1;1;"My Sheet")` gives `$'My Sheet'.$A$1`. AOO gives `'My Sheet'.$A$1`.
  - AOO (`interpr1.cxx:7108-7131`) uses only CheckTabQuotes plus `.`, with no `$`.
- **R1C1 separator.** `=ADDRESS(1;1;1;0;"Sheet1")` gives `Sheet1.R1C1`. AOO gives `Sheet1!R1C1` (`:7129`, '!' for CONV_XL_R1C1).
- **Modes 5–8.** `=ADDRESS(2;3;5)` gives Err:502. AOO accepts 5–8 as aliases of 1–4 (`:7071-7078`).
- **Invalid abs.** Out-of-range abs values give Err:502. AOO uses `PushNoValue`, which is #VALUE! (`:7068`).

### M5. SUBTOTAL rejects more than one reference
- **Where.** `crates/tally-functions/src/categories/mathematical/subtotal.rs:23-26`.
- **Repro.** `=SUBTOTAL(6;A1:A3;A1)` gives Err:504.
- **AOO.** `interpr1.cxx:6477`: `MustHaveParamCountMin(nParamCount, 2)`, so any number of references is accepted.

### M6. AutoFilter is re-applied when a cell is edited (AOO keeps the visibility until refresh)
- **Where.** `crates/tally-engine/src/filter.rs:229` (`reapply_filters_touching`).
- **Repro** (`zz_review::filters`). With the filter `= 2` active, typing 5 into the visible cell A3 immediately hides row 3. The hidden rows go from [1, 3] to [1, 2, 3].
- **AOO.** AOO does not re-filter on input; the edited row stays visible until Data ▸ Filter ▸ Refresh.
- **Assessment.** This is a UX and parity issue, not a data issue. Either document it or limit it to explicit refresh, undo and redo.

### M7. Merge silently deletes the contents of covered cells
- **Where.** `crates/tally-engine/src/ops.rs:604-625` ("Only the top-left cell keeps its content").
- **Repro.** B2 = x, C3 = y. MergeCells B2:C3 empties C3 with no warning. Undo does restore it.
- **AOO.** AOO asks "move the contents of the hidden cells into the first cell?". Answering No keeps the hidden contents, so unmerging brings them back.
- **Fix.** Keep covered contents (AOO's "No" answer) or return a warning the UI can confirm.

### M8. Two AutoFill implementations disagree, and the UI uses the client-side one
- **Where.** `packages/app/src/controller.ts:528` calls `fill.ts::fillRange`, the client emulation. It does not use the engine's `Op::FillRange`, which now exists (`bridge/src/generated/Op.ts:378`). The file header says to swap it once the op lands.
- **Differences** (engine from `zz_review::fills`, UI from `$S/c2/ts/run.ts`):

  | Source | Engine | UI | AOO (`table4.cxx`) |
  |---|---|---|---|
  | 1, 2, 4 | 2, 3, 5, 3 | copy 1, 2, 4 | 2, 3, 5, 3 (FILL_SIMPLE, +1 per repetition, :739-882) |
  | 1st | 2st, 3st | 2nd, 3rd | 2nd, 3rd (`bIsOrdinalSuffix`, :783) |
  | Sat, Mon | Wed, Fri | Wed, Fri | Tue, Wed (the list index steps by 1 from the last entry, :716-728) |
  | Item -1 | Item -2 | Item -2 | Item 0 (`lcl_DecompValueString` reads -1, :125-137) |
  | mon | tue | tue | Tue (the list's own spelling) |
  | filtered data | series | series | plain copy (`IsDataFiltered`, :198, :740) |

- **Fix.** Switch the UI to `Op::FillRange`, fix the ordinal case in `crates/tally-engine/src/fill.rs`, and then decide on the list stride.

### M9. "At a glance" sums a numeric header row and loses the category column
- **Where.** `packages/app/src/glance.ts:105` (the header is detected only when every header value is text).
- **Repro** (`$S/c2/ts/g.ts`). Rows `Region | 2023 | 2024`, `North | 10 | 12`, `South | 20 | 22`, `North | 5 | 6` give:
  - `hasHeader: false`;
  - sums of **2058** and **2064** (the expected values are 35 and 40);
  - `category: null`, because "Region" counts as a value.
- **Fix.** Treat row 1 as a header when it contains any text and the body columns have a type different from row 1, or when the row-1 numbers are integers between 1900 and 2100 above non-integer or mixed data.

### M10. Text comparison and sort still use code points (review 1's M4 is still open, and v0.2 sort now inherits it)
- **Formulas.** `="ä"<"z"` is FALSE, and `=MATCH("ä";{"a";"b";"z"};1)` gives 3. AOO, which uses `ScGlobal::GetCollator()`, gives TRUE and 1.
- **Sort.** SortRange on `a10, a9, ä, z, A` gives `A a10 a9 z ä`. AOO's collator gives `A ä a10 a9 z`.

### M11. CELL info deviations
These are in `crates/tally-functions/src/categories/information/cell.rs` and `crates/tally-engine/src/recalc.rs:965-979`.
- **"protect"** is always 0, because the engine never sets `CellFormatInfo::protected`. AOO's default cell attribute is locked, so it returns 1 (`interpr1.cxx:2281-2286`).
- **"coord"** gives `$A$1`. AOO gives the Lotus form `$A:$A$1` (`:2214-2224`).
- **"format"** never appends `()`. AOO appends it when the code contains `(` anywhere (`:2338-2341`, `lcl_FormatHasOpenPar` at `:2146`).
  - Example: `#,##0.00;(#,##0.00)` gives `,2`. AOO gives `,2()`.
  - "parentheses" checks only the first section, so it gives 0 where AOO gives 1.
- **Date codes.** `MM/DD/YY` gives D4, but AOO maps `NF_DATE_SYSTEM_SHORT` to D1 (`:2307-2315`); D4 is the Excel code. `YYYY-MM-DD HH:MM` gives D9 where AOO gives G.

---

## LOW (cosmetic, edge, docs)

- **IFS dimension mismatch.**
  - Repro: `=COUNTIFS(A1:A3;">2";B1:B2;">1")` and the other IFS functions give #VALUE!.
  - AOO: `interpr1.cxx` IterateParametersIfs (+145 and +294) sets `errIllegalArgument`, which is Err:502.
- **EASTERSUNDAY(24) gives Err:502.**
  - AOO (`interpr2.cxx:246-247`) expands two-digit years, giving 2024 (45382).
- **BAHTTEXT.**
  - Repro: `BAHTTEXT(101)` gives หนึ่งร้อยเอ็ดบาทถ้วน and `BAHTTEXT(10000001)` gives สิบล้านเอ็ดบาทถ้วน.
  - AOO: `lclAppendBlock` (`interpr2.cxx:2859`) uses เอ็ด only when the tens digit is non-zero, so it gives หนึ่งร้อยหนึ่ง… and สิบล้านหนึ่ง….
  - Tally matches Excel and common Thai usage, so document the choice.
  - The สตางค์ placement is correct for 0.25 and 1.25, and 0, 1, 11, 21, 100 and 1000000 are correct.
- **Text-to-number coercion is more lenient than AOO.**
  - Repro: `="1.5"+1` gives 2.5.
  - AOO: `ConvertStringToValue` (`interpr4.cxx:238-256`) parses with decimal separator 0, so only integers, exponents and ISO date/time convert, and the result is #VALUE!.
  - `"1,000"`, `"$5"` and `"TRUE"` correctly give #VALUE!.
- **XLSX import of Excel serial 0.**
  - A date-formatted Excel serial 0 imports as Tally 0 (1899-12-30), but the module's own table maps Excel 0 to Tally 1 (1899-12-31).
  - Export writes Tally 1 as Excel 0, so 1899-12-31 loses a day on a round trip (`zz_review::xlsx_rt`: C1 goes from 1 to 0).
  - Where: `crates/tally-io-xlsx/src/dates.rs`. Fix: use `(0.0..60.0)` on import.
- **ExcelA1 `Sheet1!#REF!`** prints back as `#REF!`, dropping the sheet (cosmetic).
- **Typing `ISOWEEKNUM(d)` gives Err:511.**
  - The UI accepts the ODF alias ISOWEEKNUM for the 2-argument built-in, so a user writing Excel's 1-argument ISOWEEKNUM gets a missing-parameter error.
  - Consider a 1-argument ISO mode, or reject the alias in the UI dialect.
- **SUBTOTAL codes 101–111 are accepted.**
  - AOO 4.1 rejects nFunc > 11 with Err:502 (`interpr1.cxx:6483`).
  - This is a harmless extension, but the module doc should say it is not AOO.
- **The SUBTOTAL module doc is out of date.** It says 101–111 "skip hidden rows" and 1–11 "include hidden rows", but neither is wired (see H1).

---

## Unconfirmed (suspected, not proven against AOO)

- **Paste Special arithmetic with a formula source.**
  - Repro: B1 `=A1*5` pasted with Add onto 10 gives the value 20.
  - AOO `ScColumn::MixData` may build a combined formula.
  - A text source with Add replaced the target number ("x").
- **Binding of names after deleting a sheet-scoped name.**
  - Repro: a formula bound to a sheet-scoped X shows #NAME? after that name is deleted, even though a workbook X exists. Re-typing the formula binds to the workbook X.
  - This probably matches AOO's index binding, but AOO 4.1's sheet-local name semantics need an oracle.
  - A global name `Y = X*10` used the sheet-scoped X. In AOO a global name's expression may resolve in global scope.
- **Error codes.** `POWER(-8;1/3)` gives #NUM!. AOO's NaN path (`TreatDoubleError`) may give #VALUE!.
- **CHAR(129)** gives "", depending on AOO's MS-1252 table.
- **The default criteria mode is wildcards**, so `SEARCH("b.";"abc")` gives #VALUE! and `VLOOKUP("ban.*";…)` gives #N/A. AOO 4.1's default is regular expressions. This is documented as a decision in `docs/architecture-plan.md:293`; I list it for completeness.

---

## Verified correct (one line each)

**Math, text and info.**
- CEILING/FLOOR with 2 and 3 arguments, all sign and mode combinations, match `ScCeil`/`ScFloor`. Negative × positive gives Err:502; a step of 0 gives 0.
- GCD/LCM over ranges, fractional truncation, and negative values (Err:502) are correct.
- BIT*: the 2^48 limit, shifts of 47 and 48, and negative shifts are correct.
- ROMAN modes 0–4 (499;4 → ID, 1999;1 → MLMVLIV), ROMAN(0) = "" and ROMAN(4000) = Err:502 are correct. So are ARABIC, BASE (minimum length, radix 37 gives Err:502) and DECIMAL (radix 36).
- CHAR/CODE follow Windows-1252 (128 ↔ €, 150 → –, Ā → 63). UNICHAR/UNICODE are correct.
- FIXED/DOLLAR rounding (half away from zero, negative decimals, no-commas flag) is correct.
- REPLACE past the end appends, position 0 gives Err:502, and REPT with count 0 or -1 is correct. SUBSTITUTE with "" or with instance 0 is correct, and FIND("") is correct.
- TYPE (1, 2, 16, 64), N, ISREF, IS*, ERRORTYPE (532, 32767) and CELL "address"/"type"/"prefix"/"width"/"row"/"col"/"color" are correct.
- ASC/JIS full-width ↔ half-width, including dakuten (ガ ↔ ｶﾞ), are correct. So are LENB/LEFTB/MIDB/RIGHTB with a split DBCS character becoming a space.
- AND/OR/XOR/NOT/IF/IFERROR/IFNA edge cases, COUNT with a string argument, and COUNTBLANK (`=""` is not blank) are correct.

**Dates.**
- TIME(25;0;0) = 1/24, and a negative total gives Err:502 (`ScTime`).
- DAYS360 US and EU, including the February cases, is correct.
- EASTERSUNDAY(2024) and EASTERSUNDAY(2000) are correct.
- YEARFRAC bases 0–4, including basis 1 across years (425/365.5), are correct.
- NETWORKDAYS reversed (-8) and with holidays, and WORKDAY with a weekend start, offset 0 and holidays, are correct.
- EDATE and EOMONTH clamping, and truncation of fractional months, are correct.
- WEEKS, MONTHS and YEARS in both modes match `datefunc.cxx:785-850`. WEEKNUM_ADD, WEEKSINYEAR and DAYSINMONTH are correct.

**Statistics.**
- STDEV/VAR of one value gives #DIV/0!.
- PERCENTILE/QUARTILE use (n-1)p interpolation.
- RANK ties are correct.
- MODE ties pick the smallest value, and no duplicates gives #VALUE! (`interpr3.cxx:3226-3258`).
- TRIMMEAN's k rule (floor, even) is correct.
- SKEW and KURT match the sample formulas to 1e-15.
- CORREL/SLOPE skip incomplete pairs.
- PROB with a non-unit sum gives #VALUE! (`interpr3.cxx:3924`).
- PERMUT and PERMUTATIONA are correct.
- STEYX, RSQ, INTERCEPT, FORECAST, COVAR, DEVSQ, AVEDEV, HARMEAN, GEOMEAN, LARGE and SMALL are correct.

**Distributions.**
- NORMDIST/NORMINV/NORMSINV, TDIST with tails 1 and 2 (negative t gives Err:502), and TINV two-tailed (2.2281) are correct.
- CHIDIST/CHIINV, FDIST/FINV, BETADIST with A/B bounds and the density flag, and BETAINV are correct.
- GAMMADIST uses the scale parameter. GAMMAINV, BINOMDIST and CRITBINOM are correct.
- NEGBINOMDIST is correct, and HYPGEOMDIST takes AOO's argument order (X; N; M; L).
- POISSON, EXPONDIST, WEIBULL, LOGNORMDIST (4-argument form exists in AOO, `interpr3.cxx:1508`), LOGINV and CONFIDENCE are correct.
- ZTEST matches AOO's `0.5 - gauss(...)`, which is the same as Excel's one-tailed value.
- TTEST types 1–3, FTEST and CHITEST (df = rows - 1 for a vector) are correct.
- GAUSS, PHI, FISHER, STANDARDIZE, B, GAMMA(-1.5) and GAMMALN are correct.

**Financial.**
- PMT/FV/PV/NPER/RATE signs and `type` are correct. IPMT for period 1 with type 1 gives 0.
- DB with the month argument and period > life+1 (Err:502) is correct. So are DDB with a factor and the salvage cap, and VDB with and without no_switch.
- CUMIPMT/CUMPRINC (type 2 treated as 1, as in `ScKumZinsZ`) are correct.
- NPV, IRR with a guess, MIRR, XIRR/XNPV, EFFECT/NOMINAL (truncated periods) and PRICE/YIELD are correct.
- Excel-documented reference values match for MDURATION, all COUP* functions, TBILLPRICE/YIELD/EQ, ACCRINT, ACCRINTM, AMORDEGRC (776), AMORLINC (360), ODDFPRICE (113.5977), ODDLPRICE, ODDFYIELD, ODDLYIELD, DOLLARDE/DOLLARFR, DISC, INTRATE, RECEIVED, PRICEDISC, PRICEMAT, YIELDDISC and YIELDMAT.
- DURATION with 6 arguments gives Err:504, as in AOO, where the add-in is DURATION_ADD.

**Array.**
- MDETERM, MINVERSE (a singular matrix gives Err:502) and MMULT are correct.
- LINEST's 5×2 statistics block matches the hand-computed values exactly: se 0.2887/0.6236, r² 0.9643, F 27, df 1 (2 with const = 0), and SS 4.5/0.1667.
- LOGEST, TREND and GROWTH shapes are correct.
- FREQUENCY uses upper-inclusive bins, which is correct apart from H3.
- SUMPRODUCT size mismatch gives #VALUE!.

**Lookup and reference.**
- LOOKUP in vector and array form, CHOOSE with ranges and fractional or zero index, and OFFSET with negative, omitted and zero size are correct.
- INDIRECT A1 forms (`$Sheet1.A3`, `Sheet1!A3`) and INDEX with 0 row or column are correct.
- MATCH -1, HLOOKUP and VLOOKUP wildcards (in wildcard mode) are correct.

**Database.**
- Field by name or index, `=` matching empty cells, `<>` matching non-empty cells, whole-cell matching, case-insensitive headers, DGET with several matches (Err:502) or none (#VALUE!), and DCOUNT with no field are correct.

**Complex, Bessel and conversion.**
- IMARGUMENT(-1) = π, and IMARGUMENT(0) and IMDIV by 0 give Err:502. Mixed i/j gives Err:502, and IMLN(-1) is correct.
- BESSELJ/Y/K/I match AOO's Abramowitz–Stegun approximations (`bessel.cxx`). BESSELK(2;0) = 0.11389388 exactly; it has about 1e-8 error against mpmath, which is AOO-faithful.
- The two's-complement limits of all 12 base conversions (±512, 10-digit hex and octal) are correct.
- CONVERT_ADD agrees with AOO's `analysishelper.cxx` table: HP uses 1.341022E-03 and "Pica" is a length unit.
- Built-in CONVERT DEM→FRF gives #N/A, which is correct. The AOO table (`officecfg/.../Calc.xcu`, `UnitConversion`) has only EUR→X entries plus inverse lookup, so cross rates need EUROCONVERT.

**Engine.**
- Name scope precedence (sheet over workbook), names inside names, a deleted name giving #NAME? with undo restoring it, redefinition dirtying users, a self-referencing name giving #NAME?, and a name pointing to a deleted sheet giving #REF! are correct. A relative name behaves relative to A1, as in AOO.
- Array formulas spread #N/A beyond the source size, replicate a single column, and refuse edits, inserts, deletes, clears, moves and sorts of part of a block. Inserting above shifts the block, copying a block pastes a new block, the ODS round trip keeps the anchor and cached values, and moving or deleting the whole block works.
- MoveRange: references into the moved block follow it, references from inside to outside stay, an overwritten destination becomes #REF!, and a range partly inside the block stays.
- Merges expand when a row is inserted inside them and shrink or vanish when rows or columns are deleted.
- Sort is stable, puts empties last in both directions, and orders case-sensitive text lowercase-first.
- Find: `from` rotation, values versus formulas mode, displayed-text search (50%), and wildcard with whole-cell matching work.
- ReplaceAll: a formula that no longer parses is kept and reported, `$1` back-references work, and values mode leaves formulas untouched.
- Borders: outer and inner edges are applied and removed correctly. Paste Special: skip-empty works, arithmetic onto empty cells works, and an empty source counts as 0.
- Fill: 1,2 → 3; 1,3 → 5; month-end series (Jan 31, Mar 31 → May 31, …, Sep 30, Nov 30, Jan 31, keeping the original day); a 28-day step; and a leap-year 4-year step are correct, as are Q1 → Q5, Item 01 → Item 02, 001x and a1b2.

**XLSX and ExcelA1.**
- These round-trip exactly: quoted sheet names (spaces, `''`, periods, numeric names, `'A1'`), 3-D `Sheet1:Data!A1`, union `(A1,B1)` ↔ `~`, intersection ` ` ↔ `!`, `_xlfn.` unknown functions, `50%`, `{1,2;3,4}` ↔ `{1;2|3;4}`, whole columns and rows, missing arguments, `"a""b"` and `_xlfn.CONCAT`.
- The 1900-01-01..1900-03-01 mapping is correct apart from serial 0 (see Low).
- Shared formulas are re-anchored per member (`sheet.rs:451-472`).
- Number formats translate correctly: `[$-409]`, `[$-F800]`, `General` → None, `[Red]`, sections, conditions, `[h]`, accounting, `[$€-407]`, CJK literals and `[DBNum1]`.
- Names (global, sheet-local and expression), merges, freeze panes, array formulas, column widths and row heights round-trip.

**UI.**
- `pointInsertion` places picks after operators, separators and `(`, and replaces a reference picked just before.
- `pretty()` skips strings and quoted sheet names.
- `a1.ts` validates ranges, and `detectSumRange` checks above the cell first, then to the left.
