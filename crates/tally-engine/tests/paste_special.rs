//! Paste Special to AOO 4.1 parity (`ScViewFunc::PasteFromClip`): the
//! Selection kinds, Link, Shift cells and their combination with the
//! operations, Skip empty cells and Transpose.
#![allow(clippy::unwrap_used)]

mod common;

use common::{range, sel, tx, Fixture};
use tally_api::{
    DocumentService, NoteDto, Op, PasteOperation, PasteOptions, PastePayload, PasteSelection,
    ShiftDir, StylePatch,
};
use tally_types::{CellAddr, ErrorKind, SheetId, Value};

fn n(x: f64) -> Value {
    Value::Number(x)
}

fn input_of(f: &Fixture, sheet: SheetId, a1: &str) -> String {
    let a = CellAddr::from_a1(sheet, a1).unwrap();
    f.svc
        .cell_edit_info(f.doc, sheet, a.row, a.col)
        .unwrap()
        .input
}

fn kinds(spec: &str) -> PasteSelection {
    PasteSelection {
        text: spec.contains('s'),
        numbers: spec.contains('v'),
        date_time: spec.contains('d'),
        formulas: spec.contains('f'),
        notes: spec.contains('n'),
        formats: spec.contains('t'),
    }
}

fn sel_opts(spec: &str) -> PasteOptions {
    PasteOptions {
        selection: Some(kinds(spec)),
        ..PasteOptions::default()
    }
}

fn copy(f: &mut Fixture, sheet: SheetId, a1: &str) -> String {
    f.svc.copy(f.doc, &sel(sheet, a1)).unwrap().token
}

fn paste(sheet: SheetId, target: &str, token: &str, options: PasteOptions) -> Op {
    Op::PasteSpecial {
        sheet,
        target: range(target),
        payload: PastePayload::Internal {
            token: token.into(),
        },
        options,
    }
}

/// Applies `ops`, checks there is no error and that undo and redo restore
/// the states exactly.
fn round_trip(f: &mut Fixture, ops: Vec<Op>) {
    let before = f.data();
    let res = f.apply(ops.clone());
    assert!(res.errors.is_empty(), "{ops:?}: {:?}", res.errors);
    let after = f.data();
    f.svc.undo(f.doc).unwrap();
    assert_eq!(f.data(), before, "undo of {ops:?}");
    f.svc.redo(f.doc).unwrap();
    assert_eq!(f.data(), after, "redo of {ops:?}");
}

fn note(f: &mut Fixture, a1: &str, text: &str) {
    let a = CellAddr::from_a1(f.sheet, a1).unwrap();
    let s = f.sheet;
    let res = f.apply(vec![Op::SetNote {
        sheet: s,
        r: a.row,
        c: a.col,
        note: NoteDto {
            r: 0,
            c: 0,
            text: text.into(),
            author: None,
            created: None,
            shown: false,
        },
    }]);
    assert!(res.errors.is_empty(), "{:?}", res.errors);
}

fn note_at(f: &Fixture, a1: &str) -> Option<String> {
    let a = CellAddr::from_a1(f.sheet, a1).unwrap();
    f.svc
        .notes(f.doc, f.sheet)
        .unwrap()
        .into_iter()
        .find(|x| x.r == a.row && x.c == a.col)
        .map(|x| x.text)
}

fn bold(f: &mut Fixture, a1: &str) {
    let s = f.sheet;
    f.apply(vec![Op::ApplyStyle {
        sheet: s,
        ranges: vec![range(a1)],
        patch: StylePatch {
            bold: Some(true),
            ..StylePatch::default()
        },
    }]);
}

fn is_bold(f: &Fixture, a1: &str) -> bool {
    let st = f.svc.cells(f.doc, f.sheet, &[range(a1)]).unwrap()[0]
        .cells
        .first()
        .map_or(0, |c| c.style);
    f.svc.styles(f.doc, &[st]).unwrap()[0].bold
}

