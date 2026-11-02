//! Read-only documents (AOO's read-only medium and Edit ▸ Edit Mode): the
//! engine itself refuses every edit, not only the UI (`--features io`).
#![allow(clippy::unwrap_used)]
#![cfg(feature = "io")]

use std::path::{Path, PathBuf};

use tally_api::{ApiError, DocumentService, FileFormat, Op, Transaction};
use tally_engine::EngineService;

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.targets/scratch/engine-read-only")
        .join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn set(sheet: u32, input: &str) -> Transaction {
    Transaction {
        ops: vec![Op::SetCellInput {
            sheet,
            r: 0,
            c: 0,
            input: input.into(),
        }],
        label: None,
        selection_before: None,
        selection_after: None,
        group: None,
    }
}

#[test]
fn a_read_only_document_refuses_every_edit() {
    let mut svc = EngineService::with_default_codec();
    let info = svc.new_workbook(None).unwrap();
    let doc = info.doc;
    let sheet = info.sheets[0].id;
    assert!(!info.read_only);
    svc.apply(doc, set(sheet, "1")).unwrap();
    // Not with unsaved changes (AOO saves or discards first).
    let e = svc.set_read_only(doc, true).unwrap_err();
    assert_eq!(e.code, ApiError::INVALID_ARGUMENT);
    let dir = scratch("refuse");
    let own = dir.join("Book.ods");
    svc.save(doc, Some(&own), FileFormat::Ods).unwrap();
    let ro = svc.set_read_only(doc, true).unwrap();
    assert!(ro.read_only);
    let version = ro.version;

    for e in [
        svc.apply(doc, set(sheet, "2")).unwrap_err(),
        svc.undo(doc).unwrap_err(),
        svc.redo(doc).unwrap_err(),
        svc.set_locale(doc, "de-DE").unwrap_err(),
    ] {
        assert_eq!(e.code, ApiError::READ_ONLY);
        assert_eq!(e.message, "Document opened in read-only mode.");
    }
    let info = svc.info(doc).unwrap();
    assert_eq!(info.version, version, "nothing changed");
    assert!(!info.dirty);
    // Save to its own file is refused, by path or as Save (no path).
    assert_eq!(
        svc.save(doc, None, FileFormat::Ods).unwrap_err().code,
        ApiError::READ_ONLY
    );
    assert_eq!(
        svc.save(doc, Some(&own), FileFormat::Ods).unwrap_err().code,
        ApiError::READ_ONLY
    );
    // Viewing still works.
    svc.set_active_sheet(doc, sheet).unwrap();
    svc.recalc_all(doc).unwrap();

    // Save As to another file: the copy is editable.
    let copy = dir.join("Copy.ods");
    let saved = svc.save(doc, Some(&copy), FileFormat::Ods).unwrap();
    assert!(!saved.read_only);
    assert_eq!(saved.title, "Copy.ods");
    svc.apply(doc, set(sheet, "3")).unwrap();
}

#[test]
fn edit_mode_switches_back_and_forth() {
    let mut svc = EngineService::with_default_codec();
    let info = svc.new_workbook(None).unwrap();
    let (doc, sheet) = (info.doc, info.sheets[0].id);
    svc.set_read_only(doc, true).unwrap();
    assert!(svc.apply(doc, set(sheet, "1")).is_err());
    let back = svc.set_read_only(doc, false).unwrap();
    assert!(!back.read_only);
    let r = svc.apply(doc, set(sheet, "1")).unwrap();
    assert!(r.errors.is_empty());
}

#[test]
fn load_readonly_opens_read_only_and_round_trips() {
    let mut svc = EngineService::with_default_codec();
    let info = svc.new_workbook(None).unwrap();
    let doc = info.doc;
    svc.document_mut(doc).unwrap().wb.load_readonly = true;
    let dir = scratch("loadro");
    let path = dir.join("Flagged.ods");
    svc.save(doc, Some(&path), FileFormat::Ods).unwrap();

    let opened = svc.open(&path).unwrap();
    assert!(opened.read_only, "LoadReadonly opens read-only");
    assert!(opened.load_readonly);
    let sheet = opened.sheets[0].id;
    assert_eq!(
        svc.apply(opened.doc, set(sheet, "x")).unwrap_err().code,
        ApiError::READ_ONLY
    );
    // Edit Mode, then a save keeps the flag in the file.
    svc.set_read_only(opened.doc, false).unwrap();
    svc.apply(opened.doc, set(sheet, "x")).unwrap();
    svc.save(opened.doc, None, FileFormat::Ods).unwrap();
    let again = svc.open(&path).unwrap();
    assert!(again.load_readonly && again.read_only);

    // A copy (recovery, Open Copy) is editable but keeps the flag.
    let copy = svc.open_untitled(&path, false).unwrap();
    assert!(!copy.read_only);
    assert!(copy.load_readonly);
    assert_eq!(copy.path, None);
}

#[test]
fn reload_reads_the_file_again_into_the_same_document() {
    let mut svc = EngineService::with_default_codec();
    let info = svc.new_workbook(None).unwrap();
    let (doc, sheet) = (info.doc, info.sheets[0].id);
    svc.apply(doc, set(sheet, "on disk")).unwrap();
    let dir = scratch("reload");
    let path = dir.join("Book.ods");
    svc.save(doc, Some(&path), FileFormat::Ods).unwrap();
    let r = svc.apply(doc, set(sheet, "unsaved")).unwrap();
    let before = r.version;
    let back = svc.reload(doc).unwrap();
    assert_eq!(back.doc, doc);
    assert!(!back.dirty);
    assert!(back.version > before, "versions only grow");
    let wb = &svc.document(doc).unwrap().wb;
    let s = wb.sheets[0].id;
    assert_eq!(
        wb.value(tally_types::CellAddr::new(s, 0, 0)),
        tally_types::Value::text("on disk")
    );
    // An untitled copy has no path, is editable and cannot be reloaded.
    svc.set_read_only(doc, true).unwrap();
    let copy = svc.detach_path(doc).unwrap();
    assert!(copy.path.is_none() && !copy.read_only);
    assert_eq!(copy.title, "Untitled");
    assert!(svc.reload(doc).is_err());
}
