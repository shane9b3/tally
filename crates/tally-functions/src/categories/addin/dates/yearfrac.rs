//! YEARFRAC (Analysis add-in).
//!
//! Day-count conventions ported from Apache OpenOffice
//! `scaddins/source/analysis/analysishelper.cxx` (`GetYearFrac`,
//! `GetDiffDate360`; Apache-2.0).

use tally_numfmt::date::{civil_from_days, days_from_civil, is_leap_year};
use tally_types::{ErrorKind, Value};

use super::day_of;
use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const PARAMS: &[Param] = &[
    Param::required("start_date", ParamKind::Number),
    Param::required("end_date", ParamKind::Number),
    Param::optional("basis", ParamKind::Integer),
];

/// `YEARFRAC(start; end [; basis])`.
pub const YEARFRAC: FunctionDef = FunctionDef::new("YEARFRAC", Category::AddIn, PARAMS, yearfrac)
    .help("Fraction of a year between two dates for a day-count basis (0..4).");

/// 30/360 day difference (AOO `GetDiffDate360`).
fn diff360(d1: i64, d2: i64, usa: bool) -> i64 {
    let (y1, m1, mut dd1) = civil_from_days(d1);
    let (y2, mut m2, mut dd2) = civil_from_days(d2);
    let (m1, dd1u) = (i64::from(m1), &mut dd1);
    if *dd1u == 31 {
        *dd1u -= 1;
    } else if usa && m1 == 2 && (*dd1u == 29 || (*dd1u == 28 && !is_leap_year(y1))) {
        *dd1u = 30;
    }
    if dd2 == 31 {
        if usa && *dd1u != 30 {
            dd2 = 1;
            m2 += 1;
        } else {
            dd2 = 30;
        }
    }
    (y2 * 360 + i64::from(m2) * 30 + i64::from(dd2)) - (y1 * 360 + m1 * 30 + i64::from(*dd1u))
}

/// Length of the year for the Actual/Actual basis.
fn act_act_year_days(d1: i64, d2: i64) -> f64 {
    let (y1, m1, dd1) = civil_from_days(d1);
    let (y2, m2, dd2) = civil_from_days(d2);
    let len = |y: i64| if is_leap_year(y) { 366.0 } else { 365.0 };
    if y1 == y2 {
        return len(y1);
    }
    let within_year = y2 == y1 + 1 && (m1 > m2 || (m1 == m2 && dd1 >= dd2));
    if within_year {
        let feb29 = |y: i64| days_from_civil(y, 2, 29);
        let hit = (is_leap_year(y1) && d1 <= feb29(y1)) || (is_leap_year(y2) && d2 >= feb29(y2));
        return if hit { 366.0 } else { 365.0 };
    }
    let total: f64 = (y1..=y2).map(len).sum();
    total / (y2 - y1 + 1) as f64
}

/// Year fraction for `basis` between two day numbers (`d1 <= d2`).
pub(crate) fn year_frac(d1: i64, d2: i64, basis: i64) -> Result<f64, ErrorKind> {
    let (d1, d2) = if d1 <= d2 { (d1, d2) } else { (d2, d1) };
    if d1 == d2 {
        return Ok(0.0);
    }
    Ok(match basis {
        0 => diff360(d1, d2, true) as f64 / 360.0,
        1 => (d2 - d1) as f64 / act_act_year_days(d1, d2),
        2 => (d2 - d1) as f64 / 360.0,
        3 => (d2 - d1) as f64 / 365.0,
        4 => diff360(d1, d2, false) as f64 / 360.0,
        _ => return Err(ErrorKind::InvalidArgument),
    })
}

fn yearfrac(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let epoch = a.ctx().settings().date_epoch;
        let d1 = day_of(epoch, a.num(0)?)?;
        let d2 = day_of(epoch, a.num(1)?)?;
        let basis = a.int_opt(2, 0)?;
        year_frac(d1, d2, basis).map(Value::Number)
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(YEARFRAC);
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_numfmt::date::serial_from_ymd;
    use tally_types::{DateEpoch, ErrorKind, Value};

    fn yf(a: (i32, u32, u32), b: (i32, u32, u32), basis: Option<f64>) -> Value {
        let s = |t: (i32, u32, u32)| serial_from_ymd(DateEpoch::D1899, t.0, t.1, t.2).unwrap();
        let mut args = vec![EvalValue::number(s(a)), EvalValue::number(s(b))];
        args.extend(basis.map(EvalValue::number));
        call(&mut MockEvalContext::new(), "YEARFRAC", args)
    }

    fn close(v: Value, want: f64) {
        match v {
            Value::Number(x) => assert!((x - want).abs() < 1e-12, "{x} vs {want}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn bases() {
        close(yf((2026, 1, 1), (2026, 7, 1), Some(0.0)), 0.5);
        close(yf((2026, 1, 1), (2026, 7, 1), None), 0.5);
        close(yf((2026, 1, 1), (2026, 7, 1), Some(1.0)), 181.0 / 365.0);
        close(yf((2026, 1, 1), (2026, 7, 1), Some(2.0)), 181.0 / 360.0);
        close(yf((2026, 1, 1), (2026, 7, 1), Some(3.0)), 181.0 / 365.0);
        close(yf((2026, 1, 31), (2026, 3, 31), Some(4.0)), 60.0 / 360.0);
        close(yf((2026, 1, 31), (2026, 3, 31), Some(0.0)), 60.0 / 360.0);
        close(yf((2026, 7, 1), (2026, 1, 1), Some(0.0)), 0.5);
        close(yf((2026, 7, 1), (2026, 7, 1), Some(1.0)), 0.0);
        close(yf((2024, 1, 1), (2024, 12, 31), Some(1.0)), 365.0 / 366.0);
    }

    #[test]
    fn multi_year_average() {
        // 2020-01-01 .. 2022-01-01: 731 days over years 2020..2022 (avg 365.333..)
        close(
            yf((2020, 1, 1), (2022, 1, 1), Some(1.0)),
            731.0 / ((366.0 + 365.0 + 365.0) / 3.0),
        );
    }

    #[test]
    fn bad_basis() {
        assert_eq!(
            yf((2026, 1, 1), (2026, 7, 1), Some(5.0)),
            Value::Error(ErrorKind::InvalidArgument)
        );
    }
}
