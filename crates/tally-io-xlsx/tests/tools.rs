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
