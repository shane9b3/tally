//! BIN2DEC, BIN2HEX, BIN2OCT, DEC2BIN, DEC2HEX, DEC2OCT, HEX2BIN, HEX2DEC,
//! HEX2OCT, OCT2BIN, OCT2DEC, OCT2HEX (Analysis add-in).
//!
//! Semantics ported from Apache OpenOffice `scaddins/source/analysis/analysishelper.cxx`
//! (`ConvertToDec`, `ConvertFromDec`; Apache-2.0): numbers have at most 10
//! digits and negative values are 10-digit two's complement.

use tally_types::{ErrorKind, Value};

use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry, ReturnKind};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

/// Number of digits of every base conversion result/argument.
const MAX_DIGITS: usize = 10;

const TEXT_PLACES: &[Param] = &[
    Param::required("number", ParamKind::Text),
    Param::optional("places", ParamKind::Integer),
];
const TEXT_ONLY: &[Param] = &[Param::required("number", ParamKind::Text)];
const NUM_PLACES: &[Param] = &[
    Param::required("number", ParamKind::Number),
    Param::optional("places", ParamKind::Integer),
];

/// Parses a two's-complement number with at most 10 digits in `base`.
fn to_dec(s: &str, base: u32) -> Result<i64, ErrorKind> {
    let n = s.chars().count();
    if n > MAX_DIGITS {
        return Err(ErrorKind::InvalidArgument);
    }
    let mut v: i64 = 0;
    for c in s.chars() {
        let d = c.to_digit(base).ok_or(ErrorKind::InvalidArgument)?;
        v = v * i64::from(base) + i64::from(d);
    }
    if n == MAX_DIGITS {
        let bits = match base {
            2 => 10,
            8 => 30,
            _ => 40,
        };
        if v >= 1_i64 << (bits - 1) {
            v -= 1_i64 << bits;
        }
    }
    Ok(v)
}

/// Formats `v` in `base`; negative values are 10-digit two's complement
/// (`places` ignored), positive ones are padded to `places` digits.
fn from_dec(v: i64, base: u32, places: Option<i64>) -> Result<String, ErrorKind> {
    let bits: u32 = match base {
        2 => 10,
        8 => 30,
        _ => 40,
    };
    let (min, max) = (-(1_i64 << (bits - 1)), (1_i64 << (bits - 1)) - 1);
    if v < min || v > max {
        return Err(ErrorKind::InvalidArgument);
    }
    let places = match places {
        Some(p) if !(1..=MAX_DIGITS as i64).contains(&p) => return Err(ErrorKind::InvalidArgument),
        p => p,
    };
    let (mut u, negative) = if v < 0 {
        ((v + (1_i64 << bits)) as u64, true)
    } else {
        (v as u64, false)
    };
    let mut digits = Vec::new();
    while u > 0 {
        digits.push(char::from_digit((u % u64::from(base)) as u32, base).unwrap_or('0'));
        u /= u64::from(base);
    }
    let want = if negative {
        MAX_DIGITS
    } else {
        places.map_or(1, |p| p as usize)
    };
    if places.is_some() && digits.len() > want && !negative {
        return Err(ErrorKind::InvalidArgument);
    }
    while digits.len() < want.max(1) {
        digits.push('0');
    }
    Ok(digits.iter().rev().collect::<String>().to_uppercase())
}

fn text_to_dec(a: &mut Args, base: u32) -> Result<i64, ErrorKind> {
    let s = a.text(0)?;
    to_dec(s.trim(), base)
}

fn to_text_conv(a: &mut Args, from: u32, to: u32) -> Result<Value, ErrorKind> {
    let v = text_to_dec(a, from)?;
    let places = if a.is_missing(1) {
        None
    } else {
        Some(a.int(1)?)
    };
    from_dec(v, to, places).map(|s| Value::text(&*s))
}