/// A1:A4 = text, number, date, formula (=A2*2).
fn source(f: &mut Fixture) -> String {
    f.set("A1", "abc");
    f.set("A2", "5");
    f.set("A3", "2024-01-15");
    f.set("A4", "=A2*2");
    let s = f.sheet;
    copy(f, s, "A1:A4")
}

#[test]
fn formula_with_numbers_only_pastes_its_value() {
    let mut f = Fixture::new();
    let s = f.sheet;
    let tok = source(&mut f);
    round_trip(&mut f, vec![paste(s, "C1", &tok, sel_opts("v"))]);
    assert_eq!(f.value("C1"), Value::Empty, "text not selected");
    assert_eq!(f.value("C2"), n(5.0));
    assert_eq!(f.value("C3"), Value::Empty, "a date is not a number");
    assert_eq!(f.value("C4"), n(10.0));
    assert_eq!(input_of(&f, s, "C4"), "10", "the result, not the formula");
    // Dates only: just the date.
    f.apply(vec![paste(s, "D1", &tok, sel_opts("d"))]);
    assert_eq!(f.value("D2"), Value::Empty);
    assert!(matches!(f.value("D3"), Value::Number(_)));
    assert_eq!(f.value("D4"), Value::Empty);
    // Text only: a formula's text result is pasted, numbers are not.
    f.set("B1", "=\"x\"&A1");
    let tok2 = copy(&mut f, s, "A1:B4");
    f.apply(vec![paste(s, "F1", &tok2, sel_opts("s"))]);
    assert_eq!(f.value("F1"), Value::text("abc"));
    assert_eq!(f.value("G1"), Value::text("xabc"));
    assert_eq!(input_of(&f, s, "G1"), "xabc");
    assert_eq!(f.value("F2"), Value::Empty);
    // Formulas only: only formula cells (the clipboard keeps the last copy).
    f.apply(vec![paste(s, "I1", &tok2, sel_opts("f"))]);
    assert_eq!(f.value("I1"), Value::Empty);
    assert_eq!(f.value("I2"), Value::Empty);
    assert_eq!(input_of(&f, s, "I4"), "=I2*2");
    assert_eq!(input_of(&f, s, "J1"), "=\"x\"&I1");
}

#[test]
fn without_skip_empty_the_target_is_cleared() {
    let mut f = Fixture::new();
    let s = f.sheet;
    let tok = source(&mut f);
    // Target holds numbers, text and a note; Text only clears all contents
    // and notes of the block first (AOO nDelFlag = IDF_CONTENTS).
    for a in ["C1", "C2", "C3", "C4"] {
        f.set(a, "9");
    }
    note(&mut f, "C2", "old");
    round_trip(&mut f, vec![paste(s, "C1", &tok, sel_opts("s"))]);
    assert_eq!(f.value("C1"), Value::text("abc"));
    assert_eq!(f.value("C2"), Value::Empty);
    assert_eq!(f.value("C4"), Value::Empty);
    assert_eq!(note_at(&f, "C2"), None);
    // With Skip empty cells the target's numbers stay where nothing is
    // pasted.
    for a in ["E1", "E2", "E3", "E4"] {
        f.set(a, "9");
    }
    note(&mut f, "E2", "old");
    let opts = PasteOptions {
        skip_empty: true,
        ..sel_opts("s")
    };
    round_trip(&mut f, vec![paste(s, "E1", &tok, opts)]);
    assert_eq!(f.value("E1"), Value::text("abc"));
    assert_eq!(f.value("E2"), n(9.0));
    assert_eq!(f.value("E4"), n(9.0));
    assert_eq!(note_at(&f, "E2").as_deref(), Some("old"));
}

