//! Security review of the 2 GiB `content.xml` cap: strings that parsers keep
//! from attributes are bounded per package (H1), and the compression ratio
//! and re-reads are budgeted per package, not per entry (M1).

#![allow(clippy::unwrap_used)] // tests may unwrap (docs/contributing.md)

use std::io::{Cursor, Write};
use std::time::{Duration, Instant};

use tally_io_ods::{read_ods, Limits, OdsError};
use tally_model::WorkbookData;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

const HEAD: &str = r#"<?xml version="1.0" encoding="UTF-8"?><office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:xlink="http://www.w3.org/1999/xlink" office:version="1.2">"#;

/// A package with `content.xml` = automatic styles + body, plus extra entries.
fn ods_with(styles: &str, body: &str, extra: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    zip.start_file("mimetype", stored).unwrap();
    zip.write_all(tally_io_ods::MIMETYPE.as_bytes()).unwrap();
    zip.start_file("content.xml", deflated).unwrap();
    write!(
        zip,
        "{HEAD}<office:automatic-styles>{styles}</office:automatic-styles>\
         <office:body><office:spreadsheet>{body}</office:spreadsheet></office:body>\
         </office:document-content>"
    )
    .unwrap();
    for (name, data) in extra {
        zip.start_file(*name, deflated).unwrap();
        zip.write_all(data).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

fn ods(styles: &str, body: &str) -> Vec<u8> {
    ods_with(styles, body, &[])
}

fn read(bytes: &[u8], limits: &Limits) -> Result<WorkbookData, OdsError> {
    read_ods(Cursor::new(bytes), limits)
}

fn limit_what(r: &Result<WorkbookData, OdsError>) -> &'static str {
    match r {
        Err(OdsError::LimitExceeded { what, .. }) => what,
        other => panic!("expected LimitExceeded, got {other:?}"),
    }
}

fn small_budget() -> Limits {
    Limits {
        max_attr_bytes: 64 * 1024,
        ..Limits::default()
    }
}

/// The review's vector: thousands of sheets with long names (9,500 × 200 KiB
/// in 10 MB reached 3.4 GB RSS). A name over 1 KiB is refused at the source.
#[test]
fn over_long_sheet_name_is_refused() {
    let name = "n".repeat(2000);
    let bytes = ods("", &format!(r#"<table:table table:name="{name}"/>"#));
    assert_eq!(
        limit_what(&read(&bytes, &Limits::default())),
        "sheet name bytes"
    );
}

/// Names under the per-name cap still add up: the package attribute budget
/// stops them.
#[test]
fn many_sheet_names_hit_the_attribute_budget() {
    let body: String = (0..100)
        .map(|i| format!(r#"<table:table table:name="{i}{}"/>"#, "n".repeat(1000)))
        .collect();
    let bytes = ods("", &body);
    assert_eq!(
        limit_what(&read(&bytes, &small_budget())),
        "XML attribute bytes"
    );
    // The default budget (256 MiB) opens the same file.
    assert_eq!(read(&bytes, &Limits::default()).unwrap().sheets.len(), 100);
}

/// `table:default-cell-style-name` is kept per column run: charged.
#[test]
fn column_default_style_names_are_charged() {
    let cols: String = (0..100)
        .map(|i| {
            format!(
                r#"<table:table-column table:default-cell-style-name="s{i}{}"/>"#,
                "x".repeat(900)
            )
        })
        .collect();
    let bytes = ods(
        "",
        &format!(r#"<table:table table:name="S">{cols}</table:table>"#),
    );
    assert_eq!(
        limit_what(&read(&bytes, &small_budget())),
        "XML attribute bytes"
    );
}

/// `table:table-source xlink:href` goes into a warning: an over-long one is
/// dropped, and every one is charged.
#[test]
fn table_source_href_is_bounded() {
    let long = "h".repeat(5000);
    let bytes = ods(
        "",
        &format!(
            r#"<table:table table:name="S"><table:table-source xlink:href="{long}"/></table:table>"#
        ),
    );
    let wb = read(&bytes, &Limits::default()).unwrap();
    let w: Vec<_> = wb
        .warnings
        .iter()
        .filter(|w| w.starts_with("tally:external-link"))
        .collect();
    assert_eq!(w, ["tally:external-link:linkedSheet"]);
    assert!(wb.warnings.iter().all(|w| w.len() < 1024));
}

/// Style names are kept in maps for the whole read: over 1 KiB is refused,
/// and many of them are charged to the budget.
#[test]
fn style_names_are_bounded() {
    let long = "s".repeat(2000);
    let one = format!(r#"<style:style style:name="{long}" style:family="table-cell"/>"#);
    let bytes = ods(&one, r#"<table:table table:name="S"/>"#);
    assert_eq!(
        limit_what(&read(&bytes, &Limits::default())),
        "style name bytes"
    );

    let many: String = (0..100)
        .map(|i| {
            format!(
                r#"<style:style style:name="c{i}{}" style:family="table-cell"/>"#,
                "s".repeat(900)
            )
        })
        .collect();
    let bytes = ods(&many, r#"<table:table table:name="S"/>"#);
    assert_eq!(
        limit_what(&read(&bytes, &small_budget())),
        "XML attribute bytes"
    );
}

/// Cell attributes are not charged (cells have their own budgets), but an
/// over-long one (never a real style name) reads as empty.
#[test]
fn over_long_cell_attribute_reads_as_empty() {
    let long = "c".repeat(5000);
    let row = format!(
        r#"<table:table-row><table:table-cell table:style-name="{long}" office:value-type="float" office:value="7"/></table:table-row>"#
    );
    let body = format!(
        r#"<table:table table:name="S">{}</table:table>"#,
        row.repeat(50)
    );
    let bytes = ods("", &body);
    let wb = read(&bytes, &small_budget()).unwrap();
    assert_eq!(wb.sheets[0].cells.len(), 50);
}

/// M1: the 10 MiB ratio-free allowance is per package. Two entries of ~6 MiB
/// of whitespace (each under the floor, ~1000:1) are refused together.
#[test]
fn ratio_floor_is_per_package() {
    let pad = |tag: &str| {
        let mut v = format!(r#"<?xml version="1.0"?><office:document-{tag} xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0">"#).into_bytes();
        v.extend(std::iter::repeat_n(b' ', 6 * 1024 * 1024));
        v.extend(format!("</office:document-{tag}>").into_bytes());
        v
    };
    let bytes = ods_with(
        "",
        r#"<table:table table:name="S"/>"#,
        &[
            ("styles.xml", pad("styles")),
            ("settings.xml", pad("settings")),
        ],
    );
    assert!(bytes.len() < 200 * 1024, "{}", bytes.len());
    let t = Instant::now();
    let r = read(
        &bytes,
        &Limits {
            max_entry_bytes: 16 * 1024 * 1024,
            ..Limits::default()
        },
    );
    assert_eq!(limit_what(&r), "compression ratio");
    assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
}
