//! GAMMA, GAMMALN, GAMMADIST, GAMMAINV, BETADIST, BETAINV.
//!
//! Algorithms ported from Apache OpenOffice (interpr3.cxx), Apache License 2.0.

use tally_types::{ErrorKind, Value};

use super::special::{
    beta_dist, beta_dist_pdf, gamma, gamma_dist, gamma_dist_pdf, iterate_inverse, log_gamma,
};
use super::{opt_bool, opt_num};
use crate::coerce::approx_floor;
use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const ONE_NUMBER: &[Param] = &[Param::required("number", ParamKind::Number)];

/// `GAMMA(number)`.
pub const GAMMA: FunctionDef =
    FunctionDef::new("GAMMA", Category::Statistical, ONE_NUMBER, gamma_fn)
        .help("Returns the value of the Gamma function.");

/// `GAMMALN(number)`.
pub const GAMMALN: FunctionDef =
    FunctionDef::new("GAMMALN", Category::Statistical, ONE_NUMBER, gammaln)
        .help("Returns the natural logarithm of the gamma function.");

const GAMMADIST_PARAMS: &[Param] = &[
    Param::required("number", ParamKind::Number),
    Param::required("alpha", ParamKind::Number),
    Param::required("beta", ParamKind::Number),
    Param::optional("cumulative", ParamKind::Bool),
];

/// `GAMMADIST(number; alpha; beta; [cumulative])`.
pub const GAMMADIST: FunctionDef =
    FunctionDef::new("GAMMADIST", Category::Statistical, GAMMADIST_PARAMS, gammadist)
        .help("Returns the value of the probability density function or the cumulative distribution for the Gamma distribution.");

const GAMMAINV_PARAMS: &[Param] = &[
    Param::required("number", ParamKind::Number),
    Param::required("alpha", ParamKind::Number),
    Param::required("beta", ParamKind::Number),
];

/// `GAMMAINV(number; alpha; beta)`.
pub const GAMMAINV: FunctionDef =
    FunctionDef::new("GAMMAINV", Category::Statistical, GAMMAINV_PARAMS, gammainv)
        .help("Values of the inverse gamma distribution.");

const BETADIST_PARAMS: &[Param] = &[
    Param::required("number", ParamKind::Number),
    Param::required("alpha", ParamKind::Number),
    Param::required("beta", ParamKind::Number),
    Param::optional("start", ParamKind::Number),
    Param::optional("end", ParamKind::Number),
    Param::optional("cumulative", ParamKind::Bool),
];

/// `BETADIST(number; alpha; beta; [start]; [end]; [cumulative])`.
pub const BETADIST: FunctionDef =
    FunctionDef::new("BETADIST", Category::Statistical, BETADIST_PARAMS, betadist)
        .help("Values of the beta distribution.");

const BETAINV_PARAMS: &[Param] = &[
    Param::required("number", ParamKind::Number),
    Param::required("alpha", ParamKind::Number),
    Param::required("beta", ParamKind::Number),
    Param::optional("start", ParamKind::Number),
    Param::optional("end", ParamKind::Number),
];

/// `BETAINV(number; alpha; beta; [start]; [end])`.
pub const BETAINV: FunctionDef =
    FunctionDef::new("BETAINV", Category::Statistical, BETAINV_PARAMS, betainv)
        .help("Values of the inverse beta distribution.");

/// AOO `ScGamma`: non-positive integers are `Err:502`, overflow `#NUM!`.
fn gamma_fn(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let x = a.num(0)?;
        if x <= 0.0 && x == approx_floor(x) {
            return Err(ErrorKind::InvalidArgument);
        }
        Ok(Value::Number(gamma(x)?))
    })
}

/// AOO `ScLogGamma`: `x <= 0` is `Err:502`.
fn gammaln(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let x = a.num(0)?;
        if x <= 0.0 {
            return Err(ErrorKind::InvalidArgument);
        }
        Ok(Value::Number(log_gamma(x)))
    })
}

/// AOO `ScGammaDist`: `alpha <= 0` or `beta <= 0` is `Err:502`.
fn gammadist(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let x = a.num(0)?;
        let alpha = a.num(1)?;
        let beta = a.num(2)?;
        let cumulative = opt_bool(&mut a, 3, true)?;
        if alpha <= 0.0 || beta <= 0.0 {
            return Err(ErrorKind::InvalidArgument);
        }
        Ok(Value::Number(if cumulative {
            gamma_dist(x, alpha, beta)?
        } else {
            gamma_dist_pdf(x, alpha, beta)?
        }))
    })
}

/// AOO `ScGammaInv`: `p` outside [0, 1), `alpha <= 0` or `beta <= 0` is
/// `Err:502`; `p = 0` is 0. Root search starts on `[αβ/2, αβ]`.
fn gammainv(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let p = a.num(0)?;
        let alpha = a.num(1)?;
        let beta = a.num(2)?;
        if alpha <= 0.0 || beta <= 0.0 || !(0.0..1.0).contains(&p) {
            return Err(ErrorKind::InvalidArgument);
        }
        if p == 0.0 {
            return Ok(Value::Number(0.0));
        }
        let mut err = None;
        let start = alpha * beta;
        let root = iterate_inverse(
            |x| match gamma_dist(x, alpha, beta) {
                Ok(v) => p - v,
                Err(e) => {
                    err.get_or_insert(e);
                    p
                }
            },
            start * 0.5,
            start,
        );
        if let Some(e) = err {
            return Err(e);
        }
        root.map(Value::Number).ok_or(ErrorKind::NoConvergence)
    })
}

