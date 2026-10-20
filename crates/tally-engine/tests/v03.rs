//! v0.3 features through the `DocumentService`: conditional formatting,
//! data validation, notes, links, protection, page setup, document
//! properties, row auto-height, locales and goal seek. Every op is checked
//! for undo/redo with `to_data()` snapshots.
#![allow(clippy::unwrap_used)]

mod common;

use common::{input, range, Fixture};
use tally_api::{
    cell_flags, BreakAxis, CellPos, CellView, CfColorStop, CfRuleDto, CfRuleKind, CfStyleDto,
    CfThreshold, CfValueKind, CmpOp, DocPropertiesDto, DocumentService, GoalSeekRequest,
    IconSetKind, ListSource, NoteDto, Op, PageScale, Paper, ProtectionOptions, StylePatch,
    ValidationAlert, ValidationDto, ValidationErrorStyle, ValidationKind, ValidationMessage,
};
use tally_types::{CellAddr, Value};

/// Applies `ops` and checks that the result cannot be undone (protection
/// changes clear the undo stacks; security review 3, LOW-9).
fn not_undoable(f: &mut Fixture, ops: Vec<Op>) {
    let res = f.apply(ops.clone());
    assert!(res.errors.is_empty(), "{ops:?}: {:?}", res.errors);
    assert!(!res.undo.can_undo, "{ops:?} must clear the undo stack");
    assert!(!res.undo.can_redo, "{ops:?} must clear the redo stack");
    let after = f.data();
    f.svc.undo(f.doc).unwrap();
    assert_eq!(f.data(), after, "undo after {ops:?} changes nothing");
}

fn round_trip(f: &mut Fixture, ops: Vec<Op>) {
    let before = f.data();
    let res = f.apply(ops.clone());
    assert!(res.errors.is_empty(), "{ops:?}: {:?}", res.errors);
    let after = f.data();
    assert_ne!(before, after, "{ops:?} changed nothing");
    f.svc.undo(f.doc).unwrap();
    assert_eq!(f.data(), before, "undo of {ops:?}");
    f.svc.redo(f.doc).unwrap();
    assert_eq!(f.data(), after, "redo of {ops:?}");
}

fn view(f: &Fixture, a1: &str) -> Option<CellView> {
    let a = CellAddr::from_a1(f.sheet, a1).unwrap();
    let r = range(a1);
    f.svc.cells(f.doc, f.sheet, &[r]).unwrap()[0]
        .cells
        .iter()
        .find(|c| (c.r, c.c) == (a.row, a.col))
        .cloned()
}

fn rule(kind: CfRuleKind, style: CfStyleDto) -> CfRuleDto {
    CfRuleDto {
        kind,
        style,
        stop_if_true: false,
    }
}

fn bold() -> CfStyleDto {
    CfStyleDto {
        bold: Some(true),
        ..CfStyleDto::default()
    }
}

fn red_fill() -> CfStyleDto {
    CfStyleDto {
        fill: Some(0xff00_00ff),
        ..CfStyleDto::default()
    }
}

fn add_cf(f: &mut Fixture, a1: &str, rules: Vec<CfRuleDto>) {
    let s = f.sheet;
    round_trip(
        f,
        vec![Op::AddConditionalFormat {
            sheet: s,
            ranges: vec![range(a1)],
            rules,
        }],
    );
}

fn cf_bold(f: &Fixture, a1: &str) -> bool {
    let Some(v) = view(f, a1) else {
        return false;
    };
    let Some(id) = v.cf_style else {
        return false;
    };
    f.svc.styles(f.doc, &[id]).unwrap()[0].bold
}

// ----- a. conditional formatting ---------------------------------------------

