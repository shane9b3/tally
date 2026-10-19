//! Regression tests for the second security review
//! (`docs/reviews/security-review-2-2026-10-19.md`), XLSX reader side:
//! HIGH-1 (merges counted and capped), HIGH-2 (array block areas count
//! against the cell budget), HIGH-4 (long `numFmt` codes), MEDIUM-1
//! (shared formulas share one text), MEDIUM-2 (`<col style>` runs), LOW-2
//! (`'a:b'!A1`) and the INFO `iterateCount` clamp. Inputs are generated
//! here (the review's `p*.py` shapes at smaller sizes).

#![allow(clippy::unwrap_used)] // tests may unwrap (docs/contributing.md)

use std::fmt::Write as _;
use std::io::{Cursor, Write};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tally_io_xlsx::{read_xlsx, Limits, XlsxError};
use tally_model::{CellContentData, WorkbookData};

const NS: &str = r#"xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;

/// A package with the given sheets (name, worksheet XML), optional styles
/// and `workbookPr`/`calcPr` XML.
fn package(sheets: &[(&str, String)], styles: Option<&str>, calc: &str) -> Vec<u8> {
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default();
    let mut rels = String::from(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
    );
    let mut list = String::new();
    for (i, (name, _)) in sheets.iter().enumerate() {
        let _ = write!(
            rels,
            r#"<Relationship Id="rId{n}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet{n}.xml"/>"#,
            n = i + 1
        );
        let _ = write!(
            list,
            r#"<sheet name="{name}" sheetId="{n}" r:id="rId{n}"/>"#,
            n = i + 1
        );
    }
    if styles.is_some() {
        rels.push_str(r#"<Relationship Id="rIdS" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>"#);
    }
    rels.push_str("</Relationships>");
    let mut entries = vec![
        (
            "xl/workbook.xml".to_string(),
            format!("<workbook {NS}><sheets>{list}</sheets>{calc}</workbook>"),
        ),
        ("xl/_rels/workbook.xml.rels".to_string(), rels),
    ];
    for (i, (_, xml)) in sheets.iter().enumerate() {
        entries.push((format!("xl/worksheets/sheet{}.xml", i + 1), xml.clone()));
    }
    if let Some(s) = styles {
        entries.push(("xl/styles.xml".to_string(), s.to_string()));
    }
    for (name, data) in entries {
        z.start_file(name, opts).unwrap();
        z.write_all(data.as_bytes()).unwrap();
    }
    z.finish().unwrap().into_inner()
}

fn ws(pre: &str, rows: &str, post: &str) -> String {
    format!("<worksheet {NS}>{pre}<sheetData>{rows}</sheetData>{post}</worksheet>")
}

fn read(bytes: Vec<u8>, limits: &Limits) -> (Result<WorkbookData, XlsxError>, Duration) {
    let t = Instant::now();
    let r = read_xlsx(Cursor::new(bytes), limits);
    (r, t.elapsed())
}

fn limit_what(r: &Result<WorkbookData, XlsxError>) -> &'static str {
    match r {
        Err(XlsxError::LimitExceeded { what, .. }) => what,
        other => panic!("expected LimitExceeded, got {other:?}"),
    }
}

fn col_name(mut c: u32) -> String {
    let mut s = String::new();
    c += 1;
    while c > 0 {
        let r = (c - 1) % 26;
        s.insert(0, (b'A' + r as u8) as char);
        c = (c - 1) / 26;
    }
    s
}

/// HIGH-1 (`p5_xmerges.py`): 300,000 `<mergeCell>`s, 8,000 per row. The
/// first 100,000 are kept (with a warning), each counted against
/// `max_cells`.
#[test]
fn merges_are_counted_and_capped() {
    let mut m = String::from("<mergeCells>");
    for i in 0..300_000u32 {
        let (r, c) = (i / 8000 + 1, 2 * (i % 8000));
        let _ = write!(
            m,
            r#"<mergeCell ref="{}{r}:{}{r}"/>"#,
            col_name(c),
            col_name(c + 1)
        );
    }
    m.push_str("</mergeCells>");
    let bytes = package(&[("S", ws("", "", &m))], None, "");
    let (r, t) = read(bytes.clone(), &Limits::default());
    let wb = r.unwrap();
    assert_eq!(wb.sheets[0].merges.len(), 100_000);
    assert!(wb.warnings.iter().any(|w| w.contains("merged areas")));
    assert!(t < Duration::from_secs(10), "took {t:?}");
    // They count against the cell cap (they were free before).
    let small = Limits {
        max_cells: 50_000,
        ..Limits::default()
    };
    assert_eq!(limit_what(&read(bytes, &small).0), "materialized cells");
}

