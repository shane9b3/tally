//! IMSIN, IMCOS, IMTAN, IMCOT, IMSEC, IMCSC, IMSINH, IMCOSH, IMSECH, IMCSCH.
//!
//! Ported from Apache OpenOffice `scaddins/source/analysis/analysishelper.cxx`
//! (class `Complex`; Apache-2.0).

use tally_types::ErrorKind;

use super::cplx::{Cx, BAD};
use crate::registry::Registry;

/// `rtl::math::isValidArcArg`: AOO rejects trigonometric arguments beyond 2^27.
fn check_arc(x: f64) -> Result<(), ErrorKind> {
    if x.abs() <= 134_217_728.0 {
        Ok(())
    } else {
        Err(BAD)
    }
}

fn with(z: &Cx, r: f64, i: f64) -> Cx {
    Cx {
        r,
        i,
        suffix: z.suffix,
    }
}

fn sin(z: &Cx) -> Result<Cx, ErrorKind> {
    check_arc(z.r)?;
    Ok(with(z, z.r.sin() * z.i.cosh(), z.r.cos() * z.i.sinh()))
}

fn cos(z: &Cx) -> Result<Cx, ErrorKind> {
    check_arc(z.r)?;
    Ok(with(z, z.r.cos() * z.i.cosh(), -(z.r.sin() * z.i.sinh())))
}

fn tan(z: &Cx) -> Result<Cx, ErrorKind> {
    check_arc(z.r)?;
    let scale = 1.0 / (z.r.cos() * z.r.cos() + z.i.sinh() * z.i.sinh());
    Ok(with(
        z,
        z.r.sin() * z.r.cos() * scale,
        z.i.sinh() * z.i.cosh() * scale,
    ))
}

fn cot(z: &Cx) -> Result<Cx, ErrorKind> {
    check_arc(z.r)?;
    let den = z.r.sin() * z.r.sin() + z.i.sinh() * z.i.sinh();
    if den == 0.0 {
        return Err(BAD);
    }
    let scale = 1.0 / den;
    Ok(with(
        z,
        z.r.sin() * z.r.cos() * scale,
        -(z.i.sinh() * z.i.cosh() * scale),
    ))
}

fn sec(z: &Cx) -> Result<Cx, ErrorKind> {
    cos(z)?.recip()
}

fn csc(z: &Cx) -> Result<Cx, ErrorKind> {
    sin(z)?.recip()
}

fn sinh(z: &Cx) -> Result<Cx, ErrorKind> {
    Ok(with(z, z.r.sinh() * z.i.cos(), z.r.cosh() * z.i.sin()))
}

fn cosh(z: &Cx) -> Result<Cx, ErrorKind> {
    Ok(with(z, z.r.cosh() * z.i.cos(), z.r.sinh() * z.i.sin()))
}

fn sech(z: &Cx) -> Result<Cx, ErrorKind> {
    cosh(z)?.recip()
}

fn csch(z: &Cx) -> Result<Cx, ErrorKind> {
    sinh(z)?.recip()
}

unary_complex!(IMSIN, "IMSIN", "Returns the sine of a complex number", sin);
unary_complex!(
    IMCOS,
    "IMCOS",
    "Returns the cosine of a complex number",
    cos
);
unary_complex!(
    IMTAN,
    "IMTAN",
    "Returns the tangent of a complex number",
    tan
);
unary_complex!(
    IMCOT,
    "IMCOT",
    "Returns the cotangent of a complex number",
    cot
);
unary_complex!(
    IMSEC,
    "IMSEC",
    "Returns the secant of a complex number",
    sec
);
unary_complex!(
    IMCSC,
    "IMCSC",
    "Returns the cosecant of a complex number",
    csc
);
unary_complex!(
    IMSINH,
    "IMSINH",
    "Returns the hyperbolic sine of a complex number",
    sinh
);
unary_complex!(
    IMCOSH,
    "IMCOSH",
    "Returns the hyperbolic cosine of a complex number",
    cosh
);
unary_complex!(
    IMSECH,
    "IMSECH",
    "Returns the hyperbolic secant of a complex number",
    sech
);
unary_complex!(
    IMCSCH,
    "IMCSCH",
    "Returns the hyperbolic cosecant of a complex number",
    csch
);

pub(super) fn register(r: &mut Registry) {
    r.register(IMSIN);
    r.register(IMCOS);
    r.register(IMTAN);
    r.register(IMCOT);
    r.register(IMSEC);
    r.register(IMCSC);
    r.register(IMSINH);
    r.register(IMCOSH);
    r.register(IMSECH);
    r.register(IMCSCH);
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

    // Reference values computed with Python's cmath.
    #[test]
    fn values() {
        let cases = [
            ("IMSIN", "1+i", "1.29845758141598+0.634963914784736i"),
            ("IMCOS", "1+i", "0.833730025131149-0.988897705762865i"),
            ("IMTAN", "1+i", "0.271752585319512+1.08392332733869i"),
            ("IMCOT", "1+i", "0.217621561854403-0.868014142895925i"),
            ("IMSEC", "1+i", "0.498337030555187+0.591083841721045i"),
            ("IMCSC", "1+i", "0.621518017170428-0.303931001628426i"),
            ("IMSINH", "1+i", "0.634963914784736+1.29845758141598i"),
            ("IMCOSH", "1+i", "0.833730025131149+0.988897705762865i"),
            ("IMSECH", "1+i", "0.498337030555187-0.591083841721045i"),
            ("IMCSCH", "1+i", "0.303931001628426-0.621518017170428i"),
        ];
        for (f, arg, want) in cases {
            assert_eq!(call_text(f, vec![t(arg)]), text(want), "{f}");
        }
        assert_eq!(call_text("IMCOS", vec![t("0")]), text("1"));
        assert_eq!(call_text("IMSIN", vec![t("0")]), text("0"));
        assert_eq!(
            call_text("IMSIN", vec![t("1e9")]),
            Value::Error(ErrorKind::InvalidArgument)
        );
        assert_eq!(
            call_text("IMCSC", vec![t("0")]),
            Value::Error(ErrorKind::InvalidArgument)
        );
    }
}
