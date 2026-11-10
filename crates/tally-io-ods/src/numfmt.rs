//! ODF data styles (`number:*-style`) ↔ OOo number format codes.
//!
//! Reading builds a [`DataStyle`] from the XML (in `styles.rs`) and turns it
//! into a format code with [`DataStyle::to_code`]. Writing tokenizes a format
//! code with [`code_to_xml_in`] and emits the equivalent data style elements; a
//! code we cannot express faithfully yields `Err(reason)` so the writer can
//! fall back to General and report a warning.

use std::collections::HashMap;
use std::fmt::Write as _;

use crate::xml::{escape_attr, escape_text};

/// The family of a data style element.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Number,
    Percentage,
    Currency,
    Date,
    Time,
    Boolean,
    Text,
}

impl Kind {
    pub fn from_element(name: &str) -> Option<Self> {
        Some(match name {
            "number:number-style" => Kind::Number,
            "number:percentage-style" => Kind::Percentage,
            "number:currency-style" => Kind::Currency,
            "number:date-style" => Kind::Date,
            "number:time-style" => Kind::Time,
            "number:boolean-style" => Kind::Boolean,
            "number:text-style" => Kind::Text,
            _ => return None,
        })
    }

    fn element(self) -> &'static str {
        match self {
            Kind::Number => "number:number-style",
            Kind::Percentage => "number:percentage-style",
            Kind::Currency => "number:currency-style",
            Kind::Date => "number:date-style",
            Kind::Time => "number:time-style",
            Kind::Boolean => "number:boolean-style",
            Kind::Text => "number:text-style",
        }
    }

    fn is_datetime(self) -> bool {
        matches!(self, Kind::Date | Kind::Time)
    }
}

/// One child element of a data style.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Part {
    /// `number:number`; `decimals: None` means "as many as needed" (General).
    Number {
        decimals: Option<u32>,
        min_decimals: Option<u32>,
        min_int: u32,
        grouping: bool,
        factor_thousands: u32,
    },
    Scientific {
        decimals: u32,
        min_int: u32,
        min_exp: u32,
        grouping: bool,
    },
    Fraction {
        min_int: Option<u32>,
        min_num: u32,
        min_den: u32,
        den_value: Option<u32>,
    },
    Text(String),
    /// `number:currency-symbol`: the symbol and, from `number:language` /
    /// `number:country`, the hex locale id of `[$€-407]`.
    Currency {
        symbol: String,
        lcid: Option<String>,
    },
    TextContent,
    Boolean,
    Fill(char),
    Year {
        long: bool,
    },
    Month {
        long: bool,
        textual: bool,
    },
    Day {
        long: bool,
    },
    DayOfWeek {
        long: bool,
    },
    Hours {
        long: bool,
    },
    Minutes {
        long: bool,
    },
    Seconds {
        long: bool,
        decimals: u32,
    },
    AmPm,
    Quarter {
        long: bool,
    },
    Week,
    Era {
        long: bool,
    },
    /// `[~calendar]` (reading: `number:calendar` changes; writing: the
    /// calendar of the following date elements).
    Calendar(String),
    /// Writing only: `AAA` / `AAAA`, a weekday in the language's other
    /// calendar (AOO writes the calendar on that element only).
    AaaDay {
        long: bool,
    },
    /// Writing only: `R` / `RR` until the section is known to be a date
    /// (otherwise the letters are text, as `R 0.00`).
    EraR {
        double: bool,
    },
}

/// A parsed data style.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DataStyle {
    pub kind: Kind,
    pub parts: Vec<Part>,
    pub color: Option<u32>,
    /// `(condition, apply-style-name)` from `style:map`.
    pub maps: Vec<(String, String)>,
    /// `number:truncate-on-overflow` (false means elapsed `[HH]`).
    pub truncate: bool,
    /// The style's `number:language` / `number:country` as a hex locale id:
    /// the format's language, `[$-407]` in the code.
    pub language: Option<String>,
    /// `number:transliteration-*`: the NatNum mode and its language (hex).
    pub natnum: Option<(u8, Option<String>)>,
}

impl DataStyle {
    pub fn new(kind: Kind) -> Self {
        Self {
            kind,
            parts: Vec::new(),
            color: None,
            maps: Vec::new(),
            truncate: true,
            language: None,
            natnum: None,
        }
    }

