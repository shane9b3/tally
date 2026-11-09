//! Regression tests for the third security review
//! (`docs/reviews/security-review-3-2026-10-20.md`), engine side. Each test
//! rebuilds the review's input shape (smaller where the shape, not the
//! size, is what matters) and asserts a functional result plus a generous
//! time bound that holds in debug builds. Release-only timings are
//! `#[ignore]`d with a note; run them with
//! `cargo test --release -p tally-engine --features io --test security3 -- --ignored`.
#![allow(clippy::unwrap_used)]

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use common::{input, range, Fixture};
use tally_api::{
    BreakAxis, CfRuleDto, CfRuleKind, CfStyleDto, DocumentService, FindMode, FindQuery, ListSource,
    NoteDto, Op, RangeDto, ValidationAlert, ValidationDto, ValidationErrorStyle, ValidationKind,
};
use tally_engine::storage::{MetaKind, MetaPart};
use tally_engine::undo::Change;
use tally_engine::{EngineService, Workbook};
use tally_model::{
    CellRangeLocal, CfKindData, CfRuleData, CfStyleData, ConditionalFormatData, LinkData, NoteData,
    SheetData, SheetProtectionData, WorkbookData,
};
use tally_types::Value;

fn assert_fast(what: &str, t: Instant, limit: Duration) {
    let took = t.elapsed();
    assert!(took < limit, "{what} took {took:?} (limit {limit:?})");
}

fn rng(r0: u32, c0: u32, r1: u32, c1: u32) -> RangeDto {
    RangeDto { r0, c0, r1, c1 }
}

fn fill_rule(formula: &str) -> CfRuleDto {
    CfRuleDto {
        kind: CfRuleKind::Formula {
            formula: formula.into(),
        },
        style: CfStyleDto {
            fill: Some(0xff00_00ff),
            ..CfStyleDto::default()
        },
        stop_if_true: false,
    }
}

fn add_cf(sheet: u32, ranges: Vec<RangeDto>, formula: &str) -> Op {
    Op::AddConditionalFormat {
        sheet,
        ranges,
        rules: vec![fill_rule(formula)],
    }
}

fn note_op(sheet: u32, r: u32, c: u32, text: &str) -> Op {
    Op::SetNote {
        sheet,
        r,
        c,
        note: NoteDto {
            r,
            c,
            text: text.into(),
            author: None,
            created: None,
            shown: false,
        },
    }
}

fn col_inputs(sheet: u32, c: u32, n: u32, f: impl Fn(u32) -> String) -> Op {
    Op::SetRangeInputs {
        sheet,
        r: 0,
        c,
        rows: (0..n).map(|r| vec![f(r)]).collect(),
    }
}

fn stop_alert() -> Option<ValidationAlert> {
    Some(ValidationAlert {
        style: ValidationErrorStyle::Stop,
        title: "t".into(),
        text: "bad".into(),
    })
}

fn open_data(data: WorkbookData) -> (EngineService, tally_api::WorkbookInfo) {
    let mut svc = EngineService::new();
    let info = svc.open_workbook(Workbook::from_data(&data), "t");
    (svc, info)
}

