//! Multi-sheet selection (transaction `group`) and Edit ▸ Fill ▸ Sheets.
#![allow(clippy::unwrap_used)]

mod common;

use common::{input, range, sel};
use tally_api::{
    CfRuleDto, CfRuleKind, CfStyleDto, ClearWhat, CmpOp, DeleteContentsFlags, Direction, DocId,
    DocumentService, FillMode, FillSheetOptions, FindMode, FindQuery, ListSource, NoteDto, Op,
    PasteOperation, PasteOptions, PastePayload, SeriesKind, ShiftDir, StylePatch, Transaction,
    ValidationAlert, ValidationDto, ValidationErrorStyle, ValidationKind,
};
use tally_engine::group::{retarget, GROUP_OPS};
use tally_engine::EngineService;
use tally_model::{SheetData, WorkbookData};
use tally_types::{CellAddr, SheetId, Value};

fn gtx(ops: Vec<Op>, group: &[SheetId]) -> Transaction {
    Transaction {
        ops,
        label: Some("group".into()),
        selection_before: None,
        selection_after: None,
        group: Some(group.to_vec()),
    }
}

/// A workbook with `n` sheets ("Sheet1", "S2", "S3", …).
struct Doc {
    svc: EngineService,
    doc: DocId,
    ids: Vec<SheetId>,
}

impl Doc {
    fn new(n: usize) -> Self {
        let mut svc = EngineService::new();
        let info = svc.new_workbook(None).unwrap();
        let doc = info.doc;
        let adds: Vec<Op> = (2..=n)
            .map(|i| Op::AddSheet {
                name: Some(format!("S{i}")),
                index: None,
            })
            .collect();
        if !adds.is_empty() {
            svc.apply(doc, common::tx(adds)).unwrap();
        }
        let ids = svc.info(doc).unwrap().sheets.iter().map(|s| s.id).collect();
        Self { svc, doc, ids }
    }

    fn apply(&mut self, ops: Vec<Op>) -> tally_api::ApplyResult {
        self.svc.apply(self.doc, common::tx(ops)).unwrap()
    }

    fn group(&mut self, ops: Vec<Op>, group: &[SheetId]) -> tally_api::ApplyResult {
        self.svc.apply(self.doc, gtx(ops, group)).unwrap()
    }

    fn data(&self) -> WorkbookData {
        self.svc.document(self.doc).unwrap().wb.to_data()
    }

    fn value(&self, sheet: SheetId, a1: &str) -> Value {
        let a = CellAddr::from_a1(sheet, a1).unwrap();
        self.svc.document(self.doc).unwrap().wb.value(a)
    }

    fn num(&self, sheet: SheetId, a1: &str) -> f64 {
        match self.value(sheet, a1) {
            Value::Number(n) => n,
            v => panic!("{a1} on {sheet}: {v:?}"),
        }
    }

    fn formula(&self, sheet: SheetId, a1: &str) -> Option<String> {
        let a = CellAddr::from_a1(sheet, a1).unwrap();
        let wb = &self.svc.document(self.doc).unwrap().wb;
        let (id, _) = wb.formula_at(a)?;
        wb.formula_text(id)
    }

    fn can_undo(&self) -> bool {
        self.svc.document(self.doc).unwrap().undo.state().can_undo
    }

    fn bold(&self, sheet: SheetId, a1: &str) -> bool {
        let r = range(a1);
        let cells = self.svc.cells(self.doc, sheet, &[r]).unwrap();
        let Some(c) = cells[0].cells.iter().find(|c| (c.r, c.c) == (r.r0, r.c0)) else {
            return false;
        };
        self.svc.styles(self.doc, &[c.style]).unwrap()[0].bold
    }
}

/// A sheet's content without its name, for comparing sheets.
fn body(d: &WorkbookData, i: usize) -> SheetData {
    let mut s = d.sheets[i].clone();
    s.name = String::new();
    s
}

/// The same content on every sheet: numbers, text, a merge and rows too
/// low for their text (for AutoFit).
fn seed(d: &mut Doc) {
    for s in d.ids.clone() {
        d.apply(vec![
            input(s, "A1", "1"),
            input(s, "A2", "2"),
            input(s, "B1", "3"),
            input(s, "B2", "4"),
            input(s, "C1", "x"),
            input(s, "C2", "two\nlines"),
            Op::MergeCells {
                sheet: s,
                range: range("G1:H1"),
            },
            Op::SetRowHeight {
                sheet: s,
                start: 0,
                end: 4,
                px: 5.0,
            },
        ]);
    }
}

fn find(text: &str) -> FindQuery {
    FindQuery {
        text: text.into(),
        sheet: None,
        in_formulas: false,
        match_case: false,
        whole_cell: false,
        mode: FindMode::Literal,
        from: None,
        in_notes: None,
        selection: None,
    }
}

