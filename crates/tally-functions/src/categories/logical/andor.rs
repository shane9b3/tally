//! AND and OR.

use tally_types::{ErrorKind, Value};

use crate::coerce::{value_to_bool, SeqSource};
use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry, ReturnKind};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const PARAMS: &[Param] = &[Param::repeat("logical_value", ParamKind::NumberSeq)];

/// `AND(logical_value; ...)`.
pub const AND: FunctionDef = FunctionDef::new("AND", Category::Logical, PARAMS, and)
    .returns(ReturnKind::Bool)
    .help("Returns TRUE if all arguments are TRUE.");

/// `OR(logical_value; ...)`.
pub const OR: FunctionDef = FunctionDef::new("OR", Category::Logical, PARAMS, or)
    .returns(ReturnKind::Bool)
    .help("Returns TRUE if an argument is TRUE.");

/// Folds every logical value. Direct arguments are coerced as booleans
/// (`"TRUE"` text works, other text is `#VALUE!`, a missing one is FALSE);
/// inside references and arrays only numbers and booleans count and text
/// and empty cells are skipped. The first error (in argument, then
/// column-major order) wins over any boolean result. No logical value at
/// all is `#VALUE!`.
fn fold(ctx: &mut dyn EvalContext, args: &mut dyn ArgList, is_and: bool) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let mut seen = false;
        let mut result = is_and;
        let mut error: Option<ErrorKind> = None;
        for i in 0..a.count() {
            a.for_each_value(i, |v, src| {
                if error.is_some() {
                    return;
                }
                let b = match (v, src) {
                    (Value::Error(e), _) => {
                        error = Some(*e);
                        return;
                    }
                    (Value::Number(_) | Value::Bool(_), _) => value_to_bool(v),
                    (_, SeqSource::Direct) => value_to_bool(v),
                    _ => return,
                };
                match b {
                    Ok(b) => {
                        seen = true;
                        if is_and {
                            result &= b;
                        } else {
                            result |= b;
                        }
                    }
                    Err(e) => error = Some(e),
                }
            });
        }
        if let Some(e) = error {
            return Err(e);
        }
        if !seen {
            return Err(ErrorKind::Value);
        }
        Ok(Value::Bool(result))
    })
}

fn and(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    fold(ctx, args, true)
}

fn or(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    fold(ctx, args, false)
}

pub(super) fn register(r: &mut Registry) {
    r.register(AND);
    r.register(OR);
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{ErrorKind, Value};

    fn sheet() -> MockEvalContext {
        let mut c = MockEvalContext::new();
        c.set(0, "A1", 1.0)
            .set(0, "A2", true)
            .set(0, "A3", "text")
            .set(0, "B1", 0.0)
            .set(0, "C1", Value::Error(ErrorKind::NA));
        c
    }

    #[test]
    fn and_or_ranges() {
        let mut c = sheet();
        let r = c.range(0, "A1:A3");
        assert_eq!(call(&mut c, "AND", vec![r.clone()]), Value::Bool(true));
        assert_eq!(call(&mut c, "OR", vec![r]), Value::Bool(true));
        let r = c.range(0, "A1:B1");
        assert_eq!(call(&mut c, "AND", vec![r.clone()]), Value::Bool(false));
        assert_eq!(call(&mut c, "OR", vec![r]), Value::Bool(true));
    }

    #[test]
    fn no_values_is_value_error() {
        let mut c = sheet();
        let r = c.range(0, "A3");
        assert_eq!(
            call(&mut c, "AND", vec![r.clone()]),
            Value::Error(ErrorKind::Value)
        );
        assert_eq!(call(&mut c, "OR", vec![r]), Value::Error(ErrorKind::Value));
        let r = c.range(0, "D1:D3");
        assert_eq!(call(&mut c, "OR", vec![r]), Value::Error(ErrorKind::Value));
    }

    #[test]
    fn direct_args_and_errors() {
        let mut c = sheet();
        assert_eq!(
            call(
                &mut c,
                "AND",
                vec![EvalValue::text("true"), EvalValue::number(2.0)]
            ),
            Value::Bool(true)
        );
        assert_eq!(
            call(&mut c, "OR", vec![EvalValue::text("abc")]),
            Value::Error(ErrorKind::Value)
        );
        let r = c.range(0, "C1");
        assert_eq!(
            call(&mut c, "OR", vec![EvalValue::bool(true), r]),
            Value::Error(ErrorKind::NA)
        );
    }
}
