//! The hand-written fixtures in `tests/fixtures/xmlss/`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;

use tally_io_xmlss::{read_xmlss, sniff_is_xmlss, Limits};
use tally_model::{
    BorderStyle, CellContentData, CellData, CellRangeLocal, HAlign, SheetData, StyleData,
    UnderlineStyleData, VAlign, WorkbookData,
};
use tally_types::{ErrorKind, Value, MAX_COLS, MAX_ROWS};

fn fixture(name: &str) -> Vec<u8> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/xmlss")
        .join(name);
    std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn basic() -> WorkbookData {
    read_xmlss(&fixture("basic.xml"), &Limits::default()).unwrap()
}

fn cell(s: &SheetData, row: u32, col: u32) -> &CellData {
    s.cells
        .iter()
        .find(|c| c.row == row && c.col == col)
        .unwrap_or_else(|| panic!("no cell at {row},{col}"))
}

fn value(s: &SheetData, row: u32, col: u32) -> Value {
    match &cell(s, row, col).content {
        CellContentData::Value(v) => v.clone(),
        other => panic!("{other:?}"),
    }
}

fn style<'a>(wb: &'a WorkbookData, c: &CellData) -> &'a StyleData {
    &wb.styles[c.style.expect("styled") as usize]
}

fn formula(s: &SheetData, row: u32, col: u32) -> (String, Option<Value>) {
    match &cell(s, row, col).content {
        CellContentData::Formula { text, cached } => (text.to_string(), cached.clone()),
        other => panic!("{other:?}"),
    }
}

#[test]
fn fixture_sniffs() {
    assert!(sniff_is_xmlss(&fixture("basic.xml")));
    assert!(sniff_is_xmlss(&fixture("prefixed.xml")));
}

#[test]
fn sheets_names_and_visibility() {
    let wb = basic();
    let names: Vec<&str> = wb.sheets.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Data", "Other", "Secret", "Data (2)"]);
    assert!(wb.sheets[2].hidden);
    assert!(!wb.sheets[0].hidden);
    assert_eq!(wb.settings.active_sheet, 1);
    assert!(wb.sheets[1].protection.is_some());
    assert!(wb.warnings.iter().any(|w| w.contains("Sheet names")));
}

#[test]
fn values_of_every_type() {
    let wb = basic();
    let s = &wb.sheets[0];
    assert_eq!(value(s, 0, 0), Value::Number(1.5));
    assert_eq!(value(s, 0, 1), Value::text("hello"));
    assert_eq!(value(s, 0, 2), Value::Bool(true));
    // 1999-12-31 in D1899.
    assert_eq!(value(s, 0, 3), Value::Number(36525.0));
    assert_eq!(
        style(&wb, cell(s, 0, 3)).number_format.as_deref(),
        Some("D-M-YY")
    );
    assert_eq!(value(s, 0, 4), Value::Error(ErrorKind::NA));
    // A DateTime without a format gets one.
    assert_eq!(value(s, 1, 3), Value::Number(46314.5));
    assert_eq!(
        style(&wb, cell(s, 1, 3)).number_format.as_deref(),
        Some("YYYY-MM-DD HH:MM:SS")
    );
    // Rich text is concatenated.
    assert_eq!(value(s, 10, 0), Value::text("Rich text"));
}

#[test]
fn r1c1_formulas() {
    let wb = basic();
    let s = &wb.sheets[0];
    assert_eq!(
        formula(s, 1, 0),
        ("of:=[.A1]*2".into(), Some(Value::Number(3.0)))
    );
    assert_eq!(formula(s, 1, 1).0, "of:=SUM([.$A$1:.$A$1])+[Other.B2]");
    // Untranslatable: original text kept, cached value too.
    assert_eq!(
        formula(s, 1, 4),
        ("of:=BOGUS((".into(), Some(Value::Number(7.0)))
    );
    assert!(wb
        .warnings
        .iter()
        .any(|w| w.contains("could not be translated")));
    let o = &wb.sheets[1];
    assert_eq!(formula(o, 1, 1).0, "of:=[Data.$A$1]+[.A1]");
}

