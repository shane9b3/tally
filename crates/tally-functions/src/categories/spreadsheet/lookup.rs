//! VLOOKUP and HLOOKUP, plus the lookup machinery MATCH shares
//! (modelled on AOO `ScInterpreter::ScVLookup` / `ScHLookup` / `ScMatch`).
//!
//! Matching rules: a numeric lookup value (booleans are numbers) only ever
//! matches numbers, a text one only text. Exact mode matches text like a
//! criterion would (case-insensitive by default, wildcards or regex per the
//! document settings, whole-cell per `whole_cell_match`) but never
//! interprets a leading `<`, `>` or `=` as an operator. Sorted mode picks
//! the last cell whose value is `<=` the lookup value, scanning down (or
//! across) the sorted line and stopping at the first larger value of the
//! same type; cells of the other type, empties and errors are skipped.

use std::cmp::Ordering;
use std::sync::Arc;

use tally_types::{approx_equal, CalcSettings, CellRange, ErrorKind, Matrix, Value};

use crate::coerce::{parse_unambiguous, scalar_value};
use crate::criteria::Criterion;
use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry, ReturnKind};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

/// A lookup table: a reference or an in-memory matrix.
pub(super) enum Table {
    Range(CellRange),
    Mat(Arc<Matrix>),
}

impl Table {
    /// Builds a table from an argument; a scalar error propagates, other
    /// scalars are a 1x1 table, a missing argument is `#VALUE!`.
    pub(super) fn from_eval(v: EvalValue) -> Result<Table, ErrorKind> {
        match v {
            EvalValue::Range(r) => Ok(Table::Range(r)),
            EvalValue::Array(m) | EvalValue::Scalar(Value::Array(m)) => Ok(Table::Mat(m)),
            EvalValue::Scalar(Value::Error(e)) => Err(e),
            EvalValue::Scalar(s) => Ok(Table::Mat(Arc::new(Matrix::new_filled(1, 1, s)))),
            EvalValue::Missing => Err(ErrorKind::Value),
        }
    }

    pub(super) fn rows(&self) -> u32 {
        match self {
            Table::Range(r) => r.rows(),
            Table::Mat(m) => m.rows,
        }
    }

    pub(super) fn cols(&self) -> u32 {
        match self {
            Table::Range(r) => r.cols(),
            Table::Mat(m) => m.cols,
        }
    }

    /// The value at (row, col) offsets.
    fn get(&self, ctx: &mut dyn EvalContext, row: u32, col: u32) -> Value {
        match self {
            Table::Range(r) => {
                let addr = tally_types::CellAddr::new(r.sheet, r.r0 + row, r.c0 + col);
                match ctx.cell_value(addr) {
                    Value::Array(m) => m.get(0, 0).cloned().unwrap_or(Value::Empty),
                    v => v,
                }
            }
            Table::Mat(m) => m.get(row, col).cloned().unwrap_or(Value::Empty),
        }
    }

    /// The non-empty cells of one column (`column == true`, `index` is the
    /// column offset, items keyed by row offset) or one row, in order.
    pub(super) fn line(
        &self,
        ctx: &mut dyn EvalContext,
        column: bool,
        index: u32,
    ) -> Vec<(u32, Value)> {
        let mut out = Vec::new();
        match self {
            Table::Range(r) => {
                let line = if column {
                    CellRange::new(r.sheet, r.r0, r.c0 + index, r.r1, r.c0 + index)
                } else {
                    CellRange::new(r.sheet, r.r0 + index, r.c0, r.r0 + index, r.c1)
                };
                let Some(line) = ctx.used_bounds(&line) else {
                    return out;
                };
                ctx.for_each_in_range(&line, true, &mut |addr, v| {
                    let off = if column {
                        addr.row - r.r0
                    } else {
                        addr.col - r.c0
                    };
                    out.push((off, v.clone()));
                    std::ops::ControlFlow::Continue(())
                });
            }
            Table::Mat(m) => {
                let n = if column { m.rows } else { m.cols };
                for off in 0..n {
                    let v = if column {
                        m.get(off, index)
                    } else {
                        m.get(index, off)
                    };
                    match v {
                        None | Some(Value::Empty) => {}
                        Some(v) => out.push((off, v.clone())),
                    }
                }
            }
        }
        out
    }
}

/// How [`search`] compares.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub(super) enum Mode {
    /// Equality (with wildcards for text).
    Exact,
    /// Sorted ascending: the largest value `<=` the key.
    Ascending,
    /// Sorted descending: the smallest value `>=` the key.
    Descending,
}

/// The lookup value, reduced to what can be compared.
pub(super) enum Key {
    Num(f64),
    Text(Arc<str>),
}

/// Turns the lookup value into a [`Key`]. Errors propagate; an empty value
/// looks up 0 (the generic scalar coercion; Excel gives `#N/A` instead).
pub(super) fn make_key(v: Value) -> Result<Key, ErrorKind> {
    match v {
        Value::Error(e) => Err(e),
        Value::Number(n) => Ok(Key::Num(n)),
        Value::Bool(b) => Ok(Key::Num(if b { 1.0 } else { 0.0 })),
        Value::Text(s) => Ok(Key::Text(s)),
        Value::Empty => Ok(Key::Num(0.0)),
        Value::Array(m) => make_key(m.get(0, 0).cloned().unwrap_or(Value::Empty)),
    }
}

