//! ABS, SIGN, MOD, POWER, SQRT.

use tally_types::{ErrorKind, Value};

use crate::coerce::approx_floor;
use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const ONE: &[Param] = &[Param::required("number", ParamKind::Number)];

/// `ABS(number)`.
pub const ABS: FunctionDef =
    FunctionDef::new("ABS", Category::Mathematical, ONE, abs).help("Absolute value of a number.");

/// `SIGN(number)`.
pub const SIGN: FunctionDef = FunctionDef::new("SIGN", Category::Mathematical, SIGN_PARAMS, sign)
    .help("Returns the algebraic sign of a number.");

/// `MOD(dividend; divisor)`.
pub const MOD: FunctionDef = FunctionDef::new("MOD", Category::Mathematical, MOD_PARAMS, modulo)
    .help("Calculates the remainder of a division.");

/// `POWER(base; exponent)`.
pub const POWER: FunctionDef =
    FunctionDef::new("POWER", Category::Mathematical, POWER_PARAMS, power)
        .help("Returns a^b, base a raised to the power of exponent b.");

/// `SQRT(number)`.
pub const SQRT: FunctionDef = FunctionDef::new("SQRT", Category::Mathematical, SQRT_PARAMS, sqrt)
    .help("Returns the square root of a number.");

const SIGN_PARAMS: &[Param] = ONE;
const SQRT_PARAMS: &[Param] = ONE;
const MOD_PARAMS: &[Param] = &[
    Param::required("dividend", ParamKind::Number),
    Param::required("divisor", ParamKind::Number),
];
const POWER_PARAMS: &[Param] = &[
    Param::required("base", ParamKind::Number),
    Param::required("exponent", ParamKind::Number),
];

fn abs(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| Ok(Value::Number(a.num(0)?.abs())))
}

fn sign(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let x = a.num(0)?;
        Ok(Value::Number(if x > 0.0 {
            1.0
        } else if x < 0.0 {
            -1.0
        } else {
            0.0
        }))
    })
}

/// Result takes the sign of the divisor. After AOO `ScInterpreter::ScMod`,
/// Apache-2.0: integral operands use the exact `fmod` (sign adjusted to
/// the divisor), so quotients beyond 15 digits stay exact
/// (`MOD(1E20;3)` is 1); otherwise `a - b * approxFloor(a / b)` with the
/// residue snapped. A zero divisor is `#DIV/0!`.
fn modulo(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let x = a.num(0)?;
        let y = a.num(1)?;
        if y == 0.0 {
            return Err(ErrorKind::Div0);
        }
        if x == x.trunc() && y == y.trunc() {
            let r = x % y;
            let r = if r == 0.0 {
                0.0 // no negative zero
            } else if (r < 0.0) != (y < 0.0) {
                r + y
            } else {
                r
            };
            return Ok(Value::Number(r));
        }
        let r = x - y * approx_floor(x / y);
        // Snap residue like approxSub: a tiny remainder relative to the
        // operands is a rounding artefact (MOD(1;0.1) is 0).
        let r = if r.abs() <= x.abs().max(y.abs()) * 1e-15 {
            0.0
        } else {
            r
        };
        Ok(Value::Number(r))
    })
}

/// `0^0` is 1 as in AOO (C `pow`); `0^negative` is `#DIV/0!`; a negative
/// base with a fractional exponent is `#NUM!` (NaN).
fn power(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let x = a.num(0)?;
        let y = a.num(1)?;
        // AOO ScPow pushes C pow() through PushDouble/TreatDoubleError
        // (interpr5.cxx, formula errorcodes.hxx GetDoubleErrorValue): an
        // infinity (overflow, 0 to a negative power) is errIllegalFPOperation
        // (#NUM!) and a NaN (negative base, fractional exponent) is
        // errNoValue (#VALUE!).
        let r = x.powf(y);
        if r.is_nan() {
            return Err(ErrorKind::Value);
        }
        if r.is_infinite() {
            return Err(ErrorKind::Num);
        }
        Ok(Value::Number(r))
    })
}

fn sqrt(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let x = a.num(0)?;
        if x < 0.0 {
            return Err(ErrorKind::InvalidArgument);
        }
        Ok(Value::Number(x.sqrt()))
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(ABS);
    r.register(SIGN);
    r.register(MOD);
    r.register(POWER);
    r.register(SQRT);
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{ErrorKind, Value};

    fn n(x: f64) -> EvalValue {
        EvalValue::number(x)
    }
    fn f(name: &str, args: Vec<EvalValue>) -> Value {
        call(&mut MockEvalContext::new(), name, args)
    }
    fn num(x: f64) -> Value {
        Value::Number(x)
    }
    fn err(e: ErrorKind) -> Value {
        Value::Error(e)
    }

    #[test]
    fn abs_sign() {
        assert_eq!(f("ABS", vec![n(-2.5)]), num(2.5));
        assert_eq!(f("ABS", vec![EvalValue::bool(true)]), num(1.0));
        assert_eq!(f("ABS", vec![EvalValue::text("x")]), err(ErrorKind::Value));
        assert_eq!(f("SIGN", vec![n(-7.0)]), num(-1.0));
        assert_eq!(f("SIGN", vec![n(0.0)]), num(0.0));
        assert_eq!(f("SIGN", vec![n(1e-300)]), num(1.0));
        assert_eq!(f("SIGN", vec![EvalValue::Missing]), num(0.0));
    }

    #[test]
    fn modulo() {
        assert_eq!(f("MOD", vec![n(5.0), n(3.0)]), num(2.0));
        assert_eq!(f("MOD", vec![n(-5.0), n(3.0)]), num(1.0));
        assert_eq!(f("MOD", vec![n(5.0), n(-3.0)]), num(-1.0));
        assert_eq!(f("MOD", vec![n(-5.0), n(-3.0)]), num(-2.0));
        assert_eq!(f("MOD", vec![n(5.0), n(0.0)]), err(ErrorKind::Div0));
        assert_eq!(f("MOD", vec![n(1.0), n(0.1)]), num(0.0));
        assert_eq!(f("MOD", vec![n(5.5), n(2.0)]), num(1.5));
        assert_eq!(f("MOD", vec![n(0.0), n(7.0)]), num(0.0));
    }

    #[test]
    fn power_sqrt() {
        assert_eq!(f("POWER", vec![n(2.0), n(10.0)]), num(1024.0));
        assert_eq!(f("POWER", vec![n(0.0), n(0.0)]), num(1.0));
        assert_eq!(f("POWER", vec![n(0.0), n(-1.0)]), err(ErrorKind::Num));
        assert_eq!(
            f("POWER", vec![n(-8.0), n(1.0 / 3.0)]),
            err(ErrorKind::Value)
        );
        assert_eq!(f("POWER", vec![n(-2.0), n(3.0)]), num(-8.0));
        assert_eq!(f("POWER", vec![n(10.0), n(400.0)]), err(ErrorKind::Num));
        assert_eq!(f("SQRT", vec![n(16.0)]), num(4.0));
        assert_eq!(f("SQRT", vec![n(-1.0)]), err(ErrorKind::InvalidArgument));
        assert_eq!(f("SQRT", vec![n(0.0)]), num(0.0));
    }
}
