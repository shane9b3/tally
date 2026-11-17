//! Writer → reader round trips at the `WorkbookData` level, the date
//! mapping across all three epochs, and fixture → writer → reader.

#![allow(clippy::unwrap_used)] // tests may unwrap (docs/contributing.md)

mod common;

use std::sync::Arc;

use common::*;
use tally_io_xlsx::{read_xlsx, write_xlsx, Limits};
use tally_model::{
    AlignData, BorderLine, BorderStyle, BordersData, CellContentData, CellData, CellRangeLocal,
    DimRun, FontData, HAlign, NameData, SheetData, StyleData, StyleRun, VAlign, WorkbookData,
};
use tally_numfmt::date::{serial_from_ymd, ymd_from_serial};
use tally_types::{DateEpoch, ErrorKind, Value, MAX_ROWS};

fn v(row: u32, col: u32, value: Value, style: Option<u32>) -> CellData {
    CellData {
        row,
        col,
        content: CellContentData::Value(value),
        style,
    }
}

fn f(row: u32, col: u32, text: &str, cached: Option<Value>) -> CellData {
    CellData {
        row,
        col,
        content: CellContentData::Formula {
            text: Arc::from(text),
            cached,
        },
        style: None,
    }
}

fn styles() -> Vec<StyleData> {
    vec![
        // 0: rich font, fill, borders, alignment, currency.
        StyleData {
            font: FontData {
                bold: true,
                italic: true,
                underline: true,
                strike: true,
                family: Some("Arial".into()),
                size_pt: Some(14.0),
                color: Some(0x1122_33ff),
                ..Default::default()
            },
            fill: Some(0xffee_ddff),
            align: AlignData {
                h: HAlign::Right,
                v: VAlign::Middle,
                wrap: true,
                indent: 1,
                ..Default::default()
            },
            borders: BordersData {
                top: Some(BorderLine {
                    style: BorderStyle::Solid,
                    width_hmm: 26,
                    color: 0x0000_00ff,
                }),
                bottom: Some(BorderLine {
                    style: BorderStyle::Double,
                    width_hmm: 79,
                    color: 0xff00_00ff,
                }),
                left: Some(BorderLine {
                    style: BorderStyle::Dashed,
                    width_hmm: 26,
                    color: 0x00ff_00ff,
                }),
                right: Some(BorderLine {
                    style: BorderStyle::Solid,
                    width_hmm: 53,
                    color: 0x0000_ffff,
                }),
                ..Default::default()
            },
            number_format: Some("#,##0.00".into()),
            display: None,
            protection: None,
            ..Default::default()
        },
        // 1: date.
        StyleData {
            number_format: Some("YYYY-MM-DD".into()),
            ..StyleData::default()
        },
        // 2: percent, centred.
        StyleData {
            number_format: Some("0.00%".into()),
            align: AlignData {
                h: HAlign::Center,
                ..AlignData::default()
            },
            ..StyleData::default()
        },
        // 3: duration (not a date).
        StyleData {
            number_format: Some("[HH]:MM:SS".into()),
            ..StyleData::default()
        },
        // 4: fill only (column style).
        StyleData {
            fill: Some(0xdde6_efff),
            ..StyleData::default()
        },
        // 5: datetime.
        StyleData {
            number_format: Some("M/D/YYYY H:MM".into()),
            align: AlignData {
                v: VAlign::Top,
                ..AlignData::default()
            },
            ..StyleData::default()
        },
    ]
}

