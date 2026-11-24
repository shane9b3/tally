//! Names of an `.xls` name each other by record; Tally's formulas name
//! them by spelling (a sheet's own name first, then the workbook's). The
//! reader must hand the names over so that a cell which uses a record
//! shows that record's value or an error, never the value of another
//! record that happens to answer to the same spelling: with names of one
//! spelling in several scopes, names used before they are defined, names
//! the engine defines under another spelling (`TAX2024` as `TAX2024_`),
//! records that are not read, and circles.
//!
//! The workbooks are built in code (the BIFF helpers of the `.xls`
//! reader's tests). Notation in the comments: `name / scope = formula`,
//! `recN` the N-th `NAME` record.
#![allow(clippy::unwrap_used, clippy::expect_used)]
#![cfg(feature = "io")]

#[path = "../../tally-io-xls/tests/biff/mod.rs"]
mod biff;

use std::path::PathBuf;

use biff::{defined_name, formula, ptg_func, ptg_int, ptg_name, ptg_num, ptg_ref3d};
use tally_api::DocumentService;
use tally_engine::EngineService;
use tally_types::{CellAddr, ErrorKind, Value};

/// One `NAME` record: its name, flags, sheet (1-based; 0 the workbook)
/// and formula.
struct Rec {
    name: &'static str,
    flags: u16,
    itab: u16,
    rgce: Vec<u8>,
}

fn r(name: &'static str, itab: u16, rgce: Vec<u8>) -> Rec {
    Rec {
        name,
        flags: 0,
        itab,
        rgce,
    }
}

/// A name in a cell formula: `PtgName` (value class) with its index.
fn used(i: u32) -> Vec<u8> {
    let mut d = vec![0x43];
    d.extend(i.to_le_bytes());
    d
}

/// A formula the reader does not read: PHONETIC(first sheet!A1).
fn bad() -> Vec<u8> {
    [ptg_ref3d(0, 0), ptg_func(360)].concat()
}

/// `recN + k`.
fn plus(n: u32, k: u16) -> Vec<u8> {
    [ptg_name(n), ptg_int(k), vec![0x03]].concat()
}

/// `recA + recB`.
fn sum(a: u32, b: u32) -> Vec<u8> {
    [ptg_name(a), ptg_name(b), vec![0x03]].concat()
}

/// A workbook with the records `recs`, the sheets `sheets`, formula cells
/// `(sheet, row, col, formula)` (stored result 0) and numbers `(sheet,
/// row, col, value)`.
fn book(
    recs: &[Rec],
    sheets: &[&str],
    cells: &[(usize, u16, u16, Vec<u8>)],
    nums: &[(usize, u16, u16, f64)],
) -> Vec<u8> {
    let mut g = biff::globals(&[], &[], &[]);
    g.extend(biff::externsheet(sheets.len() as u16));
    for n in recs {
        g.extend(defined_name(n.name, n.flags, n.itab, &n.rgce));
    }
    let mut body: Vec<(&str, Vec<u8>)> = Vec::new();
    for (i, s) in sheets.iter().enumerate() {
        let mut all: Vec<(u16, u16, Vec<u8>)> = Vec::new();
        for (si, row, col, v) in nums {
            if *si == i {
                all.push((*row, *col, biff::number(*row, *col, 15, *v)));
            }
        }
        for (si, row, col, rg) in cells {
            if *si == i {
                all.push((*row, *col, formula(*row, *col, 15, 0.0, rg)));
            }
        }
        all.sort_by_key(|x| (x.0, x.1));
        let mut d = Vec::new();
        if all.is_empty() {
            d.extend(biff::number(0, 0, 15, 1.0));
        }
        for a in all {
            d.extend(a.2);
        }
        d.extend(biff::window2(0x00B6));
        body.push((s, d));
    }
    biff::file(biff::stream(&g, &body))
}

/// An opened and recalculated workbook.
struct Opened {
    svc: EngineService,
    doc: tally_api::DocId,
    ids: Vec<tally_types::SheetId>,
    warnings: Vec<String>,
}