#[test]
fn notes_only_and_formats_only() {
    let mut f = Fixture::new();
    let s = f.sheet;
    f.set("A1", "1");
    f.set("A2", "2");
    note(&mut f, "A1", "hello");
    bold(&mut f, "A1");
    let tok = copy(&mut f, s, "A1:A2");
    f.set("C1", "keep");
    f.set("C2", "keep2");
    note(&mut f, "C2", "gone");
    // Notes only: contents stay, the block's notes are replaced.
    round_trip(&mut f, vec![paste(s, "C1", &tok, sel_opts("n"))]);
    assert_eq!(f.value("C1"), Value::text("keep"));
    assert_eq!(f.value("C2"), Value::text("keep2"));
    assert_eq!(note_at(&f, "C1").as_deref(), Some("hello"));
    assert_eq!(note_at(&f, "C2"), None);
    assert!(!is_bold(&f, "C1"));
    // Formats only.
    f.set("E1", "x");
    round_trip(&mut f, vec![paste(s, "E1", &tok, sel_opts("t"))]);
    assert_eq!(f.value("E1"), Value::text("x"));
    assert!(is_bold(&f, "E1"));
    assert_eq!(note_at(&f, "E1"), None);
    // Legacy flags still work.
    f.set("G1", "x");
    f.apply(vec![paste(
        s,
        "G1",
        &tok,
        PasteOptions {
            formats_only: true,
            ..PasteOptions::default()
        },
    )]);
    assert_eq!(f.value("G1"), Value::text("x"));
    assert!(is_bold(&f, "G1"));
}

#[test]
fn skip_empty_formats_come_from_non_empty_cells() {
    let mut f = Fixture::new();
    let s = f.sheet;
    f.set("A1", "1");
    bold(&mut f, "A2");
    let tok = copy(&mut f, s, "A1:A2");
    bold(&mut f, "C1");
    bold(&mut f, "C2");
    f.set("C2", "t");
    let opts = PasteOptions {
        skip_empty: true,
        ..PasteOptions::default()
    };
    round_trip(&mut f, vec![paste(s, "C1", &tok, opts)]);
    assert_eq!(f.value("C1"), n(1.0));
    assert!(!is_bold(&f, "C1"), "A1's (default) format is pasted");
    assert!(is_bold(&f, "C2"), "an empty source cell's format is not");
    assert_eq!(f.value("C2"), Value::text("t"));
}

#[test]
fn link_same_sheet_other_sheet_and_updates() {
    let mut f = Fixture::new();
    let s = f.sheet;
    f.set("A1", "1");
    f.set("B1", "hi");
    let tok = copy(&mut f, s, "A1:C1");
    let opts = PasteOptions {
        link: Some(true),
        ..PasteOptions::default()
    };
    round_trip(&mut f, vec![paste(s, "A3", &tok, opts.clone())]);
    // Absolute references with the sheet (AOO CreateRefCell), empty
    // cells too with Paste all.
    assert_eq!(input_of(&f, s, "A3"), "=$Sheet1.$A$1");
    assert_eq!(input_of(&f, s, "B3"), "=$Sheet1.$B$1");
    assert_eq!(input_of(&f, s, "C3"), "=$Sheet1.$C$1");
    assert_eq!(f.value("C3"), n(0.0));
    f.set("A1", "42");
    assert_eq!(f.value("A3"), n(42.0), "the link follows the source");
    // Another sheet.
    f.apply(vec![Op::AddSheet {
        name: Some("Other".into()),
        index: None,
    }]);
    let other = f.svc.info(f.doc).unwrap().sheets[1].id;
    round_trip(&mut f, vec![paste(other, "D5", &tok, opts)]);
    assert_eq!(input_of(&f, other, "D5"), "=$Sheet1.$A$1");
    assert_eq!(f.value_on(other, "D5"), n(42.0));
    // Kinds: only numbers are linked, empty cells are not.
    let opts = PasteOptions {
        link: Some(true),
        ..sel_opts("v")
    };
    f.apply(vec![paste(s, "A6", &tok, opts)]);
    assert_eq!(input_of(&f, s, "A6"), "=$Sheet1.$A$1");
    assert_eq!(input_of(&f, s, "B6"), "");
    assert_eq!(input_of(&f, s, "C6"), "");
}

