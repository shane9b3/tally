//! Reader tests over the generated fixtures in `tests/fixtures/xlsx/`
//! (`gen_fixtures.py`): Excel 365 shapes, LibreOffice shapes, the 1904 date
//! system, namespace-prefixed XML and a 100k-row sparse sheet.

#![allow(clippy::unwrap_used)] // tests may unwrap (docs/contributing.md)

mod common;

use common::*;
use tally_io_xlsx::{sniff_is_xlsx, DEFAULT_COL_WIDTH_HMM};
use tally_model::{
    BorderStyle, CellContentData, CellRangeLocal, DimRun, HAlign, NameData, StyleRun, VAlign,
};
use tally_types::{DateEpoch, ErrorKind, Value, MAX_ROWS};

#[test]
fn excel365_sheets_and_settings() {
    let wb = open("excel365-basic.xlsx");
    let names: Vec<&str> = wb.sheets.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Data", "My Sheet", "Hidden"]);
    assert!(!wb.sheets[0].hidden && !wb.sheets[1].hidden && wb.sheets[2].hidden);
    assert_eq!(wb.settings.active_sheet, 0);
    assert_eq!(wb.settings.calc_settings.date_epoch, DateEpoch::D1899);
    let data = &wb.sheets[0];
    assert_eq!(data.frozen, Some((1, 1)));
    assert_eq!(data.tab_color, Some(0x00b050ff));
    assert_eq!(
        data.merges,
        [CellRangeLocal {
            r0: 9,
            c0: 0,
            r1: 9,
            c1: 2
        }]
    );
    // B: 20.7109375 characters = 145 px; D: hidden (width 0 → default).
    assert_eq!(
        data.col_widths,
        [
            DimRun {
                start: 1,
                end: 1,
                size_hmm: 3836,
                hidden: false
            },
            DimRun {
                start: 3,
                end: 3,
                size_hmm: DEFAULT_COL_WIDTH_HMM,
                hidden: true
            }
        ]
    );
    // Row 8 hidden (default 14.5 pt), row 9 30 pt.
    assert_eq!(
        data.row_heights,
        [
            DimRun {
                start: 7,
                end: 7,
                size_hmm: 512,
                hidden: true
            },
            DimRun {
                start: 8,
                end: 8,
                size_hmm: 1058,
                hidden: false
            }
        ]
    );
    // Columns G:H carry a column style (bold).
    let bold = style(&wb, 0, "A1").unwrap();
    assert!(bold.font.bold);
    let runs: Vec<(u32, u32, u32)> = data
        .style_runs
        .iter()
        .map(|r: &StyleRun| (r.col, r.start_row, r.end_row))
        .collect();
    assert_eq!(runs, [(6, 0, MAX_ROWS - 1), (7, 0, MAX_ROWS - 1)]);
    assert_eq!(wb.styles[data.style_runs[0].style as usize], bold);
}

#[test]
fn excel365_values() {
    let wb = open("excel365-basic.xlsx");
    assert_eq!(value(&wb, 0, "A1"), text("Item"));
    assert_eq!(value(&wb, 0, "F1"), text("Inline"));
    assert_eq!(value(&wb, 0, "B2"), num(3.0));
    // Rich text runs concatenate; phonetic guides are dropped.
    assert_eq!(value(&wb, 0, "A4"), text("Rich text"));
    assert_eq!(value(&wb, 0, "A5"), text("東京"));
    assert_eq!(value(&wb, 0, "F5"), text("  padded  "));
    assert_eq!(value(&wb, 0, "A6"), text("a&b <c>"));
    assert_eq!(value(&wb, 0, "F3"), Value::Bool(true));
    assert_eq!(value(&wb, 0, "F4"), Value::Bool(false));
    assert_eq!(value(&wb, 0, "E4"), Value::Error(ErrorKind::NA));
    assert_eq!(value(&wb, 0, "A12"), num(12345.678));
    assert_eq!(value(&wb, 0, "B12"), num(1e20));
    assert_eq!(value(&wb, 0, "C12"), num(-0.000001));
    // Styled empty cells are kept (merge area), unstyled empties are not.
    assert_eq!(value(&wb, 0, "B10"), Value::Empty);
    assert!(cell(&wb, 0, "A11").is_none());
    assert_eq!(value(&wb, 2, "A1"), num(42.0));
}