fn sample() -> WorkbookData {
    let sheet1 = SheetData {
        name: "Orders".into(),
        cells: vec![
            v(0, 0, Value::text("Item"), Some(0)),
            v(0, 1, Value::text("Qty"), None),
            v(0, 2, Value::text("multi\nline \"quoted\" & <tagged>"), None),
            v(1, 0, Value::Number(1.5), Some(0)),
            v(1, 1, Value::Number(-2.0), None),
            v(1, 2, Value::Bool(true), None),
            v(1, 3, Value::Number(46314.0), Some(1)),
            v(1, 4, Value::Number(0.125), Some(2)),
            v(1, 5, Value::Number(1.5), Some(3)),
            v(1, 6, Value::Number(46314.75), Some(5)),
            f(2, 0, "of:=SUM([.A2:.B2])", Some(Value::Number(-0.5))),
            f(
                2,
                1,
                "of:=IF([.C2];\"yes\";\"no\")",
                Some(Value::text("yes")),
            ),
            f(2, 2, "of:=[.A2]>1", Some(Value::Bool(true))),
            f(2, 3, "of:=1/0", Some(Value::Error(ErrorKind::Div0))),
            f(2, 4, "of:=['Second sheet'.A1]*2", Some(Value::Number(84.0))),
            f(
                2,
                5,
                "of:=COM.MICROSOFT.IFERROR(1/0;0)",
                Some(Value::Number(0.0)),
            ),
            f(2, 6, "of:=SUM({1;2|3;4})", Some(Value::Number(10.0))),
            f(3, 0, "of:=[.A2]*2", Some(Value::Number(3.0))),
            v(3, 1, Value::Number(3.0), None),
            f(3, 2, "of:=TODAY()", None),
            v(4, 0, Value::Empty, Some(0)),
            v(5, 0, Value::Number(1e-7), None),
            v(5, 1, Value::Number(123456789012345.0), None),
            v(5, 2, Value::text("  spaces  "), None),
        ],
        col_widths: vec![
            DimRun {
                start: 0,
                end: 0,
                size_hmm: 3810, // 144 px
                hidden: false,
            },
            DimRun {
                start: 2,
                end: 3,
                size_hmm: 2540, // 96 px
                hidden: false,
            },
            DimRun {
                start: 7,
                end: 7,
                size_hmm: 1693,
                hidden: true,
            },
        ],
        row_heights: vec![
            DimRun {
                start: 0,
                end: 0,
                size_hmm: 1058, // 40 px = 30 pt
                hidden: false,
            },
            DimRun {
                start: 4,
                end: 4,
                size_hmm: 529,
                hidden: true,
            },
        ],
        merges: vec![CellRangeLocal {
            r0: 4,
            c0: 0,
            r1: 4,
            c1: 2,
        }],
        frozen: Some((1, 1)),
        tab_color: Some(0x3366_ccff),
        hidden: false,
        style_runs: vec![StyleRun {
            col: 9,
            start_row: 0,
            end_row: MAX_ROWS - 1,
            style: 4,
        }],
        array_formulas: vec![CellRangeLocal {
            r0: 3,
            c0: 0,
            r1: 4,
            c1: 0,
        }],
        ..SheetData::default()
    };
    let sheet2 = SheetData {
        name: "Second sheet".into(),
        cells: vec![v(0, 0, Value::Number(42.0), None)],
        ..SheetData::default()
    };
    let sheet3 = SheetData {
        name: "Hidden".into(),
        hidden: true,
        cells: vec![v(0, 0, Value::text("secret"), None)],
        ..SheetData::default()
    };
    let mut wb = WorkbookData {
        sheets: vec![sheet1, sheet2, sheet3],
        styles: styles(),
        names: vec![
            NameData {
                base_cell: None,
                usage: Default::default(),
                name: "Total".into(),
                sheet: None,
                formula: "of:=[$Orders.$A$2:.$B$2]".into(),
            },
            NameData {
                base_cell: None,
                usage: Default::default(),
                name: "Local".into(),
                sheet: Some(1),
                formula: "of:=[$'Second sheet'.$A$1]".into(),
            },
            NameData {
                base_cell: None,
                usage: Default::default(),
                name: "Print_Area".into(),
                sheet: Some(0),
                formula: "of:=[$Orders.$A$1:.$G$6]".into(),
            },
            NameData {
                base_cell: None,
                usage: Default::default(),
                name: "Rate".into(),
                sheet: None,
                formula: "of:=0.25".into(),
            },
        ],
        ..WorkbookData::default()
    };
    wb.settings.active_sheet = 1;
    wb.meta.title = Some("Round trip".into());
    wb
}