fn stop_list(a1: &str) -> ValidationDto {
    ValidationDto {
        id: 0,
        ranges: vec![range(a1)],
        kind: ValidationKind::List {
            source: ListSource::Values {
                items: vec!["Paid".into(), "Due".into()],
            },
        },
        allow_blank: true,
        show_dropdown: true,
        sort_ascending: false,
        input_help: None,
        error: Some(ValidationAlert {
            style: ValidationErrorStyle::Stop,
            title: "Oops".into(),
            text: "Not a status".into(),
        }),
    }
}

/// One op of every group kind, on sheet `s` (the clipboard token for the
/// pastes comes from `token`).
fn group_ops(s: SheetId, token: &str) -> Vec<Op> {
    let rs = |a1: &str| vec![range(a1)];
    vec![
        input(s, "D1", "=B1*2"),
        Op::SetRangeInputs {
            sheet: s,
            r: 5,
            c: 0,
            rows: vec![vec!["7".into(), "=A6+1".into()]],
        },
        Op::SetArrayFormula {
            sheet: s,
            range: range("E1:E2"),
            input: "=B1:B2*2".into(),
        },
        Op::Clear {
            sheet: s,
            ranges: rs("A1:A2"),
            what: ClearWhat::Contents,
        },
        Op::DeleteContents {
            sheet: s,
            ranges: rs("A1:B2"),
            what: DeleteContentsFlags {
                numbers: true,
                ..DeleteContentsFlags::default()
            },
        },
        Op::ApplyStyle {
            sheet: s,
            ranges: rs("A1:B2"),
            patch: StylePatch {
                bold: Some(true),
                ..StylePatch::default()
            },
        },
        Op::SetNumberFormat {
            sheet: s,
            ranges: rs("A1:A2"),
            code: "0.00".into(),
        },
        Op::ApplyCellStyle {
            sheet: s,
            ranges: rs("A1"),
            name: "Heading".into(),
        },
        Op::MergeCells {
            sheet: s,
            range: range("J1:K2"),
        },
        Op::UnmergeCells {
            sheet: s,
            range: range("G1:H1"),
        },
        Op::SetCellProtection {
            sheet: s,
            ranges: rs("A1:A3"),
            locked: Some(false),
            hidden_formula: None,
            hidden: None,
            hide_print: None,
        },
        Op::AddConditionalFormat {
            sheet: s,
            ranges: rs("A1:A5"),
            rules: vec![CfRuleDto {
                kind: CfRuleKind::CellValue {
                    op: CmpOp::Gt,
                    formulas: vec!["1".into()],
                },
                style: CfStyleDto {
                    bold: Some(true),
                    ..CfStyleDto::default()
                },
                stop_if_true: false,
            }],
        },
        Op::AddValidation {
            sheet: s,
            validation: stop_list("F1:F5"),
        },
        Op::InsertRows {
            sheet: s,
            at: 1,
            count: 2,
        },
        Op::DeleteRows {
            sheet: s,
            at: 0,
            count: 1,
        },
        Op::InsertCols {
            sheet: s,
            at: 1,
            count: 1,
        },
        Op::DeleteCols {
            sheet: s,
            at: 0,
            count: 1,
        },
        Op::InsertCells {
            sheet: s,
            range: range("A1:A2"),
            shift: ShiftDir::Down,
        },
        Op::DeleteCells {
            sheet: s,
            range: range("A1"),
            shift: ShiftDir::Left,
        },
        Op::SetColWidth {
            sheet: s,
            start: 0,
            end: 1,
            px: 150.0,
        },
        Op::SetRowHeight {
            sheet: s,
            start: 8,
            end: 9,
            px: 50.0,
        },
        Op::SetRowsHidden {
            sheet: s,
            start: 1,
            end: 1,
            hidden: true,
        },
        Op::SetColsHidden {
            sheet: s,
            start: 2,
            end: 2,
            hidden: true,
        },
        Op::AutoFitRows {
            sheet: s,
            start: 0,
            end: 4,
        },
        Op::ApplyAutoFormat {
            sheet: s,
            range: range("H1:J3"),
            preset: "Blue".into(),
            options: tally_api::AutoFormatOptions::default(),
        },
        Op::ApplyAutoFormatCustom {
            sheet: s,
            range: range("H5:J7"),
            cells: vec![patch_fill(0xff00_00ff); 16],
            autofit: false,
        },
        Op::Paste {
            sheet: s,
            target: range("M1"),
            payload: PastePayload::Internal {
                token: token.into(),
            },
        },
        Op::PasteSpecial {
            sheet: s,
            target: range("M5"),
            payload: PastePayload::Internal {
                token: token.into(),
            },
            options: PasteOptions {
                values_only: true,
                ..PasteOptions::default()
            },
        },
        Op::FillRange {
            sheet: s,
            source: range("A1:A2"),
            target: range("A1:A6"),
            mode: FillMode::Auto,
        },
        Op::FillSeries {
            sheet: s,
            range: range("P1:P5"),
            direction: Direction::Down,
            kind: SeriesKind::Linear,
            unit: None,
            start: Some("10".into()),
            end: None,
            increment: Some("5".into()),
        },
        Op::ReplaceAll {
            sheet: Some(s),
            query: find("x"),
            replace: "y".into(),
        },
    ]
}

