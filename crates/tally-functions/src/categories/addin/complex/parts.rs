//! COMPLEX, IMABS, IMAGINARY, IMARGUMENT, IMCONJUGATE, IMREAL.
//!
//! Ported from Apache OpenOffice `scaddins/source/analysis/analysishelper.cxx`
//! and `analysis.cxx` (Apache-2.0).

use tally_types::{ErrorKind, Value};

use super::cplx::{self, Cx, BAD};
use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry, ReturnKind};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const COMPLEX_PARAMS: &[Param] = &[
    Param::required("real_num", ParamKind::Number),
    Param::required("i_num", ParamKind::Number),
    Param::optional("suffix", ParamKind::Text),
];

/// `COMPLEX(real; imaginary; suffix)`.
pub const COMPLEX: FunctionDef =
    FunctionDef::new("COMPLEX", Category::AddIn, COMPLEX_PARAMS, complex)
        .returns(ReturnKind::Text)
        .help("Converts real and imaginary coefficients into a complex number");

fn complex(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let re = a.num(0)?;
        let im = a.num(1)?;
        let suffix = match &*a.text_opt(2, "i")? {
            "" | "i" => 'i',
            "j" => 'j',
            _ => return Err(BAD),
        };
        let z = Cx {
            r: re,
            i: im,
            suffix: Some(suffix),
        };
        Ok(Value::text(z.format()?))
    })
}

/// Defines a complex-in, number-out function.
macro_rules! to_number {
    ($konst:ident, $name:literal, $help:literal, $f:expr) => {
        pub const $konst: FunctionDef = FunctionDef::new(
            $name,
            Category::AddIn,
            cplx::UNARY_PARAMS,
            |ctx: &mut dyn EvalContext, args: &mut dyn ArgList| {
                let mut a = Args::new(ctx, args);
                run(|| {
                    let z = cplx::arg(&mut a, 0)?;
                    let f: fn(&Cx) -> Result<f64, ErrorKind> = $f;
                    Ok(Value::Number(f(&z)?))
                })
            },
        )
        .help($help);
    };
}

to_number!(
    IMABS,
    "IMABS",
    "Returns the absolute value (modulus) of a complex number",
    |z| Ok(z.abs())
);
to_number!(
    IMAGINARY,
    "IMAGINARY",
    "Returns the imaginary coefficient of a complex number",
    |z| Ok(z.i)
);
to_number!(
    IMREAL,
    "IMREAL",
    "Returns the real coefficient of a complex number",
    |z| Ok(z.r)
);
to_number!(
    IMARGUMENT,
    "IMARGUMENT",
    "Returns the argument theta, an angle expressed in radians",
    |z| z.arg()
);

unary_complex!(
    IMCONJUGATE,
    "IMCONJUGATE",
    "Returns the complex conjugate of a complex number",
    Cx::conj
);

pub(super) fn register(r: &mut Registry) {
    r.register(COMPLEX);
    r.register(IMABS);
    r.register(IMAGINARY);
    r.register(IMARGUMENT);
    r.register(IMCONJUGATE);
    r.register(IMREAL);
}

#[cfg(test)]
mod tests {
    use super::super::cplx::call_text;
    use crate::EvalValue;
    use tally_types::{ErrorKind, Value};

    fn t(s: &str) -> EvalValue {
        EvalValue::text(s)
    }
    fn n(x: f64) -> EvalValue {
        EvalValue::number(x)
    }
    fn text(s: &str) -> Value {
        Value::text(s)
    }

    #[test]
    fn complex_builds_text() {
        assert_eq!(call_text("COMPLEX", vec![n(3.0), n(4.0)]), text("3+4i"));
        assert_eq!(
            call_text("COMPLEX", vec![n(3.0), n(4.0), t("j")]),
            text("3+4j")
        );
        assert_eq!(call_text("COMPLEX", vec![n(0.0), n(1.0)]), text("i"));
        assert_eq!(call_text("COMPLEX", vec![n(1.0), n(0.0)]), text("1"));
        assert_eq!(call_text("COMPLEX", vec![n(0.0), n(-1.0)]), text("-i"));
        assert_eq!(call_text("COMPLEX", vec![n(0.0), n(0.0)]), text("0"));
        assert_eq!(call_text("COMPLEX", vec![n(1.0), n(-2.5)]), text("1-2.5i"));
        assert_eq!(
            call_text("COMPLEX", vec![n(1.0), n(1.0), t("k")]),
            Value::Error(ErrorKind::InvalidArgument)
        );
    }

    #[test]
    fn parts() {
        assert_eq!(call_text("IMABS", vec![t("3+4i")]), Value::Number(5.0));
        assert_eq!(call_text("IMREAL", vec![t("3+4i")]), Value::Number(3.0));
        assert_eq!(call_text("IMAGINARY", vec![t("3+4i")]), Value::Number(4.0));
        assert_eq!(call_text("IMAGINARY", vec![t("-j")]), Value::Number(-1.0));
        assert_eq!(call_text("IMAGINARY", vec![n(7.0)]), Value::Number(0.0));
        assert_eq!(
            call_text("IMREAL", vec![t("1e2+3.5e-1i")]),
            Value::Number(100.0)
        );
        assert_eq!(
            call_text("IMAGINARY", vec![t("1e2+3.5e-1i")]),
            Value::Number(0.35)
        );
        assert_eq!(call_text("IMCONJUGATE", vec![t("3+4j")]), text("3-4j"));
        match call_text("IMARGUMENT", vec![t("3+4i")]) {
            Value::Number(x) => assert!((x - 0.927295218001612).abs() < 1e-14),
            o => panic!("{o:?}"),
        }
        assert_eq!(
            call_text("IMARGUMENT", vec![t("0")]),
            Value::Error(ErrorKind::InvalidArgument)
        );
        for bad in ["3+4", "abc", "3+4k", "1+2i3", "i+1", "3 +4i"] {
            assert_eq!(
                call_text("IMABS", vec![t(bad)]),
                Value::Error(ErrorKind::InvalidArgument),
                "{bad}"
            );
        }
    }
}
