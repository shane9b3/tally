//! The per-recalc lookup index (`tally_engine::lookup_cache`): results
//! equal the linear scan, and edits, formula results and row insertion or
//! deletion inside the looked-up range are always seen.
#![allow(clippy::unwrap_used)]

mod common;

use common::{input, Fixture};
use tally_api::Op;
use tally_functions::mock::{call, MockEvalContext};
use tally_functions::EvalValue;
use tally_types::{ErrorKind, Value};

const NA: Value = Value::Error(ErrorKind::NA);

fn n(x: f64) -> Value {
    Value::Number(x)
}

/// A1:A200 = 1..200 (as numbers), B1:B200 = "v1".."v200".
fn table(f: &mut Fixture) {
    let mut ops = Vec::new();
    for r in 1..=200 {
        ops.push(input(f.sheet, &format!("A{r}"), &r.to_string()));
        ops.push(input(f.sheet, &format!("B{r}"), &format!("v{r}")));
    }
    f.apply(ops);
}

/// D1:D40 look up 5, 10, ..., 200 with VLOOKUP; E1:E40 with MATCH.
fn lookups(f: &mut Fixture) {
    let mut ops = Vec::new();
    for r in 1..=40 {
        ops.push(input(
            f.sheet,
            &format!("D{r}"),
            &format!("=VLOOKUP({};$A$1:$B$200;2;0)", r * 5),
        ));
        ops.push(input(
            f.sheet,
            &format!("E{r}"),
            &format!("=MATCH({};$A$1:$A$200;0)", r * 5),
        ));
    }
    f.apply(ops);
}

#[test]
fn edit_in_lookup_column_then_incremental_recalc() {
    let mut f = Fixture::new();
    table(&mut f);
    lookups(&mut f);
    assert_eq!(f.value("D3"), Value::text("v15"));
    assert_eq!(f.value("E40"), n(200.0));

    // The key 15 disappears from row 15 and shows up earlier, in row 2.
    f.set("A15", "999");
    assert_eq!(f.value("D3"), NA);
    assert_eq!(f.value("E3"), NA);
    f.set("A2", "15");
    assert_eq!(f.value("D3"), Value::text("v2"));
    assert_eq!(f.value("E3"), n(2.0));

    // A formula in the column: its new result is what lookups see.
    f.set("H1", "7");
    f.set("A1", "=H1*5");
    assert_eq!(f.value("E7"), n(1.0)); // 35 is now first in A1
    f.set("H1", "8");
    assert_eq!(f.value("E7"), n(35.0));
    assert_eq!(f.value("E8"), n(1.0));
    assert_eq!(f.value("D8"), Value::text("v1"));

    // Text and case: a text key never matches the number 20.
    f.set("A20", "Twenty");
    assert_eq!(f.value("E4"), NA);
    f.set("E4", "=MATCH(\"TWENTY\";$A$1:$A$200;0)");
    assert_eq!(f.value("E4"), n(20.0));

    // Clearing a cell.
    f.set("A20", "");
    assert_eq!(f.value("E4"), NA);
}

#[test]
fn insert_and_delete_rows_inside_the_range() {
    let mut f = Fixture::new();
    table(&mut f);
    lookups(&mut f);
    let s = f.sheet;
    // Two rows inserted above row 10: the table now spans A1:B202.
    f.apply(vec![Op::InsertRows {
        sheet: s,
        at: 9,
        count: 2,
    }]);
    assert_eq!(f.value("E2"), n(12.0)); // 10 moved to row 12
    assert_eq!(f.value("E1"), n(5.0));
    assert_eq!(f.value("D2"), Value::text("v10"));
    f.apply(vec![input(s, "A10", "10"), input(s, "B10", "new")]);
    assert_eq!(f.value("E2"), n(10.0));
    assert_eq!(f.value("D2"), Value::text("new"));
    assert_eq!(f.value("E42"), n(202.0)); // E40 moved down too

    // Delete rows 1-12: 1..9, the new 10 and the old 10 are gone; row r
    // now holds r + 10. The formulas of old rows 11..40 are rows 1..30.
    f.apply(vec![Op::DeleteRows {
        sheet: s,
        at: 0,
        count: 12,
    }]);
    assert_eq!(f.value("A1"), n(11.0));
    assert_eq!(f.value("E1"), n(45.0)); // key 55
    assert_eq!(f.value("D1"), Value::text("v55"));
    assert_eq!(f.value("E30"), n(190.0)); // key 200
    f.set("A3", "55");
    assert_eq!(f.value("E1"), n(3.0));
    assert_eq!(f.value("D1"), Value::text("v13"));
}