#[test]
fn cell_value_rules_relocate_per_cell() {
    let mut f = Fixture::new();
    for (i, v) in [1, 5, 10].iter().enumerate() {
        f.set(&format!("A{}", i + 1), &v.to_string());
        f.set(&format!("B{}", i + 1), "4");
    }
    // A1 > B1, written for the top-left cell, applies per row.
    add_cf(
        &mut f,
        "A1:A3",
        vec![rule(
            CfRuleKind::CellValue {
                op: CmpOp::Gt,
                formulas: vec!["B1".into()],
            },
            bold(),
        )],
    );
    assert!(!cf_bold(&f, "A1"));
    assert!(cf_bold(&f, "A2"));
    assert!(cf_bold(&f, "A3"));
    // Re-evaluated when a precedent changes.
    f.set("B3", "20");
    assert!(!cf_bold(&f, "A3"));
    let cfs = f.svc.conditional_formats(f.doc, f.sheet).unwrap();
    assert_eq!(cfs.len(), 1);
    match &cfs[0].rules[0].kind {
        CfRuleKind::CellValue { formulas, .. } => assert_eq!(formulas, &["B1".to_string()]),
        other => panic!("{other:?}"),
    }
    // The synthesized style keeps the cell's own style.
    let s = f.sheet;
    f.apply(vec![Op::ApplyStyle {
        sheet: s,
        ranges: vec![range("A2")],
        patch: StylePatch {
            italic: Some(true),
            ..StylePatch::default()
        },
    }]);
    let v = view(&f, "A2").unwrap();
    let st = &f.svc.styles(f.doc, &[v.cf_style.unwrap()]).unwrap()[0];
    assert!(st.bold && st.italic);
    // Inserting a row above shifts the range and the operand.
    f.apply(vec![Op::InsertRows {
        sheet: s,
        at: 0,
        count: 1,
    }]);
    let cfs = f.svc.conditional_formats(f.doc, f.sheet).unwrap();
    assert_eq!(cfs[0].ranges, vec![range("A2:A4")]);
    assert!(cf_bold(&f, "A3"));
    // Update and delete.
    let id = cfs[0].id;
    round_trip(
        &mut f,
        vec![Op::UpdateConditionalFormat {
            sheet: s,
            id,
            ranges: vec![range("A2:A4")],
            rules: vec![rule(
                CfRuleKind::CellValue {
                    op: CmpOp::Between,
                    formulas: vec!["0".into(), "2".into()],
                },
                bold(),
            )],
        }],
    );
    assert!(cf_bold(&f, "A2"));
    assert!(!cf_bold(&f, "A3"));
    round_trip(&mut f, vec![Op::DeleteConditionalFormat { sheet: s, id }]);
    assert!(view(&f, "A2").unwrap().cf_style.is_none());
}

#[test]
fn formula_text_and_blank_rules() {
    let mut f = Fixture::new();
    f.set("A1", "Paid");
    f.set("A2", "overdue");
    f.set("A4", "=1/0");
    add_cf(
        &mut f,
        "A1:A4",
        vec![
            rule(
                CfRuleKind::Text {
                    op: tally_api::CfTextOp::Contains,
                    text: "DUE".into(),
                },
                bold(),
            ),
            rule(CfRuleKind::Blank, red_fill()),
            rule(CfRuleKind::Error, red_fill()),
        ],
    );
    assert!(cf_bold(&f, "A2"));
    assert!(!cf_bold(&f, "A1"));
    // The empty A3 gets a view because a rule matches it.
    assert!(view(&f, "A3").and_then(|v| v.cf_style).is_some());
    assert!(view(&f, "A4").unwrap().cf_style.is_some());
    // Formula rule with a mixed reference.
    let mut f = Fixture::new();
    f.set("A1", "1");
    f.set("A2", "2");
    add_cf(
        &mut f,
        "A1:A2",
        vec![rule(
            CfRuleKind::Formula {
                formula: "MOD($A1,2)=0".into(),
            },
            bold(),
        )],
    );
    assert!(!cf_bold(&f, "A1"));
    assert!(cf_bold(&f, "A2"));
}

#[test]
fn scales_bars_icons_top_and_duplicates() {
    let mut f = Fixture::new();
    for i in 1..=5 {
        f.set(&format!("A{i}"), &(i * 10).to_string());
    }
    f.set("B1", "x");
    f.set("B2", "y");
    f.set("B3", "X");
    let th = |kind, value: &str| CfThreshold {
        kind,
        value: value.into(),
    };
    add_cf(
        &mut f,
        "A1:A5",
        vec![
            rule(
                CfRuleKind::ColorScale {
                    stops: vec![
                        CfColorStop {
                            threshold: th(CfValueKind::Min, ""),
                            color: 0x0000_00ff,
                        },
                        CfColorStop {
                            threshold: th(CfValueKind::Max, ""),
                            color: 0xff00_00ff,
                        },
                    ],
                },
                CfStyleDto::default(),
            ),
            rule(
                CfRuleKind::DataBar {
                    min: th(CfValueKind::Min, ""),
                    max: th(CfValueKind::Max, ""),
                    color: 0x00ff_00ff,
                    negative_color: None,
                    show_value: false,
                    gradient: true,
                },
                CfStyleDto::default(),
            ),
            rule(
                CfRuleKind::IconSet {
                    set: IconSetKind::ThreeArrows,
                    thresholds: vec![
                        th(CfValueKind::Percent, "0"),
                        th(CfValueKind::Percent, "33"),
                        th(CfValueKind::Percent, "67"),
                    ],
                    show_value: true,
                    reverse: false,
                },
                CfStyleDto::default(),
            ),
            rule(
                CfRuleKind::Top {
                    rank: 2,
                    percent: false,
                    bottom: false,
                },
                bold(),
            ),
        ],
    );
    let fill = |f: &Fixture, a1: &str| {
        let id = view(f, a1).unwrap().cf_style.unwrap();
        f.svc.styles(f.doc, &[id]).unwrap()[0].fill.unwrap()
    };
    assert_eq!(fill(&f, "A1"), 0x0000_00ff);
    assert_eq!(fill(&f, "A5"), 0xff00_00ff);
    assert_eq!(fill(&f, "A3"), 0x8000_00ff);
    let v3 = view(&f, "A3").unwrap();
    assert_eq!(v3.cf_bar.as_ref().map(|b| b.fraction), Some(0.5));
    assert_eq!(v3.text, "", "data bar without the value");
    assert_eq!(v3.cf_icon.as_ref().map(|i| i.index), Some(1));
    assert_eq!(view(&f, "A5").unwrap().cf_icon.unwrap().index, 2);
    assert!(cf_bold(&f, "A4") && cf_bold(&f, "A5") && !cf_bold(&f, "A3"));
    // Aggregates follow edits (cache per version).
    f.set("A1", "100");
    assert!(cf_bold(&f, "A1"));
    assert!(!cf_bold(&f, "A4"));
    add_cf(&mut f, "B1:B3", vec![rule(CfRuleKind::Duplicate, bold())]);
    assert!(cf_bold(&f, "B1") && cf_bold(&f, "B3") && !cf_bold(&f, "B2"));
}