    /// The OOo format code; `lookup` resolves `style:map` targets.
    ///
    /// The format's language is the main style's `number:language` (AOO
    /// reads it there, xmlnumfi.cxx), written once as a leading `[$-407]`;
    /// the mapped styles' languages are ignored.
    ///
    /// A transliteration (`[NatNumN]`) in another language than the style's
    /// (AOO `[NatNum1][$-41E]`) makes that language the code's: Tally has
    /// one language per format.
    pub fn to_code<'a>(&'a self, lookup: impl Fn(&str) -> Option<&'a DataStyle>) -> String {
        let lang = self
            .natnum
            .as_ref()
            .and_then(|(_, l)| l.as_ref())
            .or(self.language.as_ref())
            .map_or_else(String::new, |l| format!("[$-{l}]"));
        // The first section's [NatNumN] goes before the language tag
        // (`[NatNum1][$-411]0`, as AOO and Excel write it).
        let natnum_tag = |ds: &DataStyle| {
            ds.natnum
                .as_ref()
                .map_or_else(String::new, |(n, _)| format!("[NatNum{n}]"))
        };
        let code_lang = self
            .natnum
            .as_ref()
            .and_then(|(_, l)| l.clone())
            .or_else(|| self.language.clone());
        if self.maps.is_empty() {
            return natnum_tag(self) + &lang + &self.section_code(true, code_lang.as_deref());
        }
        let mut first: Option<&DataStyle> = None;
        let mapped: Vec<(String, String)> = self
            .maps
            .iter()
            .filter_map(|(cond, name)| {
                let c = parse_condition(cond)?;
                let ds = lookup(name)?;
                let is_first = first.is_none();
                first.get_or_insert(ds);
                Some((c, ds.section_code(is_first, code_lang.as_deref())))
            })
            .take(3)
            .collect();
        let first = first.unwrap_or(self);
        let main = self.section_code(std::ptr::eq(first, self), code_lang.as_deref());
        let conds: Vec<&str> = mapped.iter().map(|(c, _)| c.as_str()).collect();
        let mut out = String::new();
        if conds == [">=0"] || conds == [">0", "<0"] {
            for (_, code) in &mapped {
                out.push_str(code);
                out.push(';');
            }
        } else {
            for (cond, code) in &mapped {
                let _ = write!(out, "[{cond}]{code};");
            }
        }
        out.push_str(&main);
        natnum_tag(first) + &lang + &out
    }

    /// One section's code; `first` leaves out the `[NatNumN]` that
    /// [`DataStyle::to_code`] puts in front, `lang` is the code's language.
    fn section_code(&self, first: bool, lang: Option<&str>) -> String {
        let mut out = String::new();
        if let Some(name) = self.color.and_then(color_name) {
            let _ = write!(out, "[{name}]");
        }
        if let (false, Some((n, _))) = (first, &self.natnum) {
            let _ = write!(out, "[NatNum{n}]");
        }
        // AOO writes E / EE / R / RR with the language's other calendar on
        // every date element: that calendar is implied again by the era
        // year (Tally's own `GGGE` round-trips without `[~gengou]`).
        let other = lang
            .and_then(tally_numfmt::lang_id)
            .and_then(tally_numfmt::calendar::other_calendar);
        let has_era_year = self
            .parts
            .iter()
            .skip_while(|p| !matches!(p, Part::Era { .. }))
            .any(|p| matches!(p, Part::Year { .. }));
        let implicit = |c: &str| {
            has_era_year
                && other.is_some_and(|o| o.name().eq_ignore_ascii_case(c))
                && self.parts.iter().all(|p| match p {
                    Part::Calendar(x) => x.eq_ignore_ascii_case(c),
                    _ => true,
                })
        };
        let mut first_hours = !self.truncate;
        // xmlnumfi.cxx: a year after an era is the era's year (E / EE).
        let mut seen_era = false;
        for part in &self.parts {
            match part {
                Part::Number {
                    decimals,
                    min_decimals,
                    min_int,
                    grouping,
                    factor_thousands,
                } => match decimals {
                    None => out.push_str("General"),
                    Some(d) => {
                        push_int_pattern(&mut out, *min_int, *grouping);
                        push_decimals(&mut out, *d, min_decimals.unwrap_or(*d));
                        for _ in 0..(*factor_thousands).min(4) {
                            out.push(',');
                        }
                    }
                },
                Part::Scientific {
                    decimals,
                    min_int,
                    min_exp,
                    grouping,
                } => {
                    push_int_pattern(&mut out, (*min_int).max(1), *grouping);
                    push_decimals(&mut out, *decimals, *decimals);
                    out.push_str("E+");
                    for _ in 0..(*min_exp).clamp(1, 5) {
                        out.push('0');
                    }
                }
                Part::Fraction {
                    min_int,
                    min_num,
                    min_den,
                    den_value,
                } => {
                    if let Some(mi) = min_int {
                        push_int_pattern(&mut out, *mi, false);
                        out.push(' ');
                    }
                    for _ in 0..(*min_num).clamp(1, 9) {
                        out.push('?');
                    }
                    out.push('/');
                    match den_value {
                        Some(v) => {
                            let _ = write!(out, "{v}");
                        }
                        None => {
                            for _ in 0..(*min_den).clamp(1, 9) {
                                out.push('?');
                            }
                        }
                    }
                }
                Part::Text(t) => push_literal(&mut out, t, self.kind),
                Part::Currency { symbol, lcid } => match lcid {
                    Some(l) => {
                        let _ = write!(out, "[${symbol}-{l}]");
                    }
                    None if symbol == "$" => out.push('$'),
                    None => {
                        let _ = write!(out, "[${symbol}]");
                    }
                },
                Part::TextContent => out.push('@'),
                Part::Boolean => out.push_str("BOOLEAN"),
                Part::Fill(c) => {
                    out.push('*');
                    out.push(*c);
                }
                Part::Year { long } if seen_era => out.push_str(if *long { "EE" } else { "E" }),
                Part::Year { long } => out.push_str(if *long { "YYYY" } else { "YY" }),
                Part::Month { long, textual } => out.push_str(match (textual, long) {
                    (true, true) => "MMMM",
                    (true, false) => "MMM",
                    (false, true) => "MM",
                    (false, false) => "M",
                }),
                Part::Day { long } => out.push_str(if *long { "DD" } else { "D" }),
                Part::DayOfWeek { long } => out.push_str(if *long { "NNN" } else { "NN" }),
                Part::Hours { long } => {
                    let h = if *long { "HH" } else { "H" };
                    if first_hours {
                        let _ = write!(out, "[{h}]");
                        first_hours = false;
                    } else {
                        out.push_str(h);
                    }
                }
                Part::Minutes { long } => out.push_str(if *long { "MM" } else { "M" }),
                Part::Seconds { long, decimals } => {
                    out.push_str(if *long { "SS" } else { "S" });
                    if *decimals > 0 {
                        out.push('.');
                        for _ in 0..(*decimals).min(9) {
                            out.push('0');
                        }
                    }
                }
                Part::AmPm => out.push_str("AM/PM"),
                Part::Quarter { long } => out.push_str(if *long { "QQ" } else { "Q" }),
                Part::Week => out.push_str("WW"),
                Part::Era { long } => {
                    seen_era = true;
                    out.push_str(if *long { "GGG" } else { "G" });
                }
                Part::Calendar(c) if implicit(c) => {}
                Part::Calendar(c) => {
                    let _ = write!(out, "[~{c}]");
                }
                Part::AaaDay { long } => out.push_str(if *long { "AAAA" } else { "AAA" }),
                Part::EraR { double } => out.push_str(if *double { "RR" } else { "R" }),
            }
        }
        out
    }
}

