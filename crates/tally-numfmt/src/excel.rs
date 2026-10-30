//! Translation between Excel (OOXML) and OOo format codes.
//!
//! The two dialects share most of the grammar. Differences handled here:
//!
//! - keyword case: Excel writes `yyyy`, `dddd`, `[h]`, `[Red]`; OOo
//!   canonical form is upper case (`YYYY`, `DDDD`, `[HH]`, `[RED]`);
//! - `General` (Excel) vs `General`/`Standard` (OOo);
//! - OOo-only weekday keywords `NN`/`NNN`/`NNNN` become `ddd`/`dddd`;
//! - backslash escapes and quoted strings pass through unchanged.
//!
//! TODO(v0.2): Excel `[$-F800]`/`[$-F400]` system date/time codes, Excel
//! `B1`/`B2` (Buddhist) and `e` (era year) in East-Asian locales,
//! `[DBNumN]` ↔ `[NatNumN]` mapping, and OOo `Q`/`QQ`/`WW`/`G` which have no
//! Excel equivalent (currently emitted as quoted placeholders).

use crate::format_ast::{Color, DateTimeToken, ElapsedUnit, GeneralKeyword, NamedColor, Token};
use crate::parse_format::{parse_format, print_format, print_token, FormatError};

/// Converts an Excel format code to canonical OOo form.
pub fn from_excel_code(code: &str) -> Result<String, FormatError> {
    Ok(print_format(&parse_format(code)?))
}

fn rep(out: &mut String, c: char, n: u8) {
    for _ in 0..n {
        out.push(c);
    }
}

/// Converts an OOo format code to the Excel dialect.
///
/// AOO's `BOOLEAN` becomes `"TRUE";"TRUE";"FALSE"`; a language-only tag
/// next to a currency tag in one section (`[$-809][$$-409]0.00`) is dropped,
/// Excel takes one locale per section.
pub fn to_excel_code(code: &str) -> Result<String, FormatError> {
    let ast = parse_format(code)?;
    let boolean = |s: &crate::format_ast::Section| {
        s.tokens
            .iter()
            .any(|t| matches!(t, Token::General(GeneralKeyword::Boolean)))
    };
    if ast.sections.len() == 1 && boolean(&ast.sections[0]) {
        return Ok("\"TRUE\";\"TRUE\";\"FALSE\"".into());
    }
    let mut out = String::new();
    for (i, s) in ast.sections.iter().enumerate() {
        if i > 0 {
            out.push(';');
        }
        let has_currency = s
            .tokens
            .iter()
            .any(|t| matches!(t, Token::Currency { symbol, .. } if !symbol.is_empty()));
        for t in &s.tokens {
            match t {
                Token::Currency { symbol, .. } if symbol.is_empty() && has_currency => {}
                Token::General(GeneralKeyword::Boolean) => out.push_str("\"TRUE\""),
                Token::DateTime(dt) => match dt {
                    DateTimeToken::Day(n) | DateTimeToken::DayName(n) => rep(&mut out, 'd', *n),
                    DateTimeToken::AooDayName(2) => out.push_str("ddd"),
                    DateTimeToken::AooDayName(3) => out.push_str("dddd"),
                    DateTimeToken::AooDayName(_) => out.push_str("dddd\", \""),
                    DateTimeToken::AaaDayName(n) => rep(&mut out, 'a', *n),
                    DateTimeToken::Month(n) | DateTimeToken::Minute(n) => rep(&mut out, 'm', *n),
                    DateTimeToken::Year(n) => rep(&mut out, 'y', *n),
                    DateTimeToken::Hour(n) => rep(&mut out, 'h', *n),
                    DateTimeToken::Second(n) => rep(&mut out, 's', *n),
                    DateTimeToken::AmPm(s) | DateTimeToken::AP(s) => out.push_str(s),
                    DateTimeToken::EraYear(n) => rep(&mut out, 'e', *n),
                    DateTimeToken::Elapsed(u, n) => {
                        out.push('[');
                        let c = match u {
                            ElapsedUnit::Hours => 'h',
                            ElapsedUnit::Minutes => 'm',
                            ElapsedUnit::Seconds => 's',
                        };
                        rep(&mut out, c, *n);
                        out.push(']');
                    }
                    // TODO(v0.2): no Excel equivalent; keep the keyword quoted.
                    DateTimeToken::Quarter(n) => {
                        out.push('"');
                        rep(&mut out, 'Q', *n);
                        out.push('"');
                    }
                    DateTimeToken::Week(n) => {
                        out.push('"');
                        rep(&mut out, 'W', *n);
                        out.push('"');
                    }
                    DateTimeToken::Era(n) => {
                        out.push('"');
                        rep(&mut out, 'G', *n);
                        out.push('"');
                    }
                },
                Token::General(_) => out.push_str("General"),
                Token::Color(Color::Named(c)) => {
                    let name = match c {
                        NamedColor::Black => "Black",
                        NamedColor::Blue => "Blue",
                        NamedColor::Green => "Green",
                        NamedColor::Cyan => "Cyan",
                        NamedColor::Red => "Red",
                        NamedColor::Magenta => "Magenta",
                        // Excel has no BROWN/GREY keywords; use palette slots.
                        NamedColor::Brown => "Color12",
                        NamedColor::Grey => "Color16",
                        NamedColor::Yellow => "Yellow",
                        NamedColor::White => "White",
                    };
                    out.push('[');
                    out.push_str(name);
                    out.push(']');
                }
                Token::Color(Color::Index(n)) => {
                    out.push_str("[Color");
                    out.push_str(&n.to_string());
                    out.push(']');
                }
                // TODO(v0.2): map NatNum to DBNum; calendars have no equivalent.
                Token::NatNum(_) | Token::Calendar(_) => {}
                _ => print_token(t, &mut out),
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        assert_eq!(from_excel_code("yyyy-mm-dd").unwrap(), "YYYY-MM-DD");
        assert_eq!(from_excel_code("[h]:mm:ss").unwrap(), "[H]:MM:SS");
        assert_eq!(from_excel_code("general").unwrap(), "General");
        assert_eq!(
            from_excel_code("#,##0.00_);[Red]\\(#,##0.00\\)").unwrap(),
            "#,##0.00_);[RED]\\(#,##0.00\\)"
        );
        assert_eq!(to_excel_code("YYYY-MM-DD").unwrap(), "yyyy-mm-dd");
        assert_eq!(to_excel_code("NNN, MMMM D").unwrap(), "dddd, mmmm d");
        assert_eq!(to_excel_code("[HH]:MM").unwrap(), "[hh]:mm");
        assert_eq!(to_excel_code("Standard").unwrap(), "General");
        assert_eq!(to_excel_code("0;[RED]-0").unwrap(), "0;[Red]-0");
        assert_eq!(
            to_excel_code("BOOLEAN").unwrap(),
            "\"TRUE\";\"TRUE\";\"FALSE\""
        );
        assert_eq!(
            to_excel_code("[$-809][$$-409]#,##0.00").unwrap(),
            "[$$-409]#,##0.00"
        );
        assert_eq!(
            to_excel_code("[$-407]DD.MM.YYYY").unwrap(),
            "[$-407]dd.mm.yyyy"
        );
    }
}
