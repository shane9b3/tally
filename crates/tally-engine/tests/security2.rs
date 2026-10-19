//! Regression tests for the second security review
//! (`docs/reviews/security-review-2-2026-10-19.md`), engine side. Each test
//! rebuilds the review's input shape (smaller where the shape, not the
//! size, is what matters) and asserts a functional result plus a generous
//! time bound that holds in debug builds. Release-only timings are
//! `#[ignore]`d with a note; run them with
//! `cargo test --release -p tally-engine --features io --test security2 -- --ignored`.
#![allow(clippy::unwrap_used)]

mod common;

use std::time::{Duration, Instant};

use common::{input, Fixture};
use tally_api::Op;
use tally_engine::recalc::recalc;
use tally_engine::Workbook;
use tally_model::{CellContentData, CellData, NameData, SheetData, WorkbookData};
use tally_types::{CellAddr, ErrorKind, Value};

fn formula_cell(row: u32, col: u32, text: &str) -> CellData {
    CellData {
        row,
        col,
        content: CellContentData::Formula {
            text: text.into(),
            cached: None,
        },
        style: None,
    }
}

fn value_cell(row: u32, col: u32, v: Value) -> CellData {
    CellData {
        row,
        col,
        content: CellContentData::Value(v),
        style: None,
    }
}

fn assert_fast(what: &str, t: Instant, limit: Duration) {
    let took = t.elapsed();
    assert!(took < limit, "{what} took {took:?} (limit {limit:?})");
}

// ---------------------------------------------------------------------------
// CRITICAL-1: names are evaluated by reference
// ---------------------------------------------------------------------------

/// The review's `p1_names.py`: nm_0 = 1, nm_i = nm_{i-1}+…+nm_{i-1} (`fan`
/// copies), 16 levels, and `cells` cells `=nm_15`. Inlining built fan^15
/// nodes per cell (fan 10: killed at 6 GB after 1.4 s; fan 2 × 1000 cells:
/// 14 s and 2.5 GB).
fn fan_names(fan: usize, cells: u32) -> WorkbookData {
    let mut names = vec![NameData {
        name: "nm_0".into(),
        sheet: None,
        formula: "of:=1".into(),
    }];
    for i in 1..16 {
        names.push(NameData {
            name: format!("nm_{i}"),
            sheet: None,
            formula: format!("of:={}", vec![format!("nm_{}", i - 1); fan].join("+")),
        });
    }
    WorkbookData {
        sheets: vec![SheetData {
            name: "S".into(),
            cells: (0..cells)
                .map(|r| formula_cell(r, 0, "of:=nm_15"))
                .collect(),
            ..SheetData::default()
        }],
        names,
        ..WorkbookData::default()
    }
}

#[test]
fn fan_out_names_open_in_linear_time() {
    let t = Instant::now();
    let mut wb = Workbook::from_data(&fan_names(10, 1));
    recalc(&mut wb);
    assert_fast("fan 10 × depth 16 open", t, Duration::from_secs(1));
    // 10^15 is exact in f64.
    assert_eq!(wb.value(CellAddr::new(1, 0, 0)), Value::Number(1e15));

    let t = Instant::now();
    let mut wb = Workbook::from_data(&fan_names(2, 1000));
    recalc(&mut wb);
    assert_fast("fan 2 × 1000 cells open", t, Duration::from_secs(2));
    assert_eq!(wb.value(CellAddr::new(1, 999, 0)), Value::Number(32768.0));
}