#[test]
fn values_formulas_and_layout_survive() {
    let wb = sample();
    let back = round_trip(&wb);
    assert_eq!(back.sheets.len(), 3);
    let names: Vec<&str> = back.sheets.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Orders", "Second sheet", "Hidden"]);
    assert!(back.sheets[2].hidden && !back.sheets[0].hidden);
    assert_eq!(back.settings.active_sheet, 1);
    assert_eq!(back.meta.title.as_deref(), Some("Round trip"));

    for c in &wb.sheets[0].cells {
        let got = cell(&back, 0, &a1(c.row, c.col)).unwrap_or_else(|| panic!("{c:?}"));
        match (&c.content, &got.content) {
            (
                CellContentData::Formula { text, cached },
                CellContentData::Formula {
                    text: t2,
                    cached: c2,
                },
            ) => {
                assert_eq!(text, t2);
                // `TODAY()` had no cached value: rust_xlsxwriter writes 0.
                if cached.is_some() {
                    assert_eq!(cached, c2, "{text}");
                }
            }
            (a, b) => assert_eq!(a, b, "{}", a1(c.row, c.col)),
        }
        let want = c.style.map(|s| &wb.styles[s as usize]);
        let have = got.style.map(|s| &back.styles[s as usize]);
        assert_eq!(want, have, "style of {}", a1(c.row, c.col));
    }
    assert_eq!(value(&back, 1, "A1"), num(42.0));
    assert_eq!(value(&back, 2, "A1"), text("secret"));

    let s = &back.sheets[0];
    assert_eq!(s.frozen, Some((1, 1)));
    assert_eq!(s.tab_color, Some(0x3366_ccff));
    assert_eq!(s.merges, wb.sheets[0].merges);
    assert_eq!(s.array_formulas, wb.sheets[0].array_formulas);
    // Widths/heights go through whole pixels.
    assert_eq!(s.col_widths, wb.sheets[0].col_widths);
    assert_eq!(s.row_heights, wb.sheets[0].row_heights);
    // The whole-column style comes back as a column style.
    let runs: Vec<(u32, u32, u32)> = s
        .style_runs
        .iter()
        .map(|r| (r.col, r.start_row, r.end_row))
        .collect();
    assert_eq!(runs, [(9, 0, MAX_ROWS - 1)]);
    assert_eq!(back.styles[s.style_runs[0].style as usize], wb.styles[4]);
}

#[test]
fn names_survive() {
    let back = round_trip(&sample());
    let mut names: Vec<(String, Option<u32>, String)> = back
        .names
        .iter()
        .map(|n| (n.name.clone(), n.sheet, n.formula.clone()))
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            ("Local".into(), Some(1), "of:=[$'Second sheet'.$A$1]".into()),
            ("Rate".into(), None, "of:=0.25".into()),
            ("Total".into(), None, "of:=[$Orders.$A$2:.$B$2]".into()),
        ]
    );
    // A `Print_Area` name comes back as the sheet's print range.
    let p = back.sheets[0].page_setup.as_ref().unwrap();
    assert_eq!(
        p.print_range,
        Some(CellRangeLocal {
            r0: 0,
            c0: 0,
            r1: 5,
            c1: 6
        })
    );
}

#[test]
fn error_values_become_error_formulas() {
    // OOXML has no way for rust_xlsxwriter to write a bare error cell; it
    // is written as `=#N/A` with the cached error (Excel shows the same).
    let wb = WorkbookData {
        sheets: vec![SheetData {
            name: "E".into(),
            cells: vec![
                v(0, 0, Value::Error(ErrorKind::NA), None),
                v(0, 1, Value::Error(ErrorKind::Circular), None),
            ],
            ..SheetData::default()
        }],
        ..WorkbookData::default()
    };
    let back = round_trip(&wb);
    assert_eq!(
        formula(&back, 0, "A1"),
        ("of:=#N/A".into(), Some(Value::Error(ErrorKind::NA)))
    );
    // OOo-only codes have no Excel spelling.
    assert_eq!(
        formula(&back, 0, "B1"),
        ("of:=#VALUE!".into(), Some(Value::Error(ErrorKind::Value)))
    );
}

fn a1(row: u32, col: u32) -> String {
    format!("{}{}", tally_types::col_to_letters(col), row + 1)
}

/// The Excel serial stored in `<v>` of `ref` in the first worksheet.
fn stored_serial(bytes: &[u8], cell_ref: &str) -> f64 {
    use std::io::Read;
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut xml = String::new();
    z.by_name("xl/worksheets/sheet1.xml")
        .unwrap()
        .read_to_string(&mut xml)
        .unwrap();
    let at = xml.find(&format!("<c r=\"{cell_ref}\"")).unwrap();
    let v = xml[at..].find("<v>").unwrap() + at + 3;
    let end = xml[v..].find("</v>").unwrap() + v;
    xml[v..end].parse().unwrap()
}

fn workbook_xml(bytes: &[u8]) -> String {
    use std::io::Read;
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut xml = String::new();
    z.by_name("xl/workbook.xml")
        .unwrap()
        .read_to_string(&mut xml)
        .unwrap();
    xml
}

