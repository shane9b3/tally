//! XOR: TRUE when an odd number of the logical values are TRUE.

use tally_types::{ErrorKind, Value};

use crate::coerce::{value_to_bool, SeqSource};
use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry, ReturnKind};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const PARAMS: &[Param] = &[Param::repeat("logical_value", ParamKind::NumberSeq)];

/// `XOR(logical_value; ...)`.
pub const XOR: FunctionDef = FunctionDef::new("XOR", Category::Logical, PARAMS, xor)
    .returns(ReturnKind::Bool)
    .help("Returns TRUE if an odd number of arguments are TRUE.");

/// Same operand rules as AND/OR: direct arguments are coerced as booleans,
/// text and empties inside references are skipped, the first error wins,
/// and no logical value at all is `#VALUE!`.
fn xor(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let mut seen = false;
        let mut result = false;
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
                        result ^= b;
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

pub(super) fn register(r: &mut Registry) {
    r.register(XOR);
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{ErrorKind, Value};

    fn b(x: bool) -> EvalValue {
        EvalValue::bool(x)
    }

    #[test]
    fn xor() {
        let mut c = MockEvalContext::new();
        assert_eq!(
            call(&mut c, "XOR", vec![b(true), b(false)]),
            Value::Bool(true)
        );
        assert_eq!(
            call(&mut c, "XOR", vec![b(true), b(true)]),
            Value::Bool(false)
        );
        assert_eq!(
            call(&mut c, "XOR", vec![b(true), b(true), b(true)]),
            Value::Bool(true)
        );
        assert_eq!(
            call(&mut c, "XOR", vec![EvalValue::text("x")]),
            Value::Error(ErrorKind::Value)
        );
    }

    #[test]
    fn range_rules() {
        let mut c = MockEvalContext::new();
        c.set(0, "A1", 1.0).set(0, "A2", "text").set(0, "A3", true);
        let r = c.range(0, "A1:A3");
        assert_eq!(call(&mut c, "XOR", vec![r]), Value::Bool(false));
        let r = c.range(0, "B1:B3");
        assert_eq!(call(&mut c, "XOR", vec![r]), Value::Error(ErrorKind::Value));
    }
}
