//! Document fonts through the engine: the default font of a file reaches
//! the grid (as the family of unstyled styles), is kept on save, and is
//! used by print layout; Arial-specific metrics are checked only where
//! Arial is installed.
#![allow(clippy::unwrap_used)]
#![cfg(feature = "io")]

use std::path::PathBuf;

use tally_api::{DocumentService, FileFormat};
use tally_engine::fonts::{line_em, FontRef, INTER_LINE_EM};
use tally_engine::workbook::Workbook;
use tally_model::{CellContentData, CellData, SheetData, StyleData, WorkbookData};
use tally_types::Value;

fn book(default_font: Option<&str>) -> WorkbookData {
    let mut courier = StyleData::default();
    courier.font.family = Some("Courier New".into());
    let mut wb = WorkbookData {
        sheets: vec![SheetData {
            name: "Sheet1".into(),
            cells: vec![
                CellData {
                    row: 0,
                    col: 0,
                    content: CellContentData::Value(Value::Number(1.0)),
                    style: None,
                },
                CellData {
                    row: 1,
                    col: 0,
                    content: CellContentData::Value(Value::Text("mono".into())),
                    style: Some(0),
                },
            ],
            ..SheetData::default()
        }],
        styles: vec![courier],
        ..WorkbookData::default()
    };
    wb.settings.default_font = default_font.map(str::to_string);
    wb
}

fn file(name: &str, wb: &WorkbookData) -> PathBuf {
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let mut f = std::fs::File::create(&path).unwrap();
    tally_io_ods::write_ods(wb, &mut f).unwrap();
    path
}

#[test]
fn default_font_reaches_the_grid_and_survives_saving() {
    let mut svc = tally_engine::EngineService::with_default_codec();
    let info = svc
        .open(&file("fonts-default.ods", &book(Some("Arial"))))
        .unwrap();
    let doc = info.doc;
    // Style 0 (no direct formatting) and every style without a family
    // carry the document font; a style with its own family keeps it.
    let ids: Vec<u32> = {
        let d = svc.document(doc).unwrap();
        assert_eq!(d.wb.default_font.as_deref(), Some("Arial"));
        let sheet = d.wb.sheets[0].id;
        vec![0, d.wb.style_at(tally_types::CellAddr::new(sheet, 1, 0)).0]
    };
    let st = svc.styles(doc, &ids).unwrap();
    assert_eq!(st[0].font_family.as_deref(), Some("Arial"));
    assert_eq!(st[1].font_family.as_deref(), Some("Courier New"));
    // Saved and reopened: unchanged, and no family was baked into styles.
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("fonts-default-saved.ods");
    svc.save(doc, Some(&out), FileFormat::Ods).unwrap();
    let back = tally_io_ods::read_ods(
        std::fs::File::open(&out).unwrap(),
        &tally_io_ods::Limits::default(),
    )
    .unwrap();
    assert_eq!(back.settings.default_font.as_deref(), Some("Arial"));
    assert!(back
        .styles
        .iter()
        .all(|s| s.font.family.as_deref() != Some("Arial")));
}

#[test]
fn documents_without_a_font_keep_the_app_font() {
    let mut svc = tally_engine::EngineService::with_default_codec();
    let info = svc.open(&file("fonts-none.ods", &book(None))).unwrap();
    let st = svc.styles(info.doc, &[0]).unwrap();
    assert_eq!(st[0].font_family, None);
    let wb = Workbook::from_data(&book(None));
    assert_eq!(wb.default_font, None);
    assert_eq!(line_em(FontRef::default()), INTER_LINE_EM);
}

