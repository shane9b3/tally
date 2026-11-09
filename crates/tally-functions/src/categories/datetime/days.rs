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

/// `EASTERSUNDAY(year)`: Gregorian Easter, years 1583..=9956. Years 0..=99
/// are expanded with the two-digit-year pivot first, as in DATE (AOO
/// `interpr2.cxx` `ScEasterSunday`: `if ( nYear < 100 ) nYear =
/// pFormatter->ExpandTwoDigitYear( nYear )`).
pub const EASTERSUNDAY: FunctionDef =
    FunctionDef::new("EASTERSUNDAY", Category::DateTime, EASTER_PARAMS, easter)
        .returns(ReturnKind::Number)
        .help("Date of Easter Sunday for a year.");

fn easter(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let mut y = a.int(0)?;
        if (0..100).contains(&y) {
            let start = a.ctx().settings().year2000();
            y = i64::from(tally_types::expand_two_digit_year(y as i32, start));
        }
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

/// `WEEKNUM(date; mode)`: AOO's built-in calendar week (stored in ODF as
/// `ISOWEEKNUM`). Port of `ScInterpreter::ScGetWeekOfYear`
/// (sc/source/core/tool/interpr2.cxx): mode 1 starts weeks on Sunday, every
/// other mode on Monday, and `Date::GetWeekOfYear` applies the default
/// `nMinimumNumberOfDaysInWeek = 4`: week 1 is the first week with at least
/// four days in the new year, so 1 January can be in week 52 or 53
/// (`WEEKNUM(DATE(1995;1;1);2)` = 52, the AOO help example).
///
/// No OOXML name: Excel's WEEKNUM is "week 1 contains 1 January", which is
/// WEEKNUM_ADD (it owns the plain OOXML name), and Excel's ISOWEEKNUM takes
/// one argument. An XLSX export writes the canonical name `WEEKNUM`, which
/// Excel reads with its own (different) semantics.
pub const WEEKNUM: FunctionDef =
    FunctionDef::new("WEEKNUM", Category::DateTime, WEEKNUM_PARAMS, weeknum)
        .odf("ISOWEEKNUM")
        .returns(ReturnKind::Number)
        .help("Calendar week of a date (week 1 has at least four days of the new year).");

/// AOO `DayOfWeek`: Monday = 0 … Sunday = 6.
fn dow_monday0(unix_days: i64) -> i64 {
    // 1970-01-01 was a Thursday (3).
    (unix_days + 3).rem_euclid(7)
}

fn is_leap(y: i64) -> bool {
    tally_numfmt::date::is_leap_year(y)
}

/// Port of tools `Date::GetWeekOfYear(eStartDay, 4)` (tools/source/datetime/
/// tdate.cxx, the `nMinimumNumberOfDaysInWeek` 2..6 branch with 4).
/// `start` is the AOO `DayOfWeek` the week starts on (MONDAY = 0, SUNDAY = 6).
fn week_of_year(unix_days: i64, start: i64) -> i64 {
    const MIN_DAYS: i64 = 4;
    let (y, _, _) = civil_from_days(unix_days);
    let jan1 = days_from_civil(y, 1, 1);
    // Weekday of 1 January relative to the start day, and the 0-based day
    // of the year.
    let n1wday = (dow_monday0(jan1) + (7 - start)) % 7;
    let day_of_year = unix_days - jan1;
    let mut week = if n1wday < MIN_DAYS {
        1
    } else if n1wday == MIN_DAYS {
        53
    } else if n1wday == MIN_DAYS + 1 {
        // The year after a leap year.
        if is_leap(y - 1) {
            53
        } else {
            52
        }
    } else {
        52
    };
    if week == 1 || day_of_year + n1wday > 6 {
        if week == 1 {
            week += (day_of_year + n1wday) / 7;
        } else {
            week = (day_of_year + n1wday) / 7;
        }
        if week == 53 {
            // The last day of this week: if it is in the next year, the
            // week is week 1 there.
            let end = unix_days + 6 - (dow_monday0(unix_days) + (7 - start)) % 7;
            if end != unix_days {
                return week_of_year(end, start);
            }
        }
    }
    week
}

/// AOO's mode rule: `nFlag == 1 ? SUNDAY : MONDAY`.
fn aoo_week(ctx: &dyn EvalContext, serial: f64, mode: i64) -> Result<f64, ErrorKind> {
    let epoch = ctx.settings().date_epoch;
    let days = unix_days_from_serial(epoch, serial.floor()).ok_or(ErrorKind::InvalidArgument)?;
    let start = if mode == 1 { 6 } else { 0 };
    Ok(week_of_year(days, start) as f64)
}

fn weeknum(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let serial = a.num(0)?;
        let mode = a.int(1)?;
        aoo_week(a.ctx(), serial, mode).map(Value::Number)
    })
}

const ISOWEEKNUM_PARAMS: &[Param] = &[
    Param::required("number", ParamKind::Number),
    Param::optional("mode", ParamKind::Integer),
];

