//! Regression tests for the fourth security and correctness reviews
//! (`docs/reviews/security-review-4-2026-10-21.md`,
//! `docs/reviews/correctness-review-4-2026-10-21.md`), XLSX side:
//! C1 (one media part shown by many anchors was copied per anchor and
//! counted once), C2 (`numLit` padded to the largest `idx`, the shared
//! chart part re-parsed per anchor), H6 (long chart ranges cached and
//! cloned per series on save) and H9 (charts and pictures at the same
//! anchor overwrote each other on save). Inputs are generated here,
//! after the review's `gen5x.py`.

#![allow(clippy::unwrap_used)] // tests may unwrap (docs/contributing.md)

use std::io::{Cursor, Read, Write};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tally_io_xlsx::{read_xlsx, write_xlsx, Limits};
use tally_model::{
    CellContentData, CellData, CellRangeLocal, ChartAnchorData, ChartData, ChartKindData,
    ChartRangeData, ChartSeriesData, ChartValuesData, ImageData, SheetData, WorkbookData,
    MAX_LITERAL_POINTS, MAX_SERIES_PER_DOCUMENT, MAX_TOTAL_IMAGE_BYTES,
};
use tally_types::Value;

const NS: &str = r#"xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;
const RELS_NS: &str = r#"xmlns="http://schemas.openxmlformats.org/package/2006/relationships""#;
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const XDR: &str = r#"xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01\x08\x06\x00\x00\x00\x1f\x15\xc4\x89\x00\x00\x00\rIDATx\x9cc\xf8\xcf\xc0\xf0\x1f\x00\x05\x00\x02\x00\xde\xf2\x8a\xbb\x00\x00\x00\x00IEND\xaeB`\x82";

