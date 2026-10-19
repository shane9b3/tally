//! VALUE.

use tally_numfmt::{recognize_input_with, Locale, RecognizeContext};
use tally_types::{ErrorKind, Value};

use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry, ReturnKind};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const VALUE_PARAMS: &[Param] = &[Param::required("text", ParamKind::Number)];

/// `VALUE(text)`.
pub const VALUE: FunctionDef = FunctionDef::new("VALUE", Category::Text, VALUE_PARAMS, value)
    .returns(ReturnKind::Number)
    .help("Converts text to a number.");

/// Converts text with the full en-US input recognizer (thousands
/// separators, currency, percent, dates, times), independent of
/// `settings.text_to_number`. Only a numeric result is accepted.
pub(super) fn text_to_value(
    s: &str,
    epoch: tally_types::DateEpoch,
) -> Option<(f64, Option<String>)> {
    let ctx = RecognizeContext::now(epoch);
    let rec = recognize_input_with(s, &Locale::en_us(), &ctx);
    match rec.value {
        Value::Number(n) => Some((n, rec.implied_format)),
        _ => None,
    }
}

fn value(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| match a.value(0) {
        Value::Error(e) => Err(e),
        Value::Number(n) => Ok(Value::Number(n)),
        Value::Bool(b) => Ok(Value::Number(if b { 1.0 } else { 0.0 })),
        Value::Empty => Ok(Value::Number(0.0)),
        Value::Text(s) => {
            let epoch = a.ctx().settings().date_epoch;
            text_to_value(&s, epoch)
                .map(|(n, _)| Value::Number(n))
                .ok_or(ErrorKind::Value)
        }
        _ => Err(ErrorKind::Value),
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(VALUE);
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{ErrorKind, Value};

    fn v(x: EvalValue) -> Value {
        call(&mut MockEvalContext::new(), "VALUE", vec![x])
    }

    #[test]
    fn conversions() {
        let s = |x: &str| v(EvalValue::text(x));
        assert_eq!(s("12.5"), Value::Number(12.5));
        assert_eq!(s("$1,234.50"), Value::Number(1234.5));
        assert_eq!(s("15%"), Value::Number(0.15));
        assert_eq!(s("10/19/2026"), Value::Number(46314.0));
        assert!(matches!(s("14:30"), Value::Number(x) if (x - 870.0 / 1440.0).abs() < 1e-12));
        assert_eq!(s(" 7 "), Value::Number(7.0));
        assert_eq!(s("2026-10-19"), Value::Number(46314.0));
    }

    #[test]
    fn passthrough_and_errors() {
        assert_eq!(v(EvalValue::number(3.0)), Value::Number(3.0));
        assert_eq!(v(EvalValue::bool(true)), Value::Number(1.0));
        assert_eq!(v(EvalValue::bool(false)), Value::Number(0.0));
        assert_eq!(v(EvalValue::Missing), Value::Number(0.0));
        let bad = Value::Error(ErrorKind::Value);
        assert_eq!(v(EvalValue::text("abc")), bad);
        assert_eq!(v(EvalValue::text("")), bad);
        assert_eq!(v(EvalValue::text("TRUE")), bad);
        assert_eq!(
            v(EvalValue::error(ErrorKind::Div0)),
            Value::Error(ErrorKind::Div0)
        );
    }
}
