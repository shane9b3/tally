//! Flat XML ODF spreadsheets (`.fods`) through the ODS parsers.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{Cursor, Read};
use std::path::Path;

use tally_io_ods::{
    fods_has_macros, read_fods, read_ods, sniff_is_fods, sniff_is_ods, Limits, OdsError,
};
use tally_model::{CellContentData, WorkbookData};
use tally_types::Value;

fn fixture(rel: &str) -> Vec<u8> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(rel);
    std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn content(wb: &WorkbookData, sheet: usize, r: u32, c: u32) -> Option<CellContentData> {
    wb.sheets[sheet]
        .cells
        .iter()
        .find(|x| x.row == r && x.col == c)
        .map(|x| x.content.clone())
}

#[test]
fn hand_written_fods_reads_like_a_package() {
    let bytes = fixture("fods/basic.fods");
    assert!(sniff_is_fods(&bytes));
    assert!(!sniff_is_ods(&bytes));
    assert!(fods_has_macros(&bytes));
    let wb = read_fods(&bytes, &Limits::default()).unwrap();
    assert_eq!(wb.sheets.len(), 2);
    assert_eq!(wb.sheets[0].name, "Sheet1");
    assert_eq!(wb.sheets[1].name, "Data");
    // Meta and settings (pass 1).
    assert_eq!(wb.meta.title.as_deref(), Some("Flat test"));
    assert_eq!(wb.meta.author.as_deref(), Some("Tally"));
    assert_eq!(wb.settings.active_sheet, 1);
    assert_eq!(wb.sheets[0].frozen, Some((1, 0)));
    // Values, formulas, merges.
    assert_eq!(
        content(&wb, 0, 0, 0),
        Some(CellContentData::Value(Value::Text("Title".into())))
    );
    assert_eq!(
        content(&wb, 0, 1, 2),
        Some(CellContentData::Formula {
            text: "of:=SUM([.A2:.B2])".into(),
            cached: Some(Value::Number(3.0)),
        })
    );
    assert_eq!(
        content(&wb, 0, 2, 1),
        Some(CellContentData::Value(Value::Bool(true)))
    );
    assert_eq!(wb.sheets[0].merges.len(), 1);
    // Common styles (`office:styles`) resolve through automatic ones.
    let cell = |r: u32, c: u32| {
        wb.sheets[0]
            .cells
            .iter()
            .find(|x| x.row == r && x.col == c)
            .and_then(|x| x.style)
            .map(|s| wb.styles[s as usize].clone())
            .unwrap()
    };
    let title = cell(0, 0);
    assert!(title.font.bold);
    assert_eq!(title.font.size_pt, Some(16.0));
    assert_eq!(title.fill, Some(0xffff_00ff));
    assert_eq!(cell(2, 0).number_format.as_deref(), Some("0.00%"));
    // Column width, row height, page layout from the master page.
    assert_eq!(wb.sheets[0].col_widths[0].size_hmm, 4000);
    assert_eq!(wb.sheets[0].row_heights[0].size_hmm, 1000);
    let page = wb.sheets[0].page_setup.as_ref().expect("page setup");
    assert_eq!(page.margins.top, 1500);
    assert_eq!(wb.names.len(), 1);
    assert_eq!(wb.names[0].name, "Totals");
}

