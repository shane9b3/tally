//! Relocation (copy/paste) and structural-change adjustment.

#![allow(clippy::unwrap_used)] // Test code may unwrap.

mod common;
use common::*;
use tally_formula::{
    adjust_for_structure_change, relocate, shifted_origin, Formula, StructureChange,
};

#[test]
fn copy_c1_to_d5() {
    let f = p_at("=A1+$B$1", "C1");
    let moved = relocate(&f, a1("C1"), a1("D5"));
    assert_eq!(moved, f, "relocation is identity on the AST");
    assert_eq!(pr(&moved, "D5"), "=B5+$B$1");
    assert_eq!(pr(&f, "C1"), "=A1+$B$1");
}

#[test]
fn copy_mixed_refs() {
    let f = p_at("=$A1+A$1+SUM(A1:B2)+Sheet2.A1+A:A+1:1", "C3");
    assert_eq!(pr(&f, "D5"), "=$A3+B$1+SUM(B3:C4)+Sheet2.B3+B:B+3:3");
}

#[test]
fn copy_off_sheet_prints_ref() {
    let f = p_at("=A1+B2", "B2");
    assert_eq!(pr(&f, "A1"), "=#REF!+A1");
    let g = p_at("=SUM(A1:A3)", "B3");
    assert_eq!(pr(&g, "B1"), "=SUM(#REF!)");
    assert_eq!(pr_of(&g, "B1"), "of:=SUM([.#REF!])");
    let h = p_at("=XFD1", "A1");
    assert_eq!(pr(&h, "B1"), "=#REF!");
    let cols = p_at("=SUM(A:A)", "B1");
    assert_eq!(pr(&cols, "A1"), "=SUM(#REF!)");
}

fn adj(src: &str, at: &str, ch: StructureChange) -> (String, bool) {
    let mut f: Formula = p_at(src, at);
    let changed = adjust_for_structure_change(&mut f, a1(at), &ch);
    let new_at = shifted_origin(a1(at), &ch);
    (
        tally_formula::print(&f, new_at, calc(), &resolver()),
        changed,
    )
}

fn ins_rows(at: u32, count: u32) -> StructureChange {
    StructureChange::InsertRows {
        sheet: 0,
        at,
        count,
    }
}
fn del_rows(at: u32, count: u32) -> StructureChange {
    StructureChange::DeleteRows {
        sheet: 0,
        at,
        count,
    }
}
fn ins_cols(at: u32, count: u32) -> StructureChange {
    StructureChange::InsertCols {
        sheet: 0,
        at,
        count,
    }
}
fn del_cols(at: u32, count: u32) -> StructureChange {
    StructureChange::DeleteCols {
        sheet: 0,
        at,
        count,
    }
}

#[test]
fn insert_rows() {
    // Rows are 0-based: at=4 inserts before row 5.
    let cases = [
        ("=A1", "Z20", ins_rows(4, 2), "=A1"),
        ("=A5", "Z20", ins_rows(4, 2), "=A7"),
        ("=A10", "Z20", ins_rows(4, 2), "=A12"),
        ("=$A$10", "Z20", ins_rows(4, 2), "=$A$12"),
        ("=A$10+$A10", "Z20", ins_rows(4, 2), "=A$12+$A12"),
        // Inside an area: grows.
        ("=SUM(A1:A10)", "Z20", ins_rows(4, 2), "=SUM(A1:A12)"),
        // At the first row: shifts.
        ("=SUM(A5:A10)", "Z20", ins_rows(4, 2), "=SUM(A7:A12)"),
        // Right after the last row: unchanged.
        ("=SUM(A1:A4)", "Z20", ins_rows(4, 2), "=SUM(A1:A4)"),
        // At the last row: grows.
        ("=SUM(A1:A5)", "Z20", ins_rows(4, 2), "=SUM(A1:A7)"),
        // Reversed endpoints keep orientation.
        ("=SUM(A10:A1)", "Z20", ins_rows(4, 2), "=SUM(A12:A1)"),
        // Formula above the insertion: its own cell does not move.
        ("=A10", "B1", ins_rows(4, 2), "=A12"),
        // Formula below the insertion moves too; ref above stays.
        ("=A1", "B10", ins_rows(4, 2), "=A1"),
        // Whole columns unaffected; whole rows shift/grow.
        ("=SUM(A:A)", "Z20", ins_rows(0, 5), "=SUM(A:A)"),
        ("=SUM(3:5)", "Z20", ins_rows(3, 1), "=SUM(3:6)"),
        ("=SUM(3:5)", "Z20", ins_rows(0, 1), "=SUM(4:6)"),
        // Other sheets unaffected.
        ("=Sheet2.A10", "Z20", ins_rows(4, 2), "=Sheet2.A10"),
        // Pushed off the sheet.
        ("=A1048576", "A1", ins_rows(10, 1), "=#REF!"),
        (
            "=SUM(A1:A1048576)",
            "B1",
            ins_rows(10, 1),
            "=SUM(A1:A1048576)",
        ),
    ];
    for (src, at, ch, want) in cases {
        assert_eq!(adj(src, at, ch).0, want, "{src} at {at} {ch:?}");
    }
}

