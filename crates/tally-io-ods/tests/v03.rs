//! v0.3 features through ODS: conditional formats, data validation,
//! notes, hyperlinks, protection, page setup, breaks and properties.
#![allow(clippy::unwrap_used)]

use std::io::Cursor;

use tally_io_ods::{read_ods, write_ods_with_report, Limits};
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

fn th(kind: CfValueKindData, value: &str) -> CfThresholdData {
    CfThresholdData {
        kind,
        value: value.into(),
    }
}

fn rule(kind: CfKindData, style: CfStyleData) -> CfRuleData {
    CfRuleData {
        kind,
        style,
        stop_if_true: false,
    }
}

fn bold_red() -> CfStyleData {
    CfStyleData {
        bold: Some(true),
        color: Some(0xff00_00ff),
        fill: Some(0xffff_00ff),
        ..CfStyleData::default()
    }
}

fn sample() -> WorkbookData {
    let cfs = vec![
        ConditionalFormatData {
            ranges: vec![r(0, 0, 9, 0), r(0, 2, 9, 2)],
            base: (0, 0),
            rules: vec![
                rule(
                    CfKindData::CellValue {
                        op: CmpOpData::Between,
                        formulas: vec!["of:=1".into(), "of:=[.B1]*2".into()],
                    },
                    bold_red(),
                ),
                rule(
                    CfKindData::Formula {
                        formula: "of:=MOD([.A1];2)=0".into(),
                    },
                    CfStyleData {
                        italic: Some(true),
                        ..CfStyleData::default()
                    },
                ),
            ],
        },
        ConditionalFormatData {
            ranges: vec![r(0, 1, 9, 1)],
            base: (0, 1),
            rules: vec![
                rule(
                    CfKindData::ColorScale {
                        stops: vec![
                            CfColorStopData {
                                threshold: th(CfValueKindData::Min, ""),
                                color: 0xff00_00ff,
                            },
                            CfColorStopData {
                                threshold: th(CfValueKindData::Percentile, "50"),
                                color: 0xffff_00ff,
                            },
                            CfColorStopData {
                                threshold: th(CfValueKindData::Max, ""),
                                color: 0x00ff_00ff,
                            },
                        ],
                    },
                    CfStyleData::default(),
                ),
                rule(
                    CfKindData::IconSet {
                        set: IconSetData::FiveQuarters,
                        thresholds: vec![
                            th(CfValueKindData::Percent, "0"),
                            th(CfValueKindData::Percent, "20"),
                            th(CfValueKindData::Percent, "40"),
                            th(CfValueKindData::Number, "60"),
                            th(CfValueKindData::Percent, "80"),
                        ],
                        show_value: false,
                        reverse: true,
                    },
                    CfStyleData::default(),
                ),
                rule(
                    CfKindData::DataBar {
                        min: th(CfValueKindData::Min, ""),
                        max: th(CfValueKindData::Max, ""),
                        color: 0x638e_c6ff,
                        negative_color: None,
                        show_value: true,
                        gradient: true,
                    },
                    CfStyleData::default(),
                ),
            ],
        },
        ConditionalFormatData {
            ranges: vec![r(0, 3, 9, 3)],
            base: (0, 3),
            rules: vec![
                rule(CfKindData::Duplicate, bold_red()),
                rule(
                    CfKindData::Top {
                        rank: 10,
                        percent: true,
                        bottom: true,
                    },
                    bold_red(),
                ),
                rule(
                    CfKindData::Average {
                        above: false,
                        equal: true,
                    },
                    bold_red(),
                ),
                rule(
                    CfKindData::Text {
                        op: CfTextOpData::BeginsWith,
                        text: "ab".into(),
                    },
                    bold_red(),
                ),
                rule(CfKindData::NotBlank, bold_red()),
                rule(CfKindData::Error, bold_red()),
                rule(
                    CfKindData::DateIs {
                        period: CfDatePeriodData::LastMonth,
                    },
                    bold_red(),
                ),
            ],
        },
    ];
    let validations = vec![
        ValidationData {
            ranges: vec![r(0, 5, 9, 5)],
            base: (0, 5),
            kind: ValidationKindData::List {
                source: ListSourceData::Values(vec!["Paid".into(), "Due".into()]),
            },
            allow_blank: true,
            show_dropdown: true,
            input_help: Some(ValidationMessageData {
                title: "Status".into(),
                text: "Pick one".into(),
            }),
            error: Some(ValidationErrorData {
                style: ValidationErrorStyleData::Warning,
                title: "Oops".into(),
                text: "Not a status".into(),
            }),
        },
        ValidationData {
            ranges: vec![r(0, 6, 9, 6)],
            base: (0, 6),
            kind: ValidationKindData::Whole {
                op: CmpOpData::Between,
                formulas: vec!["of:=1".into(), "of:=[.$H$1]".into()],
            },
            allow_blank: false,
            show_dropdown: false,
            input_help: None,
            error: Some(ValidationErrorData::default()),
        },
        ValidationData {
            ranges: vec![r(0, 7, 0, 7)],
            base: (0, 7),
            kind: ValidationKindData::List {
                source: ListSourceData::Range("of:=[$Sheet1.$A$1:.$A$5]".into()),
            },
            allow_blank: true,
            show_dropdown: true,
            input_help: None,
            error: None,
        },
    ];
    let sheet = SheetData {
        name: "Sheet1".into(),
        cells: vec![
            cell(0, 0, Value::Number(1.0)),
            cell(0, 8, Value::text("Site")),
            cell(1, 8, Value::text("Other")),
        ],
        conditional_formats: cfs,
        validations,
        notes: vec![NoteData {
            row: 2,
            col: 2,
            text: "Check this".into(),
            author: Some("Ana".into()),
            created: None,
            shown: false,
        }],
        links: vec![
            LinkData {
                row: 0,
                col: 8,
                url: "https://example.com/".into(),
            },
            LinkData {
                row: 1,
                col: 8,
                url: "#Sheet1.A5".into(),
            },
        ],
        protection: Some(SheetProtectionData {
            password_hash: Some("83AF".into()),
            algorithm: Some(hash_algorithms::XLSX_LEGACY.into()),
            insert_rows: true,
            sort: true,
            ..SheetProtectionData::default()
        }),
        page_setup: Some(PageSetupData {
            orientation: OrientationData::Landscape,
            paper: PaperData::A3,
            margins: MarginsData {
                top: 2540,
                bottom: 1270,
                left: 1905,
                right: 1905,
                header: 762,
                footer: 762,
            },
            scale: PageScaleData::FitTo {
                width: 1,
                height: 0,
            },
            print_range: Some(r(0, 0, 19, 7)),
            repeat_rows: Some((0, 1)),
            repeat_cols: None,
            header: Some(HeaderFooterData {
                left: "&[FILE]".into(),
                center: String::new(),
                right: "Page &[PAGE] of &[PAGES]".into(),
            }),
            footer: Some(HeaderFooterData {
                center: "&[SHEET] & co".into(),
                ..HeaderFooterData::default()
            }),
            print_gridlines: true,
            print_headers: true,
            center_h: true,
            center_v: false,
            page_order: PageOrderData::LeftToRight,
            first_page_number: Some(3),
            ..Default::default()
        }),
        row_breaks: vec![20, 40],
        col_breaks: vec![5],
        ..SheetData::default()
    };
    let mut wb = WorkbookData {
        sheets: vec![sheet],
        ..WorkbookData::default()
    };
    wb.meta.title = Some("T".into());
    wb.meta.subject = Some("S".into());
    wb.meta.author = Some("Ana".into());
    wb.meta.keywords = vec!["a".into(), "b".into()];
    wb.meta.description = Some("D".into());
    wb.settings.structure_protection = Some(SheetProtectionData {
        password_hash: Some("CE4B".into()),
        algorithm: Some(hash_algorithms::XLSX_LEGACY.into()),
        ..SheetProtectionData::default()
    });
    wb
}

