//! End-to-end tests of the command layer over the real engine
//! (`EngineService` with ODS/CSV IO and the ⌘K rule provider), without a
//! Tauri runtime: each test drives the same inner functions the
//! `#[tauri::command]`s call.

use std::path::PathBuf;

use tally_api::{
    ApiError, CellPos, CellView, DocId, FileFormat, Op, RangeDto, SelectionDto, Transaction,
    ValueKind, WorkbookInfo,
};
use tally_types::SheetId;

use crate::assist::build_service;
use crate::commands::{
    new_workbook, open_path, save_to, with_doc, with_engine, with_new_doc, CmdResult,
};
use crate::files;
use crate::state::AppState;

const SUMIF: &str = "=SUMIF(G2:G15,\"<>Paid\",F2:F15)";

fn state() -> AppState {
    AppState::new(build_service())
}

/// A fresh path under `.targets/scratch` (saves now write lock files next
/// to the document, so the tests keep out of the system temp dir).
fn temp_path(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../.targets/scratch/desktop-tests");
    let _ = std::fs::create_dir_all(&dir);
    dir.join(format!(
        "tally-desktop-{}-{nanos}-{name}",
        std::process::id()
    ))
}

fn range(r0: u32, c0: u32, r1: u32, c1: u32) -> RangeDto {
    RangeDto { r0, c0, r1, c1 }
}

fn cells(st: &AppState, doc: DocId, sheet: SheetId, r: RangeDto) -> CmdResult<Vec<CellView>> {
    let blocks = with_doc(st, doc, |s| s.cells(doc, sheet, &[r]))?;
    Ok(blocks.into_iter().flat_map(|b| b.cells).collect())
}

fn cell(cells: &[CellView], r: u32, c: u32) -> Option<&CellView> {
    cells.iter().find(|v| v.r == r && v.c == c)
}

fn set(st: &AppState, info: &WorkbookInfo, r: u32, c: u32, input: &str) -> CmdResult<()> {
    let tx = Transaction {
        ops: vec![Op::SetCellInput {
            sheet: info.active_sheet,
            r,
            c,
            input: input.into(),
        }],
        label: Some("Typing".into()),
        selection_before: None,
        selection_after: None,
        group: None,
    };
    let res = with_doc(st, info.doc, |s| s.apply(info.doc, tx))?;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    Ok(())
}

/// The cell's number: `num` (General cells) or the formatted text parsed.
fn number(v: &CellView) -> Option<f64> {
    if v.kind != ValueKind::Number {
        return None;
    }
    v.num
        .or_else(|| v.text.replace(['$', ','], "").parse().ok())
}

/// Sum of column F (Total) over rows whose column G (Status) is not "Paid".
fn unpaid_total(view: &[CellView]) -> f64 {
    (1..=14)
        .filter(|r| cell(view, *r, 6).is_some_and(|v| v.text != "Paid"))
        .filter_map(|r| cell(view, r, 5).and_then(number))
        .sum()
}

#[test]
fn templates_are_listed_by_engine_id() -> CmdResult<()> {
    let list = files::list_templates();
    let ids: Vec<(&str, &str)> = list
        .iter()
        .map(|t| (t.id.as_str(), t.title.as_str()))
        .collect();
    assert_eq!(
        ids,
        [
            ("invoices", "Invoice tracker"),
            ("budget", "Monthly budget"),
            ("stock", "Stock count"),
        ]
    );
    let st = state();
    for t in &list {
        let info = new_workbook(&st, Some(&t.id))?;
        assert!(info.path.is_none(), "{}", t.id);
    }
    let stock = new_workbook(&st, Some("stock"))?;
    let view = cells(&st, stock.doc, stock.active_sheet, range(0, 0, 12, 6))?;
    assert_eq!(cell(&view, 0, 6).map(|v| v.text.as_str()), Some("Status"));
    let statuses: Vec<&str> = (1..=12)
        .filter_map(|r| cell(&view, r, 6).map(|v| v.text.as_str()))
        .collect();
    assert_eq!(statuses.len(), 12);
    assert!(statuses.contains(&"Low") && statuses.contains(&"OK"));
    let err = new_workbook(&st, Some("/etc/passwd")).err();
    assert_eq!(err.map(|e| e.code), Some(ApiError::NOT_FOUND.to_owned()));
    Ok(())
}

#[test]
fn invoices_sumif_assist_and_round_trips() -> CmdResult<()> {
    let st = state();
    let info = new_workbook(&st, Some("invoices"))?;
    let sheet = info.active_sheet;

    // The template: header row and numeric totals.
    let view = cells(&st, info.doc, sheet, range(0, 0, 14, 6))?;
    assert_eq!(cell(&view, 0, 5).map(|v| v.text.as_str()), Some("Total"));
    for r in 1..=14 {
        let total = cell(&view, r, 5).expect("total cell");
        assert_eq!(total.kind, ValueKind::Number, "F{}", r + 1);
        assert!(number(total).is_some_and(|n| n > 0.0), "F{}", r + 1);
    }
    let expected = unpaid_total(&view);
    assert!(expected > 0.0);

    // ⌘K at A17: the rule provider proposes the SUMIF, with a preview
    // computed by the engine without committing.
    let sel = SelectionDto {
        sheet,
        ranges: vec![range(16, 0, 16, 0)],
        active: CellPos { r: 16, c: 0 },
    };
    let out = with_doc(&st, info.doc, |s| {
        s.assist_suggest(info.doc, "total of unpaid orders", &sel)
    })?;
    let top = out.first().expect("a suggestion");
    assert_eq!(top.formula.as_deref(), Some(SUMIF), "{out:#?}");
    let preview = top.preview.clone().unwrap_or_default();
    assert!(!preview.is_empty(), "{top:#?}");
    assert!(
        preview.replace([',', '$'], "").parse::<f64>().ok()
            == Some((expected * 100.0).round() / 100.0),
        "preview {preview} vs {expected}"
    );
    assert!(
        cells(&st, info.doc, sheet, range(16, 0, 16, 0))?.is_empty(),
        "not committed"
    );
    let actions: Vec<_> = out.iter().filter(|s| s.action.is_some()).collect();
    assert!(actions.iter().all(|s| s.formula.is_none()));

    // Commit it.
    set(&st, &info, 16, 0, SUMIF)?;
    let a17 = cells(&st, info.doc, sheet, range(16, 0, 16, 0))?;
    let got = a17.first().and_then(number).unwrap_or(f64::NAN);
    assert!((got - expected).abs() < 1e-9, "{got} vs {expected}");
    let edit = with_doc(&st, info.doc, |s| s.cell_edit_info(info.doc, sheet, 16, 0))?;
    assert_eq!(edit.input, SUMIF);

    // A text cell that looks like a formula (for the CSV guard).
    set(&st, &info, 17, 1, "'=1+2")?;

    // ODS: save, open through the allowlist, identical cells.
    let ods = temp_path("invoices.ods");
    let saved = save_to(&st, info.doc, &ods, FileFormat::Ods)?;
    assert_eq!(
        saved.title,
        ods.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    );
    assert!(!saved.dirty);
    let err = files::check_open_allowed(&st, &ods).err();
    assert_eq!(
        err.map(|e| e.code),
        Some(ApiError::INVALID_ARGUMENT.to_owned())
    );
    files::allow(&st, &ods)?;
    let canon = files::check_open_allowed(&st, &ods)?;
    let reopened = open_path(&st, &canon)?;
    let all = range(0, 0, 19, 7);
    let before = cells(&st, info.doc, sheet, all)?;
    let after = cells(&st, reopened.doc, reopened.active_sheet, all)?;
    let key = |v: &CellView| {
        (
            v.r,
            v.c,
            v.text.clone(),
            v.kind,
            v.num.map(f64::to_bits),
            v.flags,
        )
    };
    let a: Vec<_> = before
        .iter()
        .filter(|v| v.kind != ValueKind::Empty)
        .map(key)
        .collect();
    let b: Vec<_> = after
        .iter()
        .filter(|v| v.kind != ValueKind::Empty)
        .map(key)
        .collect();
    assert!(!a.is_empty());
    assert_eq!(a, b);
    let edit = with_doc(&st, reopened.doc, |s| {
        s.cell_edit_info(reopened.doc, reopened.active_sheet, 16, 0)
    })?;
    assert_eq!(edit.input, SUMIF);
    let style_of = |st: &AppState, doc: DocId, v: Option<&CellView>| {
        let id = v.map_or(0, |v| v.style);
        with_doc(st, doc, |s| s.styles(doc, &[id]))
    };
    let h0 = style_of(&st, info.doc, cell(&before, 0, 0))?;
    let h1 = style_of(&st, reopened.doc, cell(&after, 0, 0))?;
    assert!(h0[0].bold && h1[0].bold, "header stays bold");
    let p0 = style_of(&st, info.doc, cell(&before, 1, 4))?;
    let p1 = style_of(&st, reopened.doc, cell(&after, 1, 4))?;
    // The currency symbol keeps its language (`[$$-409]`) through ODS; the
    // text is compared above.
    for p in [&p0[0].number_format, &p1[0].number_format] {
        assert!(
            p.contains("#,##0.00") && p.contains('$'),
            "currency survives: {p}"
        );
    }

    // CSV: header first, formula-like text guarded.
    let csv = temp_path("invoices.csv");
    save_to(&st, info.doc, &csv, FileFormat::Csv)?;
    let text =
        std::fs::read_to_string(&csv).map_err(|e| ApiError::new(ApiError::IO, e.to_string()))?;
    let first = text.lines().next().unwrap_or_default();
    assert_eq!(first, "Date,Customer,Item,Qty,Price,Total,Status");
    assert!(text.contains("'=1+2"), "{text}");
    assert!(!text.lines().any(|l| l
        .split(',')
        .any(|f| f.trim_matches('"').starts_with("=1+2"))));

    let _ = std::fs::remove_file(&ods);
    let _ = std::fs::remove_file(&csv);
    Ok(())
}

#[test]
fn format_currency_action_carries_its_range() -> CmdResult<()> {
    let st = state();
    let info = new_workbook(&st, Some("invoices"))?;
    let sel = SelectionDto {
        sheet: info.active_sheet,
        ranges: vec![range(16, 0, 16, 0)],
        active: CellPos { r: 16, c: 0 },
    };
    let out = with_doc(&st, info.doc, |s| {
        s.assist_suggest(info.doc, "format total as currency", &sel)
    })?;
    let fmt = out
        .iter()
        .find(|s| s.action.as_deref() == Some("format.currency"))
        .expect("format.currency");
    let args: Vec<(&str, &str)> = fmt
        .action_args
        .iter()
        .map(|a| (a.key.as_str(), a.value.as_str()))
        .collect();
    assert!(args.contains(&("column", "F")), "{args:?}");
    assert!(args.contains(&("range", "F2:F15")), "{args:?}");
    Ok(())
}

/// Security review M6: a panic inside an engine call is an `internal` error
/// for that call only; the mutex is not poisoned and later commands work.
#[test]
fn engine_panic_leaves_later_commands_working() -> CmdResult<()> {
    let st = state();
    let info = new_workbook(&st, None)?;
    let err = with_doc(&st, info.doc, |_| -> CmdResult<()> {
        panic!("injected test panic")
    })
    .err()
    .map(|e| (e.code, e.message));
    assert_eq!(
        err,
        Some((
            ApiError::INTERNAL.to_owned(),
            "the engine failed: injected test panic".to_owned()
        ))
    );
    assert!(!st.doc(info.doc)?.is_poisoned());
    set(&st, &info, 0, 0, "42")?;
    let v = cells(&st, info.doc, info.active_sheet, range(0, 0, 0, 0))?;
    assert_eq!(cell(&v, 0, 0).map(|c| c.text.as_str()), Some("42"));
    Ok(())
}

