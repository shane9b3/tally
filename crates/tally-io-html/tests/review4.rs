//! Security review 4 (H13, H14, M8) and correctness review 4 (M36).
#![allow(clippy::unwrap_used)]

use std::time::{Duration, Instant};

use tally_io_html::{read_html, write_html, MAX_CELL_BYTES};
use tally_model::{CellContentData, CellData, SheetData, WorkbookData};
use tally_types::Value;

#[test]
fn rowspans_are_linear() {
    // 2,000 cells with rowspan=1000 in one row, then 1,000 rows: the span
    // list was scanned per column (> 60 s at 70 KB before).
    let mut h = String::from("<table><tr>");
    for _ in 0..2000 {
        h.push_str("<td rowspan=1000>x</td>");
    }
    h.push_str("</tr>");
    for _ in 0..1000 {
        h.push_str("<tr><td>y</td></tr>");
    }
    h.push_str("</table>");
    let t = Instant::now();
    let (wb, _) = read_html(h.as_bytes()).unwrap();
    assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
    // Row 1's "y" lands after the 2,000 covered columns.
    let y = wb.sheets[0].cells.iter().find(|c| c.row == 1).unwrap();
    assert_eq!(y.col, 2000);
}

#[test]
fn rowspan_and_colspan_cover_like_before() {
    let h = "<table><tr><td rowspan=2 colspan=2>a</td><td>b</td></tr>\
             <tr><td>c</td></tr><tr><td>d</td></tr></table>";
    let (wb, _) = read_html(h.as_bytes()).unwrap();
    let at = |t: &str| {
        let c = wb.sheets[0]
            .cells
            .iter()
            .find(|c| c.content == CellContentData::Value(Value::text(t)))
            .unwrap();
        (c.row, c.col)
    };
    assert_eq!(at("b"), (0, 2));
    assert_eq!(at("c"), (1, 2));
    assert_eq!(at("d"), (2, 0));
}

#[test]
fn script_skipping_is_linear() {
    // 30,000 scripts: the rest of the input was lowercased per script.
    let mut h = String::new();
    for _ in 0..30_000 {
        h.push_str("<script>var a = 1; if (a</b) {}</script>");
    }
    h.push_str("<table><tr><td>ok</td></tr></table><STYLE>x</sTyLe>");
    let t = Instant::now();
    let (wb, _) = read_html(h.as_bytes()).unwrap();
    assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
    assert_eq!(wb.sheets[0].cells.len(), 1);
}

#[test]
fn cell_text_is_cut_at_the_cap() {
    let big = "é".repeat(MAX_CELL_BYTES); // 128 KiB, 2-byte chars
    let h = format!("<table><tr><td>a{big}</td></tr></table>");
    let (wb, _) = read_html(h.as_bytes()).unwrap();
    match &wb.sheets[0].cells[0].content {
        CellContentData::Value(Value::Text(t)) => {
            assert!(t.len() <= MAX_CELL_BYTES, "{}", t.len());
            assert!(t.len() > MAX_CELL_BYTES - 4);
        }
        o => panic!("{o:?}"),
    }
}

#[test]
fn html_lang_selects_the_locale_and_export_writes_it() {
    let (wb, _) =
        read_html(b"<html lang=\"de\"><table><tr><td>1234,5</td></tr></table></html>").unwrap();
    assert_eq!(wb.settings.locale.as_deref(), Some("de-DE"));
    let (wb, _) = read_html(b"<table><tr><td>1</td></tr></table>").unwrap();
    assert_eq!(wb.settings.locale, None);
    let (wb, _) = read_html(b"<html lang=\"tlh\"><table><tr><td>1</td></tr></table>").unwrap();
    assert_eq!(wb.settings.locale, None);

    let mut data = WorkbookData {
        sheets: vec![SheetData {
            name: "S".into(),
            cells: vec![CellData {
                row: 0,
                col: 0,
                content: CellContentData::Value(Value::Number(1234.5)),
                style: None,
            }],
            ..SheetData::default()
        }],
        ..WorkbookData::default()
    };
    data.settings.locale = Some("de-DE".into());
    let html = write_html(&data).unwrap();
    assert!(html.contains("<html lang=\"de-DE\">"), "{html}");
    let (back, _) = read_html(html.as_bytes()).unwrap();
    assert_eq!(back.settings.locale.as_deref(), Some("de-DE"));
}
