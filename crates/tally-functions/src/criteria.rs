//! COUNTIF-family criteria: parsing `"<5"`, `">=x"`, `"=abc"`, `"<>Paid"`
//! and bare values, and matching cell values against them.
//!
//! Shared by COUNTIF, SUMIF, AVERAGEIF, the `*IFS` functions, MATCH type 0,
//! the lookups, SEARCH and the D* functions.
//!
//! # Rules
//!
//! - An operator prefix (`=`, `<>`, `<`, `<=`, `>`, `>=`) is stripped; no
//!   prefix means "equal" (a *bare* criterion).
//! - The remaining text becomes the operand: a number if the document
//!   locale's input recognizer ([`crate::coerce::recognize_number`]: today's
//!   date for year-less dates) or [`crate::coerce::parse_unambiguous`]
//!   accepts it (so `">10/1/2026"`, `">1,000"`, `">50%"`, `">$5"` and
//!   `">2024-01-01"` work under en-US and `">1,5"`, `"<19.10.2026"` under
//!   de-DE, as AOO's `IsNumberFormat` makes them), a boolean for
//!   `TRUE`/`FALSE` or the locale's names (`WAHR`/`FALSCH`), an error for
//!   `#N/A`-style names, otherwise text.
//! - A number operand matches numbers (by [`approx_equal`]), booleans (as
//!   1/0, following AOO's storage model) and, for `=`, text that parses as
//!   that number. Ordering operators never match text or empty cells.
//! - A text operand matches only text, using the criteria mode for `=`/`<>`:
//!   `Wildcards` (`*`, `?`, `~` escapes; compiled once per criterion into a
//!   [`Glob`] that matches without backtracking), `Regex` (the `regex`
//!   crate, 1 MiB size limit; an invalid pattern matches nothing) or
//!   `Literal`.
//!   With `whole_cell_match` off, `=` matches when the pattern occurs
//!   anywhere in the cell. Ordering operators compare text lexically.
//! - `<>x` matches everything `=x` does not, including empty cells.
//! - Empty operands: a bare `""` matches empty cells and empty text, `"="`
//!   matches only empty cells, `"<>"` matches every non-empty cell.
//! - Case sensitivity follows `settings.case_sensitive` (default `false` in
//!   [`CalcSettings`], as in Excel and LibreOffice's UI; AOO's document
//!   default is case-sensitive, a deliberate deviation recorded in
//!   docs/adr/0001-stack.md).
//! - A non-text criterion value: a number or boolean is a bare equality; an
//!   empty cell behaves like the number 0 (AOO); an error matches cells with
//!   the same error (callers usually propagate it first).

use regex::{Regex, RegexBuilder};
use tally_numfmt::{Locale, RecognizeContext, EN_US};
use tally_types::{approx_equal, CalcSettings, CriteriaMode, DateEpoch, ErrorKind, Value};

use crate::coerce::{parse_unambiguous, recognize_number, top_left};

/// Comparison operator of a criterion.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum CmpOp {
    /// `=` or no operator.
    Eq,
    /// `<>`.
    Ne,
    /// `<`.
    Lt,
    /// `<=`.
    Le,
    /// `>`.
    Gt,
    /// `>=`.
    Ge,
}

#[derive(Clone, Debug)]
enum Operand {
    /// Empty operand of `=`/`<>`/bare: `with_empty_text` also counts `""`.
    Blank {
        with_empty_text: bool,
    },
    Number(f64),
    Bool(bool),
    Error(ErrorKind),
    Text(TextPattern),
}

#[derive(Clone, Debug)]
struct TextPattern {
    /// The operand text, lower-cased when matching is case-insensitive.
    folded: String,
    fold: bool,
    matcher: Matcher,
}

#[derive(Clone, Debug)]
enum Matcher {
    Literal {
        whole: bool,
    },
    Wildcard(Glob),
    /// `None` when the pattern failed to compile (matches nothing).
    Regex(Option<Regex>),
}

#[derive(Copy, Clone, Eq, PartialEq, Debug)]
enum WTok {
    Char(char),
    AnyOne,
    AnyMany,
}

