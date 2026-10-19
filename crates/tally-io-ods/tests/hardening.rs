//! Regression tests for the 2026-10-19 security review
//! (`docs/reviews/security-review-2026-10-19.md`): H1, H2 (reader side), M4,
//! M5, L1, L3 and L4. Every malicious input is generated here, so nothing
//! large is committed.

#![allow(clippy::unwrap_used)] // tests may unwrap (docs/contributing.md)

use std::io::{Cursor, Write};
use std::time::{Duration, Instant};

use tally_io_ods::{read_ods, sniff_is_ods, Limits, OdsError};
use tally_model::{CellContentData, WorkbookData};
use tally_types::Value;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

const HEAD: &str = r#"<?xml version="1.0" encoding="UTF-8"?><office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:version="1.2">"#;

/// An ODS package with `content.xml` = `automatic_styles` + `body`.
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

fn timed(bytes: &[u8], limits: &Limits) -> (Result<WorkbookData, OdsError>, Duration) {
    let t = Instant::now();
    let r = read_ods(Cursor::new(bytes), limits);
    (r, t.elapsed())
}

fn limit_what(r: &Result<WorkbookData, OdsError>) -> &'static str {
    match r {
        Err(OdsError::LimitExceeded { what, .. }) => what,
        other => panic!("expected LimitExceeded, got {other:?}"),
    }
}

fn value_at(wb: &WorkbookData, row: u32, col: u32) -> &CellContentData {
    &wb.sheets[0]
        .cells
        .iter()
        .find(|c| c.row == row && c.col == col)
        .unwrap()
        .content
}

/// H1: a 205-byte formula repeated over 600 × 16,384 cells (9.8M cells, the
/// review's 460-byte `formula_clone.ods`, 2 GB RSS before the fix) is
/// rejected by the materialized-text budget before anything is expanded.
#[test]
fn formula_clone_bomb_hits_text_budget_fast() {
    let formula = format!("of:=1{}", "+1".repeat(100));
    let body = sheet(&format!(
        r#"<table:table-row table:number-rows-repeated="600"><table:table-cell table:formula="{formula}" table:number-columns-repeated="16384"/></table:table-row>"#
    ));
    let bytes = ods("", &body);
    assert!(bytes.len() < 2048, "{} bytes", bytes.len());
    let (r, t) = timed(&bytes, &Limits::default());
    assert_eq!(limit_what(&r), "materialized cell text bytes");
    assert!(t < Duration::from_secs(2), "took {t:?}");
}

