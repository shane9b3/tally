//! Load Styles from another document (`ScDocShell::LoadStylesArgs`).
#![allow(clippy::unwrap_used)]

mod common;

use common::{range, Fixture};
use tally_api::{
    ApiError, CellStyleDto, DocumentService, LoadStylesOptions, Op, Orientation, StyleDto,
};
use tally_engine::loadstyles::StyleSource;
use tally_types::CellAddr;

const ALL: LoadStylesOptions = LoadStylesOptions {
    cell_styles: true,
    page_styles: true,
    overwrite: true,
};
const MERGE: LoadStylesOptions = LoadStylesOptions {
    cell_styles: true,
    page_styles: true,
    overwrite: false,
};

fn style_of(f: &Fixture, a1: &str) -> StyleDto {
    let a = CellAddr::from_a1(f.sheet, a1).unwrap();
    let wb = &f.svc.document(f.doc).unwrap().wb;
    wb.styles.to_dto(wb.style_at(a))
}

fn styles(f: &Fixture) -> Vec<CellStyleDto> {
    f.svc.cell_styles(f.doc).unwrap()
}

fn named(f: &Fixture, name: &str) -> Option<CellStyleDto> {
    styles(f).into_iter().find(|s| s.name == name)
}

fn define(f: &mut Fixture, name: &str, parent: &str, edit: impl FnOnce(&mut StyleDto)) {
    let mut base = named(f, parent).unwrap().style;
    edit(&mut base);
    let r = f.apply(vec![Op::DefineCellStyle {
        style: CellStyleDto {
            name: name.into(),
            parent: Some(parent.into()),
            builtin: false,
            style: base,
        },
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
}

/// A source document: Heading redefined (red, no italic), a new "Base"
/// with a child "Child", "Pct" with a number format, and a landscape
/// first sheet with a footer of its own.
fn source() -> StyleSource {
    let mut s = Fixture::new();
    let mut heading = named(&s, "Heading").unwrap();
    heading.style.italic = false;
    heading.style.color = Some(0xff00_00ff);
    s.apply(vec![Op::DefineCellStyle { style: heading }]);
    define(&mut s, "Base", "Default", |st| st.fill = Some(0x00ff_00ff));
    define(&mut s, "Child", "Base", |st| st.bold = true);
    define(&mut s, "Pct", "Default", |st| {
        st.number_format = "0.000%".into();
    });
    define(&mut s, "Sub", "Heading", |st| st.strike = true);
    let mut p = s.svc.page_setup(s.doc, s.sheet).unwrap();
    p.orientation = Orientation::Landscape;
    p.print_gridlines = true;
    let r = s.apply(vec![Op::SetPageSetup {
        sheet: s.sheet,
        setup: p,
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    StyleSource::from_workbook(&s.svc.document(s.doc).unwrap().wb)
}

fn load(f: &mut Fixture, src: &StyleSource, opts: LoadStylesOptions) -> tally_api::ApplyResult {
    f.svc
        .document_mut(f.doc)
        .unwrap()
        .load_styles(src, &opts, Some("Load Styles"))
        .unwrap()
}

#[test]
fn without_overwrite_only_new_names_are_added() {
    let src = source();
    let mut f = Fixture::new();
    f.set("A1", "Title");
    f.apply(vec![Op::ApplyCellStyle {
        sheet: f.sheet,
        ranges: vec![range("A1")],
        name: "Heading".into(),
    }]);
    let r = load(&mut f, &src, MERGE);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    // Heading exists here (a built-in): untouched.
    let h = named(&f, "Heading").unwrap();
    assert!(h.style.italic);
    assert_eq!(h.style.color, None);
    assert!(style_of(&f, "A1").italic);
    // New names arrive with their parents.
    let child = named(&f, "Child").unwrap();
    assert_eq!(child.parent.as_deref(), Some("Base"));
    assert!(child.style.bold);
    assert_eq!(child.style.fill, Some(0x00ff_00ff));
    // A new style on a parent that was not replaced: its own attribute
    // (strike) from the source, the inherited ones from Heading here.
    let sub = named(&f, "Sub").unwrap();
    assert_eq!(sub.parent.as_deref(), Some("Heading"));
    assert!(sub.style.strike && sub.style.italic);
    assert_eq!(sub.style.color, None);
    // The page style is not copied without Overwrite.
    let p = f.svc.page_setup(f.doc, f.sheet).unwrap();
    assert_eq!(p.orientation, Orientation::Portrait);
    // Loading again changes nothing (no undo step).
    let before = f.svc.undo_history(f.doc).unwrap().undo.len();
    load(&mut f, &src, MERGE);
    assert_eq!(f.svc.undo_history(f.doc).unwrap().undo.len(), before);
}

#[test]
fn overwrite_replaces_styles_and_cells_follow() {
    let src = source();
    let mut f = Fixture::new();
    f.set("A1", "Title");
    f.apply(vec![Op::ApplyCellStyle {
        sheet: f.sheet,
        ranges: vec![range("A1:A2")],
        name: "Heading".into(),
    }]);
    // Direct formatting on A1 stays on top of the new definition.
    f.apply(vec![Op::ApplyStyle {
        sheet: f.sheet,
        ranges: vec![range("A1")],
        patch: tally_api::StylePatch {
            fill: Some(0xffff_00ff),
            ..Default::default()
        },
    }]);
    // A style of this document based on Heading follows it.
    define(&mut f, "Mine", "Heading", |st| st.underline = true);
    let r = load(&mut f, &src, ALL);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    let h = named(&f, "Heading").unwrap();
    assert!(!h.style.italic);
    assert_eq!(h.style.color, Some(0xff00_00ff));
    let a1 = style_of(&f, "A1");
    assert!(!a1.italic && a1.bold);
    assert_eq!(a1.color, Some(0xff00_00ff));
    assert_eq!(a1.fill, Some(0xffff_00ff));
    assert_eq!(a1.style_name.as_deref(), Some("Heading"));
    let a2 = style_of(&f, "A2");
    assert!(!a2.italic && a2.fill.is_none());
    let mine = named(&f, "Mine").unwrap();
    assert!(mine.style.underline && !mine.style.italic);
    assert_eq!(mine.style.color, Some(0xff00_00ff));
    // Heading1 (built-in child of Heading) comes from the source, based
    // on the new Heading.
    let h1 = named(&f, "Heading1").unwrap();
    assert!(!h1.style.italic);
    assert!(!r.changed.is_empty());
}

#[test]
fn number_formats_travel_as_codes() {
    let src = source();
    let mut f = Fixture::new();
    f.set("B1", "0.25");
    load(&mut f, &src, MERGE);
    assert_eq!(named(&f, "Pct").unwrap().style.number_format, "0.000%");
    f.apply(vec![Op::ApplyCellStyle {
        sheet: f.sheet,
        ranges: vec![range("B1")],
        name: "Pct".into(),
    }]);
    let block = f.svc.cells(f.doc, f.sheet, &[range("B1")]).unwrap();
    assert_eq!(block[0].cells[0].text, "25.000%");
}

#[test]
fn page_style_with_overwrite_keeps_sheet_attributes() {
    let src = source();
    let mut f = Fixture::new();
    let second = f
        .svc
        .apply(
            f.doc,
            common::tx(vec![Op::AddSheet {
                index: None,
                name: Some("Two".into()),
            }]),
        )
        .unwrap();
    let two = second.info.unwrap().sheets[1].id;
    let mut p = f.svc.page_setup(f.doc, f.sheet).unwrap();
    p.print_range = Some(range("A1:C9"));
    p.repeat_rows = Some([0, 0]);
    f.apply(vec![Op::SetPageSetup {
        sheet: f.sheet,
        setup: p,
    }]);
    let r = load(
        &mut f,
        &src,
        LoadStylesOptions {
            cell_styles: false,
            page_styles: true,
            overwrite: true,
        },
    );
    assert!(r.layout_changed.contains(&f.sheet) && r.layout_changed.contains(&two));
    for s in [f.sheet, two] {
        let p = f.svc.page_setup(f.doc, s).unwrap();
        assert_eq!(p.orientation, Orientation::Landscape);
        assert!(p.print_gridlines);
    }
    let p = f.svc.page_setup(f.doc, f.sheet).unwrap();
    assert_eq!(p.print_range, Some(range("A1:C9")));
    assert_eq!(p.repeat_rows, Some([0, 0]));
    // Cell styles were not asked for.
    assert!(named(&f, "Base").is_none());
}

#[test]
fn one_undo_step_restores_styles_cells_and_pages() {
    let src = source();
    let mut f = Fixture::new();
    f.set("A1", "Title");
    f.apply(vec![Op::ApplyCellStyle {
        sheet: f.sheet,
        ranges: vec![range("A1")],
        name: "Heading".into(),
    }]);
    let before = styles(&f);
    let r = load(&mut f, &src, ALL);
    assert_eq!(r.undo.undo_label.as_deref(), Some("Load Styles"));
    assert!(f.svc.document(f.doc).unwrap().dirty);
    f.svc.undo(f.doc).unwrap();
    assert_eq!(styles(&f), before);
    assert!(style_of(&f, "A1").italic);
    let p = f.svc.page_setup(f.doc, f.sheet).unwrap();
    assert_eq!(p.orientation, Orientation::Portrait);
    f.svc.redo(f.doc).unwrap();
    assert!(named(&f, "Child").is_some());
    assert!(!style_of(&f, "A1").italic);
    let p = f.svc.page_setup(f.doc, f.sheet).unwrap();
    assert_eq!(p.orientation, Orientation::Landscape);
}

#[test]
fn hostile_sources_are_contained() {
    use tally_model::{CellStyleData, StyleData};
    let data = |name: &str, parent: Option<&str>| CellStyleData {
        name: name.into(),
        parent: parent.map(Into::into),
        style: StyleData::default(),
    };
    // A parent cycle, a missing parent, a self-parent and a bad name.
    let src = StyleSource {
        cell_styles: vec![
            data("X", Some("Y")),
            data("Y", Some("X")),
            data("Z", Some("Nowhere")),
            data("S", Some("S")),
            data("bad\nname", None),
        ],
        page_setup: None,
    };
    let mut f = Fixture::new();
    let r = load(&mut f, &src, ALL);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    let all = styles(&f);
    assert!(all.iter().all(|s| s.name != "bad\nname"));
    assert_eq!(named(&f, "Z").unwrap().parent.as_deref(), Some("Default"));
    assert_eq!(named(&f, "S").unwrap().parent.as_deref(), Some("Default"));
    // The cycle is cut: one of X / Y hangs off Default.
    let x = named(&f, "X").unwrap();
    let y = named(&f, "Y").unwrap();
    assert!(x.parent.as_deref() == Some("Default") || y.parent.as_deref() == Some("Default"));
}

#[test]
fn read_only_documents_and_unknown_templates_are_refused() {
    let mut svc = tally_engine::EngineService::new();
    let info = svc.new_workbook(None).unwrap();
    let e = svc
        .load_styles_from_template(info.doc, "nope", &ALL, None)
        .unwrap_err();
    assert_eq!(e.code, ApiError::NOT_FOUND);
    let r = svc
        .load_styles_from_template(info.doc, "invoices", &MERGE, Some("Load Styles"))
        .unwrap();
    assert!(r.errors.is_empty());
    let names: Vec<String> = svc
        .cell_styles(info.doc)
        .unwrap()
        .into_iter()
        .map(|s| s.name)
        .collect();
    assert!(names.iter().any(|n| n == "Table Header"), "{names:?}");
    svc.document_mut(info.doc).unwrap().read_only = true;
    let e = svc
        .load_styles_from_template(info.doc, "budget", &ALL, None)
        .unwrap_err();
    assert_eq!(e.code, ApiError::READ_ONLY);
}

#[cfg(feature = "io")]
mod files {
    use super::*;
    use tally_api::FileFormat;
    use tally_engine::EngineService;

    fn scratch(name: &str) -> std::path::PathBuf {
        let d =
            std::env::temp_dir().join(format!("tally-loadstyles-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Styles saved in ODS and XLSX come back through the readers.
    #[test]
    fn from_ods_and_xlsx_files() {
        let dir = scratch("files");
        for (ext, fmt) in [("ods", FileFormat::Ods), ("xlsx", FileFormat::Xlsx)] {
            let mut svc = EngineService::with_default_codec();
            let src = svc.new_workbook(None).unwrap();
            let mut base = svc.cell_styles(src.doc).unwrap()[0].style.clone();
            base.bold = true;
            base.number_format = "0.0".into();
            let tx = common::tx(vec![Op::DefineCellStyle {
                style: CellStyleDto {
                    name: "Bold One".into(),
                    parent: Some("Default".into()),
                    builtin: false,
                    style: base,
                },
            }]);
            svc.apply(src.doc, tx).unwrap();
            let path = dir.join(format!("src.{ext}"));
            svc.save(src.doc, Some(&path), fmt).unwrap();
            let dst = svc.new_workbook(None).unwrap();
            let r = svc
                .load_styles(dst.doc, &path, &MERGE, Some("Load Styles"))
                .unwrap();
            assert!(r.errors.is_empty(), "{ext}: {:?}", r.errors);
            let got = svc
                .cell_styles(dst.doc)
                .unwrap()
                .into_iter()
                .find(|s| s.name == "Bold One")
                .unwrap_or_else(|| panic!("{ext}: no style"));
            // Tally's XLSX writer keeps only the style names (the cells
            // carry the attributes); the ODS writer keeps the attributes
            // and, since Load Styles, the number format.
            if ext == "ods" {
                assert!(got.style.bold, "{ext}");
                assert_eq!(got.style.number_format, "0.0", "{ext}");
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_and_bogus_files_fail_cleanly() {
        let dir = scratch("bogus");
        let mut svc = EngineService::with_default_codec();
        let dst = svc.new_workbook(None).unwrap();
        assert!(svc
            .load_styles(dst.doc, &dir.join("missing.ods"), &ALL, None)
            .is_err());
        let junk = dir.join("junk.ods");
        std::fs::write(&junk, b"PK\x03\x04 not a zip").unwrap();
        let e = svc.load_styles(dst.doc, &junk, &ALL, None).unwrap_err();
        assert_ne!(e.code, ApiError::INTERNAL);
        // Nothing changed, nothing to undo.
        assert!(svc.undo_history(dst.doc).unwrap().undo.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