fn push_int_pattern(out: &mut String, min_int: u32, grouping: bool) {
    let min_int = min_int.min(30) as usize;
    if !grouping {
        if min_int == 0 {
            out.push('#');
        } else {
            out.extend(std::iter::repeat_n('0', min_int));
        }
        return;
    }
    let width = min_int.max(4);
    let digits: Vec<char> = (0..width)
        .map(|i| if i < width - min_int { '#' } else { '0' })
        .collect();
    for (i, c) in digits.iter().enumerate() {
        if i > 0 && (width - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(*c);
    }
}

fn push_decimals(out: &mut String, decimals: u32, min_decimals: u32) {
    let decimals = decimals.min(30);
    if decimals == 0 {
        return;
    }
    out.push('.');
    let zeros = min_decimals.min(decimals);
    for i in 0..decimals {
        out.push(if i < zeros { '0' } else { '#' });
    }
}

/// Characters that are literal without quoting in every section kind.
const SAFE_ALWAYS: &str = " -+()/:!^&~{}<>='$";

fn push_literal(out: &mut String, text: &str, kind: Kind) {
    let safe = |c: char| {
        if kind.is_datetime() {
            SAFE_ALWAYS.contains(c) || c == '.' || c == ','
        } else if c == '/' {
            false
        } else {
            SAFE_ALWAYS.contains(c) || (c == '%' && kind == Kind::Percentage)
        }
    };
    if text.chars().all(safe) {
        out.push_str(text);
        return;
    }
    out.push('"');
    for c in text.chars() {
        if c == '"' {
            out.push_str("\"\\\"\"");
        } else {
            out.push(c);
        }
    }
    out.push('"');
}

const COLORS: [(&str, u32); 8] = [
    ("BLACK", 0x0000_00ff),
    ("BLUE", 0x0000_ffff),
    ("GREEN", 0x00ff_00ff),
    ("CYAN", 0x00ff_ffff),
    ("RED", 0xff00_00ff),
    ("MAGENTA", 0xff00_ffff),
    ("YELLOW", 0xffff_00ff),
    ("WHITE", 0xffff_ffff),
];

fn color_name(c: u32) -> Option<&'static str> {
    COLORS.iter().find(|(_, v)| *v == c).map(|(n, _)| *n)
}

fn color_value(name: &str) -> Option<u32> {
    COLORS
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, v)| *v)
}

/// `value()>=0` → `>=0`.
fn parse_condition(cond: &str) -> Option<String> {
    let rest = cond.trim().strip_prefix("value()")?.trim();
    let op_len = rest
        .find(|c: char| !matches!(c, '<' | '>' | '=' | '!'))
        .unwrap_or(rest.len());
    let (op, num) = rest.split_at(op_len);
    let op = match op {
        "<" | ">" | "<=" | ">=" | "=" | "<>" => op,
        "!=" => "<>",
        "==" => "=",
        _ => return None,
    };
    let num = num.trim();
    num.parse::<f64>().ok()?;
    Some(format!("{op}{num}"))
}

// ---------------------------------------------------------------------------
// Writing: format code → data style XML
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct Section {
    color: Option<u32>,
    condition: Option<String>,
    elapsed: bool,
    parts: Vec<Part>,
    percent: bool,
    /// `[$-407]`: the format's language as a known hex locale id.
    language: Option<String>,
    /// `[NatNumN]` / `[DBNumN]`: the number and whether it is a DBNum.
    natnum: Option<(u8, bool)>,
    /// `E`, `EE`, `R`, `RR`: date elements use the language's other calendar.
    other_calendar: bool,
}