#[test]
fn dates_across_all_three_epochs() {
    let dates = [(1900, 1, 1), (1900, 2, 28), (1900, 3, 1), (2026, 10, 19)];
    let excel_1900 = [1.0, 59.0, 61.0, 46314.0];
    for epoch in [DateEpoch::D1899, DateEpoch::D1900, DateEpoch::D1904] {
        let mut wb = WorkbookData {
            styles: styles(),
            ..WorkbookData::default()
        };
        wb.settings.calc_settings.date_epoch = epoch;
        let mut cells = Vec::new();
        for (i, (y, m, d)) in dates.iter().enumerate() {
            let s = serial_from_ymd(epoch, *y, *m, *d).unwrap();
            cells.push(v(0, i as u32, Value::Number(s), Some(1)));
            // The same serial without a date format is just a number.
            cells.push(v(1, i as u32, Value::Number(s), None));
            // A date-formatted formula's cached value is mapped too.
            cells.push(CellData {
                style: Some(1),
                ..f(
                    2,
                    i as u32,
                    &format!("of:=[.{}1]", a1(0, i as u32).trim_end_matches('1')),
                    Some(Value::Number(s)),
                )
            });
        }
        wb.sheets.push(SheetData {
            name: "Dates".into(),
            cells,
            ..SheetData::default()
        });
        let mut out = std::io::Cursor::new(Vec::new());
        write_xlsx(&wb, &mut out).unwrap();
        let bytes = out.into_inner();
        let wbxml = workbook_xml(&bytes);
        assert_eq!(
            wbxml.contains("date1904=\"1\""),
            epoch == DateEpoch::D1904,
            "{epoch:?}"
        );
        for (i, (y, m, d)) in dates.iter().enumerate() {
            let s = serial_from_ymd(epoch, *y, *m, *d).unwrap();
            let r = a1(0, i as u32);
            let want = if epoch == DateEpoch::D1904 {
                s
            } else {
                excel_1900[i]
            };
            assert_eq!(stored_serial(&bytes, &r), want, "{epoch:?} {y}-{m}-{d}");
            assert_eq!(stored_serial(&bytes, &a1(1, i as u32)), s, "plain number");
            assert_eq!(stored_serial(&bytes, &a1(2, i as u32)), want, "cached");
        }
        // Reading back: D1904 stays D1904; the 1900 system reads as D1899.
        let back = read_xlsx(std::io::Cursor::new(bytes), &Limits::default()).unwrap();
        let back_epoch = back.settings.calc_settings.date_epoch;
        let want_epoch = if epoch == DateEpoch::D1904 {
            DateEpoch::D1904
        } else {
            DateEpoch::D1899
        };
        assert_eq!(back_epoch, want_epoch);
        for (i, (y, m, d)) in dates.iter().enumerate() {
            let Value::Number(n) = value(&back, 0, &a1(0, i as u32)) else {
                panic!()
            };
            let ymd = ymd_from_serial(back_epoch, n).unwrap();
            assert_eq!(ymd, (*y, *m, *d), "{epoch:?} -> {back_epoch:?}");
        }
    }
}

#[test]
fn fixtures_survive_a_write_read_cycle() {
    for fx in [
        "excel365-basic.xlsx",
        "libreoffice-style.xlsx",
        "excel-1904.xlsx",
    ] {
        let wb = open(fx);
        let back = round_trip(&wb);
        assert_eq!(back.sheets.len(), wb.sheets.len(), "{fx}");
        assert_eq!(
            back.settings.calc_settings.date_epoch,
            wb.settings.calc_settings.date_epoch
        );
        for (a, b) in wb.sheets.iter().zip(&back.sheets) {
            assert_eq!(a.name, b.name);
            assert_eq!(a.hidden, b.hidden);
            assert_eq!(a.frozen, b.frozen);
            assert_eq!(a.merges, b.merges, "{fx} {}", a.name);
            assert_eq!(a.array_formulas, b.array_formulas);
            for c in &a.cells {
                let got = b
                    .cells
                    .iter()
                    .find(|x| x.row == c.row && x.col == c.col)
                    .unwrap_or_else(|| panic!("{fx}: {} {}", a.name, a1(c.row, c.col)));
                match (&c.content, &got.content) {
                    (
                        CellContentData::Formula { text, cached },
                        CellContentData::Formula {
                            text: t2,
                            cached: c2,
                        },
                    ) => {
                        assert_eq!(text, t2, "{fx} {}", a1(c.row, c.col));
                        assert_eq!(cached, c2, "{fx} {}", a1(c.row, c.col));
                    }
                    (
                        CellContentData::Value(Value::Error(e)),
                        CellContentData::Formula { cached, .. },
                    ) => {
                        assert_eq!(cached, &Some(Value::Error(*e)));
                    }
                    (x, y) => assert_eq!(x, y, "{fx} {}", a1(c.row, c.col)),
                }
                let sa = c.style.map(|s| wb.styles[s as usize].clone());
                let sb = got.style.map(|s| back.styles[s as usize].clone());
                assert_eq!(sa, sb, "{fx} style {}", a1(c.row, c.col));
            }
        }
        let mut na: Vec<_> = wb
            .names
            .iter()
            .map(|n| (&n.name, n.sheet, &n.formula))
            .collect();
        let mut nb: Vec<_> = back
            .names
            .iter()
            .map(|n| (&n.name, n.sheet, &n.formula))
            .collect();
        na.sort();
        nb.sort();
        // `Broken` (#REF!) is dropped by Excel-side validation; everything
        // else survives.
        na.retain(|n| n.0 != "Broken");
        nb.retain(|n| n.0 != "Broken");
        assert_eq!(na, nb, "{fx}");
    }
}