fn round_trip(wb: &WorkbookData) -> WorkbookData {
    let mut buf = Cursor::new(Vec::new());
    let report = write_ods_with_report(wb, &mut buf).unwrap();
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    read_ods(Cursor::new(buf.into_inner()), &Limits::default()).unwrap()
}

#[test]
fn conditional_formats_round_trip() {
    let wb = sample();
    let back = round_trip(&wb);
    let got = &back.sheets[0].conditional_formats;
    assert_eq!(got, &wb.sheets[0].conditional_formats);
    // (Also compare rule by rule for clearer failures.)
    let flat = |cfs: &[ConditionalFormatData]| -> Vec<(Vec<CellRangeLocal>, CfRuleData)> {
        cfs.iter()
            .flat_map(|cf| cf.rules.iter().map(|r| (cf.ranges.clone(), r.clone())))
            .collect()
    };
    let want = flat(&wb.sheets[0].conditional_formats);
    let have = flat(got);
    assert_eq!(have.len(), want.len());
    for (w, h) in want.iter().zip(&have) {
        assert_eq!(w.0, h.0, "ranges");
        assert_eq!(w.1.kind, h.1.kind);
        if !matches!(
            w.1.kind,
            CfKindData::ColorScale { .. } | CfKindData::DataBar { .. } | CfKindData::IconSet { .. }
        ) {
            assert_eq!(w.1.style, h.1.style, "{:?}", w.1.kind);
        }
    }
}