impl Section {
    fn kind(&self) -> Kind {
        let date = self.parts.iter().any(|p| {
            matches!(
                p,
                Part::Year { .. }
                    | Part::Month { .. }
                    | Part::Day { .. }
                    | Part::DayOfWeek { .. }
                    | Part::Quarter { .. }
                    | Part::Week
                    | Part::Era { .. }
                    | Part::AaaDay { .. }
                    | Part::EraR { .. }
            )
        });
        let time = self.parts.iter().any(|p| {
            matches!(
                p,
                Part::Hours { .. } | Part::Minutes { .. } | Part::Seconds { .. } | Part::AmPm
            )
        });
        if date {
            Kind::Date
        } else if time {
            Kind::Time
        } else if self.parts.iter().any(|p| matches!(p, Part::Boolean)) {
            Kind::Boolean
        } else if self
            .parts
            .iter()
            .any(|p| matches!(p, Part::Currency { .. }))
        {
            Kind::Currency
        } else if self.percent {
            Kind::Percentage
        } else if self.parts.iter().any(|p| matches!(p, Part::TextContent)) {
            Kind::Text
        } else {
            Kind::Number
        }
    }
}

/// Splits a code at `;` outside quotes, brackets and escapes.
fn split_sections(code: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut quoted, mut bracket, mut escape) = (false, false, false);
    let mut start = 0;
    for (i, c) in code.char_indices() {
        if escape {
            escape = false;
            continue;
        }
        match c {
            '\\' if !quoted => escape = true,
            '"' if !bracket => quoted = !quoted,
            '[' if !quoted => bracket = true,
            ']' if !quoted => bracket = false,
            ';' if !quoted && !bracket => {
                out.push(&code[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&code[start..]);
    out
}

fn push_text(parts: &mut Vec<Part>, s: &str) {
    if let Some(Part::Text(t)) = parts.last_mut() {
        t.push_str(s);
    } else {
        parts.push(Part::Text(s.to_string()));
    }
}

fn starts_with_ci(chars: &[char], i: usize, pat: &str) -> bool {
    let n = pat.chars().count();
    i + n <= chars.len()
        && chars[i..i + n]
            .iter()
            .zip(pat.chars())
            .all(|(a, b)| a.eq_ignore_ascii_case(&b))
}

fn tokenize_section(src: &str) -> Result<Section, String> {
    let chars: Vec<char> = src.chars().collect();
    let mut s = Section::default();
    let mut i = 0;
    let mut has_number = false;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '"' => {
                let mut lit = String::new();
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    lit.push(chars[i]);
                    i += 1;
                }
                if i >= chars.len() {
                    return Err("unterminated quoted literal".into());
                }
                i += 1;
                if !lit.is_empty() {
                    push_text(&mut s.parts, &lit);
                }
            }
            '\\' => {
                let Some(&n) = chars.get(i + 1) else {
                    return Err("dangling escape".into());
                };
                push_text(&mut s.parts, &n.to_string());
                i += 2;
            }
            '_' => {
                push_text(&mut s.parts, " ");
                i += 2;
            }
            '*' => {
                let Some(&n) = chars.get(i + 1) else {
                    return Err("dangling fill character".into());
                };
                s.parts.push(Part::Fill(n));
                i += 2;
            }
            '[' => {
                let end = chars[i..]
                    .iter()
                    .position(|&x| x == ']')
                    .ok_or("unterminated bracket")?;
                let inner: String = chars[i + 1..i + end].iter().collect();
                i += end + 1;
                let up = inner.to_ascii_uppercase();
                if let Some(cur) = inner.strip_prefix('$') {
                    let (sym, hex) = match cur.split_once('-') {
                        Some((sym, hex)) => (sym, Some(hex)),
                        None => (cur, None),
                    };
                    // Only locale ids with an ISO language / country can be
                    // written; others are dropped (the symbol is kept).
                    // Excel's calendar / numeral bytes and BCP 47 tags are
                    // reduced to the language.
                    let lcid = hex
                        .filter(|h| tally_numfmt::lcid_language(h).is_some())
                        .and_then(tally_numfmt::lang_id)
                        .map(|l| l.hex());
                    if sym.is_empty() {
                        if lcid.is_some() {
                            s.language = lcid;
                        }
                    } else {
                        s.parts.push(Part::Currency {
                            symbol: sym.to_string(),
                            lcid,
                        });
                    }
                } else if inner.starts_with(['<', '>', '=']) {
                    let cond = parse_condition(&format!("value(){inner}"))
                        .ok_or_else(|| format!("bad condition [{inner}]"))?;
                    s.condition = Some(cond);
                } else if let Some(col) = color_value(&inner) {
                    s.color = Some(col);
                } else if let Some(n) = up.strip_prefix("NATNUM").and_then(|n| n.parse::<u8>().ok())
                {
                    s.natnum.get_or_insert((n, false));
                } else if let Some(n) = up.strip_prefix("DBNUM").and_then(|n| n.parse::<u8>().ok())
                {
                    s.natnum.get_or_insert((n, true));
                } else if let Some(c) = inner.strip_prefix('~') {
                    s.parts.push(Part::Calendar(c.to_string()));
                } else {
                    match up.as_str() {
                        "H" | "HH" => {
                            s.elapsed = true;
                            s.parts.push(Part::Hours {
                                long: up.len() == 2,
                            });
                        }
                        _ => return Err(format!("unsupported bracket [{inner}]")),
                    }
                }
            }
            '0' | '#' | '?' => {
                if has_number {
                    return Err("more than one number in a section".into());
                }
                has_number = true;
                i = scan_number(&chars, i, &mut s.parts)?;
            }
            '.' if matches!(chars.get(i + 1), Some('0' | '#' | '?')) && !has_number => {
                has_number = true;
                i = scan_number(&chars, i, &mut s.parts)?;
            }
            '%' => {
                s.percent = true;
                push_text(&mut s.parts, "%");
                i += 1;
            }
            '@' => {
                s.parts.push(Part::TextContent);
                i += 1;
            }
            '$' => {
                s.parts.push(Part::Currency {
                    symbol: "$".into(),
                    lcid: None,
                });
                i += 1;
            }
            _ if c.is_ascii_alphabetic() => {
                i = scan_word(&chars, i, &mut s)?;
            }
            _ => {
                push_text(&mut s.parts, &c.to_string());
                i += 1;
            }
        }
    }
    // `R` / `RR` are era codes unless they are the only date code of a
    // number section (`R 0.00`, as the engine's parser decides).
    let other_dated = s.parts.iter().any(|p| {
        !matches!(
            p,
            Part::EraR { .. }
                | Part::Text(_)
                | Part::Number { .. }
                | Part::Scientific { .. }
                | Part::Fraction { .. }
                | Part::TextContent
                | Part::Currency { .. }
                | Part::Fill(_)
                | Part::Boolean
                | Part::Calendar(_)
        )
    });
    let numbers = s.parts.iter().any(|p| {
        matches!(
            p,
            Part::Number { .. }
                | Part::Scientific { .. }
                | Part::Fraction { .. }
                | Part::TextContent
        )
    });
    if !other_dated && numbers {
        for p in &mut s.parts {
            if let Part::EraR { double } = *p {
                *p = Part::Text(if double { "RR" } else { "R" }.into());
            }
        }
        merge_texts(&mut s.parts);
    } else if s.parts.iter().any(|p| matches!(p, Part::EraR { .. })) {
        s.other_calendar = true;
        let parts = std::mem::take(&mut s.parts);
        for p in parts {
            match p {
                Part::EraR { double: false } => s.parts.push(Part::Year { long: true }),
                Part::EraR { double: true } => {
                    s.parts.push(Part::Era { long: true });
                    s.parts.push(Part::Year { long: true });
                }
                p => s.parts.push(p),
            }
        }
    }
    // `$` in a date/time section is just text.
    if s.kind().is_datetime() {
        for p in &mut s.parts {
            if let Part::Currency { symbol, .. } = p {
                *p = Part::Text(symbol.clone());
            }
        }
        merge_texts(&mut s.parts);
    }
    resolve_minutes(&mut s.parts);
    Ok(s)
}