#[test]
fn excel365_formulas() {
    let wb = open("excel365-basic.xlsx");
    // A shared formula is re-anchored for every member.
    for (r, cached) in [(2, 4.5), (3, 9.0), (4, 2.5), (5, 6.0), (6, 14.0)] {
        assert_eq!(
            formula(&wb, 0, &format!("D{r}")),
            (format!("of:=[.B{r}]*[.C{r}]"), Some(num(cached))),
        );
    }
    assert_eq!(
        formula(&wb, 0, "E5"),
        ("of:=1/0".into(), Some(Value::Error(ErrorKind::Div0)))
    );
    assert_eq!(
        formula(&wb, 0, "E6"),
        ("of:=[.A2]&\"x\"".into(), Some(text("Applex")))
    );
    assert_eq!(formula(&wb, 0, "B7").0, "of:=SUM([.B2:.B6])");
    // Registry spellings on the OpenFormula side.
    assert_eq!(
        formula(&wb, 0, "D7"),
        (
            "of:=COM.MICROSOFT.IFERROR(1/0;\"x\")".into(),
            Some(text("x"))
        )
    );
    assert_eq!(formula(&wb, 0, "E7").0, "of:=['My Sheet'.A1]*2");
    // Unknown future functions keep their prefix so they round-trip.
    assert_eq!(formula(&wb, 0, "F7").0, "of:=_xlfn.NOSUCHFN(1)");
    // 3D reference, intersection, array literals, defined name.
    assert_eq!(formula(&wb, 0, "B8").0, "of:=SUM([Data.A1:Hidden.A1])");
    assert_eq!(formula(&wb, 0, "C8").0, "of:=SUM([.B2:.B3]![.B3:.B4])");
    assert_eq!(formula(&wb, 0, "D8").0, "of:=SUM({1;2|3;4}*{1;0|0;0})+2");
    assert_eq!(
        formula(&wb, 0, "E8"),
        ("of:=TaxRate".into(), Some(num(0.15)))
    );
}

#[test]
fn excel365_array_formula() {
    let wb = open("excel365-basic.xlsx");
    assert_eq!(
        wb.sheets[0].array_formulas,
        [CellRangeLocal {
            r0: 6,
            c0: 0,
            r1: 7,
            c1: 0
        }]
    );
    assert_eq!(
        formula(&wb, 0, "A7"),
        ("of:=[.B2:.B3]*2".into(), Some(num(6.0)))
    );
    // The other cell of the block holds its cached value.
    assert_eq!(value(&wb, 0, "A8"), num(8.0));
}

#[test]
fn excel365_styles() {
    let wb = open("excel365-basic.xlsx");
    let s = style(&wb, 0, "C2").unwrap();
    assert!(s.font.italic && s.font.underline && !s.font.bold);
    assert_eq!(s.font.family.as_deref(), Some("Arial"));
    assert_eq!(s.font.size_pt, Some(14.0));
    assert_eq!(s.font.color, Some(0xff0000ff));
    assert_eq!(s.fill, Some(0xffff00ff));
    assert_eq!(s.number_format.as_deref(), Some("\"$\"#,##0.00"));
    let top = s.borders.top.unwrap();
    assert_eq!(
        (top.style, top.width_hmm, top.color),
        (BorderStyle::Solid, 53, 0x0070c0ff)
    );
    assert_eq!(s.borders.bottom.unwrap().style, BorderStyle::Double);
    assert_eq!(s.borders.left.unwrap().width_hmm, 26);
    // Theme fill with tint; centred, wrapped.
    let s = style(&wb, 0, "F1").unwrap();
    assert_eq!(s.fill, Some(0xdae3f3ff));
    assert_eq!(
        (s.align.h, s.align.v, s.align.wrap),
        (HAlign::Center, VAlign::Middle, true)
    );
    // Theme font colour with a negative tint, strike, built-in time format.
    let s = style(&wb, 0, "B9").unwrap();
    assert!(s.font.strike);
    assert_eq!(s.font.color, Some(0x2f5597ff));
    assert_eq!(s.number_format.as_deref(), Some("H:MM:SS"));
    let s = style(&wb, 0, "C9").unwrap();
    assert_eq!((s.align.h, s.align.indent), (HAlign::Right, 2));
    assert_eq!(
        style(&wb, 0, "E3").unwrap().number_format.as_deref(),
        Some("M/D/YYYY")
    );
    assert_eq!(
        style(&wb, 0, "F2").unwrap().number_format.as_deref(),
        Some("0.00%")
    );
    // The default xf maps to no style.
    assert!(cell(&wb, 0, "B2").unwrap().style.is_none());
}

