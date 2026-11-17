//! Row heights when writing: the height most rows have is the sheet's
//! default row height and those rows are not written one by one (a sheet
//! read from an Excel file has a height for every row), also when a few
//! rows have no run, and rows hidden to the end are a hidden default.

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
fn the_height_most_rows_have_is_the_default() {
    // Rows 0–2 have no run (rows inserted into a sheet read from Excel):
    // the height of the others is still the default, and the three are
    // written as rows of the application's 24 px (18 pt).
    let (xml, warnings) = sheet_xml(vec![run(3, MAX_ROWS - 1, 450, false)]);
    assert_eq!(warnings, Vec::<String>::new());
    assert!(xml.contains(r#"defaultRowHeight="12.75""#), "{xml}");
    assert_eq!(rows(&xml), 3, "{xml}");
    assert_eq!(xml.matches(r#"ht="18" customHeight="1""#).count(), 3);
    // The last row has none (a row was deleted): one `<row>`.
    let (xml, warnings) = sheet_xml(vec![run(0, MAX_ROWS - 2, 450, false)]);
    assert_eq!(warnings, Vec::<String>::new());
    assert!(xml.contains(r#"defaultRowHeight="12.75""#), "{xml}");
    assert_eq!(rows(&xml), 1, "{xml}");
    assert!(xml.contains(r#"<row r="1048576" ht="18" customHeight="1""#));
    // Most rows without a run: the application's default, and the rows
    // with a height one by one.
    let (xml, warnings) = sheet_xml(vec![run(3, 5, 450, false)]);
    assert_eq!(warnings, Vec::<String>::new());
    assert!(
        xml.contains(r#"defaultRowHeight="18" customHeight="1""#),
        "{xml}"
    );
    assert_eq!(rows(&xml), 3, "{xml}");
}

#[test]
fn rows_hidden_to_the_end_are_a_hidden_default() {
    let (xml, warnings) = sheet_xml(vec![
        run(0, 0, 450, false),
        run(1, 1, 1058, false),
        run(2, MAX_ROWS - 1, 450, true),
    ]);
    assert_eq!(warnings, Vec::<String>::new());
    assert!(xml.contains(r#"defaultRowHeight="12.75""#), "{xml}");
    assert!(xml.contains(r#"zeroHeight="1""#), "{xml}");
    // The two shown rows, and nothing for the million hidden ones.
    assert_eq!(rows(&xml), 2, "{xml}");
    assert!(!xml.contains("hidden="), "{xml}");
}

#[test]
fn more_sized_rows_than_the_cap_are_cut_with_a_warning() {
    // 150,000 rows of 30 pt in a sheet of automatic rows.
    let (xml, warnings) = sheet_xml(vec![run(0, 149_999, 1058, false)]);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("only the first 100000 row heights"));
    assert_eq!(rows(&xml), 100_000);
}
