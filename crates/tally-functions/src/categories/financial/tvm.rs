//! Time value of money: PV, FV, PMT, NPER, RATE, IPMT, PPMT, CUMIPMT,
//! CUMPRINC, ISPMT, EFFECTIVE, NOMINAL, DURATION (periods), ZGZ (RRI).
//!
//! Ported from Apache OpenOffice `sc/source/core/tool/interpr2.cxx`
//! (`ScBW`, `ScZW`, `ScRMZ`, `ScZZR`, `ScZins`/`RateIteration`,
//! `ScZinsZ`, `ScKapz`, `ScKumZinsZ`, `ScKumKapZ`, `ScISPMT`,
//! `ScEffektiv`, `ScNominal`, `ScLaufz`, `ScZGZ`), Apache License 2.0.

use tally_types::{approx_equal, round_to_15_digits, ErrorKind, Value};

use super::opt_num;
use crate::coerce::approx_floor;
use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

/// AOO `SCdEpsilon`.
pub(crate) const SC_EPSILON: f64 = 1.0e-7;

/// `ScGetBw`: present value.
pub(crate) fn get_pv(rate: f64, nper: f64, pmt: f64, fv: f64, ptype: f64) -> f64 {
    let pv = if rate == 0.0 {
        fv + pmt * nper
    } else if ptype > 0.0 {
        (fv * (1.0 + rate).powf(-nper))
            + (pmt * (1.0 - (1.0 + rate).powf(-nper + 1.0)) / rate)
            + pmt
    } else {
        (fv * (1.0 + rate).powf(-nper)) + (pmt * (1.0 - (1.0 + rate).powf(-nper)) / rate)
    };
    -pv
}

/// `ScGetZw`: future value.
pub(crate) fn get_fv(rate: f64, nper: f64, pmt: f64, pv: f64, ptype: f64) -> f64 {
    let fv = if rate == 0.0 {
        pv + pmt * nper
    } else {
        let term = (1.0 + rate).powf(nper);
        if ptype > 0.0 {
            pv * term + pmt * (1.0 + rate) * (term - 1.0) / rate
        } else {
            pv * term + pmt * (term - 1.0) / rate
        }
    };
    -fv
}

/// `ScGetRmz`: periodic payment (log1p/expm1 form of AOO trunk).
pub(crate) fn get_pmt(rate: f64, nper: f64, pv: f64, fv: f64, ptype: f64) -> f64 {
    let pmt = if rate == 0.0 {
        (pv + fv) / nper
    } else if ptype > 0.0 {
        (fv + pv * (nper * rate.ln_1p()).exp()) * rate
            / (((nper + 1.0) * rate.ln_1p()).exp_m1() - rate)
    } else {
        (fv + pv * (nper * rate.ln_1p()).exp()) * rate / (nper * rate.ln_1p()).exp_m1()
    };
    -pmt
}

/// `ScGetZinsZ`: interest part of payment `per`; also returns the payment.
fn get_ipmt(rate: f64, per: f64, nper: f64, pv: f64, fv: f64, ptype: f64) -> (f64, f64) {
    let pmt = get_pmt(rate, nper, pv, fv, ptype);
    let ipmt = if per == 1.0 {
        if ptype > 0.0 {
            0.0
        } else {
            -pv
        }
    } else if ptype > 0.0 {
        get_fv(rate, per - 2.0, pmt, pv, 1.0) - pmt
    } else {
        get_fv(rate, per - 1.0, pmt, pv, 0.0)
    };
    (ipmt * rate, pmt)
}

