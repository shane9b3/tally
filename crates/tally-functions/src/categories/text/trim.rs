//! TRIM.

use tally_types::Value;

use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry, ReturnKind};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const TRIM_PARAMS: &[Param] = &[Param::required("text", ParamKind::Text)];

/// `TRIM(text)`: removes leading and trailing spaces and collapses runs of
/// internal spaces to one (only U+0020 counts as a space).
pub const TRIM: FunctionDef = FunctionDef::new("TRIM", Category::Text, TRIM_PARAMS, trim)
    .returns(ReturnKind::Text)
    .help("Removes extra spaces from a text.");

fn trim(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let s = a.text(0)?;
        let parts: Vec<&str> = s.split(' ').filter(|p| !p.is_empty()).collect();
        Ok(Value::text(parts.join(" ")))
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(TRIM);
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{ErrorKind, Value};

    #[test]
    fn trim() {
        let mut c = MockEvalContext::new();
        let mut t = |v| call(&mut c, "TRIM", vec![v]);
        assert_eq!(t(EvalValue::text("  a   b  c ")), Value::text("a b c"));
        assert_eq!(t(EvalValue::text("   ")), Value::text(""));
        assert_eq!(t(EvalValue::text("ab")), Value::text("ab"));
        assert_eq!(t(EvalValue::text("a\tb")), Value::text("a\tb"));
        assert_eq!(t(EvalValue::number(12.0)), Value::text("12"));
        assert_eq!(
            t(EvalValue::error(ErrorKind::NA)),
            Value::Error(ErrorKind::NA)
        );
    }
}