#[test]
fn delete_rows() {
    let cases = [
        ("=A1", "Z20", del_rows(4, 2), "=A1"),
        ("=A5", "Z20", del_rows(4, 2), "=#REF!"),
        ("=A6", "Z20", del_rows(4, 2), "=#REF!"),
        ("=A7", "Z20", del_rows(4, 2), "=A5"),
        ("=$A$7", "Z20", del_rows(4, 2), "=$A$5"),
        // Part of an area: shrinks.
        ("=SUM(A1:A10)", "Z20", del_rows(2, 2), "=SUM(A1:A8)"),
        ("=SUM(A2:A10)", "Z20", del_rows(0, 3), "=SUM(A1:A7)"),
        ("=SUM(A1:A10)", "Z20", del_rows(5, 10), "=SUM(A1:A5)"),
        // Fully covered: #REF!.
        ("=SUM(A3:A4)", "Z20", del_rows(1, 5), "=SUM(#REF!)"),
        ("=SUM(A3:A4)", "Z20", del_rows(2, 2), "=SUM(#REF!)"),
        // Entirely below the deletion: shifts.
        ("=SUM(A10:B12)", "Z20", del_rows(0, 3), "=SUM(A7:B9)"),
        // Whole rows.
        ("=SUM(3:5)", "Z20", del_rows(2, 3), "=SUM(#REF!)"),
        ("=SUM(3:5)", "Z20", del_rows(3, 1), "=SUM(3:4)"),
        ("=SUM(A:A)", "Z20", del_rows(0, 100), "=SUM(A:A)"),
        // Formula cell moves up with the deletion.
        ("=A1+A20", "B10", del_rows(4, 2), "=A1+A18"),
    ];
    for (src, at, ch, want) in cases {
        assert_eq!(adj(src, at, ch).0, want, "{src} at {at} {ch:?}");
    }
}

#[test]
fn insert_and_delete_cols() {
    let cases = [
        ("=A1", "Z20", ins_cols(1, 1), "=A1"),
        ("=B1", "Z20", ins_cols(1, 1), "=C1"),
        ("=SUM(A1:D1)", "Z20", ins_cols(2, 3), "=SUM(A1:G1)"),
        ("=SUM(A:C)", "Z20", ins_cols(0, 1), "=SUM(B:D)"),
        ("=SUM(A:C)", "Z20", ins_cols(1, 1), "=SUM(A:D)"),
        ("=SUM(1:1)", "Z20", ins_cols(0, 1), "=SUM(1:1)"),
        ("=B1", "Z20", del_cols(1, 1), "=#REF!"),
        ("=C1", "Z20", del_cols(1, 1), "=B1"),
        ("=SUM(A1:D1)", "Z20", del_cols(1, 2), "=SUM(A1:B1)"),
        ("=SUM(B1:C1)", "Z20", del_cols(1, 2), "=SUM(#REF!)"),
        ("=SUM(A:C)", "Z20", del_cols(0, 1), "=SUM(A:B)"),
        ("=$D$1", "Z20", del_cols(0, 2), "=$B$1"),
        ("=XFD1", "A1", ins_cols(5, 1), "=#REF!"),
    ];
    for (src, at, ch, want) in cases {
        assert_eq!(adj(src, at, ch).0, want, "{src} at {at} {ch:?}");
    }
}

