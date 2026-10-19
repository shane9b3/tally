//! CONFIDENCE, EXPONDIST, WEIBULL, LOGNORMDIST, LOGINV.
//!
//! Algorithms ported from Apache OpenOffice (interpr3.cxx), Apache License 2.0.

use tally_types::{ErrorKind, Value};

use super::special::{gaussinv, integral_phi, phi};
use super::{floor_num, opt_bool, opt_num};
use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const CONFIDENCE_PARAMS: &[Param] = &[
    Param::required("alpha", ParamKind::Number),
    Param::required("stdev", ParamKind::Number),
    Param::required("size", ParamKind::Integer),
];

/// `CONFIDENCE(alpha; stdev; size)`.
pub const CONFIDENCE: FunctionDef = FunctionDef::new(
    "CONFIDENCE",
    Category::Statistical,
    CONFIDENCE_PARAMS,
    confidence,
)
.help("Returns a (1-alpha) confidence interval for a normal distribution.");

const EXPONDIST_PARAMS: &[Param] = &[
    Param::required("number", ParamKind::Number),
    Param::required("lambda", ParamKind::Number),
    Param::required("c", ParamKind::Bool),
];

/// `EXPONDIST(number; lambda; c)`.
pub const EXPONDIST: FunctionDef = FunctionDef::new(
    "EXPONDIST",
    Category::Statistical,
    EXPONDIST_PARAMS,
    expondist,
)
.help("Values of the exponential distribution.");

const WEIBULL_PARAMS: &[Param] = &[
    Param::required("number", ParamKind::Number),
    Param::required("alpha", ParamKind::Number),
    Param::required("beta", ParamKind::Number),
    Param::required("c", ParamKind::Bool),
];

/// `WEIBULL(number; alpha; beta; c)`.
pub const WEIBULL: FunctionDef =
    FunctionDef::new("WEIBULL", Category::Statistical, WEIBULL_PARAMS, weibull)
        .help("Returns the values of the Weibull distribution.");

const LOGNORMDIST_PARAMS: &[Param] = &[
    Param::required("number", ParamKind::Number),
    Param::optional("mean", ParamKind::Number),
    Param::optional("stdev", ParamKind::Number),
    Param::optional("cumulative", ParamKind::Bool),
];

/// `LOGNORMDIST(number; [mean]; [stdev]; [cumulative])`.
pub const LOGNORMDIST: FunctionDef = FunctionDef::new(
    "LOGNORMDIST",
    Category::Statistical,
    LOGNORMDIST_PARAMS,
    lognormdist,
)
.help("Values of the log normal distribution.");

const LOGINV_PARAMS: &[Param] = &[
    Param::required("number", ParamKind::Number),
    Param::required("mean", ParamKind::Number),
    Param::required("stdev", ParamKind::Number),
];

/// `LOGINV(number; mean; stdev)`.
pub const LOGINV: FunctionDef =
    FunctionDef::new("LOGINV", Category::Statistical, LOGINV_PARAMS, loginv)
        .help("Values of the inverse of the lognormal distribution.");

/// AOO `ScConfidence`: `size` floored; `stdev <= 0`, `alpha` outside
/// (0, 1) or `size < 1` is `Err:502`.
fn confidence(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let alpha = a.num(0)?;
        let sigma = a.num(1)?;
        let n = floor_num(&mut a, 2)?;
        if sigma <= 0.0 || alpha <= 0.0 || alpha >= 1.0 || n < 1.0 {
            return Err(ErrorKind::InvalidArgument);
        }
        Ok(Value::Number(
            gaussinv(1.0 - alpha / 2.0) * sigma / n.sqrt(),
        ))
    })
}

/// AOO `ScExpDist`: `lambda <= 0` is `Err:502`; negative `x` gives 0.
fn expondist(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let x = a.num(0)?;
        let lambda = a.num(1)?;
        let cumulative = a.bool(2)?;
        if lambda <= 0.0 {
            return Err(ErrorKind::InvalidArgument);
        }
        Ok(Value::Number(if !cumulative {
            if x >= 0.0 {
                lambda * (-lambda * x).exp()
            } else {
                0.0
            }
        } else if x > 0.0 {
            -(-lambda * x).exp_m1()
        } else {
            0.0
        }))
    })
}

