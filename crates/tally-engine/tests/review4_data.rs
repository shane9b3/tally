//! Review round 4 regressions: data tools, outline, structure, names and
//! scenarios (correctness review 4 H1/H2/H8 and Mediums; security review
//! 4 CRITICAL-T1, HIGH-T2..T5, MEDIUM-T6..T8).
#![allow(clippy::unwrap_used)]
#![cfg(feature = "io")]

mod common;

use std::time::{Duration, Instant};

use common::{range, Fixture};
use tally_api::{
    AggFn, DocumentService, FileFormat, NoteDto, Op, OutlineAxis, ShiftDir, SubtotalGroup,
    SubtotalOptions,
};
use tally_engine::EngineService;
use tally_types::{CellAddr, Value};

fn io_fixture() -> Fixture {
    let mut svc = EngineService::with_default_codec();
    let info = svc.new_workbook(None).unwrap();
    let sheet = info.sheets[0].id;
    Fixture {
        svc,
        doc: info.doc,
        sheet,
    }
}

fn rows_in(f: &mut Fixture, rows: &[&[&str]]) {
    let rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| r.iter().map(|s| s.to_string()).collect())
        .collect();
    let sheet = f.sheet;
    f.apply(vec![Op::SetRangeInputs {
        sheet,
        r: 0,
        c: 0,
        rows,
    }]);
}

fn edit(f: &Fixture, a1: &str) -> String {
    let a = CellAddr::from_a1(f.sheet, a1).unwrap();
    f.svc
        .cell_edit_info(f.doc, f.sheet, a.row, a.col)
        .unwrap()
        .input
}

fn col(f: &Fixture, c: &str, n: u32) -> Vec<String> {
    (1..=n).map(|r| f.text(&format!("{c}{r}"))).collect()
}

/// `(start, end, level, collapsed)` of the row and column groups.
type Groups = (Vec<(u32, u32, u8, bool)>, Vec<(u32, u32, u8, bool)>);

fn groups(f: &Fixture) -> Groups {
    match f.svc.sheet_layout(f.doc, f.sheet).unwrap().outline {
        None => (Vec::new(), Vec::new()),
        Some(o) => (
            o.rows
                .iter()
                .map(|g| (g.start, g.end, g.level, g.collapsed))
                .collect(),
            o.cols
                .iter()
                .map(|g| (g.start, g.end, g.level, g.collapsed))
                .collect(),
        ),
    }
}

fn group(f: &mut Fixture, axis: OutlineAxis, start: u32, end: u32) -> usize {
    let sheet = f.sheet;
    f.apply(vec![Op::Group {
        sheet,
        axis,
        start,
        end,
    }])
    .errors
    .len()
}

