//! Write → read round trips and output-shape checks.

#![allow(clippy::unwrap_used)] // tests may unwrap (docs/contributing.md)

mod common;

use std::io::{Cursor, Read};
use std::sync::Arc;

use common::normalize;
use tally_io_ods::{read_ods, sniff_is_ods, write_ods, write_ods_with_report, Limits};
use tally_model::*;
use tally_types::{CalcSettings, CriteriaMode, DateEpoch, ErrorKind, IterSettings, Matrix, Value};

fn v(row: u32, col: u32, value: Value, style: Option<u32>) -> CellData {
    CellData {
        row,
        col,
        content: CellContentData::Value(value),
        style,
    }
}

fn f(row: u32, col: u32, text: &str, cached: Option<Value>, style: Option<u32>) -> CellData {
    CellData {
        row,
        col,
        content: CellContentData::Formula {
            text: text.into(),
            cached,
        },
        style,
    }
}

fn fmt(code: &str) -> StyleData {
    StyleData {
        number_format: Some(code.into()),
        ..StyleData::default()
    }
}

pub const PHASE_A_FORMATS: [&str; 14] = [
    "0",
    "0.00",
    "#,##0",
    "#,##0.00",
    "0%",
    "0.00%",
    "$#,##0.00;[RED]-$#,##0.00",
    "0.00E+00",
    "@",
    "MM/DD/YYYY",
    "YYYY-MM-DD",
    "DD MMM YYYY",
    "HH:MM",
    "HH:MM:SS",
];