#[test]
fn cf_save_reload() {
    let mut f = Fixture::new();
    f.set("A1", "7");
    add_cf(
        &mut f,
        "A1:A9",
        vec![rule(
            CfRuleKind::CellValue {
                op: CmpOp::Ge,
                formulas: vec!["$C$1+1".into()],
            },
            CfStyleDto {
                number_format: Some("0.00".into()),
                ..bold()
            },
        )],
    );
    let data = f.data();
    assert_eq!(data.sheets[0].conditional_formats.len(), 1);
    let wb = tally_engine::Workbook::from_data(&data);
    assert_eq!(
        wb.to_data().sheets[0].conditional_formats,
        data.sheets[0].conditional_formats
    );
    // A CF number format changes the displayed text.
    assert_eq!(view(&f, "A1").unwrap().text, "7.00");
}

// ----- b. data validation ----------------------------------------------------

fn list_validation(style: ValidationErrorStyle) -> ValidationDto {
    ValidationDto {
        id: 0,
        ranges: vec![range("A1:A10")],
        kind: ValidationKind::List {
            source: ListSource::Values {
                items: vec!["Paid".into(), "Due".into()],
            },
        },
        allow_blank: true,
        show_dropdown: true,
        input_help: Some(ValidationMessage {
            title: "Status".into(),
            text: "Pick one".into(),
        }),
        error: Some(ValidationAlert {
            style,
            title: "Oops".into(),
            text: "Not a status".into(),
        }),
    }
}

#[test]
fn validation_stop_warning_and_info() {
    let mut f = Fixture::new();
    let s = f.sheet;
    round_trip(
        &mut f,
        vec![Op::AddValidation {
            sheet: s,
            validation: list_validation(ValidationErrorStyle::Stop),
        }],
    );
    let res = f.set("A1", "paid");
    assert!(res.errors.is_empty(), "case-insensitive: {:?}", res.errors);
    let res = f.set("A2", "Nope");
    assert_eq!(res.errors.len(), 1);
    assert_eq!(res.errors[0].code, "validation");
    assert_eq!(res.errors[0].message, "Not a status");
    assert_eq!(f.value("A2"), Value::Empty, "rejected input is reverted");
    // Outside the range anything goes.
    assert!(f.set("B2", "Nope").errors.is_empty());
    // Paste is validated too.
    let res = f.apply(vec![Op::Paste {
        sheet: s,
        target: range("A3"),
        payload: tally_api::PastePayload::Tsv {
            text: "Due\nBad".into(),
        },
    }]);
    assert_eq!(res.errors.len(), 1);
    assert_eq!(f.value("A3"), Value::Empty);
    // Dropdown flag and editor info.
    assert_ne!(
        view(&f, "A5").unwrap().flags & cell_flags::VALIDATION_DROPDOWN,
        0
    );
    let info = f.svc.validation_at(f.doc, s, 4, 0).unwrap().unwrap();
    assert_eq!(info.list_items, vec!["Paid", "Due"]);
    assert_eq!(info.input_help.unwrap().title, "Status");
    assert!(f.svc.validation_at(f.doc, s, 4, 3).unwrap().is_none());
    // Warning: accepted with a notice.
    let id = f.svc.validations(f.doc, s).unwrap()[0].id;
    f.apply(vec![Op::UpdateValidation {
        sheet: s,
        id,
        validation: list_validation(ValidationErrorStyle::Warning),
    }]);
    let res = f.set("A2", "Nope");
    assert!(res.errors.is_empty());
    assert_eq!(res.warnings.as_ref().map(Vec::len), Some(1));
    assert_eq!(f.value("A2"), Value::text("Nope"));
    round_trip(&mut f, vec![Op::DeleteValidation { sheet: s, id }]);
}

