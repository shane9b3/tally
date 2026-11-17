//! The default row height and column width of a worksheet
//! (`sheetFormatPr`), read and written.
//!
//! Read: every row and column of an Excel file has the size the file
//! gives it, its own or the sheet's default, as runs over the whole
//! sheet. Written: a sheet's default is the size of the run that reaches
//! the end when the runs cover the sheet, else the application's default
//! (a row or column without a run), which reads back as "no run". The
//! inputs are generated here.

#![allow(clippy::unwrap_used)] // tests may unwrap (docs/contributing.md)

mod common;

use std::io::{Cursor, Read, Write};
use std::sync::Arc;
use std::time::Instant;

use common::{round_trip, to_bytes};
use tally_io_xlsx::{read_xlsx, Limits, XlsxError, DEFAULT_COL_WIDTH_HMM, DEFAULT_ROW_HEIGHT_HMM};
use tally_model::{
    CellContentData, CellData, ChartAnchorData, DimRun, ImageData, OutlineGroupData, SheetData,
    StyleData, StyleRun, WorkbookData, APP_DEFAULT_COL_WIDTH_HMM, APP_DEFAULT_ROW_HEIGHT_HMM,
};
use tally_types::{Value, MAX_COLS, MAX_ROWS};

const NS: &str = r#"xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;
const RELS_NS: &str = r#"xmlns="http://schemas.openxmlformats.org/package/2006/relationships""#;

/// A one-sheet package without a styles part (the default font is
/// Calibri 11: digits 7 px wide).
fn package(sheet_xml: &str) -> Vec<u8> {
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default();
    let entries = [
        (
            "xl/workbook.xml",
            format!(
                r#"<workbook {NS}><sheets><sheet name="S" sheetId="1" r:id="rId1"/></sheets></workbook>"#
            ),
        ),
        (
            "xl/_rels/workbook.xml.rels",
            format!(
                r#"<Relationships {RELS_NS}><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#
            ),
        ),
        ("xl/worksheets/sheet1.xml", sheet_xml.to_string()),
    ];
    for (name, data) in entries {
        z.start_file(name, opts).unwrap();
        z.write_all(data.as_bytes()).unwrap();
    }
    z.finish().unwrap().into_inner()
}

/// A worksheet: `format` is the `sheetFormatPr` element (or nothing),
/// `cols` the `<col>` elements, `rows` the `<row>` elements.
fn ws(format: &str, cols: &str, rows: &str) -> String {
    let cols = if cols.is_empty() {
        String::new()
    } else {
        format!("<cols>{cols}</cols>")
    };
    format!("<worksheet {NS}>{format}{cols}<sheetData>{rows}</sheetData></worksheet>")
}

fn read(sheet_xml: &str) -> SheetData {
    read_xlsx(Cursor::new(package(sheet_xml)), &Limits::default())
        .unwrap()
        .sheets
        .remove(0)
}

fn run(start: u32, end: u32, size_hmm: u32, hidden: bool) -> DimRun {
    DimRun {
        start,
        end,
        size_hmm,
        hidden,
    }
}

const LAST_ROW: u32 = MAX_ROWS - 1;
const LAST_COL: u32 = MAX_COLS - 1;

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

