//! CONCATENATE (later: CONCAT/TEXTJOIN if added as extensions).

use tally_types::Value;

use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry, ReturnKind};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const CONCATENATE_PARAMS: &[Param] = &[Param::repeat("text", ParamKind::Text)];

/// `CONCATENATE(text; ...)`.
pub const CONCATENATE: FunctionDef = FunctionDef::new(
    "CONCATENATE",
    Category::Text,
    CONCATENATE_PARAMS,
    concatenate,
)
.returns(ReturnKind::Text)
.help("Combines several text items into one.");

fn concatenate(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let mut out = String::new();
        for i in 0..a.count() {
            out.push_str(&a.text(i)?);
        }
        Ok(Value::text(out))
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(CONCATENATE);
}