/// Security review M6: even a poisoned lock (a panic outside
/// `with_doc` while a document's lock was held) is recovered instead of
/// disabling that document or the app.
#[test]
fn poisoned_document_lock_is_recovered() -> CmdResult<()> {
    let st = std::sync::Arc::new(state());
    let info = new_workbook(&st, None)?;
    let slot = st.doc(info.doc)?;
    let _ = std::thread::spawn(move || {
        let _guard = slot.lock();
        panic!("poison the lock");
    })
    .join();
    assert!(st.doc(info.doc)?.is_poisoned());
    set(&st, &info, 0, 0, "1")?;
    let other = new_workbook(&st, None)?;
    assert_eq!(other.sheets.len(), 1);
    Ok(())
}

/// Per-document locks: while a long engine call holds document A (here an
/// apply that waits to be released, standing in for a slow recalc or save),
/// commands on document B, opening a new document and document-free calls
/// all complete.
#[test]
fn a_long_call_on_one_document_does_not_block_another() -> CmdResult<()> {
    use std::sync::{mpsc, Arc, Barrier};
    use std::time::Duration;

    let st = Arc::new(state());
    let a = new_workbook(&st, None)?;
    let b = new_workbook(&st, None)?;
    let started = Arc::new(Barrier::new(2));
    let (release, released) = mpsc::channel::<()>();
    let long = {
        let (st, started, a) = (Arc::clone(&st), Arc::clone(&started), a.clone());
        std::thread::spawn(move || {
            with_doc(&st, a.doc, |s| {
                started.wait();
                let _ = released.recv_timeout(Duration::from_secs(30));
                s.recalc_all(a.doc).map(|_| ())
            })
        })
    };
    started.wait();
    assert!(st.doc(a.doc)?.try_lock().is_err(), "A is held");

    let (done, finished) = mpsc::channel();
    let other = {
        let (st, b) = (Arc::clone(&st), b.clone());
        std::thread::spawn(move || {
            let run = || -> CmdResult<String> {
                set(&st, &b, 0, 0, "=6*7")?;
                let v = cells(&st, b.doc, b.active_sheet, range(0, 0, 0, 0))?;
                let c = new_workbook(&st, None)?;
                crate::commands::close_engine_doc(&st, c.doc)?;
                with_engine(&st, |s| Ok(s.list_functions().len()))?;
                Ok(cell(&v, 0, 0).map(|c| c.text.clone()).unwrap_or_default())
            };
            let _ = done.send(run());
        })
    };
    let got = finished.recv_timeout(Duration::from_secs(20));
    // Release A whatever happened, so a failure does not hang the test run.
    let _ = release.send(());
    let got = got.expect("document B was blocked by document A's long call")?;
    assert_eq!(got, "42");
    other.join().expect("B's thread");
    long.join().expect("A's thread")?;
    Ok(())
}

/// The internal clipboard is shared across documents: a copy in one
/// document pastes (formulas re-anchored) into another, each behind its
/// own lock.
#[test]
fn internal_copy_pastes_into_another_document() -> CmdResult<()> {
    let st = state();
    let a = new_workbook(&st, None)?;
    let b = new_workbook(&st, None)?;
    assert_ne!(a.doc, b.doc);
    set(&st, &a, 0, 0, "5")?;
    set(&st, &a, 0, 1, "=A1*2")?;
    let sel = SelectionDto {
        sheet: a.active_sheet,
        ranges: vec![range(0, 0, 0, 1)],
        active: CellPos { r: 0, c: 0 },
    };
    let out = with_doc(&st, a.doc, |s| s.copy(a.doc, &sel))?;
    let tx = Transaction {
        ops: vec![Op::Paste {
            sheet: b.active_sheet,
            target: range(2, 0, 2, 0),
            payload: tally_api::PastePayload::Internal { token: out.token },
        }],
        label: None,
        selection_before: None,
        selection_after: None,
        group: None,
    };
    let res = with_doc(&st, b.doc, |s| s.apply(b.doc, tx))?;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let v = cells(&st, b.doc, b.active_sheet, range(2, 0, 2, 1))?;
    assert_eq!(cell(&v, 2, 1).map(|c| c.text.as_str()), Some("10"));
    let edit = with_doc(&st, b.doc, |s| {
        s.cell_edit_info(b.doc, b.active_sheet, 2, 1)
    })?;
    assert_eq!(edit.input, "=A3*2");
    Ok(())
}

/// Security review L7: a path that was never allowed gets the same error
/// whether or not it exists (the allowlist is checked before the file
/// system is touched).
#[test]
fn open_allowlist_is_not_an_existence_oracle() -> CmdResult<()> {
    let st = state();
    let existing = std::env::temp_dir();
    let missing = temp_path("does-not-exist.ods");
    let a = files::check_open_allowed(&st, &existing).err();
    let b = files::check_open_allowed(&st, &missing).err();
    assert!(a.is_some());
    assert_eq!(a, b);
    Ok(())
}

/// Multi-window: a file already open in a document window is found by its
/// path as remembered for Save, given or canonical (a saved document keeps
/// the dialog's path, an opened one the canonical path).
#[test]
fn an_open_file_is_found_by_its_path() -> CmdResult<()> {
    let st = state();
    let info = new_workbook(&st, None)?;
    let file = temp_path("open-twice.ods");
    let saved = save_to(&st, info.doc, &file, FileFormat::Ods)?;
    assert_eq!(saved.doc, info.doc);
    let canon =
        std::fs::canonicalize(&file).map_err(|e| ApiError::new(ApiError::IO, e.to_string()))?;
    let paths = crate::state::lock(&st.paths).clone();
    assert_eq!(
        crate::commands::doc_with_path(&paths, &canon),
        Some(info.doc)
    );
    assert_eq!(
        crate::commands::doc_with_path(&paths, &temp_path("other.ods")),
        None
    );
    let _ = std::fs::remove_file(&file);
    Ok(())
}

/// Security review (multi-window S3): a new window's summary comes from the
/// engine (`DocumentService::info`); an unknown document is refused.
#[test]
fn a_new_window_gets_the_engine_summary_of_its_document() -> CmdResult<()> {
    let st = state();
    let info = new_workbook(&st, Some("invoices"))?;
    let again = with_doc(&st, info.doc, |s| s.info(info.doc))?;
    assert_eq!(again.doc, info.doc);
    assert_eq!(again.title, info.title);
    assert_eq!(again.sheets, info.sheets);
    assert_eq!(again.dirty, info.dirty);
    let err = with_doc(&st, DocId(9_999), |s| s.info(DocId(9_999))).err();
    assert_eq!(err.map(|e| e.code), Some(ApiError::NOT_FOUND.to_owned()));
    Ok(())
}

// ----- v0.2 over the real engine ------------------------------------------------

fn apply_ok(st: &AppState, doc: DocId, ops: Vec<Op>) -> CmdResult<tally_api::ApplyResult> {
    let tx = Transaction {
        ops,
        label: Some("test".into()),
        selection_before: None,
        selection_after: None,
        group: None,
    };
    let res = with_doc(st, doc, |s| s.apply(doc, tx))?;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    Ok(res)
}

/// Column F (Total) of rows 2..=15 as numbers.
fn totals(st: &AppState, info: &WorkbookInfo) -> CmdResult<Vec<f64>> {
    let view = cells(st, info.doc, info.active_sheet, range(1, 5, 14, 5))?;
    Ok((1..=14)
        .filter_map(|r| cell(&view, r, 5).and_then(number))
        .collect())
}

#[test]
fn sort_invoices_by_total_descending() -> CmdResult<()> {
    let st = state();
    let info = new_workbook(&st, Some("invoices"))?;
    let sheet = info.active_sheet;
    let table = range(0, 0, 14, 6);
    let header = with_doc(&st, info.doc, |s| s.detect_header(info.doc, sheet, table))?;
    assert!(header, "the invoices header row is detected");
    let before = totals(&st, &info)?;
    apply_ok(
        &st,
        info.doc,
        vec![Op::SortRange {
            sheet,
            range: table,
            keys: vec![tally_api::SortKey {
                col: 5,
                ascending: false,
            }],
            has_header: true,
            case_sensitive: false,
            options: None,
        }],
    )?;
    let after = totals(&st, &info)?;
    assert_eq!(after.len(), 14);
    assert!(after.windows(2).all(|w| w[0] >= w[1]), "{after:?}");
    let mut expected = before;
    expected.sort_by(|a, b| b.total_cmp(a));
    assert_eq!(after, expected);
    // The header stays; per-row formulas keep reading their own record.
    let view = cells(&st, info.doc, sheet, range(0, 0, 1, 6))?;
    assert_eq!(cell(&view, 0, 5).map(|v| v.text.as_str()), Some("Total"));
    let qty = cell(&view, 1, 3).and_then(number).unwrap_or(f64::NAN);
    let price = cell(&view, 1, 4).and_then(number).unwrap_or(f64::NAN);
    assert!((qty * price - after[0]).abs() < 1e-9);
    Ok(())
}

#[test]
fn autofilter_status_paid() -> CmdResult<()> {
    let st = state();
    let info = new_workbook(&st, Some("invoices"))?;
    let sheet = info.active_sheet;
    apply_ok(
        &st,
        info.doc,
        vec![
            Op::SetAutoFilter {
                sheet,
                range: Some(range(0, 0, 14, 6)),
            },
            Op::SetColumnFilter {
                sheet,
                col: 6,
                filter: Some(tally_api::ColumnFilter {
                    values: Some(vec!["Paid".into()]),
                    condition: None,
                }),
            },
        ],
    )?;
    let fi = with_doc(&st, info.doc, |s| s.filter_info(info.doc, sheet))?;
    assert_eq!(fi.range, Some(range(0, 0, 14, 6)));
    let status = fi
        .columns
        .iter()
        .find(|c| c.col == 6)
        .expect("status column");
    let counts: Vec<(&str, u32)> = status
        .unique
        .iter()
        .map(String::as_str)
        .zip(status.counts.iter().copied())
        .collect();
    assert_eq!(counts, [("Due", 4), ("Overdue", 2), ("Paid", 8)]);
    assert!(status.active.is_some());
    let layout = with_doc(&st, info.doc, |s| s.sheet_layout(info.doc, sheet))?;
    let filtered: u32 = layout
        .row_runs
        .iter()
        .filter(|r| r.hidden && r.filtered == Some(true))
        .map(|r| r.end - r.start + 1)
        .sum();
    assert_eq!(filtered, 6, "{:?}", layout.row_runs);
    Ok(())
}

