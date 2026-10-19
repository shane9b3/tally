//! NORMDIST, NORMINV, NORMSDIST, NORMSINV.
//!
//! Algorithms ported from Apache OpenOffice (interpr3.cxx), Apache License 2.0.

use tally_types::{ErrorKind, Value};

use super::opt_bool;
use super::special::{gaussinv, integral_phi, phi};
use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const NORMDIST_PARAMS: &[Param] = &[
    Param::required("number", ParamKind::Number),
    Param::required("mean", ParamKind::Number),
    Param::required("stdev", ParamKind::Number),
    Param::optional("cumulative", ParamKind::Bool),
];

/// `NORMDIST(number; mean; stdev; [cumulative])`.
pub const NORMDIST: FunctionDef =
    FunctionDef::new("NORMDIST", Category::Statistical, NORMDIST_PARAMS, normdist)
        .help("Values of a normal distribution.");

const NORMINV_PARAMS: &[Param] = &[
    Param::required("number", ParamKind::Number),
    Param::required("mean", ParamKind::Number),
    Param::required("stdev", ParamKind::Number),
];

/// `NORMINV(number; mean; stdev)`.
pub const NORMINV: FunctionDef =
    FunctionDef::new("NORMINV", Category::Statistical, NORMINV_PARAMS, norminv)
        .help("Values of the inverse normal distribution.");

const ONE_NUMBER: &[Param] = &[Param::required("number", ParamKind::Number)];

/// `NORMSDIST(number)`.
pub const NORMSDIST: FunctionDef =
    FunctionDef::new("NORMSDIST", Category::Statistical, ONE_NUMBER, normsdist)
        .odf("LEGACY.NORMSDIST")
        .help("Values of the standard normal cumulative distribution.");

/// `NORMSINV(number)`.
pub const NORMSINV: FunctionDef =
    FunctionDef::new("NORMSINV", Category::Statistical, ONE_NUMBER, normsinv)
        .odf("LEGACY.NORMSINV")
        .help("Values of the inverse standard normal distribution.");

/// AOO `ScNormDist`: `stdev <= 0` is `Err:502`; cumulative by default.
fn normdist(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let x = a.num(0)?;
        let mue = a.num(1)?;
        let sigma = a.num(2)?;
        let cumulative = opt_bool(&mut a, 3, true)?;
        if sigma <= 0.0 {
            return Err(ErrorKind::InvalidArgument);
        }
        let z = (x - mue) / sigma;
        Ok(Value::Number(if cumulative {
            integral_phi(z)
        } else {
            phi(z) / sigma
        }))
    })
}

/// AOO `ScNormInv`: `p` outside [0, 1] or `stdev <= 0` is `Err:502`;
/// `p` of exactly 0 or 1 is `#VALUE!`.
fn norminv(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let p = a.num(0)?;
        let mue = a.num(1)?;
        let sigma = a.num(2)?;
        if sigma <= 0.0 || !(0.0..=1.0).contains(&p) {
            return Err(ErrorKind::InvalidArgument);
        }
        if p == 0.0 || p == 1.0 {
            return Err(ErrorKind::Value);
        }
        Ok(Value::Number(gaussinv(p) * sigma + mue))
    })
}

fn normsdist(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| Ok(Value::Number(integral_phi(a.num(0)?))))
}

/// AOO `ScSNormInv`: same domain rules as NORMINV.
fn normsinv(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let p = a.num(0)?;
        if !(0.0..=1.0).contains(&p) {
            return Err(ErrorKind::InvalidArgument);
        }
        if p == 0.0 || p == 1.0 {
            return Err(ErrorKind::Value);
        }
        Ok(Value::Number(gaussinv(p)))
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(NORMDIST);
    r.register(NORMINV);
    r.register(NORMSDIST);
    r.register(NORMSINV);
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
    fn normdist() {
        let mut c = MockEvalContext::new();
        approx(
            call(
                &mut c,
                "NORMDIST",
                vec![n(1.0), n(0.0), n(1.0), EvalValue::bool(true)],
            ),
            0.841_344_746_068_542_9,
            1e-15,
        );
        approx(
            call(&mut c, "NORMDIST", vec![n(1.0), n(0.0), n(1.0)]),
            0.841_344_746_068_542_9,
            1e-15,
        );
        approx(
            call(&mut c, "NORMDIST", vec![n(1.0), n(0.0), n(2.0), n(0.0)]),
            0.176_032_663_382_149_76,
            1e-15,
        );
        approx(
            call(&mut c, "NORMDIST", vec![n(-10.0), n(0.0), n(1.0)]),
            7.619_853_024_160_474e-24,
            1e-12,
        );
        assert_eq!(
            call(&mut c, "NORMDIST", vec![n(1.0), n(0.0), n(0.0)]),
            Value::Error(ErrorKind::InvalidArgument)
        );
        assert_eq!(
            call(
                &mut c,
                "NORMDIST",
                vec![EvalValue::text("x"), n(0.0), n(1.0)]
            ),
            Value::Error(ErrorKind::Value)
        );
    }

    #[test]
    fn inverses() {
        let mut c = MockEvalContext::new();
        approx(
            call(&mut c, "NORMSINV", vec![n(0.975)]),
            1.959_963_984_540_054,
            1e-15,
        );
        approx(
            call(&mut c, "NORMINV", vec![n(0.975), n(10.0), n(2.0)]),
            13.919_927_969_080_108,
            1e-15,
        );
        approx(
            call(&mut c, "NORMSDIST", vec![n(1.96)]),
            0.975_002_104_851_779_5,
            1e-15,
        );
        assert_eq!(
            call(&mut c, "NORMSINV", vec![n(1.0)]),
            Value::Error(ErrorKind::Value)
        );
        assert_eq!(
            call(&mut c, "NORMSINV", vec![n(1.5)]),
            Value::Error(ErrorKind::InvalidArgument)
        );
        assert_eq!(
            call(&mut c, "NORMINV", vec![n(0.5), n(0.0), n(-1.0)]),
            Value::Error(ErrorKind::InvalidArgument)
        );
    }
}
