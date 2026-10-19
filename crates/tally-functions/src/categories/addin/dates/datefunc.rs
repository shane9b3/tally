//! WEEKS, MONTHS, YEARS, ISLEAPYEAR, DAYSINMONTH, DAYSINYEAR, WEEKSINYEAR, ROT13
//! (the Date add-in).
//!
//! Semantics ported from Apache OpenOffice `scaddins/source/datefunc/datefunc.cxx`
//! (Apache-2.0).

use tally_numfmt::date::{civil_from_days, days_in_month, is_leap_year};
use tally_types::{ErrorKind, Value};

use super::day_of;
use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry, ReturnKind};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const DIFF_PARAMS: &[Param] = &[
    Param::required("start_date", ParamKind::Number),
    Param::required("end_date", ParamKind::Number),
    Param::required("type", ParamKind::Integer),
];
const DATE_PARAM: &[Param] = &[Param::required("date", ParamKind::Number)];
const TEXT_PARAM: &[Param] = &[Param::required("text", ParamKind::Text)];

/// `WEEKS(start; end; type)`: type 0 counts whole 7-day intervals, 1 the difference of calendar weeks.
pub const WEEKS: FunctionDef = FunctionDef::new("WEEKS", Category::AddIn, DIFF_PARAMS, weeks)
    .odf("ORG.OPENOFFICE.WEEKS")
    .help("Number of weeks between two dates.");
/// `MONTHS(start; end; type)`.
pub const MONTHS: FunctionDef = FunctionDef::new("MONTHS", Category::AddIn, DIFF_PARAMS, months)
    .odf("ORG.OPENOFFICE.MONTHS")
    .help("Number of months between two dates.");
/// `YEARS(start; end; type)`.
pub const YEARS: FunctionDef = FunctionDef::new("YEARS", Category::AddIn, DIFF_PARAMS, years)
    .odf("ORG.OPENOFFICE.YEARS")
    .help("Number of years between two dates.");
/// `ISLEAPYEAR(date)`.
pub const ISLEAPYEAR: FunctionDef =
    FunctionDef::new("ISLEAPYEAR", Category::AddIn, DATE_PARAM, isleapyear)
        .odf("ORG.OPENOFFICE.ISLEAPYEAR")
        .returns(ReturnKind::Bool)
        .help("TRUE if the year of the date is a leap year.");
/// `DAYSINMONTH(date)`.
pub const DAYSINMONTH: FunctionDef =
    FunctionDef::new("DAYSINMONTH", Category::AddIn, DATE_PARAM, daysinmonth)
        .odf("ORG.OPENOFFICE.DAYSINMONTH")
        .help("Number of days in the month of the date.");
/// `DAYSINYEAR(date)`.
pub const DAYSINYEAR: FunctionDef =
    FunctionDef::new("DAYSINYEAR", Category::AddIn, DATE_PARAM, daysinyear)
        .odf("ORG.OPENOFFICE.DAYSINYEAR")
        .help("Number of days in the year of the date.");
/// `WEEKSINYEAR(date)`.
pub const WEEKSINYEAR: FunctionDef =
    FunctionDef::new("WEEKSINYEAR", Category::AddIn, DATE_PARAM, weeksinyear)
        .odf("ORG.OPENOFFICE.WEEKSINYEAR")
        .help("Number of ISO weeks in the year of the date.");
/// `ROT13(text)`.
pub const ROT13: FunctionDef = FunctionDef::new("ROT13", Category::AddIn, TEXT_PARAM, rot13)
    .odf("ORG.OPENOFFICE.ROT13")
    .returns(ReturnKind::Text)
    .help("Encrypts or decrypts text with the ROT13 substitution.");

/// Reads (start, end, type) of the diff functions as day numbers.
fn diff_args(a: &mut Args) -> Result<(i64, i64, i64), ErrorKind> {
    let epoch = a.ctx().settings().date_epoch;
    let s = day_of(epoch, a.num(0)?)?;
    let e = day_of(epoch, a.num(1)?)?;
    let t = a.int(2)?;
    if t != 0 && t != 1 {
        return Err(ErrorKind::InvalidArgument);
    }
    Ok((s, e, t))
}

/// Day of week with Monday = 0 for a unix day number.
fn dow_mon0(day: i64) -> i64 {
    (day + 3).rem_euclid(7)
}

fn weeks(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let (s, e, t) = diff_args(&mut a)?;
        let n = if t == 0 {
            (e - s) / 7
        } else {
            ((e - dow_mon0(e)) - (s - dow_mon0(s))) / 7
        };
        Ok(Value::Number(n as f64))
    })
}

fn months(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let (s, e, t) = diff_args(&mut a)?;
        let (y1, m1, d1) = civil_from_days(s);
        let (y2, m2, d2) = civil_from_days(e);
        let mut n = (y2 - y1) * 12 + i64::from(m2) - i64::from(m1);
        if t == 0 {
            if n > 0 && d2 < d1 {
                n -= 1;
            } else if n < 0 && d2 > d1 {
                n += 1;
            }
        }
        Ok(Value::Number(n as f64))
    })
}

fn years(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let (s, e, t) = diff_args(&mut a)?;
        let (y1, m1, d1) = civil_from_days(s);
        let (y2, m2, d2) = civil_from_days(e);
        let mut n = y2 - y1;
        if t == 0 {
            if n > 0 && (m2, d2) < (m1, d1) {
                n -= 1;
            } else if n < 0 && (m2, d2) > (m1, d1) {
                n += 1;
            }
        }
        Ok(Value::Number(n as f64))
    })
}

