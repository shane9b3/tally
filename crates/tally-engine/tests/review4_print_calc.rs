//! Review 4 (2026-10-21) fixes in printing, calculation and importers:
//! security review 4 H1–H5, M5, M6 and correctness review 4 H10–H13,
//! M22–M31, M36, M38.
#![allow(clippy::unwrap_used)]
#![cfg(feature = "io")]

mod common;

use std::time::{Duration, Instant};

use common::{range, Fixture};
use tally_api::{
    CalcSettingsDto, ChartAnchor, ChartAxisDto, ChartDto, ChartKind, ChartSeriesDto, ChartSvgDto,
    CriteriaModeDto, DataLabelsDto, DateEpochDto, DocumentService, IterationsDto, LegendPos, Op,
    PageSetupDto, PrintOptionsDto, PrintSheets, SeriesName, SheetCell, SheetRange, SolverGoal,
    SolverOp, SolverOptions, SolverRequest, SolverStatus, Stacking, StylePatch, TextToNumberDto,
};
use tally_types::{ErrorKind, Value};

fn opts() -> PrintOptionsDto {
    PrintOptionsDto {
        sheets: PrintSheets::Active,
        selection: None,
        suppress_empty_pages: true,
        page_from: None,
        page_to: None,
        active_sheet: None,
        page_ranges: None,
    }
}

