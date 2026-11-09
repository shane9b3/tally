//! Regression tests for the fourth security and correctness reviews
//! (`docs/reviews/security-review-4-2026-10-21.md`,
//! `docs/reviews/correctness-review-4-2026-10-21.md`), engine side:
//! C1 (document picture total on load), C2 (literal points and series
//! per document on load), C3 (`chart_data` work and size budget, trend
//! sampling, caching) and M20 (`suggest_chart` label detection as in
//! AOO's `chartpos.cxx`).

#![allow(clippy::unwrap_used)] // tests may unwrap (docs/contributing.md)

mod common;

use std::sync::Arc;
use std::time::Instant;

use common::{input, range, Fixture};
use tally_api::{
    ChartAnchor, ChartAxisDto, ChartDto, ChartKind, ChartSeriesDto, DataLabelsDto, DocumentService,
    LegendPos, Op, RangeDto, Stacking, TrendKind, TrendLineDto,
};
use tally_engine::workbook::Workbook;
use tally_engine::EngineService;
use tally_model::{
    ChartData, ChartRangeData, ChartSeriesData, ChartValuesData, ImageData, SheetData,
    WorkbookData, MAX_LITERAL_POINTS, MAX_SERIES_PER_DOCUMENT, MAX_TOTAL_IMAGE_BYTES,
};

fn axis() -> ChartAxisDto {
    ChartAxisDto {
        visible: true,
        title: None,
        min: None,
        max: None,
        major: None,
        minor_count: None,
        log: false,
        reverse: false,
        major_grid: false,
        minor_grid: false,
        number_format: None,
        axis_type: tally_api::AxisType::Auto,
        time_resolution: None,
        major_time: None,
        minor_time: None,
        crosses: None,
        label_position: tally_api::AxisLabelPos::NearAxis,
        mark_position: tally_api::AxisMarkPos::AtLabelsAndAxis,
    }
}

fn series(values: &str) -> ChartSeriesDto {
    ChartSeriesDto {
        name: None,
        values: values.into(),
        x_values: None,
        sizes: None,
        kind: None,
        secondary_axis: false,
        color: None,
        marker: None,
        labels: DataLabelsDto::default(),
        trend: None,
        error_bars: None,
        mean_line: false,
    }
}

/// A line chart of `n` series over `values`, no categories.
fn line_chart(values: &str, n: usize) -> ChartDto {
    ChartDto {
        id: 0,
        name: String::new(),
        anchor: ChartAnchor {
            r: 0,
            c: 5,
            dx_mm: 0.0,
            dy_mm: 0.0,
            width_mm: 160.0,
            height_mm: 90.0,
        },
        kind: ChartKind::Line,
        stacking: Stacking::None,
        lines: true,
        markers: false,
        smooth: false,
        hole: None,
        exploded: false,
        filled: false,
        stock: None,
        three_d: false,
        source: None,
        categories: None,
        series: (0..n).map(|_| series(values)).collect(),
        title: None,
        subtitle: None,
        legend: LegendPos::Right,
        x_axis: axis(),
        y_axis: axis(),
        y2_axis: None,
        background: None,
        plot_background: None,
        include_hidden: false,
        own_data: false,
        data_table: None,
    }
}

fn add(f: &mut Fixture, chart: ChartDto) -> u32 {
    let s = f.sheet;
    let res = f.apply(vec![Op::AddChart { sheet: s, chart }]);
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    res.created.as_ref().unwrap()[0].id
}

fn fill_column(f: &mut Fixture, col: u32, n: u32) {
    let s = f.sheet;
    f.apply(vec![Op::SetRangeInputs {
        sheet: s,
        r: 0,
        c: col,
        rows: (0..n).map(|i| vec![format!("{}", i % 97)]).collect(),
    }]);
}

fn open_data(data: WorkbookData) -> (EngineService, tally_api::WorkbookInfo) {
    let mut svc = EngineService::new();
    let info = svc.open_workbook(Workbook::from_data(&data), "t");
    (svc, info)
}

