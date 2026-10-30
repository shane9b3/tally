//! TEXT.

use tally_numfmt::{format_value_in_locale, parse_format_with_locale};
use tally_types::{ErrorKind, Value};

use crate::coerce::locale_of;

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
///
/// The format code is read in the document locale
/// ([`parse_format_with_locale`]: `"0,00"` and `"TT.MM.JJJJ"` under de-DE,
/// English keywords and the en-US code under en-US) and the result is
/// rendered in it, as AOO's `ScText` → `GetPreviewStringGuess` does with
/// the cell language. An unparsable code is `Err:502`.
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
        let locale = locale_of(a.ctx());
        let ast =
            parse_format_with_locale(&code, locale).map_err(|_| ErrorKind::InvalidArgument)?;
        let epoch = a.ctx().settings().date_epoch;
        // The document locale: a `[$-407]` in the code only sets the NatNum
        // language in AOO (zformat.cxx), Excel ignores it for numbers.
        let out = format_value_in_locale(&v, &ast, locale, None, epoch);
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
        // Under en-US a German code is read with en-US separators.
        assert_eq!(t(EvalValue::number(1.7), "0,00"), Value::text("002"));
        assert_eq!(t(EvalValue::number(1.2345), "0.00"), Value::text("1.23"));
        assert_eq!(t(EvalValue::bool(true), "0"), Value::text("1"));
        assert_eq!(t(EvalValue::Missing, "0.0"), Value::text("0.0"));
        // A language tag does not change the separators (AOO, Excel).
        assert_eq!(
            t(EvalValue::number(1234.5), "[$-407]#,##0.00"),
            Value::text("1,234.50")
        );
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

    /// Review 3 M6: codes are read and rendered in the document locale.
    #[test]
    fn document_locale() {
        let mut c = MockEvalContext::new();
        c.set_locale("de-DE");
        let mut t = |v: EvalValue, f: &str| call(&mut c, "TEXT", vec![v, EvalValue::text(f)]);
        assert_eq!(t(EvalValue::number(1.7), "0,00"), Value::text("1,70"));
        assert_eq!(
            t(EvalValue::number(1234.5), "#.##0,00"),
            Value::text("1.234,50")
        );
        // The en-US code under de-DE: "." groups and "," is the point.
        assert_eq!(t(EvalValue::number(1.7), "0.00"), Value::text("002"));
        assert_eq!(
            t(EvalValue::number(46314.0), "TT.MM.JJJJ"),
            Value::text("19.10.2026")
        );
        assert_eq!(
            t(EvalValue::number(46314.0), "DD.MM.YYYY"),
            Value::text("19.10.2026")
        );
        assert_eq!(
            t(EvalValue::number(46314.0), "NN, T. MMMM JJJJ"),
            Value::text("Mo, 19. Oktober 2026")
        );
        assert_eq!(t(EvalValue::number(0.5), "0,0%"), Value::text("50,0%"));
        assert_eq!(t(EvalValue::bool(true), "General"), Value::text("1"));
        let mut c = MockEvalContext::new();
        c.set_locale("fr-FR");
        assert_eq!(
            call(
                &mut c,
                "TEXT",
                vec![EvalValue::number(1234.5), EvalValue::text("# ##0,00")]
            ),
            Value::text("1\u{a0}234,50")
        );
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
