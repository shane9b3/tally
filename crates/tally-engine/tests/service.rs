//! The `DocumentService` surface.
#![allow(clippy::unwrap_used)]

mod common;

use common::{input, range, sel, Fixture};
use tally_api::{
    ApiError, CellPos, Direction, DocId, DocumentService, FileFormat, Op, PastePayload, RangeDto,
    StylePatch, Suggestion, ValueKind,
};
use tally_engine::EngineService;
use tally_types::Value;

#[test]
fn wp7_script() {
    // type 2 in A1, 3 in B1, =A1*B1 in C1 → 6; hint "A1 × B1"; bold; undo; Sum = 11.
    let mut f = Fixture::new();
    f.set("A1", "2");
    f.set("B1", "3");
    let r = f.set("C1", "=A1*B1");
    assert_eq!(f.value("C1"), Value::Number(6.0));
    assert_eq!(r.changed[0].sheet, f.sheet);
    let info = f.svc.cell_edit_info(f.doc, f.sheet, 0, 2).unwrap();
    assert_eq!(info.input, "=A1*B1");
    assert_eq!(info.hint.as_deref(), Some("A1 × B1"));
    let res = f.apply(vec![Op::ApplyStyle {
        sheet: f.sheet,
        ranges: vec![range("A1")],
        patch: StylePatch {
            bold: Some(true),
            ..StylePatch::default()
        },
    }]);
    assert!(res.undo.can_undo);
    let blocks = f.svc.cells(f.doc, f.sheet, &[range("A1:C1")]).unwrap();
    let a1 = &blocks[0].cells[0];
    assert_ne!(a1.style, 0);
    assert!(f.svc.styles(f.doc, &[a1.style]).unwrap()[0].bold);
    f.svc.undo(f.doc).unwrap();
    let blocks = f.svc.cells(f.doc, f.sheet, &[range("A1:C1")]).unwrap();
    assert_eq!(blocks[0].cells[0].style, 0);
    let stats = f
        .svc
        .selection_stats(f.doc, &sel(f.sheet, "A1:C1"))
        .unwrap();
    assert_eq!(stats.sum, Some(11.0));
    assert_eq!(stats.count, 3);
    assert_eq!(stats.count_numbers, 3);
    assert_eq!(stats.formatted_sum.as_deref(), Some("11"));
}

#[test]
fn cell_views_are_formatted() {
    let mut f = Fixture::new();
    f.set("A1", "15%");
    f.set("A2", "$1,200.50");
    f.set("A3", "10/19/2026");
    f.set("A4", "=1/0");
    f.set("A5", "'0123");
    f.set("A6", "TRUE");
    f.set("A7", "1234.5");
    assert_eq!(f.text("A1"), "15%");
    assert_eq!(f.value("A1"), Value::Number(0.15));
    assert_eq!(f.text("A2"), "$1,200.50");
    assert_eq!(f.text("A3"), "10/19/2026");
    assert_eq!(f.text("A4"), "#DIV/0!");
    assert_eq!(f.text("A5"), "0123");
    assert_eq!(f.text("A6"), "TRUE");
    let blocks = f.svc.cells(f.doc, f.sheet, &[range("A1:A7")]).unwrap();
    let cells = &blocks[0].cells;
    assert_eq!(cells.len(), 7);
    assert_eq!(cells[3].kind, ValueKind::Error);
    assert_eq!(
        cells[3].flags,
        tally_api::cell_flags::FORMULA | tally_api::cell_flags::ERROR
    );
    assert_eq!(
        cells[6].num,
        Some(1234.5),
        "General numbers carry the raw value"
    );
    assert_eq!(cells[0].num, None);
    // Formula bar text for values.
    assert_eq!(
        f.svc.cell_edit_info(f.doc, f.sheet, 0, 0).unwrap().input,
        "15%"
    );
    assert_eq!(
        f.svc.cell_edit_info(f.doc, f.sheet, 2, 0).unwrap().input,
        "10/19/2026"
    );
    assert_eq!(
        f.svc.cell_edit_info(f.doc, f.sheet, 4, 0).unwrap().input,
        "'0123"
    );
    // Negative currency in red.
    f.apply(vec![Op::SetNumberFormat {
        sheet: f.sheet,
        ranges: vec![range("B1")],
        code: "[$$-409]#,##0.00;[RED]-[$$-409]#,##0.00".into(),
    }]);
    f.set("B1", "-5");
    let b = f.svc.cells(f.doc, f.sheet, &[range("B1")]).unwrap();
    assert_eq!(b[0].cells[0].text, "-$5.00");
    assert!(b[0].cells[0].color.is_some());
}

