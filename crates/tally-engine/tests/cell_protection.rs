//! Format Cells ▸ Cell Protection at AOO 4.1 parity (`ScProtectionAttr`):
//! "Hide all" empties the cell view, the input line, copies, CSV text and
//! search while the sheet is protected and implies "Protected"; "Hide
//! formula" empties the input line and the formula view; the attributes
//! reach the style DTO and undo. Printing is covered in
//! `service/print.rs` (`hide_print_tests`).
#![allow(clippy::unwrap_used)]

mod common;

use common::{input, range, sel, Fixture};
use tally_api::{
    cell_flags, CellProtectionDto, CellView, CellsOptions, DocumentService, FindMode, FindQuery,
    Op, PasteOptions, PastePayload,
};
use tally_types::{CellAddr, Value};

fn view(f: &Fixture, a1: &str) -> Option<CellView> {
    let a = CellAddr::from_a1(f.sheet, a1).unwrap();
    f.svc.cells(f.doc, f.sheet, &[range(a1)]).unwrap()[0]
        .cells
        .iter()
        .find(|c| (c.r, c.c) == (a.row, a.col))
        .cloned()
}

fn edit_input(f: &Fixture, a1: &str) -> String {
    let a = CellAddr::from_a1(f.sheet, a1).unwrap();
    f.svc
        .cell_edit_info(f.doc, f.sheet, a.row, a.col)
        .unwrap()
        .input
}

fn set_prot(
    f: &mut Fixture,
    a1: &str,
    locked: Option<bool>,
    hidden_formula: Option<bool>,
    hidden: Option<bool>,
    hide_print: Option<bool>,
) {
    let res = f.apply(vec![Op::SetCellProtection {
        sheet: f.sheet,
        ranges: vec![range(a1)],
        locked,
        hidden_formula,
        hidden,
        hide_print,
    }]);
    assert!(res.errors.is_empty(), "{:?}", res.errors);
}

fn protect(f: &mut Fixture, on: bool) {
    let op = if on {
        Op::ProtectSheet {
            sheet: f.sheet,
            password: None,
            options: None,
        }
    } else {
        Op::UnprotectSheet {
            sheet: f.sheet,
            password: None,
        }
    };
    let res = f.apply(vec![op]);
    assert!(res.errors.is_empty(), "{:?}", res.errors);
}

#[test]
fn hide_all_empties_the_view_only_while_the_sheet_is_protected() {
    let mut f = Fixture::new();
    f.set("A1", "secret");
    f.set("A2", "=40+2");
    f.set("B1", "visible");
    f.apply(vec![Op::SetLink {
        sheet: f.sheet,
        r: 0,
        c: 0,
        url: "https://example.com/".into(),
    }]);
    f.set("C1", "=A2*2");
    set_prot(&mut f, "A1:A2", None, None, Some(true), None);
    // Not protected: shown.
    assert_eq!(view(&f, "A1").unwrap().text, "secret");
    assert_eq!(view(&f, "A2").unwrap().text, "42");
    assert_eq!(edit_input(&f, "A1"), "secret");
    protect(&mut f, true);
    for a1 in ["A1", "A2"] {
        let v = view(&f, a1).unwrap();
        assert_eq!(v.text, "", "{a1}");
        assert_eq!(v.kind, tally_api::ValueKind::Empty);
        assert_eq!(v.num, None);
        assert_eq!(v.link, None);
        assert_ne!(v.flags & cell_flags::HIDDEN, 0);
        assert_eq!(v.flags & (cell_flags::FORMULA | cell_flags::LINK), 0);
        // The input line is empty too (AOO `UpdateInputHandler`).
        assert_eq!(edit_input(&f, a1), "", "{a1}");
    }
    assert_eq!(view(&f, "B1").unwrap().text, "visible");
    assert_eq!(view(&f, "B1").unwrap().flags & cell_flags::HIDDEN, 0);
    // Formula view and value highlighting leave it empty.
    let opts = CellsOptions {
        show_formulas: true,
        value_highlighting: true,
        hide_zero_values: false,
    };
    let b = f
        .svc
        .cells_with(f.doc, f.sheet, &[range("A2")], &opts)
        .unwrap();
    assert_eq!(b[0].cells[0].text, "");
    assert_eq!(b[0].cells[0].color, None);
    // The values are still there for formulas.
    assert_eq!(f.value("C1"), Value::Number(84.0));
    protect(&mut f, false);
    assert_eq!(view(&f, "A1").unwrap().text, "secret");
    assert_eq!(edit_input(&f, "A2"), "=40+2");
}

