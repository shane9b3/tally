//! Trigonometric and hyperbolic functions (and their inverses).
//!
//! Domain handling follows AOO `ScInterpreter::PushDouble` / `GetDoubleErrorValue`
//! (sc/source/core/tool/interpr*.cxx): a NaN result (`ACOS(2)`, `ACOSH(0)`)
//! is `Err:502`; an infinite result (`COSH(1000)`, `ATANH(1)`) is `#NUM!`.
//! The reciprocal functions (`COT`, `CSC`, `COTH`, `CSCH`) divide by an exact
//! zero only at `x = 0` and report `#DIV/0!` there (Excel and LibreOffice agree).

use std::f64::consts::FRAC_PI_2;

use tally_types::{ErrorKind, Value};

use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const ONE: &[Param] = &[Param::required("number", ParamKind::Number)];
const ATAN2_PARAMS: &[Param] = &[
    Param::required("number_x", ParamKind::Number),
    Param::required("number_y", ParamKind::Number),
];

/// Maps a raw floating-point result to AOO's error classes.
fn chk(x: f64) -> Result<f64, ErrorKind> {
    if x.is_nan() {
        Err(ErrorKind::InvalidArgument)
    } else if x.is_infinite() {
        Err(ErrorKind::Num)
    } else {
        Ok(x)
    }
}

/// `1 / d`, `#DIV/0!` when `d` is exactly zero.
fn recip(d: f64) -> Result<f64, ErrorKind> {
    if d == 0.0 {
        Err(ErrorKind::Div0)
    } else {
        chk(1.0 / d)
    }
}

macro_rules! unary {
    ($def:ident, $func:ident, $name:literal, $help:literal, |$x:ident| $body:expr) => {
        #[doc = concat!("`", $name, "(number)`.")]
        pub const $def: FunctionDef =
            FunctionDef::new($name, Category::Mathematical, ONE, $func).help($help);

        fn $func(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
            let mut a = Args::new(ctx, args);
            run(|| {
                let $x = a.num(0)?;
                let r: Result<f64, ErrorKind> = $body;
                Ok(Value::Number(r?))
            })
        }
    };
}

unary!(
    ACOS,
    acos,
    "ACOS",
    "Returns the inverse cosine of a number.",
    |x| chk(x.acos())
);
unary!(
    ASIN,
    asin,
    "ASIN",
    "Returns the inverse sine of a number.",
    |x| chk(x.asin())
);
unary!(
    ACOSH,
    acosh,
    "ACOSH",
    "Returns the inverse hyperbolic cosine of a number.",
    |x| chk(x.acosh())
);
unary!(
    ASINH,
    asinh,
    "ASINH",
    "Returns the inverse hyperbolic sine of a number.",
    |x| chk(x.asinh())
);
unary!(
    ACOT,
    acot,
    "ACOT",
    "Returns the inverse cotangent of the given number.",
    |x| chk(FRAC_PI_2 - x.atan())
);
unary!(
    ATAN,
    atan,
    "ATAN",
    "Returns the inverse tangent of a number.",
    |x| chk(x.atan())
);
unary!(
    ACOTH,
    acoth,
    "ACOTH",
    "Returns the inverse hyperbolic cotangent of the given number.",
    |x| chk(0.5 * ((x + 1.0) / (x - 1.0)).ln())
);
unary!(
    ATANH,
    atanh,
    "ATANH",
    "Returns the inverse hyperbolic tangent of a number.",
    |x| chk(0.5 * (2.0 * x / (1.0 - x)).ln_1p())
);
unary!(
    COS,
    cos,
    "COS",
    "Returns the cosine of the given angle (in radians).",
    |x| chk(x.cos())
);
unary!(
    SIN,
    sin,
    "SIN",
    "Returns the sine of the given angle (in radians).",
    |x| chk(x.sin())
);
unary!(
    TAN,
    tan,
    "TAN",
    "Returns the tangent of the given angle (in radians).",
    |x| chk(x.tan())
);
unary!(
    COT,
    cot,
    "COT",
    "Returns the cotangent of the given angle (in radians).",
    |x| recip(x.tan())
);
unary!(
    COSH,
    cosh,
    "COSH",
    "Returns the hyperbolic cosine of a number.",
    |x| chk(x.cosh())
);
unary!(
    SINH,
    sinh,
    "SINH",
    "Returns the hyperbolic sine of a number.",
    |x| chk(x.sinh())
);
unary!(
    TANH,
    tanh,
    "TANH",
    "Returns the hyperbolic tangent of a number.",
    |x| chk(x.tanh())
);
unary!(
    COTH,
    coth,
    "COTH",
    "Returns the hyperbolic cotangent of a number.",
    |x| recip(x.tanh())
);
unary!(
    CSC,
    csc,
    "CSC",
    "Returns the cosecant of the given angle (in radians).",
    |x| recip(x.sin())
);
unary!(
    SEC,
    sec,
    "SEC",
    "Returns the secant of the given angle (in radians).",
    |x| recip(x.cos())
);
unary!(
    CSCH,
    csch,
    "CSCH",
    "Returns the hyperbolic cosecant of the given number.",
    |x| recip(x.sinh())
);
unary!(
    SECH,
    sech,
    "SECH",
    "Returns the hyperbolic secant of the given number.",
    |x| recip(x.cosh())
);

