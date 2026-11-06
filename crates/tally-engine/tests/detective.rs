//! Tools ▸ Detective as AOO 4.1 runs it (`detfunc.cxx`, `docfunc.cxx`,
//! `detdata.cxx`): one level per Trace, the outermost level per Remove,
//! the operation list replayed by Refresh Traces and AutoRefresh, moved
//! with rows and columns, and saved in ODS `table:detective`.
#![allow(clippy::unwrap_used)]

mod common;

use common::{input, Fixture};
use tally_api::{
    CellPos, DetectiveAction as A, DetectiveArrowDto, DetectiveState, DocumentService, Op, RangeDto,
};
use tally_types::CellAddr;

fn at(a1: &str) -> (u32, u32) {
    let a = CellAddr::from_a1(1, a1).unwrap();
    (a.row, a.col)
}

impl Fixture {
    fn det(&mut self, a1: &str, action: A) -> DetectiveState {
        let (r, c) = at(a1);
        self.svc
            .detective(self.doc, self.sheet, r, c, action)
            .unwrap()
    }

    fn arrows(&self) -> Vec<DetectiveArrowDto> {
        self.svc
            .detective_arrows(self.doc, self.sheet)
            .unwrap()
            .arrows
    }
}

/// `(from top-left, to)` as A1 text, sorted.
fn pairs(arrows: &[DetectiveArrowDto]) -> Vec<String> {
    let name = |r: u32, c: u32| tally_types::CellAddr::new(1, r, c).to_a1();
    let mut v: Vec<String> = arrows
        .iter()
        .map(|a| {
            let f = a.from.map_or("*".into(), |f| name(f.r0, f.c0));
            let t = a.to.map_or("*".into(), |t| name(t.r, t.c));
            format!("{f}>{t}")
        })
        .collect();
    v.sort();
    v
}

fn chain() -> Fixture {
    let mut f = Fixture::new();
    f.set("A1", "1");
    f.set("B1", "=A1");
    f.set("C1", "=B1");
    f.set("D1", "=C1+A1");
    f
}

#[test]
fn repeated_trace_precedents_adds_one_level_each_time() {
    let mut f = chain();
    f.svc.document_mut(f.doc).unwrap().dirty = false;
    let s = f.det("D1", A::TracePrecedents);
    assert!(s.done && s.has_ops);
    // A command that did something modifies the document (AOO
    // `ScDocShellModificator::SetDocumentModified`) and says so.
    assert!(s.dirty && f.svc.is_dirty(f.doc).unwrap());
    assert_eq!(pairs(&s.arrows), ["A1>D1", "C1>D1"]);
    let s = f.det("D1", A::TracePrecedents);
    assert_eq!(pairs(&s.arrows), ["A1>D1", "B1>C1", "C1>D1"]);
    let s = f.det("D1", A::TracePrecedents);
    assert_eq!(pairs(&s.arrows), ["A1>B1", "A1>D1", "B1>C1", "C1>D1"]);
    // Nothing deeper: not recorded.
    assert!(!f.det("D1", A::TracePrecedents).done);
    // Remove Precedents takes the outermost level away, one per call.
    let s = f.det("D1", A::RemovePrecedents);
    assert!(s.done);
    assert_eq!(pairs(&s.arrows), ["A1>D1", "B1>C1", "C1>D1"]);
    let s = f.det("D1", A::RemovePrecedents);
    assert_eq!(pairs(&s.arrows), ["A1>D1", "C1>D1"]);
    let s = f.det("D1", A::RemovePrecedents);
    assert!(s.arrows.is_empty());
    assert!(!f.det("D1", A::RemovePrecedents).done);
    // A constant has no precedents.
    assert!(!f.det("A1", A::TracePrecedents).done);
}