/// A parsed criterion. Build with [`Criterion::parse`], test with [`Criterion::matches`].
#[derive(Clone, Debug)]
pub struct Criterion {
    op: CmpOp,
    operand: Operand,
    epoch: DateEpoch,
}

const REGEX_SIZE_LIMIT: usize = 1 << 20;

fn fold(s: &str, fold: bool) -> String {
    if fold {
        s.to_lowercase()
    } else {
        s.to_owned()
    }
}

fn split_op(s: &str) -> (CmpOp, &str, bool) {
    for (prefix, op) in [
        ("<=", CmpOp::Le),
        (">=", CmpOp::Ge),
        ("<>", CmpOp::Ne),
        ("=", CmpOp::Eq),
        ("<", CmpOp::Lt),
        (">", CmpOp::Gt),
    ] {
        if let Some(rest) = s.strip_prefix(prefix) {
            return (op, rest, false);
        }
    }
    (CmpOp::Eq, s, true)
}

fn wildcard_tokens(pattern: &str, whole: bool) -> Vec<WTok> {
    let mut toks = Vec::with_capacity(pattern.len() + 2);
    if !whole {
        toks.push(WTok::AnyMany);
    }
    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '~' => match chars.peek() {
                Some(&n @ ('*' | '?' | '~')) => {
                    chars.next();
                    toks.push(WTok::Char(n));
                }
                // A lone or trailing tilde is literal.
                _ => toks.push(WTok::Char('~')),
            },
            '*' => {
                if toks.last() != Some(&WTok::AnyMany) {
                    toks.push(WTok::AnyMany);
                }
            }
            '?' => toks.push(WTok::AnyOne),
            other => toks.push(WTok::Char(other)),
        }
    }
    if !whole && toks.last() != Some(&WTok::AnyMany) {
        toks.push(WTok::AnyMany);
    }
    toks
}

/// One piece of a glob between `*`s: characters, `None` for `?`.
#[derive(Clone, Debug)]
struct Segment {
    chars: Vec<Option<char>>,
    /// The segment as a string when it has no `?` (fast substring search).
    literal: Option<String>,
}

impl Segment {
    fn new(chars: Vec<Option<char>>) -> Self {
        let literal = chars.iter().copied().collect::<Option<String>>();
        Self { chars, literal }
    }

    /// Matches at the start of `hay`; returns the byte length consumed.
    fn match_prefix(&self, hay: &str) -> Option<usize> {
        let mut it = hay.char_indices();
        for want in &self.chars {
            let (_, c) = it.next()?;
            if want.is_some_and(|w| w != c) {
                return None;
            }
        }
        Some(it.next().map_or(hay.len(), |(i, _)| i))
    }

    /// Byte offset of the start of the last `chars.len()` characters of
    /// `hay`, if the segment matches there.
    fn match_suffix(&self, hay: &str) -> Option<usize> {
        let n = self.chars.len();
        let start = if n == 0 {
            hay.len()
        } else {
            hay.char_indices().rev().nth(n - 1)?.0
        };
        self.match_prefix(&hay[start..])?;
        Some(start)
    }

    /// Leftmost occurrence in `hay`: `(start, end)` byte offsets.
    fn find(&self, hay: &str) -> Option<(usize, usize)> {
        if let Some(lit) = &self.literal {
            // Two-way substring search: linear time.
            return hay.find(lit.as_str()).map(|i| (i, i + lit.len()));
        }
        // Shift-And over the characters (`?` matches any character):
        // O(len(hay) × len(segment) / 64), no backtracking.
        let m = self.chars.len();
        if m == 0 {
            return Some((0, 0));
        }
        let words = m.div_ceil(64);
        let mut any = vec![0u64; words];
        let mut masks: std::collections::HashMap<char, Vec<u64>> = std::collections::HashMap::new();
        for (i, c) in self.chars.iter().enumerate() {
            let (w, b) = (i / 64, i % 64);
            match c {
                None => any[w] |= 1 << b,
                Some(c) => masks.entry(*c).or_insert_with(|| vec![0; words])[w] |= 1 << b,
            }
        }
        let last = ((m - 1) / 64, (m - 1) % 64);
        let mut state = vec![0u64; words];
        let mut starts: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
        for (pos, c) in hay.char_indices() {
            starts.push_back(pos);
            if starts.len() > m {
                starts.pop_front();
            }
            let mask = masks.get(&c);
            let mut carry = 1u64;
            for (w, st) in state.iter_mut().enumerate() {
                let next_carry = *st >> 63;
                let m_w = any[w] | mask.map_or(0, |v| v[w]);
                *st = ((*st << 1) | carry) & m_w;
                carry = next_carry;
            }
            if state[last.0] & (1 << last.1) != 0 {
                let start = starts.front().copied().unwrap_or(pos);
                return Some((start, pos + c.len_utf8()));
            }
        }
        None
    }
}

