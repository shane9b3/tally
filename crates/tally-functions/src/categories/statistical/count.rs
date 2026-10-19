//! COUNT and COUNTA (COUNTBLANK lives with the COUNTIF family).

use tally_types::Value;

use crate::coerce::{parse_unambiguous, SeqSource};
use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const COUNT_PARAMS: &[Param] = &[Param::repeat("value", ParamKind::ValueSeq)];

/// `COUNT(value; ...)`.
pub const COUNT: FunctionDef =
    FunctionDef::new("COUNT", Category::Statistical, COUNT_PARAMS, count)
        .help("Counts how many numbers are in the list of arguments.");

/// Counts numbers and booleans (AOO stores booleans as numbers) anywhere;
/// direct text counts when it is an unambiguous number or ISO date; a
/// missing direct argument counts (as 0); errors are never counted and
/// never propagate.
fn count(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    let epoch = a.ctx().settings().date_epoch;
    run(|| {
        let mut n = 0u64;
        for i in 0..a.count() {
            a.for_each_value(i, |v, src| match (v, src) {
                (Value::Number(_) | Value::Bool(_), _) => n += 1,
                (Value::Empty, SeqSource::Direct) => n += 1,
                (Value::Text(s), SeqSource::Direct) if parse_unambiguous(s, epoch).is_some() => {
                    n += 1
                }
                _ => {}
            });
        }
        Ok(Value::Number(n as f64))
    })
}

const COUNTA_PARAMS: &[Param] = &[Param::repeat("value", ParamKind::ValueSeq)];

/// `COUNTA(value; ...)`.
pub const COUNTA: FunctionDef =
    FunctionDef::new("COUNTA", Category::Statistical, COUNTA_PARAMS, counta)
        .help("Counts how many values are in the list of arguments.");

/// Counts every non-empty value: numbers, text (including the empty string
/// a formula returns), booleans and errors. Empty cells are not counted; a
/// missing direct argument (`COUNTA(1;;2)`) is counted, as in Excel.
fn counta(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let mut n = 0u64;
        for i in 0..a.count() {
            a.for_each_value(i, |v, src| match (v, src) {
                (Value::Empty, SeqSource::Direct) => n += 1,
                (Value::Empty, _) => {}
                _ => n += 1,
            });
        }
        Ok(Value::Number(n as f64))
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(COUNT);
    r.register(COUNTA);
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{ErrorKind, Value};

    fn sheet() -> MockEvalContext {
        let mut c = MockEvalContext::new();
        c.set(0, "A1", 1.0)
            .set(0, "A2", "x")
            .set(0, "A3", true)
            .set(0, "A4", Value::Error(ErrorKind::Div0))
            .set(0, "A5", "");
        c
    }

    #[test]
    fn counta_ranges() {
        let mut c = sheet();
        let r = c.range(0, "A1:A6");
        assert_eq!(call(&mut c, "COUNTA", vec![r]), Value::Number(5.0));
        let r = c.range(0, "B1:B3");
        assert_eq!(call(&mut c, "COUNTA", vec![r]), Value::Number(0.0));
    }

    #[test]
    fn counta_direct() {
        let mut c = sheet();
        let v = call(
            &mut c,
            "COUNTA",
            vec![
                EvalValue::number(1.0),
                EvalValue::Missing,
                EvalValue::text(""),
                EvalValue::error(ErrorKind::NA),
            ],
        );
        assert_eq!(v, Value::Number(4.0));
        assert_eq!(
            call(&mut c, "COUNTA", vec![]),
            Value::Error(ErrorKind::MissingVariable2)
        );
    }
}
