//! ISERR, ISLOGICAL, ISNONTEXT, ISREF, ISFORMULA, N and TYPE.

use tally_types::{ErrorKind, Value};

use crate::coerce::scalar_value;
use crate::registry::{Category, FnFlags, FunctionDef, Param, ParamKind, Registry, ReturnKind};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const ANY1: &[Param] = &[Param::required("value", ParamKind::Any)];
const REF1: &[Param] = &[Param::required("reference", ParamKind::Reference)];

/// `ISERR(value)`.
pub const ISERR: FunctionDef = FunctionDef::new("ISERR", Category::Information, ANY1, iserr)
    .returns(ReturnKind::Bool)
    .help("Returns TRUE for any error value except #N/A.");
/// `ISLOGICAL(value)`.
pub const ISLOGICAL: FunctionDef =
    FunctionDef::new("ISLOGICAL", Category::Information, ANY1, islogical)
        .returns(ReturnKind::Bool)
        .help("Returns TRUE if value is a logical (boolean) value.");
/// `ISNONTEXT(value)`.
pub const ISNONTEXT: FunctionDef =
    FunctionDef::new("ISNONTEXT", Category::Information, ANY1, isnontext)
        .returns(ReturnKind::Bool)
        .help("Returns TRUE if value is not text.");
/// `ISREF(value)`.
pub const ISREF: FunctionDef = FunctionDef::new("ISREF", Category::Information, REF1, isref)
    .returns(ReturnKind::Bool)
    .flags(FnFlags::REF_ADDRESS_ONLY)
    .help("Returns TRUE if the argument is a reference.");
/// `ISFORMULA(reference)`.
pub const ISFORMULA: FunctionDef =
    FunctionDef::new("ISFORMULA", Category::Information, REF1, isformula)
        .returns(ReturnKind::Bool)
        .flags(FnFlags::REF_ADDRESS_ONLY)
        .help("Returns TRUE if the referenced cell contains a formula.");
/// `N(value)`.
pub const N: FunctionDef = FunctionDef::new("N", Category::Information, ANY1, n_fn)
    .returns(ReturnKind::Number)
    .help("Converts a value to a number.");
/// `TYPE(value)`.
pub const TYPE: FunctionDef = FunctionDef::new("TYPE", Category::Information, REF1_TYPE, type_fn)
    .returns(ReturnKind::Number)
    .help("Type of a value: 1 number, 2 text, 4 logical, 16 error, 64 array.");

const REF1_TYPE: &[Param] = &[Param::required("value", ParamKind::Reference)];

fn iserr(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        Ok(Value::Bool(matches!(
            a.value(0),
            Value::Error(e) if e != ErrorKind::NA
        )))
    })
}

fn islogical(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| Ok(Value::Bool(matches!(a.value(0), Value::Bool(_)))))
}

fn isnontext(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| Ok(Value::Bool(!matches!(a.value(0), Value::Text(_)))))
}

fn isref(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| Ok(Value::Bool(matches!(a.any(0), EvalValue::Range(_)))))
}

fn isformula(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let r = a.range(0)?;
        let info = a.ctx().cell_format_info(r.start());
        Ok(Value::Bool(info.has_formula))
    })
}

fn n_fn(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| match a.value(0) {
        Value::Error(e) => Err(e),
        Value::Number(n) => Ok(Value::Number(n)),
        Value::Bool(b) => Ok(Value::Number(if b { 1.0 } else { 0.0 })),
        _ => Ok(Value::Number(0.0)),
    })
}

fn type_of(v: &Value) -> f64 {
    match v {
        Value::Number(_) | Value::Empty => 1.0,
        Value::Text(_) => 2.0,
        Value::Bool(_) => 4.0,
        Value::Error(_) => 16.0,
        Value::Array(_) => 64.0,
    }
}

