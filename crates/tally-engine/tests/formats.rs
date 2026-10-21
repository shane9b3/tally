//! The new file formats through the service (wave 5, W5-3): XLS, SXC,
//! HTML and dBase import, HTML and dBase export, sniffing by magic bytes,
//! and a malicious corpus for each reader.
#![allow(clippy::unwrap_used)]
#![cfg(feature = "io")]

use std::path::Path;

use tally_api::{DocumentService, FileFormat};
use tally_engine::EngineService;
use tally_types::{CellAddr, ErrorKind, Value};

fn repo(p: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(p)
}

fn tmp(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("tally-formats-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

#[test]
fn xls_fixture_opens_with_values_and_formulas() {
    let mut svc = EngineService::with_default_codec();
    let info = svc.open(&repo("tests/fixtures/xls/basic.xls")).unwrap();
    assert_eq!(info.sheets.len(), 2);
    let s = info.sheets[0].id;
    let wb = &svc.document(info.doc).unwrap().wb;
    assert_eq!(wb.value(CellAddr::new(s, 0, 0)), Value::Number(1.0));
    assert_eq!(wb.value(CellAddr::new(s, 1, 1)), Value::text("Zürich"));
    assert_eq!(wb.value(CellAddr::new(s, 3, 0)), Value::Number(3.0));
    assert_eq!(wb.value(CellAddr::new(s, 3, 1)), Value::Number(3.0));
    assert_eq!(
        wb.value(CellAddr::new(s, 2, 1)),
        Value::Error(ErrorKind::Div0)
    );
    let edit = svc.cell_edit_info(info.doc, s, 3, 1).unwrap();
    assert_eq!(edit.input, "=SUM(A1:A2)");
    // Sniffed without the extension.
    let copy = tmp("noext");
    std::fs::copy(repo("tests/fixtures/xls/basic.xls"), &copy).unwrap();
    assert!(svc.open(&copy).is_ok());
}

#[test]
fn sxc_fixture_opens_with_values_and_formulas() {
    let mut svc = EngineService::with_default_codec();
    let info = svc.open(&repo("tests/fixtures/sxc/ooo1.sxc")).unwrap();
    assert_eq!(info.sheets.len(), 2);
    let s = info.sheets[0].id;
    let wb = &svc.document(info.doc).unwrap().wb;
    assert_eq!(wb.value(CellAddr::new(s, 0, 0)), Value::Number(1.0));
    assert_eq!(
        wb.value(CellAddr::new(s, 1, 0)),
        Value::Number(3.0),
        "=SUM(A1:B1)"
    );
    assert_eq!(
        wb.value(CellAddr::new(s, 1, 1)),
        Value::Number(2.0),
        "=A1*2"
    );
    assert_eq!(
        wb.value(CellAddr::new(s, 1, 2)),
        Value::text("x!"),
        "=Sheet2.A1&\"!\""
    );
    assert_eq!(wb.value(CellAddr::new(s, 2, 1)), Value::Bool(true));
    assert!(
        matches!(wb.value(CellAddr::new(s, 2, 0)), Value::Number(_)),
        "date"
    );
    let edit = svc.cell_edit_info(info.doc, s, 1, 0).unwrap();
    assert_eq!(edit.input, "=SUM(A1:B1)");
    let st = svc
        .styles(info.doc, &[wb.style_at(CellAddr::new(s, 0, 2)).0])
        .unwrap();
    assert!(st[0].bold, "style:properties bold");
    let layout = svc.sheet_layout(info.doc, s).unwrap();
    assert!(
        layout.col_runs.iter().any(|r| (r.size - 132.3).abs() < 1.0),
        "3.5 cm columns: {:?}",
        layout.col_runs
    );
    assert!(svc
        .names(info.doc)
        .unwrap()
        .iter()
        .any(|n| n.name == "First"));
}

#[test]
fn html_import_and_export_round_trip() {
    let mut svc = EngineService::with_default_codec();
    let info = svc
        .open(&repo("tests/fixtures/html/two-tables.html"))
        .unwrap();
    assert_eq!(info.sheets.len(), 2, "{:?}", info.sheets);
    assert_eq!(info.sheets[0].name, "Sales");
    let s = info.sheets[0].id;
    let wb = &svc.document(info.doc).unwrap().wb;
    assert_eq!(
        wb.value(CellAddr::new(s, 1, 1)),
        Value::Number(1234.5),
        "recognized number"
    );
    assert_eq!(wb.value(CellAddr::new(s, 2, 1)), Value::Number(-17.25));
    assert!(
        matches!(wb.value(CellAddr::new(s, 2, 2)), Value::Number(_)),
        "date recognized"
    );
    assert_eq!(
        wb.value(CellAddr::new(s, 2, 0)),
        Value::text("South & East")
    );
    let s2 = info.sheets[1].id;
    let wb = &svc.document(info.doc).unwrap().wb;
    assert_eq!(
        wb.value(CellAddr::new(s2, 0, 1)),
        Value::text("=1+1"),
        "formulas stay text"
    );
    assert_eq!(wb.value(CellAddr::new(s2, 0, 0)), Value::text("A\nB"));
    assert!(
        wb.sheets.iter().all(|sh| sh.cell_count() < 20),
        "no script content"
    );
    // Export → import keeps the values.
    let path = tmp("out.html");
    svc.save(info.doc, Some(&path), FileFormat::Html).unwrap();
    let html = std::fs::read_to_string(&path).unwrap();
    assert!(html.contains("<table>") && html.contains("South &amp; East"));
    let again = svc.open(&path).unwrap();
    let wb = &svc.document(again.doc).unwrap().wb;
    let t = again.sheets[0].id;
    assert_eq!(wb.value(CellAddr::new(t, 1, 1)), Value::Number(1234.5));
    assert_eq!(
        wb.value(CellAddr::new(t, 2, 0)),
        Value::text("South & East")
    );
}

#[test]
fn dbf_import_and_export() {
    let mut svc = EngineService::with_default_codec();
    let info = svc.open(&repo("tests/fixtures/dbf/customers.dbf")).unwrap();
    let s = info.sheets[0].id;
    assert_eq!(info.sheets[0].name, "customers");
    let wb = &svc.document(info.doc).unwrap().wb;
    // AOO's header descriptors (correctness review 4, M35).
    match wb.value(CellAddr::new(s, 0, 0)) {
        Value::Text(t) => assert!(t.starts_with("NAME,C,"), "{t}"),
        v => panic!("{v:?}"),
    }
    assert_eq!(wb.value(CellAddr::new(s, 1, 0)), Value::text("Müller & Co"));
    assert_eq!(wb.value(CellAddr::new(s, 1, 1)), Value::Number(1234.5));
    assert_eq!(wb.value(CellAddr::new(s, 2, 1)), Value::Number(-7.25));
    assert_eq!(wb.value(CellAddr::new(s, 1, 3)), Value::Bool(true));
    assert_eq!(
        wb.value(CellAddr::new(s, 3, 0)),
        Value::Empty,
        "deleted record skipped"
    );
    let text = svc
        .cells(
            info.doc,
            s,
            &[tally_api::RangeDto {
                r0: 1,
                c0: 2,
                r1: 1,
                c1: 2,
            }],
        )
        .unwrap()[0]
        .cells[0]
        .text
        .clone();
    assert_eq!(text, "2026-10-20");
    let path = tmp("out.dbf");
    svc.save(info.doc, Some(&path), FileFormat::Dbf).unwrap();
    let again = svc.open(&path).unwrap();
    let wb = &svc.document(again.doc).unwrap().wb;
    let t = again.sheets[0].id;
    assert_eq!(wb.value(CellAddr::new(t, 1, 0)), Value::text("Müller & Co"));
    assert_eq!(wb.value(CellAddr::new(t, 1, 1)), Value::Number(1234.5));
}

#[test]
fn malicious_corpus_for_new_readers() {
    let mut svc = EngineService::with_default_codec();
    let sources = [
        "tests/fixtures/xls/basic.xls",
        "tests/fixtures/sxc/ooo1.sxc",
        "tests/fixtures/dbf/customers.dbf",
        "tests/fixtures/html/two-tables.html",
    ];
    for src in sources {
        let bytes = std::fs::read(repo(src)).unwrap();
        let ext = Path::new(src).extension().unwrap().to_str().unwrap();
        for (i, step) in [(0usize, 11usize), (5, 23), (2, 41)] {
            let mut m = bytes.clone();
            let mut j = i;
            while j < m.len() {
                m[j] = m[j].wrapping_add(0x33);
                j += step;
            }
            let p = tmp(&format!("mut-{i}.{ext}"));
            std::fs::write(&p, &m).unwrap();
            let _ = svc.open(&p);
        }
        for cut in [1usize, 17, 64, bytes.len() / 2] {
            let p = tmp(&format!("cut-{cut}.{ext}"));
            std::fs::write(&p, &bytes[..cut.min(bytes.len())]).unwrap();
            let _ = svc.open(&p);
        }
    }
    // A 'script-only' HTML has no table.
    let p = tmp("none.html");
    std::fs::write(&p, "<html><script>let t='<table>'</script></html>").unwrap();
    assert!(svc.open(&p).is_err());
    // An HTML with a billion cells claimed via colspan is clamped.
    let p = tmp("span.html");
    std::fs::write(
        &p,
        "<table><tr><td colspan=\"999999999\" rowspan=\"999999999\">x</td></tr></table>",
    )
    .unwrap();
    assert!(svc.open(&p).is_ok());
}
