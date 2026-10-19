//! AMORDEGRC, AMORLINC, CUMIPMT_ADD, CUMPRINC_ADD, DOLLARDE, DOLLARFR,
//! EFFECT_ADD, NOMINAL_ADD, FVSCHEDULE, XIRR, XNPV.
//!
//! Ported from Apache OpenOffice scaddins/analysis (analysishelper.cxx,
//! financial.cxx), Apache License 2.0.
//!
//! The `_ADD` functions clash with built-ins of the same base name. AOO's
//! `odffmap.cxx` maps the ODF names CUMIPMT, CUMPRINC, EFFECT and NOMINAL
//! to the built-ins (`bMapDupToInternal`), so these add-ins are stored by
//! their programmatic name and have no OOXML alias (the plain Excel names
//! mean the built-ins, see `docs/phase-a.md`).

use tally_types::{round_to_15_digits, ErrorKind, Value};

use super::daycount::{year_frac, IAE};
use super::{any_num, basis, double_array, finite, long, long_array, null_date};
use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

/// `rtl::math::round(x, 0)`: half away from zero after 15-digit snapping.
fn round0(x: f64) -> f64 {
    round_to_15_digits(x).round()
}

const AMOR_PARAMS: &[Param] = &[
    Param::required("cost", ParamKind::Number),
    Param::required("date_purchased", ParamKind::Integer),
    Param::required("first_period", ParamKind::Integer),
    Param::required("salvage", ParamKind::Number),
    Param::required("period", ParamKind::Number),
    Param::required("rate", ParamKind::Number),
    Param::optional("basis", ParamKind::Integer),
];

/// Validated AMOR* arguments: `(nd, cost, date, first, salvage, period, rate, basis)`.
type AmorArgs = (i64, f64, i64, i64, f64, u64, f64, i32);

fn amor_args(a: &mut Args<'_>) -> Result<AmorArgs, ErrorKind> {
    let nd = null_date(a);
    let cost = a.num(0)?;
    let date = long(a, 1)?;
    let first = long(a, 2)?;
    let salvage = a.num(3)?;
    let per = a.num(4)?;
    let rate = a.num(5)?;
    if date > first || rate <= 0.0 || salvage > cost {
        return Err(IAE);
    }
    let b = basis(a, 6)?;
    if b == 2 {
        return Err(IAE);
    }
    // AOO converts with `sal_uInt32( fPer )`; a negative period is not
    // meaningful (undefined behaviour in C++), so it is rejected.
    if !(0.0..4_294_967_296.0).contains(&per) {
        return Err(IAE);
    }
    Ok((nd, cost, date, first, salvage, per as u64, rate, b))
}

/// `AMORDEGRC(cost; date_purchased; first_period; salvage; period; rate; [basis])`.
pub const AMORDEGRC: FunctionDef =
    FunctionDef::new("AMORDEGRC", Category::AddIn, AMOR_PARAMS, amordegrc)
        .help("Returns the prorated linear depreciation of an asset for each accounting period");

fn amordegrc(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let (nd, mut cost, date, first, salvage, per, mut rate, b) = amor_args(&mut a)?;
        let use_per = 1.0 / rate;
        let coeff = if use_per < 3.0 {
            1.0
        } else if use_per < 5.0 {
            1.5
        } else if use_per <= 6.0 {
            2.0
        } else {
            2.5
        };
        rate *= coeff;
        let mut nrate = round0(year_frac(nd, date, first, b)? * rate * cost);
        cost -= nrate;
        let mut rest = cost - salvage;
        for n in 0..per {
            nrate = round0(rate * cost);
            rest -= nrate;
            if rest < 0.0 {
                return finite(match per - n {
                    0 | 1 => round0(cost * 0.5),
                    _ => 0.0,
                });
            }
            if nrate == 0.0 {
                // Nothing changes any more; the remaining periods are 0.
                return Ok(Value::Number(0.0));
            }
            cost -= nrate;
        }
        finite(nrate)
    })
}