fn tag(op: &Op) -> String {
    serde_json::to_value(op).unwrap()["type"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[test]
fn the_group_list_names_every_group_op() {
    let ops = group_ops(1, "t");
    let tags: Vec<String> = ops.iter().map(tag).collect();
    assert_eq!(tags, GROUP_OPS, "GROUP_OPS and the test ops differ");
    for op in &ops {
        assert!(retarget(op, 7).is_some(), "{op:?} is a group op");
    }
    // Replace All over every sheet and sheet commands are not.
    let all = Op::ReplaceAll {
        sheet: None,
        query: find("x"),
        replace: "y".into(),
    };
    assert!(retarget(&all, 7).is_none());
    assert!(retarget(&Op::DeleteSheet { sheet: 1 }, 7).is_none());
    assert!(retarget(
        &Op::SetTabColor {
            sheet: 1,
            color: None
        },
        7
    )
    .is_none());
}

#[test]
fn every_group_op_runs_on_each_sheet_with_one_undo_step() {
    let probe = Doc::new(1);
    let n = group_ops(probe.ids[0], "t").len();
    for k in 0..n {
        let mut d = Doc::new(3);
        seed(&mut d);
        let ids = d.ids.clone();
        let clip = d.svc.copy(d.doc, &sel(ids[0], "A1:B2")).unwrap();
        let op = group_ops(ids[0], &clip.token).swap_remove(k);
        let before = d.data();
        let res = d.group(vec![op.clone()], &ids);
        assert!(res.errors.is_empty(), "{op:?}: {:?}", res.errors);
        let after = d.data();
        for i in 0..3 {
            assert_ne!(body(&before, i), body(&after, i), "{op:?} on sheet {i}");
        }
        assert_eq!(body(&after, 0), body(&after, 1), "{op:?}: sheets 1 and 2");
        assert_eq!(body(&after, 0), body(&after, 2), "{op:?}: sheets 1 and 3");
        d.svc.undo(d.doc).unwrap();
        assert_eq!(d.data(), before, "one undo reverts {op:?} everywhere");
        assert!(!d.can_undo() || d.data() == before);
        d.svc.redo(d.doc).unwrap();
        assert_eq!(d.data(), after, "one redo of {op:?}");
    }
}

#[test]
fn group_ops_on_other_sheets_and_one_sheet_groups_run_once() {
    let mut d = Doc::new(3);
    let ids = d.ids.clone();
    // The op is on the second sheet, not the active (first) one.
    let res = d.group(vec![input(ids[1], "A1", "5")], &ids);
    assert!(res.errors.is_empty());
    assert_eq!(d.value(ids[0], "A1"), Value::Empty);
    assert_eq!(d.num(ids[1], "A1"), 5.0);
    assert_eq!(d.value(ids[2], "A1"), Value::Empty);
    // A group of one is no group.
    d.group(vec![input(ids[0], "B1", "6")], &ids[..1]);
    assert_eq!(d.value(ids[1], "B1"), Value::Empty);
}

#[test]
fn formulas_are_parsed_per_sheet_and_3d_refs_stay() {
    let mut d = Doc::new(4);
    let ids = d.ids.clone();
    d.apply(vec![Op::RenameSheet {
        sheet: ids[3],
        name: "Sheet9".into(),
    }]);
    d.apply(vec![
        input(ids[0], "A1", "1"),
        input(ids[1], "A1", "2"),
        input(ids[2], "A1", "3"),
        input(ids[3], "B2", "100"),
    ]);
    let g = &ids[..3];
    let res = d.group(vec![input(ids[0], "C3", "=A1+Sheet9.B2")], g);
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    for (i, s) in g.iter().enumerate() {
        assert_eq!(d.num(*s, "C3"), 101.0 + i as f64);
        assert_eq!(d.formula(*s, "C3").as_deref(), Some("=A1+Sheet9.B2"));
    }
    assert_eq!(d.value(ids[3], "C3"), Value::Empty);
}

#[test]
fn hidden_sheets_are_left_out() {
    let mut d = Doc::new(3);
    let ids = d.ids.clone();
    d.apply(vec![Op::SetSheetHidden {
        sheet: ids[1],
        hidden: true,
    }]);
    d.group(vec![input(ids[0], "A1", "1")], &ids);
    assert_eq!(d.value(ids[1], "A1"), Value::Empty);
    assert_eq!(d.num(ids[2], "A1"), 1.0);
}

#[test]
fn a_protected_sheet_refuses_the_whole_group() {
    let mut d = Doc::new(3);
    let ids = d.ids.clone();
    d.apply(vec![input(ids[0], "Z9", "keep")]);
    d.apply(vec![Op::ProtectSheet {
        sheet: ids[2],
        password: None,
        options: None,
    }]);
    let before = d.data();
    let res = d.group(
        vec![
            Op::SetColWidth {
                sheet: ids[0],
                start: 0,
                end: 0,
                px: 200.0,
            },
            input(ids[0], "A1", "1"),
        ],
        &ids,
    );
    assert_eq!(res.errors.len(), 1, "{:?}", res.errors);
    assert_eq!(res.errors[0].code, "protected");
    assert_eq!(res.errors[0].op_index, 0, "the original op index");
    assert_eq!(d.data(), before, "nothing changed");
    assert!(!d.can_undo(), "no undo step");
}

#[test]
fn a_validation_failure_on_one_sheet_reverts_all() {
    let mut d = Doc::new(3);
    let ids = d.ids.clone();
    d.apply(vec![Op::AddValidation {
        sheet: ids[1],
        validation: stop_list("A1:A10"),
    }]);
    let undo_before = d.can_undo();
    let before = d.data();
    let res = d.group(
        vec![input(ids[0], "B1", "ok"), input(ids[0], "A1", "nope")],
        &ids,
    );
    assert_eq!(res.errors.len(), 1, "{:?}", res.errors);
    assert_eq!(res.errors[0].op_index, 1);
    assert_eq!(d.data(), before);
    assert_eq!(d.can_undo(), undo_before);
    // Without a group, the same ops keep what succeeded.
    let res = d.apply(vec![input(ids[1], "B1", "ok"), input(ids[1], "A1", "nope")]);
    assert_eq!(res.errors.len(), 1);
    assert_eq!(d.value(ids[1], "B1"), Value::Text("ok".into()));
}

#[test]
fn the_cell_budget_counts_every_sheet() {
    let mut d = Doc::new(3);
    let ids = d.ids.clone();
    // A paste into a whole column counts 1,048,576 cells (and writes one);
    // 17 of them fit the 50,000,000 budget once, not three times.
    let mut ops = vec![input(ids[0], "B1", "1")];
    ops.extend((0..17).map(|_| Op::Paste {
        sheet: ids[0],
        target: range("A1:A1048576"),
        payload: PastePayload::Tsv { text: "7".into() },
    }));
    let before = d.data();
    let res = d.group(ops.clone(), &ids);
    assert_eq!(res.errors.len(), 1, "{:?}", res.errors);
    assert_eq!(res.errors[0].code, "unsupported");
    assert_eq!(res.errors[0].op_index, 16);
    assert_eq!(d.data(), before);
    let res = d.group(ops, &ids[..1]);
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    assert_eq!(d.num(ids[0], "B1"), 1.0);
}

#[test]
fn other_ops_with_a_group_run_once() {
    let mut d = Doc::new(3);
    let ids = d.ids.clone();
    let res = d.group(
        vec![
            Op::SetTabColor {
                sheet: ids[0],
                color: Some(0xff00_00ff),
            },
            Op::SetNote {
                sheet: ids[0],
                r: 0,
                c: 0,
                note: NoteDto {
                    r: 0,
                    c: 0,
                    text: "hi".into(),
                    author: None,
                    created: None,
                    shown: false,
                },
            },
        ],
        &ids,
    );
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let data = d.data();
    assert_eq!(data.sheets[0].tab_color, Some(0xff00_00ff));
    assert_eq!(data.sheets[1].tab_color, None);
    assert_eq!(data.sheets[0].notes.len(), 1);
    assert!(data.sheets[1].notes.is_empty() && data.sheets[2].notes.is_empty());
}

#[test]
fn unknown_group_sheets_are_refused() {
    let mut d = Doc::new(2);
    let ids = d.ids.clone();
    let before = d.data();
    let res = d.group(vec![input(ids[0], "A1", "1")], &[ids[0], 999]);
    assert_eq!(res.errors[0].code, "not_found");
    assert_eq!(d.data(), before);
}

#[test]
fn sheet_commands_over_a_group() {
    // Deleting three of four sheets: one step.
    let mut d = Doc::new(4);
    let ids = d.ids.clone();
    let before = d.data();
    let g = &ids[..3];
    let ops = g.iter().map(|s| Op::DeleteSheet { sheet: *s }).collect();
    let res = d.group(ops, g);
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    assert_eq!(d.data().sheets.len(), 1);
    d.svc.undo(d.doc).unwrap();
    assert_eq!(d.data(), before);

    // Deleting every sheet fails with nothing deleted.
    let mut d = Doc::new(3);
    let ids = d.ids.clone();
    let before = d.data();
    let ops = ids.iter().map(|s| Op::DeleteSheet { sheet: *s }).collect();
    let res = d.group(ops, &ids);
    assert_eq!(res.errors.len(), 1, "{:?}", res.errors);
    assert_eq!(res.errors[0].op_index, 2);
    assert_eq!(d.data(), before);

    // Hiding every visible sheet fails likewise.
    let ops = ids
        .iter()
        .map(|s| Op::SetSheetHidden {
            sheet: *s,
            hidden: true,
        })
        .collect();
    let res = d.group(ops, &ids);
    assert_eq!(res.errors.len(), 1, "{:?}", res.errors);
    assert_eq!(d.data(), before);
    assert_eq!(d.svc.document(d.doc).unwrap().wb.active_sheet, ids[0]);

    // Tab colours of the group, then copies and a move in one step.
    let ops = ids
        .iter()
        .map(|s| Op::SetTabColor {
            sheet: *s,
            color: Some(0xff),
        })
        .collect();
    assert!(d.group(ops, &ids).errors.is_empty());
    assert!(d.data().sheets.iter().all(|s| s.tab_color == Some(0xff)));
    let before = d.data();
    let res = d.group(
        vec![
            Op::CopySheet {
                sheet: ids[0],
                name: None,
                index: Some(3),
            },
            Op::CopySheet {
                sheet: ids[1],
                name: None,
                index: Some(4),
            },
            Op::MoveSheet {
                sheet: ids[0],
                to: 2,
            },
        ],
        &ids[..2],
    );
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let after = d.data();
    assert_eq!(after.sheets.len(), 5);
    assert_eq!(after.sheets[2].name, "Sheet1");
    d.svc.undo(d.doc).unwrap();
    assert_eq!(d.data(), before);
    d.svc.redo(d.doc).unwrap();
    assert_eq!(d.data(), after);

    // A failing move after a copy undoes the copy.
    let res = d.group(
        vec![
            Op::CopySheet {
                sheet: ids[0],
                name: None,
                index: None,
            },
            Op::MoveSheet { sheet: 999, to: 0 },
        ],
        &ids[..2],
    );
    assert_eq!(res.errors.len(), 1);
    assert_eq!(res.created, None);
    assert_eq!(d.data(), after);
}

#[test]
fn transactions_without_group_still_parse() {
    let tx: Transaction = serde_json::from_str(
        r#"{"ops":[],"label":null,"selectionBefore":null,"selectionAfter":null}"#,
    )
    .unwrap();
    assert_eq!(tx.group, None);
}

// ----- Fill Sheets ------------------------------------------------------------

/// Source sheet: A1 = 1, A2 = "t", A3 = a date, A4 = =A1*2, a note and
/// bold on A1, B1 empty. Targets: A1 = 9, A2 = "old", A3 = 5, A4 = 6,
/// B1 = 7.
fn fill_doc() -> Doc {
    let mut d = Doc::new(3);
    let ids = d.ids.clone();
    d.apply(vec![
        input(ids[0], "A1", "1"),
        input(ids[0], "A2", "t"),
        input(ids[0], "A3", "2024-01-15"),
        input(ids[0], "A4", "=A1*2"),
        Op::SetNote {
            sheet: ids[0],
            r: 0,
            c: 0,
            note: NoteDto {
                r: 0,
                c: 0,
                text: "src".into(),
                author: None,
                created: None,
                shown: false,
            },
        },
        Op::ApplyStyle {
            sheet: ids[0],
            ranges: vec![range("A1")],
            patch: StylePatch {
                bold: Some(true),
                ..StylePatch::default()
            },
        },
    ]);
    for s in &ids[1..] {
        d.apply(vec![
            input(*s, "A1", "9"),
            input(*s, "A2", "old"),
            input(*s, "A3", "5"),
            input(*s, "A4", "6"),
            input(*s, "B1", "7"),
        ]);
    }
    d
}

fn fill(d: &mut Doc, o: FillSheetOptions) -> tally_api::ApplyResult {
    let ids = d.ids.clone();
    d.apply(vec![Op::FillSheets {
        sheet: ids[0],
        ranges: vec![range("A1:B4")],
        to: ids[1..].to_vec(),
        options: o,
    }])
}

#[test]
fn fill_sheets_copies_each_kind() {
    let date_serial = {
        let d = fill_doc();
        d.num(d.ids[0], "A3")
    };
    let none = FillSheetOptions::default();
    // Numbers only: A1 replaced, B1 (source empty) removed, the rest kept.
    let mut d = fill_doc();
    let before = d.data();
    let res = fill(
        &mut d,
        FillSheetOptions {
            numbers: true,
            ..none.clone()
        },
    );
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    for s in d.ids[1..].iter().copied() {
        assert_eq!(d.num(s, "A1"), 1.0);
        assert_eq!(d.value(s, "A2"), Value::Text("old".into()));
        // The target's plain number goes; the source's date is not copied.
        assert_eq!(d.value(s, "A3"), Value::Empty);
        assert_eq!(d.value(s, "B1"), Value::Empty);
        assert!(!d.bold(s, "A1"));
    }
    d.svc.undo(d.doc).unwrap();
    assert_eq!(d.data(), before, "one undo step");

    // Text only.
    let mut d = fill_doc();
    fill(
        &mut d,
        FillSheetOptions {
            text: true,
            ..none.clone()
        },
    );
    let t = d.ids[2];
    assert_eq!(d.value(t, "A2"), Value::Text("t".into()));
    assert_eq!(d.num(t, "A1"), 9.0);

    // Dates and times only.
    let mut d = fill_doc();
    fill(
        &mut d,
        FillSheetOptions {
            date_time: true,
            ..none.clone()
        },
    );
    assert_eq!(d.num(t, "A3"), date_serial);
    assert_eq!(d.num(t, "A1"), 9.0);

    // Formulas only: the relative reference reads the target's A1.
    let mut d = fill_doc();
    fill(
        &mut d,
        FillSheetOptions {
            formulas: true,
            ..none.clone()
        },
    );
    assert_eq!(d.formula(t, "A4").as_deref(), Some("=A1*2"));
    assert_eq!(d.num(t, "A4"), 18.0);

    // Notes only.
    let mut d = fill_doc();
    fill(
        &mut d,
        FillSheetOptions {
            notes: true,
            ..none.clone()
        },
    );
    let notes = &d.data().sheets[2].notes;
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].text, "src");
    assert_eq!(d.num(t, "A1"), 9.0);

    // Formats only.
    let mut d = fill_doc();
    fill(
        &mut d,
        FillSheetOptions {
            formats: true,
            ..none.clone()
        },
    );
    assert!(d.bold(t, "A1"));
    assert_eq!(d.num(t, "A1"), 9.0);
}

