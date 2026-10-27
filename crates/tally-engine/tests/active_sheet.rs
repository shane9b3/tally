//! `set_active_sheet`: the UI reports the sheet the user switched to (view
//! state). It adds no undo step and leaves the document unmodified, and the
//! calls that default to the active sheet follow it: CSV save, Find across
//! sheets, `PrintSheets::Active`, and the active sheet written to ODS / XLSX.
#![allow(clippy::unwrap_used)]
#![cfg(feature = "io")]

use tally_api::{DocId, DocumentService, FileFormat, FindQuery, Op, PrintOptionsDto, Transaction};
use tally_engine::EngineService;
use tally_types::SheetId;

fn tx(ops: Vec<Op>) -> Transaction {
    Transaction {
        ops,
        label: Some("test".into()),
        selection_before: None,
        selection_after: None,
    }
}

fn input(sheet: SheetId, r: u32, c: u32, text: &str) -> Op {
    Op::SetCellInput {
        sheet,
        r,
        c,
        input: text.into(),
    }
}

/// Three sheets; sheet n holds "s<n> …" in A1:A3. The engine's active sheet
/// stays on the first (no transaction names a selection).
fn three_sheets(svc: &mut EngineService) -> (DocId, Vec<SheetId>) {
    let info = svc.new_workbook(None).unwrap();
    let doc = info.doc;
    svc.apply(
        doc,
        tx(vec![
            Op::AddSheet {
                name: Some("Two".into()),
                index: None,
            },
            Op::AddSheet {
                name: Some("Three".into()),
                index: None,
            },
        ]),
    )
    .unwrap();
    let ids: Vec<SheetId> = svc.info(doc).unwrap().sheets.iter().map(|s| s.id).collect();
    let mut ops = Vec::new();
    for (i, &s) in ids.iter().enumerate() {
        for r in 0..3 {
            ops.push(input(s, r, 0, &format!("s{} hit {r}", i + 1)));
        }
    }
    svc.apply(doc, tx(ops)).unwrap();
    assert_eq!(svc.info(doc).unwrap().active_sheet, ids[0]);
    (doc, ids)
}

fn tmp(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("tally-active-sheet-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

#[test]
fn switching_sheets_is_view_state_only() {
    let mut svc = EngineService::with_default_codec();
    let (doc, ids) = three_sheets(&mut svc);
    let path = tmp("state.ods");
    svc.save(doc, Some(&path), FileFormat::Ods).unwrap();
    assert!(!svc.info(doc).unwrap().dirty);
    svc.set_active_sheet(doc, ids[1]).unwrap();
    let info = svc.info(doc).unwrap();
    assert_eq!(info.active_sheet, ids[1]);
    assert!(!info.dirty, "switching sheets does not modify the document");
    // No undo step: undo reverts the last edit (the cell fill), not the switch.
    let undo = svc.undo(doc).unwrap();
    assert_eq!(undo.undo.undo_label.as_deref(), Some("test"));
    assert_eq!(svc.info(doc).unwrap().active_sheet, ids[1]);
    assert_eq!(
        svc.set_active_sheet(doc, 999).unwrap_err().code,
        tally_api::ApiError::NOT_FOUND
    );
    assert!(svc.set_active_sheet(DocId(77), ids[0]).is_err());
}

/// H1: CSV save wrote the stored active sheet (Sheet1) while the user and
/// the CSV dialog were on Sheet2.
#[test]
fn csv_save_writes_the_sheet_the_user_is_on() {
    let mut svc = EngineService::with_default_codec();
    let (doc, ids) = three_sheets(&mut svc);
    svc.set_active_sheet(doc, ids[1]).unwrap();
    let path = tmp("two.csv");
    svc.save(doc, Some(&path), FileFormat::Csv).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("s2 hit 0"), "{text}");
    assert!(!text.contains("s1 hit"), "{text}");
}

/// The file reopens on the sheet the user was on (ODS and XLSX view state).
#[test]
fn ods_and_xlsx_reopen_on_the_users_sheet() {
    for (name, format) in [
        ("view.ods", FileFormat::Ods),
        ("view.xlsx", FileFormat::Xlsx),
    ] {
        let mut svc = EngineService::with_default_codec();
        let (doc, ids) = three_sheets(&mut svc);
        svc.set_active_sheet(doc, ids[2]).unwrap();
        let path = tmp(name);
        svc.save(doc, Some(&path), format).unwrap();
        let back = svc.open(&path).unwrap();
        let tab = back.sheets.iter().position(|s| s.id == back.active_sheet);
        assert_eq!(tab, Some(2), "{name}");
    }
}

/// M4: Find with All sheets rotated the hits from the stale active sheet.
#[test]
fn find_all_sheets_starts_on_the_users_sheet() {
    let mut svc = EngineService::with_default_codec();
    let (doc, ids) = three_sheets(&mut svc);
    svc.set_active_sheet(doc, ids[1]).unwrap();
    let q = FindQuery {
        text: "hit".into(),
        sheet: None,
        in_formulas: false,
        match_case: false,
        whole_cell: false,
        mode: tally_api::FindMode::Literal,
        from: Some(tally_api::CellPos { r: 0, c: 0 }),
        in_notes: None,
        selection: None,
    };
    let res = svc.find(doc, &q).unwrap();
    let first: Vec<(SheetId, u32)> = res.hits.iter().take(3).map(|h| (h.sheet, h.r)).collect();
    assert_eq!(first, vec![(ids[1], 1), (ids[1], 2), (ids[2], 0)]);
    assert_eq!(res.hits.len(), 9);
}

/// `PrintSheets::Active` without an explicit sheet follows it too.
#[test]
fn print_active_follows_it() {
    let mut svc = EngineService::with_default_codec();
    let (doc, ids) = three_sheets(&mut svc);
    svc.set_active_sheet(doc, ids[2]).unwrap();
    let l = svc.print_layout(doc, &PrintOptionsDto::default()).unwrap();
    assert!(!l.pages.is_empty());
    assert!(l.pages.iter().all(|p| p.sheet == ids[2]));
}
