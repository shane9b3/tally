//! Small AOO 4.1 document options in ODS: the two-digit year window
//! (`table:calculation-settings/@table:null-year`, P17), the General
//! format's decimals (`style:decimal-places` on the default cell style)
//! and the per-sheet view zoom (settings.xml `ZoomValue`, U18).

#![allow(clippy::unwrap_used)] // tests may unwrap (docs/contributing.md)

use std::io::{Cursor, Read, Write};

use tally_io_ods::{read_ods, write_ods, Limits};
use tally_model::{SheetData, WorkbookData};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

const NS: &str = r#"xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" xmlns:config="urn:oasis:names:tc:opendocument:xmlns:config:1.0" office:version="1.2""#;

const TABLE: &str = r#"<table:table table:name="Sheet1"><table:table-row><table:table-cell office:value-type="float" office:value="3.14159265358979"/></table:table-row></table:table><table:table table:name="Sheet2"><table:table-row><table:table-cell/></table:table-row></table:table>"#;

/// A package as AOO writes it: `calc` goes before the tables, `styles`
/// into styles.xml's `office:styles`, `settings` is the whole settings.xml
/// body (if any).
fn ods(calc: &str, styles: &str, settings: Option<&str>) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    zip.start_file("mimetype", stored).unwrap();
    zip.write_all(tally_io_ods::MIMETYPE.as_bytes()).unwrap();
    zip.start_file("content.xml", stored).unwrap();
    write!(
        zip,
        r#"<?xml version="1.0" encoding="UTF-8"?><office:document-content {NS}><office:body><office:spreadsheet>{calc}{TABLE}</office:spreadsheet></office:body></office:document-content>"#
    )
    .unwrap();
    zip.start_file("styles.xml", stored).unwrap();
    write!(
        zip,
        r#"<?xml version="1.0" encoding="UTF-8"?><office:document-styles {NS}><office:styles>{styles}</office:styles></office:document-styles>"#
    )
    .unwrap();
    if let Some(body) = settings {
        zip.start_file("settings.xml", stored).unwrap();
        write!(
            zip,
            r#"<?xml version="1.0" encoding="UTF-8"?><office:document-settings {NS}><office:settings>{body}</office:settings></office:document-settings>"#
        )
        .unwrap();
    }
    zip.finish().unwrap().into_inner()
}

fn read(bytes: Vec<u8>) -> WorkbookData {
    read_ods(Cursor::new(bytes), &Limits::default()).unwrap()
}

fn write(wb: &WorkbookData) -> Vec<u8> {
    let mut buf = Cursor::new(Vec::new());
    write_ods(wb, &mut buf).unwrap();
    buf.into_inner()
}

fn part(bytes: &[u8], name: &str) -> String {
    let mut zip = ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut s = String::new();
    zip.by_name(name).unwrap().read_to_string(&mut s).unwrap();
    s
}

fn book() -> WorkbookData {
    WorkbookData {
        sheets: vec![
            SheetData {
                name: "Sheet1".into(),
                ..SheetData::default()
            },
            SheetData {
                name: "Sheet2".into(),
                ..SheetData::default()
            },
        ],
        ..WorkbookData::default()
    }
}

#[test]
fn null_year_is_read_with_aoo_defaults() {
    // No calculation settings: the document does not set the window (the
    // application option applies, AOO ScDocShell::InitOptions).
    assert_eq!(
        read(ods("", "", None))
            .settings
            .calc_settings
            .two_digit_year,
        None
    );
    // Present without `table:null-year`: 1930 (ScXMLCalculationSettingsContext).
    let wb = read(ods("<table:calculation-settings/>", "", None));
    assert_eq!(wb.settings.calc_settings.two_digit_year, Some(1930));
    let wb = read(ods(
        r#"<table:calculation-settings table:null-year="1950"/>"#,
        "",
        None,
    ));
    assert_eq!(wb.settings.calc_settings.two_digit_year, Some(1950));
    // Outside AOO's option range: ignored (1930).
    let wb = read(ods(
        r#"<table:calculation-settings table:null-year="12"/>"#,
        "",
        None,
    ));
    assert_eq!(wb.settings.calc_settings.two_digit_year, Some(1930));
}