/// HIGH-2: an array formula's block area counts against `max_cells`.
#[test]
fn array_block_area_counts() {
    let mut rows = String::new();
    for c in 0..12u32 {
        let col = col_name(c);
        let _ = write!(
            rows,
            r#"<c r="{col}1"><f t="array" ref="{col}1:{col}1048576">ROW()</f><v>1</v></c>"#
        );
    }
    let bytes = package(
        &[("S", ws("", &format!(r#"<row r="1">{rows}</row>"#), ""))],
        None,
        "",
    );
    let (r, t) = read(bytes, &Limits::default());
    assert_eq!(limit_what(&r), "materialized cells");
    assert!(t < Duration::from_secs(1), "took {t:?}");
}

/// HIGH-4 (`p9_numfmt.py`): a 1 MB `formatCode` reads as General with a
/// warning; a normal custom code is kept.
#[test]
fn over_long_format_codes_become_general() {
    let styles = format!(
        r#"<styleSheet {NS}><numFmts count="2"><numFmt numFmtId="200" formatCode="{}"/><numFmt numFmtId="201" formatCode="0.0&quot; kg&quot;"/></numFmts><cellXfs count="3"><xf numFmtId="0"/><xf numFmtId="200" applyNumberFormat="1"/><xf numFmtId="201" applyNumberFormat="1"/></cellXfs></styleSheet>"#,
        "0".repeat(1 << 20)
    );
    let rows = r#"<row r="1"><c r="A1" s="1"><v>1</v></c><c r="B1" s="2"><v>2</v></c></row>"#;
    let (r, t) = read(
        package(&[("S", ws("", rows, ""))], Some(&styles), ""),
        &Limits::default(),
    );
    let wb = r.unwrap();
    assert!(t < Duration::from_secs(2), "took {t:?}");
    let fmt = |col: u32| {
        let c = wb.sheets[0].cells.iter().find(|c| c.col == col).unwrap();
        c.style
            .and_then(|i| wb.styles[i as usize].number_format.clone())
    };
    assert_eq!(fmt(0), None);
    assert_eq!(fmt(1).as_deref(), Some("0.0\" kg\""));
    assert!(wb
        .warnings
        .iter()
        .any(|w| w.contains("Number formats longer than")));
}

/// MEDIUM-1 (`p6_shared.py`): one anchor plus 100,000 `<f t="shared"/>`
/// members. Every member shares the anchor's text allocation, and the
/// text counts once against `max_text_bytes` (150 B × 100k = 15 MB of
/// per-member text before).
#[test]
fn shared_formula_members_share_one_text() {
    let n = 100_000u32;
    let f = vec!["A1"; 50].join("+");
    let mut rows = format!(
        r#"<row r="1"><c r="B1"><f t="shared" ref="B1:B{n}" si="0">{f}</f><v>0</v></c></row>"#
    );
    for r in 2..=n {
        let _ = write!(
            rows,
            r#"<row r="{r}"><c r="B{r}"><f t="shared" si="0"/><v>{r}</v></c></row>"#
        );
    }
    let limits = Limits {
        max_text_bytes: 1024 * 1024,
        ..Limits::default()
    };
    let (r, t) = read(package(&[("S", ws("", &rows, ""))], None, ""), &limits);
    let wb = r.unwrap();
    assert!(t < Duration::from_secs(10), "took {t:?}");
    let cells = &wb.sheets[0].cells;
    assert_eq!(cells.len(), n as usize);
    let CellContentData::Formula { text: anchor, .. } = &cells[0].content else {
        panic!("{:?}", cells[0].content);
    };
    for c in &cells[1..] {
        match &c.content {
            CellContentData::SharedFormula {
                text,
                anchor_row: 0,
                anchor_col: 1,
                ..
            } => assert!(Arc::ptr_eq(text, anchor)),
            other => panic!("{other:?}"),
        }
    }
    // Consumers other than the engine can print members at their cells.
    let mut wb = wb;
    tally_io_xlsx::expand_shared_formulas(&mut wb);
    match &wb.sheets[0].cells[4].content {
        CellContentData::Formula { text, .. } => assert!(text.starts_with("of:=[.A5]+[.A5]")),
        other => panic!("{other:?}"),
    }
}

/// MEDIUM-2 (`p4_cols.py`): 10,000 `<col min="1" max="16384" style>`
/// elements made 164M style runs (5.8 s, 3.5 GB); now at most one run per
/// column, overlapping entries resolved last-wins.
#[test]
fn column_styles_are_one_run_per_column() {
    let styles = format!(
        r#"<styleSheet {NS}><fonts count="2"><font><sz val="11"/></font><font><b/><sz val="11"/></font></fonts><fills count="1"><fill/></fills><cellXfs count="3"><xf numFmtId="0" fontId="0"/><xf numFmtId="0" fontId="1" applyFont="1"/><xf numFmtId="2" fontId="0" applyNumberFormat="1"/></cellXfs></styleSheet>"#
    );
    let mut cols = String::from("<cols>");
    for _ in 0..10_000 {
        cols.push_str(r#"<col min="1" max="16384" style="1"/>"#);
    }
    cols.push_str(r#"<col min="3" max="4" style="2"/></cols>"#);
    let (r, t) = read(
        package(
            &[(
                "S",
                ws(&cols, r#"<row r="1"><c r="A1"><v>1</v></c></row>"#, ""),
            )],
            Some(&styles),
            "",
        ),
        &Limits::default(),
    );
    let wb = r.unwrap();
    assert!(t < Duration::from_secs(5), "took {t:?}");
    let runs = &wb.sheets[0].style_runs;
    assert_eq!(runs.len(), 16_384);
    // The last entry (C:D) wins over the whole-row ones.
    let st = |c: u32| runs.iter().find(|r| r.col == c).unwrap().style;
    assert_ne!(st(2), st(0));
    assert_eq!(st(3), st(2));
    assert_eq!(st(4), st(0));
    // More than 16,384 `<col>` elements: the rest are ignored, with a warning.
    let many = format!(
        "<cols>{}</cols>",
        r#"<col min="1" max="1" style="1"/>"#.repeat(20_000)
    );
    let wb = read(
        package(&[("S", ws(&many, "", ""))], Some(&styles), ""),
        &Limits::default(),
    )
    .0
    .unwrap();
    assert_eq!(wb.sheets[0].style_runs.len(), 1);
    assert!(wb.warnings.iter().any(|w| w.contains("column definitions")));
}

/// LOW-2: a sheet named `a:b` is read by `'a:b'!A1` (the ExcelA1 parser saw
/// a 3-D span `a` to `b` and gave `#VALUE!`/Err:501), while a real span
/// between existing sheets still is one.
#[test]
fn quoted_sheet_names_with_colons() {
    let one = |v: &str| {
        ws(
            "",
            &format!(r#"<row r="1"><c r="A1"><v>{v}</v></c></row>"#),
            "",
        )
    };
    let main = ws(
        "",
        r#"<row r="1"><c r="A1"><f>'a:b'!A1</f></c><c r="B1"><f>SUM('a:b'!A:A)</f></c><c r="C1"><f>"'a:b'!"&amp;'a:b'!A1</f></c><c r="D1"><f>SUM(a:b!A1)</f></c></row>"#,
        "",
    );
    let bytes = package(
        &[
            ("Main", main),
            ("a:b", one("100")),
            ("a", one("1")),
            ("b", one("2")),
        ],
        None,
        "",
    );
    let wb = read(bytes, &Limits::default()).0.unwrap();
    let text = |col: u32| match &wb.sheets[0]
        .cells
        .iter()
        .find(|c| c.col == col)
        .unwrap()
        .content
    {
        CellContentData::Formula { text, .. } => text.to_string(),
        other => panic!("{other:?}"),
    };
    assert_eq!(text(0), "of:=['a:b'.A1]");
    assert_eq!(text(1), "of:=SUM(['a:b'.A$1:.A$1048576])");
    assert_eq!(text(2), "of:=\"'a:b'!\"&['a:b'.A1]");
    assert_eq!(text(3), "of:=SUM([a.A1:b.A1])");
}

/// INFO: `calcPr iterateCount` is clamped to Excel's 32,767.
#[test]
fn iterate_count_is_clamped() {
    let bytes = package(
        &[("S", ws("", "", ""))],
        None,
        r#"<calcPr iterate="1" iterateCount="4000000000"/>"#,
    );
    let wb = read(bytes, &Limits::default()).0.unwrap();
    assert_eq!(
        wb.settings.calc_settings.iterations.map(|i| i.steps),
        Some(32_767)
    );
}