/// Multi-cell references and arrays are 64; a single cell has the type of
/// its value (an empty cell is a number).
fn type_fn(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| match a.any(0) {
        EvalValue::Array(_) | EvalValue::Scalar(Value::Array(_)) => Ok(Value::Number(64.0)),
        EvalValue::Range(r) if !r.is_single() => Ok(Value::Number(64.0)),
        other => {
            let v = scalar_value(&other, a.ctx());
            Ok(Value::Number(type_of(&v)))
        }
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(ISERR);
    r.register(ISLOGICAL);
    r.register(ISNONTEXT);
    r.register(ISREF);
    r.register(ISFORMULA);
    r.register(N);
    r.register(TYPE);
}

#[cfg(test)]
mod tests {
    use crate::context::CellFormatInfo;
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use std::sync::Arc;
    use tally_types::{ErrorKind, Matrix, Value};

    fn sheet() -> MockEvalContext {
        let mut c = MockEvalContext::new();
        c.set(0, "A1", 1.0)
            .set(0, "A2", "x")
            .set(0, "A3", true)
            .set(0, "A4", Value::Error(ErrorKind::NA))
            .set(0, "A5", Value::Error(ErrorKind::Div0));
        c
    }

    fn at(c: &mut MockEvalContext, name: &str, cell: &str) -> Value {
        let r = c.range(0, cell);
        call(c, name, vec![r])
    }

    #[test]
    fn predicates() {
        let mut c = sheet();
        assert_eq!(at(&mut c, "ISERR", "A4"), Value::Bool(false));
        assert_eq!(at(&mut c, "ISERR", "A5"), Value::Bool(true));
        assert_eq!(at(&mut c, "ISERR", "A1"), Value::Bool(false));
        assert_eq!(at(&mut c, "ISLOGICAL", "A3"), Value::Bool(true));
        assert_eq!(at(&mut c, "ISLOGICAL", "A1"), Value::Bool(false));
        assert_eq!(at(&mut c, "ISNONTEXT", "A2"), Value::Bool(false));
        assert_eq!(at(&mut c, "ISNONTEXT", "A1"), Value::Bool(true));
        assert_eq!(at(&mut c, "ISNONTEXT", "A5"), Value::Bool(true));
        assert_eq!(at(&mut c, "ISNONTEXT", "B9"), Value::Bool(true));
    }

    #[test]
    fn isref_and_formula() {
        let mut c = sheet();
        assert_eq!(at(&mut c, "ISREF", "A1"), Value::Bool(true));
        assert_eq!(at(&mut c, "ISREF", "A1:A3"), Value::Bool(true));
        assert_eq!(
            call(&mut c, "ISREF", vec![EvalValue::number(1.0)]),
            Value::Bool(false)
        );
        assert_eq!(
            call(&mut c, "ISREF", vec![EvalValue::error(ErrorKind::Ref)]),
            Value::Bool(false)
        );
        let info = CellFormatInfo {
            has_formula: true,
            ..Default::default()
        };
        c.set_format(0, "A1", info);
        assert_eq!(at(&mut c, "ISFORMULA", "A1"), Value::Bool(true));
        assert_eq!(at(&mut c, "ISFORMULA", "A2"), Value::Bool(false));
        assert_eq!(
            call(&mut c, "ISFORMULA", vec![EvalValue::number(1.0)]),
            Value::Error(ErrorKind::Value)
        );
    }

    #[test]
    fn n_fn() {
        let mut c = sheet();
        assert_eq!(at(&mut c, "N", "A1"), Value::Number(1.0));
        assert_eq!(at(&mut c, "N", "A2"), Value::Number(0.0));
        assert_eq!(at(&mut c, "N", "A3"), Value::Number(1.0));
        assert_eq!(at(&mut c, "N", "A5"), Value::Error(ErrorKind::Div0));
        assert_eq!(at(&mut c, "N", "B9"), Value::Number(0.0));
        assert_eq!(
            call(&mut c, "N", vec![EvalValue::text("7")]),
            Value::Number(0.0)
        );
    }

    #[test]
    fn type_fn() {
        let mut c = sheet();
        assert_eq!(at(&mut c, "TYPE", "A1"), Value::Number(1.0));
        assert_eq!(at(&mut c, "TYPE", "A2"), Value::Number(2.0));
        assert_eq!(at(&mut c, "TYPE", "A3"), Value::Number(4.0));
        assert_eq!(at(&mut c, "TYPE", "A5"), Value::Number(16.0));
        assert_eq!(at(&mut c, "TYPE", "A1:A3"), Value::Number(64.0));
        let m = Matrix::new_filled(1, 2, Value::Number(1.0));
        assert_eq!(
            call(&mut c, "TYPE", vec![EvalValue::Array(Arc::new(m))]),
            Value::Number(64.0)
        );
        assert_eq!(
            call(&mut c, "TYPE", vec![EvalValue::text("a")]),
            Value::Number(2.0)
        );
    }
}
