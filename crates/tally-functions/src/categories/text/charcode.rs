//! CHAR, CODE, UNICHAR, UNICODE.
//!
//! AOO's CHAR/CODE use the system 8-bit charset; Tally uses Windows-1252.

use tally_types::{ErrorKind, Value};

use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry, ReturnKind};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const INT1: &[Param] = &[Param::required("number", ParamKind::Integer)];
const TEXT1: &[Param] = &[Param::required("text", ParamKind::Text)];

/// `CHAR(number)`.
pub const CHAR: FunctionDef = FunctionDef::new("CHAR", Category::Text, INT1, char_fn)
    .returns(ReturnKind::Text)
    .help("Character for a Windows-1252 code 1-255.");
/// `CODE(text)`.
pub const CODE: FunctionDef = FunctionDef::new("CODE", Category::Text, TEXT1, code)
    .returns(ReturnKind::Number)
    .help("Windows-1252 code of the first character.");
/// `UNICHAR(number)`.
pub const UNICHAR: FunctionDef = FunctionDef::new("UNICHAR", Category::Text, INT1, unichar)
    .returns(ReturnKind::Text)
    .help("Character for a Unicode code point.");
/// `UNICODE(text)`.
pub const UNICODE: FunctionDef = FunctionDef::new("UNICODE", Category::Text, TEXT1, unicode)
    .returns(ReturnKind::Number)
    .help("Unicode code point of the first character.");

/// Windows-1252 code points for bytes 0x80..=0x9F (undefined bytes map to C1 controls).
const CP1252_HIGH: [char; 32] = [
    '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8D}', 'Ž', '\u{8F}',
    '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9D}', 'ž', 'Ÿ',
];

fn cp1252_to_char(b: u8) -> char {
    match b {
        0x80..=0x9F => CP1252_HIGH[usize::from(b - 0x80)],
        other => char::from(other),
    }
}

fn char_to_cp1252(c: char) -> u8 {
    let u = c as u32;
    if u < 0x80 || (0xA0..=0xFF).contains(&u) {
        return u as u8;
    }
    match CP1252_HIGH.iter().position(|&h| h == c) {
        Some(i) => 0x80 + i as u8,
        None => b'?',
    }
}

fn char_fn(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let n = a.int(0)?;
        if !(1..=255).contains(&n) {
            return Err(ErrorKind::InvalidArgument);
        }
        Ok(Value::text(cp1252_to_char(n as u8).to_string()))
    })
}

fn code(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let s = a.text(0)?;
        Ok(Value::Number(
            s.chars()
                .next()
                .map_or(0.0, |c| f64::from(char_to_cp1252(c))),
        ))
    })
}

fn unichar(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let n = a.int(0)?;
        if n < 1 {
            return Err(ErrorKind::InvalidArgument);
        }
        let c = u32::try_from(n)
            .ok()
            .and_then(char::from_u32)
            .ok_or(ErrorKind::InvalidArgument)?;
        Ok(Value::text(c.to_string()))
    })
}

fn unicode(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let s = a.text(0)?;
        s.chars()
            .next()
            .map(|c| Value::Number(f64::from(c as u32)))
            .ok_or(ErrorKind::InvalidArgument)
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(CHAR);
    r.register(CODE);
    r.register(UNICHAR);
    r.register(UNICODE);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::{call, MockEvalContext};

    fn n(x: f64) -> EvalValue {
        EvalValue::number(x)
    }
    fn s(x: &str) -> EvalValue {
        EvalValue::text(x)
    }
    fn t(name: &str, a: EvalValue) -> Value {
        call(&mut MockEvalContext::new(), name, vec![a])
    }

    #[test]
    fn char_code() {
        assert_eq!(t("CHAR", n(65.0)), Value::text("A"));
        assert_eq!(t("CHAR", n(128.0)), Value::text("€"));
        assert_eq!(t("CHAR", n(233.0)), Value::text("é"));
        assert_eq!(t("CHAR", n(0.0)), Value::Error(ErrorKind::InvalidArgument));
        assert_eq!(
            t("CHAR", n(256.0)),
            Value::Error(ErrorKind::InvalidArgument)
        );
        assert_eq!(t("CODE", s("A")), Value::Number(65.0));
        assert_eq!(t("CODE", s("€uro")), Value::Number(128.0));
        assert_eq!(t("CODE", s("日")), Value::Number(63.0));
        assert_eq!(t("CODE", s("")), Value::Number(0.0));
    }

    #[test]
    fn unicode_fns() {
        assert_eq!(t("UNICHAR", n(8364.0)), Value::text("€"));
        assert_eq!(t("UNICHAR", n(128512.0)), Value::text("😀"));
        assert_eq!(
            t("UNICHAR", n(0.0)),
            Value::Error(ErrorKind::InvalidArgument)
        );
        assert_eq!(
            t("UNICHAR", n(55296.0)),
            Value::Error(ErrorKind::InvalidArgument)
        );
        assert_eq!(t("UNICODE", s("😀x")), Value::Number(128512.0));
        assert_eq!(
            t("UNICODE", s("")),
            Value::Error(ErrorKind::InvalidArgument)
        );
    }
}