fn sample() -> WorkbookData {
    let mut styles = vec![
        StyleData {
            font: FontData {
                bold: true,
                ..FontData::default()
            },
            ..StyleData::default()
        },
        StyleData {
            font: FontData {
                italic: true,
                underline: true,
                strike: true,
                family: Some("DejaVu Sans".into()),
                size_pt: Some(14.5),
                color: Some(0xc921_1eff),
                ..FontData::default()
            },
            fill: Some(0xffff_00ff),
            align: AlignData {
                h: HAlign::Center,
                v: VAlign::Middle,
                wrap: true,
                indent: 0,
            },
            ..StyleData::default()
        },
        StyleData {
            align: AlignData {
                h: HAlign::Right,
                v: VAlign::Top,
                ..AlignData::default()
            },
            ..StyleData::default()
        },
        StyleData {
            font: FontData {
                bold: true,
                italic: true,
                ..FontData::default()
            },
            number_format: Some("#,##0.00".into()),
            ..StyleData::default()
        },
    ];
    let first_fmt = styles.len() as u32;
    for code in PHASE_A_FORMATS.iter().chain(
        [
            "HH:MM AM/PM",
            "[HH]:MM:SS",
            "# ?/?",
            "0;[RED]-0;\"zero\"",
            "[>100]\"big\"0;0",
        ]
        .iter(),
    ) {
        styles.push(fmt(code));
    }
    let mut cells = vec![
        v(0, 0, Value::text("Header"), Some(0)),
        v(0, 1, Value::text("  two  spaces\tand tab  "), Some(1)),
        v(0, 2, Value::text("line1\nline2\n\nline4"), Some(2)),
        v(0, 3, Value::text("<&>\"' ☺"), None),
        v(0, 5, Value::text(""), None),
        v(1, 0, Value::Number(0.1), Some(3)),
        v(1, 1, Value::Number(-1234567.891), None),
        v(1, 2, Value::Number(1e300), None),
        v(1, 3, Value::Number(-2.5e-12), None),
        v(1, 4, Value::Bool(true), None),
        v(1, 5, Value::Bool(false), None),
        v(1, 6, Value::Error(ErrorKind::NA), None),
        v(2, 0, Value::Empty, Some(0)),
        f(
            3,
            0,
            "of:=SUM([.B2:.B3])",
            Some(Value::Number(42.0)),
            Some(3),
        ),
        f(3, 1, "of:=1/0", Some(Value::Error(ErrorKind::Div0)), None),
        f(
            3,
            2,
            "of:=CONCATENATE(\"a\";\"b\")",
            Some(Value::text("ab")),
            None,
        ),
        f(3, 3, "of:=[.A2]<>[.A3]", Some(Value::Bool(true)), None),
        f(3, 4, "of:=RAND()", None, None),
        f(3, 5, "of:=\"\"", Some(Value::text("")), None),
        f(
            3,
            6,
            "of:=SQRT(-1)",
            Some(Value::Error(ErrorKind::Num)),
            None,
        ),
        f(
            3,
            7,
            "of:=CIRC()",
            Some(Value::Error(ErrorKind::Circular)),
            None,
        ),
        f(
            3,
            8,
            "oooc:=[.A1]&\"x\"",
            Some(Value::text("Headerx")),
            None,
        ),
        f(3, 9, "of:=\"a\r\nb\"", Some(Value::text("a\r\nb")), None),
    ];
    for (i, _) in styles.iter().enumerate().skip(first_fmt as usize) {
        cells.push(v(
            5,
            i as u32,
            Value::Number(46_314.604_166_666_67),
            Some(i as u32),
        ));
    }
    cells.push(v(100_000, 16_383, Value::text("far corner"), None));
    let sheet1 = SheetData {
        name: "Orders & <Stuff>".into(),
        cells,
        col_widths: vec![
            DimRun {
                start: 0,
                end: 0,
                size_hmm: 4500,
                hidden: false,
            },
            DimRun {
                start: 3,
                end: 5,
                size_hmm: 1234,
                hidden: false,
            },
            DimRun {
                start: 7,
                end: 7,
                size_hmm: 2258,
                hidden: true,
            },
        ],
        row_heights: vec![
            DimRun {
                start: 2,
                end: 4,
                size_hmm: 1000,
                hidden: false,
            },
            DimRun {
                start: 10,
                end: 10,
                size_hmm: 452,
                hidden: true,
            },
        ],
        frozen: Some((1, 2)),
        tab_color: Some(0x3366_ccff),
        ..SheetData::default()
    };
    let sheet2 = SheetData {
        name: "Empty".into(),
        hidden: true,
        ..SheetData::default()
    };
    let sheet3 = SheetData {
        name: "Third".into(),
        cells: vec![f(
            0,
            0,
            "of:=SUM([$'Orders & <Stuff>'.B2:.B3])",
            Some(Value::Number(1.0)),
            None,
        )],
        ..SheetData::default()
    };
    WorkbookData {
        sheets: vec![sheet1, sheet2, sheet3],
        styles,
        names: vec![],
        settings: DocSettings {
            active_sheet: 2,
            calc_settings: CalcSettings {
                date_epoch: DateEpoch::D1904,
                criteria_mode: CriteriaMode::Regex,
                case_sensitive: true,
                whole_cell_match: false,
                precision_as_shown: true,
                iterations: Some(IterSettings {
                    steps: 77,
                    min_change: 0.0005,
                }),
                ..CalcSettings::default()
            },
        },
        meta: DocMeta {
            title: Some("Round & trip".into()),
            generator: Some("Tally/0.1".into()),
            created: Some("2026-10-19T08:00:00".into()),
            modified: Some("2026-10-19T09:00:00".into()),
        },
    }
}

fn write_to_vec(wb: &WorkbookData) -> Vec<u8> {
    let mut buf = Cursor::new(Vec::new());
    let report = write_ods_with_report(wb, &mut buf).unwrap();
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    buf.into_inner()
}

#[test]
fn round_trip_every_supported_feature() {
    let wb = sample();
    let bytes = write_to_vec(&wb);
    let back = read_ods(Cursor::new(&bytes), &Limits::default()).unwrap();
    let a = normalize(&wb);
    let b = normalize(&back);
    for (sa, sb) in a.wb.sheets.iter().zip(&b.wb.sheets) {
        for (ca, cb) in sa.cells.iter().zip(&sb.cells) {
            assert_eq!(ca, cb);
        }
    }
    for (i, (x, y)) in a.cell_styles.iter().zip(&b.cell_styles).enumerate() {
        for (j, (p, q)) in x.iter().zip(y).enumerate() {
            assert_eq!(p, q, "sheet {i} cell {j}: {:?}", a.wb.sheets[i].cells[j]);
        }
    }
    assert_eq!(a, b);

    // A second generation is byte-identical (deterministic writer).
    assert_eq!(write_to_vec(&back), bytes);
}