impl Opened {
    fn val(&self, sheet: usize, row: u32, col: u32) -> Value {
        self.svc
            .document(self.doc)
            .unwrap()
            .wb
            .value(CellAddr::new(self.ids[sheet], row, col))
    }

    /// (name, sheet index) of the names, sorted.
    fn names(&self) -> Vec<(String, Option<usize>)> {
        let mut names: Vec<_> = self
            .svc
            .names(self.doc)
            .unwrap()
            .iter()
            .map(|n| {
                let sheet = n
                    .scope
                    .map(|s| self.ids.iter().position(|i| *i == s).unwrap());
                (n.name.clone(), sheet)
            })
            .collect();
        names.sort();
        names
    }
}

fn temp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("tally-xls-names-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d.join(format!("{tag}.xls"))
}

fn open(tag: &str, bytes: &[u8]) -> Opened {
    let path = temp(tag);
    std::fs::write(&path, bytes).unwrap();
    let mut svc = EngineService::with_default_codec();
    let info = svc
        .open(&path)
        .unwrap_or_else(|e| panic!("{tag}: {}", e.message));
    svc.recalc_all(info.doc).unwrap();
    let _ = std::fs::remove_file(&path);
    Opened {
        doc: info.doc,
        ids: info.sheets.iter().map(|s| s.id).collect(),
        warnings: info.warnings.clone(),
        svc,
    }
}

const S2: [&str; 2] = ["Main", "Other"];
const S3: [&str; 3] = ["Main", "Other", "Third"];

fn num(v: f64) -> Value {
    Value::Number(v)
}

fn is_error(v: &Value) -> bool {
    matches!(v, Value::Error(_))
}

fn left_out(o: &Opened) -> usize {
    o.warnings
        .iter()
        .find(|w| w.contains("defined name(s) with a formula"))
        .map_or(0, |w| w.split(' ').next().unwrap().parse().unwrap())
}

/// A name used before it is defined, reached through a name of the
/// workbook from another sheet (the records are in alphabetical order, as
/// Excel writes them).
#[test]
fn a_name_used_before_its_record() {
    // A / Main = rec3 + 0, B / workbook = rec1 + 0, T / Main = 7,
    // T / Other = 100.
    let recs = [
        r("A", 1, plus(3, 0)),
        r("B", 0, plus(1, 0)),
        r("T", 1, ptg_int(7)),
        r("T", 2, ptg_int(100)),
    ];
    let o = open(
        "h1",
        &book(&recs, &S2, &[(0, 0, 1, used(2)), (1, 0, 1, used(2))], &[]),
    );
    assert_eq!(o.val(0, 0, 1), num(7.0));
    assert_eq!(o.val(1, 0, 1), num(7.0));
    // The same with the T of the workbook.
    let recs = [
        r("A", 1, plus(3, 0)),
        r("B", 0, plus(1, 0)),
        r("T", 0, ptg_int(7)),
        r("T", 2, ptg_int(100)),
    ];
    let o = open(
        "h1b",
        &book(&recs, &S2, &[(0, 0, 1, used(2)), (1, 0, 1, used(2))], &[]),
    );
    assert_eq!(o.val(0, 0, 1), num(7.0));
    assert_eq!(o.val(1, 0, 1), num(7.0));
    // A / workbook = rec2 + 0, T / workbook = 7, T / Other = 100.
    let recs = [
        r("A", 0, plus(2, 0)),
        r("T", 0, ptg_int(7)),
        r("T", 2, ptg_int(100)),
    ];
    let o = open(
        "h1c",
        &book(&recs, &S2, &[(0, 0, 1, used(1)), (1, 0, 1, used(1))], &[]),
    );
    assert_eq!(o.val(0, 0, 1), num(7.0));
    assert_eq!(o.val(1, 0, 1), num(7.0));
}

#[test]
fn a_later_namesake_in_another_scope() {
    // T / workbook = 5, X / Other = rec3 + 0, T / Other = 7.
    let recs = [
        r("T", 0, ptg_int(5)),
        r("X", 2, plus(3, 0)),
        r("T", 2, ptg_int(7)),
    ];
    let o = open("m1a1", &book(&recs, &S2, &[(1, 0, 1, used(2))], &[]));
    assert_eq!(o.val(1, 0, 1), num(7.0));
    // T / Main = 111, X / Other = rec3 + 0, T / workbook = 5.
    let recs = [
        r("T", 1, ptg_int(111)),
        r("X", 2, plus(3, 0)),
        r("T", 0, ptg_int(5)),
    ];
    let o = open("m1a2", &book(&recs, &S2, &[(1, 0, 1, used(2))], &[]));
    assert_eq!(o.val(1, 0, 1), num(5.0));
}