/// A compiled wildcard pattern: `P0 * P1 * … * Pk`.
///
/// Matching is greedy and never backtracks: `P0` must match at the start,
/// `Pk` at the end, and each middle piece takes its leftmost occurrence
/// after the previous one (which is optimal for `*`, since a fixed-length
/// piece that ends earliest leaves the most room). Pieces without `?` use
/// linear substring search; others use bit-parallel Shift-And. A 4,000-
/// character pattern against 1 MB of text costs a few milliseconds instead
/// of the O(text × pattern) of a backtracking matcher.
#[derive(Clone, Debug)]
pub(crate) struct Glob {
    /// The pieces between `*`s (`pieces.len() == stars + 1`).
    pieces: Vec<Segment>,
}

impl Glob {
    fn new(toks: &[WTok]) -> Self {
        let mut pieces = Vec::new();
        let mut cur = Vec::new();
        for tok in toks {
            match tok {
                WTok::AnyMany => pieces.push(Segment::new(std::mem::take(&mut cur))),
                WTok::AnyOne => cur.push(None),
                WTok::Char(c) => cur.push(Some(*c)),
            }
        }
        pieces.push(Segment::new(cur));
        Self { pieces }
    }

    fn is_match(&self, text: &str) -> bool {
        let n = self.pieces.len();
        let first = &self.pieces[0];
        if n == 1 {
            return first.match_prefix(text) == Some(text.len());
        }
        let Some(mut pos) = first.match_prefix(text) else {
            return false;
        };
        let Some(end) = self.pieces[n - 1]
            .match_suffix(&text[pos..])
            .map(|s| pos + s)
        else {
            return false;
        };
        for piece in &self.pieces[1..n - 1] {
            match piece.find(&text[pos..end]) {
                Some((_, e)) => pos += e,
                None => return false,
            }
        }
        true
    }
}

impl TextPattern {
    fn new(text: &str, settings: &CalcSettings, whole: bool) -> Self {
        let fold_case = !settings.case_sensitive;
        let folded = fold(text, fold_case);
        let matcher = match settings.criteria_mode {
            CriteriaMode::Literal => Matcher::Literal { whole },
            CriteriaMode::Wildcards => {
                Matcher::Wildcard(Glob::new(&wildcard_tokens(&folded, whole)))
            }
            CriteriaMode::Regex => {
                let pattern = if whole {
                    format!("^(?:{text})$")
                } else {
                    text.to_owned()
                };
                Matcher::Regex(
                    RegexBuilder::new(&pattern)
                        .case_insensitive(fold_case)
                        .size_limit(REGEX_SIZE_LIMIT)
                        .dfa_size_limit(REGEX_SIZE_LIMIT)
                        .build()
                        .ok(),
                )
            }
        };
        Self {
            folded,
            fold: fold_case,
            matcher,
        }
    }

    /// Equality under the criteria mode.
    fn equals(&self, s: &str) -> bool {
        match &self.matcher {
            Matcher::Regex(Some(re)) => re.is_match(s),
            Matcher::Regex(None) => false,
            Matcher::Literal { whole } => {
                let cand = fold(s, self.fold);
                if *whole {
                    cand == self.folded
                } else {
                    cand.contains(self.folded.as_str())
                }
            }
            Matcher::Wildcard(glob) => glob.is_match(&fold(s, self.fold)),
        }
    }

