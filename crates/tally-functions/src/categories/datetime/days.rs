//! DAYS, DAYS360, EASTERSUNDAY and the built-in WEEKNUM.

use tally_numfmt::date::{
    civil_from_days, days_from_civil, serial_from_ymd, unix_days_from_serial,
};
use tally_types::{ErrorKind, Value};

use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry, ReturnKind};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const DAYS_PARAMS: &[Param] = &[
    Param::required("end_date", ParamKind::Number),
    Param::required("start_date", ParamKind::Number),
];

/// `DAYS(end; start)`: whole days between the integer parts of two dates.
pub const DAYS: FunctionDef = FunctionDef::new("DAYS", Category::DateTime, DAYS_PARAMS, days)
    .returns(ReturnKind::Number)
    .help("Number of days between two dates (end minus start).");

fn days(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let end = a.num(0)?;
        let start = a.num(1)?;
        Ok(Value::Number(end.floor() - start.floor()))
    })
}

const DAYS360_PARAMS: &[Param] = &[
    Param::required("date_1", ParamKind::Number),
    Param::required("date_2", ParamKind::Number),
    Param::optional("type", ParamKind::Bool),
];

/// `DAYS360(date1; date2 [; european])`.
pub const DAYS360: FunctionDef =
    FunctionDef::new("DAYS360", Category::DateTime, DAYS360_PARAMS, days360)
        .returns(ReturnKind::Number)
        .help("Days between two dates on a 360-day year.");

/// Port of AOO `ScInterpreter::GetDiffDate360` / `ScGetDiffDate360`
/// (sc/source/core/tool/interpr2.cxx). `us` is the NASD method (the default,
/// third argument FALSE); TRUE selects the European method. If date2 is
/// before date1 the dates are swapped and the sign flipped.
fn days360(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let european = a.bool_opt(2, false)?;
        let mut d1 = a.num(0)?.floor();
        let mut d2 = a.num(1)?.floor();
        let mut sign = 1.0;
        if d2 < d1 {
            std::mem::swap(&mut d1, &mut d2);
            sign = -1.0;
        }
        let epoch = a.ctx().settings().date_epoch;
        let ymd = |s: f64| {
            unix_days_from_serial(epoch, s)
                .map(civil_from_days)
                .ok_or(ErrorKind::InvalidArgument)
        };
        let (y1, m1, mut day1) = ymd(d1)?;
        let (mut y2, mut m2, mut day2) = ymd(d2)?;
        let us = !european;
        let leap1 = tally_numfmt::date::is_leap_year(y1);
        if day1 == 31 || (us && m1 == 2 && (day1 == 29 || (day1 == 28 && !leap1))) {
            day1 = 30;
        }
        if day2 == 31 {
            if us && day1 != 30 {
                day2 = 1;
                if m2 == 12 {
                    y2 += 1;
                    m2 = 1;
                } else {
                    m2 += 1;
                }
            } else {
                day2 = 30;
            }
        }
        let r = (y2 * 360 + i64::from(m2) * 30 + i64::from(day2))
            - (y1 * 360 + i64::from(m1) * 30 + i64::from(day1));
        Ok(Value::Number(sign * r as f64))
    })
}

const EASTER_PARAMS: &[Param] = &[Param::required("year", ParamKind::Integer)];

/// `EASTERSUNDAY(year)`: Gregorian Easter, years 1583..=9956.
pub const EASTERSUNDAY: FunctionDef =
    FunctionDef::new("EASTERSUNDAY", Category::DateTime, EASTER_PARAMS, easter)
        .returns(ReturnKind::Number)
        .help("Date of Easter Sunday for a year.");

fn easter(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let y = a.int(0)?;
        if !(1583..=9956).contains(&y) {
            return Err(ErrorKind::InvalidArgument);
        }
        // Anonymous Gregorian (Gauss/Butcher) algorithm.
        let (aa, b, c) = (y % 19, y / 100, y % 100);
        let (d, e) = (b / 4, b % 4);
        let f = (b + 8) / 25;
        let g = (b - f + 1) / 3;
        let h = (19 * aa + b - d - g + 15) % 30;
        let (i, k) = (c / 4, c % 4);
        let l = (32 + 2 * e + 2 * i - h - k) % 7;
        let m = (aa + 11 * h + 22 * l) / 451;
        let month = (h + l - 7 * m + 114) / 31;
        let day = (h + l - 7 * m + 114) % 31 + 1;
        let epoch = a.ctx().settings().date_epoch;
        serial_from_ymd(epoch, y as i32, month as u32, day as u32)
            .map(Value::Number)
            .ok_or(ErrorKind::InvalidArgument)
    })
}

const WEEKNUM_PARAMS: &[Param] = &[
    Param::required("number", ParamKind::Number),
    Param::required("mode", ParamKind::Integer),
];

/// `WEEKNUM(date; mode)`: mode 1 weeks start on Sunday, mode 2 on Monday;
/// week 1 is the week containing 1 January. Other modes are `Err:502`
/// (ISO weeks are ISOWEEKNUM; WEEKNUM_ADD is the add-in variant).
pub const WEEKNUM: FunctionDef =
    FunctionDef::new("WEEKNUM", Category::DateTime, WEEKNUM_PARAMS, weeknum)
        .odf("ISOWEEKNUM")
        .returns(ReturnKind::Number)
        .help("Calendar week of a date (week 1 contains 1 January).");

