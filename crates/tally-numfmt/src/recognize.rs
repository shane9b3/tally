//! Input recognition: typed cell text → value plus an implied format.
//!
//! Rules (en-US, following AOO's `ImpSvNumberInputScan` behaviour):
//!
//! - text starting with `=` is returned as text (formulas are handled by
//!   the caller); a leading `'` forces text and is stripped;
//! - `TRUE` / `FALSE` (any case) are booleans;
//! - numbers: optional sign, correctly placed thousands separators,
//!   decimals, exponent; `(123)` is −123; trailing `%` divides by 100;
//!   a leading currency symbol (`$5`, `-$5`, `$-5`, `($5)`) gives the
//!   currency format;
//! - mixed fractions `1 1/2`; a bare `1/2` is a **date** (January 2 of the
//!   current year), as in AOO;
//! - dates `10/19/2026`, `10/19/26` (two-digit-year pivot), `10/19`,
//!   `2026-10-19`, `19 Oct 2026`, `19-Oct-26`, `Oct 19, 2026`,
//!   `October 19 2026`, `Oct 2026`;
//! - times `14:30`, `14:30:15`, `2:30 PM`, `2:30:15.5 pm`; hours of 24 or
//!   more without AM/PM are durations (`[HH]:MM`), and so are negative
//!   times (`-6:00:00` → −0.25 with `[HH]:MM:SS`);
//! - date followed by time;
//! - anything else is text (the original input, untrimmed).

use tally_types::{round_to_15_digits, DateEpoch, Value};

use crate::builtin::{builtin_format, QuickFormat};
use crate::date::{serial_from_ymd, today_ymd};
use crate::locale::{DateOrder, Locale};

/// The result of [`recognize_input`].
#[derive(Clone, PartialEq, Debug)]
pub struct Recognized {
    /// The recognised value.
    pub value: Value,
    /// A format code to apply to the cell if it still has the General
    /// format (`None`: keep the current format).
    pub implied_format: Option<String>,
}

impl Recognized {
    fn text(s: &str) -> Self {
        Recognized {
            value: Value::text(s),
            implied_format: None,
        }
    }

    fn number(v: f64, fmt: Option<String>) -> Self {
        Recognized {
            value: Value::Number(v),
            implied_format: fmt,
        }
    }
}

/// Inputs to recognition that are not part of the typed text.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub struct RecognizeContext {
    /// Epoch for date serials.
    pub epoch: DateEpoch,
    /// Today's date `(y, m, d)`, used when the year is omitted.
    pub today: (i32, u32, u32),
}

impl RecognizeContext {
    /// A context for today's local date.
    pub fn now(epoch: DateEpoch) -> Self {
        RecognizeContext {
            epoch,
            today: today_ymd(),
        }
    }
}

/// Recognises typed input using the 1899-12-30 epoch and today's date.
///
/// Engine code should call [`recognize_input_with`] with the document epoch.
pub fn recognize_input(input: &str, locale: &Locale) -> Recognized {
    recognize_input_with(input, locale, &RecognizeContext::now(DateEpoch::D1899))
}

/// Recognises typed input.
pub fn recognize_input_with(input: &str, locale: &Locale, ctx: &RecognizeContext) -> Recognized {
    if input.starts_with('=') {
        return Recognized::text(input);
    }
    if let Some(rest) = input.strip_prefix('\'') {
        return Recognized::text(rest);
    }
    let t = input.trim();
    if t.is_empty() {
        return Recognized {
            value: Value::Empty,
            implied_format: None,
        };
    }
    let is = |name: &str| t.to_lowercase() == name.to_lowercase();
    if is(locale.true_name) || t.eq_ignore_ascii_case("TRUE") {
        return Recognized {
            value: Value::Bool(true),
            implied_format: None,
        };
    }
    if is(locale.false_name) || t.eq_ignore_ascii_case("FALSE") {
        return Recognized {
            value: Value::Bool(false),
            implied_format: None,
        };
    }
    if let Some((v, fmt)) = try_number(t, locale) {
        return Recognized::number(v, fmt);
    }
    if let Some((v, fmt)) = try_fraction(t) {
        return Recognized::number(v, Some(fmt));
    }
    if let Some((v, fmt)) = try_datetime(t, locale, ctx) {
        return Recognized::number(v, Some(fmt));
    }
    Recognized::text(input)
}