#[test]
fn too_many_cells_per_call() {
    let f = Fixture::new();
    let err = f
        .svc
        .cells(f.doc, f.sheet, &[range("A1:Z1000")])
        .unwrap_err();
    assert_eq!(err.code, ApiError::INVALID_ARGUMENT);
}

#[test]
fn layout_widths_in_pixels() {
    let mut f = Fixture::new();
    f.apply(vec![Op::SetColWidth {
        sheet: f.sheet,
        start: 2,
        end: 3,
        px: 150.0,
    }]);
    let l = f.svc.sheet_layout(f.doc, f.sheet).unwrap();
    assert_eq!(l.default_col_width_px, 100.0);
    assert_eq!(l.default_row_height_px, 24.0);
    assert_eq!(l.col_runs.len(), 1);
    assert!((l.col_runs[0].size - 150.0).abs() < 0.05);
    assert_eq!(l.used, None);
    f.set("B3", "x");
    let l = f.svc.sheet_layout(f.doc, f.sheet).unwrap();
    assert_eq!(l.used, Some(range("B3")));
}

#[test]
fn navigate_edges() {
    let mut f = Fixture::new();
    for c in ["A1", "A2", "A3", "A7", "C1", "D1"] {
        f.set(c, "1");
    }
    let s = f.sheet;
    let nav = |f: &Fixture, r, c, d| f.svc.navigate_edge(f.doc, s, CellPos { r, c }, d).unwrap();
    assert_eq!(nav(&f, 0, 0, Direction::Down), CellPos { r: 2, c: 0 });
    assert_eq!(nav(&f, 2, 0, Direction::Down), CellPos { r: 6, c: 0 });
    assert_eq!(
        nav(&f, 6, 0, Direction::Down),
        CellPos { r: 1_048_575, c: 0 }
    );
    assert_eq!(nav(&f, 6, 0, Direction::Up), CellPos { r: 2, c: 0 });
    assert_eq!(nav(&f, 0, 0, Direction::Up), CellPos { r: 0, c: 0 });
    assert_eq!(nav(&f, 0, 0, Direction::Right), CellPos { r: 0, c: 2 });
    assert_eq!(nav(&f, 0, 2, Direction::Right), CellPos { r: 0, c: 3 });
    assert_eq!(nav(&f, 0, 3, Direction::Right), CellPos { r: 0, c: 16_383 });
    assert_eq!(nav(&f, 0, 3, Direction::Left), CellPos { r: 0, c: 2 });
}

#[test]
fn name_box() {
    let mut f = Fixture::new();
    f.apply(vec![Op::AddSheet {
        name: Some("Sheet2".into()),
        index: None,
    }]);
    let s2 = f.svc.document(f.doc).unwrap().wb.sheets[1].id;
    let r = f.svc.resolve_name_box(f.doc, "B12").unwrap();
    assert_eq!((r.sheet, r.ranges[0]), (f.sheet, range("B12")));
    assert_eq!(
        f.svc.resolve_name_box(f.doc, "a1:c5").unwrap().ranges[0],
        range("A1:C5")
    );
    assert_eq!(
        f.svc.resolve_name_box(f.doc, "Sheet2.A1").unwrap().sheet,
        s2
    );
    assert_eq!(
        f.svc.resolve_name_box(f.doc, "Sheet2!$B$2").unwrap().ranges[0],
        range("B2")
    );
    assert_eq!(
        f.svc.resolve_name_box(f.doc, "C:D").unwrap().ranges[0],
        RangeDto {
            r0: 0,
            c0: 2,
            r1: 1_048_575,
            c1: 3
        }
    );
    assert_eq!(
        f.svc.resolve_name_box(f.doc, "Total").unwrap_err().code,
        ApiError::NOT_FOUND
    );
    assert_eq!(
        f.svc.resolve_name_box(f.doc, "Nope.A1").unwrap_err().code,
        ApiError::NOT_FOUND
    );
}