#[test]
fn excel365_dates_1900_system() {
    let wb = open("excel365-basic.xlsx");
    // Excel 1 (1900-01-01), 59 (02-28), 60 (fictitious 02-29), 61 (03-01).
    assert_eq!(value(&wb, 1, "B1"), num(2.0));
    assert_eq!(value(&wb, 1, "C1"), num(60.0));
    assert_eq!(value(&wb, 1, "D1"), num(60.0));
    assert_eq!(value(&wb, 1, "E1"), num(61.0));
    assert_eq!(value(&wb, 1, "F1"), num(46314.0));
    assert_eq!(value(&wb, 1, "C2"), num(46314.75));
    // Not dates: a plain number, a time of day, a duration.
    assert_eq!(value(&wb, 1, "A1"), num(3.0));
    assert_eq!(value(&wb, 1, "A2"), num(0.5));
    assert_eq!(value(&wb, 1, "B2"), num(1.5));
    // Strict OOXML `t="d"`.
    assert_eq!(value(&wb, 1, "D2"), num(46314.5));
    // A date-formatted cell before March 1900 in the main sheet.
    assert_eq!(value(&wb, 0, "E2"), num(46314.0));
}

#[test]
fn excel365_defined_names() {
    let wb = open("excel365-basic.xlsx");
    let n = |name: &str| wb.names.iter().find(|n| n.name == name).cloned();
    assert_eq!(
        n("Print_Area"),
        Some(NameData {
            name: "Print_Area".into(),
            sheet: Some(0),
            formula: "of:=[$Data.$A$1:.$F$12]".into()
        })
    );
    assert_eq!(n("Totals").unwrap().formula, "of:=[$Data.$D$2:.$D$6]");
    assert_eq!(n("TaxRate").unwrap().formula, "of:=0.15");
    assert_eq!(n("Broken").unwrap().formula, "of:=[.#REF!]");
    let items = n("Items").unwrap();
    assert_eq!(items.sheet, Some(1));
    assert_eq!(items.formula, "of:=[$'My Sheet'.$A$1:.$F$1]");
    // Other `_xlnm.*` built-ins are not names.
    assert!(wb.names.iter().all(|n| !n.name.starts_with("_xlnm")));
    assert_eq!(wb.names.len(), 5);
}

#[test]
fn excel_1904_system() {
    let wb = open("excel-1904.xlsx");
    assert_eq!(wb.settings.calc_settings.date_epoch, DateEpoch::D1904);
    // Serials are kept; the epoch moves instead.
    assert_eq!(value(&wb, 0, "A1"), num(0.0));
    assert_eq!(value(&wb, 0, "B1"), num(44852.0));
    assert_eq!(value(&wb, 0, "C1"), num(44852.5));
    assert_eq!(value(&wb, 0, "E1"), num(1.0));
    assert_eq!(value(&wb, 0, "D1"), num(100.0));
    let ymd = tally_numfmt::date::ymd_from_serial(DateEpoch::D1904, 44852.0);
    assert_eq!(ymd, Some((2026, 10, 19)));
}