#[test]
fn writer_reports_what_it_cannot_write() {
    let mut wb = sample();
    wb.sheets[0].name = "Bad/Name:With*Chars".into();
    wb.sheets[0]
        .cells
        .push(f(10, 0, "of:=SUM(", Some(Value::Number(1.0))));
    wb.sheets[0]
        .cells
        .push(v(10, 1, Value::text("x".repeat(40_000)), None));
    let mut out = std::io::Cursor::new(Vec::new());
    let report = write_xlsx(&wb, &mut out).unwrap();
    let all = report.warnings.join("\n");
    assert!(all.contains("renamed"), "{all}");
    assert!(all.contains("could not be translated"), "{all}");
    assert!(all.contains("truncated"), "{all}");
    let back = read_xlsx(std::io::Cursor::new(out.into_inner()), &Limits::default()).unwrap();
    assert_eq!(back.sheets[0].name, "Bad_Name_With_Chars");
    // Formulas that pointed at the renamed sheet follow the new name.
    assert_eq!(value(&back, 0, "A11"), num(1.0));
    match content(&back, 0, "B11") {
        CellContentData::Value(Value::Text(t)) => assert_eq!(t.chars().count(), 32_767),
        other => panic!("{other:?}"),
    }
}

#[test]
fn empty_workbook_writes_one_sheet() {
    let back = round_trip(&WorkbookData::default());
    assert_eq!(back.sheets.len(), 1);
}

/// Format Cells ▸ Numbers language and currency choices (G14) survive XLSX:
/// Excel keeps both as `[$-407]` / `[$€-407]` tags in the code.
#[test]
fn language_and_currency_tags_round_trip() {
    let codes = [
        "[$$-409]#,##0.00;[RED]-[$$-409]#,##0.00",
        "#,##0.00 [$€-407];[RED]-#,##0.00 [$€-407]",
        "[$£-809]#,##0.00;-[$£-809]#,##0.00",
        "[$￥-411]#,##0;[RED]-[$￥-411]#,##0",
        "[$SFr.-807] #,##0.00;[RED][$SFr.-807] -#,##0.00",
        "[$-407]DD.MM.YYYY",
        "[$-40C]DD/MM/YYYY HH:MM",
    ];
    let wb = WorkbookData {
        sheets: vec![SheetData {
            name: "S".into(),
            cells: (0..codes.len() as u32)
                .map(|r| v(r, 0, Value::Number(1234.5), Some(r)))
                .collect(),
            ..SheetData::default()
        }],
        styles: codes
            .iter()
            .map(|c| StyleData {
                number_format: Some((*c).into()),
                ..StyleData::default()
            })
            .collect(),
        ..WorkbookData::default()
    };
    let back = round_trip(&wb);
    for (i, code) in codes.iter().enumerate() {
        let st = style(&back, 0, &format!("A{}", i + 1)).unwrap();
        let got = st.number_format.as_deref().unwrap_or("");
        // Excel spells colours in title case; the code is otherwise kept.
        assert_eq!(got.replace("[Red]", "[RED]"), *code);
    }
}

/// A language tag on a number section is written (Excel keeps it) but
/// read back without it: in Excel and AOO it does not change separators.
#[test]
fn language_tag_on_numbers_is_dropped_on_read() {
    let codes = ["[$-407]#,##0.00;[RED]-#,##0.00", "[$-C0A]0.00%"];
    let wb = WorkbookData {
        sheets: vec![SheetData {
            name: "S".into(),
            cells: (0..codes.len() as u32)
                .map(|r| v(r, 0, Value::Number(1234.5), Some(r)))
                .collect(),
            ..SheetData::default()
        }],
        styles: codes
            .iter()
            .map(|c| StyleData {
                number_format: Some((*c).into()),
                ..StyleData::default()
            })
            .collect(),
        ..WorkbookData::default()
    };
    let back = round_trip(&wb);
    let got = |a1: &str| style(&back, 0, a1).and_then(|s| s.number_format);
    assert_eq!(got("A1").as_deref(), Some("#,##0.00;[RED]-#,##0.00"));
    assert_eq!(got("A2").as_deref(), Some("0.00%"));
}