fn merge_texts(parts: &mut Vec<Part>) {
    let mut out: Vec<Part> = Vec::with_capacity(parts.len());
    for p in parts.drain(..) {
        match (out.last_mut(), p) {
            (Some(Part::Text(a)), Part::Text(b)) => a.push_str(&b),
            (_, p) => out.push(p),
        }
    }
    *parts = out;
}

/// `M`/`MM` after hours or before seconds means minutes.
fn resolve_minutes(parts: &mut [Part]) {
    let idx: Vec<usize> = parts
        .iter()
        .enumerate()
        .filter(|(_, p)| !matches!(p, Part::Text(_)))
        .map(|(i, _)| i)
        .collect();
    for (k, &i) in idx.iter().enumerate() {
        if let Part::Month {
            long,
            textual: false,
        } = parts[i]
        {
            let prev_hours = k > 0 && matches!(parts[idx[k - 1]], Part::Hours { .. });
            let next_secs = idx
                .get(k + 1)
                .is_some_and(|&j| matches!(parts[j], Part::Seconds { .. }));
            if prev_hours || next_secs {
                parts[i] = Part::Minutes { long };
            }
        }
    }
}

fn scan_word(chars: &[char], start: usize, s: &mut Section) -> Result<usize, String> {
    for (kw, part) in [
        ("AM/PM", Part::AmPm),
        ("A/P", Part::AmPm),
        ("BOOLEAN", Part::Boolean),
    ] {
        if starts_with_ci(chars, start, kw) {
            s.parts.push(part);
            return Ok(start + kw.len());
        }
    }
    for kw in ["GENERAL", "STANDARD"] {
        if starts_with_ci(chars, start, kw) {
            s.parts.push(Part::Number {
                decimals: None,
                min_decimals: None,
                min_int: 1,
                grouping: false,
                factor_thousands: 0,
            });
            return Ok(start + kw.len());
        }
    }
    let c = chars[start].to_ascii_uppercase();
    let mut n = 0;
    while start + n < chars.len() && chars[start + n].to_ascii_uppercase() == c {
        n += 1;
    }
    let mut end = start + n;
    let part = match c {
        'Y' => Part::Year { long: n > 2 },
        'M' => Part::Month {
            long: n == 2 || n >= 4,
            textual: n >= 3,
        },
        'D' if n >= 3 => Part::DayOfWeek { long: n >= 4 },
        'D' => Part::Day { long: n >= 2 },
        'N' => Part::DayOfWeek { long: n >= 3 },
        'H' => Part::Hours { long: n >= 2 },
        'S' => {
            let mut decimals = 0;
            if chars.get(end) == Some(&'.') && chars.get(end + 1) == Some(&'0') {
                end += 1;
                while chars.get(end) == Some(&'0') {
                    decimals += 1;
                    end += 1;
                }
            }
            Part::Seconds {
                long: n >= 2,
                decimals,
            }
        }
        'Q' => Part::Quarter { long: n >= 2 },
        'W' if n == 2 => Part::Week,
        'G' => Part::Era { long: n >= 3 },
        // xmlnumfe.cxx: E / EE are a year in the language's other calendar.
        'E' => {
            s.other_calendar = true;
            end = start + n.min(2);
            Part::Year { long: n >= 2 }
        }
        'R' => {
            end = start + n.min(2);
            Part::EraR { double: n >= 2 }
        }
        'A' if n >= 3 => {
            end = start + n.min(4);
            Part::AaaDay { long: n >= 4 }
        }
        _ => {
            let word: String = chars[start..end].iter().collect();
            push_text(&mut s.parts, &word);
            return Ok(end);
        }
    };
    s.parts.push(part);
    Ok(end)
}

