//! Wave 5 (W5-2) through XLSX: outline levels, scenarios, named styles,
//! format extras, pivot cache/table parts; round trips and the Excel
//! shaped fixture.
#![allow(clippy::unwrap_used)]

mod common;

use common::{open, round_trip, to_bytes};
use tally_model::*;
use tally_types::Value;

fn r(r0: u32, c0: u32, r1: u32, c1: u32) -> CellRangeLocal {
    CellRangeLocal { r0, c0, r1, c1 }
}

fn cell(row: u32, col: u32, v: Value) -> CellData {
    CellData {
        row,
        col,
        content: CellContentData::Value(v),
        style: None,
    }
}

fn sample() -> WorkbookData {
    let mut data = SheetData {
        name: "Data".into(),
        ..SheetData::default()
    };
    for (c, h) in ["Region", "Product", "Qty", "Amount"].iter().enumerate() {
        data.cells.push(cell(0, c as u32, Value::text(*h)));
    }
    let rows = [
        ("East", "Apple", 10.0, 100.0),
        ("East", "Pear", 5.0, 60.0),
        ("West", "Apple", 20.0, 200.0),
        ("West", "Pear", 9.0, 110.0),
    ];
    for (i, (reg, prod, q, a)) in rows.iter().enumerate() {
        let row = i as u32 + 1;
        data.cells.push(cell(row, 0, Value::text(*reg)));
        data.cells.push(cell(row, 1, Value::text(*prod)));
        data.cells.push(cell(row, 2, Value::Number(*q)));
        data.cells.push(CellData {
            row,
            col: 3,
            content: CellContentData::Value(Value::Number(*a)),
            style: Some(0),
        });
    }
    data.row_groups = vec![
        OutlineGroupData {
            start: 1,
            end: 4,
            level: 1,
            collapsed: false,
        },
        OutlineGroupData {
            start: 2,
            end: 3,
            level: 2,
            collapsed: true,
        },
    ];
    data.row_heights = vec![DimRun {
        start: 2,
        end: 3,
        size_hmm: 452,
        hidden: true,
    }];
    data.col_groups = vec![OutlineGroupData {
        start: 1,
        end: 2,
        level: 1,
        collapsed: false,
    }];
    data.scenarios = vec![
        ScenarioData {
            name: "Base".into(),
            comment: "as is".into(),
            ranges: vec![r(1, 2, 2, 2)],
            cells: vec![
                cell(1, 2, Value::Number(10.0)),
                cell(2, 2, Value::Number(5.0)),
            ],
            ..ScenarioData::default()
        },
        ScenarioData {
            name: "High".into(),
            ranges: vec![r(1, 2, 2, 2)],
            cells: vec![
                cell(1, 2, Value::Number(99.0)),
                cell(2, 2, Value::text("x")),
            ],
            prevent_changes: true,
            active: true,
            ..ScenarioData::default()
        },
    ];
    data.view_options = Some(ViewOptionsData {
        show_gridlines: false,
        ..ViewOptionsData::default()
    });
    let mut pivot_sheet = SheetData {
        name: "Pivot".into(),
        ..SheetData::default()
    };
    pivot_sheet.cells.push(cell(0, 0, Value::text("Region")));
    pivot_sheet.pivots = vec![PivotData {
        name: "DataPilot1".into(),
        source: PivotSourceData::Range {
            sheet: 0,
            range: r(0, 0, 4, 3),
        },
        target: (0, 0),
        output: Some(r(0, 0, 3, 1)),
        row_fields: vec![PivotFieldData {
            source_col: 0,
            name: "Region".into(),
            ..PivotFieldData::default()
        }],
        col_fields: Vec::new(),
        page_fields: vec![PivotFieldData {
            source_col: 1,
            name: "Product".into(),
            subtotals: PivotSubtotalsData::None,
            ..PivotFieldData::default()
        }],
        data_fields: vec![PivotDataFieldData {
            source_col: 3,
            function: AggFnData::Average,
            caption: Some("Avg Amount".into()),
            show_as: None,
        }],
        data_layout: DataLayoutData::Row,
        row_totals: true,
        col_totals: false,
        ..PivotData::default()
    }];
    let mut extras = StyleData::default();
    extras.align.rotation_deg = -30;
    extras.align.shrink_to_fit = true;
    extras.align.indent = 1;
    extras.font.underline = true;
    extras.font.underline_style = UnderlineStyleData::DoubleAccounting;
    extras.borders.diagonal_up = Some(BorderLine {
        style: BorderStyle::Solid,
        width_hmm: 26,
        color: 0xff00_00ff,
    });
    let mut heading = StyleData::default();
    heading.font.bold = true;
    heading.style_name = Some("Heading".into());
    WorkbookData {
        sheets: vec![data, pivot_sheet],
        styles: vec![extras],
        cell_styles: vec![CellStyleData {
            name: "Heading".into(),
            parent: Some("Default".into()),
            style: heading,
        }],
        ..WorkbookData::default()
    }
}

