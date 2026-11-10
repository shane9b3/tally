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
                ..Default::default()
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
            locale: Some("de-DE".into()),
            structure_protection: Some(tally_model::SheetProtectionData {
                password_hash: Some("abc=".into()),
                algorithm: Some("sha256".into()),
                ..tally_model::SheetProtectionData::default()
            }),
            load_readonly: true,
            ..Default::default()
        },
        meta: DocMeta {
            title: Some("Round & trip".into()),
            generator: Some("Tally/0.1".into()),
            created: Some("2026-10-19T08:00:00".into()),
            modified: Some("2026-10-19T09:00:00".into()),
            subject: Some("Subject <x>".into()),
            keywords: vec!["one".into(), "two".into()],
            description: Some("Line 1\nLine 2".into()),
            author: Some("Ana".into()),
            last_author: Some("Ben".into()),
        },
        warnings: Vec::new(),
        ..Default::default()
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
    // AOO's File ▸ Properties ▸ Security "Open file read-only".
    assert!(settings.contains(r#"<config:config-item-set config:name="ooo:configuration-settings"><config:config-item config:name="LoadReadonly" config:type="boolean">true</config:config-item></config:config-item-set>"#));
}

/// `LoadReadonly` is read only from `ooo:configuration-settings` (AOO
/// writes `false` there for ordinary files), and an ordinary file writes
/// no configuration set at all.
#[test]
fn load_readonly_round_trips() {
    let mut wb = sample();
    wb.settings.load_readonly = false;
    let bytes = write_to_vec(&wb);
    let back = read_ods(Cursor::new(&bytes), &Limits::default()).unwrap();
    assert!(!back.settings.load_readonly);
    let mut zip = zip::ZipArchive::new(Cursor::new(&bytes)).unwrap();
    let mut settings = String::new();
    zip.by_name("settings.xml")
        .unwrap()
        .read_to_string(&mut settings)
        .unwrap();
    assert!(!settings.contains("LoadReadonly"));
    wb.settings.load_readonly = true;
    let back = read_ods(Cursor::new(write_to_vec(&wb)), &Limits::default()).unwrap();
    assert!(back.settings.load_readonly);
}

#[test]
fn unsupported_things_are_reported() {
    let mut wb = sample();
    wb.styles.push(fmt("[NatNum1]0"));
    wb.styles.push(fmt("0;-0;0;@"));
    wb.names.push(NameData {
        base_cell: None,
        usage: Default::default(),
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
    // NatNum is written as number:transliteration-* (G7); one that does
    // nothing in its language (NatNum1 without a language) is dropped.
    assert!(!w.contains("[NatNum1]0"), "{w}");
    assert!(w.contains("fourth"), "{w}");
    assert!(!w.contains("named expressions"), "{w}");
    assert!(!w.contains("merged"), "{w}");
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

/// Format Cells ▸ Numbers language and currency choices (G14): the
/// format's language is the data style's `number:language` /
/// `number:country`, a currency's locale is on `number:currency-symbol`.
#[test]
fn language_and_currency_tags_round_trip() {
    let codes = [
        "[$$-409]#,##0.00;[RED]-[$$-409]#,##0.00",
        "[$$-409]#,##0;-[$$-409]#,##0",
        "#,##0.00 [$€-407];[RED]-#,##0.00 [$€-407]",
        "#,##0.00 [$€-40C];-#,##0.00 [$€-40C]",
        "#,##0.00 [$€-C0A];[RED]-#,##0.00 [$€-C0A]",
        "[$£-809]#,##0.00;[RED]-[$£-809]#,##0.00",
        "[$￥-411]#,##0;[RED]-[$￥-411]#,##0",
        "[$-407]#,##0.00 [$€-407];[RED]-#,##0.00 [$€-407]",
        "[$-407]DD.MM.YYYY;@",
        "[$SFr.-807] #,##0.00;[RED][$SFr.-807] -#,##0.00",
        "[$$-1009]#,##0.00;-[$$-1009]#,##0.00",
        "[$$-C09]#,##0.00;-[$$-C09]#,##0.00",
        "#,##0.00 [$USD-409]",
        "[$-407]DD.MM.YYYY",
        "[$-40C]DD MMMM YYYY",
        "[$-411]YYYY/MM/DD HH:MM",
        "[$-407]#,##0.00;[RED]-#,##0.00",
        "[$-809]0.00%",
        "[$-C0A]0.00E+00",
        "$#,##0.00;[RED]-$#,##0.00",
    ];
    for code in codes {
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
        let back = read_ods(Cursor::new(bytes.clone()), &Limits::default()).unwrap();
        assert_eq!(
            back.styles.first().and_then(|s| s.number_format.as_deref()),
            Some(code)
        );
        if code == "[$-407]DD.MM.YYYY" {
            let xml = entry(&bytes, "content.xml");
            assert!(
                xml.contains(r#"<number:date-style style:name="N0" number:language="de" number:country="DE">"#),
                "{xml}"
            );
        }
        if code.contains("[$€-407]") {
            let xml = entry(&bytes, "content.xml");
            assert!(
                xml.contains(r#"<number:currency-symbol number:language="de" number:country="DE">€</number:currency-symbol>"#),
                "{xml}"
            );
        }
    }
    // A language without an ISO mapping is dropped, the format kept.
    let wb = WorkbookData {
        sheets: vec![SheetData {
            name: "S".into(),
            cells: vec![v(0, 0, Value::Number(1.5), Some(0))],
            ..SheetData::default()
        }],
        styles: vec![fmt("[$-2C0A]0.00")],
        ..WorkbookData::default()
    };
    let back = read_ods(Cursor::new(write_to_vec(&wb)), &Limits::default()).unwrap();
    assert_eq!(back.styles[0].number_format.as_deref(), Some("0.00"));
}

fn one_cell(code: &str) -> WorkbookData {
    WorkbookData {
        sheets: vec![SheetData {
            name: "S".into(),
            cells: vec![v(0, 0, Value::Number(1.5), Some(0))],
            ..SheetData::default()
        }],
        styles: vec![fmt(code)],
        ..WorkbookData::default()
    }
}

/// The package with `content.xml` rewritten by `f`.
fn with_content(bytes: &[u8], f: impl Fn(String) -> String) -> Vec<u8> {
    use std::io::Write;
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for i in 0..z.len() {
        let mut e = z.by_index(i).unwrap();
        let name = e.name().unwrap().to_string();
        let mut data = Vec::new();
        e.read_to_end(&mut data).unwrap();
        if name == "content.xml" {
            data = f(String::from_utf8(data).unwrap()).into_bytes();
        }
        let method = if name == "mimetype" {
            zip::CompressionMethod::Stored
        } else {
            zip::CompressionMethod::Deflated
        };
        let opts = zip::write::SimpleFileOptions::default().compression_method(method);
        out.start_file(name, opts).unwrap();
        out.write_all(&data).unwrap();
    }
    out.finish().unwrap().into_inner()
}

/// AOO reads a format's language from the main style (the one with the
/// style:map elements, xmlnumfi.cxx): every section's style carries it.
#[test]
fn language_is_on_the_main_style() {
    for code in [
        "[$-407]#,##0.00;[RED]-#,##0.00",
        "[$-407]DD.MM.YYYY;@",
        "[$-40C]0.00;-0.00;\"zero\"",
    ] {
        let bytes = write_to_vec(&one_cell(code));
        let xml = entry(&bytes, "content.xml");
        let main = xml
            .split("<number:")
            .find(|e| e.contains("style:name=\"N0\""))
            .unwrap();
        assert!(main.contains("number:language="), "{code}: {main}");
        let back = read_ods(Cursor::new(bytes), &Limits::default()).unwrap();
        assert_eq!(back.styles[0].number_format.as_deref(), Some(code));
    }
}

/// A currency style keeps its own language next to its symbol's: a de-DE
/// style with `€` reads back as `[$-407]…[$€-407]`, so the display stays
/// German (review M5).
#[test]
fn currency_style_keeps_its_language() {
    let bytes = write_to_vec(&one_cell("#,##0.00 [$€-407]"));
    let de = with_content(&bytes, |x| {
        x.replace(
            "<number:currency-style style:name=\"N0\">",
            "<number:currency-style style:name=\"N0\" number:language=\"de\" number:country=\"DE\">",
        )
    });
    let back = read_ods(Cursor::new(de), &Limits::default()).unwrap();
    assert_eq!(
        back.styles[0].number_format.as_deref(),
        Some("[$-407]#,##0.00 [$€-407]")
    );
    // AOO's automatic symbol (an empty currency-symbol) is the locale's.
    let auto = with_content(&bytes, |x| {
        x.replace(
            "<number:currency-symbol number:language=\"de\" number:country=\"DE\">€</number:currency-symbol>",
            "<number:currency-symbol number:language=\"de\" number:country=\"DE\"/>",
        )
    });
    let back = read_ods(Cursor::new(auto), &Limits::default()).unwrap();
    assert_eq!(
        back.styles[0].number_format.as_deref(),
        Some("#,##0.00 [$€-407]")
    );
    let bare = with_content(&bytes, |x| {
        x.replace(
            "<number:currency-symbol number:language=\"de\" number:country=\"DE\">€</number:currency-symbol>",
            "<number:currency-symbol/>",
        )
    });
    let back = read_ods(Cursor::new(bare), &Limits::default()).unwrap();
    assert_eq!(
        back.styles[0].number_format.as_deref(),
        Some("#,##0.00 [$]")
    );
}

fn entry(bytes: &[u8], name: &str) -> String {
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut s = String::new();
    z.by_name(name).unwrap().read_to_string(&mut s).unwrap();
    s
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
    // The written `table:calculation-settings` pins the 1930 window.
    assert_eq!(back.settings.calc_settings.two_digit_year, Some(1930));
    back.settings.calc_settings.two_digit_year = None;
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

/// Correctness review H4: style runs (a long run over empty cells and a
/// whole-column style) are written and read back as runs.
#[test]
fn style_runs_round_trip() {
    let bold = StyleData {
        font: FontData {
            bold: true,
            ..FontData::default()
        },
        ..StyleData::default()
    };
    let italic = StyleData {
        font: FontData {
            italic: true,
            ..FontData::default()
        },
        ..StyleData::default()
    };
    let last = tally_types::MAX_ROWS - 1;
    let runs = vec![
        StyleRun {
            col: 0,
            start_row: 0,
            end_row: last,
            style: 0,
        },
        StyleRun {
            col: 2,
            start_row: 99,
            end_row: 20_099,
            style: 1,
        },
        StyleRun {
            col: 3,
            start_row: 99,
            end_row: 20_099,
            style: 1,
        },
    ];
    let wb = WorkbookData {
        sheets: vec![SheetData {
            name: "S".into(),
            cells: vec![v(0, 1, Value::Number(1.0), None)],
            style_runs: runs.clone(),
            ..SheetData::default()
        }],
        styles: vec![bold, italic],
        ..WorkbookData::default()
    };
    let bytes = write_to_vec(&wb);
    let mut content = String::new();
    zip::ZipArchive::new(Cursor::new(bytes.clone()))
        .unwrap()
        .by_name("content.xml")
        .unwrap()
        .read_to_string(&mut content)
        .unwrap();
    assert!(
        content.contains(r#"table:default-cell-style-name="ce1""#),
        "whole column written as a column default style"
    );
    let back = read_ods(Cursor::new(bytes), &Limits::default()).unwrap();
    let sheet = &back.sheets[0];
    assert_eq!(sheet.cells, wb.sheets[0].cells);
    // Disjoint runs: their order does not matter.
    let mut got = sheet.style_runs.clone();
    got.sort_by_key(|r| (r.col, r.start_row));
    assert_eq!(got, runs);
    assert!(back.styles[got[0].style as usize].font.bold);
    assert!(back.styles[got[1].style as usize].font.italic);
}

/// Name usage flags: `table:range-usable-as` on `table:named-range`, in
/// AOO's token order (`ScXMLExport::WriteNamedExpressions`), read back
/// token by token (`ScXMLImport::GetRangeType`); expressions carry none.
#[test]
fn named_range_usable_as_round_trips() {
    let mut wb = sample();
    let all = NameUsage {
        print_range: true,
        filter: true,
        repeat_row: true,
        repeat_column: true,
    };
    wb.names.push(NameData {
        base_cell: None,
        usage: all,
        name: "Everything".into(),
        sheet: None,
        formula: "of:=[$Orders.$A$1:.$B$4]".into(),
    });
    wb.names.push(NameData {
        base_cell: None,
        usage: NameUsage {
            filter: true,
            ..NameUsage::default()
        },
        name: "Crit".into(),
        sheet: Some(0),
        formula: "of:=[$Orders.$D$1:.$D$2]".into(),
    });
    wb.names.push(NameData {
        base_cell: None,
        usage: all,
        name: "Rate".into(),
        sheet: None,
        formula: "of:=0.07".into(),
    });
    let bytes = write_to_vec(&wb);
    let xml = entry(&bytes, "content.xml");
    assert!(
        xml.contains(r#"table:range-usable-as="repeat-column repeat-row filter print-range""#),
        "{xml}"
    );
    assert!(xml.contains(r#"table:range-usable-as="filter""#), "{xml}");
    assert_eq!(
        xml.matches("range-usable-as").count(),
        2,
        "not on expressions"
    );
    let back = read_ods(Cursor::new(bytes.clone()), &Limits::default()).unwrap();
    let get = |n: &str| back.names.iter().find(|x| x.name == n).unwrap().usage;
    assert_eq!(get("Everything"), all);
    assert!(get("Crit").filter);
    assert!(!get("Crit").print_range);
    assert!(get("Rate").is_empty());
    // Extra whitespace and unknown tokens (`none`) are tolerated.
    let edited = with_content(&bytes, |x| {
        x.replace(
            r#"table:range-usable-as="filter""#,
            r#"table:range-usable-as="  none   print-range  bogus""#,
        )
    });
    let back = read_ods(Cursor::new(edited), &Limits::default()).unwrap();
    let crit = back.names.iter().find(|x| x.name == "Crit").unwrap().usage;
    assert_eq!(
        crit,
        NameUsage {
            print_range: true,
            ..NameUsage::default()
        }
    );
}

/// `[NatNumN]` ↔ `number:transliteration-*`, `[~calendar]` ↔
/// `number:calendar`, era years (xmlnumfe.cxx / xmlnumfi.cxx, G7).
#[test]
fn natnum_and_calendars_round_trip() {
    let same = [
        "[NatNum1][$-411]0",
        "[NatNum4][$-411]#,##0;[NatNum4]-#,##0",
        "[NatNum1][$-41E]0.00",
        "[NatNum1][$-401]#,##0.00",
        "[NatNum2][$-40D]0",
        "[NatNum9][$-412]0",
        "[NatNum3]0",
        "[NatNum1][$-411]YYYY/MM/DD",
        "[$-411]GGGE\"年\"M\"月\"D\"日\"",
        "[$-411]GE.MM.DD",
        "[NatNum1][$-41E][~buddhist]D/M/YYYY",
        "[$-401][~hijri]D MMMM YYYY",
        "[$-40D][~jewish]D MMMM YYYY",
        "[$-409][~jewish]D MMMM YYYY",
        "[$-409]G E",
    ];
    for code in same {
        let bytes = write_to_vec(&one_cell(code));
        let back = read_ods(Cursor::new(bytes.clone()), &Limits::default()).unwrap();
        assert_eq!(
            back.styles[0].number_format.as_deref(),
            Some(code),
            "{}",
            entry(&bytes, "content.xml")
        );
    }
    // The XML AOO writes.
    let xml = entry(&write_to_vec(&one_cell("[NatNum1][$-411]0")), "content.xml");
    assert!(
        xml.contains(r#"number:transliteration-format="一" number:transliteration-language="ja" number:transliteration-country="JP" number:transliteration-style="short""#),
        "{xml}"
    );
    let xml = entry(&write_to_vec(&one_cell("[$-411]GGGE")), "content.xml");
    assert!(
        xml.contains(r#"<number:era number:style="long" number:calendar="gengou"/><number:year number:calendar="gengou"/>"#),
        "{xml}"
    );
    // en-US's other calendar is the Jewish one (en_US.xml), so AOO's
    // lcl_GetDefaultCalendar writes it for E / EE / R / RR.
    let xml = entry(&write_to_vec(&one_cell("[$-409]G E")), "content.xml");
    assert!(
        xml.contains(r#"<number:era number:calendar="jewish"/><number:text> </number:text><number:year number:calendar="jewish"/>"#),
        "{xml}"
    );
    // Codes that read back in AOO's equivalent spelling.
    for (code, back_code) in [
        // DBNum is stored as its NatNum (zformat.cxx GetNatNumXml).
        ("[DBNum1][$-411]0", "[NatNum1][$-411]0"),
        ("[DBNum2][$-411]0", "[NatNum4][$-411]0"),
        // E / EE / R / RR: year elements in the other calendar.
        ("[$-411]RR", "[$-411]GGGEE"),
        ("[$-404][~ROC]GGGE\"年\"", "[$-404]GGGE\"年\""),
        // AAAA: a weekday element in the other calendar.
        ("[$-411]AAAA", "[$-411][~gengou]NNN"),
        // R in a number section is text (letters are quoted on reading).
        ("R #,##0.00", "\"R \"#,##0.00"),
    ] {
        let back = read_ods(
            Cursor::new(write_to_vec(&one_cell(code))),
            &Limits::default(),
        )
        .unwrap();
        assert_eq!(
            back.styles[0].number_format.as_deref(),
            Some(back_code),
            "{code}"
        );
    }
    // AOO files: a transliteration in another language than the style's.
    let bytes = write_to_vec(&one_cell("[$-409]0"));
    let th = with_content(&bytes, |x| {
        x.replace(
            "number:language=\"en\" number:country=\"US\">",
            "number:language=\"en\" number:country=\"US\" number:transliteration-format=\"๑\" number:transliteration-language=\"th\" number:transliteration-country=\"TH\" number:transliteration-style=\"short\">",
        )
    });
    let back = read_ods(Cursor::new(th), &Limits::default()).unwrap();
    assert_eq!(
        back.styles[0].number_format.as_deref(),
        Some("[NatNum1][$-41E]0")
    );
}