#[test]
fn fill_sheets_operations_and_skip_empty() {
    let all = FillSheetOptions {
        text: true,
        numbers: true,
        date_time: true,
        formulas: true,
        notes: true,
        formats: true,
        operation: PasteOperation::None,
        skip_empty: false,
    };
    // Everything, replacing: empty B1 clears the target's B1.
    let mut d = fill_doc();
    let t = d.ids[1];
    assert!(fill(&mut d, all.clone()).errors.is_empty());
    assert_eq!(d.value(t, "B1"), Value::Empty);
    assert_eq!(d.value(t, "A2"), Value::Text("t".into()));
    assert!(d.bold(t, "A1"));
    // Skip empty cells keeps B1.
    let mut d = fill_doc();
    fill(
        &mut d,
        FillSheetOptions {
            skip_empty: true,
            ..all.clone()
        },
    );
    assert_eq!(d.num(t, "B1"), 7.0);
    assert_eq!(d.num(t, "A1"), 1.0);
    // Operations combine numbers with the target's, as AOO's MixData
    // does: an empty source counts as 0 (Multiply gives 0, Divide #VALUE!).
    for (op, want, b1) in [
        (PasteOperation::Add, 10.0, Value::Number(7.0)),
        (PasteOperation::Subtract, 8.0, Value::Number(7.0)),
        (PasteOperation::Multiply, 9.0, Value::Number(0.0)),
        (
            PasteOperation::Divide,
            9.0,
            Value::Error(tally_types::ErrorKind::Value),
        ),
    ] {
        let mut d = fill_doc();
        let res = fill(
            &mut d,
            FillSheetOptions {
                numbers: true,
                operation: op,
                ..FillSheetOptions::default()
            },
        );
        assert!(res.errors.is_empty(), "{op:?}: {:?}", res.errors);
        assert_eq!(d.num(t, "A1"), want, "{op:?}");
        assert_eq!(d.value(t, "B1"), b1, "{op:?} with an empty source");
    }
}

