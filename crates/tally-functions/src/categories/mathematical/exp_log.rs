//! DEGREES, RADIANS, PI, EXP, LN, LOG, LOG10.
//!
//! Ported in behaviour from Apache OpenOffice `ScInterpreter::ScLn`, `ScLog`,
//! `ScLog10` (sc/source/core/tool/interpr1-3.cxx): a non-positive argument
//! (or a base that is not positive or equals 1) is `Err:502`; `EXP`
//! overflow is `#NUM!`.

use std::f64::consts::PI as PI_VALUE;

use tally_types::{ErrorKind, Value};

use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const ONE: &[Param] = &[Param::required("number", ParamKind::Number)];
const LOG_PARAMS: &[Param] = &[
    Param::required("number", ParamKind::Number),
    Param::optional("base", ParamKind::Number),
];

/// `DEGREES(number)`.
pub const DEGREES: FunctionDef = FunctionDef::new("DEGREES", Category::Mathematical, ONE, degrees)
    .help("Converts radians to degrees.");
/// `RADIANS(number)`.
pub const RADIANS: FunctionDef = FunctionDef::new("RADIANS", Category::Mathematical, ONE, radians)
    .help("Converts degrees to radians.");
/// `PI()`.
pub const PI: FunctionDef = FunctionDef::new("PI", Category::Mathematical, &[], pi)
    .help("Returns the value of the number pi.");
/// `EXP(number)`.
pub const EXP: FunctionDef = FunctionDef::new("EXP", Category::Mathematical, ONE, exp)
    .help("Returns e raised to the power of a number.");
/// `LN(number)`.
pub const LN: FunctionDef = FunctionDef::new("LN", Category::Mathematical, ONE, ln)
    .help("Returns the natural logarithm of a number.");
/// `LOG(number; [base])`.
pub const LOG: FunctionDef = FunctionDef::new("LOG", Category::Mathematical, LOG_PARAMS, log)
    .help("Returns the logarithm of a number to the specified base.");
/// `LOG10(number)`.
pub const LOG10: FunctionDef = FunctionDef::new("LOG10", Category::Mathematical, ONE, log10)
    .help("Returns the base-10 logarithm of a number.");

fn degrees(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| Ok(Value::Number(a.num(0)?.to_degrees())))
}

fn radians(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| Ok(Value::Number(a.num(0)?.to_radians())))
}

fn pi(_ctx: &mut dyn EvalContext, _args: &mut dyn ArgList) -> EvalValue {
    run(|| Ok(Value::Number(PI_VALUE)))
}

fn exp(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    // Overflow to infinity becomes #NUM! in `run`.
    run(|| Ok(Value::Number(a.num(0)?.exp())))
}

fn ln(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let x = a.num(0)?;
        if x > 0.0 {
            Ok(Value::Number(x.ln()))
        } else {
            Err(ErrorKind::InvalidArgument)
        }
    })
}

fn log10(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let x = a.num(0)?;
        if x > 0.0 {
            Ok(Value::Number(x.log10()))
        } else {
            Err(ErrorKind::InvalidArgument)
        }
    })
}

fn log(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let base = a.num_opt(1, 10.0)?;
        let x = a.num(0)?;
        if x > 0.0 && base > 0.0 && base != 1.0 {
            // log10 / log2 are exact on powers of the base; AOO's
            // ln(x)/ln(base) differs only in the last bit.
            let r = if base == 10.0 {
                x.log10()
            } else if base == 2.0 {
                x.log2()
            } else {
                x.ln() / base.ln()
            };
            Ok(Value::Number(r))
        } else {
            Err(ErrorKind::InvalidArgument)
        }
    })
}

pub(super) fn register(r: &mut Registry) {
    for d in [DEGREES, RADIANS, PI, EXP, LN, LOG, LOG10] {
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

    #[test]
    fn log_family() {
        assert_eq!(f("LOG", &[1000.0]), Value::Number(3.0));
        assert_eq!(f("LOG", &[8.0, 2.0]), Value::Number(3.0));
        assert_eq!(f("LN", &[0.0]), Value::Error(ErrorKind::InvalidArgument));
        assert_eq!(
            f("LOG10", &[-1.0]),
            Value::Error(ErrorKind::InvalidArgument)
        );
        assert_eq!(
            f("LOG", &[5.0, 1.0]),
            Value::Error(ErrorKind::InvalidArgument)
        );
        assert_eq!(
            f("LOG", &[5.0, -2.0]),
            Value::Error(ErrorKind::InvalidArgument)
        );
    }

    #[test]
    fn exp_and_angles() {
        assert_eq!(f("EXP", &[0.0]), Value::Number(1.0));
        assert_eq!(f("EXP", &[1000.0]), Value::Error(ErrorKind::Num));
        assert_eq!(f("EXP", &[-1000.0]), Value::Number(0.0));
        assert_eq!(f("DEGREES", &[std::f64::consts::PI]), Value::Number(180.0));
        assert_eq!(f("PI", &[]), Value::Number(std::f64::consts::PI));
        assert_eq!(f("RADIANS", &[180.0]), Value::Number(std::f64::consts::PI));
    }
}
