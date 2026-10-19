//! CHIDIST, CHIINV, CHISQDIST, CHISQINV, FDIST, FINV, TDIST, TINV.
//!
//! Algorithms ported from Apache OpenOffice (interpr3.cxx), Apache License 2.0.

use tally_types::{ErrorKind, Value};

use super::special::{chi_dist, chi_sq_dist_cdf, chi_sq_dist_pdf, f_dist, iterate_inverse, t_dist};
use super::{floor_num, opt_bool};
use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const X_DF: &[Param] = &[
    Param::required("number", ParamKind::Number),
    Param::required("degrees_freedom", ParamKind::Number),
];

/// `CHIDIST(number; degrees_freedom)`.
pub const CHIDIST: FunctionDef = FunctionDef::new("CHIDIST", Category::Statistical, X_DF, chidist)
    .odf("LEGACY.CHIDIST")
    .help("Returns the right-tail probability of the chi-square distribution.");

/// `CHIINV(number; degrees_freedom)`.
pub const CHIINV: FunctionDef = FunctionDef::new("CHIINV", Category::Statistical, X_DF, chiinv)
    .odf("LEGACY.CHIINV")
    .help("Values of the inverse of CHIDIST(x; DegreesOfFreedom).");

const CHISQDIST_PARAMS: &[Param] = &[
    Param::required("number", ParamKind::Number),
    Param::required("degrees_freedom", ParamKind::Number),
    Param::optional("cumulative", ParamKind::Bool),
];

/// `CHISQDIST(number; degrees_freedom; [cumulative])`.
pub const CHISQDIST: FunctionDef =
    FunctionDef::new("CHISQDIST", Category::Statistical, CHISQDIST_PARAMS, chisqdist)
        .help("Returns left-tail probability of the cumulative distribution function or values of the probability density function of the chi-square distribution.");

/// `CHISQINV(probability; degrees_freedom)`.
pub const CHISQINV: FunctionDef =
    FunctionDef::new("CHISQINV", Category::Statistical, X_DF, chisqinv)
        .help("Values of the inverse of CHISQDIST(x;DegreesOfFreedom;TRUE()).");

const F_PARAMS: &[Param] = &[
    Param::required("number", ParamKind::Number),
    Param::required("degrees_freedom_1", ParamKind::Number),
    Param::required("degrees_freedom_2", ParamKind::Number),
];

/// `FDIST(number; degrees_freedom_1; degrees_freedom_2)`.
pub const FDIST: FunctionDef = FunctionDef::new("FDIST", Category::Statistical, F_PARAMS, fdist)
    .odf("LEGACY.FDIST")
    .help("Values of the F probability distribution.");

/// `FINV(number; degrees_freedom_1; degrees_freedom_2)`.
pub const FINV: FunctionDef = FunctionDef::new("FINV", Category::Statistical, F_PARAMS, finv)
    .odf("LEGACY.FINV")
    .help("Values of the inverse F distribution.");

const TDIST_PARAMS: &[Param] = &[
    Param::required("number", ParamKind::Number),
    Param::required("degrees_freedom", ParamKind::Number),
    Param::required("mode", ParamKind::Integer),
];

/// `TDIST(number; degrees_freedom; mode)`.
pub const TDIST: FunctionDef =
    FunctionDef::new("TDIST", Category::Statistical, TDIST_PARAMS, tdist)
        .help("Returns the t-distribution.");

/// `TINV(number; degrees_freedom)`.
pub const TINV: FunctionDef = FunctionDef::new("TINV", Category::Statistical, X_DF, tinv)
    .help("Values of the inverse t-distribution.");

/// Runs [`iterate_inverse`] on `p - dist(x)`, propagating the first error
/// raised by `dist` and mapping a failed bracket search to `Err:523`.
fn invert(
    p: f64,
    dist: impl Fn(f64) -> Result<f64, ErrorKind>,
    ax: f64,
    bx: f64,
) -> Result<Value, ErrorKind> {
    let mut err = None;
    let root = iterate_inverse(
        |x| match dist(x) {
            Ok(v) => p - v,
            Err(e) => {
                err.get_or_insert(e);
                p
            }
        },
        ax,
        bx,
    );
    if let Some(e) = err {
        return Err(e);
    }
    root.map(Value::Number).ok_or(ErrorKind::NoConvergence)
}

/// AOO `ScChiDist`: `df` is floored; `df < 1` is `Err:502`; `x <= 0` is 1.
fn chidist(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let x = a.num(0)?;
        let df = floor_num(&mut a, 1)?;
        if df < 1.0 {
            return Err(ErrorKind::InvalidArgument);
        }
        Ok(Value::Number(chi_dist(x, df)?))
    })
}

/// AOO `ScChiInv`: `p` outside (0, 1] or `df < 1` is `Err:502`.
fn chiinv(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let p = a.num(0)?;
        let df = floor_num(&mut a, 1)?;
        if df < 1.0 || p <= 0.0 || p > 1.0 {
            return Err(ErrorKind::InvalidArgument);
        }
        invert(p, |x| chi_dist(x, df), df * 0.5, df)
    })
}

/// AOO `ScChiSqDist`: left tail (or density); `df` floored, `df < 1` is `Err:502`.
fn chisqdist(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let x = a.num(0)?;
        let df = floor_num(&mut a, 1)?;
        let cumulative = opt_bool(&mut a, 2, true)?;
        if df < 1.0 {
            return Err(ErrorKind::InvalidArgument);
        }
        Ok(Value::Number(if cumulative {
            chi_sq_dist_cdf(x, df)?
        } else {
            chi_sq_dist_pdf(x, df)
        }))
    })
}