/// `AMORLINC(cost; date_purchased; first_period; salvage; period; rate; [basis])`.
pub const AMORLINC: FunctionDef =
    FunctionDef::new("AMORLINC", Category::AddIn, AMOR_PARAMS, amorlinc)
        .help("Returns the prorated linear depreciation of an asset for each accounting period");

fn amorlinc(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let (nd, cost, date, first, salvage, per, rate, b) = amor_args(&mut a)?;
        let one = cost * rate;
        let delta = cost - salvage;
        let r0 = year_frac(nd, date, first, b)? * rate * cost;
        let full = ((cost - salvage - r0) / one).clamp(0.0, 4_294_967_295.0) as u64;
        finite(if per == 0 {
            r0
        } else if per <= full {
            one
        } else if per == full + 1 {
            delta - one * full as f64 - r0
        } else {
            0.0
        })
    })
}

/// Add-in `GetRmz`.
fn get_rmz(rate: f64, nper: f64, pv: f64, fv: f64, ptype: i64) -> f64 {
    let rmz = if rate == 0.0 {
        (pv + fv) / nper
    } else {
        let term = (1.0 + rate).powf(nper);
        let r = fv * rate / (term - 1.0) + pv * rate / (1.0 - 1.0 / term);
        if ptype > 0 {
            r / (1.0 + rate)
        } else {
            r
        }
    };
    -rmz
}

/// Add-in `GetZw`.
fn get_zw(rate: f64, nper: f64, pmt: f64, pv: f64, ptype: i64) -> f64 {
    let zw = if rate == 0.0 {
        pv + pmt * nper
    } else {
        let term = (1.0 + rate).powf(nper);
        if ptype > 0 {
            pv * term + pmt * (1.0 + rate) * (term - 1.0) / rate
        } else {
            pv * term + pmt * (term - 1.0) / rate
        }
    };
    -zw
}

const CUM_PARAMS: &[Param] = &[
    Param::required("rate", ParamKind::Number),
    Param::required("nper", ParamKind::Integer),
    Param::required("pv", ParamKind::Number),
    Param::required("start_period", ParamKind::Integer),
    Param::required("end_period", ParamKind::Integer),
    Param::required("type", ParamKind::Integer),
];

type CumArgs = (f64, i64, f64, i64, i64, i64);

fn cum_args(a: &mut Args<'_>) -> Result<CumArgs, ErrorKind> {
    let rate = a.num(0)?;
    let nper = long(a, 1)?;
    let pv = a.num(2)?;
    let start = long(a, 3)?;
    let end = long(a, 4)?;
    let t = long(a, 5)?;
    if start < 1
        || end < start
        || rate <= 0.0
        || end > nper
        || nper <= 0
        || pv <= 0.0
        || (t != 0 && t != 1)
    {
        return Err(IAE);
    }
    Ok((rate, nper, pv, start, end, t))
}

/// `CUMIPMT_ADD(rate; nper; pv; start_period; end_period; type)`.
pub const CUMIPMT_ADD: FunctionDef =
    FunctionDef::new("CUMIPMT_ADD", Category::AddIn, CUM_PARAMS, cumipmt_add)
        .odf("COM.SUN.STAR.SHEET.ADDIN.ANALYSIS.GETCUMIPMT")
        .help("Returns the cumulative interest to be paid between two periods");

fn cumipmt_add(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let (rate, nper, pv, mut start, end, t) = cum_args(&mut a)?;
        let rmz = get_rmz(rate, nper as f64, pv, 0.0, t);
        let mut sum = 0.0;
        if start == 1 {
            if t <= 0 {
                sum = -pv;
            }
            start += 1;
        }
        for i in start..=end {
            if t > 0 {
                sum += get_zw(rate, (i - 2) as f64, rmz, pv, 1) - rmz;
            } else {
                sum += get_zw(rate, (i - 1) as f64, rmz, pv, 0);
            }
        }
        finite(sum * rate)
    })
}

/// `CUMPRINC_ADD(rate; nper; pv; start_period; end_period; type)`.
pub const CUMPRINC_ADD: FunctionDef =
    FunctionDef::new("CUMPRINC_ADD", Category::AddIn, CUM_PARAMS, cumprinc_add)
        .odf("COM.SUN.STAR.SHEET.ADDIN.ANALYSIS.GETCUMPRINC")
        .help("Returns the cumulative principal on a loan to be paid between two periods");