#[test]
fn fill_sheets_refuses_a_protected_target() {
    let mut d = fill_doc();
    let ids = d.ids.clone();
    d.apply(vec![Op::ProtectSheet {
        sheet: ids[2],
        password: None,
        options: None,
    }]);
    let before = d.data();
    let res = fill(
        &mut d,
        FillSheetOptions {
            numbers: true,
            ..FillSheetOptions::default()
        },
    );
    assert_eq!(res.errors[0].code, "protected");
    assert_eq!(d.data(), before);
}

// ----- review fixes -------------------------------------------------------------

#[test]
fn a_reverted_protection_change_keeps_undo_and_dirty() {
    let mut d = Doc::new(2);
    let ids = d.ids.clone();
    d.apply(vec![input(ids[0], "B1", "1")]);
    d.svc.document_mut(d.doc).unwrap().dirty = false;
    let before = d.data();
    let res = d.group(
        vec![
            Op::ProtectSheet {
                sheet: ids[0],
                password: None,
                options: None,
            },
            input(ids[0], "A1", "2"),
        ],
        &ids,
    );
    assert_eq!(res.errors.len(), 1, "{:?}", res.errors);
    assert_eq!(res.errors[0].code, "protected");
    assert_eq!(d.data(), before);
    let doc = d.svc.document(d.doc).unwrap();
    assert!(doc.undo.state().can_undo, "the undo history survives");
    assert!(!doc.dirty, "dirty is unchanged");
}

