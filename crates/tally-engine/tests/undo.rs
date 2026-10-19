//! Undo/redo round trips for every op, and a proptest over random op
//! sequences: undo-all restores the initial `to_data()`, redo-all the final one.
#![allow(clippy::unwrap_used)]

mod common;

use common::{input, range, sel, Fixture};
use proptest::prelude::*;
use tally_api::{ClearWhat, DocumentService, Op, PastePayload, StylePatch};
use tally_types::Value;

/// Applies `op`, checks that undo restores the state before and redo the state after.
fn round_trip(f: &mut Fixture, op: Op) {
    let before = f.data();
    let res = f.apply(vec![op.clone()]);
    assert!(res.errors.is_empty(), "{op:?}: {:?}", res.errors);
    assert!(res.undo.can_undo);
    let after = f.data();
    assert_ne!(before, after, "{op:?} changed nothing");
    let u = f.svc.undo(f.doc).unwrap();
    assert!(u.undo.can_redo);
    assert_eq!(f.data(), before, "undo of {op:?}");
    f.svc.redo(f.doc).unwrap();
    assert_eq!(f.data(), after, "redo of {op:?}");
}

fn seeded() -> Fixture {
    let mut f = Fixture::new();
    f.set("A1", "1");
    f.set("A2", "2");
    f.set("B1", "=A1+A2");
    f.set("B2", "=SUM(A1:A2)*2");
    f.set("C1", "hello");
    f
}

#[test]
fn every_op_round_trips() {
    let mut f = seeded();
    let s = f.sheet;
    round_trip(&mut f, input(s, "A1", "5"));
    round_trip(&mut f, input(s, "D4", "=B1*3"));
    round_trip(&mut f, input(s, "A2", "15%"));
    round_trip(&mut f, input(s, "B1", ""));
    round_trip(
        &mut f,
        Op::SetRangeInputs {
            sheet: s,
            r: 4,
            c: 0,
            rows: vec![vec!["1".into(), "=A5*2".into()], vec!["x".into()]],
        },
    );
    for what in [ClearWhat::Contents, ClearWhat::Formats, ClearWhat::All] {
        f.apply(vec![Op::ApplyStyle {
            sheet: s,
            ranges: vec![range("A1:C2")],
            patch: StylePatch {
                italic: Some(true),
                ..StylePatch::default()
            },
        }]);
        round_trip(
            &mut f,
            Op::Clear {
                sheet: s,
                ranges: vec![range("A1:B2")],
                what,
            },
        );
    }
    round_trip(
        &mut f,
        Op::ApplyStyle {
            sheet: s,
            ranges: vec![range("A1:C3")],
            patch: StylePatch {
                bold: Some(true),
                ..StylePatch::default()
            },
        },
    );
    round_trip(
        &mut f,
        Op::SetNumberFormat {
            sheet: s,
            ranges: vec![range("A1:A2")],
            code: "0.00%".into(),
        },
    );
    round_trip(
        &mut f,
        Op::SetColWidth {
            sheet: s,
            start: 1,
            end: 3,
            px: 150.0,
        },
    );
    round_trip(
        &mut f,
        Op::SetRowHeight {
            sheet: s,
            start: 0,
            end: 0,
            px: 40.0,
        },
    );
    round_trip(
        &mut f,
        Op::AddSheet {
            name: Some("Extra".into()),
            index: Some(0),
        },
    );
    round_trip(
        &mut f,
        Op::RenameSheet {
            sheet: s,
            name: "Main".into(),
        },
    );
    // A second sheet that references the first, then delete the first.
    let res = f.apply(vec![Op::AddSheet {
        name: Some("Other".into()),
        index: None,
    }]);
    let other = res
        .info
        .unwrap()
        .sheets
        .iter()
        .find(|x| x.name == "Other")
        .unwrap()
        .id;
    f.apply(vec![input(other, "A1", "=Main.A1+1")]);
    round_trip(
        &mut f,
        Op::MoveSheet {
            sheet: other,
            to: 0,
        },
    );
    round_trip(&mut f, Op::DeleteSheet { sheet: s });
    // Paste: TSV and internal (on the remaining sheet).
    let s = other;
    round_trip(
        &mut f,
        Op::Paste {
            sheet: s,
            target: range("E1"),
            payload: PastePayload::Tsv {
                text: "1\t2\n=E1+F1\t\n".into(),
            },
        },
    );
    let clip = f.svc.copy(f.doc, &sel(s, "A1:B2")).unwrap();
    round_trip(
        &mut f,
        Op::Paste {
            sheet: s,
            target: range("H5"),
            payload: PastePayload::Internal { token: clip.token },
        },
    );
}

