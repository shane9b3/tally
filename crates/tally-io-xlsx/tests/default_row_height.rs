//! Row heights of a sheet whose runs cover every row (an imported `.xls`:
//! every row has the height of the file): the run that reaches the end of
//! the sheet is written as the sheet's default row height, not row by
//! row.

#![allow(clippy::unwrap_used)] // tests may unwrap (docs/contributing.md)

use std::io::{Cursor, Read};

use tally_io_xlsx::write_xlsx;
use tally_model::{DimRun, SheetData, WorkbookData};
use tally_types::MAX_ROWS;

fn run(start: u32, end: u32, size_hmm: u32, hidden: bool) -> DimRun {
    DimRun {
        start,
        end,
        size_hmm,
        hidden,
    }
}

fn sheet_xml(rows: Vec<DimRun>) -> (String, Vec<String>) {
    let wb = WorkbookData {
        sheets: vec![SheetData {
            name: "S".into(),
            row_heights: rows,
            ..SheetData::default()
        }],
        ..WorkbookData::default()
    };
    let mut buf = Cursor::new(Vec::new());
    let report = write_xlsx(&wb, &mut buf).unwrap();
    let mut z = zip::ZipArchive::new(Cursor::new(buf.into_inner())).unwrap();
    let mut s = String::new();
    z.by_name("xl/worksheets/sheet1.xml")
        .unwrap()
        .read_to_string(&mut s)
        .unwrap();
    (s, report.warnings)
}

fn rows(xml: &str) -> usize {
    xml.matches("<row ").count()
}

#[test]
fn full_coverage_writes_a_default_and_the_rows_that_differ() {
    // 12.75 pt (17 px) everywhere, three rows of their own, one hidden.
    let (xml, warnings) = sheet_xml(vec![
        run(0, 4, 450, false),
        run(5, 5, 1058, false),
        run(6, 99, 450, false),
        run(100, 101, 706, false),
        run(102, 102, 450, true),
        run(103, MAX_ROWS - 1, 450, false),
    ]);
    assert_eq!(warnings, Vec::<String>::new());
    assert!(xml.contains(r#"defaultRowHeight="12.75""#), "{xml}");
    assert_eq!(rows(&xml), 4, "{xml}");
    assert!(xml.contains(r#"<row r="6""#) && xml.contains(r#"<row r="103""#));
    // One run over the whole sheet, as before.
    let (xml, _) = sheet_xml(vec![run(0, MAX_ROWS - 1, 450, false)]);
    assert!(xml.contains(r#"defaultRowHeight="12.75""#), "{xml}");
    assert_eq!(rows(&xml), 0);
}

#[test]
fn rows_without_a_run_keep_the_application_default() {
    // Rows 0–2 have no run: the run that reaches the end is not the
    // sheet's default, and is cut at the writer's cap with a warning.
    let (xml, warnings) = sheet_xml(vec![run(3, MAX_ROWS - 1, 450, false)]);
    assert!(
        !xml.contains(r#"defaultRowHeight="12.75""#),
        "{}",
        &xml[..400]
    );
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    // Hidden to the end of the sheet: no default either.
    let (xml, _) = sheet_xml(vec![
        run(0, 9, 450, false),
        run(10, MAX_ROWS - 1, 450, true),
    ]);
    assert!(
        !xml.contains(r#"defaultRowHeight="12.75""#),
        "{}",
        &xml[..400]
    );
}
