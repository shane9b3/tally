//! DATE.

use tally_numfmt::date::serial_from_ymd_normalized;
use tally_types::{ErrorKind, Value};

use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry, ReturnKind};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const PARAMS: &[Param] = &[
    Param::required("year", ParamKind::Integer),
    Param::required("month", ParamKind::Integer),
    Param::required("day", ParamKind::Integer),
];

/// `DATE(year; month; day)`: month and day overflow normalizes (month 13 is
/// January of the next year, day 0 the last day of the previous month);
/// years 0..=99 are expanded with the locale's two-digit-year pivot.
pub const DATE: FunctionDef = FunctionDef::new("DATE", Category::DateTime, PARAMS, date)
    .returns(ReturnKind::Number)
    .help("Date serial for a year, month and day.");

fn date(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let mut y = a.int(0)?;
        let m = a.int(1)?;
        let d = a.int(2)?;
        if (0..100).contains(&y) {
            let start = a.ctx().settings().year2000();
            y = i64::from(tally_types::expand_two_digit_year(y as i32, start));
        }
        let epoch = a.ctx().settings().date_epoch;
        serial_from_ymd_normalized(epoch, y, m, d)
            .map(Value::Number)
            .ok_or(ErrorKind::InvalidArgument)
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(DATE);
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{DateEpoch, ErrorKind, Value};

    fn d(y: f64, m: f64, dd: f64) -> Value {
        call(
            &mut MockEvalContext::new(),
            "DATE",
            vec![
                EvalValue::number(y),
                EvalValue::number(m),
                EvalValue::number(dd),
            ],
        )
    }

    #[test]
    fn date() {
        assert_eq!(d(2026.0, 10.0, 19.0), Value::Number(46314.0));
        assert_eq!(d(1899.0, 12.0, 30.0), Value::Number(0.0));
        assert_eq!(d(2026.0, 13.0, 1.0), d(2027.0, 1.0, 1.0));
        assert_eq!(d(2026.0, 0.0, 1.0), d(2025.0, 12.0, 1.0));
        assert_eq!(d(2026.0, 3.0, 0.0), d(2026.0, 2.0, 28.0));
        assert_eq!(d(2026.0, 1.0, 32.0), d(2026.0, 2.0, 1.0));
        assert_eq!(d(2026.9, 10.9, 19.9), Value::Number(46314.0));
    }

    #[test]
    fn two_digit_years() {
        assert_eq!(d(26.0, 1.0, 1.0), d(2026.0, 1.0, 1.0));
        assert_eq!(d(99.0, 1.0, 1.0), d(1999.0, 1.0, 1.0));
        assert_eq!(d(0.0, 1.0, 1.0), d(2000.0, 1.0, 1.0));
    }

    #[test]
    fn invalid_and_epochs() {
        let bad = Value::Error(ErrorKind::InvalidArgument);
        assert_eq!(d(-1.0, 1.0, 1.0), bad);
        assert_eq!(d(10000.0, 1.0, 1.0), bad);
        assert_eq!(d(100.0, 1.0, 1.0), d(100.0, 1.0, 1.0));
        let mut c = MockEvalContext::new();
        c.settings.date_epoch = DateEpoch::D1904;
        let v = call(
            &mut c,
            "DATE",
            vec![
                EvalValue::number(1904.0),
                EvalValue::number(1.0),
                EvalValue::number(2.0),
            ],
        );
        assert_eq!(v, Value::Number(1.0));
        assert_eq!(
            call(
                &mut c,
                "DATE",
                vec![
                    EvalValue::error(ErrorKind::NA),
                    EvalValue::number(1.0),
                    EvalValue::number(1.0)
                ]
            ),
            Value::Error(ErrorKind::NA)
        );
    }
}
