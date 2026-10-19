//! LEN.

use tally_types::Value;

use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry, ReturnKind};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const LEN_PARAMS: &[Param] = &[Param::required("text", ParamKind::Text)];

/// `LEN(text)`: number of Unicode scalar values.
pub const LEN: FunctionDef = FunctionDef::new("LEN", Category::Text, LEN_PARAMS, len)
    .returns(ReturnKind::Number)
    .help("Number of characters in a text.");

fn len(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| Ok(Value::Number(a.text(0)?.chars().count() as f64)))
}

pub(super) fn register(r: &mut Registry) {
    r.register(LEN);
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{ErrorKind, Value};

    #[test]
    fn len() {
        let mut c = MockEvalContext::new();
        let mut l = |v| call(&mut c, "LEN", vec![v]);
        assert_eq!(l(EvalValue::text("hello")), Value::Number(5.0));
        assert_eq!(l(EvalValue::text("")), Value::Number(0.0));
        assert_eq!(l(EvalValue::text("a😀é")), Value::Number(3.0));
        assert_eq!(l(EvalValue::number(1.5)), Value::Number(3.0));
        assert_eq!(l(EvalValue::bool(true)), Value::Number(4.0));
        assert_eq!(l(EvalValue::Missing), Value::Number(0.0));
        assert_eq!(
            l(EvalValue::error(ErrorKind::NA)),
            Value::Error(ErrorKind::NA)
        );
    }
}