#[test]
fn names_in_a_circle_are_left_out() {
    // FOO / workbook = 1, FOO / Main = 2 + rec2 (itself, not the
    // workbook's: 3 would be another record's value).
    let recs = [
        r("FOO", 0, ptg_int(1)),
        r("FOO", 1, [ptg_int(2), ptg_name(2), vec![0x03]].concat()),
        // Two that use each other, and one on them.
        r("Ping", 0, plus(4, 1)),
        r("Pong", 0, plus(3, 1)),
        r("OnPing", 0, plus(3, 1)),
    ];
    let cells = [
        (0, 0, 1, used(2)),
        (1, 0, 1, used(1)),
        (0, 0, 2, used(3)),
        (0, 0, 3, used(5)),
    ];
    let o = open("circle", &book(&recs, &S2, &cells, &[]));
    assert!(is_error(&o.val(0, 0, 1)), "{:?}", o.val(0, 0, 1));
    assert_eq!(o.val(1, 0, 1), num(1.0));
    assert!(is_error(&o.val(0, 0, 2)));
    assert!(is_error(&o.val(0, 0, 3)));
    // The sheet's FOO stays a name of its sheet, without a value.
    assert_eq!(
        o.names(),
        [("FOO".to_string(), None), ("FOO".to_string(), Some(0))]
    );
    assert_eq!(left_out(&o), 4);
}

/// `TAX2024` is a cell in Tally's grid: the engine defines such a name as
/// `TAX2024_`. With a record spelled `TAX2024_` in the file, one entry
/// would answer to both spellings.
#[test]
fn a_name_the_engine_respells_onto_another_record() {
    for (tag, first, second) in [
        ("m1b", ("TAX2024", 2), ("TAX2024_", 1)),
        ("m1b-r", ("TAX2024_", 1), ("TAX2024", 2)),
    ] {
        let recs = [
            r(first.0, 0, ptg_int(first.1)),
            r(second.0, 0, ptg_int(second.1)),
            r("OnFirst", 0, plus(1, 0)),
            r("OnSecond", 0, plus(2, 0)),
        ];
        let cells = [
            (0, 0, 1, used(1)),
            (0, 0, 2, used(2)),
            (0, 0, 3, used(3)),
            (0, 0, 4, used(4)),
        ];
        let o = open(tag, &book(&recs, &S2, &cells, &[]));
        // `TAX2024_` and what uses it are 1; `TAX2024` (2) and what uses
        // it are left out, with the warning.
        for (col, value) in [(1, first.1), (2, second.1), (3, first.1), (4, second.1)] {
            let got = o.val(0, 0, col);
            if value == 1 {
                assert_eq!(got, num(1.0), "{tag} column {col}");
            } else {
                assert!(is_error(&got), "{tag} column {col}: {got:?}");
            }
        }
        assert_eq!(left_out(&o), 2, "{tag}: {:?}", o.warnings);
    }
    // TAX2024 / Main = 2, TAX2024_ / workbook = 1, Y / Main = rec2 + 0,
    // Z / workbook = rec2 + 0.
    let recs = [
        r("TAX2024", 1, ptg_int(2)),
        r("TAX2024_", 0, ptg_int(1)),
        r("Y", 1, plus(2, 0)),
        r("Z", 0, plus(2, 0)),
    ];
    let cells = [
        (0, 0, 1, used(3)),
        (0, 0, 2, used(2)),
        (0, 0, 3, used(4)),
        (1, 0, 1, used(4)),
        (0, 0, 4, used(1)),
    ];
    let o = open("m1b-c", &book(&recs, &S2, &cells, &[]));
    for (sheet, col) in [(0, 1), (0, 2), (0, 3), (1, 1)] {
        assert_eq!(o.val(sheet, 0, col), num(1.0), "sheet {sheet} column {col}");
    }
    assert!(is_error(&o.val(0, 0, 4)));
    // Without such a record the name is read, under the other spelling.
    let recs = [r("TAX2024", 0, ptg_int(2)), r("Y", 0, plus(1, 0))];
    let o = open(
        "m1b-alone",
        &book(&recs, &S2, &[(0, 0, 1, used(1)), (0, 0, 2, used(2))], &[]),
    );
    assert_eq!(o.val(0, 0, 1), num(2.0));
    assert_eq!(o.val(0, 0, 2), num(2.0));
    assert_eq!(o.names()[0].0, "TAX2024_");
}

