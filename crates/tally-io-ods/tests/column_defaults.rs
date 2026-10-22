//! Column default cell styles (`table:default-cell-style-name` on
//! `table:table-column`) format the cells the rows contain, not the whole
//! column, as in AOO: a sheet whose rows end at its content (one sheet of
//! an owner's price list) showed its table's fill and borders down a
//! million rows and asked to print more than 10,000 pages.

#![allow(clippy::unwrap_used)] // tests may unwrap (docs/contributing.md)

mod common;

use std::io::{Cursor, Write};

use tally_io_ods::{read_ods, write_ods, Limits};
use tally_model::{StyleData, WorkbookData};
use tally_types::MAX_ROWS;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

const HEAD: &str = r#"<?xml version="1.0" encoding="UTF-8"?><office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:version="1.2">"#;

/// A grey, boxed cell style (`box`) and a wrapping one (`wrap`).
const STYLES: &str = r##"<style:style style:name="box" style:family="table-cell"><style:table-cell-properties fo:background-color="#eeeeee" fo:border="0.002cm solid #808080"/></style:style><style:style style:name="wrap" style:family="table-cell"><style:table-cell-properties fo:wrap-option="wrap"/></style:style>"##;

fn ods(table: &str) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    zip.start_file("mimetype", stored).unwrap();
    zip.write_all(tally_io_ods::MIMETYPE.as_bytes()).unwrap();
    zip.start_file("content.xml", stored).unwrap();
    write!(
        zip,
        "{HEAD}<office:automatic-styles>{STYLES}</office:automatic-styles>\
         <office:body><office:spreadsheet>{table}</office:spreadsheet></office:body>\
         </office:document-content>"
    )
    .unwrap();
    zip.finish().unwrap().into_inner()
}

fn read(bytes: Vec<u8>) -> WorkbookData {
    read_ods(Cursor::new(bytes), &Limits::default()).unwrap()
}

/// The style a cell shows: its own, else the run covering it.
fn style_at(wb: &WorkbookData, row: u32, col: u32) -> Option<&StyleData> {
    let s = &wb.sheets[0];
    let own = s
        .cells
        .iter()
        .find(|c| c.row == row && c.col == col)
        .and_then(|c| c.style);
    let run = s
        .style_runs
        .iter()
        .find(|r| r.col == col && (r.start_row..=r.end_row).contains(&row))
        .map(|r| r.style);
    own.or(run).map(|i| &wb.styles[i as usize])
}

fn boxed(wb: &WorkbookData, row: u32, col: u32) -> bool {
    style_at(wb, row, col).is_some_and(|s| s.fill.is_some() && s.borders.top.is_some())
}

/// The structure of the sheet that printed on over 10,000 pages: column
/// defaults carry the table's formatting, the rows stop at the content,
/// and some cells name "Default" to opt out of their column's style.
const PRICE_TABLE: &str = r#"<table:table table:name="S">
<table:table-column table:default-cell-style-name="wrap"/>
<table:table-column table:number-columns-repeated="3" table:default-cell-style-name="box"/>
<table:table-column table:default-cell-style-name="Default"/>
<table:table-row><table:table-cell table:style-name="Default" office:value-type="string"><text:p>Finish basement</text:p></table:table-cell><table:table-cell table:style-name="Default" table:number-columns-repeated="3"/><table:table-cell/></table:table-row>
<table:table-row><table:table-cell office:value-type="string"><text:p>Foyer</text:p></table:table-cell><table:table-cell office:value-type="float" office:value="60"/><table:table-cell/><table:table-cell table:number-columns-repeated="2"/></table:table-row>
<table:table-row table:number-rows-repeated="2"><table:table-cell office:value-type="string"><text:p>Hall</text:p></table:table-cell><table:table-cell table:number-columns-repeated="3"/><table:table-cell/></table:table-row>
<table:table-row><table:table-cell table:style-name="Default"/><table:table-cell table:style-name="Default" table:number-columns-repeated="3"/></table:table-row>
</table:table>"#;

#[test]
fn column_default_styles_format_only_the_cells_rows_contain() {
    let wb = read(ods(PRICE_TABLE));
    // The table body takes its columns' style, values and blanks alike.
    for (r, c) in [(1, 1), (1, 2), (1, 3), (2, 1), (3, 3)] {
        assert!(boxed(&wb, r, c), "({r}, {c})");
    }
    assert!(style_at(&wb, 1, 0).is_some_and(|s| s.align.wrap));
    // Cells naming "Default" keep no style.
    for c in 0..4 {
        assert!(style_at(&wb, 0, c).is_none(), "(0, {c})");
        assert!(style_at(&wb, 4, c).is_none(), "(4, {c})");
    }
    // Nothing below the rows the sheet has.
    assert!(
        wb.sheets[0].style_runs.iter().all(|r| r.end_row <= 4),
        "{:?}",
        wb.sheets[0].style_runs
    );
    assert!(!boxed(&wb, 5, 1));
    assert!(!boxed(&wb, MAX_ROWS - 1, 1));
    // Adjacent rows are one run per column.
    let col2: Vec<_> = wb.sheets[0]
        .style_runs
        .iter()
        .filter(|r| r.col == 2)
        .map(|r| (r.start_row, r.end_row))
        .collect();
    assert_eq!(col2, [(1, 3)]);
}

