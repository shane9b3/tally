//! DATEVALUE.

use tally_numfmt::{recognize_input_with, Locale, RecognizeContext};
use tally_types::{ErrorKind, Value};

use crate::coerce::locale_of;

use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry, ReturnKind};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const PARAMS: &[Param] = &[Param::required("text", ParamKind::Text)];

/// `DATEVALUE(text)`: only text is accepted; the text must be recognized as
/// a date or date-time in the document locale (the time part is dropped).
pub const DATEVALUE: FunctionDef =
    FunctionDef::new("DATEVALUE", Category::DateTime, PARAMS, datevalue)
        .returns(ReturnKind::Number)
        .help("Date serial of a date given as text.");

fn recognize(s: &str, ctx: &RecognizeContext, locale: &Locale) -> Option<(f64, Option<String>)> {
    let rec = recognize_input_with(s, locale, ctx);
    match rec.value {
        Value::Number(n) => Some((n, rec.implied_format)),
        _ => None,
    }
}

fn datevalue(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| match a.value(0) {
        Value::Error(e) => Err(e),
        Value::Text(s) => {
            let rc = RecognizeContext::for_settings(a.ctx().settings());
            let locale = locale_of(a.ctx());
            let (n, fmt) = recognize(&s, &rc, locale).ok_or(ErrorKind::Value)?;
            // Date formats contain a day or year code; time-only ("HH:MM")
            // and plain numbers do not.
            let is_date = fmt.is_some_and(|f| f.contains(['D', 'Y']));
            if is_date {
                Ok(Value::Number(n.floor()))
            } else {
                // A number or time that is not a date: AOO's
                // PushIllegalArgument (Err:502).
                Err(ErrorKind::InvalidArgument)
            }
        }
        _ => Err(ErrorKind::Value),
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(DATEVALUE);
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{DateEpoch, ErrorKind, Value};

    fn dv(v: EvalValue) -> Value {
        call(&mut MockEvalContext::new(), "DATEVALUE", vec![v])
    }

    #[test]
    fn dates() {
        let s = |x: &str| dv(EvalValue::text(x));
        assert_eq!(s("2026-10-19"), Value::Number(46314.0));
        assert_eq!(s("10/19/2026"), Value::Number(46314.0));
        assert_eq!(s("2026-10-19 18:00"), Value::Number(46314.0));
        assert_eq!(s("1899-12-30"), Value::Number(0.0));
    }

    #[test]
    fn rejects() {
        let bad = Value::Error(ErrorKind::Value);
        assert_eq!(dv(EvalValue::text("abc")), bad);
        // Recognized, but not a date: Err:502 (AOO).
        let illegal = Value::Error(ErrorKind::InvalidArgument);
        assert_eq!(dv(EvalValue::text("14:30")), illegal);
        assert_eq!(dv(EvalValue::text("42")), illegal);
        assert_eq!(dv(EvalValue::text("")), bad);
        assert_eq!(dv(EvalValue::number(46314.0)), bad);
        assert_eq!(dv(EvalValue::bool(true)), bad);
        assert_eq!(
            dv(EvalValue::error(ErrorKind::NA)),
            Value::Error(ErrorKind::NA)
        );
    }

    /// Review 3 H2: German spellings under de-DE.
    #[test]
    fn document_locale() {
        let mut c = MockEvalContext::new();
        c.set_locale("de-DE");
        let mut dv = |x: &str| call(&mut c, "DATEVALUE", vec![EvalValue::text(x)]);
        assert_eq!(dv("19.10.2026"), Value::Number(46314.0));
        assert_eq!(dv("1.7.2026"), Value::Number(46204.0));
        assert_eq!(dv("19. Oktober 2026"), Value::Number(46314.0));
        assert_eq!(dv("2026-10-19"), Value::Number(46314.0));
        // Month-day-year is not a German date.
        assert_eq!(dv("10/19/2026"), Value::Error(ErrorKind::Value));
        let mut c = MockEvalContext::new();
        c.set_locale("en-GB");
        assert_eq!(
            call(&mut c, "DATEVALUE", vec![EvalValue::text("19/10/2026")]),
            Value::Number(46314.0)
        );
    }

    #[test]
    fn epoch() {
        let mut c = MockEvalContext::new();
        c.settings.date_epoch = DateEpoch::D1904;
        let v = call(&mut c, "DATEVALUE", vec![EvalValue::text("1904-01-02")]);
        assert_eq!(v, Value::Number(1.0));
    }
}