#[test]
fn other_sheet_formula_and_changed_flag() {
    // Formula on Sheet2 referencing Sheet1: Sheet1 rows inserted.
    let r = resolver();
    let origin = tally_types::CellAddr::new(1, 0, 0);
    let mut f = tally_formula::parse("=Sheet1.A5+A5", origin, calc(), &r).unwrap();
    let ch = ins_rows(0, 1);
    assert!(adjust_for_structure_change(&mut f, origin, &ch));
    assert_eq!(
        tally_formula::print(&f, origin, calc(), &r),
        "=Sheet1.A6+A5"
    );

    // Nothing to do.
    let (s, changed) = adj("=A1+1", "B1", ins_rows(10, 1));
    assert_eq!(s, "=A1+1");
    assert!(!changed);
    let (_, changed) = adj("=A20", "B1", ins_rows(10, 1));
    assert!(changed);
}

#[test]
fn delete_and_rename_sheet() {
    let mut f = p("=Sheet2.A1+SUM(Sheet2.A1:B2)+A1+'My Sheet'.A1");
    assert!(adjust_for_structure_change(
        &mut f,
        a1("A1"),
        &StructureChange::DeleteSheet { sheet: 1 }
    ));
    assert_eq!(pr(&f, "A1"), "=#REF!+SUM(#REF!)+A1+'My Sheet'.A1");
    let mut g = p("=Sheet2.A1");
    assert!(!adjust_for_structure_change(
        &mut g,
        a1("A1"),
        &StructureChange::RenameSheet { sheet: 1 }
    ));
}

#[test]
fn static_precedents() {
    use tally_formula::ResolvedRef;
    use tally_types::{CellRange, MAX_COLS, MAX_ROWS};
    let f = p_at(
        "=A1+SUM(B1:C2)+A:A+2:2+Sheet2.A1+Sheet1.A1:Sheet2.B2+Nowhere.A1+#REF!",
        "C3",
    );
    let got = f.static_precedents(a1("C3"));
    assert_eq!(
        got,
        vec![
            ResolvedRef::Range(CellRange::new(0, 0, 0, 0, 0)),
            ResolvedRef::Range(CellRange::new(0, 0, 1, 1, 2)),
            ResolvedRef::Range(CellRange::new(0, 0, 0, MAX_ROWS - 1, 0)),
            ResolvedRef::Range(CellRange::new(0, 1, 0, 1, MAX_COLS - 1)),
            ResolvedRef::Range(CellRange::new(1, 0, 0, 0, 0)),
            ResolvedRef::Span3d {
                first: 0,
                last: 1,
                area: CellRange::new(0, 0, 0, 1, 1)
            },
            ResolvedRef::Invalid,
            ResolvedRef::Invalid,
        ]
    );
    // Copied to A1 the relative A1 falls off the grid.
    let g = p_at("=A1", "C3");
    assert_eq!(g.static_precedents(a1("A1")), vec![ResolvedRef::Invalid]);
}

#[test]
fn walk_refs_mut_rewrites() {
    use tally_formula::RefExpr;
    let mut f = p("=A1+SUM(B1,C1)");
    let mut n = 0;
    f.expr.walk_refs_mut(|r| {
        n += 1;
        if n == 2 {
            *r = RefExpr::Invalid;
        }
    });
    assert_eq!(n, 3);
    assert_eq!(pr(&f, "A1"), "=A1+SUM(#REF!,C1)");
}