fn cumprinc_add(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let (rate, nper, pv, mut start, end, t) = cum_args(&mut a)?;
        let rmz = get_rmz(rate, nper as f64, pv, 0.0, t);
        let mut sum = 0.0;
        if start == 1 {
            sum = if t <= 0 { rmz + pv * rate } else { rmz };
            start += 1;
        }
        for i in start..=end {
            if t > 0 {
                sum += rmz - (get_zw(rate, (i - 2) as f64, rmz, pv, 1) - rmz) * rate;
            } else {
                sum += rmz - get_zw(rate, (i - 1) as f64, rmz, pv, 0) * rate;
            }
        }
        finite(sum)
    })
}

const DOLLAR_PARAMS: &[Param] = &[
    Param::required("dollar", ParamKind::Number),
    Param::required("fraction", ParamKind::Integer),
];

/// `DOLLARDE(fractional_dollar; fraction)`.
pub const DOLLARDE: FunctionDef =
    FunctionDef::new("DOLLARDE", Category::AddIn, DOLLAR_PARAMS, dollarde)
        .help("Converts a price expressed as a fraction into a price expressed as a decimal");

fn dollarde(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let x = a.num(0)?;
        let frac = long(&mut a, 1)?;
        if frac <= 0 {
            return Err(IAE);
        }
        let f = frac as f64;
        let mut r = x.fract();
        r /= f;
        r *= 10f64.powf(f.log10().ceil());
        finite(r + x.trunc())
    })
}

/// `DOLLARFR(decimal_dollar; fraction)`.
pub const DOLLARFR: FunctionDef =
    FunctionDef::new("DOLLARFR", Category::AddIn, DOLLAR_PARAMS, dollarfr)
        .help("Converts a price expressed as a decimal into a price expressed as a fraction");

fn dollarfr(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let x = a.num(0)?;
        let frac = long(&mut a, 1)?;
        if frac <= 0 {
            return Err(IAE);
        }
        let f = frac as f64;
        let mut r = x.fract();
        r *= f;
        r *= 10f64.powf(-f.log10().ceil());
        finite(r + x.trunc())
    })
}

const EFFECT_PARAMS: &[Param] = &[
    Param::required("nominal_rate", ParamKind::Number),
    Param::required("npery", ParamKind::Integer),
];

/// `EFFECT_ADD(nominal_rate; npery)`.
pub const EFFECT_ADD: FunctionDef =
    FunctionDef::new("EFFECT_ADD", Category::AddIn, EFFECT_PARAMS, effect_add)
        .odf("COM.SUN.STAR.SHEET.ADDIN.ANALYSIS.GETEFFECT")
        .help("Returns the effective annual interest rate");

fn effect_add(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let nominal = a.num(0)?;
        let n = long(&mut a, 1)?;
        if n < 1 || nominal <= 0.0 {
            return Err(IAE);
        }
        let p = n as f64;
        finite((1.0 + nominal / p).powf(p) - 1.0)
    })
}

const NOMINAL_PARAMS: &[Param] = &[
    Param::required("effective_rate", ParamKind::Number),
    Param::required("npery", ParamKind::Integer),
];

/// `NOMINAL_ADD(effective_rate; npery)`.
pub const NOMINAL_ADD: FunctionDef =
    FunctionDef::new("NOMINAL_ADD", Category::AddIn, NOMINAL_PARAMS, nominal_add)
        .odf("COM.SUN.STAR.SHEET.ADDIN.ANALYSIS.GETNOMINAL")
        .help("Returns the annual nominal interest rate");

fn nominal_add(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let rate = a.num(0)?;
        let n = long(&mut a, 1)?;
        if rate <= 0.0 || n < 0 {
            return Err(IAE);
        }
        let p = n as f64;
        finite(((rate + 1.0).powf(1.0 / p) - 1.0) * p)
    })
}

const FVSCHEDULE_PARAMS: &[Param] = &[
    Param::required("principal", ParamKind::Number),
    Param::required("schedule", ParamKind::Array),
];

