//! TRUE and FALSE.

use tally_types::Value;

use crate::registry::{Category, FunctionDef, Registry, ReturnKind};
use crate::{ArgList, EvalContext, EvalValue};

/// `TRUE()`.
pub const TRUE: FunctionDef = FunctionDef::new("TRUE", Category::Logical, &[], true_fn)
    .returns(ReturnKind::Bool)
    .help("Returns the logical value TRUE.");

/// `FALSE()`.
pub const FALSE: FunctionDef = FunctionDef::new("FALSE", Category::Logical, &[], false_fn)
    .returns(ReturnKind::Bool)
    .help("Defines the logical value as FALSE.");

fn true_fn(_ctx: &mut dyn EvalContext, _args: &mut dyn ArgList) -> EvalValue {
    EvalValue::Scalar(Value::Bool(true))
}

fn false_fn(_ctx: &mut dyn EvalContext, _args: &mut dyn ArgList) -> EvalValue {
    EvalValue::Scalar(Value::Bool(false))
}

pub(super) fn register(r: &mut Registry) {
    r.register(TRUE);
    r.register(FALSE);
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{ErrorKind, Value};

    #[test]
    fn constants() {
        let mut c = MockEvalContext::new();
        assert_eq!(call(&mut c, "TRUE", vec![]), Value::Bool(true));
        assert_eq!(call(&mut c, "FALSE", vec![]), Value::Bool(false));
        assert_eq!(
            call(&mut c, "TRUE", vec![EvalValue::number(1.0)]),
            Value::Error(ErrorKind::ParamList)
        );
    }
}