/// C1: the engine applies the 256 MiB picture total on load, counting
/// every reference (the readers share one copy of the bytes).
#[test]
fn loaded_pictures_are_counted_per_reference() {
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.resize(1024 * 1024, 0);
    let bytes: Arc<[u8]> = png.into();
    let image = ImageData {
        name: "p".into(),
        anchor: tally_model::ChartAnchorData {
            width_hmm: 1000,
            height_hmm: 1000,
            ..tally_model::ChartAnchorData::default()
        },
        mime: "image/png".into(),
        bytes: Arc::clone(&bytes),
    };
    let sheet = |name: &str| SheetData {
        name: name.into(),
        images: vec![image.clone(); 200],
        ..SheetData::default()
    };
    let (svc, info) = open_data(WorkbookData {
        sheets: vec![sheet("A"), sheet("B")],
        ..WorkbookData::default()
    });
    let n: usize = info
        .sheets
        .iter()
        .map(|s| svc.images(info.doc, s.id).unwrap().len())
        .sum();
    assert_eq!(n, MAX_TOTAL_IMAGE_BYTES / bytes.len());
    assert!(
        info.warnings.iter().any(|w| w.contains("256 MiB")),
        "{:?}",
        info.warnings
    );
}

/// C2: literal points and series are budgeted per document on load.
#[test]
fn loaded_literals_and_series_are_budgeted() {
    let lit = ChartData {
        series: vec![ChartSeriesData {
            literal_values: Some(ChartValuesData::Literal(vec![Some(1.0); 32_000])),
            ..ChartSeriesData::default()
        }],
        ..ChartData::default()
    };
    let wide = ChartData {
        series: vec![
            ChartSeriesData {
                values: ChartRangeData {
                    areas: vec![(
                        0,
                        tally_model::CellRangeLocal {
                            r0: 0,
                            c0: 0,
                            r1: 4,
                            c1: 0
                        }
                    )],
                    invalid: false,
                },
                ..ChartSeriesData::default()
            };
            255
        ],
        ..ChartData::default()
    };
    let mut charts = vec![lit; 40];
    charts.extend(vec![wide; 200]);
    let (svc, info) = open_data(WorkbookData {
        sheets: vec![SheetData {
            name: "S".into(),
            charts,
            ..SheetData::default()
        }],
        ..WorkbookData::default()
    });
    let doc = svc.document(info.doc).unwrap();
    let all: Vec<_> = doc.wb.sheets[0].meta.charts.iter().collect();
    assert_eq!(all.len(), 240);
    let pts: usize = all.iter().map(|c| c.data.literal_points()).sum();
    assert!(pts <= MAX_LITERAL_POINTS, "{pts}");
    assert_eq!(pts, MAX_LITERAL_POINTS / 32_000 * 32_000);
    let series: usize = all.iter().map(|c| c.data.series.len()).sum();
    assert!(series <= MAX_SERIES_PER_DOCUMENT, "{series}");
    assert!(info.warnings.iter().any(|w| w.contains("cached values")));
    assert!(info
        .warnings
        .iter()
        .any(|w| w.contains("series per document")));
}

/// The series budget also holds for `AddChart`.
#[test]
fn add_chart_respects_the_series_budget() {
    let mut f = Fixture::new();
    let full = MAX_SERIES_PER_DOCUMENT / 255;
    let s = f.sheet;
    let ops: Vec<Op> = (0..full)
        .map(|_| Op::AddChart {
            sheet: s,
            chart: line_chart("$Sheet1.$B$1:$B$5", 255),
        })
        .collect();
    let res = f.apply(ops);
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let res = f.apply(vec![Op::AddChart {
        sheet: s,
        chart: line_chart("$Sheet1.$B$1:$B$5", 255),
    }]);
    assert!(
        res.errors
            .iter()
            .any(|e| e.message.contains("chart series")),
        "{:?}",
        res.errors
    );
}

