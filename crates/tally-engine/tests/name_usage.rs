//! Name usage flags: AOO Define Names "More >>" options (print range,
//! filter, repeat row, repeat column; `ScRangeData` `RT_*` types), the
//! `SetNameUsage` op with undo, and the ODS `table:range-usable-as` round
//! trip.
#![allow(clippy::unwrap_used)]

mod common;

use common::Fixture;
use tally_api::{DocumentService, NameUsage, Op};

fn usage_of(f: &Fixture, name: &str) -> NameUsage {
    f.svc
        .names(f.doc)
        .unwrap()
        .into_iter()
        .find(|n| n.name == name)
        .unwrap()
        .usage
}

fn define(name: &str, refers_to: &str) -> Op {
    Op::DefineName {
        name: name.into(),
        scope: None,
        refers_to: refers_to.into(),
    }
}

fn set_usage(name: &str, usage: NameUsage) -> Op {
    Op::SetNameUsage {
        name: name.into(),
        scope: None,
        usage,
    }
}

const ALL: NameUsage = NameUsage {
    print_range: true,
    filter: true,
    repeat_row: true,
    repeat_column: true,
};

#[test]
fn set_usage_with_undo_redo() {
    let mut f = Fixture::new();
    let r = f.apply(vec![define("Crit", "$Sheet1.$A$1:$B$3")]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(usage_of(&f, "Crit"), NameUsage::default());
    let flags = NameUsage {
        filter: true,
        repeat_row: true,
        ..NameUsage::default()
    };
    let r = f.apply(vec![set_usage("crit", flags)]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(usage_of(&f, "Crit"), flags, "lookup is case-insensitive");
    f.svc.undo(f.doc).unwrap();
    assert_eq!(usage_of(&f, "Crit"), NameUsage::default());
    f.svc.redo(f.doc).unwrap();
    assert_eq!(usage_of(&f, "Crit"), flags);
}

#[test]
fn define_and_flags_in_one_transaction_undo_together() {
    let mut f = Fixture::new();
    let r = f.apply(vec![
        define("Area", "$Sheet1.$A$1:$D$9"),
        set_usage("Area", ALL),
    ]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(usage_of(&f, "Area"), ALL);
    f.svc.undo(f.doc).unwrap();
    assert!(f.svc.names(f.doc).unwrap().is_empty());
}

#[test]
fn redefining_keeps_flags_deleting_clears_them() {
    let mut f = Fixture::new();
    f.apply(vec![define("P", "$Sheet1.$A$1:$B$2"), set_usage("P", ALL)]);
    f.apply(vec![define("P", "$Sheet1.$A$1:$C$3")]);
    assert_eq!(usage_of(&f, "P"), ALL, "a redefinition keeps the flags");
    f.apply(vec![Op::DeleteName {
        name: "P".into(),
        scope: None,
    }]);
    f.apply(vec![define("P", "$Sheet1.$A$1")]);
    assert_eq!(
        usage_of(&f, "P"),
        NameUsage::default(),
        "a deleted name comes back without flags"
    );
}

#[test]
fn unknown_name_is_an_error() {
    let mut f = Fixture::new();
    let r = f.apply(vec![set_usage("Nope", ALL)]);
    assert!(!r.errors.is_empty());
}

#[test]
fn flags_round_trip_through_ods() {
    let mut f = Fixture::new();
    f.apply(vec![
        define("Crit", "$Sheet1.$A$1:$B$3"),
        set_usage(
            "Crit",
            NameUsage {
                filter: true,
                ..NameUsage::default()
            },
        ),
        define("Titles", "$Sheet1.$A$1:$Z$2"),
        set_usage(
            "Titles",
            NameUsage {
                repeat_row: true,
                repeat_column: true,
                ..NameUsage::default()
            },
        ),
        define("Plain", "$Sheet1.$C$3"),
        define("Rate", "0.07"),
    ]);
    let mut bytes = std::io::Cursor::new(Vec::new());
    tally_io_ods::write_ods(&f.data(), &mut bytes).unwrap();
    let back = tally_io_ods::read_ods(
        std::io::Cursor::new(bytes.into_inner()),
        &tally_io_ods::Limits::default(),
    )
    .unwrap();
    let wb = tally_engine::Workbook::from_data(&back);
    let get = |n: &str| {
        let (_, e) = wb.names.defined().find(|(_, e)| &*e.name == n).unwrap();
        NameUsage {
            print_range: e.usage.print_range,
            filter: e.usage.filter,
            repeat_row: e.usage.repeat_row,
            repeat_column: e.usage.repeat_column,
        }
    };
    assert_eq!(
        get("Crit"),
        NameUsage {
            filter: true,
            ..NameUsage::default()
        }
    );
    assert_eq!(
        get("Titles"),
        NameUsage {
            repeat_row: true,
            repeat_column: true,
            ..NameUsage::default()
        }
    );
    assert_eq!(get("Plain"), NameUsage::default());
    assert_eq!(get("Rate"), NameUsage::default());
}
