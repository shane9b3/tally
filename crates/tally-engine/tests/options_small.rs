//! Small AOO 4.1 document options: the two-digit year window (P17), the
//! General format's decimals ("Limit decimals for general number format")
//! and the view zoom (U18, 20–400 %), with their file round trips.
#![allow(clippy::unwrap_used)]
#![cfg(feature = "io")]

use tally_api::{CalcSettingsDto, DocId, DocumentService, FileFormat, Op, Transaction};
use tally_engine::EngineService;
use tally_types::{SheetId, Value};

fn tx(ops: Vec<Op>) -> Transaction {
    Transaction {
        ops,
        label: Some("test".into()),
        selection_before: None,
        selection_after: None,
        group: None,
    }
}

fn set(svc: &mut EngineService, doc: DocId, sheet: SheetId, r: u32, c: u32, text: &str) {
    let res = svc
        .apply(
            doc,
            tx(vec![Op::SetCellInput {
                sheet,
                r,
                c,
                input: text.into(),
            }]),
        )
        .unwrap();
    assert!(res.errors.is_empty(), "{:?}", res.errors);
}

fn value(svc: &EngineService, doc: DocId, sheet: SheetId, r: u32, c: u32) -> Value {
    svc.document(doc)
        .unwrap()
        .wb
        .value(tally_types::CellAddr::new(sheet, r, c))
}

fn text(svc: &EngineService, doc: DocId, sheet: SheetId, r: u32, c: u32) -> String {
    let b = svc
        .cells(
            doc,
            sheet,
            &[tally_api::RangeDto {
                r0: r,
                c0: c,
                r1: r,
                c1: c,
            }],
        )
        .unwrap();
    b[0].cells
        .first()
        .map(|c| c.text.clone())
        .unwrap_or_default()
}

fn new_doc(svc: &mut EngineService) -> (DocId, SheetId) {
    let info = svc.new_workbook(None).unwrap();
    (info.doc, info.sheets[0].id)
}

/// The year typed `input` lands in, read back with `=YEAR()`.
fn typed_year(svc: &mut EngineService, doc: DocId, sheet: SheetId, input: &str) -> f64 {
    set(svc, doc, sheet, 0, 0, input);
    set(svc, doc, sheet, 0, 1, "=YEAR(A1)");
    match value(svc, doc, sheet, 0, 1) {
        Value::Number(y) => y,
        v => panic!("{input}: {v:?}"),
    }
}

fn settings(svc: &EngineService, doc: DocId) -> CalcSettingsDto {
    svc.calc_settings(doc).unwrap()
}