#[test]
fn validation_numbers_ranges_and_custom() {
    let mut f = Fixture::new();
    let s = f.sheet;
    let whole = ValidationDto {
        id: 0,
        ranges: vec![range("A1:A5")],
        kind: ValidationKind::Whole {
            op: CmpOp::Between,
            formulas: vec!["1".into(), "$C$1".into()],
        },
        allow_blank: true,
        show_dropdown: false,
        input_help: None,
        error: Some(ValidationAlert {
            style: ValidationErrorStyle::Stop,
            title: String::new(),
            text: String::new(),
        }),
    };
    f.set("C1", "10");
    f.apply(vec![Op::AddValidation {
        sheet: s,
        validation: whole,
    }]);
    assert!(f.set("A1", "5").errors.is_empty());
    assert_eq!(f.set("A2", "5.5").errors.len(), 1, "not whole");
    assert_eq!(f.set("A2", "11").errors.len(), 1, "above C1");
    assert_eq!(f.set("A2", "text").errors.len(), 1);
    assert_eq!(
        f.set("A2", "=C1+5").errors.len(),
        1,
        "formula results are checked"
    );
    assert!(f.set("A2", "=C1-1").errors.is_empty());
    // A list from a range.
    f.set("E1", "red");
    f.set("E2", "green");
    let list = ValidationDto {
        id: 0,
        ranges: vec![range("B1:B5")],
        kind: ValidationKind::List {
            source: ListSource::Range {
                reference: "$E$1:$E$3".into(),
            },
        },
        allow_blank: true,
        show_dropdown: true,
        input_help: None,
        error: Some(ValidationAlert {
            style: ValidationErrorStyle::Stop,
            title: String::new(),
            text: String::new(),
        }),
    };
    f.apply(vec![Op::AddValidation {
        sheet: s,
        validation: list,
    }]);
    let info = f.svc.validation_at(f.doc, s, 2, 1).unwrap().unwrap();
    assert_eq!(info.list_items, vec!["red", "green"]);
    assert!(f.set("B1", "GREEN").errors.is_empty());
    assert_eq!(f.set("B2", "blue").errors.len(), 1);
    // Custom formula against the cell itself.
    let custom = ValidationDto {
        id: 0,
        ranges: vec![range("D1:D5")],
        kind: ValidationKind::Custom {
            formula: "LEN(D1)<=3".into(),
        },
        allow_blank: true,
        show_dropdown: false,
        input_help: None,
        error: Some(ValidationAlert {
            style: ValidationErrorStyle::Stop,
            title: String::new(),
            text: String::new(),
        }),
    };
    f.apply(vec![Op::AddValidation {
        sheet: s,
        validation: custom,
    }]);
    assert!(f.set("D2", "abc").errors.is_empty());
    assert_eq!(f.set("D3", "abcd").errors.len(), 1);
    // Saved and reloaded.
    let data = f.data();
    assert_eq!(data.sheets[0].validations.len(), 3);
    let wb = tally_engine::Workbook::from_data(&data);
    assert_eq!(
        wb.to_data().sheets[0].validations,
        data.sheets[0].validations
    );
}

// ----- c/d. notes and links --------------------------------------------------

#[test]
fn notes_follow_cells() {
    let mut f = Fixture::new();
    let s = f.sheet;
    let note = |text: &str| NoteDto {
        r: 0,
        c: 0,
        text: text.into(),
        author: Some("Ana".into()),
        created: None,
        shown: false,
    };
    round_trip(
        &mut f,
        vec![Op::SetNote {
            sheet: s,
            r: 2,
            c: 1,
            note: note("check"),
        }],
    );
    let n = f.svc.note(f.doc, s, 2, 1).unwrap().unwrap();
    assert_eq!(n.text, "check");
    assert!(n.created.is_some());
    assert_ne!(view(&f, "B3").unwrap().flags & cell_flags::COMMENT, 0);
    // Insert rows above: the note moves.
    round_trip(
        &mut f,
        vec![Op::InsertRows {
            sheet: s,
            at: 0,
            count: 2,
        }],
    );
    assert!(f.svc.note(f.doc, s, 4, 1).unwrap().is_some());
    // Move.
    f.apply(vec![Op::MoveRange {
        sheet: s,
        from: range("B5"),
        to: CellPos { r: 0, c: 5 },
    }]);
    assert!(f.svc.note(f.doc, s, 0, 5).unwrap().is_some());
    // Sort carries notes with their rows.
    f.set("A1", "b");
    f.set("A2", "a");
    f.apply(vec![Op::SetNote {
        sheet: s,
        r: 0,
        c: 0,
        note: note("on b"),
    }]);
    f.apply(vec![Op::SortRange {
        sheet: s,
        range: range("A1:A2"),
        keys: vec![tally_api::SortKey {
            col: 0,
            ascending: true,
        }],
        has_header: false,
        case_sensitive: false,
        options: None,
    }]);
    assert_eq!(f.svc.note(f.doc, s, 1, 0).unwrap().unwrap().text, "on b");
    round_trip(
        &mut f,
        vec![Op::DeleteNote {
            sheet: s,
            r: 1,
            c: 0,
        }],
    );
    assert_eq!(f.svc.notes(f.doc, s).unwrap().len(), 1);
    // Delete rows removes notes in them.
    f.apply(vec![Op::DeleteRows {
        sheet: s,
        at: 0,
        count: 1,
    }]);
    assert!(f.svc.notes(f.doc, s).unwrap().is_empty());
}