/// Evaluates argument 0 as a date and applies `f` to its (year, month).
fn with_date(
    ctx: &mut dyn EvalContext,
    args: &mut dyn ArgList,
    f: impl FnOnce(i64, i64, u32) -> Value,
) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let epoch = a.ctx().settings().date_epoch;
        let day = day_of(epoch, a.num(0)?)?;
        let (y, m, _) = civil_from_days(day);
        Ok(f(day, y, m))
    })
}

fn isleapyear(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    with_date(ctx, args, |_, y, _| Value::Bool(is_leap_year(y)))
}

fn daysinmonth(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    with_date(ctx, args, |_, y, m| {
        Value::Number(f64::from(days_in_month(y, m)))
    })
}

fn daysinyear(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    with_date(ctx, args, |_, y, _| {
        Value::Number(if is_leap_year(y) { 366.0 } else { 365.0 })
    })
}

fn weeksinyear(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    // ISO 8601: 53 weeks if January 1 is a Thursday, or a Wednesday in a leap year.
    with_date(ctx, args, |_, y, _| {
        let jan1 = tally_numfmt::date::days_from_civil(y, 1, 1);
        let dow = dow_mon0(jan1);
        let long = dow == 3 || (dow == 2 && is_leap_year(y));
        Value::Number(if long { 53.0 } else { 52.0 })
    })
}

fn rot13(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let s = a.text(0)?;
        let out: String = s
            .chars()
            .map(|c| match c {
                'a'..='z' => (((c as u8 - b'a') + 13) % 26 + b'a') as char,
                'A'..='Z' => (((c as u8 - b'A') + 13) % 26 + b'A') as char,
                _ => c,
            })
            .collect();
        Ok(Value::text(&*out))
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(WEEKS);
    r.register(MONTHS);
    r.register(YEARS);
    r.register(ISLEAPYEAR);
    r.register(DAYSINMONTH);
    r.register(DAYSINYEAR);
    r.register(WEEKSINYEAR);
    r.register(ROT13);
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_numfmt::date::serial_from_ymd;
    use tally_types::{DateEpoch, ErrorKind, Value};

    fn ser(y: i32, m: u32, d: u32) -> EvalValue {
        EvalValue::number(serial_from_ymd(DateEpoch::D1899, y, m, d).unwrap())
    }
    fn c(name: &str, args: Vec<EvalValue>) -> Value {
        call(&mut MockEvalContext::new(), name, args)
    }
    fn t(x: f64) -> EvalValue {
        EvalValue::number(x)
    }

    #[test]
    fn diffs() {
        let w = |a, b, ty| c("WEEKS", vec![a, b, t(ty)]);
        assert_eq!(
            w(ser(2026, 1, 1), ser(2026, 12, 31), 0.0),
            Value::Number(52.0)
        );
        // Thursday to the following Monday: 4 days, but the calendar weeks differ by 1.
        assert_eq!(w(ser(2026, 1, 1), ser(2026, 1, 5), 0.0), Value::Number(0.0));
        assert_eq!(w(ser(2026, 1, 1), ser(2026, 1, 5), 1.0), Value::Number(1.0));
        assert_eq!(w(ser(2026, 1, 5), ser(2026, 1, 1), 0.0), Value::Number(0.0));
        let m = |a, b, ty| c("MONTHS", vec![a, b, t(ty)]);
        assert_eq!(
            m(ser(2026, 1, 31), ser(2026, 3, 30), 0.0),
            Value::Number(1.0)
        );
        assert_eq!(
            m(ser(2026, 1, 31), ser(2026, 3, 30), 1.0),
            Value::Number(2.0)
        );
        assert_eq!(
            m(ser(2026, 3, 30), ser(2026, 1, 31), 0.0),
            Value::Number(-1.0)
        );
        let y = |a, b, ty| c("YEARS", vec![a, b, t(ty)]);
        assert_eq!(
            y(ser(2020, 6, 15), ser(2026, 6, 14), 0.0),
            Value::Number(5.0)
        );
        assert_eq!(
            y(ser(2020, 6, 15), ser(2026, 6, 14), 1.0),
            Value::Number(6.0)
        );
        assert_eq!(
            y(ser(2020, 6, 15), ser(2026, 6, 14), 2.0),
            Value::Error(ErrorKind::InvalidArgument)
        );
    }

    #[test]
    fn calendar_facts() {
        assert_eq!(c("ISLEAPYEAR", vec![ser(2024, 1, 1)]), Value::Bool(true));
        assert_eq!(c("ISLEAPYEAR", vec![ser(1900, 1, 1)]), Value::Bool(false));
        assert_eq!(
            c("DAYSINMONTH", vec![ser(2024, 2, 10)]),
            Value::Number(29.0)
        );
        assert_eq!(
            c("DAYSINYEAR", vec![ser(2024, 2, 10)]),
            Value::Number(366.0)
        );
        assert_eq!(c("WEEKSINYEAR", vec![ser(2026, 5, 1)]), Value::Number(53.0));
        assert_eq!(c("WEEKSINYEAR", vec![ser(2025, 5, 1)]), Value::Number(52.0));
        assert_eq!(c("WEEKSINYEAR", vec![ser(2020, 5, 1)]), Value::Number(53.0));
    }

    #[test]
    fn rot13() {
        assert_eq!(
            c("ROT13", vec![EvalValue::text("Hello, World 1")]),
            Value::text("Uryyb, Jbeyq 1")
        );
        assert_eq!(c("ROT13", vec![EvalValue::text("")]), Value::text(""));
    }
}
