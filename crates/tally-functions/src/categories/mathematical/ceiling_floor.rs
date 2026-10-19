//! CEILING, FLOOR, EVEN, ODD.
//!
//! `CEILING`/`FLOOR` follow AOO `ScInterpreter::ScCeil` / `ScFloor`
//! (sc/source/core/tool/interpr2.cxx): significance 0 gives 0; number and
//! significance of opposite signs are `Err:502`; the optional mode only
//! matters for negative numbers (with a negative significance):
//! mode 0 rounds `CEILING` toward zero and `FLOOR` away from zero,
//! a non-zero mode reverses that (rounding by absolute value).

use tally_types::{ErrorKind, Value};

use crate::coerce::approx_floor;
use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const ONE: &[Param] = &[Param::required("number", ParamKind::Number)];
const CEIL_PARAMS: &[Param] = &[
    Param::required("number", ParamKind::Number),
    Param::required("significance", ParamKind::Number),
    Param::optional("mode", ParamKind::Bool),
];

/// `CEILING(number; significance; [mode])`.
pub const CEILING: FunctionDef =
    FunctionDef::new("CEILING", Category::Mathematical, CEIL_PARAMS, ceiling)
        .help("Rounds a number up to the nearest multiple of significance.");
/// `FLOOR(number; significance; [mode])`.
pub const FLOOR: FunctionDef =
    FunctionDef::new("FLOOR", Category::Mathematical, CEIL_PARAMS, floor)
        .help("Rounds number down to the nearest multiple of significance.");
/// `EVEN(number)`.
pub const EVEN: FunctionDef = FunctionDef::new("EVEN", Category::Mathematical, ONE, even)
    .help("Rounds a positive number up to the next even integer and a negative number down to the next even integer.");
/// `ODD(number)`.
pub const ODD: FunctionDef = FunctionDef::new("ODD", Category::Mathematical, ONE, odd)
    .help("Rounds a positive number up to the next odd integer and a negative number down to the next odd integer.");

/// AOO `rtl::math::approxCeil`.
fn approx_ceil(x: f64) -> f64 {
    -approx_floor(-x)
}

fn ceiling(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let x = a.num(0)?;
        let sig = a.num(1)?;
        let abs_mode = a.bool_opt(2, false)?;
        if sig == 0.0 {
            Ok(Value::Number(0.0))
        } else if x * sig < 0.0 {
            Err(ErrorKind::InvalidArgument)
        } else if x < 0.0 && !abs_mode {
            Ok(Value::Number(approx_floor(x / sig) * sig))
        } else {
            Ok(Value::Number(approx_ceil(x / sig) * sig))
        }
    })
}

fn floor(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let x = a.num(0)?;
        let sig = a.num(1)?;
        let abs_mode = a.bool_opt(2, false)?;
        if sig == 0.0 {
            Ok(Value::Number(0.0))
        } else if x * sig < 0.0 {
            Err(ErrorKind::InvalidArgument)
        } else if x < 0.0 && !abs_mode {
            Ok(Value::Number(approx_ceil(x / sig) * sig))
        } else {
            Ok(Value::Number(approx_floor(x / sig) * sig))
        }
    })
}

fn even(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let x = a.num(0)?;
        let r = approx_ceil(x.abs() / 2.0) * 2.0;
        Ok(Value::Number(if x < 0.0 { -r } else { r }))
    })
}

fn odd(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let x = a.num(0)?;
        let mut r = approx_ceil(x.abs());
        if r % 2.0 == 0.0 {
            r += 1.0;
        }
        Ok(Value::Number(if x < 0.0 { -r } else { r }))
    })
}

pub(super) fn register(r: &mut Registry) {
    for d in [CEILING, FLOOR, EVEN, ODD] {
        r.register(d);
    }
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{ErrorKind, Value};

    fn f(name: &str, args: &[f64]) -> Value {
        call(
            &mut MockEvalContext::new(),
            name,
            args.iter().map(|x| EvalValue::number(*x)).collect(),
        )
    }
    fn num(x: f64) -> Value {
        Value::Number(x)
    }

    #[test]
    fn ceiling_floor() {
        assert_eq!(f("CEILING", &[2.5, 1.0]), num(3.0));
        assert_eq!(f("CEILING", &[-2.5, -1.0]), num(-2.0));
        assert_eq!(f("CEILING", &[-2.5, -1.0, 1.0]), num(-3.0));
        assert_eq!(
            f("CEILING", &[-2.5, 1.0]),
            Value::Error(ErrorKind::InvalidArgument)
        );
        assert_eq!(f("CEILING", &[7.0, 0.0]), num(0.0));
        assert_eq!(f("FLOOR", &[2.5, 1.0]), num(2.0));
        assert_eq!(f("FLOOR", &[-2.5, -1.0]), num(-3.0));
        assert_eq!(f("FLOOR", &[-2.5, -1.0, 1.0]), num(-2.0));
        assert_eq!(
            f("FLOOR", &[2.5, -1.0]),
            Value::Error(ErrorKind::InvalidArgument)
        );
    }

    #[test]
    fn even_odd() {
        assert_eq!(f("EVEN", &[1.5]), num(2.0));
        assert_eq!(f("EVEN", &[-1.0]), num(-2.0));
        assert_eq!(f("EVEN", &[0.0]), num(0.0));
        assert_eq!(f("EVEN", &[2.0]), num(2.0));
        assert_eq!(f("ODD", &[1.5]), num(3.0));
        assert_eq!(f("ODD", &[-2.0]), num(-3.0));
        assert_eq!(f("ODD", &[0.0]), num(1.0));
        assert_eq!(f("ODD", &[3.0]), num(3.0));
    }
}