#[test]
fn link_transposed_and_refused_for_text() {
    let mut f = Fixture::new();
    let s = f.sheet;
    f.set("A1", "1");
    f.set("B1", "2");
    let tok = copy(&mut f, s, "A1:B1");
    let opts = PasteOptions {
        link: Some(true),
        transpose: true,
        ..PasteOptions::default()
    };
    round_trip(&mut f, vec![paste(s, "D1", &tok, opts)]);
    assert_eq!(input_of(&f, s, "D1"), "=$Sheet1.$A$1");
    assert_eq!(input_of(&f, s, "D2"), "=$Sheet1.$B$1");
    assert_eq!(f.value("D2"), n(2.0));
    // TSV (system clipboard): no link.
    let res = f.apply(vec![Op::PasteSpecial {
        sheet: s,
        target: range("F1"),
        payload: PastePayload::Tsv { text: "1".into() },
        options: PasteOptions {
            link: Some(true),
            ..PasteOptions::default()
        },
    }]);
    assert_eq!(res.errors.len(), 1);
    // Another document: no link.
    let other = f.svc.new_workbook(None).unwrap();
    let os = other.sheets[0].id;
    let res = f
        .svc
        .apply(
            other.doc,
            tx(vec![paste(
                os,
                "A1",
                &tok,
                PasteOptions {
                    link: Some(true),
                    ..PasteOptions::default()
                },
            )]),
        )
        .unwrap();
    assert_eq!(res.errors.len(), 1, "{:?}", res.errors);
    // A plain paste into the other document still works.
    let res = f
        .svc
        .apply(
            other.doc,
            tx(vec![paste(os, "A1", &tok, PasteOptions::default())]),
        )
        .unwrap();
    assert!(res.errors.is_empty());
}

#[test]
fn shift_down_and_right_with_undo() {
    let mut f = Fixture::new();
    let s = f.sheet;
    f.set("A1", "1");
    f.set("A2", "2");
    let tok = copy(&mut f, s, "A1:A2");
    f.set("C1", "x");
    f.set("C2", "y");
    f.set("D1", "z");
    f.set("C5", "=C1");
    let before = f.data();
    let res = f.apply(vec![paste(
        s,
        "C1",
        &tok,
        PasteOptions {
            shift: Some(ShiftDir::Down),
            ..PasteOptions::default()
        },
    )]);
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    assert_eq!(f.value("C1"), n(1.0));
    assert_eq!(f.value("C2"), n(2.0));
    assert_eq!(f.value("C3"), Value::text("x"));
    assert_eq!(f.value("C4"), Value::text("y"));
    assert_eq!(
        f.value("D1"),
        Value::text("z"),
        "only the block's columns move"
    );
    assert_eq!(input_of(&f, s, "C7"), "=C3", "references follow the shift");
    f.svc.undo(f.doc).unwrap();
    assert_eq!(f.data(), before, "one undo step");
    f.svc.redo(f.doc).unwrap();
    assert_eq!(f.value("C3"), Value::text("x"));
    // Right.
    let res = f.apply(vec![paste(
        s,
        "F1",
        &tok,
        PasteOptions {
            shift: Some(ShiftDir::Right),
            ..PasteOptions::default()
        },
    )]);
    assert!(res.errors.is_empty());
    f.set("H1", "r");
    let before = f.data();
    f.apply(vec![paste(
        s,
        "H1",
        &tok,
        PasteOptions {
            shift: Some(ShiftDir::Right),
            ..PasteOptions::default()
        },
    )]);
    assert_eq!(f.value("H1"), n(1.0));
    assert_eq!(f.value("I1"), Value::text("r"));
    f.svc.undo(f.doc).unwrap();
    assert_eq!(f.data(), before);
    // Up / left are refused.
    let res = f.apply(vec![paste(
        s,
        "H1",
        &tok,
        PasteOptions {
            shift: Some(ShiftDir::Up),
            ..PasteOptions::default()
        },
    )]);
    assert_eq!(res.errors.len(), 1);
}