/// `RateIteration`: Newton-Raphson for RATE, at most 150 steps; integer and
/// non-integer `nper` are handled separately as in AOO. Returns the last
/// approximation and whether it converged.
fn rate_iteration(nper: f64, pmt: f64, pv: f64, fv: f64, ptype: f64, guess: f64) -> (f64, bool) {
    const MAX_ITER: u32 = 150;
    const EPS_SMALL: f64 = 1.0e-14;
    let fv = fv - pmt * ptype;
    let pv = pv + pmt * ptype;
    let mut valid = true;
    let mut found = false;
    let mut count = 0;
    let mut x;
    if nper == round_to_15_digits(nper).round() {
        x = guess;
        while !found && count < MAX_ITER {
            let pow_n1 = (1.0 + x).powf(nper - 1.0);
            let pow_n = pow_n1 * (1.0 + x);
            let (geo, geo_d) = if approx_equal(x.abs(), 0.0) {
                (nper, nper * (nper - 1.0) / 2.0)
            } else {
                let g = (pow_n - 1.0) / x;
                (g, nper * pow_n1 / x - g / x)
            };
            let term = fv + pv * pow_n + pmt * geo;
            let term_d = pv * nper * pow_n1 + pmt * geo_d;
            if term.abs() < EPS_SMALL {
                found = true;
            } else {
                let xnew = if approx_equal(term_d.abs(), 0.0) {
                    x + 1.1 * SC_EPSILON
                } else {
                    x - term / term_d
                };
                count += 1;
                found = (xnew - x).abs() < SC_EPSILON;
                x = xnew;
            }
        }
    } else {
        x = if guess < -1.0 { -1.0 } else { guess };
        while valid && !found && count < MAX_ITER {
            let (geo, geo_d) = if approx_equal(x.abs(), 0.0) {
                (nper, nper * (nper - 1.0) / 2.0)
            } else {
                let g = ((1.0 + x).powf(nper) - 1.0) / x;
                (g, nper * (1.0 + x).powf(nper - 1.0) / x - g / x)
            };
            let term = fv + pv * (1.0 + x).powf(nper) + pmt * geo;
            let term_d = pv * nper * (1.0 + x).powf(nper - 1.0) + pmt * geo_d;
            if term.abs() < EPS_SMALL {
                found = true;
            } else {
                let xnew = if approx_equal(term_d.abs(), 0.0) {
                    x + 1.1 * SC_EPSILON
                } else {
                    x - term / term_d
                };
                count += 1;
                found = (xnew - x).abs() < SC_EPSILON;
                x = xnew;
                valid = x >= -1.0;
            }
        }
    }
    (x, valid && found)
}

const PV_PARAMS: &[Param] = &[
    Param::required("rate", ParamKind::Number),
    Param::required("nper", ParamKind::Number),
    Param::required("pmt", ParamKind::Number),
    Param::optional("fv", ParamKind::Number),
    Param::optional("type", ParamKind::Integer),
];

/// `PV(rate; nper; pmt; [fv]; [type])`.
pub const PV: FunctionDef = FunctionDef::new("PV", Category::Financial, PV_PARAMS, pv)
    .help("Present value. Calculates the present value of an investment.");

fn pv(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let rate = a.num(0)?;
        let nper = a.num(1)?;
        let pmt = a.num(2)?;
        let fv = opt_num(&mut a, 3, 0.0)?;
        let t = opt_num(&mut a, 4, 0.0)?;
        Ok(Value::Number(get_pv(rate, nper, pmt, fv, t)))
    })
}

const FV_PARAMS: &[Param] = &[
    Param::required("rate", ParamKind::Number),
    Param::required("nper", ParamKind::Number),
    Param::required("pmt", ParamKind::Number),
    Param::optional("pv", ParamKind::Number),
    Param::optional("type", ParamKind::Integer),
];

/// `FV(rate; nper; pmt; [pv]; [type])`.
pub const FV: FunctionDef = FunctionDef::new("FV", Category::Financial, FV_PARAMS, fv)
    .help("Future value. Returns the future value of an investment based on regular payments and a constant interest rate.");

fn fv(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let rate = a.num(0)?;
        let nper = a.num(1)?;
        let pmt = a.num(2)?;
        let pv = opt_num(&mut a, 3, 0.0)?;
        let t = opt_num(&mut a, 4, 0.0)?;
        Ok(Value::Number(get_fv(rate, nper, pmt, pv, t)))
    })
}

const PMT_PARAMS: &[Param] = &[
    Param::required("rate", ParamKind::Number),
    Param::required("nper", ParamKind::Number),
    Param::required("pv", ParamKind::Number),
    Param::optional("fv", ParamKind::Number),
    Param::optional("type", ParamKind::Integer),
];