#[test]
fn hide_all_implies_protected() {
    let mut f = Fixture::new();
    f.set("A1", "x");
    // Unlocked but hidden: still refused on a protected sheet (AOO
    // HASATTR_PROTECTED is GetProtection() || GetHideCell()).
    set_prot(&mut f, "A1:B1", Some(false), None, None, None);
    set_prot(&mut f, "A1", None, None, Some(true), None);
    protect(&mut f, true);
    let res = f.apply(vec![input(f.sheet, "A1", "y")]);
    assert_eq!(res.errors.len(), 1, "hidden cells are protected");
    let res = f.apply(vec![input(f.sheet, "B1", "y")]);
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    // Tab skips it.
    let next = f
        .svc
        .next_unlocked(
            f.doc,
            f.sheet,
            tally_api::CellPos { r: 0, c: 1 },
            tally_api::Direction::Right,
            None,
        )
        .unwrap();
    assert_eq!(next, Some(tally_api::CellPos { r: 0, c: 1 }));
}

#[test]
fn hide_formula_empties_the_input_line_and_formula_view() {
    let mut f = Fixture::new();
    f.set("A1", "=6*7");
    f.set("A2", "plain");
    set_prot(&mut f, "A1:A2", None, Some(true), None, None);
    protect(&mut f, true);
    assert_eq!(edit_input(&f, "A1"), "");
    // Constants are not formulas: still shown.
    assert_eq!(edit_input(&f, "A2"), "plain");
    // The result is shown...
    assert_eq!(view(&f, "A1").unwrap().text, "42");
    // ...but not in formula view (AOO `FillInfo`: bFormulaMode && bHideFormula).
    let opts = CellsOptions {
        show_formulas: true,
        value_highlighting: false,
        hide_zero_values: false,
    };
    let b = f
        .svc
        .cells_with(f.doc, f.sheet, &[range("A1:A2")], &opts)
        .unwrap();
    let texts: Vec<&str> = b[0].cells.iter().map(|v| v.text.as_str()).collect();
    assert_eq!(texts, ["", "plain"]);
    protect(&mut f, false);
    assert_eq!(edit_input(&f, "A1"), "=6*7");
}

#[test]
fn copy_from_a_protected_sheet_removes_hidden_content() {
    let mut f = Fixture::new();
    f.set("A1", "secret");
    f.set("B1", "=2+3");
    f.set("C1", "open");
    f.apply(vec![Op::SetNote {
        sheet: f.sheet,
        r: 0,
        c: 0,
        note: tally_api::NoteDto {
            r: 0,
            c: 0,
            text: "hidden note".into(),
            author: None,
            created: None,
            shown: false,
        },
    }]);
    set_prot(&mut f, "A1", None, None, Some(true), None);
    set_prot(&mut f, "B1", None, Some(true), None, None);
    // Unprotected: everything is copied.
    let out = f.svc.copy(f.doc, &sel(f.sheet, "A1:C1")).unwrap();
    assert_eq!(out.tsv, "secret\t5\topen\n");
    protect(&mut f, true);
    let out = f.svc.copy(f.doc, &sel(f.sheet, "A1:C1")).unwrap();
    // AOO `ScColumn::RemoveProtected`: hidden cells lose their content,
    // hidden formulas become values.
    assert_eq!(out.tsv, "\t5\topen\n");
    assert!(!out.html.contains("secret"));
    let info = f.svc.new_workbook(None).unwrap();
    let doc2 = info.doc;
    let s2 = info.sheets[0].id;
    let res = f
        .svc
        .apply(
            doc2,
            common::tx(vec![Op::PasteSpecial {
                sheet: s2,
                target: range("A1"),
                payload: PastePayload::Internal { token: out.token },
                options: PasteOptions::default(),
            }]),
        )
        .unwrap();
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let ei = |r: u32, c: u32| f.svc.cell_edit_info(doc2, s2, r, c).unwrap().input;
    assert_eq!(ei(0, 0), "");
    assert_eq!(ei(0, 1), "5", "the formula became its value");
    assert_eq!(ei(0, 2), "open");
    let wb = &f.svc.document(doc2).unwrap().wb;
    assert!(
        wb.sheet(s2).unwrap().meta.notes.is_empty(),
        "the note of a hidden cell is not copied"
    );
}