#[test]
fn operations_combine_by_aoo_mix_rules() {
    let mut f = Fixture::new();
    let s = f.sheet;
    // Source A1:A5 = 2, empty, text, =A1*5, 3.
    f.set("A1", "2");
    f.set("A3", "t");
    f.set("A4", "=A1*5");
    f.set("A5", "3");
    let tok = copy(&mut f, s, "A1:A5");
    // Target C1:C5 = 10, 7, 4, 1, text.
    f.set("C1", "10");
    f.set("C2", "7");
    f.set("C3", "4");
    f.set("C4", "1");
    f.set("C5", "u");
    let mul = PasteOptions {
        operation: PasteOperation::Multiply,
        ..PasteOptions::default()
    };
    round_trip(&mut f, vec![paste(s, "C1", &tok, mul.clone())]);
    assert_eq!(f.value("C1"), n(20.0));
    assert_eq!(f.value("C2"), n(0.0), "an empty source counts as 0");
    assert_eq!(
        f.value("C3"),
        n(4.0),
        "text is never combined: the target stays"
    );
    assert_eq!(
        input_of(&f, s, "C4"),
        "=1*(C1*5)",
        "a pasted formula combines"
    );
    assert_eq!(f.value("C5"), Value::text("u"));
    // Skip empty: the empty source leaves the target.
    f.set("E2", "7");
    let opts = PasteOptions {
        skip_empty: true,
        ..mul.clone()
    };
    f.apply(vec![paste(s, "E1", &tok, opts)]);
    assert_eq!(f.value("E2"), n(7.0));
    assert_eq!(f.value("E1"), n(0.0), "empty target counts as 0");
    // Numbers only (formula results as values) + Add.
    f.set("G4", "1");
    let opts = PasteOptions {
        operation: PasteOperation::Add,
        ..sel_opts("v")
    };
    f.apply(vec![paste(s, "G1", &tok, opts)]);
    assert_eq!(input_of(&f, s, "G4"), "11");
    assert_eq!(
        f.value("G3"),
        Value::Empty,
        "text is not pasted with Numbers only"
    );
    // Division by zero: #VALUE! (AOO lcl_DoFunction → errNoValue).
    f.set("J1", "5");
    let div = PasteOptions {
        operation: PasteOperation::Divide,
        ..PasteOptions::default()
    };
    f.set("I1", "0");
    let tok0 = copy(&mut f, s, "I1");
    f.apply(vec![paste(s, "J1", &tok0, div)]);
    assert_eq!(f.value("J1"), Value::Error(ErrorKind::Value));
    // Link + operation: a formula on both sides.
    f.set("L1", "4");
    let opts = PasteOptions {
        link: Some(true),
        operation: PasteOperation::Add,
        ..PasteOptions::default()
    };
    f.apply(vec![paste(s, "L1", &tok0, opts)]);
    assert_eq!(input_of(&f, s, "L1"), "=4+($Sheet1.$I$1)");
}