fn sheet_range_doc() -> Doc {
    // Sheet1, S2, S3, S4; S4.A1 sums A5 over Sheet1..S3; a name too.
    let mut d = Doc::new(4);
    let ids = d.ids.clone();
    let mut ops: Vec<Op> = ids[..3].iter().map(|s| input(*s, "A5", "1")).collect();
    ops.extend(ids[..3].iter().map(|s| input(*s, "A6", "10")));
    ops.push(input(ids[3], "A1", "=SUM(Sheet1.A5:S3.A5)"));
    ops.push(Op::DefineName {
        name: "Band".into(),
        scope: None,
        refers_to: "$Sheet1.$A$5:$S3.$A$5".into(),
    });
    ops.push(input(ids[3], "A2", "=SUM(Band)"));
    let res = d.apply(ops);
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    d
}

/// The definition of the name `Band`.
fn band(d: &Doc) -> String {
    d.svc
        .names(d.doc)
        .unwrap()
        .into_iter()
        .find(|n| n.name == "Band")
        .unwrap()
        .refers_to
}

fn insert_row(s: SheetId) -> Op {
    Op::InsertRows {
        sheet: s,
        at: 0,
        count: 1,
    }
}

#[test]
fn sheet_range_references_follow_aoo() {
    // One sheet's insert leaves a reference over several sheets alone
    // (AOO ScRefUpdate::Update: the changed sheets must contain it).
    let mut d = sheet_range_doc();
    let ids = d.ids.clone();
    d.apply(vec![insert_row(ids[0])]);
    assert!(
        d.formula(ids[3], "A1").unwrap().contains("A5:"),
        "{:?}",
        d.formula(ids[3], "A1")
    );

    // A grouped insert on all three sheets moves it once (A5 → A6).
    let mut d = sheet_range_doc();
    let before = d.data();
    let res = d.group(vec![insert_row(ids[0])], &ids[..3]);
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let f = d.formula(ids[3], "A1").unwrap();
    assert!(f.contains("A6:") && f.ends_with("A6)"), "{f}");
    assert!(
        band(&d).ends_with("$A$6"),
        "the name moved once too: {}",
        band(&d)
    );
    let after = d.data();
    d.svc.undo(d.doc).unwrap();
    assert_eq!(d.data(), before);
    d.svc.redo(d.doc).unwrap();
    assert_eq!(d.data(), after);

    // A group without S3 does not cover Sheet1..S3: no move.
    let mut d = sheet_range_doc();
    d.group(vec![insert_row(ids[0])], &ids[..2]);
    assert!(d.formula(ids[3], "A1").unwrap().contains("A5:"));

    // Grouped deletes shrink or move once as well.
    let mut d = sheet_range_doc();
    d.group(
        vec![Op::DeleteRows {
            sheet: ids[0],
            at: 0,
            count: 2,
        }],
        &ids[..3],
    );
    let f = d.formula(ids[3], "A1").unwrap();
    assert!(f.contains("A3:") && f.ends_with("A3)"), "{f}");

    // A whole-width cell insert is a row insert.
    let mut d = sheet_range_doc();
    d.group(
        vec![Op::InsertCells {
            sheet: ids[0],
            range: tally_api::RangeDto {
                r0: 0,
                c0: 0,
                r1: 0,
                c1: tally_types::MAX_COLS - 1,
            },
            shift: ShiftDir::Down,
        }],
        &ids[..3],
    );
    assert!(d.formula(ids[3], "A1").unwrap().ends_with("A6)"));
}