    fn is_broken(&self) -> bool {
        matches!(self.matcher, Matcher::Regex(None))
    }
}

fn ordering_holds(op: CmpOp, ord: std::cmp::Ordering) -> bool {
    use std::cmp::Ordering::*;
    match op {
        CmpOp::Lt => ord == Less,
        CmpOp::Le => ord != Greater,
        CmpOp::Gt => ord == Greater,
        CmpOp::Ge => ord != Less,
        CmpOp::Eq => ord == Equal,
        CmpOp::Ne => ord != Equal,
    }
}

fn cmp_numbers(a: f64, b: f64) -> std::cmp::Ordering {
    if approx_equal(a, b) {
        std::cmp::Ordering::Equal
    } else if a < b {
        std::cmp::Ordering::Less
    } else {
        std::cmp::Ordering::Greater
    }
}

/// The numeric value of a criterion operand: the locale's input recognizer
/// first (so `"10/1/2026"`, `"1,000"`, `"50%"` and `"$5"` are numbers under
/// en-US, as AOO's `IsNumberFormat` makes them), then the unambiguous parser.
fn criterion_number(rest: &str, settings: &CalcSettings, locale: &Locale) -> Option<f64> {
    recognize_number(rest, locale, &RecognizeContext::for_settings(settings))
}

/// Case-insensitive match of a boolean name (`TRUE`, or the locale's `WAHR`).
fn is_name(s: &str, name: &str) -> bool {
    s.len() <= name.len() * 4 && s.to_lowercase() == name.to_lowercase()
}

impl Criterion {
    /// Parses a criterion from a cell or argument value (see the module
    /// docs) with en-US number spellings; [`Criterion::parse_in`] takes the
    /// document locale, which is what functions use.
    pub fn parse(value: &Value, settings: &CalcSettings) -> Criterion {
        Criterion::parse_in(value, settings, &EN_US)
    }

    /// Parses a criterion whose numeric operands are spelled in `locale`
    /// (see the module docs).
    pub fn parse_in(value: &Value, settings: &CalcSettings, locale: &Locale) -> Criterion {
        let epoch = settings.date_epoch;
        let (op, operand) = match value {
            Value::Empty => (CmpOp::Eq, Operand::Number(0.0)),
            Value::Number(n) => (CmpOp::Eq, Operand::Number(*n)),
            Value::Bool(b) => (CmpOp::Eq, Operand::Bool(*b)),
            Value::Error(e) => (CmpOp::Eq, Operand::Error(*e)),
            Value::Array(m) => {
                return Criterion::parse_in(&top_left(m).unwrap_or(Value::Empty), settings, locale)
            }
            Value::Text(s) => {
                let (op, rest, bare) = split_op(s);
                let operand = if rest.is_empty() {
                    match op {
                        CmpOp::Eq | CmpOp::Ne => Operand::Blank {
                            with_empty_text: bare || op == CmpOp::Ne,
                        },
                        _ => Operand::Text(TextPattern::new("", settings, true)),
                    }
                } else if let Some(n) = criterion_number(rest, settings, locale) {
                    Operand::Number(n)
                } else if is_name(rest, "TRUE") || is_name(rest, locale.true_name) {
                    Operand::Bool(true)
                } else if is_name(rest, "FALSE") || is_name(rest, locale.false_name) {
                    Operand::Bool(false)
                } else if let Some(e) = ErrorKind::parse(rest) {
                    Operand::Error(e)
                } else {
                    Operand::Text(TextPattern::new(rest, settings, settings.whole_cell_match))
                };
                (op, operand)
            }
        };
        Criterion { op, operand, epoch }
    }

    /// The comparison operator.
    pub fn op(&self) -> CmpOp {
        self.op
    }

    /// `true` if the operand is numeric (a number or a boolean).
    pub fn is_numeric(&self) -> bool {
        matches!(self.operand, Operand::Number(_) | Operand::Bool(_))
    }