fn tmp(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("tally-options-small-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

#[test]
fn year_window_defaults_to_1930() {
    let mut svc = EngineService::with_default_codec();
    let (doc, s) = new_doc(&mut svc);
    assert_eq!(settings(&svc, doc).two_digit_year, Some(1930));
    assert_eq!(typed_year(&mut svc, doc, s, "1/1/45"), 1945.0);
    assert_eq!(typed_year(&mut svc, doc, s, "1/1/29"), 2029.0);
    assert_eq!(typed_year(&mut svc, doc, s, "1/1/30"), 1930.0);
}

#[test]
fn application_window_applies_to_new_documents_only() {
    let mut svc = EngineService::with_default_codec();
    let (old, old_s) = new_doc(&mut svc);
    svc.set_two_digit_year_default(1950).unwrap();
    let (doc, s) = new_doc(&mut svc);
    assert_eq!(settings(&svc, doc).two_digit_year, Some(1950));
    assert_eq!(typed_year(&mut svc, doc, s, "1/1/45"), 2045.0);
    assert_eq!(typed_year(&mut svc, doc, s, "1/1/50"), 1950.0);
    assert_eq!(typed_year(&mut svc, doc, s, "1/1/49"), 2049.0);
    // A document already open keeps its window (AOO copies the option
    // when a document is created or loaded).
    assert_eq!(typed_year(&mut svc, old, old_s, "1/1/45"), 1945.0);
    // Forks share the option.
    let mut fork = svc.fork();
    let (fd, fs) = new_doc(&mut fork);
    assert_eq!(typed_year(&mut fork, fd, fs, "1/1/45"), 2045.0);
    svc.set_two_digit_year_default(2000).unwrap();
    let (doc, s) = new_doc(&mut svc);
    assert_eq!(typed_year(&mut svc, doc, s, "1/1/45"), 2045.0);
    assert_eq!(typed_year(&mut svc, doc, s, "1/1/99"), 2099.0);
    assert_eq!(typed_year(&mut svc, doc, s, "1/1/00"), 2000.0);
    // AOO's field allows 1583–9857.
    assert!(svc.set_two_digit_year_default(1582).is_err());
    assert!(svc.set_two_digit_year_default(9858).is_err());
}

#[test]
fn functions_follow_the_document_window() {
    let mut svc = EngineService::with_default_codec();
    svc.set_two_digit_year_default(1950).unwrap();
    let (doc, s) = new_doc(&mut svc);
    set(&mut svc, doc, s, 0, 0, r#"=YEAR(DATEVALUE("1/1/45"))"#);
    set(&mut svc, doc, s, 1, 0, "=YEAR(DATE(45;1;1))");
    set(&mut svc, doc, s, 2, 0, "=YEAR(DATE(50;1;1))");
    set(&mut svc, doc, s, 3, 0, r#"=YEAR(VALUE("1/1/45"))"#);
    set(&mut svc, doc, s, 4, 0, "=EASTERSUNDAY(45)");
    set(&mut svc, doc, s, 5, 0, "=EASTERSUNDAY(2045)");
    for r in 0..2 {
        assert_eq!(value(&svc, doc, s, r, 0), Value::Number(2045.0), "row {r}");
    }
    assert_eq!(value(&svc, doc, s, 2, 0), Value::Number(1950.0));
    assert_eq!(value(&svc, doc, s, 3, 0), Value::Number(2045.0));
    assert_eq!(value(&svc, doc, s, 4, 0), value(&svc, doc, s, 5, 0));
    // Changing the document's window (SetCalcSettings) recalculates.
    let mut c = settings(&svc, doc);
    c.two_digit_year = Some(1930);
    svc.apply(doc, tx(vec![Op::SetCalcSettings { settings: c }]))
        .unwrap();
    assert_eq!(value(&svc, doc, s, 0, 0), Value::Number(1945.0));
    assert_eq!(value(&svc, doc, s, 1, 0), Value::Number(1945.0));
    // A client that omits the window keeps it.
    let mut c = settings(&svc, doc);
    c.two_digit_year = None;
    c.case_sensitive = true;
    svc.apply(doc, tx(vec![Op::SetCalcSettings { settings: c }]))
        .unwrap();
    assert_eq!(settings(&svc, doc).two_digit_year, Some(1930));
    // Out of range is refused.
    let mut c = settings(&svc, doc);
    c.two_digit_year = Some(1000);
    let res = svc.apply(doc, tx(vec![Op::SetCalcSettings { settings: c }]));
    assert!(res.map(|r| !r.errors.is_empty()).unwrap_or(true));
}

#[test]
fn text_to_columns_and_csv_use_the_window() {
    let mut svc = EngineService::with_default_codec();
    svc.set_two_digit_year_default(1950).unwrap();
    // CSV import: recognised while loading, with the application's window.
    let path = tmp("dates.csv");
    std::fs::write(&path, "1/1/45\n1/1/50\n").unwrap();
    let info = svc.open(&path).unwrap();
    let s = info.sheets[0].id;
    let doc = info.doc;
    set(&mut svc, doc, s, 0, 1, "=YEAR(A1)");
    set(&mut svc, doc, s, 1, 1, "=YEAR(A2)");
    assert_eq!(value(&svc, doc, s, 0, 1), Value::Number(2045.0));
    assert_eq!(value(&svc, doc, s, 1, 1), Value::Number(1950.0));
    // Data ▸ Text to Columns with a D/M/Y column type.
    let (doc, s) = new_doc(&mut svc);
    set(&mut svc, doc, s, 0, 0, "'31.12.45;x");
    let options = tally_api::CsvImportOptionsDto {
        delimiters: ";".into(),
        column_types: vec![tally_api::CsvColumnType::DateDmy],
        ..Default::default()
    };
    let range = tally_api::RangeDto {
        r0: 0,
        c0: 0,
        r1: 0,
        c1: 0,
    };
    let res = svc
        .apply(
            doc,
            tx(vec![Op::TextToColumns {
                sheet: s,
                range,
                options,
            }]),
        )
        .unwrap();
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    set(&mut svc, doc, s, 1, 0, "=YEAR(A1)");
    assert_eq!(value(&svc, doc, s, 1, 0), Value::Number(2045.0));
}

#[test]
fn ods_keeps_the_document_window() {
    let mut svc = EngineService::with_default_codec();
    svc.set_two_digit_year_default(1950).unwrap();
    let (doc, s) = new_doc(&mut svc);
    set(&mut svc, doc, s, 0, 0, "x");
    let path = tmp("window.ods");
    svc.save(doc, Some(&path), FileFormat::Ods).unwrap();
    // Opened where the application says 2000: the file's 1950 wins.
    svc.set_two_digit_year_default(2000).unwrap();
    let back = svc.open(&path).unwrap();
    assert_eq!(settings(&svc, back.doc).two_digit_year, Some(1950));
    assert_eq!(
        typed_year(&mut svc, back.doc, back.sheets[0].id, "1/1/45"),
        2045.0
    );
    // XLSX has no such setting: the application's window applies.
    let x = tmp("window.xlsx");
    svc.save(doc, Some(&x), FileFormat::Xlsx).unwrap();
    let back = svc.open(&x).unwrap();
    assert_eq!(settings(&svc, back.doc).two_digit_year, Some(2000));
}

#[test]
fn general_decimals_limit_display_and_round_trip() {
    let mut svc = EngineService::with_default_codec();
    let (doc, s) = new_doc(&mut svc);
    set(&mut svc, doc, s, 0, 0, "=PI()");
    set(&mut svc, doc, s, 1, 0, "1.5");
    set(&mut svc, doc, s, 2, 0, "=A1*100");
    set(&mut svc, doc, s, 3, 0, "2");
    assert_eq!(text(&svc, doc, s, 0, 0), "3.14159265358979");
    let mut c = settings(&svc, doc);
    assert_eq!(c.standard_decimals, None);
    c.standard_decimals = Some(2);
    svc.apply(
        doc,
        tx(vec![Op::SetCalcSettings {
            settings: c.clone(),
        }]),
    )
    .unwrap();
    assert_eq!(text(&svc, doc, s, 0, 0), "3.14");
    // TEXT(x; "General") follows the limit (AOO formats with the document's formatter) …
    set(&mut svc, doc, s, 4, 0, r#"=TEXT(A1;"General")"#);
    assert_eq!(value(&svc, doc, s, 4, 0), Value::text("3.14"));
    // … and so does HTML export.
    let html = tally_io_html::write_html(&svc.document(doc).unwrap().wb.to_data()).unwrap();
    assert!(html.contains(">3.14<"), "{html}");
    assert_eq!(text(&svc, doc, s, 1, 0), "1.5");
    assert_eq!(text(&svc, doc, s, 3, 0), "2");
    // Only the display changes …
    assert_eq!(
        value(&svc, doc, s, 2, 0),
        Value::Number(std::f64::consts::PI * 100.0)
    );
    // … unless "precision as shown" is on (AOO RoundValueAsShown: General
    // rounds to the limited decimals).
    c.precision_as_shown = true;
    svc.apply(
        doc,
        tx(vec![Op::SetCalcSettings {
            settings: c.clone(),
        }]),
    )
    .unwrap();
    assert_eq!(value(&svc, doc, s, 2, 0), Value::Number(314.0));
    // A fixed format is not affected.
    c.precision_as_shown = false;
    c.standard_decimals = Some(0);
    svc.apply(doc, tx(vec![Op::SetCalcSettings { settings: c }]))
        .unwrap();
    assert_eq!(text(&svc, doc, s, 0, 0), "3");
    // ODS keeps it (default cell style `style:decimal-places`).
    let path = tmp("decimals.ods");
    svc.save(doc, Some(&path), FileFormat::Ods).unwrap();
    let back = svc.open(&path).unwrap();
    assert_eq!(settings(&svc, back.doc).standard_decimals, Some(0));
    assert_eq!(text(&svc, back.doc, back.sheets[0].id, 0, 0), "3");
    // 21 is refused (AOO's field allows 0–20).
    let mut c = settings(&svc, doc);
    c.standard_decimals = Some(21);
    let res = svc.apply(doc, tx(vec![Op::SetCalcSettings { settings: c }]));
    assert!(res.map(|r| !r.errors.is_empty()).unwrap_or(true));
}

#[test]
fn view_zoom_is_view_state_saved_with_the_file() {
    let mut svc = EngineService::with_default_codec();
    let (doc, s) = new_doc(&mut svc);
    svc.apply(
        doc,
        tx(vec![Op::AddSheet {
            name: Some("Two".into()),
            index: None,
        }]),
    )
    .unwrap();
    let ids: Vec<SheetId> = svc.info(doc).unwrap().sheets.iter().map(|s| s.id).collect();
    set(&mut svc, doc, s, 0, 0, "x");
    let path = tmp("zoom-clean.ods");
    svc.save(doc, Some(&path), FileFormat::Ods).unwrap();
    assert!(!svc.is_dirty(doc).unwrap());
    assert_eq!(svc.view_options(doc, s).unwrap().zoom, None);
    svc.set_view_zoom(doc, 20).unwrap();
    // Every sheet (AOO's "synchronize sheets"), no undo step, not modified.
    for &id in &ids {
        assert_eq!(svc.view_options(doc, id).unwrap().zoom, Some(20));
    }
    assert!(!svc.is_dirty(doc).unwrap());
    assert!(svc.set_view_zoom(doc, 19).is_err());
    assert!(svc.set_view_zoom(doc, 401).is_err());
    svc.set_view_zoom(doc, 400).unwrap();
    for (format, name) in [
        (FileFormat::Ods, "zoom.ods"),
        (FileFormat::Xlsx, "zoom.xlsx"),
    ] {
        let p = tmp(name);
        svc.save(doc, Some(&p), format).unwrap();
        let back = svc.open(&p).unwrap();
        for sh in &back.sheets {
            assert_eq!(
                svc.view_options(back.doc, sh.id).unwrap().zoom,
                Some(400),
                "{name}"
            );
        }
    }
    svc.set_view_zoom(doc, 100).unwrap();
    assert_eq!(svc.view_options(doc, s).unwrap().zoom, None);
}