#[test]
fn shift_and_link_in_a_group() {
    let mut f = Fixture::new();
    let s = f.sheet;
    f.apply(vec![Op::AddSheet {
        name: Some("S2".into()),
        index: None,
    }]);
    let s2 = f.svc.info(f.doc).unwrap().sheets[1].id;
    f.set("A1", "1");
    f.set("C1", "x");
    f.apply(vec![common::input(s2, "C1", "y")]);
    let tok = copy(&mut f, s, "A1");
    let mut t = tx(vec![paste(
        s,
        "C1",
        &tok,
        PasteOptions {
            link: Some(true),
            shift: Some(ShiftDir::Down),
            ..PasteOptions::default()
        },
    )]);
    t.group = Some(vec![s, s2]);
    let before = f.data();
    let res = f.svc.apply(f.doc, t).unwrap();
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    for (sh, moved) in [(s, "x"), (s2, "y")] {
        assert_eq!(input_of(&f, sh, "C1"), "=$Sheet1.$A$1");
        assert_eq!(f.value_on(sh, "C2"), Value::text(moved));
    }
    f.svc.undo(f.doc).unwrap();
    assert_eq!(f.data(), before, "one undo step for the group");
}

#[test]
fn protected_target_is_refused_whole() {
    let mut f = Fixture::new();
    let s = f.sheet;
    f.set("A1", "1");
    let tok = copy(&mut f, s, "A1");
    f.set("C1", "keep");
    f.apply(vec![Op::ProtectSheet {
        sheet: s,
        password: None,
        options: None,
    }]);
    let before = f.data();
    for opts in [
        sel_opts("v"),
        PasteOptions {
            link: Some(true),
            ..PasteOptions::default()
        },
        PasteOptions {
            shift: Some(ShiftDir::Down),
            ..PasteOptions::default()
        },
    ] {
        let res = f.apply(vec![paste(s, "C1", &tok, opts)]);
        assert_eq!(res.errors.len(), 1, "{:?}", res.errors);
        assert_eq!(f.data(), before);
    }
}

#[test]
fn old_payloads_deserialize() {
    // Options without the new fields (older front ends) still parse.
    let o: PasteOptions = serde_json::from_str(
        r#"{"valuesOnly":true,"formatsOnly":false,"transpose":false,"skipEmpty":false,"operation":"none"}"#,
    )
    .unwrap();
    assert!(o.selection.is_none() && o.link.is_none() && o.shift.is_none());
}

#[test]
fn notes_or_formats_only_are_refused_on_protected_cells() {
    // AOO tests the whole area whatever is pasted (viewfun3.cxx:1258).
    let mut f = Fixture::new();
    let s = f.sheet;
    f.set("A1", "1");
    note(&mut f, "A1", "n");
    bold(&mut f, "A1");
    let tok = copy(&mut f, s, "A1");
    f.apply(vec![Op::ProtectSheet {
        sheet: s,
        password: None,
        options: None,
    }]);
    let before = f.data();
    for spec in ["n", "t"] {
        let res = f.apply(vec![paste(s, "C1", &tok, sel_opts(spec))]);
        assert_eq!(res.errors.len(), 1, "{spec}: {:?}", res.errors);
        assert_eq!(res.errors[0].code, "protected");
        assert_eq!(f.data(), before);
    }
}

#[test]
fn mixed_formulas_keep_every_digit() {
    let mut f = Fixture::new();
    let s = f.sheet;
    f.set("A1", "=1/3");
    let tok = copy(&mut f, s, "A1");
    f.set("C1", "0.1");
    f.apply(vec![paste(
        s,
        "C1",
        &tok,
        PasteOptions {
            operation: PasteOperation::Add,
            ..PasteOptions::default()
        },
    )]);
    assert_eq!(input_of(&f, s, "C1"), "=0.1+(1/3)");
    f.set("E1", "=2");
    f.set("G1", "0.3333333333333333");
    let tok = copy(&mut f, s, "G1");
    f.apply(vec![paste(
        s,
        "E1",
        &tok,
        PasteOptions {
            operation: PasteOperation::Add,
            ..PasteOptions::default()
        },
    )]);
    assert_eq!(input_of(&f, s, "E1"), "=(2)+0.3333333333333333");
}

