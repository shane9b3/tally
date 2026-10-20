//! HOUR, MINUTE, SECOND, TIME and TIMEVALUE.

use tally_numfmt::{recognize_input_with, RecognizeContext};
use tally_types::{ErrorKind, Value};

use crate::coerce::locale_of;

use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry, ReturnKind};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const ONE: &[Param] = &[Param::required("number", ParamKind::Number)];

/// `HOUR(number)`.
pub const HOUR: FunctionDef = FunctionDef::new("HOUR", Category::DateTime, ONE, hour)
    .returns(ReturnKind::Number)
    .help("Hour (0-23) of a time value.");
/// `MINUTE(number)`.
pub const MINUTE: FunctionDef = FunctionDef::new("MINUTE", Category::DateTime, ONE, minute)
    .returns(ReturnKind::Number)
    .help("Minute (0-59) of a time value.");
/// `SECOND(number)`.
pub const SECOND: FunctionDef = FunctionDef::new("SECOND", Category::DateTime, ONE, second)
    .returns(ReturnKind::Number)
    .help("Second (0-59) of a time value.");

/// Seconds since midnight of the fractional day, rounded to the nearest
/// second (AOO `ScHour`/`ScMinute`/`ScSecond`: `floor(frac * 86400 + 0.5)`),
/// wrapped to 0..86399 so 23:59:59.7 reads as 0:00:00.
fn day_seconds(
    ctx: &mut dyn EvalContext,
    args: &mut dyn ArgList,
    pick: fn(i64) -> i64,
) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let x = a.num(0)?;
        let frac = x - x.floor();
        let secs = (frac * 86400.0 + 0.5).floor() as i64 % 86400;
        Ok(Value::Number(pick(secs) as f64))
    })
}

fn hour(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    day_seconds(ctx, args, |s| s / 3600)
}
fn minute(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    day_seconds(ctx, args, |s| (s / 60) % 60)
}
fn second(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    day_seconds(ctx, args, |s| s % 60)
}

const TIME_PARAMS: &[Param] = &[
    Param::required("hour", ParamKind::Number),
    Param::required("minute", ParamKind::Number),
    Param::required("second", ParamKind::Number),
];

/// `TIME(hour; minute; second)`: each argument is truncated; the result is
/// the time of day (`fmod(total seconds; 86400) / 86400`), so `TIME(25;0;0)`
/// is 1:00. Any argument above 32767 or a negative total is `Err:502`
/// (AOO `ScEncodeTime`).
pub const TIME: FunctionDef = FunctionDef::new("TIME", Category::DateTime, TIME_PARAMS, time)
    .returns(ReturnKind::Number)
    .help("Time value from hour, minute and second.");

fn time(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let h = a.num(0)?.trunc();
        let m = a.num(1)?.trunc();
        let s = a.num(2)?.trunc();
        if h > 32767.0 || m > 32767.0 || s > 32767.0 {
            return Err(ErrorKind::InvalidArgument);
        }
        let total = h * 3600.0 + m * 60.0 + s;
        if total < 0.0 {
            return Err(ErrorKind::InvalidArgument);
        }
        Ok(Value::Number((total % 86400.0) / 86400.0))
    })
}

const TV_PARAMS: &[Param] = &[Param::required("text", ParamKind::Text)];

/// `TIMEVALUE(text)`: the time-of-day fraction of text recognized (in the
/// document locale) as a time or date-time (a bare date gives 0). Text that
/// is not a date or time is `Err:502`, as in AOO.
pub const TIMEVALUE: FunctionDef =
    FunctionDef::new("TIMEVALUE", Category::DateTime, TV_PARAMS, timevalue)
        .returns(ReturnKind::Number)
        .help("Time value of a time given as text.");