/// Where Arial is installed, a 13 pt Arial row prints at Arial's height
/// (AOO: 14.52 pt of text + 41 hmm), lower than Inter's.
#[test]
fn optimal_rows_use_the_document_font_metrics() {
    let arial = FontRef {
        family: Some("Arial"),
        ..FontRef::default()
    };
    if tally_io_pdf::resolve_font(
        &tally_io_pdf::FontSpec {
            family: Some("Arial".into()),
            ..tally_io_pdf::FontSpec::default()
        },
        true,
    )
    .family
        != "Arial"
    {
        eprintln!("Arial is not installed; skipped");
        return;
    }
    let mut data = book(Some("Arial"));
    let mut big = StyleData::default();
    big.font.size_pt = Some(13.0);
    data.styles.push(big);
    data.sheets[0].cells.push(CellData {
        row: 2,
        col: 0,
        content: CellContentData::Value(Value::Text("Unit base price".into())),
        style: Some(1),
    });
    let wb = Workbook::from_data(&data);
    let sheet = &wb.sheets[0];
    let h = tally_engine::print::optimal_row_heights(&wb, sheet, 0, 10);
    let expect = (13.0 * line_em(arial) * 2540.0 / 72.0).ceil() as u32 + 41;
    assert_eq!(h.get(&2), Some(&expect));
    assert!(expect < 596, "Inter would give 596 hmm");
}

/// Review (HIGH-2): many distinct fonts in wrapped cells — more names than
/// a font context resolves — lay out in bounded time: each style's font
/// is resolved once per operation and limits are sticky (no reload loop).
#[test]
fn many_fonts_lay_out_in_bounded_time() {
    let mut data = book(Some("Arial"));
    data.styles.clear();
    data.sheets[0].cells.clear();
    let text = "the quick brown fox jumps over the lazy dog again and again";
    for i in 0..400u32 {
        let mut st = StyleData::default();
        st.font.family = Some(format!("Tally No Such Family {i}"));
        st.font.bold = i % 2 == 0;
        st.align.wrap = true;
        st.font.size_pt = Some(12.0);
        data.styles.push(st);
    }
    for r in 0..5000u32 {
        data.sheets[0].cells.push(CellData {
            row: r,
            col: 0,
            content: CellContentData::Value(Value::Text(text.into())),
            style: Some(r % 400),
        });
    }
    let wb = Workbook::from_data(&data);
    let t = std::time::Instant::now();
    let h = tally_engine::print::optimal_row_heights(&wb, &wb.sheets[0], 0, 10_000);
    assert_eq!(h.len(), 5000);
    let mut m = tally_engine::fonts::Measurer::new();
    for r in 0..5000 {
        assert!(tally_engine::autofit::needed_height_px(&mut m, &wb, &wb.sheets[0], r).is_some());
    }
    assert!(t.elapsed().as_secs() < 20, "{:?}", t.elapsed());
}

/// Review: the UI shows the document font as the family of unstyled
/// cells; applying it (or clearing the family) must not bake it into the
/// styles, while another family is stored.
#[test]
fn applying_the_document_font_keeps_styles_clean() {
    let mut svc = tally_engine::EngineService::with_default_codec();
    let info = svc
        .open(&file("fonts-patch.ods", &book(Some("Arial"))))
        .unwrap();
    let sheet = info.sheets[0].id;
    let apply = |svc: &mut tally_engine::EngineService, family: &str, bold: bool| {
        let tx = tally_api::Transaction {
            ops: vec![tally_api::Op::ApplyStyle {
                sheet,
                ranges: vec![tally_api::RangeDto {
                    r0: 0,
                    c0: 0,
                    r1: 1,
                    c1: 0,
                }],
                patch: tally_api::StylePatch {
                    font_family: Some(family.into()),
                    bold: Some(bold),
                    ..tally_api::StylePatch::default()
                },
            }],
            label: None,
            selection_before: None,
            selection_after: None,
            group: None,
        };
        svc.apply(info.doc, tx).unwrap();
    };
    let family_at = |svc: &tally_engine::EngineService, r: u32| {
        let d = svc.document(info.doc).unwrap();
        let id = d.wb.style_at(tally_types::CellAddr::new(sheet, r, 0));
        d.wb.styles.get(id).data.font.family.clone()
    };
    apply(&mut svc, "Times New Roman", false);
    assert_eq!(family_at(&svc, 0).as_deref(), Some("Times New Roman"));
    apply(&mut svc, "arial", true);
    assert_eq!(family_at(&svc, 0), None);
    assert_eq!(family_at(&svc, 1), None);
    apply(&mut svc, "", false);
    assert_eq!(family_at(&svc, 0), None);
}