#[test]
fn paste_costs_the_whole_block() {
    let mut d = Doc::new(3);
    let ids = d.ids.clone();
    let ops = (1..=100)
        .map(|i| input(ids[0], &format!("A{i}"), "1"))
        .collect();
    d.apply(ops);
    let clip = d.svc.copy(d.doc, &sel(ids[0], "A1:A100")).unwrap();
    let paste = |s: SheetId| Op::Paste {
        sheet: s,
        target: range("C1"),
        payload: PastePayload::Internal {
            token: clip.token.clone(),
        },
    };
    let doc = d.svc.document(d.doc).unwrap();
    let (snap, _, _) = tally_engine::clipboard::copy_range(
        &doc.wb,
        tally_types::CellRange::new(ids[0], 0, 0, 99, 0),
        &mut tally_engine::workbook::Rng::new(1),
    );
    let local = Op::Paste {
        sheet: ids[0],
        target: range("C1"),
        payload: PastePayload::Internal {
            token: snap.token.clone(),
        },
    };
    assert_eq!(
        tally_engine::ops::op_cost(&doc.wb, &local, Some(&snap)),
        100
    );
    let tsv = Op::Paste {
        sheet: ids[0],
        target: range("C1"),
        payload: PastePayload::Tsv {
            text: "1\t2\t3\n4\t5\n6".into(),
        },
    };
    assert_eq!(tally_engine::ops::op_cost(&doc.wb, &tsv, None), 9);
    // Grouped, every sheet receives the whole block.
    let res = d.group(vec![paste(ids[0])], &ids);
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    for s in &ids {
        assert_eq!(d.num(*s, "C100"), 1.0);
    }
}