fn last_record_parts(f: &Fixture) -> Vec<MetaPart> {
    let doc = f.svc.document(f.doc).unwrap();
    let rec = doc.undo.undo_records().last().expect("an undo record");
    rec.changes
        .iter()
        .flat_map(|c| match c {
            Change::Meta { parts, .. } => parts.clone(),
            _ => Vec::new(),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// CRITICAL-1: conditional-format evaluation per viewport fetch
// ---------------------------------------------------------------------------

/// The review's `cfsame` shape: the maximum number of formats, all with
/// the same rule on the same range (10,000 of them took a minute per
/// fetch). The identical rectangles collapse into one index entry, rules
/// that cannot add a property are skipped, and the call's budget bounds
/// the rest; the first cells are decorated and the block says when the
/// budget ran out.
#[test]
fn stacked_identical_rules_render_within_the_budget() {
    let mut f = Fixture::new();
    let s = f.sheet;
    let n = tally_engine::condfmt::MAX_CF_PER_SHEET as u32;
    f.set("A1", "1");
    let t = Instant::now();
    let res = f.apply(
        (0..n)
            .map(|_| add_cf(s, vec![rng(0, 0, 999, 19)], "A1>0"))
            .collect(),
    );
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    assert_fast("add 2048 formats", t, Duration::from_secs(20));
    // One more is refused.
    let res = f.apply(vec![add_cf(s, vec![rng(0, 0, 0, 0)], "A1>0")]);
    assert_eq!(res.errors.len(), 1, "the 2,049th format is refused");
    let t = Instant::now();
    let blocks = f.svc.cells(f.doc, s, &[rng(0, 0, 999, 19)]).unwrap();
    assert_fast(
        "viewport with 2048 stacked rules",
        t,
        Duration::from_secs(20),
    );
    let a1 = blocks[0]
        .cells
        .iter()
        .find(|c| (c.r, c.c) == (0, 0))
        .unwrap();
    assert!(a1.cf_style.is_some(), "the first cell gets its format");
    // The cached second fetch is cheap and identical.
    let t = Instant::now();
    let again = f.svc.cells(f.doc, s, &[rng(0, 0, 999, 19)]).unwrap();
    assert_fast("cached viewport", t, Duration::from_secs(5));
    assert_eq!(again[0].cells.len(), blocks[0].cells.len());
    assert_eq!(again[0].cf_truncated, blocks[0].cf_truncated);
}

/// The budget itself: with a tiny budget the block is flagged, the first
/// cell is decorated and later cells are left plain; with a large budget
/// every covered cell is decorated and the flag is absent.
#[test]
fn cf_budget_truncates_and_flags_the_block() {
    let mut f = Fixture::new();
    let s = f.sheet;
    let ops: Vec<Op> = (0..20)
        .map(|_| add_cf(s, vec![rng(0, 0, 99, 9)], "ROW()>0"))
        .collect();
    f.apply(ops);
    let doc = f.svc.document(f.doc).unwrap();
    let req = [rng(0, 0, 99, 9)];
    let small =
        tally_engine::service::view::cells_with_cf_budget(&doc.wb, doc.version, s, &req, 2_000)
            .unwrap();
    assert_eq!(small[0].cf_truncated, Some(true));
    let styled = small[0]
        .cells
        .iter()
        .filter(|c| c.cf_style.is_some())
        .count();
    assert!((1..1000).contains(&styled), "{styled} cells decorated");
    let full = tally_engine::service::view::cells_with_cf_budget(
        &doc.wb,
        doc.version,
        s,
        &req,
        tally_engine::condfmt::MAX_CF_WORK_PER_CALL,
    )
    .unwrap();
    assert_eq!(full[0].cf_truncated, None);
    assert_eq!(
        full[0]
            .cells
            .iter()
            .filter(|c| c.cf_style.is_some())
            .count(),
        1000
    );
    // The default path matches the full one.
    let api = f.svc.cells(f.doc, s, &req).unwrap();
    assert_eq!(api[0].cells.len(), 1000);
    assert_eq!(api[0].cf_truncated, None);
}

/// The review's `cf 1000 1024`: many formats × many disjoint blocks
/// (36 s per fetch). The range index finds the covering block in
/// O(log n); only the covered cells are decorated.
#[test]
fn range_index_handles_many_disjoint_blocks() {
    let mut f = Fixture::new();
    let s = f.sheet;
    let k = tally_engine::condfmt::MAX_CF_RANGES as u32;
    let ops: Vec<Op> = (0..200)
        .map(|_| {
            add_cf(
                s,
                (0..k)
                    .map(|j| rng(j * 1000, 0, j * 1000 + 999, 19))
                    .collect(),
                "ROW()>0",
            )
        })
        .collect();
    let res = f.apply(ops);
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let t = Instant::now();
    let blocks = f
        .svc
        .cells(f.doc, s, &[rng(63_500, 0, 64_499, 19)])
        .unwrap();
    assert_fast(
        "viewport across a block boundary",
        t,
        Duration::from_secs(10),
    );
    let styled = blocks[0]
        .cells
        .iter()
        .filter(|c| c.cf_style.is_some())
        .count();
    // Rows 63,500..63,999 are in block 63; rows 64,000.. are outside.
    assert_eq!(styled, 500 * 20, "only the covered half is decorated");
    assert_eq!(blocks[0].cf_truncated, None);
    // 65 ranges are refused.
    let res = f.apply(vec![add_cf(
        s,
        (0..=k).map(|j| rng(j, 0, j, 0)).collect(),
        "A1>0",
    )]);
    assert_eq!(res.errors.len(), 1);
}

/// The review's `cf_scan`: `SUM($A:$A)>A1` over a column of numbers,
/// evaluated once per cell (a minute for one viewport). The absolute
/// `SUM($A:$A)` folds to a literal once per version, so each cell costs
/// one trivial comparison.
#[test]
fn absolute_subexpressions_are_evaluated_once_per_version() {
    let mut f = Fixture::new();
    let s = f.sheet;
    let n = 50_000u32;
    f.apply(vec![col_inputs(s, 0, n, |r| format!("{r}"))]);
    f.apply(vec![add_cf(
        s,
        vec![rng(0, 0, 1_048_575, 19)],
        "SUM($A:$A)>A1",
    )]);
    let t = Instant::now();
    let blocks = f.svc.cells(f.doc, s, &[rng(0, 0, 999, 19)]).unwrap();
    assert_fast("viewport with SUM($A:$A)>A1", t, Duration::from_secs(10));
    assert_eq!(blocks[0].cf_truncated, None);
    assert_eq!(
        blocks[0]
            .cells
            .iter()
            .filter(|c| c.cf_style.is_some())
            .count(),
        20_000,
        "every cell gets the format"
    );
    // Direct check of the work: one folded evaluation per cell.
    let doc = f.svc.document(f.doc).unwrap();
    let eval = tally_engine::condfmt::CfEval::new(&doc.wb, s, doc.version + 1);
    for r in 0..100 {
        assert!(eval
            .decor(r, 0, tally_engine::storage::StyleId::DEFAULT)
            .is_some());
    }
    assert!(
        eval.work() < 100 * 64,
        "100 cells cost {} work units: the SUM was not folded",
        eval.work()
    );
    // Semantics: a value above the sum is not formatted.
    f.set("B1", "1e12");
    let blocks = f.svc.cells(f.doc, s, &[rng(0, 1, 0, 1)]).unwrap();
    assert!(blocks[0].cells[0].cf_style.is_none());
}

/// Loading more than the cap drops the extras with a warning, and a
/// format with too many ranges is truncated with a warning.
#[test]
fn loaded_conditional_formats_are_capped_with_warnings() {
    let cf = |ranges: Vec<CellRangeLocal>| ConditionalFormatData {
        ranges,
        base: (0, 0),
        rules: vec![CfRuleData {
            kind: CfKindData::Formula {
                formula: "of:=[.A1]>0".into(),
            },
            style: CfStyleData {
                fill: Some(0xff00_00ff),
                ..CfStyleData::default()
            },
            stop_if_true: false,
        }],
    };
    let one = CellRangeLocal {
        r0: 0,
        c0: 0,
        r1: 9,
        c1: 9,
    };
    let mut formats: Vec<ConditionalFormatData> = (0..tally_engine::condfmt::MAX_CF_PER_SHEET + 5)
        .map(|_| cf(vec![one]))
        .collect();
    formats[0] = cf((0..100u32)
        .map(|i| CellRangeLocal {
            r0: i,
            c0: 0,
            r1: i,
            c1: 0,
        })
        .collect());
    let (svc, info) = open_data(WorkbookData {
        sheets: vec![SheetData {
            name: "S".into(),
            conditional_formats: formats,
            ..SheetData::default()
        }],
        ..WorkbookData::default()
    });
    let cfs = svc
        .conditional_formats(info.doc, info.sheets[0].id)
        .unwrap();
    assert_eq!(cfs.len(), tally_engine::condfmt::MAX_CF_PER_SHEET);
    assert_eq!(cfs[0].ranges.len(), tally_engine::condfmt::MAX_CF_RANGES);
    assert!(
        info.warnings
            .iter()
            .any(|w| w.contains("2048 conditional formats")),
        "{:?}",
        info.warnings
    );
    assert!(
        info.warnings
            .iter()
            .any(|w| w.contains("at most 64 ranges")),
        "{:?}",
        info.warnings
    );
}

// ---------------------------------------------------------------------------
// CRITICAL-2: sheet meta snapshots on undo
// ---------------------------------------------------------------------------

/// 10,000 `SetNote` in one transaction cloned the whole meta twice per
/// op (1.4 s and 5 GB). Each op now records the one entry it replaced.
#[test]
fn many_note_ops_in_one_transaction_record_single_entries() {
    let mut f = Fixture::new();
    let s = f.sheet;
    let n = 10_000u32;
    let t = Instant::now();
    let res = f.apply(
        (0..n)
            .map(|i| note_op(s, i, 0, &format!("note {i}")))
            .collect(),
    );
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    assert_fast("10k SetNote in one transaction", t, Duration::from_secs(10));
    let parts = last_record_parts(&f);
    assert_eq!(parts.len(), n as usize, "one part per op");
    assert!(
        parts
            .iter()
            .all(|p| matches!(p, MetaPart::NoteEntries(e) if e.len() == 1 && e[0].1.is_none())),
        "every part is one replaced entry"
    );
    assert_eq!(f.svc.notes(f.doc, s).unwrap().len(), n as usize);
    f.svc.undo(f.doc).unwrap();
    assert_eq!(f.svc.notes(f.doc, s).unwrap().len(), 0);
    f.svc.redo(f.doc).unwrap();
    assert_eq!(f.svc.notes(f.doc, s).unwrap().len(), n as usize);
    // The live map is not shared with any undo record.
    let wb = &f.svc.document(f.doc).unwrap().wb;
    assert_eq!(Arc::strong_count(&wb.sheets[0].meta.notes), 1);
}

/// 100 single note edits on a sheet with 100,000 notes kept 100 copies of
/// the map (13 MB each). Each edit now records one entry and the map is
/// edited in place.
#[test]
fn single_note_edits_on_a_large_sheet_share_the_map() {
    let notes: Vec<NoteData> = (0..100_000u32)
        .map(|r| NoteData {
            row: r,
            col: 0,
            text: format!("n{r}"),
            author: None,
            created: None,
            shown: false,
        })
        .collect();
    let (mut svc, info) = open_data(WorkbookData {
        sheets: vec![SheetData {
            name: "S".into(),
            notes,
            ..SheetData::default()
        }],
        ..WorkbookData::default()
    });
    let (doc, s) = (info.doc, info.sheets[0].id);
    let t = Instant::now();
    for i in 0..100u32 {
        let res = svc
            .apply(doc, common::tx(vec![note_op(s, 200_000 + i, 0, "x")]))
            .unwrap();
        assert!(res.errors.is_empty());
    }
    assert_fast("100 note edits on 100k notes", t, Duration::from_secs(5));
    let d = svc.document(doc).unwrap();
    assert_eq!(d.undo.undo_records().len(), 100);
    for rec in d.undo.undo_records() {
        assert_eq!(rec.changes.len(), 1);
        match &rec.changes[0] {
            Change::Meta { parts, .. } => {
                assert_eq!(parts.len(), 1);
                assert!(matches!(&parts[0], MetaPart::NoteEntries(e) if e.len() == 1));
            }
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(Arc::strong_count(&d.wb.sheets[0].meta.notes), 1);
    // Undo of a single edit on the large sheet is also cheap.
    let t = Instant::now();
    svc.undo(doc).unwrap();
    assert_fast("undo one note edit", t, Duration::from_secs(2));
    assert_eq!(svc.notes(doc, s).unwrap().len(), 100_099);
}

/// Every meta op records only the collections it changed (pointer
/// identity before/after), so a merge on a sheet with many conditional
/// formats holds the merges, not the formats.
#[test]
fn meta_ops_record_only_the_touched_collections() {
    let mut f = Fixture::new();
    let s = f.sheet;
    f.apply(
        (0..100)
            .map(|i| add_cf(s, vec![rng(i, 0, i, 5)], "A1>0"))
            .collect(),
    );
    let kinds = |parts: &[MetaPart]| -> Vec<Option<MetaKind>> {
        parts.iter().map(MetaPart::kind).collect()
    };
    assert_eq!(
        kinds(&last_record_parts(&f))
            .iter()
            .filter(|k| **k == Some(MetaKind::CondFormats))
            .count(),
        1,
        "the formats collection is recorded once per transaction"
    );
    let before = f.svc.document(f.doc).unwrap().wb.sheets[0]
        .meta
        .cond_formats
        .clone();
    f.apply(vec![Op::MergeCells {
        sheet: s,
        range: rng(200, 0, 201, 1),
    }]);
    let parts = last_record_parts(&f);
    assert_eq!(kinds(&parts), vec![Some(MetaKind::Merges)]);
    f.apply(vec![Op::SetFrozen {
        sheet: s,
        rows: 2,
        cols: 0,
    }]);
    assert_eq!(kinds(&last_record_parts(&f)), vec![Some(MetaKind::Frozen)]);
    f.apply(vec![Op::SetPageBreak {
        sheet: s,
        axis: BreakAxis::Row,
        at: 10,
        present: true,
    }]);
    assert_eq!(
        kinds(&last_record_parts(&f)),
        vec![Some(MetaKind::RowBreaks)]
    );
    f.apply(vec![Op::SetLink {
        sheet: s,
        r: 0,
        c: 0,
        url: "https://example.com".into(),
    }]);
    assert!(matches!(last_record_parts(&f).as_slice(), [MetaPart::LinkEntries(e)] if e.len() == 1));
    // The formats collection was never copied by those ops.
    let after = &f.svc.document(f.doc).unwrap().wb.sheets[0]
        .meta
        .cond_formats;
    assert!(Arc::ptr_eq(&before, after));
    // Undo/redo restore everything.
    for _ in 0..4 {
        f.svc.undo(f.doc).unwrap();
    }
    let m = &f.svc.document(f.doc).unwrap().wb.sheets[0].meta;
    assert!(
        m.merges.is_empty() && m.frozen.is_none() && m.row_breaks.is_empty() && m.links.is_empty()
    );
    for _ in 0..4 {
        f.svc.redo(f.doc).unwrap();
    }
    let m = &f.svc.document(f.doc).unwrap().wb.sheets[0].meta;
    assert!(m.merges.len() == 1 && m.frozen == Some((2, 0)) && m.row_breaks.contains(&10));
    assert_eq!(m.links.len(), 1);
}

/// A failing meta op leaves the meta as it was (the snapshotted fields
/// are restored).
#[test]
fn failing_meta_ops_leave_the_meta_untouched() {
    let mut f = Fixture::new();
    let s = f.sheet;
    f.apply(vec![add_cf(s, vec![rng(0, 0, 9, 9)], "A1>0")]);
    let before = f.data();
    let res = f.apply(vec![
        Op::UpdateConditionalFormat {
            sheet: s,
            id: 1,
            ranges: vec![rng(0, 0, 9, 9)],
            rules: vec![fill_rule("SUM(")],
        },
        Op::DeleteNote {
            sheet: s,
            r: 0,
            c: 0,
        },
        Op::AddValidation {
            sheet: s,
            validation: ValidationDto {
                id: 0,
                ranges: vec![],
                kind: ValidationKind::Any,
                allow_blank: true,
                show_dropdown: false,
                sort_ascending: false,
                input_help: None,
                error: None,
            },
        },
    ]);
    assert_eq!(res.errors.len(), 3, "{:?}", res.errors);
    assert_eq!(f.data(), before);
    assert_eq!(f.svc.document(f.doc).unwrap().wb.sheets[0].meta.next_id, 2);
}

/// Meta ops per transaction and note bytes per transaction are capped.
#[test]
fn meta_ops_per_transaction_are_capped() {
    let mut f = Fixture::new();
    let s = f.sheet;
    let big = "x".repeat(tally_engine::ops_v03::MAX_NOTE_LEN);
    let per_tx = (tally_engine::ops_v03::MAX_NOTE_BYTES_PER_TX
        / tally_engine::ops_v03::MAX_NOTE_LEN as u64) as u32;
    let ops: Vec<Op> = (0..=per_tx).map(|i| note_op(s, i, 0, &big)).collect();
    let res = f.apply(ops);
    assert_eq!(res.errors.len(), 1, "{:?}", res.errors.len());
    assert_eq!(res.errors[0].op_index, per_tx);
    assert_eq!(res.errors[0].code, "unsupported");
    assert_eq!(f.svc.notes(f.doc, s).unwrap().len(), per_tx as usize);
    let many: Vec<Op> = (0..=tally_engine::ops_v03::MAX_META_OPS_PER_TX as u32)
        .map(|i| Op::SetLink {
            sheet: s,
            r: i % 1000,
            c: i / 1000,
            url: "https://e.com".into(),
        })
        .collect();
    let t = Instant::now();
    let res = f.apply(many);
    assert_fast("100k SetLink", t, Duration::from_secs(30));
    assert_eq!(res.errors.len(), 1);
    assert_eq!(
        res.errors[0].op_index,
        tally_engine::ops_v03::MAX_META_OPS_PER_TX as u32
    );
}

// ---------------------------------------------------------------------------
// HIGH-4: validation cost
// ---------------------------------------------------------------------------

/// The review's `val_list`: a list source of 10,000 text items and a fill
/// of many cells (a minute before). The list is resolved once per op into
/// a set; semantics are unchanged (case-insensitive text, numbers by
/// value and by their general text).
#[test]
fn list_validation_resolves_the_list_once_per_op() {
    let mut f = Fixture::new();
    let s = f.sheet;
    let n = 10_000u32;
    f.apply(vec![col_inputs(s, 1, n, |r| format!("item{r}"))]);
    f.apply(vec![Op::AddValidation {
        sheet: s,
        validation: ValidationDto {
            id: 0,
            ranges: vec![rng(0, 0, 1_048_575, 0)],
            kind: ValidationKind::List {
                source: ListSource::Range {
                    reference: "$B:$B".into(),
                },
            },
            allow_blank: true,
            show_dropdown: true,
            sort_ascending: false,
            input_help: None,
            error: stop_alert(),
        },
    }]);
    f.set("A1", &format!("ITEM{}", n - 1));
    let t = Instant::now();
    let res = f.apply(vec![Op::FillRange {
        sheet: s,
        source: rng(0, 0, 0, 0),
        target: rng(0, 0, 19_999, 0),
        mode: tally_api::FillMode::Copy,
    }]);
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    assert_fast("fill 20k validated cells", t, Duration::from_secs(10));
    assert_eq!(
        f.value("A20000"),
        Value::text(format!("ITEM{}", n - 1).as_str())
    );
    assert_eq!(f.set("A5", "nope").errors[0].code, "validation");
    assert_eq!(
        f.value("A5"),
        Value::text(format!("ITEM{}", n - 1).as_str())
    );
    // Numbers match numeric items and their text.
    f.apply(vec![input(s, "B1", "42")]);
    assert!(f.set("A6", "42").errors.is_empty());
    assert!(f.set("A7", "=42").errors.is_empty());
    assert!(f.set("A8", "43").errors[0].code == "validation");
}

/// The review's `val_custom`: `SUM($B:$B)>A1` with a long column (31 s for
/// 10,000 cells). The absolute `SUM` folds once per op.
#[test]
fn custom_validation_folds_absolute_parts_once_per_op() {
    let mut f = Fixture::new();
    let s = f.sheet;
    f.apply(vec![col_inputs(s, 1, 50_000, |r| format!("{r}"))]);
    f.apply(vec![Op::AddValidation {
        sheet: s,
        validation: ValidationDto {
            id: 0,
            ranges: vec![rng(0, 0, 1_048_575, 0)],
            kind: ValidationKind::Custom {
                formula: "SUM($B:$B)>A1".into(),
            },
            allow_blank: true,
            show_dropdown: false,
            sort_ascending: false,
            input_help: None,
            error: stop_alert(),
        },
    }]);
    f.set("A1", "1");
    let t = Instant::now();
    let res = f.apply(vec![Op::FillRange {
        sheet: s,
        source: rng(0, 0, 0, 0),
        target: rng(0, 0, 4_999, 0),
        mode: tally_api::FillMode::Copy,
    }]);
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    assert_fast(
        "fill 5k cells with a SUM validation",
        t,
        Duration::from_secs(10),
    );
    assert_eq!(f.set("A2", "1e15").errors[0].code, "validation");
}

/// Past the per-op budget the remaining cells are accepted with a notice.
#[test]
fn validation_budget_accepts_the_rest_with_a_notice() {
    let mut wb = Workbook::new();
    let s = wb.sheets[0].id;
    let mut fx = tally_engine::undo::Effects::default();
    for r in 0..200u32 {
        wb.set_input(tally_types::CellAddr::new(s, r, 1), "1");
    }
    tally_engine::ops::apply_op(
        &mut wb,
        &Op::AddValidation {
            sheet: s,
            validation: ValidationDto {
                id: 0,
                ranges: vec![rng(0, 0, 999, 0)],
                kind: ValidationKind::Custom {
                    // Not foldable (SUMPRODUCT forces arrays); costs a few
                    // hundred element operations per cell.
                    formula: "SUMPRODUCT($B$1:$B$200;$B$1:$B$200)<A1".into(),
                },
                allow_blank: true,
                show_dropdown: false,
                sort_ascending: false,
                input_help: None,
                error: stop_alert(),
            },
        },
        None,
        &mut fx,
    )
    .unwrap();
    for r in 0..1000u32 {
        wb.set_input(tally_types::CellAddr::new(s, r, 0), "0");
    }
    let cells: Vec<(u32, u32)> = (0..1000).map(|r| (r, 0)).collect();
    // A budget for a few cells: the first fail with Stop.
    let r = tally_engine::validation::check_cells_with_budget(&mut wb, s, &cells, 2_000);
    assert_eq!(r.unwrap_err().code, "validation");
    // All valid values: the budget runs out and the rest are accepted.
    for r in 0..1000u32 {
        wb.set_input(tally_types::CellAddr::new(s, r, 0), "1e12");
    }
    let notices =
        tally_engine::validation::check_cells_with_budget(&mut wb, s, &cells, 2_000).unwrap();
    assert_eq!(notices.len(), 1, "{notices:?}");
    assert!(notices[0].message.contains("accepted without checking"));
    assert!(
        tally_engine::validation::check_cells(&mut wb, s, &cells)
            .unwrap()
            .is_empty(),
        "the default budget covers 1,000 cells"
    );
}

// ---------------------------------------------------------------------------
// MEDIUM-5, MEDIUM-8, LOW-10, LOW-12: loading
// ---------------------------------------------------------------------------

/// `spinCount` is clamped and over-long hash material is dropped (the
/// sheet stays protected and any password unlocks it).
#[test]
fn spin_count_is_clamped_and_long_hashes_dropped() {
    let prot = |spin: u32, salt: usize| SheetProtectionData {
        password_hash: Some("AAAA".into()),
        algorithm: Some("SHA-512".into()),
        salt: Some("A".repeat(salt)),
        spin_count: Some(spin),
        ..SheetProtectionData::default()
    };
    let (mut svc, info) = open_data(WorkbookData {
        sheets: vec![
            SheetData {
                name: "A".into(),
                protection: Some(prot(10_000_000, 16)),
                ..SheetData::default()
            },
            SheetData {
                name: "B".into(),
                protection: Some(prot(100, 4096)),
                ..SheetData::default()
            },
        ],
        ..WorkbookData::default()
    });
    let d = svc.document(info.doc).unwrap();
    let a = d.wb.sheets[0].meta.protection().unwrap();
    assert_eq!(a.spin_count, Some(tally_engine::protection::MAX_SPIN_COUNT));
    let b = d.wb.sheets[1].meta.protection().unwrap();
    assert!(b.password_hash.is_none() && b.salt.is_none());
    assert!(
        info.warnings.iter().any(|w| w.contains("password hash")),
        "{:?}",
        info.warnings
    );
    let t = Instant::now();
    let res = svc
        .apply(
            info.doc,
            common::tx(vec![Op::UnprotectSheet {
                sheet: info.sheets[0].id,
                password: Some("wrong".into()),
            }]),
        )
        .unwrap();
    assert_eq!(res.errors[0].code, "protected");
    assert_fast(
        "one unlock attempt at the clamped spin count",
        t,
        Duration::from_secs(30),
    );
    let res = svc
        .apply(
            info.doc,
            common::tx(vec![Op::UnprotectSheet {
                sheet: info.sheets[1].id,
                password: Some("anything".into()),
            }]),
        )
        .unwrap();
    assert!(res.errors.is_empty(), "{:?}", res.errors);
}

/// Imported notes are truncated to the note cap, counted against a
/// per-workbook budget, and hostile link targets are dropped (LOW-10).
#[test]
fn imported_notes_and_links_are_capped() {
    let max = tally_engine::ops_v03::MAX_NOTE_LEN;
    let per_wb = (tally_engine::convert::MAX_NOTE_BYTES_PER_WORKBOOK / max as u64) as u32;
    let notes: Vec<NoteData> = (0..per_wb + 10)
        .map(|r| NoteData {
            row: r,
            col: 0,
            text: "é".repeat(max / 2 + 100),
            author: None,
            created: None,
            shown: false,
        })
        .collect();
    let urls = [
        "javascript:alert(1)",
        "JaVaScRiPt:alert(1)",
        "java\nscript:alert(1)",
        "data:text/html,x",
        "vbscript:x",
        " javascript:1",
        "https://example.com/\u{0}",
        "https://example.com/ok",
        "#Sheet1.A1",
        "mailto:a@b.c",
    ];
    let links: Vec<LinkData> = urls
        .iter()
        .enumerate()
        .map(|(i, u)| LinkData {
            row: i as u32,
            col: 1,
            url: Arc::from(*u),
        })
        .collect();
    let (svc, info) = open_data(WorkbookData {
        sheets: vec![SheetData {
            name: "S".into(),
            notes,
            links,
            ..SheetData::default()
        }],
        ..WorkbookData::default()
    });
    let (doc, s) = (info.doc, info.sheets[0].id);
    let all = svc.notes(doc, s).unwrap();
    assert_eq!(all.len(), per_wb as usize, "notes past 64 MiB are dropped");
    assert!(all
        .iter()
        .all(|n| n.text.len() <= max && n.text.len() > max - 4));
    assert!(
        info.warnings.iter().any(|w| w.contains("truncated")),
        "{:?}",
        info.warnings
    );
    assert!(
        info.warnings.iter().any(|w| w.contains("64 MiB")),
        "{:?}",
        info.warnings
    );
    let kept: Vec<String> = svc.cells(doc, s, &[rng(0, 1, 9, 1)]).unwrap()[0]
        .cells
        .iter()
        .filter_map(|c| c.link.clone())
        .collect();
    assert_eq!(
        kept,
        vec!["https://example.com/ok", "#Sheet1.A1", "mailto:a@b.c"]
    );
    assert!(
        info.warnings.iter().any(|w| w.contains("7 hyperlinks")),
        "{:?}",
        info.warnings
    );
}

/// LOW-12: undoing a sheet deletion restores the rule formulas and
/// validations of the other sheets that pointed at it.
#[test]
fn undo_of_delete_sheet_restores_rule_formulas() {
    let mut f = Fixture::new();
    let s = f.sheet;
    let res = f.apply(vec![Op::AddSheet {
        name: Some("Other".into()),
        index: None,
    }]);
    let other = res
        .info
        .unwrap()
        .sheets
        .iter()
        .find(|x| x.name == "Other")
        .unwrap()
        .id;
    f.apply(vec![col_inputs(other, 0, 10, |r| format!("{r}"))]);
    f.apply(vec![
        add_cf(s, vec![rng(0, 0, 9, 9)], "SUM(Other.A:A)>A1"),
        Op::AddValidation {
            sheet: s,
            validation: ValidationDto {
                id: 0,
                ranges: vec![rng(0, 0, 9, 0)],
                kind: ValidationKind::List {
                    source: ListSource::Range {
                        reference: "Other.$A$1:$A$10".into(),
                    },
                },
                allow_blank: true,
                show_dropdown: true,
                sort_ascending: false,
                input_help: None,
                error: stop_alert(),
            },
        },
    ]);
    let text = |f: &Fixture| match &f.svc.conditional_formats(f.doc, s).unwrap()[0].rules[0].kind {
        CfRuleKind::Formula { formula } => formula.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(text(&f), "SUM(Other.A:A)>A1");
    f.apply(vec![Op::DeleteSheet { sheet: other }]);
    assert_eq!(text(&f), "SUM(#REF!)>A1");
    assert_eq!(f.set("A1", "5").errors[0].code, "validation");
    f.svc.undo(f.doc).unwrap();
    assert_eq!(text(&f), "SUM(Other.A:A)>A1");
    let v = f.svc.validation_at(f.doc, s, 0, 0).unwrap().unwrap();
    assert_eq!(v.list_items.len(), 10);
    assert!(f.set("A1", "5").errors.is_empty());
    let blocks = f.svc.cells(f.doc, s, &[rng(0, 0, 0, 0)]).unwrap();
    assert!(
        blocks[0].cells[0].cf_style.is_some(),
        "the rule evaluates again"
    );
}

// ---------------------------------------------------------------------------
// LOW-9: protection gaps
// ---------------------------------------------------------------------------

#[test]
fn protection_gates_page_setup_names_undo_and_find() {
    let mut f = Fixture::new();
    let s = f.sheet;
    f.apply(vec![
        input(s, "A1", "41"),
        input(s, "B1", "=A1+1"),
        Op::SetCellProtection {
            sheet: s,
            ranges: vec![rng(0, 1, 0, 1)],
            locked: Some(true),
            hidden_formula: Some(true),
            hidden: None,
            hide_print: None,
        },
        Op::DefineName {
            name: "nm".into(),
            scope: None,
            refers_to: "$A$1".into(),
        },
    ]);
    f.set("A1", "edited");
    let res = f.apply(vec![Op::ProtectSheet {
        sheet: s,
        password: Some("pw".into()),
        options: None,
    }]);
    assert!(res.errors.is_empty());
    assert!(!res.undo.can_undo, "protection clears the undo stack");
    f.svc.undo(f.doc).unwrap();
    assert!(f.svc.protection_info(f.doc, s).unwrap().sheet_protected);
    assert_eq!(f.value("A1"), Value::text("edited"), "nothing was undone");
    let setup = f.svc.page_setup(f.doc, s).unwrap();
    let code = |res: tally_api::ApplyResult| res.errors.first().map(|e| e.code.clone());
    assert_eq!(
        code(f.apply(vec![Op::SetPageSetup { sheet: s, setup }])),
        Some("protected".into())
    );
    assert_eq!(
        code(f.apply(vec![Op::SetPageBreak {
            sheet: s,
            axis: BreakAxis::Row,
            at: 3,
            present: true,
        }])),
        Some("protected".into())
    );
    assert_eq!(
        code(f.apply(vec![Op::SetFrozen {
            sheet: s,
            rows: 1,
            cols: 0,
        }])),
        None,
        "freezing panes is a view setting"
    );
    assert_eq!(code(f.apply(vec![Op::RefreshFilter { sheet: s }])), None);
    assert_eq!(
        code(f.apply(vec![Op::DefineName {
            name: "nm".into(),
            scope: None,
            refers_to: "$A$2".into(),
        }])),
        None,
        "names are workbook state"
    );
    // Hidden formulas are not searchable while protected.
    let q = FindQuery {
        text: "A1".into(),
        sheet: Some(s),
        in_formulas: true,
        match_case: false,
        whole_cell: false,
        mode: FindMode::Literal,
        from: None,
        in_notes: None,
        selection: None,
    };
    assert!(f.svc.find(f.doc, &q).unwrap().hits.is_empty());
    f.apply(vec![Op::UnprotectSheet {
        sheet: s,
        password: Some("pw".into()),
    }]);
    assert_eq!(f.svc.find(f.doc, &q).unwrap().hits.len(), 1);
    // Names are refused once the document structure is protected.
    f.apply(vec![Op::ProtectDocument { password: None }]);
    assert_eq!(
        code(f.apply(vec![Op::DeleteName {
            name: "nm".into(),
            scope: None,
        }])),
        Some("protected".into())
    );
    let _ = range("A1");
}

// ---------------------------------------------------------------------------
// Release-only timings (the review's sizes)
// ---------------------------------------------------------------------------

#[test]
#[ignore = "release-only timing; run with --release -- --ignored"]
fn cf_viewports_render_under_100ms_release() {
    let mut f = Fixture::new();
    let s = f.sheet;
    f.apply(vec![col_inputs(s, 0, 1_000_000, |r| format!("{r}"))]);
    f.apply(
        (0..tally_engine::condfmt::MAX_CF_PER_SHEET)
            .map(|_| add_cf(s, vec![rng(0, 0, 999, 19)], "A1>0"))
            .collect(),
    );
    f.apply(vec![add_cf(
        s,
        vec![rng(0, 0, 1_048_575, 19)],
        "SUM($A:$A)>A1",
    )]);
    for _ in 0..3 {
        let t = Instant::now();
        let _ = f.svc.cells(f.doc, s, &[rng(0, 0, 999, 19)]).unwrap();
        assert_fast("viewport", t, Duration::from_millis(100));
        f.set("Z5000", "1");
    }
}

#[test]
#[ignore = "release-only timing; run with --release -- --ignored"]
fn note_transactions_release_timing() {
    let mut f = Fixture::new();
    let s = f.sheet;
    let t = Instant::now();
    f.apply((0..10_000u32).map(|i| note_op(s, i, 0, "note")).collect());
    assert_fast("10k SetNote", t, Duration::from_millis(300));
}
