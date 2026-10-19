//! FIXED and DOLLAR.
//!
//! Both round half away from zero (after snapping to 15 digits) and then
//! format with a generated number format code through `tally-numfmt`.

use tally_numfmt::{format_value_with, parse_format, Locale};
use tally_types::{round_to_15_digits, ErrorKind, Value};

use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry, ReturnKind};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const FIXED_PARAMS: &[Param] = &[
    Param::required("number", ParamKind::Number),
    Param::optional("decimals", ParamKind::Integer),
    Param::optional("no_thousands_separators", ParamKind::Bool),
];
const DOLLAR_PARAMS: &[Param] = &[
    Param::required("value", ParamKind::Number),
    Param::optional("decimals", ParamKind::Integer),
];

/// `FIXED(number [; decimals [; no_thousands_separators]])`.
pub const FIXED: FunctionDef = FunctionDef::new("FIXED", Category::Text, FIXED_PARAMS, fixed)
    .returns(ReturnKind::Text)
    .help("Formats a number with a fixed number of decimals as text.");
/// `DOLLAR(value [; decimals])`.
pub const DOLLAR: FunctionDef = FunctionDef::new("DOLLAR", Category::Text, DOLLAR_PARAMS, dollar)
    .returns(ReturnKind::Text)
    .help("Formats a number as currency text.");

/// Rounds half away from zero to `decimals` places (negative = left of the point).
fn round_dec(x: f64, decimals: i64) -> f64 {
    if decimals >= 0 && x.abs() >= 1e15 {
        return x;
    }
    let p = 10f64.powi(decimals.unsigned_abs() as i32);
    let r = if decimals >= 0 {
        round_to_15_digits(x * p).round() / p
    } else {
        round_to_15_digits(x / p).round() * p
    };
    if r == 0.0 {
        0.0
    } else {
        r
    }
}

fn digits_code(decimals: i64) -> String {
    let mut s = String::from("#,##0");
    if decimals > 0 {
        s.push('.');
        s.push_str(&"0".repeat(decimals as usize));
    }
    s
}

fn render(
    a: &mut Args<'_>,
    x: f64,
    decimals: i64,
    code: impl Fn(&str) -> String,
    keep_group: bool,
) -> Result<Value, ErrorKind> {
    if !(-15..=15).contains(&decimals) {
        return Err(ErrorKind::InvalidArgument);
    }
    let r = round_dec(x, decimals);
    let mut body = digits_code(decimals);
    if !keep_group {
        body = body.replacen("#,##0", "0", 1);
    }
    let ast = parse_format(&code(&body)).map_err(|_| ErrorKind::InvalidArgument)?;
    let epoch = a.ctx().settings().date_epoch;
    let out = format_value_with(&Value::Number(r), &ast, &Locale::en_us(), None, epoch);
    Ok(Value::text(out.text))
}

fn fixed(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let x = a.num(0)?;
        let decimals = a.int_opt(1, 2)?;
        let no_commas = a.bool_opt(2, false)?;
        render(&mut a, x, decimals, str::to_owned, !no_commas)
    })
}

fn dollar(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let x = a.num(0)?;
        let decimals = a.int_opt(1, 2)?;
        render(
            &mut a,
            x,
            decimals,
            |body| format!("\"$\"{body};-\"$\"{body}"),
            true,
        )
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(FIXED);
    r.register(DOLLAR);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::{call, MockEvalContext};

    fn n(x: f64) -> EvalValue {
        EvalValue::number(x)
    }
    fn t(name: &str, a: Vec<EvalValue>) -> Value {
        call(&mut MockEvalContext::new(), name, a)
    }

    #[test]
    fn fixed() {
        assert_eq!(t("FIXED", vec![n(1234.567)]), Value::text("1,234.57"));
        assert_eq!(
            t("FIXED", vec![n(1234.567), n(1.0)]),
            Value::text("1,234.6")
        );
        assert_eq!(
            t("FIXED", vec![n(1234.567), n(0.0), EvalValue::bool(true)]),
            Value::text("1235")
        );
        assert_eq!(t("FIXED", vec![n(1234.567), n(-2.0)]), Value::text("1,200"));
        assert_eq!(
            t("FIXED", vec![n(-1234.5), n(1.0)]),
            Value::text("-1,234.5")
        );
        assert_eq!(t("FIXED", vec![n(-0.001)]), Value::text("0.00"));
        assert_eq!(t("FIXED", vec![n(2.675)]), Value::text("2.68"));
        assert_eq!(
            t("FIXED", vec![n(1.0), n(16.0)]),
            Value::Error(ErrorKind::InvalidArgument)
        );
    }

    #[test]
    fn dollar() {
        assert_eq!(t("DOLLAR", vec![n(1234.567)]), Value::text("$1,234.57"));
        assert_eq!(t("DOLLAR", vec![n(-1234.567)]), Value::text("-$1,234.57"));
        assert_eq!(
            t("DOLLAR", vec![n(1234.567), n(0.0)]),
            Value::text("$1,235")
        );
        assert_eq!(
            t("DOLLAR", vec![n(1234.567), n(-2.0)]),
            Value::text("$1,200")
        );
        assert_eq!(t("DOLLAR", vec![n(0.0)]), Value::text("$0.00"));
    }
}