#[test]
fn copy_paste_reanchors_formulas() {
    let mut f = Fixture::new();
    f.set("A1", "1");
    f.set("B1", "10");
    f.set("C1", "=A1+$B$1");
    f.apply(vec![Op::ApplyStyle {
        sheet: f.sheet,
        ranges: vec![range("C1")],
        patch: StylePatch {
            italic: Some(true),
            ..StylePatch::default()
        },
    }]);
    let clip = f.svc.copy(f.doc, &sel(f.sheet, "C1")).unwrap();
    assert!(clip
        .html
        .contains(&format!("data-tally-clip=\"{}\"", clip.token)));
    assert_eq!(clip.tsv, "11\n");
    f.apply(vec![Op::Paste {
        sheet: f.sheet,
        target: range("D5"),
        payload: PastePayload::Internal {
            token: clip.token.clone(),
        },
    }]);
    assert_eq!(
        f.svc.cell_edit_info(f.doc, f.sheet, 4, 3).unwrap().input,
        "=B5+$B$1"
    );
    let d5 = f.svc.cells(f.doc, f.sheet, &[range("D5")]).unwrap();
    assert!(f.svc.styles(f.doc, &[d5[0].cells[0].style]).unwrap()[0].italic);
    // Fill a 3-row target by repetition.
    f.apply(vec![Op::Paste {
        sheet: f.sheet,
        target: range("E1:E3"),
        payload: PastePayload::Internal { token: clip.token },
    }]);
    assert_eq!(
        f.svc.cell_edit_info(f.doc, f.sheet, 2, 4).unwrap().input,
        "=C3+$B$1"
    );
    let bad = f.apply(vec![Op::Paste {
        sheet: f.sheet,
        target: range("A9"),
        payload: PastePayload::Internal {
            token: "nope".into(),
        },
    }]);
    assert_eq!(bad.errors[0].code, ApiError::NOT_FOUND);
    // TSV goes through the recognizer.
    f.apply(vec![Op::Paste {
        sheet: f.sheet,
        target: range("G1"),
        payload: PastePayload::Tsv {
            text: "5\t50%\n=G1*2\tabc\n".into(),
        },
    }]);
    assert_eq!(f.value("G2"), Value::Number(10.0));
    assert_eq!(f.value("H1"), Value::Number(0.5));
}

#[test]
fn sheets_add_rename_delete_move() {
    let mut f = Fixture::new();
    let r = f.apply(vec![Op::AddSheet {
        name: None,
        index: None,
    }]);
    assert!(r.sheets_changed);
    let info = r.info.unwrap();
    assert_eq!(info.sheets[1].name, "Sheet2");
    let s2 = info.sheets[1].id;
    let r = f.apply(vec![Op::RenameSheet {
        sheet: s2,
        name: "sheet1".into(),
    }]);
    assert_eq!(
        r.errors[0].code,
        ApiError::INVALID_ARGUMENT,
        "names are unique, case-insensitively"
    );
    let r = f.apply(vec![Op::RenameSheet {
        sheet: s2,
        name: "Data".into(),
    }]);
    assert_eq!(r.info.unwrap().sheets[1].name, "Data");
    let r = f.apply(vec![Op::MoveSheet { sheet: s2, to: 0 }]);
    assert_eq!(r.info.unwrap().sheets[0].id, s2);
    let r = f.apply(vec![Op::DeleteSheet { sheet: s2 }]);
    assert_eq!(r.info.unwrap().sheets.len(), 1);
    let r = f.apply(vec![Op::DeleteSheet { sheet: f.sheet }]);
    assert_eq!(
        r.errors[0].code,
        ApiError::INVALID_ARGUMENT,
        "the last sheet stays"
    );
    let r = f.apply(vec![Op::AddSheet {
        name: None,
        index: None,
    }]);
    assert!(r.info.unwrap().sheets[1].id > s2, "ids are never reused");
}

