//! IMSUM, IMSUB, IMPRODUCT, IMDIV.
//!
//! Ported from Apache OpenOffice `scaddins/source/analysis/analysishelper.cxx`
//! (`Complex`, `ComplexList`; Apache-2.0).

use tally_types::{ErrorKind, Value};

use super::cplx::{self, Cx};
use crate::coerce::value_to_text;
use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry, ReturnKind};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const SEQ_PARAMS: &[Param] = &[Param::repeat("complex_number", ParamKind::ValueSeq)];
const TWO_PARAMS: &[Param] = &[
    Param::required("complex_number_1", ParamKind::Text),
    Param::required("complex_number_2", ParamKind::Text),
];

/// Folds every complex number in all arguments (cells of references and
/// arrays; empty cells are skipped, errors propagate).
fn fold_all(
    ctx: &mut dyn EvalContext,
    args: &mut dyn ArgList,
    op: fn(&Cx, &Cx) -> Result<Cx, ErrorKind>,
) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let mut acc: Option<Cx> = None;
        let mut err: Option<ErrorKind> = None;
        for i in 0..a.count() {
            a.for_each_value(i, |v, _| {
                if err.is_some() {
                    return;
                }
                let step = match v {
                    Value::Empty => return,
                    Value::Error(e) => Err(*e),
                    Value::Number(x) => Ok(Cx::real(*x)),
                    other => value_to_text(other).and_then(|t| Cx::parse(&t)),
                };
                match step.and_then(|z| match &acc {
                    None => Ok(z),
                    Some(p) => op(p, &z),
                }) {
                    Ok(z) => acc = Some(z),
                    Err(e) => err = Some(e),
                }
            });
            if let Some(e) = err {
                return Err(e);
            }
        }
        let z = acc.unwrap_or(Cx::real(0.0));
        Ok(Value::text(z.format()?))
    })
}

/// `IMSUM(complex; ...)`.
pub const IMSUM: FunctionDef = FunctionDef::new("IMSUM", Category::AddIn, SEQ_PARAMS, |c, a| {
    fold_all(c, a, Cx::add)
})
.returns(ReturnKind::Text)
.help("Returns the sum of complex numbers");

/// `IMPRODUCT(complex; ...)`.
pub const IMPRODUCT: FunctionDef =
    FunctionDef::new("IMPRODUCT", Category::AddIn, SEQ_PARAMS, |c, a| {
        fold_all(c, a, Cx::mul)
    })
    .returns(ReturnKind::Text)
    .help("Returns the product of complex numbers");

fn binary(
    ctx: &mut dyn EvalContext,
    args: &mut dyn ArgList,
    op: fn(&Cx, &Cx) -> Result<Cx, ErrorKind>,
) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let x = cplx::arg(&mut a, 0)?;
        let y = cplx::arg(&mut a, 1)?;
        Ok(Value::text(op(&x, &y)?.format()?))
    })
}

/// `IMSUB(a; b)`.
pub const IMSUB: FunctionDef = FunctionDef::new("IMSUB", Category::AddIn, TWO_PARAMS, |c, a| {
    binary(c, a, Cx::sub)
})
.returns(ReturnKind::Text)
.help("Returns the difference of two complex numbers");

/// `IMDIV(a; b)`.
pub const IMDIV: FunctionDef = FunctionDef::new("IMDIV", Category::AddIn, TWO_PARAMS, |c, a| {
    binary(c, a, Cx::div)
})
.returns(ReturnKind::Text)
.help("Returns the quotient of two complex numbers");

pub(super) fn register(r: &mut Registry) {
    r.register(IMSUM);
    r.register(IMSUB);
    r.register(IMPRODUCT);
    r.register(IMDIV);
}

#[cfg(test)]
mod tests {
    use super::super::cplx::call_text;
    use crate::mock::{call, MockEvalContext};
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
    fn sum_and_product() {
        assert_eq!(call_text("IMSUM", vec![t("3+4i"), t("-3-4i")]), text("0"));
        assert_eq!(
            call_text("IMSUM", vec![t("1+2i"), t("3+4i"), t("5")]),
            text("9+6i")
        );
        assert_eq!(call_text("IMSUM", vec![t("1+2j"), t("3")]), text("4+2j"));
        assert_eq!(call_text("IMSUM", vec![t("1+2i"), t("3+4j")]), BAD);
        assert_eq!(call_text("IMPRODUCT", vec![t("i"), t("i")]), text("-1"));
        assert_eq!(
            call_text("IMPRODUCT", vec![t("3+4i"), t("5-3i")]),
            text("27+11i")
        );
        assert_eq!(
            call_text("IMSUM", vec![EvalValue::number(2.0), t("i")]),
            text("2+i")
        );
        assert_eq!(
            call_text("IMSUM", vec![EvalValue::error(ErrorKind::NA)]),
            Value::Error(ErrorKind::NA)
        );
    }

    #[test]
    fn sum_over_range() {
        let mut c = MockEvalContext::new();
        c.set(0, "A1", "1+i");
        c.set(0, "A2", 2.0);
        c.set(0, "A4", "3-4i");
        let r = c.range(0, "A1:A4");
        assert_eq!(call(&mut c, "IMSUM", vec![r.clone()]), text("6-3i"));
        assert_eq!(call(&mut c, "IMPRODUCT", vec![r]), text("14-2i"));
    }

    #[test]
    fn sub_div() {
        assert_eq!(call_text("IMSUB", vec![t("13+4i"), t("5+3i")]), text("8+i"));
        assert_eq!(
            call_text("IMDIV", vec![t("-238+240i"), t("10+24i")]),
            text("5+12i")
        );
        assert_eq!(call_text("IMDIV", vec![t("1"), t("0")]), BAD);
        assert_eq!(call_text("IMSUB", vec![t("1+i"), t("1+j")]), BAD);
    }
}
