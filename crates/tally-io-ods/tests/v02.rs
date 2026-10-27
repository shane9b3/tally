//! v0.2 features: merges, array formulas, named expressions, AutoFilter
//! (database ranges and filtered rows), borders and frozen panes.

#![allow(clippy::unwrap_used)] // tests may unwrap (docs/contributing.md)

use std::io::{Cursor, Read, Write};
use std::sync::Arc;

use tally_io_ods::{read_ods, write_ods_with_report, Limits};
use tally_model::*;
use tally_types::Value;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

fn v(row: u32, col: u32, value: Value) -> CellData {
    CellData {
        row,
        col,
        content: CellContentData::Value(value),
        style: None,
    }
}

fn f(row: u32, col: u32, text: &str, cached: Value) -> CellData {
    CellData {
        row,
        col,
        content: CellContentData::Formula {
            text: Arc::from(text),
            cached: Some(cached),
        },
        style: None,
    }
}

fn rect(r0: u32, c0: u32, r1: u32, c1: u32) -> CellRangeLocal {
    CellRangeLocal { r0, c0, r1, c1 }
}

fn one_sheet(sheet: SheetData) -> WorkbookData {
    WorkbookData {
        sheets: vec![sheet],
        ..WorkbookData::default()
    }
}

fn write(wb: &WorkbookData) -> Vec<u8> {
    let mut buf = Cursor::new(Vec::new());
    let report = write_ods_with_report(wb, &mut buf).unwrap();
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    buf.into_inner()
}

fn round_trip(wb: &WorkbookData) -> WorkbookData {
    read_ods(Cursor::new(write(wb)), &Limits::default()).unwrap()
}

fn content_xml(bytes: &[u8]) -> String {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut s = String::new();
    zip.by_name("content.xml")
        .unwrap()
        .read_to_string(&mut s)
        .unwrap();
    s
}

const HEAD: &str = r#"<?xml version="1.0" encoding="UTF-8"?><office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:version="1.2">"#;

fn ods(automatic_styles: &str, body: &str) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    zip.start_file("mimetype", stored).unwrap();
    zip.write_all(tally_io_ods::MIMETYPE.as_bytes()).unwrap();
    zip.start_file("content.xml", stored).unwrap();
    write!(
        zip,
        "{HEAD}<office:automatic-styles>{automatic_styles}</office:automatic-styles>\
         <office:body><office:spreadsheet>{body}</office:spreadsheet></office:body>\
         </office:document-content>"
    )
    .unwrap();
    zip.finish().unwrap().into_inner()
}

/// Counts cell positions per row in `xml` (cells and covered cells, with
/// repeats), checking the table is rectangular-consistent: every row of the
/// written table must place covered cells exactly at the covered positions.
fn covered_positions(xml: &str) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    let mut row: u32 = 0;
    for row_xml in xml.split("<table:table-row").skip(1) {
        let rep: u32 = attr(
            row_xml.split('>').next().unwrap(),
            "table:number-rows-repeated",
        )
        .map_or(1, |x| x.parse().unwrap());
        let body = row_xml.split("</table:table-row>").next().unwrap();
        let mut col = 0;
        let mut positions = Vec::new();
        for (i, part) in body.split('<').enumerate() {
            if i == 0 {
                continue;
            }
            let is_cell = part.starts_with("table:table-cell");
            let is_cov = part.starts_with("table:covered-table-cell");
            if !is_cell && !is_cov {
                continue;
            }
            let tag = part.split('>').next().unwrap();
            let n: u32 =
                attr(tag, "table:number-columns-repeated").map_or(1, |x| x.parse().unwrap());
            if is_cov {
                for k in 0..n {
                    positions.push(col + k);
                }
            }
            col += n;
        }
        for r in 0..rep {
            for c in &positions {
                out.push((row + r, *c));
            }
        }
        row += rep;
    }
    out
}