#[test]
fn links_and_hyperlink_formulas() {
    let mut f = Fixture::new();
    let s = f.sheet;
    f.set("A1", "Site");
    round_trip(
        &mut f,
        vec![Op::SetLink {
            sheet: s,
            r: 0,
            c: 0,
            url: "https://example.com".into(),
        }],
    );
    let v = view(&f, "A1").unwrap();
    assert_eq!(v.link.as_deref(), Some("https://example.com"));
    assert_ne!(v.flags & cell_flags::LINK, 0);
    // Links on empty cells still get a view.
    f.apply(vec![Op::SetLink {
        sheet: s,
        r: 5,
        c: 5,
        url: "mailto:a@b.c".into(),
    }]);
    assert!(view(&f, "F6").is_some());
    let bad = f.apply(vec![Op::SetLink {
        sheet: s,
        r: 0,
        c: 1,
        url: "javascript:alert(1)".into(),
    }]);
    assert_eq!(bad.errors.len(), 1);
    round_trip(
        &mut f,
        vec![Op::ClearLink {
            sheet: s,
            ranges: vec![range("A1:F6")],
        }],
    );
    assert!(view(&f, "A1").unwrap().link.is_none());
    f.set("B1", "=HYPERLINK(\"https://tally.app\",\"Tally\")");
    let v = view(&f, "B1").unwrap();
    assert_eq!(v.text, "Tally");
    assert_eq!(v.link.as_deref(), Some("https://tally.app"));
    // Clear all removes links and notes.
    f.apply(vec![Op::SetLink {
        sheet: s,
        r: 0,
        c: 0,
        url: "https://x.y".into(),
    }]);
    f.apply(vec![Op::Clear {
        sheet: s,
        ranges: vec![range("A1")],
        what: tally_api::ClearWhat::All,
    }]);
    assert!(f.data().sheets[0].links.is_empty());
}

// ----- e. protection ---------------------------------------------------------

#[test]
fn sheet_and_cell_protection() {
    let mut f = Fixture::new();
    let s = f.sheet;
    f.set("A1", "=1+1");
    round_trip(
        &mut f,
        vec![Op::SetCellProtection {
            sheet: s,
            ranges: vec![range("B1:B5")],
            locked: Some(false),
            hidden_formula: None,
        }],
    );
    f.apply(vec![Op::SetCellProtection {
        sheet: s,
        ranges: vec![range("A1")],
        locked: None,
        hidden_formula: Some(true),
    }]);
    not_undoable(
        &mut f,
        vec![Op::ProtectSheet {
            sheet: s,
            password: Some("pw".into()),
            options: None,
        }],
    );
    let info = f.svc.protection_info(f.doc, s).unwrap();
    assert!(info.sheet_protected && info.sheet_has_password);
    let res = f.set("A2", "x");
    assert_eq!(res.errors[0].code, "protected");
    assert!(
        f.set("B2", "ok").errors.is_empty(),
        "unlocked cells stay editable"
    );
    let res = f.apply(vec![Op::InsertRows {
        sheet: s,
        at: 0,
        count: 1,
    }]);
    assert_eq!(res.errors[0].code, "protected");
    let res = f.apply(vec![Op::ApplyStyle {
        sheet: s,
        ranges: vec![range("A1")],
        patch: StylePatch {
            bold: Some(true),
            ..StylePatch::default()
        },
    }]);
    assert_eq!(res.errors[0].code, "protected");
    // Hidden formula on a protected sheet.
    assert_eq!(f.svc.cell_edit_info(f.doc, s, 0, 0).unwrap().input, "");
    // Pasting over locked cells is refused and reverted.
    let res = f.apply(vec![Op::Paste {
        sheet: s,
        target: range("A3"),
        payload: tally_api::PastePayload::Tsv { text: "1".into() },
    }]);
    assert_eq!(res.errors[0].code, "protected");
    assert_eq!(f.value("A3"), Value::Empty);
    let res = f.apply(vec![Op::UnprotectSheet {
        sheet: s,
        password: Some("nope".into()),
    }]);
    assert_eq!(res.errors[0].code, "protected");
    not_undoable(
        &mut f,
        vec![Op::UnprotectSheet {
            sheet: s,
            password: Some("pw".into()),
        }],
    );
    assert!(f.set("A2", "x").errors.is_empty());
    assert_eq!(f.svc.cell_edit_info(f.doc, s, 0, 0).unwrap().input, "=1+1");
    // Options allow some actions.
    f.apply(vec![Op::ProtectSheet {
        sheet: s,
        password: None,
        options: Some(ProtectionOptions {
            insert_rows: true,
            ..ProtectionOptions::default()
        }),
    }]);
    assert!(f
        .apply(vec![Op::InsertRows {
            sheet: s,
            at: 0,
            count: 1,
        }])
        .errors
        .is_empty());
    // Saved with the hash; reloads protected.
    let data = f.data();
    assert!(data.sheets[0].protection.is_some());
    assert_eq!(
        data.styles
            .iter()
            .filter(|st| st.protection.is_some_and(|p| !p.locked))
            .count(),
        1
    );
}