fn setup(f: &mut Fixture, edit: impl FnOnce(&mut PageSetupDto)) {
    let s = f.sheet;
    let mut p = f.svc.page_setup(f.doc, s).unwrap();
    edit(&mut p);
    let r = f.apply(vec![Op::SetPageSetup { sheet: s, setup: p }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
}

fn fill(f: &mut Fixture, rows: u32, cols: u32) {
    let s = f.sheet;
    let mut ops = Vec::new();
    for r in 0..rows {
        for c in 0..cols {
            ops.push(Op::SetCellInput {
                sheet: s,
                r,
                c,
                input: format!("{}", r * 100 + c),
            });
        }
    }
    let res = f.apply(ops);
    assert!(res.errors.is_empty());
}

fn settings(iterations: Option<(u32, f64)>, precision: bool) -> CalcSettingsDto {
    CalcSettingsDto {
        iterations: iterations.map(|(steps, min_change)| IterationsDto { steps, min_change }),
        date_epoch: DateEpochDto::D1899,
        case_sensitive: false,
        precision_as_shown: precision,
        whole_cell_match: true,
        criteria_mode: CriteriaModeDto::Wildcards,
        text_to_number: TextToNumberDto::ConvertUnambiguous,
        two_digit_year: None,
        standard_decimals: None,
    }
}

fn fmt(f: &mut Fixture, a1: &str, code: &str) {
    let s = f.sheet;
    let r = f.apply(vec![Op::ApplyStyle {
        sheet: s,
        ranges: vec![range(a1)],
        patch: StylePatch {
            number_format: Some(code.into()),
            ..StylePatch::default()
        },
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
}

/// A fixture whose service can save and open files.
fn io_fixture() -> Fixture {
    let mut svc = tally_engine::EngineService::with_default_codec();
    let info = svc.new_workbook(None).unwrap();
    Fixture {
        svc,
        doc: info.doc,
        sheet: info.sheets[0].id,
    }
}

fn num(v: Value) -> f64 {
    match v {
        Value::Number(n) => n,
        other => panic!("{other:?}"),
    }
}

// ------------------------------------------------------------------ print

#[test]
fn default_rows_and_columns_paginate_like_aoo() {
    // Correctness review 4, M22: 53 rows × 7 columns per A4 page with the
    // default page setup (AOO); the 24 px screen default gave 38 rows.
    let mut f = Fixture::new();
    fill(&mut f, 120, 10);
    let l = f.svc.print_layout(f.doc, &opts()).unwrap();
    assert_eq!(l.pages[0].rows, [0, 52], "{:?}", l.pages[0]);
    assert_eq!(l.pages[0].cols, [0, 6], "{:?}", l.pages[0]);
}

#[test]
fn implicit_area_starts_at_a1_and_takes_overflowing_text() {
    // M23: data only at C10 prints from A1.
    let mut f = Fixture::new();
    f.set("C10", "x");
    let b = f.svc.page_breaks(f.doc, f.sheet).unwrap();
    let a = b.area.unwrap();
    assert_eq!((a.r0, a.c0, a.r1, a.c1), (0, 0, 9, 2));
    // M24: a long text in A1 extends the area to the columns it covers.
    let mut f = Fixture::new();
    f.set("A1", &"long text ".repeat(40));
    let a = f.svc.page_breaks(f.doc, f.sheet).unwrap().area.unwrap();
    assert!(a.c1 >= 8, "{a:?}");
    // A text that is blocked by its neighbour does not.
    let mut f = Fixture::new();
    f.set("A1", &"long text ".repeat(40));
    f.set("B1", "x");
    let a = f.svc.page_breaks(f.doc, f.sheet).unwrap().area.unwrap();
    assert_eq!(a.c1, 1, "{a:?}");
}

#[test]
fn manual_break_on_a_hidden_row_is_kept_and_headers_shrink_the_body() {
    // M25.
    let mut f = Fixture::new();
    fill(&mut f, 20, 1);
    setup(&mut f, |p| p.row_breaks = vec![10]);
    let s = f.sheet;
    f.apply(vec![Op::SetRowsHidden {
        sheet: s,
        start: 10,
        end: 11,
        hidden: true,
    }]);
    let l = f.svc.print_layout(f.doc, &opts()).unwrap();
    assert_eq!(l.pages.len(), 2, "{:?}", l.pages);
    assert_eq!(l.pages[1].rows[0], 12);
    let b = f.svc.page_breaks(f.doc, s).unwrap();
    assert!(b.row_breaks.iter().any(|x| x.at == 12 && x.manual), "{b:?}");
    // M26: printed headers take their space from the body.
    let mut f = Fixture::new();
    fill(&mut f, 200, 1);
    assert_eq!(
        f.svc.print_layout(f.doc, &opts()).unwrap().pages[0].rows,
        [0, 52]
    );
    setup(&mut f, |p| p.print_headers = true);
    assert_eq!(
        f.svc.print_layout(f.doc, &opts()).unwrap().pages[0].rows,
        [0, 51]
    );
}

#[test]
fn print_titles_taller_than_the_page_are_left_off() {
    // Security review 4, H1: repeat rows 0–19,999 over 40,000 rows made
    // every page hold one row (90k: > 60 s, > 2.2 GB).
    let mut f = Fixture::new();
    let s = f.sheet;
    f.apply(vec![
        Op::SetCellInput {
            sheet: s,
            r: 0,
            c: 0,
            input: "top".into(),
        },
        Op::SetCellInput {
            sheet: s,
            r: 39_999,
            c: 0,
            input: "bottom".into(),
        },
    ]);
    setup(&mut f, |p| p.repeat_rows = Some([0, 19_999]));
    let mut o = opts();
    o.suppress_empty_pages = false;
    let t = Instant::now();
    let l = f.svc.print_layout(f.doc, &o).unwrap();
    assert!(t.elapsed() < Duration::from_secs(5), "{:?}", t.elapsed());
    assert!(l.pages.len() < 1000, "{} pages", l.pages.len());
    assert!(l.pages.iter().all(|p| p.repeat_rows.is_none()));
    // Titles that fit are still printed on the following pages.
    setup(&mut f, |p| p.repeat_rows = Some([0, 1]));
    let l = f.svc.print_layout(f.doc, &o).unwrap();
    assert_eq!(l.pages[1].repeat_rows, Some([0, 1]));
    assert_eq!(l.pages[1].rows[1] - l.pages[1].rows[0], 50);
}

#[test]
fn page_caps_agree_with_the_renderer() {
    // M5: more than 10,000 pages is refused at layout, not after caching.
    let mut f = Fixture::new();
    f.set("A600000", "x");
    let mut o = opts();
    o.suppress_empty_pages = false;
    let e = f.svc.print_layout(f.doc, &o).unwrap_err();
    assert!(e.message.contains("10000"), "{e:?}");
    assert_eq!(tally_engine::print::MAX_PAGES, tally_io_pdf::MAX_PAGES);
}

#[test]
fn print_job_cost_is_bounded_by_the_cells_it_visits() {
    // H5: 1 hmm rows and columns put 24,000 × 650 cells on one page;
    // 100 pages took 48 s, one API page 12.6 s.
    let mut f = Fixture::new();
    let s = f.sheet;
    f.apply(vec![
        Op::SetRowHeight {
            sheet: s,
            start: 0,
            end: 1_048_575,
            px: 0.04,
        },
        Op::SetColWidth {
            sheet: s,
            start: 0,
            end: 16_383,
            px: 0.04,
        },
        Op::SetCellInput {
            sheet: s,
            r: 0,
            c: 0,
            input: "a".into(),
        },
        Op::SetCellInput {
            sheet: s,
            r: 1_048_575,
            c: 16_383,
            input: "z".into(),
        },
    ]);
    let t = Instant::now();
    let r = f.svc.prepare_print_job(f.doc, &opts(), &[]);
    assert!(t.elapsed() < Duration::from_secs(10), "{:?}", t.elapsed());
    let Err(e) = r else {
        panic!("the job was not refused");
    };
    assert!(e.message.contains("too large"), "{e:?}");
}

fn chart(r: u32) -> ChartDto {
    let axis = ChartAxisDto {
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
    };
    ChartDto {
        id: 0,
        name: String::new(),
        anchor: ChartAnchor {
            r,
            c: 1,
            dx_mm: 0.0,
            dy_mm: 0.0,
            width_mm: 20.0,
            height_mm: 2.0,
        },
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
        categories: None,
        series: vec![ChartSeriesDto {
            name: Some(SeriesName::Ref {
                r#ref: "$Sheet1.$A$1".into(),
            }),
            values: "$Sheet1.$A$2:$A$6".into(),
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
        }],
        title: None,
        subtitle: None,
        legend: LegendPos::Right,
        x_axis: axis.clone(),
        y_axis: axis,
        y2_axis: None,
        background: None,
        plot_background: None,
        include_hidden: false,
        data_table: None,
    }
}

#[test]
fn a_chart_in_a_repeated_row_prints_once() {
    // M6: the chart (and its SVG) was copied onto every page.
    let mut f = Fixture::new();
    fill(&mut f, 600, 1);
    let s = f.sheet;
    let r = f.apply(vec![Op::AddChart {
        sheet: s,
        chart: chart(1),
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    let id = r.created.unwrap()[0].id;
    // A large SVG (many paths) makes a copy per page visible in the PDF.
    let mut svg = String::from("<svg xmlns='http://www.w3.org/2000/svg' width='200' height='20'>");
    for i in 0..3000 {
        svg.push_str(&format!(
            "<rect x='{}' y='{}' width='1' height='1' fill='#123456'/>",
            i % 200,
            i / 200
        ));
    }
    svg.push_str("</svg>");
    let svgs = [ChartSvgDto { sheet: s, id, svg }];
    let size = |f: &mut Fixture| {
        let j = f.svc.prepare_print_job(f.doc, &opts(), &svgs).unwrap();
        (
            j.layout.pages.len(),
            f.svc.print_job_pdf(j.job).unwrap().len(),
        )
    };
    let (pages_plain, plain) = size(&mut f);
    setup(&mut f, |p| p.repeat_rows = Some([0, 2]));
    let (pages, repeated) = size(&mut f);
    assert!(pages >= 10 && pages_plain >= 10);
    assert!(
        repeated < plain + plain / 2,
        "{repeated} bytes with the chart row repeated vs {plain}"
    );
}

// ------------------------------------------------------------------ calc

#[test]
fn iteration_work_is_bounded_per_recalc() {
    // Security review 4, H2: 32,767 steps over a cycle that never
    // converges (min change 0) ran for minutes.
    let mut f = Fixture::new();
    let s = f.sheet;
    let mut ops = vec![Op::SetCalcSettings {
        settings: settings(Some((32_767, 0.0)), false),
    }];
    // A 2,000-cell cycle: A1 = A2000 + 1, Ai = A(i-1) + 1.
    ops.push(Op::SetCellInput {
        sheet: s,
        r: 0,
        c: 0,
        input: "=A2000+1".into(),
    });
    for r in 1..2000 {
        ops.push(Op::SetCellInput {
            sheet: s,
            r,
            c: 0,
            input: format!("=A{r}+1"),
        });
    }
    let t = Instant::now();
    let res = f.apply(ops);
    assert!(res.errors.is_empty());
    // 2,000 cells × 32,767 steps = 65M evaluations > the 20M budget.
    assert!(t.elapsed() < Duration::from_secs(60), "{:?}", t.elapsed());
    assert_eq!(f.value("A1"), Value::Error(ErrorKind::NoConvergence));
}

#[test]
fn table_ops_have_a_recalc_budget_and_nest() {
    let mut f = Fixture::new();
    // AOO's help example.
    f.set("B1", "10");
    f.set("B2", "2");
    f.set("B3", "10000");
    f.set("B4", "2000");
    f.set("B5", "=B1*B4-(B3+B2*B4)");
    f.set("E2", "=MULTIPLE.OPERATIONS($B$5;$B$4;1000)");
    assert_eq!(num(f.value("E2")), -2000.0);
    // Correctness review 4, M31: nested MULTIPLE.OPERATIONS.
    f.set("N5", "=MULTIPLE.OPERATIONS(B5;B1;20)");
    f.set("N6", "=MULTIPLE.OPERATIONS(N5;B4;1000)");
    assert_eq!(num(f.value("N6")), 8000.0);
    // Security review 4, H3 / LOW-P10: a call whose dependency cone is
    // larger than the sandbox gives Err:512, not a partial result.
    let mut f = Fixture::new();
    let s = f.sheet;
    let mut ops = vec![Op::SetCellInput {
        sheet: s,
        r: 0,
        c: 0,
        input: "1".into(),
    }];
    for r in 1..=200_001u32 {
        ops.push(Op::SetCellInput {
            sheet: s,
            r,
            c: 1,
            input: "=$A$1*2".into(),
        });
    }
    ops.push(Op::SetCellInput {
        sheet: s,
        r: 0,
        c: 2,
        input: "=SUM(B2:B200002)".into(),
    });
    f.apply(ops);
    let t = Instant::now();
    f.set("D1", "=MULTIPLE.OPERATIONS(C1;A1;3)");
    assert!(t.elapsed() < Duration::from_secs(60), "{:?}", t.elapsed());
    assert_eq!(f.value("D1"), Value::Error(ErrorKind::FormulaOverflow));
}

#[test]
fn table_op_work_budget_stops_many_calls() {
    // H3: many calls over a large cone shared one budget of 20M units.
    let mut f = Fixture::new();
    let s = f.sheet;
    let mut ops = vec![Op::SetCellInput {
        sheet: s,
        r: 0,
        c: 0,
        input: "1".into(),
    }];
    for r in 1..=50_000u32 {
        ops.push(Op::SetCellInput {
            sheet: s,
            r,
            c: 1,
            input: "=$A$1*2".into(),
        });
    }
    ops.push(Op::SetCellInput {
        sheet: s,
        r: 0,
        c: 2,
        input: "=SUM(B2:B50001)".into(),
    });
    for r in 0..600u32 {
        ops.push(Op::SetCellInput {
            sheet: s,
            r,
            c: 4,
            input: format!("=MULTIPLE.OPERATIONS($C$1;$A$1;{r})"),
        });
    }
    let t = Instant::now();
    f.apply(ops);
    assert!(t.elapsed() < Duration::from_secs(120), "{:?}", t.elapsed());
    // The first calls are exact, the last ones over budget.
    assert_eq!(num(f.value("E2")), 100_000.0);
    assert_eq!(f.value("E600"), Value::Error(ErrorKind::FormulaOverflow));
}

#[test]
fn precision_as_shown_follows_format_changes() {
    // Correctness review 4, H11.
    let mut f = Fixture::new();
    f.apply(vec![Op::SetCalcSettings {
        settings: settings(None, true),
    }]);
    f.set("A1", "0.333");
    f.set("B1", "=A1*3");
    f.set("C1", "=1/3");
    fmt(&mut f, "A1", "0.0");
    fmt(&mut f, "C1", "0.00");
    assert!(
        (num(f.value("B1")) - 0.9).abs() < 1e-12,
        "{:?}",
        f.value("B1")
    );
    assert_eq!(num(f.value("C1")), 0.33);
    // M29: scientific formats round to the mantissa's decimals.
    f.set("D1", "0.000123456");
    fmt(&mut f, "D1", "0.00E+00");
    f.set("E1", "=D1*1");
    assert!(
        (num(f.value("E1")) - 0.000123).abs() < 1e-15,
        "{:?}",
        f.value("E1")
    );
    // M30: `#,##0,` shows thousands but keeps the value's precision.
    f.set("F1", "1234");
    fmt(&mut f, "F1", "#,##0,");
    f.set("G1", "=F1+0");
    assert_eq!(num(f.value("G1")), 1234.0);
}

#[test]
fn trace_error_follows_every_error_precedent() {
    // M38 (engine function; no service API yet).
    let mut f = Fixture::new();
    f.set("A1", "=1/0");
    f.set("A2", "=NA()");
    f.set("A3", "=A2");
    f.set("B1", "=A1+A3");
    let wb = &f.svc.document(f.doc).unwrap().wb;
    let e = tally_engine::detective::error_source(wb, f.sheet, 0, 1).unwrap();
    let cells: Vec<(u32, u32)> = e.iter().map(|t| (t.range.r0, t.range.c0)).collect();
    assert_eq!(cells, vec![(0, 0), (2, 0), (1, 0)]);
}

#[test]
fn solver_timeout_covers_sampling_and_options_are_capped() {
    // H4: sampling ran outside the timeout (31 s against 10 s).
    let mut f = Fixture::new();
    let s = f.sheet;
    let mut ops = Vec::new();
    for r in 0..200u32 {
        ops.push(Op::SetCellInput {
            sheet: s,
            r,
            c: 0,
            input: "1".into(),
        });
    }
    ops.push(Op::SetCellInput {
        sheet: s,
        r: 0,
        c: 1,
        input: "=SUM(A1:A200)".into(),
    });
    // Formulas reading the variables make every sample slower.
    for r in 0..20_000u32 {
        ops.push(Op::SetCellInput {
            sheet: s,
            r,
            c: 3,
            input: format!("=SUM($A$1:$A$200)*{r}"),
        });
    }
    f.apply(ops);
    let req = SolverRequest {
        objective: SheetCell {
            sheet: s,
            r: 0,
            c: 1,
        },
        goal: SolverGoal::Maximize,
        variables: vec![SheetRange {
            sheet: s,
            range: range("A1:A200"),
        }],
        constraints: vec![tally_api::SolverConstraint {
            lhs: SheetRange {
                sheet: s,
                range: range("A1:A200"),
            },
            op: SolverOp::Le,
            rhs: "10".into(),
        }],
        options: SolverOptions {
            timeout_ms: 300,
            ..SolverOptions::default()
        },
    };
    let t = Instant::now();
    let r = f.svc.solver(f.doc, &req).unwrap();
    let took = t.elapsed();
    assert!(took < Duration::from_secs(8), "{took:?}");
    assert!(
        matches!(r.status, SolverStatus::Limit | SolverStatus::Optimal),
        "{:?}",
        r.status
    );
    assert_eq!(tally_engine::solver::MAX_TIMEOUT_MS, 60_000);
    assert_eq!(tally_engine::solver::MAX_ITERATIONS, 100_000);
}

#[test]
fn deep_branch_and_bound_is_iterative_and_capped() {
    // H4: `2x - 2y = 1` in integers has no solution; the recursion held an
    // LP clone per level (945 MB) with uncapped options.
    let mut f = Fixture::new();
    f.set("A1", "0");
    f.set("A2", "0");
    f.set("B1", "=2*A1-2*A2");
    f.set("C1", "=A1+A2");
    let s = f.sheet;
    let req = SolverRequest {
        objective: SheetCell {
            sheet: s,
            r: 0,
            c: 2,
        },
        goal: SolverGoal::Minimize,
        variables: vec![SheetRange {
            sheet: s,
            range: range("A1:A2"),
        }],
        constraints: vec![
            tally_api::SolverConstraint {
                lhs: SheetRange {
                    sheet: s,
                    range: range("B1"),
                },
                op: SolverOp::Eq,
                rhs: "1".into(),
            },
            tally_api::SolverConstraint {
                lhs: SheetRange {
                    sheet: s,
                    range: range("A1:A2"),
                },
                op: SolverOp::Integer,
                rhs: String::new(),
            },
        ],
        options: SolverOptions {
            // u32::MAX is clamped to 60 s; 3 s keeps the test short.
            timeout_ms: 3_000,
            max_iterations: u32::MAX,
            assume_non_negative: true,
            ..SolverOptions::default()
        },
    };
    let t = Instant::now();
    let r = f.svc.solver(f.doc, &req).unwrap();
    assert!(t.elapsed() < Duration::from_secs(10), "{:?}", t.elapsed());
    assert!(
        matches!(r.status, SolverStatus::Limit | SolverStatus::Infeasible),
        "{:?}",
        r.status
    );
}

// ------------------------------------------------------------- importers

fn repo(p: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(p)
}

#[test]
fn sxc_names_quotes_and_typed_values() {
    // Correctness review 4, H13 (TAX1 is a name in OOo 1.x), M27 (`''` in
    // quoted sheet names) and M28 (typed values get a standard format).
    let mut svc = tally_engine::EngineService::with_default_codec();
    let info = svc.open(&repo("tests/fixtures/sxc/review4.sxc")).unwrap();
    let s = info.sheets[0].id;
    let wb = &svc.document(info.doc).unwrap().wb;
    let v = |r: u32, c: u32| wb.value(tally_types::CellAddr::new(s, r, c));
    assert_eq!(v(1, 1), Value::Number(10.0), "=TAX1*2");
    assert_eq!(v(0, 2), Value::Number(101.0), "='Bob''s'.A1+1");
    assert_eq!(v(1, 3), Value::Number(100.0), "=$'Bob''s'.$A$1");
    assert_eq!(v(0, 1), Value::Number(20.0), "='My Sheet'.A1*2");
    let cells = svc
        .cells(
            info.doc,
            s,
            &[tally_api::RangeDto {
                r0: 2,
                c0: 0,
                r1: 2,
                c1: 2,
            }],
        )
        .unwrap();
    let text = |c: u32| {
        cells[0]
            .cells
            .iter()
            .find(|x| x.c == c)
            .map(|x| x.text.clone())
            .unwrap_or_default()
    };
    assert_eq!(text(0), "12:30:00");
    assert_eq!(text(1), "25%");
    // The file's locale is de-DE.
    assert!(text(2).contains("9,50"), "{}", text(2));
}

#[test]
fn string_literals_are_not_external_links() {
    // Correctness review 4, H12: the literal survives a save and reload.
    let mut f = io_fixture();
    f.set("A1", "=\"Call DDE(x) later\"");
    f.set("A2", "=\"see 'file:///etc/a.ods'#$S.A1\"");
    for ext in ["ods", "xlsx"] {
        let p = std::env::temp_dir().join(format!("tally-r4-ext-{}.{ext}", std::process::id()));
        let fmt = if ext == "ods" {
            tally_api::FileFormat::Ods
        } else {
            tally_api::FileFormat::Xlsx
        };
        f.svc.save(f.doc, Some(&p), fmt).unwrap();
        let back = f.svc.open(&p).unwrap();
        assert!(
            back.external_links.as_ref().is_none_or(|v| v.is_empty()),
            "{ext}: {:?}",
            back.external_links
        );
        let wb = &f.svc.document(back.doc).unwrap().wb;
        let s = back.sheets[0].id;
        assert_eq!(
            wb.value(tally_types::CellAddr::new(s, 0, 0)),
            Value::text("Call DDE(x) later"),
            "{ext}"
        );
        let _ = std::fs::remove_file(&p);
    }
}

#[test]
fn html_lang_drives_number_recognition() {
    // Correctness review 4, M36: a de-DE export reimports as numbers.
    let mut f = io_fixture();
    f.set("A1", "1234.5");
    f.svc.set_locale(f.doc, "de-DE").unwrap();
    let p = std::env::temp_dir().join(format!("tally-r4-de-{}.html", std::process::id()));
    f.svc
        .save(f.doc, Some(&p), tally_api::FileFormat::Html)
        .unwrap();
    let back = f.svc.open(&p).unwrap();
    let wb = &f.svc.document(back.doc).unwrap().wb;
    let s = back.sheets[0].id;
    assert_eq!(
        wb.value(tally_types::CellAddr::new(s, 0, 0)),
        Value::Number(1234.5)
    );
    let _ = std::fs::remove_file(&p);
}

#[test]
fn imported_iteration_steps_are_capped() {
    // H2: a file's 32,767 steps become 1,000 with a warning.
    let mut data = tally_model::WorkbookData::default();
    data.sheets.push(tally_model::SheetData {
        name: "S".into(),
        ..tally_model::SheetData::default()
    });
    data.settings.calc_settings.iterations = Some(tally_types::IterSettings {
        steps: 32_767,
        min_change: 0.0,
    });
    let wb = tally_engine::Workbook::from_data(&data);
    assert_eq!(wb.settings.iterations.unwrap().steps, 1_000);
    assert!(wb.import_warnings.iter().any(|w| w.contains("iteration")));
}