/// The API path (`probe op names_cycle`): DefineName gx_0..gx_15 with fan
/// 10, then a cell `=gx_15` (timed out at 60 s with 2.6 GB before).
#[test]
fn fan_out_names_through_the_api() {
    let mut f = Fixture::new();
    let mut ops = vec![Op::DefineName {
        name: "gx_0".into(),
        scope: None,
        refers_to: "2".into(),
    }];
    for i in 1..16 {
        ops.push(Op::DefineName {
            name: format!("gx_{i}"),
            scope: None,
            refers_to: vec![format!("gx_{}", i - 1); 10].join("+"),
        });
    }
    let t = Instant::now();
    f.apply(ops);
    f.set("A1", "=gx_15");
    f.set("A2", "=gx_15*0+gx_3");
    assert_fast("define + use", t, Duration::from_secs(1));
    assert_eq!(f.value("A1"), Value::Number(2e15));
    assert_eq!(f.value("A2"), Value::Number(2000.0));
    // Redefining the root refreshes every user through the chain.
    let t = Instant::now();
    f.apply(vec![Op::DefineName {
        name: "gx_0".into(),
        scope: None,
        refers_to: "$Sheet1.$B$1".into(),
    }]);
    f.set("B1", "3");
    assert_fast("redefine", t, Duration::from_secs(1));
    assert_eq!(f.value("A1"), Value::Number(3e15));
}

/// By-reference evaluation keeps the inlining semantics: relative
/// references are relative to the using cell, a name's range feeds
/// reference functions, self-reference and over-deep chains are `#NAME?`,
/// and editing a precedent of a name recalculates its users.
#[test]
fn names_by_reference_keep_semantics() {
    let mut f = Fixture::new();
    f.apply(vec![
        Op::DefineName {
            name: "Below".into(),
            scope: None,
            // Relative to the base A1: "the cell below the user".
            refers_to: "$Sheet1.A2".into(),
        },
        Op::DefineName {
            name: "Data".into(),
            scope: None,
            refers_to: "$Sheet1.$D$1:$D$3".into(),
        },
        Op::DefineName {
            name: "Loop".into(),
            scope: None,
            refers_to: "Loop+1".into(),
        },
    ]);
    for (a1, v) in [("B6", "7"), ("D1", "1"), ("D2", "2"), ("D3", "3")] {
        f.set(a1, v);
    }
    f.set("B5", "=Below*2");
    f.set("E1", "=SUM(Data)+ROWS(Data)+INDEX(Data;2)");
    f.set("E2", "=Loop");
    assert_eq!(f.value("B5"), Value::Number(14.0));
    assert_eq!(f.value("E1"), Value::Number(11.0));
    assert_eq!(f.value("E2"), Value::Error(ErrorKind::Name));
    f.set("D2", "20");
    assert_eq!(f.value("E1"), Value::Number(47.0));
    // 17 levels: one more than MAX_NAME_DEPTH.
    let mut ops = vec![Op::DefineName {
        name: "dd_0".into(),
        scope: None,
        refers_to: "1".into(),
    }];
    for i in 1..17 {
        ops.push(Op::DefineName {
            name: format!("dd_{i}"),
            scope: None,
            refers_to: format!("dd_{}+1", i - 1),
        });
    }
    f.apply(ops);
    f.set("F1", "=dd_15");
    f.set("F2", "=dd_16");
    assert_eq!(f.value("F1"), Value::Number(16.0));
    assert_eq!(f.value("F2"), Value::Error(ErrorKind::Name));
    let _ = input;
}

// ---------------------------------------------------------------------------
// MEDIUM-3: per-formula work budget
// ---------------------------------------------------------------------------

/// The review's `=SUM(MMULT(MUNIT(1448)+1;MUNIT(1448)+1))` repeated 20
/// times took 46 s; it now stops before the multiplication with Err:512.
#[test]
fn matrix_work_budget_stops_huge_mmult() {
    let mut f = Fixture::new();
    let one = "SUM(MMULT(MUNIT(1448)+1;MUNIT(1448)+1))";
    let t = Instant::now();
    f.set("A1", &format!("={}", vec![one; 20].join("+")));
    assert_fast("20 × MMULT(1448²)", t, Duration::from_secs(3));
    assert_eq!(f.value("A1"), Value::Error(ErrorKind::FormulaOverflow));
    f.set("A2", "=MDETERM(MUNIT(1448))");
    assert_eq!(f.value("A2"), Value::Error(ErrorKind::FormulaOverflow));
    // Ordinary matrix work is untouched.
    f.set("A3", "=SUM(MMULT(MUNIT(50)+1;MUNIT(50)+1))");
    assert_eq!(f.value("A3"), Value::Number(130_050.0));
    f.set("A4", "=MDETERM(MUNIT(100)*2)");
    assert_eq!(f.value("A4"), Value::Number(2f64.powi(100)));
}

