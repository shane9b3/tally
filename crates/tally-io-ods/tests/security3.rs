//! Regression tests for the third security review
//! (`docs/reviews/security-review-3-2026-10-20.md`), ODS reader side:
//! MEDIUM-8 (note text), MEDIUM-5 (protection key length), HIGH-3-alike
//! link sharing, and the conditional-format caps. Inputs are generated
//! here.

#![allow(clippy::unwrap_used)] // tests may unwrap (docs/contributing.md)

use std::fmt::Write as _;
use std::io::{Cursor, Write};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tally_io_ods::{read_ods, Limits};
use tally_model::WorkbookData;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

const HEAD: &str = r#"<?xml version="1.0" encoding="UTF-8"?><office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:number="urn:oasis:names:tc:opendocument:xmlns:datastyle:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" xmlns:calcext="urn:org:documentfoundation:names:experimental:calc:xmlns:calcext:1.0" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:xlink="http://www.w3.org/1999/xlink" office:version="1.2">"#;

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

fn read(bytes: Vec<u8>) -> (WorkbookData, Duration) {
    let t = Instant::now();
    let r = read_ods(Cursor::new(bytes), &Limits::default()).unwrap();
    (r, t.elapsed())
}

fn cell(inner: &str) -> String {
    format!(
        r#"<table:table-cell office:value-type="float" office:value="1">{inner}<text:p>1</text:p></table:table-cell>"#
    )
}

/// MEDIUM-8 (`notebig`): one note of 4,096 paragraphs × 8 KiB (33 MB)
/// loaded whole. Notes are cut at the engine's 32 KiB cap.
#[test]
fn long_notes_are_truncated() {
    let mut ann =
        String::from(r#"<office:annotation office:display="true"><dc:creator>bob</dc:creator>"#);
    for _ in 0..64 {
        let _ = write!(ann, "<text:p>{}</text:p>", "é".repeat(4096));
    }
    ann.push_str("</office:annotation>");
    let body = format!(
        r#"<table:table table:name="S"><table:table-row>{}</table:table-row></table:table>"#,
        cell(&ann)
    );
    let (wb, took) = read(ods("", &body));
    assert!(took < Duration::from_secs(5), "{took:?}");
    let n = &wb.sheets[0].notes[0];
    assert!(
        n.text.len() <= 32 * 1024 && n.text.len() > 32 * 1024 - 4,
        "{}",
        n.text.len()
    );
    assert_eq!(n.author.as_deref(), Some("bob"));
}

/// MEDIUM-5 (`prot`): a 12 MB `table:protection-key` was kept (and
/// cloned with every meta op). Keys over 1 KiB are dropped; the sheet
/// stays protected.
#[test]
fn long_protection_keys_are_dropped() {
    let body = format!(
        r#"<table:table table:name="S" table:protected="true" table:protection-key="{}" table:protection-key-digest-algorithm="http://www.w3.org/2000/09/xmldsig#sha256"><table:table-row>{}</table:table-row></table:table>"#,
        "A".repeat(4096),
        cell("")
    );
    let (wb, _) = read(ods("", &body));
    let p = wb.sheets[0].protection.as_ref().unwrap();
    assert!(p.password_hash.is_none());
    let body = format!(
        r#"<table:table table:name="S" table:protected="true" table:protection-key="{}" table:protection-key-digest-algorithm="http://www.w3.org/2000/09/xmldsig#sha256"><table:table-row>{}</table:table-row></table:table>"#,
        "A".repeat(44),
        cell("")
    );
    let (wb, _) = read(ods("", &body));
    assert!(wb.sheets[0]
        .protection
        .as_ref()
        .unwrap()
        .password_hash
        .is_some());
}

/// A link on a repeated cell is one allocation shared by every cell.
#[test]
fn repeated_links_share_one_url() {
    let body = r#"<table:table table:name="S"><table:table-row table:number-rows-repeated="100"><table:table-cell office:value-type="string" table:number-columns-repeated="10"><text:p><text:a xlink:href="https://example.com/x" xlink:type="simple">t</text:a></text:p></table:table-cell></table:table-row></table:table>"#;
    let (wb, _) = read(ods("", body));
    let links = &wb.sheets[0].links;
    assert_eq!(links.len(), 1000);
    assert!(links.iter().all(|l| Arc::ptr_eq(&l.url, &links[0].url)));
}

/// CRITICAL-1 (`cf 10000 1 100` and `cf 1000 1024 100`): only the first
/// 2,048 conditional formats of a sheet are kept and each keeps at most
/// 64 ranges.
#[test]
fn conditional_formats_are_capped() {
    let mut cfs = String::new();
    for _ in 0..2500 {
        cfs.push_str(r#"<calcext:conditional-format calcext:target-range-address="S.A1:S.T1000"><calcext:condition calcext:apply-style-name="cs" calcext:value="formula-is([.A1]>0)" calcext:base-cell-address="S.A1"/></calcext:conditional-format>"#);
    }
    let ranges: String = (0..100)
        .map(|j| format!("S.A{}:S.T{} ", j * 1000 + 1, j * 1000 + 1000))
        .collect();
    let _ = write!(
        cfs,
        r#"<calcext:conditional-format calcext:target-range-address="{ranges}"><calcext:condition calcext:apply-style-name="cs" calcext:value="formula-is([.A1]>0)" calcext:base-cell-address="S.A1"/></calcext:conditional-format>"#
    );
    let body = format!(
        r#"<table:table table:name="S"><table:table-row>{}</table:table-row><calcext:conditional-formats>{cfs}</calcext:conditional-formats></table:table>"#,
        cell("")
    );
    let auto = r##"<style:style style:name="cs" style:family="table-cell"><style:table-cell-properties fo:background-color="#ff0000"/></style:style>"##;
    let (wb, took) = read(ods(auto, &body));
    assert!(took < Duration::from_secs(10), "{took:?}");
    assert_eq!(wb.sheets[0].conditional_formats.len(), 2048);
    let body = format!(
        r#"<table:table table:name="S"><table:table-row>{}</table:table-row><calcext:conditional-formats><calcext:conditional-format calcext:target-range-address="{ranges}"><calcext:condition calcext:apply-style-name="cs" calcext:value="formula-is([.A1]>0)" calcext:base-cell-address="S.A1"/></calcext:conditional-format></calcext:conditional-formats></table:table>"#,
        cell("")
    );
    let (wb, _) = read(ods(auto, &body));
    assert_eq!(wb.sheets[0].conditional_formats[0].ranges.len(), 64);
}