#[test]
fn xlsx_tools_round_trip() {
    let wb = sample();
    let back = round_trip(&wb);
    let d = &back.sheets[0];
    assert_eq!(d.row_groups, wb.sheets[0].row_groups, "{:?}", d.row_groups);
    assert_eq!(
        d.col_groups
            .iter()
            .map(|g| (g.start, g.end, g.level))
            .collect::<Vec<_>>(),
        vec![(1, 2, 1)]
    );
    assert_eq!(d.scenarios.len(), 2);
    assert_eq!(d.scenarios[0].name, "Base");
    assert_eq!(d.scenarios[0].comment, "as is");
    assert_eq!(d.scenarios[0].cells.len(), 2);
    assert_eq!(
        d.scenarios[1].cells[0].content,
        CellContentData::Value(Value::Number(99.0))
    );
    assert_eq!(
        d.scenarios[1].cells[1].content,
        CellContentData::Value(Value::text("x"))
    );
    assert!(d.scenarios[1].active && d.scenarios[1].prevent_changes);
    assert!(!d.view_options.as_ref().unwrap().show_gridlines);
    // Format extras.
    let d2 = d.cells.iter().find(|c| c.row == 1 && c.col == 3).unwrap();
    let st = &back.styles[d2.style.unwrap() as usize];
    assert_eq!(st.align.rotation_deg, -30);
    assert!(st.align.shrink_to_fit);
    assert_eq!(st.align.indent, 1);
    assert_eq!(
        st.font.underline_style,
        UnderlineStyleData::DoubleAccounting
    );
    assert!(st.borders.diagonal_up.is_some() && st.borders.diagonal_down.is_none());
    // Named style (the name; the attributes stay on the cells).
    assert!(back.cell_styles.iter().any(|s| s.name == "Heading"));
    // Pivot definition (best effort: fields, functions, location).
    let p = &back.sheets[1].pivots[0];
    assert_eq!(p.name, "DataPilot1");
    assert_eq!(
        p.source,
        PivotSourceData::Range {
            sheet: 0,
            range: r(0, 0, 4, 3)
        }
    );
    assert_eq!(p.output, Some(r(0, 0, 3, 1)));
    assert_eq!(p.row_fields[0].source_col, 0);
    assert_eq!(p.row_fields[0].name, "Region");
    assert_eq!(p.page_fields[0].source_col, 1);
    assert_eq!(p.page_fields[0].subtotals, PivotSubtotalsData::None);
    assert_eq!(p.data_fields[0].function, AggFnData::Average);
    assert_eq!(p.data_fields[0].caption.as_deref(), Some("Avg Amount"));
    assert!(p.row_totals && !p.col_totals);
    // The package has the pivot parts and content types.
    let bytes = to_bytes(&wb);
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let names: Vec<String> = (0..zip.len())
        .map(|i| zip.by_index(i).unwrap().name().unwrap().into_owned())
        .collect();
    assert!(names.contains(&"xl/pivotCache/pivotCacheDefinition1.xml".to_string()));
    assert!(names.contains(&"xl/pivotTables/pivotTable1.xml".to_string()));
    assert!(names.contains(&"xl/worksheets/_rels/sheet2.xml.rels".to_string()));
    let mut ct = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("[Content_Types].xml").unwrap(), &mut ct)
        .unwrap();
    assert!(ct.contains("pivotTable+xml"));
}