// ---------------------------------------------------------------------------
// HIGH-3: many formulas reading one cell
// ---------------------------------------------------------------------------

/// `n` rows of `A{r}` = r and `B{r}` = `=A{r}*$C$1` (the review's
/// `p8c.py fanin`: 200k rows took 5.4 s to open and 23 s to recalc).
fn fan_in(n: u32) -> WorkbookData {
    let mut cells = vec![value_cell(0, 2, Value::Number(2.0))];
    for r in 0..n {
        cells.push(value_cell(r, 0, Value::Number(f64::from(r))));
        cells.push(formula_cell(r, 1, &format!("of:=[.A{}]*[.$C$1]", r + 1)));
    }
    cells.sort_by_key(|c| (c.row, c.col));
    WorkbookData {
        sheets: vec![SheetData {
            name: "S".into(),
            cells,
            ..SheetData::default()
        }],
        ..WorkbookData::default()
    }
}

fn check_fan_in(n: u32, open: Duration, hard: Duration) {
    let data = fan_in(n);
    let t = Instant::now();
    let mut wb = Workbook::from_data(&data);
    recalc(&mut wb);
    assert_fast("fan-in open", t, open);
    let last = CellAddr::new(1, n - 1, 1);
    assert_eq!(wb.value(last), Value::Number(f64::from(n - 1) * 2.0));
    let t = Instant::now();
    tally_engine::recalc::hard_recalc(&mut wb);
    assert_fast("fan-in hard recalc", t, hard);
    let t = Instant::now();
    wb.set_input(CellAddr::new(1, 0, 2), "3");
    recalc(&mut wb);
    assert_fast("fan-in edit", t, hard);
    assert_eq!(wb.value(last), Value::Number(f64::from(n - 1) * 3.0));
}

#[test]
fn fan_in_is_linear() {
    check_fan_in(20_000, Duration::from_secs(3), Duration::from_secs(3));
}

/// Release-only timing (debug builds are ~10× slower): 200k formulas open
/// in < 2 s and hard-recalculate in < 3 s.
#[test]
#[ignore = "release-only timing; run with --release -- --ignored"]
fn fan_in_200k_release_timing() {
    check_fan_in(200_000, Duration::from_secs(2), Duration::from_secs(3));
}

/// INDIRECT re-registers its dynamic dependencies on every evaluation;
/// with n of them reading `$D$1`, each edit of D1 cost O(n²) (100k: 6 s).
fn check_indirect(n: u32, limit: Duration) {
    let mut f = Fixture::new();
    let rows: Vec<Vec<String>> = (0..n).map(|r| vec![r.to_string()]).collect();
    let formulas: Vec<Vec<String>> = (0..n)
        .map(|_| vec!["=INDIRECT(\"A\"&(ROW()+$D$1))".to_string()])
        .collect();
    let s = f.sheet;
    f.apply(vec![
        input(s, "D1", "0"),
        Op::SetRangeInputs {
            sheet: s,
            r: 0,
            c: 0,
            rows,
        },
        Op::SetRangeInputs {
            sheet: s,
            r: 0,
            c: 1,
            rows: formulas,
        },
    ]);
    assert_eq!(f.value("B5"), Value::Number(4.0));
    let t = Instant::now();
    f.set("D1", "1");
    assert_fast("edit D1", t, limit);
    assert_eq!(f.value("B5"), Value::Number(5.0));
    let t = Instant::now();
    f.set("D1", "2");
    assert_fast("edit D1 again", t, limit);
    assert_eq!(f.value("B5"), Value::Number(6.0));
}