#[test]
fn trailing_repeated_row_formats_to_the_end() {
    // AOO writes rows to the last one, so its column formats cover them.
    let table = r#"<table:table table:name="S">
<table:table-column table:default-cell-style-name="box"/>
<table:table-column table:number-columns-repeated="1023" table:default-cell-style-name="Default"/>
<table:table-row><table:table-cell office:value-type="float" office:value="1"/><table:table-cell table:number-columns-repeated="1023"/></table:table-row>
<table:table-row table:number-rows-repeated="1048575"><table:table-cell table:number-columns-repeated="1024"/></table:table-row>
</table:table>"#;
    let wb = read(ods(table));
    assert!(boxed(&wb, 0, 0));
    assert!(boxed(&wb, MAX_ROWS - 1, 0));
    assert!(!boxed(&wb, 5, 1));
    let runs = &wb.sheets[0].style_runs;
    assert_eq!(runs.len(), 1, "{runs:?}");
    assert_eq!((runs[0].start_row, runs[0].end_row), (0, MAX_ROWS - 1));
}

#[test]
fn row_default_style_replaces_column_defaults() {
    let table = r#"<table:table table:name="S">
<table:table-column table:number-columns-repeated="2" table:default-cell-style-name="box"/>
<table:table-row table:default-cell-style-name="wrap"><table:table-cell office:value-type="float" office:value="1"/><table:table-cell/></table:table-row>
<table:table-row table:default-cell-style-name="Default"><table:table-cell office:value-type="float" office:value="2"/><table:table-cell/></table:table-row>
<table:table-row><table:table-cell office:value-type="float" office:value="3"/><table:table-cell/></table:table-row>
</table:table>"#;
    let wb = read(ods(table));
    for c in 0..2 {
        assert!(style_at(&wb, 0, c).is_some_and(|s| s.align.wrap && s.fill.is_none()));
        assert!(style_at(&wb, 1, c).is_none(), "(1, {c})");
        assert!(boxed(&wb, 2, c), "(2, {c})");
    }
}

/// Whole-column formatting written by Tally survives a reload: the writer
/// gives the columns cells down to the last row, as AOO does.
#[test]
fn whole_column_styles_round_trip() {
    let wb = read(ods(PRICE_TABLE));
    let mut whole = wb.clone();
    let st = whole.sheets[0].style_runs[0].style;
    whole.sheets[0].style_runs.push(tally_model::StyleRun {
        col: 6,
        start_row: 0,
        end_row: MAX_ROWS - 1,
        style: st,
    });
    for mut book in [wb, whole] {
        let mut buf = Cursor::new(Vec::new());
        write_ods(&book, &mut buf).unwrap();
        let back = read(buf.into_inner());
        book.meta.generator.clone_from(&back.meta.generator);
        assert_eq!(common::normalize(&back), common::normalize(&book));
    }
}

/// A column default over all 16,384 columns and rows whose first cell
/// alternates between a style of its own and none: the runs of the other
/// columns continue, so this opens with about one run per change instead
/// of one per column per row (review: 2,000 such rows exceeded the
/// materialized-cell limit).
#[test]
fn alternating_rows_keep_whole_column_runs() {
    const COLS: u32 = 16_384;
    const ROWS: u32 = 2_000;
    let styles = r#"<table:table-column table:number-columns-repeated="16384" table:default-cell-style-name="box"/>"#;
    let mut table = format!(r#"<table:table table:name="S">{styles}"#);
    for r in 0..ROWS {
        let own = if r % 2 == 0 {
            r#" table:style-name="wrap""#
        } else {
            ""
        };
        table.push_str(&format!(
            r#"<table:table-row><table:table-cell{own} office:value-type="float" office:value="{r}"/><table:table-cell table:number-columns-repeated="{}"/></table:table-row>"#,
            COLS - 1
        ));
    }
    table.push_str(&format!(
        r#"<table:table-row table:number-rows-repeated="{}"><table:table-cell table:number-columns-repeated="{COLS}"/></table:table-row></table:table>"#,
        MAX_ROWS - ROWS
    ));
    let t = std::time::Instant::now();
    let wb = read(ods(&table));
    assert!(t.elapsed().as_secs() < 20, "{:?}", t.elapsed());
    let runs = &wb.sheets[0].style_runs;
    assert!(runs.len() < 2 * COLS as usize, "{} runs", runs.len());
    assert!(style_at(&wb, 0, 0).is_some_and(|s| s.align.wrap && s.fill.is_none()));
    assert!(boxed(&wb, 1, 0));
    assert!(boxed(&wb, ROWS, 0));
    for c in [1, 5_000, COLS - 1] {
        let col: Vec<_> = runs.iter().filter(|r| r.col == c).collect();
        assert_eq!(col.len(), 1, "column {c}: {col:?}");
        assert_eq!((col[0].start_row, col[0].end_row), (0, MAX_ROWS - 1));
    }
}
