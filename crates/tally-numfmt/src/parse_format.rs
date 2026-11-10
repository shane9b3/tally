//! Format-code parser ([`parse_format`]) and printer ([`print_format`]).
//!
//! The grammar follows OOo's `zforscan.cxx` as summarised in
//! `docs/calc-features-inventory.md` §6. Keywords are ASCII
//! case-insensitive; the printer writes them in canonical upper case.

use crate::format_ast::{
    Color, CompareOp, Condition, DateLayout, DateTimeToken, Digit, ElapsedUnit, ExponentLayout,
    FormatAst, FractionLayout, GeneralKeyword, NamedColor, NumberLayout, Section, SectionKind,
    Token,
};
use crate::locale::Locale;

/// Why a format code could not be parsed.
#[derive(Clone, PartialEq, Eq, Debug, thiserror::Error)]
pub enum FormatError {
    /// A `"` string is not closed.
    #[error("unterminated quoted string")]
    UnterminatedQuote,
    /// A `[` modifier is not closed.
    #[error("unterminated [ ] modifier")]
    UnterminatedBracket,
    /// The code ends with `\`, `*` or `_` and no character after it.
    #[error("format code ends after '{0}'")]
    DanglingEscape(char),
    /// More than four sections.
    #[error("more than four sections")]
    TooManySections,
    /// An unknown `[...]` modifier.
    #[error("unknown modifier [{0}]")]
    UnknownBracket(String),
    /// A malformed `[<0]`-style condition.
    #[error("invalid condition [{0}]")]
    InvalidCondition(String),
    /// `[COLORn]` with n outside 1..=64.
    #[error("colour index {0} out of range 1..=64")]
    InvalidColorIndex(u32),
    /// A section mixes the `@` text placeholder with digit placeholders.
    #[error("section mixes @ with digit placeholders")]
    TextAndNumber,
}

/// Parses an OOo format code. An empty code means `General`.
pub fn parse_format(code: &str) -> Result<FormatAst, FormatError> {
    let parts = split_sections(code)?;
    if parts.len() > 4 {
        return Err(FormatError::TooManySections);
    }
    let total = parts.len();
    let mut sections = Vec::with_capacity(total);
    for (i, part) in parts.iter().enumerate() {
        sections.push(build_section(part, i, total)?);
    }
    Ok(FormatAst { sections })
}

/// Parses a format code written with `locale`'s conventions, as AOO's
/// scanner reads a code in the document language (`TEXT(1,7;"0,00")` in a
/// German document is `1,70`):
///
/// - the locale's decimal separator is the decimal point and its grouping
///   separator the grouping comma (de-DE `#.##0,00` is `#,##0.00`); a `.`
///   that is neither is a literal (fr-FR `0.0` prints the dot). A space
///   counts as the grouping separator only between two digit placeholders,
///   so `0 %` keeps its blank;
/// - in date and time sections the separators are kept as literals
///   (`TT.MM.JJJJ`), except the decimal separator before `0`s of fractional
///   seconds (`SS,00`);
/// - the locale's keyword letters ([`Locale::format_letters`]: German `T`
///   and `J`, French `J` and `A`) are accepted next to the English ones;
/// - `;` always separates sections, and quoted text, `[..]` modifiers and
///   escaped characters are untouched.
///
/// For a locale with en-US separators and no localized letters this is
/// [`parse_format`].
pub fn parse_format_with_locale(code: &str, locale: &Locale) -> Result<FormatAst, FormatError> {
    if locale.decimal_sep == '.' && locale.group_sep == ',' && locale.format_letters.is_empty() {
        return parse_format(code);
    }
    let parts = split_sections(code)?;
    if parts.len() > 4 {
        return Err(FormatError::TooManySections);
    }
    let total = parts.len();
    let mut sections = Vec::with_capacity(total);
    for (i, part) in parts.iter().enumerate() {
        let canon = canonicalize(part, locale, true);
        let mut section = build_section(&canon, i, total)?;
        if matches!(section.kind, SectionKind::Date(_)) {
            let canon = canonicalize(part, locale, false);
            section = build_section(&canon, i, total)?;
        }
        sections.push(section);
    }
    Ok(FormatAst { sections })
}