/// Details of a parsed unsigned number.
struct NumInfo {
    value: f64,
    frac_digits: usize,
    grouped: bool,
    exponent: bool,
}

/// Parses `digits[,ddd…][.digits][e[±]digits]` covering all of `b`.
fn parse_unsigned(b: &[u8], dec: u8, grp: Option<u8>) -> Option<NumInfo> {
    let mut clean = String::with_capacity(b.len() + 2);
    let mut i = 0;
    let mut int_len = 0;
    while i < b.len() && b[i].is_ascii_digit() {
        clean.push(char::from(b[i]));
        i += 1;
        int_len += 1;
    }
    let mut grouped = false;
    if let Some(g) = grp {
        if i < b.len() && b[i] == g && (1..=3).contains(&int_len) {
            while i < b.len() && b[i] == g {
                let grp_digits = &b.get(i + 1..i + 4)?;
                if grp_digits.len() != 3 || !grp_digits.iter().all(u8::is_ascii_digit) {
                    return None;
                }
                for &c in grp_digits.iter() {
                    clean.push(char::from(c));
                }
                i += 4;
                grouped = true;
            }
            if i < b.len() && b[i].is_ascii_digit() {
                return None;
            }
        }
    }
    let mut frac_digits = 0;
    if i < b.len() && b[i] == dec {
        clean.push('.');
        i += 1;
        while i < b.len() && b[i].is_ascii_digit() {
            clean.push(char::from(b[i]));
            i += 1;
            frac_digits += 1;
        }
    }
    if int_len + frac_digits == 0 {
        return None;
    }
    let mut exponent = false;
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        if grouped {
            return None;
        }
        clean.push('e');
        i += 1;
        if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
            clean.push(char::from(b[i]));
            i += 1;
        }
        let start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            clean.push(char::from(b[i]));
            i += 1;
        }
        if i == start {
            return None;
        }
        exponent = true;
    }
    if i != b.len() {
        return None;
    }
    let value: f64 = clean.parse().ok()?;
    if !value.is_finite() {
        return None;
    }
    Some(NumInfo {
        value,
        frac_digits,
        grouped,
        exponent,
    })
}

/// A leading sign and the blanks after it (AOO `ScanStartString`:
/// `GetSign` then `SkipBlanks`, so `- 5` is −5).
fn strip_sign(s: &str) -> (Option<bool>, &str) {
    let blanks = [' ', '\u{a0}'];
    if let Some(r) = s.strip_prefix('-') {
        (Some(true), r.trim_start_matches(blanks))
    } else if let Some(r) = s.strip_prefix('+') {
        (Some(false), r.trim_start_matches(blanks))
    } else {
        (None, s)
    }
}

fn try_number(t: &str, locale: &Locale) -> Option<(f64, Option<String>)> {
    let mut s = t;
    let mut paren = false;
    if s.starts_with('(') && s.ends_with(')') && s.len() >= 2 {
        paren = true;
        s = &s[1..s.len() - 1];
    }
    let (mut sign, rest) = strip_sign(s);
    s = rest;
    let mut currency = false;
    // The half- and full-width yen signs are the same currency (AOO ja-JP
    // uses `￥`).
    let alt = match locale.currency_symbol {
        "￥" => "¥",
        "¥" => "￥",
        other => other,
    };
    let prefix = s
        .strip_prefix(locale.currency_symbol)
        .or_else(|| s.strip_prefix(alt));
    if let Some(r) = prefix {
        currency = true;
        s = r.trim_start_matches([' ', '\u{a0}']);
        if sign.is_none() {
            let (sg, r) = strip_sign(s);
            sign = sg;
            s = r;
        }
    } else if let Some(r) = s
        .strip_suffix(locale.currency_symbol)
        .or_else(|| s.strip_suffix(alt))
        .filter(|_| locale.currency_after)
    {
        // `12,50 €` (symbol after the amount, with or without a space).
        currency = true;
        s = r.trim_end_matches([' ', '\u{a0}', '\u{202f}']);
    }
    let mut percent = false;
    if let Some(r) = s.strip_suffix('%') {
        percent = true;
        s = r.trim_end_matches(['\u{a0}', '\u{202f}']);
    }
    if paren && sign.is_some() || currency && percent {
        return None;
    }
    // Map the locale's separators onto `.` and `,`; the other ASCII
    // punctuation becomes a byte that never parses.
    let mapped: String = s
        .chars()
        .map(|c| {
            if c == locale.decimal_sep {
                '.'
            } else if locale.is_group_char(c) {
                ','
            } else if c == '.' || c == ',' {
                '\u{1}'
            } else {
                c
            }
        })
        .collect();
    if !mapped.is_ascii() {
        return None;
    }
    let info = parse_unsigned(mapped.as_bytes(), b'.', Some(b','))?;
    if info.exponent && (currency || percent) {
        return None;
    }
    let mut v = info.value;
    if sign == Some(true) || paren {
        v = -v;
    }
    if percent {
        v = round_to_15_digits(v / 100.0);
    }
    let fmt = if currency {
        Some(builtin_format(QuickFormat::Currency, locale))
    } else if percent {
        Some(if info.frac_digits > 0 { "0.00%" } else { "0%" }.to_string())
    } else if info.exponent {
        Some(builtin_format(QuickFormat::Scientific, locale))
    } else if info.grouped {
        Some(
            if info.frac_digits > 0 {
                "#,##0.00"
            } else {
                "#,##0"
            }
            .to_string(),
        )
    } else {
        None
    };
    Some((v, fmt))
}