#[test]
fn a_larger_selection_is_tiled_and_cut() {
    // AOO CopyFromClip fills the selection with copies, cutting the last.
    let mut f = Fixture::new();
    let s = f.sheet;
    f.set("A1", "1");
    f.set("A2", "2");
    let tok = copy(&mut f, s, "A1:A2");
    f.set("C6", "below");
    round_trip(
        &mut f,
        vec![paste(s, "C1:C5", &tok, PasteOptions::default())],
    );
    let col: Vec<Value> = ["C1", "C2", "C3", "C4", "C5"]
        .iter()
        .map(|a| f.value(a))
        .collect();
    assert_eq!(col, vec![n(1.0), n(2.0), n(1.0), n(2.0), n(1.0)]);
    assert_eq!(f.value("C6"), Value::text("below"));
    // A smaller selection gets the whole block.
    f.apply(vec![paste(s, "E1", &tok, PasteOptions::default())]);
    assert_eq!(f.value("E2"), n(2.0));
    // Shift cells inserts the tiled area.
    f.apply(vec![paste(
        s,
        "C1:C3",
        &tok,
        PasteOptions {
            shift: Some(ShiftDir::Down),
            ..PasteOptions::default()
        },
    )]);
    assert_eq!(f.value("C3"), n(1.0));
    assert_eq!(f.value("C4"), n(1.0), "the old C1 moved down 3 rows");
    assert_eq!(f.value("C9"), Value::text("below"));
}

#[test]
fn overwrite_check_counts_notes() {
    let mut f = Fixture::new();
    let s = f.sheet;
    f.set("A1", "1");
    let tok = copy(&mut f, s, "A1");
    note(&mut f, "C1", "n");
    let hit = f
        .svc
        .paste_overwrites(
            f.doc,
            s,
            range("C1"),
            PastePayload::Internal { token: tok },
            PasteOptions::default(),
        )
        .unwrap();
    assert!(hit, "a note is data (AOO IsBlockEmpty, bIgnoreNotes off)");
}

#[test]
fn the_budget_counts_link_blocks_and_shifted_bands() {
    let mut f = Fixture::new();
    let s = f.sheet;
    f.set("A1", "1");
    for r in 1..=50 {
        f.set(&format!("C{r}"), "x");
    }
    let wb = &f.svc.document(f.doc).unwrap().wb;
    let r = tally_types::CellRange::from_a1(s, "A1:A1000").unwrap();
    let (snap, _, _) =
        tally_engine::clipboard::copy_range(wb, r, &mut tally_engine::workbook::Rng::new(1));
    let op = |options: PasteOptions| Op::PasteSpecial {
        sheet: s,
        target: range("C1"),
        payload: PastePayload::Internal {
            token: snap.token.clone(),
        },
        options,
    };
    let cost = |o| tally_engine::ops::op_cost(wb, &op(o), Some(&snap));
    let plain = cost(PasteOptions::default());
    // Link with Paste all writes 1000 formulas, though one cell is copied.
    let link = cost(PasteOptions {
        link: Some(true),
        ..PasteOptions::default()
    });
    assert_eq!(link, 1000);
    assert!(plain < link);
    // Shift cells also moves C1:C50.
    let shift = cost(PasteOptions {
        shift: Some(ShiftDir::Down),
        ..PasteOptions::default()
    });
    assert!(shift >= plain + 50, "{shift} vs {plain}");
}

#[test]
fn link_with_paste_all_counts_the_whole_block() {
    // An almost empty block linked over a large area costs every cell.
    let mut f = Fixture::new();
    let s = f.sheet;
    f.set("A1", "1");
    let tok = copy(&mut f, s, "A1:A1000");
    let res = f.apply(vec![paste(
        s,
        "C1:CV1000",
        &tok,
        PasteOptions {
            link: Some(true),
            ..PasteOptions::default()
        },
    )]);
    // 100 000 formulas fit; the cost check must not see 1 cell.
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    assert_eq!(input_of(&f, s, "CV1000"), "=$Sheet1.$A$1000");
}