/// `PMT(rate; nper; pv; [fv]; [type])`.
pub const PMT: FunctionDef = FunctionDef::new("PMT", Category::Financial, PMT_PARAMS, pmt)
    .help("Regular payments. Returns the periodic payment of an annuity, based on regular payments and a fixed periodic interest rate.");

fn pmt(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let rate = a.num(0)?;
        let nper = a.num(1)?;
        let pv = a.num(2)?;
        let fv = opt_num(&mut a, 3, 0.0)?;
        let t = opt_num(&mut a, 4, 0.0)?;
        Ok(Value::Number(get_pmt(rate, nper, pv, fv, t)))
    })
}

const NPER_PARAMS: &[Param] = &[
    Param::required("rate", ParamKind::Number),
    Param::required("pmt", ParamKind::Number),
    Param::required("pv", ParamKind::Number),
    Param::optional("fv", ParamKind::Number),
    Param::optional("type", ParamKind::Integer),
];

/// `NPER(rate; pmt; pv; [fv]; [type])`.
pub const NPER: FunctionDef = FunctionDef::new("NPER", Category::Financial, NPER_PARAMS, nper)
    .help("Payment period. Calculates the number of payment periods for an investment based on regular payments and a constant interest rate.");

fn nper(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let rate = a.num(0)?;
        let pmt = a.num(1)?;
        let pv = a.num(2)?;
        let fv = opt_num(&mut a, 3, 0.0)?;
        let t = opt_num(&mut a, 4, 0.0)?;
        let n = if rate == 0.0 {
            -(pv + fv) / pmt
        } else if t > 0.0 {
            (-(rate * fv - pmt * (1.0 + rate)) / (rate * pv + pmt * (1.0 + rate))).ln()
                / (1.0 + rate).ln()
        } else {
            (-(rate * fv - pmt) / (rate * pv + pmt)).ln() / (1.0 + rate).ln()
        };
        Ok(Value::Number(n))
    })
}

const RATE_PARAMS: &[Param] = &[
    Param::required("nper", ParamKind::Number),
    Param::required("pmt", ParamKind::Number),
    Param::required("pv", ParamKind::Number),
    Param::optional("fv", ParamKind::Number),
    Param::optional("type", ParamKind::Integer),
    Param::optional("guess", ParamKind::Number),
];

/// `RATE(nper; pmt; pv; [fv]; [type]; [guess])`.
pub const RATE: FunctionDef = FunctionDef::new("RATE", Category::Financial, RATE_PARAMS, rate)
    .help("Calculates the constant interest rate of an investment with regular payments.");

fn rate(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let nper = a.num(0)?;
        let pmt = a.num(1)?;
        let pv = a.num(2)?;
        let fv = opt_num(&mut a, 3, 0.0)?;
        let t = opt_num(&mut a, 4, 0.0)?;
        let guess = opt_num(&mut a, 5, 0.1)?;
        if nper <= 0.0 {
            return Err(ErrorKind::InvalidArgument);
        }
        match rate_iteration(nper, pmt, pv, fv, t, guess) {
            (x, true) => Ok(Value::Number(x)),
            _ => Err(ErrorKind::NoConvergence),
        }
    })
}

const IPMT_PARAMS: &[Param] = &[
    Param::required("rate", ParamKind::Number),
    Param::required("period", ParamKind::Number),
    Param::required("nper", ParamKind::Number),
    Param::required("pv", ParamKind::Number),
    Param::optional("fv", ParamKind::Number),
    Param::optional("type", ParamKind::Integer),
];

/// `IPMT(rate; period; nper; pv; [fv]; [type])`.
pub const IPMT: FunctionDef = FunctionDef::new("IPMT", Category::Financial, IPMT_PARAMS, ipmt)
    .help("Compounded interest. Calculates the interest payment on the principal for an investment with regular payments and a constant interest rate for a given period.");

/// `PPMT(rate; period; nper; pv; [fv]; [type])`.
pub const PPMT: FunctionDef = FunctionDef::new("PPMT", Category::Financial, IPMT_PARAMS, ppmt)
    .help("Repayment. Calculates the repayment amount for a period for an investment whereby the payments are at regular intervals and the interest rate constant.");