#[test]
fn merge_names_find() -> CmdResult<()> {
    let st = state();
    let info = new_workbook(&st, Some("invoices"))?;
    let sheet = info.active_sheet;
    let doc = info.doc;

    // Merge A1:B1: the layout reports it; only the top-left value stays.
    apply_ok(
        &st,
        doc,
        vec![Op::MergeCells {
            sheet,
            range: range(0, 0, 0, 1),
        }],
    )?;
    let layout = with_doc(&st, doc, |s| s.sheet_layout(doc, sheet))?;
    assert_eq!(layout.merges, [range(0, 0, 0, 1)]);
    let view = cells(&st, doc, sheet, range(0, 0, 0, 1))?;
    assert_eq!(cell(&view, 0, 0).map(|v| v.text.as_str()), Some("Date"));
    assert!(cell(&view, 0, 1).is_none_or(|v| v.text.is_empty()));

    // A workbook name used in a formula.
    apply_ok(
        &st,
        doc,
        vec![Op::DefineName {
            name: "TaxRate".into(),
            scope: None,
            refers_to: "0.1".into(),
        }],
    )?;
    let names = with_doc(&st, doc, |s| s.names(doc))?;
    assert_eq!(names.len(), 1);
    assert_eq!(names[0].name, "TaxRate");
    set(&st, &info, 1, 7, "=F2*TaxRate")?;
    let view = cells(&st, doc, sheet, range(1, 5, 1, 7))?;
    let f2 = cell(&view, 1, 5).and_then(number).unwrap_or(f64::NAN);
    let h2 = cell(&view, 1, 7).and_then(number).unwrap_or(f64::NAN);
    assert!((h2 - f2 * 0.1).abs() < 1e-9, "{h2} vs {f2}");
    let edit = with_doc(&st, doc, |s| s.cell_edit_info(doc, sheet, 1, 7))?;
    assert_eq!(edit.input, "=F2*TaxRate");

    // Find "Paid": eight status cells.
    let found = with_doc(&st, doc, |s| {
        s.find(
            doc,
            &tally_api::FindQuery {
                text: "Paid".into(),
                sheet: Some(sheet),
                in_formulas: false,
                match_case: false,
                whole_cell: true,
                mode: tally_api::FindMode::Literal,
                from: None,
                in_notes: None,
                selection: None,
            },
        )
    })?;
    assert_eq!(found.hits.len(), 8);
    assert!(found.hits.iter().all(|h| h.c == 6));
    assert!(!found.truncated);
    Ok(())
}

#[test]
fn xlsx_save_and_reopen_keeps_values() -> CmdResult<()> {
    let st = state();
    let info = new_workbook(&st, Some("invoices"))?;
    set(&st, &info, 16, 0, SUMIF)?;
    let path = temp_path("invoices.xlsx");
    assert_eq!(crate::dialogs::format_for(&path), FileFormat::Xlsx);
    let saved = save_to(&st, info.doc, &path, FileFormat::Xlsx)?;
    assert!(!saved.dirty);
    files::allow(&st, &path)?;
    let canon = files::check_open_allowed(&st, &path)?;
    let reopened = open_path(&st, &canon)?;
    let all = range(0, 0, 16, 6);
    let key = |v: &CellView| (v.r, v.c, v.text.clone(), v.kind);
    let before: Vec<_> = cells(&st, info.doc, info.active_sheet, all)?
        .iter()
        .filter(|v| v.kind != ValueKind::Empty)
        .map(key)
        .collect();
    let after: Vec<_> = cells(&st, reopened.doc, reopened.active_sheet, all)?
        .iter()
        .filter(|v| v.kind != ValueKind::Empty)
        .map(key)
        .collect();
    assert!(before.len() > 100);
    assert_eq!(before, after);
    let edit = with_doc(&st, reopened.doc, |s| {
        s.cell_edit_info(reopened.doc, reopened.active_sheet, 16, 0)
    })?;
    assert_eq!(edit.input, SUMIF);
    let _ = std::fs::remove_file(&path);
    Ok(())
}

#[test]
fn v02_commands_answer() -> CmdResult<()> {
    let st = state();
    let info = new_workbook(&st, Some("invoices"))?;
    let res = with_doc(&st, info.doc, |s| s.recalc_all(info.doc))?;
    assert!(res.errors.is_empty());
    let fi = with_doc(&st, info.doc, |s| {
        s.filter_info(info.doc, info.active_sheet)
    })?;
    assert!(fi.range.is_none());
    Ok(())
}

// ----- v0.3 over the real engine ------------------------------------------------

fn tx_of(ops: Vec<Op>) -> Transaction {
    Transaction {
        ops,
        label: Some("test".into()),
        selection_before: None,
        selection_after: None,
        group: None,
    }
}

/// Applies without asserting success (validation / protection refusals).
fn apply_any(st: &AppState, doc: DocId, ops: Vec<Op>) -> CmdResult<tally_api::ApplyResult> {
    with_doc(st, doc, |s| s.apply(doc, tx_of(ops)))
}

#[test]
fn conditional_format_style_shows_in_cells() -> CmdResult<()> {
    use tally_api::{CfRuleDto, CfRuleKind, CfStyleDto, CmpOp};
    let st = state();
    let info = new_workbook(&st, Some("invoices"))?;
    let sheet = info.active_sheet;
    let fill = 0xfee2_e2ff_u32;
    let rule = CfRuleDto {
        kind: CfRuleKind::CellValue {
            op: CmpOp::Gt,
            formulas: vec!["50".into()],
        },
        style: CfStyleDto {
            fill: Some(fill),
            bold: Some(true),
            ..CfStyleDto::default()
        },
        stop_if_true: false,
    };
    apply_ok(
        &st,
        info.doc,
        vec![Op::AddConditionalFormat {
            sheet,
            ranges: vec![range(1, 5, 14, 5)],
            rules: vec![rule],
        }],
    )?;
    let listed = with_doc(&st, info.doc, |s| s.conditional_formats(info.doc, sheet))?;
    assert_eq!(listed.len(), 1);
    let view = cells(&st, info.doc, sheet, range(1, 5, 14, 5))?;
    let mut hits = 0;
    for v in &view {
        let big = number(v).is_some_and(|n| n > 50.0);
        match v.cf_style {
            Some(id) => {
                assert!(big, "row {} has a cf style but is not > 50", v.r);
                assert!(id >= 0x4000_0000);
                let styles = with_doc(&st, info.doc, |s| s.styles(info.doc, &[id]))?;
                assert_eq!(styles.first().and_then(|s| s.fill), Some(fill));
                assert!(styles.first().is_some_and(|s| s.bold));
                hits += 1;
            }
            None => assert!(!big, "row {} is > 50 without a cf style", v.r),
        }
    }
    assert!(hits > 0);
    Ok(())
}

#[test]
fn validation_stop_rejects_input() -> CmdResult<()> {
    use tally_api::{CmpOp, ValidationAlert, ValidationDto, ValidationErrorStyle, ValidationKind};
    let st = state();
    let info = new_workbook(&st, Some("invoices"))?;
    let sheet = info.active_sheet;
    let validation = ValidationDto {
        id: 0,
        ranges: vec![range(1, 3, 14, 3)],
        kind: ValidationKind::Whole {
            op: CmpOp::Between,
            formulas: vec!["1".into(), "10".into()],
        },
        allow_blank: true,
        show_dropdown: false,
        input_help: None,
        error: Some(ValidationAlert {
            style: ValidationErrorStyle::Stop,
            title: "Quantity".into(),
            text: "Enter 1 to 10.".into(),
        }),
    };
    apply_ok(&st, info.doc, vec![Op::AddValidation { sheet, validation }])?;
    let before = cells(&st, info.doc, sheet, range(1, 3, 1, 3))?;
    let res = apply_any(
        &st,
        info.doc,
        vec![Op::SetCellInput {
            sheet,
            r: 1,
            c: 3,
            input: "50".into(),
        }],
    )?;
    let err = res.errors.first().expect("a validation error");
    assert_eq!(err.code, "validation");
    assert_eq!(err.message, "Enter 1 to 10.");
    let after = cells(&st, info.doc, sheet, range(1, 3, 1, 3))?;
    assert_eq!(
        before.first().map(|v| &v.text),
        after.first().map(|v| &v.text)
    );
    // A valid value goes through.
    apply_ok(
        &st,
        info.doc,
        vec![Op::SetCellInput {
            sheet,
            r: 1,
            c: 3,
            input: "7".into(),
        }],
    )?;
    let at = with_doc(&st, info.doc, |s| s.validation_at(info.doc, sheet, 1, 3))?;
    assert_eq!(
        at.and_then(|v| v.error).map(|e| e.title),
        Some("Quantity".into())
    );
    Ok(())
}

#[test]
fn note_round_trips_through_ods() -> CmdResult<()> {
    use tally_api::NoteDto;
    let st = state();
    let info = new_workbook(&st, Some("invoices"))?;
    let sheet = info.active_sheet;
    let note = NoteDto {
        r: 2,
        c: 1,
        text: "Call before shipping".into(),
        author: Some("QA".into()),
        created: None,
        shown: false,
    };
    apply_ok(
        &st,
        info.doc,
        vec![Op::SetNote {
            sheet,
            r: 2,
            c: 1,
            note,
        }],
    )?;
    let flagged = cells(&st, info.doc, sheet, range(2, 1, 2, 1))?;
    assert!(flagged.first().is_some_and(|v| v.flags & 8 != 0));
    let path = temp_path("notes.ods");
    save_to(&st, info.doc, &path, FileFormat::Ods)?;
    let reopened = open_path(&st, &path)?;
    let back = with_doc(&st, reopened.doc, |s| {
        s.note(reopened.doc, reopened.active_sheet, 2, 1)
    })?
    .expect("the note survives the round trip");
    assert_eq!(back.text, "Call before shipping");
    assert_eq!(back.author.as_deref(), Some("QA"));
    let _ = std::fs::remove_file(&path);
    Ok(())
}

#[test]
fn protected_sheet_rejects_an_edit() -> CmdResult<()> {
    let st = state();
    let info = new_workbook(&st, Some("invoices"))?;
    let sheet = info.active_sheet;
    apply_ok(
        &st,
        info.doc,
        vec![Op::ProtectSheet {
            sheet,
            password: Some("secret".into()),
            options: None,
        }],
    )?;
    let p = with_doc(&st, info.doc, |s| s.protection_info(info.doc, sheet))?;
    assert!(p.sheet_protected && p.sheet_has_password);
    let res = apply_any(
        &st,
        info.doc,
        vec![Op::SetCellInput {
            sheet,
            r: 1,
            c: 1,
            input: "changed".into(),
        }],
    )?;
    assert_eq!(
        res.errors.first().map(|e| e.code.as_str()),
        Some("protected")
    );
    // A wrong password keeps the protection; the right one lifts it.
    let wrong = apply_any(
        &st,
        info.doc,
        vec![Op::UnprotectSheet {
            sheet,
            password: Some("nope".into()),
        }],
    )?;
    assert_eq!(
        wrong.errors.first().map(|e| e.code.as_str()),
        Some("protected")
    );
    apply_ok(
        &st,
        info.doc,
        vec![Op::UnprotectSheet {
            sheet,
            password: Some("secret".into()),
        }],
    )?;
    set(&st, &info, 1, 1, "changed")?;
    Ok(())
}

#[test]
fn goal_seek_sets_a_total_by_varying_qty() -> CmdResult<()> {
    use tally_api::GoalSeekRequest;
    let st = state();
    let info = new_workbook(&st, Some("invoices"))?;
    let sheet = info.active_sheet;
    let req = GoalSeekRequest {
        formula_sheet: sheet,
        formula_cell: CellPos { r: 1, c: 5 },
        target: 100.0,
        variable_sheet: sheet,
        variable_cell: CellPos { r: 1, c: 3 },
    };
    let res = with_doc(&st, info.doc, |s| s.goal_seek(info.doc, &req))?;
    assert!(res.found, "{res:?}");
    assert!((res.result - 100.0).abs() < 1e-6, "{res:?}");
    // Nothing changed until the value is applied.
    set(&st, &info, 1, 3, &res.value.to_string())?;
    let view = cells(&st, info.doc, sheet, range(1, 5, 1, 5))?;
    let total = view.first().and_then(number).unwrap_or(f64::NAN);
    assert!((total - 100.0).abs() < 0.01, "{total}");
    Ok(())
}