/// `ATAN2(number_x; number_y)`: the angle of the point (x, y); `ATAN2(0; 0)` is `#DIV/0!`.
pub const ATAN2: FunctionDef =
    FunctionDef::new("ATAN2", Category::Mathematical, ATAN2_PARAMS, atan2)
        .help("Returns the inverse tangent of the specified x and y coordinates.");

fn atan2(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let x = a.num(0)?;
        let y = a.num(1)?;
        if x == 0.0 && y == 0.0 {
            return Err(ErrorKind::Div0);
        }
        Ok(Value::Number(y.atan2(x)))
    })
}

pub(super) fn register(r: &mut Registry) {
    for d in [
        ACOS, ASIN, ACOSH, ASINH, ACOT, ATAN, ACOTH, ATANH, COS, SIN, TAN, COT, COSH, SINH, TANH,
        COTH, CSC, SEC, CSCH, SECH, ATAN2,
    ] {
        r.register(d);
    }
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{ErrorKind, Value};

    fn n(name: &str, x: f64) -> Value {
        call(
            &mut MockEvalContext::new(),
            name,
            vec![EvalValue::number(x)],
        )
    }
    fn close(v: Value, want: f64) -> bool {
        matches!(v, Value::Number(x) if (x - want).abs() < 1e-12)
    }

    #[test]
    fn values() {
        assert!(close(n("SIN", 0.5), 0.5_f64.sin()));
        assert!(close(n("COS", 0.0), 1.0));
        assert!(close(n("ACOT", 0.0), std::f64::consts::FRAC_PI_2));
        assert!(close(n("ACOTH", 2.0), 0.5 * 3.0_f64.ln()));
        assert!(close(n("ATANH", 0.5), 0.5 * 3.0_f64.ln()));
        assert!(close(n("SECH", 0.0), 1.0));
    }

    #[test]
    fn domain_errors() {
        assert_eq!(n("ACOS", 2.0), Value::Error(ErrorKind::InvalidArgument));
        assert_eq!(n("ACOSH", 0.5), Value::Error(ErrorKind::InvalidArgument));
        assert_eq!(n("ATANH", 1.0), Value::Error(ErrorKind::Num));
        assert_eq!(n("ATANH", 2.0), Value::Error(ErrorKind::InvalidArgument));
        assert_eq!(n("COSH", 1000.0), Value::Error(ErrorKind::Num));
        assert_eq!(n("COT", 0.0), Value::Error(ErrorKind::Div0));
        assert_eq!(n("CSCH", 0.0), Value::Error(ErrorKind::Div0));
    }

    #[test]
    fn atan2_args() {
        let mut c = MockEvalContext::new();
        let v = call(
            &mut c,
            "ATAN2",
            vec![EvalValue::number(1.0), EvalValue::number(1.0)],
        );
        assert!(close(v, std::f64::consts::FRAC_PI_4));
        let z = call(
            &mut c,
            "ATAN2",
            vec![EvalValue::number(0.0), EvalValue::number(0.0)],
        );
        assert_eq!(z, Value::Error(ErrorKind::Div0));
    }
}