#[test]
fn structural_ops_round_trip_and_adjust_references() {
    let mut f = seeded();
    let s = f.sheet;
    f.set("D1", "=SUM(A1:A2)");
    f.set("D5", "=A2*10");
    round_trip(
        &mut f,
        Op::InsertRows {
            sheet: s,
            at: 1,
            count: 2,
        },
    );
    // Now A2 is at A4; D5 moved to D7 and still reads it.
    assert_eq!(f.value("A4"), Value::Number(2.0));
    assert_eq!(
        f.svc.cell_edit_info(f.doc, s, 6, 3).unwrap().input,
        "=A4*10"
    );
    assert_eq!(
        f.svc.cell_edit_info(f.doc, s, 0, 3).unwrap().input,
        "=SUM(A1:A4)"
    );
    round_trip(
        &mut f,
        Op::DeleteRows {
            sheet: s,
            at: 3,
            count: 1,
        },
    );
    assert_eq!(
        f.svc.cell_edit_info(f.doc, s, 5, 3).unwrap().input,
        "=#REF!*10"
    );
    assert_eq!(f.value("D6"), Value::Error(tally_types::ErrorKind::Ref));
    round_trip(
        &mut f,
        Op::InsertCols {
            sheet: s,
            at: 0,
            count: 1,
        },
    );
    assert_eq!(
        f.svc.cell_edit_info(f.doc, s, 0, 4).unwrap().input,
        "=SUM(B1:B3)"
    );
    round_trip(
        &mut f,
        Op::DeleteCols {
            sheet: s,
            at: 0,
            count: 2,
        },
    );
    let bad = f.apply(vec![Op::InsertRows {
        sheet: s,
        at: 0,
        count: 0,
    }]);
    assert_eq!(bad.errors[0].code, "invalid_argument");
}

#[test]
fn undo_restores_selection_and_labels() {
    let mut f = Fixture::new();
    let s = f.sheet;
    let tx = tally_api::Transaction {
        ops: vec![input(s, "A1", "1")],
        label: Some("Typing".into()),
        selection_before: Some(sel(s, "A1")),
        selection_after: Some(sel(s, "A2")),
    };
    let r = f.svc.apply(f.doc, tx).unwrap();
    assert_eq!(r.undo.undo_label.as_deref(), Some("Typing"));
    assert_eq!(r.selection, Some(sel(s, "A2")));
    let u = f.svc.undo(f.doc).unwrap();
    assert_eq!(u.selection, Some(sel(s, "A1")));
    assert_eq!(u.undo.redo_label.as_deref(), Some("Typing"));
    assert_eq!(f.value("A1"), Value::Empty);
    let r2 = f.svc.redo(f.doc).unwrap();
    assert_eq!(r2.selection, Some(sel(s, "A2")));
    assert!(r2.version > u.version);
}

#[test]
fn undo_depth_is_100() {
    let mut f = Fixture::new();
    for i in 0..120 {
        f.set("A1", &i.to_string());
    }
    let mut n = 0;
    while f.svc.document(f.doc).unwrap().undo.state().can_undo {
        f.svc.undo(f.doc).unwrap();
        n += 1;
    }
    assert_eq!(n, 100);
    assert_eq!(f.value("A1"), Value::Number(19.0));
}

