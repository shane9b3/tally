//! YEAR, MONTH and DAY.

use tally_numfmt::date::ymd_from_serial;
use tally_types::{ErrorKind, Value};

use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry, ReturnKind};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const PARAMS: &[Param] = &[Param::required("number", ParamKind::Number)];

/// `YEAR(number)`.
pub const YEAR: FunctionDef = FunctionDef::new("YEAR", Category::DateTime, PARAMS, year)
    .returns(ReturnKind::Number)
    .help("Year of a date serial.");
/// `MONTH(number)`.
pub const MONTH: FunctionDef = FunctionDef::new("MONTH", Category::DateTime, PARAMS, month)
    .returns(ReturnKind::Number)
    .help("Month (1-12) of a date serial.");
/// `DAY(number)`.
pub const DAY: FunctionDef = FunctionDef::new("DAY", Category::DateTime, PARAMS, day)
    .returns(ReturnKind::Number)
    .help("Day of the month of a date serial.");

fn part(
    ctx: &mut dyn EvalContext,
    args: &mut dyn ArgList,
    pick: fn((i32, u32, u32)) -> f64,
) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let serial = a.num(0)?;
        let epoch = a.ctx().settings().date_epoch;
        ymd_from_serial(epoch, serial)
            .map(|ymd| Value::Number(pick(ymd)))
            .ok_or(ErrorKind::InvalidArgument)
    })
}

fn year(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    part(ctx, args, |(y, _, _)| f64::from(y))
}
fn month(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    part(ctx, args, |(_, m, _)| f64::from(m))
}
fn day(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    part(ctx, args, |(_, _, d)| f64::from(d))
}

pub(super) fn register(r: &mut Registry) {
    r.register(YEAR);
    r.register(MONTH);
    r.register(DAY);
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{DateEpoch, ErrorKind, Value};

    fn f(name: &str, v: EvalValue) -> Value {
        call(&mut MockEvalContext::new(), name, vec![v])
    }

    #[test]
    fn parts() {
        let n = EvalValue::number(46314.75);
        assert_eq!(f("YEAR", n.clone()), Value::Number(2026.0));
        assert_eq!(f("MONTH", n.clone()), Value::Number(10.0));
        assert_eq!(f("DAY", n), Value::Number(19.0));
        assert_eq!(f("YEAR", EvalValue::number(0.0)), Value::Number(1899.0));
        assert_eq!(f("DAY", EvalValue::number(-1.0)), Value::Number(29.0));
        assert_eq!(f("MONTH", EvalValue::Missing), Value::Number(12.0));
    }

    #[test]
    fn text_and_errors() {
        assert_eq!(
            f("YEAR", EvalValue::text("2026-10-19")),
            Value::Number(2026.0)
        );
        assert_eq!(
            f("YEAR", EvalValue::text("x")),
            Value::Error(ErrorKind::Value)
        );
        assert_eq!(
            f("MONTH", EvalValue::error(ErrorKind::NA)),
            Value::Error(ErrorKind::NA)
        );
        assert_eq!(
            f("YEAR", EvalValue::number(1e9)),
            Value::Error(ErrorKind::InvalidArgument)
        );
    }

    #[test]
    fn epoch_1904() {
        let mut c = MockEvalContext::new();
        c.settings.date_epoch = DateEpoch::D1904;
        let v = call(&mut c, "YEAR", vec![EvalValue::number(0.0)]);
        assert_eq!(v, Value::Number(1904.0));
    }
}