/// `ISOWEEKNUM(date [; mode])` (extension; Excel 2013 `_xlfn.ISOWEEKNUM`):
/// the ISO 8601 week, i.e. AOO's WEEKNUM with Monday start. The optional
/// second argument exists only so that an ODF file's `ISOWEEKNUM(date; mode)`
/// (AOO's stored name for the built-in WEEKNUM) still evaluates as AOO's
/// WEEKNUM when it is resolved by its canonical name; LibreOffice reads a
/// 2-argument ODFF ISOWEEKNUM as WEEKNUM in the same way (tdf#50950). Stored
/// in ODF as `COM.MICROSOFT.ISOWEEKNUM` because plain `ISOWEEKNUM` is the
/// built-in WEEKNUM's ODF name.
pub const ISOWEEKNUM: FunctionDef = FunctionDef::new(
    "ISOWEEKNUM",
    Category::DateTime,
    ISOWEEKNUM_PARAMS,
    isoweeknum,
)
.odf("COM.MICROSOFT.ISOWEEKNUM")
.extension()
.returns(ReturnKind::Number)
.help("ISO 8601 week number of a date (weeks start on Monday).");

fn isoweeknum(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let serial = a.num(0)?;
        let mode = if a.is_missing(1) { 2 } else { a.int(1)? };
        aoo_week(a.ctx(), serial, mode).map(Value::Number)
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(DAYS);
    r.register(DAYS360);
    r.register(EASTERSUNDAY);
    r.register(WEEKNUM);
    r.register(ISOWEEKNUM);
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
        // Two-digit years use the DATE pivot (review 2 Low).
        assert_eq!(e(24.0), Value::Number(ser(2024, 3, 31)));
        assert_eq!(e(99.0), Value::Number(ser(1999, 4, 4)));
        assert_eq!(e(0.0), Value::Number(ser(2000, 4, 23)));
        assert_eq!(e(9957.0), Value::Error(ErrorKind::InvalidArgument));
    }

    #[test]
    fn weeknum() {
        let w = |s: f64, m: f64| f("WEEKNUM", vec![n(s), n(m)]);
        // AOO help example and the interpr2.cxx / tdate.cxx 4-day rule.
        assert_eq!(w(ser(1995, 1, 1), 2.0), Value::Number(52.0));
        assert_eq!(w(ser(1995, 1, 1), 1.0), Value::Number(1.0));
        assert_eq!(w(ser(2021, 1, 1), 2.0), Value::Number(53.0));
        assert_eq!(w(ser(2021, 1, 4), 2.0), Value::Number(1.0));
        // 2026-01-01 is a Thursday: the Sunday-start week has three days.
        assert_eq!(w(ser(2026, 1, 1), 1.0), Value::Number(53.0));
        assert_eq!(w(ser(2026, 1, 1), 2.0), Value::Number(1.0));
        assert_eq!(w(ser(2026, 1, 4), 1.0), Value::Number(1.0));
        assert_eq!(w(ser(2026, 10, 19), 2.0), Value::Number(43.0));
        assert_eq!(w(ser(2026, 10, 19), 1.0), Value::Number(42.0));
        // Week 53 that is really week 1 of the next year.
        assert_eq!(w(ser(2024, 12, 30), 2.0), Value::Number(1.0));
        assert_eq!(w(ser(2020, 12, 31), 2.0), Value::Number(53.0));
        // Every mode other than 1 is Monday.
        assert_eq!(w(ser(1995, 1, 1), 3.0), Value::Number(52.0));
        assert_eq!(w(ser(1995, 1, 1), 0.0), Value::Number(52.0));
    }

    /// The port agrees with the plain ISO 8601 rule (Monday start) and its
    /// Sunday-start analogue on every day of 1900..2100.
    #[test]
    fn weeknum_matches_four_day_rule() {
        use tally_numfmt::date::{civil_from_days, days_from_civil};
        for start in [0i64, 6] {
            for d in days_from_civil(1900, 1, 1)..days_from_civil(2100, 12, 31) {
                // The week's "fourth day" decides the year it belongs to.
                let dow = (d + 3).rem_euclid(7);
                let week_start = d - (dow + 7 - start) % 7;
                let thursday = week_start + 3;
                let (y, _, _) = civil_from_days(thursday);
                let want = (thursday - days_from_civil(y, 1, 1)) / 7 + 1;
                assert_eq!(super::week_of_year(d, start), want, "day {d} start {start}");
            }
        }
    }

    #[test]
    fn isoweeknum() {
        let w = |v: Vec<EvalValue>| f("ISOWEEKNUM", v);
        assert_eq!(w(vec![n(ser(1995, 1, 1))]), Value::Number(52.0));
        assert_eq!(w(vec![n(ser(2021, 1, 1))]), Value::Number(53.0));
        assert_eq!(w(vec![n(ser(2026, 1, 1))]), Value::Number(1.0));
        // ODF `ISOWEEKNUM(date; 1)` is AOO's WEEKNUM mode 1.
        assert_eq!(w(vec![n(ser(2026, 1, 1)), n(1.0)]), Value::Number(53.0));
    }
}
