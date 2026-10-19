//! Regression tests for the second security review
//! (`docs/reviews/security-review-2-2026-10-19.md`), ODS reader and writer
//! side: HIGH-1 (merge cap), HIGH-2 (array block areas count against the
//! cell budget), HIGH-4 (long number formats), LOW-1 (writer merge lookups)
//! and the INFO iteration-count clamp. Inputs are generated here.

#![allow(clippy::unwrap_used)] // tests may unwrap (docs/contributing.md)

use std::io::{Cursor, Write};
use std::time::{Duration, Instant};

use tally_io_ods::{read_ods, write_ods, Limits, OdsError};
use tally_model::{CellRangeLocal, SheetData, WorkbookData};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

const HEAD: &str = r#"<?xml version="1.0" encoding="UTF-8"?><office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:number="urn:oasis:names:tc:opendocument:xmlns:datastyle:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:version="1.2">"#;

fn ods(automatic_styles: &str, body: &str) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    zip.start_file("mimetype", stored).unwrap();
    zip.write_all(tally_io_ods::MIMETYPE.as_bytes()).unwrap();
    zip.start_file("content.xml", deflated).unwrap();
    write!(
        zip,
        "{HEAD}<office:automatic-styles>{automatic_styles}</office:automatic-styles>\
         <office:body><office:spreadsheet>{body}</office:spreadsheet></office:body>\
         </office:document-content>"
    )
    .unwrap();
    zip.finish().unwrap().into_inner()
}

fn sheet(rows: &str) -> String {
    format!(r#"<table:table table:name="S">{rows}</table:table>"#)
}

fn limit_what(r: &Result<WorkbookData, OdsError>) -> &'static str {
    match r {
        Err(OdsError::LimitExceeded { what, .. }) => what,
        other => panic!("expected LimitExceeded, got {other:?}"),
    }
}

/// HIGH-1 (`p2_merges.py`): one 2-column merge per row. The reader keeps
/// the first 100,000 merges of a sheet and says so.
#[test]
fn merges_per_sheet_are_capped() {
    let row = r#"<table:table-row><table:table-cell table:number-columns-spanned="2"/><table:covered-table-cell/></table:table-row>"#;
    let bytes = ods("", &sheet(&row.repeat(120_000)));
    let t = Instant::now();
    let wb = read_ods(Cursor::new(bytes), &Limits::default()).unwrap();
    let took = t.elapsed();
    assert_eq!(wb.sheets[0].merges.len(), 100_000);
    assert!(
        wb.warnings.iter().any(|w| w.contains("merged areas")),
        "{:?}",
        wb.warnings
    );
    assert!(took < Duration::from_secs(10), "took {took:?}");
}

/// HIGH-2 (`p3_arrays.py`): every covered cell of an array block counts
/// against `max_cells`. Twenty 1M-row blocks in a 1 KB file (21M array
/// parts, 2.3 GB before) are refused at once.
#[test]
fn array_block_areas_count_against_the_cell_cap() {
    let cell = r#"<table:table-cell table:formula="of:=ROW([.A1:.A1048576])" table:number-matrix-columns-spanned="1" table:number-matrix-rows-spanned="1048576" office:value-type="float" office:value="1"/>"#;
    let bytes = ods(
        "",
        &sheet(&format!(
            "<table:table-row>{}</table:table-row>",
            cell.repeat(20)
        )),
    );
    assert!(bytes.len() < 2048);
    let t = Instant::now();
    let r = read_ods(Cursor::new(bytes), &Limits::default());
    assert_eq!(limit_what(&r), "materialized cells");
    assert!(t.elapsed() < Duration::from_secs(1), "{:?}", t.elapsed());
    // Five blocks (5M cells) are within the default 10M.
    let bytes = ods(
        "",
        &sheet(&format!(
            "<table:table-row>{}</table:table-row>",
            cell.repeat(5)
        )),
    );
    let wb = read_ods(Cursor::new(bytes), &Limits::default()).unwrap();
    assert_eq!(wb.sheets[0].array_formulas.len(), 5);
}

/// HIGH-4: 256 `<number:text>` parts of 4,000 characters (a 1 MB code;
/// `cells()` took 53 s before) read as General with a warning; a normal
/// code with text parts is kept.
#[test]
fn over_long_number_formats_become_general() {
    let part = format!("<number:text>{}</number:text>", "x".repeat(4000));
    let styles = format!(
        r#"<number:number-style style:name="N1"><number:number number:decimal-places="0"/>{}</number:number-style><style:style style:name="ce1" style:family="table-cell" style:data-style-name="N1"/><number:number-style style:name="N2"><number:number number:decimal-places="2"/><number:text> kg</number:text></number:number-style><style:style style:name="ce2" style:family="table-cell" style:data-style-name="N2"/>"#,
        part.repeat(256)
    );
    let body = sheet(
        r#"<table:table-row><table:table-cell table:style-name="ce1" office:value-type="float" office:value="1"/><table:table-cell table:style-name="ce2" office:value-type="float" office:value="2"/></table:table-row>"#,
    );
    let t = Instant::now();
    let wb = read_ods(Cursor::new(ods(&styles, &body)), &Limits::default()).unwrap();
    assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
    let fmt = |col: u32| {
        let c = wb.sheets[0].cells.iter().find(|c| c.col == col).unwrap();
        c.style
            .and_then(|i| wb.styles[i as usize].number_format.clone())
    };
    assert_eq!(fmt(0), None, "General");
    assert_eq!(fmt(1).as_deref(), Some("#.00\" kg\""));
    assert!(wb
        .warnings
        .iter()
        .any(|w| w.contains("Number formats longer than")));
}

/// INFO: an absurd iteration count is clamped to Excel's 32,767.
#[test]
fn iteration_steps_are_clamped() {
    let body = r#"<table:calculation-settings><table:iteration table:status="enable" table:steps="4000000000"/></table:calculation-settings>"#;
    let wb = read_ods(
        Cursor::new(ods("", &format!("{body}{}", sheet("")))),
        &Limits::default(),
    )
    .unwrap();
    assert_eq!(
        wb.settings.calc_settings.iterations.map(|i| i.steps),
        Some(32_767)
    );
}

/// LOW-1: saving 100k merges laid out 8,000 per row (3.6 s before) is
/// linear in the merges of each row.
#[test]
fn writer_merge_lookups_are_logarithmic() {
    let merges: Vec<CellRangeLocal> = (0..100_000u32)
        .map(|i| {
            let (r, c) = (i / 8000, 2 * (i % 8000));
            CellRangeLocal {
                r0: r,
                c0: c,
                r1: r,
                c1: c + 1,
            }
        })
        .collect();
    let data = WorkbookData {
        sheets: vec![SheetData {
            name: "S".into(),
            merges: merges.clone(),
            ..SheetData::default()
        }],
        ..WorkbookData::default()
    };
    let t = Instant::now();
    let mut out = Cursor::new(Vec::new());
    write_ods(&data, &mut out).unwrap();
    let took = t.elapsed();
    assert!(took < Duration::from_secs(5), "took {took:?}");
    let back = read_ods(Cursor::new(out.into_inner()), &Limits::default()).unwrap();
    assert_eq!(back.sheets[0].merges, merges);
}