fn rels(items: &[(&str, &str, &str)]) -> String {
    let mut s = format!(r#"<?xml version="1.0"?><Relationships {RELS_NS}>"#);
    for (id, t, target) in items {
        s.push_str(&format!(
            r#"<Relationship Id="{id}" Type="{R}/{t}" Target="{target}"/>"#
        ));
    }
    s.push_str("</Relationships>");
    s
}

/// A one-sheet package whose sheet has `drawing` (anchors) plus `files`.
fn package(drawing: &str, files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default();
    let mut entries: Vec<(String, Vec<u8>)> = vec![
        (
            "xl/workbook.xml".into(),
            format!(r#"<workbook {NS}><sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets></workbook>"#).into_bytes(),
        ),
        (
            "xl/_rels/workbook.xml.rels".into(),
            rels(&[("rId1", "worksheet", "worksheets/sheet1.xml")]).into_bytes(),
        ),
        (
            "xl/worksheets/sheet1.xml".into(),
            format!(r#"<worksheet {NS}><sheetData><row r="1"><c r="A1"><v>1</v></c><c r="B1"><v>2</v></c></row></sheetData><drawing r:id="rIdD"/></worksheet>"#).into_bytes(),
        ),
        (
            "xl/worksheets/_rels/sheet1.xml.rels".into(),
            rels(&[("rIdD", "drawing", "../drawings/drawing1.xml")]).into_bytes(),
        ),
        (
            "xl/drawings/drawing1.xml".into(),
            format!(r#"<?xml version="1.0"?><xdr:wsDr {XDR}>{drawing}</xdr:wsDr>"#).into_bytes(),
        ),
    ];
    for (n, d) in files {
        entries.push(((*n).to_string(), d.to_vec()));
    }
    for (name, data) in entries {
        z.start_file(name, opts).unwrap();
        z.write_all(&data).unwrap();
    }
    z.finish().unwrap().into_inner()
}

fn anchor(i: usize, inner: &str) -> String {
    let r = i % 100;
    format!(
        "<xdr:twoCellAnchor><xdr:from><xdr:col>0</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>{r}</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from><xdr:to><xdr:col>3</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>{}</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:to>{inner}<xdr:clientData/></xdr:twoCellAnchor>",
        r + 5
    )
}

fn chart_frames(n: usize) -> String {
    (0..n)
        .map(|i| anchor(i, &format!(r#"<xdr:graphicFrame><xdr:nvGraphicFramePr><xdr:cNvPr id="{}" name="Chart {i}"/><xdr:cNvGraphicFramePr/></xdr:nvGraphicFramePr><xdr:xfrm/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart r:id="rIdC"/></a:graphicData></a:graphic></xdr:graphicFrame>"#, i + 2)))
        .collect()
}

fn read(bytes: Vec<u8>) -> (WorkbookData, Duration) {
    let t = Instant::now();
    let r = read_xlsx(Cursor::new(bytes), &Limits::default()).unwrap();
    (r, t.elapsed())
}

/// C1 (`gen5x.py imgdup … 1000`): 1,000 anchors → one media part. The
/// bytes are read once and shared; every anchor counts against 256 MiB.
#[test]
fn shared_media_is_read_once_and_counted_per_anchor() {
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.resize(1024 * 1024, 0);
    let drawing: String = (0..1000)
        .map(|i| anchor(i, &format!(r#"<xdr:pic><xdr:nvPicPr><xdr:cNvPr id="{}" name="Pic {i}"/><xdr:cNvPicPr/></xdr:nvPicPr><xdr:blipFill><a:blip r:embed="rIdI"/></xdr:blipFill><xdr:spPr/></xdr:pic>"#, i + 2)))
        .collect();
    let drels = rels(&[("rIdI", "image", "../media/image1.png")]);
    let (wb, took) = read(package(
        &drawing,
        &[
            ("xl/media/image1.png", &png),
            ("xl/drawings/_rels/drawing1.xml.rels", drels.as_bytes()),
        ],
    ));
    assert!(took < Duration::from_secs(10), "{took:?}");
    let images = &wb.sheets[0].images;
    assert_eq!(images.len(), MAX_TOTAL_IMAGE_BYTES / png.len());
    assert!(images
        .iter()
        .all(|i| Arc::ptr_eq(&i.bytes, &images[0].bytes)));
    assert!(
        wb.warnings.iter().any(|w| w.contains("256 MiB")),
        "{:?}",
        wb.warnings
    );
}

/// C2 (`gen5x.py chartdup … 1000 32000 255`): 255 `numLit` series with
/// one `<c:pt idx="31999">` each, shown by 1,000 anchors, was killed at
/// 3.7 GB. A literal holds at most a small multiple of its points, the
/// part is parsed once, and the document budgets hold.
#[test]
fn literal_padding_is_bounded_and_shared_parts_are_budgeted() {
    let sers: String = (0..255)
        .map(|k| format!(r#"<c:ser><c:idx val="{k}"/><c:order val="{k}"/><c:val><c:numLit><c:ptCount val="32000"/><c:pt idx="31999"><c:v>1.5</c:v></c:pt></c:numLit></c:val></c:ser>"#))
        .collect();
    let cx = format!(
        r#"<?xml version="1.0"?><c:chartSpace {XDR}><c:chart><c:plotArea><c:barChart><c:barDir val="col"/>{sers}</c:barChart></c:plotArea></c:chart></c:chartSpace>"#
    );
    let drels = rels(&[("rIdC", "chart", "../charts/chart1.xml")]);
    let (wb, took) = read(package(
        &chart_frames(1000),
        &[
            ("xl/charts/chart1.xml", cx.as_bytes()),
            ("xl/drawings/_rels/drawing1.xml.rels", drels.as_bytes()),
        ],
    ));
    assert!(took < Duration::from_secs(20), "{took:?}");
    let charts = &wb.sheets[0].charts;
    assert_eq!(charts.len(), 1000);
    let pts: usize = charts.iter().map(ChartData::literal_points).sum();
    assert!(pts <= MAX_LITERAL_POINTS, "{pts}");
    let series: usize = charts.iter().map(|c| c.series.len()).sum();
    assert!(series <= MAX_SERIES_PER_DOCUMENT, "{series}");
    // `idx` 31999 with one point is past 4 × 1 + 64 slots: dropped.
    assert!(charts[0]
        .series
        .iter()
        .all(|s| s.literal_values.as_ref().is_none_or(|v| v.len() <= 68)));
}

/// A dense `numLit` keeps every point at its index.
#[test]
fn dense_literals_keep_their_indices() {
    let pts: String = (0..100)
        .filter(|i| i % 3 != 1)
        .map(|i| format!(r#"<c:pt idx="{i}"><c:v>{i}</c:v></c:pt>"#))
        .collect();
    let cx = format!(
        r#"<?xml version="1.0"?><c:chartSpace {XDR}><c:chart><c:plotArea><c:barChart><c:barDir val="col"/><c:ser><c:idx val="0"/><c:order val="0"/><c:val><c:numLit><c:ptCount val="100"/>{pts}</c:numLit></c:val></c:ser></c:barChart></c:plotArea></c:chart></c:chartSpace>"#
    );
    let drels = rels(&[("rIdC", "chart", "../charts/chart1.xml")]);
    let (wb, _) = read(package(
        &chart_frames(1),
        &[
            ("xl/charts/chart1.xml", cx.as_bytes()),
            ("xl/drawings/_rels/drawing1.xml.rels", drels.as_bytes()),
        ],
    ));
    match &wb.sheets[0].charts[0].series[0].literal_values {
        Some(ChartValuesData::Literal(v)) => {
            assert_eq!(v.len(), 100);
            assert_eq!(v[99], Some(99.0));
            assert_eq!(v[1], None);
        }
        other => panic!("{other:?}"),
    }
}

fn area(r0: u32, c0: u32, r1: u32, c1: u32) -> ChartRangeData {
    ChartRangeData {
        areas: vec![(0, CellRangeLocal { r0, c0, r1, c1 })],
        invalid: false,
    }
}

fn chart(name: &str, kind: ChartKindData, values: ChartRangeData, n: usize) -> ChartData {
    ChartData {
        name: name.into(),
        anchor: ChartAnchorData {
            row: 1,
            col: 5,
            dx_hmm: 0,
            dy_hmm: 0,
            width_hmm: 16_000,
            height_hmm: 9_000,
        },
        kind,
        categories: Some(area(1, 0, 5, 0)),
        series: (0..n)
            .map(|_| ChartSeriesData {
                values: values.clone(),
                ..ChartSeriesData::default()
            })
            .collect(),
        ..ChartData::default()
    }
}

fn sheet(cells: Vec<CellData>) -> SheetData {
    SheetData {
        name: "Sheet1".into(),
        cells,
        ..SheetData::default()
    }
}

fn num(row: u32, col: u32, v: f64) -> CellData {
    CellData {
        row,
        col,
        content: CellContentData::Value(Value::Number(v)),
        style: None,
    }
}

fn write(wb: &WorkbookData) -> (Vec<u8>, Vec<String>) {
    let mut buf = Cursor::new(Vec::new());
    let report = write_xlsx(wb, &mut buf).unwrap();
    (buf.into_inner(), report.warnings)
}

fn part(bytes: &[u8], name: &str) -> String {
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut s = String::new();
    z.by_name(name).unwrap().read_to_string(&mut s).unwrap();
    s
}

/// H9 (`c4_chpr::ch09_xlsx_same_anchor_cell`): two charts (and two
/// pictures) at the same anchor became one on save. Each is written,
/// later ones one pixel to the right.
#[test]
fn objects_sharing_an_anchor_are_all_written() {
    let mut cells = Vec::new();
    for r in 1..=5 {
        cells.push(num(r, 1, f64::from(r)));
        cells.push(num(r, 2, f64::from(r * 2)));
    }
    let mut s = sheet(cells);
    s.charts = vec![
        chart("A", ChartKindData::Column, area(1, 1, 5, 1), 1),
        chart("B", ChartKindData::Pie, area(1, 2, 5, 2), 1),
        chart("C", ChartKindData::Line, area(1, 2, 5, 2), 1),
    ];
    let img = ImageData {
        name: "Logo".into(),
        anchor: ChartAnchorData {
            row: 3,
            col: 3,
            dx_hmm: 0,
            dy_hmm: 0,
            width_hmm: 2000,
            height_hmm: 1500,
        },
        mime: "image/png".into(),
        bytes: PNG.into(),
    };
    s.images = vec![
        img.clone(),
        ImageData {
            name: "Logo 2".into(),
            ..img
        },
    ];
    let wb = WorkbookData {
        sheets: vec![s],
        ..WorkbookData::default()
    };
    let (bytes, _) = write(&wb);
    let back = read_xlsx(Cursor::new(bytes), &Limits::default()).unwrap();
    let charts = &back.sheets[0].charts;
    assert_eq!(charts.len(), 3);
    let mut kinds: Vec<_> = charts.iter().map(|c| format!("{:?}", c.kind)).collect();
    kinds.sort();
    assert_eq!(kinds, ["Column", "Line", "Pie"]);
    assert!(charts
        .iter()
        .all(|c| (c.anchor.row, c.anchor.col) == (1, 5)));
    assert_eq!(back.sheets[0].images.len(), 2);
}

/// H6: a chart of 8 series over `B1:B1048576` was cached and cloned per
/// series on save (8 series × 1M rows: killed at 3 GB). Long ranges are
/// written as bare references, short ones keep their cache, and the
/// ranges read back unchanged.
#[test]
fn long_chart_ranges_are_saved_without_caches() {
    let mut cells = Vec::new();
    for r in 0..2000 {
        cells.push(num(r, 1, f64::from(r)));
    }
    let mut s = sheet(cells);
    let long = area(0, 1, 1_048_575, 1);
    s.charts = vec![
        chart("Long", ChartKindData::Line, long.clone(), 8),
        chart("Short", ChartKindData::Line, area(0, 1, 9, 1), 1),
    ];
    s.charts[1].anchor.row = 30;
    let wb = WorkbookData {
        sheets: vec![s],
        ..WorkbookData::default()
    };
    let t = Instant::now();
    let (bytes, warnings) = write(&wb);
    assert!(t.elapsed() < Duration::from_secs(20), "{:?}", t.elapsed());
    assert!(
        warnings.iter().any(|w| w.contains("without cached values")),
        "{warnings:?}"
    );
    let mut long_xml = String::new();
    let mut short_xml = String::new();
    for name in ["xl/charts/chart1.xml", "xl/charts/chart2.xml"] {
        let x = part(&bytes, name);
        if x.contains("$B$1048576") {
            long_xml = x;
        } else {
            short_xml = x;
        }
    }
    assert_eq!(
        long_xml
            .matches("<c:val><c:numRef><c:f>(Sheet1!$B$1:$B$1048576)</c:f></c:numRef></c:val>")
            .count(),
        8,
        "{long_xml}"
    );
    assert!(short_xml.contains("<c:numCache>"), "{short_xml}");
    let back = read_xlsx(Cursor::new(bytes), &Limits::default()).unwrap();
    let c = back.sheets[0]
        .charts
        .iter()
        .find(|c| c.series.len() == 8)
        .unwrap();
    assert!(c.series.iter().all(|s| s.values == long));
}