#[test]
fn find_skips_hidden_cells_of_a_protected_sheet() {
    let mut f = Fixture::new();
    f.set("A1", "needle");
    f.set("A2", "needle");
    set_prot(&mut f, "A1", None, None, Some(true), None);
    let q = FindQuery {
        text: "needle".into(),
        sheet: None,
        in_formulas: false,
        match_case: false,
        whole_cell: false,
        mode: FindMode::Literal,
        from: None,
        in_notes: None,
        selection: None,
    };
    assert_eq!(f.svc.find(f.doc, &q).unwrap().hits.len(), 2);
    protect(&mut f, true);
    let hits = f.svc.find(f.doc, &q).unwrap().hits;
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].r, 1);
    let qf = FindQuery {
        in_formulas: true,
        ..q
    };
    assert_eq!(f.svc.find(f.doc, &qf).unwrap().hits.len(), 1);
}

#[test]
fn the_attributes_reach_the_style_and_undo() {
    let mut f = Fixture::new();
    f.set("A1", "x");
    let style_of = |f: &Fixture| {
        let id = view(f, "A1").unwrap().style;
        f.svc.styles(f.doc, &[id]).unwrap()[0].protection
    };
    assert_eq!(style_of(&f), None);
    let before = f.data();
    set_prot(
        &mut f,
        "A1",
        Some(false),
        Some(true),
        Some(true),
        Some(true),
    );
    assert_eq!(
        style_of(&f),
        Some(CellProtectionDto {
            locked: false,
            hidden_formula: true,
            hidden: true,
            hide_print: true,
        })
    );
    let after = f.data();
    let p = after.styles.iter().find_map(|s| s.protection).unwrap();
    assert!(p.hidden && p.hide_print && p.hidden_formula && !p.locked);
    f.svc.undo(f.doc).unwrap();
    assert_eq!(f.data(), before);
    f.svc.redo(f.doc).unwrap();
    assert_eq!(f.data(), after);
    // `None` leaves an attribute unchanged.
    set_prot(&mut f, "A1", None, None, Some(false), None);
    assert_eq!(
        style_of(&f),
        Some(CellProtectionDto {
            locked: false,
            hidden_formula: true,
            hidden: false,
            hide_print: true,
        })
    );
    // Refused while protected.
    protect(&mut f, true);
    let res = f.apply(vec![Op::SetCellProtection {
        sheet: f.sheet,
        ranges: vec![range("A1")],
        locked: None,
        hidden_formula: None,
        hidden: None,
        hide_print: Some(false),
    }]);
    assert_eq!(res.errors.len(), 1);
}

#[test]
fn hide_print_cells_still_count_for_the_print_area() {
    // AOO `ScTable::GetPrintArea` does not look at the protection attribute.
    let mut f = Fixture::new();
    f.set("A1", "a");
    f.set("D9", "x");
    set_prot(&mut f, "D9", None, None, None, Some(true));
    let breaks = f.svc.page_breaks(f.doc, f.sheet).unwrap();
    let area = breaks.area.unwrap();
    assert_eq!((area.r1, area.c1), (8, 3));
}

#[test]
fn absent_fields_leave_each_cells_flags_as_they_are() {
    // A1 has Hide when printing, A2 not; setting Hide formula on A1:A2
    // with the other fields absent keeps A1's flag (the dialog's
    // indeterminate boxes).
    let mut f = Fixture::new();
    set_prot(&mut f, "A1", None, None, None, Some(true));
    set_prot(&mut f, "A1:A2", None, Some(true), None, None);
    let prot = |f: &Fixture, a1: &str| {
        let id = view(f, a1).unwrap().style;
        f.svc.styles(f.doc, &[id]).unwrap()[0].protection.unwrap()
    };
    let a1 = prot(&f, "A1");
    let a2 = prot(&f, "A2");
    assert!(a1.hide_print && a1.hidden_formula && a1.locked && !a1.hidden);
    assert!(!a2.hide_print && a2.hidden_formula && a2.locked && !a2.hidden);
}
