//! Charts and pictures in XLSX (wave 5, W5-1): write → read round trips
//! for every supported kind, the degradations (bubble, stock volume),
//! anchors, the Excel-365-shaped fixture, hardening, and ODS ⇄ XLSX.

#![allow(clippy::unwrap_used)] // tests may unwrap (docs/contributing.md)

mod common;

use std::io::{Cursor, Write};
use std::time::{Duration, Instant};

use common::read_fixture;
use tally_io_xlsx::{read_xlsx, write_xlsx, Limits, XlsxError};
use tally_model::{
    CellContentData, CellData, CellRangeLocal, ChartAnchorData, ChartAxisData, ChartData,
    ChartKindData, ChartRangeData, ChartSeriesData, ChartValuesData, DataLabelsData,
    ErrorBarDirData, ErrorBarKindData, ErrorBarsData, ImageData, LabelPosData, LegendPosData,
    MarkerShapeData, SeriesNameData, SheetData, StackingData, StockVariantData, TrendKindData,
    TrendLineData, WorkbookData,
};
use tally_types::Value;

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01\x08\x06\x00\x00\x00\x1f\x15\xc4\x89\x00\x00\x00\rIDATx\x9cc\xf8\xcf\xc0\xf0\x1f\x00\x05\x00\x02\x00\xde\xf2\x8a\xbb\x00\x00\x00\x00IEND\xaeB`\x82";

fn cell(row: u32, col: u32, v: Value) -> CellData {
    CellData {
        row,
        col,
        content: CellContentData::Value(v),
        style: None,
    }
}

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
        name: "Chart 1".into(),
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
        title: Some("Sales".into()),
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

fn write(wb: &WorkbookData) -> (Vec<u8>, Vec<String>) {
    let mut buf = Cursor::new(Vec::new());
    let report = write_xlsx(wb, &mut buf).unwrap();
    (buf.into_inner(), report.warnings)
}

fn round_trip(wb: &WorkbookData) -> (WorkbookData, Vec<String>) {
    let (bytes, warnings) = write(wb);
    let back = read_xlsx(Cursor::new(bytes), &Limits::default()).unwrap();
    (back, warnings)
}

/// The chart read back after a write (one chart in the workbook).
fn back(c: ChartData) -> (ChartData, Vec<String>) {
    let (wb, warnings) = round_trip(&workbook(vec![c]));
    assert_eq!(
        wb.sheets[0].charts.len(),
        1,
        "{warnings:?} {:?}",
        wb.warnings
    );
    (wb.sheets[0].charts[0].clone(), warnings)
}

/// Excel has no `source`; the title is written as rich text and comes
/// back; everything else the test compares explicitly.
fn assert_core(got: &ChartData, want: &ChartData) {
    assert_eq!(got.kind, want.kind, "kind");
    assert_eq!(got.stacking, want.stacking, "stacking");
    assert_eq!(got.title, want.title, "title");
    assert_eq!(got.categories, want.categories, "categories");
    assert_eq!(got.series.len(), want.series.len(), "series count");
    for (g, w) in got.series.iter().zip(&want.series) {
        assert_eq!(g.values, w.values, "values");
        assert_eq!(g.name, w.name, "name");
        assert_eq!(g.kind, w.kind, "series kind");
        assert_eq!(g.secondary_axis, w.secondary_axis, "secondary");
    }
    assert_eq!(got.legend, want.legend, "legend");
    assert_eq!(got.anchor.row, want.anchor.row);
    assert_eq!(got.anchor.col, want.anchor.col);
}

#[test]
fn roundtrip_column_bar_line_area_with_stacking() {
    for kind in [
        ChartKindData::Column,
        ChartKindData::Bar,
        ChartKindData::Line,
        ChartKindData::Area,
    ] {
        for stacking in [
            StackingData::None,
            StackingData::Stacked,
            StackingData::Percent,
        ] {
            let mut c = chart(kind);
            c.stacking = stacking;
            let (got, warnings) = back(c.clone());
            assert!(warnings.is_empty(), "{warnings:?}");
            assert_core(&got, &c);
        }
    }
}

#[test]
fn roundtrip_pie_and_donut() {
    let mut c = chart(ChartKindData::Pie);
    c.series.truncate(1);
    c.series[0].labels = DataLabelsData {
        value: false,
        percent: true,
        category: true,
        series_name: false,
        position: LabelPosData::Outside,
    };
    let (got, _) = back(c.clone());
    assert_core(&got, &c);
    assert!(got.series[0].labels.percent && got.series[0].labels.category);
    assert_eq!(got.hole, None);
    c.hole = Some(0.4);
    let (got, _) = back(c.clone());
    assert_core(&got, &c);
    assert!((got.hole.unwrap() - 0.4).abs() < 0.01);
}

#[test]
fn roundtrip_scatter_variants() {
    for (lines, markers, smooth) in [
        (false, true, false),
        (true, false, false),
        (true, true, false),
        (true, false, true),
        (true, true, true),
    ] {
        let mut c = chart(ChartKindData::Scatter);
        c.categories = None;
        c.lines = lines;
        c.markers = markers;
        c.smooth = smooth;
        for s in &mut c.series {
            s.x_values = Some(area(1, 1, 5, 1));
        }
        let (got, warnings) = back(c.clone());
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_core(&got, &c);
        assert_eq!(
            (got.lines, got.markers, got.smooth),
            (lines, markers, smooth)
        );
        assert_eq!(got.series[0].x_values, Some(area(1, 1, 5, 1)));
    }
}

#[test]
fn roundtrip_radar_variants() {
    for (markers, filled) in [(false, false), (true, false), (false, true)] {
        let mut c = chart(ChartKindData::Radar);
        c.markers = markers;
        c.filled = filled;
        let (got, _) = back(c.clone());
        assert_core(&got, &c);
        assert_eq!((got.markers, got.filled), (markers, filled));
    }
}

#[test]
fn roundtrip_stock_hlc_ohlc() {
    let mut c = chart(ChartKindData::Stock);
    c.stock = Some(StockVariantData::Hlc);
    c.series = vec![series(1), series(2), series(1)];
    let (got, warnings) = back(c.clone());
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_core(&got, &c);
    assert_eq!(got.stock, Some(StockVariantData::Hlc));
    c.stock = Some(StockVariantData::Ohlc);
    c.series = vec![series(1), series(2), series(1), series(2)];
    let (got, _) = back(c.clone());
    assert_core(&got, &c);
    assert_eq!(got.stock, Some(StockVariantData::Ohlc));
}

#[test]
fn stock_volume_dropped_with_warning() {
    let mut c = chart(ChartKindData::Stock);
    c.stock = Some(StockVariantData::VolumeOhlc);
    c.series = vec![series(2), series(1), series(2), series(1), series(2)];
    let (got, warnings) = back(c);
    assert!(
        warnings.iter().any(|w| w.contains("volume")),
        "{warnings:?}"
    );
    assert_eq!(got.series.len(), 4);
    assert_eq!(got.stock, Some(StockVariantData::Ohlc));
}

#[test]
fn bubble_degrades_to_scatter_with_warning() {
    let mut c = chart(ChartKindData::Bubble);
    c.categories = None;
    c.series.truncate(1);
    c.series[0].x_values = Some(area(1, 1, 5, 1));
    c.series[0].sizes = Some(area(1, 2, 5, 2));
    let (got, warnings) = back(c);
    assert!(
        warnings.iter().any(|w| w.contains("bubble")),
        "{warnings:?}"
    );
    assert_eq!(got.kind, ChartKindData::Scatter);
    assert_eq!(got.series[0].x_values, Some(area(1, 1, 5, 1)));
    assert_eq!(got.series[0].sizes, None);
}

#[test]
fn roundtrip_combo_and_secondary_axis() {
    let mut c = chart(ChartKindData::Column);
    c.series[1].kind = Some(ChartKindData::Line);
    c.series[1].secondary_axis = true;
    c.y2_axis = Some(ChartAxisData {
        title: Some("Costs".into()),
        min: Some(1.0),
        max: Some(1000.0),
        log: true,
        ..ChartAxisData::default()
    });
    let (got, warnings) = back(c.clone());
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_core(&got, &c);
    let y2 = got.y2_axis.expect("secondary axis");
    assert!(y2.log);
    assert_eq!((y2.min, y2.max), (Some(1.0), Some(1000.0)));
    assert_eq!(y2.title.as_deref(), Some("Costs"));
}

#[test]
fn roundtrip_trend_lines_error_bars_labels_colours() {
    let mut c = chart(ChartKindData::Line);
    c.markers = true;
    c.series[0].trend = Some(TrendLineData {
        kind: TrendKindData::Exponential,
        show_equation: true,
        show_r2: true,
        name: Some("Fit".into()),
    });
    c.series[0].error_bars = Some(ErrorBarsData {
        kind: ErrorBarKindData::StdDev { k: 2.0 },
        direction: ErrorBarDirData::Plus,
    });
    c.series[0].color = Some(0x1122_33ff);
    c.series[0].marker = Some(MarkerShapeData::Diamond);
    c.series[1].error_bars = Some(ErrorBarsData {
        kind: ErrorBarKindData::Range {
            plus: area(1, 2, 5, 2),
            minus: area(1, 2, 5, 2),
        },
        direction: ErrorBarDirData::Both,
    });
    c.series[1].labels = DataLabelsData {
        value: true,
        percent: false,
        category: false,
        series_name: true,
        position: LabelPosData::Above,
    };
    c.series[1].name = Some(SeriesNameData::Text("Literal".into()));
    c.legend = LegendPosData::Bottom;
    c.subtitle = Some("sub".into());
    c.x_axis.title = Some("Month".into());
    c.y_axis = ChartAxisData {
        title: Some("k€".into()),
        min: Some(0.0),
        max: Some(120.0),
        major: Some(20.0),
        reverse: true,
        major_grid: true,
        minor_grid: true,
        number_format: Some("#,##0".into()),
        ..ChartAxisData::default()
    };
    c.background = Some(0xffff_f0ff);
    c.plot_background = Some(0xe6e6_e6ff);
    c.include_hidden = true;
    let (got, warnings) = back(c.clone());
    assert!(warnings.is_empty(), "{warnings:?}");
    let s0 = &got.series[0];
    assert_eq!(s0.trend, c.series[0].trend);
    assert_eq!(s0.error_bars, c.series[0].error_bars);
    assert_eq!(s0.color, Some(0x1122_33ff));
    assert_eq!(s0.marker, Some(MarkerShapeData::Diamond));
    let s1 = &got.series[1];
    assert_eq!(s1.error_bars, c.series[1].error_bars);
    assert!(s1.labels.value && s1.labels.series_name);
    assert_eq!(s1.labels.position, LabelPosData::Above);
    assert_eq!(s1.name, Some(SeriesNameData::Text("Literal".into())));
    assert_eq!(got.title.as_deref(), Some("Sales\nsub"));
    assert_eq!(got.legend, LegendPosData::Bottom);
    assert_eq!(got.x_axis.title.as_deref(), Some("Month"));
    let y = &got.y_axis;
    assert_eq!(
        (y.min, y.max, y.major),
        (Some(0.0), Some(120.0), Some(20.0))
    );
    assert!(y.reverse && y.major_grid && y.minor_grid);
    assert_eq!(y.title.as_deref(), Some("k€"));
    assert_eq!(y.number_format.as_deref(), Some("#,##0"));
    assert_eq!(got.background, Some(0xffff_f0ff));
    assert_eq!(got.plot_background, Some(0xe6e6_e6ff));
    assert!(got.include_hidden);
}

#[test]
fn roundtrip_three_d_written_flat_with_warning() {
    let mut c = chart(ChartKindData::Column);
    c.three_d = true;
    let (got, warnings) = back(c);
    assert!(warnings.iter().any(|w| w.contains("3D")), "{warnings:?}");
    assert!(!got.three_d);
}

#[test]
fn roundtrip_anchor_and_size() {
    let mut c = chart(ChartKindData::Column);
    c.anchor = ChartAnchorData {
        row: 10,
        col: 7,
        dx_hmm: 1200,
        dy_hmm: 300,
        width_hmm: 12_000,
        height_hmm: 6_000,
    };
    let (got, _) = back(c);
    assert_eq!((got.anchor.row, got.anchor.col), (10, 7));
    // Pixel rounding both ways: within 1/4 mm.
    assert!(
        (i64::from(got.anchor.dx_hmm) - 1200).abs() < 30,
        "{}",
        got.anchor.dx_hmm
    );
    assert!(
        (i64::from(got.anchor.dy_hmm) - 300).abs() < 30,
        "{}",
        got.anchor.dy_hmm
    );
    assert!(
        (i64::from(got.anchor.width_hmm) - 12_000).abs() < 60,
        "{}",
        got.anchor.width_hmm
    );
    assert!(
        (i64::from(got.anchor.height_hmm) - 6_000).abs() < 60,
        "{}",
        got.anchor.height_hmm
    );
}

#[test]
fn roundtrip_images() {
    let mut wb = workbook(vec![chart(ChartKindData::Column)]);
    wb.sheets[0].images = vec![ImageData {
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
    }];
    let (back, warnings) = round_trip(&wb);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(back.sheets[0].images.len(), 1);
    let img = &back.sheets[0].images[0];
    assert_eq!(img.mime, "image/png");
    assert_eq!(&*img.bytes, PNG);
    assert_eq!((img.anchor.row, img.anchor.col), (3, 3));
    assert!((i64::from(img.anchor.width_hmm) - 2000).abs() < 60);
    assert_eq!(back.sheets[0].charts.len(), 1);
}

#[test]
fn invalid_series_dropped_with_warning() {
    let mut c = chart(ChartKindData::Column);
    c.series[0].values = ChartRangeData {
        areas: Vec::new(),
        invalid: true,
    };
    let (got, warnings) = back(c);
    assert!(warnings.iter().any(|w| w.contains("#REF!")), "{warnings:?}");
    assert_eq!(got.series.len(), 1);
}

// ---------------------------------------------------------------------------
// Fixture shaped like Excel 365 output
// ---------------------------------------------------------------------------

#[test]
fn reads_excel_fixture() {
    let wb = read_xlsx(
        Cursor::new(read_fixture("charts/excel365-charts.xlsx")),
        &Limits::default(),
    )
    .unwrap();
    let s = &wb.sheets[0];
    assert_eq!(s.charts.len(), 3, "{:?}", wb.warnings);
    let col = &s.charts[0];
    assert_eq!(col.name, "Chart 1");
    assert_eq!(col.kind, ChartKindData::Column);
    assert_eq!(col.stacking, StackingData::None);
    assert_eq!(col.title.as_deref(), Some("Sales 2026"));
    assert_eq!(col.legend, LegendPosData::Right);
    assert_eq!(col.categories, Some(area(1, 0, 5, 0)));
    assert_eq!(col.series.len(), 2);
    assert_eq!(col.series[0].values, area(1, 1, 5, 1));
    assert_eq!(
        col.series[0].name,
        Some(SeriesNameData::Ref(area(0, 1, 0, 1)))
    );
    assert_eq!(col.series[0].color, Some(0x4472_c4ff)); // accent1 of the theme
    assert_eq!(col.series[1].color, Some(0xff42_0eff));
    assert!(col.series[1].labels.value);
    assert_eq!(col.series[1].labels.position, LabelPosData::Outside);
    assert_eq!(col.x_axis.title.as_deref(), Some("Month"));
    assert_eq!(col.y_axis.title.as_deref(), Some("k€"));
    assert!(col.y_axis.major_grid);
    assert_eq!(
        (col.y_axis.min, col.y_axis.max, col.y_axis.major),
        (Some(0.0), Some(120.0), Some(20.0))
    );
    assert_eq!(col.y_axis.number_format.as_deref(), Some("#,##0"));
    assert_eq!(col.background, Some(0xffff_f0ff));
    assert!(!col.include_hidden);

    let sc = &s.charts[1];
    assert_eq!(sc.name, "Chart 2");
    assert_eq!(sc.kind, ChartKindData::Scatter);
    assert!(sc.lines && sc.markers && !sc.smooth);
    assert_eq!(sc.title, None);
    assert_eq!(sc.legend, LegendPosData::None);
    assert!(sc.include_hidden);
    assert_eq!(sc.series[0].name, Some(SeriesNameData::Text("Fit".into())));
    assert_eq!(sc.series[0].x_values, Some(area(1, 1, 5, 1)));
    assert_eq!(sc.series[0].values, area(1, 2, 5, 2));
    assert_eq!(sc.series[0].marker, Some(MarkerShapeData::Diamond));
    assert_eq!(sc.series[0].color, Some(0x579d_1cff));
    assert_eq!(
        sc.series[0].trend,
        Some(TrendLineData {
            kind: TrendKindData::Linear,
            show_equation: true,
            show_r2: true,
            name: Some("Linear fit".into()),
        })
    );
    assert_eq!(
        sc.series[0].error_bars,
        Some(ErrorBarsData {
            kind: ErrorBarKindData::StdDev { k: 1.0 },
            direction: ErrorBarDirData::Both,
        })
    );
    assert!(sc.y_axis.log && sc.y_axis.major_grid && sc.y_axis.minor_grid);
    assert_eq!((sc.y_axis.min, sc.y_axis.max), (Some(1.0), Some(1000.0)));

    let pie = &s.charts[2];
    assert_eq!(pie.kind, ChartKindData::Pie);
    assert!(pie.three_d && pie.exploded);
    assert_eq!(pie.legend, LegendPosData::Bottom);
    assert!(pie.series[0].labels.percent && pie.series[0].labels.category);
    assert_eq!(
        pie.literal_categories,
        Some(ChartValuesData::LiteralText(vec![
            "A".into(),
            "B".into(),
            "C".into()
        ]))
    );
    assert_eq!(
        pie.series[0].literal_values,
        Some(ChartValuesData::Literal(vec![
            Some(1.0),
            Some(2.0),
            Some(3.0)
        ]))
    );
    // Absolute anchor at (7200000, 7200000) EMU = 20000 hmm: column A has
    // a custom width, the other columns are 1693 hmm, rows 529 hmm.
    let col_a = s.col_widths.iter().find(|r| r.start == 0).unwrap().size_hmm;
    let rest = 20000 - col_a;
    assert_eq!(pie.anchor.col, 1 + rest / 1693);
    assert_eq!(pie.anchor.dx_hmm, rest % 1693);
    assert_eq!(pie.anchor.row, 20000 / 529);

    assert_eq!(s.images.len(), 1);
    assert_eq!(&*s.images[0].bytes, PNG);
    assert_eq!((s.images[0].anchor.row, s.images[0].anchor.col), (1, 14));
    assert!(
        wb.warnings.iter().any(|w| w.contains("skipped")),
        "{:?}",
        wb.warnings
    );
}

#[test]
fn two_cell_anchor_emu_to_hmm() {
    let wb = read_xlsx(
        Cursor::new(read_fixture("charts/excel365-charts.xlsx")),
        &Limits::default(),
    )
    .unwrap();
    let a = wb.sheets[0].charts[0].anchor;
    // from (col 4, row 1) with offsets 190500 / 95250 EMU.
    assert_eq!((a.row, a.col), (1, 4));
    assert_eq!(a.dx_hmm, 190_500 / 360);
    assert_eq!(a.dy_hmm, 95_250 / 360);
    // to (col 12, row 16) with colOff 285750: 8 default columns minus the
    // from offset plus the to offset; 15 default rows.
    assert_eq!(a.width_hmm as i64, 8 * 1693 - 190_500 / 360 + 285_750 / 360);
    assert_eq!(a.height_hmm as i64, 15 * 529 - 95_250 / 360);
    // one-cell anchor: ext 4320000 × 2880000 EMU.
    let b = wb.sheets[0].charts[1].anchor;
    assert_eq!(
        (b.row, b.col, b.width_hmm, b.height_hmm),
        (18, 4, 12_000, 8_000)
    );
}

// ---------------------------------------------------------------------------
// Hardening
// ---------------------------------------------------------------------------

const RT: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/";

fn chart_part(series: &str) -> String {
    format!(
        r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><c:chart><c:plotArea><c:barChart><c:barDir val="col"/>{series}<c:axId val="1"/><c:axId val="2"/></c:barChart><c:catAx><c:axId val="1"/></c:catAx><c:valAx><c:axId val="2"/></c:valAx></c:plotArea></c:chart></c:chartSpace>"#
    )
}

/// A package with one sheet, a drawing of `n` chart anchors and the given
/// chart parts (`charts[i]` is `chart{i+1}.xml`).
fn package(n: usize, charts: &[String]) -> Vec<u8> {
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default();
    let wb_rels = format!(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{RT}worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#
    );
    let wb = r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="S" sheetId="1" r:id="rId1"/></sheets></workbook>"#;
    let sheet = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheetData><row r="1"><c r="A1"><v>1</v></c></row></sheetData><drawing r:id="rId1"/></worksheet>"#;
    let sheet_rels = format!(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{RT}drawing" Target="../drawings/drawing1.xml"/></Relationships>"#
    );
    let mut drawing = String::from(
        r#"<xdr:wsDr xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart">"#,
    );
    let mut drels = String::from(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
    );
    for i in 1..=n {
        drawing.push_str(&format!(r#"<xdr:oneCellAnchor><xdr:from><xdr:col>1</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>{i}</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from><xdr:ext cx="360000" cy="360000"/><xdr:graphicFrame><xdr:nvGraphicFramePr><xdr:cNvPr id="{i}" name="Chart {i}"/></xdr:nvGraphicFramePr><a:graphic><a:graphicData><c:chart r:id="rId{i}"/></a:graphicData></a:graphic></xdr:graphicFrame></xdr:oneCellAnchor>"#));
        drels.push_str(&format!(
            r#"<Relationship Id="rId{i}" Type="{RT}chart" Target="../charts/chart{i}.xml"/>"#
        ));
    }
    drawing.push_str("</xdr:wsDr>");
    drels.push_str("</Relationships>");
    let mut entries: Vec<(String, Vec<u8>)> = vec![
        ("xl/workbook.xml".into(), wb.as_bytes().to_vec()),
        ("xl/_rels/workbook.xml.rels".into(), wb_rels.into_bytes()),
        ("xl/worksheets/sheet1.xml".into(), sheet.as_bytes().to_vec()),
        (
            "xl/worksheets/_rels/sheet1.xml.rels".into(),
            sheet_rels.into_bytes(),
        ),
        ("xl/drawings/drawing1.xml".into(), drawing.into_bytes()),
        (
            "xl/drawings/_rels/drawing1.xml.rels".into(),
            drels.into_bytes(),
        ),
    ];
    for (i, c) in charts.iter().enumerate() {
        entries.push((
            format!("xl/charts/chart{}.xml", i + 1),
            c.as_bytes().to_vec(),
        ));
    }
    for (name, data) in entries {
        z.start_file(name, opts).unwrap();
        z.write_all(&data).unwrap();
    }
    z.finish().unwrap().into_inner()
}

fn timed(bytes: &[u8]) -> (Result<WorkbookData, XlsxError>, Duration) {
    let t = Instant::now();
    let r = read_xlsx(Cursor::new(bytes), &Limits::default());
    (r, t.elapsed())
}

#[test]
fn too_many_charts_capped() {
    let n = 1_200;
    let part = chart_part(
        r#"<c:ser><c:idx val="0"/><c:order val="0"/><c:val><c:numRef><c:f>S!$A$1</c:f></c:numRef></c:val></c:ser>"#,
    );
    let charts: Vec<String> = (0..n).map(|_| part.clone()).collect();
    let (r, took) = timed(&package(n, &charts));
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
fn huge_chart_xml_rejected_fast() {
    let mut pad = String::with_capacity(100 << 20);
    pad.push_str("<!--");
    pad.extend(std::iter::repeat_n('x', 100 << 20));
    pad.push_str("-->");
    let part = chart_part(&pad);
    let (r, took) = timed(&package(1, &[part]));
    assert!(
        matches!(r, Err(XlsxError::LimitExceeded { .. })),
        "{:?}",
        r.map(|_| ())
    );
    assert!(took < Duration::from_secs(5), "{took:?}");
}

#[test]
fn ten_thousand_series_capped() {
    let series: String = (0..10_000)
        .map(|i| format!(r#"<c:ser><c:idx val="{i}"/><c:order val="{i}"/><c:val><c:numRef><c:f>S!$A$1</c:f></c:numRef></c:val></c:ser>"#))
        .collect();
    let (r, took) = timed(&package(1, &[chart_part(&series)]));
    let wb = r.unwrap();
    assert_eq!(wb.sheets[0].charts[0].series.len(), 255);
    assert!(
        wb.warnings.iter().any(|w| w.contains("255")),
        "{:?}",
        wb.warnings
    );
    assert!(took < Duration::from_secs(5), "{took:?}");
}

#[test]
fn deep_recursion_in_chart_xml_rejected() {
    let depth = 5_000;
    let mut body = String::new();
    for _ in 0..depth {
        body.push_str("<c:ser>");
    }
    for _ in 0..depth {
        body.push_str("</c:ser>");
    }
    let (r, took) = timed(&package(1, &[chart_part(&body)]));
    assert!(matches!(r, Err(XlsxError::LimitExceeded { .. })));
    assert!(took < Duration::from_secs(5), "{took:?}");
}

#[test]
fn node_bomb_in_chart_xml_rejected() {
    // Half a million tiny elements: refused by the node cap.
    let body: String = (0..500_000).map(|_| "<c:x/>").collect();
    let (r, took) = timed(&package(1, &[chart_part(&body)]));
    assert!(matches!(r, Err(XlsxError::LimitExceeded { .. })));
    assert!(took < Duration::from_secs(5), "{took:?}");
}

#[test]
fn doctype_in_chart_xml_rejected() {
    let part = format!(r#"<!DOCTYPE x [<!ENTITY a "b">]>{}"#, chart_part(""));
    let (r, _) = timed(&package(1, &[part]));
    assert!(matches!(r, Err(XlsxError::ForbiddenDoctype)));
}

#[test]
fn missing_chart_part_and_bad_rels_skipped() {
    let (r, _) = timed(&package(1, &[]));
    let wb = r.unwrap();
    assert!(wb.sheets[0].charts.is_empty());
    assert_eq!(wb.sheets[0].cells.len(), 1);
}

// ---------------------------------------------------------------------------
// Cross format
// ---------------------------------------------------------------------------

fn via_ods(wb: &WorkbookData) -> WorkbookData {
    let mut buf = Cursor::new(Vec::new());
    tally_io_ods::write_ods(wb, &mut buf).unwrap();
    tally_io_ods::read_ods(
        Cursor::new(buf.into_inner()),
        &tally_io_ods::Limits::default(),
    )
    .unwrap()
}

#[test]
fn cross_format_ods_xlsx_ods() {
    for kind in [
        ChartKindData::Column,
        ChartKindData::Line,
        ChartKindData::Pie,
    ] {
        let mut c = chart(kind);
        if kind == ChartKindData::Pie {
            c.series.truncate(1);
        }
        c.legend = LegendPosData::Bottom;
        let wb = workbook(vec![c.clone()]);
        let ods1 = via_ods(&wb);
        assert_eq!(ods1.sheets[0].charts.len(), 1);
        let (xlsx, warnings) = round_trip(&ods1);
        assert!(warnings.is_empty(), "{warnings:?}");
        let ods2 = via_ods(&xlsx);
        let got = &ods2.sheets[0].charts[0];
        assert_core(got, &c);
        assert_eq!(got.series.len(), c.series.len());
    }
}

// ---------------------------------------------------------------------------
// Date axis and crossing (T18)
// ---------------------------------------------------------------------------

#[test]
fn roundtrip_axis_type_intervals_and_crossing() {
    use tally_model::{
        AxisCrossData, AxisLabelPosData, AxisTypeData, TimeIntervalData, TimeUnitData,
    };
    // Date axis: c:dateAx without c:auto, its time units, and crossings.
    let mut c = chart(ChartKindData::Line);
    c.x_axis.axis_type = AxisTypeData::Date;
    c.x_axis.major_time = Some(TimeIntervalData {
        number: 2,
        unit: TimeUnitData::Months,
    });
    c.x_axis.minor_time = Some(TimeIntervalData {
        number: 10,
        unit: TimeUnitData::Days,
    });
    c.x_axis.min = Some(46_023.0);
    c.x_axis.crosses = Some(AxisCrossData::Value(50.0));
    c.x_axis.label_position = AxisLabelPosData::OutsideStart;
    c.y_axis.crosses = Some(AxisCrossData::End);
    c.y_axis.label_position = AxisLabelPosData::OutsideEnd;
    let (got, _) = back(c.clone());
    assert_core(&got, &c);
    assert_eq!(got.x_axis.axis_type, AxisTypeData::Date);
    assert_eq!(got.x_axis.major_time, c.x_axis.major_time);
    assert_eq!(got.x_axis.minor_time, c.x_axis.minor_time);
    assert_eq!(got.x_axis.min, Some(46_023.0));
    assert_eq!(got.x_axis.crosses, Some(AxisCrossData::Value(50.0)));
    assert_eq!(got.x_axis.label_position, AxisLabelPosData::OutsideStart);
    assert_eq!(got.y_axis.crosses, Some(AxisCrossData::End));
    assert_eq!(got.y_axis.label_position, AxisLabelPosData::OutsideEnd);
    // Text (c:catAx, no c:auto) and automatic (c:auto val=1); the value
    // axis crossing a text axis at a category number (c:crossesAt).
    for t in [AxisTypeData::Text, AxisTypeData::Auto] {
        let mut c = chart(ChartKindData::Column);
        c.x_axis.axis_type = t;
        c.y_axis.crosses = Some(AxisCrossData::Value(2.0));
        c.x_axis.crosses = Some(AxisCrossData::Start);
        let (got, _) = back(c);
        assert_eq!(got.x_axis.axis_type, t);
        assert_eq!(got.y_axis.crosses, Some(AxisCrossData::Value(2.0)));
        assert_eq!(got.x_axis.crosses, Some(AxisCrossData::Start));
    }
    // Automatic with date intervals: c:dateAx with c:auto.
    let mut c = chart(ChartKindData::Area);
    c.x_axis.major_time = Some(TimeIntervalData {
        number: 1,
        unit: TimeUnitData::Years,
    });
    let (got, _) = back(c);
    assert_eq!(got.x_axis.axis_type, AxisTypeData::Auto);
    assert_eq!(
        got.x_axis.major_time.map(|t| t.unit),
        Some(TimeUnitData::Years)
    );
}

#[test]
fn reads_excel_date_axis_as_aoo() {
    use tally_model::{AxisCrossData, AxisTypeData, TimeIntervalData, TimeUnitData};
    let part = |axis: &str| {
        format!(
            r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart><c:plotArea><c:lineChart><c:ser><c:idx val="0"/><c:val><c:numRef><c:f>S!$A$1:$A$3</c:f></c:numRef></c:val></c:ser><c:axId val="1"/><c:axId val="2"/></c:lineChart>{axis}<c:valAx><c:axId val="2"/><c:crosses val="max"/></c:valAx></c:plotArea></c:chart></c:chartSpace>"#
        )
    };
    let read = |axis: &str| {
        let wb = read_xlsx(Cursor::new(package(1, &[part(axis)])), &Limits::default()).unwrap();
        wb.sheets[0].charts[0].clone()
    };
    // oox AxisConverter: dateAx without auto = DATE; base unit, major unit
    // and time unit become the time increment.
    let c = read(
        r#"<c:dateAx><c:axId val="1"/><c:baseTimeUnit val="months"/><c:majorUnit val="3"/><c:majorTimeUnit val="months"/><c:crossesAt val="12.5"/></c:dateAx>"#,
    );
    assert_eq!(c.x_axis.axis_type, AxisTypeData::Date);
    assert_eq!(c.x_axis.time_resolution, Some(TimeUnitData::Months));
    assert_eq!(
        c.x_axis.major_time,
        Some(TimeIntervalData {
            number: 3,
            unit: TimeUnitData::Months
        })
    );
    assert_eq!(c.x_axis.major, None);
    assert_eq!(c.x_axis.crosses, Some(AxisCrossData::Value(12.5)));
    assert_eq!(c.y_axis.crosses, Some(AxisCrossData::End));
    // catAx with c:auto (Excel's default) is AOO's automatic axis.
    let c = read(r#"<c:catAx><c:axId val="1"/><c:auto val="1"/></c:catAx>"#);
    assert_eq!(c.x_axis.axis_type, AxisTypeData::Auto);
    assert_eq!(c.x_axis.crosses, None);
    let c = read(r#"<c:dateAx><c:axId val="1"/><c:auto val="1"/></c:dateAx>"#);
    assert_eq!(c.x_axis.axis_type, AxisTypeData::Auto);
}