#[test]
fn trace_and_remove_dependents_by_level() {
    let mut f = chain();
    let s = f.det("A1", A::TraceDependents);
    assert_eq!(pairs(&s.arrows), ["A1>B1", "A1>D1"]);
    let s = f.det("A1", A::TraceDependents);
    assert_eq!(pairs(&s.arrows), ["A1>B1", "A1>D1", "B1>C1"]);
    let s = f.det("A1", A::TraceDependents);
    assert_eq!(pairs(&s.arrows), ["A1>B1", "A1>D1", "B1>C1", "C1>D1"]);
    let s = f.det("A1", A::RemoveDependents);
    assert_eq!(pairs(&s.arrows), ["A1>B1", "A1>D1", "B1>C1"]);
    let s = f.det("A1", A::RemoveDependents);
    assert_eq!(pairs(&s.arrows), ["A1>B1", "A1>D1"]);
    assert!(f.det("A1", A::RemoveDependents).arrows.is_empty());
}

#[test]
fn area_references_draw_one_arrow_from_the_range() {
    let mut f = Fixture::new();
    f.set("A1", "1");
    f.set("A2", "=1/0");
    f.set("B1", "=SUM(A1:A3)");
    let s = f.det("B1", A::TracePrecedents);
    assert_eq!(s.arrows.len(), 1);
    let a = s.arrows[0];
    assert_eq!(
        a.from,
        Some(RangeDto {
            r0: 0,
            c0: 0,
            r1: 2,
            c1: 0
        })
    );
    assert_eq!(a.to, Some(CellPos { r: 0, c: 1 }));
    assert!(a.error, "the range holds an error");
    // The next level follows the formula cells inside the range.
    f.set("A3", "=A1");
    let s = f.det("B1", A::TracePrecedents);
    assert_eq!(pairs(&s.arrows), ["A1>A3", "A1>B1"]);
}

#[test]
fn insert_and_delete_rows_move_operations_and_arrows() {
    let mut f = chain();
    f.det("D1", A::TracePrecedents);
    let sheet = f.sheet;
    let r = f.apply(vec![Op::InsertRows {
        sheet,
        at: 0,
        count: 2,
    }]);
    assert_eq!(r.detective_changed, Some(true));
    assert_eq!(pairs(&f.arrows()), ["A3>D3", "C3>D3"]);
    // Refresh replays the moved operation (at D3, not D1).
    let s = f.det("A1", A::Refresh);
    assert!(s.done);
    assert_eq!(pairs(&s.arrows), ["A3>D3", "C3>D3"]);
    let ops = &f.svc.document(f.doc).unwrap().wb.sheets[0]
        .meta
        .detective
        .ops;
    assert_eq!((ops[0].row, ops[0].col), (2, 3));
    // Deleting the operation's row drops it.
    f.apply(vec![Op::DeleteRows {
        sheet,
        at: 2,
        count: 1,
    }]);
    assert!(f.arrows().is_empty());
    assert!(!f.det("A1", A::Refresh).has_ops);
}

#[test]
fn auto_refresh_redraws_after_formula_edits() {
    let mut f = chain();
    let s = f.det("D1", A::TracePrecedents);
    assert_eq!(s.arrows.len(), 2);
    // Removing a precedent from the formula removes its arrow.
    let r = f.set("D1", "=C1");
    assert_eq!(r.detective_changed, Some(true));
    assert_eq!(pairs(&f.arrows()), ["C1>D1"]);
    // A new precedent shows up.
    f.set("D1", "=C1+E5");
    assert_eq!(pairs(&f.arrows()), ["C1>D1", "E5>D1"]);
    // Typing a constant does not replay (no formula changed, no Trace Error).
    let r = f.set("Z9", "5");
    assert_eq!(r.detective_changed, None);
}

#[test]
fn auto_refresh_off_keeps_arrows_until_refresh() {
    let mut f = chain();
    f.svc.set_detective_auto(f.doc, false).unwrap();
    f.det("D1", A::TracePrecedents);
    let r = f.set("D1", "=C1");
    assert_eq!(r.detective_changed, None);
    assert_eq!(
        pairs(&f.arrows()),
        ["A1>D1", "C1>D1"],
        "stale until refreshed"
    );
    let s = f.det("A1", A::Refresh);
    assert_eq!(pairs(&s.arrows), ["C1>D1"]);
    // Back on: the next formula change redraws.
    f.svc.set_detective_auto(f.doc, true).unwrap();
    f.set("D1", "=B1");
    assert_eq!(pairs(&f.arrows()), ["B1>D1"]);
}

