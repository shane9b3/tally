//! IMPOWER, IMSQRT, IMEXP, IMLN, IMLOG2, IMLOG10.
//!
//! Ported from Apache OpenOffice `scaddins/source/analysis/analysishelper.cxx`
//! (class `Complex`; Apache-2.0).

use tally_types::{ErrorKind, Value};

use super::cplx::{self, Cx, BAD};
use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry, ReturnKind};
use crate::{run, ArgList, Args, EvalContext};

fn with(z: &Cx, r: f64, i: f64) -> Cx {
    Cx {
        r,
        i,
        suffix: z.suffix,
    }
}

fn power(z: &Cx, p: f64) -> Result<Cx, ErrorKind> {
    if z.r == 0.0 && z.i == 0.0 {
        return if p > 0.0 {
            Ok(with(z, 0.0, 0.0))
        } else {
            Err(BAD)
        };
    }
    // Whole powers by repeated multiplication: exact for results such as
    // (2+3i)^3 = -46+9i, which the polar form turns into 9.00000000000001.
    if p.fract() == 0.0 && p.abs() <= 1024.0 {
        let mut acc = Cx::real(1.0);
        let mut base = *z;
        let mut n = p.abs() as u32;
        while n > 0 {
            if n & 1 == 1 {
                acc = acc.mul(&base)?;
            }
            base = base.mul(&base)?;
            n >>= 1;
        }
        if p < 0.0 {
            acc = Cx::real(1.0).div(&acc)?;
        }
        return Ok(with(z, acc.r, acc.i));
    }
    let phi = z.arg()? * p;
    let rad = z.abs().powf(p);
    Ok(with(z, rad * phi.cos(), rad * phi.sin()))
}

fn sqrt(z: &Cx) -> Result<Cx, ErrorKind> {
    let p = z.abs();
    let im = (0.5 * (p - z.r)).sqrt();
    let re = (0.5 * (p + z.r)).sqrt();
    Ok(with(z, re, if z.i < 0.0 { -im } else { im }))
}

fn exp(z: &Cx) -> Result<Cx, ErrorKind> {
    let e = z.r.exp();
    Ok(with(z, e * z.i.cos(), e * z.i.sin()))
}

fn ln(z: &Cx) -> Result<Cx, ErrorKind> {
    if z.r == 0.0 && z.i == 0.0 {
        return Err(BAD);
    }
    Ok(with(z, z.abs().ln(), z.arg()?))
}

fn log10(z: &Cx) -> Result<Cx, ErrorKind> {
    let l = ln(z)?;
    Ok(with(
        z,
        l.r * std::f64::consts::LOG10_E,
        l.i * std::f64::consts::LOG10_E,
    ))
}

fn log2(z: &Cx) -> Result<Cx, ErrorKind> {
    let l = ln(z)?;
    Ok(with(
        z,
        l.r * std::f64::consts::LOG2_E,
        l.i * std::f64::consts::LOG2_E,
    ))
}

const POWER_PARAMS: &[Param] = &[
    Param::required("complex_number", ParamKind::Text),
    Param::required("power", ParamKind::Number),
];

/// `IMPOWER(z; n)`.
pub const IMPOWER: FunctionDef = FunctionDef::new(
    "IMPOWER",
    Category::AddIn,
    POWER_PARAMS,
    |ctx: &mut dyn EvalContext, args: &mut dyn ArgList| {
        let mut a = Args::new(ctx, args);
        run(|| {
            let z = cplx::arg(&mut a, 0)?;
            let p = a.num(1)?;
            Ok(Value::text(power(&z, p)?.format()?))
        })
    },
)
.returns(ReturnKind::Text)
.help("Returns a complex number raised to a power");

unary_complex!(
    IMSQRT,
    "IMSQRT",
    "Returns the square root of a complex number",
    sqrt
);
unary_complex!(
    IMEXP,
    "IMEXP",
    "Returns the algebraic form of the exponential of a complex number",
    exp
);
unary_complex!(
    IMLN,
    "IMLN",
    "Returns the natural logarithm of a complex number",
    ln
);
unary_complex!(
    IMLOG2,
    "IMLOG2",
    "Returns the base-2 logarithm of a complex number",
    log2
);
unary_complex!(
    IMLOG10,
    "IMLOG10",
    "Returns the base-10 logarithm of a complex number",
    log10
);

pub(super) fn register(r: &mut Registry) {
    r.register(IMPOWER);
    r.register(IMSQRT);
    r.register(IMEXP);
    r.register(IMLN);
    r.register(IMLOG2);
    r.register(IMLOG10);
}

#[cfg(test)]
mod tests {
    use super::super::cplx::call_text;
    use crate::EvalValue;
    use tally_types::{ErrorKind, Value};

    fn t(s: &str) -> EvalValue {
        EvalValue::text(s)
    }
    fn text(s: &str) -> Value {
        Value::text(s)
    }
    const BAD: Value = Value::Error(ErrorKind::InvalidArgument);

    #[test]
    fn values() {
        assert_eq!(
            call_text("IMSQRT", vec![t("1+i")]),
            text("1.09868411346781+0.455089860562227i")
        );
        assert_eq!(
            call_text("IMEXP", vec![t("1+i")]),
            text("1.46869393991589+2.28735528717884i")
        );
        assert_eq!(
            call_text("IMLN", vec![t("3+4i")]),
            text("1.6094379124341+0.927295218001612i")
        );
        assert_eq!(
            call_text("IMPOWER", vec![t("2+3i"), EvalValue::number(3.0)]),
            text("-46+9i")
        );
        assert_eq!(call_text("IMSQRT", vec![t("-4")]), text("2i"));
        assert_eq!(call_text("IMLOG10", vec![t("100")]), text("2"));
        assert_eq!(call_text("IMLOG2", vec![t("8")]), text("3"));
        assert_eq!(call_text("IMEXP", vec![t("0")]), text("1"));
        assert_eq!(call_text("IMLN", vec![t("0")]), BAD);
        assert_eq!(
            call_text("IMPOWER", vec![t("0"), EvalValue::number(-1.0)]),
            BAD
        );
        assert_eq!(
            call_text("IMPOWER", vec![t("0"), EvalValue::number(2.0)]),
            text("0")
        );
        assert_eq!(
            call_text("IMSQRT", vec![t("1+j")]),
            text("1.09868411346781+0.455089860562227j")
        );
    }
}