#[test]
fn open_url_rejects_script_and_file_links() {
    let mut launched = Vec::new();
    for url in [
        "javascript:alert(1)",
        "file:///etc/passwd",
        "data:text/html,x",
        "  ",
    ] {
        let e = crate::links::open_url_with(url, |u| {
            launched.push(u.to_owned());
            Ok(())
        })
        .unwrap_err();
        assert_eq!(e.code, ApiError::INVALID_ARGUMENT, "{url}");
    }
    assert!(launched.is_empty());
    assert!(crate::links::open_url_with("https://example.com/x", |u| {
        launched.push(u.to_owned());
        Ok(())
    })
    .is_ok());
    assert_eq!(launched, vec!["https://example.com/x".to_owned()]);
}

#[test]
fn oversized_arguments_are_rejected_before_the_engine() -> CmdResult<()> {
    use crate::limits;
    use tally_api::{FindMode, FindQuery, PastePayload};
    let st = state();
    let info = new_workbook(&st, Some("invoices"))?;
    let sheet = info.active_sheet;
    let big_input = tx_of(vec![Op::SetCellInput {
        sheet,
        r: 0,
        c: 0,
        input: "x".repeat(limits::MAX_TEXT + 1),
    }]);
    assert_eq!(
        limits::transaction(&big_input).map_err(|e| e.code),
        Err(ApiError::INVALID_ARGUMENT.to_owned())
    );
    let query = |text: String, mode| FindQuery {
        text,
        sheet: None,
        in_formulas: false,
        match_case: false,
        whole_cell: false,
        mode,
        from: None,
        in_notes: None,
        selection: None,
    };
    assert!(
        limits::find_query(&query("a".repeat(limits::MAX_TEXT + 1), FindMode::Literal)).is_err()
    );
    assert!(limits::find_query(&query("(a)".repeat(30_000), FindMode::Regex)).is_err());
    let long_name = tx_of(vec![Op::DefineName {
        name: "N".repeat(limits::MAX_NAME_CHARS + 1),
        scope: None,
        refers_to: "1".into(),
    }]);
    assert!(limits::transaction(&long_name).is_err());
    let rename = tx_of(vec![Op::RenameSheet {
        sheet,
        name: "S".repeat(limits::MAX_NAME_CHARS + 1),
    }]);
    assert!(limits::transaction(&rename).is_err());
    let huge_paste = tx_of(vec![Op::Paste {
        sheet,
        target: range(0, 0, 0, 0),
        payload: PastePayload::Tsv {
            text: "1\t".repeat(limits::MAX_TSV / 2 + 1),
        },
    }]);
    assert!(limits::transaction(&huge_paste).is_err());
    let outside = tx_of(vec![Op::Clear {
        sheet,
        ranges: vec![range(0, 0, tally_types::MAX_ROWS, 0)],
        what: tally_api::ClearWhat::Contents,
    }]);
    assert!(limits::transaction(&outside).is_err());
    assert!(limits::ranges("get_cells", &[range(0, 0, 0, tally_types::MAX_COLS)]).is_err());
    // Within the limits the same shapes pass and apply.
    let ok = tx_of(vec![Op::SetCellInput {
        sheet,
        r: 20,
        c: 0,
        input: "x".repeat(1000),
    }]);
    limits::transaction(&ok)?;
    let res = with_doc(&st, info.doc, |s| s.apply(info.doc, ok))?;
    assert!(res.errors.is_empty());
    Ok(())
}

// ----- wave 5 over the real engine (integration pass) ---------------------------

mod wave5 {
    use std::io::Read as _;

    use tally_api::{
        AggFn, CalcSettingsDto, ChartKind, CriteriaModeDto, CsvImportOptionsDto, DataLayout,
        DateEpochDto, IterationsDto, ListSource, OutlineAxis, PivotDataFieldDto, PivotDto,
        PivotFieldDto, PivotLayout, PivotSort, PivotSource, PivotSubtotals, PrintOptionsDto,
        PrintSheets, ScenarioDto, SheetCell, SheetRange, SolverConstraint, SolverGoal, SolverOp,
        SolverOptions, SolverRequest, SolverStatus, SplitDto, StdCondition, StdConnector,
        StdFilterOp, StdFilterOptions, SubtotalGroup, SubtotalOptions, TextToNumberDto,
        ValidationAlert, ValidationDto, ValidationErrorStyle, ValidationKind, ViewOptionsDto,
    };

    use super::*;

    fn repo(rel: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../..")
            .join(rel)
    }

    fn text_at(view: &[CellView], r: u32, c: u32) -> String {
        cell(view, r, c).map(|v| v.text.clone()).unwrap_or_default()
    }

    /// A column chart of Qty, Price and Total (D1:F15) on the invoices
    /// template, as `suggest_chart` proposes it; returns the chart id.
    fn add_invoice_chart(st: &AppState, info: &WorkbookInfo) -> CmdResult<u32> {
        let sheet = info.active_sheet;
        let chart = with_doc(st, info.doc, |s| {
            s.suggest_chart(info.doc, sheet, range(0, 3, 14, 5), ChartKind::Column)
        })?;
        assert_eq!(chart.kind, ChartKind::Column);
        assert!(!chart.series.is_empty(), "{chart:?}");
        let res = apply_ok(st, info.doc, vec![Op::AddChart { sheet, chart }])?;
        assert_eq!(res.objects_changed.as_deref(), Some(&[sheet][..]));
        let created = res.created.unwrap_or_default();
        assert_eq!(created.len(), 1);
        Ok(created[0].id)
    }

    fn series_values(st: &AppState, doc: DocId, sheet: SheetId, id: u32) -> CmdResult<Vec<f64>> {
        let data = with_doc(st, doc, |s| s.chart_data(doc, sheet, id))?;
        assert!(data.problems.is_empty(), "{:?}", data.problems);
        Ok(data
            .series
            .iter()
            .flat_map(|s| s.values.iter().map(|v| v.unwrap_or(f64::NAN)))
            .collect())
    }

    fn zip_entry(path: &std::path::Path, name: &str) -> String {
        let file = std::fs::File::open(path).expect("saved file");
        let mut z = zip::ZipArchive::new(file).expect("a zip");
        let mut s = String::new();
        z.by_name(name)
            .expect("entry")
            .read_to_string(&mut s)
            .expect("utf-8");
        s
    }

    #[test]
    fn chart_on_invoices_has_data_and_round_trips_through_ods_and_xlsx() -> CmdResult<()> {
        let st = state();
        let info = new_workbook(&st, Some("invoices"))?;
        let sheet = info.active_sheet;
        let id = add_invoice_chart(&st, &info)?;
        let data = with_doc(&st, info.doc, |s| s.chart_data(info.doc, sheet, id))?;
        assert_eq!(data.series.len(), 3, "{:?}", data.series);
        assert!(data.series.iter().all(|s| s.values.len() == 14));
        let before = series_values(&st, info.doc, sheet, id)?;
        assert!(before.iter().all(|v| v.is_finite() && *v > 0.0));

        // ODS: save, check the frame's update ranges, reopen.
        let ods = temp_path("chart.ods");
        save_to(&st, info.doc, &ods, FileFormat::Ods)?;
        let content = zip_entry(&ods, "content.xml");
        assert!(
            content.contains("draw:notify-on-update-of-ranges=\"Orders.D2:Orders.D15"),
            "notify ranges missing"
        );
        let again = open_path(&st, &ods)?;
        let s2 = again.sheets[0].id;
        let charts = with_doc(&st, again.doc, |s| s.charts(again.doc, s2))?;
        assert_eq!(charts.len(), 1);
        assert_eq!(series_values(&st, again.doc, s2, charts[0].id)?, before);

        // XLSX: save, reopen.
        let xlsx = temp_path("chart.xlsx");
        save_to(&st, info.doc, &xlsx, FileFormat::Xlsx)?;
        let x = open_path(&st, &xlsx)?;
        let s3 = x.sheets[0].id;
        let charts = with_doc(&st, x.doc, |s| s.charts(x.doc, s3))?;
        assert_eq!(charts.len(), 1, "{charts:?}");
        assert_eq!(charts[0].kind, ChartKind::Column);
        assert_eq!(series_values(&st, x.doc, s3, charts[0].id)?, before);
        let _ = std::fs::remove_file(&ods);
        let _ = std::fs::remove_file(&xlsx);
        Ok(())
    }

    fn field(col: u32, name: &str) -> PivotFieldDto {
        PivotFieldDto {
            source_col: col,
            name: name.into(),
            subtotals: PivotSubtotals::Auto,
            show_empty: false,
            sort: PivotSort::Name { ascending: true },
            hidden_items: Vec::new(),
            page_item: None,
            layout: PivotLayout::Tabular,
            empty_line_after: false,
            auto_show: None,
            group: None,
        }
    }

    /// Sum of Total by Status, at J1 of the Orders sheet.
    fn status_pivot(sheet: SheetId) -> PivotDto {
        PivotDto {
            id: 0,
            name: String::new(),
            source: PivotSource::Range {
                sheet,
                range: range(0, 0, 14, 6),
            },
            target_sheet: sheet,
            target: CellPos { r: 0, c: 9 },
            output: None,
            row_fields: vec![field(6, "Status")],
            col_fields: Vec::new(),
            page_fields: Vec::new(),
            data_fields: vec![PivotDataFieldDto {
                source_col: 5,
                function: AggFn::Sum,
                caption: None,
                show_as: None,
            }],
            data_layout: DataLayout::Row,
            row_totals: true,
            col_totals: true,
            ignore_empty_rows: false,
            identify_categories: false,
            show_filter_button: false,
            drill_down: true,
        }
    }

    #[test]
    fn pivot_sum_of_total_by_status_and_getpivotdata() -> CmdResult<()> {
        let st = state();
        let info = new_workbook(&st, Some("invoices"))?;
        let sheet = info.active_sheet;
        let table = cells(&st, info.doc, sheet, range(0, 0, 14, 6))?;
        let paid: f64 = (1..=14)
            .filter(|r| text_at(&table, *r, 6) == "Paid")
            .filter_map(|r| cell(&table, r, 5).and_then(number))
            .sum();
        let all: f64 = (1..=14)
            .filter_map(|r| cell(&table, r, 5).and_then(number))
            .sum();
        let res = apply_ok(
            &st,
            info.doc,
            vec![Op::CreatePivot {
                pivot: status_pivot(sheet),
                overwrite: false,
            }],
        )?;
        assert_eq!(res.pivots_changed, Some(true));
        let out = cells(&st, info.doc, sheet, range(0, 9, 10, 10))?;
        // AOO tabular layout without column fields: the row field name and
        // the data caption share the header row.
        assert_eq!(text_at(&out, 0, 9), "Status");
        assert_eq!(text_at(&out, 0, 10), "Sum - Total");
        let row_of = |label: &str| (0..=10).find(|r| text_at(&out, *r, 9) == label);
        let paid_row = row_of("Paid").expect("a Paid row");
        let total_row = row_of("Total Result").expect("a total row");
        let got = cell(&out, paid_row, 10)
            .and_then(number)
            .unwrap_or(f64::NAN);
        assert!((got - paid).abs() < 1e-6, "{got} vs {paid}");
        let total = cell(&out, total_row, 10)
            .and_then(number)
            .unwrap_or(f64::NAN);
        assert!((total - all).abs() < 1e-6, "{total} vs {all}");
        // GETPIVOTDATA reads the output.
        set(
            &st,
            &info,
            19,
            0,
            "=GETPIVOTDATA(\"Sum - Total\",J1,\"Status\",\"Paid\")",
        )?;
        let v = cells(&st, info.doc, sheet, range(19, 0, 19, 0))?;
        let gp = cell(&v, 19, 0).and_then(number).unwrap_or(f64::NAN);
        assert!((gp - paid).abs() < 1e-6, "{gp} vs {paid}");
        assert_eq!(with_doc(&st, info.doc, |s| s.pivots(info.doc))?.len(), 1);
        Ok(())
    }

