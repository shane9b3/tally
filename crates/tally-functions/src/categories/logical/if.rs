//! IF (later: IFERROR, IFNA, AND, OR, NOT, TRUE, FALSE in their own files).

use tally_types::Value;

use crate::registry::{Category, FnFlags, FunctionDef, Param, ParamKind, Registry, ReturnKind};
use crate::{run_ref, ArgList, Args, EvalContext, EvalValue};

const IF_PARAMS: &[Param] = &[
    Param::required("test", ParamKind::Bool),
    Param::optional("then_value", ParamKind::Lazy),
    Param::optional("otherwise_value", ParamKind::Lazy),
];

/// `IF(test; [then_value]; [otherwise_value])`.
pub const IF: FunctionDef = FunctionDef::new("IF", Category::Logical, IF_PARAMS, eval_if)
    .returns(ReturnKind::Any)
    .flags(FnFlags::LAZY_ARGS.union(FnFlags::RETURNS_REF))
    .help("Specifies a logical test to be performed.");

/// Only the chosen branch is evaluated. An absent branch yields the test
/// result (`IF(x)` is TRUE when x is true, `IF(x; 1)` is FALSE otherwise);
/// an explicitly empty branch (`IF(x;;2)`) yields 0. References pass
/// through unchanged so `SUM(IF(c; A1:A3))` sees the whole range.
fn eval_if(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run_ref(|| {
        let test = a.bool(0)?;
        let branch = if test { 1 } else { 2 };
        if branch >= a.count() {
            return Ok(EvalValue::Scalar(Value::Bool(test)));
        }
        Ok(match a.any(branch) {
            EvalValue::Missing => EvalValue::number(0.0),
            v => v,
        })
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(IF);
}
