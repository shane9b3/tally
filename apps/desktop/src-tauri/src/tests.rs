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