#[test]
fn structure_protection() {
    let mut f = Fixture::new();
    not_undoable(
        &mut f,
        vec![Op::ProtectDocument {
            password: Some("x".into()),
        }],
    );
    let res = f.apply(vec![Op::AddSheet {
        name: None,
        index: None,
    }]);
    assert_eq!(res.errors[0].code, "protected");
    assert!(
        f.svc
            .protection_info(f.doc, f.sheet)
            .unwrap()
            .structure_protected
    );
    assert_eq!(
        f.apply(vec![Op::UnprotectDocument { password: None }])
            .errors[0]
            .code,
        "protected"
    );
    not_undoable(
        &mut f,
        vec![Op::UnprotectDocument {
            password: Some("x".into()),
        }],
    );
}

// ----- f. page setup ---------------------------------------------------------

#[test]
fn page_setup_and_breaks() {
    let mut f = Fixture::new();
    let s = f.sheet;
    let mut p = f.svc.page_setup(f.doc, s).unwrap();
    assert_eq!(p.paper, Paper::A4);
    p.paper = Paper::Letter;
    p.scale = PageScale::FitTo {
        width: 1,
        height: 0,
    };
    p.print_range = Some(range("A1:D20"));
    p.repeat_rows = Some([0, 0]);
    p.header = Some(tally_api::HeaderFooter {
        left: "&[FILE]".into(),
        center: String::new(),
        right: "&[DATE]".into(),
    });
    round_trip(
        &mut f,
        vec![Op::SetPageSetup {
            sheet: s,
            setup: p.clone(),
        }],
    );
    round_trip(
        &mut f,
        vec![Op::SetPageBreak {
            sheet: s,
            axis: BreakAxis::Row,
            at: 10,
            present: true,
        }],
    );
    let got = f.svc.page_setup(f.doc, s).unwrap();
    assert_eq!(got.paper, Paper::Letter);
    assert_eq!(got.row_breaks, vec![10]);
    f.apply(vec![Op::InsertRows {
        sheet: s,
        at: 5,
        count: 2,
    }]);
    let got = f.svc.page_setup(f.doc, s).unwrap();
    assert_eq!(got.row_breaks, vec![12]);
    assert_eq!(got.print_range, Some(range("A1:D22")));
}

// ----- g. document properties ------------------------------------------------

#[test]
fn document_properties() {
    let mut f = Fixture::new();
    round_trip(
        &mut f,
        vec![Op::SetDocumentProperties {
            props: DocPropertiesDto {
                title: Some("Budget".into()),
                subject: Some("2026".into()),
                keywords: vec!["money".into(), " ".into()],
                description: None,
                author: Some("Ana".into()),
                ..DocPropertiesDto::default()
            },
        }],
    );
    let p = f.svc.document_properties(f.doc).unwrap();
    assert_eq!(p.title.as_deref(), Some("Budget"));
    assert_eq!(p.keywords, vec!["money"]);
    assert_eq!(f.data().meta.author.as_deref(), Some("Ana"));
}

// ----- h. auto-fit -----------------------------------------------------------

