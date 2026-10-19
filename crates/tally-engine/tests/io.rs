//! File round trips through the IO crates (`--features io`).
#![allow(clippy::unwrap_used)]
#![cfg(feature = "io")]

use std::path::Path;

use tally_api::{DocumentService, FileFormat};
use tally_engine::EngineService;
use tally_types::{CellAddr, Value};

fn repo(p: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(p)
}

#[test]
fn csv_open_ods_round_trip() {
    let mut svc = EngineService::with_default_codec();
    let info = svc
        .open(&repo("apps/desktop/src-tauri/templates/invoices.csv"))
        .unwrap();
    let s = info.sheets[0].id;
    assert_eq!(info.sheets[0].name, "invoices");
    let wb = &svc.document(info.doc).unwrap().wb;
    assert_eq!(
        wb.value(CellAddr::new(s, 1, 5)),
        Value::Number(54.0),
        "=D2*E2 recognised and computed"
    );
    let before = wb.to_data();
    let dir = std::env::temp_dir().join(format!("tally-engine-io-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let ods = dir.join("invoices.ods");
    svc.save(info.doc, Some(&ods), FileFormat::Ods).unwrap();
    let again = svc.open(&ods).unwrap();
    let mut after = svc.document(again.doc).unwrap().wb.to_data();
    after.meta = before.meta.clone();
    assert_eq!(after.sheets[0].cells, before.sheets[0].cells);
    let csv = dir.join("out.csv");
    svc.save(again.doc, Some(&csv), FileFormat::Csv).unwrap();
    let text = std::fs::read_to_string(&csv).unwrap();
    assert!(text.starts_with("Invoice,Customer,Date"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn templates_save_as_ods() {
    let mut svc = EngineService::with_default_codec();
    let info = svc.new_workbook(Some("invoices")).unwrap();
    let dir = std::env::temp_dir().join(format!("tally-engine-tpl-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("t.ods");
    svc.save(info.doc, Some(&p), FileFormat::Ods).unwrap();
    let back = svc.open(&p).unwrap();
    let a = svc.document(info.doc).unwrap().wb.to_data();
    let mut b = svc.document(back.doc).unwrap().wb.to_data();
    b.meta = a.meta.clone();
    assert_eq!(a.sheets[0].cells, b.sheets[0].cells);
    assert_eq!(a.styles.len(), b.styles.len());
    // Number format codes may be respelled by the ODS writer ([$$-409] → $);
    // the displayed text must be identical.
    let s1 = info.sheets[0].id;
    let s2 = back.sheets[0].id;
    let r = tally_api::RangeDto {
        r0: 0,
        c0: 0,
        r1: 14,
        c1: 6,
    };
    let t1: Vec<String> = svc.cells(info.doc, s1, &[r]).unwrap()[0]
        .cells
        .iter()
        .map(|c| c.text.clone())
        .collect();
    let t2: Vec<String> = svc.cells(back.doc, s2, &[r]).unwrap()[0]
        .cells
        .iter()
        .map(|c| c.text.clone())
        .collect();
    assert_eq!(t1, t2);
    let _ = std::fs::remove_dir_all(&dir);
}