/// C3 (`probe5 op hidden`): a series over `A1:IU1048576` with its
/// columns hidden took > 60 s for 255 series and returned 0 points.
/// A fully hidden column set costs one pass over the columns.
#[test]
fn hidden_columns_are_skipped_in_one_step() {
    let mut f = Fixture::new();
    let s = f.sheet;
    f.apply(vec![
        input(s, "A1", "1"),
        input(s, "A1048576", "2"),
        Op::SetColsHidden {
            sheet: s,
            start: 0,
            end: 254,
            hidden: true,
        },
    ]);
    let id = add(&mut f, line_chart("$Sheet1.$A$1:$IU$1048576", 255));
    let t = Instant::now();
    let d = f.svc.chart_data(f.doc, s, id).unwrap();
    assert!(t.elapsed().as_secs() < 10, "{:?}", t.elapsed());
    assert_eq!(d.series.iter().map(|s| s.values.len()).sum::<usize>(), 0);
}

/// C3: hidden rows are charged to the visit budget, so walking them
/// stops at 2M cells with `truncated`.
#[test]
fn hidden_rows_count_against_the_visit_budget() {
    let mut f = Fixture::new();
    let s = f.sheet;
    f.apply(vec![
        input(s, "B1", "1"),
        input(s, "B1048576", "2"),
        Op::SetRowsHidden {
            sheet: s,
            start: 1,
            end: 1_048_574,
            hidden: true,
        },
    ]);
    let id = add(&mut f, line_chart("$Sheet1.$B$1:$B$1048576", 3));
    let d = f.svc.chart_data(f.doc, s, id).unwrap();
    assert!(d.truncated);
    assert_eq!(d.series[0].values, vec![Some(1.0), Some(2.0)]);
    assert!(d.series.len() < 3);
}

/// C3: a response holds at most 100,000 points over all series, and a
/// trend line at most 500.
#[test]
fn responses_are_capped_and_trends_sampled() {
    let mut f = Fixture::new();
    fill_column(&mut f, 1, 32_000);
    let id = add(&mut f, line_chart("$Sheet1.$B$1:$B$32000", 4));
    let d = f.svc.chart_data(f.doc, f.sheet, id).unwrap();
    assert!(d.truncated);
    let pts: usize = d.series.iter().map(|s| s.values.len()).sum();
    assert!(pts <= tally_engine::charts::MAX_CHART_POINTS, "{pts}");
    assert_eq!(d.series[0].values.len(), 32_000);

    let mut c = line_chart("$Sheet1.$B$1:$B$2000", 1);
    c.series[0].trend = Some(TrendLineDto {
        kind: TrendKind::Linear,
        show_equation: false,
        show_r2: false,
        name: None,
    });
    let id = add(&mut f, c);
    let d = f.svc.chart_data(f.doc, f.sheet, id).unwrap();
    let t = d.series[0].trend.as_ref().unwrap();
    assert_eq!(t.points.len(), tally_engine::charts::MAX_TREND_POINTS);
    assert_eq!(t.points[0][0], 1.0);
    assert_eq!(t.points.last().unwrap()[0], 2000.0);
}

/// C3: responses are cached per document version, shared by charts with
/// the same data definition, and recomputed after an edit.
#[test]
fn chart_data_is_cached_per_version() {
    let mut f = Fixture::new();
    fill_column(&mut f, 1, 10);
    let a = add(&mut f, line_chart("$Sheet1.$B$1:$B$10", 2));
    let mut other = line_chart("$Sheet1.$B$1:$B$10", 2);
    other.anchor.r = 20;
    let b = add(&mut f, other);
    let s = f.sheet;
    let da = f.svc.chart_data(f.doc, s, a).unwrap();
    let db = f.svc.chart_data(f.doc, s, b).unwrap();
    assert_eq!((da.chart.id, db.chart.id), (a, b));
    assert_eq!(da.series, db.series);
    assert_eq!(f.svc.chart_data(f.doc, s, a).unwrap(), da);
    f.apply(vec![input(s, "B2", "1234")]);
    let d2 = f.svc.chart_data(f.doc, s, a).unwrap();
    assert_eq!(d2.series[0].values[1], Some(1234.0));
    assert!(d2.version > da.version);
    f.svc.undo(f.doc).unwrap();
    let d3 = f.svc.chart_data(f.doc, s, b).unwrap();
    assert_eq!(d3.series[0].values[1], da.series[0].values[1]);
}

