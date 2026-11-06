//! Charts and pictures through the `DocumentService` (wave 5, W5-1):
//! the ops with undo/redo, structure adjustment, change reporting,
//! `chart_data` resolution (values, labels, hidden rows, gaps, trend
//! lines, error bars), `suggest_chart`, protection and limits.

#![allow(clippy::unwrap_used)]

mod common;

use common::{input, range, Fixture};
use tally_api::{
    ApplyResult, CellPos, ChartAnchor, ChartAxisDto, ChartDto, ChartKind, ChartSeriesDto,
    ChartSourceDto, DataLabelsDto, DocumentService, ErrorBarDir, ErrorBarKind, ErrorBarsDto,
    ImageDto, LegendPos, Op, RangeDto, SeriesName, Stacking, TrendKind, TrendLineDto,
};
use tally_types::SheetId;

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01\x08\x06\x00\x00\x00\x1f\x15\xc4\x89\x00\x00\x00\rIDATx\x9cc\xf8\xcf\xc0\xf0\x1f\x00\x05\x00\x02\x00\xde\xf2\x8a\xbb\x00\x00\x00\x00IEND\xaeB`\x82";

fn anchor(r: u32, c: u32) -> ChartAnchor {
    ChartAnchor {
        r,
        c,
        dx_mm: 1.5,
        dy_mm: 0.5,
        width_mm: 160.0,
        height_mm: 90.0,
    }
}

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

fn series(values: &str, name: Option<&str>) -> ChartSeriesDto {
    ChartSeriesDto {
        name: name.map(|r| SeriesName::Ref { r#ref: r.into() }),
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

/// A column chart of `$Sheet1.$B$2:$B$6` with categories in column A.
fn column_chart() -> ChartDto {
    ChartDto {
        id: 0,
        name: String::new(),
        anchor: anchor(1, 5),
        kind: ChartKind::Column,
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
        categories: Some("$Sheet1.$A$2:$A$6".into()),
        series: vec![
            series("$Sheet1.$B$2:$B$6", Some("$Sheet1.$B$1")),
            series("$Sheet1.$C$2:$C$6", Some("$Sheet1.$C$1")),
        ],
        title: Some("Sales".into()),
        subtitle: None,
        legend: LegendPos::Right,
        x_axis: axis(),
        y_axis: ChartAxisDto {
            major_grid: true,
            ..axis()
        },
        y2_axis: None,
        background: None,
        plot_background: None,
        include_hidden: false,
    }
}

/// Fills A1:C6 with headers and five rows of numbers.
fn fill_data(f: &mut Fixture) {
    let s = f.sheet;
    let mut ops = vec![
        input(s, "A1", "Month"),
        input(s, "B1", "Sales"),
        input(s, "C1", "Costs"),
    ];
    for (i, (m, a, b)) in [
        ("Jan", 10, 4),
        ("Feb", 20, 8),
        ("Mar", 35, 12),
        ("Apr", 50, 20),
        ("May", 100, 40),
    ]
    .iter()
    .enumerate()
    {
        let r = i + 2;
        ops.push(input(s, &format!("A{r}"), m));
        ops.push(input(s, &format!("B{r}"), &a.to_string()));
        ops.push(input(s, &format!("C{r}"), &b.to_string()));
    }
    let res = f.apply(ops);
    assert!(res.errors.is_empty(), "{:?}", res.errors);
}

fn add(f: &mut Fixture, chart: ChartDto) -> (u32, ApplyResult) {
    let s = f.sheet;
    let res = f.apply(vec![Op::AddChart { sheet: s, chart }]);
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let id = res.created.as_ref().unwrap()[0].id;
    (id, res)
}

fn charts(f: &Fixture) -> Vec<ChartDto> {
    f.svc.charts(f.doc, f.sheet).unwrap()
}

fn round_trip(f: &mut Fixture, ops: Vec<Op>) -> ApplyResult {
    let before = f.data();
    let res = f.apply(ops.clone());
    assert!(res.errors.is_empty(), "{ops:?}: {:?}", res.errors);
    let after = f.data();
    assert_ne!(before, after, "{ops:?} changed nothing");
    f.svc.undo(f.doc).unwrap();
    assert_eq!(f.data(), before, "undo of {ops:?}");
    f.svc.redo(f.doc).unwrap();
    assert_eq!(f.data(), after, "redo of {ops:?}");
    res
}

#[test]
fn add_chart_returns_id_and_undo_removes() {
    let mut f = Fixture::new();
    fill_data(&mut f);
    let (id, res) = add(&mut f, column_chart());
    assert_eq!(res.created.unwrap()[0].op_index, 0);
    assert_eq!(res.objects_changed, Some(vec![f.sheet]));
    let cs = charts(&f);
    assert_eq!(cs.len(), 1);
    let c = &cs[0];
    assert_eq!(c.id, id);
    assert_eq!(c.name, format!("Object {id}"));
    assert_eq!(c.categories.as_deref(), Some("$Sheet1.$A$2:$A$6"));
    assert_eq!(c.series[0].values, "$Sheet1.$B$2:$B$6");
    assert_eq!(
        c.series[0].name,
        Some(SeriesName::Ref {
            r#ref: "$Sheet1.$B$1".into()
        })
    );
    assert_eq!(c.anchor, anchor(1, 5));
    assert!(f.data().sheets[0].charts.len() == 1);
    f.svc.undo(f.doc).unwrap();
    assert!(charts(&f).is_empty());
    f.svc.redo(f.doc).unwrap();
    assert_eq!(charts(&f).len(), 1);
}

#[test]
fn add_chart_derives_series_from_source() {
    let mut f = Fixture::new();
    fill_data(&mut f);
    let mut c = column_chart();
    c.series.clear();
    c.categories = None;
    c.source = Some(ChartSourceDto {
        range: "$Sheet1.$A$1:$C$6".into(),
        series_in_rows: false,
        first_row_label: true,
        first_col_label: true,
    });
    add(&mut f, c);
    let c = &charts(&f)[0];
    assert_eq!(c.categories.as_deref(), Some("$Sheet1.$A$2:$A$6"));
    assert_eq!(c.series.len(), 2);
    assert_eq!(c.series[1].values, "$Sheet1.$C$2:$C$6");
    assert_eq!(
        c.series[1].name,
        Some(SeriesName::Ref {
            r#ref: "$Sheet1.$C$1".into()
        })
    );
    // Series in rows.
    let mut c = column_chart();
    c.series.clear();
    c.categories = None;
    c.source = Some(ChartSourceDto {
        range: "$Sheet1.$A$1:$C$6".into(),
        series_in_rows: true,
        first_row_label: true,
        first_col_label: true,
    });
    add(&mut f, c);
    let c = &charts(&f)[1];
    assert_eq!(c.categories.as_deref(), Some("$Sheet1.$B$1:$C$1"));
    assert_eq!(c.series.len(), 5);
    assert_eq!(c.series[0].values, "$Sheet1.$B$2:$C$2");
}

#[test]
fn bad_ranges_are_refused() {
    let mut f = Fixture::new();
    let mut c = column_chart();
    c.series[0].values = "not a range".into();
    let s = f.sheet;
    let res = f.apply(vec![Op::AddChart { sheet: s, chart: c }]);
    assert_eq!(res.errors[0].code, "invalid_argument");
    let mut c = column_chart();
    c.anchor.r = 2_000_000;
    let res = f.apply(vec![Op::AddChart { sheet: s, chart: c }]);
    assert_eq!(res.errors[0].code, "invalid_argument");
    let mut c = column_chart();
    c.anchor.width_mm = 0.0;
    let res = f.apply(vec![Op::AddChart { sheet: s, chart: c }]);
    assert_eq!(res.errors[0].code, "invalid_argument");
    assert!(charts(&f).is_empty());
}

#[test]
fn update_delete_and_move_to_sheet() {
    let mut f = Fixture::new();
    fill_data(&mut f);
    let (id, _) = add(&mut f, column_chart());
    let s = f.sheet;
    let mut c = column_chart();
    c.kind = ChartKind::Line;
    c.title = Some("Updated".into());
    c.series.truncate(1);
    let res = round_trip(
        &mut f,
        vec![Op::UpdateChart {
            sheet: s,
            id,
            chart: c,
        }],
    );
    assert_eq!(res.objects_changed, Some(vec![s]));
    let got = &charts(&f)[0];
    assert_eq!(got.id, id);
    assert_eq!(got.kind, ChartKind::Line);
    assert_eq!(got.title.as_deref(), Some("Updated"));
    assert_eq!(got.series.len(), 1);
    // Unknown id.
    let res = f.apply(vec![Op::DeleteChart { sheet: s, id: 999 }]);
    assert_eq!(res.errors[0].code, "not_found");
    // Move to another sheet.
    let res = f.apply(vec![Op::AddSheet {
        name: Some("Other".into()),
        index: None,
    }]);
    assert!(res.errors.is_empty());
    let other: SheetId = res.info.unwrap().sheets[1].id;
    let res = round_trip(
        &mut f,
        vec![Op::MoveChartToSheet {
            sheet: s,
            id,
            to: other,
        }],
    );
    assert_eq!(res.objects_changed, Some(vec![s, other]));
    assert!(charts(&f).is_empty());
    let moved = f.svc.charts(f.doc, other).unwrap();
    assert_eq!(moved.len(), 1);
    // The series still point at Sheet1.
    assert_eq!(moved[0].series[0].values, "$Sheet1.$B$2:$B$6");
    let new_id = moved[0].id;
    round_trip(
        &mut f,
        vec![Op::DeleteChart {
            sheet: other,
            id: new_id,
        }],
    );
    assert!(f.svc.charts(f.doc, other).unwrap().is_empty());
}

#[test]
fn move_resize_undo_redo() {
    let mut f = Fixture::new();
    let (id, _) = add(&mut f, column_chart());
    let s = f.sheet;
    let a = ChartAnchor {
        r: 10,
        c: 2,
        dx_mm: 0.0,
        dy_mm: 2.25,
        width_mm: 80.0,
        height_mm: 45.0,
    };
    let res = round_trip(
        &mut f,
        vec![Op::MoveResizeChart {
            sheet: s,
            id,
            anchor: a,
        }],
    );
    assert_eq!(res.objects_changed, Some(vec![s]));
    assert_eq!(charts(&f)[0].anchor, a);
    f.svc.undo(f.doc).unwrap();
    assert_eq!(charts(&f)[0].anchor, anchor(1, 5));
}

#[test]
fn insert_rows_above_shifts_anchor_and_ranges() {
    let mut f = Fixture::new();
    fill_data(&mut f);
    add(&mut f, column_chart());
    let s = f.sheet;
    round_trip(
        &mut f,
        vec![Op::InsertRows {
            sheet: s,
            at: 0,
            count: 2,
        }],
    );
    let c = &charts(&f)[0];
    assert_eq!(c.anchor.r, 3);
    assert_eq!(c.categories.as_deref(), Some("$Sheet1.$A$4:$A$8"));
    assert_eq!(c.series[0].values, "$Sheet1.$B$4:$B$8");
    assert_eq!(
        c.series[0].name,
        Some(SeriesName::Ref {
            r#ref: "$Sheet1.$B$3".into()
        })
    );
    // Inserting inside the range grows it; inserting columns moves the
    // anchor column.
    round_trip(
        &mut f,
        vec![Op::InsertRows {
            sheet: s,
            at: 5,
            count: 1,
        }],
    );
    assert_eq!(charts(&f)[0].series[0].values, "$Sheet1.$B$4:$B$9");
    assert_eq!(
        charts(&f)[0].categories.as_deref(),
        Some("$Sheet1.$A$4:$A$9")
    );
    round_trip(
        &mut f,
        vec![Op::InsertCols {
            sheet: s,
            at: 1,
            count: 1,
        }],
    );
    let c = &charts(&f)[0];
    assert_eq!(c.anchor.c, 6);
    assert_eq!(c.series[0].values, "$Sheet1.$C$4:$C$9");
    assert_eq!(c.categories.as_deref(), Some("$Sheet1.$A$4:$A$9"));
}

#[test]
fn delete_rows_inside_series_shrinks_range() {
    let mut f = Fixture::new();
    fill_data(&mut f);
    add(&mut f, column_chart());
    let s = f.sheet;
    round_trip(
        &mut f,
        vec![Op::DeleteRows {
            sheet: s,
            at: 2,
            count: 2,
        }],
    );
    let c = &charts(&f)[0];
    assert_eq!(c.series[0].values, "$Sheet1.$B$2:$B$4");
    // The anchor row (1) is above the deletion and stays.
    assert_eq!(c.anchor.r, 1);
    // Deleting the anchor row keeps the chart where the rows were.
    round_trip(
        &mut f,
        vec![Op::DeleteRows {
            sheet: s,
            at: 1,
            count: 1,
        }],
    );
    assert_eq!(charts(&f)[0].anchor.r, 1);
    assert_eq!(charts(&f)[0].series[0].values, "$Sheet1.$B$2:$B$3");
}

#[test]
fn delete_whole_series_range_marks_invalid() {
    let mut f = Fixture::new();
    fill_data(&mut f);
    add(&mut f, column_chart());
    let s = f.sheet;
    round_trip(
        &mut f,
        vec![Op::DeleteCols {
            sheet: s,
            at: 1,
            count: 1,
        }],
    );
    let c = &charts(&f)[0];
    assert_eq!(c.series[0].values, "#REF!");
    assert_eq!(
        c.series[0].name,
        Some(SeriesName::Ref {
            r#ref: "#REF!".into()
        })
    );
    assert_eq!(c.series[1].values, "$Sheet1.$B$2:$B$6");
    let data = f.svc.chart_data(f.doc, s, c.id).unwrap();
    assert!(data.series[0].values.is_empty());
    assert!(data.problems.iter().any(|p| p.contains("#REF!")));
    assert_eq!(data.series[1].values.len(), 5);
}

#[test]
fn delete_referenced_sheet_invalidates_series() {
    let mut f = Fixture::new();
    fill_data(&mut f);
    let res = f.apply(vec![Op::AddSheet {
        name: Some("Other".into()),
        index: None,
    }]);
    let other: SheetId = res.info.unwrap().sheets[1].id;
    let mut c = column_chart();
    c.series[1].values = "$Other.$A$1:$A$3".into();
    c.series[1].name = None;
    add(&mut f, c);
    let before = f.data();
    let res = f.apply(vec![Op::DeleteSheet { sheet: other }]);
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let c = &charts(&f)[0];
    assert_eq!(c.series[1].values, "#REF!");
    assert_eq!(c.series[0].values, "$Sheet1.$B$2:$B$6");
    let data = f.svc.chart_data(f.doc, f.sheet, c.id).unwrap();
    assert!(data.series[1].values.is_empty());
    f.svc.undo(f.doc).unwrap();
    assert_eq!(f.data(), before);
    assert_eq!(charts(&f)[0].series[1].values, "$Other.$A$1:$A$3");
}

#[test]
fn cut_paste_moves_series_ranges() {
    let mut f = Fixture::new();
    fill_data(&mut f);
    add(&mut f, column_chart());
    let s = f.sheet;
    round_trip(
        &mut f,
        vec![Op::MoveRange {
            sheet: s,
            from: range("A1:C6"),
            to: CellPos { r: 10, c: 4 },
        }],
    );
    let c = &charts(&f)[0];
    assert_eq!(c.series[0].values, "$Sheet1.$F$12:$F$16");
    assert_eq!(c.categories.as_deref(), Some("$Sheet1.$E$12:$E$16"));
    assert_eq!(
        c.series[1].name,
        Some(SeriesName::Ref {
            r#ref: "$Sheet1.$G$11".into()
        })
    );
    // A chart anchored inside the moved block moves with it.
    let mut c2 = column_chart();
    c2.anchor = anchor(12, 5);
    add(&mut f, c2);
    round_trip(
        &mut f,
        vec![Op::MoveRange {
            sheet: s,
            from: range("E11:G16"),
            to: CellPos { r: 0, c: 0 },
        }],
    );
    let cs = charts(&f);
    assert_eq!(cs[1].anchor.r, 2);
    assert_eq!(cs[1].anchor.c, 1);
    assert_eq!(cs[0].series[0].values, "$Sheet1.$B$2:$B$6");
}

#[test]
fn edit_in_series_reports_charts_changed() {
    let mut f = Fixture::new();
    fill_data(&mut f);
    let (id, _) = add(&mut f, column_chart());
    let res = f.set("B3", "99");
    assert_eq!(
        res.charts_changed,
        Some(vec![tally_api::ChartRef { sheet: f.sheet, id }])
    );
    // Category cell too.
    let res = f.set("A4", "March");
    assert!(res.charts_changed.is_some());
}

#[test]
fn edit_elsewhere_reports_nothing() {
    let mut f = Fixture::new();
    fill_data(&mut f);
    add(&mut f, column_chart());
    let res = f.set("H20", "1");
    assert_eq!(res.charts_changed, None);
}

#[test]
fn recalc_dependent_reports_charts_changed() {
    let mut f = Fixture::new();
    fill_data(&mut f);
    f.set("B4", "=H1*2");
    let (id, _) = add(&mut f, column_chart());
    let res = f.set("H1", "7");
    assert_eq!(
        res.charts_changed,
        Some(vec![tally_api::ChartRef { sheet: f.sheet, id }])
    );
    let data = f.svc.chart_data(f.doc, f.sheet, id).unwrap();
    assert_eq!(data.series[0].values[2], Some(14.0));
}

#[test]
fn chart_data_values_labels_and_gaps() {
    let mut f = Fixture::new();
    fill_data(&mut f);
    f.set("B4", "oops");
    f.set("B5", "=1/0");
    f.set("C3", "");
    let (id, _) = add(&mut f, column_chart());
    let d = f.svc.chart_data(f.doc, f.sheet, id).unwrap();
    assert_eq!(d.chart.id, id);
    assert_eq!(d.categories, vec!["Jan", "Feb", "Mar", "Apr", "May"]);
    assert_eq!(d.series.len(), 2);
    assert_eq!(d.series[0].name, "Sales");
    assert_eq!(d.series[1].name, "Costs");
    assert_eq!(
        d.series[0].values,
        vec![Some(10.0), Some(20.0), None, None, Some(100.0)]
    );
    assert_eq!(d.series[0].labels[0], "10");
    assert_eq!(d.series[0].labels[2], "oops");
    assert_eq!(d.series[0].labels[3], "#DIV/0!");
    assert_eq!(d.series[1].values[1], None);
    assert_eq!(d.series[0].color, 0x0045_86ff);
    assert_eq!(d.series[1].color, 0xff42_0eff);
    assert!(!d.truncated && d.problems.is_empty());
    assert_eq!(d.version, f.svc.document(f.doc).unwrap().version);
}

#[test]
fn chart_data_on_invoices_template() {
    let mut svc = tally_engine::EngineService::new();
    let info = svc.new_workbook(Some("invoices")).unwrap();
    let sheet = info.sheets[0].id;
    let proposal = svc
        .suggest_chart(
            info.doc,
            sheet,
            RangeDto {
                r0: 0,
                c0: 3,
                r1: 14,
                c1: 5,
            },
            ChartKind::Column,
        )
        .unwrap();
    let res = svc
        .apply(
            info.doc,
            common::tx(vec![Op::AddChart {
                sheet,
                chart: proposal,
            }]),
        )
        .unwrap();
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let id = res.created.unwrap()[0].id;
    let d = svc.chart_data(info.doc, sheet, id).unwrap();
    assert_eq!(
        d.series.len(),
        3,
        "{:?}",
        d.series.iter().map(|s| &s.name).collect::<Vec<_>>()
    );
    assert_eq!(d.series[0].name, "Qty");
    assert_eq!(d.series[1].name, "Price");
    assert_eq!(d.series[2].name, "Total");
    assert_eq!(d.series[0].values.len(), 14);
    assert_eq!(d.series[0].values[0], Some(12.0));
    // Totals are formulas: cached values and formatted labels.
    assert_eq!(d.series[2].values[0], Some(54.0));
    assert!(d.series[1].number_format.is_some(), "price format");
    assert_eq!(d.categories.len(), 14);
}

#[test]
fn hidden_rows_excluded_unless_include_hidden() {
    let mut f = Fixture::new();
    fill_data(&mut f);
    let (id, _) = add(&mut f, column_chart());
    let s = f.sheet;
    let res = f.apply(vec![Op::SetRowsHidden {
        sheet: s,
        start: 2,
        end: 3,
        hidden: true,
    }]);
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    assert!(res.charts_changed.is_some(), "visibility change reported");
    let d = f.svc.chart_data(f.doc, s, id).unwrap();
    assert_eq!(
        d.series[0].values,
        vec![Some(10.0), Some(50.0), Some(100.0)]
    );
    assert_eq!(d.categories, vec!["Jan", "Apr", "May"]);
    let mut c = column_chart();
    c.include_hidden = true;
    f.apply(vec![Op::UpdateChart {
        sheet: s,
        id,
        chart: c,
    }]);
    let d = f.svc.chart_data(f.doc, s, id).unwrap();
    assert_eq!(d.series[0].values.len(), 5);
}

#[test]
fn trend_linear_matches_slope_intercept_rsq() {
    let mut f = Fixture::new();
    fill_data(&mut f);
    for i in 1..=5 {
        f.set(&format!("D{}", i + 1), &i.to_string());
    }
    f.set("E1", "=SLOPE(B2:B6,D2:D6)");
    f.set("E2", "=INTERCEPT(B2:B6,D2:D6)");
    f.set("E3", "=RSQ(B2:B6,D2:D6)");
    let mut c = column_chart();
    c.series[0].trend = Some(TrendLineDto {
        kind: TrendKind::Linear,
        show_equation: true,
        show_r2: true,
        name: None,
    });
    let (id, _) = add(&mut f, c);
    let d = f.svc.chart_data(f.doc, f.sheet, id).unwrap();
    let t = d.series[0].trend.as_ref().unwrap();
    let slope = match f.value("E1") {
        tally_types::Value::Number(n) => n,
        v => panic!("{v:?}"),
    };
    let icpt = match f.value("E2") {
        tally_types::Value::Number(n) => n,
        v => panic!("{v:?}"),
    };
    let rsq = match f.value("E3") {
        tally_types::Value::Number(n) => n,
        v => panic!("{v:?}"),
    };
    assert!((t.r2 - rsq).abs() < 1e-12, "{} vs {rsq}", t.r2);
    assert_eq!(t.points.len(), 5);
    assert!((t.points[0][1] - (slope + icpt)).abs() < 1e-9);
    assert!((t.points[4][1] - (slope * 5.0 + icpt)).abs() < 1e-9);
    assert!(t.equation.starts_with("f(x) = "), "{}", t.equation);
}

#[test]
fn trend_x_is_category_index_or_scatter_x() {
    let mut f = Fixture::new();
    fill_data(&mut f);
    let mut c = column_chart();
    c.series[0].trend = Some(TrendLineDto {
        kind: TrendKind::Linear,
        show_equation: false,
        show_r2: false,
        name: None,
    });
    let (id, _) = add(&mut f, c);
    let t = f.svc.chart_data(f.doc, f.sheet, id).unwrap().series[0]
        .trend
        .clone()
        .unwrap();
    let xs: Vec<f64> = t.points.iter().map(|p| p[0]).collect();
    assert_eq!(xs, vec![1.0, 2.0, 3.0, 4.0, 5.0]);
    let mut c = column_chart();
    c.kind = ChartKind::Scatter;
    c.categories = None;
    c.series.truncate(1);
    c.series[0].x_values = Some("$Sheet1.$C$2:$C$6".into());
    c.series[0].trend = Some(TrendLineDto {
        kind: TrendKind::Linear,
        show_equation: false,
        show_r2: false,
        name: None,
    });
    let (id, _) = add(&mut f, c);
    let t = f.svc.chart_data(f.doc, f.sheet, id).unwrap().series[0]
        .trend
        .clone()
        .unwrap();
    let xs: Vec<f64> = t.points.iter().map(|p| p[0]).collect();
    assert_eq!(xs, vec![4.0, 8.0, 12.0, 20.0, 40.0]);
}

#[test]
fn trend_log_exp_power() {
    let mut f = Fixture::new();
    fill_data(&mut f);
    for kind in [
        TrendKind::Logarithmic,
        TrendKind::Exponential,
        TrendKind::Power,
    ] {
        let mut c = column_chart();
        c.series[0].trend = Some(TrendLineDto {
            kind,
            show_equation: false,
            show_r2: false,
            name: None,
        });
        let (id, _) = add(&mut f, c);
        let d = f.svc.chart_data(f.doc, f.sheet, id).unwrap();
        let t = d.series[0].trend.as_ref().unwrap();
        assert!(t.r2 > 0.0 && t.r2 <= 1.0, "{kind:?}: {}", t.r2);
        assert_eq!(t.points.len(), 5);
        assert!(t.points.iter().all(|p| p[1].is_finite()));
    }
    // Exponential data fits exactly.
    let mut f = Fixture::new();
    let s = f.sheet;
    let ops: Vec<Op> = (1..=5)
        .map(|i| {
            input(
                s,
                &format!("A{i}"),
                &format!("{}", 2.0 * (0.3 * i as f64).exp()),
            )
        })
        .collect();
    f.apply(ops);
    let mut c = column_chart();
    c.categories = None;
    c.series = vec![series("$Sheet1.$A$1:$A$5", None)];
    c.series[0].trend = Some(TrendLineDto {
        kind: TrendKind::Exponential,
        show_equation: true,
        show_r2: true,
        name: None,
    });
    let (id, _) = add(&mut f, c);
    let t = f.svc.chart_data(f.doc, s, id).unwrap().series[0]
        .trend
        .clone()
        .unwrap();
    assert!((t.r2 - 1.0).abs() < 1e-9);
    assert!(
        t.equation.starts_with("f(x) = 2·exp(0.3x)"),
        "{}",
        t.equation
    );
}

#[test]
fn error_bars_stdev_matches_stdev_fn() {
    let mut f = Fixture::new();
    fill_data(&mut f);
    f.set("E1", "=STDEV(B2:B6)");
    f.set("E2", "=VAR(B2:B6)");
    let mut c = column_chart();
    c.series[0].error_bars = Some(ErrorBarsDto {
        kind: ErrorBarKind::StdDev { k: 2.0 },
        direction: ErrorBarDir::Both,
    });
    c.series[1].error_bars = Some(ErrorBarsDto {
        kind: ErrorBarKind::Variance,
        direction: ErrorBarDir::Minus,
    });
    let (id, _) = add(&mut f, c);
    let d = f.svc.chart_data(f.doc, f.sheet, id).unwrap();
    let sd = match f.value("E1") {
        tally_types::Value::Number(n) => n,
        v => panic!("{v:?}"),
    };
    let e = d.series[0].errors.as_ref().unwrap();
    assert_eq!(e.len(), 5);
    assert!((e[0][0] - 2.0 * sd).abs() < 1e-9 && (e[0][1] - 2.0 * sd).abs() < 1e-9);
    let e1 = d.series[1].errors.as_ref().unwrap();
    assert_eq!(e1[0][1], 0.0, "minus only");
    assert!(e1[0][0] > 0.0);
    // Standard error and error margin.
    let mut c = column_chart();
    c.series[0].error_bars = Some(ErrorBarsDto {
        kind: ErrorBarKind::StdError,
        direction: ErrorBarDir::Both,
    });
    c.series[1].error_bars = Some(ErrorBarsDto {
        kind: ErrorBarKind::ErrorMargin { percent: 10.0 },
        direction: ErrorBarDir::Both,
    });
    let (id, _) = add(&mut f, c);
    let d = f.svc.chart_data(f.doc, f.sheet, id).unwrap();
    let se = d.series[0].errors.as_ref().unwrap()[0][0];
    assert!((se - sd / 5f64.sqrt()).abs() < 1e-9);
    assert!((d.series[1].errors.as_ref().unwrap()[0][0] - 4.0).abs() < 1e-9);
}

#[test]
fn percent_error_bars() {
    let mut f = Fixture::new();
    fill_data(&mut f);
    let mut c = column_chart();
    c.series[0].error_bars = Some(ErrorBarsDto {
        kind: ErrorBarKind::Percent { percent: 10.0 },
        direction: ErrorBarDir::Plus,
    });
    c.series[1].error_bars = Some(ErrorBarsDto {
        kind: ErrorBarKind::Range {
            plus: "$Sheet1.$C$2:$C$6".into(),
            minus: "$Sheet1.$B$2:$B$6".into(),
        },
        direction: ErrorBarDir::Both,
    });
    c.series[1].mean_line = true;
    let (id, _) = add(&mut f, c);
    let d = f.svc.chart_data(f.doc, f.sheet, id).unwrap();
    let e = d.series[0].errors.as_ref().unwrap();
    assert_eq!(e[0], [0.0, 1.0]);
    assert_eq!(e[4], [0.0, 10.0]);
    let e = d.series[1].errors.as_ref().unwrap();
    assert_eq!(e[0], [10.0, 4.0]);
    assert_eq!(
        d.series[1].mean,
        Some((4.0 + 8.0 + 12.0 + 20.0 + 40.0) / 5.0)
    );
}

#[test]
fn suggest_chart_detects_region_and_labels() {
    let mut f = Fixture::new();
    fill_data(&mut f);
    let p = f
        .svc
        .suggest_chart(f.doc, f.sheet, range("B3"), ChartKind::Column)
        .unwrap();
    let src = p.source.as_ref().unwrap();
    assert_eq!(src.range, "$Sheet1.$A$1:$C$6");
    assert!(src.first_row_label && src.first_col_label && !src.series_in_rows);
    assert_eq!(p.categories.as_deref(), Some("$Sheet1.$A$2:$A$6"));
    assert_eq!(p.series.len(), 2);
    assert_eq!(p.series[0].values, "$Sheet1.$B$2:$B$6");
    assert_eq!((p.anchor.r, p.anchor.c), (0, 4));
    assert_eq!((p.anchor.width_mm, p.anchor.height_mm), (160.0, 90.0));
    assert_eq!(p.kind, ChartKind::Column);
    assert_eq!(p.legend, LegendPos::Right);
    // An explicit wide selection: series in rows.
    let p = f
        .svc
        .suggest_chart(f.doc, f.sheet, range("A1:C2"), ChartKind::Line)
        .unwrap();
    assert!(p.source.as_ref().unwrap().series_in_rows);
    assert_eq!(p.kind, ChartKind::Line);
    // The proposal adds as-is.
    let s = f.sheet;
    let res = f.apply(vec![Op::AddChart { sheet: s, chart: p }]);
    assert!(res.errors.is_empty(), "{:?}", res.errors);
}

#[test]
fn protected_sheet_refuses_chart_ops() {
    let mut f = Fixture::new();
    fill_data(&mut f);
    let (id, _) = add(&mut f, column_chart());
    let s = f.sheet;
    let res = f.apply(vec![Op::ProtectSheet {
        sheet: s,
        password: None,
        options: None,
    }]);
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    for op in [
        Op::AddChart {
            sheet: s,
            chart: column_chart(),
        },
        Op::UpdateChart {
            sheet: s,
            id,
            chart: column_chart(),
        },
        Op::MoveResizeChart {
            sheet: s,
            id,
            anchor: anchor(0, 0),
        },
        Op::DeleteChart { sheet: s, id },
        Op::AddImage {
            sheet: s,
            image: ImageDto {
                id: 0,
                name: String::new(),
                anchor: anchor(0, 0),
                mime: "image/png".into(),
                data: tally_engine::protection::base64(PNG),
            },
        },
    ] {
        let res = f.apply(vec![op.clone()]);
        assert_eq!(res.errors.len(), 1, "{op:?}");
        assert_eq!(res.errors[0].code, "protected", "{op:?}");
    }
    assert_eq!(charts(&f).len(), 1);
}

#[test]
fn limits_series_and_points_truncate() {
    let mut f = Fixture::new();
    let s = f.sheet;
    let mut c = column_chart();
    c.categories = None;
    c.series = (0..256)
        .map(|_| series("$Sheet1.$A$1:$A$2", None))
        .collect();
    let res = f.apply(vec![Op::AddChart { sheet: s, chart: c }]);
    assert_eq!(res.errors[0].code, "invalid_argument");
    // 40,000 points: the data is capped at 32,000 and flagged.
    let mut c = column_chart();
    c.categories = None;
    c.series = vec![series("$Sheet1.$A$1:$A$40000", None)];
    f.set("A40000", "1");
    let (id, _) = add(&mut f, c);
    let d = f.svc.chart_data(f.doc, s, id).unwrap();
    assert!(d.truncated);
    assert_eq!(d.series[0].values.len(), 32_000);
    // Union ranges and whole columns.
    let mut c = column_chart();
    c.categories = None;
    c.series = vec![series("$Sheet1.$A$1:$A$2~$Sheet1.$B$1:$B$2", None)];
    let (id, _) = add(&mut f, c);
    assert_eq!(
        charts(&f)[1].series[0].values,
        "$Sheet1.$A$1:$A$2~$Sheet1.$B$1:$B$2"
    );
    let d = f.svc.chart_data(f.doc, s, id).unwrap();
    assert_eq!(d.series[0].values.len(), 4);
}

#[test]
fn images_add_move_delete_with_limits() {
    let mut f = Fixture::new();
    let s = f.sheet;
    let img = ImageDto {
        id: 0,
        name: "Logo".into(),
        anchor: anchor(2, 2),
        mime: "image/png".into(),
        data: tally_engine::protection::base64(PNG),
    };
    let res = round_trip(
        &mut f,
        vec![Op::AddImage {
            sheet: s,
            image: img.clone(),
        }],
    );
    let id = res.created.unwrap()[0].id;
    assert_eq!(res.objects_changed, Some(vec![s]));
    let list = f.svc.images(f.doc, s).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, id);
    assert_eq!(list[0].mime, "image/png");
    assert!(list[0].data.is_empty(), "bytes are not listed");
    assert_eq!(list[0].anchor, anchor(2, 2));
    let full = f.svc.image_data(f.doc, s, id).unwrap();
    assert_eq!(tally_engine::protection::base64_decode(&full).unwrap(), PNG);
    assert_eq!(
        f.svc.image_data(f.doc, s, id + 7).unwrap_err().code,
        "not_found"
    );
    assert_eq!(&*f.data().sheets[0].images[0].bytes, PNG);
    round_trip(
        &mut f,
        vec![Op::MoveResizeImage {
            sheet: s,
            id,
            anchor: anchor(5, 5),
        }],
    );
    assert_eq!(f.svc.images(f.doc, s).unwrap()[0].anchor, anchor(5, 5));
    // Rows inserted above move it.
    round_trip(
        &mut f,
        vec![Op::InsertRows {
            sheet: s,
            at: 0,
            count: 3,
        }],
    );
    assert_eq!(f.svc.images(f.doc, s).unwrap()[0].anchor.r, 8);
    // Bad data.
    let mut bad = img.clone();
    bad.data = tally_engine::protection::base64(b"GIF89a....");
    let res = f.apply(vec![Op::AddImage {
        sheet: s,
        image: bad,
    }]);
    assert_eq!(res.errors[0].code, "invalid_argument");
    let mut bad = img.clone();
    bad.mime = "image/jpeg".into();
    let res = f.apply(vec![Op::AddImage {
        sheet: s,
        image: bad,
    }]);
    assert_eq!(res.errors[0].code, "invalid_argument");
    let mut bad = img;
    bad.data = "not base64 !!".into();
    let res = f.apply(vec![Op::AddImage {
        sheet: s,
        image: bad,
    }]);
    assert_eq!(res.errors[0].code, "invalid_argument");
    round_trip(&mut f, vec![Op::DeleteImage { sheet: s, id }]);
    assert!(f.svc.images(f.doc, s).unwrap().is_empty());
}

#[test]
fn too_large_image_refused() {
    let mut f = Fixture::new();
    let s = f.sheet;
    let mut bytes = PNG.to_vec();
    bytes.resize(16 * 1024 * 1024 + 1, 0);
    let res = f.apply(vec![Op::AddImage {
        sheet: s,
        image: ImageDto {
            id: 0,
            name: String::new(),
            anchor: anchor(0, 0),
            mime: String::new(),
            data: tally_engine::protection::base64(&bytes),
        },
    }]);
    assert_eq!(res.errors[0].code, "invalid_argument");
}

#[test]
fn charts_survive_save_and_reopen() {
    let mut f = Fixture::new();
    fill_data(&mut f);
    let mut c = column_chart();
    c.series[0].trend = Some(TrendLineDto {
        kind: TrendKind::Linear,
        show_equation: true,
        show_r2: false,
        name: None,
    });
    add(&mut f, c);
    let s = f.sheet;
    f.apply(vec![Op::AddImage {
        sheet: s,
        image: ImageDto {
            id: 0,
            name: "Logo".into(),
            anchor: anchor(8, 1),
            mime: "image/png".into(),
            data: tally_engine::protection::base64(PNG),
        },
    }]);
    let data = f.data();
    assert_eq!(data.sheets[0].charts.len(), 1);
    assert_eq!(data.sheets[0].charts[0].series[0].values.areas[0].0, 0);
    let wb = tally_engine::Workbook::from_data(&data);
    let again = wb.to_data();
    assert_eq!(again.sheets[0].charts, data.sheets[0].charts);
    assert_eq!(again.sheets[0].images, data.sheets[0].images);
}

// ---------------------------------------------------------------------------
// Date axis (T18)
// ---------------------------------------------------------------------------

/// Days since 1970-01-01 of a date.
fn unix_day(y: i32, m: u32, d: u32) -> f64 {
    tally_numfmt::date::serial_from_ymd(tally_types::DateEpoch::D1899, y, m, d).unwrap() - 25_569.0
}

/// A1:B5 with dates (ISO input: date-formatted numbers) and values.
fn fill_dates(f: &mut Fixture, dates: &[&str]) {
    let s = f.sheet;
    let mut ops = vec![input(s, "A1", "Day"), input(s, "B1", "Sales")];
    for (i, d) in dates.iter().enumerate() {
        ops.push(input(s, &format!("A{}", i + 2), d));
        ops.push(input(
            s,
            &format!("B{}", i + 2),
            &((i + 1) * 10).to_string(),
        ));
    }
    let res = f.apply(ops);
    assert!(res.errors.is_empty(), "{:?}", res.errors);
}

fn date_chart(kind: ChartKind, n: usize) -> ChartDto {
    ChartDto {
        kind,
        categories: Some(format!("$Sheet1.$A$2:$A${}", n + 1)),
        series: vec![series(&format!("$Sheet1.$B$2:$B${}", n + 1), None)],
        ..column_chart()
    }
}

#[test]
fn automatic_axis_is_a_date_axis_over_date_categories() {
    let mut f = Fixture::new();
    fill_dates(
        &mut f,
        &["2026-01-05", "2026-01-06", "2026-01-08", "2026-01-09"],
    );
    let (id, _) = add(&mut f, date_chart(ChartKind::Line, 4));
    let d = f.svc.chart_data(f.doc, f.sheet, id).unwrap();
    let axis = d.date_axis.expect("AOO's automatic type picks a date axis");
    // Two dates share a month: resolution days (calculateTimeResolutionOnXAxis).
    assert_eq!(axis.resolution, tally_api::TimeUnit::Days);
    assert_eq!(
        axis.days,
        vec![
            Some(unix_day(2026, 1, 5)),
            Some(unix_day(2026, 1, 6)),
            Some(unix_day(2026, 1, 8)),
            Some(unix_day(2026, 1, 9))
        ]
    );
    assert!(axis.number_format.is_some_and(|f| f.contains("YY")));
    // Text type, and kinds without a date axis (pie, net), keep text categories.
    let mut text = date_chart(ChartKind::Line, 4);
    text.x_axis.axis_type = tally_api::AxisType::Text;
    let (id, _) = add(&mut f, text);
    assert!(f
        .svc
        .chart_data(f.doc, f.sheet, id)
        .unwrap()
        .date_axis
        .is_none());
    let (id, _) = add(&mut f, date_chart(ChartKind::Pie, 4));
    assert!(f
        .svc
        .chart_data(f.doc, f.sheet, id)
        .unwrap()
        .date_axis
        .is_none());
}

#[test]
fn date_axis_resolution_and_explicit_type() {
    let mut f = Fixture::new();
    // One date per month: months; the days floor to the 1st.
    fill_dates(&mut f, &["2026-01-15", "2026-02-15", "2026-04-15"]);
    let (id, _) = add(&mut f, date_chart(ChartKind::Column, 3));
    let d = f
        .svc
        .chart_data(f.doc, f.sheet, id)
        .unwrap()
        .date_axis
        .unwrap();
    assert_eq!(d.resolution, tally_api::TimeUnit::Months);
    assert_eq!(d.days[0], Some(unix_day(2026, 1, 1)));
    assert_eq!(d.days[2], Some(unix_day(2026, 4, 1)));
    // An explicit resolution wins.
    let mut c = date_chart(ChartKind::Column, 3);
    c.x_axis.time_resolution = Some(tally_api::TimeUnit::Years);
    let (id, _) = add(&mut f, c);
    let d = f
        .svc
        .chart_data(f.doc, f.sheet, id)
        .unwrap()
        .date_axis
        .unwrap();
    assert_eq!(d.days, vec![Some(unix_day(2026, 1, 1)); 3]);
    // Plain numbers: automatic stays text, Date places them as dates.
    let mut f = Fixture::new();
    fill_dates(&mut f, &["46000", "46001", "46003"]);
    let s = f.sheet;
    let (id, _) = add(&mut f, date_chart(ChartKind::Line, 3));
    assert!(f
        .svc
        .chart_data(f.doc, f.sheet, id)
        .unwrap()
        .date_axis
        .is_none());
    let mut c = date_chart(ChartKind::Line, 3);
    c.x_axis.axis_type = tally_api::AxisType::Date;
    let (id, _) = add(&mut f, c);
    let d = f
        .svc
        .chart_data(f.doc, f.sheet, id)
        .unwrap()
        .date_axis
        .unwrap();
    assert_eq!(d.days[2], Some(46_003.0 - 25_569.0));
    // A text category makes it a text axis even when Date is asked for.
    f.apply(vec![input(s, "A3", "later")]);
    let d = f.svc.chart_data(f.doc, f.sheet, id).unwrap();
    assert!(d.date_axis.is_none());
}

#[test]
fn axis_settings_validated_and_kept() {
    let mut f = Fixture::new();
    fill_data(&mut f);
    let mut c = column_chart();
    c.x_axis.axis_type = tally_api::AxisType::Date;
    c.x_axis.major_time = Some(tally_api::TimeIntervalDto {
        number: 2,
        unit: tally_api::TimeUnit::Months,
    });
    c.x_axis.crosses = Some(tally_api::AxisCross::Value { value: 50.0 });
    c.x_axis.label_position = tally_api::AxisLabelPos::OutsideStart;
    c.y_axis.crosses = Some(tally_api::AxisCross::End);
    c.y_axis.mark_position = tally_api::AxisMarkPos::AtAxis;
    let (id, _) = add(&mut f, c.clone());
    let got = charts(&f).into_iter().find(|x| x.id == id).unwrap();
    assert_eq!(got.x_axis, c.x_axis);
    assert_eq!(got.y_axis, c.y_axis);
    let mut bad = column_chart();
    bad.x_axis.major_time = Some(tally_api::TimeIntervalDto {
        number: 0,
        unit: tally_api::TimeUnit::Days,
    });
    let res = f.apply(vec![Op::AddChart {
        sheet: f.sheet,
        chart: bad,
    }]);
    assert!(!res.errors.is_empty());
    let mut bad = column_chart();
    bad.y_axis.crosses = Some(tally_api::AxisCross::Value { value: f64::NAN });
    let res = f.apply(vec![Op::AddChart {
        sheet: f.sheet,
        chart: bad,
    }]);
    assert!(!res.errors.is_empty());
}
