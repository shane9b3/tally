//! The document's default font: written as the `Normal` style's font
//! (fonts[0]) and read back as `DocSettings::default_font`; cells in the
//! default font keep no family of their own.

#![allow(clippy::unwrap_used)] // tests may unwrap (docs/contributing.md)

mod common;

use common::*;
use tally_model::{CellContentData, CellData, CellRangeLocal, SheetData, StyleData, WorkbookData};
use tally_types::Value;

fn book(default_font: Option<&str>) -> WorkbookData {
    let mut bold = StyleData::default();
    bold.font.bold = true;
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
                    content: CellContentData::Value(Value::Number(2.0)),
                    style: Some(0),
                },
                CellData {
                    row: 2,
                    col: 0,
                    content: CellContentData::Value(Value::Number(3.0)),
                    style: Some(1),
                },
            ],
            merges: vec![CellRangeLocal {
                r0: 4,
                c0: 0,
                r1: 4,
                c1: 2,
            }],
            ..SheetData::default()
        }],
        styles: vec![bold, courier],
        ..WorkbookData::default()
    };
    wb.settings.default_font = default_font.map(str::to_string);
    wb
}

#[test]
fn default_font_round_trips() {
    let back = round_trip(&book(Some("Arial")));
    assert_eq!(back.settings.default_font.as_deref(), Some("Arial"));
    // Bold in the default font: no family of its own.
    let b = style(&back, 0, "A2").unwrap();
    assert!(b.font.bold);
    assert_eq!(b.font.family, None);
    let c = style(&back, 0, "A3").unwrap();
    assert_eq!(c.font.family.as_deref(), Some("Courier New"));
    // Once more: unchanged.
    let again = round_trip(&back);
    assert_eq!(again.settings.default_font.as_deref(), Some("Arial"));
    assert_eq!(style(&again, 0, "A2").unwrap().font.family, None);
}

#[test]
fn without_a_default_font_the_package_keeps_calibri() {
    let back = round_trip(&book(None));
    assert_eq!(back.settings.default_font.as_deref(), Some("Calibri"));
    assert_eq!(style(&back, 0, "A2").unwrap().font.family, None);
}

fn part(bytes: &[u8], name: &str) -> String {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes.to_vec())).unwrap();
    let mut f = z.by_name(name).unwrap();
    let mut s = String::new();
    std::io::Read::read_to_string(&mut f, &mut s).unwrap();
    s
}

/// `<col width>` of the first `<col>` element.
fn first_col_width(sheet_xml: &str) -> f64 {
    let at = sheet_xml.find("<col ").unwrap();
    let w = &sheet_xml[at..];
    let w = &w[w.find("width=\"").unwrap() + 7..];
    w[..w.find('"').unwrap()].parse().unwrap()
}

/// Review (MED-2): with an Arial default, `<col width>` counts in Arial
/// 11's 8 px digits (Excel draws the column at the same pixel width), and
/// reading it back gives the same width.
#[test]
fn column_widths_follow_the_default_font() {
    let mut wb = book(Some("Arial"));
    wb.sheets[0].col_widths = vec![tally_model::DimRun {
        start: 0,
        end: 0,
        size_hmm: 2646, // 100 px
        hidden: false,
    }];
    let arial = to_bytes(&wb);
    wb.settings.default_font = None;
    let calibri = to_bytes(&wb);
    let (wa, wc) = (
        first_col_width(&part(&arial, "xl/worksheets/sheet1.xml")),
        first_col_width(&part(&calibri, "xl/worksheets/sheet1.xml")),
    );
    // 100 px: (100 - 5) / 8 vs (100 - 5) / 7 digits (+ padding).
    assert!(wa < wc * 0.9, "{wa} vs {wc}");
    for bytes in [arial, calibri] {
        let back = tally_io_xlsx::read_xlsx(
            std::io::Cursor::new(bytes),
            &tally_io_xlsx::Limits::default(),
        )
        .unwrap();
        let w = back.sheets[0].col_widths[0].size_hmm;
        assert!((i64::from(w) - 2646).abs() <= 27, "{w}");
    }
}

/// Excel accepts font names of at most 31 characters; unstyled links use
/// the default font.
#[test]
fn long_font_names_and_links() {
    let long = "A Very Long Font Family Name That Excel Refuses";
    let mut wb = book(Some(long));
    wb.sheets[0].links = vec![tally_model::LinkData {
        row: 3,
        col: 0,
        url: "https://example.com/".into(),
    }];
    let bytes = to_bytes(&wb);
    let styles = part(&bytes, "xl/styles.xml");
    let cut: String = long.chars().take(31).collect();
    assert!(
        styles.contains(&format!("<name val=\"{cut}\"/>")),
        "{styles}"
    );
    assert!(!styles.contains(long));
    assert!(!styles.contains("Calibri"), "{styles}");
}