    #[test]
    fn copy_sheet_repoints_charts_and_pivots_at_the_copy() -> CmdResult<()> {
        let st = state();
        let info = new_workbook(&st, Some("invoices"))?;
        let sheet = info.active_sheet;
        add_invoice_chart(&st, &info)?;
        apply_ok(
            &st,
            info.doc,
            vec![Op::CreatePivot {
                pivot: status_pivot(sheet),
                overwrite: false,
            }],
        )?;
        let res = apply_ok(
            &st,
            info.doc,
            vec![Op::CopySheet {
                sheet,
                name: Some("Copy".into()),
                index: None,
            }],
        )?;
        assert_eq!(res.pivots_changed, Some(true));
        let sheets = res.info.map(|i| i.sheets).unwrap_or_default();
        let copy = sheets
            .iter()
            .find(|s| s.name == "Copy")
            .map(|s| s.id)
            .expect("the copy");
        let charts = with_doc(&st, info.doc, |s| s.charts(info.doc, copy))?;
        assert_eq!(charts.len(), 1);
        for s in &charts[0].series {
            assert!(s.values.contains("Copy."), "{}", s.values);
            assert!(!s.values.contains("Orders"), "{}", s.values);
        }
        // The copy's chart follows the copy's cells, not the source's.
        set(&st, &info, 1, 3, "1000")?; // Orders D2
        let src = series_values(&st, info.doc, sheet, charts[0].id)?;
        let cpy = series_values(&st, info.doc, copy, charts[0].id)?;
        assert!(src.contains(&1000.0) && !cpy.contains(&1000.0));
        // Pivots: one on each sheet, the copy's reading the copy.
        let pivots = with_doc(&st, info.doc, |s| s.pivots(info.doc))?;
        assert_eq!(pivots.len(), 2, "{pivots:?}");
        let on_copy = pivots
            .iter()
            .find(|p| p.target_sheet == copy)
            .expect("a pivot on the copy");
        assert!(matches!(on_copy.source, PivotSource::Range { sheet: s, .. } if s == copy));
        let out = cells(&st, info.doc, copy, range(0, 10, 0, 10))?;
        assert_eq!(text_at(&out, 0, 10), "Sum - Total");
        // Undo removes the copy with its objects.
        with_doc(&st, info.doc, |s| s.undo(info.doc))?;
        with_doc(&st, info.doc, |s| s.undo(info.doc))?;
        assert_eq!(with_doc(&st, info.doc, |s| s.pivots(info.doc))?.len(), 1);
        Ok(())
    }

    #[test]
    fn group_and_collapse_rows_hides_them() -> CmdResult<()> {
        let st = state();
        let info = new_workbook(&st, Some("invoices"))?;
        let sheet = info.active_sheet;
        let res = apply_ok(
            &st,
            info.doc,
            vec![Op::Group {
                sheet,
                axis: OutlineAxis::Rows,
                start: 2,
                end: 5,
            }],
        )?;
        // The UI's outline gutter refetches the layout on this signal.
        assert!(res.layout_changed.contains(&sheet), "{res:?}");
        let layout = with_doc(&st, info.doc, |s| s.sheet_layout(info.doc, sheet))?;
        let outline = layout.outline.expect("an outline");
        assert_eq!(outline.rows.len(), 1);
        assert!(!layout.row_runs.iter().any(|r| r.hidden));
        let res = apply_ok(
            &st,
            info.doc,
            vec![Op::SetOutlineCollapsed {
                sheet,
                axis: OutlineAxis::Rows,
                start: 2,
                end: 5,
                collapsed: true,
            }],
        )?;
        assert!(res.layout_changed.contains(&sheet));
        let layout = with_doc(&st, info.doc, |s| s.sheet_layout(info.doc, sheet))?;
        let hidden: Vec<(u32, u32)> = layout
            .row_runs
            .iter()
            .filter(|r| r.hidden)
            .map(|r| (r.start, r.end))
            .collect();
        assert_eq!(hidden, [(2, 5)]);
        Ok(())
    }

    #[test]
    fn standard_filter_subtotals_and_text_to_columns() -> CmdResult<()> {
        let st = state();
        let info = new_workbook(&st, Some("invoices"))?;
        let sheet = info.active_sheet;
        let table = cells(&st, info.doc, sheet, range(0, 0, 14, 6))?;
        let not_paid = (1..=14)
            .filter(|r| text_at(&table, *r, 6) != "Paid")
            .count() as u32;
        apply_ok(
            &st,
            info.doc,
            vec![Op::StandardFilter {
                sheet,
                range: range(0, 0, 14, 6),
                conditions: vec![StdCondition {
                    connector: StdConnector::And,
                    col: 6,
                    op: StdFilterOp::Eq,
                    value: "Paid".into(),
                }],
                options: StdFilterOptions {
                    has_header: true,
                    ..StdFilterOptions::default()
                },
            }],
        )?;
        let layout = with_doc(&st, info.doc, |s| s.sheet_layout(info.doc, sheet))?;
        let filtered: u32 = layout
            .row_runs
            .iter()
            .filter(|r| r.hidden && r.filtered == Some(true))
            .map(|r| r.end - r.start + 1)
            .sum();
        assert_eq!(filtered, not_paid);
        apply_ok(&st, info.doc, vec![Op::ResetFilter { sheet }])?;

        // Subtotals of Total by Status.
        apply_ok(
            &st,
            info.doc,
            vec![Op::Subtotals {
                sheet,
                range: range(0, 0, 14, 6),
                groups: vec![SubtotalGroup {
                    group_by: 6,
                    columns: vec![5],
                    function: AggFn::Sum,
                }],
                options: SubtotalOptions::default(),
            }],
        )?;
        let view = cells(&st, info.doc, sheet, range(0, 0, 30, 6))?;
        let labels: Vec<String> = (0..=30).map(|r| text_at(&view, r, 6)).collect();
        assert!(labels.iter().any(|l| l == "Paid Sum"), "{labels:?}");
        assert!(labels.iter().any(|l| l == "Grand Total"), "{labels:?}");
        let layout = with_doc(&st, info.doc, |s| s.sheet_layout(info.doc, sheet))?;
        assert!(layout.outline.is_some_and(|o| !o.rows.is_empty()));

        // Text to columns on a fresh cell.
        set(&st, &info, 40, 0, "north;12;x")?;
        apply_ok(
            &st,
            info.doc,
            vec![Op::TextToColumns {
                sheet,
                range: range(40, 0, 40, 0),
                options: CsvImportOptionsDto {
                    delimiters: ";".into(),
                    ..CsvImportOptionsDto::default()
                },
            }],
        )?;
        let row = cells(&st, info.doc, sheet, range(40, 0, 40, 2))?;
        assert_eq!(text_at(&row, 40, 0), "north");
        assert_eq!(cell(&row, 40, 1).and_then(number), Some(12.0));
        assert_eq!(text_at(&row, 40, 2), "x");
        Ok(())
    }

    #[test]
    fn scenarios_and_cell_styles() -> CmdResult<()> {
        let st = state();
        let info = new_workbook(&st, None)?;
        let sheet = info.active_sheet;
        set(&st, &info, 0, 0, "10")?;
        set(&st, &info, 1, 0, "20")?;
        set(&st, &info, 0, 1, "=A1+A2")?;
        let scenario = |name: &str| ScenarioDto {
            id: 0,
            name: name.into(),
            comment: String::new(),
            ranges: vec![range(0, 0, 1, 0)],
            border_color: None,
            display_border: true,
            copy_back: true,
            copy_styles: false,
            prevent_changes: false,
            active: false,
        };
        let res = apply_ok(
            &st,
            info.doc,
            vec![Op::CreateScenario {
                sheet,
                scenario: scenario("Base"),
            }],
        )?;
        let base = res.created.unwrap_or_default()[0].id;
        set(&st, &info, 0, 0, "100")?;
        apply_ok(
            &st,
            info.doc,
            vec![Op::CreateScenario {
                sheet,
                scenario: scenario("High"),
            }],
        )?;
        assert_eq!(
            with_doc(&st, info.doc, |s| s.scenarios(info.doc, sheet))?.len(),
            2
        );
        apply_ok(&st, info.doc, vec![Op::ShowScenario { sheet, id: base }])?;
        let v = cells(&st, info.doc, sheet, range(0, 0, 0, 1))?;
        assert_eq!(cell(&v, 0, 0).and_then(number), Some(10.0));
        assert_eq!(cell(&v, 0, 1).and_then(number), Some(30.0));

        // Apply Heading, then edit the style: the cell follows.
        set(&st, &info, 5, 0, "Title")?;
        apply_ok(
            &st,
            info.doc,
            vec![Op::ApplyCellStyle {
                sheet,
                ranges: vec![range(5, 0, 5, 0)],
                name: "Heading".into(),
            }],
        )?;
        let style_of = |st: &AppState| -> CmdResult<tally_api::StyleDto> {
            let v = cells(st, info.doc, sheet, range(5, 0, 5, 0))?;
            let id = cell(&v, 5, 0).map_or(0, |c| c.style);
            let styles = with_doc(st, info.doc, |s| s.styles(info.doc, &[id]))?;
            styles
                .into_iter()
                .next()
                .ok_or_else(|| ApiError::not_found("no style"))
        };
        let s = style_of(&st)?;
        assert!(s.bold);
        assert_eq!(s.style_name.as_deref(), Some("Heading"));
        let mut heading = with_doc(&st, info.doc, |s| s.cell_styles(info.doc))?
            .into_iter()
            .find(|s| s.name == "Heading")
            .expect("Heading");
        heading.style.color = Some(0xff00_00ff);
        apply_ok(&st, info.doc, vec![Op::DefineCellStyle { style: heading }])?;
        assert_eq!(style_of(&st)?.color, Some(0xff00_00ff));
        Ok(())
    }

    fn all_sheets() -> PrintOptionsDto {
        PrintOptionsDto {
            sheets: PrintSheets::All,
            selection: None,
            suppress_empty_pages: true,
            page_from: None,
            page_to: None,
            active_sheet: None,
            page_ranges: None,
        }
    }

    #[test]
    fn print_layout_pdf_and_png_pages() -> CmdResult<()> {
        let st = state();
        let info = new_workbook(&st, Some("invoices"))?;
        let layout = with_doc(&st, info.doc, |s| s.print_layout(info.doc, &all_sheets()))?;
        assert!(!layout.pages.is_empty());
        let stats = with_doc(&st, info.doc, |s| s.document_statistics(info.doc))?;
        assert_eq!(stats.pages, Some(layout.pages.len() as u32));
        let job = with_doc(&st, info.doc, |s| {
            s.prepare_print_job(info.doc, &all_sheets(), &[])
        })?;
        assert_eq!(job.layout.pages.len(), layout.pages.len());
        let pdf = with_engine(&st, |s| s.print_job_pdf(job.job))?;
        assert!(pdf.starts_with(b"%PDF"), "a PDF");
        let png64 = with_engine(&st, |s| s.print_job_page_png(job.job, 0, 72))?;
        let png = tally_engine::protection::base64_decode(&png64).expect("base64");
        let (w, h, pixels) = tally_io_pdf::png_pixels(&png).expect("a PNG");
        assert!(w > 500 && h > 700, "{w}×{h} at 72 dpi");
        assert!(pixels.chunks(4).any(|p| p[0] < 128), "ink on the page");
        Ok(())
    }