    /// Tests a cell value against the criterion.
    pub fn matches(&self, value: &Value) -> bool {
        let value = match value {
            Value::Array(m) => return self.matches(&top_left(m).unwrap_or(Value::Empty)),
            v => v,
        };
        match &self.operand {
            Operand::Blank { with_empty_text } => {
                let blank = match value {
                    Value::Empty => true,
                    Value::Text(s) => *with_empty_text && s.is_empty(),
                    _ => false,
                };
                match self.op {
                    CmpOp::Ne => !matches!(value, Value::Empty),
                    _ => blank,
                }
            }
            Operand::Number(n) => self.match_number(*n, value),
            Operand::Bool(b) => self.match_number(if *b { 1.0 } else { 0.0 }, value),
            Operand::Error(e) => {
                let same = value.as_error() == Some(*e);
                match self.op {
                    CmpOp::Eq => same,
                    CmpOp::Ne => !same,
                    _ => false,
                }
            }
            Operand::Text(p) => match self.op {
                CmpOp::Eq => matches!(value, Value::Text(s) if p.equals(s)),
                CmpOp::Ne => !p.is_broken() && !matches!(value, Value::Text(s) if p.equals(s)),
                op => match value {
                    Value::Text(s) => {
                        ordering_holds(op, fold(s, p.fold).as_str().cmp(p.folded.as_str()))
                    }
                    _ => false,
                },
            },
        }
    }

    fn match_number(&self, n: f64, value: &Value) -> bool {
        let cell = match value {
            Value::Number(x) => Some(*x),
            Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
            Value::Text(s) if matches!(self.op, CmpOp::Eq | CmpOp::Ne) => {
                parse_unambiguous(s, self.epoch)
            }
            _ => None,
        };
        match (self.op, cell) {
            (CmpOp::Ne, None) => true,
            (_, None) => false,
            (op, Some(x)) => ordering_holds(op, cmp_numbers(x, n)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reference matcher: plain recursion (exponential, tiny inputs only).
    fn reference(toks: &[WTok], text: &[char]) -> bool {
        match toks.split_first() {
            None => text.is_empty(),
            Some((WTok::AnyMany, rest)) => (0..=text.len()).any(|i| reference(rest, &text[i..])),
            Some((WTok::AnyOne, rest)) => !text.is_empty() && reference(rest, &text[1..]),
            Some((WTok::Char(c), rest)) => text.first() == Some(c) && reference(rest, &text[1..]),
        }
    }

    /// Every pattern over `{a, b, é, ?, *}` up to length 5 against every
    /// text over `{a, b, é}` up to length 5 (wide enough for multi-word
    /// Shift-And to be exercised separately below).
    #[test]
    fn glob_agrees_with_reference() {
        let pat_alpha = ['a', 'b', 'é', '?', '*'];
        let text_alpha = ['a', 'b', 'é'];
        fn all(alpha: &[char], max: usize) -> Vec<String> {
            let mut out = vec![String::new()];
            let mut level = vec![String::new()];
            for _ in 0..max {
                let mut next = Vec::new();
                for s in &level {
                    for c in alpha {
                        next.push(format!("{s}{c}"));
                    }
                }
                out.extend(next.iter().cloned());
                level = next;
            }
            out
        }
        let texts = all(&text_alpha, 5);
        for p in all(&pat_alpha, 5) {
            let toks = wildcard_tokens(&p, true);
            let glob = Glob::new(&toks);
            for t in &texts {
                let chars: Vec<char> = t.chars().collect();
                assert_eq!(glob.is_match(t), reference(&toks, &chars), "{p:?} vs {t:?}");
            }
        }
    }

    #[test]
    fn shift_and_spans_words() {
        // 130-character piece with `?`s: three 64-bit words.
        let piece: String = (0..130)
            .map(|i| if i % 7 == 0 { '?' } else { 'a' })
            .collect();
        let toks = wildcard_tokens(&format!("*{piece}*"), true);
        let glob = Glob::new(&toks);
        assert!(glob.is_match(&format!("xx{}yy", "a".repeat(130))));
        assert!(!glob.is_match(&"a".repeat(129)));
        let mut t = "a".repeat(130);
        t.replace_range(1..2, "b");
        assert!(!glob.is_match(&t));
        t.replace_range(1..2, "a");
        t.replace_range(7..8, "é");
        assert!(glob.is_match(&t));
    }
}