#[test]
fn reads_excel_tools_fixture() {
    let wb = open("tools-excel.xlsx");
    let d = &wb.sheets[0];
    assert_eq!(
        d.row_groups,
        vec![
            OutlineGroupData {
                start: 1,
                end: 4,
                level: 1,
                collapsed: false
            },
            OutlineGroupData {
                start: 2,
                end: 3,
                level: 2,
                collapsed: true
            }
        ]
    );
    assert_eq!(
        d.col_groups,
        vec![OutlineGroupData {
            start: 1,
            end: 2,
            level: 1,
            collapsed: false
        }]
    );
    assert_eq!(d.scenarios.len(), 2);
    assert_eq!(d.scenarios[1].name, "High");
    assert!(d.scenarios[1].active && d.scenarios[1].prevent_changes);
    assert_eq!(d.scenarios[0].comment, "as is");
    let v = d.view_options.as_ref().unwrap();
    assert!(!v.show_gridlines && !v.show_zero_values);
    let heading = wb.cell_styles.iter().find(|s| s.name == "Heading").unwrap();
    assert!(heading.style.font.bold);
    assert_eq!(
        heading.style.font.underline_style,
        UnderlineStyleData::Double
    );
    let a1 = d.cells.iter().find(|c| c.row == 0 && c.col == 0).unwrap();
    assert_eq!(
        wb.styles[a1.style.unwrap() as usize].style_name.as_deref(),
        Some("Heading")
    );
    let d2 = d.cells.iter().find(|c| c.row == 1 && c.col == 3).unwrap();
    let st = &wb.styles[d2.style.unwrap() as usize];
    assert_eq!(st.align.rotation_deg, 45);
    assert!(st.align.shrink_to_fit && st.align.indent == 1);
    assert!(st.borders.diagonal_up.is_some());
    let p = &wb.sheets[1].pivots[0];
    assert_eq!(p.name, "PivotTable1");
    assert_eq!(
        p.source,
        PivotSourceData::Range {
            sheet: 0,
            range: r(0, 0, 6, 3)
        }
    );
    assert_eq!(p.row_fields[0].name, "Region");
    assert_eq!(p.page_fields[0].name, "Product");
    assert_eq!(p.data_fields[0].source_col, 3);
    assert_eq!(p.data_fields[0].function, AggFnData::Sum);
    assert_eq!(p.output, Some(r(0, 0, 3, 1)));
}

