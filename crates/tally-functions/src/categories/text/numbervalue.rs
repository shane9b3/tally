//! NUMBERVALUE.
//!
//! Declared but unimplemented in AOO 4.1, so Tally registers it as an
//! extension with the ODFF 1.2 / Excel 2013 signature
//! `NUMBERVALUE(text; [decimal_separator]; [group_separator])` (master-list
//! override in `docs/functions/gen-master-list.py`). Semantics follow Excel
//! and LibreOffice: whitespace is ignored, group separators are dropped
//! before the decimal separator, trailing `%` divide by 100, empty text is
//! 0, only the first character of a separator argument counts, and an empty
//! separator argument is `#VALUE!`. The defaults are the document locale's
//! separators (`.` and `,` under en-US, `,` and `.` under de-DE); when only
//! the decimal separator is given, the group separator is `.` for a `,`
//! decimal and `,` otherwise.

use tally_types::{ErrorKind, Value};

use crate::coerce::locale_of;

use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry, ReturnKind};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const PARAMS: &[Param] = &[
    Param::required("text", ParamKind::Text),
    Param::optional("decimal_separator", ParamKind::Text),
    Param::optional("group_separator", ParamKind::Text),
];

/// `NUMBERVALUE(text; [decimal_separator]; [group_separator])`.
pub const NUMBERVALUE: FunctionDef =
    FunctionDef::new("NUMBERVALUE", Category::Text, PARAMS, numbervalue)
        .returns(ReturnKind::Number)
        .extension()
        .help("Converts text to a number using explicit separators.");

/// Parses `text` with the given separators (`None` group separator means
/// "," or "." when the decimal separator is ",").
pub(super) fn number_value(
    text: &str,
    decimal: char,
    group: Option<char>,
) -> Result<f64, ErrorKind> {
    let group = group.unwrap_or(if decimal == ',' { '.' } else { ',' });
    if group == decimal {
        return Err(ErrorKind::Value);
    }
    let mut body: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    let mut percent = 0;
    while body.ends_with('%') {
        body.pop();
        percent += 1;
    }
    if body.is_empty() {
        return Ok(0.0);
    }
    let mut norm = String::with_capacity(body.len());
    let mut seen_decimal = false;
    for c in body.chars() {
        if c == decimal {
            if seen_decimal {
                return Err(ErrorKind::Value);
            }
            seen_decimal = true;
            norm.push('.');
        } else if c == group {
            if seen_decimal {
                return Err(ErrorKind::Value);
            }
        } else if c.is_ascii_digit() || matches!(c, '+' | '-' | 'e' | 'E') {
            norm.push(c);
        } else {
            return Err(ErrorKind::Value);
        }
    }
    let mut n: f64 = norm.parse().map_err(|_| ErrorKind::Value)?;
    for _ in 0..percent {
        n /= 100.0;
    }
    Ok(n)
}

fn numbervalue(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let text = a.text(0)?;
        let sep = |a: &mut Args, i: usize| -> Result<Option<char>, ErrorKind> {
            if a.is_missing(i) {
                return Ok(None);
            }
            a.text(i)?.chars().next().map(Some).ok_or(ErrorKind::Value)
        };
        let locale = locale_of(a.ctx());
        let explicit_decimal = sep(&mut a, 1)?;
        let decimal = explicit_decimal.unwrap_or(locale.decimal_sep);
        let group = match sep(&mut a, 2)? {
            Some(g) => Some(g),
            None if explicit_decimal.is_none() => Some(locale.group_sep),
            None => None,
        };
        Ok(Value::Number(number_value(&text, decimal, group)?))
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(NUMBERVALUE);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::{call, MockEvalContext};

    fn nv(s: &str, d: &str) -> Value {
        call(
            &mut MockEvalContext::new(),
            "NUMBERVALUE",
            vec![EvalValue::text(s), EvalValue::text(d)],
        )
    }

    #[test]
    fn parses() {
        assert_eq!(nv("1,234.5", "."), Value::Number(1234.5));
        assert_eq!(nv("1.234,5", ","), Value::Number(1234.5));
        assert_eq!(nv(" 3 000 ", "."), Value::Number(3000.0));
        assert_eq!(nv("50%", "."), Value::Number(0.5));
        assert_eq!(nv("5%%", "."), Value::Number(0.0005));
        assert_eq!(nv("", "."), Value::Number(0.0));
        assert_eq!(nv("-1.5e2", "."), Value::Number(-150.0));
        assert_eq!(nv("1.2.3", "."), Value::Error(ErrorKind::Value));
        assert_eq!(nv("abc", "."), Value::Error(ErrorKind::Value));
        assert_eq!(nv("inf", "."), Value::Error(ErrorKind::Value));
    }

    #[test]
    fn optional_separators() {
        let mut c = MockEvalContext::new();
        let mut nv3 = |args: Vec<EvalValue>| call(&mut c, "NUMBERVALUE", args);
        assert_eq!(nv3(vec![EvalValue::text("1,234.5")]), Value::Number(1234.5));
        assert_eq!(
            nv3(vec![
                EvalValue::text("1 234;5"),
                EvalValue::text(";"),
                EvalValue::text(" ")
            ]),
            Value::Number(1234.5)
        );
        assert_eq!(
            nv3(vec![
                EvalValue::text("1'234,5"),
                EvalValue::text(",x"),
                EvalValue::text("'")
            ]),
            Value::Number(1234.5)
        );
        assert_eq!(
            nv3(vec![EvalValue::text("1"), EvalValue::text("")]),
            Value::Error(ErrorKind::Value)
        );
    }

    #[test]
    fn document_locale_defaults() {
        let mut c = MockEvalContext::new();
        c.set_locale("de-DE");
        let mut nv = |args: Vec<EvalValue>| call(&mut c, "NUMBERVALUE", args);
        assert_eq!(nv(vec![EvalValue::text("1.234,5")]), Value::Number(1234.5));
        assert_eq!(nv(vec![EvalValue::text("1,5")]), Value::Number(1.5));
        assert_eq!(
            nv(vec![EvalValue::text("1,234.5")]),
            Value::Error(ErrorKind::Value)
        );
        // Explicit separators win over the locale.
        assert_eq!(
            nv(vec![EvalValue::text("1,234.5"), EvalValue::text(".")]),
            Value::Number(1234.5)
        );
        let mut c = MockEvalContext::new();
        c.set_locale("fr-FR");
        assert_eq!(
            call(&mut c, "NUMBERVALUE", vec![EvalValue::text("1\u{a0}234,5")]),
            Value::Number(1234.5)
        );
    }

    #[test]
    fn explicit_group() {
        assert_eq!(number_value("1 234;5", ';', Some(' ')), Ok(1234.5));
        assert_eq!(number_value("1,2", ',', Some(',')), Err(ErrorKind::Value));
    }
}
