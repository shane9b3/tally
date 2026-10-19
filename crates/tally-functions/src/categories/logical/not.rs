//! NOT.

use tally_types::Value;

use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry, ReturnKind};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const PARAMS: &[Param] = &[Param::required("logical_value", ParamKind::Bool)];

/// `NOT(logical_value)`.
pub const NOT: FunctionDef = FunctionDef::new("NOT", Category::Logical, PARAMS, not)
    .returns(ReturnKind::Bool)
    .help("Reverses the value of the argument.");

fn not(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| Ok(Value::Bool(!a.bool(0)?)))
}

pub(super) fn register(r: &mut Registry) {
    r.register(NOT);
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{ErrorKind, Value};

    #[test]
    fn not_cases() {
        let mut c = MockEvalContext::new();
        let mut f = |v| call(&mut c, "NOT", vec![v]);
        assert_eq!(f(EvalValue::bool(true)), Value::Bool(false));
        assert_eq!(f(EvalValue::number(0.0)), Value::Bool(true));
        assert_eq!(f(EvalValue::Missing), Value::Bool(true));
        assert_eq!(f(EvalValue::text("TRUE")), Value::Bool(false));
        assert_eq!(f(EvalValue::text("x")), Value::Error(ErrorKind::Value));
        assert_eq!(
            f(EvalValue::error(ErrorKind::Div0)),
            Value::Error(ErrorKind::Div0)
        );
    }
}
