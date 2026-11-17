//! Cell positions of a sheet whose first used cell is not A1: calamine's
//! `Range::used_cells()` yields positions relative to the range's start,
//! while merges and the formula range are absolute.
#![allow(clippy::unwrap_used)]

mod biff;

use tally_io_xls::{read_xls, Limits};
use tally_model::{CellContentData, CellRangeLocal};
use tally_types::Value;

#[test]
fn cells_keep_their_position_when_the_sheet_does_not_start_at_a1() {
    // C3 = 5, D4 = C3*2 (cached 10), C5 = text under the merge C5:D5.
    let mut rgce = biff::ptg_ref(2, 2);
    rgce.extend(biff::ptg_int(2));
    rgce.push(0x05);
    let mut sheet = biff::number(2, 2, 15, 5.0);
    sheet.extend(biff::formula(3, 3, 15, 10.0, &rgce));
    sheet.extend(biff::label(4, 2, 15, "merged"));
    sheet.extend(biff::merged(&[(4, 4, 2, 3)]));
    let bytes = biff::file(biff::stream(
        &biff::globals(&[], &[], &[]),
        &[("Offset", sheet)],
    ));
    let wb = read_xls(&bytes, &Limits::default()).unwrap();
    let s = &wb.sheets[0];
    let at: Vec<(u32, u32)> = s.cells.iter().map(|c| (c.row, c.col)).collect();
    assert_eq!(at, vec![(2, 2), (3, 3), (4, 2)]);
    assert_eq!(
        s.cells[0].content,
        CellContentData::Value(Value::Number(5.0))
    );
    match &s.cells[1].content {
        CellContentData::Formula { text, cached } => {
            assert_eq!(&**text, "of:=[.C3]*2");
            assert_eq!(*cached, Some(Value::Number(10.0)));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        s.cells[2].content,
        CellContentData::Value(Value::text("merged"))
    );
    assert_eq!(
        s.merges,
        vec![CellRangeLocal {
            r0: 4,
            c0: 2,
            r1: 4,
            c1: 3
        }]
    );
}

#[test]
fn a_formula_range_that_starts_later_than_the_values_still_matches() {
    // A1 = 1 (value range starts at A1), C3 = A1+1 (formula range starts
    // at C3).
    let mut rgce = biff::ptg_ref(0, 0);
    rgce.extend(biff::ptg_int(1));
    rgce.push(0x03);
    let mut sheet = biff::number(0, 0, 15, 1.0);
    sheet.extend(biff::formula(2, 2, 15, 2.0, &rgce));
    let bytes = biff::file(biff::stream(&biff::globals(&[], &[], &[]), &[("S", sheet)]));
    let wb = read_xls(&bytes, &Limits::default()).unwrap();
    let s = &wb.sheets[0];
    assert_eq!((s.cells[1].row, s.cells[1].col), (2, 2));
    match &s.cells[1].content {
        CellContentData::Formula { text, .. } => assert_eq!(&**text, "of:=[.A1]+1"),
        other => panic!("{other:?}"),
    }
}