fn cmp_num(a: f64, b: f64) -> Ordering {
    if approx_equal(a, b) {
        Ordering::Equal
    } else if a < b {
        Ordering::Less
    } else {
        Ordering::Greater
    }
}

fn cmp_text(a: &str, b: &str, case_sensitive: bool) -> Ordering {
    if case_sensitive {
        a.cmp(b)
    } else {
        a.to_lowercase().cmp(&b.to_lowercase())
    }
}

/// Compares a cell with the key; `None` when the types differ.
fn compare(cell: &Value, key: &Key, s: &CalcSettings) -> Option<Ordering> {
    match (cell, key) {
        (Value::Number(x), Key::Num(k)) => Some(cmp_num(*x, *k)),
        (Value::Bool(x), Key::Num(k)) => Some(cmp_num(if *x { 1.0 } else { 0.0 }, *k)),
        (Value::Text(x), Key::Text(k)) => Some(cmp_text(x, k, s.case_sensitive)),
        _ => None,
    }
}

/// Matcher for an exact text lookup.
enum TextMatch {
    Literal(Arc<str>),
    Crit(Criterion),
}

impl TextMatch {
    fn new(s: &Arc<str>, settings: &CalcSettings) -> TextMatch {
        let plain = s.is_empty()
            || parse_unambiguous(s, settings.date_epoch).is_some()
            || s.eq_ignore_ascii_case("TRUE")
            || s.eq_ignore_ascii_case("FALSE")
            || ErrorKind::parse(s).is_some();
        if plain {
            TextMatch::Literal(s.clone())
        } else {
            // A leading "=" makes the rest literal text, whatever it starts with.
            TextMatch::Crit(Criterion::parse(&Value::text(format!("={s}")), settings))
        }
    }

    fn matches(&self, cell: &str, settings: &CalcSettings) -> bool {
        match self {
            TextMatch::Literal(l) => cmp_text(cell, l, settings.case_sensitive) == Ordering::Equal,
            TextMatch::Crit(c) => c.matches(&Value::text(cell)),
        }
    }
}

/// Finds the offset of the matching item (see the module docs).
pub(super) fn search(
    items: &[(u32, Value)],
    key: &Key,
    mode: Mode,
    settings: &CalcSettings,
) -> Option<u32> {
    match mode {
        Mode::Exact => {
            let text = match key {
                Key::Text(s) => Some(TextMatch::new(s, settings)),
                Key::Num(_) => None,
            };
            items.iter().find_map(|(off, v)| {
                let hit = match (&text, v) {
                    (Some(m), Value::Text(c)) => m.matches(c, settings),
                    (None, _) => compare(v, key, settings) == Some(Ordering::Equal),
                    _ => false,
                };
                hit.then_some(*off)
            })
        }
        Mode::Ascending | Mode::Descending => {
            let mut best = None;
            for (off, v) in items {
                let Some(ord) = compare(v, key, settings) else {
                    continue;
                };
                let stop = if mode == Mode::Ascending {
                    ord == Ordering::Greater
                } else {
                    ord == Ordering::Less
                };
                if stop {
                    break;
                }
                best = Some(*off);
            }
            best
        }
    }
}

const VLOOKUP_PARAMS: &[Param] = &[
    Param::required("search_criterion", ParamKind::Any),
    Param::required("array", ParamKind::Reference),
    Param::required("index", ParamKind::Integer),
    Param::optional("sort_order", ParamKind::Bool),
];

/// `VLOOKUP(search_criterion; array; index; [sort_order])`.
pub const VLOOKUP: FunctionDef =
    FunctionDef::new("VLOOKUP", Category::Spreadsheet, VLOOKUP_PARAMS, vlookup)
        .returns(ReturnKind::Any)
        .help("Vertical search and reference to indicated cells.");

const HLOOKUP_PARAMS: &[Param] = &[
    Param::required("search_criterion", ParamKind::Any),
    Param::required("array", ParamKind::Reference),
    Param::required("index", ParamKind::Integer),
    Param::optional("sorted", ParamKind::Bool),
];

/// `HLOOKUP(search_criterion; array; index; [sorted])`.
pub const HLOOKUP: FunctionDef =
    FunctionDef::new("HLOOKUP", Category::Spreadsheet, HLOOKUP_PARAMS, hlookup)
        .returns(ReturnKind::Any)
        .help("Horizontal search and reference to the cells located below.");