#[test]
fn auto_fit_rows() {
    let mut f = Fixture::new();
    let s = f.sheet;
    f.set(
        "A1",
        "a long line of text that certainly needs several lines in a narrow column",
    );
    f.set("A2", "short");
    f.apply(vec![Op::ApplyStyle {
        sheet: s,
        ranges: vec![range("A1:A2")],
        patch: StylePatch {
            wrap: Some(true),
            ..StylePatch::default()
        },
    }]);
    let before = f.data();
    let res = f.apply(vec![Op::AutoFitRows {
        sheet: s,
        start: 0,
        end: 5,
    }]);
    assert!(res.errors.is_empty());
    let fitted = res.fitted_rows.unwrap();
    assert_eq!(fitted[0].start, 0);
    assert!(fitted[0].size > 24.0 * 2.0, "{fitted:?}");
    assert!(fitted
        .iter()
        .any(|r| r.start <= 1 && 1 <= r.end && r.size == 24.0));
    let layout = f.svc.sheet_layout(f.doc, s).unwrap();
    assert!(layout
        .row_runs
        .iter()
        .any(|r| r.start == 0 && r.size > 48.0));
    f.svc.undo(f.doc).unwrap();
    assert_eq!(f.data(), before);
}

// ----- i. locales ------------------------------------------------------------

#[test]
fn locale_changes_formatting_and_input() {
    let mut f = Fixture::new();
    let s = f.sheet;
    f.set("A1", "1234.5");
    f.apply(vec![Op::SetNumberFormat {
        sheet: s,
        ranges: vec![range("A1")],
        code: "#,##0.00".into(),
    }]);
    assert_eq!(f.text("A1"), "1,234.50");
    let res = f.svc.set_locale(f.doc, "de-DE").unwrap();
    assert!(!res.changed.is_empty());
    assert_eq!(f.text("A1"), "1.234,50");
    f.set("A2", "2,5");
    assert_eq!(f.value("A2"), Value::Number(2.5));
    f.set("A3", "19.10.2026");
    assert_eq!(f.text("A3"), "19.10.2026");
    f.set("A4", "=SUM(A1,A2)");
    assert_eq!(
        f.svc.cell_edit_info(f.doc, s, 3, 0).unwrap().input,
        "=SUM(A1;A2)",
        "the UI dialect uses ; in de-DE"
    );
    assert_eq!(f.data().settings.locale.as_deref(), Some("de-DE"));
    assert!(f.svc.set_locale(f.doc, "xx").is_err());
    assert_eq!(f.svc.list_locales().len(), 6);
}

// ----- j. goal seek ----------------------------------------------------------

#[test]
fn goal_seek_through_the_service() {
    let mut f = Fixture::new();
    let s = f.sheet;
    f.set("A1", "100");
    f.set("B1", "=A1*1.2-10");
    let r = f
        .svc
        .goal_seek(
            f.doc,
            &GoalSeekRequest {
                formula_sheet: s,
                formula_cell: CellPos { r: 0, c: 1 },
                target: 50.0,
                variable_sheet: s,
                variable_cell: CellPos { r: 0, c: 0 },
            },
        )
        .unwrap();
    assert!(r.found);
    assert!((r.value - 50.0).abs() < 1e-9);
    assert_eq!(f.value("A1"), Value::Number(100.0), "nothing applied");
    f.apply(vec![input(s, "A1", &r.value.to_string())]);
    match f.value("B1") {
        Value::Number(n) => assert!((n - 50.0).abs() < 1e-9 * 50.0),
        other => panic!("{other:?}"),
    }
}

// ----- review fixes ------------------------------------------------------------

#[test]
fn reference_unions_in_sequence_arguments() {
    let mut f = Fixture::new();
    f.set("A1", "1");
    f.set("B2", "5");
    f.set("C3", "10");
    f.set("D1", "=SUM(A1~B2~C3)");
    assert_eq!(f.value("D1"), Value::Number(16.0));
    f.set("D2", "=SUM((A1~B2),C3,1)");
    assert_eq!(f.value("D2"), Value::Number(17.0));
    f.set("D3", "=COUNT(A1:A2~B2)");
    assert_eq!(f.value("D3"), Value::Number(2.0));
    f.set("D4", "=ABS(A1~B2)");
    assert_eq!(f.value("D4"), Value::Error(tally_types::ErrorKind::Value));
    // Dependencies of every area are tracked.
    f.set("C3", "20");
    assert_eq!(f.value("D1"), Value::Number(26.0));
}

#[test]
fn sort_keeps_relative_references_relative() {
    let mut f = Fixture::new();
    let s = f.sheet;
    f.set("A1", "2");
    f.set("A2", "1");
    f.set("D1", "100");
    f.set("D2", "200");
    // B1 refers outside the sorted range (D1, relative).
    f.set("B1", "=D1");
    f.set("B2", "=D2");
    f.apply(vec![Op::SortRange {
        sheet: s,
        range: range("A1:B2"),
        keys: vec![tally_api::SortKey {
            col: 0,
            ascending: true,
        }],
        has_header: false,
        case_sensitive: false,
        options: None,
    }]);
    // Row 1 (A=2, B=D1) moved to row 2 and now reads D2 (AOO SwapRow).
    let edit = |a: &str| {
        let c = CellAddr::from_a1(s, a).unwrap();
        f.svc.cell_edit_info(f.doc, s, c.row, c.col).unwrap().input
    };
    assert_eq!(edit("B2"), "=D2");
    assert_eq!(edit("B1"), "=D1");
    assert_eq!(f.value("A1"), Value::Number(1.0));
}