/// AOO `ScBetaDist`: `end - start <= 0`, `alpha <= 0` or `beta <= 0` is
/// `Err:502`. Outside [start, end] the CDF is 0 or 1 and the density 0.
fn betadist(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let x = a.num(0)?;
        let alpha = a.num(1)?;
        let beta = a.num(2)?;
        let lower = opt_num(&mut a, 3, 0.0)?;
        let upper = opt_num(&mut a, 4, 1.0)?;
        let cumulative = opt_bool(&mut a, 5, true)?;
        let scale = upper - lower;
        if scale <= 0.0 || alpha <= 0.0 || beta <= 0.0 {
            return Err(ErrorKind::InvalidArgument);
        }
        if cumulative {
            if x < lower {
                return Ok(Value::Number(0.0));
            }
            if x > upper {
                return Ok(Value::Number(1.0));
            }
            Ok(Value::Number(beta_dist((x - lower) / scale, alpha, beta)))
        } else {
            if x < lower || x > upper {
                return Ok(Value::Number(0.0));
            }
            Ok(Value::Number(
                beta_dist_pdf((x - lower) / scale, alpha, beta)? / scale,
            ))
        }
    })
}

/// AOO `ScBetaInv`: `p` outside [0, 1), `start = end`, `alpha <= 0` or
/// `beta <= 0` is `Err:502`; `p = 0` is 0 (not `start`, as in AOO).
fn betainv(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let p = a.num(0)?;
        let alpha = a.num(1)?;
        let beta = a.num(2)?;
        let lower = opt_num(&mut a, 3, 0.0)?;
        let upper = opt_num(&mut a, 4, 1.0)?;
        if !(0.0..1.0).contains(&p) || lower == upper || alpha <= 0.0 || beta <= 0.0 {
            return Err(ErrorKind::InvalidArgument);
        }
        if p == 0.0 {
            return Ok(Value::Number(0.0));
        }
        // 0..1 as the start interval so it is not extended beyond the valid range.
        let root = iterate_inverse(|x| p - beta_dist(x, alpha, beta), 0.0, 1.0)
            .ok_or(ErrorKind::NoConvergence)?;
        Ok(Value::Number(lower + root * (upper - lower)))
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(GAMMA);
    r.register(GAMMALN);
    r.register(GAMMADIST);
    r.register(GAMMAINV);
    r.register(BETADIST);
    r.register(BETAINV);
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{ErrorKind, Value};

    fn n(x: f64) -> EvalValue {
        EvalValue::number(x)
    }

    fn approx(v: Value, want: f64, rel: f64) {
        match v {
            Value::Number(x) => assert!(
                (x - want).abs() <= rel * want.abs(),
                "got {x:e}, want {want:e}"
            ),
            other => panic!("expected {want}, got {other:?}"),
        }
    }

    #[test]
    fn gamma_functions() {
        let mut c = MockEvalContext::new();
        assert_eq!(call(&mut c, "GAMMA", vec![n(6.0)]), Value::Number(120.0));
        approx(
            call(&mut c, "GAMMA", vec![n(-0.5)]),
            -3.544_907_701_811_032,
            1e-14,
        );
        assert_eq!(
            call(&mut c, "GAMMA", vec![n(-2.0)]),
            Value::Error(ErrorKind::InvalidArgument)
        );
        assert_eq!(
            call(&mut c, "GAMMA", vec![n(172.0)]),
            Value::Error(ErrorKind::Num)
        );
        approx(
            call(&mut c, "GAMMALN", vec![n(10.0)]),
            12.801_827_480_081_469,
            1e-15,
        );
        assert_eq!(
            call(&mut c, "GAMMALN", vec![n(0.0)]),
            Value::Error(ErrorKind::InvalidArgument)
        );
    }

    #[test]
    fn gamma_dist() {
        let mut c = MockEvalContext::new();
        approx(
            call(
                &mut c,
                "GAMMADIST",
                vec![n(2.0), n(2.0), n(1.0), EvalValue::bool(true)],
            ),
            0.593_994_150_290_161_6,
            1e-14,
        );
        approx(
            call(&mut c, "GAMMADIST", vec![n(2.0), n(2.0), n(1.0), n(0.0)]),
            0.270_670_566_473_225_4,
            1e-14,
        );
        approx(
            call(&mut c, "GAMMAINV", vec![n(0.5), n(2.0), n(1.0)]),
            1.678_346_990_016_661_2,
            1e-12,
        );
        assert_eq!(
            call(&mut c, "GAMMAINV", vec![n(1.0), n(2.0), n(1.0)]),
            Value::Error(ErrorKind::InvalidArgument)
        );
    }

    #[test]
    fn beta_dist() {
        let mut c = MockEvalContext::new();
        approx(
            call(&mut c, "BETADIST", vec![n(0.5), n(2.0), n(3.0)]),
            0.6875,
            1e-15,
        );
        approx(
            call(
                &mut c,
                "BETADIST",
                vec![
                    n(0.5),
                    n(2.0),
                    n(3.0),
                    n(0.0),
                    n(1.0),
                    EvalValue::bool(false),
                ],
            ),
            1.5,
            1e-15,
        );
        approx(
            call(&mut c, "BETAINV", vec![n(0.5), n(2.0), n(3.0)]),
            0.385_727_568_132_389_5,
            1e-12,
        );
        approx(
            call(
                &mut c,
                "BETAINV",
                vec![n(0.6875), n(2.0), n(3.0), n(1.0), n(3.0)],
            ),
            2.0,
            1e-12,
        );
        assert_eq!(
            call(&mut c, "BETADIST", vec![n(0.5), n(0.0), n(3.0)]),
            Value::Error(ErrorKind::InvalidArgument)
        );
    }
}
