//! Charts and pictures in ODS (wave 5, W5-1): write → read round trips
//! per chart kind and feature, the LibreOffice-shaped fixture, and the
//! hardening of the object reader.

#![allow(clippy::unwrap_used)] // tests may unwrap (docs/contributing.md)

mod common;

use std::io::{Cursor, Write};
use std::time::{Duration, Instant};

use common::read_fixture;
use tally_io_ods::{read_ods, write_ods, write_ods_with_report, Limits, OdsError};
use tally_model::{
    CellContentData, CellData, CellRangeLocal, ChartAnchorData, ChartAxisData, ChartData,
    ChartKindData, ChartRangeData, ChartSeriesData, ChartSourceData, ChartValuesData,
    DataLabelsData, ErrorBarDirData, ErrorBarKindData, ErrorBarsData, ImageData, LabelPosData,
    LegendPosData, MarkerShapeData, SeriesNameData, SheetData, StackingData, StockVariantData,
    TrendKindData, TrendLineData, WorkbookData,
};
use tally_types::Value;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01\x08\x06\x00\x00\x00\x1f\x15\xc4\x89\x00\x00\x00\rIDATx\x9cc\xf8\xcf\xc0\xf0\x1f\x00\x05\x00\x02\x00\xde\xf2\x8a\xbb\x00\x00\x00\x00IEND\xaeB`\x82";

fn cell(row: u32, col: u32, v: Value) -> CellData {
    CellData {
        row,
        col,
        content: CellContentData::Value(v),
        style: None,
    }
}

/// `Data` sheet: A1:C6 = Month/Sales/Costs with five rows.
fn data_sheet() -> SheetData {
    let mut cells = vec![
        cell(0, 0, Value::text("Month")),
        cell(0, 1, Value::text("Sales")),
        cell(0, 2, Value::text("Costs")),
    ];
    for (i, (m, a, b)) in [
        ("Jan", 10.0, 4.0),
        ("Feb", 20.0, 8.0),
        ("Mar", 35.0, 12.0),
        ("Apr", 50.0, 20.0),
        ("May", 100.0, 40.0),
    ]
    .iter()
    .enumerate()
    {
        let r = i as u32 + 1;
        cells.push(cell(r, 0, Value::text(*m)));
        cells.push(cell(r, 1, Value::Number(*a)));
        cells.push(cell(r, 2, Value::Number(*b)));
    }
    SheetData {
        name: "Data".into(),
        cells,
        ..SheetData::default()
    }
}

fn area(r0: u32, c0: u32, r1: u32, c1: u32) -> ChartRangeData {
    ChartRangeData {
        areas: vec![(0, CellRangeLocal { r0, c0, r1, c1 })],
        invalid: false,
    }
}

fn series(col: u32) -> ChartSeriesData {
    ChartSeriesData {
        name: Some(SeriesNameData::Ref(area(0, col, 0, col))),
        values: area(1, col, 5, col),
        ..ChartSeriesData::default()
    }
}

fn chart(kind: ChartKindData) -> ChartData {
    ChartData {
        name: "Object 1".into(),
        anchor: ChartAnchorData {
            row: 1,
            col: 4,
            dx_hmm: 300,
            dy_hmm: 100,
            width_hmm: 16_000,
            height_hmm: 9_000,
        },
        kind,
        categories: Some(area(1, 0, 5, 0)),
        series: vec![series(1), series(2)],
        ..ChartData::default()
    }
}

fn workbook(charts: Vec<ChartData>) -> WorkbookData {
    let mut sheet = data_sheet();
    sheet.charts = charts;
    WorkbookData {
        sheets: vec![sheet],
        ..WorkbookData::default()
    }
}

fn round_trip(wb: &WorkbookData) -> WorkbookData {
    let mut buf = Cursor::new(Vec::new());
    write_ods(wb, &mut buf).unwrap();
    read_ods(Cursor::new(buf.into_inner()), &Limits::default()).unwrap()
}

/// Writes, reads back and compares the chart (`source` is dropped from
/// the comparison when it was not set: the reader never invents one).
fn assert_round_trip(c: ChartData) -> ChartData {
    let wb = workbook(vec![c.clone()]);
    let back = round_trip(&wb);
    assert_eq!(back.sheets[0].charts.len(), 1, "{:?}", back.warnings);
    let got = back.sheets[0].charts[0].clone();
    assert_eq!(got, c);
    got
}

#[test]
fn roundtrip_column() {
    assert_round_trip(chart(ChartKindData::Column));
}

#[test]
fn roundtrip_column_stacked() {
    let mut c = chart(ChartKindData::Column);
    c.stacking = StackingData::Stacked;
    assert_round_trip(c);
}

#[test]
fn roundtrip_column_percent() {
    let mut c = chart(ChartKindData::Column);
    c.stacking = StackingData::Percent;
    assert_round_trip(c);
}

#[test]
fn roundtrip_bar() {
    assert_round_trip(chart(ChartKindData::Bar));
}

#[test]
fn roundtrip_line_points_lines() {
    let mut c = chart(ChartKindData::Line);
    c.markers = true;
    c.lines = true;
    assert_round_trip(c);
    let mut c = chart(ChartKindData::Line);
    c.markers = true;
    c.lines = false;
    assert_round_trip(c);
}

#[test]
fn roundtrip_line_smooth() {
    let mut c = chart(ChartKindData::Line);
    c.smooth = true;
    assert_round_trip(c);
}

#[test]
fn roundtrip_area_stacked() {
    let mut c = chart(ChartKindData::Area);
    c.stacking = StackingData::Stacked;
    assert_round_trip(c);
}

#[test]
fn roundtrip_pie() {
    let mut c = chart(ChartKindData::Pie);
    c.series.truncate(1);
    assert_round_trip(c);
}

#[test]
fn roundtrip_pie_exploded() {
    let mut c = chart(ChartKindData::Pie);
    c.series.truncate(1);
    c.exploded = true;
    assert_round_trip(c);
}

#[test]
fn roundtrip_donut() {
    let mut c = chart(ChartKindData::Pie);
    c.series.truncate(1);
    c.hole = Some(0.4);
    assert_round_trip(c);
}

#[test]
fn roundtrip_scatter_lines() {
    let mut c = chart(ChartKindData::Scatter);
    c.categories = None;
    c.markers = true;
    c.lines = true;
    for s in &mut c.series {
        s.x_values = Some(area(1, 1, 5, 1));
    }
    assert_round_trip(c);
}

#[test]
fn roundtrip_bubble() {
    let mut c = chart(ChartKindData::Bubble);
    c.categories = None;
    c.series.truncate(1);
    c.series[0].x_values = Some(area(1, 1, 5, 1));
    c.series[0].values = area(1, 2, 5, 2);
    c.series[0].sizes = Some(area(1, 1, 5, 1));
    assert_round_trip(c);
}

#[test]
fn roundtrip_radar() {
    let mut c = chart(ChartKindData::Radar);
    c.markers = true;
    assert_round_trip(c);
}

#[test]
fn roundtrip_radar_filled() {
    let mut c = chart(ChartKindData::Radar);
    c.filled = true;
    assert_round_trip(c);
}

fn stock(variant: StockVariantData, n: usize) -> ChartData {
    let mut c = chart(ChartKindData::Stock);
    c.stock = Some(variant);
    c.series = (0..n).map(|i| series(1 + (i as u32 % 2))).collect();
    c
}

#[test]
fn roundtrip_stock_hlc() {
    assert_round_trip(stock(StockVariantData::Hlc, 3));
}

#[test]
fn roundtrip_stock_ohlc() {
    assert_round_trip(stock(StockVariantData::Ohlc, 4));
}

#[test]
fn roundtrip_stock_volume_ohlc() {
    assert_round_trip(stock(StockVariantData::VolumeOhlc, 5));
    assert_round_trip(stock(StockVariantData::VolumeHlc, 4));
}

#[test]
fn roundtrip_combo_column_line() {
    let mut c = chart(ChartKindData::Column);
    c.series[1].kind = Some(ChartKindData::Line);
    assert_round_trip(c);
}

#[test]
fn roundtrip_secondary_axis() {
    let mut c = chart(ChartKindData::Line);
    c.series[1].secondary_axis = true;
    c.y2_axis = Some(ChartAxisData {
        title: Some("Costs".into()),
        log: true,
        min: Some(1.0),
        max: Some(1000.0),
        ..ChartAxisData::default()
    });
    assert_round_trip(c);
}

#[test]
fn roundtrip_trend_eq_r2() {
    let mut c = chart(ChartKindData::Line);
    for (i, kind) in [
        TrendKindData::Linear,
        TrendKindData::Logarithmic,
        TrendKindData::Exponential,
        TrendKindData::Power,
    ]
    .into_iter()
    .enumerate()
    {
        c.series[i % 2].trend = Some(TrendLineData {
            kind,
            show_equation: true,
            show_r2: i % 2 == 0,
            name: Some(format!("fit {i}")),
        });
        c = assert_round_trip(c);
    }
}

#[test]
fn roundtrip_error_bars_range() {
    let mut c = chart(ChartKindData::Column);
    c.series[0].error_bars = Some(ErrorBarsData {
        kind: ErrorBarKindData::Range {
            plus: area(1, 2, 5, 2),
            minus: area(1, 2, 5, 2),
        },
        direction: ErrorBarDirData::Both,
    });
    c.series[1].error_bars = Some(ErrorBarsData {
        kind: ErrorBarKindData::StdDev { k: 2.0 },
        direction: ErrorBarDirData::Plus,
    });
    let c = assert_round_trip(c);
    for kind in [
        ErrorBarKindData::Constant {
            plus: 1.5,
            minus: 0.5,
        },
        ErrorBarKindData::Percent { percent: 5.0 },
        ErrorBarKindData::Variance,
        ErrorBarKindData::StdError,
        ErrorBarKindData::ErrorMargin { percent: 10.0 },
    ] {
        let mut c = c.clone();
        c.series[0].error_bars = Some(ErrorBarsData {
            kind,
            direction: ErrorBarDirData::Minus,
        });
        assert_round_trip(c);
    }
}

#[test]
fn roundtrip_axis_log_min_max() {
    let mut c = chart(ChartKindData::Column);
    c.y_axis = ChartAxisData {
        visible: true,
        title: Some("k€".into()),
        min: Some(0.0),
        max: Some(120.0),
        major: Some(20.0),
        minor_count: Some(4),
        log: false,
        reverse: true,
        major_grid: true,
        minor_grid: true,
        number_format: Some("#,##0.00".into()),
        ..ChartAxisData::default()
    };
    c.x_axis.visible = false;
    c.x_axis.title = Some("Month".into());
    assert_round_trip(c);
}

#[test]
fn roundtrip_legend_positions() {
    for l in [
        LegendPosData::None,
        LegendPosData::Left,
        LegendPosData::Right,
        LegendPosData::Top,
        LegendPosData::Bottom,
    ] {
        let mut c = chart(ChartKindData::Column);
        c.legend = l;
        assert_round_trip(c);
    }
}

#[test]
fn roundtrip_titles() {
    let mut c = chart(ChartKindData::Column);
    c.title = Some("Sales & <Costs>".into());
    c.subtitle = Some("2026 \"Q4\"".into());
    c.background = Some(0xffff_ccff);
    c.plot_background = Some(0xe6e6_e6ff);
    c.series[0].color = Some(0x1122_33ff);
    c.series[1].name = Some(SeriesNameData::Text("Literal name".into()));
    c.include_hidden = true;
    assert_round_trip(c);
}

#[test]
fn roundtrip_data_labels() {
    let mut c = chart(ChartKindData::Pie);
    c.series.truncate(1);
    c.series[0].labels = DataLabelsData {
        value: true,
        percent: true,
        category: true,
        series_name: true,
        position: LabelPosData::Outside,
    };
    assert_round_trip(c);
    let mut c = chart(ChartKindData::Line);
    c.series[0].marker = Some(MarkerShapeData::Diamond);
    c.series[1].marker = Some(MarkerShapeData::None);
    c.series[1].labels.value = true;
    c.series[1].labels.position = LabelPosData::Above;
    c.series[1].mean_line = true;
    assert_round_trip(c);
}

#[test]
fn roundtrip_three_d_flag() {
    let mut c = chart(ChartKindData::Column);
    c.three_d = true;
    assert_round_trip(c);
}

#[test]
fn roundtrip_anchor_offsets_size() {
    let mut c = chart(ChartKindData::Column);
    c.anchor = ChartAnchorData {
        row: 10,
        col: 7,
        dx_hmm: 1234,
        dy_hmm: 56,
        width_hmm: 12_345,
        height_hmm: 6_789,
    };
    c.source = Some(ChartSourceData {
        range: area(0, 0, 5, 2),
        series_in_rows: false,
        first_row_label: true,
        first_col_label: true,
    });
    assert_round_trip(c);
}

#[test]
fn roundtrip_literal_values_via_local_table() {
    let mut c = chart(ChartKindData::Column);
    c.categories = None;
    c.literal_categories = Some(ChartValuesData::LiteralText(vec!["a".into(), "b".into()]));
    c.series = vec![ChartSeriesData {
        name: Some(SeriesNameData::Text("Lit".into())),
        literal_values: Some(ChartValuesData::Literal(vec![Some(1.0), None])),
        ..ChartSeriesData::default()
    }];
    assert_round_trip(c);
}

#[test]
fn roundtrip_two_sheets_cross_references() {
    let mut wb = workbook(vec![chart(ChartKindData::Column)]);
    let mut other = SheetData {
        name: "Other 'sheet'".into(),
        ..SheetData::default()
    };
    let mut c = chart(ChartKindData::Line);
    c.name = "Object 2".into();
    c.series[0].values.areas[0].0 = 1;
    other.charts = vec![c.clone()];
    other.cells = vec![cell(1, 1, Value::Number(1.0))];
    wb.sheets.push(other);
    let back = round_trip(&wb);
    assert_eq!(back.sheets[1].charts, vec![c]);
    assert_eq!(back.sheets[0].charts.len(), 1);
}

#[test]
fn roundtrip_images() {
    let mut wb = workbook(vec![chart(ChartKindData::Column)]);
    let img = ImageData {
        name: "Logo".into(),
        anchor: ChartAnchorData {
            row: 3,
            col: 3,
            dx_hmm: 10,
            dy_hmm: 20,
            width_hmm: 2000,
            height_hmm: 1500,
        },
        mime: "image/png".into(),
        bytes: PNG.into(),
    };
    wb.sheets[0].images = vec![
        img.clone(),
        ImageData {
            name: "Same bytes".into(),
            ..img.clone()
        },
    ];
    let back = round_trip(&wb);
    assert_eq!(back.sheets[0].images, wb.sheets[0].images);
    assert_eq!(back.sheets[0].charts.len(), 1);
}

#[test]
fn write_report_warns_on_too_many_series() {
    let mut c = chart(ChartKindData::Column);
    c.series = (0..300).map(|_| series(1)).collect();
    let mut buf = Cursor::new(Vec::new());
    let report = write_ods_with_report(&workbook(vec![c]), &mut buf).unwrap();
    assert!(
        report.warnings.iter().any(|w| w.contains("255")),
        "{:?}",
        report.warnings
    );
    let back = read_ods(Cursor::new(buf.into_inner()), &Limits::default()).unwrap();
    assert_eq!(back.sheets[0].charts[0].series.len(), 255);
}

// ---------------------------------------------------------------------------
// Fixture shaped like LibreOffice output
// ---------------------------------------------------------------------------

#[test]
fn reads_libreoffice_fixture() {
    let wb = read_ods(
        Cursor::new(read_fixture("ods/charts/libreoffice-charts.ods")),
        &Limits::default(),
    )
    .unwrap();
    let s = &wb.sheets[0];
    assert_eq!(s.charts.len(), 3, "{:?}", wb.warnings);
    let names: Vec<&str> = s.charts.iter().map(|c| c.name.as_str()).collect();
    assert!(
        names.contains(&"Object 1") && names.contains(&"Object 2") && names.contains(&"Object 3")
    );
    let col = s.charts.iter().find(|c| c.name == "Object 1").unwrap();
    assert_eq!(col.kind, ChartKindData::Column);
    assert_eq!(col.anchor.row, 1);
    assert_eq!(col.anchor.col, 3);
    assert_eq!((col.anchor.dx_hmm, col.anchor.dy_hmm), (300, 100));
    assert_eq!(
        (col.anchor.width_hmm, col.anchor.height_hmm),
        (16_000, 9_000)
    );
    assert_eq!(col.title.as_deref(), Some("Sales 2026"));
    assert_eq!(col.legend, LegendPosData::Right);
    assert_eq!(col.categories, Some(area(1, 0, 5, 0)));
    assert_eq!(col.series.len(), 2);
    assert_eq!(col.series[0].values, area(1, 1, 5, 1));
    assert_eq!(
        col.series[0].name,
        Some(SeriesNameData::Ref(area(0, 1, 0, 1)))
    );
    assert_eq!(col.series[0].color, Some(0x0045_86ff));
    assert!(col.series[1].labels.value);
    assert_eq!(col.series[1].labels.position, LabelPosData::Outside);
    assert_eq!(col.x_axis.title.as_deref(), Some("Month"));
    assert_eq!(col.y_axis.title.as_deref(), Some("k€"));
    assert!(col.y_axis.major_grid);
    assert_eq!(col.y_axis.max, Some(120.0));
    assert_eq!(col.y_axis.major, Some(20.0));
    assert_eq!(
        col.source
            .as_ref()
            .map(|s| (s.first_row_label, s.first_col_label)),
        Some((true, true))
    );
    assert!(!col.three_d);

    let pie = s.charts.iter().find(|c| c.name == "Object 2").unwrap();
    assert_eq!(pie.kind, ChartKindData::Pie);
    assert!(pie.three_d && pie.exploded);
    assert_eq!(pie.legend, LegendPosData::Bottom);
    assert_eq!(pie.background, Some(0xffff_ccff));
    assert!(pie.series[0].labels.percent && pie.series[0].labels.category);
    // Page anchored at (9cm, 6cm): default columns are 2258 hmm wide and
    // rows 452 hmm high.
    assert_eq!(pie.anchor.col, 3);
    assert_eq!(pie.anchor.dx_hmm, 9000 - 3 * 2258);
    assert_eq!(pie.anchor.row, 13);
    assert_eq!(pie.anchor.dy_hmm, 6000 - 13 * 452);

    let line = s.charts.iter().find(|c| c.name == "Object 3").unwrap();
    assert_eq!(line.kind, ChartKindData::Line);
    assert!(line.smooth && line.markers && line.lines);
    assert_eq!(line.title.as_deref(), Some("Trend"));
    assert_eq!(line.subtitle.as_deref(), Some("with regression"));
    assert_eq!(line.legend, LegendPosData::Top);
    assert!(line.x_axis.minor_grid && line.y_axis.major_grid);
    let y2 = line.y2_axis.as_ref().unwrap();
    assert!(y2.log);
    assert_eq!((y2.min, y2.max), (Some(1.0), Some(1000.0)));
    assert_eq!(line.series[0].marker, Some(MarkerShapeData::Diamond));
    let t = line.series[0].trend.as_ref().unwrap();
    assert_eq!(t.kind, TrendKindData::Linear);
    assert!(t.show_equation && t.show_r2);
    let e = line.series[0].error_bars.as_ref().unwrap();
    assert_eq!(e.kind, ErrorBarKindData::StdDev { k: 1.0 });
    assert!(line.series[0].mean_line);
    assert!(line.series[1].secondary_axis);
    assert_eq!(line.series[1].marker, Some(MarkerShapeData::None));

    assert_eq!(s.images.len(), 1);
    assert_eq!(s.images[0].mime, "image/png");
    assert_eq!(&*s.images[0].bytes, PNG);
    assert_eq!((s.images[0].anchor.row, s.images[0].anchor.col), (1, 5));
    // The Math object is skipped and counted.
    assert!(
        wb.warnings.iter().any(|w| w.contains("skipped")),
        "{:?}",
        wb.warnings
    );
    // And the whole thing round-trips.
    let back = round_trip(&wb);
    let by_name = |v: &[ChartData]| {
        let mut v = v.to_vec();
        v.sort_by(|a, b| a.name.cmp(&b.name));
        v
    };
    assert_eq!(by_name(&back.sheets[0].charts), by_name(&s.charts));
    assert_eq!(back.sheets[0].images, s.images);
}

// ---------------------------------------------------------------------------
// Hardening
// ---------------------------------------------------------------------------

const HEAD: &str = r#"<?xml version="1.0" encoding="UTF-8"?><office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0" xmlns:xlink="http://www.w3.org/1999/xlink" xmlns:chart="urn:oasis:names:tc:opendocument:xmlns:chart:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" office:version="1.2">"#;

fn manifest(objects: usize) -> String {
    let mut m = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?><manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.2"><manifest:file-entry manifest:full-path="/" manifest:media-type="application/vnd.oasis.opendocument.spreadsheet"/><manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/>"#,
    );
    for n in 1..=objects {
        m.push_str(&format!(r#"<manifest:file-entry manifest:full-path="Object {n}/" manifest:media-type="application/vnd.oasis.opendocument.chart"/><manifest:file-entry manifest:full-path="Object {n}/content.xml" manifest:media-type="text/xml"/>"#));
    }
    m.push_str("</manifest:manifest>");
    m
}

fn frame(n: usize) -> String {
    format!(
        r#"<draw:frame draw:name="Object {n}" svg:x="0cm" svg:y="0cm" svg:width="5cm" svg:height="3cm"><draw:object xlink:href="./Object {n}"/></draw:frame>"#
    )
}

fn chart_xml(body: &str) -> String {
    format!(
        "{HEAD}<office:body><office:chart><chart:chart chart:class=\"chart:bar\"><chart:plot-area>{body}</chart:plot-area></chart:chart></office:chart></office:body></office:document-content>"
    )
}

/// A package with `content.xml` holding one sheet whose A1 carries
/// `frames`, plus the given object entries.
fn package(frames: &str, objects: &[(String, Vec<u8>)], manifest_xml: &str) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    zip.start_file("mimetype", stored).unwrap();
    zip.write_all(tally_io_ods::MIMETYPE.as_bytes()).unwrap();
    zip.start_file("META-INF/manifest.xml", deflated).unwrap();
    zip.write_all(manifest_xml.as_bytes()).unwrap();
    zip.start_file("content.xml", deflated).unwrap();
    write!(
        zip,
        r#"{HEAD}<office:body><office:spreadsheet><table:table table:name="S"><table:table-row><table:table-cell office:value-type="float" office:value="1">{frames}<text:p>1</text:p></table:table-cell></table:table-row></table:table></office:spreadsheet></office:body></office:document-content>"#
    )
    .unwrap();
    for (name, bytes) in objects {
        zip.start_file(name, deflated).unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

fn timed(bytes: &[u8]) -> (Result<WorkbookData, OdsError>, Duration) {
    let t = Instant::now();
    let r = read_ods(Cursor::new(bytes), &Limits::default());
    (r, t.elapsed())
}

#[test]
fn href_traversal_rejected() {
    let frames = concat!(
        r#"<draw:frame draw:name="a" svg:width="1cm" svg:height="1cm"><draw:object xlink:href="../Object 1"/></draw:frame>"#,
        r#"<draw:frame draw:name="b" svg:width="1cm" svg:height="1cm"><draw:object xlink:href="./Object 1/../../content.xml"/></draw:frame>"#,
        r#"<draw:frame draw:name="c" svg:width="1cm" svg:height="1cm"><draw:image xlink:href="Pictures/../content.xml"/></draw:frame>"#,
        r#"<draw:frame draw:name="d" svg:width="1cm" svg:height="1cm"><draw:image xlink:href="/etc/passwd"/></draw:frame>"#,
    );
    let objects = vec![(
        "Object 1/content.xml".to_string(),
        chart_xml("").into_bytes(),
    )];
    let (r, _) = timed(&package(frames, &objects, &manifest(1)));
    let wb = r.unwrap();
    assert!(wb.sheets[0].charts.is_empty() && wb.sheets[0].images.is_empty());
    assert!(wb.warnings.iter().any(|w| w.contains("skipped")));
}

#[test]
fn missing_object_skipped() {
    let (r, _) = timed(&package(&frame(1), &[], &manifest(1)));
    let wb = r.unwrap();
    assert!(wb.sheets[0].charts.is_empty());
    assert_eq!(wb.cells_of(0).len(), 1);
}

#[test]
fn object_not_in_manifest_as_chart_is_skipped() {
    let objects = vec![(
        "Object 1/content.xml".to_string(),
        chart_xml("").into_bytes(),
    )];
    let (r, _) = timed(&package(&frame(1), &objects, &manifest(0)));
    let wb = r.unwrap();
    assert!(wb.sheets[0].charts.is_empty());
}

#[test]
fn too_many_charts_capped() {
    let n = 1_200;
    let frames: String = (1..=n).map(frame).collect();
    let objects: Vec<(String, Vec<u8>)> = (1..=n)
        .map(|i| {
            (
                format!("Object {i}/content.xml"),
                chart_xml("").into_bytes(),
            )
        })
        .collect();
    let (r, took) = timed(&package(&frames, &objects, &manifest(n)));
    let wb = r.unwrap();
    assert_eq!(wb.sheets[0].charts.len(), 1_000);
    assert!(
        wb.warnings.iter().any(|w| w.contains("1000")),
        "{:?}",
        wb.warnings
    );
    assert!(took < Duration::from_secs(10), "{took:?}");
}

#[test]
fn huge_object_xml_rejected_fast() {
    // 100 MB of padding inside the object XML: it inflates past the 16 MiB
    // object cap and must be refused without being read whole.
    let mut body = String::with_capacity(100 << 20);
    body.push_str("<!--");
    body.extend(std::iter::repeat_n('x', 100 << 20));
    body.push_str("-->");
    let objects = vec![(
        "Object 1/content.xml".to_string(),
        chart_xml(&body).into_bytes(),
    )];
    let (r, took) = timed(&package(&frame(1), &objects, &manifest(1)));
    assert!(
        matches!(r, Err(OdsError::LimitExceeded { .. })),
        "{:?}",
        r.map(|_| ())
    );
    assert!(took < Duration::from_secs(5), "{took:?}");
}

#[test]
fn ten_thousand_series_capped() {
    let series: String = (0..10_000)
        .map(|_| r#"<chart:series chart:values-cell-range-address="S.A1:S.A1"/>"#)
        .collect();
    let objects = vec![(
        "Object 1/content.xml".to_string(),
        chart_xml(&series).into_bytes(),
    )];
    let (r, took) = timed(&package(&frame(1), &objects, &manifest(1)));
    let wb = r.unwrap();
    assert_eq!(wb.sheets[0].charts[0].series.len(), 255);
    assert!(wb.warnings.iter().any(|w| w.contains("255")));
    assert!(took < Duration::from_secs(5), "{took:?}");
}

#[test]
fn deep_recursion_in_chart_xml_rejected() {
    let depth = 5_000;
    let mut body = String::new();
    for _ in 0..depth {
        body.push_str("<chart:series>");
    }
    for _ in 0..depth {
        body.push_str("</chart:series>");
    }
    let objects = vec![(
        "Object 1/content.xml".to_string(),
        chart_xml(&body).into_bytes(),
    )];
    let (r, took) = timed(&package(&frame(1), &objects, &manifest(1)));
    assert!(matches!(r, Err(OdsError::LimitExceeded { .. })));
    assert!(took < Duration::from_secs(5), "{took:?}");
}

#[test]
fn doctype_in_chart_xml_rejected() {
    let xml = format!(
        r#"<?xml version="1.0"?><!DOCTYPE x [<!ENTITY a "b">]>{}"#,
        chart_xml("").trim_start_matches(r#"<?xml version="1.0" encoding="UTF-8"?>"#)
    );
    let objects = vec![("Object 1/content.xml".to_string(), xml.into_bytes())];
    let (r, _) = timed(&package(&frame(1), &objects, &manifest(1)));
    assert!(matches!(r, Err(OdsError::ForbiddenDoctype)));
}

trait CellsOf {
    fn cells_of(&self, sheet: usize) -> &[CellData];
}

impl CellsOf for WorkbookData {
    fn cells_of(&self, sheet: usize) -> &[CellData] {
        &self.sheets[sheet].cells
    }
}

// ---------------------------------------------------------------------------
// Axis type, date scale and positioning (T18)
// ---------------------------------------------------------------------------

#[test]
fn roundtrip_date_axis_and_crossing() {
    use tally_model::{
        AxisCrossData, AxisLabelPosData, AxisMarkPosData, AxisTypeData, TimeIntervalData,
        TimeUnitData,
    };
    let mut c = chart(ChartKindData::Line);
    c.x_axis.axis_type = AxisTypeData::Date;
    c.x_axis.time_resolution = Some(TimeUnitData::Months);
    c.x_axis.major_time = Some(TimeIntervalData {
        number: 2,
        unit: TimeUnitData::Months,
    });
    c.x_axis.minor_time = Some(TimeIntervalData {
        number: 7,
        unit: TimeUnitData::Days,
    });
    c.x_axis.min = Some(46_023.0);
    c.x_axis.crosses = Some(AxisCrossData::Value(50.0));
    c.x_axis.label_position = AxisLabelPosData::OutsideStart;
    c.x_axis.mark_position = AxisMarkPosData::AtAxis;
    c.y_axis.crosses = Some(AxisCrossData::End);
    c.y_axis.label_position = AxisLabelPosData::NearAxisOtherSide;
    c.y_axis.mark_position = AxisMarkPosData::AtLabels;
    assert_round_trip(c.clone());
    for t in [AxisTypeData::Auto, AxisTypeData::Text] {
        let mut c = chart(ChartKindData::Column);
        c.x_axis.axis_type = t;
        c.y_axis.crosses = Some(AxisCrossData::Start);
        assert_round_trip(c);
    }
    let mut c = chart(ChartKindData::Column);
    c.y_axis.crosses = Some(AxisCrossData::Value(3.0));
    c.x_axis.crosses = Some(AxisCrossData::Value(-2.5));
    assert_round_trip(c);
}

#[test]
fn writes_aoo_axis_attributes() {
    use tally_model::{AxisCrossData, AxisTypeData, TimeIntervalData, TimeUnitData};
    let mut c = chart(ChartKindData::Line);
    c.x_axis.axis_type = AxisTypeData::Date;
    c.x_axis.major_time = Some(TimeIntervalData {
        number: 1,
        unit: TimeUnitData::Years,
    });
    c.x_axis.crosses = Some(AxisCrossData::Value(50.0));
    let mut buf = Cursor::new(Vec::new());
    write_ods(&workbook(vec![c]), &mut buf).unwrap();
    let mut zip = zip::ZipArchive::new(Cursor::new(buf.into_inner())).unwrap();
    let mut xml = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("Object 1/content.xml").unwrap(), &mut xml)
        .unwrap();
    // As AOO 4.1 writes them (SchXMLExport.cxx, XMLAxisPositionPropertyHdl).
    assert!(xml.contains(r#"xmlns:chartooo="http://openoffice.org/2010/chart""#));
    assert!(xml.contains(r#"chartooo:axis-type="date""#), "{xml}");
    assert!(xml.contains(
        r#"<chartooo:date-scale chart:major-interval-value="1" chart:major-interval-unit="years"/>"#
    ));
    assert!(xml.contains(r#"chart:axis-position="50""#));
}

fn axis_chart_xml(x_axis_attrs: &str, x_axis_body: &str) -> String {
    format!(
        concat!(
            "{head}<office:automatic-styles>",
            r#"<style:style style:name="ax" style:family="chart"><style:chart-properties chart:axis-position="0" chart:axis-label-position="outside-minimum" chart:tick-mark-position="at-axis"/></style:style>"#,
            r#"<style:style style:name="ay" style:family="chart"><style:chart-properties chart:axis-position="end" chart:axis-label-position="near-axis"/></style:style>"#,
            "</office:automatic-styles><office:body><office:chart>",
            r#"<chart:chart chart:class="chart:line" xmlns:chartooo="http://openoffice.org/2010/chart"><chart:plot-area>"#,
            r#"<chart:axis chart:dimension="x" chart:name="primary-x" chart:style-name="ax"{attrs}>{body}</chart:axis>"#,
            r#"<chart:axis chart:dimension="y" chart:name="primary-y" chart:style-name="ay"/>"#,
            "</chart:plot-area></chart:chart></office:chart></office:body></office:document-content>"
        ),
        head = HEAD,
        attrs = x_axis_attrs,
        body = x_axis_body
    )
}

fn read_axis_chart(xml: String) -> ChartData {
    let objects = vec![("Object 1/content.xml".to_string(), xml.into_bytes())];
    let wb = read_ods(
        Cursor::new(package(&frame(1), &objects, &manifest(1))),
        &Limits::default(),
    )
    .unwrap();
    wb.sheets[0].charts[0].clone()
}

#[test]
fn reads_aoo_axis_type_and_positions() {
    use tally_model::{
        AxisCrossData, AxisLabelPosData, AxisMarkPosData, AxisTypeData, TimeIntervalData,
        TimeUnitData,
    };
    // AOO 4.1 with an automatic date axis: `auto` plus a date scale.
    let c = read_axis_chart(axis_chart_xml(
        r#" chartooo:axis-type="auto""#,
        r#"<chartooo:date-scale chart:base-time-unit="months" chart:major-interval-value="3" chart:major-interval-unit="months" chart:minor-interval-unit="days"/>"#,
    ));
    assert_eq!(c.x_axis.axis_type, AxisTypeData::Auto);
    assert_eq!(c.x_axis.time_resolution, Some(TimeUnitData::Months));
    assert_eq!(
        c.x_axis.major_time,
        Some(TimeIntervalData {
            number: 3,
            unit: TimeUnitData::Months
        })
    );
    // Only the unit given: AOO starts from TimeInterval(1, DAY).
    assert_eq!(
        c.x_axis.minor_time,
        Some(TimeIntervalData {
            number: 1,
            unit: TimeUnitData::Days
        })
    );
    // "0" is AOO's default crossing; outside-minimum is the 1.2 alias.
    assert_eq!(c.x_axis.crosses, None);
    assert_eq!(c.x_axis.label_position, AxisLabelPosData::OutsideStart);
    assert_eq!(c.x_axis.mark_position, AxisMarkPosData::AtAxis);
    assert_eq!(c.y_axis.crosses, Some(AxisCrossData::End));
    // Older documents (no axis type, no date scale) keep text categories
    // (SchXMLAxisContext::EndElement sets AutoDateAxis = false).
    let old = read_axis_chart(axis_chart_xml("", ""));
    assert_eq!(old.x_axis.axis_type, AxisTypeData::Text);
    let auto_only = read_axis_chart(axis_chart_xml(r#" chartooo:axis-type="auto""#, ""));
    assert_eq!(auto_only.x_axis.axis_type, AxisTypeData::Text);
    // The chart: namespace spelling of the attribute and the element.
    let date = read_axis_chart(axis_chart_xml(
        r#" chart:axis-type="date""#,
        r#"<chart:date-scale chart:major-interval-value="2"/>"#,
    ));
    assert_eq!(date.x_axis.axis_type, AxisTypeData::Date);
    assert_eq!(
        date.x_axis.major_time,
        Some(TimeIntervalData {
            number: 2,
            unit: TimeUnitData::Days
        })
    );
}
