//! MIN and MAX.

use tally_types::Value;

use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const PARAMS: &[Param] = &[Param::repeat("number", ParamKind::NumberSeq)];

/// `MIN(number; ...)`.
pub const MIN: FunctionDef = FunctionDef::new("MIN", Category::Statistical, PARAMS, min)
    .help("Returns the minimum value in a list of arguments.");

/// `MAX(number; ...)`.
pub const MAX: FunctionDef = FunctionDef::new("MAX", Category::Statistical, PARAMS, max)
    .help("Returns the maximum value in a list of arguments.");

fn extreme(ctx: &mut dyn EvalContext, args: &mut dyn ArgList, want_max: bool) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let mut best: Option<f64> = None;
        a.for_each_number_all(|x| {
            best = Some(match best {
                None => x,
                Some(b) if want_max => b.max(x),
                Some(b) => b.min(x),
            });
        })?;
        // AOO: no numbers at all gives 0 (ScMin/ScMax push 0).
        Ok(Value::Number(best.unwrap_or(0.0)))
    })
}

fn min(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    extreme(ctx, args, false)
}

fn max(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    extreme(ctx, args, true)
}

pub(super) fn register(r: &mut Registry) {
    r.register(MIN);
    r.register(MAX);
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{ErrorKind, Value};

    fn n(x: f64) -> EvalValue {
        EvalValue::number(x)
    }

    fn sheet() -> MockEvalContext {
        let mut c = MockEvalContext::new();
        c.set(0, "A1", 3.0)
            .set(0, "A2", "x")
            .set(0, "A3", -2.0)
            .set(0, "B1", Value::Error(ErrorKind::NA))
            .set(0, "C1", true);
        c
    }

    #[test]
    fn min_max_basic() {
        let mut c = sheet();
        let r = c.range(0, "A1:A3");
        assert_eq!(call(&mut c, "MIN", vec![r.clone()]), Value::Number(-2.0));
        assert_eq!(call(&mut c, "MAX", vec![r]), Value::Number(3.0));
        assert_eq!(
            call(&mut c, "MAX", vec![n(1.0), n(5.0), n(2.0)]),
            Value::Number(5.0)
        );
    }

    #[test]
    fn no_numbers_is_zero() {
        let mut c = sheet();
        let r = c.range(0, "A2");
        assert_eq!(call(&mut c, "MIN", vec![r.clone()]), Value::Number(0.0));
        assert_eq!(call(&mut c, "MAX", vec![r]), Value::Number(0.0));
    }

    #[test]
    fn errors_bools_text() {
        let mut c = sheet();
        let r = c.range(0, "A1:B1");
        assert_eq!(call(&mut c, "MAX", vec![r]), Value::Error(ErrorKind::NA));
        let r = c.range(0, "C1");
        assert_eq!(call(&mut c, "MIN", vec![r, n(5.0)]), Value::Number(1.0));
        assert_eq!(
            call(&mut c, "MIN", vec![EvalValue::text("x")]),
            Value::Error(ErrorKind::Value)
        );
        assert_eq!(
            call(&mut c, "MAX", vec![EvalValue::text("7"), n(2.0)]),
            Value::Number(7.0)
        );
    }
}
