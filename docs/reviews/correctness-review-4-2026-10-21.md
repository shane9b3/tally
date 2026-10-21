# Correctness review 4 — Tally wave 5 (b03e603, diff from e58a2bb)

Date: 2026-10-21. Oracle: AOO 4.1 source at `$S/openoffice-src/main` (`$A`). `$S` = `/private/tmp/claude-501/-Volumes-FILES-Projects-OSX-Office/5227e701-ee6d-4d46-9584-0099831db42f/scratchpad`.

Method: detached worktree at b03e603, shared build dir `$S/corr4-target`. Six parallel sub-reviews ran scratch integration tests and vitest files that print values. A coordinator then re-ran a sample of the High findings independently, as noted inline.

- **Scratch tests:** saved to `$S/c4/scratch-tests/` (the worktree has been removed): `c4_pivot.rs`, `c4_outl.rs`, `c4_struct.rs`, `c4_chpr.rs`, `c4_calc.rs`, `c4_calc_io.rs`, `c4_ui_autoinput.rs`, `c4_ui1.test.ts`, `c4_ui2.test.ts`.
- **Generated fixtures:** `$S/c4pivot/`, `$S/c4/c4_calc_fx/`, `$S/c4/chpr/`.
- **How to reproduce:** copy a test into `crates/tally-engine/tests/` and run `CARGO_TARGET_DIR=... cargo test -p tally-engine --features io --test <name> -- --nocapture --test-threads=1`.
- **Experiment count:** about 130 tests, roughly 450 printed observations, plus about 35 `tally-cli eval` runs. This is well above the 60 required.
- **AOO source gaps:** the checkout is sparse. It has no `editeng/`, `chart2/source/{view,tools}`, `sc/source/filter/{html,excel}`, `connectivity/` or `sc/source/ui/app/inputhdl.cxx`. Claims that depend on those are marked "from knowledge" or listed as unconfirmed.

Totals: 15 High, 45 Medium, 33 Low, plus an unconfirmed list.

---

## 1. Summary by priority

### High (silently wrong / data loss / unreadable file)

| # | Area | Title | Tally location |
|---|---|---|---|
| H1 | Outline / ODS | Row/col groups extending past the last written row → unclosed `<table:table-row-group>`; the file can't be reopened, or groups vanish | `tally-io-ods/src/write.rs:1371,1290,1522,1352` |
| H2 | Data tools | RemoveDuplicates shifts cells *below the selection* (incl. formulas) up | `tally-engine/src/tools.rs:1763` |
| H3 | DataPilot | Text/bool/error data: Sum/Min/Max/Product of text = 0 (AOO empty), TRUE not numeric, errors short-circuit Count, #DIV/0! instead of #VALUE! | `tools.rs:198,222,226-230`; `pivot.rs:1813,1823` |
| H4 | DataPilot | Item identity keyed on display text: 2 and '2 merged; 1.2/1.4 under "0" merged; Apple/apple split | `pivot.rs:585-596` |
| H5 | DataPilot | Show-as reference modes: subtotals/totals keep raw values; missing base → empty/#N/A instead of AOO rules | `pivot.rs:1842,1845,1913-1916` |
| H6 | DataPilot | AutoShow top-N drops ties, and totals still include auto-hidden items | `pivot.rs:987,1041-1062` |
| H7 | DataPilot / ODS | ODS import maps unknown or group-dimension field names to column 0 | `tally-io-ods/src/tools.rs:1228-1233,1254` |
| H8 | Structure | InsertCells/DeleteCells: notes and merges in an empty band don't move (the `shift_meta_only` shortcut) | `structure.rs:746,947-989` |
| H9 | Charts / XLSX | Charts sharing an anchor are silently dropped on XLSX save (rust_xlsxwriter keyed map); `suggest_chart` produces such anchors | `tally-io-xlsx/src/chart_write.rs:536-552` |
| H10 | Importers | XLS: time-only/duration serials < 60 get +1 day (0.75 → 1.75) | `tally-io-xls/src/lib.rs:188`; `tally-io-xlsx/src/dates.rs:47` |
| H11 | Calc | Precision-as-shown: a number-format change doesn't recalc, so stored results stay stale | `ops.rs:691-711,713ff` |
| H12 | Calc / IO | External-link classifier fires on string literals: `="Call DDE(x) later"` → #N/A after reopen | `external.rs:94-140`; `convert.rs:193-203` |
| H13 | Importers | SXC: names like `TAX1` bracketed as cell refs (16384-col grid vs OOo 1.x 256 cols) → wrong value | `tally-io-ods/src/sxc.rs:221-235,104` |
| H14 | UI | Editing a DataPilot layout discards field grouping (`group: null`) | `packages/app/src/pivot/pivotModel.ts:157,160` |
| H15 | UI i18n | de/fr/es: decimal comma not converted: `=SUMME(1;1,5)` → `SUM(1;1,5)` = 7 | `packages/app/src/i18n/formulaNames.ts:84-87` |

### Medium (wrong error / notable AOO deviation)