#[test]
fn a_later_record_of_a_name_is_not_defined_in_place_of_the_first() {
    // Baz / workbook = rec2 + rec3, FOO / Other = 2 + rec3, FOO / Other =
    // 4: rec2 uses a record that is not the one defined, so it is left
    // out; rec3 must not be defined in its place.
    for second in ["FOO", "Foo"] {
        let recs = [
            r("Baz", 0, sum(2, 3)),
            r("FOO", 2, [ptg_int(2), ptg_name(3), vec![0x03]].concat()),
            r(second, 2, ptg_int(4)),
        ];
        let cells = [(1, 0, 1, used(2)), (0, 0, 1, used(1))];
        let o = open("la", &book(&recs, &S2, &cells, &[]));
        assert!(is_error(&o.val(1, 0, 1)), "{second}: {:?}", o.val(1, 0, 1));
        assert!(is_error(&o.val(0, 0, 1)));
        assert_eq!(o.names(), []);
        assert_eq!(left_out(&o), 3);
    }
    // The first is not read at all (PHONETIC), the workbook has the name
    // too: Foo / Main = ?, Foo / Main = 7, Foo / workbook = 5.
    let recs = [
        r("Foo", 1, bad()),
        r("Foo", 1, ptg_int(7)),
        r("Foo", 0, ptg_int(5)),
    ];
    let cells = [(0, 0, 1, used(1)), (1, 0, 1, used(3))];
    let o = open("la2", &book(&recs, &S2, &cells, &[]));
    assert!(is_error(&o.val(0, 0, 1)), "{:?}", o.val(0, 0, 1));
    assert_eq!(o.val(1, 0, 1), num(5.0));
}

#[test]
fn a_record_of_a_sheet_that_is_not_a_name_still_stands_before_the_workbooks() {
    // Mac / Main is a macro (not a name of the model), Mac / workbook =
    // 6; Bar / Other is not read, BAR / workbook = 9, UsesBar / Other =
    // rec3 + 0; TAX2024 / Main is not read, TAX2024 / workbook = 5.
    let recs = [
        Rec {
            flags: 0x0008,
            ..r("Mac", 1, ptg_int(1))
        },
        r("Mac", 0, ptg_int(6)),
        r("Bar", 2, bad()),
        r("BAR", 0, ptg_int(9)),
        r("UsesBar", 2, plus(3, 0)),
        r("TAX2024", 1, bad()),
        r("TAX2024", 0, ptg_int(5)),
    ];
    let cells = [
        (0, 0, 1, used(1)),
        (1, 0, 1, used(2)),
        (1, 0, 2, used(3)),
        (0, 0, 2, used(4)),
        (1, 0, 3, used(5)),
        (0, 0, 3, used(6)),
        (1, 0, 4, used(7)),
    ];
    let o = open("lb", &book(&recs, &S2, &cells, &[]));
    assert_eq!(o.val(0, 0, 1), Value::Error(ErrorKind::Name));
    assert_eq!(o.val(1, 0, 1), num(6.0));
    assert_eq!(o.val(1, 0, 2), Value::Error(ErrorKind::Name));
    assert_eq!(o.val(0, 0, 2), num(9.0));
    assert!(is_error(&o.val(1, 0, 3)));
    assert_eq!(o.val(0, 0, 3), Value::Error(ErrorKind::Name));
    assert_eq!(o.val(1, 0, 4), num(5.0));
}