#[test]
fn mimetype_is_first_and_stored() {
    let bytes = write_to_vec(&sample());
    // Raw local header: the very first entry.
    assert_eq!(&bytes[0..4], b"PK\x03\x04");
    assert_eq!(u16::from_le_bytes([bytes[8], bytes[9]]), 0, "STORED");
    let name_len = u16::from_le_bytes([bytes[26], bytes[27]]) as usize;
    let extra_len = u16::from_le_bytes([bytes[28], bytes[29]]) as usize;
    assert_eq!(&bytes[30..30 + name_len], b"mimetype");
    let start = 30 + name_len + extra_len;
    assert_eq!(
        &bytes[start..start + 46],
        b"application/vnd.oasis.opendocument.spreadsheet"
    );
    let mut zip = zip::ZipArchive::new(Cursor::new(&bytes)).unwrap();
    let names: Vec<String> = (0..zip.len())
        .map(|i| zip.by_index(i).unwrap().name().unwrap().to_string())
        .collect();
    assert_eq!(
        names,
        [
            "mimetype",
            "META-INF/manifest.xml",
            "content.xml",
            "styles.xml",
            "meta.xml",
            "settings.xml"
        ]
    );
    let first = zip.by_index(0).unwrap();
    assert_eq!(first.compression(), zip::CompressionMethod::Stored);
    assert!(sniff_is_ods(&bytes));
}