#[test]
fn positions_merges_spans_and_links() {
    let wb = basic();
    let s = &wb.sheets[0];
    // Row 5 (index 4): merged A5:C6, then D5 and E5 after the merge.
    assert!(s.merges.contains(&CellRangeLocal {
        r0: 4,
        c0: 0,
        r1: 5,
        c1: 2
    }));
    assert_eq!(value(s, 4, 3), Value::Number(0.25));
    assert_eq!(value(s, 4, 4), Value::Number(1234.5));
    assert_eq!(s.links.len(), 1);
    assert_eq!((s.links[0].row, s.links[0].col), (4, 4));
    assert_eq!(&*s.links[0].url, "https://example.com/");
    // Row 8 with Span 2: rows 8–10 (indexes 7–9), all styled by the row.
    for r in 7..=9 {
        assert_eq!(value(s, r, 0), Value::text("spanned"));
        assert_eq!(style(&wb, cell(s, r, 0)).fill, Some(0xffff_00ff));
    }
    // Row 11: an empty cell at C (Index 3) is unstyled and dropped; D is a
    // styled empty cell.
    assert!(!s.cells.iter().any(|c| c.row == 10 && c.col == 2));
    assert_eq!(value(s, 10, 3), Value::Empty);
    // Sorted row-major.
    assert!(s
        .cells
        .windows(2)
        .all(|w| (w[0].row, w[0].col) < (w[1].row, w[1].col)));
    // A note.
    assert_eq!(s.notes.len(), 1);
    assert_eq!(s.notes[0].text, "Ada: a note");
    assert_eq!(s.notes[0].author.as_deref(), Some("Ada"));
    assert!(s.notes[0].shown);
}

#[test]
fn styles() {
    let wb = basic();
    let s = &wb.sheets[0];
    assert_eq!(wb.settings.default_font.as_deref(), Some("Arial"));
    let b = style(&wb, cell(s, 4, 0));
    assert!(b.font.bold && b.font.italic && b.font.underline && b.font.strike);
    assert_eq!(b.font.underline_style, UnderlineStyleData::Single);
    assert_eq!(b.font.size_pt, Some(14.0));
    assert_eq!(b.font.family, None, "Arial is the default font");
    assert_eq!(b.font.color, Some(0xff00_00ff));
    // Parent inheritance: sChild = sFill + a percent format.
    let c = style(&wb, cell(s, 4, 3));
    assert_eq!(c.number_format.as_deref(), Some("0.00%"));
    assert_eq!(c.fill, Some(0xffff_00ff));
    assert_eq!(c.align.h, HAlign::Center);
    assert_eq!(c.align.v, VAlign::Top);
    assert!(c.align.wrap);
    assert_eq!(c.align.indent, 2);
    assert_eq!(c.align.rotation_deg, 45);
    let bottom = c.borders.bottom.as_ref().unwrap();
    assert_eq!(
        (bottom.style, bottom.width_hmm, bottom.color),
        (BorderStyle::Solid, 88, 0xff)
    );
    let left = c.borders.left.as_ref().unwrap();
    assert_eq!((left.style, left.color), (BorderStyle::Double, 0x0000_ffff));
    assert_eq!(c.borders.top.as_ref().unwrap().style, BorderStyle::Dashed);
    assert!(c.borders.right.is_none());
    assert!(!c.protection.unwrap().locked);
    assert_eq!(
        style(&wb, cell(s, 4, 4)).number_format.as_deref(),
        Some("#,##0.00")
    );
    // Column style: a run, and the cell in that column.
    assert!(s
        .style_runs
        .iter()
        .any(|r| r.col == 5 && r.start_row == 0 && r.end_row == MAX_ROWS - 1));
    assert_eq!(
        style(&wb, cell(s, 0, 5)).font.family.as_deref(),
        Some("Courier New")
    );
    // Deduplicated.
    let mut dbg: Vec<String> = wb.styles.iter().map(|s| format!("{s:?}")).collect();
    dbg.sort();
    dbg.dedup();
    assert_eq!(dbg.len(), wb.styles.len());
}

