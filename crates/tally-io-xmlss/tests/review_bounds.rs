//! Regression tests for the 2026-11 format review (H1, H2, L2): work and
//! output are bounded and fast.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Mutex;
use std::time::{Duration, Instant};

use tally_io_xmlss::{read_xmlss, Limits, XmlssError};

// The workspace forbids `unsafe`, so there is no counting allocator here:
// memory is bounded by construction (the budgets refuse before anything is
// materialized) and checked through the sizes of what is returned and the
// time taken (the reviewer's harness measured RSS: see docs/security.md).

/// One measurement at a time (the peak is process-wide).
static SERIAL: Mutex<()> = Mutex::new(());

/// Runs `f`; returns its result and the time taken.
fn measure<T>(f: impl FnOnce() -> T) -> (T, Duration, ()) {
    let t = Instant::now();
    let out = f();
    (out, t.elapsed(), ())
}

const HEAD: &str = r#"<?xml version="1.0"?><Workbook xmlns="urn:schemas-microsoft-com:office:spreadsheet" xmlns:ss="urn:schemas-microsoft-com:office:spreadsheet"><Styles><Style ss:ID="s"><Font ss:Bold="1"/></Style></Styles>"#;

fn doc(body: &str) -> String {
    format!("{HEAD}{body}</Workbook>")
}

#[test]
fn h1_column_styles_are_budgeted_and_only_the_first_table_is_read() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // 200 tables in one sheet, each styling every column: one table read.
    let tables = r#"<Table><Column ss:Span="16383" ss:StyleID="s"/><Row><Cell><Data ss:Type="Number">1</Data></Cell></Row></Table>"#
        .repeat(200);
    let s = doc(&format!(r#"<Worksheet ss:Name="A">{tables}</Worksheet>"#));
    let (r, t, _) = measure(|| read_xmlss(s.as_bytes(), &Limits::default()));
    let wb = r.unwrap();
    assert_eq!(wb.sheets[0].style_runs.len(), 16_384);
    assert_eq!(
        wb.sheets[0].cells.len(),
        1,
        "no duplicate cells from later tables"
    );
    assert!(wb.warnings.iter().any(|w| w.contains("first table")));
    assert!(t < Duration::from_secs(5), "{t:?}");
    // Many sheets styling every column: the runs count against the cells.
    let sheets: String = (0..20)
        .map(|i| format!(r#"<Worksheet ss:Name="S{i}"><Table><Column ss:Span="16383" ss:StyleID="s"/></Table></Worksheet>"#))
        .collect();
    let s = doc(&sheets);
    let small = Limits {
        max_cells: 100_000,
        ..Limits::default()
    };
    let (r, t, _) = measure(|| read_xmlss(s.as_bytes(), &small));
    assert!(matches!(r, Err(XmlssError::LimitExceeded { .. })), "{r:?}");
    assert!(t < Duration::from_secs(2), "{t:?}");
}

#[test]
fn h2_spanned_rows_cost_what_they_place() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // A row repeated over the whole sheet with 16,384 empty cells.
    let s = doc(&format!(
        r#"<Worksheet ss:Name="A"><Table><Row ss:Span="1048575">{}</Row></Table></Worksheet>"#,
        "<Cell/>".repeat(16_384)
    ));
    let (r, t, _) = measure(|| read_xmlss(s.as_bytes(), &Limits::default()));
    assert!(r.unwrap().sheets[0].cells.is_empty());
    assert!(t < Duration::from_secs(5), "{t:?}");
    // The same with a hyperlink on every cell: refused before placing.
    let s = doc(&format!(
        r#"<Worksheet ss:Name="A"><Table><Row ss:Span="1048575">{}</Row></Table></Worksheet>"#,
        r#"<Cell ss:HRef="http://x"/>"#.repeat(2_000)
    ));
    let (r, t, _) = measure(|| read_xmlss(s.as_bytes(), &Limits::default()));
    assert!(matches!(r, Err(XmlssError::LimitExceeded { .. })), "{r:?}");
    assert!(t < Duration::from_secs(5), "{t:?}");
    // A note repeated down the sheet: its text is checked first.
    let note = "n".repeat(30_000);
    let s = doc(&format!(
        r#"<Worksheet ss:Name="A"><Table><Row ss:Span="1048575"><Cell><Comment><Data>{note}</Data></Comment></Cell></Row></Table></Worksheet>"#
    ));
    let (r, t, _) = measure(|| read_xmlss(s.as_bytes(), &Limits::default()));
    assert!(matches!(r, Err(XmlssError::LimitExceeded { .. })), "{r:?}");
    assert!(t < Duration::from_secs(2), "{t:?}");
    // Past the per-sheet link cap, placing stays linear (one warning).
    let s = doc(&format!(
        r#"<Worksheet ss:Name="A"><Table><Row ss:Span="1199"><Cell ss:HRef="http://x"/>{}</Row></Table></Worksheet>"#,
        r#"<Cell ss:HRef="http://y"/>"#.repeat(99)
    ));
    let (r, t, _) = measure(|| read_xmlss(s.as_bytes(), &Limits::default()));
    let wb = r.unwrap();
    assert_eq!(wb.sheets[0].links.len(), 100_000);
    assert_eq!(
        wb.warnings
            .iter()
            .filter(|w| w.contains("hyperlinks"))
            .count(),
        1
    );
    assert!(t < Duration::from_secs(5), "{t:?}");
}

#[test]
fn l2_repeated_sheet_names_are_linear() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let sheets = r#"<Worksheet ss:Name="A"><Table/></Worksheet>"#.repeat(10_000);
    let s = doc(&sheets);
    let (r, t, _) = measure(|| read_xmlss(s.as_bytes(), &Limits::default()));
    let wb = r.unwrap();
    assert_eq!(wb.sheets.len(), 10_000);
    assert_eq!(wb.sheets[9_999].name, "A (10000)");
    let mut names: Vec<_> = wb.sheets.iter().map(|s| s.name.to_lowercase()).collect();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), 10_000);
    assert!(t < Duration::from_secs(3), "{t:?}");
}