#[test]
fn paste_special_transpose_transposes_references() {
    let mut f = Fixture::new();
    let s = f.sheet;
    // A1:A2 = [1; =A1*5]; transposed to F2:G2 the formula reads F2.
    f.set("A1", "1");
    f.set("A2", "=A1*5");
    f.set("B2", "=$A$1+A1");
    let clip = f.svc.copy(f.doc, &common::sel(s, "A1:B2")).unwrap();
    let res = f.apply(vec![Op::PasteSpecial {
        sheet: s,
        target: range("F2"),
        payload: tally_api::PastePayload::Internal { token: clip.token },
        options: tally_api::PasteOptions {
            transpose: true,
            ..tally_api::PasteOptions::default()
        },
    }]);
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let edit = |a: &str| {
        let c = CellAddr::from_a1(s, a).unwrap();
        f.svc.cell_edit_info(f.doc, s, c.row, c.col).unwrap().input
    };
    // Source A2 (row 1, col 0) lands at G2 (row 1 → col 1).
    assert_eq!(edit("G2"), "=F2*5");
    assert_eq!(f.value("G2"), Value::Number(5.0));
    // B2 lands at G3; $A$1 stays, A1 (up-left) stays up-left.
    assert_eq!(edit("G3"), "=$A$1+F2");
}

#[test]
fn indirect_r1c1() {
    let mut f = Fixture::new();
    f.set("A1", "7");
    f.set("B3", "9");
    f.set("C5", "=INDIRECT(\"R1C1\",0)");
    assert_eq!(f.value("C5"), Value::Number(7.0));
    f.set("C6", "=INDIRECT(\"R[-3]C[-1]\",FALSE())");
    assert_eq!(f.value("C6"), Value::Number(9.0));
    f.set("C7", "=SUM(INDIRECT(\"R1C1:R3C2\",0))");
    assert_eq!(f.value("C7"), Value::Number(16.0));
    f.set("C8", "=INDIRECT(\"A1\",0)");
    assert_eq!(
        f.value("C8"),
        Value::Error(tally_types::ErrorKind::InvalidArgument)
    );
}

#[test]
fn odf_function_names_resolve_odf_first() {
    let mut data = tally_model::WorkbookData::default();
    let cell = |col: u32, text: &str| tally_model::CellData {
        row: 0,
        col,
        content: tally_model::CellContentData::Formula {
            text: text.into(),
            cached: None,
        },
        style: None,
    };
    data.sheets.push(tally_model::SheetData {
        name: "S".into(),
        cells: vec![
            cell(0, "of:=DURATION(DATE(2026;1;1);DATE(2036;1;1);0.05;0.06;2)"),
            cell(1, "of:=PDURATION(0.1;100;200)"),
            cell(2, "of:=CONVERT(1;\"m\";\"cm\")"),
        ],
        ..tally_model::SheetData::default()
    });
    let mut wb = tally_engine::Workbook::from_data(&data);
    tally_engine::recalc::recalc(&mut wb);
    let s = wb.sheets[0].id;
    let reg = tally_functions::registry();
    let name_at = |c: u32| {
        let (_, f) = wb.formula_at(CellAddr::new(s, 0, c)).unwrap();
        match &f.formula.expr {
            tally_formula::Expr::Call {
                func: tally_formula::FuncRef::Known(id),
                ..
            } => reg.get(tally_functions::FuncId(id.0)).unwrap().name,
            other => panic!("{other:?}"),
        }
    };
    assert_eq!(
        name_at(0),
        "DURATION_ADD",
        "ODF DURATION is the bond add-in"
    );
    assert_eq!(name_at(1), "DURATION", "PDURATION is the built-in");
    assert_eq!(name_at(2), "CONVERT_ADD");
    // ... and they are written back with their ODF names.
    let out = wb.to_data();
    match &out.sheets[0].cells[0].content {
        tally_model::CellContentData::Formula { text, .. } => {
            assert!(text.starts_with("of:=DURATION("), "{text}")
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn cell_protect_info() {
    let mut f = Fixture::new();
    let s = f.sheet;
    f.set("A1", "=CELL(\"protect\";B1)");
    assert_eq!(f.value("A1"), Value::Number(1.0), "locked by default");
    f.apply(vec![Op::SetCellProtection {
        sheet: s,
        ranges: vec![range("B1")],
        locked: Some(false),
        hidden_formula: None,
    }]);
    f.svc.recalc_all(f.doc).unwrap();
    assert_eq!(f.value("A1"), Value::Number(0.0));
}
