//! ISBLANK, ISNUMBER, ISTEXT, ISERROR, ISNA.
//!
//! The parameter is `ParamKind::Any`, so a range is reduced by implicit
//! intersection before the function runs and an error in a cell or in an
//! argument is *not* propagated: `Args::value` hands the raw value over.

use tally_types::{ErrorKind, Value};

use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry, ReturnKind};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const PARAMS: &[Param] = &[Param::required("value", ParamKind::Any)];

/// `ISBLANK(value)`.
pub const ISBLANK: FunctionDef =
    FunctionDef::new("ISBLANK", Category::Information, PARAMS, isblank)
        .returns(ReturnKind::Bool)
        .help("Returns TRUE if value refers to an empty cell.");
/// `ISNUMBER(value)`.
pub const ISNUMBER: FunctionDef =
    FunctionDef::new("ISNUMBER", Category::Information, PARAMS, isnumber)
        .returns(ReturnKind::Bool)
        .help("Returns TRUE if value is a number.");
/// `ISTEXT(value)`.
pub const ISTEXT: FunctionDef = FunctionDef::new("ISTEXT", Category::Information, PARAMS, istext)
    .returns(ReturnKind::Bool)
    .help("Returns TRUE if value is text.");
/// `ISERROR(value)`.
pub const ISERROR: FunctionDef =
    FunctionDef::new("ISERROR", Category::Information, PARAMS, iserror)
        .returns(ReturnKind::Bool)
        .help("Returns TRUE if the value is an error value.");
/// `ISNA(value)`.
pub const ISNA: FunctionDef = FunctionDef::new("ISNA", Category::Information, PARAMS, isna)
    .returns(ReturnKind::Bool)
    .help("Returns TRUE if value equals #N/A.");

fn test(ctx: &mut dyn EvalContext, args: &mut dyn ArgList, pred: fn(&Value) -> bool) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| Ok(Value::Bool(pred(&a.value(0)))))
}

/// Only a truly empty cell (or omitted argument): a formula returning `""`
/// is text, not blank.
fn isblank(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    test(ctx, args, |v| matches!(v, Value::Empty))
}

/// Booleans are numbers in AOO (stored as 1/0 with a boolean format), so
/// `ISNUMBER(TRUE)` is TRUE here; Excel says FALSE.
fn isnumber(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    test(ctx, args, |v| {
        matches!(v, Value::Number(_) | Value::Bool(_))
    })
}

fn istext(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    test(ctx, args, |v| matches!(v, Value::Text(_)))
}

fn iserror(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    test(ctx, args, |v| matches!(v, Value::Error(_)))
}

fn isna(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    test(ctx, args, |v| matches!(v, Value::Error(ErrorKind::NA)))
}

pub(super) fn register(r: &mut Registry) {
    r.register(ISBLANK);
    r.register(ISNUMBER);
    r.register(ISTEXT);
    r.register(ISERROR);
    r.register(ISNA);
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{ErrorKind, Value};

    fn sheet() -> MockEvalContext {
        let mut c = MockEvalContext::new();
        c.set(0, "A1", 1.0)
            .set(0, "A2", "x")
            .set(0, "A3", true)
            .set(0, "A4", Value::Error(ErrorKind::NA))
            .set(0, "A5", Value::Error(ErrorKind::Div0))
            .set(0, "A6", "");
        c
    }

    fn t(c: &mut MockEvalContext, f: &str, cell: &str) -> Value {
        let r = c.range(0, cell);
        call(c, f, vec![r])
    }

    #[test]
    fn predicates_on_cells() {
        let mut c = sheet();
        let yes = Value::Bool(true);
        let no = Value::Bool(false);
        assert_eq!(t(&mut c, "ISNUMBER", "A1"), yes);
        assert_eq!(t(&mut c, "ISNUMBER", "A3"), yes);
        assert_eq!(t(&mut c, "ISNUMBER", "A2"), no);
        assert_eq!(t(&mut c, "ISTEXT", "A2"), yes);
        assert_eq!(t(&mut c, "ISTEXT", "A6"), yes);
        assert_eq!(t(&mut c, "ISTEXT", "A1"), no);
        assert_eq!(t(&mut c, "ISBLANK", "B9"), yes);
        assert_eq!(t(&mut c, "ISBLANK", "A6"), no);
        assert_eq!(t(&mut c, "ISERROR", "A4"), yes);
        assert_eq!(t(&mut c, "ISERROR", "A5"), yes);
        assert_eq!(t(&mut c, "ISNA", "A4"), yes);
        assert_eq!(t(&mut c, "ISNA", "A5"), no);
    }

    #[test]
    fn errors_do_not_propagate() {
        let mut c = sheet();
        assert_eq!(
            call(&mut c, "ISERROR", vec![EvalValue::error(ErrorKind::Num)]),
            Value::Bool(true)
        );
        assert_eq!(
            call(&mut c, "ISNUMBER", vec![EvalValue::error(ErrorKind::Num)]),
            Value::Bool(false)
        );
        assert_eq!(
            call(&mut c, "ISBLANK", vec![EvalValue::Missing]),
            Value::Bool(true)
        );
    }

    #[test]
    fn range_intersection() {
        let mut c = sheet();
        c.set_origin(0, "D4");
        let r = c.range(0, "A1:A6");
        assert_eq!(call(&mut c, "ISNA", vec![r.clone()]), Value::Bool(true));
        c.set_origin(0, "D20");
        assert_eq!(call(&mut c, "ISERROR", vec![r]), Value::Bool(true));
    }
}