    /// The owner's report: print and the preview took the first sheet. The
    /// UI names the sheet it is on (`active_sheet`); the job that `wb_print`
    /// and `wb_export_pdf` send is built from those options, so it holds that
    /// sheet's pages only, and "All sheets" holds every sheet.
    #[test]
    fn print_job_follows_the_users_sheet() -> CmdResult<()> {
        let st = state();
        let info = new_workbook(&st, None)?;
        apply_ok(
            &st,
            info.doc,
            vec![Op::AddSheet {
                name: Some("Two".into()),
                index: None,
            }],
        )?;
        let ids: Vec<SheetId> = with_doc(&st, info.doc, |s| s.info(info.doc))?
            .sheets
            .iter()
            .map(|s| s.id)
            .collect();
        let fill = |sheet: SheetId, rows: u32| {
            (0..rows)
                .map(|r| Op::SetCellInput {
                    sheet,
                    r,
                    c: 0,
                    input: format!("{sheet}/{r}"),
                })
                .collect::<Vec<_>>()
        };
        apply_ok(&st, info.doc, fill(ids[0], 3))?;
        apply_ok(&st, info.doc, fill(ids[1], 120))?;
        let opts = PrintOptionsDto {
            sheets: PrintSheets::Active,
            active_sheet: Some(ids[1]),
            ..PrintOptionsDto::default()
        };
        crate::limits::print_options(&opts)?;
        let job = with_doc(&st, info.doc, |s| s.prepare_print_job(info.doc, &opts, &[]))?;
        assert!(job.layout.pages.len() > 1);
        assert!(job.layout.pages.iter().all(|p| p.sheet == ids[1]));
        assert!(crate::print::pdf_bytes(&st, job.job)?.starts_with(b"%PDF"));
        let all = with_doc(&st, info.doc, |s| {
            s.prepare_print_job(info.doc, &all_sheets(), &[])
        })?;
        assert_eq!(all.layout.pages.len(), job.layout.pages.len() + 1);
        assert_eq!(all.layout.pages[0].sheet, ids[0]);
        Ok(())
    }

    #[test]
    fn iterations_converge_self_reference() -> CmdResult<()> {
        let st = state();
        let info = new_workbook(&st, None)?;
        let sheet = info.active_sheet;
        set(&st, &info, 0, 0, "=A1/2+1")?;
        let v = cells(&st, info.doc, sheet, range(0, 0, 0, 0))?;
        assert_eq!(text_at(&v, 0, 0), "Err:522");
        apply_ok(
            &st,
            info.doc,
            vec![Op::SetCalcSettings {
                settings: CalcSettingsDto {
                    iterations: Some(IterationsDto {
                        steps: 100,
                        min_change: 0.001,
                    }),
                    date_epoch: DateEpochDto::D1899,
                    case_sensitive: false,
                    precision_as_shown: false,
                    whole_cell_match: true,
                    criteria_mode: CriteriaModeDto::Wildcards,
                    text_to_number: TextToNumberDto::ConvertUnambiguous,
                },
            }],
        )?;
        let v = cells(&st, info.doc, sheet, range(0, 0, 0, 0))?;
        let x = cell(&v, 0, 0).and_then(number).unwrap_or(f64::NAN);
        assert!((x - 2.0).abs() < 0.001, "{x}");
        Ok(())
    }

    #[test]
    fn solver_max_profit_and_not_linear() -> CmdResult<()> {
        let st = state();
        let info = new_workbook(&st, None)?;
        let sheet = info.active_sheet;
        // AOO help "maximum profit": unit profits 5, 4, 6; material ≤ 100,
        // hours ≤ 180, product A ≤ 40.
        for r in 0..3 {
            set(&st, &info, r, 1, "0")?;
        }
        set(&st, &info, 0, 2, "=5*B1+4*B2+6*B3")?;
        set(&st, &info, 1, 2, "=B1+B2+B3")?;
        set(&st, &info, 2, 2, "=2*B1+B2+3*B3")?;
        let sr = |r0, c0, r1, c1| SheetRange {
            sheet,
            range: range(r0, c0, r1, c1),
        };
        let con = |r, rhs: &str| SolverConstraint {
            lhs: sr(r, 2, r, 2),
            op: SolverOp::Le,
            rhs: rhs.into(),
        };
        let mut req = SolverRequest {
            objective: SheetCell { sheet, r: 0, c: 2 },
            goal: SolverGoal::Maximize,
            variables: vec![sr(0, 1, 2, 1)],
            constraints: vec![
                con(1, "100"),
                con(2, "180"),
                SolverConstraint {
                    lhs: sr(0, 1, 0, 1),
                    op: SolverOp::Le,
                    rhs: "40".into(),
                },
            ],
            options: SolverOptions {
                assume_non_negative: true,
                ..SolverOptions::default()
            },
        };
        let r = with_doc(&st, info.doc, |s| s.solver(info.doc, &req))?;
        assert_eq!(r.status, SolverStatus::Optimal);
        assert!((r.objective - 480.0).abs() < 1e-6, "{r:?}");
        // The sandbox leaves the document alone.
        let v = cells(&st, info.doc, sheet, range(0, 1, 0, 1))?;
        assert_eq!(cell(&v, 0, 1).and_then(number), Some(0.0));
        set(&st, &info, 0, 2, "=B1*B2+B3")?;
        req.options.assume_non_negative = true;
        let r = with_doc(&st, info.doc, |s| s.solver(info.doc, &req))?;
        assert_eq!(r.status, SolverStatus::NotLinear, "{r:?}");
        Ok(())
    }

    #[test]
    fn legacy_formats_open_through_the_codec() -> CmdResult<()> {
        let st = state();
        let xls = open_path(&st, &repo("tests/fixtures/xls/basic.xls"))?;
        let v = cells(&st, xls.doc, xls.sheets[0].id, range(0, 0, 3, 1))?;
        assert_eq!(text_at(&v, 1, 1), "Zürich");
        assert_eq!(cell(&v, 3, 1).and_then(number), Some(3.0));
        let edit = with_doc(&st, xls.doc, |s| {
            s.cell_edit_info(xls.doc, xls.sheets[0].id, 3, 1)
        })?;
        assert_eq!(edit.input, "=SUM(A1:A2)");

        let sxc = open_path(&st, &repo("tests/fixtures/sxc/ooo1.sxc"))?;
        let v = cells(&st, sxc.doc, sxc.sheets[0].id, range(0, 0, 1, 2))?;
        assert_eq!(cell(&v, 1, 0).and_then(number), Some(3.0));
        assert_eq!(text_at(&v, 1, 2), "x!");

        let html = open_path(&st, &repo("tests/fixtures/html/two-tables.html"))?;
        assert_eq!(html.sheets.len(), 2);
        assert_eq!(html.sheets[0].name, "Sales");
        let v = cells(&st, html.doc, html.sheets[0].id, range(0, 0, 2, 2))?;
        assert_eq!(cell(&v, 1, 1).and_then(number), Some(1234.5));
        assert_eq!(text_at(&v, 2, 0), "South & East");

        let dbf = open_path(&st, &repo("tests/fixtures/dbf/customers.dbf"))?;
        let v = cells(&st, dbf.doc, dbf.sheets[0].id, range(0, 0, 1, 1))?;
        // AOO-style header descriptor (name,type,length), see review-4 M35.
        assert_eq!(text_at(&v, 0, 0), "NAME,C,12");
        assert_eq!(text_at(&v, 1, 0), "Müller & Co");
        assert_eq!(cell(&v, 1, 1).and_then(number), Some(1234.5));

        // Save As HTML and dBase through the command layer.
        for (fmt, ext) in [(FileFormat::Html, "html"), (FileFormat::Dbf, "dbf")] {
            let p = temp_path(&format!("export.{ext}"));
            let saved = save_to(&st, dbf.doc, &p, fmt)?;
            assert_eq!(
                saved.path.as_deref(),
                Some(p.display().to_string().as_str())
            );
            let back = open_path(&st, &p)?;
            let v = cells(&st, back.doc, back.sheets[0].id, range(1, 0, 1, 0))?;
            assert_eq!(text_at(&v, 1, 0), "Müller & Co", "{ext}");
            let _ = std::fs::remove_file(&p);
        }
        Ok(())
    }

    #[test]
    fn de_locale_countif_uses_comma_decimals() -> CmdResult<()> {
        let st = state();
        let info = new_workbook(&st, None)?;
        let sheet = info.active_sheet;
        with_doc(&st, info.doc, |s| s.set_locale(info.doc, "de-DE"))?;
        set(&st, &info, 0, 0, "1")?;
        set(&st, &info, 1, 0, "2")?;
        set(&st, &info, 2, 0, "1,7")?;
        set(&st, &info, 0, 1, "=COUNTIF(A1:A3;\">1,5\")")?;
        let v = cells(&st, info.doc, sheet, range(0, 1, 0, 1))?;
        assert_eq!(cell(&v, 0, 1).and_then(|c| c.num), Some(2.0));
        Ok(())
    }

    #[test]
    fn invalid_cells_marks_values_outside_the_list() -> CmdResult<()> {
        let st = state();
        let info = new_workbook(&st, Some("invoices"))?;
        let sheet = info.active_sheet;
        let table = cells(&st, info.doc, sheet, range(0, 0, 14, 6))?;
        let expected: Vec<u32> = (1..=14)
            .filter(|r| !matches!(text_at(&table, *r, 6).as_str(), "Paid" | "Due"))
            .collect();
        assert!(!expected.is_empty());
        apply_ok(
            &st,
            info.doc,
            vec![Op::AddValidation {
                sheet,
                validation: ValidationDto {
                    id: 0,
                    ranges: vec![range(1, 6, 14, 6)],
                    kind: ValidationKind::List {
                        source: ListSource::Values {
                            items: vec!["Paid".into(), "Due".into()],
                        },
                    },
                    allow_blank: true,
                    show_dropdown: true,
                    input_help: None,
                    error: Some(ValidationAlert {
                        style: ValidationErrorStyle::Stop,
                        title: "Status".into(),
                        text: "Paid or Due".into(),
                    }),
                },
            }],
        )?;
        let bad = with_doc(&st, info.doc, |s| s.invalid_cells(info.doc, sheet, None))?;
        let rows: Vec<u32> = bad.iter().map(|p| p.r).collect();
        assert_eq!(rows, expected);
        assert!(bad.iter().all(|p| p.c == 6));
        let none = with_doc(&st, info.doc, |s| {
            s.invalid_cells(info.doc, sheet, Some(range(0, 0, 14, 5)))
        })?;
        assert!(none.is_empty());
        Ok(())
    }

