//! BASE and DECIMAL (radix conversion, radix 2..=36).

use tally_types::{ErrorKind, Value};

use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry, ReturnKind};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const BASE_PARAMS: &[Param] = &[
    Param::required("number", ParamKind::Number),
    Param::required("radix", ParamKind::Integer),
    Param::optional("min_length", ParamKind::Integer),
];
const DECIMAL_PARAMS: &[Param] = &[
    Param::required("text", ParamKind::Text),
    Param::required("radix", ParamKind::Integer),
];

/// `BASE(number; radix [; min_length])`.
pub const BASE: FunctionDef = FunctionDef::new("BASE", Category::Text, BASE_PARAMS, base)
    .returns(ReturnKind::Text)
    .help("Converts a non-negative integer to text in the given radix.");
/// `DECIMAL(text; radix)`.
pub const DECIMAL: FunctionDef =
    FunctionDef::new("DECIMAL", Category::Text, DECIMAL_PARAMS, decimal)
        .returns(ReturnKind::Number)
        .help("Converts text in the given radix to a decimal number.");

fn radix_arg(r: i64) -> Result<u32, ErrorKind> {
    if (2..=36).contains(&r) {
        Ok(r as u32)
    } else {
        Err(ErrorKind::InvalidArgument)
    }
}

fn base(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let x = a.num(0)?.trunc();
        let radix = radix_arg(a.int(1)?)?;
        let min = a.int_opt(2, 0)?;
        if !(0.0..9_007_199_254_740_992.0).contains(&x) || !(0..=255).contains(&min) {
            return Err(ErrorKind::InvalidArgument);
        }
        let mut n = x as u64;
        let mut digits: Vec<char> = Vec::new();
        while n > 0 {
            digits.push(char::from_digit((n % u64::from(radix)) as u32, radix).unwrap_or('0'));
            n /= u64::from(radix);
        }
        while (digits.len() as i64) < min.max(1) {
            digits.push('0');
        }
        Ok(Value::text(
            digits.iter().rev().collect::<String>().to_ascii_uppercase(),
        ))
    })
}

fn decimal(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let s = a.text(0)?;
        let radix = radix_arg(a.int(1)?)?;
        let mut acc = 0.0f64;
        for c in s.chars() {
            let d = c.to_digit(radix).ok_or(ErrorKind::InvalidArgument)?;
            acc = acc * f64::from(radix) + f64::from(d);
        }
        Ok(Value::Number(acc))
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(BASE);
    r.register(DECIMAL);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::{call, MockEvalContext};

    fn n(x: f64) -> EvalValue {
        EvalValue::number(x)
    }
    fn t(name: &str, args: Vec<EvalValue>) -> Value {
        call(&mut MockEvalContext::new(), name, args)
    }

    #[test]
    fn base_basic() {
        assert_eq!(t("BASE", vec![n(255.0), n(16.0)]), Value::text("FF"));
        assert_eq!(
            t("BASE", vec![n(7.0), n(2.0), n(8.0)]),
            Value::text("00000111")
        );
        assert_eq!(t("BASE", vec![n(0.0), n(2.0)]), Value::text("0"));
        assert_eq!(t("BASE", vec![n(35.0), n(36.0)]), Value::text("Z"));
        assert_eq!(
            t("BASE", vec![n(-1.0), n(2.0)]),
            Value::Error(ErrorKind::InvalidArgument)
        );
        assert_eq!(
            t("BASE", vec![n(5.0), n(37.0)]),
            Value::Error(ErrorKind::InvalidArgument)
        );
    }

    #[test]
    fn decimal_basic() {
        let d = |s: &str, r: f64| t("DECIMAL", vec![EvalValue::text(s), n(r)]);
        assert_eq!(d("ff", 16.0), Value::Number(255.0));
        assert_eq!(d("FF", 16.0), Value::Number(255.0));
        assert_eq!(d("111", 2.0), Value::Number(7.0));
        assert_eq!(d("", 2.0), Value::Number(0.0));
        assert_eq!(d("12", 2.0), Value::Error(ErrorKind::InvalidArgument));
        assert_eq!(d("1", 1.0), Value::Error(ErrorKind::InvalidArgument));
    }
}
