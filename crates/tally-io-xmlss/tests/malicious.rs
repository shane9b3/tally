//! Hostile inputs: every one must return quickly with a clean result or
//! error, never panic or allocate without bound.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use tally_io_xmlss::{read_xmlss, sniff_is_xmlss, Limits, XmlssError};
use tally_types::{MAX_COLS, MAX_ROWS};

const HEAD: &str = r#"<?xml version="1.0"?><Workbook xmlns="urn:schemas-microsoft-com:office:spreadsheet" xmlns:ss="urn:schemas-microsoft-com:office:spreadsheet">"#;

fn doc(body: &str) -> String {
    format!("{HEAD}{body}</Workbook>")
}

fn sheet(table: &str) -> String {
    doc(&format!(
        r#"<Worksheet ss:Name="S"><Table>{table}</Table></Worksheet>"#
    ))
}

fn read(s: &str) -> Result<tally_model::WorkbookData, XmlssError> {
    read_xmlss(s.as_bytes(), &Limits::default())
}

fn limit(r: Result<tally_model::WorkbookData, XmlssError>) -> &'static str {
    match r {
        Err(XmlssError::LimitExceeded { what, .. }) => what,
        other => panic!("expected a limit error, got {other:?}"),
    }
}

#[test]
fn doctype_with_entity_is_forbidden() {
    let s = format!(
        r#"<?xml version="1.0"?><!DOCTYPE Workbook [<!ENTITY xxe SYSTEM "file:///etc/passwd"><!ENTITY a "aaaaaaaaaa"><!ENTITY b "&a;&a;&a;&a;&a;&a;">]>{}"#,
        &sheet(r#"<Row><Cell><Data ss:Type="String">&xxe;&b;</Data></Cell></Row>"#)[21..]
    );
    assert!(matches!(read(&s), Err(XmlssError::ForbiddenDoctype)));
    // An undeclared entity is an error, not an expansion.
    let s = sheet(r#"<Row><Cell><Data ss:Type="String">&xxe;</Data></Cell></Row>"#);
    assert!(matches!(read(&s), Err(XmlssError::Xml(_))));
}

#[test]
fn deep_nesting_hits_depth_cap() {
    let n = 100_000;
    let s = sheet(&format!(
        "<Row><Cell><Data ss:Type=\"String\">{}{}</Data></Cell></Row>",
        "<b>".repeat(n),
        "</b>".repeat(n)
    ));
    let t = Instant::now();
    assert_eq!(limit(read(&s)), "XML element depth");
    assert!(t.elapsed() < Duration::from_secs(5));
}

#[test]
fn attribute_flood_hits_cap() {
    let attrs: String = (0..300).map(|i| format!(" a{i}=\"1\"")).collect();
    let s = sheet(&format!("<Row><Cell{attrs}/></Row>"));
    assert_eq!(limit(read(&s)), "XML attributes per element");
}

#[test]
fn huge_indexes_are_clamped_or_dropped() {
    let s = sheet(&format!(
        r#"<Row ss:Index="99999999999999999999"><Cell><Data ss:Type="Number">1</Data></Cell></Row>
        <Row ss:Index="{}"><Cell ss:Index="{}"><Data ss:Type="Number">2</Data></Cell><Cell><Data ss:Type="Number">3</Data></Cell></Row>
        <Row><Cell><Data ss:Type="Number">4</Data></Cell></Row>"#,
        MAX_ROWS, MAX_COLS
    ));
    let wb = read(&s).unwrap();
    let cells = &wb.sheets[0].cells;
    // The junk index is ignored (row 1), the last row and column are kept,
    // the cell after the last column and the row after the last row are
    // dropped with warnings.
    let pos: Vec<(u32, u32)> = cells.iter().map(|c| (c.row, c.col)).collect();
    assert_eq!(pos, [(0, 0), (MAX_ROWS - 1, MAX_COLS - 1)]);
    assert!(wb.warnings.iter().any(|w| w.contains("last column")));
    assert!(wb.warnings.iter().any(|w| w.contains("last row")));
    // Backwards indexes are ignored: positions keep increasing.
    let s = sheet(
        r#"<Row ss:Index="5"><Cell ss:Index="4"/><Cell ss:Index="2"><Data ss:Type="Number">1</Data></Cell></Row>
        <Row ss:Index="2"><Cell><Data ss:Type="Number">2</Data></Cell></Row>"#,
    );
    let wb = read(&s).unwrap();
    let pos: Vec<(u32, u32)> = wb.sheets[0].cells.iter().map(|c| (c.row, c.col)).collect();
    assert_eq!(pos, [(4, 4), (5, 0)]);
}

#[test]
fn huge_merge_and_span_are_clamped() {
    let s = sheet(
        r#"<Column ss:Span="4000000000" ss:Width="1e308"/><Column ss:Width="-5"/>
        <Row ss:Height="NaN"><Cell ss:MergeAcross="4294967296" ss:MergeDown="99999999"><Data ss:Type="Number">1</Data></Cell><Cell><Data ss:Type="Number">2</Data></Cell></Row>
        <Row ss:Span="99999999"/>
        <Row><Cell><Data ss:Type="Number">3</Data></Cell></Row>"#,
    );
    let wb = read(&s).unwrap();
    let sh = &wb.sheets[0];
    assert_eq!(
        sh.merges,
        [tally_model::CellRangeLocal {
            r0: 0,
            c0: 0,
            r1: MAX_ROWS - 1,
            c1: MAX_COLS - 1
        }]
    );
    assert_eq!(sh.cells.len(), 1);
    assert_eq!(sh.col_widths.len(), 1);
    assert_eq!(sh.col_widths[0].end, MAX_COLS - 1);
    assert!(sh.col_widths[0].size_hmm <= 2000 * 2540 / 72 + 1);
}

#[test]
fn cell_count_bomb_hits_cell_cap_fast() {
    // One row of 100 cells spanned over a million rows: 100 M cells.
    let cells = "<Cell><Data ss:Type=\"Number\">1</Data></Cell>".repeat(100);
    let s = sheet(&format!("<Row ss:Span=\"1048575\">{cells}</Row>"));
    let t = Instant::now();
    assert_eq!(limit(read(&s)), "materialized cells");
    assert!(t.elapsed() < Duration::from_secs(20), "{:?}", t.elapsed());
    // A small cap applies to ordinary files too.
    let lim = Limits {
        max_cells: 10,
        ..Limits::default()
    };
    let s = sheet(&"<Row><Cell><Data ss:Type=\"Number\">1</Data></Cell></Row>".repeat(11));
    assert!(matches!(
        read_xmlss(s.as_bytes(), &lim),
        Err(XmlssError::LimitExceeded { .. })
    ));
}

#[test]
fn text_budget_and_formula_length() {
    let lim = Limits {
        max_text_bytes: 1000,
        ..Limits::default()
    };
    let s = sheet(&format!(
        "<Row ss:Span=\"10\"><Cell><Data ss:Type=\"String\">{}</Data></Cell></Row>",
        "x".repeat(200)
    ));
    assert!(matches!(
        read_xmlss(s.as_bytes(), &lim),
        Err(XmlssError::LimitExceeded {
            what: "materialized cell text bytes",
            ..
        })
    ));
    let s = sheet(&format!(
        "<Row><Cell ss:Formula=\"={}\"/></Row>",
        "1+".repeat(5000)
    ));
    assert_eq!(limit(read(&s)), "formula length");
}

#[test]
fn sheet_style_and_size_caps() {
    let lim = Limits {
        max_sheets: 3,
        ..Limits::default()
    };
    let s = doc(&"<Worksheet/>".repeat(4));
    assert!(matches!(
        read_xmlss(s.as_bytes(), &lim),
        Err(XmlssError::LimitExceeded { what: "sheets", .. })
    ));
    let lim = Limits {
        max_styles: 10,
        ..Limits::default()
    };
    let styles: String = (0..11)
        .map(|i| format!("<Style ss:ID=\"s{i}\"/>"))
        .collect();
    let s = doc(&format!("<Styles>{styles}</Styles><Worksheet/>"));
    assert!(matches!(
        read_xmlss(s.as_bytes(), &lim),
        Err(XmlssError::LimitExceeded { what: "styles", .. })
    ));
    let lim = Limits {
        max_bytes: 100,
        ..Limits::default()
    };
    let s = doc("<Worksheet/>");
    assert!(matches!(
        read_xmlss(s.as_bytes(), &lim),
        Err(XmlssError::LimitExceeded {
            what: "input bytes",
            ..
        })
    ));
    let lim = Limits {
        max_event_bytes: 1024,
        ..Limits::default()
    };
    let s = sheet(&format!(
        "<Row><Cell><Data ss:Type=\"String\">{}</Data></Cell></Row>",
        "y".repeat(4096)
    ));
    assert!(matches!(
        read_xmlss(s.as_bytes(), &lim),
        Err(XmlssError::LimitExceeded {
            what: "XML event bytes",
            ..
        })
    ));
}

#[test]
fn style_parent_cycle_is_cut() {
    let s = doc(r#"<Styles>
        <Style ss:ID="a" ss:Parent="b"><Font ss:Bold="1"/></Style>
        <Style ss:ID="b" ss:Parent="c"><Font ss:Italic="1"/></Style>
        <Style ss:ID="c" ss:Parent="a"/>
        <Style ss:ID="self" ss:Parent="self"/>
        </Styles>
        <Worksheet ss:Name="S"><Table><Row>
        <Cell ss:StyleID="a"><Data ss:Type="Number">1</Data></Cell>
        <Cell ss:StyleID="self"><Data ss:Type="Number">1</Data></Cell>
        <Cell ss:StyleID="missing"><Data ss:Type="Number">1</Data></Cell>
        </Row></Table></Worksheet>"#);
    let wb = read(&s).unwrap();
    let c = &wb.sheets[0].cells[0];
    let st = &wb.styles[c.style.unwrap() as usize];
    assert!(st.font.bold && st.font.italic);
    assert!(wb.warnings.iter().any(|w| w.contains("cycle")));
    assert_eq!(wb.sheets[0].cells[1].style, None);
    assert_eq!(wb.sheets[0].cells[2].style, None);
    // A long parent chain is cut at its depth limit.
    let chain: String = (0..500)
        .map(|i| format!("<Style ss:ID=\"s{i}\" ss:Parent=\"s{}\"/>", i + 1))
        .collect();
    let s = doc(&format!(
        r#"<Styles>{chain}</Styles><Worksheet><Table><Row><Cell ss:StyleID="s0"/></Row></Table></Worksheet>"#
    ));
    let wb = read(&s).unwrap();
    assert!(wb.warnings.iter().any(|w| w.contains("deeper")));
}

#[test]
fn empty_truncated_and_invalid_inputs() {
    assert!(matches!(read(""), Err(XmlssError::NotXmlss)));
    assert!(matches!(read("   "), Err(XmlssError::NotXmlss)));
    assert!(matches!(read(&doc("")), Err(XmlssError::NoSheets)));
    assert!(matches!(
        read("<html><body/></html>"),
        Err(XmlssError::NotXmlss)
    ));
    let full = sheet(r#"<Row><Cell><Data ss:Type="Number">1</Data></Cell></Row>"#);
    for cut in [10, 50, 120, full.len() - 20, full.len() - 1] {
        assert!(read(&full[..cut]).is_err(), "cut at {cut}");
    }
    let mut bad = full.clone().into_bytes();
    let at = bad.len() - 40;
    bad[at] = 0xff;
    assert!(matches!(
        read_xmlss(&bad, &Limits::default()),
        Err(XmlssError::Encoding(_))
    ));
    // UTF-16 input is decoded.
    let mut utf16 = vec![0xFF, 0xFE];
    for u in full.encode_utf16() {
        utf16.extend_from_slice(&u.to_le_bytes());
    }
    let wb = read_xmlss(&utf16, &Limits::default()).unwrap();
    assert_eq!(wb.sheets[0].cells.len(), 1);
    utf16.push(0);
    assert!(read_xmlss(&utf16, &Limits::default()).is_err());
    // All sheets hidden: the active one is shown.
    let s = doc(
        r#"<Worksheet ss:Name="A"><WorksheetOptions><Visible>SheetHidden</Visible></WorksheetOptions></Worksheet>"#,
    );
    let wb = read(&s).unwrap();
    assert!(!wb.sheets[0].hidden);
}

#[test]
fn junk_values_never_panic() {
    let s = sheet(
        r#"<Column ss:Index="-1" ss:Width="abc" ss:StyleID=""/>
        <Row ss:Index="0" ss:Height="-1" ss:Span="-3">
        <Cell ss:Index="0" ss:MergeAcross="-1" ss:MergeDown="NaN" ss:ArrayRange="R[-99]C[999999999]" ss:Formula="="><Data ss:Type="DateTime">9999999-99-99</Data></Cell>
        <Cell ss:ArrayRange="RC:R[1048576]C[20000]" ss:Formula="=1"><Data ss:Type="Number">inf</Data></Cell>
        <Cell ss:HRef="" ss:StyleID="x"><Data ss:Type="Boolean">maybe</Data><Comment/></Cell>
        <Cell><Data ss:Type="Error">#WHAT</Data></Cell>
        </Row>"#,
    );
    let wb = read(&s).unwrap();
    let sh = &wb.sheets[0];
    assert!(sh
        .array_formulas
        .iter()
        .all(|a| a.r1 < MAX_ROWS && a.c1 < MAX_COLS));
}

#[test]
fn random_mutations_never_panic() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/xmlss/basic.xml");
    let base = std::fs::read(path).unwrap();
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut rnd = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let interesting: &[&[u8]] = &[
        b"<",
        b">",
        b"\"",
        b"&",
        b"/",
        b"9999999999",
        b"-1",
        b"\0",
        b"\xff",
        b"<!DOCTYPE x>",
        b"R[-9]C",
    ];
    for _ in 0..2000 {
        let mut b = base.clone();
        for _ in 0..(rnd() % 8 + 1) {
            if b.is_empty() {
                break;
            }
            let at = (rnd() as usize) % b.len();
            match rnd() % 4 {
                0 => b[at] = rnd() as u8,
                1 => {
                    b.remove(at);
                }
                2 => {
                    let ins = interesting[(rnd() as usize) % interesting.len()];
                    b.splice(at..at, ins.iter().copied());
                }
                _ => b.truncate(at.max(1)),
            }
        }
        let _ = sniff_is_xmlss(&b);
        let _ = read_xmlss(&b, &Limits::default());
    }
}