    #[test]
    fn view_options_round_trip_and_render() -> CmdResult<()> {
        let st = state();
        let info = new_workbook(&st, Some("invoices"))?;
        let sheet = info.active_sheet;
        assert_eq!(
            with_doc(&st, info.doc, |s| s.view_options(info.doc, sheet))?,
            ViewOptionsDto::default()
        );
        let opts = ViewOptionsDto {
            split: None,
            show_formulas: true,
            value_highlighting: true,
            show_zero_values: false,
            show_gridlines: false,
            show_headers: true,
        };
        apply_ok(
            &st,
            info.doc,
            vec![
                Op::SetViewOptions {
                    sheet,
                    options: opts.clone(),
                },
                Op::SetSplit {
                    sheet,
                    split: Some(SplitDto {
                        x_px: 240.0,
                        y_px: 120.0,
                    }),
                },
            ],
        )?;
        let got = with_doc(&st, info.doc, |s| s.view_options(info.doc, sheet))?;
        assert_eq!(got.split.map(|s| (s.x_px, s.y_px)), Some((240.0, 120.0)));
        let ods = temp_path("view.ods");
        save_to(&st, info.doc, &ods, FileFormat::Ods)?;
        let again = open_path(&st, &ods)?;
        let back = with_doc(&st, again.doc, |s| {
            s.view_options(again.doc, again.sheets[0].id)
        })?;
        assert!(back.show_formulas && back.value_highlighting);
        assert!(!back.show_zero_values && !back.show_gridlines && back.show_headers);
        let split = back
            .split
            .unwrap_or_else(|| panic!("the split is lost: {back:?}"));
        assert!((split.x_px - 240.0).abs() < 1.5 && (split.y_px - 120.0).abs() < 1.5);
        let _ = std::fs::remove_file(&ods);

        // cells_with: formulas, highlighting, zero values.
        set(&st, &info, 20, 0, "0")?;
        let co = tally_api::CellsOptions {
            show_formulas: true,
            value_highlighting: true,
            hide_zero_values: true,
        };
        let blocks = with_doc(&st, info.doc, |s| {
            s.cells_with(info.doc, sheet, &[range(1, 0, 20, 6)], &co)
        })?;
        let v: Vec<CellView> = blocks.into_iter().flat_map(|b| b.cells).collect();
        let total = cell(&v, 1, 5).expect("F2");
        assert!(total.text.starts_with('='), "{}", total.text);
        assert_eq!(total.color, Some(0x0080_00ff), "formulas are green");
        assert_eq!(cell(&v, 1, 3).and_then(|c| c.color), Some(0x0000_ffff));
        assert_eq!(cell(&v, 1, 1).and_then(|c| c.color), Some(0x0000_00ff));
        assert_eq!(text_at(&v, 20, 0), "", "zero hidden");
        let plain = cells(&st, info.doc, sheet, range(1, 5, 1, 5))?;
        assert!(!text_at(&plain, 1, 5).starts_with('='));
        Ok(())
    }

    #[test]
    fn save_copy_leaves_path_title_and_dirty_alone() -> CmdResult<()> {
        let st = state();
        let info = new_workbook(&st, Some("invoices"))?;
        let sheet = info.active_sheet;
        let real = temp_path("Real.ods");
        save_to(&st, info.doc, &real, FileFormat::Ods)?;
        set(&st, &info, 30, 0, "edited")?;
        let snap = temp_path("snap.ods");
        with_doc(&st, info.doc, |s| {
            s.save_copy(info.doc, &snap, FileFormat::Ods)
        })?;
        assert!(snap.is_file());
        // A sheet rename reports the document info.
        let res = apply_ok(
            &st,
            info.doc,
            vec![Op::RenameSheet {
                sheet,
                name: "Orders2".into(),
            }],
        )?;
        let after = res.info.expect("info");
        assert!(after.dirty, "still dirty after the copy");
        assert_eq!(
            after.path.as_deref(),
            Some(real.display().to_string().as_str())
        );
        assert!(after.title.starts_with("tally-desktop-"), "{}", after.title);
        // The copy holds the edit; opening it untitled forgets its path.
        let rec = with_new_doc(&st, |s| s.open_untitled(&snap, true))?;
        assert_eq!(
            (rec.path, rec.title.as_str(), rec.dirty),
            (None, "Untitled", true)
        );
        let v = cells(&st, rec.doc, rec.sheets[0].id, range(30, 0, 30, 0))?;
        assert_eq!(text_at(&v, 30, 0), "edited");
        let _ = std::fs::remove_file(&real);
        let _ = std::fs::remove_file(&snap);
        Ok(())
    }

    #[test]
    fn function_info_params_and_preview_formula() -> CmdResult<()> {
        let st = state();
        let fns = with_engine(&st, |s| Ok(s.list_functions()))?;
        let sumif = fns.iter().find(|f| f.name == "SUMIF").expect("SUMIF");
        let names: Vec<(&str, bool)> = sumif
            .params
            .iter()
            .map(|p| (p.name.as_str(), p.optional))
            .collect();
        assert_eq!(names.len(), 3);
        assert!(names[2].1, "sum_range is optional: {names:?}");
        assert!(sumif.example.starts_with("=SUMIF("));
        let sum = fns.iter().find(|f| f.name == "SUM").expect("SUM");
        assert!(sum.params.iter().any(|p| p.repeat));
        assert!(fns.iter().all(|f| !f.example.is_empty()));

        let info = new_workbook(&st, Some("invoices"))?;
        let sheet = info.active_sheet;
        let view = cells(&st, info.doc, sheet, range(0, 0, 14, 6))?;
        let unpaid = unpaid_total(&view);
        let p = with_doc(&st, info.doc, |s| {
            s.preview_formula(info.doc, sheet, 16, 0, SUMIF)
        })?
        .expect("a result");
        let n: f64 = p.replace([',', '$'], "").parse().unwrap_or(f64::NAN);
        assert!((n - unpaid).abs() < 0.005, "{p} vs {unpaid}");
        // Without "=", and errors.
        let p = with_doc(&st, info.doc, |s| {
            s.preview_formula(info.doc, sheet, 16, 0, "1+1")
        })?;
        assert_eq!(p.as_deref(), Some("2"));
        // The wizard builds calls with `;` (AOO's separator), which en-US accepts too.
        let p = with_doc(&st, info.doc, |s| {
            s.preview_formula(info.doc, sheet, 16, 0, "=SUMIF(G2:G15;\"Paid\";D2:D15)")
        })?;
        assert!(
            p.as_deref().is_some_and(|t| t.parse::<f64>().is_ok()),
            "{p:?}"
        );
        let p = with_doc(&st, info.doc, |s| {
            s.preview_formula(info.doc, sheet, 16, 0, "=1/0")
        })?;
        assert_eq!(p.as_deref(), Some("#DIV/0!"));
        let p = with_doc(&st, info.doc, |s| {
            s.preview_formula(info.doc, sheet, 16, 0, "=SUM(")
        })?;
        assert_eq!(p, None);
        // Nothing was committed.
        let v = cells(&st, info.doc, sheet, range(16, 0, 16, 0))?;
        assert_eq!(text_at(&v, 16, 0), "");
        Ok(())
    }
}

// ----- app ACL manifest (build.rs) ---------------------------------------------

mod app_acl {
    include!("../app_commands.rs");
}

/// Security review (multi-window S1): with an app manifest Tauri checks every
/// app command against the capabilities. The manifest must list exactly the
/// registered commands, and only the document webviews (`doc-*`, matched by
/// webview label; never a `win-*` window or a print window) may call them.
#[test]
fn every_registered_command_is_in_the_acl_manifest_and_granted_to_document_windows() {
    let lib = include_str!("lib.rs");
    let start = lib.find("generate_handler![").unwrap_or(0);
    let body = &lib[start..];
    let body = &body[..body.find("])").unwrap_or(body.len())];
    let registered: Vec<&str> = body
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with("//"))
        .filter_map(|l| l.strip_suffix(',')?.rsplit("::").next())
        .collect();
    assert_eq!(registered, app_acl::APP_COMMANDS);

    let caps: serde_json::Value =
        serde_json::from_str(include_str!("../capabilities/default.json")).unwrap_or_default();
    assert_eq!(caps["webviews"], serde_json::json!(["doc-*"]));
    // No window pattern: a capability matches a window label OR a webview
    // label, and the `win-*` windows / print windows must match nothing.
    assert!(caps.get("windows").is_none());
    // The native title is the shell's (windows.rs), not the webviews'.
    assert!(!caps.to_string().contains("allow-set-title"));
    let granted: Vec<String> = caps["permissions"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|p| p.as_str())
                .filter(|p| p.starts_with("allow-"))
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let expected: Vec<String> = app_acl::APP_COMMANDS
        .iter()
        .map(|c| format!("allow-{}", c.replace('_', "-")))
        .collect();
    assert_eq!(granted, expected);
    // The other capability grants no app command.
    let titlebar = include_str!("../capabilities/windows-titlebar.json");
    assert!(!titlebar.contains("\"allow-"));
    let titlebar: serde_json::Value = serde_json::from_str(titlebar).unwrap_or_default();
    assert_eq!(titlebar["webviews"], serde_json::json!(["doc-*"]));
    assert!(titlebar.get("windows").is_none());
}

/// A CSV save waiting for its Text Export options is dropped with the
/// document (review L5).
#[test]
fn closing_a_document_drops_its_pending_csv_save() -> CmdResult<()> {
    let st = state();
    let info = new_workbook(&st, None)?;
    crate::state::lock(&st.pending_csv).insert(info.doc.0, temp_path("p.csv"));
    crate::commands::close_engine_doc(&st, info.doc)?;
    assert!(crate::state::lock(&st.pending_csv).is_empty());
    Ok(())
}

// -- Document lock files and read-only mode (doclock.rs) ----------------------

fn lock_test_state() -> AppState {
    let st = state();
    st.locks.set_identity(crate::doclock::Identity {
        name: "Test User".into(),
        sys_user: "tester".into(),
        host: "test-host".into(),
        user_url: "file:///test/tally".into(),
    });
    st
}

/// A saved one-cell ODS file in its own scratch directory.
fn saved_book(st: &AppState, name: &str) -> CmdResult<PathBuf> {
    let dir = temp_path(name);
    std::fs::create_dir_all(&dir).map_err(|e| ApiError::new(ApiError::IO, e.to_string()))?;
    let path = dir.join("Book.ods");
    let info = new_workbook(st, None)?;
    set(st, &info, 0, 0, "42")?;
    save_to(st, info.doc, &path, FileFormat::Ods)?;
    crate::commands::close_engine_doc(st, info.doc)?;
    crate::state::lock(&st.paths).remove(&info.doc.0);
    Ok(std::fs::canonicalize(&path).unwrap_or(path))
}

const AOO_LOCK: &[u8] =
    b"Jane Doe,jane,other-mac.local,02.11.2026 15:04,file:///Users/jane/Library/Application%20Support/OpenOffice/4;";