/// Rewrites one section from `locale`'s conventions to the canonical en-US
/// code (see [`parse_format_with_locale`]). With `separators` the number
/// separators are mapped; without, only the fractional-seconds decimal
/// separator is (date and time sections).
fn canonicalize(s: &[char], locale: &Locale, separators: bool) -> Vec<char> {
    let mut out = Vec::with_capacity(s.len() + 2);
    let is_digit = |c: Option<&char>| matches!(c, Some('#' | '0' | '?'));
    let mut i = 0;
    while i < s.len() {
        let c = s[i];
        match c {
            '"' => {
                let mut j = i + 1;
                while j < s.len() && s[j] != '"' {
                    j += 1;
                }
                out.extend_from_slice(&s[i..(j + 1).min(s.len())]);
                i = j + 1;
            }
            '[' => {
                let mut j = i + 1;
                while j < s.len() && s[j] != ']' {
                    j += 1;
                }
                out.extend_from_slice(&s[i..(j + 1).min(s.len())]);
                i = j + 1;
            }
            '\\' | '*' | '_' => {
                out.extend_from_slice(&s[i..(i + 2).min(s.len())]);
                i += 2;
            }
            _ if c.is_ascii_alphabetic() => {
                let mut j = i + 1;
                while j < s.len() && s[j].is_ascii_alphabetic() {
                    j += 1;
                }
                let run = &s[i..j];
                let upper = c.to_ascii_uppercase();
                let uniform = run.iter().all(|x| x.to_ascii_uppercase() == upper);
                // `A/P` is the 12-hour marker, not a run of the French year letter.
                let a_p = run.len() == 1
                    && upper == 'A'
                    && s.get(j) == Some(&'/')
                    && s.get(j + 1).is_some_and(|c| c.eq_ignore_ascii_case(&'P'));
                let mapped = locale
                    .format_letters
                    .iter()
                    .find(|(from, _)| *from == upper)
                    .map(|(_, to)| *to);
                match mapped {
                    Some(to) if uniform && !a_p => {
                        out.extend(std::iter::repeat_n(to, run.len()));
                    }
                    _ => out.extend_from_slice(run),
                }
                i = j;
            }
            _ if separators && c == locale.decimal_sep => {
                out.push('.');
                i += 1;
            }
            _ if separators
                && locale.is_group_char(c)
                && (!c.is_whitespace() || (is_digit(out.last()) && is_digit(s.get(i + 1)))) =>
            {
                out.push(',');
                i += 1;
            }
            '.' | ',' if separators => {
                // Neither separator of this locale: a literal.
                out.push('\\');
                out.push(c);
                i += 1;
            }
            _ if !separators && c == locale.decimal_sep && s.get(i + 1) == Some(&'0') => {
                out.push('.');
                i += 1;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

/// Tokenizes and classifies section `i` of `total`.
fn build_section(part: &[char], i: usize, total: usize) -> Result<Section, FormatError> {
    let (mut tokens, r_src) = tokenize(part)?;
    if total == 1 && tokens.is_empty() {
        tokens.push(Token::General(GeneralKeyword::General));
    }
    let kind = analyze(&mut tokens, i, &r_src)?;
    let condition = tokens.iter().find_map(|t| match t {
        Token::Condition(c) => Some(*c),
        _ => None,
    });
    let color = tokens.iter().find_map(|t| match t {
        Token::Color(c) => Some(*c),
        _ => None,
    });
    // The first modifier counts (AOO rejects a second one).
    let natnum = tokens.iter().find_map(|t| match t {
        Token::NatNum(n) => Some(crate::natnum::NatNum {
            num: *n,
            dbnum: false,
        }),
        Token::DbNum(n) => Some(crate::natnum::NatNum {
            num: *n,
            dbnum: true,
        }),
        _ => None,
    });
    Ok(Section {
        condition,
        color,
        tokens,
        kind,
        natnum,
    })
}

/// Splits at top-level `;` (outside quotes, brackets and escapes).
fn split_sections(code: &str) -> Result<Vec<Vec<char>>, FormatError> {
    let mut out = Vec::new();
    let mut cur = Vec::new();
    let mut chars = code.chars();
    while let Some(c) = chars.next() {
        match c {
            ';' => out.push(std::mem::take(&mut cur)),
            '"' => {
                cur.push(c);
                loop {
                    match chars.next() {
                        Some('"') => {
                            cur.push('"');
                            break;
                        }
                        Some(x) => cur.push(x),
                        None => return Err(FormatError::UnterminatedQuote),
                    }
                }
            }
            '\\' | '*' | '_' => {
                cur.push(c);
                match chars.next() {
                    Some(x) => cur.push(x),
                    None => return Err(FormatError::DanglingEscape(c)),
                }
            }
            '[' => {
                cur.push(c);
                loop {
                    match chars.next() {
                        Some(']') => {
                            cur.push(']');
                            break;
                        }
                        Some(x) => cur.push(x),
                        None => return Err(FormatError::UnterminatedBracket),
                    }
                }
            }
            _ => cur.push(c),
        }
    }
    out.push(cur);
    Ok(out)
}

fn starts_with_ci(s: &[char], at: usize, kw: &str) -> bool {
    let mut i = at;
    for k in kw.chars() {
        match s.get(i) {
            Some(c) if c.eq_ignore_ascii_case(&k) => i += 1,
            _ => return false,
        }
    }
    true
}

/// The length of the run of `letter` at `at`, at most 255 (what a count
/// keeps): run-capped keywords rescan the run on every call, so a long run
/// would cost O(n²).
fn run_len(s: &[char], at: usize, letter: char) -> usize {
    s[at..]
        .iter()
        .take(255)
        .take_while(|c| c.eq_ignore_ascii_case(&letter))
        .count()
}

fn count_u8(n: usize) -> u8 {
    n.min(255) as u8
}

/// Tokenizes one section (already split, so brackets and quotes are closed).
/// Also returns the source characters of the `R` / `RR` tokens, in order,
/// for [`analyze`] to turn them back into literals as typed.
fn tokenize(s: &[char]) -> Result<(Vec<Token>, Vec<char>), FormatError> {
    let mut out = Vec::new();
    let mut r_src = Vec::new();
    let mut i = 0;
    while i < s.len() {
        let c = s[i];
        match c {
            '"' => {
                let start = i + 1;
                let mut j = start;
                while j < s.len() && s[j] != '"' {
                    j += 1;
                }
                if j >= s.len() {
                    return Err(FormatError::UnterminatedQuote);
                }
                out.push(Token::Quoted(s[start..j].iter().collect()));
                i = j + 1;
            }
            '\\' | '*' | '_' => {
                let Some(&x) = s.get(i + 1) else {
                    return Err(FormatError::DanglingEscape(c));
                };
                out.push(match c {
                    '\\' => Token::Escaped(x),
                    '*' => Token::Fill(x),
                    _ => Token::Skip(x),
                });
                i += 2;
            }
            '[' => {
                let start = i + 1;
                let mut j = start;
                while j < s.len() && s[j] != ']' {
                    j += 1;
                }
                if j >= s.len() {
                    return Err(FormatError::UnterminatedBracket);
                }
                let content: String = s[start..j].iter().collect();
                out.push(parse_bracket(&content)?);
                i = j + 1;
            }
            '0' => {
                out.push(Token::Digit(Digit::Zero));
                i += 1;
            }
            '#' => {
                out.push(Token::Digit(Digit::Hash));
                i += 1;
            }
            '?' => {
                out.push(Token::Digit(Digit::Question));
                i += 1;
            }
            '.' => {
                out.push(Token::DecimalPoint);
                i += 1;
            }
            ',' => {
                out.push(Token::Comma);
                i += 1;
            }
            '%' => {
                out.push(Token::Percent);
                i += 1;
            }
            '@' => {
                out.push(Token::At);
                i += 1;
            }
            '/' => {
                out.push(Token::Slash);
                i += 1;
            }
            'E' | 'e' if matches!(s.get(i + 1), Some('+') | Some('-')) => {
                out.push(Token::Exponent {
                    lower: c == 'e',
                    plus: s[i + 1] == '+',
                });
                i += 2;
            }
            _ if c.is_ascii_alphabetic() => {
                let (tok, n) = keyword(s, i);
                if matches!(tok, Token::DateTime(DateTimeToken::EraAndYear(_))) {
                    r_src.extend_from_slice(&s[i..i + n]);
                }
                out.push(tok);
                i += n;
            }
            _ => {
                out.push(Token::Literal(c));
                i += 1;
            }
        }
    }
    Ok((out, r_src))
}

/// Recognises a keyword at `s[i]` (an ASCII letter). Returns the token and
/// the number of characters consumed.
fn keyword(s: &[char], i: usize) -> (Token, usize) {
    if starts_with_ci(s, i, "GENERAL") {
        return (Token::General(GeneralKeyword::General), 7);
    }
    if starts_with_ci(s, i, "STANDARD") {
        return (Token::General(GeneralKeyword::Standard), 8);
    }
    if starts_with_ci(s, i, "BOOLEAN") {
        return (Token::General(GeneralKeyword::Boolean), 7);
    }
    if starts_with_ci(s, i, "AM/PM") {
        return (
            Token::DateTime(DateTimeToken::AmPm(s[i..i + 5].iter().collect())),
            5,
        );
    }
    if starts_with_ci(s, i, "A/P") {
        return (
            Token::DateTime(DateTimeToken::AP(s[i..i + 3].iter().collect())),
            3,
        );
    }
    let c = s[i].to_ascii_uppercase();
    let n = run_len(s, i, c);
    let k = count_u8(n);
    let dt = match c {
        'D' if n <= 2 => DateTimeToken::Day(k),
        'D' => DateTimeToken::DayName(k),
        'M' => DateTimeToken::Month(k),
        'Y' => DateTimeToken::Year(k),
        'H' => DateTimeToken::Hour(k),
        'S' => DateTimeToken::Second(k),
        'N' if n >= 2 => DateTimeToken::AooDayName(k.min(4)),
        'A' if n >= 3 => DateTimeToken::AaaDayName(k.min(4)),
        'Q' => DateTimeToken::Quarter(k.min(2)),
        'W' if n >= 2 => DateTimeToken::Week(k),
        'G' => DateTimeToken::Era(k.min(3)),
        'E' => DateTimeToken::EraYear(k.min(2)),
        'R' => DateTimeToken::EraAndYear(k.min(2)),
        _ => return (Token::Literal(s[i]), 1),
    };
    // Tokens with a capped count consume only what they print back as.
    let consumed = match &dt {
        DateTimeToken::AooDayName(k)
        | DateTimeToken::AaaDayName(k)
        | DateTimeToken::Quarter(k)
        | DateTimeToken::Era(k)
        | DateTimeToken::EraYear(k)
        | DateTimeToken::EraAndYear(k) => usize::from(*k),
        _ => n,
    };
    (Token::DateTime(dt), consumed)
}

fn parse_bracket(content: &str) -> Result<Token, FormatError> {
    let unknown = || FormatError::UnknownBracket(content.to_string());
    let first = content.chars().next().ok_or_else(unknown)?;
    match first {
        '$' => {
            let body = &content[1..];
            // `[$-ja-JP-x-gannen]`: a BCP 47 language tag (Excel).
            if let Some(tag) = body.strip_prefix('-') {
                if tag.contains('-') && crate::locale::lang_id(tag).is_some() {
                    return Ok(Token::Currency {
                        symbol: String::new(),
                        locale: Some(tag.to_string()),
                    });
                }
            }
            if let Some(pos) = body.rfind('-') {
                let loc = &body[pos + 1..];
                if !loc.is_empty() && loc.chars().all(|c| c.is_ascii_hexdigit()) {
                    return Ok(Token::Currency {
                        symbol: body[..pos].to_string(),
                        locale: Some(loc.to_string()),
                    });
                }
            }
            Ok(Token::Currency {
                symbol: body.to_string(),
                locale: None,
            })
        }
        '~' => Ok(Token::Calendar(content[1..].to_string())),
        '<' | '>' | '=' => parse_condition(content),
        _ => {
            let upper = content.to_ascii_uppercase();
            // zformat.cxx accepts NatNum0..19 and DBNum1..9.
            if let Some(n) = upper.strip_prefix("NATNUM") {
                return match n.parse::<u8>() {
                    Ok(v) if v <= 19 => Ok(Token::NatNum(v)),
                    _ => Err(unknown()),
                };
            }
            if let Some(n) = upper.strip_prefix("DBNUM") {
                return match n.parse::<u8>() {
                    Ok(v) if (1..=9).contains(&v) => Ok(Token::DbNum(v)),
                    _ => Err(unknown()),
                };
            }
            if let Some(n) = upper.strip_prefix("COLOR") {
                let n: u32 = n.parse().map_err(|_| unknown())?;
                if !(1..=64).contains(&n) {
                    return Err(FormatError::InvalidColorIndex(n));
                }
                return Ok(Token::Color(Color::Index(n as u8)));
            }
            if upper == "GRAY" {
                return Ok(Token::Color(Color::Named(NamedColor::Grey)));
            }
            if let Some(c) = NamedColor::ALL.iter().find(|c| c.keyword() == upper) {
                return Ok(Token::Color(Color::Named(*c)));
            }
            let letter = upper.chars().next().ok_or_else(unknown)?;
            if upper.chars().all(|c| c == letter) {
                let unit = match letter {
                    'H' => Some(ElapsedUnit::Hours),
                    'M' => Some(ElapsedUnit::Minutes),
                    'S' => Some(ElapsedUnit::Seconds),
                    _ => None,
                };
                if let Some(unit) = unit {
                    return Ok(Token::DateTime(DateTimeToken::Elapsed(
                        unit,
                        count_u8(upper.len()),
                    )));
                }
            }
            Err(unknown())
        }
    }
}

fn parse_condition(content: &str) -> Result<Token, FormatError> {
    let (op, rest) = if let Some(r) = content.strip_prefix("<=") {
        (CompareOp::Le, r)
    } else if let Some(r) = content.strip_prefix(">=") {
        (CompareOp::Ge, r)
    } else if let Some(r) = content.strip_prefix("<>") {
        (CompareOp::Ne, r)
    } else if let Some(r) = content.strip_prefix('<') {
        (CompareOp::Lt, r)
    } else if let Some(r) = content.strip_prefix('>') {
        (CompareOp::Gt, r)
    } else if let Some(r) = content.strip_prefix('=') {
        (CompareOp::Eq, r)
    } else {
        return Err(FormatError::InvalidCondition(content.to_string()));
    };
    let rest = rest.trim();
    let valid = !rest.is_empty()
        && rest
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E'));
    match rest.parse::<f64>() {
        Ok(v) if valid && v.is_finite() => Ok(Token::Condition(Condition { op, value: v })),
        _ => Err(FormatError::InvalidCondition(content.to_string())),
    }
}

fn is_seconds(t: &Token) -> bool {
    matches!(
        t,
        Token::DateTime(DateTimeToken::Second(_))
            | Token::DateTime(DateTimeToken::Elapsed(ElapsedUnit::Seconds, _))
    )
}

/// Classifies a section and normalises token roles in place.
fn analyze(
    tokens: &mut Vec<Token>,
    index: usize,
    r_src: &[char],
) -> Result<SectionKind, FormatError> {
    let mut has_dt = tokens.iter().any(|t| matches!(t, Token::DateTime(_)));
    let only_r = has_dt
        && tokens.iter().all(|t| {
            !matches!(t, Token::DateTime(_))
                || matches!(t, Token::DateTime(DateTimeToken::EraAndYear(_)))
        });
    if only_r
        && tokens
            .iter()
            .any(|t| matches!(t, Token::Digit(_) | Token::General(_) | Token::At))
    {
        // `R 0.00` (rand): the era code R only counts in a date section.
        // Rebuilt in one pass, each R as typed (`r0` stays `r0`).
        let mut src = r_src.iter().copied();
        let old = std::mem::take(tokens);
        tokens.reserve(old.len() + r_src.len());
        for t in old {
            if let Token::DateTime(DateTimeToken::EraAndYear(n)) = t {
                for _ in 0..n {
                    tokens.push(Token::Literal(src.next().unwrap_or('R')));
                }
            } else {
                tokens.push(t);
            }
        }
        has_dt = false;
    }
    let has_general = tokens.iter().any(|t| matches!(t, Token::General(_)));
    let has_at = tokens.iter().any(|t| matches!(t, Token::At));
    let digits = tokens
        .iter()
        .filter(|t| matches!(t, Token::Digit(_)))
        .count();
    if has_dt {
        return Ok(SectionKind::Date(analyze_date(tokens)));
    }
    if has_general || has_at || index == 3 {
        if has_at && digits > 0 && !has_general && index != 3 {
            return Err(FormatError::TextAndNumber);
        }
        literalize_number_tokens(tokens);
        return Ok(if has_general && !has_at {
            SectionKind::General
        } else {
            SectionKind::Text
        });
    }
    Ok(SectionKind::Number(analyze_number(tokens)))
}

/// Turns number-only tokens into literals (for General and text sections).
fn literalize_number_tokens(tokens: &mut [Token]) {
    for t in tokens.iter_mut() {
        let lit = match t {
            Token::Comma => ',',
            Token::Slash => '/',
            Token::DecimalPoint => '.',
            Token::Percent => '%',
            Token::Digit(d) => d.as_char(),
            _ => continue,
        };
        *t = Token::Literal(lit);
    }
}

fn analyze_date(tokens: &mut [Token]) -> DateLayout {
    let mut lay = DateLayout::default();
    // Month vs minute: M/MM right after an hour keyword or right before a
    // seconds keyword (ignoring literals in between) means minutes.
    let dt_idx: Vec<usize> = tokens
        .iter()
        .enumerate()
        .filter(|(_, t)| matches!(t, Token::DateTime(_)))
        .map(|(i, _)| i)
        .collect();
    for (k, &i) in dt_idx.iter().enumerate() {
        if let Token::DateTime(DateTimeToken::Month(n)) = tokens[i] {
            if n > 2 {
                continue;
            }
            let after_hour = k > 0
                && matches!(
                    tokens[dt_idx[k - 1]],
                    Token::DateTime(DateTimeToken::Hour(_))
                        | Token::DateTime(DateTimeToken::Elapsed(ElapsedUnit::Hours, _))
                );
            let before_sec = dt_idx.get(k + 1).is_some_and(|&j| is_seconds(&tokens[j]));
            if after_hour || before_sec {
                tokens[i] = Token::DateTime(DateTimeToken::Minute(n));
            }
        }
    }
    // Fractional seconds: a decimal point right after a seconds keyword
    // followed by `0`s. Every other number token becomes a literal.
    let mut i = 0;
    while i < tokens.len() {
        if matches!(tokens[i], Token::DecimalPoint) && i > 0 && is_seconds(&tokens[i - 1]) {
            let mut j = i + 1;
            while j < tokens.len() && matches!(tokens[j], Token::Digit(Digit::Zero)) {
                j += 1;
            }
            lay.frac_sec_digits = lay.frac_sec_digits.max(count_u8(j - i - 1).min(9));
            i = j;
            continue;
        }
        let lit = match &tokens[i] {
            Token::Comma => Some(','),
            Token::Slash => Some('/'),
            Token::DecimalPoint => Some('.'),
            Token::Percent => Some('%'),
            Token::At => Some('@'),
            Token::Digit(d) => Some(d.as_char()),
            _ => None,
        };
        if let Some(c) = lit {
            tokens[i] = Token::Literal(c);
        }
        i += 1;
    }
    for t in tokens.iter() {
        if let Token::DateTime(dt) = t {
            match dt {
                DateTimeToken::AmPm(_) | DateTimeToken::AP(_) => lay.twelve_hour = true,
                DateTimeToken::Elapsed(u, _) => {
                    lay.has_time = true;
                    if lay.elapsed.is_none() {
                        lay.elapsed = Some(*u);
                    }
                }
                DateTimeToken::Hour(_) | DateTimeToken::Minute(_) | DateTimeToken::Second(_) => {
                    lay.has_time = true
                }
                DateTimeToken::Era(_)
                | DateTimeToken::EraYear(_)
                | DateTimeToken::EraAndYear(_)
                | DateTimeToken::AaaDayName(_) => {
                    lay.has_date = true;
                    lay.calendar = true;
                }
                _ => lay.has_date = true,
            }
        }
        match t {
            Token::Calendar(_) => lay.calendar = true,
            Token::Currency {
                symbol,
                locale: Some(tag),
            } if symbol.is_empty() && tag.contains('-') => lay.calendar = true,
            _ => {}
        }
    }
    lay
}

fn analyze_number(tokens: &mut [Token]) -> NumberLayout {
    let mut lay = NumberLayout::default();
    let len = tokens.len();
    let exp_idx = tokens
        .iter()
        .position(|t| matches!(t, Token::Exponent { .. }));
    let mantissa_end = exp_idx.unwrap_or(len);
    let dec_idx = tokens[..mantissa_end]
        .iter()
        .position(|t| matches!(t, Token::DecimalPoint));
    // Extra decimal points are literals.
    for (i, t) in tokens.iter_mut().enumerate() {
        if matches!(t, Token::DecimalPoint) && Some(i) != dec_idx {
            *t = Token::Literal('.');
        }
    }
    // Fraction: a slash directly after a digit placeholder, no decimal
    // point, no exponent.
    let mut fraction = None;
    if exp_idx.is_none() && dec_idx.is_none() {
        if let Some(slash) = tokens.iter().position(|t| matches!(t, Token::Slash)) {
            fraction = fraction_layout(tokens, slash);
        }
    }
    for (i, t) in tokens.iter_mut().enumerate() {
        if matches!(t, Token::Slash) && fraction.map(|f| f.slash) != Some(i) {
            *t = Token::Literal('/');
        }
    }
    // Commas: grouping between placeholders in the integer part, scaling
    // when trailing a placeholder, literal otherwise.
    let int_end = dec_idx.unwrap_or(mantissa_end);
    let mut seen_digit = false;
    let mut prev_scale = false;
    for i in 0..len {
        match tokens[i] {
            Token::Digit(_) => {
                seen_digit = true;
                prev_scale = false;
            }
            Token::Comma => {
                let prev_digit = i > 0 && matches!(tokens[i - 1], Token::Digit(_));
                let next_digit = matches!(tokens.get(i + 1), Some(Token::Digit(_)));
                let in_mantissa = i < mantissa_end;
                if seen_digit && prev_digit && next_digit && i < int_end && fraction.is_none() {
                    lay.grouping = true;
                    prev_scale = false;
                } else if in_mantissa && (prev_digit || prev_scale) && !next_digit {
                    lay.scale_commas = lay.scale_commas.saturating_add(1);
                    prev_scale = true;
                } else {
                    tokens[i] = Token::Literal(',');
                    prev_scale = false;
                }
            }
            Token::Percent => {
                lay.percent = lay.percent.saturating_add(1);
                prev_scale = false;
            }
            _ => prev_scale = false,
        }
    }
    lay.decimal_index = dec_idx;
    lay.int_end = int_end;
    lay.frac_end = mantissa_end;
    for (i, t) in tokens.iter().enumerate() {
        if let Token::Digit(d) = t {
            if i < int_end {
                lay.int_digits += 1;
                if *d == Digit::Zero {
                    lay.min_int += 1;
                }
            } else if dec_idx.is_some() && i < mantissa_end {
                lay.frac_digits += 1;
                if *d == Digit::Zero {
                    lay.min_frac += 1;
                }
            }
        }
    }
    if let Some(e) = exp_idx {
        if let Token::Exponent { plus, .. } = tokens[e] {
            let digits = tokens[e + 1..]
                .iter()
                .take_while(|t| matches!(t, Token::Digit(_)))
                .count();
            lay.exponent = Some(ExponentLayout {
                index: e,
                digits: count_u8(digits.max(1)),
                plus,
            });
        }
    }
    lay.fraction = fraction;
    lay
}

fn fraction_layout(tokens: &mut [Token], slash: usize) -> Option<FractionLayout> {
    if slash == 0 || !matches!(tokens[slash - 1], Token::Digit(_)) {
        return None;
    }
    let mut ns = slash;
    while ns > 0 && matches!(tokens[ns - 1], Token::Digit(_)) {
        ns -= 1;
    }
    let int_range = if tokens[..ns].iter().any(|t| matches!(t, Token::Digit(_))) {
        Some((0, ns))
    } else {
        None
    };
    let mut j = slash + 1;
    let mut fixed: Option<u32> = None;
    if let Some(Token::Literal(c)) = tokens.get(j) {
        if ('1'..='9').contains(c) {
            let mut v: u32 = 0;
            while j < tokens.len() {
                let d = match tokens[j] {
                    Token::Literal(c) if c.is_ascii_digit() => c as u32 - '0' as u32,
                    Token::Digit(Digit::Zero) => 0,
                    _ => break,
                };
                v = v.saturating_mul(10).saturating_add(d);
                j += 1;
            }
            fixed = Some(v);
        }
    }
    let mut den_digits = 0u8;
    if fixed.is_none() {
        while j < tokens.len() && matches!(tokens[j], Token::Digit(_)) {
            j += 1;
            den_digits = den_digits.saturating_add(1);
        }
        if den_digits == 0 {
            return None;
        }
    }
    Some(FractionLayout {
        int_range,
        num_range: (ns, slash),
        slash,
        den_range: (slash + 1, j),
        den_digits,
        fixed_den: fixed,
    })
}

/// Prints a format code. `print_format(&parse_format(c)?)` reproduces `c`
/// for codes written with canonical upper-case keywords.
pub fn print_format(ast: &FormatAst) -> String {
    let mut out = String::new();
    for (i, s) in ast.sections.iter().enumerate() {
        if i > 0 {
            out.push(';');
        }
        for t in &s.tokens {
            print_token(t, &mut out);
        }
    }
    out
}

fn repeat(out: &mut String, c: char, n: u8) {
    for _ in 0..n {
        out.push(c);
    }
}

pub(crate) fn print_token(t: &Token, out: &mut String) {
    match t {
        Token::Digit(d) => out.push(d.as_char()),
        Token::DecimalPoint => out.push('.'),
        Token::Comma => out.push(','),
        Token::Percent => out.push('%'),
        Token::Exponent { lower, plus } => {
            out.push(if *lower { 'e' } else { 'E' });
            out.push(if *plus { '+' } else { '-' });
        }
        Token::Slash => out.push('/'),
        Token::At => out.push('@'),
        Token::Quoted(s) => {
            out.push('"');
            out.push_str(s);
            out.push('"');
        }
        Token::Escaped(c) => {
            out.push('\\');
            out.push(*c);
        }
        Token::Literal(c) => out.push(*c),
        Token::Fill(c) => {
            out.push('*');
            out.push(*c);
        }
        Token::Skip(c) => {
            out.push('_');
            out.push(*c);
        }
        Token::DateTime(dt) => print_dt(dt, out),
        Token::General(GeneralKeyword::General) => out.push_str("General"),
        Token::General(GeneralKeyword::Standard) => out.push_str("Standard"),
        Token::General(GeneralKeyword::Boolean) => out.push_str("BOOLEAN"),
        Token::Color(Color::Named(c)) => {
            out.push('[');
            out.push_str(c.keyword());
            out.push(']');
        }
        Token::Color(Color::Index(n)) => {
            out.push_str("[COLOR");
            out.push_str(&n.to_string());
            out.push(']');
        }
        Token::Condition(c) => {
            out.push('[');
            out.push_str(c.op.as_str());
            out.push_str(&format!("{}", c.value));
            out.push(']');
        }
        Token::Currency { symbol, locale } => {
            out.push_str("[$");
            out.push_str(symbol);
            if let Some(l) = locale {
                out.push('-');
                out.push_str(l);
            }
            out.push(']');
        }
        Token::NatNum(n) => out.push_str(&format!("[NatNum{n}]")),
        Token::DbNum(n) => out.push_str(&format!("[DBNum{n}]")),
        Token::Calendar(c) => {
            out.push_str("[~");
            out.push_str(c);
            out.push(']');
        }
    }
}

fn print_dt(dt: &DateTimeToken, out: &mut String) {
    match dt {
        DateTimeToken::Day(n) | DateTimeToken::DayName(n) => repeat(out, 'D', *n),
        DateTimeToken::AooDayName(n) => repeat(out, 'N', *n),
        DateTimeToken::AaaDayName(n) => repeat(out, 'A', *n),
        DateTimeToken::Month(n) | DateTimeToken::Minute(n) => repeat(out, 'M', *n),
        DateTimeToken::Year(n) => repeat(out, 'Y', *n),
        DateTimeToken::Hour(n) => repeat(out, 'H', *n),
        DateTimeToken::Second(n) => repeat(out, 'S', *n),
        DateTimeToken::AmPm(s) | DateTimeToken::AP(s) => out.push_str(s),
        DateTimeToken::Quarter(n) => repeat(out, 'Q', *n),
        DateTimeToken::Week(n) => repeat(out, 'W', *n),
        DateTimeToken::Era(n) => repeat(out, 'G', *n),
        DateTimeToken::EraYear(n) => repeat(out, 'E', *n),
        DateTimeToken::EraAndYear(n) => repeat(out, 'R', *n),
        DateTimeToken::Elapsed(u, n) => {
            out.push('[');
            let c = match u {
                ElapsedUnit::Hours => 'H',
                ElapsedUnit::Minutes => 'M',
                ElapsedUnit::Seconds => 'S',
            };
            repeat(out, c, *n);
            out.push(']');
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tally_types::Value;

    #[test]
    fn minutes_vs_months() {
        let ast = parse_format("MM/DD/YYYY HH:MM").unwrap();
        let toks = &ast.sections[0].tokens;
        assert_eq!(toks[0], Token::DateTime(DateTimeToken::Month(2)));
        assert_eq!(
            toks.last(),
            Some(&Token::DateTime(DateTimeToken::Minute(2)))
        );
        let ast = parse_format("MM:SS").unwrap();
        assert_eq!(
            ast.sections[0].tokens[0],
            Token::DateTime(DateTimeToken::Minute(2))
        );
    }

    #[test]
    fn number_layout() {
        let ast = parse_format("#,##0.00").unwrap();
        let SectionKind::Number(l) = ast.sections[0].kind else {
            panic!()
        };
        assert!(l.grouping);
        assert_eq!(
            (l.int_digits, l.min_int, l.frac_digits, l.min_frac),
            (4, 1, 2, 2)
        );
        let ast = parse_format("#,##0,").unwrap();
        let SectionKind::Number(l) = ast.sections[0].kind else {
            panic!()
        };
        assert_eq!(l.scale_commas, 1);
        let ast = parse_format("# ??/16").unwrap();
        let SectionKind::Number(l) = ast.sections[0].kind else {
            panic!()
        };
        assert_eq!(l.fraction.unwrap().fixed_den, Some(16));
    }

    #[test]
    fn errors() {
        assert_eq!(parse_format("\"abc"), Err(FormatError::UnterminatedQuote));
        assert_eq!(parse_format("[RED"), Err(FormatError::UnterminatedBracket));
        assert_eq!(parse_format("0;0;0;@;0"), Err(FormatError::TooManySections));
        assert!(matches!(
            parse_format("[FOO]0"),
            Err(FormatError::UnknownBracket(_))
        ));
        assert_eq!(
            parse_format("[COLOR65]0"),
            Err(FormatError::InvalidColorIndex(65))
        );
        assert!(matches!(
            parse_format("[<abc]0"),
            Err(FormatError::InvalidCondition(_))
        ));
        assert_eq!(parse_format("0@"), Err(FormatError::TextAndNumber));
        assert_eq!(parse_format("0\\"), Err(FormatError::DanglingEscape('\\')));
    }

    #[test]
    fn localized_separators() {
        use crate::{format_value, DE_DE, EN_US, FR_FR};
        let de = |code: &str| parse_format_with_locale(code, &DE_DE).unwrap();
        assert_eq!(print_format(&de("0,00")), "0.00");
        assert_eq!(print_format(&de("#.##0,00")), "#,##0.00");
        assert_eq!(
            print_format(&de("#.##0,00 [$€-407];[RED]-#.##0,00 [$€-407]")),
            "#,##0.00 [$€-407];[RED]-#,##0.00 [$€-407]"
        );
        // Scaling commas and the percent sign work in the locale spelling.
        assert_eq!(print_format(&de("#.##0.")), "#,##0,");
        assert_eq!(print_format(&de("0,0%")), "0.0%");
        // Quoted text, modifiers and escapes are untouched.
        assert_eq!(
            print_format(&de("0,00 \"Stück, netto\"")),
            "0.00 \"Stück, netto\""
        );
        assert_eq!(print_format(&de("[>1.5]0,0;0")), "[>1.5]0.0;0");
        assert_eq!(print_format(&de("0\\.0,0")), "0\\.0.0");
        let r = format_value(&Value::Number(1.7), &de("0,00"), &DE_DE, None);
        assert_eq!(r.text, "1,70");
        let r = format_value(&Value::Number(1234.5), &de("#.##0,00"), &DE_DE, None);
        assert_eq!(r.text, "1.234,50");
        // en-US codes still parse under en-US; the en-US code under de-DE
        // reads "." as the grouping separator and "," as the decimal point.
        assert_eq!(print_format(&de("0.00")), "0,00");
        let r = format_value(&Value::Number(1.7), &de("0.00"), &DE_DE, None);
        assert_eq!(r.text, "002");
        assert_eq!(
            print_format(&parse_format_with_locale("#,##0.00", &EN_US).unwrap()),
            "#,##0.00"
        );
        // fr-FR: NBSP or a space between placeholders groups; a dot is literal.
        let fr = |code: &str| parse_format_with_locale(code, &FR_FR).unwrap();
        assert_eq!(print_format(&fr("# ##0,00")), "#,##0.00");
        assert_eq!(print_format(&fr("#\u{a0}##0,00")), "#,##0.00");
        assert_eq!(print_format(&fr("0 %")), "0 %");
        assert_eq!(print_format(&fr("0.0")), "0\\.0");
        let r = format_value(&Value::Number(1234.5), &fr("# ##0,00"), &FR_FR, None);
        assert_eq!(r.text, "1\u{a0}234,50");
    }

    #[test]
    fn localized_keywords_and_dates() {
        use crate::{format_value, DE_DE, ES_ES, FR_FR};
        let de = |code: &str| parse_format_with_locale(code, &DE_DE).unwrap();
        assert_eq!(print_format(&de("TT.MM.JJJJ")), "DD.MM.YYYY");
        assert_eq!(print_format(&de("tt.mm.jj")), "DD.MM.YY");
        assert_eq!(print_format(&de("DD.MM.YYYY")), "DD.MM.YYYY");
        assert_eq!(print_format(&de("NN, T. MMMM JJJJ")), "NN, D. MMMM YYYY");
        assert_eq!(print_format(&de("HH:MM:SS,00")), "HH:MM:SS.00");
        assert_eq!(print_format(&de("[HH]:MM:SS")), "[HH]:MM:SS");
        // 2026-10-19 (serial 46314) in the 1899 epoch.
        let r = format_value(&Value::Number(46314.0), &de("TT.MM.JJJJ"), &DE_DE, None);
        assert_eq!(r.text, "19.10.2026");
        let r = format_value(&Value::Number(46314.0), &de("DD.MM.YYYY"), &DE_DE, None);
        assert_eq!(r.text, "19.10.2026");
        // Mixed letter runs and AM/PM are not keyword runs.
        assert_eq!(print_format(&de("HH:MM AM/PM")), "HH:MM AM/PM");
        let fr = |code: &str| parse_format_with_locale(code, &FR_FR).unwrap();
        assert_eq!(print_format(&fr("JJ/MM/AAAA")), "DD/MM/YYYY");
        assert_eq!(print_format(&fr("HH:MM A/P")), "HH:MM A/P");
        assert_eq!(print_format(&fr("HH:MM AM/PM")), "HH:MM AM/PM");
        let es = |code: &str| parse_format_with_locale(code, &ES_ES).unwrap();
        assert_eq!(print_format(&es("DD/MM/AAAA")), "DD/MM/YYYY");
        // Text and General sections are unchanged.
        assert_eq!(print_format(&de("@")), "@");
        assert_eq!(
            print_format(&de("Standard")),
            print_format(&parse_format("Standard").unwrap())
        );
        assert_eq!(
            parse_format_with_locale("0,0;0;0;@;0", &DE_DE),
            Err(FormatError::TooManySections)
        );
    }
}
