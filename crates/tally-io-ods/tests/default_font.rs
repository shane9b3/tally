//! The document's default font: `style:default-style` (family
//! `table-cell`) and the `Default` cell style, resolved through the
//! font-face declarations, read into `DocSettings::default_font` and
//! written back unchanged.
#![allow(clippy::unwrap_used)]

use std::io::Cursor;

use tally_io_ods::{read_ods, write_ods, Limits};
use tally_model::{CellContentData, CellData, SheetData, StyleData, WorkbookData};
use tally_types::Value;

fn book(default_font: Option<&str>) -> WorkbookData {
    let mut times = StyleData::default();
    times.font.family = Some("Times New Roman".into());
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
            ],
            ..SheetData::default()
        }],
        styles: vec![times],
        ..WorkbookData::default()
    };
    wb.settings.default_font = default_font.map(str::to_string);
    wb
}

fn write(wb: &WorkbookData) -> Vec<u8> {
    let mut buf = Cursor::new(Vec::new());
    write_ods(wb, &mut buf).unwrap();
    buf.into_inner()
}

fn read(bytes: Vec<u8>) -> WorkbookData {
    read_ods(Cursor::new(bytes), &Limits::default()).unwrap()
}

fn part(bytes: &[u8], name: &str) -> String {
    let mut z = zip::ZipArchive::new(Cursor::new(bytes.to_vec())).unwrap();
    let mut f = z.by_name(name).unwrap();
    let mut s = String::new();
    std::io::Read::read_to_string(&mut f, &mut s).unwrap();
    s
}

/// The package with `styles.xml` passed through `edit`.
fn with_styles(bytes: Vec<u8>, edit: impl Fn(String) -> String) -> Vec<u8> {
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for i in 0..z.len() {
        let mut f = z.by_index(i).unwrap();
        let name = f.name().unwrap().to_string();
        let mut data = Vec::new();
        std::io::Read::read_to_end(&mut f, &mut data).unwrap();
        if name == "styles.xml" {
            data = edit(String::from_utf8(data).unwrap()).into_bytes();
        }
        out.start_file(name.as_str(), zip::write::SimpleFileOptions::default())
            .unwrap();
        std::io::Write::write_all(&mut out, &data).unwrap();
    }
    out.finish().unwrap().into_inner()
}

const DEFAULT_STYLE_OPEN: &str = r#"<style:default-style style:family="table-cell"><style:paragraph-properties style:tab-stop-distance="1.25cm"/>"#;
const DEFAULT_CELL_STYLE: &str = r#"<style:style style:name="Default" style:family="table-cell"/>"#;

/// styles.xml as AOO writes it for an Arial document (the owner's file).
fn aoo_like(xml: String, default_style: &str) -> String {
    xml.replacen(
        "<office:font-face-decls>",
        r#"<office:font-face-decls><style:font-face style:name="Arial" svg:font-family="Arial" style:font-family-generic="swiss" style:font-pitch="variable"/><style:font-face style:name="Arial Unicode MS" svg:font-family="&apos;Arial Unicode MS&apos;" style:font-family-generic="system" style:font-pitch="variable"/>"#,
        1,
    )
    .replacen(
        DEFAULT_STYLE_OPEN,
        &format!(
            r#"{DEFAULT_STYLE_OPEN}<style:text-properties style:font-name="Arial" fo:language="en" fo:country="CA" style:font-name-asian="Arial Unicode MS" style:font-name-complex="Tahoma"/>"#
        ),
        1,
    )
    .replacen(DEFAULT_CELL_STYLE, default_style, 1)
}

#[test]
fn default_style_font_is_imported() {
    let bytes = with_styles(write(&book(None)), |x| {
        aoo_like(
            x,
            r#"<style:style style:name="Default" style:family="table-cell"><style:text-properties style:font-name-complex="Arial Unicode MS"/></style:style>"#,
        )
    });
    let wb = read(bytes);
    assert_eq!(wb.settings.default_font.as_deref(), Some("Arial"));
    assert_eq!(wb.settings.locale.as_deref(), Some("en-CA"));
    // Unstyled cells keep no family (they use the default); a styled cell
    // keeps its own.
    assert_eq!(wb.sheets[0].cells[0].style, None);
    let st = wb.sheets[0].cells[1].style.unwrap();
    assert_eq!(
        wb.styles[st as usize].font.family.as_deref(),
        Some("Times New Roman")
    );
}

#[test]
fn default_cell_style_font_wins_over_the_default_style() {
    let bytes = with_styles(write(&book(None)), |x| {
        aoo_like(
            x,
            r#"<style:style style:name="Default" style:family="table-cell"><style:text-properties fo:font-family="'DejaVu Sans', sans-serif"/></style:style>"#,
        )
    });
    assert_eq!(
        read(bytes).settings.default_font.as_deref(),
        Some("DejaVu Sans")
    );
}

#[test]
fn default_font_round_trips_unchanged() {
    let bytes = write(&book(Some("Arial")));
    let styles = part(&bytes, "styles.xml");
    assert!(
        styles.contains(r#"<style:font-face style:name="Arial" svg:font-family="Arial"/>"#),
        "{styles}"
    );
    assert!(
        styles.contains(&format!(
            r#"{DEFAULT_STYLE_OPEN}<style:text-properties style:font-name="Arial"/>"#
        )),
        "{styles}"
    );
    let back = read(bytes);
    assert_eq!(back.settings.default_font.as_deref(), Some("Arial"));
    let again = read(write(&back));
    assert_eq!(again.settings, back.settings);
    assert_eq!(again.styles, back.styles);
    // No default font: nothing is written for it.
    let plain = write(&book(None));
    assert!(!part(&plain, "styles.xml").contains("style:font-name"));
    assert_eq!(read(plain).settings.default_font, None);
}

#[test]
fn hostile_font_names_stay_names() {
    let long = "N".repeat(1000);
    let bytes = with_styles(write(&book(None)), |x| {
        x.replacen(
            "<office:font-face-decls>",
            &format!(
                r#"<office:font-face-decls><style:font-face style:name="P" svg:font-family="../../../../etc/passwd"/><style:font-face style:name="C" svg:font-family="Bad&#x1;Name"/><style:font-face style:name="L" svg:font-family="{long}"/>"#
            ),
            1,
        )
        .replacen(
            DEFAULT_STYLE_OPEN,
            &format!(r#"{DEFAULT_STYLE_OPEN}<style:text-properties style:font-name="P"/>"#),
            1,
        )
    });
    // A path-like name is only ever a family name to match against
    // installed fonts (tally-io-pdf); it is kept as text.
    assert_eq!(
        read(bytes.clone()).settings.default_font.as_deref(),
        Some("../../../../etc/passwd")
    );
    let ctl = with_styles(bytes.clone(), |x| {
        x.replacen(r#"style:font-name="P""#, r#"style:font-name="C""#, 1)
    });
    // A control character makes the declaration unusable: the reference
    // itself is taken as the family, as for any undeclared font name.
    assert_eq!(read(ctl).settings.default_font.as_deref(), Some("C"));
    let long_ref = with_styles(bytes, |x| {
        x.replacen(r#"style:font-name="P""#, r#"style:font-name="L""#, 1)
    });
    assert_eq!(
        read(long_ref).settings.default_font.map(|f| f.len()),
        Some(tally_model::MAX_FONT_NAME_LEN)
    );
}
