//! FISHER, FISHERINV, GAUSS, PHI (AOO `ScGauss`/`ScPhi`: the shared
//! [`crate::stats_util::gauss`] / [`crate::stats_util::phi`]).

use tally_types::{ErrorKind, Value};

use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry};
use crate::stats_util;
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const ONE: &[Param] = &[Param::required("number", ParamKind::Number)];

/// `FISHER(x)`.
pub const FISHER: FunctionDef = FunctionDef::new("FISHER", Category::Statistical, ONE, fisher)
    .help("Returns the Fisher transformation.");
/// `FISHERINV(y)`.
pub const FISHERINV: FunctionDef =
    FunctionDef::new("FISHERINV", Category::Statistical, ONE, fisherinv)
        .help("Returns the inverse of the Fisher transformation.");
/// `GAUSS(x)`.
pub const GAUSS: FunctionDef = FunctionDef::new("GAUSS", Category::Statistical, ONE, gauss)
    .help("Returns the standard normal cumulative distribution minus 0.5.");
/// `PHI(x)`.
pub const PHI: FunctionDef = FunctionDef::new("PHI", Category::Statistical, ONE, phi)
    .help("Returns the density of the standard normal distribution.");

fn fisher(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let x = a.num(0)?;
        if x <= -1.0 || x >= 1.0 {
            return Err(ErrorKind::InvalidArgument);
        }
        Ok(Value::Number(x.atanh()))
    })
}

fn fisherinv(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| Ok(Value::Number(a.num(0)?.tanh())))
}

fn gauss(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| Ok(Value::Number(stats_util::gauss(a.num(0)?))))
}

fn phi(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| Ok(Value::Number(stats_util::phi(a.num(0)?))))
}

pub(super) fn register(r: &mut Registry) {
    for d in [FISHER, FISHERINV, GAUSS, PHI] {
        r.register(d);
    }
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use crate::stats_util::gauss as gauss_value;
    use crate::EvalValue;
    use tally_types::{ErrorKind, Value};

    fn n(x: f64) -> EvalValue {
        EvalValue::number(x)
    }

    #[test]
    fn gauss_accuracy() {
        let cases = [
            (0.0, 0.0),
            (1.0, 0.341_344_746_068_542_9),
            (2.0, 0.477_249_868_051_820_8),
            (3.0, 0.498_650_101_968_369_9),
            (-1.0, -0.341_344_746_068_542_9),
            (5.0, 0.499999713348428),
        ];
        for (x, want) in cases {
            let got = gauss_value(x);
            assert!(
                (got - want).abs() <= 1e-15 * want.abs().max(1e-3),
                "{x}: {got} vs {want}"
            );
        }
        // Continuity across erf's rational-approximation intervals.
        for z in [0.5, 1.5, 2.5, 3.5, 4.5] {
            assert!((gauss_value(z - 1e-9) - gauss_value(z + 1e-9)).abs() < 1e-9);
        }
    }

    #[test]
    fn functions() {
        let mut c = MockEvalContext::new();
        let v = call(&mut c, "PHI", vec![n(0.0)]);
        assert!(matches!(v, Value::Number(r) if (r - 0.3989422804014327).abs() < 1e-15));
        let v = call(&mut c, "FISHER", vec![n(0.75)]);
        assert!(matches!(v, Value::Number(r) if (r - 0.9729550745276566).abs() < 1e-14));
        let v = call(&mut c, "FISHERINV", vec![n(0.9729550745276566)]);
        assert!(matches!(v, Value::Number(r) if (r - 0.75).abs() < 1e-14));
        assert_eq!(
            call(&mut c, "FISHER", vec![n(1.0)]),
            Value::Error(ErrorKind::InvalidArgument)
        );
        assert_eq!(
            call(&mut c, "GAUSS", vec![EvalValue::text("x")]),
            Value::Error(ErrorKind::Value)
        );
    }
}