/// Builds a flat file from an ODS package's parts, the way AOO's flat
/// export lays them out (meta, settings, fonts, styles, automatic styles,
/// master styles, body).
fn flatten(ods: &[u8]) -> Vec<u8> {
    let mut zip = zip::ZipArchive::new(Cursor::new(ods)).unwrap();
    let mut part = |name: &str| {
        let mut s = String::new();
        if let Ok(mut f) = zip.by_name(name) {
            f.read_to_string(&mut s).unwrap();
        }
        s
    };
    let (content, styles, meta, settings) = (
        part("content.xml"),
        part("styles.xml"),
        part("meta.xml"),
        part("settings.xml"),
    );
    // The children of the root element of `xml` (empty if there is none).
    fn inner(xml: &str) -> &str {
        let Some(open) = xml.find("<office:document") else {
            return "";
        };
        let start = xml[open..].find('>').map_or(xml.len(), |i| open + i + 1);
        let end = xml.rfind("</office:").unwrap_or(xml.len()).max(start);
        &xml[start..end]
    }
    // The root's attributes of content.xml (its namespace declarations).
    let open = content.find("<office:document-content").unwrap();
    let attrs_end = open + content[open..].find('>').unwrap();
    let attrs = &content[open + "<office:document-content".len()..attrs_end];
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str("<office:document");
    out.push_str(attrs.trim_end_matches('/'));
    if !attrs.contains("xmlns:config=") {
        out.push_str(r#" xmlns:config="urn:oasis:names:tc:opendocument:xmlns:config:1.0""#);
    }
    out.push_str(r#" office:mimetype="application/vnd.oasis.opendocument.spreadsheet">"#);
    for part in [&meta, &settings, &styles, &content] {
        out.push_str(inner(part));
    }
    out.push_str("</office:document>");
    out.into_bytes()
}

#[test]
fn flattened_packages_read_the_same() {
    for name in ["ods/aoo-style.ods", "ods/libreoffice-style.ods"] {
        let ods = fixture(name);
        let flat = flatten(&ods);
        assert!(sniff_is_fods(&flat), "{name}");
        let a = read_ods(Cursor::new(&ods), &Limits::default()).unwrap();
        let b = read_fods(&flat, &Limits::default()).unwrap();
        assert_eq!(a.sheets.len(), b.sheets.len(), "{name}");
        for (sa, sb) in a.sheets.iter().zip(&b.sheets) {
            assert_eq!(sa.name, sb.name, "{name}");
            assert_eq!(sa.cells.len(), sb.cells.len(), "{name} {}", sa.name);
            for (ca, cb) in sa.cells.iter().zip(&sb.cells) {
                assert_eq!((ca.row, ca.col), (cb.row, cb.col), "{name}");
                assert_eq!(ca.content, cb.content, "{name}");
                let style =
                    |wb: &WorkbookData, s: Option<u32>| s.map(|i| wb.styles[i as usize].clone());
                assert_eq!(style(&a, ca.style), style(&b, cb.style), "{name}");
            }
            assert_eq!(sa.merges, sb.merges, "{name}");
            assert_eq!(sa.col_widths, sb.col_widths, "{name}");
            assert_eq!(sa.row_heights, sb.row_heights, "{name}");
            assert_eq!(sa.frozen, sb.frozen, "{name}");
            assert_eq!(sa.page_setup, sb.page_setup, "{name}");
        }
        assert_eq!(a.names, b.names, "{name}");
        assert_eq!(a.meta, b.meta, "{name}");
        assert_eq!(a.settings.active_sheet, b.settings.active_sheet, "{name}");
    }
}

#[test]
fn sniffing_rejects_other_documents() {
    let text = br#"<?xml version="1.0"?><office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" office:mimetype="application/vnd.oasis.opendocument.text"><office:body/></office:document>"#;
    assert!(!sniff_is_fods(text));
    assert!(matches!(
        read_fods(text, &Limits::default()),
        Err(OdsError::NotOds)
    ));
    let content = br#"<office:document-content office:mimetype="application/vnd.oasis.opendocument.spreadsheet"/>"#;
    assert!(!sniff_is_fods(content));
    assert!(!sniff_is_fods(b""));
    assert!(!sniff_is_fods(b"<html><body>office:document</body></html>"));
    assert!(read_fods(b"", &Limits::default()).is_err());
    assert!(read_fods(b"<a/>", &Limits::default()).is_err());
}

const HEAD: &str = r#"<?xml version="1.0"?><office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" office:mimetype="application/vnd.oasis.opendocument.spreadsheet">"#;

#[test]
fn hostile_flat_files_are_refused_quickly() {
    let limits = Limits::default();
    // DOCTYPE with an entity.
    let dtd = format!(
        "<?xml version=\"1.0\"?><!DOCTYPE x [<!ENTITY a \"aaaa\">]>{}</office:document>",
        &HEAD[HEAD.find("<office:document").unwrap()..]
    );
    assert!(read_fods(dtd.as_bytes(), &limits).is_err());
    // Deep nesting.
    let deep = format!("{HEAD}{}", "<x>".repeat(100_000));
    assert!(read_fods(deep.as_bytes(), &limits).is_err());
    // Repeat bomb: the cell cap applies as in content.xml.
    let bomb = format!(
        "{HEAD}<office:body><office:spreadsheet><table:table table:name=\"S\">{}</table:table></office:spreadsheet></office:body></office:document>",
        "<table:table-row table:number-rows-repeated=\"1048576\"><table:table-cell table:number-columns-repeated=\"16384\" office:value-type=\"float\" office:value=\"1\"/></table:table-row>"
    );
    let t = std::time::Instant::now();
    assert!(read_fods(bomb.as_bytes(), &limits).is_err());
    assert!(t.elapsed().as_secs() < 5);
    // Over the byte cap.
    let small = Limits {
        max_content_xml_bytes: 64,
        ..Limits::default()
    };
    assert!(matches!(
        read_fods(HEAD.as_bytes(), &small),
        Err(OdsError::LimitExceeded { .. })
    ));
    // Truncated and mutated input never panics.
    let good = fixture("fods/basic.fods");
    for cut in (0..good.len()).step_by(97) {
        let _ = read_fods(&good[..cut], &limits);
    }
    let mut seed = 0x2545_f491_u32;
    for _ in 0..300 {
        let mut m = good.clone();
        for _ in 0..8 {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let i = seed as usize % m.len();
            m[i] = (seed >> 8) as u8;
        }
        let _ = read_fods(&m, &limits);
    }
}

/// Review L3: the macro check resolves namespaces and ignores comments,
/// CDATA and text.
#[test]
fn macro_detection_is_namespace_aware() {
    let doc = |inner: &str| {
        format!(
            r#"<?xml version="1.0"?><office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:o="http://openoffice.org/2004/office" xmlns:x="urn:other" office:mimetype="application/vnd.oasis.opendocument.spreadsheet">{inner}<office:body/></office:document>"#
        )
    };
    assert!(fods_has_macros(
        doc("<office:scripts><o:libraries><o:module o:name=\"M\"/></o:libraries></office:scripts>")
            .as_bytes()
    ));
    assert!(!fods_has_macros(doc("<!-- <ooo:module> -->").as_bytes()));
    assert!(!fods_has_macros(
        doc("<x:a><![CDATA[<ooo:module>]]></x:a>").as_bytes()
    ));
    assert!(!fods_has_macros(doc("<x:module/>").as_bytes()));
    // After the body (cell text) it is not looked for.
    let late = r#"<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:ooo="http://openoffice.org/2004/office"><office:body/><ooo:module/></office:document>"#;
    assert!(!fods_has_macros(late.as_bytes()));
    assert!(!fods_has_macros(b"<unclosed"));
}