#[test]
fn open_reports_a_foreign_lock_and_its_answers_work() -> CmdResult<()> {
    use crate::commands::{open_with_mode, OpenMode};
    let st = lock_test_state();
    let path = saved_book(&st, "foreign")?;
    let lp = crate::doclock::lock_path(&path).unwrap_or_default();
    assert!(!lp.exists(), "closing removed our lock");
    std::fs::write(&lp, AOO_LOCK).map_err(|e| ApiError::new(ApiError::IO, e.to_string()))?;

    let err = open_with_mode(&st, &path, OpenMode::Normal).unwrap_err();
    assert_eq!(err.code, ApiError::LOCKED);
    let v: serde_json::Value = serde_json::from_str(&err.message).unwrap_or_default();
    assert_eq!(v["owner"], "Jane Doe");
    assert_eq!(v["time"], "02.11.2026 15:04");
    assert_eq!(v["file"], "Book.ods");
    assert_eq!(v["own"], false);
    // "Open" is only for our own account's lock.
    assert_eq!(
        open_with_mode(&st, &path, OpenMode::IgnoreLock)
            .unwrap_err()
            .code,
        ApiError::LOCKED
    );

    // Open Read-Only: no lock taken, edits refused, Save refused.
    let ro = open_with_mode(&st, &path, OpenMode::ReadOnly)?;
    assert!(ro.read_only);
    assert!(!st.locks.holds(ro.doc.0));
    let tx = Transaction {
        ops: vec![Op::SetCellInput {
            sheet: ro.active_sheet,
            r: 0,
            c: 0,
            input: "x".into(),
        }],
        label: None,
        selection_before: None,
        selection_after: None,
        group: None,
    };
    let e = with_doc(&st, ro.doc, |s| s.apply(ro.doc, tx)).unwrap_err();
    assert_eq!(e.code, ApiError::READ_ONLY);
    // Edit Mode: still locked by Jane.
    let e = crate::commands::set_edit_mode(&st, ro.doc, false, crate::commands::EditMode::Plain)
        .unwrap_err();
    assert_eq!(e.code, ApiError::LOCKED);
    crate::commands::close_engine_doc(&st, ro.doc)?;

    // Open Copy: untitled, editable, no path, no lock.
    let copy = open_with_mode(&st, &path, OpenMode::Copy)?;
    assert!(!copy.read_only);
    assert_eq!(copy.path, None);
    crate::commands::close_engine_doc(&st, copy.doc)?;

    // Jane's lock was never touched.
    assert_eq!(std::fs::read(&lp).unwrap_or_default(), AOO_LOCK);
    Ok(())
}

#[test]
fn open_locks_close_unlocks_and_save_as_moves_the_lock() -> CmdResult<()> {
    use crate::commands::{open_with_mode, OpenMode};
    let st = lock_test_state();
    let path = saved_book(&st, "move")?;
    let lp = crate::doclock::lock_path(&path).unwrap_or_default();
    let info = open_with_mode(&st, &path, OpenMode::Normal)?;
    assert!(!info.read_only);
    let text = std::fs::read_to_string(&lp).unwrap_or_default();
    assert!(text.starts_with("Test User,tester,test-host,"), "{text}");
    assert!(text.ends_with(",file:///test/tally;"), "{text}");

    // Save As: the lock moves with the document.
    let other = path.with_file_name("Other.ods");
    save_to(&st, info.doc, &other, FileFormat::Ods)?;
    assert!(!lp.exists(), "old lock removed");
    let other_lp = crate::doclock::lock_path(&other).unwrap_or_default();
    assert!(other_lp.exists(), "new lock written");

    // Save As onto a file Jane is editing is refused, her lock kept.
    let janes = path.with_file_name("Janes.ods");
    std::fs::write(
        crate::doclock::lock_path(&janes).unwrap_or_default(),
        AOO_LOCK,
    )
    .map_err(|e| ApiError::new(ApiError::IO, e.to_string()))?;
    let e = save_to(&st, info.doc, &janes, FileFormat::Ods).unwrap_err();
    assert_eq!(e.code, ApiError::LOCKED);
    assert!(!janes.exists());
    assert!(other_lp.exists(), "still holding our lock");

    // Close removes it.
    crate::commands::close_engine_doc(&st, info.doc)?;
    assert!(!other_lp.exists());
    Ok(())
}

#[test]
fn our_stale_lock_is_taken_over_and_own_lock_elsewhere_is_asked() -> CmdResult<()> {
    use crate::commands::{open_with_mode, OpenMode};
    let st = lock_test_state();
    let path = saved_book(&st, "stale")?;
    let lp = crate::doclock::lock_path(&path).unwrap_or_default();
    // A crash left our lock: opened for editing without asking.
    std::fs::write(
        &lp,
        b"Test User,tester,test-host,01.01.2026 10:00,file:///test/tally;",
    )
    .map_err(|e| ApiError::new(ApiError::IO, e.to_string()))?;
    let info = open_with_mode(&st, &path, OpenMode::Normal)?;
    assert!(st.locks.holds(info.doc.0));
    crate::commands::close_engine_doc(&st, info.doc)?;
    assert!(!lp.exists());
    // Our account in OpenOffice: asked ("locked by yourself"), then Open.
    std::fs::write(
        &lp,
        b"Test User,tester,test-host,01.01.2026 10:00,file:///Users/tester/OpenOffice/4;",
    )
    .map_err(|e| ApiError::new(ApiError::IO, e.to_string()))?;
    let err = open_with_mode(&st, &path, OpenMode::Normal).unwrap_err();
    let v: serde_json::Value = serde_json::from_str(&err.message).unwrap_or_default();
    assert_eq!(v["own"], true);
    let info = open_with_mode(&st, &path, OpenMode::IgnoreLock)?;
    assert!(!info.read_only);
    crate::commands::close_engine_doc(&st, info.doc)?;
    assert!(!lp.exists());
    Ok(())
}

#[test]
fn edit_mode_round_trip_and_unwritable_files() -> CmdResult<()> {
    use crate::commands::{open_with_mode, set_edit_mode, EditMode, OpenMode};
    let st = lock_test_state();
    let path = saved_book(&st, "editmode")?;
    let lp = crate::doclock::lock_path(&path).unwrap_or_default();
    let info = open_with_mode(&st, &path, OpenMode::Normal)?;
    let doc = info.doc;
    // To read-only with unsaved changes needs Save or Discard first.
    set(&st, &info, 1, 0, "unsaved")?;
    assert!(set_edit_mode(&st, doc, true, EditMode::Plain).is_err());
    let r = set_edit_mode(&st, doc, true, EditMode::Discard)?;
    assert!(r.info.read_only && r.reloaded && !r.info.dirty);
    assert!(!lp.exists(), "read-only holds no lock");
    // Back to editing: locked again, not reloaded (unchanged on disk).
    let r = set_edit_mode(&st, doc, false, EditMode::Plain)?;
    assert!(!r.info.read_only && !r.reloaded);
    assert!(lp.exists());
    // Changed on disk while read-only: reloaded when switching back.
    set_edit_mode(&st, doc, true, EditMode::Plain)?;
    std::thread::sleep(std::time::Duration::from_millis(20));
    let writer = new_workbook(&st, None)?;
    set(&st, &writer, 0, 0, "from elsewhere")?;
    with_doc(&st, writer.doc, |s| {
        s.save_copy(writer.doc, &path, FileFormat::Ods)
    })?;
    crate::commands::close_engine_doc(&st, writer.doc)?;
    let r = set_edit_mode(&st, doc, false, EditMode::Plain)?;
    assert!(r.reloaded);
    let view = cells(&st, doc, r.info.active_sheet, range(0, 0, 0, 0))?;
    assert_eq!(
        cell(&view, 0, 0).map(|v| v.text.as_str()),
        Some("from elsewhere")
    );

    // A file that cannot be written: opens read-only, Edit Mode refuses.
    set_edit_mode(&st, doc, true, EditMode::Plain)?;
    let mut p = std::fs::metadata(&path)
        .map_err(|e| ApiError::new(ApiError::IO, e.to_string()))?
        .permissions();
    p.set_readonly(true);
    std::fs::set_permissions(&path, p).map_err(|e| ApiError::new(ApiError::IO, e.to_string()))?;
    let e = set_edit_mode(&st, doc, false, EditMode::Plain).unwrap_err();
    assert_eq!(e.code, ApiError::NOT_WRITABLE);
    let other = open_with_mode(&st, &path, OpenMode::Normal);
    // "edit a copy": untitled and editable.
    let r = set_edit_mode(&st, doc, false, EditMode::Copy)?;
    assert!(!r.info.read_only && r.info.path.is_none());
    let mut p = std::fs::metadata(&path)
        .map_err(|e| ApiError::new(ApiError::IO, e.to_string()))?
        .permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    p.set_readonly(false);
    std::fs::set_permissions(&path, p).map_err(|e| ApiError::new(ApiError::IO, e.to_string()))?;
    let other = other?;
    assert!(other.read_only, "an unwritable file opens read-only");
    assert!(!st.locks.holds(other.doc.0));
    crate::commands::close_engine_doc(&st, other.doc)?;
    crate::commands::close_engine_doc(&st, doc)?;
    assert!(!lp.exists());
    Ok(())
}

#[test]
fn save_as_onto_a_file_open_in_another_document_is_refused() -> CmdResult<()> {
    use crate::commands::{open_with_mode, OpenMode};
    let st = lock_test_state();
    let path = saved_book(&st, "saveas-open")?;
    let lp = crate::doclock::lock_path(&path).unwrap_or_default();
    let a = open_with_mode(&st, &path, OpenMode::Normal)?;
    let before = std::fs::read(&path).unwrap_or_default();
    let b = new_workbook(&st, None)?;
    set(&st, &b, 0, 0, "other")?;
    let e = save_to(&st, b.doc, &path, FileFormat::Ods).unwrap_err();
    assert_eq!(e.code, ApiError::LOCKED);
    let v: serde_json::Value = serde_json::from_str(&e.message).unwrap_or_default();
    assert_eq!(v["owner"], "Test User");
    assert_eq!(v["own"], false);
    assert_eq!(
        std::fs::read(&path).unwrap_or_default(),
        before,
        "not written"
    );
    // Closing the other document leaves A's lock alone.
    crate::commands::close_engine_doc(&st, b.doc)?;
    assert!(lp.exists());
    // A read-only (unlocked) document's file is protected too.
    crate::commands::close_engine_doc(&st, a.doc)?;
    let ro = open_with_mode(&st, &path, OpenMode::ReadOnly)?;
    let c = new_workbook(&st, None)?;
    assert_eq!(
        save_to(&st, c.doc, &path, FileFormat::Ods)
            .unwrap_err()
            .code,
        ApiError::LOCKED
    );
    crate::commands::close_engine_doc(&st, ro.doc)?;
    crate::commands::close_engine_doc(&st, c.doc)?;
    Ok(())
}

#[test]
fn concurrent_opens_of_one_file_open_it_once() -> CmdResult<()> {
    use crate::commands::{open_unique, OpenMode};
    let st = lock_test_state();
    let path = saved_book(&st, "race")?;
    let lp = crate::doclock::lock_path(&path).unwrap_or_default();
    for _ in 0..8 {
        let results: Vec<CmdResult<WorkbookInfo>> = std::thread::scope(|s| {
            let hs: Vec<_> = (0..4)
                .map(|_| s.spawn(|| open_unique(&st, &path, OpenMode::Normal)))
                .collect();
            hs.into_iter()
                .map(|h| {
                    h.join()
                        .unwrap_or_else(|_| Err(ApiError::new(ApiError::INTERNAL, "panic")))
                })
                .collect()
        });
        let opened: Vec<&WorkbookInfo> = results.iter().filter_map(|r| r.as_ref().ok()).collect();
        assert_eq!(opened.len(), 1, "{results:?}");
        for r in &results {
            if let Err(e) = r {
                assert_eq!(e.code, ApiError::CANCELLED);
            }
        }
        let doc = opened[0].doc;
        assert!(st.locks.holds(doc.0));
        crate::commands::close_engine_doc(&st, doc)?;
        crate::state::lock(&st.paths).remove(&doc.0);
        assert!(!lp.exists(), "the one lock went with the one document");
    }
    Ok(())
}
