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
//! - Excel's extended locale ids `[$-NNCCLLLL]` (numeral shape `NN`,
//!   calendar `CC`, language `LLLL`) become `[NatNumN]` / `[~calendar]`
//!   with the language tag, as LibreOffice reads them
//!   (`ImpObtainCalendarAndNumerals`), and back; `[NatNumN]` of Chinese,
//!   Japanese and Korean is written as `[DBNumN]` where one maps
//!   (`SvNumberNatNum::MapNatNumToDBNum`); era codes `G`/`E`/`R` become
//!   `g`/`e` in East-Asian sections. See `docs/numfmt-natnum.md`.
//!
//! TODO(v0.2): Excel `[$-F800]`/`[$-F400]` system date/time codes, Excel
//! `B1`/`B2` (Buddhist / Hijri year codes), and OOo `Q`/`QQ`/`WW` which
//! have no Excel equivalent (currently emitted as quoted placeholders).

use crate::calendar::{has_calendar, CalendarId};
use crate::format_ast::{
    Color, DateTimeToken, ElapsedUnit, GeneralKeyword, NamedColor, Section, SectionKind, Token,
};
use crate::natnum::{excel_numeral, excel_numeral_shape, map_natnum_to_dbnum, LangId};
use crate::parse_format::{parse_format, print_format, print_token, FormatError};

/// Converts an Excel format code to canonical OOo form.
pub fn from_excel_code(code: &str) -> Result<String, FormatError> {
    let mut ast = parse_format(code)?;
    let mut changed = false;
    for sec in &mut ast.sections {
        changed |= expand_extended_lcid(sec);
    }
    let out = print_format(&ast);
    if changed {
        // Re-parse so the new modifiers are analysed.
        return Ok(print_format(&parse_format(&out)?));
    }
    Ok(out)
}

/// `[$-D07041E]` → `[NatNum1][$-41E][~buddhist]` (LibreOffice
/// `ImpObtainCalendarAndNumerals`): the numeral shape becomes a NatNum, the
/// calendar a `[~calendar]` at the start of the section, and the language
/// the one that has those numerals / that calendar. Returns whether the
/// section changed.
fn expand_extended_lcid(sec: &mut Section) -> bool {
    let Some(pos) = sec.tokens.iter().position(|t| {
        matches!(t, Token::Currency { symbol, locale: Some(hex) }
            if symbol.is_empty() && hex.len() > 4 && u32::from_str_radix(hex, 16).is_ok_and(|v| v > 0xFFFF))
    }) else {
        return false;
    };
    let Token::Currency {
        locale: Some(hex), ..
    } = &sec.tokens[pos]
    else {
        return false;
    };
    let raw = u32::from_str_radix(hex, 16).unwrap_or(0);
    let mut lang = LangId((raw & 0xFFFF) as u16);
    let cal_byte = ((raw >> 16) & 0xFF) as u8;
    let numeral = ((raw >> 24) & 0xFF) as u8;
    let mut calendar = None;
    if let Some((cal, home)) = CalendarId::from_excel(cal_byte) {
        if !has_calendar(lang, cal) && cal != CalendarId::Jewish {
            lang = home;
        }
        calendar = Some(cal);
    }
    let mut natnum = None;
    if sec.natnum.is_none() {
        if let Some((l, n)) = excel_numeral(numeral, lang) {
            lang = l;
            natnum = Some(n);
        }
    }
    let mut new = Vec::with_capacity(3);
    if let Some(n) = natnum {
        new.push(Token::NatNum(n));
    }
    new.push(Token::Currency {
        symbol: String::new(),
        locale: Some(lang.hex()),
    });
    if let Some(c) = calendar {
        new.push(Token::Calendar(c.name().to_string()));
    }
    sec.tokens.splice(pos..=pos, new);
    true
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
    // The code's language: the first language tag (as the formatter's).
    let code_lang = ast
        .sections
        .iter()
        .find_map(|s| section_tag(s).map(|(_, l)| l));
    let mut out = String::new();
    for (i, s) in ast.sections.iter().enumerate() {
        if i > 0 {
            out.push(';');
        }
        let has_currency = s
            .tokens
            .iter()
            .any(|t| matches!(t, Token::Currency { symbol, .. } if !symbol.is_empty()));
        let tag = section_tag(s);
        let lang = tag.map(|(_, l)| l).or(code_lang);
        let east_asian = lang.is_some_and(LangId::is_cjk);
        let date = matches!(s.kind, SectionKind::Date(d) if d.has_date);
        // [NatNumN] → [DBNumN] or a numeral shape; [~cal] → calendar byte.
        let mut dbnum = None;
        let mut shape = 0u8;
        if let (Some(nn), Some(l)) = (s.natnum, lang) {
            if nn.dbnum {
                dbnum = Some(nn.num);
            } else {
                let d = map_natnum_to_dbnum(nn.num, l, date);
                if l.is_cjk() && d > 0 {
                    dbnum = Some(d);
                } else if let Some(sh) = excel_numeral_shape(l, nn.num) {
                    shape = sh;
                }
            }
        }
        let cal_byte = s
            .tokens
            .iter()
            .find_map(|t| match t {
                Token::Calendar(c) => CalendarId::from_name(c).map(CalendarId::excel_byte),
                _ => None,
            })
            .unwrap_or(0);
        let extended = |l: LangId| {
            format!(
                "[$-{:X}]",
                (u32::from(shape) << 24) | (u32::from(cal_byte) << 16) | u32::from(l.0)
            )
        };
        let needs_tag = (shape != 0 || cal_byte != 0) && !has_currency;
        if let Some(d) = dbnum {
            out.push_str(&format!("[DBNum{d}]"));
        }
        if needs_tag && tag.is_none() {
            if let Some(l) = lang {
                out.push_str(&extended(l));
            }
        }
        for t in &s.tokens {
            match t {
                Token::Currency { symbol, .. } if symbol.is_empty() && has_currency => {}
                Token::Currency {
                    symbol,
                    locale: Some(_),
                } if symbol.is_empty() && needs_tag => {
                    if let Some((_, l)) = tag {
                        out.push_str(&extended(l));
                    }
                }
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
                    DateTimeToken::EraAndYear(1) => out.push_str("ee"),
                    DateTimeToken::EraAndYear(_) => out.push_str("gggee"),
                    DateTimeToken::Era(n) if east_asian => rep(&mut out, 'g', *n),
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
                // Written above as [DBNumN] / the extended locale id.
                Token::NatNum(_) | Token::DbNum(_) | Token::Calendar(_) => {}
                _ => print_token(t, &mut out),
            }
        }
    }
    Ok(out)
}