fn dec_to_text(a: &mut Args, to: u32) -> Result<Value, ErrorKind> {
    let x = a.num(0)?.trunc();
    if !x.is_finite() || x.abs() > 1e12 {
        return Err(ErrorKind::InvalidArgument);
    }
    let places = if a.is_missing(1) {
        None
    } else {
        Some(a.int(1)?)
    };
    from_dec(x as i64, to, places).map(|s| Value::text(&*s))
}

macro_rules! text_fn {
    ($name:ident, $from:expr, $to:expr) => {
        fn $name(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
            let mut a = Args::new(ctx, args);
            run(|| to_text_conv(&mut a, $from, $to))
        }
    };
}
macro_rules! to_dec_fn {
    ($name:ident, $from:expr) => {
        fn $name(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
            let mut a = Args::new(ctx, args);
            run(|| text_to_dec(&mut a, $from).map(|v| Value::Number(v as f64)))
        }
    };
}
macro_rules! from_dec_fn {
    ($name:ident, $to:expr) => {
        fn $name(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
            let mut a = Args::new(ctx, args);
            run(|| dec_to_text(&mut a, $to))
        }
    };
}

text_fn!(bin2oct, 2, 8);
text_fn!(bin2hex, 2, 16);
text_fn!(oct2bin, 8, 2);
text_fn!(oct2hex, 8, 16);
text_fn!(hex2bin, 16, 2);
text_fn!(hex2oct, 16, 8);
to_dec_fn!(bin2dec, 2);
to_dec_fn!(oct2dec, 8);
to_dec_fn!(hex2dec, 16);
from_dec_fn!(dec2bin, 2);
from_dec_fn!(dec2oct, 8);
from_dec_fn!(dec2hex, 16);

macro_rules! def_text {
    ($c:ident, $name:literal, $f:ident, $help:literal) => {
        #[doc = concat!("`", $name, "(number [; places])`.")]
        pub const $c: FunctionDef = FunctionDef::new($name, Category::AddIn, TEXT_PLACES, $f)
            .returns(ReturnKind::Text)
            .help($help);
    };
}
macro_rules! def_dec {
    ($c:ident, $name:literal, $f:ident, $help:literal) => {
        #[doc = concat!("`", $name, "(number)`.")]
        pub const $c: FunctionDef =
            FunctionDef::new($name, Category::AddIn, TEXT_ONLY, $f).help($help);
    };
}
macro_rules! def_from_dec {
    ($c:ident, $name:literal, $f:ident, $help:literal) => {
        #[doc = concat!("`", $name, "(number [; places])`.")]
        pub const $c: FunctionDef = FunctionDef::new($name, Category::AddIn, NUM_PLACES, $f)
            .returns(ReturnKind::Text)
            .help($help);
    };
}

def_text!(BIN2OCT, "BIN2OCT", bin2oct, "Binary number to octal text.");
def_text!(
    BIN2HEX,
    "BIN2HEX",
    bin2hex,
    "Binary number to hexadecimal text."
);
def_text!(OCT2BIN, "OCT2BIN", oct2bin, "Octal number to binary text.");
def_text!(
    OCT2HEX,
    "OCT2HEX",
    oct2hex,
    "Octal number to hexadecimal text."
);
def_text!(
    HEX2BIN,
    "HEX2BIN",
    hex2bin,
    "Hexadecimal number to binary text."
);
def_text!(
    HEX2OCT,
    "HEX2OCT",
    hex2oct,
    "Hexadecimal number to octal text."
);
def_dec!(BIN2DEC, "BIN2DEC", bin2dec, "Binary number to decimal.");
def_dec!(OCT2DEC, "OCT2DEC", oct2dec, "Octal number to decimal.");
def_dec!(
    HEX2DEC,
    "HEX2DEC",
    hex2dec,
    "Hexadecimal number to decimal."
);
def_from_dec!(
    DEC2BIN,
    "DEC2BIN",
    dec2bin,
    "Decimal number to binary text."
);
def_from_dec!(DEC2OCT, "DEC2OCT", dec2oct, "Decimal number to octal text.");
def_from_dec!(
    DEC2HEX,
    "DEC2HEX",
    dec2hex,
    "Decimal number to hexadecimal text."
);