#[test]
fn dimensions_freeze_filter_and_names() {
    let wb = basic();
    let s = &wb.sheets[0];
    let w = |c: u32| {
        s.col_widths
            .iter()
            .find(|r| r.start <= c && c <= r.end)
            .cloned()
    };
    assert_eq!(w(0).unwrap().size_hmm, 2822);
    let hidden = w(2).unwrap();
    assert!(hidden.hidden && hidden.end == 3 && hidden.size_hmm == 1058);
    assert!(w(1).is_none());
    assert_eq!(w(5).unwrap().size_hmm, 2117);
    let row0 = s.row_heights.iter().find(|r| r.start == 0).unwrap();
    assert_eq!((row0.size_hmm, row0.hidden), (706, false));
    assert!(s.row_heights.iter().any(|r| r.start == 4 && r.hidden));
    assert_eq!(s.frozen, Some((1, 2)));
    assert_eq!(
        s.auto_filter.as_ref().unwrap().range,
        CellRangeLocal {
            r0: 0,
            c0: 0,
            r1: 4,
            c1: 1
        }
    );
    assert!(wb
        .warnings
        .iter()
        .any(|w| w.contains("Conditional formatting")));
    let p = s.page_setup.as_ref().unwrap();
    assert_eq!(
        p.print_range,
        Some(CellRangeLocal {
            r0: 0,
            c0: 0,
            r1: 9,
            c1: 3
        })
    );
    assert_eq!(p.repeat_rows, Some((0, 1)));
    let name = |n: &str| wb.names.iter().find(|x| x.name == n).cloned();
    let rates = name("Rates").unwrap();
    assert_eq!(rates.formula, "of:=[$Data.$A$1:.$B$5]");
    assert_eq!(rates.sheet, None);
    assert_eq!(name("TaxRate").unwrap().formula, "of:=0.2");
    let local = name("Local").unwrap();
    assert_eq!(
        (local.sheet, local.formula.as_str()),
        (Some(0), "of:=[$Data.$B$2]")
    );
    assert!(name("_FilterDatabase").is_none() && name("Print_Area").is_none());
    let _ = MAX_COLS;
}

#[test]
fn document_properties() {
    let m = basic().meta;
    assert_eq!(m.title.as_deref(), Some("Quarterly figures"));
    assert_eq!(m.subject.as_deref(), Some("Test"));
    assert_eq!(m.author.as_deref(), Some("Ada"));
    assert_eq!(m.last_author.as_deref(), Some("Grace"));
    assert_eq!(m.created.as_deref(), Some("2003-05-01T10:00:00Z"));
    assert_eq!(m.modified.as_deref(), Some("2003-05-02T11:00:00Z"));
    assert_eq!(m.keywords, ["alpha", "beta"]);
    assert_eq!(m.description.as_deref(), Some("Hand-written fixture"));
}

#[test]
fn prefixed_and_1904() {
    let wb = read_xmlss(&fixture("prefixed.xml"), &Limits::default()).unwrap();
    assert_eq!(wb.sheets.len(), 1);
    assert_eq!(wb.sheets[0].name, "Sheet1");
    let s = &wb.sheets[0];
    assert_eq!(
        wb.settings.calc_settings.date_epoch,
        tally_types::DateEpoch::D1904
    );
    // 1904-01-02 is serial 1 in the 1904 system.
    assert_eq!(value(s, 0, 0), Value::Number(1.0));
    assert_eq!(value(s, 0, 2), Value::text("x"));
    // Hidden Default font size/family apply to unstyled cells only via the
    // document font.
    assert_eq!(wb.settings.default_font.as_deref(), Some("Calibri"));
    // The time-only value gets a time format.
    assert_eq!(
        style(&wb, cell(s, 1, 0)).number_format.as_deref(),
        Some("HH:MM:SS")
    );
}
