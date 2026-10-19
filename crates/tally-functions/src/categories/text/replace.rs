//! REPLACE.

use tally_types::{ErrorKind, Value};

use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry, ReturnKind};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const PARAMS: &[Param] = &[
    Param::required("text", ParamKind::Text),
    Param::required("position", ParamKind::Integer),
    Param::required("length", ParamKind::Integer),
    Param::required("new_text", ParamKind::Text),
];

/// `REPLACE(text; position; length; new_text)`.
pub const REPLACE: FunctionDef = FunctionDef::new("REPLACE", Category::Text, PARAMS, replace)
    .returns(ReturnKind::Text)
    .help("Replaces part of a text with another text.");

/// Positions count Unicode scalar values. `position < 1` or `length < 0` is
/// `Err:502`; a position past the end appends the new text.
fn replace(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let s = a.text(0)?;
        let pos = a.int(1)?;
        let len = a.int(2)?;
        let new = a.text(3)?;
        if pos < 1 || len < 0 {
            return Err(ErrorKind::InvalidArgument);
        }
        let skip = usize::try_from(pos - 1).unwrap_or(usize::MAX);
        let del = usize::try_from(len).unwrap_or(usize::MAX);
        let mut out = String::with_capacity(s.len() + new.len());
        let mut chars = s.chars();
        out.extend(chars.by_ref().take(skip));
        for _ in 0..del {
            if chars.next().is_none() {
                break;
            }
        }
        out.push_str(&new);
        out.extend(chars);
        Ok(Value::text(out))
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(REPLACE);
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{ErrorKind, Value};

    fn r(s: &str, p: f64, l: f64, n: &str) -> Value {
        call(
            &mut MockEvalContext::new(),
            "REPLACE",
            vec![
                EvalValue::text(s),
                EvalValue::number(p),
                EvalValue::number(l),
                EvalValue::text(n),
            ],
        )
    }

    #[test]
    fn replace() {
        assert_eq!(r("abcdef", 2.0, 3.0, "X"), Value::text("aXef"));
        assert_eq!(r("abcdef", 1.0, 0.0, "X"), Value::text("Xabcdef"));
        assert_eq!(r("abc", 10.0, 2.0, "X"), Value::text("abcX"));
        assert_eq!(r("abc", 2.0, 99.0, ""), Value::text("a"));
        assert_eq!(r("héllo", 2.0, 1.0, "e"), Value::text("hello"));
        assert_eq!(
            r("abc", 0.0, 1.0, "X"),
            Value::Error(ErrorKind::InvalidArgument)
        );
        assert_eq!(
            r("abc", 1.0, -1.0, "X"),
            Value::Error(ErrorKind::InvalidArgument)
        );
    }
}