| # | Area | Title |
|---|---|---|
| M1 | Filter | Text/empty cells pass numeric `<`/`>`; numbers pass text `<`/`>` (`tools.rs:589-591`) |
| M2 | Filter | Top/Bottom N and N% ignore ties and AOO rounding; N=0 → nothing (`tools.rs:500-504`) |
| M3 | Filter | Contains/begins/ends test display text, not the input string (`tools.rs:630`) |
| M4 | Outline | AutoOutline detection differs (adjacency required, single refs ignored, multi-ref accepted, existing outline kept) (`outline.rs:364-372,403`) |
| M5 | Outline | Max depth 8 vs AOO 7; the XLSX writer silently drops level 8 (`outline.rs:35`, `tally-io-xlsx/src/tools.rs:75,95`) |
| M6 | Subtotals | Captions: "Count (Numbers only)"/"StDev (Sample)"; no "Result" for multiple columns; empty group → " Sum" (`tools.rs:1055,116-130`) |
| M7 | Fill | Weekday series counts N working days; AOO adds N calendar days then skips the weekend (`tools.rs:2000-2013`) |
| M8 | DataPilot | Item labels always text; numeric items not numbers; empty item "" not "(empty)" (`pivot.rs:1648-1656`) |
| M9 | DataPilot | Innermost field gets subtotals when only the Data field follows it (`pivot.rs:1173`) |
| M10 | DataPilot | GETPIVOTDATA: returns raw values under show-as; item name accepted as data field; page "- all -" not rejected; error cell not #REF! (`pivot.rs:1706/1734,2375,2450-2457`) |
| M11 | DataPilot | Numeric grouping: auto start/end, labels, end bucket (`pivot.rs:682-718`) |
| M12 | DataPilot | Date grouping ignores start/end; day labels and alignment wrong (`pivot.rs:721,752-767`) |
| M13 | DataPilot | RefreshPivot overwrites non-empty cells without the target-not-empty check (`pivot.rs:2184`) |
| M14 | DataPilot / ODS | date-start/end written as serials; captions in `application-data` instead of `tableooo:display-name` (`tally-io-ods/src/tools.rs:822-863,1011,1192,1511`) |
| M15 | Structure | InsertCells/DeleteCells don't adjust CF, validation, scenarios, DB ranges, AutoFilter (`structure.rs:692-945`) |
| M16 | CopySheet | Absolute-sheet refs `$Sheet1.A1` are re-pointed to the copy (`tools.rs:2113-2131`) |
| M17 | Structure | Full-width/height InsertCells ≠ InsertRows: whole-row refs and row heights/col widths not moved (`structure.rs:653-680`) |
| M18 | Names | CreateNamesFromLabels: no corner name, numeric labels become names, ref-like labels dropped, non-ASCII mangled (`tools.rs:2385-2500`) |
| M19 | AutoFormat | Default preset ≠ autoform.cxx; autofit no-op; no 3×3 minimum (`cellstyles.rs:518-530,743`) |
| M20 | Charts | `suggest_chart` label detection ≠ chartpos.cxx (dates/blanks are labels); invoices template plots Date as a series (`charts.rs:1595-1606`) |
| M21 | Charts | CopySheet: copied charts still plot the source sheet; `clone_for_sheet` exists but is unused (`charts.rs:223`, `tools.rs:2210`) |
| M22 | Print | Default rows/cols paginate at 24 px/100 px, but the file says 0.452 cm/2.258 cm: 38 vs 53 rows per page (`print.rs:332-338`) |
| M23 | Print | Implicit print area starts at the first used cell instead of A1 (`print.rs:389`) |
| M24 | Print | Text overflow doesn't extend the print area (`print.rs`) |
| M25 | Print | A manual break on a hidden row is lost (`print.rs:453,143`) |
| M26 | Print | "Print headers" doesn't shrink the page body (`print.rs:471`) |
| M27 | SXC | `'Bob''s'.A1` mangled → Err:501 (`sxc.rs:131-136`) |
| M28 | ODS / SXC | value-type date/time/percent/currency without a data style gets no standard format (`tally-io-ods/src/content.rs:843-861`) |
| M29 | Calc | Precision-as-shown doesn't round scientific formats (`tally-numfmt/src/precision.rs:30`) |
| M30 | Calc | Precision-as-shown rounds `#,##0,` to thousands; AOO uses nCntPost only (`precision.rs:36`) |
| M31 | Calc | Nested MULTIPLE.OPERATIONS → Err:504 (`recalc.rs:1480-1483`) |
| M32 | DBF | VFP (0x30/0x31) headers rejected (`tally-io-dbf/src/lib.rs:100-101`) |
| M33 | DBF | FoxPro `Y` → garbage text, `I` dropped (`lib.rs:312,358`) |
| M34 | DBF | Language driver 0/unknown → cp1252; AOO → IBM850 (`lib.rs:172`) |
| M35 | DBF | Header row lacks AOO `NAME,C,12` type descriptors (`lib.rs:263-274`) |
| M36 | HTML | Import locale always en-US; de export doesn't round-trip (`service/codec.rs:131-141`) |
| M37 | Calc | CORREL etc. accept reference unions (`interpr3.cxx:3955` → error) |
| M38 | Detective | Trace Error follows only the first error precedent; no service API (`detective.rs:159-186`) |
| M39 | UI | Multiple Operations builder writes outside the selection (`packages/app/src/data/builders.ts:360,369`) |
| M40 | UI | Format Cells can't clear a fill or reset the colour to Automatic (`formatCells.ts:166-167`, `styles.rs:403`) |
| M41 | UI i18n | Function-name localisation rewrites inside `'quoted sheet'` names (`formulaNames.ts:47-75`) |
| M42 | UI | AutoCorrect re-processes untouched existing text (iPhone → IPhone; MHz → Mhz) and capitalises lone words (`w56.ts:933-935`, `controller.ts:529`) |
| M43 | UI | TWo INitial CApitals: ASCII-only, mangles `ÄRGer`; no exception list (`autocorrect.ts:42-44`) |
| M44 | UI / engine | AutoInput limited to the contiguous block and offers formula results (`autocomplete.rs:36,63-66`) |
| M45 | UI charts | Percent stacking normalises across kinds and axes in combo charts (`toEcharts.ts:398`) |

### Low

- L1: Subtotals grand total also includes other groups' columns (`tools.rs:1191-1210`).
- L2: Consolidate makes a row for an empty label.
- L3: Consolidate "link" structure differs from AOO.
- L4: TextToColumns treats a quote mid-field as a toggle.
- L5: RemoveDuplicates compares display text.
- L6: DataPilot subtotal captions with several data fields or custom functions.
- L7: Outline layout ignored.
- L8: Drill-down sheet naming and position.
- L9: Delete Contents ▸ Formats keeps merges/CF/validation.
- L10: Deleting a cell style moves its cells to the parent instead of Default.
- L11: ODS cell styles written fully resolved.
- L12: XLSX loses named-style definitions and xfId.
- L13: Scenario "prevent changes" has no effect.
- L14: CopySheet duplicates DB range names.
- L15: Built-in `Result2` hard-codes `[$$-409]`.
- L16: ODS double-accounting underline → double.
- L17: XLSX diagonals with different styles collapse to one.
- L18: Formulas-print + zero suppression hides zero formulas.
- L19: Notes-at-end pages not counted in page totals.
- L20: Notes don't extend the print area.
- L21: A whole-row fill makes the area 16384 columns wide.
- L22: XLSX chart round-trip losses (bubble→scatter, dLblPos, mean line, pie grid, anchor drift).
- L23: The invoices template prints on 2 pages.
- L24: Excel `&P` codes not resolved in headers (by design).
- L25: Typed external ref gives Err:501, not the policy #REF! with a warning.
- L26: DDE warning detail is empty.
- L27: HTML whitespace, `'0012` and error round trip.
- L28: DBF overflow/invalid date become text.
- L29: XLS non-date number formats lost (documented).
- L30: AutoCorrect rewrites ordinal suffixes instead of superscripting them.
- L31: Shortcut mismatches (⇧F11 unbound under Tauri; ⌘L/E/R/J, ⇧⌘V, ⌘M deviations undocumented; Ctrl+D conflict; dangling `docs/parity-deviations.md` link).
- L32: 40 `t()` keys missing in de.json.
- L33: Format Cells always shows Protected checked.

---