#[test]
fn an_excel_sheet_has_excels_defaults_everywhere() {
    // What Excel writes for an untouched sheet: 15 pt rows, no `<cols>`.
    let s = read(&ws(
        r#"<sheetFormatPr defaultRowHeight="15"/>"#,
        "",
        r#"<row r="1"><c r="A1"><v>1</v></c></row><row r="3" ht="15"><c r="A3"><v>2</v></c></row>"#,
    ));
    assert_eq!((DEFAULT_ROW_HEIGHT_HMM, DEFAULT_COL_WIDTH_HMM), (529, 1693));
    // 15 pt = 20 px and 64 px, as one run each.
    assert_eq!(s.row_heights, [run(0, LAST_ROW, 529, false)]);
    assert_eq!(s.col_widths, [run(0, LAST_COL, 1693, false)]);
    // No `sheetFormatPr` at all: the same.
    let s = read(&ws("", "", r#"<row r="5"><c r="A5"><v>1</v></c></row>"#));
    assert_eq!(s.row_heights, [run(0, LAST_ROW, 529, false)]);
    assert_eq!(s.col_widths, [run(0, LAST_COL, 1693, false)]);
    // Not even a row.
    let s = read(&ws("", "", ""));
    assert_eq!(s.row_heights, [run(0, LAST_ROW, 529, false)]);
}

#[test]
fn rows_take_their_height_or_the_sheets_default() {
    let s = read(&ws(
        r#"<sheetFormatPr defaultRowHeight="12.75"/>"#,
        "",
        concat!(
            // Set by the user.
            r#"<row r="2" ht="30" customHeight="1"/>"#,
            // Fitted by Excel: the stored height counts all the same.
            r#"<row r="3" ht="25.5"/>"#,
            // No height: the default.
            r#"<row r="4"/>"#,
            // Hidden, with and without a height.
            r#"<row r="6" ht="30" hidden="1"/>"#,
            r#"<row r="7" hidden="1"/>"#,
            // A height of 0 is a hidden row of the default height.
            r#"<row r="8" ht="0" customHeight="1"/>"#,
            // Without `r`: the next row.
            r#"<row ht="30"/>"#,
        ),
    ));
    assert_eq!(
        s.row_heights,
        [
            run(0, 0, 450, false),
            run(1, 1, 1058, false),
            run(2, 2, 900, false),
            run(3, 4, 450, false),
            run(5, 5, 1058, true),
            run(6, 7, 450, true),
            run(8, 8, 1058, false),
            run(9, LAST_ROW, 450, false),
        ]
    );
}

#[test]
fn a_hidden_default_hides_the_rows_without_an_element() {
    for format in [
        r#"<sheetFormatPr defaultRowHeight="15" zeroHeight="1"/>"#,
        // A default height of 0 is the same thing.
        r#"<sheetFormatPr defaultRowHeight="0"/>"#,
    ] {
        let s = read(&ws(
            format,
            "",
            r#"<row r="2"><c r="A2"><v>1</v></c></row><row r="3" ht="30"/><row r="4" hidden="1"/>"#,
        ));
        assert_eq!(
            s.row_heights,
            [
                run(0, 0, 529, true),
                run(1, 1, 529, false),
                run(2, 2, 1058, false),
                run(3, LAST_ROW, 529, true),
            ],
            "{format}"
        );
    }
}

#[test]
fn the_default_column_width_is_the_files_or_derived_from_the_base_width() {
    let width = |format: &str| {
        let s = read(&ws(format, "", ""));
        assert_eq!(s.col_widths.len(), 1, "{format}");
        assert_eq!((s.col_widths[0].start, s.col_widths[0].end), (0, LAST_COL));
        assert!(!s.col_widths[0].hidden);
        s.col_widths[0].size_hmm
    };
    // Nothing: 8 digits of 7 px + 5 px = 61 px, rounded up to a multiple
    // of 8: Excel's 64 px.
    assert_eq!(width(r#"<sheetFormatPr defaultRowHeight="15"/>"#), 1693);
    // `baseColWidth`: 10 · 7 + 5 = 75 → 80 px; 1 · 7 + 5 = 12 → 16 px.
    assert_eq!(width(r#"<sheetFormatPr baseColWidth="10"/>"#), 2117);
    assert_eq!(width(r#"<sheetFormatPr baseColWidth="1"/>"#), 423);
    // `defaultColWidth` is in the unit of `<col width>` and wins:
    // 12.7109375 characters = 89 px.
    assert_eq!(
        width(r#"<sheetFormatPr baseColWidth="10" defaultColWidth="12.7109375"/>"#),
        2355
    );
}

#[test]
fn columns_take_their_width_or_the_sheets_default() {
    let s = read(&ws(
        r#"<sheetFormatPr defaultColWidth="12.7109375" defaultRowHeight="15"/>"#,
        concat!(
            // 20.7109375 characters = 145 px.
            r#"<col min="2" max="3" width="20.7109375" customWidth="1"/>"#,
            // A width without `customWidth` counts too (64 px).
            r#"<col min="5" max="5" width="9.140625"/>"#,
            // Hidden with its width, and hidden through a width of 0.
            r#"<col min="7" max="7" width="20.7109375" hidden="1" customWidth="1"/>"#,
            r#"<col min="8" max="8" width="0" hidden="1" customWidth="1"/>"#,
            r#"<col min="9" max="9" width="0"/>"#,
            // Only a style: the default width.
            r#"<col min="11" max="11" style="0"/>"#,
            // Of two entries for a column the last one counts.
            r#"<col min="2" max="2" width="9.140625" customWidth="1"/>"#,
        ),
        "",
    ));
    assert_eq!(
        s.col_widths,
        [
            run(0, 0, 2355, false),
            run(1, 1, 1693, false),
            run(2, 2, 3836, false),
            run(3, 3, 2355, false),
            run(4, 4, 1693, false),
            run(5, 5, 2355, false),
            run(6, 6, 3836, true),
            run(7, 8, 2355, true),
            run(9, LAST_COL, 2355, false),
        ]
    );
}

#[test]
fn hostile_sizes_are_ignored() {
    for bad in [
        "NaN", "-5", "1e308", "1e-320", "inf", "", "x", "99999", "-0",
    ] {
        let xml = ws(
            &format!(
                r#"<sheetFormatPr defaultRowHeight="{bad}" defaultColWidth="{bad}" baseColWidth="{bad}"/>"#
            ),
            &format!(r#"<col min="2" max="2" width="{bad}" customWidth="1"/>"#),
            &format!(r#"<row r="2" ht="{bad}" customHeight="1"/>"#),
        );
        let s = read(&xml);
        // Every size stays a real one: Excel's defaults, or (a value that
        // rounds to 0) hidden rows and columns of the default size.
        assert!(!s.row_heights.is_empty() && !s.col_widths.is_empty());
        for r in s.row_heights.iter().chain(&s.col_widths) {
            assert!(r.start <= r.end, "{bad}: {r:?}");
            assert!(
                r.size_hmm == 529 || r.size_hmm == 1693,
                "{bad}: {r:?} is not a default size"
            );
        }
        assert_eq!(s.row_heights.last().unwrap().end, LAST_ROW, "{bad}");
        assert_eq!(s.col_widths.last().unwrap().end, LAST_COL, "{bad}");
        assert!(s.row_heights.len() <= 3 && s.col_widths.len() <= 3, "{bad}");
    }
    // Rows and columns outside the sheet.
    let s = read(&ws(
        "",
        r#"<col min="0" max="5" width="30"/><col min="16385" max="16390" width="30"/><col min="9" max="2" width="30"/><col min="16384" max="4294967295" width="30"/>"#,
        r#"<row r="0" ht="30"/><row r="1048577" ht="30"/><row r="4294967295" ht="30"/>"#,
    ));
    assert_eq!(s.row_heights, [run(0, LAST_ROW, 529, false)]);
    assert_eq!(
        s.col_widths,
        [
            run(0, LAST_COL - 1, 1693, false),
            // 30 characters = 210 px.
            run(LAST_COL, LAST_COL, 5556, false)
        ]
    );
}

#[test]
fn rows_out_of_order_and_given_twice_resolve_to_the_last() {
    let s = read(&ws(
        r#"<sheetFormatPr defaultRowHeight="15"/>"#,
        "",
        concat!(
            r#"<row r="5" ht="30"/>"#,
            r#"<row r="2" ht="45"/>"#,
            r#"<row r="5" ht="60"/>"#,
            r#"<row r="3" ht="45"/>"#,
            r#"<row r="2" ht="30"/>"#,
        ),
    ));
    assert_eq!(
        s.row_heights,
        [
            run(0, 0, 529, false),
            run(1, 1, 1058, false),
            run(2, 2, 1588, false),
            run(3, 3, 529, false),
            run(4, 4, 2117, false),
            run(5, LAST_ROW, 529, false),
        ]
    );
}

#[test]
fn a_million_rows_are_runs_not_rows() {
    // Every row of the sheet with the same fitted height, and the first
    // 200,000 of them again.
    let mut rows = String::with_capacity(32 * 1024 * 1024);
    for last in [MAX_ROWS, 200_000] {
        for r in 1..=last {
            rows.push_str(r#"<row r=""#);
            rows.push_str(&r.to_string());
            rows.push_str(r#"" ht="12.75"/>"#);
        }
    }
    let bytes = package(&ws(r#"<sheetFormatPr defaultRowHeight="15"/>"#, "", &rows));
    let t = Instant::now();
    let s = read_xlsx(Cursor::new(bytes), &Limits::default())
        .unwrap()
        .sheets
        .remove(0);
    assert_eq!(s.row_heights, [run(0, LAST_ROW, 450, false)]);
    assert!(t.elapsed().as_secs() < 30, "took {:?}", t.elapsed());
}

/// Security review of 2026-11-17, M1: rows given again and again made the
/// reader's memory grow with the file. The reader now keeps one entry per
/// row of the sheet from the first row that is out of order (unit test
/// `sheet::tests::repeated_rows_are_bounded_by_the_sheet` checks what it
/// holds); here, that the last `<row>` of each row counts.
#[test]
fn millions_of_repeated_rows_resolve_to_the_last() {
    let mut rows = String::with_capacity(64 * 1024 * 1024);
    // Rows of alternating heights (no two merge), then two rows three
    // million times, then a descending tail.
    for r in 1..=200_000u32 {
        let ht = if r % 2 == 0 { 15 } else { 30 };
        rows.push_str(&format!(r#"<row r="{r}" ht="{ht}"/>"#));
    }
    for i in 0..3_000_000u32 {
        rows.push_str(if i % 2 == 0 {
            r#"<row r="1" ht="45"/>"#
        } else {
            r#"<row r="2" hidden="1"/>"#
        });
    }
    for r in (5..=8u32).rev() {
        rows.push_str(&format!(r#"<row r="{r}" ht="60"/>"#));
    }
    let bytes = package(&ws(r#"<sheetFormatPr defaultRowHeight="15"/>"#, "", &rows));
    drop(rows);
    let t = Instant::now();
    let s = read_xlsx(Cursor::new(bytes), &Limits::default())
        .unwrap()
        .sheets
        .remove(0);
    assert!(t.elapsed().as_secs() < 30, "took {:?}", t.elapsed());
    assert_eq!(
        s.row_heights[..6],
        [
            run(0, 0, 1588, false),
            run(1, 1, 529, true),
            run(2, 2, 1058, false),
            run(3, 3, 529, false),
            run(4, 7, 2117, false),
            run(8, 8, 1058, false),
        ]
    );
    // Rows 10 … 200,000 alternate; the rest of the sheet is one run that
    // starts with row 200,000 (15 pt, like the default).
    assert_eq!(s.row_heights.len(), 6 + (200_000 - 9));
    assert_eq!(
        s.row_heights.last(),
        Some(&run(199_999, LAST_ROW, 529, false))
    );
}

/// A size that is not 0 but rounds to nothing is not "hidden": it is
/// ignored, like any other size that cannot be used.
#[test]
fn only_an_explicit_zero_hides() {
    let s = read(&ws(
        r#"<sheetFormatPr defaultRowHeight="0.0000001"/>"#,
        r#"<col min="2" max="2" width="0.0000001" customWidth="1"/><col min="4" max="4" width="0.0"/>"#,
        r#"<row r="2" ht="0.0000001" customHeight="1"/><row r="4" ht="0.00"/>"#,
    ));
    assert_eq!(
        s.row_heights,
        [
            run(0, 2, 529, false),
            run(3, 3, 529, true),
            run(4, LAST_ROW, 529, false)
        ]
    );
    assert_eq!(
        s.col_widths,
        [
            run(0, 2, 1693, false),
            run(3, 3, 1693, true),
            run(4, LAST_COL, 1693, false)
        ]
    );
}

#[test]
fn size_runs_count_against_the_cell_limit() {
    // 20,000 rows of alternating heights are 20,000 runs.
    let mut rows = String::new();
    for r in 1..=20_000u32 {
        let ht = if r % 2 == 0 { 30 } else { 45 };
        rows.push_str(&format!(r#"<row r="{r}" ht="{ht}"/>"#));
    }
    let bytes = package(&ws("", "", &rows));
    let ok = read_xlsx(Cursor::new(bytes.clone()), &Limits::default()).unwrap();
    assert_eq!(ok.sheets[0].row_heights.len(), 20_001);
    let limits = Limits {
        max_cells: 10_000,
        ..Limits::default()
    };
    assert!(matches!(
        read_xlsx(Cursor::new(bytes), &limits),
        Err(XlsxError::LimitExceeded { .. })
    ));
}

#[test]
fn the_applications_own_default_stays_no_entry() {
    assert_eq!(
        (APP_DEFAULT_ROW_HEIGHT_HMM, APP_DEFAULT_COL_WIDTH_HMM),
        (635, 2646)
    );
    // 18 pt = 24 px rows and 14.28515625 characters = 100 px columns: what
    // Tally shows for a row or column without an entry.
    let s = read(&ws(
        r#"<sheetFormatPr defaultColWidth="14.28515625" defaultRowHeight="18" customHeight="1"/>"#,
        concat!(
            r#"<col min="2" max="2" width="14.28515625" customWidth="1"/>"#,
            r#"<col min="4" max="4" width="20.7109375" customWidth="1"/>"#,
            r#"<col min="6" max="6" hidden="1"/>"#,
        ),
        concat!(
            r#"<row r="1"><c r="A1"><v>1</v></c></row>"#,
            r#"<row r="2" ht="18" customHeight="1"/>"#,
            r#"<row r="3" ht="30" customHeight="1"/>"#,
            r#"<row r="5" hidden="1"/>"#,
        ),
    ));
    assert_eq!(
        s.row_heights,
        [run(2, 2, 1058, false), run(4, 4, 635, true)]
    );
    assert_eq!(
        s.col_widths,
        [run(3, 3, 3836, false), run(5, 5, 2646, true)]
    );
}

// ---------------------------------------------------------------------------
// Writing, and reading it back
// ---------------------------------------------------------------------------

fn cell(row: u32, col: u32, v: f64) -> CellData {
    CellData {
        row,
        col,
        content: CellContentData::Value(Value::Number(v)),
        style: None,
    }
}

fn book(sheet: SheetData) -> WorkbookData {
    WorkbookData {
        sheets: vec![sheet],
        ..WorkbookData::default()
    }
}

fn sheet_xml(bytes: &[u8]) -> String {
    let mut z = zip::ZipArchive::new(Cursor::new(bytes.to_vec())).unwrap();
    let mut s = String::new();
    z.by_name("xl/worksheets/sheet1.xml")
        .unwrap()
        .read_to_string(&mut s)
        .unwrap();
    s
}

fn element<'a>(xml: &'a str, name: &str) -> &'a str {
    let at = xml
        .find(&format!("<{name} "))
        .unwrap_or_else(|| panic!("no <{name}> in {xml}"));
    &xml[at..at + xml[at..].find('>').unwrap() + 1]
}

#[test]
fn a_sheet_made_in_tally_says_tallys_defaults_and_reads_back_without_sizes() {
    for font in [None, Some("Arial"), Some("Courier New")] {
        let mut wb = book(SheetData {
            name: "S".into(),
            cells: vec![cell(0, 0, 1.0), cell(4, 2, 2.0)],
            ..SheetData::default()
        });
        wb.settings.default_font = font.map(str::to_string);
        let bytes = to_bytes(&wb);
        let xml = sheet_xml(&bytes);
        let format = element(&xml, "sheetFormatPr");
        // 24 px rows; 100 px columns in digits of the default font.
        assert!(format.contains(r#"defaultRowHeight="18""#), "{format}");
        assert!(format.contains(r#"customHeight="1""#), "{format}");
        let chars = match font {
            None => "14.28515625",
            Some("Arial") => "12.5",
            _ => "11.109375",
        };
        assert!(
            format.contains(&format!(r#"defaultColWidth="{chars}""#)),
            "{font:?}: {format}"
        );
        // The rows carry no height (Excel fits them to their content) and
        // no column is written.
        assert_eq!(xml.matches("<row ").count(), 2, "{xml}");
        assert!(!xml.contains(" ht="), "{xml}");
        assert!(!xml.contains("<cols>"), "{xml}");

        let back = read_xlsx(Cursor::new(bytes), &Limits::default()).unwrap();
        assert!(back.sheets[0].row_heights.is_empty(), "{font:?}");
        assert!(back.sheets[0].col_widths.is_empty(), "{font:?}");
        assert_eq!(back.sheets[0].cells.len(), 2);
    }
    // A workbook without a sheet of its own gets one, read back the same.
    let back = round_trip(&WorkbookData::default());
    assert!(back.sheets[0].row_heights.is_empty() && back.sheets[0].col_widths.is_empty());
}

#[test]
fn sizes_set_in_tally_round_trip_next_to_the_defaults() {
    let bold = StyleData {
        font: tally_model::FontData {
            bold: true,
            ..Default::default()
        },
        ..StyleData::default()
    };
    let sheet = SheetData {
        name: "S".into(),
        cells: vec![cell(0, 0, 1.0), cell(1, 0, 2.0), cell(6, 0, 3.0)],
        col_widths: vec![
            run(1, 1, 3810, false), // 144 px
            run(3, 3, 1693, false), // 64 px: Excel's default, not Tally's
            run(5, 6, 2646, true),  // hidden, of the default width
            run(8, 8, 3810, true),
            run(15, 15, 1693, true), // hidden at 64 px
        ],
        row_heights: vec![
            run(1, 1, 1058, false), // 40 px
            run(3, 3, 529, false),  // 20 px: Excel's default, not Tally's
            run(5, 5, 635, true),   // hidden, of the default height
            run(8, 9, 1058, true),
        ],
        // A whole-column format and a column group make `<col>` elements
        // of their own: they must not change the width.
        style_runs: vec![StyleRun {
            col: 10,
            start_row: 0,
            end_row: LAST_ROW,
            style: 0,
        }],
        col_groups: vec![OutlineGroupData {
            start: 12,
            end: 13,
            level: 1,
            collapsed: false,
        }],
        row_groups: vec![OutlineGroupData {
            start: 11,
            end: 12,
            level: 1,
            collapsed: false,
        }],
        ..SheetData::default()
    };
    let mut wb = book(sheet);
    wb.styles = vec![bold];
    let back = round_trip(&wb);
    assert_eq!(back.sheets[0].col_widths, wb.sheets[0].col_widths);
    assert_eq!(back.sheets[0].row_heights, wb.sheets[0].row_heights);
    assert_eq!(back.sheets[0].style_runs.len(), 1);
    assert_eq!(back.sheets[0].col_groups, wb.sheets[0].col_groups);
    assert_eq!(back.sheets[0].row_groups, wb.sheets[0].row_groups);
    // And again: nothing moves.
    let again = round_trip(&back);
    assert_eq!(again.sheets[0].col_widths, wb.sheets[0].col_widths);
    assert_eq!(again.sheets[0].row_heights, wb.sheets[0].row_heights);

    // The styled column and the grouped ones are written at 100 px.
    let xml = sheet_xml(&to_bytes(&wb));
    let cols = &xml[xml.find("<cols>").unwrap()..xml.find("</cols>").unwrap()];
    for min in [11, 13] {
        let at = cols.find(&format!(r#"<col min="{min}""#)).unwrap();
        let col = &cols[at..at + cols[at..].find("/>").unwrap()];
        assert!(col.contains(r#"width="14.28515625""#), "{col}");
    }
}

#[test]
fn an_excel_sheet_round_trips_through_tally() {
    let source = ws(
        r#"<sheetFormatPr defaultRowHeight="12.75"/>"#,
        concat!(
            r#"<col min="2" max="3" width="20.7109375" customWidth="1"/>"#,
            r#"<col min="5" max="5" width="0" hidden="1" customWidth="1"/>"#,
        ),
        concat!(
            r#"<row r="1"><c r="A1"><v>1</v></c></row>"#,
            r#"<row r="2" ht="30" customHeight="1"><c r="A2"><v>2</v></c></row>"#,
            r#"<row r="3" ht="25.5"/>"#,
            r#"<row r="5" hidden="1"/>"#,
        ),
    );
    let first = read_xlsx(Cursor::new(package(&source)), &Limits::default()).unwrap();
    let s = &first.sheets[0];
    assert_eq!(
        s.row_heights,
        [
            run(0, 0, 450, false),
            run(1, 1, 1058, false),
            run(2, 2, 900, false),
            run(3, 3, 450, false),
            run(4, 4, 450, true),
            run(5, LAST_ROW, 450, false),
        ]
    );
    assert_eq!(
        s.col_widths,
        [
            run(0, 0, 1693, false),
            run(1, 2, 3836, false),
            run(3, 3, 1693, false),
            run(4, 4, 1693, true),
            run(5, LAST_COL, 1693, false),
        ]
    );
    // Written: the file's defaults again, and only what differs.
    let bytes = to_bytes(&first);
    let xml = sheet_xml(&bytes);
    let format = element(&xml, "sheetFormatPr");
    assert!(format.contains(r#"defaultRowHeight="12.75""#), "{format}");
    assert!(!format.contains("defaultColWidth"), "{format}");
    assert_eq!(xml.matches("<col ").count(), 2, "{xml}");
    assert_eq!(xml.matches("<row ").count(), 4, "{xml}");
    // Read again, and once more: the same sizes.
    let second = read_xlsx(Cursor::new(bytes), &Limits::default()).unwrap();
    assert_eq!(second.sheets[0].row_heights, s.row_heights);
    assert_eq!(second.sheets[0].col_widths, s.col_widths);
    let third = round_trip(&second);
    assert_eq!(third.sheets[0].row_heights, s.row_heights);
    assert_eq!(third.sheets[0].col_widths, s.col_widths);

    // The plain Excel sheet (15 pt, 64 px) writes no size at all.
    let plain = read_xlsx(
        Cursor::new(package(&ws(
            r#"<sheetFormatPr defaultRowHeight="15"/>"#,
            "",
            r#"<row r="1"><c r="A1"><v>1</v></c></row>"#,
        ))),
        &Limits::default(),
    )
    .unwrap();
    let xml = sheet_xml(&to_bytes(&plain));
    assert_eq!(
        element(&xml, "sheetFormatPr"),
        r#"<sheetFormatPr defaultRowHeight="15"/>"#
    );
    assert!(!xml.contains("<cols>") && !xml.contains(" ht="), "{xml}");
    let back = round_trip(&plain);
    assert_eq!(back.sheets[0].row_heights, [run(0, LAST_ROW, 529, false)]);
    assert_eq!(back.sheets[0].col_widths, [run(0, LAST_COL, 1693, false)]);
}

#[test]
fn rows_hidden_to_the_end_of_the_sheet_round_trip() {
    // Two shown rows, then everything hidden (Excel's "hide the unused
    // rows"), with a cell and a formatted blank among the hidden rows and
    // one shown row further down.
    let rows = vec![
        run(0, 0, 529, false),
        run(1, 1, 1058, false),
        run(2, 49, 529, true),
        run(50, 50, 529, false),
        run(51, LAST_ROW, 529, true),
    ];
    let mut wb = book(SheetData {
        name: "S".into(),
        cells: vec![cell(0, 0, 1.0), cell(10, 0, 2.0), cell(50, 1, 3.0)],
        row_heights: rows.clone(),
        style_runs: vec![StyleRun {
            col: 2,
            start_row: 20,
            end_row: 21,
            style: 0,
        }],
        ..SheetData::default()
    });
    wb.styles = vec![StyleData {
        fill: Some(0xff00_00ff),
        ..StyleData::default()
    }];
    let mut out = Cursor::new(Vec::new());
    let report = tally_io_xlsx::write_xlsx(&wb, &mut out).unwrap();
    assert_eq!(report.warnings, Vec::<String>::new());
    let bytes = out.into_inner();
    let xml = sheet_xml(&bytes);
    let format = element(&xml, "sheetFormatPr");
    assert!(format.contains(r#"zeroHeight="1""#), "{format}");
    // The three shown rows and the three hidden ones that have content.
    assert_eq!(xml.matches("<row ").count(), 6, "{xml}");
    assert!(xml.len() < 4000, "{} bytes", xml.len());

    let back = read_xlsx(Cursor::new(bytes), &Limits::default()).unwrap();
    assert_eq!(back.sheets[0].row_heights, rows);
    assert_eq!(back.sheets[0].cells.len(), 5);
    let again = round_trip(&back);
    assert_eq!(again.sheets[0].row_heights, rows);

    // Read from a file that says it with `zeroHeight`, and back.
    let source = ws(
        r#"<sheetFormatPr defaultRowHeight="15" zeroHeight="1"/>"#,
        "",
        r#"<row r="1"><c r="A1"><v>1</v></c></row><row r="2" ht="30" customHeight="1"/>"#,
    );
    let first = read_xlsx(Cursor::new(package(&source)), &Limits::default()).unwrap();
    let want = [
        run(0, 0, 529, false),
        run(1, 1, 1058, false),
        run(2, LAST_ROW, 529, true),
    ];
    assert_eq!(first.sheets[0].row_heights, want);
    let bytes = to_bytes(&first);
    assert_eq!(sheet_xml(&bytes).matches("<row ").count(), 2);
    let second = read_xlsx(Cursor::new(bytes), &Limits::default()).unwrap();
    assert_eq!(second.sheets[0].row_heights, want);
}

#[test]
fn a_sheet_with_rows_or_columns_missing_from_its_runs_keeps_its_default() {
    // What inserting and deleting leaves of a sheet read from Excel: a few
    // rows and columns without a run among a million with the file's
    // size.
    let wb = book(SheetData {
        name: "S".into(),
        cells: vec![cell(0, 0, 1.0), cell(3, 0, 2.0)],
        row_heights: vec![
            run(0, 2, 529, false),
            run(5, 5, 1058, false),
            run(6, LAST_ROW - 1, 529, false),
        ],
        col_widths: vec![run(0, 1, 1693, false), run(4, LAST_COL - 2, 1693, false)],
        ..SheetData::default()
    });
    let mut out = Cursor::new(Vec::new());
    let report = tally_io_xlsx::write_xlsx(&wb, &mut out).unwrap();
    assert_eq!(report.warnings, Vec::<String>::new());
    let bytes = out.into_inner();
    let xml = sheet_xml(&bytes);
    assert_eq!(
        element(&xml, "sheetFormatPr"),
        r#"<sheetFormatPr defaultRowHeight="15"/>"#
    );
    // Rows 4, 5 (no run), 6 (30 pt) and the last one; row 1 has a cell.
    assert_eq!(xml.matches("<row ").count(), 5, "{xml}");
    assert_eq!(xml.matches("<col ").count(), 2, "{xml}");
    let back = read_xlsx(Cursor::new(bytes), &Limits::default()).unwrap();
    // The rows without a run come back as rows of 24 px; the columns
    // without one as columns without one (100 px).
    assert_eq!(
        back.sheets[0].row_heights,
        [
            run(0, 2, 529, false),
            run(3, 4, 635, false),
            run(5, 5, 1058, false),
            run(6, LAST_ROW - 1, 529, false),
            run(LAST_ROW, LAST_ROW, 635, false),
        ]
    );
    assert_eq!(back.sheets[0].col_widths, wb.sheets[0].col_widths);
    let again = round_trip(&back);
    assert_eq!(again.sheets[0].row_heights, back.sheets[0].row_heights);
    assert_eq!(again.sheets[0].col_widths, back.sheets[0].col_widths);
}

/// A 1 × 1 px PNG.
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01\x08\x06\x00\x00\x00\x1f\x15\xc4\x89\x00\x00\x00\rIDATx\x9cc\xf8\xcf\xc0\xf0\x1f\x00\x05\x00\x02\x00\xde\xf2\x8a\xbb\x00\x00\x00\x00IEND\xaeB`\x82";

fn drawing_xml(bytes: &[u8]) -> String {
    let mut z = zip::ZipArchive::new(Cursor::new(bytes.to_vec())).unwrap();
    let mut s = String::new();
    z.by_name("xl/drawings/drawing1.xml")
        .unwrap()
        .read_to_string(&mut s)
        .unwrap();
    s
}

fn tag_text<'a>(xml: &'a str, tag: &str) -> &'a str {
    let open = format!("<{tag}>");
    let at = xml.find(&open).unwrap() + open.len();
    &xml[at..at + xml[at..].find('<').unwrap()]
}

#[test]
fn pictures_are_placed_by_the_sizes_the_sheet_has() {
    // 250 × 60 px at B2, in a sheet of Tally's default columns (100 px)
    // and rows (24 px), with one row of 40 px.
    let picture = ImageData {
        name: "P".into(),
        anchor: ChartAnchorData {
            row: 1,
            col: 1,
            dx_hmm: 0,
            dy_hmm: 0,
            width_hmm: 6615,
            height_hmm: 1588,
        },
        mime: "image/png".into(),
        bytes: Arc::from(PNG),
    };
    let wb = book(SheetData {
        name: "S".into(),
        cells: vec![cell(0, 0, 1.0)],
        row_heights: vec![run(1, 1, 1058, false)],
        images: vec![picture.clone()],
        ..SheetData::default()
    });
    let bytes = to_bytes(&wb);
    // The end cell: two whole columns and 50 px into D; the 40 px row and
    // 20 px into row 3.
    let drawing = drawing_xml(&bytes);
    let to = &drawing[drawing.find("<xdr:to>").unwrap()..];
    assert_eq!(tag_text(to, "xdr:col"), "3", "{drawing}");
    assert_eq!(tag_text(to, "xdr:colOff"), (50 * 9525).to_string());
    assert_eq!(tag_text(to, "xdr:row"), "2", "{drawing}");
    assert_eq!(tag_text(to, "xdr:rowOff"), (20 * 9525).to_string());

    let back = read_xlsx(Cursor::new(bytes), &Limits::default()).unwrap();
    let s = &back.sheets[0];
    assert!(s.col_widths.is_empty());
    assert_eq!(s.row_heights, wb.sheets[0].row_heights);
    let a = s.images[0].anchor;
    assert_eq!((a.row, a.col, a.dx_hmm, a.dy_hmm), (1, 1, 0, 0));
    assert!((i64::from(a.width_hmm) - 6615).abs() <= 27, "{a:?}");
    assert!((i64::from(a.height_hmm) - 1588).abs() <= 27, "{a:?}");
}