/// The sheet of a name's base cell is chosen for the names it uses; its
/// relative references still are offsets from the cell that uses it, on
/// the sheet they name.
#[test]
fn relative_references_with_a_base_cell_on_another_sheet() {
    // Loc / Other = 7; Rel / workbook = rec1 * Main!(same row, one
    // column left); Same / workbook = rec1 * Main!(same cell); Pre /
    // Third = as Rel.
    let left = vec![0x3A, 0, 0, 0, 0, 0xFF, 0xC0];
    let same = vec![0x3A, 0, 0, 0, 0, 0x00, 0xC0];
    let recs = [
        r("Loc", 2, ptg_int(7)),
        r("Rel", 0, [ptg_name(1), left.clone(), vec![0x05]].concat()),
        r("Same", 0, [ptg_name(1), same, vec![0x05]].concat()),
        r("Pre", 3, [ptg_name(1), left, vec![0x05]].concat()),
    ];
    let cells = [
        (0, 2, 2, used(2)),
        (1, 2, 2, used(2)),
        (1, 2, 3, used(3)),
        (2, 2, 2, used(4)),
        (2, 5, 5, used(3)),
    ];
    let nums = [
        (0, 2, 1, 10.0),
        (1, 2, 1, 1000.0),
        (0, 2, 3, 3.0),
        (2, 2, 1, 5000.0),
        (0, 5, 5, 11.0),
    ];
    let o = open("rel", &book(&recs, &S3, &cells, &nums));
    assert_eq!(o.val(0, 2, 2), num(70.0));
    assert_eq!(o.val(1, 2, 2), num(70.0));
    assert_eq!(o.val(1, 2, 3), num(21.0));
    assert_eq!(o.val(2, 2, 2), num(70.0));
    assert_eq!(o.val(2, 5, 5), num(77.0));
}

/// A name of a sheet whose formula the reader takes but the engine's
/// parser would refuse (too deep) must not leave the sheet's cells with
/// the workbook's namesake.
#[test]
fn a_deep_name_of_a_sheet_and_its_workbook_namesake() {
    let depths = [50usize, 150, 250, 500, 1000];
    let names = ["Da", "Db", "Dc", "Dd", "De"];
    for kind in 0..3 {
        let mut recs = Vec::new();
        let mut cells = Vec::new();
        for (i, d) in depths.iter().enumerate() {
            let mut rg = ptg_int(1);
            for _ in 0..*d {
                match kind {
                    0 => rg.push(0x13),
                    1 => rg.extend(ptg_func(24)),
                    _ => {
                        rg.extend(ptg_int(1));
                        rg.extend([0x03, 0x15]);
                    }
                }
            }
            recs.push(r(names[i], 1, rg));
            recs.push(r(names[i], 0, ptg_int(5)));
            cells.push((0usize, 0u16, 1 + i as u16, used(2 * i as u32 + 1)));
        }
        let o = open("deep", &book(&recs, &S2, &cells, &[]));
        for (i, d) in depths.iter().enumerate() {
            // 1 negated d times, ABS of 1, or 1 + d: never the 5 of the
            // workbook's name.
            let want = match kind {
                0 if d % 2 == 1 => -1.0,
                0 | 1 => 1.0,
                _ => 1.0 + *d as f64,
            };
            let got = o.val(0, 0, 1 + i as u32);
            assert!(
                is_error(&got) || got == num(want),
                "kind {kind} depth {d}: {got:?}"
            );
        }
    }
}