/// AOO `ScChiSqInv`: `p` outside [0, 1) or `df < 1` is `Err:502`.
fn chisqinv(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let p = a.num(0)?;
        let df = floor_num(&mut a, 1)?;
        if df < 1.0 || !(0.0..1.0).contains(&p) {
            return Err(ErrorKind::InvalidArgument);
        }
        invert(p, |x| chi_sq_dist_cdf(x, df), df * 0.5, df)
    })
}

fn f_args(a: &mut Args<'_>) -> Result<(f64, f64, f64), ErrorKind> {
    let x = a.num(0)?;
    let f1 = floor_num(a, 1)?;
    let f2 = floor_num(a, 2)?;
    if f1 < 1.0 || f2 < 1.0 || f1 >= 1.0e10 || f2 >= 1.0e10 {
        return Err(ErrorKind::InvalidArgument);
    }
    Ok((x, f1, f2))
}

/// AOO `ScFDist`: degrees of freedom floored and in [1, 1e10); `x < 0` is `Err:502`.
fn fdist(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let (x, f1, f2) = f_args(&mut a)?;
        if x < 0.0 {
            return Err(ErrorKind::InvalidArgument);
        }
        Ok(Value::Number(f_dist(x, f1, f2)))
    })
}

/// AOO `ScFInv`: `p` outside (0, 1] is `Err:502`.
fn finv(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let (p, f1, f2) = f_args(&mut a)?;
        if p <= 0.0 || p > 1.0 {
            return Err(ErrorKind::InvalidArgument);
        }
        invert(p, |x| Ok(f_dist(x, f1, f2)), f1 * 0.5, f1)
    })
}

/// AOO `ScTDist`: `df` and `mode` floored; `df < 1`, `x < 0` or a mode
/// other than 1 or 2 is `Err:502`.
fn tdist(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let t = a.num(0)?;
        let df = floor_num(&mut a, 1)?;
        let mode = floor_num(&mut a, 2)?;
        if df < 1.0 || t < 0.0 || (mode != 1.0 && mode != 2.0) {
            return Err(ErrorKind::InvalidArgument);
        }
        let r = t_dist(t, df);
        Ok(Value::Number(if mode == 1.0 { r } else { 2.0 * r }))
    })
}

/// AOO `ScTInv` (two-tailed): `df` floored and in [1, 1e5); `p` outside
/// (0, 1] is `Err:502`.
fn tinv(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let p = a.num(0)?;
        let df = floor_num(&mut a, 1)?;
        if !(1.0..1.0e5).contains(&df) || p <= 0.0 || p > 1.0 {
            return Err(ErrorKind::InvalidArgument);
        }
        invert(p, |x| Ok(2.0 * t_dist(x, df)), df * 0.5, df)
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(CHIDIST);
    r.register(CHIINV);
    r.register(CHISQDIST);
    r.register(CHISQINV);
    r.register(FDIST);
    r.register(FINV);
    r.register(TDIST);
    r.register(TINV);
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

    #[test]
    fn chi() {
        let mut c = MockEvalContext::new();
        approx(
            call(&mut c, "CHIDIST", vec![n(3.0), n(2.0)]),
            0.223_130_160_148_429_82,
            1e-14,
        );
        assert_eq!(
            call(&mut c, "CHIDIST", vec![n(-1.0), n(2.0)]),
            Value::Number(1.0)
        );
        assert_eq!(call(&mut c, "CHIDIST", vec![n(1.0), n(0.5)]), BAD);
        approx(
            call(&mut c, "CHIINV", vec![n(0.05), n(10.0)]),
            18.307_038_053_275_147,
            1e-13,
        );
        approx(
            call(&mut c, "CHISQDIST", vec![n(3.0), n(2.0)]),
            0.776_869_839_851_570_2,
            1e-14,
        );
        approx(
            call(
                &mut c,
                "CHISQDIST",
                vec![n(3.0), n(2.0), EvalValue::bool(false)],
            ),
            0.111_565_080_074_214_91,
            1e-14,
        );
        approx(
            call(&mut c, "CHISQINV", vec![n(0.95), n(10.0)]),
            18.307_038_053_275_147,
            1e-13,
        );
        assert_eq!(call(&mut c, "CHISQINV", vec![n(1.0), n(10.0)]), BAD);
    }

    #[test]
    fn f_and_t() {
        let mut c = MockEvalContext::new();
        approx(
            call(&mut c, "FDIST", vec![n(2.0), n(5.0), n(10.0)]),
            0.164_194_950_899_738_76,
            1e-13,
        );
        approx(
            call(&mut c, "FINV", vec![n(0.05), n(5.0), n(10.0)]),
            3.325_834_530_413_011_2,
            1e-12,
        );
        assert_eq!(call(&mut c, "FDIST", vec![n(-2.0), n(5.0), n(10.0)]), BAD);
        approx(
            call(&mut c, "TDIST", vec![n(2.0), n(10.0), n(2.0)]),
            0.073_388_034_770_740_39,
            1e-13,
        );
        approx(
            call(&mut c, "TDIST", vec![n(2.0), n(10.0), n(1.0)]),
            0.036_694_017_385_370_2,
            1e-13,
        );
        assert_eq!(call(&mut c, "TDIST", vec![n(2.0), n(10.0), n(3.0)]), BAD);
        approx(
            call(&mut c, "TINV", vec![n(0.05), n(10.0)]),
            2.228_138_851_986_275,
            1e-13,
        );
        assert_eq!(call(&mut c, "TINV", vec![n(0.0), n(10.0)]), BAD);
    }
}