## 2. High findings (detail)

### H1. ODS writer: groups past the last written row or column are unclosed or dropped (High, data loss)
- **Where:** `crates/tally-io-ods/src/write.rs:1371`. `end_row` ignores `sheet.row_groups` ends. Group close tags are written only inside `while r < end_row` (`:1522`). Columns have the same problem (`end_col` `:1290`, closes at `:1352`).
- **Repro:** `c4_outl::o9_groups_past_data` (A3 data, group rows 1..5). Re-run by the coordinator:
  ```
  O9 ods rows 1..5 data in row 2 -> Err("open: ApiError { code: \"bad_file\", message: \"XML error: ill-formed document: expected `</table:table-row-group>`, but `</table:table>` was found\" }")
  ```
  Groups starting after the last row vanish (`O8 two-from-row1 after: none`). Column groups past the data vanish (`O9 ods cols 4..7 past data -> Ok("none")`). XLSX is correct (`rows=[(1, 5, 1, false)]`).
- **Expected:** AOO writes the covered rows, so groups round-trip.
- **Fix:** include group ends in `max_style_row`/`end_col`, close any still-open groups after the loop, and add a regression test.

### H2. RemoveDuplicates moves cells below the selected range (High)
- **Where:** `crates/tally-engine/src/tools.rs:1763`. `shift_cells` with a column `Band` shifts to the sheet bottom.
- **Repro:** `c4_outl::r2_remove_dups_outside` (A1:A4 = K,a,a,b; A6 = "below-range"; A7 = `=A6`). Re-run by the coordinator:
  ```
  R2 errs=[] A1..A7=["K", "a", "b", "", "below-range", "below-range", ""] A6 edit="=A5" A5 edit="below-range"
  ```
- **Expected:** the doc comment says it shifts "inside the range, as Excel does". Cells outside the selection must not move. AOO 4.1 has no such tool, so Excel is the reference.
- **Fix:** move contents only within `start..=range.r1` and blank the range's tail rows.

### H3. DataPilot aggregation over text, bool and error cells (High)
- **Where:** `tally-engine/src/tools.rs:198` (Bool/Text only bump count), `:222` (an error short-circuits all functions including Count), `:226-230`; `pivot.rs:1813,1823`.
- **Repro:** `c4_pivot::c4_e04_aggfn_text_bool_error`. Groups: txt={a,b}, bool={TRUE,TRUE}, one={5}, err={=1/0,3}.
  - Sum: txt 0, bool 0, err #DIV/0!, Total #DIV/0!.
  - Count: err #DIV/0!.
  - Average/Min/Max/Product of txt: #DIV/0!/0/0/0.
  - StDev of one: #DIV/0!.
