//! The bulk-load fast paths give the same workbook as the plain per-cell
//! calls (perf work of 2026-10-27, docs/perf-baseline.md):
//!
//! - plain decimal numbers skip the input recognizer;
//! - a CSV load reads today's date once, caches recognized inputs and the
//!   styles derived from implied number formats;
//! - `Workbook::from_data_owned` frees the model while loading;
//! - equal neighbouring formulas in a column share one parsed AST;
//! - the cell maps are packed after a load.
#![allow(clippy::unwrap_used)]

use std::sync::Arc;

use proptest::prelude::*;
use tally_engine::recalc::recalc;
use tally_engine::Workbook;
use tally_model::{CellContentData, CellData, SheetData, StyleData, WorkbookData};
use tally_numfmt::{recognize_input_with, RecognizeContext, LOCALES};
use tally_types::{CellAddr, Value};

fn wb_with_locale(tag: &str) -> Workbook {
    let mut data = WorkbookData::default();
    data.sheets.push(SheetData {
        name: "Sheet1".into(),
        ..SheetData::default()
    });
    data.settings.locale = Some(tag.into());
    Workbook::from_data(&data)
}

/// The value and number format a typed input gets, compared with what the
/// recognizer says it should get (bitwise for numbers, so `-0` counts).
fn check_input(tag: &str, input: &str) {
    let mut wb = wb_with_locale(tag);
    let sheet = wb.sheets[0].id;
    let addr = CellAddr::new(sheet, 0, 0);
    wb.set_constant_input(addr, input);
    let locale = wb.locale;
    let rec = recognize_input_with(
        input,
        locale,
        &RecognizeContext::now(wb.settings.date_epoch),
    );
    let got = wb.value(addr);
    match (&got, &rec.value) {
        (Value::Number(a), Value::Number(b)) => {
            assert_eq!(a.to_bits(), b.to_bits(), "{tag} {input:?}: {a} vs {b}");
        }
        (a, b) => assert_eq!(a, b, "{tag} {input:?}"),
    }
    let code = wb.styles.get(wb.style_at(addr)).format_code.clone();
    let want = rec.implied_format.as_deref().unwrap_or("General");
    assert_eq!(&*code, want, "{tag} {input:?}");
}

#[test]
fn plain_numbers_match_the_recognizer() {
    let inputs = [
        "0",
        "-0",
        "7",
        "007",
        "-007",
        "12",
        "-12",
        "1.5",
        "-1.5",
        "0.25",
        ".5",
        "-.5",
        "5.",
        "1.50",
        "123456789012345",
        "1234567890123456",
        "99999999999999999999",
        "1,5",
        "1,234",
        "1.234",
        "1.234,5",
        "1 234",
        "+5",
        "--5",
        "-",
        ".",
        "",
        " 5",
        "5 ",
        "1e5",
        "1E-3",
        "5%",
        "$5",
        "5€",
        "(5)",
        "1/2",
        "1-2",
        "1.2.3",
        "12:30",
        "2024-01-05",
        "TRUE",
        "wahr",
        "１２",
        "٣",
        "0x10",
        "1_000",
    ];
    for l in LOCALES {
        for i in inputs {
            check_input(l.tag, i);
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(400))]
    #[test]
    fn numeric_looking_inputs_match_the_recognizer(s in "[-+]?[0-9]{0,18}([.,][0-9]{0,18})?[.,%e]?") {
        for l in LOCALES {
            check_input(l.tag, &s);
        }
    }
}

fn text_cell(row: u32, col: u32, s: &str, style: Option<u32>) -> CellData {
    CellData {
        row,
        col,
        content: CellContentData::Value(Value::text(s)),
        style,
    }
}

/// Every kind of input the recognizer handles, repeated so that the
/// per-load caches are hit, on a General column and on columns styled
/// as percent and as text.
fn csv_like(tag: &str) -> WorkbookData {
    let inputs = [
        "12",
        "-3.25",
        "007",
        "1,234",
        "1.234,5",
        "5%",
        "12,5 %",
        "$4.50",
        "4,50 €",
        "TRUE",
        "false",
        "WAHR",
        "VRAI",
        "2024-01-05",
        "1/2/2024",
        "05.01.2024",
        "5/1",
        "Jan 5",
        "12:30",
        "12:30:15",
        "1:30 PM",
        "2024-01-05 12:30",
        "1 1/2",
        "1e3",
        "'0123",
        "=1+2",
        "=SUM(A1:A3)",
        " 12 ",
        "abc",
        "",
        "١٢",
        "-0",
        ".5",
        "5.",
    ];
    let mut cells = Vec::new();
    let mut row = 0;
    for _ in 0..3 {
        for i in inputs {
            if !i.is_empty() {
                cells.push(text_cell(row, 0, i, None));
                cells.push(text_cell(row, 1, i, Some(1)));
                cells.push(text_cell(row, 2, i, Some(2)));
            }
            row += 1;
        }
    }
    let mut data = WorkbookData {
        styles: vec![
            StyleData::default(),
            StyleData {
                number_format: Some("0.00%".into()),
                ..StyleData::default()
            },
            StyleData {
                number_format: Some("@".into()),
                ..StyleData::default()
            },
        ],
        sheets: vec![SheetData {
            name: "Sheet1".into(),
            cells,
            ..SheetData::default()
        }],
        ..WorkbookData::default()
    };
    data.settings.locale = Some(tag.into());
    data
}

