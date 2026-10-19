//! EDATE, EOMONTH, NETWORKDAYS, WORKDAY, WEEKNUM_ADD (Analysis add-in).
//!
//! Semantics ported from Apache OpenOffice `scaddins/source/analysis/analysishelper.cxx`
//! and `analysis.cxx` (Apache-2.0).

use std::collections::HashSet;

use tally_numfmt::date::{civil_from_days, days_from_civil, days_in_month};
use tally_types::{ErrorKind, Value};

use super::{day_of, serial_of};
use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const EDATE_PARAMS: &[Param] = &[
    Param::required("start_date", ParamKind::Number),
    Param::required("months", ParamKind::Integer),
];
const NET_PARAMS: &[Param] = &[
    Param::required("start_date", ParamKind::Number),
    Param::required("end_date", ParamKind::Number),
    Param::optional("holidays", ParamKind::NumberSeq),
];
const WORKDAY_PARAMS: &[Param] = &[
    Param::required("start_date", ParamKind::Number),
    Param::required("days", ParamKind::Integer),
    Param::optional("holidays", ParamKind::NumberSeq),
];
const WEEKNUM_PARAMS: &[Param] = &[
    Param::required("date", ParamKind::Number),
    Param::required("mode", ParamKind::Integer),
];

/// `EDATE(start; months)`.
pub const EDATE: FunctionDef = FunctionDef::new("EDATE", Category::AddIn, EDATE_PARAMS, edate)
    .help("Date a number of months before or after the start date.");
/// `EOMONTH(start; months)`.
pub const EOMONTH: FunctionDef =
    FunctionDef::new("EOMONTH", Category::AddIn, EDATE_PARAMS, eomonth)
        .help("Last day of the month a number of months from the start date.");
/// `NETWORKDAYS(start; end [; holidays])`.
pub const NETWORKDAYS: FunctionDef =
    FunctionDef::new("NETWORKDAYS", Category::AddIn, NET_PARAMS, networkdays)
        .help("Number of working days between two dates (Saturday and Sunday are free).");
/// `WORKDAY(start; days [; holidays])`.
pub const WORKDAY: FunctionDef =
    FunctionDef::new("WORKDAY", Category::AddIn, WORKDAY_PARAMS, workday)
        .help("Date a number of working days before or after the start date.");
/// `WEEKNUM(date; mode)` of the Analysis add-in (mode 1: weeks start on Sunday, 2: Monday).
///
/// Owns the plain ODF and OOXML name WEEKNUM (AOO rule; the built-in is ISOWEEKNUM).
pub const WEEKNUM_ADD: FunctionDef =
    FunctionDef::new("WEEKNUM_ADD", Category::AddIn, WEEKNUM_PARAMS, weeknum)
        .odf("WEEKNUM")
        .help("Calendar week of a date; week 1 contains January 1.");

/// Adds `months` to a day number, clamping the day to the month length
/// (AOO `AddMonthsToDate`).
fn add_months(days: i64, months: i64) -> Result<(i64, u32, u32), ErrorKind> {
    let (y, m, d) = civil_from_days(days);
    let total = y
        .checked_mul(12)
        .and_then(|v| v.checked_add(i64::from(m) - 1))
        .and_then(|v| v.checked_add(months))
        .ok_or(ErrorKind::InvalidArgument)?;
    let ny = total.div_euclid(12);
    let nm = (total.rem_euclid(12) + 1) as u32;
    if !(1..=9999).contains(&ny) {
        return Err(ErrorKind::InvalidArgument);
    }
    Ok((ny, nm, d.min(days_in_month(ny, nm))))
}

fn edate(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let epoch = a.ctx().settings().date_epoch;
        let start = day_of(epoch, a.num(0)?)?;
        let months = a.int(1)?;
        let (y, m, d) = add_months(start, months)?;
        Ok(Value::Number(serial_of(epoch, days_from_civil(y, m, d))?))
    })
}

fn eomonth(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let epoch = a.ctx().settings().date_epoch;
        let start = day_of(epoch, a.num(0)?)?;
        let months = a.int(1)?;
        let (y, m, _) = add_months(start, months)?;
        let last = days_in_month(y, m);
        Ok(Value::Number(serial_of(
            epoch,
            days_from_civil(y, m, last),
        )?))
    })
}

/// `true` for Saturday and Sunday (1970-01-01 was a Thursday).
fn is_weekend(day: i64) -> bool {
    matches!((day + 4).rem_euclid(7), 0 | 6)
}

fn holidays(
    a: &mut Args,
    idx: usize,
    epoch: tally_types::DateEpoch,
) -> Result<HashSet<i64>, ErrorKind> {
    let mut out = HashSet::new();
    if a.is_missing(idx) {
        return Ok(out);
    }
    let mut bad = false;
    a.for_each_number(idx, |v| match day_of(epoch, v) {
        Ok(d) => {
            out.insert(d);
        }
        Err(_) => bad = true,
    })?;
    if bad {
        return Err(ErrorKind::InvalidArgument);
    }
    Ok(out)
}

/// Number of Monday..Friday days in `[from, to]`.
fn weekdays_between(from: i64, to: i64) -> i64 {
    let n = to - from + 1;
    let mut count = (n / 7) * 5;
    for i in 0..(n % 7) {
        if !is_weekend(from + (n / 7) * 7 + i) {
            count += 1;
        }
    }
    count
}