fn suggest(f: &Fixture, sel: &str) -> ChartDto {
    f.svc
        .suggest_chart(f.doc, f.sheet, range(sel), ChartKind::Column)
        .unwrap()
}

/// M20 (`c4_chpr::ch01`): AOO's `CheckColRowHeaders` treats dates and
/// blanks as labels and a plain number anywhere in the first row or
/// column (the corner included) as data.
#[test]
fn suggest_detects_labels_like_aoo() {
    // (a) A date column is the category axis.
    let mut f = Fixture::new();
    let s = f.sheet;
    f.apply(
        [
            ("A1", "Date"),
            ("B1", "Sales"),
            ("A2", "2026-01-01"),
            ("B2", "10"),
            ("A3", "2026-02-01"),
            ("B3", "20"),
        ]
        .iter()
        .map(|(a, t)| input(s, a, t))
        .collect(),
    );
    let p = suggest(&f, "B2");
    let src = p.source.as_ref().unwrap();
    assert!(src.first_row_label && src.first_col_label);
    assert_eq!(p.categories.as_deref(), Some("$Sheet1.$A$2:$A$3"));
    assert_eq!(p.series.len(), 1);
    // (b) A blank header cell still makes the first row labels.
    let mut f = Fixture::new();
    let s = f.sheet;
    f.apply(
        [
            ("A1", "Month"),
            ("C1", "Costs"),
            ("A2", "Jan"),
            ("A3", "Feb"),
            ("B2", "1"),
            ("B3", "2"),
            ("C2", "3"),
            ("C3", "4"),
        ]
        .iter()
        .map(|(a, t)| input(s, a, t))
        .collect(),
    );
    let p = suggest(&f, "A1:C3");
    assert!(p.source.as_ref().unwrap().first_row_label);
    assert_eq!(p.series.len(), 2);
    // (c) Numeric years in the first row are data, not names.
    let mut f = Fixture::new();
    let s = f.sheet;
    f.apply(
        [
            ("A1", "Item"),
            ("B1", "2024"),
            ("C1", "2025"),
            ("A2", "x"),
            ("B2", "1"),
            ("C2", "2"),
            ("A3", "y"),
            ("B3", "3"),
            ("C3", "4"),
        ]
        .iter()
        .map(|(a, t)| input(s, a, t))
        .collect(),
    );
    let src = suggest(&f, "A1:C3").source.unwrap();
    assert!(!src.first_row_label && src.first_col_label);
    // (d) A number in the corner: neither labels.
    f.apply(vec![
        input(s, "A1", "5"),
        input(s, "B1", "S1"),
        input(s, "C1", "S2"),
    ]);
    let src = suggest(&f, "A1:C3").source.unwrap();
    assert!(!src.first_row_label && !src.first_col_label);
}

/// M20 (`c4_chpr::ch02`): on the invoices template the Date column was
/// plotted as a series (46296…); it is the category axis.
#[test]
fn suggest_on_invoices_uses_dates_as_categories() {
    let mut svc = EngineService::new();
    let info = svc.new_workbook(Some("invoices")).unwrap();
    let s = info.sheets[0].id;
    let p = svc
        .suggest_chart(
            info.doc,
            s,
            RangeDto {
                r0: 3,
                c0: 3,
                r1: 3,
                c1: 3,
            },
            ChartKind::Column,
        )
        .unwrap();
    let src = p.source.as_ref().unwrap();
    assert!(src.first_row_label && src.first_col_label, "{src:?}");
    let cats = p.categories.as_deref().unwrap();
    assert!(cats.contains(".$A$2:$A$"), "{cats}");
    assert!(p.series.iter().all(|x| !x.values.contains("$A$")));
}