/// AutoFilter: value lists, a custom condition and AOO's "Top 10" as
/// `<top10>`; filtered rows are hidden rows in Excel and read back as the
/// filter's rows.
#[test]
fn autofilter_round_trip_with_top10() {
    let mut wb = sample();
    // No outline (its collapsed group hides rows of its own).
    wb.sheets[0].row_groups.clear();
    wb.sheets[0].row_heights.retain(|h| !h.hidden);
    let ops = [
        (0, Some(vec!["East".to_string(), String::new()]), None),
        (
            1,
            None,
            Some(FilterConditionData {
                op: FilterOpData::BeginsWith,
                value: "A*p".into(),
            }),
        ),
        (
            2,
            None,
            Some(FilterConditionData {
                op: FilterOpData::TopValues,
                value: "10".into(),
            }),
        ),
        (
            3,
            None,
            Some(FilterConditionData {
                op: FilterOpData::BottomPercent,
                value: "25".into(),
            }),
        ),
    ];
    wb.sheets[0].auto_filter = Some(AutoFilterData {
        range: r(0, 0, 4, 3),
        columns: ops
            .iter()
            .map(|(col, values, condition)| FilterColumnData {
                col: *col,
                values: values.clone(),
                condition: condition.clone(),
            })
            .collect(),
        filtered_rows: vec![(3, 4)],
    });
    let bytes = to_bytes(&wb);
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
    let mut xml = String::new();
    std::io::Read::read_to_string(
        &mut zip.by_name("xl/worksheets/sheet1.xml").unwrap(),
        &mut xml,
    )
    .unwrap();
    assert!(xml.contains(r#"<autoFilter ref="A1:D5">"#), "{xml}");
    assert!(
        xml.contains(
            r#"<filterColumn colId="2"><top10 top="1" val="10" percent="0"/></filterColumn>"#
        ),
        "{xml}"
    );
    assert!(
        xml.contains(r#"<top10 top="0" val="25" percent="1"/>"#),
        "{xml}"
    );
    assert!(xml.contains(r#"<customFilter val="A~*p*"/>"#), "{xml}");
    assert!(
        xml.contains(r#"<filters blank="1"><filter val="East"/></filters>"#),
        "{xml}"
    );
    // Schema order: the autoFilter comes before mergeCells / pageMargins.
    let at = xml.find("<autoFilter").unwrap();
    assert!(at < xml.find("<pageMargins").unwrap());
    // Excel's hidden `_xlnm._FilterDatabase` name is written, and not read
    // back as a user name.
    let mut wbx = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("xl/workbook.xml").unwrap(), &mut wbx).unwrap();
    assert!(
        wbx.contains(r#"<definedName name="_xlnm._FilterDatabase" localSheetId="0" hidden="1">'Data'!$A$1:$D$5</definedName>"#),
        "{wbx}"
    );
    let back = round_trip(&wb);
    let af = back.sheets[0].auto_filter.as_ref().unwrap();
    assert_eq!(af, wb.sheets[0].auto_filter.as_ref().unwrap());
    assert!(!back.sheets[0].row_heights.iter().any(|h| h.hidden));
    assert!(!back.names.iter().any(|n| n.name.contains("FilterDatabase")));
}

/// Replaces `xl/worksheets/sheet1.xml` of a package.
fn with_sheet1(bytes: &[u8], f: impl Fn(String) -> String) -> Vec<u8> {
    let mut zin = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut out = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for i in 0..zin.len() {
        let mut file = zin.by_index(i).unwrap();
        let name = file.name().unwrap().to_string();
        let mut data = Vec::new();
        std::io::Read::read_to_end(&mut file, &mut data).unwrap();
        if name == "xl/worksheets/sheet1.xml" {
            data = f(String::from_utf8(data).unwrap()).into_bytes();
        }
        // Stored: the crafted parts would trip the compression-ratio guard.
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        out.start_file(name, opts).unwrap();
        std::io::Write::write_all(&mut out, &data).unwrap();
    }
    out.finish().unwrap().into_inner()
}

/// Control characters in filter values survive (`&#9;` … and `_xHHHH_`);
/// a column with two custom conditions opens unfiltered with a warning.
#[test]
fn autofilter_control_characters_and_two_conditions() {
    let mut wb = sample();
    wb.sheets[0].row_groups.clear();
    wb.sheets[0].row_heights.retain(|h| !h.hidden);
    let values = vec![
        "a\tb".to_string(),
        "x\u{1}y\n".to_string(),
        "_x0041_".to_string(),
    ];
    wb.sheets[0].auto_filter = Some(AutoFilterData {
        range: r(0, 0, 4, 3),
        columns: vec![FilterColumnData {
            col: 0,
            values: Some(values.clone()),
            condition: None,
        }],
        filtered_rows: Vec::new(),
    });
    let bytes = to_bytes(&wb);
    let back = tally_io_xlsx::read_xlsx(
        std::io::Cursor::new(&bytes),
        &tally_io_xlsx::Limits::default(),
    )
    .unwrap();
    assert_eq!(
        back.sheets[0].auto_filter.as_ref().unwrap().columns[0].values,
        Some(values)
    );
    let two = with_sheet1(&bytes, |x| {
        x.replace(
            "</autoFilter>",
            r#"<filterColumn colId="2"><customFilters and="1"><customFilter operator="greaterThan" val="5"/><customFilter operator="lessThan" val="30"/></customFilters></filterColumn></autoFilter>"#,
        )
    });
    let back =
        tally_io_xlsx::read_xlsx(std::io::Cursor::new(two), &tally_io_xlsx::Limits::default())
            .unwrap();
    let af = back.sheets[0].auto_filter.as_ref().unwrap();
    assert!(af.columns.iter().all(|c| c.col != 2), "{af:?}");
    assert!(
        back.warnings.iter().any(|w| w.contains("two conditions")),
        "{:?}",
        back.warnings
    );
}

/// A crafted filter with more values than allowed keeps the cap and warns.
#[test]
fn autofilter_values_are_capped_with_a_warning() {
    let wb = sample();
    let bytes = to_bytes(&wb);
    let big = "v".repeat(30_000);
    let flood = with_sheet1(&bytes, |x| {
        let mut f = String::from(r#"<autoFilter ref="A1:D5">"#);
        for c in 0..2 {
            f.push_str(&format!(r#"<filterColumn colId="{c}"><filters>"#));
            for i in 0..600 {
                f.push_str(&format!(r#"<filter val="{big}{i}"/>"#));
            }
            f.push_str("</filters></filterColumn>");
        }
        f.push_str("</autoFilter>");
        x.replacen("<pageMargins", &format!("{f}<pageMargins"), 1)
    });
    let back = tally_io_xlsx::read_xlsx(
        std::io::Cursor::new(flood),
        &tally_io_xlsx::Limits::default(),
    )
    .unwrap();
    let af = back.sheets[0].auto_filter.as_ref().unwrap();
    let total: usize = af
        .columns
        .iter()
        .map(|c| {
            c.values
                .as_ref()
                .map_or(0, |v| v.iter().map(String::len).sum::<usize>())
        })
        .sum();
    assert!(total <= 64 << 20, "{total}");
    assert!(
        af.columns[0]
            .values
            .as_ref()
            .unwrap()
            .iter()
            .map(String::len)
            .sum::<usize>()
            <= 16 << 20
    );
    assert!(
        back.warnings
            .iter()
            .any(|w| w.contains("AutoFilter lists more values")),
        "{:?}",
        back.warnings
    );
}
