//! Security review of the 2 GiB worksheet cap: strings that parsers keep
//! from attributes are bounded per package (H1), and the compression ratio,
//! shared worksheet parts and re-reads are budgeted per package (M1).

#![allow(clippy::unwrap_used)] // tests may unwrap (docs/contributing.md)

use std::io::{Cursor, Write};
use std::time::{Duration, Instant};

use tally_io_xlsx::{read_xlsx, Limits, XlsxError};
use tally_model::WorkbookData;

const NS: &str = r#"xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;
const REL_WS: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet";

/// A workbook whose sheets point at the given worksheet targets, plus parts.
fn package(sheets: &[(&str, &str)], parts: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default();
    let mut rels = String::from(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
    );
    let mut wb = format!("<workbook {NS}><sheets>");
    for (i, (name, target)) in sheets.iter().enumerate() {
        rels.push_str(&format!(
            r#"<Relationship Id="rId{i}" Type="{REL_WS}" Target="{target}"/>"#
        ));
        wb.push_str(&format!(
            r#"<sheet name="{name}" sheetId="{}" r:id="rId{i}"/>"#,
            i + 1
        ));
    }
    rels.push_str("</Relationships>");
    wb.push_str("</sheets></workbook>");
    z.start_file("xl/workbook.xml", opts).unwrap();
    z.write_all(wb.as_bytes()).unwrap();
    z.start_file("xl/_rels/workbook.xml.rels", opts).unwrap();
    z.write_all(rels.as_bytes()).unwrap();
    for (name, data) in parts {
        z.start_file(name.as_str(), opts).unwrap();
        z.write_all(data).unwrap();
    }
    z.finish().unwrap().into_inner()
}

fn sheet_xml(body: &str, after: &str) -> Vec<u8> {
    format!("<worksheet {NS}><sheetData>{body}</sheetData>{after}</worksheet>").into_bytes()
}

fn read(bytes: Vec<u8>, limits: &Limits) -> Result<WorkbookData, XlsxError> {
    read_xlsx(Cursor::new(bytes), limits)
}

fn limit_what(r: &Result<WorkbookData, XlsxError>) -> &'static str {
    match r {
        Err(XlsxError::LimitExceeded { what, .. }) => what,
        other => panic!("expected LimitExceeded, got {other:?}"),
    }
}

fn small_budget() -> Limits {
    Limits {
        max_attr_bytes: 64 * 1024,
        ..Limits::default()
    }
}

/// The review's vector: hyperlinks with 20 KB `r:id`s (99k of them reached
/// 2.2 GB). An over-long id is dropped, and all of them are charged.
#[test]
fn hyperlink_ids_are_bounded() {
    let long = "i".repeat(20_000);
    let links: String = (1..=10)
        .map(|r| format!(r#"<hyperlink ref="A{r}" r:id="{long}{r}"/>"#))
        .collect();
    let ws = sheet_xml(
        r#"<row r="1"><c r="A1"><v>1</v></c></row>"#,
        &format!("<hyperlinks>{links}</hyperlinks>"),
    );
    let bytes = package(
        &[("S", "worksheets/sheet1.xml")],
        &[("xl/worksheets/sheet1.xml".into(), ws)],
    );
    // Opens (the links have no target), then the budget refuses many.
    assert_eq!(
        read(bytes.clone(), &Limits::default())
            .unwrap()
            .sheets
            .len(),
        1
    );
    assert_eq!(
        limit_what(&read(bytes, &small_budget())),
        "XML attribute bytes"
    );
}

#[test]
fn over_long_sheet_name_is_refused() {
    let name = "n".repeat(2000);
    let bytes = package(
        &[(name.as_str(), "worksheets/sheet1.xml")],
        &[("xl/worksheets/sheet1.xml".into(), sheet_xml("", ""))],
    );
    assert_eq!(
        limit_what(&read(bytes, &Limits::default())),
        "sheet name bytes"
    );
}

/// M1: 400 sheets pointing at one part made a 25 KB file cost 34 s of CPU.
#[test]
fn sheets_cannot_share_a_worksheet_part() {
    let names: Vec<String> = (0..3).map(|i| format!("S{i}")).collect();
    let sheets: Vec<(&str, &str)> = names
        .iter()
        .map(|n| (n.as_str(), "worksheets/sheet1.xml"))
        .collect();
    let bytes = package(
        &sheets,
        &[("xl/worksheets/sheet1.xml".into(), sheet_xml("", ""))],
    );
    match read(bytes, &Limits::default()) {
        Err(XlsxError::Xml(m)) => assert!(m.contains("same worksheet part"), "{m}"),
        other => panic!("{other:?}"),
    }
}

/// M1: the 10 MiB ratio-free allowance is per package, so 20 sheets of
/// 4 MiB of whitespace each (~1000:1, each under the floor) are refused
/// after about 10 MiB of output in total.
#[test]
fn ratio_floor_is_per_package() {
    let names: Vec<String> = (0..20).map(|i| format!("S{i}")).collect();
    let targets: Vec<String> = (0..20).map(|i| format!("worksheets/s{i}.xml")).collect();
    let sheets: Vec<(&str, &str)> = names
        .iter()
        .zip(&targets)
        .map(|(n, t)| (n.as_str(), t.as_str()))
        .collect();
    let pad = " ".repeat(4 * 1024 * 1024);
    let parts: Vec<(String, Vec<u8>)> = targets
        .iter()
        .map(|t| (format!("xl/{t}"), sheet_xml(&pad, "")))
        .collect();
    let bytes = package(&sheets, &parts);
    assert!(bytes.len() < 1024 * 1024, "{}", bytes.len());
    let t = Instant::now();
    assert_eq!(
        limit_what(&read(bytes, &Limits::default())),
        "compression ratio"
    );
    assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
}
