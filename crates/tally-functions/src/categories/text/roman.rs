//! ROMAN and ARABIC.

use tally_types::{ErrorKind, Value};

use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry, ReturnKind};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const ROMAN_PARAMS: &[Param] = &[
    Param::required("number", ParamKind::Integer),
    Param::optional("mode", ParamKind::Integer),
];
const ARABIC_PARAMS: &[Param] = &[Param::required("text", ParamKind::Text)];

/// `ROMAN(number [; mode])`.
pub const ROMAN: FunctionDef = FunctionDef::new("ROMAN", Category::Text, ROMAN_PARAMS, roman)
    .returns(ReturnKind::Text)
    .help("Converts a number to a Roman numeral (mode 0-4 = classic to simplified).");
/// `ARABIC(text)`.
pub const ARABIC: FunctionDef = FunctionDef::new("ARABIC", Category::Text, ARABIC_PARAMS, arabic)
    .returns(ReturnKind::Number)
    .help("Converts a Roman numeral to a number.");

const CHARS: [char; 7] = ['M', 'D', 'C', 'L', 'X', 'V', 'I'];
const VALUES: [i32; 7] = [1000, 500, 100, 50, 10, 5, 1];

/// Port of the algorithm of `ScInterpreter::ScRoman` (AOO
/// `sc/source/core/tool/interpr1.cxx`). `n` is 1..=3999, `mode` 0..=4.
fn to_roman(mut n: i32, mode: usize) -> String {
    let mut out = String::new();
    for i in 0..=3usize {
        let mut index = 2 * i;
        let digit = n / VALUES[index];
        if digit % 5 == 4 {
            let index2 = if digit == 4 { index - 1 } else { index - 2 };
            let mut steps = 0;
            while steps < mode && index < 6 {
                steps += 1;
                if VALUES[index2] - VALUES[index + 1] <= n {
                    index += 1;
                } else {
                    steps = mode;
                }
            }
            out.push(CHARS[index]);
            out.push(CHARS[index2]);
            n = n + VALUES[index] - VALUES[index2];
        } else {
            let mut d = digit;
            if d > 4 {
                out.push(CHARS[index - 1]);
                d -= 5;
            }
            for _ in 0..d {
                out.push(CHARS[index]);
            }
            n %= VALUES[index];
        }
    }
    out
}

fn roman(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let n = a.int(0)?;
        let mode = a.int_opt(1, 0)?;
        if !(0..=3999).contains(&n) || !(0..=4).contains(&mode) {
            return Err(ErrorKind::InvalidArgument);
        }
        Ok(Value::text(to_roman(n as i32, mode as usize)))
    })
}

fn char_value(c: char) -> Option<i32> {
    Some(match c {
        'M' => 1000,
        'D' => 500,
        'C' => 100,
        'L' => 50,
        'X' => 10,
        'V' => 5,
        'I' => 1,
        _ => return None,
    })
}

/// Parses a Roman numeral (case-insensitive, optional leading `-`). The
/// grammar is lax like Excel's (`IIII` is 4); the magnitude is capped at 3999.
fn from_roman(s: &str) -> Option<i32> {
    let upper = s.to_uppercase();
    let (neg, body) = match upper.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, upper.as_str()),
    };
    let vals: Vec<i32> = body.chars().map(char_value).collect::<Option<_>>()?;
    let mut total = 0i32;
    let mut i = 0;
    while i < vals.len() {
        let d1 = vals[i];
        let d2 = vals.get(i + 1).copied().unwrap_or(0);
        if d1 < d2 {
            total += d2 - d1;
            i += 2;
        } else {
            total += d1;
            i += 1;
        }
        if total > 3999 {
            return None;
        }
    }
    Some(if neg { -total } else { total })
}

fn arabic(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let s = a.text(0)?;
        from_roman(&s)
            .map(|n| Value::Number(f64::from(n)))
            .ok_or(ErrorKind::InvalidArgument)
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(ROMAN);
    r.register(ARABIC);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::{call, MockEvalContext};

    fn t(name: &str, args: Vec<EvalValue>) -> Value {
        call(&mut MockEvalContext::new(), name, args)
    }

    #[test]
    fn roman_modes() {
        let r = |n: f64, m: f64| t("ROMAN", vec![EvalValue::number(n), EvalValue::number(m)]);
        assert_eq!(r(1994.0, 0.0), Value::text("MCMXCIV"));
        assert_eq!(r(499.0, 0.0), Value::text("CDXCIX"));
        assert_eq!(r(499.0, 1.0), Value::text("LDVLIV"));
        assert_eq!(r(499.0, 2.0), Value::text("XDIX"));
        assert_eq!(r(499.0, 3.0), Value::text("VDIV"));
        assert_eq!(r(499.0, 4.0), Value::text("ID"));
        assert_eq!(r(99.0, 1.0), Value::text("VCIV"));
        assert_eq!(r(3999.0, 0.0), Value::text("MMMCMXCIX"));
        assert_eq!(r(0.0, 0.0), Value::text(""));
        assert_eq!(r(4000.0, 0.0), Value::Error(ErrorKind::InvalidArgument));
        assert_eq!(r(5.0, 5.0), Value::Error(ErrorKind::InvalidArgument));
        assert_eq!(t("ROMAN", vec![EvalValue::number(4.0)]), Value::text("IV"));
    }

    #[test]
    fn arabic_values() {
        let a = |s: &str| t("ARABIC", vec![EvalValue::text(s)]);
        assert_eq!(a("MCMXCIV"), Value::Number(1994.0));
        assert_eq!(a("mcmxciv"), Value::Number(1994.0));
        assert_eq!(a("-IV"), Value::Number(-4.0));
        assert_eq!(a(""), Value::Number(0.0));
        assert_eq!(a("ABC"), Value::Error(ErrorKind::InvalidArgument));
        assert_eq!(a("MMMM"), Value::Error(ErrorKind::InvalidArgument));
    }
}