#[test]
fn output_shape_matches_odf_conventions() {
    let bytes = write_to_vec(&sample());
    let mut zip = zip::ZipArchive::new(Cursor::new(&bytes)).unwrap();
    let mut content = String::new();
    zip.by_name("content.xml")
        .unwrap()
        .read_to_string(&mut content)
        .unwrap();
    // Empty spans are run-length encoded, never expanded.
    assert!(
        content.contains(r#"table:number-rows-repeated="99989""#),
        "row gap"
    );
    assert!(
        content.contains(r#"table:number-columns-repeated="16383""#),
        "col gap"
    );
    assert!(
        content.len() < 64 * 1024,
        "content.xml is {} bytes",
        content.len()
    );
    assert!(content.contains(r#"table:formula="of:=SUM([.B2:.B3])""#));
    assert!(content.contains(r#"calcext:value-type="error"><text:p>#DIV/0!</text:p>"#));
    assert!(content.contains("<number:currency-style"));
    assert!(content.contains(r#"<style:map style:condition="value()&gt;=0""#));
    assert!(content.contains(r#"style:column-width="4.5cm""#));
    assert!(content.contains(
        r#"<text:s text:c="2"/>two <text:s/>spaces<text:tab/>and tab<text:s text:c="2"/>"#
    ));
    let mut meta = String::new();
    zip.by_name("meta.xml")
        .unwrap()
        .read_to_string(&mut meta)
        .unwrap();
    assert!(meta.contains("<meta:generator>Tally/0.1</meta:generator>"));
    let mut settings = String::new();
    zip.by_name("settings.xml")
        .unwrap()
        .read_to_string(&mut settings)
        .unwrap();
    assert!(settings.contains(r#"<config:config-item config:name="ActiveTable" config:type="string">Third</config:config-item>"#));
}

#[test]
fn unsupported_things_are_reported() {
    let mut wb = sample();
    wb.styles.push(fmt("[NatNum1]0"));
    wb.styles.push(fmt("0;-0;0;@"));
    wb.names.push(NameData {
        name: "N".into(),
        sheet: None,
        formula: "of:=[$Orders.$A$1]".into(),
    });
    wb.sheets[0].merges.push(CellRangeLocal {
        r0: 0,
        c0: 0,
        r1: 0,
        c1: 1,
    });
    wb.sheets[0].cells.push(v(
        200_000,
        0,
        Value::Array(Arc::new(Matrix::new_filled(1, 2, Value::Number(3.0)))),
        None,
    ));
    let mut buf = Cursor::new(Vec::new());
    let report = write_ods_with_report(&wb, &mut buf).unwrap();
    let w = report.warnings.join("\n");
    assert!(w.contains("[NatNum1]0"), "{w}");
    assert!(w.contains("fourth"), "{w}");
    assert!(w.contains("named expressions"), "{w}");
    assert!(w.contains("merged"), "{w}");
    assert!(w.contains("array value"), "{w}");
    let back = read_ods(Cursor::new(buf.into_inner()), &Limits::default()).unwrap();
    // Unsupported formats fall back to General (no number format).
    let i = back.sheets[0]
        .cells
        .iter()
        .find(|c| c.row == 200_000)
        .unwrap();
    assert_eq!(i.content, CellContentData::Value(Value::Number(3.0)));
}

#[test]
fn every_phase_a_format_round_trips_exactly() {
    for code in PHASE_A_FORMATS {
        let wb = WorkbookData {
            sheets: vec![SheetData {
                name: "S".into(),
                cells: vec![v(0, 0, Value::Number(1.5), Some(0))],
                ..SheetData::default()
            }],
            styles: vec![fmt(code)],
            ..WorkbookData::default()
        };
        let bytes = write_to_vec(&wb);
        let back = read_ods(Cursor::new(bytes), &Limits::default()).unwrap();
        assert_eq!(
            back.styles.first().and_then(|s| s.number_format.as_deref()),
            Some(code)
        );
    }
}

#[test]
fn empty_workbook_round_trips() {
    let wb = WorkbookData {
        sheets: vec![SheetData {
            name: "Sheet1".into(),
            ..SheetData::default()
        }],
        ..WorkbookData::default()
    };
    let mut buf = Cursor::new(Vec::new());
    write_ods(&wb, &mut buf).unwrap();
    let mut back = read_ods(Cursor::new(buf.into_inner()), &Limits::default()).unwrap();
    assert_eq!(back.meta.generator.as_deref(), Some("Tally/0.1"));
    back.meta = DocMeta::default();
    assert_eq!(back, wb);
}

/// Writes the round-trip sample to `$TALLY_ODS_OUT_DIR/tally-sample.ods` so an
/// external office suite can be pointed at it (CI: `soffice --headless
/// --convert-to csv`). Run with `--ignored`.
#[test]
#[ignore = "writes a file for external validation"]
fn write_sample_for_external_check() {
    let Some(dir) = std::env::var_os("TALLY_ODS_OUT_DIR") else {
        eprintln!("TALLY_ODS_OUT_DIR not set; skipping");
        return;
    };
    let path = std::path::Path::new(&dir).join("tally-sample.ods");
    std::fs::write(&path, write_to_vec(&sample())).unwrap();
    eprintln!("wrote {}", path.display());
}

/// Run with `cargo test -p tally-io-ods --release -- --ignored`.
#[test]
#[ignore = "performance test; run in release mode"]
fn one_million_cells_write_and_read() {
    let cells: Vec<CellData> = (0..100_000u32)
        .flat_map(|r| (0..10u32).map(move |c| v(r, c, Value::Number(f64::from(r * 10 + c)), None)))
        .collect();
    let wb = WorkbookData {
        sheets: vec![SheetData {
            name: "Big".into(),
            cells,
            ..SheetData::default()
        }],
        ..WorkbookData::default()
    };
    let t = std::time::Instant::now();
    let mut buf = Cursor::new(Vec::new());
    write_ods(&wb, &mut buf).unwrap();
    let written = t.elapsed();
    let bytes = buf.into_inner();
    let t = std::time::Instant::now();
    let back = read_ods(Cursor::new(&bytes), &Limits::default()).unwrap();
    let read = t.elapsed();
    eprintln!(
        "1M cells: write {written:?}, read {read:?}, {} bytes",
        bytes.len()
    );
    assert_eq!(back.sheets[0].cells.len(), 1_000_000);
}
