//! Header and footer margins through XLSX. Excel's header margin is the
//! header's distance from the page edge and its top margin the body's;
//! Tally (AOO, ODF) puts the header area inside the top margin. An
//! imported sheet prints its header at Excel's header margin and its body
//! at Excel's top margin, and the mapping round-trips.
#![allow(clippy::unwrap_used)]

use std::io::{Cursor, Read, Write};

use tally_io_xlsx::{read_xlsx, write_xlsx, Limits};
use tally_model::*;

fn book(p: PageSetupData) -> WorkbookData {
    WorkbookData {
        sheets: vec![SheetData {
            name: "S".into(),
            page_setup: Some(p),
            ..SheetData::default()
        }],
        ..WorkbookData::default()
    }
}

fn write(wb: &WorkbookData) -> Vec<u8> {
    let mut buf = Cursor::new(Vec::new());
    write_xlsx(wb, &mut buf).unwrap();
    buf.into_inner()
}

fn read(bytes: Vec<u8>) -> PageSetupData {
    let wb = read_xlsx(Cursor::new(bytes), &Limits::default()).unwrap();
    wb.sheets[0].page_setup.clone().unwrap()
}

/// The package with `edit` applied to the first worksheet's XML.
fn patch_sheet(bytes: &[u8], edit: impl Fn(&str) -> String) -> (Vec<u8>, String) {
    let mut zin = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut zout = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let mut sheet = String::new();
    for i in 0..zin.len() {
        let mut f = zin.by_index(i).unwrap();
        let name = f.name().unwrap().to_string();
        let mut data = Vec::new();
        f.read_to_end(&mut data).unwrap();
        if name == "xl/worksheets/sheet1.xml" {
            sheet = edit(&String::from_utf8(data).unwrap());
            data = sheet.clone().into_bytes();
        }
        zout.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zout.write_all(&data).unwrap();
    }
    (zout.finish().unwrap().into_inner(), sheet)
}

fn page_margins(sheet: &str) -> &str {
    let i = sheet.find("<pageMargins").unwrap();
    &sheet[i..i + sheet[i..].find("/>").unwrap()]
}

#[test]
fn excel_margins_import_like_excel_prints() {
    let p = PageSetupData {
        footer: None,
        ..PageSetupData::default()
    };
    let bytes = write(&book(p));
    // Excel's defaults: 0.75" top, header 0.3" from the edge.
    let (bytes, _) = patch_sheet(&bytes, |s| {
        let m = page_margins(s).to_string();
        s.replace(
            &m,
            r#"<pageMargins left="0.7" right="0.7" top="0.75" bottom="0.75" header="0.3" footer="0.3""#,
        )
    });
    let p = read(bytes);
    assert!(p.header_area.on);
    assert!(p.footer.is_none() && !p.footer_area.on);
    // The header starts at 0.3" and the body at 0.75".
    assert_eq!(p.margins.top, 762);
    let h = p.header_shown().unwrap();
    assert_eq!(
        p.margins.top + p.header_area.printed_height(h, p.margins.header),
        1905
    );
    assert_eq!(p.header_area.spacing, 250);
    // No footer: the bottom margin is Excel's.
    assert_eq!(p.margins.bottom, 1905);
}

