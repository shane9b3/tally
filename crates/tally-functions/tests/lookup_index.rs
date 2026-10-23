//! Differential tests: exact-match lookups answered from a
//! [`tally_functions::LookupIndex`] must equal the linear scan, over
//! randomised lines mixing numbers, booleans, text in several cases,
//! wildcard characters, duplicates, errors and gaps, under every
//! combination of the text-matching settings.
#![allow(clippy::unwrap_used)]

use tally_functions::mock::{call, MockEvalContext};
use tally_functions::EvalValue;
use tally_types::{CriteriaMode, ErrorKind, Value};

/// xorshift64*: deterministic and dependency-free.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// Cell contents and keys are drawn from this pool; `None` is an empty cell.
fn pool() -> Vec<Option<Value>> {
    let mut v: Vec<Option<Value>> = vec![None, None, None];
    for n in [
        0.0,
        -0.0,
        1.0,
        2.0,
        3.0,
        5.0,
        7.0,
        0.3,
        0.1 + 0.2,
        1e20,
        1e20 * (1.0 + 1e-16),
        -4.5,
        42.0,
        1000.0,
    ] {
        v.push(Some(Value::Number(n)));
    }
    v.push(Some(Value::Bool(true)));
    v.push(Some(Value::Bool(false)));
    for t in [
        "apple", "Apple", "APPLE", "banana", "Banana", "a*e", "a?ple", "b~c", "ß", "SS", "straße",
        "STRASSE", "Σ", "σ", "", "5", "TRUE", "true", "#N/A", "1,000", "x y", "*", "?", "~", "~*",
        "ap", "pp",
    ] {
        v.push(Some(Value::text(t)));
    }
    v.push(Some(Value::Error(ErrorKind::NA)));
    v.push(Some(Value::Error(ErrorKind::Div0)));
    v
}

fn keys() -> Vec<Value> {
    let mut k: Vec<Value> = pool().into_iter().flatten().collect();
    for t in ["a*", "?pple", "*e", "b~~c", "zzz", "APP*", "1000", "1,000"] {
        k.push(Value::text(t));
    }
    k.push(Value::Number(1e20 * (1.0 + 4e-15)));
    k.push(Value::Number(99.0));
    k.push(Value::Empty);
    k
}

fn col_name(c: u32) -> String {
    let mut s = String::new();
    let mut n = c + 1;
    while n > 0 {
        let r = (n - 1) % 26;
        s.insert(0, (b'A' + r as u8) as char);
        n = (n - 1) / 26;
    }
    s
}

fn key_arg(v: &Value) -> EvalValue {
    match v {
        Value::Empty => EvalValue::Missing,
        other => EvalValue::Scalar(other.clone()),
    }
}

/// One randomised sheet: a vertical line in A1:A{n} (B holds the row),
/// a horizontal one in row 300 (row 301 holds the column).
fn sheet(rng: &mut Rng, n: u32) -> MockEvalContext {
    let pool = pool();
    let mut c = MockEvalContext::new();
    for r in 0..n {
        // Draw from a small slice of the pool now and then to force duplicates.
        let v = if rng.below(4) == 0 {
            pool[rng.below(8)].clone()
        } else {
            pool[rng.below(pool.len())].clone()
        };
        if let Some(v) = v.clone() {
            c.set(0, &format!("A{}", r + 1), v);
        }
        c.set(0, &format!("B{}", r + 1), f64::from(r + 1));
        if let Some(v) = v {
            c.set(0, &format!("{}300", col_name(r)), v);
        }
        c.set(0, &format!("{}301", col_name(r)), f64::from(r + 1));
    }
    c
}