/// The engine keeps a name it respells (`K70000` as `K70000_`) under the
/// key of the file's spelling too, and keys are lower case: the Kelvin
/// sign's is `k`. A name spelled with it and `70000` has the key the
/// respelled one still answers to.
#[test]
fn a_spelling_with_the_key_of_another_names_alias() {
    const KELVIN: &str = "\u{212a}70000";
    // K70000 / workbook = 2, (Kelvin)70000 / workbook = 1, X = rec1 + 0,
    // Y = rec2 + 0; and the two records the other way round. Neither is
    // a name: each would answer to the other's spelling.
    for (tag, a, b) in [
        ("kelvin-1", "K70000", KELVIN),
        ("kelvin-2", KELVIN, "K70000"),
    ] {
        let recs = [
            r(a, 0, ptg_int(2)),
            r(b, 0, ptg_int(1)),
            r("X", 0, plus(1, 0)),
            r("Y", 0, plus(2, 0)),
            r("Good", 0, ptg_int(3)),
        ];
        let cells = [
            (0, 0, 1, used(1)),
            (0, 0, 2, used(2)),
            (0, 0, 3, used(3)),
            (0, 0, 4, used(4)),
            (0, 0, 5, used(5)),
        ];
        let o = open(tag, &book(&recs, &S2, &cells, &[]));
        for col in 1..=4 {
            assert!(
                is_error(&o.val(0, 0, col)),
                "{tag} {col}: {:?}",
                o.val(0, 0, col)
            );
        }
        assert_eq!(o.val(0, 0, 5), num(3.0));
        // All four are counted, and the engine left nothing out unseen.
        assert_eq!(left_out(&o), 4, "{tag}: {:?}", o.warnings);
        assert!(
            !o.warnings.iter().any(|w| w.contains("defined twice")),
            "{:?}",
            o.warnings
        );
    }
    // K70000 / Main is not read, K70000 / workbook = 5, (Kelvin)70000 /
    // Main = 7: the Main cell on rec1 must not show 7 or 5.
    let recs = [
        r("K70000", 1, bad()),
        r("K70000", 0, ptg_int(5)),
        r(KELVIN, 1, ptg_int(7)),
    ];
    let cells = [(0, 0, 1, used(1)), (0, 0, 2, used(3)), (1, 0, 1, used(2))];
    let o = open("kelvin-3", &book(&recs, &S2, &cells, &[]));
    assert!(is_error(&o.val(0, 0, 1)), "{:?}", o.val(0, 0, 1));
    assert!(is_error(&o.val(0, 0, 2)), "{:?}", o.val(0, 0, 2));
    assert!(is_error(&o.val(1, 0, 1)), "{:?}", o.val(1, 0, 1));
    // Alone, each is a name.
    for (tag, name, listed) in [
        ("kelvin-4", "K70000", "K70000_"),
        ("kelvin-5", KELVIN, KELVIN),
    ] {
        let recs = [r(name, 0, ptg_int(2)), r("X", 0, plus(1, 0))];
        let cells = [(0, 0, 1, used(1)), (0, 0, 2, used(2))];
        let o = open(tag, &book(&recs, &S2, &cells, &[]));
        assert_eq!(o.val(0, 0, 1), num(2.0), "{tag}");
        assert_eq!(o.val(0, 0, 2), num(2.0), "{tag}");
        assert!(o.names().iter().any(|n| n.0 == listed), "{tag}");
    }
    // Lower cases that are one, upper cases that are not (the two sharp
    // s): not one name, so neither is defined.
    let recs = [
        r("Stra\u{df}e", 0, ptg_int(1)),
        r("STRA\u{1e9e}E", 0, ptg_int(2)),
        r("STRASSE", 0, ptg_int(4)),
    ];
    let cells = [(0, 0, 1, used(1)), (0, 0, 2, used(2)), (0, 0, 3, used(3))];
    let o = open("eszett", &book(&recs, &S2, &cells, &[]));
    assert!(is_error(&o.val(0, 0, 1)));
    assert!(is_error(&o.val(0, 0, 2)));
    assert_eq!(o.val(0, 0, 3), num(4.0));
}