/// AOO `ScWeibull`: `alpha <= 0`, `beta <= 0` or `x < 0` is `Err:502`.
fn weibull(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let x = a.num(0)?;
        let alpha = a.num(1)?;
        let beta = a.num(2)?;
        let cumulative = a.bool(3)?;
        if alpha <= 0.0 || beta <= 0.0 || x < 0.0 {
            return Err(ErrorKind::InvalidArgument);
        }
        Ok(Value::Number(if cumulative {
            -(-(x / beta).powf(alpha)).exp_m1()
        } else {
            alpha / beta.powf(alpha) * x.powf(alpha - 1.0) * (-(x / beta).powf(alpha)).exp()
        }))
    })
}

/// AOO `ScLogNormDist` (#i100119#): mean 0, stdev 1, cumulative by default;
/// `stdev <= 0` is `Err:502`; the CDF is 0 for `x <= 0`, the density is
/// `Err:502` there.
fn lognormdist(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let x = a.num(0)?;
        let mue = opt_num(&mut a, 1, 0.0)?;
        let sigma = opt_num(&mut a, 2, 1.0)?;
        let cumulative = opt_bool(&mut a, 3, true)?;
        if sigma <= 0.0 {
            return Err(ErrorKind::InvalidArgument);
        }
        if cumulative {
            if x <= 0.0 {
                return Ok(Value::Number(0.0));
            }
            return Ok(Value::Number(integral_phi((x.ln() - mue) / sigma)));
        }
        if x <= 0.0 {
            return Err(ErrorKind::InvalidArgument);
        }
        Ok(Value::Number(phi((x.ln() - mue) / sigma) / sigma / x))
    })
}

/// AOO `ScLogNormInv`: `p` outside (0, 1) or `stdev <= 0` is `Err:502`.
fn loginv(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let y = a.num(0)?;
        let mue = a.num(1)?;
        let sigma = a.num(2)?;
        if sigma <= 0.0 || y <= 0.0 || y >= 1.0 {
            return Err(ErrorKind::InvalidArgument);
        }
        Ok(Value::Number((mue + sigma * gaussinv(y)).exp()))
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(CONFIDENCE);
    r.register(EXPONDIST);
    r.register(WEIBULL);
    r.register(LOGNORMDIST);
    r.register(LOGINV);
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

    const BAD: Value = Value::Error(ErrorKind::InvalidArgument);
    const ONE_MINUS_INV_E: f64 = 0.632_120_558_828_557_7;

    #[test]
    fn misc() {
        let mut c = MockEvalContext::new();
        approx(
            call(&mut c, "CONFIDENCE", vec![n(0.05), n(2.5), n(50.0)]),
            0.692_951_912_174_838_9,
            1e-14,
        );
        assert_eq!(
            call(&mut c, "CONFIDENCE", vec![n(0.05), n(2.5), n(0.5)]),
            BAD
        );
        approx(
            call(
                &mut c,
                "EXPONDIST",
                vec![n(1.0), n(1.0), EvalValue::bool(true)],
            ),
            ONE_MINUS_INV_E,
            1e-15,
        );
        assert_eq!(
            call(&mut c, "EXPONDIST", vec![n(-1.0), n(1.0), n(1.0)]),
            Value::Number(0.0)
        );
        approx(
            call(
                &mut c,
                "WEIBULL",
                vec![n(1.0), n(1.0), n(1.0), EvalValue::bool(true)],
            ),
            ONE_MINUS_INV_E,
            1e-15,
        );
        assert_eq!(
            call(&mut c, "WEIBULL", vec![n(-1.0), n(1.0), n(1.0), n(1.0)]),
            BAD
        );
        approx(call(&mut c, "LOGNORMDIST", vec![n(1.0)]), 0.5, 1e-15);
        approx(
            call(&mut c, "LOGNORMDIST", vec![n(2.0), n(0.0), n(1.0), n(0.0)]),
            0.156_874_019_278_981_12,
            1e-14,
        );
        assert_eq!(
            call(&mut c, "LOGNORMDIST", vec![n(0.0), n(0.0), n(1.0), n(0.0)]),
            BAD
        );
        approx(
            call(&mut c, "LOGINV", vec![n(0.5), n(1.0), n(2.0)]),
            std::f64::consts::E,
            1e-15,
        );
        assert_eq!(call(&mut c, "LOGINV", vec![n(1.0), n(1.0), n(2.0)]), BAD);
    }
}
