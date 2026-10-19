//! SUBSTITUTE.

use tally_types::{ErrorKind, Value};

use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry, ReturnKind};
use crate::{check_text_len, run, ArgList, Args, EvalContext, EvalValue};

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
            None => {
                // Size the result before building it (Err:513 past the cap).
                if to.len() > from.len() {
                    let n = text.matches(&*from).count();
                    let grow = n.saturating_mul(to.len() - from.len());
                    check_text_len(text.len().saturating_add(grow))?;
                }
                Ok(Value::text(text.replace(&*from, &to)))
            }
            Some(n) => match nth_overlapping(&text, &from, n) {
                None => Ok(Value::Text(text)),
                Some(i) => {
                    check_text_len((text.len() - from.len()).saturating_add(to.len()))?;
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

/// Byte offset of the `n`th (1-based) occurrence of `from` in `text`,
/// counting overlapping occurrences: after a match that is not the `n`th,
/// the search resumes one character later, as in AOO's `ScSubstitute`
/// (`nPos++`). `SUBSTITUTE("aaaa";"aa";"b";2)` is therefore `"aba"`.
fn nth_overlapping(text: &str, from: &str, n: usize) -> Option<usize> {
    let mut pos = 0;
    let mut count = 0;
    while let Some(off) = text.get(pos..)?.find(from) {
        let i = pos + off;
        count += 1;
        if count == n {
            return Some(i);
        }
        pos = i + text[i..].chars().next().map_or(1, char::len_utf8);
    }
    None
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
        // The nth occurrence counts overlapping matches (AOO).
        let n = EvalValue::number;
        assert_eq!(
            t(vec![s("aaaa"), s("aa"), s("b"), n(2.0)]),
            Value::text("aba")
        );
        assert_eq!(
            t(vec![s("aaaa"), s("aa"), s("b"), n(3.0)]),
            Value::text("aab")
        );
        assert_eq!(
            t(vec![s("aaaa"), s("aa"), s("b"), n(4.0)]),
            Value::text("aaaa")
        );
        assert_eq!(
            t(vec![s("ééé"), s("éé"), s("x"), n(2.0)]),
            Value::text("éx")
        );
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