fn ipmt_args(a: &mut Args<'_>) -> Result<(f64, f64), ErrorKind> {
    let rate = a.num(0)?;
    let per = a.num(1)?;
    let nper = a.num(2)?;
    let pv = a.num(3)?;
    let fv = opt_num(a, 4, 0.0)?;
    let t = opt_num(a, 5, 0.0)?;
    if per < 1.0 || per > nper {
        return Err(ErrorKind::InvalidArgument);
    }
    Ok(get_ipmt(rate, per, nper, pv, fv, t))
}

fn ipmt(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| Ok(Value::Number(ipmt_args(&mut a)?.0)))
}

fn ppmt(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let (i, p) = ipmt_args(&mut a)?;
        Ok(Value::Number(p - i))
    })
}

const CUM_PARAMS: &[Param] = &[
    Param::required("rate", ParamKind::Number),
    Param::required("nper", ParamKind::Number),
    Param::required("pv", ParamKind::Number),
    Param::required("start", ParamKind::Integer),
    Param::required("end", ParamKind::Integer),
    Param::required("type", ParamKind::Integer),
];

/// `CUMIPMT(rate; nper; pv; start; end; type)`.
pub const CUMIPMT: FunctionDef = FunctionDef::new("CUMIPMT", Category::Financial, CUM_PARAMS, cumipmt)
    .help("Cumulative compounded interest. Calculates the total amount of the interest share in a period for an investment with a constant interest rate.");

/// `CUMPRINC(rate; nper; pv; start; end; type)`.
pub const CUMPRINC: FunctionDef = FunctionDef::new("CUMPRINC", Category::Financial, CUM_PARAMS, cumprinc)
    .help("Cumulative Capital. Calculates the total amount of the repayment share in a period for an investment with constant interest rate.");

/// Validated CUMIPMT/CUMPRINC arguments: `(rate, nper, pv, start, end, type)`.
fn cum_args(a: &mut Args<'_>) -> Result<(f64, f64, f64, u64, u64, f64), ErrorKind> {
    let rate = a.num(0)?;
    let nper = a.num(1)?;
    let pv = a.num(2)?;
    let start = approx_floor(a.num(3)?);
    let end = approx_floor(a.num(4)?);
    let t = a.num(5)?;
    if start < 1.0 || end < start || rate <= 0.0 || end > nper || nper <= 0.0 || pv <= 0.0 {
        return Err(ErrorKind::InvalidArgument);
    }
    Ok((rate, nper, pv, start as u64, end as u64, t))
}

fn cumipmt(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let (rate, nper, pv, mut start, end, t) = cum_args(&mut a)?;
        let pmt = get_pmt(rate, nper, pv, 0.0, t);
        let mut sum = 0.0;
        if start == 1 {
            if t <= 0.0 {
                sum = -pv;
            }
            start += 1;
        }
        for i in start..=end {
            let i = i as f64;
            if t > 0.0 {
                sum += get_fv(rate, i - 2.0, pmt, pv, 1.0) - pmt;
            } else {
                sum += get_fv(rate, i - 1.0, pmt, pv, 0.0);
            }
        }
        Ok(Value::Number(sum * rate))
    })
}

fn cumprinc(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let (rate, nper, pv, mut start, end, t) = cum_args(&mut a)?;
        let pmt = get_pmt(rate, nper, pv, 0.0, t);
        let mut sum = 0.0;
        if start == 1 {
            sum = if t <= 0.0 { pmt + pv * rate } else { pmt };
            start += 1;
        }
        for i in start..=end {
            let i = i as f64;
            if t > 0.0 {
                sum += pmt - (get_fv(rate, i - 2.0, pmt, pv, 1.0) - pmt) * rate;
            } else {
                sum += pmt - get_fv(rate, i - 1.0, pmt, pv, 0.0) * rate;
            }
        }
        Ok(Value::Number(sum))
    })
}

const ISPMT_PARAMS: &[Param] = &[
    Param::required("rate", ParamKind::Number),
    Param::required("period", ParamKind::Number),
    Param::required("total_periods", ParamKind::Number),
    Param::required("invest", ParamKind::Number),
];

/// `ISPMT(rate; period; total_periods; invest)`.
pub const ISPMT: FunctionDef = FunctionDef::new("ISPMT", Category::Financial, ISPMT_PARAMS, ispmt)
    .help("Returns the amount of interest for constant amortization rates.");