fn weeknum(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let serial = a.num(0)?;
        let mode = a.int(1)?;
        if mode != 1 && mode != 2 {
            return Err(ErrorKind::InvalidArgument);
        }
        let epoch = a.ctx().settings().date_epoch;
        let days = unix_days_from_serial(epoch, serial).ok_or(ErrorKind::InvalidArgument)?;
        let (y, _, _) = civil_from_days(days);
        let jan1 = days_from_civil(y, 1, 1);
        // 1970-01-01 was a Thursday; Sunday = 0.
        let dow_jan1 = (jan1 + 4).rem_euclid(7);
        let start = if mode == 1 { 0 } else { 1 };
        let offset = (dow_jan1 - start).rem_euclid(7);
        Ok(Value::Number(((days - jan1 + offset) / 7 + 1) as f64))
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(DAYS);
    r.register(DAYS360);
    r.register(EASTERSUNDAY);
    r.register(WEEKNUM);
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{ErrorKind, Value};

    fn n(x: f64) -> EvalValue {
        EvalValue::number(x)
    }
    fn f(name: &str, args: Vec<EvalValue>) -> Value {
        call(&mut MockEvalContext::new(), name, args)
    }
    fn ser(y: i32, m: u32, d: u32) -> f64 {
        tally_numfmt::date::serial_from_ymd(tally_types::DateEpoch::D1899, y, m, d).unwrap_or(-1.0)
    }

    #[test]
    fn days() {
        assert_eq!(f("DAYS", vec![n(46314.0), n(46300.5)]), Value::Number(14.0));
        assert_eq!(f("DAYS", vec![n(10.0), n(20.0)]), Value::Number(-10.0));
        assert_eq!(
            f("DAYS", vec![EvalValue::text("x"), n(1.0)]),
            Value::Error(ErrorKind::Value)
        );
    }

    #[test]
    fn days360() {
        let d = |a: (i32, u32, u32), b: (i32, u32, u32), eu: Option<bool>| {
            let mut v = vec![n(ser(a.0, a.1, a.2)), n(ser(b.0, b.1, b.2))];
            v.extend(eu.map(EvalValue::bool));
            f("DAYS360", v)
        };
        assert_eq!(d((2011, 1, 1), (2011, 12, 31), None), Value::Number(360.0));
        assert_eq!(d((2011, 1, 31), (2011, 3, 31), None), Value::Number(60.0));
        // US: day1=30 after 31->30, so Mar 31 -> 30 as well.
        assert_eq!(
            d((2011, 1, 30), (2011, 3, 31), Some(false)),
            Value::Number(60.0)
        );
        // US: start Jan 15, end 31st -> day2 becomes 1st of next month.
        assert_eq!(d((2011, 1, 15), (2011, 3, 31), None), Value::Number(76.0));
        assert_eq!(
            d((2011, 1, 15), (2011, 3, 31), Some(true)),
            Value::Number(75.0)
        );
        // February end of month (US method).
        assert_eq!(d((2011, 2, 28), (2011, 3, 31), None), Value::Number(30.0));
        assert_eq!(
            d((2011, 2, 28), (2011, 3, 31), Some(true)),
            Value::Number(32.0)
        );
        // Reversed dates flip the sign.
        assert_eq!(d((2011, 3, 31), (2011, 1, 31), None), Value::Number(-60.0));
    }

    #[test]
    fn easter() {
        let e = |y: f64| f("EASTERSUNDAY", vec![n(y)]);
        assert_eq!(e(2026.0), Value::Number(ser(2026, 4, 5)));
        assert_eq!(e(2024.0), Value::Number(ser(2024, 3, 31)));
        assert_eq!(e(2000.0), Value::Number(ser(2000, 4, 23)));
        assert_eq!(e(1583.0), Value::Number(ser(1583, 4, 10)));
        assert_eq!(e(1582.0), Value::Error(ErrorKind::InvalidArgument));
        assert_eq!(e(9957.0), Value::Error(ErrorKind::InvalidArgument));
    }

    #[test]
    fn weeknum() {
        let w = |s: f64, m: f64| f("WEEKNUM", vec![n(s), n(m)]);
        assert_eq!(w(ser(2026, 10, 19), 1.0), Value::Number(43.0));
        assert_eq!(w(ser(2026, 10, 19), 2.0), Value::Number(43.0));
        // 2026-01-01 is a Thursday.
        assert_eq!(w(ser(2026, 1, 1), 1.0), Value::Number(1.0));
        assert_eq!(w(ser(2026, 1, 4), 1.0), Value::Number(2.0));
        assert_eq!(w(ser(2026, 1, 4), 2.0), Value::Number(1.0));
        assert_eq!(w(ser(2026, 1, 5), 2.0), Value::Number(2.0));
        assert_eq!(w(ser(2026, 12, 31), 1.0), Value::Number(53.0));
        assert_eq!(w(1.0, 3.0), Value::Error(ErrorKind::InvalidArgument));
    }
}