/// Excel allows a dot in a defined name, and `Main.A1` there is that
/// name, never the cell A1 of the sheet `Main`; in Tally's formulas it
/// would be the cell. Such a name cannot be defined: the cells that use
/// it show `#NAME?`, not the value of that cell.
#[test]
fn a_name_spelled_like_a_cell_of_a_sheet() {
    let recs = [
        r("Main.A1", 0, ptg_int(1)),
        r("Other.IW1", 0, ptg_int(8)),
        r("\u{e4}b.A1", 0, ptg_int(2)),
        // A dot that makes no cell of a sheet: a name like any other.
        r("Tax.Rate", 0, ptg_num(0.25)),
        r("OnTax", 0, [ptg_name(4), ptg_int(4), vec![0x05]].concat()),
        r("OnMain", 0, plus(1, 0)),
    ];
    let cells = [
        (0, 0, 1, used(1)),
        (0, 0, 2, used(2)),
        (0, 0, 3, used(3)),
        (0, 0, 4, used(4)),
        (0, 0, 5, used(5)),
        (0, 0, 6, used(6)),
        // In a larger formula: Main.A1 + 1.
        (0, 1, 1, [used(1), ptg_int(1), vec![0x03]].concat()),
    ];
    let nums = [(0, 0, 0, 999.0), (2, 0, 0, 999.0)];
    let sheets = ["Main", "Other", "\u{c4}B"];
    let o = open("dotted", &book(&recs, &sheets, &cells, &nums));
    for (row, col) in [(0, 1), (0, 2), (0, 3), (0, 6), (1, 1)] {
        assert_eq!(
            o.val(0, row, col),
            Value::Error(ErrorKind::Name),
            "row {row} column {col}"
        );
    }
    assert_eq!(o.val(0, 0, 4), num(0.25));
    assert_eq!(o.val(0, 0, 5), num(1.0));
    assert_eq!(
        o.names(),
        [("OnTax".to_string(), None), ("Tax.Rate".to_string(), None)]
    );
    assert_eq!(left_out(&o), 4, "{:?}", o.warnings);
}

// ---------------------------------------------------------------- oracle

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        (self.next() >> 11) % n
    }
}

/// A generated record: its value is 2^index plus the values of the
/// records it uses; `bad` is a formula the reader does not read.
#[derive(Clone, Debug)]
struct Gen {
    name: &'static str,
    itab: u16,
    bad: bool,
    uses: Vec<usize>,
}

/// Whether record `i` uses itself, directly or through others.
fn cyclic(recs: &[Gen], i: usize, path: &mut Vec<usize>) -> bool {
    if path.contains(&i) {
        return true;
    }
    path.push(i);
    let c = recs[i].uses.iter().any(|&u| cyclic(recs, u, path));
    path.pop();
    c
}

/// The value of record `i` by the records it names; `None` for one that
/// has none (not read, or on one that is not).
fn by_record(recs: &[Gen], i: usize) -> Option<f64> {
    if recs[i].bad {
        return None;
    }
    let mut v = (1u64 << i) as f64;
    for &u in &recs[i].uses {
        v += by_record(recs, u)?;
    }
    Some(v)
}