#[test]
fn indirect_fan_in_edit_is_linear() {
    check_indirect(10_000, Duration::from_secs(2));
}

/// Release-only timing: 100k INDIRECT dependents, each edit < 1 s.
#[test]
#[ignore = "release-only timing; run with --release -- --ignored"]
fn indirect_fan_in_100k_release_timing() {
    check_indirect(100_000, Duration::from_secs(1));
}

// ---------------------------------------------------------------------------
// MEDIUM-4 and LOW-3: transaction budget, Replace All and find limits
// ---------------------------------------------------------------------------

fn find_query(text: &str) -> tally_api::FindQuery {
    tally_api::FindQuery {
        text: text.into(),
        sheet: None,
        in_formulas: false,
        match_case: false,
        whole_cell: false,
        mode: tally_api::FindMode::Literal,
        from: None,
    }
}

/// One transaction may touch at most 50M cells (summed estimates): 64 ×
/// SetArrayFormula of 1M cells took 11 s and 1.7 GB. Replace All counts its
/// 1M-cell maximum, so the 51st of 51 cheap Replace Alls is refused.
#[test]
fn transaction_cell_budget() {
    let mut f = Fixture::new();
    f.set("A1", "x");
    let ops: Vec<Op> = (0..51)
        .map(|_| Op::ReplaceAll {
            sheet: None,
            query: find_query("nothing-matches"),
            replace: "y".into(),
        })
        .collect();
    let r = f.apply(ops);
    assert_eq!(r.errors.len(), 1, "{:?}", r.errors);
    assert_eq!(r.errors[0].op_index, 50);
    assert_eq!(r.errors[0].code, tally_api::ApiError::UNSUPPORTED);
    // Ordinary transactions are unaffected.
    let r = f.apply(vec![
        input(f.sheet, "B1", "1"),
        input(f.sheet, "B2", "=B1+1"),
    ]);
    assert!(r.errors.is_empty());
    assert_eq!(f.value("B2"), Value::Number(2.0));
}

/// Replace All: a replacement over 64 KiB is refused; a cell whose result
/// would pass 32,767 characters is left unchanged (ReplaceAll of 4 KB over
/// 1M cells held 5.9 GB).
#[test]
fn replace_all_limits() {
    let mut f = Fixture::new();
    f.set("A1", "ab");
    f.set("A2", &"a".repeat(20_000));
    let big = Op::ReplaceAll {
        sheet: None,
        query: find_query("a"),
        replace: "z".repeat(70_000),
    };
    let r = f.apply(vec![big]);
    assert_eq!(r.errors.len(), 1, "{:?}", r.errors);
    let r = f.apply(vec![Op::ReplaceAll {
        sheet: None,
        query: find_query("a"),
        replace: "zz".into(),
    }]);
    assert_eq!(f.value("A1"), Value::text("zzb"));
    assert!(
        matches!(f.value("A2"), Value::Text(t) if t.len() == 20_000),
        "unchanged"
    );
    assert!(
        r.errors.iter().any(|e| e.message.contains("32767")),
        "{:?}",
        r.errors
    );
}

/// LOW-3: a 1 MB find pattern (640 MB while compiling) is refused before
/// the regex compiler runs.
#[test]
fn huge_find_pattern_is_refused_early() {
    let f = Fixture::new();
    let t = Instant::now();
    let r = tally_api::DocumentService::find(&f.svc, f.doc, &find_query(&"a".repeat(1 << 20)));
    assert!(r.is_err());
    assert_fast("1 MB pattern", t, Duration::from_millis(200));
    let r = tally_api::DocumentService::find(&f.svc, f.doc, &find_query(&"a".repeat(1000)));
    assert!(r.is_ok());
}