#[derive(Clone, Debug)]
enum GenOp {
    Input(usize, u32, u32, &'static str),
    Clear(usize, u32, u32, u8),
    Bold(usize, u32, u32, bool),
    Format(usize, u32, &'static str),
    Width(usize, u32, f32),
    AddSheet,
    Rename(usize, u8),
    Delete(usize),
    Move(usize, u32),
    Tsv(usize, u32, u32),
    Rows(usize, u32, u32, bool),
    Cols(usize, u32, u32, bool),
}

const INPUTS: &[&str] = &[
    "1",
    "2.5",
    "abc",
    "",
    "=A1+B1",
    "=SUM(A1:C3)",
    "=1/0",
    "=B2*2",
    "TRUE",
    "15%",
    "$3.50",
    "=A1&\"x\"",
    "=Sheet2.A1+1",
    "10/19/2026",
    "=C3",
    "=IF(A1>1,A2,A3)",
];
const FORMATS: &[&str] = &["0.00", "#,##0", "0%", "General", "MM/DD/YYYY"];

fn gen_op() -> impl Strategy<Value = GenOp> {
    prop_oneof![
        6 => (0usize..3, 0u32..4, 0u32..4, proptest::sample::select(INPUTS)).prop_map(|(s, r, c, i)| GenOp::Input(s, r, c, i)),
        1 => (0usize..3, 0u32..4, 0u32..4, 0u8..3).prop_map(|(s, r, c, w)| GenOp::Clear(s, r, c, w)),
        1 => (0usize..3, 0u32..4, 0u32..4, any::<bool>()).prop_map(|(s, r, c, b)| GenOp::Bold(s, r, c, b)),
        1 => (0usize..3, 0u32..4, proptest::sample::select(FORMATS)).prop_map(|(s, c, f)| GenOp::Format(s, c, f)),
        1 => (0usize..3, 0u32..4, 20.0f32..200.0).prop_map(|(s, c, w)| GenOp::Width(s, c, w)),
        1 => Just(GenOp::AddSheet),
        1 => (0usize..3, 0u8..4).prop_map(|(s, n)| GenOp::Rename(s, n)),
        1 => (0usize..3).prop_map(GenOp::Delete),
        1 => (0usize..3, 0u32..3).prop_map(|(s, t)| GenOp::Move(s, t)),
        1 => (0usize..3, 0u32..4, 0u32..4).prop_map(|(s, r, c)| GenOp::Tsv(s, r, c)),
        1 => (0usize..3, 0u32..4, 1u32..3, any::<bool>()).prop_map(|(s, a, n, i)| GenOp::Rows(s, a, n, i)),
        1 => (0usize..3, 0u32..4, 1u32..3, any::<bool>()).prop_map(|(s, a, n, i)| GenOp::Cols(s, a, n, i)),
    ]
}

fn to_op(f: &Fixture, g: &GenOp) -> Op {
    let wb = &f.svc.document(f.doc).unwrap().wb;
    let sid = |i: usize| wb.sheets[i % wb.sheets.len()].id;
    let one = |r: u32, c: u32| tally_api::RangeDto {
        r0: r,
        c0: c,
        r1: r + 1,
        c1: c + 1,
    };
    match g {
        GenOp::Input(s, r, c, i) => Op::SetCellInput {
            sheet: sid(*s),
            r: *r,
            c: *c,
            input: (*i).into(),
        },
        GenOp::Clear(s, r, c, w) => Op::Clear {
            sheet: sid(*s),
            ranges: vec![one(*r, *c)],
            what: [ClearWhat::Contents, ClearWhat::Formats, ClearWhat::All][*w as usize],
        },
        GenOp::Bold(s, r, c, b) => Op::ApplyStyle {
            sheet: sid(*s),
            ranges: vec![one(*r, *c)],
            patch: StylePatch {
                bold: Some(*b),
                ..StylePatch::default()
            },
        },
        GenOp::Format(s, c, code) => Op::SetNumberFormat {
            sheet: sid(*s),
            ranges: vec![tally_api::RangeDto {
                r0: 0,
                c0: *c,
                r1: 1_048_575,
                c1: *c,
            }],
            code: (*code).into(),
        },
        GenOp::Width(s, c, w) => Op::SetColWidth {
            sheet: sid(*s),
            start: *c,
            end: *c,
            px: *w,
        },
        GenOp::AddSheet => Op::AddSheet {
            name: None,
            index: None,
        },
        GenOp::Rename(s, n) => Op::RenameSheet {
            sheet: sid(*s),
            name: format!("Sheet{}", n + 1),
        },
        GenOp::Delete(s) => Op::DeleteSheet { sheet: sid(*s) },
        GenOp::Move(s, t) => Op::MoveSheet {
            sheet: sid(*s),
            to: *t,
        },
        GenOp::Tsv(s, r, c) => Op::Paste {
            sheet: sid(*s),
            target: one(*r, *c),
            payload: PastePayload::Tsv {
                text: "7\t=A1*2\nq\t".into(),
            },
        },
        GenOp::Rows(s, at, count, true) => Op::InsertRows {
            sheet: sid(*s),
            at: *at,
            count: *count,
        },
        GenOp::Rows(s, at, count, false) => Op::DeleteRows {
            sheet: sid(*s),
            at: *at,
            count: *count,
        },
        GenOp::Cols(s, at, count, true) => Op::InsertCols {
            sheet: sid(*s),
            at: *at,
            count: *count,
        },
        GenOp::Cols(s, at, count, false) => Op::DeleteCols {
            sheet: sid(*s),
            at: *at,
            count: *count,
        },
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

    #[test]
    fn random_ops_then_undo_all_restores_initial(ops in proptest::collection::vec(gen_op(), 1..25)) {
        let mut f = seeded();
        f.apply(vec![Op::AddSheet { name: Some("Sheet2".into()), index: None }]);
        let initial = f.data();
        let start_depth = f.svc.document(f.doc).unwrap().undo.undo_len();
        for g in &ops {
            let op = to_op(&f, g);
            f.apply(vec![op]);
        }
        let fin = f.data();
        let mut steps = 0;
        while f.svc.document(f.doc).unwrap().undo.undo_len() > start_depth {
            f.svc.undo(f.doc).unwrap();
            steps += 1;
        }
        prop_assert_eq!(&f.data(), &initial);
        for _ in 0..steps {
            f.svc.redo(f.doc).unwrap();
        }
        prop_assert_eq!(&f.data(), &fin);
    }
}
