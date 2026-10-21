//! Correctness review 3 fixes (engine side, wave 5, W5-3): H1, M3, M4, M5,
//! M8, M12, M13 and the LOW items in conditional formatting, validation,
//! goal seek and page setup.
#![allow(clippy::unwrap_used)]

mod common;

use std::path::Path;

use common::{range, sel, Fixture};
use tally_api::{
    cell_flags, CellPos, CfRuleDto, CfRuleKind, CfStyleDto, ClearWhat, CmpOp, DocumentService,
    FindMode, FindQuery, GoalSeekRequest, ListSource, NoteDto, Op, PastePayload, ValidationAlert,
    ValidationDto, ValidationErrorStyle, ValidationKind,
};
use tally_types::{CellAddr, Value};

fn repo(p: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(p)
}

fn cf_rule(op: CmpOp, operand: &str, style: CfStyleDto) -> CfRuleDto {
    CfRuleDto {
        kind: CfRuleKind::CellValue {
            op,
            formulas: vec![operand.into()],
        },
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

fn cf_bold(f: &Fixture, a1: &str) -> bool {
    let a = CellAddr::from_a1(f.sheet, a1).unwrap();
    let r = range(a1);
    let v = f.svc.cells(f.doc, f.sheet, &[r]).unwrap()[0]
        .cells
        .iter()
        .find(|c| (c.r, c.c) == (a.row, a.col))
        .cloned();
    v.and_then(|v| v.cf_style)
        .map(|id| f.svc.styles(f.doc, &[id]).unwrap()[0].bold)
        .unwrap_or(false)
}

fn validation(
    ranges: &str,
    kind: ValidationKind,
    allow_blank: bool,
    style: ValidationErrorStyle,
) -> ValidationDto {
    ValidationDto {
        id: 0,
        ranges: vec![range(ranges)],
        kind,
        allow_blank,
        show_dropdown: false,
        input_help: None,
        error: Some(ValidationAlert {
            style,
            title: String::new(),
            text: "bad".into(),
        }),
    }
}

// ----- H1: AOO style:map import -----------------------------------------------

#[test]
#[cfg(feature = "io")]
fn h1_style_maps_per_style_first_match() {
    let mut svc = tally_engine::EngineService::with_default_codec();
    let info = svc
        .open(&repo("tests/fixtures/ods/review3-stylemap.ods"))
        .unwrap();
    let s = info.sheets[0].id;
    let cfs = svc.conditional_formats(info.doc, s).unwrap();
    assert!(cfs.len() >= 3, "one format per automatic style: {cfs:?}");
    assert!(cfs.iter().all(|cf| cf.rules.iter().all(|r| r.stop_if_true)));
    let style_of = |a1: &str| {
        let a = CellAddr::from_a1(s, a1).unwrap();
        let r = range(a1);
        svc.cells(info.doc, s, &[r]).unwrap()[0]
            .cells
            .iter()
            .find(|c| (c.r, c.c) == (a.row, a.col))
            .and_then(|v| v.cf_style)
            .map(|id| svc.styles(info.doc, &[id]).unwrap()[0].clone())
    };
    // A1 = 3: `<5` → bold wins over `<10` → red (first match only).
    let a1 = style_of("A1").expect("A1 has a conditional style");
    assert!(a1.bold && a1.fill.is_none(), "{a1:?}");
    // B3 = 100 > A1 (base B1): bold from the second style's own map.
    let b3 = style_of("B3").expect("B3");
    assert!(b3.bold, "{b3:?}");
}

// ----- M3: merge reports cleared contents ---------------------------------------

#[test]
fn m3_merge_warns_about_cleared_cells() {
    let mut f = Fixture::new();
    let s = f.sheet;
    f.set("B2", "x");
    f.set("C3", "y");
    let res = f.apply(vec![Op::MergeCells {
        sheet: s,
        range: range("B2:C3"),
    }]);
    assert!(res.errors.is_empty());
    let w = res.warnings.expect("a warning about cleared contents");
    assert_eq!(w.len(), 1);
    assert_eq!(w[0].code, "merge_cleared");
    assert!(w[0].message.contains("1 covered cell"), "{}", w[0].message);
    assert_eq!(f.value("B2"), Value::text("x"));
    let res = f.apply(vec![Op::MergeCells {
        sheet: s,
        range: range("E1:F1"),
    }]);
    assert!(
        res.warnings.is_none(),
        "nothing cleared: {:?}",
        res.warnings
    );
}

// ----- M4: CF, validations and notes follow move and copy/paste ----------------

#[test]
fn m4_rules_and_notes_follow_cells() {
    let mut f = Fixture::new();
    let s = f.sheet;
    for r in 1..=3 {
        f.set(&format!("B{r}"), &(r * 10).to_string());
    }
    f.apply(vec![Op::AddConditionalFormat {
        sheet: s,
        ranges: vec![range("B1:B3")],
        rules: vec![cf_rule(CmpOp::Gt, "15", bold())],
    }]);
    f.apply(vec![Op::AddValidation {
        sheet: s,
        validation: validation(
            "B1:B3",
            ValidationKind::Whole {
                op: CmpOp::Between,
                formulas: vec!["0".into(), "100".into()],
            },
            true,
            ValidationErrorStyle::Stop,
        ),
    }]);
    f.apply(vec![Op::SetNote {
        sheet: s,
        r: 1,
        c: 1,
        note: NoteDto {
            r: 1,
            c: 1,
            text: "note".into(),
            author: None,
            created: None,
            shown: false,
        },
    }]);
    assert!(cf_bold(&f, "B2"));
    // Move B1:B3 → D1.
    f.apply(vec![Op::MoveRange {
        sheet: s,
        from: range("B1:B3"),
        to: CellPos { r: 0, c: 3 },
    }]);
    let cfs = f.svc.conditional_formats(f.doc, s).unwrap();
    assert_eq!(cfs.len(), 1);
    assert_eq!(cfs[0].ranges, vec![range("D1:D3")], "{cfs:?}");
    assert!(cf_bold(&f, "D2"));
    assert!(!cf_bold(&f, "B2"));
    let vals = f.svc.validations(f.doc, s).unwrap();
    assert_eq!(vals[0].ranges, vec![range("D1:D3")]);
    assert!(f.set("D1", "500").errors.len() == 1, "validation moved");
    assert!(f.set("B1", "500").errors.is_empty(), "nothing left behind");
    assert!(f.svc.note(f.doc, s, 1, 3).unwrap().is_some());
    // Undo restores the old places.
    f.svc.undo(f.doc).unwrap();
    f.svc.undo(f.doc).unwrap();
    assert_eq!(
        f.svc.conditional_formats(f.doc, s).unwrap()[0].ranges,
        vec![range("B1:B3")]
    );
    f.svc.redo(f.doc).unwrap();
    f.svc.redo(f.doc).unwrap();
    // Copy D1:D3 → paste at F1: rules and notes are cloned.
    let out = f.svc.copy(f.doc, &sel(s, "D1:D3")).unwrap();
    f.apply(vec![Op::Paste {
        sheet: s,
        target: range("F1"),
        payload: PastePayload::Internal { token: out.token },
    }]);
    let cfs = f.svc.conditional_formats(f.doc, s).unwrap();
    assert_eq!(cfs.len(), 2, "{cfs:?}");
    assert!(cfs.iter().any(|cf| cf.ranges == vec![range("F1:F3")]));
    assert!(cf_bold(&f, "F2"));
    assert_eq!(f.svc.validations(f.doc, s).unwrap().len(), 2);
    assert_eq!(f.set("F1", "500").errors.len(), 1, "validation copied");
    assert_eq!(
        f.svc.note(f.doc, s, 1, 5).unwrap().map(|n| n.text),
        Some("note".into())
    );
    // Pasting over the block replaces its rules (no duplicates pile up).
    let out = f.svc.copy(f.doc, &sel(s, "A1:A3")).unwrap();
    f.apply(vec![Op::Paste {
        sheet: s,
        target: range("F1"),
        payload: PastePayload::Internal { token: out.token },
    }]);
    let cfs = f.svc.conditional_formats(f.doc, s).unwrap();
    assert_eq!(cfs.len(), 1, "{cfs:?}");
    assert!(f.svc.note(f.doc, s, 1, 5).unwrap().is_none());
}

// ----- M5: inline list items recognised as numbers -----------------------------

#[test]
fn m5_inline_list_numbers() {
    let mut f = Fixture::new();
    let s = f.sheet;
    f.apply(vec![Op::AddValidation {
        sheet: s,
        validation: validation(
            "A1:A10",
            ValidationKind::List {
                source: ListSource::Values {
                    items: vec!["1.50".into(), "10".into(), "x".into(), "1,000".into()],
                },
            },
            true,
            ValidationErrorStyle::Stop,
        ),
    }]);
    assert!(f.set("A1", "1.5").errors.is_empty());
    assert!(f.set("A2", "1.50").errors.is_empty());
    assert!(f.set("A3", "1000").errors.is_empty());
    assert!(f.set("A4", "1,000").errors.is_empty());
    assert!(f.set("A5", "x").errors.is_empty());
    assert_eq!(f.set("A6", "2").errors.len(), 1);
    assert_eq!(f.set("A7", "y").errors.len(), 1);
}

// ----- M12: ReplaceAll honours validation ---------------------------------------

#[test]
fn m12_replace_all_validation() {
    let mut f = Fixture::new();
    let s = f.sheet;
    f.apply(vec![Op::AddValidation {
        sheet: s,
        validation: validation(
            "A1:A10",
            ValidationKind::Whole {
                op: CmpOp::Between,
                formulas: vec!["1".into(), "3".into()],
            },
            true,
            ValidationErrorStyle::Stop,
        ),
    }]);
    f.set("A1", "2");
    f.set("B1", "2");
    let q = FindQuery {
        text: "2".into(),
        sheet: None,
        in_formulas: false,
        match_case: false,
        whole_cell: true,
        mode: FindMode::Literal,
        from: None,
    };
    let res = f.apply(vec![Op::ReplaceAll {
        sheet: Some(s),
        query: q.clone(),
        replace: "99".into(),
    }]);
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    assert_eq!(f.value("A1"), Value::Number(2.0), "Stop refuses the cell");
    assert_eq!(
        f.value("B1"),
        Value::Number(99.0),
        "unvalidated cell replaced"
    );
    assert_eq!(res.replaced, Some(1));
    let w = res.warnings.expect("notice for the refused cell");
    assert!(
        w.iter()
            .any(|e| e.code == "validation" && e.message.contains("A1")),
        "{w:?}"
    );
    // Warning style accepts with a notice.
    f.apply(vec![Op::DeleteValidation { sheet: s, id: 1 }]);
    f.apply(vec![Op::AddValidation {
        sheet: s,
        validation: validation(
            "A1:A10",
            ValidationKind::Whole {
                op: CmpOp::Between,
                formulas: vec!["1".into(), "3".into()],
            },
            true,
            ValidationErrorStyle::Warning,
        ),
    }]);
    let res = f.apply(vec![Op::ReplaceAll {
        sheet: Some(s),
        query: q,
        replace: "99".into(),
    }]);
    assert_eq!(f.value("A1"), Value::Number(99.0));
    assert!(res.warnings.is_some());
}

// ----- M13 + LOW: blanks, whole numbers, text length as typed ------------------

#[test]
fn m13_blank_whole_and_text_length() {
    let mut f = Fixture::new();
    let s = f.sheet;
    f.apply(vec![Op::AddValidation {
        sheet: s,
        validation: validation(
            "A1:A10",
            ValidationKind::Whole {
                op: CmpOp::Between,
                formulas: vec!["1".into(), "10".into()],
            },
            false,
            ValidationErrorStyle::Stop,
        ),
    }]);
    f.set("A1", "5");
    assert_eq!(
        f.set("A1", "").errors.len(),
        1,
        "blank refused when not allowed"
    );
    assert_eq!(f.value("A1"), Value::Number(5.0));
    assert!(
        f.set("A2", "=2+3").errors.is_empty(),
        "the formula result is checked"
    );
    assert!(
        f.set("A3", "=1.0000000000000002*3").errors.is_empty(),
        "approx whole"
    );
    assert_eq!(f.set("A4", "2.5").errors.len(), 1);
    // Text length measures the typed text.
    f.apply(vec![Op::AddValidation {
        sheet: s,
        validation: validation(
            "C1:C10",
            ValidationKind::TextLength {
                op: CmpOp::Eq,
                formulas: vec!["5".into()],
            },
            true,
            ValidationErrorStyle::Stop,
        ),
    }]);
    assert!(
        f.set("C1", "12.50").errors.is_empty(),
        "5 characters as typed"
    );
    assert_eq!(f.set("C2", "12.5").errors.len(), 1);
    assert!(f.set("C3", "hello").errors.is_empty());
}

// ----- LOW: CF text operand on numbers, duplicates by value, clear all ---------

#[test]
fn low_cf_text_operand_duplicates_and_clear_all() {
    let mut f = Fixture::new();
    let s = f.sheet;
    f.set("A1", "5");
    f.set("A2", "m");
    f.apply(vec![Op::AddConditionalFormat {
        sheet: s,
        ranges: vec![range("A1:A2")],
        rules: vec![cf_rule(CmpOp::Lt, "\"m\"", bold())],
    }]);
    assert!(!cf_bold(&f, "A1"), "a number never matches a text operand");
    f.apply(vec![Op::AddConditionalFormat {
        sheet: s,
        ranges: vec![range("A1:A2")],
        rules: vec![cf_rule(CmpOp::Ne, "\"m\"", bold())],
    }]);
    assert!(cf_bold(&f, "A1"), "except for not-equal");
    // Duplicates compare by value.
    f.set("B1", "0.1");
    f.set("B2", "=0.3-0.2");
    f.set("B3", "ABC");
    f.set("B4", "abc");
    f.apply(vec![Op::AddConditionalFormat {
        sheet: s,
        ranges: vec![range("B1:B4")],
        rules: vec![CfRuleDto {
            kind: CfRuleKind::Duplicate,
            style: bold(),
            stop_if_true: false,
        }],
    }]);
    assert!(
        cf_bold(&f, "B1") && cf_bold(&f, "B2"),
        "0.1 and 0.3-0.2 are duplicates"
    );
    assert!(cf_bold(&f, "B3") && cf_bold(&f, "B4"));
    // Clear All removes the conditional format from the cleared cells.
    f.apply(vec![Op::Clear {
        sheet: s,
        ranges: vec![range("B1:B2")],
        what: ClearWhat::All,
    }]);
    let cfs = f.svc.conditional_formats(f.doc, s).unwrap();
    let dup = cfs
        .iter()
        .find(|cf| matches!(cf.rules[0].kind, CfRuleKind::Duplicate))
        .unwrap();
    assert_eq!(dup.ranges, vec![range("B3:B4")], "{cfs:?}");
    f.svc.undo(f.doc).unwrap();
    let cfs = f.svc.conditional_formats(f.doc, s).unwrap();
    let dup = cfs
        .iter()
        .find(|cf| matches!(cf.rules[0].kind, CfRuleKind::Duplicate))
        .unwrap();
    assert_eq!(dup.ranges, vec![range("B1:B4")]);
    // Clear Contents keeps it.
    f.apply(vec![Op::Clear {
        sheet: s,
        ranges: vec![range("B1:B2")],
        what: ClearWhat::Contents,
    }]);
    assert_eq!(
        f.svc.conditional_formats(f.doc, s).unwrap().len(),
        cfs.len()
    );
    let _ = cell_flags::FORMULA;
}

// ----- LOW: goal seek asymptotes --------------------------------------------------

#[test]
fn low_goal_seek_no_asymptote_false_positive() {
    let mut f = Fixture::new();
    let s = f.sheet;
    f.set("A1", "1");
    f.set("B1", "=EXP(A1)");
    let req = |target: f64| GoalSeekRequest {
        formula_sheet: s,
        formula_cell: CellPos { r: 0, c: 1 },
        target,
        variable_sheet: s,
        variable_cell: CellPos { r: 0, c: 0 },
    };
    assert!(
        !f.svc.goal_seek(f.doc, &req(0.0)).unwrap().found,
        "EXP(x) = 0 has no root"
    );
    let r = f.svc.goal_seek(f.doc, &req(1.0)).unwrap();
    assert!(r.found && r.value.abs() < 1e-6, "{r:?}");
    f.set("B1", "=A1*3+2");
    let r = f.svc.goal_seek(f.doc, &req(20.0)).unwrap();
    assert!(r.found && (r.value - 6.0).abs() < 1e-9, "{r:?}");
    f.set("B1", "=1/A1");
    assert!(!f.svc.goal_seek(f.doc, &req(0.0)).unwrap().found);
}

// ----- M8: ODS protection key AOO can verify ------------------------------------

#[test]
#[cfg(feature = "io")]
fn m8_protection_key_is_sha1_utf16() {
    use tally_api::{FileFormat, ProtectionOptions};
    let mut f = Fixture::new();
    let s = f.sheet;
    f.apply(vec![Op::ProtectSheet {
        sheet: s,
        password: Some("password".into()),
        options: Some(ProtectionOptions::default()),
    }]);
    let dir = std::env::temp_dir().join(format!("tally-m8-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("prot.ods");
    let mut svc = tally_engine::EngineService::with_default_codec();
    let info = svc.open_workbook(f.svc.document(f.doc).unwrap().wb.clone(), "p");
    svc.save(info.doc, Some(&path), FileFormat::Ods).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut content = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("content.xml").unwrap(), &mut content).unwrap();
    // SHA-1 of the UTF-16LE "password", as SvPasswordHelper computes it.
    assert!(content.contains("xmldsig#sha1"), "{content}");
    assert!(
        content.contains("table:protection-key=\"6Pl/upEE0epQR5SObftn+s2fW3M=\""),
        "{content}"
    );
    // The reopened file still verifies with the password.
    let again = svc.open(&path).unwrap();
    let res = svc
        .apply(
            again.doc,
            common::tx(vec![Op::UnprotectSheet {
                sheet: again.sheets[0].id,
                password: Some("password".into()),
            }]),
        )
        .unwrap();
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    // AOO's own key (UTF-8 SHA-1 generator fixture) still verifies too.
    let legacy = svc
        .open(&repo("tests/fixtures/ods/review3-sha1.ods"))
        .unwrap();
    let res = svc
        .apply(
            legacy.doc,
            common::tx(vec![Op::UnprotectSheet {
                sheet: legacy.sheets[0].id,
                password: Some("password".into()),
            }]),
        )
        .unwrap();
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let _ = std::fs::remove_dir_all(&dir);
}

// ----- LOW: SetPageSetup honours breaks ------------------------------------------

#[test]
fn low_set_page_setup_breaks() {
    let mut f = Fixture::new();
    let s = f.sheet;
    let mut setup = f.svc.page_setup(f.doc, s).unwrap();
    setup.row_breaks = vec![10, 20];
    setup.col_breaks = vec![3];
    f.apply(vec![Op::SetPageSetup { sheet: s, setup }]);
    let p = f.svc.page_setup(f.doc, s).unwrap();
    assert_eq!(p.row_breaks, vec![10, 20]);
    assert_eq!(p.col_breaks, vec![3]);
    let mut bad = p.clone();
    bad.row_breaks = vec![0];
    assert_eq!(
        f.apply(vec![Op::SetPageSetup {
            sheet: s,
            setup: bad
        }])
        .errors
        .len(),
        1
    );
}

// ----- M7: unknown locale tag warns ---------------------------------------------

#[test]
fn m7_unknown_locale_warns_and_external_links_reported() {
    use tally_model::{CellContentData, CellData, SheetData, WorkbookData};
    let mut data = WorkbookData::default();
    data.settings.locale = Some("de-CH".into());
    data.sheets.push(SheetData {
        name: "S".into(),
        cells: vec![
            CellData {
                row: 0,
                col: 0,
                content: CellContentData::Formula {
                    text: "of:=['file:///tmp/other.ods'#$Sheet1.A1]".into(),
                    cached: Some(Value::Number(7.0)),
                },
                style: None,
            },
            CellData {
                row: 1,
                col: 0,
                content: CellContentData::Formula {
                    text: "of:=DDE(\"soffice\";\"x.ods\";\"A1\")".into(),
                    cached: None,
                },
                style: None,
            },
        ],
        ..SheetData::default()
    });
    let wb = tally_engine::Workbook::from_data(&data);
    let mut svc = tally_engine::EngineService::new();
    let info = svc.open_workbook(wb, "x");
    assert!(
        info.warnings.iter().any(|w| w.contains("de-CH")),
        "{:?}",
        info.warnings
    );
    let links = info.external_links.expect("external links reported");
    assert_eq!(links.len(), 2, "{links:?}");
    let s = info.sheets[0].id;
    let wb = &svc.document(info.doc).unwrap().wb;
    assert_eq!(
        wb.value(CellAddr::new(s, 0, 0)),
        Value::Error(tally_types::ErrorKind::Ref)
    );
    assert_eq!(
        wb.value(CellAddr::new(s, 1, 0)),
        Value::Error(tally_types::ErrorKind::NA)
    );
    let edit = svc.cell_edit_info(info.doc, s, 0, 0).unwrap();
    assert!(
        edit.input.contains("file:///tmp/other.ods"),
        "{}",
        edit.input
    );
}