/// `FVSCHEDULE(principal; schedule)`.
pub const FVSCHEDULE: FunctionDef =
    FunctionDef::new("FVSCHEDULE", Category::AddIn, FVSCHEDULE_PARAMS, fvschedule)
        .help("Returns the future value of the initial principal after a series of compound interest rates are applied");

fn fvschedule(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let mut p = a.num(0)?;
        for r in double_array(&mut a, 1)? {
            p *= 1.0 + r;
        }
        finite(p)
    })
}

const XNPV_PARAMS: &[Param] = &[
    Param::required("rate", ParamKind::Number),
    Param::required("values", ParamKind::Array),
    Param::required("dates", ParamKind::Array),
];

/// `XNPV(rate; values; dates)`.
pub const XNPV: FunctionDef = FunctionDef::new("XNPV", Category::AddIn, XNPV_PARAMS, xnpv)
    .help("Returns the net present value for a non-periodic schedule of payments");

fn xnpv(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let rate = a.num(0)? + 1.0;
        let values = double_array(&mut a, 1)?;
        let dates = long_array(&mut a, 2)?;
        if values.len() != dates.len() || values.len() < 2 {
            return Err(IAE);
        }
        let d0 = dates[0];
        let sum: f64 = values
            .iter()
            .zip(&dates)
            .map(|(v, d)| v / rate.powf((d - d0) / 365.0))
            .sum();
        finite(sum)
    })
}

/// `lcl_sca_XirrResult`.
fn xirr_result(values: &[f64], dates: &[f64], rate: f64) -> f64 {
    let r = rate + 1.0;
    let mut res = values[0];
    for i in 1..values.len() {
        res += values[i] / r.powf((dates[i] - dates[0]) / 365.0);
    }
    res
}

/// `lcl_sca_XirrResult_Deriv1`.
fn xirr_deriv(values: &[f64], dates: &[f64], rate: f64) -> f64 {
    let r = rate + 1.0;
    let mut res = 0.0;
    for i in 1..values.len() {
        let e = (dates[i] - dates[0]) / 365.0;
        res -= e * values[i] / r.powf(e + 1.0);
    }
    res
}

const XIRR_PARAMS: &[Param] = &[
    Param::required("values", ParamKind::Array),
    Param::required("dates", ParamKind::Array),
    Param::optional("guess", ParamKind::Number),
];

/// `XIRR(values; dates; [guess])`.
pub const XIRR: FunctionDef = FunctionDef::new("XIRR", Category::AddIn, XIRR_PARAMS, xirr)
    .help("Returns the internal rate of return for a non-periodic schedule of payments");