pub(super) fn register(r: &mut Registry) {
    for def in [
        BIN2OCT, BIN2HEX, OCT2BIN, OCT2HEX, HEX2BIN, HEX2OCT, BIN2DEC, OCT2DEC, HEX2DEC, DEC2BIN,
        DEC2OCT, DEC2HEX,
    ] {
        r.register(def);
    }
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{ErrorKind, Value};

    fn c(name: &str, args: Vec<EvalValue>) -> Value {
        call(&mut MockEvalContext::new(), name, args)
    }
    fn t(s: &str) -> EvalValue {
        EvalValue::text(s)
    }
    fn n(x: f64) -> EvalValue {
        EvalValue::number(x)
    }
    const BAD: Value = Value::Error(ErrorKind::InvalidArgument);

    #[test]
    fn to_decimal() {
        assert_eq!(c("BIN2DEC", vec![t("1111111111")]), Value::Number(-1.0));
        assert_eq!(c("BIN2DEC", vec![t("1000000000")]), Value::Number(-512.0));
        assert_eq!(c("BIN2DEC", vec![t("101")]), Value::Number(5.0));
        assert_eq!(c("HEX2DEC", vec![t("FFFFFFFFFF")]), Value::Number(-1.0));
        assert_eq!(c("HEX2DEC", vec![t("ff")]), Value::Number(255.0));
        assert_eq!(c("OCT2DEC", vec![t("7777777777")]), Value::Number(-1.0));
        assert_eq!(c("OCT2DEC", vec![t("777")]), Value::Number(511.0));
        assert_eq!(c("BIN2DEC", vec![t("12")]), BAD);
        assert_eq!(c("BIN2DEC", vec![t("11111111111")]), BAD);
    }

    #[test]
    fn from_decimal() {
        assert_eq!(c("DEC2HEX", vec![n(-1.0)]), Value::text("FFFFFFFFFF"));
        assert_eq!(c("DEC2HEX", vec![n(255.0), n(4.0)]), Value::text("00FF"));
        assert_eq!(c("DEC2BIN", vec![n(-1.0)]), Value::text("1111111111"));
        assert_eq!(c("DEC2BIN", vec![n(5.0), n(8.0)]), Value::text("00000101"));
        assert_eq!(c("DEC2BIN", vec![n(512.0)]), BAD);
        assert_eq!(c("DEC2BIN", vec![n(-513.0)]), BAD);
        assert_eq!(c("DEC2BIN", vec![n(5.0), n(2.0)]), BAD);
        assert_eq!(
            c("DEC2OCT", vec![n(-1.0), n(2.0)]),
            Value::text("7777777777")
        );
        assert_eq!(c("DEC2OCT", vec![n(8.0)]), Value::text("10"));
        assert_eq!(c("DEC2HEX", vec![n(0.0)]), Value::text("0"));
        assert_eq!(c("DEC2HEX", vec![n(10.9)]), Value::text("A"));
    }

    #[test]
    fn between_bases() {
        assert_eq!(
            c("BIN2HEX", vec![t("1111111111")]),
            Value::text("FFFFFFFFFF")
        );
        assert_eq!(
            c("BIN2OCT", vec![t("1111111111")]),
            Value::text("7777777777")
        );
        assert_eq!(c("HEX2BIN", vec![t("FF")]), Value::text("11111111"));
        assert_eq!(c("HEX2BIN", vec![t("200")]), BAD);
        assert_eq!(c("HEX2OCT", vec![t("FF")]), Value::text("377"));
        assert_eq!(
            c("OCT2BIN", vec![t("7777777777")]),
            Value::text("1111111111")
        );
        assert_eq!(c("OCT2HEX", vec![t("17"), n(4.0)]), Value::text("000F"));
    }
}
