//! IFERROR and IFNA (extensions: not in AOO 4.1, ODF `COM.MICROSOFT.*`).

use std::sync::Arc;

use tally_types::{ErrorKind, Matrix, Value};

use crate::coerce::scalar_value;
use crate::registry::{Category, FnFlags, FunctionDef, Param, ParamKind, Registry, ReturnKind};
use crate::{run_ref, ArgList, Args, EvalContext, EvalValue};

const PARAMS: &[Param] = &[
    Param::required("value", ParamKind::Lazy),
    Param::required("value_if_error", ParamKind::Lazy),
];

/// `IFERROR(value; value_if_error)`.
pub const IFERROR: FunctionDef = FunctionDef::new("IFERROR", Category::Logical, PARAMS, iferror)
    .odf("COM.MICROSOFT.IFERROR")
    .ooxml("_xlfn.IFERROR")
    .extension()
    .returns(ReturnKind::Any)
    .flags(FnFlags::LAZY_ARGS.union(FnFlags::RETURNS_REF))
    .help("Returns the value, or the second argument if the value is an error.");

/// `IFNA(value; value_if_na)`.
pub const IFNA: FunctionDef = FunctionDef::new("IFNA", Category::Logical, PARAMS, ifna)
    .odf("COM.MICROSOFT.IFNA")
    .ooxml("_xlfn.IFNA")
    .extension()
    .returns(ReturnKind::Any)
    .flags(FnFlags::LAZY_ARGS.union(FnFlags::RETURNS_REF))
    .help("Returns the value, or the second argument if the value is #N/A.");

/// The fallback argument (evaluated only when needed); an empty fallback
/// yields 0 as in Excel.
fn fallback(a: &mut Args<'_>) -> EvalValue {
    match a.any(1) {
        EvalValue::Missing => EvalValue::number(0.0),
        v => v,
    }
}

fn guard(
    ctx: &mut dyn EvalContext,
    args: &mut dyn ArgList,
    is_err: fn(ErrorKind) -> bool,
) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run_ref(|| {
        let first = a.any(0);
        let multi = match &first {
            EvalValue::Range(r) => !r.is_single(),
            EvalValue::Array(m) => m.rows * m.cols != 1,
            _ => false,
        };
        if multi && a.ctx().in_array_context() {
            return elementwise(&mut a, &first, is_err);
        }
        let v = scalar_value(&first, a.ctx());
        match v {
            Value::Error(e) if is_err(e) => Ok(fallback(&mut a)),
            v => Ok(EvalValue::from_value(v)),
        }
    })
}

/// Array context: apply the test to each element, evaluating the fallback
/// at most once and only if some element needs it. A fallback array is
/// reduced to its top-left element.
fn elementwise(
    a: &mut Args<'_>,
    first: &EvalValue,
    is_err: fn(ErrorKind) -> bool,
) -> Result<EvalValue, ErrorKind> {
    let mut m = match first {
        EvalValue::Array(m) => (**m).clone(),
        EvalValue::Range(r) => {
            let mut m = Matrix::new_filled(r.rows(), r.cols(), Value::Empty);
            let (r0, c0) = (r.r0, r.c0);
            a.ctx().for_each_in_range(r, true, &mut |addr, v| {
                m.set(addr.row - r0, addr.col - c0, v.clone());
                std::ops::ControlFlow::Continue(())
            });
            m
        }
        _ => return Err(ErrorKind::Value),
    };
    let mut fb: Option<Value> = None;
    for x in &mut m.data {
        if matches!(x, Value::Error(e) if is_err(*e)) {
            if fb.is_none() {
                let f = fallback(a);
                fb = Some(scalar_value(&f, a.ctx()));
            }
            if let Some(f) = &fb {
                *x = f.clone();
            }
        }
    }
    Ok(EvalValue::Array(Arc::new(m)))
}

fn iferror(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    guard(ctx, args, |_| true)
}

fn ifna(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    guard(ctx, args, |e| e == ErrorKind::NA)
}

pub(super) fn register(r: &mut Registry) {
    r.register(IFERROR);
    r.register(IFNA);
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, call_with, MockArgs, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{ErrorKind, Value};

    #[test]
    fn iferror_basic() {
        let mut c = MockEvalContext::new();
        let e = EvalValue::error(ErrorKind::Div0);
        assert_eq!(
            call(&mut c, "IFERROR", vec![e.clone(), EvalValue::number(7.0)]),
            Value::Number(7.0)
        );
        assert_eq!(
            call(
                &mut c,
                "IFERROR",
                vec![EvalValue::number(1.0), EvalValue::number(7.0)]
            ),
            Value::Number(1.0)
        );
        assert_eq!(
            call(&mut c, "IFERROR", vec![e, EvalValue::Missing]),
            Value::Number(0.0)
        );
    }

    #[test]
    fn fallback_is_lazy() {
        let mut c = MockEvalContext::new();
        let mut args = MockArgs::new(vec![EvalValue::number(1.0), EvalValue::number(2.0)]);
        assert_eq!(call_with(&mut c, "IFERROR", &mut args), Value::Number(1.0));
        assert!(!args.was_evaluated(1));
    }

    #[test]
    fn ifna_only_na() {
        let mut c = MockEvalContext::new();
        c.set(0, "A1", Value::Error(ErrorKind::NA));
        let r = c.range(0, "A1");
        assert_eq!(
            call(&mut c, "IFNA", vec![r, EvalValue::text("x")]),
            Value::text("x")
        );
        assert_eq!(
            call(
                &mut c,
                "IFNA",
                vec![EvalValue::error(ErrorKind::Div0), EvalValue::text("x")]
            ),
            Value::Error(ErrorKind::Div0)
        );
    }

    #[test]
    fn array_context_elementwise() {
        let mut c = MockEvalContext::new();
        c.array_context = true;
        c.set(0, "A1", 1.0)
            .set(0, "A2", Value::Error(ErrorKind::NA));
        let r = c.range(0, "A1:A2");
        let v = call(&mut c, "IFERROR", vec![r, EvalValue::number(0.0)]);
        match v {
            Value::Array(m) => assert_eq!(m.data, vec![Value::Number(1.0), Value::Number(0.0)]),
            other => panic!("expected array, got {other:?}"),
        }
    }
}