fn ispmt(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let rate = a.num(0)?;
        let per = a.num(1)?;
        let total = a.num(2)?;
        let invest = a.num(3)?;
        Ok(Value::Number(invest * rate * (per / total - 1.0)))
    })
}

const EFFECTIVE_PARAMS: &[Param] = &[
    Param::required("nominal_rate", ParamKind::Number),
    Param::required("npery", ParamKind::Number),
];

/// `EFFECTIVE(nominal_rate; npery)` (ODF/Excel `EFFECT`).
pub const EFFECTIVE: FunctionDef = FunctionDef::new(
    "EFFECTIVE",
    Category::Financial,
    EFFECTIVE_PARAMS,
    effective,
)
.odf("EFFECT")
.help("Calculates the annual net interest rate for a nominal interest rate.");

fn effective(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let nominal = a.num(0)?;
        let periods = a.num(1)?;
        if periods < 1.0 || nominal <= 0.0 {
            return Err(ErrorKind::InvalidArgument);
        }
        let p = approx_floor(periods);
        Ok(Value::Number((1.0 + nominal / p).powf(p) - 1.0))
    })
}

const NOMINAL_PARAMS: &[Param] = &[
    Param::required("effect_rate", ParamKind::Number),
    Param::required("npery", ParamKind::Number),
];

/// `NOMINAL(effect_rate; npery)`.
pub const NOMINAL: FunctionDef =
    FunctionDef::new("NOMINAL", Category::Financial, NOMINAL_PARAMS, nominal)
        .help("Calculates the yearly nominal interest rate as an effective interest rate.");

fn nominal(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let eff = a.num(0)?;
        let periods = a.num(1)?;
        if periods < 1.0 || eff <= 0.0 {
            return Err(ErrorKind::InvalidArgument);
        }
        let p = approx_floor(periods);
        Ok(Value::Number(((eff + 1.0).powf(1.0 / p) - 1.0) * p))
    })
}

const DURATION_PARAMS: &[Param] = &[
    Param::required("rate", ParamKind::Number),
    Param::required("pv", ParamKind::Number),
    Param::required("fv", ParamKind::Number),
];

/// `DURATION(rate; pv; fv)`: number of periods to grow `pv` to `fv`
/// (ODF `PDURATION`, Excel `PDURATION`). The bond duration is `DURATION_ADD`.
pub const DURATION: FunctionDef =
    FunctionDef::new("DURATION", Category::Financial, DURATION_PARAMS, duration)
        .odf("PDURATION")
        .help("Duration. Calculates the number of periods required by an investment to attain the desired value.");

fn duration(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let rate = a.num(0)?;
        let pv = a.num(1)?;
        let fv = a.num(2)?;
        Ok(Value::Number((fv / pv).ln() / (1.0 + rate).ln()))
    })
}

const ZGZ_PARAMS: &[Param] = &[
    Param::required("periods", ParamKind::Number),
    Param::required("pv", ParamKind::Number),
    Param::required("fv", ParamKind::Number),
];

/// `ZGZ(periods; pv; fv)`: equivalent interest rate (ODF and Excel `RRI`;
/// AOO's German-derived UI name is ZGZ).
pub const ZGZ: FunctionDef = FunctionDef::new("ZGZ", Category::Financial, ZGZ_PARAMS, zgz)
    .odf("RRI")
    .help("Interest. Calculates the interest rate which represents the rate of return from an investment.");