// ---------------------------------------------------------------------------
// HIGH-5: AutoFilter value lists
// ---------------------------------------------------------------------------

fn check_filter(rows: u32, values: usize, limit: Duration) {
    let mut f = Fixture::new();
    let s = f.sheet;
    let col: Vec<Vec<String>> = std::iter::once(vec!["Head".to_string()])
        .chain((1..rows).map(|r| vec![format!("v{}", r % 20_000)]))
        .collect();
    f.apply(vec![Op::SetRangeInputs {
        sheet: s,
        r: 0,
        c: 0,
        rows: col,
    }]);
    f.apply(vec![Op::SetAutoFilter {
        sheet: s,
        range: Some(tally_api::RangeDto {
            r0: 0,
            c0: 0,
            r1: rows - 1,
            c1: 0,
        }),
    }]);
    // Upper-case values: matching is case-insensitive.
    let keep: Vec<String> = (0..values).map(|i| format!("V{i}")).collect();
    let t = Instant::now();
    let r = f.apply(vec![Op::SetColumnFilter {
        sheet: s,
        col: 0,
        filter: Some(tally_api::ColumnFilter {
            values: Some(keep),
            condition: None,
        }),
    }]);
    assert_fast("SetColumnFilter", t, limit);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    let wb = &f.svc.document(f.doc).unwrap().wb;
    let sheet = wb.sheet(s).unwrap();
    let hidden: u64 = sheet
        .filtered_rows
        .spans()
        .iter()
        .map(|(a, b)| u64::from(b - a) + 1)
        .sum();
    let kept = (1..rows)
        .filter(|r| ((r % 20_000) as usize) < values)
        .count() as u64;
    assert_eq!(hidden, u64::from(rows - 1) - kept);
}

#[test]
fn filter_value_list_is_a_set() {
    check_filter(20_000, 10_000, Duration::from_secs(5));
    // Lists past 100,000 values are refused.
    let mut f = Fixture::new();
    let s = f.sheet;
    f.apply(vec![Op::SetAutoFilter {
        sheet: s,
        range: Some(tally_api::RangeDto {
            r0: 0,
            c0: 0,
            r1: 10,
            c1: 0,
        }),
    }]);
    let r = f.apply(vec![Op::SetColumnFilter {
        sheet: s,
        col: 0,
        filter: Some(tally_api::ColumnFilter {
            values: Some(vec!["x".into(); 100_001]),
            condition: None,
        }),
    }]);
    assert_eq!(r.errors.len(), 1);
}

/// Release-only timing: 10,000 values × 100,000 rows apply in < 200 ms
/// (11.9 s before).
#[test]
#[ignore = "release-only timing; run with --release -- --ignored"]
fn filter_100k_rows_10k_values_release_timing() {
    check_filter(100_000, 10_000, Duration::from_millis(200));
}

// ---------------------------------------------------------------------------
// LOW-2: sheet names from files
// ---------------------------------------------------------------------------

