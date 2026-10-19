//! Read performance: 1,000,000 cells (100,000 rows × 10 columns of numbers,
//! shared strings and formulas) must load in under 3 s in a release build.
//!
//! `cargo test --release -p tally-io-xlsx --test perf -- --ignored --nocapture`

#![allow(clippy::unwrap_used)] // tests may unwrap (docs/contributing.md)

use std::fmt::Write as _;
use std::io::{Cursor, Write};
use std::time::{Duration, Instant};

use tally_io_xlsx::{read_xlsx, Limits};

fn million_cells() -> Vec<u8> {
    let mut sheet = String::with_capacity(64 * 1024 * 1024);
    sheet.push_str(r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>"#);
    for r in 1..=100_000u32 {
        let _ = write!(sheet, r#"<row r="{r}" spans="1:10">"#);
        for (i, col) in ["A", "B", "C", "D", "E", "F", "G", "H"].iter().enumerate() {
            let _ = write!(sheet, r#"<c r="{col}{r}"><v>{}.{}</v></c>"#, r, i);
        }
        let _ = write!(sheet, r#"<c r="I{r}" t="s"><v>{}</v></c>"#, r % 1000);
        if r == 1 {
            sheet.push_str(
                r#"<c r="J1"><f t="shared" ref="J1:J100000" si="0">A1+B1</f><v>1</v></c>"#,
            );
        } else {
            let _ = write!(sheet, r#"<c r="J{r}"><f t="shared" si="0"/><v>1</v></c>"#);
        }
        sheet.push_str("</row>");
    }
    sheet.push_str("</sheetData></worksheet>");
    let mut sst =
        String::from(r#"<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">"#);
    for i in 0..1000 {
        let _ = write!(sst, "<si><t>string number {i}</t></si>");
    }
    sst.push_str("</sst>");
    let rels = concat!(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
        r#"<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>"#,
        r#"<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="sharedStrings.xml"/>"#,
        "</Relationships>"
    );
    let wb = concat!(
        r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" "#,
        r#"xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">"#,
        r#"<sheets><sheet name="Big" sheetId="1" r:id="rId1"/></sheets></workbook>"#
    );
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default().compression_level(Some(1));
    for (name, data) in [
        ("xl/workbook.xml", wb),
        ("xl/_rels/workbook.xml.rels", rels),
        ("xl/sharedStrings.xml", sst.as_str()),
        ("xl/worksheets/sheet1.xml", sheet.as_str()),
    ] {
        z.start_file(name, opts).unwrap();
        z.write_all(data.as_bytes()).unwrap();
    }
    z.finish().unwrap().into_inner()
}

#[test]
#[ignore = "perf: run in release with --ignored"]
fn read_one_million_cells_under_three_seconds() {
    let bytes = million_cells();
    eprintln!("package: {} bytes", bytes.len());
    let t = Instant::now();
    let wb = read_xlsx(Cursor::new(bytes), &Limits::default()).unwrap();
    let elapsed = t.elapsed();
    eprintln!("TIMING read 1M cells: {elapsed:?}");
    assert_eq!(wb.sheets[0].cells.len(), 1_000_000);
    assert!(elapsed < Duration::from_secs(3), "took {elapsed:?}");
}