/// xorshift64*.
struct Rng(u64);

impl Rng {
    fn below(&mut self, k: u64) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) % k
    }
}

const INPUTS: &[&str] = &[
    "",
    "1",
    "2",
    "3",
    "0",
    "=0.1+0.2",
    "0.3",
    "TRUE",
    "FALSE",
    "apple",
    "Apple",
    "APPLE",
    "a*e",
    "b?",
    "=NA()",
    "=1/0",
    "=2+3",
    "=\"pear\"",
    "Pear",
    "'5",
    "5",
    "",
];

/// The engine (indexed) agrees with the functions' linear scan over the
/// same values, for every key in the pool, across edits.
#[test]
fn randomised_engine_results_equal_linear_scan() {
    let mut rng = Rng(0x1234_5678_9ABC_DEF1);
    let mut f = Fixture::new();
    let s = f.sheet;
    let rows = 150u32;
    let mut ops = Vec::new();
    for r in 1..=rows {
        let t = INPUTS[rng.below(INPUTS.len() as u64) as usize];
        ops.push(input(s, &format!("A{r}"), t));
        ops.push(input(s, &format!("B{r}"), &r.to_string()));
    }
    let keys: Vec<&str> = INPUTS.iter().copied().filter(|t| !t.is_empty()).collect();
    for (i, k) in keys.iter().enumerate() {
        let r = i + 1;
        ops.push(input(s, &format!("C{r}"), k));
        ops.push(input(
            s,
            &format!("D{r}"),
            &format!("=MATCH(C{r};$A$1:$A${rows};0)"),
        ));
        ops.push(input(
            s,
            &format!("E{r}"),
            &format!("=VLOOKUP(C{r};$A$1:$B${rows};2;0)"),
        ));
        ops.push(input(
            s,
            &format!("F{r}"),
            &format!("=MATCH(C{r};$A$1:$A${rows};1)"),
        ));
    }
    f.apply(ops);
    for round in 0..6 {
        // Reference: the same values through the functions' linear scan.
        let mut m = MockEvalContext::new();
        for r in 1..=rows {
            let v = f.value(&format!("A{r}"));
            if v != Value::Empty {
                m.set(0, &format!("A{r}"), v);
            }
            m.set(0, &format!("B{r}"), f64::from(r));
        }
        let col = m.range(0, &format!("A1:A{rows}"));
        let tab = m.range(0, &format!("A1:B{rows}"));
        for (i, text) in keys.iter().enumerate() {
            let r = i + 1;
            let key = match f.value(&format!("C{r}")) {
                Value::Empty => EvalValue::Missing,
                v => EvalValue::Scalar(v),
            };
            let num = EvalValue::number;
            let want = [
                call(&mut m, "MATCH", vec![key.clone(), col.clone(), num(0.0)]),
                call(
                    &mut m,
                    "VLOOKUP",
                    vec![key.clone(), tab.clone(), num(2.0), EvalValue::bool(false)],
                ),
                call(&mut m, "MATCH", vec![key, col.clone(), num(1.0)]),
            ];
            let got = [
                f.value(&format!("D{r}")),
                f.value(&format!("E{r}")),
                f.value(&format!("F{r}")),
            ];
            assert_eq!(got, want, "round {round}, key {text}");
        }
        // Random edits inside the column before the next round.
        let mut ops = Vec::new();
        for _ in 0..10 {
            let r = 1 + rng.below(u64::from(rows));
            let t = INPUTS[rng.below(INPUTS.len() as u64) as usize];
            ops.push(input(s, &format!("A{r}"), t));
        }
        f.apply(ops);
    }
}