#[test]
fn a_single_replace_stays_on_the_active_sheet() {
    let mut d = Doc::new(3);
    let ids = d.ids.clone();
    for s in &ids {
        d.apply(vec![input(*s, "A1", "x"), input(*s, "A2", "x")]);
    }
    let replace = |selection: Vec<tally_api::RangeDto>| Op::ReplaceAll {
        sheet: Some(ids[0]),
        query: FindQuery {
            sheet: Some(ids[0]),
            selection: Some(selection),
            ..find("x")
        },
        replace: "y".into(),
    };
    assert!(tally_engine::group::group_sheet(&replace(vec![range("A1")])).is_none());
    d.group(vec![replace(vec![range("A1")])], &ids);
    assert_eq!(d.value(ids[0], "A1"), Value::Text("y".into()));
    assert_eq!(d.value(ids[1], "A1"), Value::Text("x".into()));
    // A selection of several cells is a Replace All on each sheet.
    d.group(vec![replace(vec![range("A1:A2")])], &ids);
    assert_eq!(d.value(ids[2], "A2"), Value::Text("y".into()));
}

#[test]
fn replace_all_over_a_large_group_fits_the_budget() {
    let mut d = Doc::new(60);
    let ids = d.ids.clone();
    for s in &ids {
        d.apply(vec![input(*s, "A1", "x")]);
    }
    let res = d.group(
        vec![Op::ReplaceAll {
            sheet: Some(ids[0]),
            query: find("x"),
            replace: "y".into(),
        }],
        &ids,
    );
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    assert_eq!(d.value(ids[59], "A1"), Value::Text("y".into()));
}

#[test]
fn fill_sheets_targets_and_whole_columns() {
    let mut d = fill_doc();
    let ids = d.ids.clone();
    d.apply(vec![Op::SetSheetHidden {
        sheet: ids[2],
        hidden: true,
    }]);
    let numbers = FillSheetOptions {
        numbers: true,
        ..FillSheetOptions::default()
    };
    let op = |to: Vec<SheetId>, cols: &str| Op::FillSheets {
        sheet: ids[0],
        ranges: vec![range(cols)],
        to,
        options: numbers.clone(),
    };
    // The source, a repeat and a hidden sheet are not targets, nor costed.
    let wb = &d.svc.document(d.doc).unwrap().wb;
    let one = tally_engine::ops::op_cost(wb, &op(vec![ids[1]], "A1:B4"), None);
    let many =
        tally_engine::ops::op_cost(wb, &op(vec![ids[0], ids[1], ids[1], ids[2]], "A1:B4"), None);
    assert_eq!(one, many);
    // Whole columns cost (and touch) the used area only.
    let whole = tally_engine::ops::op_cost(wb, &op(vec![ids[1]], "A1:B1048576"), None);
    assert!(whole < 100, "{whole}");
    let res = d.apply(vec![op(
        vec![ids[0], ids[1], ids[1], ids[2]],
        "A1:B1048576",
    )]);
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    assert_eq!(d.num(ids[1], "A1"), 1.0);
    assert_eq!(d.num(ids[2], "A1"), 9.0, "hidden sheets are left alone");
}

#[test]
fn ops_run_in_order_after_the_group_copies() {
    // A cut-paste sent with a group: the paste runs on every group sheet,
    // then the clear of the cut cells runs once, on the clip's sheet, and
    // wipes what the paste put there. The UI leaves moves ungrouped.
    let mut d = Doc::new(2);
    let ids = d.ids.clone();
    d.apply(vec![input(ids[1], "A1", "5")]);
    let clip = d.svc.copy(d.doc, &sel(ids[1], "A1")).unwrap();
    let res = d.group(
        vec![
            Op::Paste {
                sheet: ids[0],
                target: range("A1"),
                payload: PastePayload::Internal { token: clip.token },
            },
            Op::Clear {
                sheet: ids[1],
                ranges: vec![range("A1")],
                what: ClearWhat::All,
            },
        ],
        &ids,
    );
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    assert_eq!(d.num(ids[0], "A1"), 5.0);
    assert_eq!(d.value(ids[1], "A1"), Value::Empty);
}

fn patch_fill(fill: u32) -> tally_api::StylePatch {
    let mut p: tally_api::StylePatch = serde_json::from_str(
        r#"{"bold":null,"italic":null,"underline":null,"strike":null,"fontFamily":null,"fontSizePt":null,"color":null,"fill":null,"hAlign":null,"vAlign":null,"wrap":null,"numberFormat":null}"#,
    )
    .unwrap();
    p.fill = Some(fill);
    p
}
