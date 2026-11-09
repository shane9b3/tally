//! The Chart Data Table (T18; AOO 4.1 View ▸ Data Table,
//! `chart2/source/controller/dialogs/DataBrowserModel.cxx`): charts that
//! read no cells carry their own data, `charts()` flags them (`own_data`)
//! and `chart_data_table` hands the table out (capped),
//! `SetChartDataTable` replaces it as one undo step (rows
//! and series inserted, deleted and moved, formatting following its
//! series), charts over cells refuse it, and the edited data survive an
//! ODS save.

#![allow(clippy::unwrap_used)]

mod common;

use common::Fixture;
use tally_api::{
    ChartAnchor, ChartAxisDto, ChartDataTableDto, ChartDto, ChartKind, ChartSeriesDto,
    DataLabelsDto, DataTableSeriesDto, DocumentService, LegendPos, Op, SeriesName, Stacking,
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

fn series(values: &str, color: Option<u32>) -> ChartSeriesDto {
    ChartSeriesDto {
        name: None,
        values: values.into(),
        x_values: None,
        sizes: None,
        kind: None,
        secondary_axis: false,
        color,
        marker: None,
        labels: DataLabelsDto::default(),
        trend: None,
        error_bars: None,
        mean_line: false,
    }
}

fn chart(
    kind: ChartKind,
    series: Vec<ChartSeriesDto>,
    table: Option<ChartDataTableDto>,
) -> ChartDto {
    ChartDto {
        id: 0,
        name: String::new(),
        anchor: ChartAnchor {
            r: 1,
            c: 4,
            dx_mm: 0.0,
            dy_mm: 0.0,
            width_mm: 160.0,
            height_mm: 90.0,
        },
        kind,
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
        series,
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
        data_table: table,
    }
}

fn col(source: Option<u32>, name: &str, values: &[Option<f64>]) -> DataTableSeriesDto {
    DataTableSeriesDto {
        source,
        name: name.into(),
        values: values.to_vec(),
        x: None,
        sizes: None,
    }
}

fn table(cats: &[&str], series: Vec<DataTableSeriesDto>) -> ChartDataTableDto {
    ChartDataTableDto {
        categories: cats.iter().map(|c| (*c).to_string()).collect(),
        series,
    }
}

/// A pasted column chart with its own data: Q1..Q3, "North" 1/2/3 (red)
/// and "South" 4/5/6.
fn own_chart(f: &mut Fixture) -> u32 {
    let t = table(
        &["Q1", "Q2", "Q3"],
        vec![
            col(Some(0), "North", &[Some(1.0), Some(2.0), Some(3.0)]),
            col(Some(1), "South", &[Some(4.0), Some(5.0), Some(6.0)]),
        ],
    );
    let c = chart(
        ChartKind::Column,
        vec![series("", Some(0xff0000ff)), series("", None)],
        Some(t),
    );
    let s = f.sheet;
    let res = f.apply(vec![Op::AddChart { sheet: s, chart: c }]);
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    res.created.unwrap()[0].id
}

fn dto(f: &Fixture, id: u32) -> ChartDto {
    f.svc
        .charts(f.doc, f.sheet)
        .unwrap()
        .into_iter()
        .find(|c| c.id == id)
        .unwrap()
}

fn own_table(f: &Fixture, id: u32) -> ChartDataTableDto {
    f.svc.chart_data_table(f.doc, f.sheet, id).unwrap()
}

fn set(f: &mut Fixture, id: u32, t: ChartDataTableDto) -> tally_api::ApplyResult {
    let s = f.sheet;
    f.apply(vec![Op::SetChartDataTable {
        sheet: s,
        id,
        table: t,
    }])
}

#[test]
fn own_data_is_handed_out_and_drawn() {
    let mut f = Fixture::new();
    let id = own_chart(&mut f);
    let c = dto(&f, id);
    // The list flags the chart; the table is a separate query.
    assert!(c.own_data);
    assert!(c.data_table.is_none());
    let t = own_table(&f, id);
    assert_eq!(t.categories, ["Q1", "Q2", "Q3"]);
    assert_eq!(t.series[0].name, "North");
    assert_eq!(t.series[1].source, Some(1));
    let d = f.svc.chart_data(f.doc, f.sheet, id).unwrap();
    assert_eq!(d.categories, ["Q1", "Q2", "Q3"]);
    assert_eq!(d.series[0].name, "North");
    assert_eq!(d.series[1].values, [Some(4.0), Some(5.0), Some(6.0)]);
}

#[test]
fn rows_and_series_are_edited_in_one_undo_step() {
    let mut f = Fixture::new();
    let id = own_chart(&mut f);
    let before = f.data();
    // Insert Row after Q1 with a value, delete series North, insert a new
    // series after South, edit a value.
    let t = table(
        &["Q1", "", "Q2", "Q3"],
        vec![
            col(
                Some(1),
                "South",
                &[Some(4.0), Some(4.5), Some(5.0), Some(60.0)],
            ),
            col(None, "", &[None, None, Some(1.0), None]),
        ],
    );
    let res = set(&mut f, id, t.clone());
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let d = f.svc.chart_data(f.doc, f.sheet, id).unwrap();
    assert_eq!(d.categories.len(), 4);
    assert_eq!(d.series.len(), 2);
    assert_eq!(d.series[0].name, "South");
    assert_eq!(
        d.series[0].values,
        [Some(4.0), Some(4.5), Some(5.0), Some(60.0)]
    );
    let after = dto(&f, id);
    assert_eq!(own_table(&f, id).categories, t.categories);
    // The new series has default formatting; South kept its own.
    assert_eq!(after.series[0].color, None);
    assert_eq!(after.series[1].color, None);
    let edited = f.data();
    f.svc.undo(f.doc).unwrap();
    assert_eq!(f.data(), before);
    f.svc.redo(f.doc).unwrap();
    assert_eq!(f.data(), edited);
}

#[test]
fn moving_a_series_takes_its_formatting_along() {
    let mut f = Fixture::new();
    let id = own_chart(&mut f);
    // Move Series Right: North (red) after South.
    let t = table(
        &["Q1", "Q2", "Q3"],
        vec![
            col(Some(1), "South", &[Some(4.0), Some(5.0), Some(6.0)]),
            col(Some(0), "North", &[Some(1.0), Some(2.0), Some(3.0)]),
        ],
    );
    assert!(set(&mut f, id, t).errors.is_empty());
    let c = dto(&f, id);
    assert_eq!(c.series[1].color, Some(0xff0000ff));
    assert_eq!(own_table(&f, id).series[1].name, "North");
    // Move Row Down: Q1 after Q2.
    let t = table(
        &["Q2", "Q1", "Q3"],
        vec![
            col(Some(0), "South", &[Some(5.0), Some(4.0), Some(6.0)]),
            col(Some(1), "North", &[Some(2.0), Some(1.0), Some(3.0)]),
        ],
    );
    assert!(set(&mut f, id, t).errors.is_empty());
    let d = f.svc.chart_data(f.doc, f.sheet, id).unwrap();
    assert_eq!(d.categories, ["Q2", "Q1", "Q3"]);
    assert_eq!(d.series[1].values, [Some(2.0), Some(1.0), Some(3.0)]);
}

#[test]
fn bad_tables_and_charts_over_cells_are_refused() {
    let mut f = Fixture::new();
    let id = own_chart(&mut f);
    // A column shorter than the categories.
    let res = set(
        &mut f,
        id,
        table(&["a", "b"], vec![col(None, "", &[Some(1.0)])]),
    );
    assert_eq!(res.errors[0].code, "invalid_argument");
    // Not a number.
    let res = set(
        &mut f,
        id,
        table(&["a"], vec![col(None, "", &[Some(f64::NAN)])]),
    );
    assert_eq!(res.errors[0].code, "invalid_argument");
    // A chart over cells edits its cells instead (AOO disables the dialog).
    f.set("A1", "1");
    let s = f.sheet;
    let res = f.apply(vec![Op::AddChart {
        sheet: s,
        chart: chart(
            ChartKind::Column,
            vec![series("$Sheet1.$A$1:$A$1", None)],
            None,
        ),
    }]);
    let cell_chart = res.created.unwrap()[0].id;
    assert!(!dto(&f, cell_chart).own_data);
    let err = f
        .svc
        .chart_data_table(f.doc, f.sheet, cell_chart)
        .unwrap_err();
    assert_eq!(err.code, "invalid_argument");
    let res = set(
        &mut f,
        cell_chart,
        table(&["a"], vec![col(None, "", &[Some(1.0)])]),
    );
    assert_eq!(res.errors[0].code, "invalid_argument");
}

#[test]
fn scatter_x_values_and_a_name_ref_free_chart() {
    let mut f = Fixture::new();
    let mut t = table(&["", ""], vec![col(None, "Fit", &[Some(2.0), Some(4.0)])]);
    t.series[0].x = Some(vec![Some(1.0), Some(2.0)]);
    let s = f.sheet;
    let res = f.apply(vec![Op::AddChart {
        sheet: s,
        chart: chart(ChartKind::Scatter, Vec::new(), Some(t)),
    }]);
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let id = res.created.unwrap()[0].id;
    let c = dto(&f, id);
    let got = own_table(&f, id);
    assert_eq!(got.series[0].x, Some(vec![Some(1.0), Some(2.0)]));
    // All-empty categories are not kept.
    assert_eq!(got.categories, ["", ""]);
    assert!(matches!(c.series[0].name, Some(SeriesName::Text { .. })));
    let d = f.svc.chart_data(f.doc, f.sheet, id).unwrap();
    assert_eq!(d.series[0].x, Some(vec![Some(1.0), Some(2.0)]));
    // `UpdateChart` (the chart editor) keeps the own data, X included.
    let mut c = dto(&f, id);
    c.title = Some("XY".into());
    c.data_table = None;
    let res = f.apply(vec![Op::UpdateChart {
        sheet: s,
        id,
        chart: c,
    }]);
    assert!(res.errors.is_empty());
    let d = f.svc.chart_data(f.doc, f.sheet, id).unwrap();
    assert_eq!(d.series[0].x, Some(vec![Some(1.0), Some(2.0)]));
}

#[test]
fn edited_own_data_survive_an_ods_save() {
    let mut f = Fixture::new();
    let id = own_chart(&mut f);
    let t = table(
        &["Q1", "Q2", "Q3", "Q4"],
        vec![
            col(Some(0), "North", &[Some(1.0), None, Some(3.0), Some(7.5)]),
            col(
                Some(1),
                "South",
                &[Some(4.0), Some(5.0), Some(6.0), Some(8.0)],
            ),
        ],
    );
    assert!(set(&mut f, id, t.clone()).errors.is_empty());
    let data = f.data();
    let mut bytes = std::io::Cursor::new(Vec::new());
    tally_io_ods::write_ods(&data, &mut bytes).unwrap();
    let back = tally_io_ods::read_ods(
        std::io::Cursor::new(bytes.into_inner()),
        &tally_io_ods::Limits::default(),
    )
    .unwrap();
    let wb = tally_engine::Workbook::from_data(&back);
    let sheet = wb.sheets[0].id;
    let chart = wb.sheets[0].meta.charts[0].clone();
    assert!(tally_engine::charts::to_api(&wb, sheet, &chart).own_data);
    let got = tally_engine::charts::data_table_of(&chart.data).unwrap();
    assert_eq!(got.categories, t.categories);
    assert_eq!(got.series[0].values, t.series[0].values);
    assert_eq!(got.series[1].name, "South");
    assert_eq!(chart.data.series[0].color, Some(0xff0000ff));
}

/// Review 5 (HIGH-1): a file can hold thousands of literal categories next
/// to hundreds of empty series. The list never carries a table, and the
/// padded table is refused above `MAX_DATA_TABLE_CELLS` instead of built.
#[test]
fn huge_padded_tables_are_refused_and_never_listed() {
    use tally_model::{ChartData, ChartSeriesData, ChartValuesData};
    let mut c = ChartData {
        literal_categories: Some(ChartValuesData::LiteralText(vec!["c".into(); 32_000])),
        series: (0..255)
            .map(|_| ChartSeriesData {
                literal_values: Some(ChartValuesData::Literal(Vec::new())),
                ..ChartSeriesData::default()
            })
            .collect(),
        ..ChartData::default()
    };
    assert!(c.has_own_data());
    let err = tally_engine::charts::data_table_of(&c).unwrap_err();
    assert_eq!(err.code, "invalid_argument");
    assert!(err.message.contains("too large"), "{}", err.message);
    // Under the cap it is handed out, padded; absent X / size columns are not.
    c.series.truncate(20);
    let t = tally_engine::charts::data_table_of(&c).unwrap();
    assert_eq!(t.series.len(), 20);
    assert!(t
        .series
        .iter()
        .all(|s| s.values.len() == 32_000 && s.x.is_none() && s.sizes.is_none()));
}

/// Review 5 (L1): an XY series without X values keeps none after an edit
/// (X stays the index, the trend line stays), unless X values are typed.
#[test]
fn absent_x_values_stay_absent_after_an_edit() {
    let mut f = Fixture::new();
    let t = table(&["", ""], vec![col(Some(0), "Y", &[Some(2.0), Some(4.0)])]);
    let s = f.sheet;
    let mut sc = series("", None);
    sc.trend = Some(tally_api::TrendLineDto {
        kind: tally_api::TrendKind::Linear,
        show_equation: false,
        show_r2: false,
        name: None,
    });
    let res = f.apply(vec![Op::AddChart {
        sheet: s,
        chart: chart(ChartKind::Scatter, vec![sc], Some(t)),
    }]);
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let id = res.created.unwrap()[0].id;
    assert_eq!(own_table(&f, id).series[0].x, None);
    // The dialog shows an empty X column and sends it back unchanged.
    let mut t = own_table(&f, id);
    t.series[0].values[1] = Some(5.0);
    t.series[0].x = Some(vec![None, None]);
    assert!(set(&mut f, id, t).errors.is_empty());
    assert_eq!(own_table(&f, id).series[0].x, None);
    let d = f.svc.chart_data(f.doc, f.sheet, id).unwrap();
    assert_eq!(d.series[0].x, None);
    assert!(d.series[0].trend.is_some(), "the trend line is kept");
    // Typed X values are kept.
    let mut t = own_table(&f, id);
    t.series[0].x = Some(vec![Some(1.0), None]);
    assert!(set(&mut f, id, t).errors.is_empty());
    assert_eq!(own_table(&f, id).series[0].x, Some(vec![Some(1.0), None]));
}
