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
use crate::commands::{new_workbook, open_path, save_to, with_service, CmdResult};
use crate::files;
use crate::state::AppState;

const SUMIF: &str = "=SUMIF(G2:G15,\"<>Paid\",F2:F15)";

fn state() -> AppState {
    AppState::new(Box::new(build_service()))
}

fn temp_path(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    std::env::temp_dir().join(format!(
        "tally-desktop-{}-{nanos}-{name}",
        std::process::id()
    ))
}

fn range(r0: u32, c0: u32, r1: u32, c1: u32) -> RangeDto {
    RangeDto { r0, c0, r1, c1 }
}

fn cells(st: &AppState, doc: DocId, sheet: SheetId, r: RangeDto) -> CmdResult<Vec<CellView>> {
    let blocks = with_service(st, |s| s.cells(doc, sheet, &[r]))?;
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
    };
    let res = with_service(st, |s| s.apply(info.doc, tx))?;
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
    let out = with_service(&st, |s| {
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
    let edit = with_service(&st, |s| s.cell_edit_info(info.doc, sheet, 16, 0))?;
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
    let edit = with_service(&st, |s| {
        s.cell_edit_info(reopened.doc, reopened.active_sheet, 16, 0)
    })?;
    assert_eq!(edit.input, SUMIF);
    let style_of = |st: &AppState, doc: DocId, v: Option<&CellView>| {
        let id = v.map_or(0, |v| v.style);
        with_service(st, |s| s.styles(doc, &[id]))
    };
    let h0 = style_of(&st, info.doc, cell(&before, 0, 0))?;
    let h1 = style_of(&st, reopened.doc, cell(&after, 0, 0))?;
    assert!(h0[0].bold && h1[0].bold, "header stays bold");
    let p0 = style_of(&st, info.doc, cell(&before, 1, 4))?;
    let p1 = style_of(&st, reopened.doc, cell(&after, 1, 4))?;
    // The code is normalised on read (`[$$-409]#,##0.00` comes back as
    // `$#,##0.00`); both render the same text, compared above.
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
    let out = with_service(&st, |s| {
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
    let err = with_service(&st, |_| -> CmdResult<()> { panic!("injected test panic") })
        .err()
        .map(|e| (e.code, e.message));
    assert_eq!(
        err,
        Some((
            ApiError::INTERNAL.to_owned(),
            "the engine failed: injected test panic".to_owned()
        ))
    );
    assert!(!st.service.is_poisoned());
    set(&st, &info, 0, 0, "42")?;
    let v = cells(&st, info.doc, info.active_sheet, range(0, 0, 0, 0))?;
    assert_eq!(cell(&v, 0, 0).map(|c| c.text.as_str()), Some("42"));
    Ok(())
}

/// Security review M6: even a poisoned lock (a panic outside
/// `with_service` while it was held) is recovered instead of disabling the
/// app.
#[test]
fn poisoned_service_lock_is_recovered() -> CmdResult<()> {
    let st = std::sync::Arc::new(state());
    let st2 = std::sync::Arc::clone(&st);
    let _ = std::thread::spawn(move || {
        let _guard = st2.service.lock();
        panic!("poison the lock");
    })
    .join();
    assert!(st.service.is_poisoned());
    let info = new_workbook(&st, None)?;
    assert_eq!(info.sheets.len(), 1);
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

// ----- v0.2 over the real engine ------------------------------------------------

fn apply_ok(st: &AppState, doc: DocId, ops: Vec<Op>) -> CmdResult<tally_api::ApplyResult> {
    let tx = Transaction {
        ops,
        label: Some("test".into()),
        selection_before: None,
        selection_after: None,
    };
    let res = with_service(st, |s| s.apply(doc, tx))?;
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
    let header = with_service(&st, |s| s.detect_header(info.doc, sheet, table))?;
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
    let fi = with_service(&st, |s| s.filter_info(info.doc, sheet))?;
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
    let layout = with_service(&st, |s| s.sheet_layout(info.doc, sheet))?;
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
    let layout = with_service(&st, |s| s.sheet_layout(doc, sheet))?;
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
    let names = with_service(&st, |s| s.names(doc))?;
    assert_eq!(names.len(), 1);
    assert_eq!(names[0].name, "TaxRate");
    set(&st, &info, 1, 7, "=F2*TaxRate")?;
    let view = cells(&st, doc, sheet, range(1, 5, 1, 7))?;
    let f2 = cell(&view, 1, 5).and_then(number).unwrap_or(f64::NAN);
    let h2 = cell(&view, 1, 7).and_then(number).unwrap_or(f64::NAN);
    assert!((h2 - f2 * 0.1).abs() < 1e-9, "{h2} vs {f2}");
    let edit = with_service(&st, |s| s.cell_edit_info(doc, sheet, 1, 7))?;
    assert_eq!(edit.input, "=F2*TaxRate");

    // Find "Paid": eight status cells.
    let found = with_service(&st, |s| {
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
    let edit = with_service(&st, |s| {
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
    let res = with_service(&st, |s| s.recalc_all(info.doc))?;
    assert!(res.errors.is_empty());
    let fi = with_service(&st, |s| s.filter_info(info.doc, info.active_sheet))?;
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
    }
}

/// Applies without asserting success (validation / protection refusals).
fn apply_any(st: &AppState, doc: DocId, ops: Vec<Op>) -> CmdResult<tally_api::ApplyResult> {
    with_service(st, |s| s.apply(doc, tx_of(ops)))
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
    let listed = with_service(&st, |s| s.conditional_formats(info.doc, sheet))?;
    assert_eq!(listed.len(), 1);
    let view = cells(&st, info.doc, sheet, range(1, 5, 14, 5))?;
    let mut hits = 0;
    for v in &view {
        let big = number(v).is_some_and(|n| n > 50.0);
        match v.cf_style {
            Some(id) => {
                assert!(big, "row {} has a cf style but is not > 50", v.r);
                assert!(id >= 0x4000_0000);
                let styles = with_service(&st, |s| s.styles(info.doc, &[id]))?;
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
    let at = with_service(&st, |s| s.validation_at(info.doc, sheet, 1, 3))?;
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
    let back = with_service(&st, |s| s.note(reopened.doc, reopened.active_sheet, 2, 1))?
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
    let p = with_service(&st, |s| s.protection_info(info.doc, sheet))?;
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
    let res = with_service(&st, |s| s.goal_seek(info.doc, &req))?;
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
    let res = with_service(&st, |s| s.apply(info.doc, ok))?;
    assert!(res.errors.is_empty());
    Ok(())
}