#[test]
fn validations_notes_links_round_trip() {
    let wb = sample();
    let back = round_trip(&wb);
    let s = &back.sheets[0];
    assert_eq!(s.validations, wb.sheets[0].validations);
    assert_eq!(s.notes, wb.sheets[0].notes);
    assert_eq!(s.links, wb.sheets[0].links);
    assert_eq!(
        s.cells.iter().find(|c| c.col == 8).map(|c| &c.content),
        Some(&CellContentData::Value(Value::text("Site")))
    );
}

#[test]
fn protection_page_setup_and_properties_round_trip() {
    let wb = sample();
    let back = round_trip(&wb);
    let s = &back.sheets[0];
    assert_eq!(s.protection, wb.sheets[0].protection);
    assert_eq!(s.page_setup, wb.sheets[0].page_setup);
    assert_eq!(s.row_breaks, vec![20, 40]);
    assert_eq!(s.col_breaks, vec![5]);
    assert_eq!(
        back.settings.structure_protection,
        wb.settings.structure_protection
    );
    assert_eq!(back.meta.subject.as_deref(), Some("S"));
    assert_eq!(back.meta.author.as_deref(), Some("Ana"));
    assert_eq!(back.meta.keywords, vec!["a", "b"]);
    assert_eq!(back.meta.description.as_deref(), Some("D"));
}

#[test]
fn cell_protection_round_trips() {
    let mut wb = sample();
    wb.styles.push(StyleData {
        protection: Some(CellProtectionData {
            locked: false,
            hidden_formula: true,
            ..CellProtectionData::default()
        }),
        ..StyleData::default()
    });
    wb.sheets[0].cells[0].style = Some(0);
    let back = round_trip(&wb);
    let st = back.sheets[0].cells[0].style.unwrap();
    assert_eq!(back.styles[st as usize].protection, wb.styles[0].protection);
}

#[test]
fn ods_specifics() {
    let mut wb = sample();
    wb.settings.locale = Some("fr-FR".into());
    wb.sheets[0].page_setup.as_mut().unwrap().repeat_cols = Some((0, 1));
    wb.sheets[0].protection = Some(SheetProtectionData {
        password_hash: Some("abc=".into()),
        algorithm: Some(hash_algorithms::SHA256.into()),
        format_cells: true,
        ..SheetProtectionData::default()
    });
    let back = round_trip(&wb);
    assert_eq!(back.settings.locale.as_deref(), Some("fr-FR"));
    assert_eq!(back.sheets[0].protection, wb.sheets[0].protection);
    assert_eq!(back.sheets[0].page_setup, wb.sheets[0].page_setup);
    // The style:map fallback for OpenOffice is written next to calcext.
    let mut buf = Cursor::new(Vec::new());
    write_ods_with_report(&wb, &mut buf).unwrap();
    let mut z = zip::ZipArchive::new(Cursor::new(buf.into_inner())).unwrap();
    let mut xml = String::new();
    std::io::Read::read_to_string(&mut z.by_name("content.xml").unwrap(), &mut xml).unwrap();
    assert!(
        xml.contains("<style:map style:condition=\"cell-content-is-between(1,[.B1]*2)\""),
        "{xml}"
    );
    assert!(xml.contains("calcext:conditional-formats"));
    // Without the calcext block, the style:map is read back (AOO files).
    let stripped = {
        let a = xml.find("<calcext:conditional-formats>").unwrap();
        let b = xml.find("</calcext:conditional-formats>").unwrap()
            + "</calcext:conditional-formats>".len();
        format!("{}{}", &xml[..a], &xml[b..])
    };
    // Re-pack the package with the stripped content.xml.
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for i in 0..z.len() {
        let mut f = z.by_index(i).unwrap();
        let name = f.name().unwrap().to_string();
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut f, &mut bytes).unwrap();
        let opts = zip::write::SimpleFileOptions::default();
        out.start_file(name.as_str(), opts).unwrap();
        let body = if name == "content.xml" {
            stripped.as_bytes().to_vec()
        } else {
            bytes
        };
        std::io::Write::write_all(&mut out, &body).unwrap();
    }
    let bytes = out.finish().unwrap().into_inner();
    let legacy = read_ods(Cursor::new(bytes), &Limits::default()).unwrap();
    let cfs = &legacy.sheets[0].conditional_formats;
    assert_eq!(cfs.len(), 1, "{cfs:?}");
    // Only the existing cell A1 carries the derived style.
    assert_eq!(cfs[0].ranges, vec![r(0, 0, 0, 0)]);
    assert_eq!(cfs[0].rules.len(), 2);
    assert_eq!(
        cfs[0].rules[0].kind,
        wb.sheets[0].conditional_formats[0].rules[0].kind
    );
    assert_eq!(cfs[0].rules[0].style, bold_red());
}
