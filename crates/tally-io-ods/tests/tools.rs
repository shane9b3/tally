//! Wave 5 (W5-2) through ODS: named styles, format extras, outline
//! groups, scenarios, named database ranges with filters and subtotal
//! rules, DataPilot tables and view options; round trips and the
//! LibreOffice-shaped fixture.
#![allow(clippy::unwrap_used)]

mod common;

use std::io::Cursor;

use common::read_fixture;
use tally_io_ods::{read_ods, write_ods, Limits};
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

fn round_trip(wb: &WorkbookData) -> WorkbookData {
    let mut out = Cursor::new(Vec::new());
    write_ods(wb, &mut out).unwrap();
    read_ods(Cursor::new(out.into_inner()), &Limits::default()).unwrap()
}

fn sample() -> WorkbookData {
    let mut heading = StyleData::default();
    heading.font.bold = true;
    heading.font.size_pt = Some(16.0);
    heading.style_name = Some("Heading".into());
    let mut money = StyleData {
        fill: Some(0xffff_ccff),
        ..StyleData::default()
    };
    money.align.shrink_to_fit = true;
    money.align.indent = 2;
    money.font.underline = true;
    money.font.underline_style = UnderlineStyleData::Double;
    money.borders.diagonal_up = Some(BorderLine {
        style: BorderStyle::Solid,
        width_hmm: 26,
        color: 0xff00_00ff,
    });
    money.style_name = Some("Money".into());
    let mut rotated = StyleData::default();
    rotated.align.rotation_deg = 45;
    rotated.align.stacked = false;
    rotated.borders.diagonal_down = Some(BorderLine {
        style: BorderStyle::Dashed,
        width_hmm: 26,
        color: 0x0000_00ff,
    });
    let mut cell_on_heading = heading.clone();
    cell_on_heading.font.color = Some(0xff00_00ff);
    let mut data = SheetData {
        name: "Data".into(),
        ..SheetData::default()
    };
    let header = ["Region", "Product", "Qty", "Amount"];
    for (c, h) in header.iter().enumerate() {
        data.cells.push(CellData {
            row: 0,
            col: c as u32,
            content: CellContentData::Value(Value::text(*h)),
            style: (c == 0).then_some(0),
        });
    }
    let rows = [
        ("East", "Apple", 10.0, 100.0),
        ("East", "Pear", 5.0, 60.0),
        ("West", "Apple", 20.0, 200.0),
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
            style: Some(if i == 0 { 1 } else { 2 }),
        });
    }
    data.row_groups = vec![
        OutlineGroupData {
            start: 1,
            end: 3,
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
    data.scenarios = vec![ScenarioData {
        name: "High".into(),
        comment: "what if".into(),
        ranges: vec![r(1, 2, 2, 2)],
        cells: vec![
            cell(1, 2, Value::Number(99.0)),
            cell(2, 2, Value::Number(55.0)),
        ],
        border_color: Some(0x0000_ffff),
        display_border: true,
        copy_back: false,
        copy_styles: false,
        prevent_changes: true,
        active: true,
    }];
    data.database_ranges = vec![DatabaseRangeData {
        name: "Sales".into(),
        range: r(0, 0, 3, 3),
        has_header: true,
        filter_buttons: false,
        filter: Some(StandardFilterData {
            range: r(0, 0, 3, 3),
            conditions: vec![
                StdConditionData {
                    connector: StdConnectorData::And,
                    col: 0,
                    op: StdFilterOpData::Eq,
                    value: "East".into(),
                },
                StdConditionData {
                    connector: StdConnectorData::Or,
                    col: 2,
                    op: StdFilterOpData::Largest,
                    value: "2".into(),
                },
                StdConditionData {
                    connector: StdConnectorData::And,
                    col: 1,
                    op: StdFilterOpData::NotBeginsWith,
                    value: "P".into(),
                },
            ],
            options: StdFilterOptionsData {
                case_sensitive: true,
                has_header: true,
                no_duplicates: true,
                ..StdFilterOptionsData::default()
            },
            criteria: None,
        }),
        subtotals: Some(SubtotalsData {
            range: r(0, 0, 3, 3),
            groups: vec![SubtotalGroupData {
                group_by: 0,
                columns: vec![3, 2],
                function: AggFnData::Average,
            }],
            page_breaks: true,
            case_sensitive: false,
            presort: true,
            ascending: false,
            include_formats: false,
        }),
    }];
    data.view_options = Some(ViewOptionsData {
        show_gridlines: false,
        show_zero_values: false,
        ..ViewOptionsData::default()
    });
    let mut pivot_sheet = SheetData {
        name: "Pivot".into(),
        ..SheetData::default()
    };
    pivot_sheet.cells.push(cell(0, 0, Value::text("Region")));
    pivot_sheet.pivots = vec![PivotData {
        name: "DataPilot1".into(),
        source: PivotSourceData::DatabaseRange {
            name: "Sales".into(),
        },
        target: (0, 0),
        output: Some(r(0, 0, 3, 1)),
        row_fields: vec![PivotFieldData {
            source_col: 0,
            name: "Region".into(),
            sort: PivotSortData::Data {
                data_field: 0,
                ascending: false,
            },
            ..PivotFieldData::default()
        }],
        col_fields: vec![PivotFieldData {
            source_col: 2,
            name: "Qty".into(),
            subtotals: PivotSubtotalsData::Custom(vec![AggFnData::Max, AggFnData::Min]),
            show_empty: true,
            group: Some(PivotGroupData::Numeric {
                start: Some(0.0),
                end: None,
                step: 5.0,
            }),
            ..PivotFieldData::default()
        }],
        page_fields: vec![PivotFieldData {
            source_col: 1,
            name: "Product".into(),
            page_item: Some("Apple".into()),
            hidden_items: vec!["Plum".into()],
            subtotals: PivotSubtotalsData::None,
            ..PivotFieldData::default()
        }],
        data_fields: vec![
            PivotDataFieldData {
                source_col: 3,
                function: AggFnData::Sum,
                caption: None,
                show_as: Some(ShowAsData {
                    kind: ShowAsKindData::PercentOfRow,
                    base_field: Some(0),
                    base_item: None,
                }),
            },
            PivotDataFieldData {
                source_col: 3,
                function: AggFnData::Count,
                caption: Some("Cnt".into()),
                show_as: None,
            },
        ],
        data_layout: DataLayoutData::Column,
        row_totals: true,
        col_totals: false,
        ignore_empty_rows: true,
        identify_categories: false,
        show_filter_button: false,
        drill_down: true,
    }];
    WorkbookData {
        sheets: vec![data, pivot_sheet],
        styles: vec![cell_on_heading, money.clone(), rotated],
        cell_styles: vec![
            CellStyleData {
                name: "Heading".into(),
                parent: Some("Default".into()),
                style: heading,
            },
            CellStyleData {
                name: "Money".into(),
                parent: Some("Heading".into()),
                style: money,
            },
        ],
        ..WorkbookData::default()
    }
}

#[test]
fn ods_tools_round_trip() {
    let wb = sample();
    let back = round_trip(&wb);
    let d = &back.sheets[0];
    assert_eq!(back.sheets.len(), 2, "the scenario sheet folds back");
    // Named styles and cell extras.
    let names: Vec<&str> = back.cell_styles.iter().map(|s| s.name.as_str()).collect();
    assert!(
        names.contains(&"Heading") && names.contains(&"Money"),
        "{names:?}"
    );
    let money = back.cell_styles.iter().find(|s| s.name == "Money").unwrap();
    assert_eq!(money.parent.as_deref(), Some("Heading"));
    assert!(money.style.font.bold, "inherited from Heading");
    assert_eq!(money.style.font.underline_style, UnderlineStyleData::Double);
    assert!(money.style.align.shrink_to_fit);
    assert_eq!(money.style.align.indent, 2);
    assert!(money.style.borders.diagonal_up.is_some());
    let a1 = d.cells.iter().find(|c| c.row == 0 && c.col == 0).unwrap();
    let a1_style = &back.styles[a1.style.unwrap() as usize];
    assert_eq!(a1_style.style_name.as_deref(), Some("Heading"));
    assert_eq!(a1_style.font.color, Some(0xff00_00ff));
    let d3 = d.cells.iter().find(|c| c.row == 2 && c.col == 3).unwrap();
    let d3_style = &back.styles[d3.style.unwrap() as usize];
    assert_eq!(d3_style.align.rotation_deg, 45);
    assert_eq!(
        d3_style.borders.diagonal_down.as_ref().map(|b| b.style),
        Some(BorderStyle::Dashed)
    );
    assert!(d3_style.style_name.is_none());
    // Outline.
    assert_eq!(d.row_groups, wb.sheets[0].row_groups);
    assert_eq!(d.col_groups, wb.sheets[0].col_groups);
    // Scenario.
    assert_eq!(d.scenarios.len(), 1);
    let sc = &d.scenarios[0];
    assert_eq!(sc.name, "High");
    assert_eq!(sc.comment, "what if");
    assert_eq!(sc.ranges, vec![r(1, 2, 2, 2)]);
    assert_eq!(sc.cells.len(), 2);
    assert_eq!(sc.border_color, Some(0x0000_ffff));
    assert!(sc.active && !sc.copy_back && sc.prevent_changes);
    // Database range with filter and subtotal rules.
    assert_eq!(d.database_ranges.len(), 1);
    let db = &d.database_ranges[0];
    assert_eq!(db.name, "Sales");
    assert_eq!(db.range, r(0, 0, 3, 3));
    let f = db.filter.as_ref().unwrap();
    assert_eq!(f.conditions.len(), 3);
    assert_eq!(f.conditions[1].connector, StdConnectorData::Or);
    assert_eq!(f.conditions[1].op, StdFilterOpData::Largest);
    assert_eq!(f.conditions[2].connector, StdConnectorData::And);
    assert_eq!(f.conditions[2].op, StdFilterOpData::NotBeginsWith);
    assert!(f.options.case_sensitive && f.options.no_duplicates);
    let st = db.subtotals.as_ref().unwrap();
    assert_eq!(st.groups[0].columns, vec![3, 2]);
    assert_eq!(st.groups[0].function, AggFnData::Average);
    assert!(st.page_breaks && st.presort && !st.ascending);
    assert_eq!(d.view_options, wb.sheets[0].view_options);
    // Pivot.
    let p = &back.sheets[1].pivots[0];
    assert_eq!(p.name, "DataPilot1");
    assert_eq!(
        p.source,
        PivotSourceData::DatabaseRange {
            name: "Sales".into()
        }
    );
    assert_eq!(p.output, Some(r(0, 0, 3, 1)));
    assert_eq!(p.row_fields.len(), 1);
    assert_eq!(p.row_fields[0].source_col, 0);
    assert_eq!(
        p.row_fields[0].sort,
        PivotSortData::Data {
            data_field: 0,
            ascending: false
        }
    );
    assert_eq!(p.col_fields[0].source_col, 2);
    assert_eq!(
        p.col_fields[0].subtotals,
        PivotSubtotalsData::Custom(vec![AggFnData::Max, AggFnData::Min])
    );
    assert!(p.col_fields[0].show_empty);
    assert!(
        matches!(p.col_fields[0].group, Some(PivotGroupData::Numeric { step, .. }) if step == 5.0)
    );
    assert_eq!(p.page_fields[0].page_item.as_deref(), Some("Apple"));
    assert_eq!(p.page_fields[0].hidden_items, vec!["Plum".to_string()]);
    assert_eq!(p.data_fields.len(), 2);
    assert_eq!(p.data_fields[0].function, AggFnData::Sum);
    assert_eq!(
        p.data_fields[0]
            .show_as
            .as_ref()
            .map(|s| (s.kind, s.base_field)),
        Some((ShowAsKindData::PercentOfRow, Some(0)))
    );
    assert_eq!(p.data_fields[1].caption.as_deref(), Some("Cnt"));
    assert_eq!(p.data_layout, DataLayoutData::Column);
    assert!(p.row_totals && !p.col_totals && p.ignore_empty_rows && !p.show_filter_button);
}

#[test]
fn reads_libreoffice_tools_fixture() {
    let wb = read_ods(
        Cursor::new(read_fixture("ods/tools-lo.ods")),
        &Limits::default(),
    )
    .unwrap();
    assert_eq!(wb.sheets.len(), 2);
    let d = &wb.sheets[0];
    assert_eq!(d.name, "Data");
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
    assert_eq!(d.col_groups.len(), 1);
    assert_eq!((d.col_groups[0].start, d.col_groups[0].end), (1, 2));
    assert_eq!(d.scenarios.len(), 1);
    assert_eq!(d.scenarios[0].name, "High");
    assert_eq!(d.scenarios[0].cells.len(), 2);
    assert_eq!(d.scenarios[0].ranges, vec![r(1, 2, 2, 2)]);
    let db = &d.database_ranges[0];
    assert_eq!(db.name, "Sales");
    let f = db.filter.as_ref().unwrap();
    assert_eq!(f.conditions.len(), 3);
    assert_eq!(f.conditions[1].connector, StdConnectorData::Or);
    assert_eq!(f.conditions[2].connector, StdConnectorData::And);
    assert_eq!(f.conditions[2].op, StdFilterOpData::BeginsWith);
    assert!(f.options.no_duplicates && f.options.case_sensitive);
    let st = db.subtotals.as_ref().unwrap();
    assert_eq!(st.groups[0].columns, vec![3, 2]);
    assert!(st.page_breaks && !st.ascending);
    let v = d.view_options.as_ref().unwrap();
    assert!(!v.show_gridlines && !v.show_zero_values && v.show_headers);
    // Named styles and cell extras.
    let heading1 = wb
        .cell_styles
        .iter()
        .find(|s| s.name == "Heading 1")
        .unwrap();
    assert_eq!(heading1.parent.as_deref(), Some("Heading"));
    assert!(heading1.style.font.bold && heading1.style.align.rotation_deg == 90);
    let money = wb.cell_styles.iter().find(|s| s.name == "Money").unwrap();
    assert_eq!(money.style.font.underline_style, UnderlineStyleData::Double);
    assert_eq!(money.style.align.indent, 2);
    assert!(money.style.align.shrink_to_fit && money.style.borders.diagonal_up.is_some());
    let a1 = d.cells.iter().find(|c| c.row == 0 && c.col == 0).unwrap();
    let st = &wb.styles[a1.style.unwrap() as usize];
    assert_eq!(st.style_name.as_deref(), Some("Heading"));
    assert!(st.font.bold && st.font.color == Some(0xff00_00ff));
    let d6 = d.cells.iter().find(|c| c.row == 5 && c.col == 3).unwrap();
    let st = &wb.styles[d6.style.unwrap() as usize];
    assert_eq!(st.align.rotation_deg, 45);
    assert!(st.align.shrink_to_fit && st.borders.diagonal_down.is_some());
    // Pivot.
    let p = &wb.sheets[1].pivots[0];
    assert_eq!(p.name, "DataPilot1");
    assert_eq!(p.row_fields[0].name, "Region");
    assert_eq!(
        p.row_fields[0].sort,
        PivotSortData::Data {
            data_field: 0,
            ascending: false
        }
    );
    assert_eq!(p.page_fields[0].page_item.as_deref(), Some("Apple"));
    assert_eq!(p.col_fields[0].source_col, 2);
    assert_eq!(p.data_fields.len(), 2);
    assert_eq!(p.data_layout, DataLayoutData::Column);
    assert_eq!(p.output, Some(r(0, 0, 3, 1)));
    // And it round-trips.
    let back = round_trip(&wb);
    assert_eq!(back.sheets[0].row_groups, d.row_groups);
    assert_eq!(back.sheets[0].scenarios.len(), 1);
    assert_eq!(back.sheets[1].pivots.len(), 1);
    assert_eq!(back.cell_styles.len(), wb.cell_styles.len());
}