#[test]
fn bulk_text_load_equals_per_cell_input() {
    for l in LOCALES {
        for formulas in [false, true] {
            let data = csv_like(l.tag);
            let mut bulk = Workbook::from_data_opts(&data, true, formulas);
            recalc(&mut bulk);
            let mut owned = Workbook::from_data_owned(data.clone(), true, formulas);
            recalc(&mut owned);
            // The same workbook, filled one cell at a time through the
            // public input calls.
            let mut empty = data.clone();
            let cells = std::mem::take(&mut empty.sheets[0].cells);
            let mut by_cell = Workbook::from_data_opts(&empty, true, formulas);
            let sheet = by_cell.sheets[0].id;
            let ids: Vec<_> = empty
                .styles
                .iter()
                .map(|s| by_cell.styles.intern(s.clone()))
                .collect();
            for c in &cells {
                let addr = CellAddr::new(sheet, c.row, c.col);
                if let Some(st) = c.style {
                    let col = by_cell.sheets[0].column_mut(c.col).unwrap();
                    col.styles.set(c.row, c.row, ids[st as usize]);
                }
                let CellContentData::Value(Value::Text(t)) = &c.content else {
                    unreachable!()
                };
                if formulas {
                    by_cell.set_input(addr, t);
                } else {
                    by_cell.set_constant_input(addr, t);
                }
            }
            recalc(&mut by_cell);
            let want = by_cell.to_data();
            assert_eq!(bulk.to_data(), want, "{} formulas={formulas}", l.tag);
            assert_eq!(owned.to_data(), want, "{} formulas={formulas}", l.tag);
        }
    }
}

fn formula(row: u32, col: u32, text: &str) -> CellData {
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

#[test]
fn fill_down_formulas_share_one_ast() {
    let mut cells = Vec::new();
    for r in 0..50u32 {
        let n = r + 1;
        cells.push(CellData {
            row: r,
            col: 0,
            content: CellContentData::Value(Value::Number(f64::from(r))),
            style: None,
        });
        cells.push(formula(r, 1, &format!("of:=[.A{n}]*2")));
        // Absolute reference: the AST differs per row only by the row.
        cells.push(formula(r, 2, &format!("of:=[.$A${n}]+1")));
    }
    // A different formula in the middle of column B breaks the run.
    cells[3 * 25 + 1] = formula(25, 1, "of:=[.A26]*3");
    let mut data = WorkbookData::default();
    data.sheets.push(SheetData {
        name: "Sheet1".into(),
        cells,
        ..SheetData::default()
    });
    let mut wb = Workbook::from_data_owned(data.clone(), false, false);
    recalc(&mut wb);
    let s = wb.sheets[0].id;
    let f = |r: u32, c: u32| Arc::clone(&wb.formula_at(CellAddr::new(s, r, c)).unwrap().1.formula);
    assert!(Arc::ptr_eq(&f(0, 1), &f(24, 1)));
    assert!(!Arc::ptr_eq(&f(24, 1), &f(25, 1)));
    assert!(Arc::ptr_eq(&f(26, 1), &f(49, 1)));
    assert!(!Arc::ptr_eq(&f(0, 2), &f(1, 2)), "$A$1 and $A$2 differ");
    for r in 0..50u32 {
        let k = if r == 25 { 3.0 } else { 2.0 };
        assert_eq!(
            wb.value(CellAddr::new(s, r, 1)),
            Value::Number(f64::from(r) * k)
        );
        assert_eq!(
            wb.value(CellAddr::new(s, r, 2)),
            Value::Number(f64::from(r) + 1.0)
        );
    }
    // Same result as the borrowing loader.
    let mut by_ref = Workbook::from_data(&data);
    recalc(&mut by_ref);
    assert_eq!(wb.to_data(), by_ref.to_data());
}

#[cfg(feature = "io")]
#[test]
fn owned_and_borrowed_loads_agree_on_fixtures() {
    use tally_engine::FileCodec;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
    let mut n = 0;
    for dir in ["ods", "xlsx", "csv", "xls", "ods/charts", "xlsx/charts"] {
        let Ok(entries) = std::fs::read_dir(root.join(dir)) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if !p.is_file() {
                continue;
            }
            let ext = p
                .extension()
                .map(|e| e.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default();
            if ext == "py" {
                continue;
            }
            let bytes = std::fs::read(&p).unwrap();
            let Ok(d) = tally_engine::IoCodec.read(&bytes, &ext) else {
                continue;
            };
            let mut a = Workbook::from_data_opts(&d.data, d.recognize_text, d.evaluate_formulas);
            recalc(&mut a);
            let mut b = Workbook::from_data_owned(d.data, d.recognize_text, d.evaluate_formulas);
            recalc(&mut b);
            assert_eq!(a.to_data(), b.to_data(), "{}", p.display());
            n += 1;
        }
    }
    assert!(n >= 15, "only {n} fixtures read");
}