- **AOO:**
  - Strings are ignored except by Count (CNT2 counts strings and errors): `$A/sc/source/core/data/dptabres.cxx:431-442`.
  - No data gives an empty result: `:507-529,583-597`.
  - StDev/Var need n ≥ 2, else an error: `:524-527`.
  - Every error is output as errNoValue (#VALUE!): `$A/sc/source/core/data/dpoutput.cxx:573-575`.
  - TRUE is a value in the cache.
  - Expected values: Sum txt empty, bool 2, err #VALUE!; Count err 2, Total 9; StDev(one) #VALUE!.
- **Fix:** give the pivot a dedicated aggregator mirroring `ScDPAggData::Update/Calculate`.

### H4. DataPilot item identity uses display text (High)
- **Where:** `pivot.rs:585-596`.
- **Repro:** `c4_e03_item_identity_and_order`:
  ```
  TRUE 512 | '1' 384 | '2' 24 | '10' 4 | 'Apple' 2 | 'apple' 1 | 'banana' 64 | '' 32
  ```
  Number 2 and text '2 are merged; 1.2 and 1.4 formatted "0" are merged; Apple/apple are split.
- **AOO:** `ScDPItemData::Compare` (`$A/sc/source/core/data/dptablecache.cxx:72,265-295,872`). Values compare by value; strings compare with the case-insensitive collator (`global.cxx:1993`).
- **Fix:** key items on (is_value, f64) or a case-folded string.

### H5. Show-as reference modes on totals and missing bases (High)
- **Where:** `pivot.rs:1842` (`_ => continue` keeps raw), `:1845`, `:1913-1916`.
- **Repro:** `c4_e05_show_as_modes`, `c4_e06`.
  - "Difference from previous": the Total Result column shows raw 850/1465.
  - "% of previous": Q1 shows #N/A.
  - Named base missing: empty (diff) or #N/A (%).
  - With base field Rep (not in the layout), values are left raw.
  - The East Result / West Result subtotal rows are raw (850 and 1465).
- **AOO:** `ScDPDataMember::UpdateRunningTotals` (`dptabres.cxx:2330-2530`):
  - a total in the reference dimension gives empty (`:2343-2363,2522-2525`);
  - previous/next not found gives empty (`:2516`);
  - a named item not found gives an error (`:2519`);
  - a base field not in the layout gives an error (`:2413`);
  - errors are #VALUE!;
  - subtotal rows are computed (East 0, West 615).
- **Fix:** port the `pSelectDim` and relative-position logic.

### H6. AutoShow ties and totals (High)
- **Where:** `pivot.rs:987` (`take(count)`). Aggregates are computed before AutoShow (`:1041-1062`).
- **Repro:** `c4_e09_autoshow_ties_and_totals`, top 2 of a=10, b=5, c=5, d=1:
  ```
  a 10 | b 5 | Total Result 21
  ```
- **AOO:** ties with the last included member are included (`dptabres.cxx:3164-3184`). Auto-hidden members are invalid and the results are recalculated without them (`:1250-1260`; `dptabsrc.cxx:986-995`). Expected: a, b, c, Total 20.

### H7. ODS data-pilot import maps unresolved field names to column 0 (High)
- **Where:** `tally-io-ods/src/tools.rs:1228-1233` (`map_or(0, …)`), `:1254`.
- **Repro:** `c4_e16_open_lo_fixture` with `$S/c4pivot/lo_dategroups2.ods` (LibreOffice-shaped: group dimension "Date2" with `source-field-name="Date"`, plus a column field "Nonexistent"):
  ```
  rows [(0,"Date2",Years), (1,"Date",Months)] cols [(0,"Nonexistent")]
  ```
  After refresh, the items are '1899' … '1900 Result', and the column items are Amount values.
- **AOO:** a group dimension resolves through `data-pilot-groups/@source-field-name`; unknown dimensions are dropped (`$A/sc/source/filter/xml/xmldpimp.cxx:1630-1700`).

### H8. InsertCells/DeleteCells: notes and merges in an empty band don't move (High)
- **Where:** `crates/tally-engine/src/structure.rs:746`. It returns `shift_meta_only` when the used cells don't intersect the band. `shift_meta_only` (`:947-989`) rewrites only formulas and names.
- **Repro:** `c4_struct::a05_notes_empty_band`:
  - Note on empty B5, then InsertCells A2:C2 down: `a05 notes [(5, 1, "n41")]`. The note stays on row 5; it should be on row 6.
  - Merge B5:C5 on an empty sheet: unchanged (`r0: 4`).
  - Control `a10_notes_with_content`: with content in the band, both move.
- **AOO:** `ScDocument::InsertRow` always calls `ScTable::InsertRow`, which shifts attributes and notes (`$A/sc/source/core/data/document.cxx:826-880`).
- **Fix:** always run the note/link/merge step.

### H9. XLSX save drops charts with identical anchors (High)
- **Where:** `crates/tally-io-xlsx/src/chart_write.rs:536-552`. `insert_chart_with_offset` stores charts in a map keyed by `(row, col, y_off, x_off)` (rust_xlsxwriter 0.100 `worksheet.rs:5770`).
- **Repro:** `c4_chpr::ch09_xlsx_same_anchor_cell`:
  ```
  save -> Ok; reopened 1 charts: [(Pie, 1, 5, 0.0)]
  ```
  `ch06` with 14 charts at F2: `xlsx: 14 charts before, 1 after`.
- `suggest_chart` always anchors at `(region.r0, region.c1+2)` with zero offset (`charts.rs:1625-1631`), so two suggested charts on the same data region collide.
- **Fix:** nudge duplicate anchors by 1 px, or warn. Images likely have the same issue (unconfirmed).

### H10. XLS import shifts time/duration serials by +1 day (High)
- **Where:** `crates/tally-io-xls/src/lib.rs:188` calls `excel_serial_to_tally` for every DateTime. `tally-io-xlsx/src/dates.rs:47` adds 1 for `0.0..60.0`.
- **Repro:** `c4_calc_io::c4_calc_io_xls` (`x1900.xls`, A5 = 0.75, format `h:mm`). Re-run by the coordinator:
  ```
  A5 v=Number(1.75) text="1899-12-31 18:00:00" input="12/31/1899 18:00:00"
  ```
  The 1904 file gives `A5 v=Number(0.75) text="18:00:00"`. As a result `=A5*24` gives 42 instead of 18. The XLSX path shifts only date-formatted cells ≥ 1 (`tally-io-xlsx/src/sheet.rs:635`) and is correct.
- **Fix:** shift only date/datetime XFs with serial ≥ 1.

### H11. Precision-as-shown: a format change doesn't recalculate (High)
- **Where:** `crates/tally-engine/src/ops.rs:691-711` (ApplyStyle) and `:713ff` (SetNumberFormat). Formulas are never dirtied.
- **Repro:** `c4_calc::c4_calc_precision_stale_after_format` (PAS on; A1=0.333, B1=`=A1*3`, C1=`=1/3`, then the formats change):
  ```
  ST1 after format: A1 text "0.3" B1 0.9990000000000001 (AOO 0.9) C1 0.3333333333333333 text "0.33"
  ST3 after hard recalc: B1 0.8999999999999999 C1 0.33
  ```
  Stale values persist and are saved to the file.
- **AOO:** `$A/sc/source/core/data/documen8.cxx:419` broadcasts when the number format changes and CalcAsShown is on.
- **Fix:** dirty the range's formulas and their dependents on a format change when PAS is on.

### H12. External-link classifier matches inside string literals (High)
- **Where:** `crates/tally-engine/src/external.rs:94-115` (`DDE(` substring) and `:117-140` (`'…file:…'#`). Applied at load in `convert.rs:193-203`.
- **Repro:** `c4_calc_io::c4_calc_io_external_ods`. Re-run by the coordinator after an ODS and an XLSX round trip:
  ```
  A1 v=Error(NA) text="#N/A" input="=\"Call DDE(x) later\""
  A2 v=Error(Ref) text="#REF!" input="=\"see 'file:///etc/a.ods'#$S.A1\""
  ```
  Before the save both cells were text.
- **Fix:** classify on tokens, or skip `"…"` literals, as `excel_external_target` already does.

### H13. SXC import brackets names such as `TAX1` as cell references (High)
- **Where:** `crates/tally-io-ods/src/sxc.rs:221-235` (`is_cell_or_range` accepts 3 letters) and `:104`.
- **Repro:** `c4_calc_io_sxc` (name TAX1 → `$A$1` = 5; B2 = `=TAX1*2`):
  - Warning: "renamed … TAX1 → TAX1_".
  - Result: `B2 v=Number(0.0)`. Expected 10.
- **Fix:** in SXC, treat a token as a cell only if its column is ≤ IV and its row is ≤ 32000.

### H14. UI: editing a DataPilot layout drops grouping (High)
- **Where:** `packages/app/src/pivot/pivotModel.ts:157` (`group: null`) and `:160`. The OK flow goes through `w56.ts:345,369`.
- **Repro:** `c4_ui1` E1:
  ```
  before group: {"type":"numeric","start":0,"end":100,"step":10} after edit-layout round trip group: null
  ```
- **Fix:** carry `group` through `LayoutField`.

### H15. UI: decimal comma not localised in de/fr/es formulas (High)
- **Where:** `packages/app/src/i18n/formulaNames.ts:84-87`. Only function names are mapped.
- **Repro:** `c4_ui1` E10b: `=SUMME(1;1,5)` → `=SUM(1;1,5)`. Re-run by the coordinator:
  ```
  $ tally-cli eval '=SUM(1;1,5)'      -> 7
  $ tally-cli eval '=ROUND(2,345;2)'  -> Err:504
  ```
  The display direction shows `=SUMME(1;1.5)`.
- **AOO:** the UI grammar uses the locale's decimal separator.
- **Fix:** swap `,` ↔ `.` in numeric literals, outside strings and sheet names, in both directions.

---

## 3. Medium findings (detail, terse)

**Standard filter / data tools** (`c4_outl.rs`)

- **M1. Mixed-type comparisons.** `tools.rs:589-591`.
  - Repro `f2_mixed_types`, Pay = 10,30,20,40,50,"n/a",empty:
    - `>15` shows the "n/a" row;
    - `<15` shows the empty row;
    - `<abc` shows every number.
  - AOO `$A/sc/source/core/data/table3.cxx:1060-1235`: the value branch only for value cells and the string branch only for string cells, otherwise false; `bMixedComparison` is false for filters (`sc/source/core/tool/queryparam.cxx:209`).
  - Fix: Lt/Gt/Le/Ge return false on a type mismatch or an empty cell.
- **M2. Top/Bottom N.** `tools.rs:500-504`.
  - Repro `f3_top_n`: Largest 2 → `["20","30"]` (AOO: 20,20,30); SmallestPct 20 → `5,6` (AOO: `<=7` → 5,6,7); Largest 0 → `[]` (AOO clamps to 1 → 30).
  - AOO `table3.cxx:1280-1375` TopTenQuery.
- **M3. Contains tests the display text.** `tools.rs:630`.
  - Repro `f7`: 1234.5 under `#,##0` (shown as "1,235") does not match "234.5".
  - AOO `table3.cxx:1104-1110` uses GetInputString.
- **M4. AutoOutline.** `outline.rs:364-372,403`.
  - Repro `o5_auto_outline`: no group for refs below, non-adjacent refs, single cells, or left non-adjacent/right refs. `A4=SUM(A1:A3)+B1` is grouped. An existing group is kept.
  - AOO `table2.cxx:3094-3131` (DiffSign rule), `cell2.cxx:665-713` (single expressible reference), `olinefun.cxx:321-324` (clears first).
- **M5. Depth 8.**
  - `o1_depth` accepts level 8, and XLSX drops it silently (`O9 xlsx 8 levels -> … (6,24,7,false)]`).
  - AOO `sc/inc/olinetab.hxx:31` `#define SC_OL_MAXDEPTH 7`, confirmed by the coordinator; `olinetab.cxx:240-243`.
- **M6. Subtotal captions.** `tools.rs:1055`.
  - Repro `s1`/`s3`: "East Count (Numbers only)", "East StDev (Sample)", " Sum".
  - AOO `table3.cxx:812-840`: more than one result column gives "X Result"; otherwise Sum/Count/Average/Max/Min/Product/StDev/Var; an empty value gives "(empty)" (`globstr.src:692-736`).
- **M7. Weekday fill series.** `tools.rs:2000-2013`.
  - Repro `fs1`: Fri 2026-10-23 step 2 → 10-27, 10-29, 11-02. AOO → 10-26, 10-28, 10-30.
  - AOO `$A/sc/source/core/data/table4.cxx:1198-1216` (`aDate += nInc`, then Sat +2 / Sun +1), confirmed by the coordinator.

**DataPilot** (`c4_pivot.rs`)

- **M8. Item labels are always text.**
  - `c4_e03`: `K3 type: Text("1")`; the empty item is `Text("")`.
  - AOO `dpoutput.cxx:619-629` (numeric header cells) and `dptabres.cxx:1414-1415` ("(empty)").
- **M9. Spurious innermost subtotals when the Data field follows.** `pivot.rs:1173`.
  - `c4_e19`, `c4_e02`: "Apple Result" after each item, and East repeated.
  - AOO `dptabsrc.cxx:239-244` lcl_TestSubTotal.
- **M10. GETPIVOTDATA deviations.**
  - (a) Under show-as it returns raw 230; AOO returns the displayed value (`dpoutput.cxx:1820-1828`).
  - (b) `GETPIVOTDATA("East";K1)` → 850; AOO #REF! (`:1743-1749`).
  - (c) A page field at "- all -" filtered by item → 2315; AOO #REF! (`:1388-1410`).
  - (d) An error result cell → #DIV/0!; AOO #REF!.
  - Locations: `pivot.rs:1706/1734,2375,2450-2457`; `tally-functions/src/categories/spreadsheet/pivot.rs:79`.
- **M11. Numeric grouping.** `pivot.rs:682-718`.
  - `c4_e07`: an auto start yields a "<0" bucket; with a non-integer source the labels still use -1; 0..20 step 10 gives "20-29" and ">20".
  - AOO `dpgroup.cxx:161-272,934-1016`: auto start/end = source min/max; labels lo-hi; the End value joins the last group "10-20"; the last group is clamped.
- **M12. Date grouping.** `pivot.rs:721,752-767`.
  - `c4_e08`: start/end ignored; days step 1 labelled with the year; step 7 aligned to serial multiples of 7.
  - AOO `dpgroup.cxx:439-478,678-681,186-190,961-967`.
- **M13. Refresh clobbers cells.** `pivot.rs:2184`.
  - `c4_e14_refresh_clobbers`: K5 = "precious" becomes 'd'; no error.
  - AOO `$A/sc/source/ui/docshell/dbdocfun.cxx:1349-1371` (lcl_EmptyExcept, STR_PIVOT_NOTEMPTY).
- **M14. ODS interop.**
  - `c4_e15` writes `table:date-start="45292"` and the caption in `table:application-data`.
  - `c4_e16` reads `2024-01-01T00:00:00` as None and ignores `tableooo:display-name`.
  - AOO `XMLExportDataPilot.cxx:560-581,690-696`; `xmldpimp.cxx:1637-1657`.
  - Tally→Tally round trip is lossless.

**Structure / CopySheet / names / AutoFormat** (`c4_struct.rs`)

- **M15. InsertCells/DeleteCells skip CF, validation, scenarios, DB ranges and AutoFilter.** `structure.rs:692-945`.
  - `a03`: CF range and operand `$A$5` unchanged after InsertCells A1:A2 down; validation stays, so 999 is rejected in the new empty A3.
  - `a09`: DB range and AutoFilter unchanged. `f01`: scenario ranges unchanged.
  - AOO `$A/sc/source/core/data/documen3.cxx:882-918` UpdateReference.
- **M16. CopySheet re-points absolute-sheet refs.** `tools.rs:2113-2131`.
  - `b01`: `=$Sheet1.A1` becomes `=$Sheet1_2.A1` (100); expected 5.
  - AOO `documen2.cxx:912-916` UpdateInsertTabAbs; `cell2.cxx:1155` (`!rRef1.IsTabRel()`, confirmed by the coordinator).
  - Fix: change only `abs:false` sheet refs, and also in CF/validation formulas.
- **M17. Full-width InsertCells ≠ InsertRows.** `structure.rs:653-680`.
  - `a07`: `=SUM(2:3)` unchanged and the row height not moved.
  - `a11`: `=SUM(C:C)` unchanged after a full-height insert-right, and C's width stays.
  - AOO `refupdat.cxx:301`; `table2.cxx:138-142,253-258`.
- **M18. CreateNamesFromLabels.** `tools.rs:2385-2500`.
  - `d01`: no corner name; `_2024` created; `A1` and `r1` dropped; `Größe` → `Gr__e`.
  - AOO `$A/sc/source/ui/docshell/docfunc.cxx:4497,4624-4631`; `rangenam.cxx:479-519` MakeValidName (prefixes `_`, keeps Unicode letters).
- **M19. AutoFormat Default.** `cellstyles.rs:518-530,743`.
  - `h01`: no fills or colours, bold header, autofit is a no-op, 2×2 accepted.
  - AOO `$A/sc/source/core/tool/autoform.cxx:939-991`: blue header with white text, gray70 left column, gray20 right column and bottom row, 10 pt, thin borders. `docfunc.cxx:3714,3752` (optimal sizes). `cellsh3.cxx:849-850` (3×3 minimum).

**Charts / print** (`c4_chpr.rs`)

- **M20. `suggest_chart` label detection.** `charts.rs:1595-1606`.
  - `ch01`: a date column becomes a series; a header row with a blank cell is not detected.
  - `ch02`: on the invoices template the region `$Orders.$A$1:$G$15` plots Date (46296…) as a series, and Customer/Item/Status become empty series.
  - AOO `$A/sc/source/core/tool/chartpos.cxx:37-49,287-313`.
- **M21. CopySheet charts still plot the source sheet.**
  - `ch07`: `sheet 2 chart src=("$'My Data'.$A$1:$B$5"…)`.
  - AOO `charthelper.cxx:103-108` via `viewfun2.cxx:2841-2843`.
  - Fix: call the existing `charts::clone_for_sheet` from `copy_sheet`.
- **M22. Pagination uses 24 px/100 px defaults.** `print.rs:332-338`, `units.rs:4-6`.
  - `pr01`: 38 rows × 6 cols per page; the file says 0.452 cm/2.258 cm, which gives 53 × 7 in AOO (`stlsheet.cxx:58-59,186-202`); the XLSX-reopened doc gives 40 rows.
  - The same document paginates three ways.
- **M23. Implicit print area starts at the first used cell.** `print.rs:389`.
  - `pr03a`: area `{r0:9,c0:2}`. AOO `printfun.cxx:679-684` starts at 0,0.
- **M24. Overflow text is not included.**
  - `pr03b`: a 400-character A1 gives `c[0,0]`. AOO `printfun.cxx:730-737` ExtendPrintArea.
- **M25. A manual break on a hidden row is lost.** `print.rs:453`.
  - `pr03d`: 1 page, `row_breaks=[]`. AOO `table5.cxx:212-218,250-257` gives 2 pages.
- **M26. Print headers don't shrink the body.** `print.rs:471`.
  - `pr02`: `r[0,52]` both with and without headers. AOO `printfun.cxx:2953-2957` gives 52 rows.

**Calc / importers** (`c4_calc*.rs`)

- **M27. SXC `''` in quoted sheet names.** `sxc.rs:131-136`.
  - `='Bob''s'.A1+1` becomes `oooc:='Bob'['s'.A1]+1`, giving Err:501.
- **M28. value-type without a data style.** `tally-io-ods/src/content.rs:843-861`.
  - Shipped `ooo1.sxc` A3 shows "46315" for a date; time/percent/currency cells show raw numbers.
  - AOO `$A/sc/source/filter/xml/xmlimprt.cxx:2343-2387` SetType.
- **M29. PAS ignores scientific formats.** `tally-numfmt/src/precision.rs:30`.
  - `1.23E-04` is read as 0.000123456.
  - AOO `documen4.cxx:519-526`.
- **M30. PAS `#,##0,`.** `precision.rs:36`.
  - 1234 is read as 1000; AOO uses nCntPost only, giving 1234 (`zformat.hxx:262`, `documen4.cxx:514`).
- **M31. Nested MULTIPLE.OPERATIONS.** `recalc.rs:1480-1483`.
  - Err:504; expected 8000.
  - AOO `interpr4.cxx:2984-3075` keeps a table-op stack.
- **M32. VFP headers refused.** `tally-io-dbf/src/lib.rs:100-101`.
  - "not a dBase table"; 0x30 is documented as supported.
- **M33. FoxPro Y/I types.** `lib.rs:312,358`.
  - Y gives `Text("@\u{fffd}\u{1}\0…")`; I is dropped. Expected 12.3456 and 42.
- **M34. Language driver 0.** `lib.rs:172`.
  - Bytes 0x81 0x84 are decoded as cp1252; AOO uses IBM850 ("üä") (`$A/sc/source/ui/docshell/docsh.cxx:1247-1253`).
  - 0x03, 0x57 and 0xC9 are correct.
- **M35. DBF header row.** `lib.rs:263-274`.
  - Writes names only; AOO writes `NAME,C,12` etc. (`docsh8.cxx:320-351,445-490`).
- **M36. HTML import locale fixed to en-US.** `service/codec.rs:131-141`.
  - A de-DE export reimports `1234,5` as text; `<html lang="de">` is ignored.
  - The AOO HTML filter is not in the checkout, so AOO behaviour here is from knowledge (Import Options dialog).
- **M37. Unions accepted by matrix functions.**
  - CLI: `=CORREL(A1:A2~C1:C2;A1:A4)` → 1.
  - AOO `interpr3.cxx:3955-3960` → `interpr5.cxx:515-519` raises an error.
- **M38. Trace Error.** `detective.rs:159-186`.
  - A1 = 1/0, A2 = NA(), A3 = A2, B1 = A1+A3 gives `[A1]` only; the function is not exposed through the service.
  - AOO `detfunc.cxx:1016-1040` InsertErrorLevel follows every erroneous reference.

**UI** (`c4_ui1/2.test.ts`, `c4_ui_autoinput.rs`)

- **M39. Multiple Operations builder.** `builders.ts:360,369`.
  - E11c: formulas B1:D1 with selection A2:B3 write columns C and D.
  - AOO `documen4.cxx:251-253,264-266` clamps to the selection.
- **M40. Format Cells can't clear.** `formatCells.ts:166-167`, `styles.rs:403`.
  - E3b: `fill->None patch.fill: null patchIsEmpty: true`. E3c: the same for colour → Automatic.
- **M41. Function-name mapping inside sheet names.** `formulaNames.ts:47-75`.
  - E17: `='Summe (2024)'.A1*2` becomes `='SUM (2024)'.A1*2`.
- **M42. AutoCorrect on untouched text.** `w56.ts:933-935`, `controller.ts:529`.
  - E16: iPhone → IPhone, eBay → EBay, MHz → Mhz, tel. → Tel.
  - E6: `hello` → `Hello`.
  - AOO behaviour is from knowledge, since editeng is not in the tree: DoAutoCorrect only acts at typed delimiters.
- **M43. TWo INitial CApitals.** `autocorrect.ts:42-44`.
  - E5: `ÄRGer` → `ÄRger`; CDs → Cds; IDs → Ids.
- **M44. AutoInput.** `autocomplete.rs:36,63-66`.
  - `c4_ui_autoinput_gap`: no suggestion across a gap; formula results (`="Bfx"`) are offered.
  - AOO `$A/sc/source/core/data/column3.cxx:1567-1620` scans the whole column for string/edit cells only; `documen3.cxx:1441` excludes numbers and formulas.
- **M45. Percent stacking in combo charts.** `toEcharts.ts:398`.
  - E15f: bars `[0.99,0.50,0.33]` because they were normalised together with a secondary-axis line.

---

## 4. Low findings (one line each; repro test in brackets)

- **L1.** Subtotals grand total includes the second group's columns: `Grand Total … =SUBTOTAL(4,C2:C14)`. AOO `table3.cxx:719` uses group 0 only. `tools.rs:1191-1210`. [`s2`]
- **L2.** Consolidate creates a row for an empty label. AOO `consoli.cxx:300,317`. `tools.rs:1415`. [`c1`]
- **L3.** Consolidate link writes a single `=COUNTA($Sheet1.B3,$Sheet1.E3)` with no detail rows or outline. AOO `consoli.cxx:737-818`. `tools.rs:1501`. [`c1`]
- **L4.** TextToColumns: `ab"c,d"e,f` → `abc,de | f`. AOO `impex.cxx:1386-1407` recognises a quote only at field start. `tools.rs:1547`. [`t1`]
- **L5.** RemoveDuplicates treats 1 and 1 formatted "0.0" as distinct. [`r1`]
- **L6.** Pivot captions with several data fields: "East Result | _ | Sum - Revenue" vs AOO "East Sum - Revenue"; custom function "East Max" vs "East Max - Revenue". `dptabres.cxx:866-899,1484-1535`. [`c4_e02`, `c4_e02b`]
- **L7.** Pivot outline layout is ignored (`lay_out` never reads `layout`). AOO `dptabres.cxx:1440-1452`. [`c4_e17`]
- **L8.** Drill-down sheet is named "DataPilot1_Data" and placed after the pivot's sheet; AOO uses `CreateValidTabName` and inserts at the current tab (`dbfunc3.cxx:2190-2193`). [`c4_e12`]
- **L9.** Delete Contents ▸ Formats keeps merges, CF and validation. AOO `attarray.cxx:2240-2247`. `tools.rs:1830-1833`. [`e01`]
- **L10.** Deleting a style moves its cells to the parent, not to Default. AOO `attarray.cxx:1751-1766`. `cellstyles.rs:435-452`. [`g03`]
- **L11.** ODS child styles are written fully resolved, so in AOO edits to the parent won't propagate. [`i02`]
- **L12.** XLSX: `cellStyleXfs` are all identical and every cell has `xfId="0"`, so named styles are lost. `tally-io-xlsx/src/tools.rs:885-920`. [`i02`]
- **L13.** Scenario "prevent changes" is ignored on a protected sheet. AOO `table2.cxx:1700-1716`. [`f01`]
- **L14.** CopySheet duplicates DB range names (`db1` twice). [`b03`]
- **L15.** `Result2` style hard-codes `[$$-409]` under de-DE. AOO `stlsheet.cxx:252-260`. `cellstyles.rs:52-56`. [`g02`]
- **L16.** ODS DoubleAccounting underline → Double; L17. XLSX diagonals with different up/down styles collapse to one. Neither warns. [`i01`]
- **L18.** Formulas-print + hide zeros hides zero-result formulas. AOO `cellform.cxx:79-149`. `service/print.rs:587`. [`pr06`]
- **L19.** Notes-at-end pages are not counted ("1 of 1", PDF `/Count 2`). AOO `printfun.cxx:1051-1053`. `print.rs:752`. [`pr07`]
- **L20.** Notes don't extend the print area. AOO `table1.cxx:475-490`. [`pr07`]
- **L21.** A whole-row fill makes the area 16384 columns wide; with suppression off that is 2731 pages. AOO `table1.cxx:506-537`. [`pr03c`, `pr09`]
- **L22.** XLSX chart losses: bubble → scatter, `dLblPos` not written, mean line dropped, pie gains a major grid, 100 % chart gains `0%`, combo line gains a marker, anchors drift by 0.05 mm. [`ch06`]
- **L23.** The invoices template prints on 2 pages (174.7 mm of columns vs 170 mm). Suggest landscape or fit-to-width. [`pr08`]
- **L24.** Excel `&P/&N/&A` are not resolved in Tally headers. This is by design (converted on XLSX I/O). [`pr05`]
- **L25.** A typed `='file:///x.ods'#$Sheet1.A1` gives Err:501 with no warning (the policy runs only at load). Typed DDE gives #N/A with no warning. [`c4_calc_external_links_typed`]
- **L26.** The DDE warning reads "(first: )." when the arguments are unquoted. `external.rs:94-113`.
- **L27.** HTML round trip: whitespace collapsed, `'0012` → 12, `#DIV/0!` comes back as text. [`c4_calc_io_html`]
- **L28.** DBF: `**********` overflow and invalid date `20260230` become text. [`c4_calc_io_dbf`]
- **L29.** XLS non-date number formats are lost (documented calamine limitation).
- **L30.** AutoCorrect "ordinals" rewrites suffixes (`1th` → `1st`) instead of superscripting. AOO behaviour is from knowledge. `autocorrect.ts:68-76`. [E4]
- **L31.** Shortcuts [E13]:
  - ⇧F11 (Save as Template) is dead under Tauri: `menu: true` but no accelerator in `menu.rs:42`.
  - These AOO deviations are undocumented: ⌘L/E/R/J alignment, ⇧⌘V, ⇧⌘F2, ⇧F4, ⌘M vs AOO ⌃M (⌘M collides with Window ▸ Minimize).
  - Ctrl+D is taken by the selection list on Windows/Linux (`W56Chrome.tsx:223`).
  - `docs/shortcuts.md:138` links to a missing `docs/parity-deviations.md`.
- **L32.** 40 of 243 `t()` keys are missing from `de.json` (e.g. `cmd.format.cells`, `cmd.edit.fillSeries`). The strings that are present match AOO German. [E14]
- **L33.** Format Cells always shows Protected = checked (`formatCells.ts:110`). [E3f]

---

## 5. Verified correct (brief)

**DataPilot**
- Page block with a blank row, "Filter" button row, "- all -" (`dpoutput.cxx:692-722,831-845`).
- "Total Result"; corner caption "Sum - Revenue".
- Hidden items excluded from totals; GETPIVOTDATA on a hidden item → #REF!.
- Item order: numbers < text < empty, case-insensitive.
- % of row/col/total and Index.
- Difference/% difference: base empty, previous-at-first empty.
- Running total; StDevP(n=1) = 0.
- Month/Quarter/Year+Month labels.
- GETPIVOTDATA: pair form, Excel form, unknown field → #REF!.
- Drill-down row counts 3/12/24.
- Empty line after each item; Tally↔Tally ODS round trip.

**Outline**
- Partial overlap refused; nesting on a shared start or end (`olinetab.cxx:193-223`).
- Collapse kept separate from user-hidden and filtered rows (`olinefun.cxx:420-428`).
- SUBTOTAL 1–11 vs 101–111 over collapsed and filtered rows (`dociter.cxx:345,1077`).
- ShowOutlineLevel semantics (`olinefun.cxx:391-412`).
- ODS/XLSX round trips when the data covers the groups.

**Filters**
- AND binds tighter than OR (`table3.cxx:1240-1266`). The brief said AOO evaluates connectors left to right; that is not what AOO does.
- Case and regex whole-cell matching; `<>` keeps text and empty cells.
- Advanced filter: OR across rows, AND across columns.

**Subtotals, consolidate, text to columns, fill series**
- Subtotals: presort, SUBTOTAL codes, nested groups and outline, undo, RemoveSubtotals.
- Consolidate: case-sensitive labels, correct sums.
- Text to columns: merge delimiters, fixed width, Text type keeps "0012".
- Fill series: month clamping (Jan 31 → Feb 29 → Mar 31, `table4.cxx:1220-1255`); linear and growth with a stop value.

**Insert/delete cells and structure**
- InsertCells/DeleteCells refs: inside, spanning, partial and #REF! cases (`refupdat.cxx:301`).
- Partial merge or array refused (`docfunc.cxx:1480-1486`); push-off-sheet refused; refs from another sheet adjusted.

**CopySheet, sort, names, Delete Contents, scenarios, styles**
- CopySheet: relative and sheet-relative refs follow the copy; global names stay; sheet-scoped names are cloned.
- Sort: natural order, custom lists, left-to-right, include-formats off, booleans with numbers.
- Names from labels: `_1st_row`, leading blanks stripped, top+bottom.
- Delete Contents numbers vs dates semantics (`column3.cxx`).
- Scenario show and copy-back.
- Style parent-chain propagation; Default edits reach children.
- Rotation, stacked, shrink and double underline ODS/XLSX round trips.

**Charts**
- Trend lines: linear, exp, log and power coefficients and R² equal LINEST/LOGEST/RSQ via `tally-cli` (linear R² 0.998210666107). The LibreOffice fixture trend gives R² 0.885542168674699.
- Error bars: stdev/var/stderr/percent/margin/constant/range.
- `chart_data`: gaps, booleans, hidden rows and columns excluded.
- ODS round trip of 14 chart variants with 0 differences; XLSX core kinds round-trip.
- Structure adjustment on insert/delete and on sheet rename.

**Print**
- 53 rows per page at 4.52 mm (`table5.cxx:206`).
- FitTo zoom search matches AOO's binary search (`printfun.cxx:2836-2905`).
- Manual breaks under fit-to (`table5.cxx:131-145`).
- Repeat rows/cols; page order; header/footer fields and first page number; empty-page suppression.
- Invoices preview PNG is sane.

**Calc**
- Iterations: `=A1/2+1` → 1.99902 with 100 steps / 0.001 (`cell.cxx:1590-1606`) and exactly 2 with eps = 0; `=A1+1` → Err:523; 2-cell cycle; Err:522 when iterations are off; settings undo.
- PAS: percent +2 decimals (`documen4.cxx:516`); dates and General not rounded; negative section; toggling off restores values.
- MULTIPLE.OPERATIONS: AOO one- and two-variable help examples (−6000 … 35000).
- Reference unions: SUM/COUNT/LARGE/RANK/MEDIAN/AREAS/INDEX; COLUMNS → Err:504 (`interpr1.cxx:3918-3955`).
- Detective precedents and dependents.
- Solver: LP 21 at (3, 1.5), MILP 20 at (4, 0), binary, unbounded/infeasible, non-linear refused.

**Importers**
- XLS 1904 epoch and its round trip.
- SXC quoted sheet names and union.
- HTML colspan/rowspan merges, entities, en-US number/date/percent recognition, export escaping and merges.
- DBF N/F/D/L, cp1252/cp1251, deleted records skipped.

**UI**
- Pivot default function and caption (`pvlaydlg.cxx:272-299`).
- Format Cells empty-patch and border semantics.
- Standard filter, subtotal, fill, sort and dedup DTOs.
- Multiple Operations argument order (`documen4.cxx:242-282`).
- AutoCorrect skips formulas and quoted text.
- AutoInput: case-insensitive, shortest-first, numbers ignored.
- SUMME ↔ SUM round trip for every name in de/fr/es.
- ECharts: percent stack, exploded pie, stock role order (`chart2/.../DialogModel.cxx:149-152`).
- Most accelerators match `Accelerators.xcu`.
- Options defaults (`docoptio.cxx:110-124`).

## 6. Unconfirmed (code reading or missing oracle)

**DataPilot**
- Header with row fields and no column field: AOO may leave B1 empty (`dpoutput.cxx:850-930`).
- Whether AOO drill-down excludes hidden members.
- GETPIVOTDATA function suffixes (`dpobject.cxx:1147-1200`).

**Fill series**
- Fill Year from Feb 29 (`table4.cxx:1259-1268` SetYear, which probably normalises to Mar 1).

**Text to columns**
- How much of the target area Text to Columns clears.

**Structure and styles**
- AOO's merge-extend cases when inserting cells into a merge (`docfunc.cxx:1460-1580`).
- Error-value sort position (`table3.cxx:262-300`).
- Child-style attributes after the parent style is deleted.
- Colours of the non-Default AutoFormat presets (`autotbl.fmt` is not in the tree).
- Whether CreateNames replaces existing names.

**Charts**
- Error-bar population vs sample statistics; exponential trend R² base (chart2 tools are missing).
- Wizard orientation choice.
- XLSX images at identical anchors.
- Whether a chart series is removed when its whole range is deleted.

**Calc and importers**
- COUNTIFS over a union.
- PAS with fraction formats.
- PAS with an inherited formula format (`cell.cxx:1693-1700`).
- AOO XLS handling of serials below 61.
- dBase driver byte mapping (`connectivity/` is missing).

**UI**
- Iteration-steps maximum: AOO's dialog is 1..1000 (`$A/sc/source/ui/src/optdlg.src:58-59`), while Tally allows 32767 and its comment claims AOO does too (`calcsettings.rs:24`).
- `customLists()` may overwrite document lists after reopen.
- ⌘M is swallowed by Minimize.
- The Ctrl+D selection list ignores validation lists (`documen3.cxx:1448-1459`).
- `savePivot` to a new sheet takes two undo steps.
- Negative pie values.
- Sentence-capitalisation exceptions (`etc. and` → `Etc. And`).