/// Scans a digit-placeholder run (number, scientific or fraction).
fn scan_number(chars: &[char], start: usize, parts: &mut Vec<Part>) -> Result<usize, String> {
    let is_ph = |c: char| matches!(c, '0' | '#' | '?');
    let mut i = start;
    while i < chars.len() && (is_ph(chars[i]) || chars[i] == ',' || chars[i] == '.') {
        i += 1;
    }
    let run: String = chars[start..i].iter().collect();
    // Fraction: `#/#`, `# ?/?` or `# ??/16`.
    let frac_after = |j: usize| -> Option<(usize, u32, u32, Option<u32>)> {
        let mut k = j;
        let mut num = 0;
        while k < chars.len() && is_ph(chars[k]) {
            num += 1;
            k += 1;
        }
        if num == 0 || chars.get(k) != Some(&'/') {
            return None;
        }
        k += 1;
        let ds = k;
        while k < chars.len() && (is_ph(chars[k]) || chars[k].is_ascii_digit()) {
            k += 1;
        }
        let den: String = chars[ds..k].iter().collect();
        if den.is_empty() {
            return None;
        }
        let value = if den.chars().all(|c| c.is_ascii_digit()) {
            den.parse().ok()
        } else {
            None
        };
        Some((k, num, den.chars().count() as u32, value))
    };
    if chars.get(i) == Some(&'/') && !run.contains(['.', ',']) {
        if let Some((end, num, den, value)) = frac_after(start) {
            parts.push(Part::Fraction {
                min_int: None,
                min_num: num,
                min_den: den,
                den_value: value,
            });
            return Ok(end);
        }
    }
    if chars.get(i) == Some(&' ') && !run.contains(['.', ',']) {
        if let Some((end, num, den, value)) = frac_after(i + 1) {
            parts.push(Part::Fraction {
                min_int: Some(run.chars().filter(|&c| c == '0').count() as u32),
                min_num: num,
                min_den: den,
                den_value: value,
            });
            return Ok(end);
        }
    }
    let (int, frac) = match run.find('.') {
        Some(p) => (&run[..p], &run[p + 1..]),
        None => (run.as_str(), ""),
    };
    let min_int = int.chars().filter(|&c| c == '0').count() as u32;
    let trimmed = int.trim_end_matches(',');
    let factor =
        (int.len() - trimmed.len()) as u32 + (frac.len() - frac.trim_end_matches(',').len()) as u32;
    let grouping = trimmed.contains(',');
    let frac_ph: Vec<char> = frac.chars().filter(|&c| is_ph(c)).collect();
    let decimals = frac_ph.len() as u32;
    let min_dec = frac_ph.iter().take_while(|&&c| c == '0').count() as u32;
    // Scientific: `E+00` / `E-00`.
    if matches!(chars.get(i), Some('E' | 'e')) && matches!(chars.get(i + 1), Some('+' | '-')) {
        if chars[i + 1] == '-' {
            return Err("scientific format with E- is not representable in ODF 1.2".into());
        }
        let mut k = i + 2;
        let mut exp = 0;
        while k < chars.len() && is_ph(chars[k]) {
            exp += 1;
            k += 1;
        }
        if exp == 0 {
            return Err("scientific format without exponent digits".into());
        }
        if min_dec != decimals {
            return Err("optional decimals in scientific format".into());
        }
        parts.push(Part::Scientific {
            decimals,
            min_int,
            min_exp: exp,
            grouping,
        });
        return Ok(k);
    }
    parts.push(Part::Number {
        decimals: Some(decimals),
        min_decimals: Some(min_dec),
        min_int,
        grouping,
        factor_thousands: factor,
    });
    Ok(i)
}

