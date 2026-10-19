//! ASC and JIS: conversion between full-width and half-width characters
//! (ASCII letters/digits/symbols and Katakana).

use tally_types::Value;

use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry, ReturnKind};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const PARAMS: &[Param] = &[Param::required("text", ParamKind::Text)];

/// `ASC(text)`: full-width to half-width.
pub const ASC: FunctionDef = FunctionDef::new("ASC", Category::Text, PARAMS, asc)
    .returns(ReturnKind::Text)
    .help("Converts full-width characters to half-width.");
/// `JIS(text)`: half-width to full-width.
pub const JIS: FunctionDef = FunctionDef::new("JIS", Category::Text, PARAMS, jis)
    .returns(ReturnKind::Text)
    .help("Converts half-width characters to full-width.");

/// Half-width katakana block U+FF66..=U+FF9F (58 characters) ...
const HALF: &str = "ｦｧｨｩｪｫｬｭｮｯｰｱｲｳｴｵｶｷｸｹｺｻｼｽｾｿﾀﾁﾂﾃﾄﾅﾆﾇﾈﾉﾊﾋﾌﾍﾎﾏﾐﾑﾒﾓﾔﾕﾖﾗﾘﾙﾚﾛﾜﾝﾞﾟ";
/// ... and the corresponding (unvoiced) full-width characters, same order.
const FULL: &str =
    "ヲァィゥェォャュョッーアイウエオカキクケコサシスセソタチツテトナニヌネノハヒフヘホマミムメモヤユヨラリルレロワン゛゜";

const DAKUTEN: char = '\u{FF9E}';
const HANDAKUTEN: char = '\u{FF9F}';

fn pairs() -> impl Iterator<Item = (char, char)> {
    HALF.chars().zip(FULL.chars())
}

/// Full-width characters that take a dakuten (voiced mark) in half-width form.
fn voiced_of(base: char) -> Option<char> {
    let c = base as u32;
    let rows = "カキクケコサシスセソタチツテトハヒフヘホ";
    if rows.contains(base) {
        char::from_u32(c + 1)
    } else if base == 'ウ' {
        Some('ヴ')
    } else {
        None
    }
}

fn semivoiced_of(base: char) -> Option<char> {
    if "ハヒフヘホ".contains(base) {
        char::from_u32(base as u32 + 2)
    } else {
        None
    }
}

/// Symbols that exist as full-width forms outside the FF01..FF5E block.
const SYMBOLS: [(char, char); 6] = [
    ('¢', '\u{FFE0}'),
    ('£', '\u{FFE1}'),
    ('¬', '\u{FFE2}'),
    ('¯', '\u{FFE3}'),
    ('¦', '\u{FFE4}'),
    ('¥', '\u{FFE5}'),
];

/// Full-width to half-width.
pub(super) fn to_half(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        let u = c as u32;
        if (0xFF01..=0xFF5E).contains(&u) {
            out.push(char::from_u32(u - 0xFEE0).unwrap_or(c));
        } else if c == '\u{3000}' {
            out.push(' ');
        } else if let Some((h, _)) = SYMBOLS.iter().find(|(_, f)| *f == c) {
            out.push(*h);
        } else if let Some((h, _)) = pairs().find(|(_, f)| *f == c) {
            out.push(h);
        } else if let Some((h, _)) = pairs().find(|(_, f)| voiced_of(*f) == Some(c)) {
            out.push(h);
            out.push(DAKUTEN);
        } else if let Some((h, _)) = pairs().find(|(_, f)| semivoiced_of(*f) == Some(c)) {
            out.push(h);
            out.push(HANDAKUTEN);
        } else {
            match c {
                '。' => out.push('｡'),
                '「' => out.push('｢'),
                '」' => out.push('｣'),
                '、' => out.push('､'),
                '・' => out.push('･'),
                _ => out.push(c),
            }
        }
    }
    out
}

/// Half-width to full-width.
pub(super) fn to_full(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len() * 3);
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let u = c as u32;
        i += 1;
        if (0x21..=0x7E).contains(&u) {
            out.push(char::from_u32(u + 0xFEE0).unwrap_or(c));
        } else if c == ' ' {
            out.push('\u{3000}');
        } else if let Some((_, f)) = SYMBOLS.iter().find(|(h, _)| *h == c) {
            out.push(*f);
        } else if let Some((_, f)) = pairs().find(|(h, _)| *h == c) {
            let next = chars.get(i).copied();
            let combined = match next {
                Some(DAKUTEN) => voiced_of(f),
                Some(HANDAKUTEN) => semivoiced_of(f),
                _ => None,
            };
            match combined {
                Some(v) => {
                    out.push(v);
                    i += 1;
                }
                None => out.push(f),
            }
        } else {
            match c {
                '｡' => out.push('。'),
                '｢' => out.push('「'),
                '｣' => out.push('」'),
                '､' => out.push('、'),
                '･' => out.push('・'),
                _ => out.push(c),
            }
        }
    }
    out
}

fn asc(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| Ok(Value::text(to_half(&a.text(0)?))))
}

fn jis(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| Ok(Value::text(to_full(&a.text(0)?))))
}

pub(super) fn register(r: &mut Registry) {
    r.register(ASC);
    r.register(JIS);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::{call, MockEvalContext};

    fn t(name: &str, s: &str) -> Value {
        call(&mut MockEvalContext::new(), name, vec![EvalValue::text(s)])
    }

    #[test]
    fn tables_align() {
        assert_eq!(HALF.chars().count(), 58);
        assert_eq!(FULL.chars().count(), 58);
    }

    #[test]
    fn ascii_forms() {
        assert_eq!(t("ASC", "ＡＢＣ１２３　！"), Value::text("ABC123 !"));
        assert_eq!(t("JIS", "ABC 123"), Value::text("ＡＢＣ　１２３"));
        assert_eq!(t("ASC", "￥"), Value::text("¥"));
    }

    #[test]
    fn katakana() {
        assert_eq!(t("ASC", "カタカナ"), Value::text("ｶﾀｶﾅ"));
        assert_eq!(t("ASC", "ガパヴ"), Value::text("ｶﾞﾊﾟｳﾞ"));
        assert_eq!(t("JIS", "ｶﾞﾊﾟｳﾞｱ"), Value::text("ガパヴア"));
        assert_eq!(t("JIS", "ｶﾀｶﾅ"), Value::text("カタカナ"));
        assert_eq!(t("ASC", "ー、。"), Value::text("ｰ､｡"));
        assert_eq!(t("JIS", "漢字abc"), Value::text("漢字ａｂｃ"));
    }
}