fn reload(f: &mut Fixture, fmt: FileFormat, tag: &str) -> Fixture {
    let ext = match fmt {
        FileFormat::Ods => "ods",
        _ => "xlsx",
    };
    let dir = std::env::temp_dir().join(format!("tally-r4-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join(format!("t.{ext}"));
    f.svc.save(f.doc, Some(&p), fmt).unwrap();
    let mut svc = EngineService::with_default_codec();
    let info = svc.open(&p).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    Fixture {
        svc,
        doc: info.doc,
        sheet: info.sheets[0].id,
    }
}

// ---------------------------------------------------------------- H1

#[test]
fn h1_ods_groups_past_the_data_round_trip() {
    // Rows 1..5 grouped, data only in row 3 (was: unclosed group, file
    // could not be reopened).
    let mut f = io_fixture();
    f.set("A3", "x");
    assert_eq!(group(&mut f, OutlineAxis::Rows, 1, 5), 0);
    // A collapsed group entirely after the data.
    assert_eq!(group(&mut f, OutlineAxis::Rows, 10, 12), 0);
    let sheet = f.sheet;
    f.apply(vec![Op::SetOutlineCollapsed {
        sheet,
        axis: OutlineAxis::Rows,
        start: 10,
        end: 12,
        collapsed: true,
    }]);
    // Column groups past the data.
    assert_eq!(group(&mut f, OutlineAxis::Cols, 4, 7), 0);
    assert_eq!(group(&mut f, OutlineAxis::Cols, 5, 6), 0);
    let before = groups(&f);
    for fmt in [FileFormat::Ods, FileFormat::Xlsx] {
        let g = reload(&mut f, fmt, &format!("h1{fmt:?}"));
        assert_eq!(groups(&g), before, "{fmt:?}");
        assert_eq!(g.value("A3"), Value::text("x"));
    }
}

#[test]
fn h1_ods_seven_levels_and_groups_starting_after_data() {
    let mut f = io_fixture();
    f.set("A1", "x");
    for i in 0..7u32 {
        assert_eq!(group(&mut f, OutlineAxis::Rows, 1 + i, 31 - i), 0);
    }
    let before = groups(&f);
    let g = reload(&mut f, FileFormat::Ods, "h1l7");
    assert_eq!(groups(&g), before);
}

// ---------------------------------------------------------------- M5

#[test]
fn m5_outline_depth_is_seven() {
    let mut f = Fixture::new();
    for i in 0..7u32 {
        assert_eq!(group(&mut f, OutlineAxis::Rows, i, 30 - i), 0);
    }
    // An eighth level (inside or around) is refused (SC_OL_MAXDEPTH 7).
    assert_eq!(group(&mut f, OutlineAxis::Rows, 10, 12), 1);
    assert_eq!(group(&mut f, OutlineAxis::Rows, 0, 40), 1);
}

// ---------------------------------------------------------------- M4

#[test]
fn m4_auto_outline_follows_aoo() {
    let mut f = Fixture::new();
    for r in 1..=3 {
        f.set(&format!("A{r}"), &r.to_string());
    }
    // Rows are scanned top to bottom: B1 (row 1) refers below → rows 2..4;
    // A4 refers above (rows 1..3) and partially overlaps it: skipped.
    f.set("A4", "=SUM(A1:A3)");
    f.set("B1", "=SUM(B2:B4)");
    f.set("C6", "=C8"); // single reference below → group row 8
    f.set("A10", "=SUM(A1:A3)+B1"); // two references, not one function → nothing
    f.set("D12", "=SUM(D10;D11)"); // two refs as sole args forming D10:D11
                                   // Columns: E20 = SUM(B20:D20) → columns B..D.
    f.set("E20", "=SUM(B20:D20)");
    // An existing group is cleared first.
    group(&mut f, OutlineAxis::Rows, 30, 31);
    let sheet = f.sheet;
    let r = f.apply(vec![Op::AutoOutline { sheet, range: None }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    let (rows, cols) = groups(&f);
    let spans: Vec<(u32, u32)> = rows.iter().map(|g| (g.0, g.1)).collect();
    assert!(spans.contains(&(1, 3)), "{spans:?}");
    assert!(!spans.contains(&(0, 2)), "{spans:?}");
    assert!(spans.contains(&(7, 7)), "{spans:?}");
    assert!(spans.contains(&(9, 10)), "{spans:?}");
    assert!(!spans.contains(&(30, 31)), "old outline kept: {spans:?}");
    assert_eq!(
        cols.iter().map(|g| (g.0, g.1)).collect::<Vec<_>>(),
        vec![(1, 3)]
    );
}

// ---------------------------------------------------------------- H2 / C5

#[test]
fn h2_remove_duplicates_keeps_cells_below_the_range() {
    let mut f = Fixture::new();
    rows_in(&mut f, &[&["K"], &["a"], &["a"], &["b"]]);
    f.set("A6", "below-range");
    f.set("A7", "=A6");
    let sheet = f.sheet;
    let r = f.apply(vec![Op::RemoveDuplicates {
        sheet,
        range: range("A1:A4"),
        cols: vec![],
        has_header: true,
    }]);
    assert!(r.errors.is_empty());
    assert_eq!(
        col(&f, "A", 7),
        ["K", "a", "b", "", "", "below-range", "below-range"]
    );
    assert_eq!(edit(&f, "A7"), "=A6");
    f.svc.undo(f.doc).unwrap();
    assert_eq!(col(&f, "A", 4), ["K", "a", "a", "b"]);
}

#[test]
fn c5_remove_duplicates_is_one_pass() {
    // 20,000 rows, every other one a duplicate (was 3.2 s / 1.67 GB at
    // 10,000 rows in release; quadratic).
    let n = 20_000u32;
    let mut f = Fixture::new();
    let rows: Vec<Vec<String>> = (0..n)
        .map(|i| {
            vec![if i % 2 == 0 {
                "dup".to_string()
            } else {
                format!("u{i}")
            }]
        })
        .collect();
    let sheet = f.sheet;
    f.apply(vec![Op::SetRangeInputs {
        sheet,
        r: 0,
        c: 0,
        rows,
    }]);
    let t = Instant::now();
    let r = f.apply(vec![Op::RemoveDuplicates {
        sheet,
        range: range(&format!("A1:A{n}")),
        cols: vec![0],
        has_header: false,
    }]);
    let took = t.elapsed();
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert!(took < Duration::from_secs(10), "{took:?}");
    assert_eq!(f.text("A1"), "dup");
    assert_eq!(f.text("A2"), "u1");
    assert_eq!(f.text(&format!("A{}", n / 2 + 1)), format!("u{}", n - 1));
    assert_eq!(f.text(&format!("A{}", n / 2 + 2)), "");
    eprintln!("c5 dedup {n} rows: {took:?}");
}

// ---------------------------------------------------------------- H8 / M15 / M17

fn note(sheet: u32, r: u32, c: u32) -> Op {
    Op::SetNote {
        sheet,
        r,
        c,
        note: NoteDto {
            r,
            c,
            text: format!("n{r}{c}"),
            author: None,
            created: None,
            shown: false,
        },
    }
}

fn shift(f: &mut Fixture, a1: &str, shift: ShiftDir, insert: bool) -> usize {
    let sheet = f.sheet;
    let op = if insert {
        Op::InsertCells {
            sheet,
            range: range(a1),
            shift,
        }
    } else {
        Op::DeleteCells {
            sheet,
            range: range(a1),
            shift,
        }
    };
    f.apply(vec![op]).errors.len()
}

#[test]
fn h8_notes_and_merges_move_in_an_empty_band() {
    let mut f = Fixture::new();
    let s = f.sheet;
    f.apply(vec![note(s, 4, 1)]); // B5 on an empty sheet
    f.apply(vec![Op::MergeCells {
        sheet: s,
        range: range("B7:C7"),
    }]);
    assert_eq!(shift(&mut f, "A2:C2", ShiftDir::Down, true), 0);
    let notes: Vec<(u32, u32)> = f
        .svc
        .notes(f.doc, s)
        .unwrap()
        .iter()
        .map(|n| (n.r, n.c))
        .collect();
    assert_eq!(notes, vec![(5, 1)]);
    let merges = f.svc.sheet_layout(f.doc, s).unwrap().merges;
    assert_eq!((merges[0].r0, merges[0].r1), (7, 7));
    // Delete cells up moves them back; undo restores.
    assert_eq!(shift(&mut f, "A2:C2", ShiftDir::Up, false), 0);
    let notes: Vec<(u32, u32)> = f
        .svc
        .notes(f.doc, s)
        .unwrap()
        .iter()
        .map(|n| (n.r, n.c))
        .collect();
    assert_eq!(notes, vec![(4, 1)]);
    f.svc.undo(f.doc).unwrap();
    f.svc.undo(f.doc).unwrap();
    let notes: Vec<(u32, u32)> = f
        .svc
        .notes(f.doc, s)
        .unwrap()
        .iter()
        .map(|n| (n.r, n.c))
        .collect();
    assert_eq!(notes, vec![(4, 1)]);
}

#[test]
fn m15_insert_cells_moves_ranges_inside_the_band() {
    let mut f = Fixture::new();
    let s = f.sheet;
    for r in 1..=5 {
        f.set(&format!("A{r}"), &r.to_string());
    }
    f.apply(vec![Op::DefineDatabaseRange {
        name: "db".into(),
        sheet: s,
        range: range("A3:A5"),
        has_header: false,
    }]);
    f.apply(vec![Op::SetAutoFilter {
        sheet: s,
        range: Some(range("A3:A5")),
    }]);
    f.apply(vec![Op::AddValidation {
        sheet: s,
        validation: tally_api::ValidationDto {
            id: 0,
            ranges: vec![range("A3:A5")],
            kind: tally_api::ValidationKind::Whole {
                op: tally_api::CmpOp::Between,
                formulas: vec!["0".into(), "100".into()],
            },
            allow_blank: true,
            show_dropdown: false,
            sort_ascending: false,
            input_help: None,
            error: None,
        },
    }]);
    assert_eq!(shift(&mut f, "A1:A2", ShiftDir::Down, true), 0);
    let db = f.svc.database_ranges(f.doc).unwrap();
    assert_eq!((db[0].range.r0, db[0].range.r1), (4, 6));
    let v = f.svc.validations(f.doc, s).unwrap();
    assert_eq!((v[0].ranges[0].r0, v[0].ranges[0].r1), (4, 6));
    // A range wider than the band stays.
    assert_eq!(shift(&mut f, "B1:B1", ShiftDir::Down, true), 0);
    let v = f.svc.validations(f.doc, s).unwrap();
    assert_eq!((v[0].ranges[0].r0, v[0].ranges[0].r1), (4, 6));
}

#[test]
fn m17_full_width_insert_cells_is_insert_rows() {
    let mut f = Fixture::new();
    let s = f.sheet;
    for r in 1..=4 {
        f.set(&format!("A{r}"), &r.to_string());
    }
    f.set("C1", "=SUM(2:3)");
    f.apply(vec![Op::SetRowHeight {
        sheet: s,
        start: 2,
        end: 2,
        px: 50.0,
    }]);
    assert_eq!(shift(&mut f, "A2:XFD2", ShiftDir::Down, true), 0);
    assert_eq!(edit(&f, "C1"), "=SUM(3:4)");
    let runs = f.svc.sheet_layout(f.doc, s).unwrap().row_runs;
    assert!(
        runs.iter()
            .any(|r| r.start == 3 && r.end == 3 && r.size > 40.0),
        "{runs:?}"
    );
}

// ---------------------------------------------------------------- H9 / M6

fn sub(f: &mut Fixture, rng: &str, groups: Vec<SubtotalGroup>) -> Vec<String> {
    let sheet = f.sheet;
    f.apply(vec![Op::Subtotals {
        sheet,
        range: range(rng),
        groups,
        options: SubtotalOptions {
            presort: false,
            ..SubtotalOptions::default()
        },
    }])
    .errors
    .iter()
    .map(|e| e.message.clone())
    .collect()
}

#[test]
fn h9_m6_subtotals_layout_captions_and_undo() {
    let mut f = Fixture::new();
    rows_in(
        &mut f,
        &[
            &["K", "V", "W"],
            &["East", "1", "10"],
            &["East", "2", "20"],
            &["", "4", "40"],
            &["West", "3", "30"],
        ],
    );
    f.set("E1", "=B3"); // follows its row like InsertRows
    f.set("E2", "=SUM(B2:B5)");
    let errs = sub(
        &mut f,
        "A1:C5",
        vec![SubtotalGroup {
            group_by: 0,
            columns: vec![1],
            function: AggFn::Count,
        }],
    );
    assert!(errs.is_empty(), "{errs:?}");
    // Unsorted: the empty cell continues the East group (AOO).
    assert_eq!(
        col(&f, "A", 8),
        [
            "K",
            "East",
            "East",
            "",
            "East Count",
            "West",
            "West Count",
            "Grand Total"
        ]
    );
    assert_eq!(edit(&f, "B5"), "=SUBTOTAL(3,B2:B4)");
    assert_eq!(edit(&f, "B7"), "=SUBTOTAL(3,B6:B6)");
    assert_eq!(edit(&f, "B8"), "=SUBTOTAL(3,B2:B7)");
    assert_eq!(edit(&f, "E1"), "=B3");
    assert_eq!(edit(&f, "E2"), "=SUM(B2:B6)");
    // Remove gives the data back; references follow.
    let sheet = f.sheet;
    let r = f.apply(vec![Op::RemoveSubtotals {
        sheet,
        range: range("A1:C8"),
    }]);
    assert!(r.errors.is_empty());
    assert_eq!(col(&f, "A", 6), ["K", "East", "East", "", "West", ""]);
    assert_eq!(edit(&f, "E2"), "=SUM(B2:B5)");
    f.svc.undo(f.doc).unwrap();
    f.svc.undo(f.doc).unwrap();
    assert_eq!(col(&f, "A", 6), ["K", "East", "East", "", "West", ""]);
    assert_eq!(edit(&f, "E2"), "=SUM(B2:B5)");
    assert!(groups(&f).0.is_empty());

    // Several result columns: "X Result"; empty value: "(empty)".
    let mut g = Fixture::new();
    rows_in(
        &mut g,
        &[&["K", "V", "W"], &["", "1", "2"], &["a", "3", "4"]],
    );
    let gs = g.sheet;
    let errs = sub(
        &mut g,
        "A1:C3",
        vec![SubtotalGroup {
            group_by: 0,
            columns: vec![1, 2],
            function: AggFn::Sum,
        }],
    );
    let _ = gs;
    assert!(errs.is_empty(), "{errs:?}");
    assert_eq!(f_text(&g, "A3"), "(empty) Result");
    assert_eq!(f_text(&g, "A5"), "a Result");
}

fn f_text(f: &Fixture, a1: &str) -> String {
    f.text(a1)
}

#[test]
fn h9_subtotals_with_many_groups_is_fast() {
    // 3,000 groups (was 10 s each way in release; one InsertRows per group).
    let n = 9_000u32;
    let mut f = Fixture::new();
    let mut rows: Vec<Vec<String>> = vec![vec!["K".into(), "V".into()]];
    rows.extend((0..n).map(|i| vec![format!("g{:05}", i / 3), (i % 7).to_string()]));
    let sheet = f.sheet;
    f.apply(vec![Op::SetRangeInputs {
        sheet,
        r: 0,
        c: 0,
        rows,
    }]);
    let t = Instant::now();
    let errs = sub(
        &mut f,
        &format!("A1:B{}", n + 1),
        vec![SubtotalGroup {
            group_by: 0,
            columns: vec![1],
            function: AggFn::Sum,
        }],
    );
    let add = t.elapsed();
    assert!(errs.is_empty(), "{errs:?}");
    assert_eq!(f.text("A5"), "g00000 Sum");
    let last = n + n / 3 + 2;
    assert_eq!(f.text(&format!("A{last}")), "Grand Total");
    let t = Instant::now();
    let r = f.apply(vec![Op::RemoveSubtotals {
        sheet,
        range: range(&format!("A1:B{last}")),
    }]);
    let remove = t.elapsed();
    assert!(r.errors.is_empty());
    assert_eq!(
        f.text(&format!("A{}", n + 1)),
        format!("g{:05}", (n - 1) / 3)
    );
    assert_eq!(f.text(&format!("A{}", n + 2)), "");
    eprintln!(
        "h9 subtotals {} groups: add {add:?}, remove {remove:?}",
        n / 3
    );
    assert!(add < Duration::from_secs(20) && remove < Duration::from_secs(20));
}

// ---------------------------------------------------------------- MEDIUM-T6

#[test]
fn t6_define_name_transaction_keeps_one_snapshot() {
    let mut f = Fixture::new();
    let ops: Vec<Op> = (0..5_000)
        .map(|i| Op::DefineName {
            name: format!("name_{i}"),
            scope: None,
            refers_to: "$Sheet1.$A$1".into(),
        })
        .collect();
    let t = Instant::now();
    let r = f.apply(ops);
    assert!(r.errors.is_empty());
    assert!(t.elapsed() < Duration::from_secs(20), "{:?}", t.elapsed());
    assert_eq!(f.svc.names(f.doc).unwrap().len(), 5_000);
    f.svc.undo(f.doc).unwrap();
    assert_eq!(f.svc.names(f.doc).unwrap().len(), 0);
    f.svc.redo(f.doc).unwrap();
    assert_eq!(f.svc.names(f.doc).unwrap().len(), 5_000);
}

// ---------------------------------------------------------------- M1/M2/M3

fn stdf(f: &mut Fixture, rng: &str, conds: Vec<tally_api::StdCondition>) -> Vec<String> {
    let sheet = f.sheet;
    let r = f.apply(vec![Op::StandardFilter {
        sheet,
        range: range(rng),
        conditions: conds,
        options: tally_api::StdFilterOptions {
            has_header: true,
            ..Default::default()
        },
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    let a = range(rng);
    let layout = f.svc.sheet_layout(f.doc, f.sheet).unwrap();
    let hidden = |row: u32| {
        layout
            .row_runs
            .iter()
            .any(|x| x.hidden && x.start <= row && row <= x.end)
    };
    (a.r0 + 1..=a.r1)
        .filter(|r| !hidden(*r))
        .map(|r| f.text(&format!("A{}", r + 1)))
        .collect()
}

fn cond(col: u32, op: tally_api::StdFilterOp, v: &str) -> tally_api::StdCondition {
    tally_api::StdCondition {
        connector: tally_api::StdConnector::And,
        col,
        op,
        value: v.into(),
    }
}

fn pay(f: &mut Fixture) {
    rows_in(
        f,
        &[
            &["Name", "Pay"],
            &["Ann", "10"],
            &["Bob", "30"],
            &["Cid", "20"],
            &["Fay", "n/a"],
            &["Gus", ""],
        ],
    );
}

#[test]
fn m1_mixed_type_comparisons() {
    use tally_api::StdFilterOp as F;
    let mut f = Fixture::new();
    pay(&mut f);
    // Text and empty cells never pass a numeric < / >.
    assert_eq!(
        stdf(&mut f, "A1:B6", vec![cond(1, F::Gt, "15")]),
        ["Bob", "Cid"]
    );
    let mut f = Fixture::new();
    pay(&mut f);
    assert_eq!(stdf(&mut f, "A1:B6", vec![cond(1, F::Lt, "15")]), ["Ann"]);
    // A text query compares text cells only.
    let mut f = Fixture::new();
    pay(&mut f);
    assert_eq!(stdf(&mut f, "A1:B6", vec![cond(1, F::Lt, "zzz")]), ["Fay"]);
    // <> keeps text and empty cells.
    let mut f = Fixture::new();
    pay(&mut f);
    assert_eq!(
        stdf(&mut f, "A1:B6", vec![cond(1, F::Ne, "10")]),
        ["Bob", "Cid", "Fay", "Gus"]
    );
}

#[test]
fn m2_top_n_ties_and_rounding() {
    use tally_api::StdFilterOp as F;
    let data: &[&[&str]] = &[
        &["V"],
        &["10"],
        &["20"],
        &["20"],
        &["30"],
        &["5"],
        &["6"],
        &["7"],
        &["8"],
        &["9"],
        &["11"],
    ];
    let run = |op, v: &str| {
        let mut f = Fixture::new();
        rows_in(&mut f, data);
        let mut shown = stdf(&mut f, "A1:A11", vec![cond(0, op, v)]);
        shown.sort_by_key(|s| s.parse::<i32>().unwrap());
        shown
    };
    assert_eq!(run(F::Largest, "2"), ["20", "20", "30"]);
    assert_eq!(run(F::Smallest, "2"), ["5", "6"]);
    assert_eq!(run(F::SmallestPct, "20"), ["5", "6", "7"]);
    assert_eq!(run(F::Largest, "0"), ["30"]);
}

#[test]
fn m3_contains_tests_the_input_string() {
    use tally_api::StdFilterOp as F;
    let mut f = Fixture::new();
    rows_in(&mut f, &[&["V"], &["1234.5"], &["99"]]);
    let sheet = f.sheet;
    f.apply(vec![Op::SetNumberFormat {
        sheet,
        ranges: vec![range("A2:A3")],
        code: "#,##0".into(),
    }]);
    assert_eq!(f.text("A2"), "1,235");
    assert_eq!(
        stdf(&mut f, "A1:A3", vec![cond(0, F::Contains, "234.5")]),
        ["1,235"]
    );
}

// ---------------------------------------------------------------- M7

#[test]
fn m7_weekday_series_adds_days_then_skips_the_weekend() {
    // 2026-10-23 is a Friday; AOO IncDate: +2 → Sun → Mon 10-26, …
    let mut f = Fixture::new();
    f.set("A1", "2026-10-23");
    let sheet = f.sheet;
    f.apply(vec![Op::FillSeries {
        sheet,
        range: range("A1:A4"),
        direction: tally_api::Direction::Down,
        kind: tally_api::SeriesKind::Date,
        unit: Some(tally_api::DateUnit::Weekday),
        start: None,
        end: None,
        increment: Some("2".into()),
    }]);
    let serial = |t: &str| {
        let mut g = Fixture::new();
        g.set("A1", t);
        g.value("A1")
    };
    assert_eq!(f.value("A2"), serial("2026-10-26"));
    assert_eq!(f.value("A3"), serial("2026-10-28"));
    assert_eq!(f.value("A4"), serial("2026-10-30"));
}

// ---------------------------------------------------------------- M16

#[test]
fn m16_copy_sheet_keeps_absolute_sheet_refs() {
    let mut f = Fixture::new();
    let s = f.sheet;
    f.set("A1", "5");
    f.set("B1", "=$Sheet1.A1");
    f.set("B2", "=Sheet1.A1");
    f.set("B3", "=A1");
    let r = f.apply(vec![Op::CopySheet {
        sheet: s,
        name: None,
        index: None,
    }]);
    assert!(r.errors.is_empty());
    let copy = r.created.unwrap()[0].id;
    f.apply(vec![common::input(copy, "A1", "100")]);
    assert_eq!(f.value_on(copy, "B1"), Value::Number(5.0));
    assert_eq!(f.value_on(copy, "B2"), Value::Number(100.0));
    assert_eq!(f.value_on(copy, "B3"), Value::Number(100.0));
}

// ---------------------------------------------------------------- M18

#[test]
fn m18_create_names_from_labels_follows_aoo() {
    let mut f = Fixture::new();
    let s = f.sheet;
    rows_in(
        &mut f,
        &[
            &["Corner", "Price ($)", "2024", "A1", "Größe"],
            &["r1", "1", "2", "3", "4"],
            &["  Lead", "6", "7", "8", "9"],
        ],
    );
    let r = f.apply(vec![Op::CreateNamesFromLabels {
        sheet: s,
        range: range("A1:E3"),
        from: tally_api::NameLabelFlags {
            top: true,
            left: true,
            ..Default::default()
        },
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    let mut names: Vec<(String, String)> = f
        .svc
        .names(f.doc)
        .unwrap()
        .into_iter()
        .map(|n| (n.name, n.refers_to))
        .collect();
    names.sort();
    let has = |n: &str| names.iter().any(|(x, _)| x == n);
    assert!(has("Corner"), "corner name: {names:?}");
    assert!(has("Price____"), "{names:?}");
    assert!(!has("_2024"), "a number is not a label: {names:?}");
    assert!(has("_A1"), "{names:?}");
    assert!(has("_r1"), "{names:?}");
    assert!(has("Lead"), "{names:?}");
    assert!(has("Größe"), "{names:?}");
    let corner = names.iter().find(|(x, _)| x == "Corner").unwrap();
    assert!(corner.1.ends_with("$B$2:$E$3"), "{corner:?}");
}

// ---------------------------------------------------------------- M19

#[test]
fn m19_autoformat_default_preset_and_autofit() {
    let mut f = Fixture::new();
    for r in 1..=4 {
        for c in ["A", "B", "C", "D"] {
            f.set(&format!("{c}{r}"), if r == 1 { "Head" } else { "1.5" });
        }
    }
    f.set("A1", "A very very long heading text here");
    let sheet = f.sheet;
    let r = f.apply(vec![Op::ApplyAutoFormat {
        sheet,
        range: range("A1:D4"),
        preset: "Default".into(),
        options: tally_api::AutoFormatOptions::default(),
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    let st = |a: &str| {
        let a = CellAddr::from_a1(sheet, a).unwrap();
        let wb = &f.svc.document(f.doc).unwrap().wb;
        wb.styles.to_dto(wb.style_at(a))
    };
    assert_eq!(
        (st("B1").fill, st("B1").color),
        (Some(0x0000_80ff), Some(0xffff_ffff))
    );
    assert_eq!(
        (st("A2").fill, st("A2").color),
        (Some(0x4d4d_4dff), Some(0xffff_ffff))
    );
    assert_eq!(st("D2").fill, Some(0xcccc_ccff));
    assert_eq!(st("B4").fill, Some(0xcccc_ccff));
    assert_eq!(st("B2").fill, Some(0xffff_ffff));
    assert_eq!(st("B2").font_size_pt, 10.0);
    assert!(st("B2").borders.top.is_some());
    // Autofit widened column A.
    let cols = f.svc.sheet_layout(f.doc, sheet).unwrap().col_runs;
    assert!(
        cols.iter().any(|c| c.start == 0 && c.size > 150.0),
        "{cols:?}"
    );
    // 2 × 2 is refused (AOO needs 3 × 3).
    let r = f.apply(vec![Op::ApplyAutoFormat {
        sheet,
        range: range("F1:G2"),
        preset: "Default".into(),
        options: tally_api::AutoFormatOptions::default(),
    }]);
    assert_eq!(r.errors.len(), 1);
}

// ---------------------------------------------------------------- M40

#[test]
fn m40_style_patch_clears_fill_and_colour() {
    let mut f = Fixture::new();
    let sheet = f.sheet;
    let patch = |p: tally_api::StylePatch| Op::ApplyStyle {
        sheet,
        ranges: vec![range("A1")],
        patch: p,
    };
    f.apply(vec![patch(tally_api::StylePatch {
        fill: Some(0xff00_00ff),
        color: Some(0x00ff_00ff),
        ..Default::default()
    })]);
    let st = |f: &Fixture| {
        let a = CellAddr::from_a1(sheet, "A1").unwrap();
        let wb = &f.svc.document(f.doc).unwrap().wb;
        wb.styles.to_dto(wb.style_at(a))
    };
    assert_eq!(st(&f).fill, Some(0xff00_00ff));
    f.apply(vec![patch(tally_api::StylePatch {
        clear_fill: Some(true),
        clear_color: Some(true),
        ..Default::default()
    })]);
    assert_eq!((st(&f).fill, st(&f).color), (None, None));
    f.apply(vec![patch(tally_api::StylePatch {
        fill: Some(0xff00_00ff),
        ..Default::default()
    })]);
    // The UI's transparent sentinel clears too, never stored.
    f.apply(vec![patch(tally_api::StylePatch {
        fill: Some(0xffff_ff00),
        color: Some(0xffff_ff00),
        ..Default::default()
    })]);
    assert_eq!((st(&f).fill, st(&f).color), (None, None));
}

// ---------------------------------------------------------------- M44

#[test]
fn m44_autoinput_scans_the_whole_column_for_text_only() {
    let mut f = Fixture::new();
    f.set("A1", "Apple");
    f.set("A5", "Apricot"); // not adjacent to the edited cell
    f.set("A3", "=\"Apex\""); // a formula result is never offered
    f.set("A10", "x");
    let ac = f.svc.autocomplete(f.doc, f.sheet, 9, 0, "Ap").unwrap();
    assert_eq!(ac.items, ["Apple", "Apricot"]);
    assert_eq!(ac.completion.as_deref(), Some("Apple"));
    // Long texts: the list item is cut, the completion is whole.
    let long = format!("Zz{}", "y".repeat(1000));
    f.set("A20", &long);
    let ac = f.svc.autocomplete(f.doc, f.sheet, 30, 0, "Zz").unwrap();
    assert_eq!(ac.completion.as_deref(), Some(long.as_str()));
    assert_eq!(ac.items[0].chars().count(), 256);
}

// ---------------------------------------------------------------- H10 / H11

#[test]
fn h10_text_to_columns_output_budget() {
    let mut f = Fixture::new();
    let sheet = f.sheet;
    // 100 rows of 16,000 fields = 1.6M cells > the 1M budget: refused.
    let wide = "a,".repeat(16_000);
    let rows: Vec<Vec<String>> = (0..100).map(|_| vec![wide.clone()]).collect();
    f.apply(vec![Op::SetRangeInputs {
        sheet,
        r: 0,
        c: 0,
        rows,
    }]);
    let t = Instant::now();
    let r = f.apply(vec![Op::TextToColumns {
        sheet,
        range: range("A1:A100"),
        options: tally_api::CsvImportOptionsDto {
            delimiters: ",".into(),
            ..Default::default()
        },
    }]);
    assert_eq!(r.errors.len(), 1, "refused");
    assert!(t.elapsed() < Duration::from_secs(10));
    // Fixed widths past the text make no fields.
    let mut g = Fixture::new();
    g.set("A1", "abcdef");
    let r = g.apply(vec![Op::TextToColumns {
        sheet: g.sheet,
        range: range("A1"),
        options: tally_api::CsvImportOptionsDto {
            fixed_widths: Some((0..16_000).collect()),
            ..Default::default()
        },
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(g.text("F1"), "f");
    assert_eq!(g.text("G1"), "");
}

#[test]
fn h11_consolidate_budget_and_link_length() {
    let mut f = Fixture::new();
    let sheet = f.sheet;
    let rows: Vec<Vec<String>> = (0..10).map(|i| vec![i.to_string()]).collect();
    f.apply(vec![Op::SetRangeInputs {
        sheet,
        r: 0,
        c: 0,
        rows,
    }]);
    let src = tally_api::SheetRange {
        sheet,
        range: range("A1:A10"),
    };
    // 255 sources × 10 rows, linked: each cell has 255 references, longer
    // than 8,192 characters with this sheet name → values and a warning.
    let long = "S".repeat(40);
    f.apply(vec![Op::RenameSheet { sheet, name: long }]);
    let r = f.apply(vec![Op::Consolidate {
        function: AggFn::Sum,
        sources: vec![src; 255],
        dest: tally_api::SheetCell { sheet, r: 0, c: 3 },
        by_row_labels: false,
        by_col_labels: false,
        link: true,
    }]);
    assert_eq!(r.errors.len(), 1, "one warning: {:?}", r.errors);
    assert_eq!(f.value("D2"), Value::Number(255.0));
    // The total of all sources is capped.
    let big = tally_api::SheetRange {
        sheet,
        range: range("A1:A1048576"),
    };
    f.set("A1048576", "1");
    let r = f.apply(vec![Op::Consolidate {
        function: AggFn::Sum,
        sources: vec![big; 2],
        dest: tally_api::SheetCell { sheet, r: 0, c: 5 },
        by_row_labels: false,
        by_col_labels: false,
        link: false,
    }]);
    assert_eq!(r.errors.len(), 1);
}

// ---------------------------------------------------------------- MEDIUM-T7

#[test]
fn t7_scenarios_document_budget() {
    let mut f = Fixture::new();
    let sheet = f.sheet;
    let scen = |name: &str, r: &str| tally_api::ScenarioDto {
        id: 0,
        name: name.into(),
        comment: String::new(),
        ranges: vec![range(r)],
        border_color: None,
        display_border: true,
        copy_back: false,
        copy_styles: false,
        prevent_changes: false,
        active: false,
    };
    // Two 1M-cell scenarios fit (2M per document); a third does not.
    for (i, ok) in [(0, true), (1, true), (2, false)] {
        let r = f.apply(vec![Op::CreateScenario {
            sheet,
            scenario: scen(&format!("s{i}"), "A1:A1000000"),
        }]);
        assert_eq!(r.errors.is_empty(), ok, "{i}: {:?}", r.errors);
    }
}

// ---------------------------------------------------------------- MEDIUM-T8

#[test]
fn t8_transaction_budget_counts_moved_cells() {
    let mut f = Fixture::new();
    let sheet = f.sheet;
    // The used area spans the whole column: every shift of A1 up counts
    // 1M cells, so 60 of them (60M) exceed MAX_TX_CELLS (50M).
    f.set("B1", "x");
    f.set("A1048575", "y");
    let ops: Vec<Op> = (0..60)
        .map(|_| Op::DeleteCells {
            sheet,
            range: range("A1"),
            shift: ShiftDir::Up,
        })
        .collect();
    let r = f.apply(ops);
    assert!(!r.errors.is_empty());
    assert!(
        r.errors
            .iter()
            .all(|e| e.code == tally_api::ApiError::UNSUPPORTED),
        "{:?}",
        r.errors[0]
    );
    // Advanced filter criteria are capped at 1,000 rows.
    let r = f.apply(vec![Op::AdvancedFilter {
        sheet,
        range: range("A1:A100"),
        criteria: tally_api::SheetRange {
            sheet,
            range: range("C1:C5000"),
        },
        options: tally_api::StdFilterOptions::default(),
    }]);
    assert_eq!(r.errors.len(), 1);
}

// ---------------------------------------------------------------- H12

#[test]
fn h12_many_groups_build_and_save_in_linear_time() {
    let mut f = io_fixture();
    let sheet = f.sheet;
    let n = 100_000u32;
    let rows: Vec<Vec<String>> = (0..n).map(|i| vec![i.to_string()]).collect();
    f.apply(vec![Op::SetRangeInputs {
        sheet,
        r: 0,
        c: 0,
        rows,
    }]);
    // 10,000 one-level groups and 1,000 outer ones (was 10.5 s on save).
    let t = Instant::now();
    let mut ops: Vec<Op> = (0..10_000u32)
        .map(|i| Op::Group {
            sheet,
            axis: OutlineAxis::Rows,
            start: i * 10,
            end: i * 10 + 4,
        })
        .collect();
    ops.extend((0..1_000u32).map(|i| Op::Group {
        sheet,
        axis: OutlineAxis::Rows,
        start: i * 100,
        end: i * 100 + 98,
    }));
    let r = f.apply(ops);
    assert!(r.errors.is_empty(), "{:?}", r.errors.first());
    let build = t.elapsed();
    let before = groups(&f);
    assert_eq!(before.0.len(), 11_000);
    let t = Instant::now();
    let g = reload(&mut f, FileFormat::Ods, "h12");
    let save = t.elapsed();
    assert_eq!(groups(&g), before);
    eprintln!("h12 build {build:?}, ODS save+open {save:?}");
    assert!(build < Duration::from_secs(20) && save < Duration::from_secs(60));
}

// ---------------------------------------------------------------- H9 dependents

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01\x08\x06\x00\x00\x00\x1f\x15\xc4\x89\x00\x00\x00\rIDATx\x9cc\xf8\xcf\xc0\xf0\x1f\x00\x05\x00\x02\x00\xde\xf2\x8a\xbb\x00\x00\x00\x00IEND\xaeB`\x82";

fn chart_anchor(r: u32) -> tally_api::ChartAnchor {
    tally_api::ChartAnchor {
        r,
        c: 4,
        dx_mm: 0.0,
        dy_mm: 0.0,
        width_mm: 80.0,
        height_mm: 50.0,
    }
}

/// A list with a chart and a picture anchored inside and below it, a
/// second sheet with a conditional format, a validation, a formula and a
/// name pointing into the list.
fn dependents_setup() -> (Fixture, u32) {
    let mut f = Fixture::new();
    let s = f.sheet;
    rows_in(
        &mut f,
        &[
            &["K", "V"],
            &["a", "1"],
            &["a", "2"],
            &["b", "3"],
            &["b", "4"],
        ],
    );
    let axis = tally_api::ChartAxisDto {
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
    let chart = tally_api::ChartDto {
        id: 0,
        name: String::new(),
        anchor: chart_anchor(3),
        kind: tally_api::ChartKind::Column,
        stacking: tally_api::Stacking::None,
        lines: true,
        markers: false,
        smooth: false,
        hole: None,
        exploded: false,
        filled: false,
        stock: None,
        three_d: false,
        source: None,
        categories: Some("$Sheet1.$A$2:$A$5".into()),
        series: vec![tally_api::ChartSeriesDto {
            name: None,
            values: "$Sheet1.$B$2:$B$5".into(),
            x_values: None,
            sizes: None,
            kind: None,
            secondary_axis: false,
            color: None,
            marker: None,
            labels: tally_api::DataLabelsDto::default(),
            trend: None,
            error_bars: None,
            mean_line: false,
        }],
        title: None,
        subtitle: None,
        legend: tally_api::LegendPos::Right,
        x_axis: axis.clone(),
        y_axis: axis,
        y2_axis: None,
        background: None,
        plot_background: None,
        include_hidden: false,
    };
    let r = f.apply(vec![
        Op::AddChart { sheet: s, chart },
        Op::AddImage {
            sheet: s,
            image: tally_api::ImageDto {
                id: 0,
                name: String::new(),
                anchor: chart_anchor(9),
                mime: "image/png".into(),
                data: tally_engine::protection::base64(PNG),
            },
        },
        Op::AddSheet {
            name: None,
            index: None,
        },
        Op::DefineName {
            name: "bvals".into(),
            scope: None,
            refers_to: "$Sheet1.$B$4:$B$5".into(),
        },
    ]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    let s2 = f.svc.document(f.doc).unwrap().info().sheets[1].id;
    let r = f.apply(vec![
        common::input(s2, "A1", "=SUM($Sheet1.B2:B5)"),
        common::input(s2, "A2", "=SUM(bvals)"),
        Op::AddConditionalFormat {
            sheet: s2,
            ranges: vec![range("A1")],
            rules: vec![tally_api::CfRuleDto {
                kind: tally_api::CfRuleKind::CellValue {
                    op: tally_api::CmpOp::Gt,
                    formulas: vec!["$Sheet1.$B$4".into()],
                },
                style: tally_api::CfStyleDto::default(),
                stop_if_true: false,
            }],
        },
    ]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    (f, s2)
}

/// What the dependents look like, for comparison.
fn dependents(f: &Fixture, s2: u32) -> String {
    let s = f.sheet;
    let charts: Vec<String> = f
        .svc
        .charts(f.doc, s)
        .unwrap()
        .iter()
        .map(|c| format!("{} {:?} {:?}", c.anchor.r, c.categories, c.series[0].values))
        .collect();
    let images: Vec<u32> = f
        .svc
        .images(f.doc, s)
        .unwrap()
        .iter()
        .map(|i| i.anchor.r)
        .collect();
    let cf = format!(
        "{:?}",
        f.svc.conditional_formats(f.doc, s2).unwrap()[0].rules[0].kind
    );
    let names: Vec<String> = f
        .svc
        .names(f.doc)
        .unwrap()
        .iter()
        .map(|n| n.refers_to.clone())
        .collect();
    let e = |a1: &str| {
        let a = CellAddr::from_a1(s2, a1).unwrap();
        f.svc.cell_edit_info(f.doc, s2, a.row, a.col).unwrap().input
    };
    format!(
        "charts={charts:?} images={images:?} cf={cf} names={names:?} A1={} A2={}",
        e("A1"),
        e("A2")
    )
}

#[test]
fn h9_subtotals_move_dependents_like_row_inserts() {
    // Subtotals over a/a/b/b insert one row after the a group (before
    // row 4) and two at the end (b subtotal, grand total): the same as
    // these two InsertRows.
    let (mut f, s2) = dependents_setup();
    let errs = sub(
        &mut f,
        "A1:B5",
        vec![SubtotalGroup {
            group_by: 0,
            columns: vec![1],
            function: AggFn::Sum,
        }],
    );
    assert!(errs.is_empty(), "{errs:?}");
    let after = dependents(&f, s2);

    let (mut g, gs2) = dependents_setup();
    let sheet = g.sheet;
    g.apply(vec![
        Op::InsertRows {
            sheet,
            at: 5,
            count: 2,
        },
        Op::InsertRows {
            sheet,
            at: 3,
            count: 1,
        },
    ]);
    assert_eq!(after, dependents(&g, gs2));
    // The chart grew over the inner subtotal row, the picture below moved
    // down three rows, the other sheet's rule and formulas followed.
    assert!(after.contains("$B$2:$B$6"), "{after}");
    assert!(after.contains("images=[12]"), "{after}");
    assert!(after.contains("$B$5"), "{after}");

    // Remove Subtotals brings everything back; so does undo.
    let (h, hs2) = dependents_setup();
    let before = dependents(&h, hs2);
    let r = f.apply(vec![Op::RemoveSubtotals {
        sheet: f.sheet,
        range: range("A1:B8"),
    }]);
    assert!(r.errors.is_empty());
    assert_eq!(dependents(&f, s2), before);
    f.svc.undo(f.doc).unwrap();
    assert_eq!(dependents(&f, s2), after);
    f.svc.undo(f.doc).unwrap();
    assert_eq!(dependents(&f, s2), before);
}