/// Emits the data style(s) for `code` under style name `name` (mapped
/// sections become `{name}P0`, `{name}P1`, …). `Err` means the code cannot be
/// written faithfully.
///
/// `doc_lang` (hex) is the document language: the language of a
/// `[NatNumN]` or an implicit calendar in a code without a tag.
pub(crate) fn code_to_xml_in(
    code: &str,
    name: &str,
    out: &mut String,
    doc_lang: Option<&str>,
) -> Result<(), String> {
    let mut sections = Vec::new();
    for src in split_sections(code) {
        sections.push(tokenize_section(src)?);
    }
    if sections.len() > 4 {
        return Err("more than four sections".into());
    }
    if sections.len() == 4 {
        return Err("a fourth (text) section is not supported".into());
    }
    for s in &sections {
        if s.parts.is_empty() {
            return Err("empty section".into());
        }
        let kind = s.kind();
        let numbers = s
            .parts
            .iter()
            .filter(|p| {
                matches!(
                    p,
                    Part::Number { .. } | Part::Scientific { .. } | Part::Fraction { .. }
                )
            })
            .count();
        if kind.is_datetime() && numbers > 0 {
            return Err("digit placeholders inside a date/time section".into());
        }
        if kind == Kind::Text && numbers > 0 {
            return Err("text placeholder mixed with numbers".into());
        }
    }
    // The code's language goes on every section's style: AOO reads it from
    // the main style, the one with the style:map elements.
    let language = sections.iter().find_map(|s| s.language.clone());
    for s in &mut sections {
        s.language.clone_from(&language);
    }
    let lang = language
        .as_deref()
        .or(doc_lang)
        .and_then(tally_numfmt::lang_id);
    let n = sections.len();
    let mut main = sections.pop().ok_or("empty code")?;
    if main.condition.is_some() && n == 1 {
        main.condition = None;
    }
    let implied: &[&str] = match n {
        2 => &[">=0"],
        3 => &[">0", "<0"],
        _ => &[],
    };
    let mut maps = Vec::new();
    let mut buf = String::new();
    for (i, s) in sections.iter().enumerate() {
        let cond = s
            .condition
            .clone()
            .or_else(|| implied.get(i).map(|c| (*c).to_string()))
            .ok_or("missing condition")?;
        let mapped = format!("{name}P{i}");
        emit_style(s, &mapped, &[], &mut buf, lang);
        maps.push((cond, mapped));
    }
    emit_style(&main, name, &maps, &mut buf, lang);
    out.push_str(&buf);
    Ok(())
}

fn emit_style(
    s: &Section,
    name: &str,
    maps: &[(String, String)],
    out: &mut String,
    lang: Option<tally_numfmt::LangId>,
) {
    let kind = s.kind();
    let _ = write!(out, "<{} style:name=\"", kind.element());
    escape_attr(name, out);
    out.push('"');
    if s.elapsed {
        out.push_str(" number:truncate-on-overflow=\"false\"");
    }
    if let Some(l) = &s.language {
        push_language(l, out);
    }
    // xmlnumfe.cxx: number:transliteration-* from GetNatNumXml. A NatNum
    // that does nothing in its language is not written.
    // Without a language (no tag, no document locale) only the
    // language-independent NatNums are written, without a language.
    if let Some((n, db)) = s.natnum {
        let l = lang.unwrap_or(tally_numfmt::LangId::EN_US);
        let mode = tally_numfmt::NatNum { num: n, dbnum: db }.mode(l, kind == Kind::Date);
        if mode > 0 && tally_numfmt::natnum::is_valid_natnum(l, mode) {
            let (format, style) = tally_numfmt::natnum::to_xml_attributes(l, mode);
            out.push_str(" number:transliteration-format=\"");
            escape_attr(&format.to_string(), out);
            out.push('"');
            if let Some((tl, tc)) = lang.and_then(|l| tally_numfmt::lcid_language(&l.hex())) {
                let _ = write!(
                    out,
                    " number:transliteration-language=\"{tl}\" number:transliteration-country=\"{tc}\""
                );
            }
            let _ = write!(out, " number:transliteration-style=\"{style}\"");
        }
    }
    out.push('>');
    if let Some(c) = s.color {
        let _ = write!(
            out,
            "<style:text-properties fo:color=\"{}\"/>",
            crate::units::color_hex(c)
        );
    }
    // number:calendar: a [~calendar] from its position on, E / EE / R / RR
    // the language's other calendar for every element (xmlnumfe.cxx
    // lcl_GetDefaultCalendar), AAA / AAAA only for their element.
    let other = lang
        .and_then(tally_numfmt::calendar::other_calendar)
        .map(|c| c.name().to_string());
    let mut calendar = if s.other_calendar {
        other.clone()
    } else {
        None
    };
    for part in &s.parts {
        match part {
            Part::Calendar(c) => calendar = Some(c.clone()),
            Part::AaaDay { long } => emit_dated(
                &Part::DayOfWeek { long: *long },
                calendar.as_deref().or(other.as_deref()),
                out,
            ),
            _ => emit_dated(part, calendar.as_deref(), out),
        }
    }
    for (cond, target) in maps {
        out.push_str("<style:map style:condition=\"");
        escape_attr(&format!("value(){cond}"), out);
        out.push_str("\" style:apply-style-name=\"");
        escape_attr(target, out);
        out.push_str("\"/>");
    }
    let _ = write!(out, "</{}>", kind.element());
}

/// ` number:language="de" number:country="DE"` for a hex locale id.
fn push_language(lcid: &str, out: &mut String) {
    if let Some((lang, country)) = tally_numfmt::lcid_language(lcid) {
        let _ = write!(
            out,
            " number:language=\"{lang}\" number:country=\"{country}\""
        );
    }
}

/// The hex locale id of an element's `number:language` / `number:country`.
pub(crate) fn language_lcid(language: Option<&str>, country: Option<&str>) -> Option<String> {
    tally_numfmt::lcid_for_language(language?, country).map(str::to_string)
}