#[test]
fn null_year_round_trips_and_1930_is_not_written() {
    for (year, attr) in [(1930, None), (1950, Some("1950")), (2000, Some("2000"))] {
        let mut wb = book();
        wb.settings.calc_settings.two_digit_year = Some(year);
        let bytes = write(&wb);
        let content = part(&bytes, "content.xml");
        match attr {
            // AOO ScXMLExport::WriteCalculationSettings writes it only when ≠ 1930.
            None => assert!(!content.contains("table:null-year"), "{content}"),
            Some(y) => assert!(content.contains(&format!(r#"table:null-year="{y}""#))),
        }
        assert_eq!(
            read(bytes).settings.calc_settings.two_digit_year,
            Some(year)
        );
    }
}

#[test]
fn general_decimals_round_trip_on_the_default_cell_style() {
    let styles = r#"<style:default-style style:family="table-cell"><style:table-cell-properties style:decimal-places="2"/><style:paragraph-properties/></style:default-style>"#;
    let wb = read(ods("", styles, None));
    assert_eq!(wb.settings.calc_settings.standard_decimals, Some(2));
    // Absent: unlimited (AOO docsh3.cxx InitOptions, #i112123).
    assert_eq!(
        read(ods("", "", None))
            .settings
            .calc_settings
            .standard_decimals,
        None
    );
    // AOO's field allows 0–20.
    let big = styles.replace(r#""2""#, r#""99""#);
    assert_eq!(
        read(ods("", &big, None))
            .settings
            .calc_settings
            .standard_decimals,
        Some(20)
    );

    let mut wb = book();
    wb.settings.calc_settings.standard_decimals = Some(3);
    let bytes = write(&wb);
    let st = part(&bytes, "styles.xml");
    assert!(
        st.contains(r#"<style:default-style style:family="table-cell"><style:table-cell-properties style:decimal-places="3"/>"#),
        "{st}"
    );
    assert_eq!(
        read(bytes).settings.calc_settings.standard_decimals,
        Some(3)
    );
    // Unlimited writes nothing.
    let st = part(&write(&book()), "styles.xml");
    assert!(!st.contains("style:decimal-places"), "{st}");
}

fn settings(per_sheet: &[(&str, &str)], view: Option<&str>) -> String {
    let mut s = String::from(
        r#"<config:config-item-set config:name="ooo:view-settings"><config:config-item-map-indexed config:name="Views"><config:config-item-map-entry><config:config-item-map-named config:name="Tables">"#,
    );
    for (sheet, zoom) in per_sheet {
        s.push_str(&format!(
            r#"<config:config-item-map-entry config:name="{sheet}"><config:config-item config:name="ZoomType" config:type="short">0</config:config-item><config:config-item config:name="ZoomValue" config:type="int">{zoom}</config:config-item></config:config-item-map-entry>"#
        ));
    }
    s.push_str("</config:config-item-map-named>");
    if let Some(z) = view {
        s.push_str(&format!(
            r#"<config:config-item config:name="ZoomValue" config:type="int">{z}</config:config-item>"#
        ));
    }
    s.push_str(
        "</config:config-item-map-entry></config:config-item-map-indexed></config:config-item-set>",
    );
    s
}

#[test]
fn zoom_is_read_per_sheet_and_clamped() {
    let wb = read(ods(
        "",
        "",
        Some(&settings(&[("Sheet1", "250"), ("Sheet2", "100")], None)),
    ));
    assert_eq!((wb.sheets[0].zoom, wb.sheets[1].zoom), (Some(250), None));
    // AOO MINZOOM / MAXZOOM: 20–400 %.
    let wb = read(ods(
        "",
        "",
        Some(&settings(&[("Sheet1", "10"), ("Sheet2", "999")], None)),
    ));
    assert_eq!(
        (wb.sheets[0].zoom, wb.sheets[1].zoom),
        (Some(20), Some(400))
    );
    let wb = read(ods("", "", Some(&settings(&[("Sheet1", "0")], None))));
    assert_eq!(wb.sheets[0].zoom, None);
    // A view-level value covers sheets without their own.
    let wb = read(ods(
        "",
        "",
        Some(&settings(&[("Sheet1", "75")], Some("150"))),
    ));
    assert_eq!(
        (wb.sheets[0].zoom, wb.sheets[1].zoom),
        (Some(75), Some(150))
    );
}

#[test]
fn zoom_round_trips() {
    let mut wb = book();
    wb.sheets[0].zoom = Some(20);
    wb.sheets[1].zoom = Some(400);
    let bytes = write(&wb);
    let s = part(&bytes, "settings.xml");
    assert!(s.contains(r#"<config:config-item config:name="ZoomValue" config:type="int">20</config:config-item>"#), "{s}");
    let back = read(bytes);
    assert_eq!(
        (back.sheets[0].zoom, back.sheets[1].zoom),
        (Some(20), Some(400))
    );
    // 100 % is written as 100 and reads back as the default.
    let back = read(write(&book()));
    assert_eq!((back.sheets[0].zoom, back.sheets[1].zoom), (None, None));
}

#[test]
fn calculation_settings_are_left_out_when_all_are_aoo_defaults() {
    // AOO's defaults (case-sensitive, regular expressions, …, 1930): no
    // element, so an AOO reader applies its own two-digit year option.
    let mut wb = book();
    let c = &mut wb.settings.calc_settings;
    c.case_sensitive = true;
    c.criteria_mode = tally_types::CriteriaMode::Regex;
    c.two_digit_year = Some(1930);
    let bytes = write(&wb);
    let content = part(&bytes, "content.xml");
    assert!(!content.contains("table:calculation-settings"), "{content}");
    // Tally reads its own file back with the same settings; the window is
    // left to the application option.
    let back = read(bytes).settings.calc_settings;
    assert!(back.case_sensitive);
    assert_eq!(back.criteria_mode, tally_types::CriteriaMode::Regex);
    assert_eq!(back.two_digit_year, None);
    // Any other setting writes the element (here the 1950 window alone).
    wb.settings.calc_settings.two_digit_year = Some(1950);
    let content = part(&write(&wb), "content.xml");
    assert!(content.contains(r#"table:null-year="1950""#), "{content}");
    // Tally's own defaults differ from AOO's (case-insensitive, wildcards).
    let content = part(&write(&book()), "content.xml");
    assert!(content.contains("table:calculation-settings"), "{content}");
}