#[test]
fn libreoffice_shapes() {
    let wb = open("libreoffice-style.xlsx");
    assert_eq!(wb.sheets.len(), 2);
    assert_eq!(wb.settings.active_sheet, 1);
    let s = &wb.sheets[0];
    assert_eq!(s.frozen, Some((1, 0)));
    assert_eq!(s.col_widths.len(), 1);
    assert_eq!(s.col_widths[0].start, 0);
    // Only customHeight / hidden rows are recorded (12.8 pt auto heights
    // are not); row 3 is 24.05 pt, row 4 hidden.
    let rows: Vec<(u32, bool)> = s.row_heights.iter().map(|r| (r.start, r.hidden)).collect();
    assert_eq!(rows, [(2, false), (3, true)]);
    assert_eq!(s.row_heights[0].size_hmm, 848);
    assert!(wb.sheets[1].row_heights.is_empty());
    assert_eq!(value(&wb, 0, "C1"), text("Total"));
    assert_eq!(value(&wb, 0, "A3"), Value::Bool(true));
    assert_eq!(
        formula(&wb, 0, "C2"),
        ("of:=[.B2]*100".into(), Some(num(12.5)))
    );
    assert_eq!(
        formula(&wb, 0, "B3"),
        ("of:=#N/A".into(), Some(Value::Error(ErrorKind::NA)))
    );
    assert_eq!(
        formula(&wb, 0, "A4"),
        ("of:=SUM([.C2:.C2])+[Second.A1]".into(), Some(num(13.5)))
    );
    let h = style(&wb, 0, "A1").unwrap();
    assert!(h.font.bold);
    assert_eq!(h.font.family.as_deref(), Some("Liberation Sans"));
    assert_eq!(h.fill, Some(0xdee6efff));
    assert_eq!(h.align.h, HAlign::Center);
    // `numFmt 164 = General` is no number format.
    assert!(cell(&wb, 0, "A2").unwrap().style.is_none());
    assert_eq!(
        style(&wb, 0, "B2").unwrap().number_format.as_deref(),
        Some("0.00%")
    );
    assert_eq!(s.merges.len(), 1);
    assert_eq!(wb.names.len(), 2);
    assert_eq!(wb.names[1].formula, "of:=[$Sheet1.$B$2]");
}

#[test]
fn prefixed_namespaces_and_implied_positions() {
    let wb = open("prefixed-ns.xlsx");
    assert_eq!(wb.sheets[0].name, "Prefixed");
    assert_eq!(value(&wb, 0, "A1"), text("a"));
    assert_eq!(value(&wb, 0, "B1"), num(2.0));
    assert!(cell(&wb, 0, "A2").is_none());
    assert_eq!(
        formula(&wb, 0, "B2"),
        ("of:=[.B1]*3".into(), Some(num(6.0)))
    );
}

#[test]
fn sparse_100k_rows() {
    let t = std::time::Instant::now();
    let wb = open("sparse-100k.xlsx");
    eprintln!("TIMING sparse-100k.xlsx: {:?}", t.elapsed());
    let s = &wb.sheets[0];
    assert_eq!(s.cells.len(), 1000 + 100 + 1);
    assert_eq!(value(&wb, 0, "A99901"), num(99901.0));
    assert_eq!(formula(&wb, 0, "C99001").0, "of:=[.A99001]*2");
    assert_eq!(value(&wb, 0, "XFD100000"), num(7.0));
    // Sorted row-major.
    assert!(s
        .cells
        .windows(2)
        .all(|w| (w[0].row, w[0].col) < (w[1].row, w[1].col)));
}

#[test]
fn sniffing() {
    for f in [
        "excel365-basic.xlsx",
        "libreoffice-style.xlsx",
        "excel-1904.xlsx",
        "prefixed-ns.xlsx",
    ] {
        assert!(sniff_is_xlsx(&read_fixture(f)), "{f}");
    }
    for f in [
        "malicious/not-a-zip.xlsx",
        "malicious/ods-renamed.xlsx",
        "malicious/eocd-20k-entries.xlsx",
    ] {
        assert!(!sniff_is_xlsx(&read_fixture(f)), "{f}");
    }
    assert!(!sniff_is_xlsx(b""));
    assert!(!sniff_is_xlsx(b"PK\x03\x04garbage"));
}

#[test]
fn formula_cells_never_lose_their_cached_value() {
    let wb = open("excel365-basic.xlsx");
    for sheet in &wb.sheets {
        for c in &sheet.cells {
            if let CellContentData::Formula { text, cached } = &c.content {
                assert!(text.starts_with("of:="), "{text}");
                assert!(cached.is_some(), "{text}");
            }
        }
    }
}