fn attr<'a>(tag: &'a str, key: &str) -> Option<&'a str> {
    let i = tag.find(&format!("{key}=\""))?;
    let rest = &tag[i + key.len() + 2..];
    rest.split('"').next()
}

#[test]
fn merges_round_trip_with_covered_cells() {
    let sheet = SheetData {
        name: "S".into(),
        cells: vec![
            v(0, 0, Value::text("Title")),
            v(0, 5, Value::Number(1.0)),
            v(3, 2, Value::text("x")),
            v(10, 0, Value::Number(2.0)),
        ],
        // A 1×3 merge with content, an empty 3×2 merge in an empty area, a
        // tall merge over repeated empty rows, and one past every cell.
        merges: vec![
            rect(0, 0, 0, 2),
            rect(4, 1, 6, 2),
            rect(3, 2, 8, 2),
            rect(20, 7, 21, 9),
        ],
        style_runs: vec![StyleRun {
            col: 1,
            start_row: 4,
            end_row: 20_000,
            style: 0,
        }],
        ..SheetData::default()
    };
    let mut wb = one_sheet(sheet);
    wb.styles.push(StyleData {
        fill: Some(0xffee_ddff),
        ..StyleData::default()
    });
    // The rect(3,2,8,2) overlaps rect(4,1,6,2) at column 2: drop it, merges
    // never overlap in practice.
    wb.sheets[0].merges.remove(2);
    wb.sheets[0].merges.push(rect(8, 4, 15, 4));
    let bytes = write(&wb);
    let xml = content_xml(&bytes);
    assert!(xml.contains(r#"table:number-columns-spanned="3" table:number-rows-spanned="1""#));
    let mut expect = Vec::new();
    for m in &wb.sheets[0].merges {
        for r in m.r0..=m.r1 {
            for c in m.c0..=m.c1 {
                if (r, c) != (m.r0, m.c0) {
                    expect.push((r, c));
                }
            }
        }
    }
    expect.sort_unstable();
    let mut got = covered_positions(&xml);
    got.sort_unstable();
    assert_eq!(got, expect);
    let back = read_ods(Cursor::new(bytes), &Limits::default()).unwrap();
    let mut a = back.sheets[0].merges.clone();
    let mut b = wb.sheets[0].merges.clone();
    a.sort_by_key(|m| (m.r0, m.c0));
    b.sort_by_key(|m| (m.r0, m.c0));
    assert_eq!(a, b);
    // Styled empty covered cells come back as styled empty cells.
    let filled = back.sheets[0]
        .cells
        .iter()
        .filter(|c| c.content != CellContentData::Value(Value::Empty))
        .count();
    assert_eq!(filled, 4);
}

#[test]
fn covered_cell_content_is_dropped_on_read() {
    let body = r#"<table:table table:name="S"><table:table-row>
        <table:table-cell table:number-columns-spanned="2" office:value-type="float" office:value="1"><text:p>1</text:p></table:table-cell>
        <table:covered-table-cell office:value-type="float" office:value="9"><text:p>9</text:p></table:covered-table-cell>
        <table:table-cell office:value-type="float" office:value="3"><text:p>3</text:p></table:table-cell>
        </table:table-row></table:table>"#;
    let wb = read_ods(Cursor::new(ods("", body)), &Limits::default()).unwrap();
    let s = &wb.sheets[0];
    assert_eq!(s.merges, vec![rect(0, 0, 0, 1)]);
    let cells: Vec<(u32, u32)> = s.cells.iter().map(|c| (c.row, c.col)).collect();
    assert_eq!(cells, vec![(0, 0), (0, 2)]);
}

#[test]
fn array_formulas_round_trip() {
    let sheet = SheetData {
        name: "S".into(),
        cells: vec![
            f(1, 1, "of:=[.A1:.A2]*2", Value::Number(2.0)),
            v(1, 2, Value::Number(2.0)),
            v(2, 1, Value::Number(4.0)),
            v(2, 2, Value::Number(4.0)),
        ],
        array_formulas: vec![rect(1, 1, 2, 2)],
        ..SheetData::default()
    };
    let wb = one_sheet(sheet);
    let bytes = write(&wb);
    let xml = content_xml(&bytes);
    assert!(xml.contains(
        r#"table:number-matrix-columns-spanned="2" table:number-matrix-rows-spanned="2""#
    ));
    let back = read_ods(Cursor::new(bytes), &Limits::default()).unwrap();
    assert_eq!(back.sheets[0].array_formulas, vec![rect(1, 1, 2, 2)]);
    assert_eq!(back.sheets[0].cells, wb.sheets[0].cells);
}

#[test]
fn named_expressions_round_trip() {
    let mut wb = WorkbookData {
        sheets: vec![
            SheetData {
                name: "Sheet1".into(),
                ..SheetData::default()
            },
            SheetData {
                name: "My Sheet".into(),
                ..SheetData::default()
            },
        ],
        ..WorkbookData::default()
    };
    wb.names = vec![
        NameData {
            base_cell: None,
            name: "Total".into(),
            sheet: None,
            formula: "of:=[$Sheet1.$A$1:.$B$5]".into(),
        },
        NameData {
            base_cell: None,
            name: "Rate".into(),
            sheet: None,
            formula: "of:=0.07*2".into(),
        },
        NameData {
            base_cell: None,
            name: "Local".into(),
            sheet: Some(1),
            formula: "of:=[$'My Sheet'.$C$3]".into(),
        },
    ];
    let bytes = write(&wb);
    let xml = content_xml(&bytes);
    assert!(xml.contains(r#"<table:named-range table:name="Total" table:base-cell-address="$Sheet1.$A$1" table:cell-range-address="$Sheet1.$A$1:.$B$5"/>"#), "{xml}");
    assert!(xml.contains(r#"table:expression="of:=0.07*2""#));
    let back = read_ods(Cursor::new(bytes), &Limits::default()).unwrap();
    let mut a = back.names.clone();
    let mut b = wb.names.clone();
    a.sort_by(|x, y| x.name.cmp(&y.name));
    b.sort_by(|x, y| x.name.cmp(&y.name));
    assert_eq!(a, b);
}

#[test]
fn named_ranges_from_other_producers_are_normalized() {
    let body = r#"<table:table table:name="S"><table:table-row><table:table-cell/></table:table-row>
        <table:named-expressions><table:named-range table:name="Loc" table:base-cell-address="$S.$B$2" table:cell-range-address="$S.$A$1"/></table:named-expressions>
        </table:table>
        <table:named-expressions>
          <table:named-range table:name="Both" table:base-cell-address="$S.$A$1" table:cell-range-address="$S.$A$1:$S.$B$2"/>
          <table:named-range table:name="Spaced" table:base-cell-address="$S.$A$1" table:cell-range-address="'My Sheet'.A1:'My Sheet'.B2"/>
          <table:named-range table:name="Two" table:base-cell-address="$S.$A$1" table:cell-range-address="$S.$A$1 $S.$C$3"/>
          <table:named-expression table:name="Bare" table:base-cell-address="$S.$A$1" table:expression="=1+2"/>
        </table:named-expressions>"#;
    let wb = read_ods(Cursor::new(ods("", body)), &Limits::default()).unwrap();
    let get = |n: &str| wb.names.iter().find(|x| x.name == n).unwrap().clone();
    assert_eq!(get("Loc").sheet, Some(0));
    assert_eq!(get("Loc").formula, "of:=[$S.$A$1]");
    assert_eq!(get("Both").formula, "of:=[$S.$A$1:.$B$2]");
    assert_eq!(get("Both").sheet, None);
    assert_eq!(get("Spaced").formula, "of:=['My Sheet'.A1:.B2]");
    assert_eq!(get("Two").formula, "of:=[$S.$A$1]~[$S.$C$3]");
    assert_eq!(get("Bare").formula, "of:=1+2");
}

#[test]
fn autofilter_round_trip() {
    let mut cells = vec![v(0, 0, Value::text("Name")), v(0, 1, Value::text("Qty"))];
    for r in 1..=8 {
        cells.push(v(r, 0, Value::text(if r % 2 == 0 { "a" } else { "b" })));
        cells.push(v(r, 1, Value::Number(f64::from(r))));
    }
    let sheet = SheetData {
        name: "Data Sheet".into(),
        cells,
        row_heights: vec![DimRun {
            start: 6,
            end: 6,
            size_hmm: 452,
            hidden: true,
        }],
        auto_filter: Some(AutoFilterData {
            range: rect(0, 0, 8, 1),
            columns: vec![
                FilterColumnData {
                    col: 0,
                    values: Some(vec!["a".into(), String::new()]),
                    condition: None,
                },
                FilterColumnData {
                    col: 1,
                    values: None,
                    condition: Some(FilterConditionData {
                        op: FilterOpData::Ge,
                        value: "3".into(),
                    }),
                },
            ],
            filtered_rows: vec![(1, 3), (5, 5), (7, 7)],
        }),
        ..SheetData::default()
    };
    let wb = one_sheet(sheet);
    let bytes = write(&wb);
    let xml = content_xml(&bytes);
    assert!(
        xml.contains(r#"table:target-range-address="'Data Sheet'.A1:'Data Sheet'.B9""#),
        "{xml}"
    );
    assert!(xml.contains(r#"table:display-filter-buttons="true""#));
    assert!(xml.contains(r#"table:operator="&gt;=""#));
    assert!(xml.contains(r#"table:visibility="filter""#));
    let back = read_ods(Cursor::new(bytes), &Limits::default()).unwrap();
    assert_eq!(back.sheets[0].auto_filter, wb.sheets[0].auto_filter);
    assert_eq!(back.sheets[0].row_heights, wb.sheets[0].row_heights);
    for op in [
        FilterOpData::Eq,
        FilterOpData::Ne,
        FilterOpData::Gt,
        FilterOpData::Lt,
        FilterOpData::Le,
        FilterOpData::Contains,
        FilterOpData::BeginsWith,
        FilterOpData::EndsWith,
        FilterOpData::Empty,
        FilterOpData::NotEmpty,
        FilterOpData::TopValues,
        FilterOpData::BottomValues,
        FilterOpData::TopPercent,
        FilterOpData::BottomPercent,
    ] {
        let mut w = wb.clone();
        let af = w.sheets[0].auto_filter.as_mut().unwrap();
        af.columns = vec![FilterColumnData {
            col: 1,
            values: None,
            condition: Some(FilterConditionData {
                op,
                value: "x".into(),
            }),
        }];
        let back = round_trip(&w);
        assert_eq!(
            back.sheets[0].auto_filter, w.sheets[0].auto_filter,
            "{op:?}"
        );
    }
}

/// AutoFilter "Top 10" is AOO's `table:operator="top values"` with value 10.
#[test]
fn autofilter_top_values_operators_are_written_as_aoo() {
    let sheet = SheetData {
        name: "S".into(),
        auto_filter: Some(AutoFilterData {
            range: rect(0, 0, 4, 0),
            columns: vec![FilterColumnData {
                col: 0,
                values: None,
                condition: Some(FilterConditionData {
                    op: FilterOpData::TopValues,
                    value: "10".into(),
                }),
            }],
            filtered_rows: Vec::new(),
        }),
        ..SheetData::default()
    };
    let wb = one_sheet(sheet);
    let xml = content_xml(&write(&wb));
    assert!(
        xml.contains(r#"table:value="10" table:operator="top values" table:data-type="number""#),
        "{xml}"
    );
    assert_eq!(
        round_trip(&wb).sheets[0].auto_filter,
        wb.sheets[0].auto_filter
    );
}

#[test]
fn filter_rows_without_autofilter_are_hidden_and_odd_filters_are_skipped() {
    let body = r#"<table:table table:name="S">
        <table:table-row table:visibility="filter" table:number-rows-repeated="2"><table:table-cell/></table:table-row>
        <table:table-row><table:table-cell/></table:table-row>
        </table:table>
        <table:table table:name="T">
        <table:table-row><table:table-cell/></table:table-row>
        <table:table-row table:visibility="filter"><table:table-cell/></table:table-row>
        </table:table>
        <table:database-ranges>
          <table:database-range table:name="__Anonymous_Sheet_DB__1" table:target-range-address="T.A1:T.C5" table:display-filter-buttons="true">
            <table:filter><table:filter-or>
              <table:filter-condition table:field-number="0" table:value="1" table:operator="="/>
              <table:filter-condition table:field-number="1" table:value="2" table:operator="="/>
            </table:filter-or></table:filter>
          </table:database-range>
          <table:database-range table:name="x" table:target-range-address="S.A1:S.B2">
            <table:filter><table:filter-condition table:field-number="0" table:value="1" table:operator="="/></table:filter>
          </table:database-range>
        </table:database-ranges>"#;
    let wb = read_ods(Cursor::new(ods("", body)), &Limits::default()).unwrap();
    assert!(wb.sheets[0].auto_filter.is_none());
    assert_eq!(
        wb.sheets[0].row_heights,
        vec![DimRun {
            start: 0,
            end: 1,
            size_hmm: 452,
            hidden: true
        }]
    );
    let af = wb.sheets[1].auto_filter.as_ref().unwrap();
    assert_eq!(af.range, rect(0, 0, 4, 2));
    assert!(af.columns.is_empty(), "an OR over two fields is skipped");
    assert_eq!(af.filtered_rows, vec![(1, 1)]);
    assert!(wb.sheets[1].row_heights.is_empty());
}

#[test]
fn libreoffice_filter_set_items_become_values() {
    let body = r#"<table:table table:name="S"><table:table-row><table:table-cell/></table:table-row></table:table>
        <table:database-ranges>
          <table:database-range table:name="__Anonymous_Sheet_DB__0" table:target-range-address="S.B2:S.D9" table:display-filter-buttons="true">
            <table:filter><table:filter-and>
              <table:filter-condition table:field-number="1" table:value="a" table:operator="=">
                <table:filter-set-item table:value="a"/><table:filter-set-item table:value="b"/>
              </table:filter-condition>
              <table:filter-condition table:field-number="2" table:value="x.*" table:operator="match"/>
            </table:filter-and></table:filter>
          </table:database-range>
        </table:database-ranges>"#;
    let wb = read_ods(Cursor::new(ods("", body)), &Limits::default()).unwrap();
    let af = wb.sheets[0].auto_filter.as_ref().unwrap();
    assert_eq!(af.range, rect(1, 1, 8, 3));
    assert_eq!(
        af.columns,
        vec![FilterColumnData {
            col: 2,
            values: Some(vec!["a".into(), "b".into()]),
            condition: None,
        }]
    );
}

#[test]
fn borders_round_trip() {
    let line = |w: u32, style: BorderStyle, color: u32| BorderLine {
        style,
        width_hmm: w,
        color,
    };
    let all = StyleData {
        borders: BordersData {
            top: Some(line(26, BorderStyle::Solid, 0x0000_00ff)),
            bottom: Some(line(26, BorderStyle::Solid, 0x0000_00ff)),
            left: Some(line(26, BorderStyle::Solid, 0x0000_00ff)),
            right: Some(line(26, BorderStyle::Solid, 0x0000_00ff)),
            ..Default::default()
        },
        ..StyleData::default()
    };
    let mixed = StyleData {
        borders: BordersData {
            top: Some(line(88, BorderStyle::Double, 0xff00_00ff)),
            bottom: None,
            left: Some(line(2, BorderStyle::Dotted, 0x00ff_00ff)),
            right: Some(line(53, BorderStyle::Dashed, 0x1234_56ff)),
            ..Default::default()
        },
        ..StyleData::default()
    };
    let mut wb = one_sheet(SheetData {
        name: "S".into(),
        cells: vec![
            CellData {
                style: Some(0),
                ..v(0, 0, Value::Number(1.0))
            },
            CellData {
                style: Some(1),
                ..v(1, 1, Value::Number(2.0))
            },
        ],
        ..SheetData::default()
    });
    wb.styles = vec![all.clone(), mixed.clone()];
    let bytes = write(&wb);
    let xml = content_xml(&bytes);
    assert!(xml.contains(r#"fo:border="0.74pt solid #000000""#), "{xml}");
    assert!(
        xml.contains(r#"fo:border-top="2.49pt double #ff0000""#),
        "{xml}"
    );
    let back = read_ods(Cursor::new(bytes), &Limits::default()).unwrap();
    let st = |r: u32| {
        let c = back.sheets[0].cells.iter().find(|c| c.row == r).unwrap();
        back.styles[c.style.unwrap() as usize].clone()
    };
    assert_eq!(st(0).borders, all.borders);
    assert_eq!(st(1).borders, mixed.borders);
}

#[test]
fn border_inheritance_and_side_override() {
    let styles = r#"<style:style style:name="p" style:family="table-cell"><style:table-cell-properties fo:border="1pt solid #000000"/></style:style>
        <style:style style:name="c" style:family="table-cell" style:parent-style-name="p"><style:table-cell-properties fo:border-left="none" fo:border-top="0.06cm dashed #00ff00"/></style:style>"#;
    let body = r#"<table:table table:name="S"><table:table-row><table:table-cell table:style-name="c" office:value-type="float" office:value="1"/></table:table-row></table:table>"#;
    let wb = read_ods(Cursor::new(ods(styles, body)), &Limits::default()).unwrap();
    let b = &wb.styles[wb.sheets[0].cells[0].style.unwrap() as usize].borders;
    assert!(b.left.is_none());
    assert_eq!(b.top.as_ref().unwrap().style, BorderStyle::Dashed);
    assert_eq!(b.top.as_ref().unwrap().width_hmm, 60);
    assert_eq!(b.bottom.as_ref().unwrap().width_hmm, 35);
    assert_eq!(b.right.as_ref().unwrap().color, 0x0000_00ff);
}

#[test]
fn frozen_panes_round_trip() {
    for frozen in [Some((1, 0)), Some((0, 2)), Some((3, 4)), None] {
        let wb = WorkbookData {
            sheets: vec![
                SheetData {
                    name: "A".into(),
                    frozen,
                    ..SheetData::default()
                },
                SheetData {
                    name: "B b".into(),
                    frozen: Some((2, 1)),
                    ..SheetData::default()
                },
            ],
            ..WorkbookData::default()
        };
        let back = round_trip(&wb);
        assert_eq!(back.sheets[0].frozen, frozen);
        assert_eq!(back.sheets[1].frozen, Some((2, 1)));
    }
}

#[test]
fn many_names_are_capped() {
    let mut names = String::from("<table:named-expressions>");
    for i in 0..100_001 {
        names.push_str(&format!(
            r#"<table:named-expression table:name="n{i}" table:expression="of:=1"/>"#
        ));
    }
    names.push_str("</table:named-expressions>");
    let body = format!(
        r#"<table:table table:name="S"><table:table-row><table:table-cell/></table:table-row></table:table>{names}"#
    );
    let r = read_ods(Cursor::new(ods("", &body)), &Limits::default());
    assert!(matches!(
        r,
        Err(tally_io_ods::OdsError::LimitExceeded {
            what: "named expressions",
            ..
        })
    ));
}
