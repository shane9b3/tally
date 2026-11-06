//! `table:detective` reading: operations belong to the first cell of a
//! repeated run, and the document keeps at most 1,000 operations, those
//! with the highest `table:index` (security/correctness review of the
//! detective work, M3 and L1: a 256 KB file expanded to 1.8 GB).

#![allow(clippy::unwrap_used)] // tests may unwrap (docs/contributing.md)

use std::io::{Cursor, Write};

use tally_io_ods::{read_ods, Limits};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

const NS: &str = r#"xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0""#;

fn package(rows: &str) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    zip.start_file("mimetype", stored).unwrap();
    zip.write_all(tally_io_ods::MIMETYPE.as_bytes()).unwrap();
    zip.start_file("META-INF/manifest.xml", deflated).unwrap();
    zip.write_all(br#"<?xml version="1.0"?><manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.3"><manifest:file-entry manifest:full-path="/" manifest:media-type="application/vnd.oasis.opendocument.spreadsheet"/><manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/></manifest:manifest>"#).unwrap();
    zip.start_file("content.xml", deflated).unwrap();
    write!(
        zip,
        r#"<?xml version="1.0" encoding="UTF-8"?><office:document-content {NS} office:version="1.2"><office:body><office:spreadsheet><table:table table:name="Sheet1">{rows}</table:table></office:spreadsheet></office:body></office:document-content>"#
    )
    .unwrap();
    zip.finish().unwrap().into_inner()
}

fn ops(from: u32, n: u32) -> String {
    (from..from + n)
        .map(|i| format!(r#"<table:operation table:name="trace-precedents" table:index="{i}"/>"#))
        .collect()
}

#[test]
fn repeated_cells_and_the_document_cap() {
    // 1,000 columns × 1,000 rows repeating a cell with 3 operations: 3 kept.
    let rows = format!(
        r#"<table:table-row table:number-rows-repeated="1000"><table:table-cell table:number-columns-repeated="1000"><table:detective>{}</table:detective></table:table-cell></table:table-row>"#,
        ops(0, 3)
    );
    let wb = read_ods(Cursor::new(package(&rows)), &Limits::default()).unwrap();
    let list = &wb.sheets[0].detective_ops;
    assert_eq!(list.len(), 3);
    assert!(list.iter().all(|o| (o.row, o.col) == (0, 0)));
    // One cell with 5,000 operations: the 1,000 highest indexes survive.
    let rows = format!(
        r#"<table:table-row><table:table-cell><table:detective>{}</table:detective></table:table-cell></table:table-row>"#,
        ops(0, 5_000)
    );
    let wb = read_ods(Cursor::new(package(&rows)), &Limits::default()).unwrap();
    let mut idx: Vec<u32> = wb.sheets[0].detective_ops.iter().map(|o| o.index).collect();
    idx.sort_unstable();
    assert_eq!(idx.len(), 1_000);
    assert_eq!(idx[0], 4_000);
    // Across cells the document cap holds too, keeping the newest.
    let rows: String = (0..3)
        .map(|r| {
            format!(
                r#"<table:table-row><table:table-cell><table:detective>{}</table:detective></table:table-cell></table:table-row>"#,
                ops(r * 800, 800)
            )
        })
        .collect();
    let wb = read_ods(Cursor::new(package(&rows)), &Limits::default()).unwrap();
    let list = &wb.sheets[0].detective_ops;
    assert_eq!(list.len(), 1_000);
    assert!(list.iter().filter(|o| o.index >= 1_600).count() == 800);
}