#[test]
fn trace_error_follows_errors_and_auto_refresh_tracks_values() {
    let mut f = Fixture::new();
    f.set("A1", "0");
    f.set("A2", "=1/A1");
    f.set("A3", "=A2+B3");
    f.set("A4", "=A3*2");
    let s = f.det("A4", A::TraceError);
    assert!(s.done);
    assert_eq!(pairs(&s.arrows), ["A1>A2", "A2>A3", "A3>A4"]);
    assert!(s
        .arrows
        .iter()
        .filter(|a| a.to == Some(CellPos { r: 1, c: 0 }))
        .all(|a| !a.error));
    assert!(s
        .arrows
        .iter()
        .filter(|a| a.to != Some(CellPos { r: 1, c: 0 }))
        .all(|a| a.error));
    // A Trace Error in the list redraws after a value change too.
    let r = f.set("A1", "2");
    assert_eq!(r.detective_changed, Some(true));
    assert!(f.arrows().is_empty(), "no error left");
    // A cell without an error is not traced.
    assert!(!f.det("A4", A::TraceError).done);
}

#[test]
fn remove_all_clears_the_operation_list() {
    let mut f = chain();
    f.det("D1", A::TracePrecedents);
    f.det("A1", A::TraceDependents);
    let s = f.det("A1", A::RemoveAll);
    assert!(s.done && s.arrows.is_empty() && !s.has_ops);
    assert!(!f.det("A1", A::Refresh).done);
}

#[test]
fn arrows_to_and_from_other_sheets() {
    let mut f = Fixture::new();
    let r = f.apply(vec![Op::AddSheet {
        name: Some("Two".into()),
        index: None,
    }]);
    let two = r.info.unwrap().sheets[1].id;
    f.set("A1", "4");
    f.apply(vec![input(two, "B2", "=$Sheet1.A1*2")]);
    // On Two: an arrow from another sheet into B2.
    let s = f
        .svc
        .detective(f.doc, two, 1, 1, A::TracePrecedents)
        .unwrap();
    assert_eq!(s.arrows.len(), 1);
    assert_eq!(s.arrows[0].from, None);
    assert_eq!(s.arrows[0].to, Some(CellPos { r: 1, c: 1 }));
    // On Sheet1: an arrow from A1 to another sheet.
    let s = f.det("A1", A::TraceDependents);
    assert_eq!(s.arrows.len(), 1);
    assert_eq!(s.arrows[0].to, None);
    assert_eq!(s.arrows[0].from.map(|r| (r.r0, r.c0)), Some((0, 0)));
    // Each sheet keeps its own arrows; Refresh replays both.
    let s = f.det("A1", A::Refresh);
    assert_eq!(s.arrows.len(), 1);
    assert_eq!(f.svc.detective_arrows(f.doc, two).unwrap().arrows.len(), 1);
}

#[test]
fn circular_references_terminate() {
    let mut f = Fixture::new();
    f.set("A1", "=B1");
    f.set("B1", "=A1");
    for _ in 0..4 {
        f.det("A1", A::TracePrecedents);
        f.det("A1", A::TraceDependents);
    }
    assert_eq!(pairs(&f.arrows()), ["A1>B1", "B1>A1"]);
}

#[test]
fn replay_is_bounded_on_large_sheets() {
    let mut f = Fixture::new();
    let sheet = f.sheet;
    // 20,000 formulas read by one SUM, each reading its own constant.
    let n = 20_000u32;
    let mut ops: Vec<Op> = Vec::new();
    for r in 0..n {
        ops.push(Op::SetCellInput {
            sheet,
            r,
            c: 0,
            input: format!("=B{}", r + 1),
        });
    }
    ops.push(input(sheet, "D1", &format!("=SUM(A1:A{n})")));
    f.apply(ops);
    let t = std::time::Instant::now();
    f.det("D1", A::TracePrecedents);
    let s = f.det("D1", A::TracePrecedents);
    assert!(s.arrows.len() > 1 && s.arrows.len() <= tally_engine::detective::MAX_TRACES);
    // AutoRefresh on a formula edit replays within its budget.
    f.set("E1", "=D1");
    assert!(
        t.elapsed() < std::time::Duration::from_secs(30),
        "{:?}",
        t.elapsed()
    );
}

