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
    // A child of Heading: styles are stored resolved, so it carries the
    // bold and the size it inherits.
    let mut money = StyleData {
        fill: Some(0xffff_ccff),
        ..heading.clone()
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
        sort: Some(DbSortData {
            keys: vec![(2, false), (0, true)],
            case_sensitive: true,
            include_formats: false,
            natural: false,
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
            layout: PivotLayoutData::OutlineBottom,
            empty_line_after: true,
            ..PivotFieldData::default()
        }],
        col_fields: vec![PivotFieldData {
            source_col: 2,
            name: "Qty".into(),
            layout: PivotLayoutData::OutlineTop,
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
        source_filter: None,
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
    // Its sort (`table:sort`, field numbers from the range's first column).
    assert_eq!(db.sort, wb.sheets[0].database_ranges[0].sort);
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
    // `table:data-pilot-layout-info` (layout mode and empty lines).
    assert_eq!(p.row_fields[0].layout, PivotLayoutData::OutlineBottom);
    assert!(p.row_fields[0].empty_line_after);
    assert_eq!(p.col_fields[0].layout, PivotLayoutData::OutlineTop);
    assert!(!p.col_fields[0].empty_line_after);
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

#[test]
fn window_split_round_trips_and_frozen_panes_win() {
    let split = SplitData {
        x_hmm: 6350,
        y_hmm: 3175,
    };
    let sheet = |name: &str, frozen: Option<(u32, u32)>| SheetData {
        name: name.into(),
        frozen,
        split: Some(split),
        cells: vec![cell(0, 0, Value::Number(1.0))],
        ..SheetData::default()
    };
    let wb = WorkbookData {
        sheets: vec![sheet("Split", None), sheet("Frozen", Some((1, 2)))],
        ..WorkbookData::default()
    };
    let back = round_trip(&wb);
    // 240 × 120 px at 96 dpi.
    assert_eq!(back.sheets[0].split, Some(split));
    assert_eq!(back.sheets[0].frozen, None);
    assert_eq!(back.sheets[1].split, None, "a frozen pane wins");
    assert_eq!(back.sheets[1].frozen, Some((1, 2)));
}

/// A child style (named, or a cell's direct format) that turns an
/// attribute of its parent back off must not get it back from the parent
/// chain on reading (parity X13, review 4 L11).
#[test]
fn child_styles_keep_overrides_back_to_the_default() {
    let mut heading = StyleData {
        fill: Some(0xffff_ccff),
        style_name: Some("Heading".into()),
        ..StyleData::default()
    };
    heading.font.bold = true;
    heading.font.italic = true;
    heading.font.underline = true;
    heading.font.size_pt = Some(16.0);
    heading.align.h = HAlign::Center;
    heading.align.wrap = true;
    heading.borders.top = Some(BorderLine::default());
    // The named child: Heading without bold, fill, centring and border.
    let mut plain = StyleData {
        style_name: Some("Plain".into()),
        ..StyleData::default()
    };
    plain.font.italic = true;
    plain.font.size_pt = Some(16.0);
    // A cell on Heading with direct formatting that removes the rest.
    let direct = StyleData {
        style_name: Some("Heading".into()),
        ..StyleData::default()
    };
    let wb = WorkbookData {
        sheets: vec![SheetData {
            name: "S".into(),
            cells: vec![
                CellData {
                    row: 0,
                    col: 0,
                    content: CellContentData::Value(Value::Number(1.0)),
                    style: Some(0),
                },
                CellData {
                    row: 1,
                    col: 0,
                    content: CellContentData::Value(Value::Number(2.0)),
                    style: Some(1),
                },
            ],
            ..SheetData::default()
        }],
        styles: vec![direct.clone(), plain.clone()],
        cell_styles: vec![
            CellStyleData {
                name: "Heading".into(),
                parent: Some("Default".into()),
                style: heading,
            },
            CellStyleData {
                name: "Plain".into(),
                parent: Some("Heading".into()),
                style: plain.clone(),
            },
        ],
        ..WorkbookData::default()
    };
    let back = round_trip(&wb);
    let named = back.cell_styles.iter().find(|s| s.name == "Plain").unwrap();
    assert_eq!(named.parent.as_deref(), Some("Heading"));
    assert_eq!(named.style, plain);
    let style_of = |row: u32| {
        let c = back.sheets[0].cells.iter().find(|c| c.row == row).unwrap();
        back.styles[c.style.unwrap() as usize].clone()
    };
    // The default size is not stored; under a parent with a size it is
    // written, and read back, as the size it is shown at.
    let mut direct_back = direct.clone();
    direct_back.font.size_pt = Some(DEFAULT_FONT_SIZE_PT);
    assert_eq!(style_of(0), direct_back);
    assert_eq!(style_of(1), plain);
}

/// The other attributes a parent can set and a child turn off again:
/// number format, text colour, strike-through, rotation, shrink to fit,
/// indent, vertical alignment, diagonal lines and protection.
#[test]
fn child_styles_reset_every_inherited_attribute() {
    let mut parent = StyleData {
        number_format: Some("0.00".into()),
        protection: Some(CellProtectionData {
            locked: false,
            hidden_formula: true,
            hidden: false,
            hide_print: true,
        }),
        style_name: Some("Loud".into()),
        ..StyleData::default()
    };
    parent.font.strike = true;
    parent.font.color = Some(0xff00_00ff);
    parent.align.rotation_deg = 45;
    parent.align.shrink_to_fit = true;
    parent.align.indent = 2;
    parent.align.v = VAlign::Top;
    let line = BorderLine {
        style: BorderStyle::Solid,
        width_hmm: 26,
        color: 0x0000_00ff,
    };
    parent.borders.diagonal_down = Some(line.clone());
    parent.borders.diagonal_up = Some(line.clone());
    parent.borders.left = Some(line);
    let quiet = StyleData {
        style_name: Some("Quiet".into()),
        ..StyleData::default()
    };
    let direct = StyleData {
        style_name: Some("Loud".into()),
        ..StyleData::default()
    };
    let wb = WorkbookData {
        sheets: vec![SheetData {
            name: "S".into(),
            cells: vec![CellData {
                row: 0,
                col: 0,
                content: CellContentData::Value(Value::Number(1.5)),
                style: Some(0),
            }],
            ..SheetData::default()
        }],
        styles: vec![direct.clone()],
        cell_styles: vec![
            CellStyleData {
                name: "Loud".into(),
                parent: Some("Default".into()),
                style: parent.clone(),
            },
            CellStyleData {
                name: "Quiet".into(),
                parent: Some("Loud".into()),
                style: quiet.clone(),
            },
        ],
        ..WorkbookData::default()
    };
    let back = round_trip(&wb);
    let named = |n: &str| &back.cell_styles.iter().find(|s| s.name == n).unwrap().style;
    assert_eq!(named("Loud"), &parent);
    assert_eq!(named("Quiet"), &quiet);
    let c = &back.sheets[0].cells[0];
    assert_eq!(back.styles[c.style.unwrap() as usize], direct);
}

fn part(bytes: &[u8], name: &str) -> String {
    let mut z = zip::ZipArchive::new(Cursor::new(bytes.to_vec())).unwrap();
    let mut f = z.by_name(name).unwrap();
    let mut s = String::new();
    std::io::Read::read_to_string(&mut f, &mut s).unwrap();
    s
}

/// A child style is written as its difference from the parent, as AOO
/// writes only a style's own items (`XMLStyleExport::exportStyle`): what
/// it shares with the parent stays inherited, so an edit to the parent in
/// AOO reaches the child (review 4 L11).
#[test]
fn child_styles_are_written_as_the_difference_from_the_parent() {
    let mut heading = StyleData {
        fill: Some(0xffff_ccff),
        number_format: Some("0.00".into()),
        style_name: Some("Heading".into()),
        ..StyleData::default()
    };
    heading.font.bold = true;
    heading.font.size_pt = Some(16.0);
    heading.align.h = HAlign::Center;
    // The child adds italic and nothing else.
    let mut child = heading.clone();
    child.font.italic = true;
    child.style_name = Some("Child".into());
    // A cell on the child with a red text colour as direct formatting.
    let mut direct = child.clone();
    direct.font.color = Some(0xff00_00ff);
    let wb = WorkbookData {
        sheets: vec![SheetData {
            name: "S".into(),
            cells: vec![CellData {
                row: 0,
                col: 0,
                content: CellContentData::Value(Value::Number(1.0)),
                style: Some(0),
            }],
            ..SheetData::default()
        }],
        styles: vec![direct.clone()],
        cell_styles: vec![
            CellStyleData {
                name: "Heading".into(),
                parent: Some("Default".into()),
                style: heading.clone(),
            },
            CellStyleData {
                name: "Child".into(),
                parent: Some("Heading".into()),
                style: child.clone(),
            },
        ],
        ..WorkbookData::default()
    };
    let mut out = Cursor::new(Vec::new());
    write_ods(&wb, &mut out).unwrap();
    let bytes = out.into_inner();
    let styles = part(&bytes, "styles.xml");
    assert!(
        styles.contains(
            r#"<style:style style:name="Child" style:family="table-cell" style:parent-style-name="Heading" style:display-name="Child"><style:text-properties fo:font-style="italic" style:font-style-asian="italic" style:font-style-complex="italic"/></style:style>"#
        ),
        "{styles}"
    );
    let content = part(&bytes, "content.xml");
    assert!(
        content.contains(
            r##"<style:style style:name="ce1" style:family="table-cell" style:parent-style-name="Child"><style:text-properties fo:color="#ff0000"/></style:style>"##
        ),
        "{content}"
    );
    // Read back, every style is whole again.
    let back = read_ods(Cursor::new(bytes), &Limits::default()).unwrap();
    let named = |n: &str| &back.cell_styles.iter().find(|s| s.name == n).unwrap().style;
    assert_eq!(named("Heading"), &heading);
    assert_eq!(named("Child"), &child);
    let c = &back.sheets[0].cells[0];
    assert_eq!(back.styles[c.style.unwrap() as usize], direct);
}

/// One sheet with a number in A1.. down column A, cell `i` on style `i`.
fn styled_book(styles: Vec<StyleData>, cell_styles: Vec<CellStyleData>) -> WorkbookData {
    WorkbookData {
        sheets: vec![SheetData {
            name: "S".into(),
            cells: (0..styles.len() as u32)
                .map(|i| CellData {
                    row: i,
                    col: 0,
                    content: CellContentData::Value(Value::Number(1.0)),
                    style: Some(i),
                })
                .collect(),
            ..SheetData::default()
        }],
        styles,
        cell_styles,
        ..WorkbookData::default()
    }
}

fn named(name: &str, parent: &str, style: &StyleData) -> CellStyleData {
    CellStyleData {
        name: name.into(),
        parent: Some(parent.into()),
        style: StyleData {
            style_name: Some(name.into()),
            ..style.clone()
        },
    }
}

fn style_of_row(wb: &WorkbookData, row: u32) -> StyleData {
    let c = wb.sheets[0].cells.iter().find(|c| c.row == row).unwrap();
    wb.styles[c.style.unwrap() as usize].clone()
}

/// A style without a size under a parent with one is written with the
/// size it is shown at, the one default every part of Tally uses
/// (review 6 M2: the writer said 10 pt, the engine shows 11).
#[test]
fn default_size_under_a_sized_parent_is_written_as_shown() {
    let mut heading = StyleData::default();
    heading.font.size_pt = Some(16.0);
    let small = StyleData::default();
    let wb = styled_book(
        vec![StyleData {
            style_name: Some("Small".into()),
            ..StyleData::default()
        }],
        vec![
            named("Heading", "Default", &heading),
            named("Small", "Heading", &small),
        ],
    );
    let mut out = Cursor::new(Vec::new());
    write_ods(&wb, &mut out).unwrap();
    let bytes = out.into_inner();
    let size = format!(r#"fo:font-size="{DEFAULT_FONT_SIZE_PT}pt""#);
    let styles = part(&bytes, "styles.xml");
    let at = styles.find(r#"<style:style style:name="Small""#).unwrap();
    assert!(styles[at..].contains(&size), "{styles}");
    let back = read_ods(Cursor::new(bytes), &Limits::default()).unwrap();
    let shown = |s: &StyleData| s.font.size_pt.unwrap_or(DEFAULT_FONT_SIZE_PT);
    let small_back = &back
        .cell_styles
        .iter()
        .find(|s| s.name == "Small")
        .unwrap()
        .style;
    assert_eq!(shown(small_back), DEFAULT_FONT_SIZE_PT);
    assert_eq!(shown(&style_of_row(&back, 0)), DEFAULT_FONT_SIZE_PT);
}

/// A style without a font under a parent with one names the font it is
/// drawn with: the document's default font, else the application font
/// (review 6 L2: nothing was written without a default font, and the
/// parent's font came back).
#[test]
fn font_reset_under_a_parent_names_the_font_in_use() {
    let mut fancy = StyleData::default();
    fancy.font.family = Some("Georgia".into());
    let plain = StyleData::default();
    let book = || {
        styled_book(
            vec![StyleData {
                style_name: Some("Fancy".into()),
                ..StyleData::default()
            }],
            vec![
                named("Fancy", "Default", &fancy),
                named("Plain", "Fancy", &plain),
            ],
        )
    };
    for default_font in [None, Some("Arial")] {
        let mut wb = book();
        wb.settings.default_font = default_font.map(str::to_string);
        let want = default_font.unwrap_or(APP_FONT_FAMILY);
        let mut out = Cursor::new(Vec::new());
        write_ods(&wb, &mut out).unwrap();
        let bytes = out.into_inner();
        let attr = format!(r#"style:font-name="{want}""#);
        let face = format!(r#"<style:font-face style:name="{want}""#);
        for name in ["styles.xml", "content.xml"] {
            let xml = part(&bytes, name);
            assert!(xml.contains(&face), "{name}: {xml}");
            let style = if name == "styles.xml" {
                r#"<style:style style:name="Plain""#
            } else {
                r#"<style:style style:name="ce1""#
            };
            let at = xml.find(style).unwrap();
            let end = at + xml[at..].find("</style:style>").unwrap();
            assert!(xml[at..end].contains(&attr), "{name}: {xml}");
        }
        let back = read_ods(Cursor::new(bytes), &Limits::default()).unwrap();
        let plain_back = &back
            .cell_styles
            .iter()
            .find(|s| s.name == "Plain")
            .unwrap()
            .style;
        // Not the parent's Georgia: the font in use, by name.
        assert_eq!(plain_back.font.family.as_deref(), Some(want));
        assert_eq!(style_of_row(&back, 0).font.family.as_deref(), Some(want));
    }
}

/// A number format the writer cannot express is written as General; under
/// a parent with a number format that has to be said, or the parent's
/// format comes back (review 6 L3).
#[test]
fn unwritable_number_format_is_general_not_the_parents() {
    // ODF 1.2 has no `E-` exponent.
    let bad = "0.00E-00";
    let loud = StyleData {
        number_format: Some("0.00".into()),
        ..StyleData::default()
    };
    let sci = StyleData {
        number_format: Some(bad.into()),
        ..StyleData::default()
    };
    let wb = styled_book(
        vec![
            StyleData {
                style_name: Some("Loud".into()),
                ..sci.clone()
            },
            // No named style: nothing to override, nothing written.
            sci.clone(),
        ],
        vec![named("Loud", "Default", &loud), named("Sci", "Loud", &sci)],
    );
    let mut out = Cursor::new(Vec::new());
    let report = tally_io_ods::write_ods_with_report(&wb, &mut out).unwrap();
    assert!(
        report.warnings.iter().any(|w| w.contains(bad)),
        "{:?}",
        report.warnings
    );
    let bytes = out.into_inner();
    let styles = part(&bytes, "styles.xml");
    assert!(
        styles.contains(r#"<style:style style:name="Sci" style:family="table-cell" style:parent-style-name="Loud" style:display-name="Sci" style:data-style-name="NSGeneral">"#),
        "{styles}"
    );
    let content = part(&bytes, "content.xml");
    assert!(
        content.contains(r#"<style:style style:name="ce1" style:family="table-cell" style:parent-style-name="Loud" style:data-style-name="NGeneral">"#),
        "{content}"
    );
    assert!(
        content.contains(r#"<style:style style:name="ce2" style:family="table-cell" style:parent-style-name="Default">"#),
        "{content}"
    );
    let back = read_ods(Cursor::new(bytes), &Limits::default()).unwrap();
    let fmt = |n: &str| {
        back.cell_styles
            .iter()
            .find(|s| s.name == n)
            .unwrap()
            .style
            .number_format
            .clone()
    };
    assert_eq!(fmt("Loud").as_deref(), Some("0.00"));
    assert_eq!(fmt("Sci"), None);
    assert_eq!(style_of_row(&back, 0).number_format, None);
    assert_eq!(style_of_row(&back, 0).style_name.as_deref(), Some("Loud"));
    // The plain cell is General, with or without a style of its own.
    let plain = back.sheets[0].cells.iter().find(|c| c.row == 1).unwrap();
    assert!(plain
        .style
        .is_none_or(|i| back.styles[i as usize].number_format.is_none()));
}

/// Styles are written as differences, so a chain of any depth has to be
/// walked to its root on reading (review 6 L1: the walk stopped after 16
/// parents and the root's attributes were lost).
#[test]
fn deep_parent_chains_keep_the_roots_attributes() {
    let mut bold = StyleData::default();
    bold.font.bold = true;
    bold.fill = Some(0x1122_33ff);
    let depth = 40;
    let mut cell_styles = vec![named("L0", "Default", &bold)];
    for i in 1..depth {
        cell_styles.push(named(&format!("L{i}"), &format!("L{}", i - 1), &bold));
    }
    let leaf = format!("L{}", depth - 1);
    let on_leaf = StyleData {
        style_name: Some(leaf.clone()),
        ..bold.clone()
    };
    let wb = styled_book(vec![on_leaf.clone()], cell_styles);
    let mut out = Cursor::new(Vec::new());
    write_ods(&wb, &mut out).unwrap();
    let bytes = out.into_inner();
    // Only the root says bold.
    assert_eq!(
        part(&bytes, "styles.xml").matches("fo:font-weight").count(),
        1
    );
    let back = read_ods(Cursor::new(bytes), &Limits::default()).unwrap();
    for s in &back.cell_styles {
        assert!(s.style.font.bold, "{}", s.name);
        assert_eq!(s.style.fill, Some(0x1122_33ff), "{}", s.name);
    }
    assert_eq!(back.cell_styles.len(), depth);
    assert_eq!(style_of_row(&back, 0), on_leaf);
}

/// The package with `styles.xml` passed through `edit`.
fn with_styles(bytes: Vec<u8>, edit: impl Fn(String) -> String) -> Vec<u8> {
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for i in 0..z.len() {
        let mut f = z.by_index(i).unwrap();
        let name = f.name().unwrap().to_string();
        let mut data = Vec::new();
        std::io::Read::read_to_end(&mut f, &mut data).unwrap();
        if name == "styles.xml" {
            data = edit(String::from_utf8(data).unwrap()).into_bytes();
        }
        out.start_file(name.as_str(), zip::write::SimpleFileOptions::default())
            .unwrap();
        std::io::Write::write_all(&mut out, &data).unwrap();
    }
    out.finish().unwrap().into_inner()
}

/// A parent cycle in a file ends where it closes, and a long chain that
/// runs into one is read in time linear in its length.
#[test]
fn parent_cycles_end() {
    let mut bold = StyleData::default();
    bold.font.bold = true;
    let mut italic = bold.clone();
    italic.font.italic = true;
    let n = 3000;
    let mut cell_styles = vec![named("A", "Default", &bold), named("B", "A", &italic)];
    // A long tail hanging off the cycle.
    for i in 0..n {
        let parent = if i == 0 {
            "B".to_string()
        } else {
            format!("T{}", i - 1)
        };
        cell_styles.push(named(&format!("T{i}"), &parent, &italic));
    }
    let wb = styled_book(
        vec![StyleData {
            style_name: Some(format!("T{}", n - 1)),
            ..italic.clone()
        }],
        cell_styles,
    );
    let mut out = Cursor::new(Vec::new());
    write_ods(&wb, &mut out).unwrap();
    // A's parent becomes B: A → B → A.
    let a = r#"<style:style style:name="A" style:family="table-cell" style:parent-style-name="Default""#;
    let bytes = with_styles(out.into_inner(), |x| {
        assert!(x.contains(a), "{x}");
        x.replacen(a, &a.replace(r#""Default""#, r#""B""#), 1)
    });
    let t = std::time::Instant::now();
    let back = read_ods(Cursor::new(bytes), &Limits::default()).unwrap();
    assert!(t.elapsed() < std::time::Duration::from_secs(20));
    assert_eq!(back.cell_styles.len(), n + 2);
    // The cycle is cut at one of its two links (which one depends on the
    // style that is resolved first), so its members keep at least their
    // own attributes; everything behind it has both.
    for s in &back.cell_styles {
        let f = &s.style.font;
        match s.name.as_str() {
            "A" => assert!(f.bold),
            "B" => assert!(f.italic),
            _ => assert!(f.bold && f.italic, "{}", s.name),
        }
    }
    let cell = style_of_row(&back, 0);
    assert!(cell.font.bold && cell.font.italic);
}
