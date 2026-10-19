//! LEFT, RIGHT and MID. Counts are Unicode scalar values (not bytes or
//! UTF-16 units).

use tally_types::{ErrorKind, Value};

use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry, ReturnKind};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const LR_PARAMS: &[Param] = &[
    Param::required("text", ParamKind::Text),
    Param::optional("number", ParamKind::Integer),
];
const MID_PARAMS: &[Param] = &[
    Param::required("text", ParamKind::Text),
    Param::required("start", ParamKind::Integer),
    Param::required("number", ParamKind::Integer),
];

/// `LEFT(text [; number])`.
pub const LEFT: FunctionDef = FunctionDef::new("LEFT", Category::Text, LR_PARAMS, left)
    .returns(ReturnKind::Text)
    .help("Leading characters of a text.");
/// `RIGHT(text [; number])`.
pub const RIGHT: FunctionDef = FunctionDef::new("RIGHT", Category::Text, LR_PARAMS, right)
    .returns(ReturnKind::Text)
    .help("Trailing characters of a text.");
/// `MID(text; start; number)`.
pub const MID: FunctionDef = FunctionDef::new("MID", Category::Text, MID_PARAMS, mid)
    .returns(ReturnKind::Text)
    .help("Characters from the middle of a text.");

fn count(n: i64) -> Result<usize, ErrorKind> {
    if n < 0 {
        return Err(ErrorKind::InvalidArgument);
    }
    Ok(usize::try_from(n).unwrap_or(usize::MAX))
}

fn left(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let s = a.text(0)?;
        let n = count(a.int_opt(1, 1)?)?;
        Ok(Value::text(s.chars().take(n).collect::<String>()))
    })
}

fn right(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let s = a.text(0)?;
        let n = count(a.int_opt(1, 1)?)?;
        let len = s.chars().count();
        Ok(Value::text(
            s.chars().skip(len.saturating_sub(n)).collect::<String>(),
        ))
    })
}

fn mid(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let s = a.text(0)?;
        let start = a.int(1)?;
        let n = count(a.int(2)?)?;
        if start < 1 {
            return Err(ErrorKind::InvalidArgument);
        }
        let skip = usize::try_from(start - 1).unwrap_or(usize::MAX);
        Ok(Value::text(
            s.chars().skip(skip).take(n).collect::<String>(),
        ))
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(LEFT);
    r.register(RIGHT);
    r.register(MID);
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{ErrorKind, Value};

    fn t(name: &str, args: Vec<EvalValue>) -> Value {
        call(&mut MockEvalContext::new(), name, args)
    }
    fn s(x: &str) -> EvalValue {
        EvalValue::text(x)
    }
    fn n(x: f64) -> EvalValue {
        EvalValue::number(x)
    }

    #[test]
    fn left_right() {
        assert_eq!(t("LEFT", vec![s("hello"), n(2.0)]), Value::text("he"));
        assert_eq!(t("LEFT", vec![s("hello")]), Value::text("h"));
        assert_eq!(t("LEFT", vec![s("hello"), n(0.0)]), Value::text(""));
        assert_eq!(t("LEFT", vec![s("hi"), n(9.0)]), Value::text("hi"));
        assert_eq!(t("RIGHT", vec![s("hello"), n(2.0)]), Value::text("lo"));
        assert_eq!(t("RIGHT", vec![s("hello")]), Value::text("o"));
        assert_eq!(t("RIGHT", vec![s("hi"), n(9.0)]), Value::text("hi"));
        assert_eq!(t("LEFT", vec![n(12345.0), n(2.0)]), Value::text("12"));
    }

    #[test]
    fn unicode_chars() {
        assert_eq!(t("LEFT", vec![s("héllo"), n(2.0)]), Value::text("hé"));
        assert_eq!(t("RIGHT", vec![s("a😀b"), n(2.0)]), Value::text("😀b"));
        assert_eq!(t("MID", vec![s("a😀b"), n(2.0), n(1.0)]), Value::text("😀"));
    }

    #[test]
    fn mid() {
        assert_eq!(
            t("MID", vec![s("hello"), n(2.0), n(3.0)]),
            Value::text("ell")
        );
        assert_eq!(
            t("MID", vec![s("hello"), n(4.0), n(99.0)]),
            Value::text("lo")
        );
        assert_eq!(t("MID", vec![s("hello"), n(9.0), n(2.0)]), Value::text(""));
    }

    #[test]
    fn errors() {
        let bad = Value::Error(ErrorKind::InvalidArgument);
        assert_eq!(t("LEFT", vec![s("a"), n(-1.0)]), bad);
        assert_eq!(t("RIGHT", vec![s("a"), n(-1.0)]), bad);
        assert_eq!(t("MID", vec![s("a"), n(0.0), n(1.0)]), bad);
        assert_eq!(t("MID", vec![s("a"), n(1.0), n(-1.0)]), bad);
        assert_eq!(
            t("LEFT", vec![EvalValue::error(ErrorKind::Div0)]),
            Value::Error(ErrorKind::Div0)
        );
        assert_eq!(t("LEFT", vec![]), Value::Error(ErrorKind::MissingVariable2));
    }
}