fn all_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// `[±]int num/den` with at least one space.
fn try_fraction(t: &str) -> Option<(f64, String)> {
    let mut parts = t.split_whitespace();
    let (a, b) = (parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None;
    }
    let (sign, a) = strip_sign(a);
    let (num, den) = b.split_once('/')?;
    if !all_digits(a) || !all_digits(num) || !all_digits(den) || den.len() > 9 {
        return None;
    }
    let (ip, n, d): (f64, f64, f64) = (a.parse().ok()?, num.parse().ok()?, den.parse().ok()?);
    if d == 0.0 {
        return None;
    }
    let mut v = ip + n / d;
    if sign == Some(true) {
        v = -v;
    }
    let q = "?".repeat(den.len().min(3));
    Some((round_to_15_digits(v), format!("# {q}/{q}")))
}

/// A parsed time of day (or duration).
struct TimeParse {
    /// Fraction of a day (may exceed 1 for durations).
    value: f64,
    format: String,
    duration: bool,
}

fn parse_time(s: &str, locale: &Locale) -> Option<TimeParse> {
    let s = s.trim();
    let lower = s.to_lowercase();
    let am = locale.am_pm.0.to_lowercase();
    let pm = locale.am_pm.1.to_lowercase();
    let mut ampm: Option<bool> = None; // Some(true) = PM
    let mut body = s;
    let candidates = [
        (am.clone(), false),
        (pm.clone(), true),
        ("am".to_string(), false),
        ("pm".to_string(), true),
        ("a".to_string(), false),
        ("p".to_string(), true),
    ];
    // Lowercasing may change byte lengths outside ASCII; only strip when
    // the lengths agree.
    if lower.len() == s.len() {
        for (suf, is_pm) in candidates.iter() {
            if lower.ends_with(suf.as_str()) {
                ampm = Some(*is_pm);
                body = s[..s.len() - suf.len()].trim_end();
                break;
            }
        }
        if ampm.is_none() {
            // Markers before the time (`午前 9:30`).
            for (pre, is_pm) in candidates.iter().take(2) {
                if !pre.is_empty() && lower.starts_with(pre.as_str()) {
                    ampm = Some(*is_pm);
                    body = s[pre.len()..].trim_start();
                    break;
                }
            }
        }
    }
    let parts: Vec<&str> = body.split(':').collect();
    if parts.len() < 2 || parts.len() > 3 {
        return None;
    }
    if !all_digits(parts[0]) || parts[0].len() > 6 {
        return None;
    }
    if !all_digits(parts[1]) || parts[1].len() > 2 {
        return None;
    }
    let h: u32 = parts[0].parse().ok()?;
    let m: u32 = parts[1].parse().ok()?;
    if m >= 60 {
        return None;
    }
    let mut sec = 0.0;
    let mut has_sec = false;
    let mut has_frac = false;
    if let Some(sp) = parts.get(2) {
        has_sec = true;
        let (whole, frac) = match sp.split_once(['.', locale.decimal_sep]) {
            Some((w, f)) => (w, Some(f)),
            None => (*sp, None),
        };
        if !all_digits(whole) || whole.len() > 2 {
            return None;
        }
        let w: u32 = whole.parse().ok()?;
        if w >= 60 {
            return None;
        }
        sec = f64::from(w);
        if let Some(f) = frac {
            if !all_digits(f) {
                return None;
            }
            has_frac = true;
            sec += format!("0.{f}").parse::<f64>().ok()?;
        }
    }
    let hour = match ampm {
        Some(is_pm) => {
            if h > 12 {
                return None;
            }
            h % 12 + if is_pm { 12 } else { 0 }
        }
        None => h,
    };
    let duration = ampm.is_none() && hour >= 24;
    let value = (f64::from(hour) * 3600.0 + f64::from(m) * 60.0 + sec) / 86_400.0;
    let mut format = String::from(if duration { "[HH]:MM" } else { "HH:MM" });
    if has_sec {
        format.push_str(":SS");
    }
    if has_frac {
        format.push_str(".00");
    }
    if ampm.is_some() {
        format.push_str(" AM/PM");
    }
    Some(TimeParse {
        value,
        format,
        duration,
    })
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Tk<'a> {
    Num(&'a str),
    Word(&'a str),
    Sep(char),
}

fn date_tokens(s: &str) -> Option<Vec<Tk<'_>>> {
    let mut out: Vec<Tk<'_>> = Vec::new();
    let chars: Vec<(usize, char)> = s.char_indices().collect();
    let end_of = |k: usize| chars.get(k).map_or(s.len(), |(i, _)| *i);
    let mut k = 0;
    while k < chars.len() {
        let (i, c) = chars[k];
        if c.is_ascii_digit() {
            let st = i;
            while k < chars.len() && chars[k].1.is_ascii_digit() {
                k += 1;
            }
            out.push(Tk::Num(&s[st..end_of(k)]));
        } else if c.is_alphabetic() {
            let st = i;
            while k < chars.len() && chars[k].1.is_alphabetic() {
                k += 1;
            }
            out.push(Tk::Word(&s[st..end_of(k)]));
            // The dot of an abbreviation (`févr.`, `Okt.`) is dropped.
            if k < chars.len() && chars[k].1 == '.' {
                k += 1;
            }
        } else if matches!(c, ' ' | '\t' | '\u{a0}') {
            while k < chars.len() && matches!(chars[k].1, ' ' | '\t' | '\u{a0}') {
                k += 1;
            }
            match out.last() {
                Some(Tk::Sep(_)) => {}
                _ => out.push(Tk::Sep(' ')),
            }
        } else if matches!(c, '/' | '-' | ',' | '.') {
            if let Some(Tk::Sep(' ')) = out.last() {
                out.pop();
            }
            if let Some(Tk::Sep(_)) = out.last() {
                return None;
            }
            out.push(Tk::Sep(c));
            k += 1;
            // ", " and "- " collapse into the punctuation.
            while k < chars.len() && chars[k].1 == ' ' {
                k += 1;
            }
        } else {
            return None;
        }
    }
    // A trailing separator (`19.10.`) is dropped.
    if let Some(Tk::Sep(' ' | '.')) = out.last() {
        out.pop();
    }
    Some(out)
}

fn year_from(s: &str, locale: &Locale) -> Option<(i32, &'static str)> {
    if !all_digits(s) || s.len() > 4 {
        return None;
    }
    let y: i32 = s.parse().ok()?;
    if s.len() <= 2 {
        Some((locale.expand_two_digit_year(y), "YY"))
    } else {
        Some((y, "YYYY"))
    }
}

fn small(s: &str) -> Option<u32> {
    if !all_digits(s) || s.len() > 2 {
        return None;
    }
    s.parse().ok()
}

/// Parses a date. Returns `(y, m, d, format)`.
fn parse_date(s: &str, locale: &Locale, ctx: &RecognizeContext) -> Option<(i32, u32, u32, String)> {
    let toks = date_tokens(s)?;
    // Separators of numeric dates: `/` and `-`, plus the locale's own
    // (`.` in de-DE).
    let num_sep = |c: &char| matches!(c, '/' | '-') || (*c == locale.date_sep && *c != ',');
    let cur_year = ctx.today.0;
    let res = match toks.as_slice() {
        // 2026-10-19 (ISO order whenever the first number has 4 digits)
        [Tk::Num(y), Tk::Sep(s1), Tk::Num(m), Tk::Sep(s2), Tk::Num(d)]
            if y.len() == 4 && s1 == s2 && matches!(s1, '-' | '/') =>
        {
            (
                y.parse().ok()?,
                small(m)?,
                small(d)?,
                format!("YYYY{s1}MM{s1}DD"),
            )
        }
        // Numeric dates in locale order.
        [Tk::Num(a), Tk::Sep(s1), Tk::Num(b), Tk::Sep(s2), Tk::Num(c)]
            if s1 == s2 && num_sep(s1) =>
        {
            let (y, m, d, yf) = match locale.date_order {
                DateOrder::MDY => {
                    let (y, yf) = year_from(c, locale)?;
                    (y, small(a)?, small(b)?, yf)
                }
                DateOrder::DMY => {
                    let (y, yf) = year_from(c, locale)?;
                    (y, small(b)?, small(a)?, yf)
                }
                DateOrder::YMD => {
                    let (y, yf) = year_from(a, locale)?;
                    (y, small(b)?, small(c)?, yf)
                }
            };
            let f = match locale.date_order {
                DateOrder::MDY => format!("MM{s1}DD{s1}{yf}"),
                DateOrder::DMY => format!("DD{s1}MM{s1}{yf}"),
                DateOrder::YMD => format!("{yf}{s1}MM{s1}DD"),
            };
            (y, m, d, f)
        }
        // 10/19 (current year)
        [Tk::Num(a), Tk::Sep(s1), Tk::Num(b)] if num_sep(s1) => {
            let (m, d) = match locale.date_order {
                DateOrder::DMY => (small(b)?, small(a)?),
                _ => (small(a)?, small(b)?),
            };
            let f = match locale.date_order {
                DateOrder::DMY => format!("DD{s1}MM{s1}YYYY"),
                _ => format!("MM{s1}DD{s1}YYYY"),
            };
            (cur_year, m, d, f)
        }
        // 19 Oct 2026, 19-Oct-26, 19 October, 19. Oktober 2026
        [Tk::Num(d), Tk::Sep(s0), Tk::Word(mn), rest @ ..] if matches!(s0, ' ' | '-' | '.') => {
            let s1 = if *s0 == '.' { &' ' } else { s0 };
            let (m, long) = locale.month_from_name(mn)?;
            let mf = if long { "MMMM" } else { "MMM" };
            let (y, yf) = match rest {
                [] => (cur_year, "YYYY"),
                [Tk::Sep(s2), Tk::Num(y)] if s2 == s1 => year_from(y, locale)?,
                // `19 févr. 2026`: the abbreviation dot ate the separator.
                [Tk::Num(y)] => year_from(y, locale)?,
                _ => return None,
            };
            (y, m, small(d)?, format!("DD{s1}{mf}{s1}{yf}"))
        }
        // Oct 2026
        [Tk::Word(mn), Tk::Sep(' '), Tk::Num(y)] if y.len() == 4 => {
            let (m, long) = locale.month_from_name(mn)?;
            let mf = if long { "MMMM" } else { "MMM" };
            (y.parse().ok()?, m, 1, format!("{mf} YYYY"))
        }
        // Oct 19, 2026 / October 19 2026 / Oct 19
        [Tk::Word(mn), Tk::Sep(' '), Tk::Num(d), rest @ ..] => {
            let (m, long) = locale.month_from_name(mn)?;
            let mf = if long { "MMMM" } else { "MMM" };
            let (y, yf, sep) = match rest {
                [] => (cur_year, "YYYY", ","),
                [Tk::Sep(s2), Tk::Num(y)] if matches!(s2, ',' | ' ') => {
                    let (y, yf) = year_from(y, locale)?;
                    (y, yf, if *s2 == ',' { "," } else { "" })
                }
                _ => return None,
            };
            (y, m, small(d)?, format!("{mf} DD{sep} {yf}"))
        }
        _ => return None,
    };
    let (y, m, d, f) = res;
    // Validate (serial_from_ymd checks month/day/year ranges).
    serial_from_ymd(ctx.epoch, y, m, d)?;
    Some((y, m, d, f))
}

/// Splits `date time` at the whitespace (or ISO `T`) before the time.
fn split_date_time(t: &str) -> (Option<&str>, Option<&str>) {
    let Some(colon) = t.find(':') else {
        return (Some(t), None);
    };
    let before = &t[..colon];
    let split = before.rfind([' ', '\t']).or_else(|| {
        // ISO 8601 `2026-10-19T14:30`
        let p = before.rfind('T')?;
        (p > 0 && before.as_bytes()[p - 1].is_ascii_digit()).then_some(p)
    });
    match split {
        Some(p) => {
            let d = t[..p].trim();
            let tm = t[p + 1..].trim();
            (if d.is_empty() { None } else { Some(d) }, Some(tm))
        }
        None => (None, Some(t)),
    }
}

fn try_datetime(t: &str, locale: &Locale, ctx: &RecognizeContext) -> Option<(f64, String)> {
    // A negative time is a negative duration (`-6:00`, `-36:00:00`), shown
    // with elapsed hours. Dates and AM/PM times cannot be negative.
    if let Some(body) = t.strip_prefix('-') {
        if !body.contains(':') || body.contains([' ', '\t', '-', '+']) {
            return None;
        }
        let tp = parse_time(body, locale)?;
        if tp.format.ends_with("AM/PM") {
            return None;
        }
        let format = if tp.duration {
            tp.format
        } else {
            tp.format.replacen("HH", "[HH]", 1)
        };
        return Some((-tp.value, format));
    }
    let (date, time) = split_date_time(t);
    let date = match date {
        Some(ds) => match parse_date(ds, locale, ctx) {
            Some(d) => Some(d),
            // A marker before the time (`午前 9:30`).
            None => {
                let tp = parse_time(t, locale)?;
                return Some((tp.value, tp.format));
            }
        },
        None => None,
    };
    let time = match time {
        Some(ts) => Some(parse_time(ts, locale)?),
        None => None,
    };
    match (date, time) {
        (Some((y, m, d, df)), None) => Some((serial_from_ymd(ctx.epoch, y, m, d)?, df)),
        (None, Some(tp)) => Some((tp.value, tp.format)),
        (Some((y, m, d, df)), Some(tp)) => {
            if tp.duration {
                return None;
            }
            let serial = serial_from_ymd(ctx.epoch, y, m, d)? + tp.value;
            Some((serial, format!("{df} {}", tp.format)))
        }
        (None, None) => None,
    }
}

/// [`parse_number_unambiguous_with`] using the 1899-12-30 epoch.
pub fn parse_number_unambiguous(s: &str) -> Option<f64> {
    parse_number_unambiguous_with(s, DateEpoch::D1899)
}

/// Text-to-number conversion for `TextToNumber::ConvertUnambiguous`.
///
/// Accepts surrounding whitespace and: plain decimal numbers with optional
/// sign and exponent (`-1.5`, `.5`, `1e3`); ISO 8601 dates `YYYY-MM-DD`;
/// times `HH:MM[:SS[.fff]]` (hours may exceed 23); and ISO date-times
/// joined by a space or `T`. No thousands separators, currency, percent,
/// AM/PM or locale dates. Returns `None` for anything else (callers raise
/// `#VALUE!`), including the empty string.
pub fn parse_number_unambiguous_with(s: &str, epoch: DateEpoch) -> Option<f64> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    let (sign, body) = strip_sign(t);
    if let Some(info) = parse_unsigned(body.as_bytes(), b'.', None) {
        return Some(if sign == Some(true) {
            -info.value
        } else {
            info.value
        });
    }
    let (date, time) = split_date_time(t);
    let mut v = 0.0;
    if let Some(ds) = date {
        let parts: Vec<&str> = ds.split('-').collect();
        let [y, m, d] = parts.as_slice() else {
            return None;
        };
        if y.len() != 4 || !all_digits(y) {
            return None;
        }
        v += serial_from_ymd(epoch, y.parse().ok()?, small(m)?, small(d)?)?;
    }
    if let Some(ts) = time {
        if ts.bytes().any(|b| b.is_ascii_alphabetic()) {
            return None;
        }
        let tp = parse_time(ts, &crate::locale::EN_US)?;
        if date.is_some() && tp.duration {
            return None;
        }
        v += tp.value;
    }
    Some(v)
}