/// Shared body: `vertical` searches the first column and returns from column `index`.
fn lookup(ctx: &mut dyn EvalContext, args: &mut dyn ArgList, vertical: bool) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let key = make_key(a.value(0))?;
        let table = Table::from_eval(a.any(1))?;
        let index = a.int(2)?;
        let sorted = a.bool_opt(3, true)?;
        let extent = i64::from(if vertical { table.cols() } else { table.rows() });
        if index < 1 || index > extent {
            // AOO: illegal argument (Excel: #REF!, #VALUE! for index < 1).
            return Err(ErrorKind::InvalidArgument);
        }
        let items = table.line(a.ctx(), vertical, 0);
        let mode = if sorted { Mode::Ascending } else { Mode::Exact };
        let settings = a.ctx().settings().clone();
        let pos = search(&items, &key, mode, &settings).ok_or(ErrorKind::NA)?;
        let off = (index - 1) as u32;
        let v = if vertical {
            table.get(a.ctx(), pos, off)
        } else {
            table.get(a.ctx(), off, pos)
        };
        Ok(scalar_value(&EvalValue::from_value(v), a.ctx()))
    })
}

fn vlookup(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    lookup(ctx, args, true)
}

fn hlookup(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    lookup(ctx, args, false)
}

pub(super) fn register(r: &mut Registry) {
    r.register(VLOOKUP);
    r.register(HLOOKUP);
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{ErrorKind, Value};

    fn n(x: f64) -> EvalValue {
        EvalValue::number(x)
    }

    /// A1:B5 = (1,"a") (3,"b") (5,"c") ("x","d") (7, "e"); D1:G2 horizontal.
    fn sheet() -> MockEvalContext {
        let mut c = MockEvalContext::new();
        c.set(0, "A1", 1.0).set(0, "B1", "a");
        c.set(0, "A2", 3.0).set(0, "B2", "b");
        c.set(0, "A3", 5.0).set(0, "B3", "c");
        c.set(0, "A4", "x").set(0, "B4", "d");
        c.set(0, "A5", 7.0).set(0, "B5", "e");
        c.set(0, "D1", 10.0).set(0, "E1", 20.0).set(0, "F1", 30.0);
        c.set(0, "D2", "p").set(0, "E2", "q").set(0, "F2", "r");
        c
    }

    #[test]
    fn vlookup_sorted() {
        let mut c = sheet();
        let r = c.range(0, "A1:B3");
        let f = |c: &mut MockEvalContext, k: f64| call(c, "VLOOKUP", vec![n(k), r.clone(), n(2.0)]);
        assert_eq!(f(&mut c, 3.0), Value::text("b"));
        assert_eq!(f(&mut c, 4.0), Value::text("b"));
        assert_eq!(f(&mut c, 100.0), Value::text("c"));
        assert_eq!(f(&mut c, 0.0), Value::Error(ErrorKind::NA));
    }

    #[test]
    fn vlookup_exact() {
        let mut c = sheet();
        let r = c.range(0, "A1:B5");
        let args = |k: EvalValue| vec![k, r.clone(), n(2.0), EvalValue::bool(false)];
        assert_eq!(call(&mut c, "VLOOKUP", args(n(5.0))), Value::text("c"));
        assert_eq!(
            call(&mut c, "VLOOKUP", args(n(4.0))),
            Value::Error(ErrorKind::NA)
        );
        assert_eq!(
            call(&mut c, "VLOOKUP", args(EvalValue::text("X"))),
            Value::text("d")
        );
        assert_eq!(
            call(&mut c, "VLOOKUP", args(EvalValue::text("5"))),
            Value::Error(ErrorKind::NA)
        );
        assert_eq!(
            call(&mut c, "VLOOKUP", args(EvalValue::text("?"))),
            Value::text("d")
        );
    }

    #[test]
    fn vlookup_errors() {
        let mut c = sheet();
        let r = c.range(0, "A1:B3");
        let bad =
            |c: &mut MockEvalContext, i: f64| call(c, "VLOOKUP", vec![n(1.0), r.clone(), n(i)]);
        assert_eq!(bad(&mut c, 3.0), Value::Error(ErrorKind::InvalidArgument));
        assert_eq!(bad(&mut c, 0.0), Value::Error(ErrorKind::InvalidArgument));
        assert_eq!(
            call(
                &mut c,
                "VLOOKUP",
                vec![EvalValue::error(ErrorKind::Div0), r.clone(), n(1.0)]
            ),
            Value::Error(ErrorKind::Div0)
        );
        assert_eq!(
            call(&mut c, "VLOOKUP", vec![n(1.0), n(5.0), n(1.0)]),
            Value::Error(ErrorKind::NA)
        );
    }

    #[test]
    fn hlookup() {
        let mut c = sheet();
        let r = c.range(0, "D1:F2");
        assert_eq!(
            call(&mut c, "HLOOKUP", vec![n(25.0), r.clone(), n(2.0)]),
            Value::text("q")
        );
        assert_eq!(
            call(
                &mut c,
                "HLOOKUP",
                vec![n(30.0), r.clone(), n(1.0), EvalValue::bool(false)]
            ),
            Value::Number(30.0)
        );
        assert_eq!(
            call(&mut c, "HLOOKUP", vec![n(5.0), r, n(2.0)]),
            Value::Error(ErrorKind::NA)
        );
    }
}