/// H1: repeated formula cells share one allocation.
#[test]
fn repeated_formula_text_is_shared() {
    let body = sheet(
        r#"<table:table-row table:number-rows-repeated="10"><table:table-cell table:formula="of:=1+2" table:number-columns-repeated="10"/></table:table-row>"#,
    );
    let wb = read_ods(Cursor::new(ods("", &body)), &Limits::default()).unwrap();
    let texts: Vec<&std::sync::Arc<str>> = wb.sheets[0]
        .cells
        .iter()
        .map(|c| match &c.content {
            CellContentData::Formula { text, .. } => text,
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(texts.len(), 100);
    assert!(texts
        .iter()
        .all(|t| std::sync::Arc::ptr_eq(t, texts[0]) && &***t == "of:=1+2"));
}

/// H1: formulas longer than the parser limit (8,192 characters) reject the
/// file; exactly 8,192 is fine.
#[test]
fn over_long_formula_is_rejected() {
    let cell = |n: usize| {
        let f = format!("of:={}", "1".repeat(n - 4));
        sheet(&format!(
            r#"<table:table-row><table:table-cell table:formula="{f}"/></table:table-row>"#
        ))
    };
    let ok = read_ods(Cursor::new(ods("", &cell(8192))), &Limits::default());
    assert!(ok.is_ok(), "{ok:?}");
    let (r, _) = timed(&ods("", &cell(8193)), &Limits::default());
    assert_eq!(limit_what(&r), "formula length");
}

/// H1: `office:string-value` has the same 16 MiB cap as `text:p`.
#[test]
fn huge_string_value_is_rejected() {
    let big = "x".repeat(16 * 1024 * 1024 + 1);
    let body = sheet(&format!(
        r#"<table:table-row><table:table-cell office:value-type="string" office:string-value="{big}"/></table:table-row>"#
    ));
    // 16 MiB of `x` deflates ~1000:1; switch the ratio guard off so the
    // string cap itself is exercised.
    let limits = Limits {
        max_ratio: u32::MAX,
        ..Limits::default()
    };
    let (r, _) = timed(&ods("", &body), &limits);
    assert_eq!(limit_what(&r), "cell text bytes");
}

/// H1: the text budget counts string values too, per materialized cell.
#[test]
fn text_budget_counts_repeated_strings() {
    let body = sheet(
        r#"<table:table-row table:number-rows-repeated="100"><table:table-cell office:value-type="string" table:number-columns-repeated="100"><text:p>0123456789</text:p></table:table-cell></table:table-row>"#,
    );
    let tight = Limits {
        max_text_bytes: 99_999,
        ..Limits::default()
    };
    let (r, _) = timed(&ods("", &body), &tight);
    assert_eq!(limit_what(&r), "materialized cell text bytes");
    let fits = Limits {
        max_text_bytes: 100_000,
        ..Limits::default()
    };
    assert!(read_ods(Cursor::new(ods("", &body)), &fits).is_ok());
}

/// H2 (reader side): the sheet count is capped (10,000 by default).
#[test]
fn sheet_count_is_capped() {
    let body = "<table:table/>".repeat(10_001);
    let (r, t) = timed(&ods("", &body), &Limits::default());
    assert_eq!(limit_what(&r), "sheets");
    assert!(t < Duration::from_secs(2), "took {t:?}");
    let body = "<table:table/>".repeat(10_000);
    assert_eq!(
        read_ods(Cursor::new(ods("", &body)), &Limits::default())
            .unwrap()
            .sheets
            .len(),
        10_000
    );
}

/// M4: 60,000 distinct automatic styles (each used once) dedupe in linear
/// time (3.2 s before the fix).
#[test]
fn many_distinct_styles_are_fast() {
    const N: usize = 60_000;
    let mut styles = String::new();
    let mut rows = String::new();
    for i in 0..N {
        styles.push_str(&format!(
            r#"<style:style style:name="c{i}" style:family="table-cell"><style:text-properties fo:font-size="{}pt"/></style:style>"#,
            1.0 + i as f64 / 100.0
        ));
        rows.push_str(&format!(
            r#"<table:table-row><table:table-cell table:style-name="c{i}" office:value-type="float" office:value="1"/></table:table-row>"#
        ));
    }
    let bytes = ods(&styles, &sheet(&rows));
    let (r, t) = timed(&bytes, &Limits::default());
    let wb = r.unwrap();
    assert_eq!(wb.styles.len(), N);
    // Debug builds parse the 15 MB of XML itself several times slower; the
    // dedup step is what this guards (quadratic: seconds even in release).
    eprintln!("TIMING {N} styles: {t:?}");
    let budget = if cfg!(debug_assertions) {
        Duration::from_millis(5000)
    } else {
        Duration::from_millis(500)
    };
    assert!(t < budget, "took {t:?}");
}

/// A hand-built zip64 like the review's `many2m.ods`: one stored local entry
/// and `n` central-directory records all pointing at it.
fn zip64_flood(n: u64) -> Vec<u8> {
    let mut out = Vec::new();
    // Local file header for an empty stored entry named "x".
    out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
    out.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    out.extend_from_slice(&[0; 12]);
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.push(b'x');
    let cd_off = out.len() as u64;
    for i in 0..n {
        let name = format!("{i:x}");
        out.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        out.extend_from_slice(&45u16.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes());
        out.extend_from_slice(&[0; 16]);
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&[0; 12]);
        out.extend_from_slice(name.as_bytes());
    }
    let cd_size = out.len() as u64 - cd_off;
    let z64_off = out.len() as u64;
    out.extend_from_slice(&0x0606_4b50u32.to_le_bytes());
    out.extend_from_slice(&44u64.to_le_bytes());
    out.extend_from_slice(&45u16.to_le_bytes());
    out.extend_from_slice(&45u16.to_le_bytes());
    out.extend_from_slice(&[0; 8]);
    out.extend_from_slice(&n.to_le_bytes());
    out.extend_from_slice(&n.to_le_bytes());
    out.extend_from_slice(&cd_size.to_le_bytes());
    out.extend_from_slice(&cd_off.to_le_bytes());
    out.extend_from_slice(&0x0706_4b50u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&z64_off.to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&[0, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF]);
    out.extend_from_slice(&[0xFF; 8]);
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

/// M5: a zip64 central-directory flood is rejected from the EOCD records,
/// before the zip crate indexes anything (822 MB RSS for 2M records before).
#[test]
fn zip64_entry_flood_is_rejected_before_indexing() {
    let bytes = zip64_flood(20_000);
    let (r, t) = timed(&bytes, &Limits::default());
    eprintln!("TIMING zip64 flood: {t:?}");
    assert_eq!(limit_what(&r), "zip entries");
    assert!(t < Duration::from_millis(50), "took {t:?}");
    // Sniffing never builds the archive either.
    let t = Instant::now();
    assert!(!sniff_is_ods(&bytes));
    assert!(t.elapsed() < Duration::from_millis(50), "{:?}", t.elapsed());
}

/// M5: a classic EOCD that declares a huge central directory is rejected.
#[test]
fn huge_central_directory_is_rejected() {
    let mut bytes = ods("", &sheet(""));
    let n = bytes.len();
    // EOCD (no comment): central-directory size at offset 12.
    bytes[n - 22 + 12..n - 22 + 16].copy_from_slice(&(64u32 << 20).to_le_bytes());
    let (r, _) = timed(&bytes, &Limits::default());
    assert_eq!(limit_what(&r), "zip central directory bytes");
}

/// M5: an EOCD that lies low about the entry count still parses as the zip
/// crate sees it (it trusts the count), so nothing past the cap is indexed.
#[test]
fn entry_cap_uses_declared_count() {
    let bytes = ods("", &sheet(""));
    let tight = Limits {
        max_entries: 1,
        ..Limits::default()
    };
    let (r, _) = timed(&bytes, &tight);
    assert_eq!(limit_what(&r), "zip entries");
}

/// L1: absurd years no longer overflow the day arithmetic (a debug-build
/// panic before); the cell falls back to its text.
#[test]
fn huge_date_years_do_not_panic() {
    for y in [
        "9223372036854775807-01-01",
        "100000000000000000-01-01",
        "-9223372036854775807-01-01",
        "100000-01-01",
    ] {
        let body = sheet(&format!(
            r#"<table:table-row><table:table-cell office:value-type="date" office:date-value="{y}"><text:p>d</text:p></table:table-cell></table:table-row>"#
        ));
        let wb = read_ods(Cursor::new(ods("", &body)), &Limits::default()).unwrap();
        assert_eq!(
            value_at(&wb, 0, 0),
            &CellContentData::Value(Value::text("d"))
        );
    }
    let body = sheet(
        r#"<table:table-row><table:table-cell office:value-type="date" office:date-value="99999-12-31"/></table:table-row>"#,
    );
    let wb = read_ods(Cursor::new(ods("", &body)), &Limits::default()).unwrap();
    assert!(
        matches!(value_at(&wb, 0, 0), CellContentData::Value(Value::Number(n)) if n.is_finite())
    );
}

/// L3: merges count against `max_cells`.
#[test]
fn merges_count_against_cell_cap() {
    let row = r#"<table:table-row><table:table-cell table:number-columns-spanned="2"/><table:covered-table-cell/></table:table-row>"#;
    let body = sheet(&row.repeat(11));
    let limits = Limits {
        max_cells: 10,
        ..Limits::default()
    };
    let (r, _) = timed(&ods("", &body), &limits);
    assert_eq!(limit_what(&r), "materialized cells");
    let body = sheet(&row.repeat(10));
    let wb = read_ods(Cursor::new(ods("", &body)), &limits).unwrap();
    assert_eq!(wb.sheets[0].merges.len(), 10);
}

/// L4: NaN and infinities from `office:value` become empty cells.
#[test]
fn non_finite_numbers_become_empty() {
    let cells: String = ["NaN", "inf", "-inf", "1e309", "2.5"]
        .iter()
        .map(|v| {
            format!(r#"<table:table-cell office:value-type="float" office:value="{v}" table:style-name="x"/>"#)
        })
        .collect();
    let styles = r#"<style:style style:name="x" style:family="table-cell"><style:text-properties fo:font-weight="bold"/></style:style>"#;
    let body = sheet(&format!("<table:table-row>{cells}</table:table-row>"));
    let wb = read_ods(Cursor::new(ods(styles, &body)), &Limits::default()).unwrap();
    for c in 0..4 {
        assert_eq!(
            value_at(&wb, 0, c),
            &CellContentData::Value(Value::Empty),
            "col {c}"
        );
    }
    assert_eq!(
        value_at(&wb, 0, 4),
        &CellContentData::Value(Value::Number(2.5))
    );
    // Cached formula results too.
    let body = sheet(
        r#"<table:table-row><table:table-cell table:formula="of:=1/0" office:value-type="float" office:value="inf"/></table:table-row>"#,
    );
    let wb = read_ods(Cursor::new(ods("", &body)), &Limits::default()).unwrap();
    assert!(matches!(
        value_at(&wb, 0, 0),
        CellContentData::Formula { cached: None, .. }
    ));
}

/// M5: sniffing reads the first local header, and otherwise only an
/// archive that passes the EOCD precheck.
#[test]
fn sniff_is_bounded() {
    assert!(sniff_is_ods(&ods("", &sheet(""))));
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    zip.start_file("content.xml", stored).unwrap();
    zip.write_all(b"<x/>").unwrap();
    zip.start_file("mimetype", stored).unwrap();
    zip.write_all(tally_io_ods::MIMETYPE.as_bytes()).unwrap();
    let bytes = zip.finish().unwrap().into_inner();
    assert!(sniff_is_ods(&bytes));
    let flood = zip64_flood(200_000);
    let t = Instant::now();
    assert!(!sniff_is_ods(&flood));
    let took = t.elapsed();
    eprintln!("TIMING sniff flood: {took:?}");
    assert!(took < Duration::from_millis(50), "{took:?}");
}