fn zgz(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let n = a.num(0)?;
        let pv = a.num(1)?;
        let fv = a.num(2)?;
        Ok(Value::Number((fv / pv).powf(1.0 / n) - 1.0))
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(PV);
    r.register(FV);
    r.register(PMT);
    r.register(NPER);
    r.register(RATE);
    r.register(IPMT);
    r.register(PPMT);
    r.register(CUMIPMT);
    r.register(CUMPRINC);
    r.register(ISPMT);
    r.register(EFFECTIVE);
    r.register(NOMINAL);
    r.register(DURATION);
    r.register(ZGZ);
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{ErrorKind, Value};

    fn n(x: f64) -> EvalValue {
        EvalValue::number(x)
    }

    fn num(c: &mut MockEvalContext, f: &str, args: &[f64]) -> f64 {
        match call(c, f, args.iter().map(|&x| n(x)).collect()) {
            Value::Number(v) => v,
            other => panic!("{f}: {other:?}"),
        }
    }

    fn close(a: f64, b: f64, tol: f64) {
        assert!((a - b).abs() <= tol, "{a} vs {b}");
    }

    #[test]
    fn pv_fv_pmt_nper() {
        let mut c = MockEvalContext::new();
        close(
            num(&mut c, "PMT", &[0.08 / 12.0, 10.0, 10000.0]),
            -1037.03208935916,
            1e-9,
        );
        close(
            num(&mut c, "FV", &[0.06 / 12.0, 10.0, -200.0, -500.0, 1.0]),
            2581.4033740601,
            1e-9,
        );
        close(
            num(&mut c, "PV", &[0.08 / 12.0, 240.0, 500.0]),
            -59777.1458511178,
            1e-6,
        );
        close(
            num(&mut c, "NPER", &[0.01, -100.0, -1000.0, 10000.0, 1.0]),
            59.6738656742,
            1e-9,
        );
        close(num(&mut c, "PMT", &[0.0, 10.0, 1000.0]), -100.0, 1e-12);
        close(num(&mut c, "PV", &[0.0, 10.0, 100.0]), -1000.0, 1e-12);
    }

    #[test]
    fn rate_converges() {
        let mut c = MockEvalContext::new();
        close(
            num(&mut c, "RATE", &[48.0, -200.0, 8000.0]),
            0.0077014724882,
            1e-12,
        );
        assert_eq!(
            call(&mut c, "RATE", vec![n(0.0), n(-200.0), n(8000.0)]),
            Value::Error(ErrorKind::InvalidArgument)
        );
        assert_eq!(
            call(&mut c, "RATE", vec![n(10.0), n(100.0), n(100.0)]),
            Value::Error(ErrorKind::NoConvergence)
        );
    }

    #[test]
    fn ipmt_ppmt_cum() {
        let mut c = MockEvalContext::new();
        close(
            num(&mut c, "IPMT", &[0.1 / 12.0, 1.0, 36.0, 8000.0]),
            -66.6666666667,
            1e-9,
        );
        let p = num(&mut c, "PMT", &[0.1 / 12.0, 36.0, 8000.0]);
        let pp = num(&mut c, "PPMT", &[0.1 / 12.0, 1.0, 36.0, 8000.0]);
        close(pp, p + 66.6666666667, 1e-9);
        close(
            num(
                &mut c,
                "CUMIPMT",
                &[0.09 / 12.0, 360.0, 125000.0, 13.0, 24.0, 0.0],
            ),
            -11135.2321307508,
            1e-6,
        );
        close(
            num(
                &mut c,
                "CUMPRINC",
                &[0.09 / 12.0, 360.0, 125000.0, 13.0, 24.0, 0.0],
            ),
            -934.1071234,
            1e-6,
        );
        assert_eq!(
            call(&mut c, "IPMT", vec![n(0.1), n(0.0), n(3.0), n(100.0)]),
            Value::Error(ErrorKind::InvalidArgument)
        );
    }

    #[test]
    fn effect_nominal_duration_zgz() {
        let mut c = MockEvalContext::new();
        close(num(&mut c, "EFFECTIVE", &[0.0525, 4.0]), 0.0535426673, 1e-9);
        close(num(&mut c, "NOMINAL", &[0.053543, 4.0]), 0.0525003, 1e-6);
        close(
            num(&mut c, "DURATION", &[0.04, 100.0, 150.0]),
            10.3380350,
            1e-6,
        );
        close(
            num(&mut c, "ZGZ", &[96.0, 10000.0, 11000.0]),
            0.000993,
            1e-6,
        );
        close(
            num(&mut c, "ISPMT", &[0.1, 1.0, 3.0, 8000000.0]),
            -533333.333333,
            1e-5,
        );
        assert_eq!(
            call(&mut c, "EFFECTIVE", vec![n(0.1), n(0.5)]),
            Value::Error(ErrorKind::InvalidArgument)
        );
    }
}
