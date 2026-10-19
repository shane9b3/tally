//! TEXT.

use tally_numfmt::{format_value_with, parse_format, Locale};
use tally_types::{ErrorKind, Value};

use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry, ReturnKind};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const TEXT_PARAMS: &[Param] = &[
    Param::required("value", ParamKind::Number),
    Param::required("format", ParamKind::Text),
];

/// `TEXT(value; format_code)`.
pub const TEXT: FunctionDef = FunctionDef::new("TEXT", Category::Text, TEXT_PARAMS, text)
    .returns(ReturnKind::Text)
    .help("Formats a value with a number format code.");

/// Booleans and empty cells are numbers in AOO's storage model, so they
/// format as 1/0 and 0; text goes through the format's text section.
fn text(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let v = match a.value(0) {
            Value::Error(e) => return Err(e),
            Value::Bool(b) => Value::Number(if b { 1.0 } else { 0.0 }),
            Value::Empty => Value::Number(0.0),
            other => other,
        };
        let code = a.text(1)?;
        let ast = parse_format(&code).map_err(|_| ErrorKind::InvalidArgument)?;
        let epoch = a.ctx().settings().date_epoch;
        let out = format_value_with(&v, &ast, &Locale::en_us(), None, epoch);
        Ok(Value::text(out.text))
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(TEXT);
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{DateEpoch, ErrorKind, Value};

    fn t(v: EvalValue, f: &str) -> Value {
        call(
            &mut MockEvalContext::new(),
            "TEXT",
            vec![v, EvalValue::text(f)],
        )
    }

    #[test]
    fn numbers() {
        assert_eq!(
            t(EvalValue::number(1234.5), "#,##0.00"),
            Value::text("1,234.50")
        );
        assert_eq!(t(EvalValue::number(0.5), "0%"), Value::text("50%"));
        assert_eq!(t(EvalValue::number(1.2345), "0.00"), Value::text("1.23"));
        assert_eq!(t(EvalValue::bool(true), "0"), Value::text("1"));
        assert_eq!(t(EvalValue::Missing, "0.0"), Value::text("0.0"));
    }

    #[test]
    fn dates() {
        assert_eq!(
            t(EvalValue::number(46314.0), "YYYY-MM-DD"),
            Value::text("2026-10-19")
        );
        let mut c = MockEvalContext::new();
        c.settings.date_epoch = DateEpoch::D1904;
        let v = call(
            &mut c,
            "TEXT",
            vec![EvalValue::number(0.0), EvalValue::text("YYYY-MM-DD")],
        );
        assert_eq!(v, Value::text("1904-01-01"));
    }

    #[test]
    fn text_and_errors() {
        assert_eq!(t(EvalValue::text("abc"), "0.00"), Value::text("abc"));
        assert_eq!(
            t(EvalValue::error(ErrorKind::NA), "0"),
            Value::Error(ErrorKind::NA)
        );
        assert_eq!(
            t(EvalValue::number(1.0), "[[["),
            Value::Error(ErrorKind::InvalidArgument)
        );
    }
}