#[cfg(feature = "io")]
#[test]
fn ods_round_trip_of_table_detective() {
    use tally_api::FileFormat;
    use tally_engine::EngineService;
    let mut svc = EngineService::with_default_codec();
    let info = svc.new_workbook(None).unwrap();
    let (doc, sheet) = (info.doc, info.sheets[0].id);
    for (a1, t) in [("A1", "1"), ("B1", "=A1"), ("C1", "=B1"), ("D1", "=C1+A1")] {
        svc.apply(doc, common::tx(vec![input(sheet, a1, t)]))
            .unwrap();
    }
    svc.detective(doc, sheet, 0, 3, A::TracePrecedents).unwrap();
    svc.detective(doc, sheet, 0, 3, A::TracePrecedents).unwrap();
    svc.detective(doc, sheet, 0, 3, A::RemovePrecedents)
        .unwrap();
    svc.detective(doc, sheet, 0, 0, A::TraceDependents).unwrap();
    let before = pairs(&svc.detective_arrows(doc, sheet).unwrap().arrows);
    assert!(svc.is_dirty(doc).unwrap());
    let dir = std::env::temp_dir().join(format!("tally-detective-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("det.ods");
    svc.save(doc, Some(&p), FileFormat::Ods).unwrap();
    // The operations are in content.xml as AOO writes them.
    let bytes = std::fs::read(&p).unwrap();
    let content = tally_io_ods_content(&bytes);
    assert!(
        content.contains(r#"<table:operation table:name="trace-precedents" table:index="0"/>"#),
        "{content}"
    );
    assert!(content.contains(r#"table:name="remove-precedents" table:index="2""#));
    assert!(content.contains(r#"table:name="trace-dependents" table:index="3""#));
    assert!(content.contains(r#"table:direction="from-same-table""#));
    let back = svc.open(&p).unwrap();
    let s2 = back.sheets[0].id;
    assert_eq!(
        pairs(&svc.detective_arrows(back.doc, s2).unwrap().arrows),
        before
    );
    let ops = svc.document(back.doc).unwrap().wb.sheets[0]
        .meta
        .detective
        .ops
        .clone();
    assert_eq!(ops.len(), 4);
    // Refresh after load replays the same list.
    let s = svc.detective(back.doc, s2, 0, 0, A::Refresh).unwrap();
    assert_eq!(pairs(&s.arrows), before);
    // XLSX has no equivalent: nothing is written, the file still opens.
    let x = dir.join("det.xlsx");
    svc.save(back.doc, Some(&x), FileFormat::Xlsx).unwrap();
    let xb = svc.open(&x).unwrap();
    assert!(svc
        .detective_arrows(xb.doc, xb.sheets[0].id)
        .unwrap()
        .arrows
        .is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

/// `content.xml` of an ODS package.
#[cfg(feature = "io")]
fn tally_io_ods_content(bytes: &[u8]) -> String {
    use std::io::Read;
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut s = String::new();
    z.by_name("content.xml")
        .unwrap()
        .read_to_string(&mut s)
        .unwrap();
    s
}

#[test]
fn undo_of_a_row_change_keeps_later_traces() {
    let mut f = chain();
    let sheet = f.sheet;
    f.apply(vec![Op::InsertRows {
        sheet,
        at: 0,
        count: 1,
    }]);
    f.det("D2", A::TracePrecedents);
    f.det("D2", A::TracePrecedents);
    assert_eq!(pairs(&f.arrows()), ["A2>D2", "B2>C2", "C2>D2"]);
    // Undo moves them back with the cells (AOO's list survives undo).
    let r = f.svc.undo(f.doc).unwrap();
    assert_eq!(r.detective_changed, Some(true));
    assert_eq!(pairs(&f.arrows()), ["A1>D1", "B1>C1", "C1>D1"]);
    let ops = f.svc.document(f.doc).unwrap().wb.sheets[0]
        .meta
        .detective
        .ops
        .clone();
    assert_eq!(ops.len(), 2);
    assert!(ops.iter().all(|o| (o.row, o.col) == (0, 3)));
    f.svc.redo(f.doc).unwrap();
    assert_eq!(pairs(&f.arrows()), ["A2>D2", "B2>C2", "C2>D2"]);
}

/// A long error chain traced many times, then Remove Precedents: the level
/// search is iterative and replay (also on open) runs on its own stack,
/// so a 2 MiB thread (a Tauri worker) does not overflow.
#[cfg(feature = "io")]
#[test]
fn deep_chains_do_not_overflow_a_small_stack() {
    let h = std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(|| {
            use tally_api::FileFormat;
            let mut svc = tally_engine::EngineService::with_default_codec();
            let info = svc.new_workbook(None).unwrap();
            let (doc, sh) = (info.doc, info.sheets[0].id);
            let n = 12_000u32;
            let rows: Vec<Vec<String>> = (0..n)
                .map(|i| {
                    vec![if i == 0 {
                        "=1/0".into()
                    } else {
                        format!("=B{i}")
                    }]
                })
                .collect();
            svc.apply(
                doc,
                common::tx(vec![Op::SetRangeInputs {
                    sheet: sh,
                    r: 0,
                    c: 1,
                    rows,
                }]),
            )
            .unwrap();
            for k in 0..11 {
                svc.detective(doc, sh, n - 1 - k * 999, 1, A::TraceError)
                    .unwrap();
            }
            svc.detective(doc, sh, n - 1, 1, A::RemovePrecedents)
                .unwrap();
            svc.detective(doc, sh, 0, 1, A::TraceDependents).unwrap();
            svc.detective(doc, sh, 0, 1, A::RemoveDependents).unwrap();
            let dir = std::env::temp_dir().join(format!("tally-det-deep-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let p = dir.join("deep.ods");
            svc.save(doc, Some(&p), FileFormat::Ods).unwrap();
            let back = svc.open(&p).unwrap();
            assert!(
                svc.detective(back.doc, back.sheets[0].id, 0, 0, A::Refresh)
                    .unwrap()
                    .done
            );
            let _ = std::fs::remove_dir_all(&dir);
        })
        .unwrap();
    h.join().unwrap();
}

/// AutoRefresh with 1,000 operations on a 50,000-formula chain stays
/// within its budget and says when it stopped early.
#[test]
fn auto_refresh_is_bounded_and_marks_incomplete_arrows() {
    use std::sync::Arc;
    use tally_engine::detective::{DetOp, OpKind, SheetDetective};
    let mut f = Fixture::new();
    let sheet = f.sheet;
    let n = 50_000u32;
    let rows: Vec<Vec<String>> = (0..n)
        .map(|i| {
            vec![if i == 0 {
                "=A1".into()
            } else {
                format!("=B{i}")
            }]
        })
        .collect();
    f.apply(vec![
        input(sheet, "A1", "1"),
        Op::SetRangeInputs {
            sheet,
            r: 0,
            c: 1,
            rows,
        },
    ]);
    let ops = vec![
        DetOp {
            row: 0,
            col: 0,
            kind: OpKind::TraceDependents,
        };
        1_000
    ];
    f.svc.document_mut(f.doc).unwrap().wb.sheets[0]
        .meta
        .detective = Arc::new(SheetDetective {
        ops,
        ..SheetDetective::default()
    });
    let t = std::time::Instant::now();
    let r = f.set("D1", "=1");
    let took = t.elapsed();
    assert_eq!(r.detective_changed, Some(true));
    let st = f.svc.detective_arrows(f.doc, sheet).unwrap();
    assert!(
        st.incomplete && !st.arrows.is_empty(),
        "{}",
        st.arrows.len()
    );
    // 250,000 steps or 40 ms of replay (debug builds are slower).
    assert!(took < std::time::Duration::from_secs(5), "{took:?}");
}
