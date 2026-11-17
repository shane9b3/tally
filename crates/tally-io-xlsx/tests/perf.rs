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

/// Save time of a 200,000-cell sheet (20,000 rows × 10 columns): one made
/// in Tally (rows and columns without a size: the sheet XML is edited
/// after `rust_xlsxwriter` wrote it) and the same cells in a sheet read
/// from Excel (every row and column has a size: nothing to edit). Prints
/// the two times; the first should stay close to the second.
#[test]
#[ignore = "perf: run in release with --ignored"]
fn write_200k_cells_timing() {
    use tally_model::{CellContentData, CellData, DimRun, SheetData, WorkbookData};
    use tally_types::{Value, MAX_COLS, MAX_ROWS};
    let cells: Vec<CellData> = (0..20_000u32)
        .flat_map(|row| {
            (0..10u32).map(move |col| CellData {
                row,
                col,
                content: CellContentData::Value(Value::Number(f64::from(row * 10 + col) + 0.5)),
                style: None,
            })
        })
        .collect();
    let whole = |count: u32, size_hmm: u32| {
        vec![DimRun {
            start: 0,
            end: count - 1,
            size_hmm,
            hidden: false,
        }]
    };
    let made_here = SheetData {
        name: "S".into(),
        cells,
        ..SheetData::default()
    };
    let from_excel = SheetData {
        row_heights: whole(MAX_ROWS, 529),
        col_widths: whole(MAX_COLS, 1693),
        ..made_here.clone()
    };
    for (what, sheet) in [
        ("made in Tally", made_here),
        ("read from Excel", from_excel),
    ] {
        let wb = WorkbookData {
            sheets: vec![sheet],
            ..WorkbookData::default()
        };
        let mut best = Duration::MAX;
        let mut size = 0;
        for _ in 0..7 {
            let mut out = Cursor::new(Vec::new());
            let t = Instant::now();
            tally_io_xlsx::write_xlsx(&wb, &mut out).unwrap();
            best = best.min(t.elapsed());
            size = out.into_inner().len();
        }
        eprintln!("TIMING write 200k cells, {what}: {best:?} ({size} bytes)");
    }
}