/// Random workbooks of a few names over three sheets and the workbook,
/// spelled from `pool`, each `2^index + the records it uses` (earlier and
/// later ones, itself); a cell for every record on every sheet from which
/// Excel's own lookup of the spelling means that record. Every cell must
/// show the record's value or an error. Returns (cells, right, errors).
fn oracle(seed: u64, books: u32, pool: &[&'static str]) -> (u32, u32, u32) {
    let mut rng = Rng(seed);
    let (mut cells_n, mut right, mut errors) = (0, 0, 0);
    for round in 0..books {
        let n = 3 + (round as usize % 6);
        let mut recs: Vec<Gen> = Vec::new();
        for _ in 0..n {
            let name = pool[rng.below(pool.len() as u64) as usize];
            let itab = rng.below(4) as u16;
            let bad = rng.below(8) == 0;
            let mut uses = Vec::new();
            if !bad {
                for _ in 0..rng.below(3) {
                    uses.push(rng.below(n as u64) as usize);
                }
            }
            recs.push(Gen {
                name,
                itab,
                bad,
                uses,
            });
        }
        let raw: Vec<Rec> = recs
            .iter()
            .enumerate()
            .map(|(i, g)| {
                let rg = if g.bad {
                    bad()
                } else {
                    let mut d = ptg_num((1u64 << i) as f64);
                    for &u in &g.uses {
                        d.extend(ptg_name(u as u32 + 1));
                        d.push(0x03);
                    }
                    d
                };
                r(g.name, g.itab, rg)
            })
            .collect();
        // Two spellings are one name when they differ by case only, in
        // both directions: `Foo` and `FOO`, but not `K70000` and the
        // same with a Kelvin sign (one lower case, two upper cases), nor
        // `Straße` and `STRASSE`.
        let low = |g: &Gen| (g.name.to_lowercase(), g.name.to_uppercase());
        let mut cells = Vec::new();
        let mut expect = Vec::new();
        for i in 0..n {
            let first =
                (0..i).all(|j| low(&recs[j]) != low(&recs[i]) || recs[j].itab != recs[i].itab);
            if !first {
                continue;
            }
            for s in 0..3usize {
                let own = recs[i].itab as usize == s + 1;
                let through = recs[i].itab == 0
                    && !recs
                        .iter()
                        .any(|g| low(g) == low(&recs[i]) && g.itab as usize == s + 1);
                if own || through {
                    cells.push((s, i as u16, 1u16, used(i as u32 + 1)));
                    let want = if cyclic(&recs, i, &mut Vec::new()) {
                        None
                    } else {
                        by_record(&recs, i)
                    };
                    expect.push((s, i, want));
                }
            }
        }
        let o = open("oracle", &book(&raw, &S3, &cells, &[]));
        for (s, i, want) in &expect {
            cells_n += 1;
            match (o.val(*s, *i as u32, 1), want) {
                (Value::Error(_), _) => errors += 1,
                (Value::Number(x), Some(w)) if x == *w => right += 1,
                (got, _) => panic!(
                    "seed {seed:#x} round {round}: the cell on sheet {s} for record {} shows \
                     {got:?}, the record is {want:?}\n{recs:#?}",
                    i + 1
                ),
            }
        }
    }
    (cells_n, right, errors)
}

const PLAIN: [&str; 4] = ["Foo", "FOO", "Bar", "Baz"];
const RESPELLED: [&str; 5] = ["Foo", "FOO", "Bar", "TAX2024", "TAX2024_"];
/// `K70000` is defined as `K70000_` and still answers to `k70000`,
/// which is also the lower case of the name with a Kelvin sign.
const KELVIN: [&str; 5] = ["K70000", "\u{212a}70000", "k70000", "K70000_", "Foo"];
/// Spellings whose lower and upper cases do not pair up.
const UNICODE: [&str; 12] = [
    "Stra\u{df}e",
    "STRASSE",
    "STRA\u{1e9e}E",
    "\u{130}x",
    "ix",
    "\u{131}x",
    "\u{1c5}a",
    "\u{1c4}a",
    "\u{39f}\u{3a3}",
    "\u{3bf}\u{3c3}",
    "K70000",
    "\u{212a}70000",
];

#[test]
fn random_names_show_their_record_or_an_error() {
    let (cells, right, errors) = oracle(0x9E37_79B9_7F4A_7C15, 1_000, &RESPELLED);
    assert_eq!(cells, right + errors);
    // Not vacuous: a good share of the cells has its record's value.
    assert!(right * 4 > cells, "{right} of {cells}");
    let (cells, right, errors) = oracle(0x0123_4567_89AB_CDEF, 500, &PLAIN);
    assert_eq!(cells, right + errors);
    assert!(right * 4 > cells, "{right} of {cells}");
    let (cells, right, errors) = oracle(0x61C8_8646_80B5_83EB, 500, &KELVIN);
    assert_eq!(cells, right + errors);
    assert!(right > 0);
    let (cells, right, errors) = oracle(0x61C8_8646_80B5_83EB, 1_000, &UNICODE);
    assert_eq!(cells, right + errors);
    assert!(right * 4 > cells, "{right} of {cells}");
}

/// The same over more workbooks and seeds:
/// `cargo test -p tally-engine --features io --test xls_name_scopes -- --ignored`.
#[test]
#[ignore = "slow: 80,000 workbooks"]
fn random_names_show_their_record_or_an_error_at_length() {
    let mut seed = 0xDEAD_BEEF_CAFE_1234u64;
    let (mut cells, mut right) = (0, 0);
    for k in 0..20 {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(k) | 1;
        for pool in [&PLAIN[..], &RESPELLED[..], &KELVIN[..], &UNICODE[..]] {
            let (c, r, _) = oracle(seed, 1_000, pool);
            cells += c;
            right += r;
        }
    }
    println!("{cells} cells, {right} with their record's value");
    assert!(right * 4 > cells);
}
