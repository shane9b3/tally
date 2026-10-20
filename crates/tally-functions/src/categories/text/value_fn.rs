//! VALUE.

use tally_numfmt::{recognize_input_with, Locale, RecognizeContext};
use tally_types::{ErrorKind, Value};

use crate::coerce::locale_of;

use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry, ReturnKind};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const VALUE_PARAMS: &[Param] = &[Param::required("text", ParamKind::Number)];

/// `VALUE(text)`.
pub const VALUE: FunctionDef = FunctionDef::new("VALUE", Category::Text, VALUE_PARAMS, value)
    .returns(ReturnKind::Number)
    .help("Converts text to a number.");

/// Converts text with the full input recognizer of the document locale
/// (thousands separators, currency, percent, dates, times in that
/// locale's spelling; AOO `ScValue` → `IsNumberFormat`), independent of
/// `settings.text_to_number`. Only a numeric result is accepted.
pub(super) fn text_to_value(
    s: &str,
    epoch: tally_types::DateEpoch,
    locale: &Locale,
) -> Option<(f64, Option<String>)> {
    let ctx = RecognizeContext::now(epoch);
    let rec = recognize_input_with(s, locale, &ctx);
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
        // AOO pushes an illegal argument (Err:502) for empty text.
        Value::Text(s) if s.trim().is_empty() => Err(ErrorKind::InvalidArgument),
        Value::Text(s) => {
            let epoch = a.ctx().settings().date_epoch;
            let locale = locale_of(a.ctx());
            text_to_value(&s, epoch, locale)
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

    /// Review 3 H2: the document locale decides the spelling (AOO uses
    /// the document's number formatter).
    #[test]
    fn document_locale() {
        let mut c = MockEvalContext::new();
        c.set_locale("de-DE");
        let mut v = |x: &str| call(&mut c, "VALUE", vec![EvalValue::text(x)]);
        assert_eq!(v("1,7"), Value::Number(1.7));
        assert_eq!(v("1.234,5"), Value::Number(1234.5));
        assert_eq!(v("19.10.2026"), Value::Number(46314.0));
        assert_eq!(v("2026-10-19"), Value::Number(46314.0));
        assert_eq!(v("12,5%"), Value::Number(0.125));
        assert_eq!(v("1,5 €"), Value::Number(1.5));
        // en-US spellings are not German numbers: `1.7` is 1 July.
        assert_eq!(v("1.7"), Value::Number(46204.0));
        assert_eq!(v("$1,234.50"), Value::Error(ErrorKind::Value));
        let mut c = MockEvalContext::new();
        c.set_locale("fr-FR");
        assert_eq!(
            call(&mut c, "VALUE", vec![EvalValue::text("1\u{a0}234,5")]),
            Value::Number(1234.5)
        );
    }

    #[test]
    fn passthrough_and_errors() {
        assert_eq!(v(EvalValue::number(3.0)), Value::Number(3.0));
        assert_eq!(v(EvalValue::bool(true)), Value::Number(1.0));
        assert_eq!(v(EvalValue::bool(false)), Value::Number(0.0));
        assert_eq!(v(EvalValue::Missing), Value::Number(0.0));
        let bad = Value::Error(ErrorKind::Value);
        assert_eq!(v(EvalValue::text("abc")), bad);
        assert_eq!(
            v(EvalValue::text("")),
            Value::Error(ErrorKind::InvalidArgument)
        );
        assert_eq!(v(EvalValue::text("TRUE")), bad);
        assert_eq!(
            v(EvalValue::error(ErrorKind::Div0)),
            Value::Error(ErrorKind::Div0)
        );
    }
}