fn style_attr(long: bool) -> &'static str {
    if long {
        " number:style=\"long\""
    } else {
        ""
    }
}

/// A part with `number:calendar` on date elements.
fn emit_dated(part: &Part, calendar: Option<&str>, out: &mut String) {
    let dated = matches!(
        part,
        Part::Year { .. }
            | Part::Month { .. }
            | Part::Day { .. }
            | Part::DayOfWeek { .. }
            | Part::Quarter { .. }
            | Part::Week
            | Part::Era { .. }
    );
    match calendar {
        Some(c) if dated => {
            let mut el = String::new();
            emit_part(part, &mut el);
            // `<number:year number:style="long"/>` → insert before `/>`.
            if let Some(stem) = el.strip_suffix("/>") {
                out.push_str(stem);
                out.push_str(" number:calendar=\"");
                escape_attr(c, out);
                out.push_str("\"/>");
            } else {
                out.push_str(&el);
            }
        }
        _ => emit_part(part, out),
    }
}

fn emit_part(part: &Part, out: &mut String) {
    match part {
        Part::Number {
            decimals,
            min_decimals,
            min_int,
            grouping,
            factor_thousands,
        } => {
            out.push_str("<number:number");
            if let Some(d) = decimals {
                let _ = write!(out, " number:decimal-places=\"{d}\"");
                if let Some(md) = min_decimals.filter(|md| md != d) {
                    let _ = write!(out, " number:min-decimal-places=\"{md}\"");
                }
            }
            let _ = write!(out, " number:min-integer-digits=\"{min_int}\"");
            if *grouping {
                out.push_str(" number:grouping=\"true\"");
            }
            if *factor_thousands > 0 {
                let f = 1000u64.saturating_pow((*factor_thousands).min(4));
                let _ = write!(out, " number:display-factor=\"{f}\"");
            }
            out.push_str("/>");
        }
        Part::Scientific {
            decimals,
            min_int,
            min_exp,
            grouping,
        } => {
            let _ = write!(
                out,
                "<number:scientific-number number:decimal-places=\"{decimals}\" number:min-integer-digits=\"{min_int}\" number:min-exponent-digits=\"{min_exp}\"{}/>",
                if *grouping { " number:grouping=\"true\"" } else { "" }
            );
        }
        Part::Fraction {
            min_int,
            min_num,
            min_den,
            den_value,
        } => {
            out.push_str("<number:fraction");
            if let Some(mi) = min_int {
                let _ = write!(out, " number:min-integer-digits=\"{mi}\"");
            }
            let _ = write!(
                out,
                " number:min-numerator-digits=\"{min_num}\" number:min-denominator-digits=\"{min_den}\""
            );
            if let Some(v) = den_value {
                let _ = write!(out, " number:denominator-value=\"{v}\"");
            }
            out.push_str("/>");
        }
        Part::Text(t) => {
            out.push_str("<number:text>");
            escape_text(t, out);
            out.push_str("</number:text>");
        }
        Part::Currency { symbol, lcid } => {
            out.push_str("<number:currency-symbol");
            if let Some(l) = lcid {
                push_language(l, out);
            }
            out.push('>');
            escape_text(symbol, out);
            out.push_str("</number:currency-symbol>");
        }
        Part::TextContent => out.push_str("<number:text-content/>"),
        Part::Boolean => out.push_str("<number:boolean/>"),
        Part::Fill(c) => {
            out.push_str("<number:fill-character>");
            escape_text(&c.to_string(), out);
            out.push_str("</number:fill-character>");
        }
        Part::Year { long } => {
            let _ = write!(out, "<number:year{}/>", style_attr(*long));
        }
        Part::Month { long, textual } => {
            let _ = write!(
                out,
                "<number:month{}{}/>",
                if *textual {
                    " number:textual=\"true\""
                } else {
                    ""
                },
                style_attr(*long)
            );
        }
        Part::Day { long } => {
            let _ = write!(out, "<number:day{}/>", style_attr(*long));
        }
        Part::DayOfWeek { long } => {
            let _ = write!(out, "<number:day-of-week{}/>", style_attr(*long));
        }
        Part::Hours { long } => {
            let _ = write!(out, "<number:hours{}/>", style_attr(*long));
        }
        Part::Minutes { long } => {
            let _ = write!(out, "<number:minutes{}/>", style_attr(*long));
        }
        Part::Seconds { long, decimals } => {
            let _ = write!(out, "<number:seconds{}", style_attr(*long));
            if *decimals > 0 {
                let _ = write!(out, " number:decimal-places=\"{decimals}\"");
            }
            out.push_str("/>");
        }
        Part::AmPm => out.push_str("<number:am-pm/>"),
        Part::Quarter { long } => {
            let _ = write!(out, "<number:quarter{}/>", style_attr(*long));
        }
        Part::Week => out.push_str("<number:week-of-year/>"),
        Part::Era { long } => {
            let _ = write!(out, "<number:era{}/>", style_attr(*long));
        }
        // Handled by emit_style / tokenize_section.
        Part::Calendar(_) | Part::AaaDay { .. } | Part::EraR { .. } => {}
    }
}

/// Name → data style table used while reading.
pub(crate) type DataStyles = HashMap<String, DataStyle>;