/// A section's own language tag: the token index and language.
fn section_tag(s: &Section) -> Option<(usize, LangId)> {
    s.tokens.iter().enumerate().find_map(|(i, t)| match t {
        Token::Currency {
            symbol,
            locale: Some(tag),
        } if symbol.is_empty() => crate::locale::lang_id(tag).map(|l| (i, l)),
        _ => None,
    })
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

    #[test]
    fn natnum_and_calendars() {
        // Excel → OOo: numeral and calendar bytes (LibreOffice's reading).
        let from = |c: &str| from_excel_code(c).unwrap();
        assert_eq!(
            from("[$-D07041E]d/m/yyyy"),
            "[NatNum1][$-41E][~buddhist]D/M/YYYY"
        );
        assert_eq!(from("[$-1070409]d/m/yyyy"), "[$-41E][~buddhist]D/M/YYYY");
        assert_eq!(from("[$-2000401]0.00"), "[NatNum1][$-401]0.00");
        assert_eq!(from("[$-1060401]yyyy/mm/dd"), "[$-401][~hijri]YYYY/MM/DD");
        assert_eq!(from("[$-30411]ggge\"年\""), "[$-411][~gengou]GGGE\"年\"");
        assert_eq!(from("[$-1010409]mmm"), "[$-409]MMM");
        assert_eq!(from("[DBNum1][$-411]0"), "[DBNum1][$-411]0");
        assert_eq!(from("[$-ja-JP-x-gannen]ggge"), "[$-ja-JP-x-gannen]GGGE");
        // OOo → Excel.
        let to = |c: &str| to_excel_code(c).unwrap();
        assert_eq!(
            to("[NatNum1][$-41E][~buddhist]D/M/YYYY"),
            "[$-D07041E]d/m/yyyy"
        );
        assert_eq!(to("[NatNum1][$-401]0.00"), "[$-2000401]0.00");
        assert_eq!(to("[$-401][~hijri]YYYY"), "[$-60401]yyyy");
        assert_eq!(to("[NatNum1][$-411]0"), "[DBNum1][$-411]0");
        assert_eq!(to("[NatNum4][$-411]0"), "[DBNum2][$-411]0");
        assert_eq!(to("[NatNum4][$-804]0"), "[DBNum1][$-804]0");
        assert_eq!(to("[NatNum1][$-804]0"), "[$-1E000804]0");
        assert_eq!(to("[DBNum2][$-412]0"), "[DBNum2][$-412]0");
        assert_eq!(to("[$-411]GGGE\"年\"M\"月\""), "[$-411]ggge\"年\"m\"月\"");
        assert_eq!(to("[$-411]RR"), "[$-411]gggee");
        assert_eq!(to("GGG YYYY"), "\"GGG\" yyyy");
        // A NatNum without a language Excel can show is dropped.
        assert_eq!(to("[NatNum1]0"), "0");
        for code in [
            "[NatNum1][$-41E][~buddhist]D/M/YYYY",
            "[NatNum1][$-401]0.00",
            "[$-411]GGGE\"年\"",
            "[DBNum1][$-411]0",
        ] {
            assert_eq!(from(&to(code)), code, "{code}");
        }
    }
}
