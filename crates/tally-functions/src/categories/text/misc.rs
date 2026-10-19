//! CLEAN, EXACT, T and REPT.

use tally_types::{round_to_15_digits, ErrorKind, Value};

use crate::args::check_text_len;
use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry, ReturnKind};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const TEXT1: &[Param] = &[Param::required("text", ParamKind::Text)];
const EXACT_PARAMS: &[Param] = &[
    Param::required("text_1", ParamKind::Text),
    Param::required("text_2", ParamKind::Text),
];
const T_PARAMS: &[Param] = &[Param::required("value", ParamKind::Any)];
const REPT_PARAMS: &[Param] = &[
    Param::required("text", ParamKind::Text),
    Param::required("number", ParamKind::Number),
];

/// `CLEAN(text)`.
pub const CLEAN: FunctionDef = FunctionDef::new("CLEAN", Category::Text, TEXT1, clean)
    .returns(ReturnKind::Text)
    .help("Removes non-printable characters (code < 32).");
/// `EXACT(text_1; text_2)`.
pub const EXACT: FunctionDef = FunctionDef::new("EXACT", Category::Text, EXACT_PARAMS, exact)
    .returns(ReturnKind::Bool)
    .help("Case-sensitive comparison of two texts.");
/// `T(value)`.
pub const T: FunctionDef = FunctionDef::new("T", Category::Text, T_PARAMS, t_fn)
    .returns(ReturnKind::Text)
    .help("Returns the text itself, or an empty text for anything else.");
/// `REPT(text; number)`.
pub const REPT: FunctionDef = FunctionDef::new("REPT", Category::Text, REPT_PARAMS, rept)
    .returns(ReturnKind::Text)
    .help("Repeats a text.");

fn clean(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let s = a.text(0)?;
        Ok(Value::text(
            s.chars().filter(|&c| c >= ' ').collect::<String>(),
        ))
    })
}

fn exact(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let x = a.text(0)?;
        let y = a.text(1)?;
        Ok(Value::Bool(x == y))
    })
}

fn t_fn(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| match a.value(0) {
        Value::Error(e) => Err(e),
        Value::Text(s) => Ok(Value::Text(s)),
        _ => Ok(Value::text("")),
    })
}

fn rept(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let s = a.text(0)?;
        let n = round_to_15_digits(a.num(1)?).trunc();
        if n < 0.0 {
            return Err(ErrorKind::InvalidArgument);
        }
        if s.is_empty() || n == 0.0 {
            return Ok(Value::text(""));
        }
        // Check before allocating; `n` may be astronomically large.
        let total = (s.len() as f64) * n;
        if total > crate::args::MAX_TEXT_BYTES as f64 {
            return Err(ErrorKind::StringOverflow);
        }
        let n = n as usize;
        check_text_len(s.len().saturating_mul(n))?;
        Ok(Value::text(s.repeat(n)))
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(CLEAN);
    r.register(EXACT);
    r.register(T);
    r.register(REPT);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::{call, MockEvalContext};

    fn n(x: f64) -> EvalValue {
        EvalValue::number(x)
    }
    fn s(x: &str) -> EvalValue {
        EvalValue::text(x)
    }
    fn t(name: &str, a: Vec<EvalValue>) -> Value {
        call(&mut MockEvalContext::new(), name, a)
    }

    #[test]
    fn clean_exact() {
        assert_eq!(t("CLEAN", vec![s("a\u{1}b\nc\td")]), Value::text("abcd"));
        assert_eq!(t("EXACT", vec![s("a"), s("a")]), Value::Bool(true));
        assert_eq!(t("EXACT", vec![s("a"), s("A")]), Value::Bool(false));
        assert_eq!(t("EXACT", vec![n(1.0), s("1")]), Value::Bool(true));
    }

    #[test]
    fn t_fn() {
        assert_eq!(t("T", vec![s("x")]), Value::text("x"));
        assert_eq!(t("T", vec![n(1.0)]), Value::text(""));
        assert_eq!(t("T", vec![EvalValue::bool(true)]), Value::text(""));
        assert_eq!(
            t("T", vec![EvalValue::error(ErrorKind::NA)]),
            Value::Error(ErrorKind::NA)
        );
    }

    #[test]
    fn rept() {
        assert_eq!(t("REPT", vec![s("ab"), n(3.0)]), Value::text("ababab"));
        assert_eq!(t("REPT", vec![s("ab"), n(0.0)]), Value::text(""));
        assert_eq!(
            t("REPT", vec![s("ab"), n(-1.0)]),
            Value::Error(ErrorKind::InvalidArgument)
        );
        assert_eq!(
            t("REPT", vec![s("ab"), n(1e12)]),
            Value::Error(ErrorKind::StringOverflow)
        );
        assert_eq!(t("REPT", vec![s(""), n(1e12)]), Value::text(""));
        assert_eq!(
            t("REPT", vec![s("x"), n(4_194_305.0)]),
            Value::Error(ErrorKind::StringOverflow)
        );
    }
}