#[test]
fn invoices_template() {
    let mut svc = EngineService::new();
    let info = svc.new_workbook(Some("invoices")).unwrap();
    let s = info.sheets[0].id;
    let doc = info.doc;
    let wb = &svc.document(doc).unwrap().wb;
    let used = wb.sheet(s).unwrap().used_range().unwrap();
    assert_eq!((used.r1, used.c1), (14, 6), "header + 14 rows, A:G");
    let edit = svc.cell_edit_info(doc, s, 1, 5).unwrap();
    assert_eq!(edit.input, "=D2*E2");
    assert_eq!(edit.hint.as_deref(), Some("Qty × Price"));
    let blocks = svc.cells(doc, s, &[range("F2")]).unwrap();
    assert_eq!(blocks[0].cells[0].text, "$54.00");
    let header = svc.cells(doc, s, &[range("A1")]).unwrap();
    assert!(svc.styles(doc, &[header[0].cells[0].style]).unwrap()[0].bold);
    // The assist golden formula evaluates.
    let mut svc2 = svc;
    svc2.apply(
        doc,
        common::tx(vec![input(s, "I1", "=SUMIF(G2:G15,\"<>Paid\",F2:F15)")]),
    )
    .unwrap();
    let v = svc2
        .document(doc)
        .unwrap()
        .wb
        .value(tally_types::CellAddr::new(s, 0, 8));
    assert!(matches!(v, Value::Number(n) if n > 0.0));
    let budget = svc2.new_workbook(Some("budget")).unwrap();
    assert_eq!(budget.sheets[0].name, "Budget");
    assert_eq!(
        svc2.new_workbook(Some("nope")).unwrap_err().code,
        ApiError::NOT_FOUND
    );
}

#[test]
fn errors_for_unknown_ids_and_files() {
    let mut svc = EngineService::new();
    let bad = DocId(99);
    assert_eq!(
        svc.sheet_layout(bad, 1).unwrap_err().code,
        ApiError::NOT_FOUND
    );
    assert_eq!(svc.undo(bad).unwrap_err().code, ApiError::NOT_FOUND);
    assert_eq!(svc.close(bad).unwrap_err().code, ApiError::NOT_FOUND);
    let info = svc.new_workbook(None).unwrap();
    assert_eq!(
        svc.sheet_layout(info.doc, 77).unwrap_err().code,
        ApiError::NOT_FOUND
    );
    assert_eq!(
        svc.save(info.doc, None, FileFormat::Ods).unwrap_err().code,
        ApiError::INVALID_ARGUMENT
    );
    let tmp = std::env::temp_dir().join("tally-engine-unsupported.ods");
    assert_eq!(
        svc.save(info.doc, Some(&tmp), FileFormat::Ods)
            .unwrap_err()
            .code,
        ApiError::UNSUPPORTED,
        "no codec wired without the io feature"
    );
    assert_eq!(
        svc.save(info.doc, Some(&tmp), FileFormat::Xlsx)
            .unwrap_err()
            .code,
        ApiError::UNSUPPORTED
    );
    svc.close(info.doc).unwrap();
}

#[test]
fn functions_and_assist_hook() {
    let mut f = Fixture::new();
    let fns = f.svc.list_functions();
    let sum = fns.iter().find(|x| x.name == "SUM").unwrap();
    assert_eq!(sum.category, "Mathematical");
    assert!(sum.signature.starts_with("SUM("));
    assert!(!sum.signature.contains(';'));
    let s = sel(f.sheet, "A1");
    assert!(f.svc.assist_suggest(f.doc, "total", &s).unwrap().is_empty());
    f.set("A1", "Paid");
    f.svc.set_assist_provider(Box::new(|req| {
        vec![Suggestion {
            title: format!("{} / {}", req.query, req.sheet.text(0, 0)),
            formula: None,
            explanation: String::new(),
            preview: None,
            confidence: 1.0,
            action: None,
        }]
    }));
    let out = f.svc.assist_suggest(f.doc, "total", &s).unwrap();
    assert_eq!(out[0].title, "total / Paid");
}

#[test]
fn edit_info_for_header_rows_and_text() {
    let mut f = Fixture::new();
    f.set("A1", "Amount");
    f.set("A2", "5");
    f.set("A3", "7");
    f.set("A4", "=SUM(A2:A3)");
    let e = f.svc.cell_edit_info(f.doc, f.sheet, 3, 0).unwrap();
    assert_eq!(e.input, "=SUM(A2:A3)");
    assert!(e.hint.is_some());
    f.set("B1", "=\"x\"&A1");
    assert_eq!(f.value("B1"), Value::text("xAmount"));
}

#[test]
fn service_is_send_for_the_tauri_mutex() {
    fn assert_send<T: Send>() {}
    assert_send::<EngineService>();
    let boxed: Box<dyn DocumentService + Send> = Box::new(EngineService::new());
    assert!(!boxed.list_functions().is_empty());
}
