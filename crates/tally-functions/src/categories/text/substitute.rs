//! SUBSTITUTE.

use tally_types::{ErrorKind, Value};

use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry, ReturnKind};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const PARAMS: &[Param] = &[
    Param::required("text", ParamKind::Text),
    Param::required("search_text", ParamKind::Text),
    Param::required("new_text", ParamKind::Text),
    Param::optional("occurrence", ParamKind::Integer),
];

/// `SUBSTITUTE(text; search_text; new_text [; occurrence])`: case-sensitive;
/// without `occurrence` every match is replaced.
pub const SUBSTITUTE: FunctionDef =
    FunctionDef::new("SUBSTITUTE", Category::Text, PARAMS, substitute)
        .returns(ReturnKind::Text)
        .help("Replaces occurrences of a text within a text.");

fn substitute(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let text = a.text(0)?;
        let from = a.text(1)?;
        let to = a.text(2)?;
        let nth = if a.is_missing(3) {
            None
        } else {
            let n = a.int(3)?;
            if n < 1 {
                return Err(ErrorKind::InvalidArgument);
            }
            Some(usize::try_from(n).unwrap_or(usize::MAX))
        };
        if from.is_empty() {
            return Ok(Value::Text(text));
        }
        match nth {
            None => Ok(Value::text(text.replace(&*from, &to))),
            Some(n) => match text.match_indices(&*from).nth(n - 1) {
                None => Ok(Value::Text(text)),
                Some((i, _)) => {
                    let mut out = String::with_capacity(text.len() + to.len());
                    out.push_str(&text[..i]);
                    out.push_str(&to);
                    out.push_str(&text[i + from.len()..]);
                    Ok(Value::text(out))
                }
            },
        }
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(SUBSTITUTE);
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{ErrorKind, Value};

    fn s(x: &str) -> EvalValue {
        EvalValue::text(x)
    }
    fn t(args: Vec<EvalValue>) -> Value {
        call(&mut MockEvalContext::new(), "SUBSTITUTE", args)
    }

    #[test]
    fn substitute() {
        assert_eq!(t(vec![s("a-b-c"), s("-"), s("+")]), Value::text("a+b+c"));
        assert_eq!(
            t(vec![s("a-b-c"), s("-"), s("+"), EvalValue::number(2.0)]),
            Value::text("a-b+c")
        );
        assert_eq!(
            t(vec![s("a-b"), s("-"), s("+"), EvalValue::number(5.0)]),
            Value::text("a-b")
        );
        assert_eq!(t(vec![s("Abc"), s("a"), s("x")]), Value::text("Abc"));
        assert_eq!(t(vec![s("abc"), s(""), s("x")]), Value::text("abc"));
        assert_eq!(t(vec![s("aaa"), s("aa"), s("b")]), Value::text("ba"));
        assert_eq!(t(vec![s("héé"), s("é"), s("e")]), Value::text("hee"));
        assert_eq!(t(vec![s("abc"), s("b"), s("")]), Value::text("ac"));
    }

    #[test]
    fn errors() {
        assert_eq!(
            t(vec![s("a"), s("a"), s("b"), EvalValue::number(0.0)]),
            Value::Error(ErrorKind::InvalidArgument)
        );
        assert_eq!(
            t(vec![EvalValue::error(ErrorKind::NA), s("a"), s("b")]),
            Value::Error(ErrorKind::NA)
        );
        assert_eq!(
            t(vec![s("a"), s("a")]),
            Value::Error(ErrorKind::MissingVariable2)
        );
    }
}