fn results(c: &mut MockEvalContext, n: u32, key: &Value) -> Vec<Value> {
    let col = c.range(0, &format!("A1:A{n}"));
    let table = c.range(0, &format!("A1:B{n}"));
    let row = c.range(0, &format!("A300:{}301", col_name(n - 1)));
    let row1 = c.range(0, &format!("A300:{}300", col_name(n - 1)));
    let k = key_arg(key);
    let num = EvalValue::number;
    vec![
        call(c, "MATCH", vec![k.clone(), col.clone(), num(0.0)]),
        call(c, "MATCH", vec![k.clone(), row1.clone(), num(0.0)]),
        call(c, "MATCH", vec![k.clone(), col.clone(), num(1.0)]),
        call(c, "MATCH", vec![k.clone(), col.clone(), num(-1.0)]),
        call(
            c,
            "VLOOKUP",
            vec![k.clone(), table.clone(), num(2.0), EvalValue::bool(false)],
        ),
        call(c, "VLOOKUP", vec![k.clone(), table.clone(), num(2.0)]),
        call(
            c,
            "HLOOKUP",
            vec![k.clone(), row.clone(), num(2.0), EvalValue::bool(false)],
        ),
        call(c, "HLOOKUP", vec![k.clone(), row, num(2.0)]),
        call(c, "LOOKUP", vec![k, col]),
    ]
}

#[test]
fn indexed_lookups_equal_linear_scans() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let keys = keys();
    let mut compared = 0usize;
    for round in 0..30 {
        let n = [1, 3, 20, 70, 150][round % 5];
        let mut c = sheet(&mut rng, n);
        for mode in [
            CriteriaMode::Wildcards,
            CriteriaMode::Literal,
            CriteriaMode::Regex,
        ] {
            for case_sensitive in [false, true] {
                for whole in [true, false] {
                    c.settings.criteria_mode = mode;
                    c.settings.case_sensitive = case_sensitive;
                    c.settings.whole_cell_match = whole;
                    for key in &keys {
                        c.set_lookup_index(false);
                        let linear = results(&mut c, n, key);
                        c.set_lookup_index(true);
                        let indexed = results(&mut c, n, key);
                        assert_eq!(
                            linear, indexed,
                            "n={n} key={key:?} mode={mode:?} cs={case_sensitive} whole={whole}"
                        );
                        compared += linear.len();
                    }
                }
            }
        }
    }
    assert!(compared > 100_000);
}

/// The index really answers (not only the fallback): a first match wins
/// over later duplicates, text matches case-insensitively, numbers by
/// approximate equality, and numbers never match text.
#[test]
fn indexed_lookup_rules() {
    let mut c = MockEvalContext::new();
    c.set_lookup_index(true);
    let rows: [Value; 8] = [
        Value::text("5"),
        Value::Error(ErrorKind::NA),
        Value::Number(5.0),
        Value::text("Pear"),
        Value::text("pear"),
        Value::Bool(true),
        Value::Number(0.1 + 0.2),
        Value::Number(5.0),
    ];
    for (i, v) in rows.iter().enumerate() {
        c.set(0, &format!("A{}", i + 1), v.clone());
    }
    let col = c.range(0, "A1:A8");
    let m = |c: &mut MockEvalContext, k: EvalValue| {
        call(c, "MATCH", vec![k, col.clone(), EvalValue::number(0.0)])
    };
    assert_eq!(m(&mut c, EvalValue::number(5.0)), Value::Number(3.0));
    assert_eq!(m(&mut c, EvalValue::text("5")), Value::Number(1.0));
    assert_eq!(m(&mut c, EvalValue::text("PEAR")), Value::Number(4.0));
    assert_eq!(m(&mut c, EvalValue::bool(true)), Value::Number(6.0));
    assert_eq!(m(&mut c, EvalValue::number(1.0)), Value::Number(6.0));
    assert_eq!(m(&mut c, EvalValue::number(0.3)), Value::Number(7.0));
    assert_eq!(m(&mut c, EvalValue::text("p*")), Value::Number(4.0));
    assert_eq!(
        m(&mut c, EvalValue::number(6.0)),
        Value::Error(ErrorKind::NA)
    );
    c.settings.case_sensitive = true;
    assert_eq!(m(&mut c, EvalValue::text("pear")), Value::Number(5.0));
}
