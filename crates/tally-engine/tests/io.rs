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
    let info = svc.open(&repo("tests/fixtures/csv/invoices.csv")).unwrap();
    let s = info.sheets[0].id;
    assert_eq!(info.sheets[0].name, "invoices");
    let wb = &svc.document(info.doc).unwrap().wb;
    // CSV `=` fields import as text (AOO's "Evaluate formulas" is off by
    // default; security review M3), while numbers are still recognised.
    assert_eq!(
        wb.value(CellAddr::new(s, 1, 5)),
        Value::text("=D2*E2"),
        "=D2*E2 stays text"
    );
    assert_eq!(wb.value(CellAddr::new(s, 1, 3)), Value::Number(12.0));
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

/// Correctness review H4: long style runs and whole-column styles survive
/// an ODS save and reload.
#[test]
fn long_style_runs_survive_ods() {
    use tally_api::{Op, RangeDto, StylePatch, Transaction};
    use tally_engine::storage::StyleId;
    use tally_types::MAX_ROWS;
    let mut svc = EngineService::with_default_codec();
    let info = svc.new_workbook(None).unwrap();
    let s = info.sheets[0].id;
    let style = |range: RangeDto, patch: StylePatch| Op::ApplyStyle {
        sheet: s,
        ranges: vec![range],
        patch,
    };
    let italic = StylePatch {
        italic: Some(true),
        ..StylePatch::default()
    };
    let bold = StylePatch {
        bold: Some(true),
        ..StylePatch::default()
    };
    let ops = vec![
        style(
            RangeDto {
                r0: 99,
                c0: 2,
                r1: 20_099,
                c1: 2,
            },
            italic,
        ),
        style(
            RangeDto {
                r0: 0,
                c0: 0,
                r1: MAX_ROWS - 1,
                c1: 0,
            },
            bold,
        ),
        Op::SetCellInput {
            sheet: s,
            r: 0,
            c: 1,
            input: "x".into(),
        },
    ];
    svc.apply(
        info.doc,
        Transaction {
            ops,
            label: None,
            selection_before: None,
            selection_after: None,
        },
    )
    .unwrap();
    let dir = std::env::temp_dir().join(format!("tally-engine-runs-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("runs.ods");
    svc.save(info.doc, Some(&p), FileFormat::Ods).unwrap();
    let back = svc.open(&p).unwrap();
    let before = svc.document(info.doc).unwrap().wb.to_data();
    let wb = &svc.document(back.doc).unwrap().wb;
    let s2 = back.sheets[0].id;
    let st = |r: u32, c: u32| wb.style_at(CellAddr::new(s2, r, c));
    let it = st(5000, 2);
    assert_ne!(it, StyleId::DEFAULT);
    assert!(wb.styles.get(it).data.font.italic);
    for r in [99, 20_099] {
        assert_eq!(st(r, 2), it, "row {r}");
    }
    assert_eq!(st(98, 2), StyleId::DEFAULT);
    assert_eq!(st(20_100, 2), StyleId::DEFAULT);
    let b = st(MAX_ROWS - 1, 0);
    assert!(wb.styles.get(b).data.font.bold);
    assert_eq!(st(0, 0), b);
    assert_eq!(st(500_000, 0), b);
    assert_eq!(st(0, 1), StyleId::DEFAULT);
    let after = wb.to_data();
    assert_eq!(after.sheets[0].style_runs, before.sheets[0].style_runs);
    assert_eq!(after.sheets[0].cells, before.sheets[0].cells);
    let _ = std::fs::remove_dir_all(&dir);
}