/// Imported names follow the app's rules (`[]*?:/\` and control characters
/// become `_`); the file's formulas still find the renamed sheet.
#[test]
fn imported_sheet_names_are_sanitized() {
    let data = WorkbookData {
        sheets: vec![
            SheetData {
                name: "Main".into(),
                cells: vec![
                    formula_cell(0, 0, "of:=['a:b'.A1]"),
                    formula_cell(0, 1, "of:=SUM(['a:b'.A1:.A3])"),
                    formula_cell(0, 2, "of:=['x]y\u{1}'.A1]"),
                ],
                ..SheetData::default()
            },
            SheetData {
                name: "a:b".into(),
                cells: vec![value_cell(0, 0, Value::Number(100.0))],
                ..SheetData::default()
            },
            SheetData {
                name: "x]y\u{1}".into(),
                cells: vec![value_cell(0, 0, Value::Number(7.0))],
                ..SheetData::default()
            },
            SheetData {
                name: "a_b".into(),
                ..SheetData::default()
            },
        ],
        ..WorkbookData::default()
    };
    let mut wb = Workbook::from_data(&data);
    recalc(&mut wb);
    let names: Vec<&str> = wb.sheets.iter().map(|s| &*s.name).collect();
    assert_eq!(names, ["Main", "a_b", "x_y_", "a_b_2"]);
    for n in &names {
        assert!(tally_engine::ops::check_sheet_name(n).is_ok(), "{n}");
    }
    let main = wb.sheets[0].id;
    assert_eq!(wb.value(CellAddr::new(main, 0, 0)), Value::Number(100.0));
    assert_eq!(wb.value(CellAddr::new(main, 0, 1)), Value::Number(100.0));
    assert_eq!(wb.value(CellAddr::new(main, 0, 2)), Value::Number(7.0));
    assert!(wb.import_warnings.iter().any(|w| w.contains("Sheet names")));
    // The aliases are gone after loading.
    assert!(
        wb.parse_formula("='a:b'.A1", CellAddr::new(main, 5, 5))
            .is_err()
            || !matches!(
                wb.eval_formula_text_at(CellAddr::new(main, 5, 5), "='a:b'.A1"),
                Some(Value::Number(_))
            )
    );
}

// ---------------------------------------------------------------------------
// HIGH-2: array block budget in the engine
// ---------------------------------------------------------------------------

/// Readers count block areas, but the engine has its own workbook-wide
/// budget for data from any source: blocks past 10M covered cells load as
/// plain values (with a warning) and are never materialized.
#[test]
#[ignore = "materializes 10M array parts; release-only (run with --release -- --ignored)"]
fn engine_array_budget_release() {
    let mut cells = Vec::new();
    let mut blocks = Vec::new();
    for c in 0..12u32 {
        cells.push(formula_cell(0, c, "of:=ROW()"));
        blocks.push(tally_model::CellRangeLocal {
            r0: 0,
            c0: c,
            r1: 1_048_575,
            c1: c,
        });
    }
    let data = WorkbookData {
        sheets: vec![SheetData {
            name: "S".into(),
            cells,
            array_formulas: blocks,
            ..SheetData::default()
        }],
        ..WorkbookData::default()
    };
    let t = Instant::now();
    let wb = Workbook::from_data(&data);
    assert_fast("12 × 1M blocks", t, Duration::from_secs(20));
    let arrays = wb
        .formulas
        .iter()
        .filter(|(_, f)| f.array.is_some())
        .count();
    assert_eq!(arrays, 9, "9 × 1,048,576 ≤ 10M");
    assert!(wb
        .import_warnings
        .iter()
        .any(|w| w.contains("Array formulas")));
}

// ---------------------------------------------------------------------------
// File round trips (ODS and XLSX through the service)
// ---------------------------------------------------------------------------

#[cfg(feature = "io")]
mod files {
    use super::*;
    use std::io::Write as _;
    use std::path::{Path, PathBuf};
    use tally_api::{DocumentService, RangeDto};
    use tally_engine::EngineService;
    use tally_model::CellRangeLocal;

