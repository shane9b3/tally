//! Validity list order and database-range sorts in hand-written ODS
//! (review M1, M2): LibreOffice's `sort-ascending`, and `table:sort`
//! elements that Refresh Range cannot repeat.
#![allow(clippy::unwrap_used)]

use std::io::{Cursor, Write};

use tally_io_ods::{read_ods, Limits};
use tally_model::WorkbookData;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

const NS: &str = r#"xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0""#;

/// One sheet "Sheet1" (3 × 3 values), then `tail` (content validations,
/// database ranges) in `office:spreadsheet`.
fn read(validations: &str, cell_attr: &str, tail: &str) -> WorkbookData {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    zip.start_file("mimetype", stored).unwrap();
    zip.write_all(tally_io_ods::MIMETYPE.as_bytes()).unwrap();
    zip.start_file("META-INF/manifest.xml", deflated).unwrap();
    zip.write_all(br#"<?xml version="1.0"?><manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.2"><manifest:file-entry manifest:full-path="/" manifest:media-type="application/vnd.oasis.opendocument.spreadsheet"/><manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/></manifest:manifest>"#).unwrap();
    zip.start_file("content.xml", deflated).unwrap();
    let row = r#"<table:table-row><table:table-cell office:value-type="string"><text:p>b</text:p></table:table-cell><table:table-cell office:value-type="string"><text:p>a</text:p></table:table-cell><table:table-cell office:value-type="float" office:value="1"><text:p>1</text:p></table:table-cell></table:table-row>"#;
    write!(
        zip,
        r#"<?xml version="1.0" encoding="UTF-8"?><office:document-content {NS} office:version="1.2"><office:body><office:spreadsheet><table:content-validations>{validations}</table:content-validations><table:table table:name="Sheet1"><table:table-row><table:table-cell{cell_attr} office:value-type="string"><text:p>h</text:p></table:table-cell></table:table-row>{row}{row}</table:table>{tail}</office:spreadsheet></office:body></office:document-content>"#
    )
    .unwrap();
    let bytes = zip.finish().unwrap().into_inner();
    read_ods(Cursor::new(bytes), &Limits::default()).unwrap()
}

fn list(display: &str) -> (bool, bool) {
    let v = format!(
        r#"<table:content-validation table:name="v1" table:condition="of:cell-content-is-in-list(&quot;b&quot;;&quot;a&quot;)" table:display-list="{display}" table:base-cell-address="Sheet1.A1"/>"#
    );
    let wb = read(&v, r#" table:content-validation-name="v1""#, "");
    let d = &wb.sheets[0].validations[0];
    (d.show_dropdown, d.sort_ascending)
}

/// AOO writes `sorted-ascending`, LibreOffice (and ODF 1.2) `sort-ascending`.
#[test]
fn display_list_accepts_both_sorted_spellings() {
    assert_eq!(list("sorted-ascending"), (true, true));
    assert_eq!(list("sort-ascending"), (true, true));
    assert_eq!(list("unsorted"), (true, false));
    assert_eq!(list("no"), (false, false));
    assert_eq!(list("none"), (false, false));
}

fn db_sort(range_attrs: &str, sort_attrs: &str) -> Option<tally_model::DbSortData> {
    let tail = format!(
        r#"<table:database-ranges><table:database-range table:name="Data" table:target-range-address="Sheet1.A1:Sheet1.C3"{range_attrs}><table:sort{sort_attrs}><table:sort-by table:field-number="1" table:data-type="automatic" table:order="descending"/></table:sort></table:database-range></table:database-ranges>"#
    );
    let wb = read("", "", &tail);
    wb.sheets[0].database_ranges[0].sort.clone()
}

/// Only in-place top-to-bottom sorts are kept for Refresh Range.
#[test]
fn copied_and_left_to_right_sorts_are_not_kept() {
    let kept = db_sort("", "").unwrap();
    assert_eq!(kept.keys, vec![(1, false)]);
    assert!(db_sort(r#" table:orientation="column""#, "").is_none());
    assert!(db_sort("", r#" table:target-range-address="Sheet1.E1""#).is_none());
}
