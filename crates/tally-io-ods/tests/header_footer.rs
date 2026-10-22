//! Header and footer areas through ODS: "Header on" / "Footer on"
//! (`style:display`), AutoFit (`fo:min-height`) or fixed (`svg:height`)
//! height, spacing and side margins, as AOO writes and reads them.
#![allow(clippy::unwrap_used)]

use std::io::{Cursor, Write};

use tally_io_ods::{read_ods, write_ods_with_report, Limits};
use tally_model::*;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

const NS: &str = r#"xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0" office:version="1.2""#;

/// A one-sheet ODS whose "Default" master page is `master` over the page
/// layout `layout`.
fn ods(layout: &str, master: &str) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    zip.start_file("mimetype", stored).unwrap();
    zip.write_all(tally_io_ods::MIMETYPE.as_bytes()).unwrap();
    zip.start_file("content.xml", deflated).unwrap();
    write!(
        zip,
        r#"<?xml version="1.0" encoding="UTF-8"?><office:document-content {NS}><office:body><office:spreadsheet><table:table table:name="S"><table:table-row><table:table-cell office:value-type="string"><text:p>x</text:p></table:table-cell></table:table-row></table:table></office:spreadsheet></office:body></office:document-content>"#
    )
    .unwrap();
    zip.start_file("styles.xml", deflated).unwrap();
    write!(
        zip,
        r#"<?xml version="1.0" encoding="UTF-8"?><office:document-styles {NS}><office:automatic-styles><style:page-layout style:name="Mpm1">{layout}</style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Default" style:page-layout-name="Mpm1">{master}</style:master-page></office:master-styles></office:document-styles>"#
    )
    .unwrap();
    zip.finish().unwrap().into_inner()
}

fn read(bytes: Vec<u8>) -> PageSetupData {
    let wb = read_ods(Cursor::new(bytes), &Limits::default()).unwrap();
    wb.sheets[0].page_setup.clone().unwrap_or_default()
}

fn round_trip(p: &PageSetupData) -> PageSetupData {
    let wb = WorkbookData {
        sheets: vec![SheetData {
            name: "S".into(),
            page_setup: Some(p.clone()),
            ..SheetData::default()
        }],
        ..WorkbookData::default()
    };
    let mut buf = Cursor::new(Vec::new());
    write_ods_with_report(&wb, &mut buf).unwrap();
    read(buf.into_inner())
}

#[test]
fn aoo_header_off_keeps_its_text_and_a_fixed_footer_is_read() {
    let p = read(ods(
        r#"<style:page-layout-properties fo:margin-top="1cm" fo:margin-bottom="1cm"/><style:header-style/><style:footer-style><style:header-footer-properties svg:height="1.5cm" fo:margin-left="0.5cm" fo:margin-right="0.2cm" fo:margin-top="0.3cm"/></style:footer-style>"#,
        r#"<style:header style:display="false"><style:region-center><text:p>Hidden</text:p></style:region-center></style:header><style:footer><text:p>Page <text:page-number>1</text:page-number></text:p><text:p>two</text:p></style:footer>"#,
    ));
    assert!(!p.header_area.on);
    assert_eq!(p.header.as_ref().unwrap().center, "Hidden");
    assert!(p.header_shown().is_none());
    assert!(p.footer_area.on);
    assert_eq!(p.footer.as_ref().unwrap().center, "Page &[PAGE]\ntwo");
    assert_eq!(p.margins.footer, 1500);
    assert_eq!(
        p.footer_area,
        HfAreaData {
            on: true,
            spacing: 300,
            autofit: false,
            left: 500,
            right: 200,
        }
    );
}

#[test]
fn absent_header_element_is_off_and_min_height_is_autofit() {
    let p = read(ods(
        r#"<style:page-layout-properties fo:margin-top="2cm"/><style:header-style><style:header-footer-properties fo:min-height="0.75cm" fo:margin-bottom="0.25cm"/></style:header-style>"#,
        r#"<style:footer><text:p>f</text:p></style:footer>"#,
    ));
    assert!(p.header.is_none());
    assert!(!p.header_area.on);
    assert_eq!(p.margins.header, 750);
    assert!(p.header_area.autofit);
    assert_eq!(p.header_area.spacing, 250);
}

#[test]
fn header_areas_round_trip() {
    let mut p = PageSetupData {
        header: Some(HeaderFooterData {
            center: "The Redwood - 10/22/2026\nBlock 90 - North Side".into(),
            ..HeaderFooterData::default()
        }),
        ..PageSetupData::default()
    };
    p.margins.top = 1000;
    p.margins.header = 1300;
    p.header_area = HfAreaData {
        on: true,
        spacing: 400,
        autofit: false,
        left: 300,
        right: 100,
    };
    p.footer_area.spacing = 500;
    let back = round_trip(&p);
    assert_eq!(back.header, p.header);
    assert_eq!(back.header_area, p.header_area);
    assert_eq!(back.footer_area, p.footer_area);
    assert_eq!(back.margins, p.margins);
    // Switched off: the texts survive, the area is written empty (AOO).
    p.header_area.on = false;
    let back = round_trip(&p);
    assert!(!back.header_area.on);
    assert_eq!(back.header, p.header);
    assert!(back.footer_area.on);
}

#[test]
fn line_breaks_in_a_region_are_new_lines() {
    let p = read(ods(
        r#"<style:page-layout-properties/>"#,
        r#"<style:header><style:region-center><text:p>The Redwood<text:line-break/>Block 90</text:p></style:region-center></style:header>"#,
    ));
    assert_eq!(p.header.unwrap().center, "The Redwood\nBlock 90");
}