    fn temp_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let d = std::env::temp_dir().join(format!(
            "tally-security2-{tag}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn write_ods(path: &Path, data: &WorkbookData) {
        let mut cur = std::io::Cursor::new(Vec::new());
        tally_io_ods::write_ods(data, &mut cur).unwrap();
        std::fs::write(path, cur.into_inner()).unwrap();
    }

    /// HIGH-1: 200k merges (5.2 s before; 1M took 128 s) open fast; the
    /// first 100,000 are kept and the user is told.
    #[test]
    fn many_merges_open_fast() {
        let dir = temp_dir("merges");
        let p = dir.join("m.ods");
        let merges: Vec<CellRangeLocal> = (0..200_000u32)
            .map(|r| CellRangeLocal {
                r0: r,
                c0: 0,
                r1: r,
                c1: 1,
            })
            .collect();
        write_ods(
            &p,
            &WorkbookData {
                sheets: vec![SheetData {
                    name: "S".into(),
                    merges,
                    ..SheetData::default()
                }],
                ..WorkbookData::default()
            },
        );
        let mut svc = EngineService::with_default_codec();
        let t = Instant::now();
        let info = svc.open(&p).unwrap();
        assert_fast("200k merges open", t, Duration::from_secs(10));
        let wb = &svc.document(info.doc).unwrap().wb;
        assert_eq!(wb.sheets[0].meta.merges.len(), 100_000);
        assert!(
            info.warnings.iter().any(|w| w.contains("merged areas")),
            "{:?}",
            info.warnings
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// HIGH-4: a style with a 1 MB number format (`cells(A1:T1000)` took
    /// 41–85 s and 1–3 GB) opens as General; a 4,000-character format is
    /// kept but each cell's view text is capped at 1,024 characters, as is
    /// a long text value. `SetNumberFormat` refuses codes over 4,096.
    fn check_long_formats(limit: Duration) {
        use std::fmt::Write as _;
        let dir = temp_dir("numfmt");
        let p = dir.join("nf.xlsx");
        // The review's `p9_numfmt.py` shape: 1000 × 20 cells; odd columns
        // use a 4,000-character code, even ones a 1 MB code.
        let styles = format!(
            r#"<numFmts count="2"><numFmt numFmtId="200" formatCode="&quot;{}&quot;0"/><numFmt numFmtId="201" formatCode="&quot;{}&quot;0"/></numFmts><cellXfs count="3"><xf numFmtId="0"/><xf numFmtId="200" applyNumberFormat="1"/><xf numFmtId="201" applyNumberFormat="1"/></cellXfs>"#,
            "x".repeat(1 << 20),
            "y".repeat(4000)
        );
        let mut rows = String::new();
        for r in 1..=1000u32 {
            let _ = write!(rows, r#"<row r="{r}">"#);
            for c in 0..20u32 {
                let col = (b'A' + c as u8) as char;
                let _ = write!(
                    rows,
                    r#"<c r="{col}{r}" s="{}"><v>{}</v></c>"#,
                    1 + c % 2,
                    f64::from(r) * 1.5 + f64::from(c)
                );
            }
            rows.push_str("</row>");
        }
        let _ = write!(
            rows,
            r#"<row r="1001"><c r="A1001" t="inlineStr"><is><t>{}</t></is></c></row>"#,
            "z".repeat(100_000)
        );
        std::fs::write(&p, xlsx(&rows, &styles)).unwrap();
        let mut svc = EngineService::with_default_codec();
        let info = svc.open(&p).unwrap();
        let sheet = info.sheets[0].id;
        let t = Instant::now();
        let blocks = svc
            .cells(
                info.doc,
                sheet,
                &[RangeDto {
                    r0: 0,
                    c0: 0,
                    r1: 999,
                    c1: 19,
                }],
            )
            .unwrap();
        assert_fast("cells(A1:T1000)", t, limit);
        let cells = &blocks[0].cells;
        assert_eq!(cells[0].text, "1.5", "the 1 MB format became General");
        let long_text = &cells[1].text;
        assert_eq!(long_text.chars().count(), 1024);
        assert!(long_text.starts_with("yyy") && long_text.ends_with('…'));
        let tail = svc
            .cells(
                info.doc,
                sheet,
                &[RangeDto {
                    r0: 1000,
                    c0: 0,
                    r1: 1000,
                    c1: 0,
                }],
            )
            .unwrap();
        assert_eq!(tail[0].cells[0].text.chars().count(), 1024);
        let r = svc
            .apply(
                info.doc,
                common::tx(vec![Op::SetNumberFormat {
                    sheet,
                    ranges: vec![RangeDto {
                        r0: 0,
                        c0: 0,
                        r1: 0,
                        c1: 0,
                    }],
                    code: "0".repeat(5000),
                }]),
            )
            .unwrap();
        assert_eq!(r.errors.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn long_number_formats_render_fast() {
        check_long_formats(Duration::from_secs(3));
    }

    /// Release-only timing: a screen of 20,000 cells renders in < 100 ms.
    #[test]
    #[ignore = "release-only timing; run with --release -- --ignored"]
    fn long_number_formats_render_100ms_release() {
        check_long_formats(Duration::from_millis(100));
    }

    /// An XLSX package with one worksheet (and `styleSheet` content).
    fn xlsx(sheet_xml: &str, styles: &str) -> Vec<u8> {
        let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default();
        let ns = r#"xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;
        let entries = [
            (
                "xl/workbook.xml",
                format!(r#"<workbook {ns}><sheets><sheet name="S" sheetId="1" r:id="rId1"/></sheets></workbook>"#),
            ),
            (
                "xl/_rels/workbook.xml.rels",
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#.to_string(),
            ),
            ("xl/styles.xml", format!("<styleSheet {ns}>{styles}</styleSheet>")),
            (
                "xl/worksheets/sheet1.xml",
                format!("<worksheet {ns}><sheetData>{sheet_xml}</sheetData></worksheet>"),
            ),
        ];
        for (name, data) in entries {
            z.start_file(name, opts).unwrap();
            z.write_all(data.as_bytes()).unwrap();
        }
        z.finish().unwrap().into_inner()
    }

    /// MEDIUM-1 (`p6_shared.py`): 1M shared-formula members took 21.5 s and
    /// 4.4 GB. Members now share the anchor's parsed formula (one parse per
    /// group); relative references still move per cell.
    fn check_shared(n: u32, limit: Duration) {
        use std::fmt::Write as _;
        let f = vec!["A1"; 50].join("+");
        let mut rows = format!(
            r#"<row r="1"><c r="A1"><v>1</v></c><c r="B1"><f t="shared" ref="B1:B{n}" si="0">{f}</f><v>0</v></c></row>"#
        );
        for r in 2..=n {
            let _ = write!(
                rows,
                r#"<row r="{r}"><c r="A{r}"><v>{r}</v></c><c r="B{r}"><f t="shared" si="0"/><v>0</v></c></row>"#
            );
        }
        let dir = temp_dir("shared");
        let p = dir.join("s.xlsx");
        std::fs::write(&p, xlsx(&rows, "")).unwrap();
        let mut svc = EngineService::with_default_codec();
        let t = Instant::now();
        let info = svc.open(&p).unwrap();
        assert_fast("shared open", t, limit);
        let wb = &svc.document(info.doc).unwrap().wb;
        let s = info.sheets[0].id;
        assert_eq!(wb.value(CellAddr::new(s, 0, 1)), Value::Number(50.0));
        assert_eq!(
            wb.value(CellAddr::new(s, n - 1, 1)),
            Value::Number(50.0 * f64::from(n))
        );
        let (_, a) = wb.formula_at(CellAddr::new(s, 1, 1)).unwrap();
        let (_, b) = wb.formula_at(CellAddr::new(s, n - 1, 1)).unwrap();
        assert!(
            std::sync::Arc::ptr_eq(&a.formula, &b.formula),
            "one AST per group"
        );
        assert_eq!(wb.value(CellAddr::new(s, 1, 1)), Value::Number(100.0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn shared_formulas_parse_once() {
        check_shared(20_000, Duration::from_secs(10));
    }

    /// Release-only timing: 1M members open in < 10 s (21.5 s before).
    #[test]
    #[ignore = "release-only timing; run with --release -- --ignored"]
    fn shared_formulas_1m_release_timing() {
        check_shared(1_000_000, Duration::from_secs(10));
    }
}
