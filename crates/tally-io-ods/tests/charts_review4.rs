//! Regression tests for the fourth security review
//! (`docs/reviews/security-review-4-2026-10-21.md`), ODS reader side:
//! C1 (one picture shown by many frames was copied per frame and counted
//! once against the 256 MiB total) and C2 (`local-table` data padded to
//! 32,000 rows × 255 series, and the shared chart object re-parsed per
//! frame). Inputs are generated here, after the review's `gen5.py`.

#![allow(clippy::unwrap_used)] // tests may unwrap (docs/contributing.md)

use std::fmt::Write as _;
use std::io::{Cursor, Write};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tally_io_ods::{read_ods, Limits};
use tally_model::{
    ChartValuesData, WorkbookData, MAX_LITERAL_POINTS, MAX_SERIES_PER_DOCUMENT,
    MAX_TOTAL_IMAGE_BYTES,
};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

const NS: &str = r#"xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:xlink="http://www.w3.org/1999/xlink" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0" xmlns:chart="urn:oasis:names:tc:opendocument:xmlns:chart:1.0""#;

const CHART_MEDIA: &str = "application/vnd.oasis.opendocument.chart";

/// A package with one sheet holding `shapes` (frames) and one value,
/// the manifest `entries` and the extra zip `files`.
fn package(shapes: &str, entries: &[(&str, &str)], files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    zip.start_file("mimetype", stored).unwrap();
    zip.write_all(tally_io_ods::MIMETYPE.as_bytes()).unwrap();
    let mut man = String::from(
        r#"<?xml version="1.0"?><manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.3"><manifest:file-entry manifest:full-path="/" manifest:media-type="application/vnd.oasis.opendocument.spreadsheet"/><manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/>"#,
    );
    for (p, m) in entries {
        let _ = write!(
            man,
            r#"<manifest:file-entry manifest:full-path="{p}" manifest:media-type="{m}"/>"#
        );
    }
    man.push_str("</manifest:manifest>");
    zip.start_file("META-INF/manifest.xml", deflated).unwrap();
    zip.write_all(man.as_bytes()).unwrap();
    zip.start_file("content.xml", deflated).unwrap();
    write!(
        zip,
        r#"<?xml version="1.0" encoding="UTF-8"?><office:document-content {NS} office:version="1.2"><office:body><office:spreadsheet><table:table table:name="Sheet1"><table:shapes>{shapes}</table:shapes><table:table-row><table:table-cell office:value-type="float" office:value="1"><text:p>1</text:p></table:table-cell></table:table-row></table:table></office:spreadsheet></office:body></office:document-content>"#
    )
    .unwrap();
    for (name, data) in files {
        zip.start_file(*name, deflated).unwrap();
        zip.write_all(data).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

fn read(bytes: Vec<u8>) -> (WorkbookData, Duration) {
    let t = Instant::now();
    let r = read_ods(Cursor::new(bytes), &Limits::default()).unwrap();
    (r, t.elapsed())
}

/// A chart object `content.xml` with `series` and a `local-table`.
fn chart_xml(series: &str, local_table: &str) -> String {
    format!(
        r#"<?xml version="1.0"?><office:document-content {NS}><office:body><office:chart><chart:chart chart:class="chart:bar"><chart:plot-area>{series}</chart:plot-area>{local_table}</chart:chart></office:chart></office:body></office:document-content>"#
    )
}

fn object_frames(n: usize) -> String {
    (0..n)
        .map(|i| {
            format!(
                r#"<draw:frame draw:name="c{i}" svg:width="5cm" svg:height="5cm" svg:x="0cm" svg:y="0cm"><draw:object xlink:href="./Object 1"/></draw:frame>"#
            )
        })
        .collect()
}

/// C1 (`gen5.py imgdup … 1000 1 1048576`): 1,000 frames → one 1 MiB PNG
/// took 1.33 GB (a copy per frame). The bytes are read once and shared,
/// and every frame counts against the 256 MiB total.
#[test]
fn shared_picture_is_read_once_and_counted_per_frame() {
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.resize(1024 * 1024, 0);
    let frames: String = (0..1000)
        .map(|i| {
            format!(
                r#"<draw:frame draw:name="p{i}" svg:width="1cm" svg:height="1cm" svg:x="0cm" svg:y="0cm"><draw:image xlink:href="Pictures/a.png"/></draw:frame>"#
            )
        })
        .collect();
    let (wb, took) = read(package(
        &frames,
        &[("Pictures/a.png", "image/png")],
        &[("Pictures/a.png", &png)],
    ));
    assert!(took < Duration::from_secs(10), "{took:?}");
    let images = &wb.sheets[0].images;
    assert_eq!(images.len(), MAX_TOTAL_IMAGE_BYTES / png.len());
    assert!(images
        .iter()
        .all(|i| Arc::ptr_eq(&i.bytes, &images[0].bytes)));
    let referenced: usize = images.iter().map(|i| i.bytes.len()).sum();
    assert!(referenced <= MAX_TOTAL_IMAGE_BYTES);
    assert!(
        wb.warnings.iter().any(|w| w.contains("256 MiB")),
        "{:?}",
        wb.warnings
    );
}

/// C2 (`gen5.py localbomb … 1000 32000`): 255 range-less series and a
/// `local-table` of 32,000 empty rows, shown by 1,000 frames, was killed
/// at 2.9 GB (≈ 130 GB projected). Empty rows hold nothing, and the
/// object is parsed once.
#[test]
fn empty_local_table_rows_hold_no_points() {
    let rows: String = (0..32_000)
        .map(|i| format!(r#"<table:table-row table:style-name="r{i}"/>"#))
        .collect();
    let lt = format!(
        r#"<table:table table:name="local-table"><table:table-rows>{rows}</table:table-rows></table:table>"#
    );
    let cx = chart_xml(&"<chart:series/>".repeat(255), &lt);
    let (wb, took) = read(package(
        &object_frames(1000),
        &[("Object 1/", CHART_MEDIA)],
        &[("Object 1/content.xml", cx.as_bytes())],
    ));
    assert!(took < Duration::from_secs(20), "{took:?}");
    let charts = &wb.sheets[0].charts;
    assert_eq!(charts.len(), 1000);
    assert_eq!(charts.iter().map(|c| c.literal_points()).sum::<usize>(), 0);
    let series: usize = charts.iter().map(|c| c.series.len()).sum();
    assert!(series <= MAX_SERIES_PER_DOCUMENT, "{series}");
    assert!(wb
        .warnings
        .iter()
        .any(|w| w.contains("series per document")));
}

/// C2: a `local-table` whose last row has a value in every column keeps
/// one point per row and series, within the document's literal budget,
/// counted again for every frame that shows the object.
#[test]
fn local_table_values_are_budgeted_per_document() {
    let mut rows = String::from(r#"<table:table-row/>"#).repeat(31_999);
    rows.push_str("<table:table-row>");
    rows.push_str(
        r#"<table:table-cell office:value-type="string"><text:p>last</text:p></table:table-cell>"#,
    );
    for _ in 0..40 {
        rows.push_str(r#"<table:table-cell office:value-type="float" office:value="7"><text:p>7</text:p></table:table-cell>"#);
    }
    rows.push_str("</table:table-row>");
    let lt = format!(
        r#"<table:table table:name="local-table"><table:table-rows>{rows}</table:table-rows></table:table>"#
    );
    let cx = chart_xml(&"<chart:series/>".repeat(40), &lt);
    let (wb, took) = read(package(
        &object_frames(10),
        &[("Object 1/", CHART_MEDIA)],
        &[("Object 1/content.xml", cx.as_bytes())],
    ));
    assert!(took < Duration::from_secs(20), "{took:?}");
    let charts = &wb.sheets[0].charts;
    assert_eq!(charts.len(), 10);
    let total: usize = charts.iter().map(|c| c.literal_points()).sum();
    assert!(total <= MAX_LITERAL_POINTS, "{total}");
    // The first chart keeps its values (31 series × 32,000 fit).
    let first = &charts[0].series[0];
    match &first.literal_values {
        Some(ChartValuesData::Literal(v)) => {
            assert_eq!(v.len(), 32_000);
            assert_eq!(v[31_999], Some(7.0));
        }
        other => panic!("{other:?}"),
    }
    assert!(charts[9].literal_points() == 0);
    assert!(
        wb.warnings.iter().any(|w| w.contains("cached values")),
        "{:?}",
        wb.warnings
    );
}
