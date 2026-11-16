//! Named cell styles and AutoFormat (wave 5, W5-2).
#![allow(clippy::unwrap_used)]

mod common;

use common::{range, Fixture};
use tally_api::{AutoFormatOptions, CellStyleDto, DocumentService, Op, StyleDto};
use tally_types::CellAddr;

fn style_of(f: &Fixture, a1: &str) -> StyleDto {
    let a = CellAddr::from_a1(f.sheet, a1).unwrap();
    let wb = &f.svc.document(f.doc).unwrap().wb;
    wb.styles.to_dto(wb.style_at(a))
}

#[test]
fn apply_heading_then_edit_the_style_updates_cells() {
    let mut f = Fixture::new();
    f.set("A1", "Title");
    let styles = f.svc.cell_styles(f.doc).unwrap();
    assert_eq!(styles[0].name, "Default");
    assert!(styles
        .iter()
        .any(|s| s.name == "Heading" && s.builtin && s.style.bold));
    let r = f.apply(vec![Op::ApplyCellStyle {
        sheet: f.sheet,
        ranges: vec![range("A1:A2")],
        name: "Heading".into(),
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    let s = style_of(&f, "A1");
    assert!(s.bold && s.italic);
    assert_eq!(s.style_name.as_deref(), Some("Heading"));
    // Direct formatting on top.
    f.apply(vec![Op::ApplyStyle {
        sheet: f.sheet,
        ranges: vec![range("A1")],
        patch: tally_api::StylePatch {
            fill: Some(0xffff_00ff),
            ..Default::default()
        },
    }]);
    // Redefine Heading: not italic, red; A1 keeps its fill, A2 follows.
    let mut heading = styles.iter().find(|s| s.name == "Heading").unwrap().clone();
    heading.style.italic = false;
    heading.style.color = Some(0xff00_00ff);
    let r = f.apply(vec![Op::DefineCellStyle { style: heading }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    let a1 = style_of(&f, "A1");
    assert!(a1.bold && !a1.italic);
    assert_eq!(a1.color, Some(0xff00_00ff));
    assert_eq!(a1.fill, Some(0xffff_00ff));
    let a2 = style_of(&f, "A2");
    assert!(!a2.italic && a2.fill.is_none());
    // Heading1 (child of Heading) follows the parent's change.
    let h1 = f
        .svc
        .cell_styles(f.doc)
        .unwrap()
        .into_iter()
        .find(|s| s.name == "Heading1")
        .unwrap();
    assert!(!h1.style.italic);
    assert_eq!(h1.style.rotation_deg, Some(90));
    f.svc.undo(f.doc).unwrap();
    assert!(style_of(&f, "A2").italic);
    f.svc.redo(f.doc).unwrap();
    assert!(!style_of(&f, "A2").italic);
}

#[test]
fn user_style_define_and_delete() {
    let mut f = Fixture::new();
    let mut dto = CellStyleDto {
        name: "Money".into(),
        parent: Some("Result".into()),
        builtin: false,
        style: f.svc.cell_styles(f.doc).unwrap()[1].style.clone(),
    };
    dto.style.number_format = "#,##0.00".into();
    f.apply(vec![Op::DefineCellStyle { style: dto }]);
    f.set("B1", "1234.5");
    f.apply(vec![Op::ApplyCellStyle {
        sheet: f.sheet,
        ranges: vec![range("B1")],
        name: "Money".into(),
    }]);
    assert_eq!(f.text("B1"), "1,234.50");
    assert!(style_of(&f, "B1").bold);
    let r = f.apply(vec![Op::DeleteCellStyle {
        name: "Heading".into(),
    }]);
    assert!(!r.errors.is_empty(), "built-ins cannot be deleted");
    f.apply(vec![Op::DeleteCellStyle {
        name: "Money".into(),
    }]);
    // The cell takes Default, not the parent (AOO
    // `ScAttrArray::FindStyleSheet` with bReset): no bold, General.
    let s = style_of(&f, "B1");
    assert!(matches!(s.style_name.as_deref(), None | Some("Default")));
    assert!(!s.bold && !s.italic && !s.underline);
    assert_eq!(f.text("B1"), "1234.5");
    assert!(!f
        .svc
        .cell_styles(f.doc)
        .unwrap()
        .iter()
        .any(|s| s.name == "Money"));
    // Round trip through the model keeps the table.
    let data = f.data();
    assert!(data.cell_styles.iter().all(|s| s.name != "Money"));
}

/// Parity G8 (review 4 L10). Deleting a style: its cells take Default and
/// keep their direct formatting; its children move to its parent and lose
/// what they inherited from it (AOO `SfxStyleSheetBasePool::Remove` →
/// `ChangeParent`); undo brings the style and the children back, the
/// cells stay on Default (AOO `ScUndoModifyStyle` restores the style only).
#[test]
fn delete_style_moves_cells_to_default_and_children_to_the_parent() {
    let mut f = Fixture::new();
    let styles = f.svc.cell_styles(f.doc).unwrap();
    let result = styles.iter().find(|s| s.name == "Result").unwrap().clone();
    // Mid (parent Result): red text on top of Result's bold/italic/underline.
    let mut mid = CellStyleDto {
        name: "Mid".into(),
        parent: Some("Result".into()),
        builtin: false,
        style: result.style.clone(),
    };
    mid.style.color = Some(0xff00_00ff);
    f.apply(vec![Op::DefineCellStyle { style: mid.clone() }]);
    // Leaf (parent Mid): adds a fill.
    let mut leaf = CellStyleDto {
        name: "Leaf".into(),
        parent: Some("Mid".into()),
        builtin: false,
        style: mid.style.clone(),
    };
    leaf.style.fill = Some(0x00ff_00ff);
    f.apply(vec![Op::DefineCellStyle { style: leaf }]);
    for (cell, name) in [("A1", "Mid"), ("A2", "Mid"), ("A3", "Leaf")] {
        f.set(cell, "1");
        f.apply(vec![Op::ApplyCellStyle {
            sheet: f.sheet,
            ranges: vec![range(cell)],
            name: name.into(),
        }]);
    }
    // Direct formatting on A2.
    f.apply(vec![Op::ApplyStyle {
        sheet: f.sheet,
        ranges: vec![range("A2")],
        patch: tally_api::StylePatch {
            fill: Some(0xffff_00ff),
            ..Default::default()
        },
    }]);

    let r = f.apply(vec![Op::DeleteCellStyle { name: "Mid".into() }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);

    // Cells of the deleted style: Default, not Result.
    let a1 = style_of(&f, "A1");
    assert!(matches!(a1.style_name.as_deref(), None | Some("Default")));
    assert!(!a1.bold && !a1.italic && !a1.underline);
    assert_eq!(a1.color, None);
    // Direct formatting survives.
    let a2 = style_of(&f, "A2");
    assert!(matches!(a2.style_name.as_deref(), None | Some("Default")));
    assert!(!a2.bold);
    assert_eq!(a2.fill, Some(0xffff_00ff));
    // The child hangs under Result now, without Mid's red.
    let styles = f.svc.cell_styles(f.doc).unwrap();
    let leaf = styles.iter().find(|s| s.name == "Leaf").unwrap();
    assert_eq!(leaf.parent.as_deref(), Some("Result"));
    assert!(leaf.style.bold);
    assert_eq!(leaf.style.color, None);
    assert_eq!(leaf.style.fill, Some(0x00ff_00ff));
    let a3 = style_of(&f, "A3");
    assert_eq!(a3.style_name.as_deref(), Some("Leaf"));
    assert!(a3.bold);
    assert_eq!(a3.color, None);
    assert_eq!(a3.fill, Some(0x00ff_00ff));

    f.svc.undo(f.doc).unwrap();
    let styles = f.svc.cell_styles(f.doc).unwrap();
    assert!(styles.iter().any(|s| s.name == "Mid"));
    let leaf = styles.iter().find(|s| s.name == "Leaf").unwrap();
    assert_eq!(leaf.parent.as_deref(), Some("Mid"));
    assert_eq!(leaf.style.color, Some(0xff00_00ff));
    assert_eq!(style_of(&f, "A3").color, Some(0xff00_00ff));
    let a1 = style_of(&f, "A1");
    assert!(matches!(a1.style_name.as_deref(), None | Some("Default")));
}

/// Parity G8 (review 4 L15), a deviation that stays: `Result2` carries
/// the en-US currency format in every document locale. AOO takes the
/// office locale's default currency format the first time the style's
/// items are read (`ScStyleSheet::GetItemSet`, `HID_SC_SHEET_CELL_ERG1`).
#[test]
fn result2_currency_format_does_not_follow_the_locale() {
    let mut f = Fixture::new();
    f.svc.set_locale(f.doc, "de-DE").unwrap();
    let styles = f.svc.cell_styles(f.doc).unwrap();
    let r2 = styles.iter().find(|s| s.name == "Result2").unwrap();
    assert_eq!(r2.parent.as_deref(), Some("Result"));
    assert_eq!(
        r2.style.number_format,
        "[$$-409]#,##0.00;[RED]-[$$-409]#,##0.00"
    );
}

#[test]
fn autoformat_blue_preset() {
    let mut f = Fixture::new();
    for r in 1..=5 {
        for c in ["A", "B", "C", "D"] {
            f.set(&format!("{c}{r}"), "1");
        }
    }
    let r = f.apply(vec![Op::ApplyAutoFormat {
        sheet: f.sheet,
        range: range("A1:D5"),
        preset: "Blue".into(),
        options: AutoFormatOptions::default(),
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    let header = style_of(&f, "B1");
    assert_eq!(header.fill, Some(0x0000_80ff));
    assert!(header.bold);
    assert_eq!(header.color, Some(0xffff_ffff));
    let body = style_of(&f, "B3");
    assert_eq!(body.fill, Some(0xccff_ffff));
    assert!(body.borders.top.is_some());
    let total = style_of(&f, "B5");
    assert_eq!(total.fill, Some(0x0000_80ff));
    let r = f.apply(vec![Op::ApplyAutoFormat {
        sheet: f.sheet,
        range: range("A1:D5"),
        preset: "Nope".into(),
        options: AutoFormatOptions::default(),
    }]);
    assert!(!r.errors.is_empty());
    f.svc.undo(f.doc).unwrap();
    assert!(style_of(&f, "B1").fill.is_none());
}

fn fill_patch(fill: u32, bold: bool) -> tally_api::StylePatch {
    let mut p: tally_api::StylePatch = serde_json::from_str(
        r#"{"bold":null,"italic":null,"underline":null,"strike":null,"fontFamily":null,"fontSizePt":null,"color":null,"fill":null,"hAlign":null,"vAlign":null,"wrap":null,"numberFormat":null}"#,
    )
    .unwrap();
    p.fill = Some(fill);
    p.bold = Some(bold);
    p
}

#[test]
fn custom_auto_format_lays_the_16_slots_over_the_range() {
    let mut f = Fixture::new();
    let cells: Vec<_> = (0..16u32)
        .map(|i| fill_patch(((i + 1) << 8) | 0xff, i < 4))
        .collect();
    let r = f.apply(vec![Op::ApplyAutoFormatCustom {
        sheet: f.sheet,
        range: range("B2:G9"),
        cells,
        autofit: false,
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    let fill = |a1: &str| style_of(&f, a1).fill;
    let slot = |n: u32| Some(((n + 1) << 8) | 0xff);
    assert_eq!(fill("B2"), slot(0));
    assert!(style_of(&f, "C2").bold);
    assert_eq!(fill("G2"), slot(3));
    assert_eq!(fill("C3"), slot(5)); // odd inner row, odd inner column
    assert_eq!(fill("D3"), slot(6)); // odd row, even column
    assert_eq!(fill("C4"), slot(9)); // even row, odd column
    assert_eq!(fill("G9"), slot(15));
    assert_eq!(fill("A1"), None);
    // too small, wrong slot count
    let bad = f.apply(vec![Op::ApplyAutoFormatCustom {
        sheet: f.sheet,
        range: range("A1:B5"),
        cells: vec![fill_patch(1, false); 16],
        autofit: false,
    }]);
    assert!(!bad.errors.is_empty());
    let bad = f.apply(vec![Op::ApplyAutoFormatCustom {
        sheet: f.sheet,
        range: range("A1:C5"),
        cells: vec![fill_patch(1, false); 3],
        autofit: false,
    }]);
    assert!(!bad.errors.is_empty());
}