fn xirr(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let values = double_array(&mut a, 0)?;
        let dates = long_array(&mut a, 1)?;
        if values.len() < 2 || values.len() != dates.len() {
            return Err(IAE);
        }
        let mut rate = any_num(&mut a, 2, 0.1)?;
        if rate <= -1.0 {
            return Err(IAE);
        }
        const MAX_EPS: f64 = 1e-10;
        const MAX_ITER: u32 = 50;
        // Newton from the guess; if that fails, restart from -0.99, -0.98,
        // ... (200 scans), as AOO does.
        let mut scan = 0;
        let mut cont;
        loop {
            if scan >= 1 {
                rate = -0.99 + f64::from(scan - 1) * 0.01;
            }
            let mut iter = 0;
            let mut value;
            loop {
                value = xirr_result(&values, &dates, rate);
                let new = rate - value / xirr_deriv(&values, &dates, rate);
                let eps = (new - rate).abs();
                rate = new;
                cont = eps > MAX_EPS && value.abs() > MAX_EPS;
                iter += 1;
                if !(cont && iter < MAX_ITER) {
                    break;
                }
            }
            if !rate.is_finite() || !value.is_finite() {
                cont = true;
            }
            scan += 1;
            if !(cont && scan < 200) {
                break;
            }
        }
        if cont {
            return Err(IAE);
        }
        finite(rate)
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(AMORDEGRC);
    r.register(AMORLINC);
    r.register(CUMIPMT_ADD);
    r.register(CUMPRINC_ADD);
    r.register(DOLLARDE);
    r.register(DOLLARFR);
    r.register(EFFECT_ADD);
    r.register(NOMINAL_ADD);
    r.register(FVSCHEDULE);
    r.register(XIRR);
    r.register(XNPV);
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{ErrorKind, Matrix, Value};

    fn row(xs: &[f64]) -> EvalValue {
        let mut m = Matrix::new_filled(1, xs.len() as u32, Value::Empty);
        for (i, &x) in xs.iter().enumerate() {
            m.set(0, i as u32, Value::Number(x));
        }
        EvalValue::Array(Arc::new(m))
    }

    fn n(x: f64) -> EvalValue {
        EvalValue::number(x)
    }

    fn num(v: Value) -> f64 {
        match v {
            Value::Number(n) => n,
            other => panic!("{other:?}"),
        }
    }

    fn close(a: f64, b: f64, tol: f64) {
        assert!((a - b).abs() <= tol, "{a} vs {b}");
    }

    #[test]
    fn amortisation() {
        let mut c = MockEvalContext::new();
        // 2008-08-19, 2008-12-31
        let args = [2400.0, 39679.0, 39813.0, 300.0, 1.0, 0.15, 1.0];
        let v = |c: &mut MockEvalContext, f| num(call(c, f, args.map(n).to_vec()));
        assert_eq!(v(&mut c, "AMORDEGRC"), 776.0);
        assert_eq!(v(&mut c, "AMORLINC"), 360.0);
        let mut b2 = args;
        b2[6] = 2.0;
        assert_eq!(
            call(&mut c, "AMORLINC", b2.map(n).to_vec()),
            Value::Error(ErrorKind::InvalidArgument)
        );
    }

    #[test]
    fn cumulative_add() {
        let mut c = MockEvalContext::new();
        let args = [0.09 / 12.0, 360.0, 125000.0, 13.0, 24.0, 0.0];
        close(
            num(call(&mut c, "CUMIPMT_ADD", args.map(n).to_vec())),
            -11135.23213,
            1e-5,
        );
        close(
            num(call(&mut c, "CUMPRINC_ADD", args.map(n).to_vec())),
            -934.1071234,
            1e-6,
        );
        let mut t2 = args;
        t2[5] = 2.0;
        assert_eq!(
            call(&mut c, "CUMIPMT_ADD", t2.map(n).to_vec()),
            Value::Error(ErrorKind::InvalidArgument)
        );
    }

    #[test]
    fn dollars_and_rates() {
        let mut c = MockEvalContext::new();
        close(
            num(call(&mut c, "DOLLARDE", vec![n(1.02), n(16.0)])),
            1.125,
            1e-12,
        );
        close(
            num(call(&mut c, "DOLLARFR", vec![n(1.125), n(16.0)])),
            1.02,
            1e-12,
        );
        close(
            num(call(&mut c, "EFFECT_ADD", vec![n(0.0525), n(4.0)])),
            0.0535426673,
            1e-9,
        );
        close(
            num(call(&mut c, "NOMINAL_ADD", vec![n(0.053543), n(4.0)])),
            0.0525003,
            1e-6,
        );
        close(
            num(call(
                &mut c,
                "FVSCHEDULE",
                vec![n(1.0), row(&[0.09, 0.11, 0.1])],
            )),
            1.33089,
            1e-12,
        );
    }

    #[test]
    fn x_functions() {
        let mut c = MockEvalContext::new();
        let v = row(&[-10000.0, 2750.0, 4250.0, 3250.0, 2750.0]);
        let d = row(&[39448.0, 39508.0, 39751.0, 39859.0, 39904.0]);
        close(
            num(call(&mut c, "XNPV", vec![n(0.09), v.clone(), d.clone()])),
            2086.6476020,
            1e-6,
        );
        close(num(call(&mut c, "XIRR", vec![v, d])), 0.373362535, 1e-8);
        assert_eq!(
            call(&mut c, "XNPV", vec![n(0.09), row(&[1.0, 2.0]), row(&[1.0])]),
            Value::Error(ErrorKind::InvalidArgument)
        );
    }
}