fn networkdays(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let epoch = a.ctx().settings().date_epoch;
        let s = day_of(epoch, a.num(0)?)?;
        let e = day_of(epoch, a.num(1)?)?;
        let hol = holidays(&mut a, 2, epoch)?;
        let (lo, hi, sign) = if s <= e { (s, e, 1.0) } else { (e, s, -1.0) };
        let mut n = weekdays_between(lo, hi);
        n -= hol
            .iter()
            .filter(|d| (lo..=hi).contains(*d) && !is_weekend(**d))
            .count() as i64;
        Ok(Value::Number(sign * n as f64))
    })
}

fn workday(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let epoch = a.ctx().settings().date_epoch;
        let start = day_of(epoch, a.num(0)?)?;
        let days = a.int(1)?;
        let hol = holidays(&mut a, 2, epoch)?;
        if days == 0 {
            return Ok(Value::Number(serial_of(epoch, start)?));
        }
        // 5 working days span at least 5 calendar days; anything beyond the
        // supported calendar range is an error anyway.
        if days.unsigned_abs() > 4_000_000 {
            return Err(ErrorKind::InvalidArgument);
        }
        let step = days.signum();
        let mut d = start;
        for _ in 0..days.abs() {
            d += step;
            while is_weekend(d) || hol.contains(&d) {
                d += step;
            }
        }
        Ok(Value::Number(serial_of(epoch, d)?))
    })
}

fn weeknum(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let epoch = a.ctx().settings().date_epoch;
        let date = day_of(epoch, a.num(0)?)?;
        let mode = a.int(1)?;
        if mode != 1 && mode != 2 {
            return Err(ErrorKind::InvalidArgument);
        }
        let (y, _, _) = civil_from_days(date);
        let first = days_from_civil(y, 1, 1);
        // Monday = 0 .. Sunday = 6
        let first_dow = (first + 3).rem_euclid(7);
        let offset = if mode == 1 {
            (first_dow + 1) % 7
        } else {
            first_dow
        };
        Ok(Value::Number(((date - first + offset) / 7 + 1) as f64))
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(EDATE);
    r.register(EOMONTH);
    r.register(NETWORKDAYS);
    r.register(WORKDAY);
    r.register(WEEKNUM_ADD);
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_numfmt::date::serial_from_ymd;
    use tally_types::{DateEpoch, ErrorKind, Value};

    fn ser(y: i32, m: u32, d: u32) -> f64 {
        serial_from_ymd(DateEpoch::D1899, y, m, d).unwrap()
    }
    fn n(x: f64) -> EvalValue {
        EvalValue::number(x)
    }
    fn c(name: &str, args: Vec<EvalValue>) -> Value {
        call(&mut MockEvalContext::new(), name, args)
    }

    #[test]
    fn edate_eomonth() {
        let r = c("EDATE", vec![n(ser(2026, 1, 31)), n(1.0)]);
        assert_eq!(r, Value::Number(ser(2026, 2, 28)));
        let r = c("EDATE", vec![n(ser(2026, 3, 15)), n(-5.0)]);
        assert_eq!(r, Value::Number(ser(2025, 10, 15)));
        let r = c("EOMONTH", vec![n(ser(2026, 2, 10)), n(0.0)]);
        assert_eq!(r, Value::Number(ser(2026, 2, 28)));
        let r = c("EOMONTH", vec![n(ser(2024, 1, 31)), n(1.0)]);
        assert_eq!(r, Value::Number(ser(2024, 2, 29)));
        assert_eq!(
            c("EDATE", vec![n(ser(9999, 12, 1)), n(1.0)]),
            Value::Error(ErrorKind::InvalidArgument)
        );
    }

    #[test]
    fn network_and_workday() {
        let r = c(
            "NETWORKDAYS",
            vec![n(ser(2026, 10, 1)), n(ser(2026, 10, 31))],
        );
        assert_eq!(r, Value::Number(22.0));
        let r = c(
            "NETWORKDAYS",
            vec![n(ser(2026, 10, 31)), n(ser(2026, 10, 1))],
        );
        assert_eq!(r, Value::Number(-22.0));
        let r = c(
            "NETWORKDAYS",
            vec![
                n(ser(2026, 10, 1)),
                n(ser(2026, 10, 31)),
                n(ser(2026, 10, 12)),
            ],
        );
        assert_eq!(r, Value::Number(21.0));
        let r = c("WORKDAY", vec![n(ser(2026, 10, 19)), n(5.0)]);
        assert_eq!(r, Value::Number(ser(2026, 10, 26)));
        let r = c("WORKDAY", vec![n(ser(2026, 10, 19)), n(-1.0)]);
        assert_eq!(r, Value::Number(ser(2026, 10, 16)));
        let r = c(
            "WORKDAY",
            vec![n(ser(2026, 10, 19)), n(1.0), n(ser(2026, 10, 20))],
        );
        assert_eq!(r, Value::Number(ser(2026, 10, 21)));
    }

    #[test]
    fn weeknum() {
        // 2026-01-01 is a Thursday.
        assert_eq!(
            c("WEEKNUM_ADD", vec![n(ser(2026, 1, 1)), n(1.0)]),
            Value::Number(1.0)
        );
        assert_eq!(
            c("WEEKNUM_ADD", vec![n(ser(2026, 1, 4)), n(1.0)]),
            Value::Number(2.0)
        );
        assert_eq!(
            c("WEEKNUM_ADD", vec![n(ser(2026, 1, 4)), n(2.0)]),
            Value::Number(1.0)
        );
        assert_eq!(
            c("WEEKNUM_ADD", vec![n(ser(2026, 1, 5)), n(2.0)]),
            Value::Number(2.0)
        );
        assert_eq!(
            c("WEEKNUM_ADD", vec![n(ser(2026, 1, 5)), n(3.0)]),
            Value::Error(ErrorKind::InvalidArgument)
        );
    }
}