#[test]
fn header_area_round_trips_through_excel_margins() {
    let mut p = PageSetupData::default();
    p.margins.top = 1000;
    p.margins.header = 1200;
    p.margins.bottom = 1500;
    p.margins.footer = 900;
    let bytes = write(&book(p.clone()));
    let (_, sheet) = patch_sheet(&bytes, |s| s.to_string());
    // Excel: header 10 mm from the edge, body at 10 + 12 mm.
    let m = page_margins(&sheet);
    assert!(m.contains(r#"header="0.3937"#), "{m}");
    assert!(m.contains(r#"top="0.866"#), "{m}");
    let back = read(bytes);
    assert_eq!(back.margins, p.margins);
    // Excel's gap is fixed, so the areas come back without AutoFit.
    let fixed = |a: HfAreaData| HfAreaData {
        autofit: false,
        ..a
    };
    assert_eq!(back.header_area, fixed(p.header_area));
    assert_eq!(back.footer_area, fixed(p.footer_area));
    assert_eq!(back.header, p.header);
}

#[test]
fn header_off_or_empty_is_no_excel_header() {
    let mut p = PageSetupData::default();
    p.header_area.on = false;
    let bytes = write(&book(p));
    let (bytes, sheet) = patch_sheet(&bytes, |s| s.to_string());
    assert!(!sheet.contains("<oddHeader"), "{sheet}");
    // The body starts at the top margin as before.
    assert!(page_margins(&sheet).contains(r#"top="0.787"#));
    let back = read(bytes);
    assert!(back.header.is_none() && !back.header_area.on);
    assert_eq!(back.margins.top, 2000);
    // An empty `oddHeader` is no header.
    let bytes = write(&book(PageSetupData::default()));
    let (bytes, _) = patch_sheet(&bytes, |s| {
        let a = s.find("<oddHeader>").unwrap();
        let b = s.find("</oddHeader>").unwrap();
        format!("{}<oddHeader></oddHeader>{}", &s[..a], &s[b + 12..])
    });
    let back = read(bytes);
    assert!(back.header.is_none() && !back.header_area.on);
}

/// The `pageMargins` attributes as written.
fn margin_attrs(sheet: &str) -> Vec<(String, String)> {
    let m = page_margins(sheet);
    let mut out: Vec<(String, String)> = m
        .split_whitespace()
        .skip(1)
        .filter_map(|kv| {
            let (k, v) = kv.split_once('=')?;
            Some((k.to_string(), v.trim_matches('"').to_string()))
        })
        .collect();
    out.sort();
    out
}

/// An Excel sheet with `margins` (pageMargins attributes) and a header of
/// `lines` lines, and the same sheet after a Tally round trip.
fn excel_round_trip(
    margins: &str,
    header: &str,
    edit: impl Fn(&mut PageSetupData),
) -> (String, String) {
    let bytes = write(&book(PageSetupData::default()));
    let (bytes, input) = patch_sheet(&bytes, |s| {
        let m = page_margins(s).to_string();
        let s = s.replace(&m, &format!("<pageMargins {margins}"));
        let a = s.find("<oddHeader>").unwrap();
        let b = s.find("</oddHeader>").unwrap();
        format!("{}<oddHeader>{header}{}", &s[..a], &s[b..])
    });
    let mut wb = read_xlsx(Cursor::new(bytes), &Limits::default()).unwrap();
    let p = wb.sheets[0].page_setup.as_mut().unwrap();
    // Excel's gap is fixed: the area does not grow with the text.
    assert!(!p.header_area.autofit);
    edit(p);
    let (_, output) = patch_sheet(&write(&wb), |s| s.to_string());
    (input, output)
}

#[test]
fn untouched_excel_margins_round_trip_exactly() {
    let defaults = r#"left="0.7" right="0.7" top="0.75" bottom="0.75" header="0.3" footer="0.3""#;
    for (margins, header) in [
        (defaults, "&amp;CThe Redwood\nBlock 90"),
        (defaults, "&amp;Cone\ntwo\nthree"),
        (
            r#"left="0.7" right="0.7" top="0.5" bottom="0.75" header="0.3" footer="0.3""#,
            "&amp;Cone\ntwo",
        ),
        // The header margin at or below Excel's top margin.
        (
            r#"left="0.7" right="0.7" top="0.3" bottom="0.75" header="0.5" footer="0.3""#,
            "&amp;Cx",
        ),
        (
            r#"left="0.7" right="0.7" top="0.4" bottom="0.75" header="0.4" footer="0.3""#,
            "&amp;Cx",
        ),
    ] {
        let (input, output) = excel_round_trip(margins, header, |_| {});
        assert_eq!(
            margin_attrs(&output),
            margin_attrs(&input),
            "{margins} {header}"
        );
        // Edits that leave the margins and areas alone keep them too.
        let (input, output) = excel_round_trip(margins, header, |p| {
            p.orientation = OrientationData::Landscape;
            p.header.as_mut().unwrap().center.push_str("\nmore");
        });
        assert_eq!(
            margin_attrs(&output),
            margin_attrs(&input),
            "{margins} {header}"
        );
    }
}

#[test]
fn edited_excel_margins_are_derived_again() {
    let defaults = r#"left="0.7" right="0.7" top="0.75" bottom="0.75" header="0.3" footer="0.3""#;
    // Tally's top margin moved to 10 mm: Excel's header margin follows,
    // the gap (the 11.43 mm header area) is kept.
    let (_, output) = excel_round_trip(defaults, "&amp;Cx", |p| p.margins.top = 1000);
    let m = margin_attrs(&output);
    let get = |k: &str| {
        m.iter()
            .find(|(a, _)| a == k)
            .unwrap()
            .1
            .parse::<f64>()
            .unwrap()
    };
    assert!((get("header") - 1000.0 / 2540.0).abs() < 1e-3, "{m:?}");
    assert!((get("top") - 2143.0 / 2540.0).abs() < 1e-3, "{m:?}");
}

#[test]
fn an_empty_header_switched_on_keeps_the_body_where_tally_prints_it() {
    let mut p = PageSetupData {
        header: Some(HeaderFooterData::default()),
        ..PageSetupData::default()
    };
    p.margins.top = 1000;
    let (_, sheet) = patch_sheet(&write(&book(p)), |s| s.to_string());
    assert!(!sheet.contains("<oddHeader"), "{sheet}");
    // Body at 10 mm + the 7.5 mm area.
    assert!(
        page_margins(&sheet).contains(r#"top="0.68897"#),
        "{}",
        page_margins(&sheet)
    );
}