fn timevalue(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let s = a.text(0)?;
        let epoch = a.ctx().settings().date_epoch;
        let locale = locale_of(a.ctx());
        let rec = recognize_input_with(&s, locale, &RecognizeContext::now(epoch));
        let n = match rec.value {
            Value::Number(n) => n,
            _ => return Err(ErrorKind::InvalidArgument),
        };
        let is_time_or_date = rec
            .implied_format
            .is_some_and(|f| f.contains(['H', 'D', 'Y']));
        if !is_time_or_date {
            return Err(ErrorKind::InvalidArgument);
        }
        Ok(Value::Number(n - n.floor()))
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(HOUR);
    r.register(MINUTE);
    r.register(SECOND);
    r.register(TIME);
    r.register(TIMEVALUE);
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{approx_equal, ErrorKind, Value};

    fn n(x: f64) -> EvalValue {
        EvalValue::number(x)
    }
    fn f(name: &str, args: Vec<EvalValue>) -> Value {
        call(&mut MockEvalContext::new(), name, args)
    }
    fn num(v: Value) -> f64 {
        match v {
            Value::Number(x) => x,
            other => panic!("not a number: {other:?}"),
        }
    }

    #[test]
    fn parts() {
        let t = 46314.0 + (14.0 * 3600.0 + 30.0 * 60.0 + 15.0) / 86400.0;
        assert_eq!(f("HOUR", vec![n(t)]), Value::Number(14.0));
        assert_eq!(f("MINUTE", vec![n(t)]), Value::Number(30.0));
        assert_eq!(f("SECOND", vec![n(t)]), Value::Number(15.0));
        assert_eq!(f("HOUR", vec![n(0.5)]), Value::Number(12.0));
        assert_eq!(f("HOUR", vec![n(-0.25)]), Value::Number(18.0));
    }

    #[test]
    fn rounding_to_second() {
        // 14:29:59.9999 reads as 14:30:00.
        let t = (14.0 * 3600.0 + 29.0 * 60.0 + 59.9999) / 86400.0;
        assert_eq!(f("MINUTE", vec![n(t)]), Value::Number(30.0));
        assert_eq!(f("SECOND", vec![n(t)]), Value::Number(0.0));
        let t = 86399.9999 / 86400.0;
        assert_eq!(f("HOUR", vec![n(t)]), Value::Number(0.0));
    }

    #[test]
    fn time() {
        assert!(approx_equal(
            num(f("TIME", vec![n(12.0), n(0.0), n(0.0)])),
            0.5
        ));
        assert!(approx_equal(
            num(f("TIME", vec![n(25.0), n(0.0), n(0.0)])),
            1.0 / 24.0
        ));
        assert!(approx_equal(
            num(f("TIME", vec![n(0.0), n(90.0), n(0.0)])),
            1.5 / 24.0
        ));
        assert_eq!(
            f("TIME", vec![n(0.0), n(0.0), n(-1.0)]),
            Value::Error(ErrorKind::InvalidArgument)
        );
        assert_eq!(
            f("TIME", vec![n(40000.0), n(0.0), n(0.0)]),
            Value::Error(ErrorKind::InvalidArgument)
        );
    }

    #[test]
    fn timevalue() {
        let tv = |s: &str| f("TIMEVALUE", vec![EvalValue::text(s)]);
        assert!(approx_equal(num(tv("12:00")), 0.5));
        assert!(approx_equal(num(tv("6:00:00 PM")), 0.75));
        assert!(approx_equal(num(tv("2026-10-19 18:00")), 0.75));
        assert_eq!(tv("2026-10-19"), Value::Number(0.0));
        assert_eq!(tv("abc"), Value::Error(ErrorKind::InvalidArgument));
        assert_eq!(tv("42"), Value::Error(ErrorKind::InvalidArgument));
    }

    #[test]
    fn timevalue_document_locale() {
        let mut c = MockEvalContext::new();
        c.set_locale("de-DE");
        let mut tv = |s: &str| call(&mut c, "TIMEVALUE", vec![EvalValue::text(s)]);
        assert!(approx_equal(num(tv("19.10.2026 18:00")), 0.75));
        assert!(approx_equal(num(tv("12:30:00")), 12.5 / 24.0));
        assert_eq!(
            tv("10/19/2026 18:00"),
            Value::Error(ErrorKind::InvalidArgument)
        );
    }
}
