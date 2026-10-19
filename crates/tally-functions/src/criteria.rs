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
//! - The remaining text becomes the operand: a number if
//!   [`crate::coerce::parse_unambiguous`] accepts it (so `">2024-01-01"`
//!   works), a boolean for `TRUE`/`FALSE`, an error for `#N/A`-style names,
//!   otherwise text.
//! - A number operand matches numbers (by [`approx_equal`]), booleans (as
//!   1/0, following AOO's storage model) and, for `=`, text that parses as
//!   that number. Ordering operators never match text or empty cells.
//! - A text operand matches only text, using the criteria mode for `=`/`<>`:
//!   `Wildcards` (`*`, `?`, `~` escapes), `Regex` (the `regex` crate,
//!   1 MiB size limit; an invalid pattern matches nothing) or `Literal`.
//!   With `whole_cell_match` off, `=` matches when the pattern occurs
//!   anywhere in the cell. Ordering operators compare text lexically.
//! - `<>x` matches everything `=x` does not, including empty cells.
//! - Empty operands: a bare `""` matches empty cells and empty text, `"="`
//!   matches only empty cells, `"<>"` matches every non-empty cell.
//! - Case sensitivity follows `settings.case_sensitive` (default `false` in
//!   [`CalcSettings`], matching AOO's default for criteria).
//! - A non-text criterion value: a number or boolean is a bare equality; an
//!   empty cell behaves like the number 0 (AOO); an error matches cells with
//!   the same error (callers usually propagate it first).

use regex::{Regex, RegexBuilder};
use tally_types::{approx_equal, CalcSettings, CriteriaMode, DateEpoch, ErrorKind, Value};

use crate::coerce::{parse_unambiguous, top_left};

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
    Wildcard(Vec<WTok>),
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

/// Glob match with single-star backtracking: O(len(text) × len(pattern)) worst case.
fn wildcard_match(toks: &[WTok], text: &[char]) -> bool {
    let (mut p, mut t) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None;
    while t < text.len() {
        match toks.get(p) {
            Some(WTok::AnyMany) => {
                star = Some((p, t));
                p += 1;
            }
            Some(WTok::AnyOne) => {
                p += 1;
                t += 1;
            }
            Some(WTok::Char(c)) if *c == text[t] => {
                p += 1;
                t += 1;
            }
            _ => match star {
                Some((sp, st)) => {
                    p = sp + 1;
                    t = st + 1;
                    star = Some((sp, st + 1));
                }
                None => return false,
            },
        }
    }
    toks[p.min(toks.len())..]
        .iter()
        .all(|tok| *tok == WTok::AnyMany)
}

impl TextPattern {
    fn new(text: &str, settings: &CalcSettings, whole: bool) -> Self {
        let fold_case = !settings.case_sensitive;
        let folded = fold(text, fold_case);
        let matcher = match settings.criteria_mode {
            CriteriaMode::Literal => Matcher::Literal { whole },
            CriteriaMode::Wildcards => Matcher::Wildcard(wildcard_tokens(&folded, whole)),
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
            Matcher::Wildcard(toks) => {
                let cand: Vec<char> = fold(s, self.fold).chars().collect();
                wildcard_match(toks, &cand)
            }
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

impl Criterion {
    /// Parses a criterion from a cell or argument value (see the module docs).
    pub fn parse(value: &Value, settings: &CalcSettings) -> Criterion {
        let epoch = settings.date_epoch;
        let (op, operand) = match value {
            Value::Empty => (CmpOp::Eq, Operand::Number(0.0)),
            Value::Number(n) => (CmpOp::Eq, Operand::Number(*n)),
            Value::Bool(b) => (CmpOp::Eq, Operand::Bool(*b)),
            Value::Error(e) => (CmpOp::Eq, Operand::Error(*e)),
            Value::Array(m) => {
                return Criterion::parse(&top_left(m).unwrap_or(Value::Empty), settings)
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
                } else if let Some(n) = parse_unambiguous(rest, epoch) {
                    Operand::Number(n)
                } else if rest.eq_ignore_ascii_case("TRUE") {
                    Operand::Bool(true)
                } else if rest.eq_ignore_ascii_case("FALSE") {
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
