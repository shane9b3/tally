//! Candidate generation: aggregate + target column + conditions.

use tally_formula::{parse, Dialect, SimpleResolver};
use tally_types::{col_to_letters, Value};

use crate::lexicon::{
    find_aggregate, is_stopword, jaro_winkler, month_number, parse_comparator, parse_number,
    singular, tokenize, value_words, Agg, Cmp, Token,
};
use crate::region::{analyze, detect_range, ColumnType, Region};
use crate::{AssistRequest, Suggestion, SuggestionKind};

/// Longest query considered, in bytes.
const MAX_QUERY_LEN: usize = 400;
/// Maximum suggestions returned.
pub const MAX_SUGGESTIONS: usize = 5;

const ALIASES: &[&[&str]] = &[
    &["qty", "quantity", "unit", "piece"],
    &["price", "cost", "rate"],
    &["total", "amount", "revenue", "sale"],
    &["customer", "client", "buyer"],
    &["item", "product", "sku"],
    &["status", "state"],
    &["date", "day", "when"],
];

const MONEY_WORDS: &[&str] = &[
    "total", "amount", "price", "cost", "revenue", "sale", "fee", "salary", "balance", "payment",
    "value",
];

const DEFAULT_TARGETS: &[&str] = &[
    "total", "amount", "price", "value", "cost", "revenue", "sale", "balance", "payment",
];

#[derive(Clone, Debug, PartialEq)]
enum Operand {
    Text(String),
    Num(f64),
}

#[derive(Clone, Debug)]
struct Cond {
    col: Option<usize>,
    cmp: Cmp,
    operand: Operand,
    fuzzy: bool,
    pos: usize,
}

/// Best header match among the query tokens for a column-type predicate.
type HeaderFinder<'a> = dyn Fn(&dyn Fn(ColumnType) -> bool) -> Option<(usize, f64)> + 'a;

struct ValueHit {
    col: usize,
    value: String,
    negate: bool,
    fuzzy: bool,
    lo: usize,
    hi: usize,
}

fn fmt_num(n: f64) -> String {
    format!("{n}")
}

fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

/// Criteria argument for a condition, e.g. `"<>Paid"`, `">10"` or `20`.
fn criteria(c: &Cond) -> String {
    match &c.operand {
        Operand::Text(v) => {
            let lead = v.starts_with(['<', '>', '=']);
            match c.cmp {
                Cmp::Ne => quote(&format!("<>{v}")),
                _ if lead => quote(&format!("={v}")),
                _ => quote(v),
            }
        }
        Operand::Num(n) => match c.cmp {
            Cmp::Eq => fmt_num(*n),
            cmp => quote(&format!("{}{}", cmp.symbol(), fmt_num(*n))),
        },
    }
}

fn operand_text(c: &Cond) -> String {
    match &c.operand {
        Operand::Text(v) => v.clone(),
        Operand::Num(n) => fmt_num(*n),
    }
}

fn cond_text(reg: &Region, c: &Cond) -> String {
    let label = c.col.map(|i| reg.label(i)).unwrap_or_default();
    let words = c.cmp.words();
    let val = operand_text(c);
    match (c.cmp, &c.operand) {
        (Cmp::Eq, _) => format!("{label} is {val}"),
        (Cmp::Ne, _) => format!("{label} is not {val}"),
        _ => format!("{label} is {words} {val}"),
    }
}

/// Human explanation, e.g. "Sum of Total where Status is not Paid".
pub fn explain(agg: Agg, target: Option<&str>, conditions: &[String]) -> String {
    let head = match (agg, target) {
        (Agg::Sum, Some(t)) => format!("Sum of {t}"),
        (Agg::Average, Some(t)) => format!("Average of {t}"),
        (Agg::Max, Some(t)) => format!("Maximum of {t}"),
        (Agg::Min, Some(t)) => format!("Minimum of {t}"),
        (Agg::Median, Some(t)) => format!("Median of {t}"),
        (Agg::Count, Some(t)) => format!("Count of values in {t}"),
        (Agg::Count, None) => "Count of rows".to_string(),
        (_, None) => "Result".to_string(),
    };
    if conditions.is_empty() {
        head
    } else {
        format!("{head} where {}", conditions.join(" and "))
    }
}

/// Formats a preview value: up to 2 decimals, thousands separators, `$`
/// prefix when `money`.
pub fn format_preview(v: &Value, money: bool) -> Option<String> {
    match v {
        Value::Number(n) if n.is_finite() => {
            let neg = *n < 0.0;
            let s = format!("{:.2}", n.abs());
            let (int, frac) = s.split_once('.').unwrap_or((&s, "00"));
            let frac = frac.trim_end_matches('0');
            let mut grouped = String::new();
            for (i, ch) in int.chars().enumerate() {
                if i > 0 && (int.len() - i) % 3 == 0 {
                    grouped.push(',');
                }
                grouped.push(ch);
            }
            if !frac.is_empty() {
                grouped.push('.');
                grouped.push_str(frac);
            }
            let sign = if neg && grouped.chars().any(|c| matches!(c, '1'..='9')) {
                "-"
            } else {
                ""
            };
            Some(format!("{sign}{}{grouped}", if money { "$" } else { "" }))
        }
        Value::Text(s) => Some(s.to_string()),
        Value::Bool(b) => Some(if *b { "TRUE" } else { "FALSE" }.to_string()),
        _ => None,
    }
}

fn header_is_money(reg: &Region, idx: usize) -> bool {
    reg.columns
        .get(idx)
        .and_then(|c| c.header.as_deref())
        .is_some_and(|h| {
            value_words(h)
                .iter()
                .any(|w| MONEY_WORDS.contains(&w.as_str()))
        })
}

fn alias_group(w: &str) -> Option<usize> {
    ALIASES.iter().position(|g| g.contains(&w))
}

/// Score of a query word against a header word.
fn word_score(q: &str, h: &str) -> f64 {
    if q == h {
        return 1.0;
    }
    if let (Some(a), Some(b)) = (alias_group(q), alias_group(h)) {
        if a == b {
            return 0.9;
        }
    }
    // Misspelled alias ("quantiy" for "qty").
    if let Some(g) = alias_group(h) {
        if q.chars().count() >= 5
            && ALIASES[g]
                .iter()
                .any(|a| a.chars().next() == q.chars().next() && jaro_winkler(q, a) >= 0.9)
        {
            return 0.88;
        }
    }
    if q.chars().count() >= 4
        && h.chars().count() >= 4
        && q.chars().next() == h.chars().next()
        && similar_len(q, h)
    {
        let s = jaro_winkler(q, h);
        if s >= 0.88 {
            return s * 0.97;
        }
    }
    0.0
}

struct Analysis<'a> {
    reg: &'a Region,
    toks: Vec<Token>,
    sing: Vec<String>,
    /// Best matching column per token.
    tok_col: Vec<Option<(usize, f64)>>,
}

impl<'a> Analysis<'a> {
    fn new(reg: &'a Region, toks: Vec<Token>) -> Self {
        let sing: Vec<String> = toks.iter().map(|t| singular(&t.text)).collect();
        let header_words: Vec<Vec<String>> = reg
            .columns
            .iter()
            .map(|c| c.header.as_deref().map(value_words).unwrap_or_default())
            .collect();
        let tok_col = sing
            .iter()
            .map(|w| {
                if is_stopword(w) || parse_number(w).is_some() || w.len() < 2 {
                    return None;
                }
                let mut best: Option<(usize, f64)> = None;
                for (ci, hw) in header_words.iter().enumerate() {
                    let scale = if hw.len() > 1 { 0.95 } else { 1.0 };
                    let s = hw.iter().map(|h| word_score(w, h)).fold(0.0, f64::max) * scale;
                    if s >= 0.85 && best.is_none_or(|(_, b)| s > b) {
                        best = Some((ci, s));
                    }
                }
                best
            })
            .collect();
        Self {
            reg,
            toks,
            sing,
            tok_col,
        }
    }

    fn find_values(&self) -> Vec<ValueHit> {
        let n = self.toks.len();
        let mut cands: Vec<(f64, ValueHit)> = Vec::new();
        for (ci, col) in self.reg.columns.iter().enumerate() {
            if col.ty != ColumnType::Category {
                continue;
            }
            for v in &col.distinct {
                let words = value_words(v);
                let k = words.len();
                if k == 0 || k > 4 || k > n {
                    continue;
                }
                for i in 0..=(n - k) {
                    if let Some((score, un, fuzzy)) = self.window_score(i, &words) {
                        let negate = un || self.negated_before(i);
                        cands.push((
                            score + k as f64 * 0.001,
                            ValueHit {
                                col: ci,
                                value: v.clone(),
                                negate,
                                fuzzy,
                                lo: i,
                                hi: i + k,
                            },
                        ));
                    }
                }
            }
        }
        cands.sort_by(|a, b| b.0.total_cmp(&a.0));
        let mut used = vec![false; n];
        let mut cols_used = vec![false; self.reg.columns.len()];
        let mut out = Vec::new();
        for (_, h) in cands {
            if cols_used[h.col] || used[h.lo..h.hi].iter().any(|u| *u) {
                continue;
            }
            cols_used[h.col] = true;
            used[h.lo..h.hi].iter_mut().for_each(|u| *u = true);
            out.push(h);
        }
        out.sort_by_key(|h| h.lo);
        out
    }

    /// Score, "un-" prefix flag and fuzzy flag for a value window at `i`.
    fn window_score(&self, i: usize, words: &[String]) -> Option<(f64, bool, bool)> {
        let mut total = 0.0;
        let mut un = false;
        let mut fuzzy = false;
        for (j, w) in words.iter().enumerate() {
            let q = &self.sing[i + j];
            if parse_number(q).is_some() && q != w {
                return None;
            }
            let header_exact = self.tok_col[i + j].is_some_and(|(_, s)| s >= 0.9);
            let mut best = 0.0;
            if q == w {
                best = 1.0;
            } else if j == 0 && !header_exact {
                // "unpaid" / "nonpaid" against "Paid".
                for p in ["un", "non"] {
                    if let Some(rest) = q.strip_prefix(p) {
                        if rest.chars().count() >= 3 && (rest == w || fuzzy_eq(rest, w)) {
                            best = if rest == w { 0.99 } else { 0.9 };
                            un = true;
                            fuzzy |= rest != w;
                        }
                    }
                }
            }
            if best == 0.0 && !header_exact && !is_stopword(q) && fuzzy_eq(q, w) {
                best = jaro_winkler(q, w);
                fuzzy = true;
            }
            if best == 0.0 {
                return None;
            }
            total += best;
        }
        Some((total / words.len() as f64, un, fuzzy))
    }

    fn negated_before(&self, i: usize) -> bool {
        let t = |k: usize| self.toks.get(k).map(|t| t.text.as_str());
        let neg = |s: &str| {
            matches!(
                s,
                "not" | "non" | "except" | "excluding" | "without" | "<>" | "exclude"
            )
        };
        if i >= 1 {
            if let Some(p) = t(i - 1) {
                if neg(p) || (p == "than" && i >= 2 && t(i - 2) == Some("other")) {
                    return true;
                }
            }
        }
        false
    }

    /// Nearest column-name token within three tokens before `i`.
    fn col_before(&self, i: usize, consumed: &[bool]) -> Option<(usize, usize)> {
        (i.saturating_sub(3)..i).rev().find_map(|j| {
            if consumed[j] {
                return None;
            }
            self.tok_col[j].map(|(c, _)| (c, j))
        })
    }
}

/// Lengths within 25% of each other ("over" must not match "overdue").
fn similar_len(a: &str, b: &str) -> bool {
    let (x, y) = (a.chars().count(), b.chars().count());
    x.min(y) * 4 >= x.max(y) * 3
}

fn fuzzy_eq(a: &str, b: &str) -> bool {
    a.chars().count() >= 4
        && b.chars().count() >= 4
        && a.chars().next() == b.chars().next()
        && similar_len(a, b)
        && jaro_winkler(a, b) >= 0.85
}

fn is_cmd(sing: &[String], words: &[&str]) -> bool {
    sing.iter().any(|s| words.contains(&s.as_str()))
}

struct Draft {
    s: Suggestion,
}

fn command(
    id: &str,
    title: String,
    explanation: String,
    confidence: f32,
    args: Vec<(String, String)>,
) -> Draft {
    Draft {
        s: Suggestion {
            title,
            formula: None,
            explanation,
            preview: None,
            confidence,
            action: Some(id.to_string()),
            kind: SuggestionKind::Command,
            action_args: args,
        },
    }
}

fn letter(reg: &Region, idx: usize) -> String {
    reg.columns
        .get(idx)
        .map(|c| col_to_letters(c.col))
        .unwrap_or_default()
}

fn default_target(reg: &Region) -> Option<usize> {
    let numeric = |c: &crate::region::Column| c.ty == ColumnType::Number;
    for pref in DEFAULT_TARGETS {
        if let Some(i) = reg.columns.iter().position(|c| {
            numeric(c)
                && c.header
                    .as_deref()
                    .is_some_and(|h| value_words(h).iter().any(|w| w == pref))
        }) {
            return Some(i);
        }
    }
    reg.columns.iter().position(numeric)
}

fn fullest_column(reg: &Region) -> Option<usize> {
    let mut best: Option<(usize, usize)> = None;
    for (i, c) in reg.columns.iter().enumerate() {
        if best.is_none_or(|(_, f)| c.filled > f) {
            best = Some((i, c.filled));
        }
    }
    best.filter(|(_, f)| *f > 0).map(|(i, _)| i)
}

/// Generates suggestions for a request. `resolver` validates formulas.
pub fn suggest_rules(req: &AssistRequest, resolver: &SimpleResolver) -> Vec<Suggestion> {
    let q = req.query.trim();
    if q.is_empty() || q.len() > MAX_QUERY_LEN {
        return Vec::new();
    }
    let Some(range) = detect_range(req.view, req.active, req.selection) else {
        return Vec::new();
    };
    let reg = analyze(req.view, range);
    if reg.columns.is_empty() || reg.range.rows() < 1 {
        return Vec::new();
    }
    let an = Analysis::new(&reg, tokenize(q));
    if an.toks.is_empty() {
        return Vec::new();
    }
    let n = an.toks.len();
    let mut consumed = vec![false; n];

    // Category value conditions.
    let hits = an.find_values();
    let mut conds: Vec<Cond> = Vec::new();
    for h in &hits {
        consumed[h.lo..h.hi].iter_mut().for_each(|c| *c = true);
        // A directly preceding negation word is consumed too.
        if h.negate && h.lo > 0 && an.negated_before(h.lo) {
            consumed[h.lo - 1] = true;
        }
        conds.push(Cond {
            col: Some(h.col),
            cmp: if h.negate { Cmp::Ne } else { Cmp::Eq },
            operand: Operand::Text(h.value.clone()),
            fuzzy: h.fuzzy,
            pos: h.lo,
        });
    }

    // Numeric conditions.
    let mut cond_col_tokens: Vec<usize> = Vec::new();
    let mut i = 0;
    while i < n {
        if consumed[i] {
            i += 1;
            continue;
        }
        let num_at = |k: usize| an.toks.get(k).and_then(|t| parse_number(&t.text));
        if an.toks[i].text == "between" {
            if let (Some(a), Some("and"), Some(b)) = (
                num_at(i + 1),
                an.toks.get(i + 2).map(|t| t.text.as_str()),
                num_at(i + 3),
            ) {
                let col = an.col_before(i, &consumed);
                if let Some((_, tj)) = col {
                    cond_col_tokens.push(tj);
                    consumed[tj] = true;
                }
                let colidx = col.map(|(c, _)| c);
                for (cmp, v) in [(Cmp::Ge, a.min(b)), (Cmp::Le, a.max(b))] {
                    conds.push(Cond {
                        col: colidx,
                        cmp,
                        operand: Operand::Num(v),
                        fuzzy: false,
                        pos: i,
                    });
                }
                consumed[i..(i + 4).min(n)]
                    .iter_mut()
                    .for_each(|c| *c = true);
                i += 4;
                continue;
            }
        }
        if let Some((cmp, len)) = parse_comparator(&an.toks, i) {
            if let Some(v) = num_at(i + len) {
                let col = an.col_before(i, &consumed);
                let explicit_needed = matches!(cmp, Cmp::Eq | Cmp::Ne);
                let col_ok = match col {
                    Some((c, _)) => reg.columns[c].ty == ColumnType::Number,
                    None => !explicit_needed,
                };
                if col_ok {
                    if let Some((_, tj)) = col {
                        cond_col_tokens.push(tj);
                        consumed[tj] = true;
                    }
                    conds.push(Cond {
                        col: col.map(|(c, _)| c),
                        cmp,
                        operand: Operand::Num(v),
                        fuzzy: false,
                        pos: i,
                    });
                    consumed[i..i + len + 1].iter_mut().for_each(|c| *c = true);
                    i += len + 1;
                    continue;
                }
            }
        }
        i += 1;
    }

    // "<text column> is <literal>" when the literal is not a known category.
    for j in 0..n.saturating_sub(2) {
        if consumed[j] {
            continue;
        }
        let Some((ci, _)) = an.tok_col[j] else {
            continue;
        };
        if !matches!(reg.columns[ci].ty, ColumnType::Category | ColumnType::Text)
            || conds.iter().any(|c| c.col == Some(ci))
        {
            continue;
        }
        let mut k = j + 1;
        if !matches!(an.toks[k].text.as_str(), "is" | "=" | "equals" | "<>") {
            continue;
        }
        let mut cmp = if an.toks[k].text == "<>" {
            Cmp::Ne
        } else {
            Cmp::Eq
        };
        k += 1;
        if an.toks.get(k).is_some_and(|t| t.text == "not") {
            cmp = Cmp::Ne;
            k += 1;
        }
        if let Some(t) = an.toks.get(k) {
            if !is_stopword(&t.text) && parse_number(&t.text).is_none() && !consumed[k] {
                consumed[j] = true;
                consumed[k] = true;
                conds.push(Cond {
                    col: Some(ci),
                    cmp,
                    operand: Operand::Text(t.orig.clone()),
                    fuzzy: true,
                    pos: j,
                });
            }
        }
    }
    conds.sort_by_key(|c| c.pos);

    // Month conditions are not supported in Phase A.
    let has_date = reg.columns.iter().any(|c| c.ty == ColumnType::Date);
    let month_unsupported = has_date
        && an.toks.iter().enumerate().any(|(i, t)| {
            i > 0
                && month_number(&t.text).is_some()
                && matches!(an.toks[i - 1].text.as_str(), "in" | "during" | "for" | "of")
        });

    // Command words.
    let wants_chart = is_cmd(
        &an.sing,
        &["chart", "graph", "plot", "visualize", "visualise"],
    );
    let wants_format = is_cmd(&an.sing, &["format"]);
    let wants_filter = is_cmd(&an.sing, &["filter", "show", "only", "hide"]);
    let any_command = wants_chart || wants_format || wants_filter;

    let skip = |i: usize| consumed[i];
    let agg = find_aggregate(&an.toks, &skip);
    let agg_range = agg.map(|a| a.start..a.start + a.len);

    // Header candidates not used by conditions or the aggregate keyword.
    let header_cands: Vec<(usize, usize, f64)> = an
        .tok_col
        .iter()
        .enumerate()
        .filter_map(|(j, m)| {
            let (c, s) = (*m)?;
            let in_agg = agg_range.as_ref().is_some_and(|r| r.contains(&j));
            (!consumed[j] && !in_agg && !cond_col_tokens.contains(&j)).then_some((j, c, s))
        })
        .collect();
    let best_header = |pred: &dyn Fn(ColumnType) -> bool| -> Option<(usize, f64)> {
        header_cands
            .iter()
            .filter(|(_, c, _)| pred(reg.columns[*c].ty))
            .fold(None, |acc: Option<(usize, f64)>, (_, c, s)| match acc {
                Some((_, bs)) if bs >= *s => acc,
                _ => Some((*c, *s)),
            })
    };

    let mut drafts: Vec<Draft> = Vec::new();

    // ---- Formula suggestion ----
    let strong_agg = agg.filter(|a| !a.weak);
    let formula_agg = if any_command { strong_agg } else { agg };
    let implicit_count = agg.is_none() && !any_command && !conds.is_empty();
    if let Some(a) = formula_agg
        .map(|a| (a.agg, a.fuzzy))
        .or(implicit_count.then_some((Agg::Count, false)))
    {
        let (agg_kind, agg_fuzzy) = a;
        if let Some(d) = build_formula(
            req,
            resolver,
            &reg,
            agg_kind,
            agg_fuzzy,
            implicit_count,
            conds.clone(),
            &an,
            agg.map(|a| a.start + a.len),
            &cond_col_tokens,
            &best_header,
            month_unsupported,
        ) {
            drafts.push(d);
        }
    }

    // ---- Command suggestions ----
    let first_cond = conds.iter().find(|c| c.col.is_some() || c.cmp != Cmp::Eq);
    if wants_filter {
        if let Some(c) = first_cond.filter(|c| c.col.is_some()) {
            drafts.push(filter_draft(&reg, c, if agg.is_some() { 0.4 } else { 0.9 }));
        }
    } else if agg.is_some() || implicit_count {
        if let Some(c) = conds.iter().find(|c| {
            c.col
                .is_some_and(|i| reg.columns[i].ty == ColumnType::Category)
        }) {
            drafts.push(filter_draft(&reg, c, 0.4));
        }
    }

    let group_from_query = group_column(&an, &reg);
    if wants_chart {
        let by = group_from_query.or_else(|| {
            reg.columns
                .iter()
                .position(|c| c.ty == ColumnType::Category)
        });
        let val = best_header(&|t| t == ColumnType::Number)
            .map(|(c, _)| c)
            .filter(|c| Some(*c) != by)
            .or_else(|| default_target(&reg));
        if let (Some(v), Some(b)) = (val, by) {
            drafts.push(chart_draft(&reg, v, b, 0.9));
        }
    }
    if wants_format {
        let target = best_header(&|t| t == ColumnType::Number).map(|(c, _)| c);
        let explicit = target.is_some();
        if let Some(t) = target.or_else(|| default_target(&reg)) {
            let l = reg.label(t);
            drafts.push(command(
                "format.currency",
                format!("Format {l} as currency"),
                format!("Show {l} values as currency."),
                if explicit { 0.9 } else { 0.55 },
                vec![("column".into(), letter(&reg, t))],
            ));
        }
    }

    // Companion actions for aggregate formulas.
    if let (Some(d), false) = (drafts.first(), wants_chart || wants_format) {
        if d.s.kind == SuggestionKind::InsertFormula && agg.is_some_and(|a| a.agg != Agg::Count) {
            let target = formula_target(
                &reg,
                &best_header,
                &conds,
                &an,
                agg.map(|a| a.start + a.len),
                &cond_col_tokens,
            )
            .map(|(c, _)| c);
            if let Some(t) = target {
                let by = conds
                    .iter()
                    .filter_map(|c| c.col)
                    .find(|i| reg.columns[*i].ty == ColumnType::Category)
                    .or_else(|| {
                        reg.columns
                            .iter()
                            .position(|c| c.ty == ColumnType::Category)
                    });
                if let Some(b) = by {
                    drafts.push(chart_draft(&reg, t, b, 0.35));
                }
                if header_is_money(&reg, t) {
                    let l = reg.label(t);
                    drafts.push(command(
                        "format.currency",
                        format!("Format {l} as currency"),
                        format!("Show {l} values as currency."),
                        0.3,
                        vec![("column".into(), letter(&reg, t))],
                    ));
                }
            }
        }
    }

    // Rank, dedupe, cap.
    let mut out: Vec<Suggestion> = Vec::new();
    for d in drafts {
        let dup = out
            .iter()
            .any(|o| o.formula == d.s.formula && o.action == d.s.action && o.title == d.s.title);
        if !dup {
            out.push(d.s);
        }
    }
    out.sort_by(|a, b| b.confidence.total_cmp(&a.confidence));
    out.truncate(MAX_SUGGESTIONS);
    out
}

fn filter_draft(reg: &Region, c: &Cond, confidence: f32) -> Draft {
    let Some(ci) = c.col else {
        return command(
            "filter.byColumn",
            String::new(),
            String::new(),
            0.0,
            Vec::new(),
        );
    };
    let label = reg.label(ci);
    let val = operand_text(c);
    let title = match (c.cmp, &c.operand) {
        (Cmp::Eq, Operand::Text(_)) => format!("Show only {val} rows"),
        _ => format!("Show only rows where {}", cond_text(reg, c)),
    };
    command(
        "filter.byColumn",
        title,
        format!("Filter the table by {label}."),
        confidence,
        vec![
            ("column".into(), letter(reg, ci)),
            (
                "op".into(),
                if c.cmp == Cmp::Eq {
                    "=".into()
                } else {
                    c.cmp.symbol().into()
                },
            ),
            ("value".into(), val),
        ],
    )
}

fn chart_draft(reg: &Region, value: usize, by: usize, confidence: f32) -> Draft {
    let (v, b) = (reg.label(value), reg.label(by));
    command(
        "chart.byColumn",
        format!("Chart {v} by {b}"),
        format!("Chart {v} grouped by {b}."),
        confidence,
        vec![
            ("value".into(), letter(reg, value)),
            ("by".into(), letter(reg, by)),
        ],
    )
}

/// The column named after "by"/"per"/"vs".
fn group_column(an: &Analysis, reg: &Region) -> Option<usize> {
    (0..an.toks.len()).find_map(|j| {
        if !matches!(an.toks[j].text.as_str(), "by" | "per" | "vs" | "versus") {
            return None;
        }
        (j + 1..(j + 3).min(an.toks.len()))
            .find_map(|k| an.tok_col[k])
            .map(|(c, _)| c)
            .filter(|c| reg.columns[*c].ty != ColumnType::Number)
    })
}

/// Target column for a non-count aggregate.
fn formula_target(
    reg: &Region,
    best_header: &HeaderFinder,
    conds: &[Cond],
    an: &Analysis,
    agg_end: Option<usize>,
    cond_col_tokens: &[usize],
) -> Option<(usize, bool)> {
    if let Some((c, _)) = best_header(&|t| matches!(t, ColumnType::Number)) {
        return Some((c, true));
    }
    // "sum of qty over 10": the operand column is the condition column.
    if let Some(end) = agg_end {
        if let Some(&tc) = cond_col_tokens.iter().min() {
            let adjacent = tc >= end
                && an.toks[end..tc]
                    .iter()
                    .all(|t| matches!(t.text.as_str(), "of" | "the" | "all" | "a"));
            if adjacent {
                if let Some((c, _)) = an.tok_col[tc] {
                    if reg.columns[c].ty == ColumnType::Number {
                        let _ = conds;
                        return Some((c, true));
                    }
                }
            }
        }
    }
    default_target(reg).map(|c| (c, false))
}

#[allow(clippy::too_many_arguments)]
fn build_formula(
    req: &AssistRequest,
    resolver: &SimpleResolver,
    reg: &Region,
    agg: Agg,
    agg_fuzzy: bool,
    implicit: bool,
    mut conds: Vec<Cond>,
    an: &Analysis,
    agg_end: Option<usize>,
    cond_col_tokens: &[usize],
    best_header: &HeaderFinder,
    month_unsupported: bool,
) -> Option<Draft> {
    let mut confidence: f32 = 0.9;
    let mut notes: Vec<String> = Vec::new();
    if agg_fuzzy {
        confidence -= 0.1;
    }
    if implicit {
        confidence = 0.55;
    }
    if conds.iter().any(|c| c.fuzzy) {
        confidence -= 0.05;
    }

    // Resolve the target column.
    let explicit_any = best_header(&|t| t != ColumnType::Empty).map(|(c, _)| c);
    let explicit_num = best_header(&|t| t == ColumnType::Number).map(|(c, _)| c);
    let mut named = true;
    let target: Option<usize> = match agg {
        Agg::Count => {
            if conds.is_empty() {
                explicit_any.or_else(|| fullest_column(reg))
            } else {
                None
            }
        }
        _ => {
            let from_header = if matches!(agg, Agg::Max | Agg::Min) {
                explicit_num.or_else(|| best_header(&|t| t == ColumnType::Date).map(|(c, _)| c))
            } else {
                explicit_num
            };
            match from_header {
                Some(c) => Some(c),
                None => formula_target(reg, best_header, &conds, an, agg_end, cond_col_tokens).map(
                    |(c, n)| {
                        named = n;
                        c
                    },
                ),
            }
        }
    };
    if !named {
        confidence -= 0.05;
    }
    if agg != Agg::Count {
        let t = target?;
        if !matches!(reg.columns[t].ty, ColumnType::Number | ColumnType::Date) {
            return None;
        }
    }

    // Conditions without a named column apply to the target column.
    let mut dropped = false;
    conds.retain_mut(|c| {
        if c.col.is_none() {
            match target {
                Some(t) if reg.columns[t].ty == ColumnType::Number => c.col = Some(t),
                _ => {
                    dropped = true;
                    return false;
                }
            }
        }
        true
    });
    if dropped {
        confidence -= 0.3;
        notes.push("a condition without a clear column was ignored".into());
    }
    if month_unsupported {
        confidence -= 0.25;
        notes.push("month filters are not supported yet, so no date condition was applied".into());
    }

    let cond_texts: Vec<String> = conds.iter().map(|c| cond_text(reg, c)).collect();
    let target_label = target.map(|t| reg.label(t));
    let rng = |i: usize| reg.body_a1(i);

    let formula = match agg {
        Agg::Count => {
            if conds.is_empty() {
                let t = target?;
                let f = if reg.columns[t].ty == ColumnType::Number {
                    "COUNT"
                } else {
                    "COUNTA"
                };
                if target_label.is_none() || explicit_any.is_none() {
                    confidence -= 0.1;
                }
                format!("={f}({})", rng(t)?)
            } else if conds.len() == 1 {
                let c = &conds[0];
                format!("=COUNTIF({},{})", rng(c.col?)?, criteria(c))
            } else {
                let mut parts = Vec::new();
                for c in &conds {
                    parts.push(format!("{},{}", rng(c.col?)?, criteria(c)));
                }
                format!("=COUNTIFS({})", parts.join(","))
            }
        }
        Agg::Sum | Agg::Average => {
            let t = target?;
            let (plain, one, many) = if agg == Agg::Sum {
                ("SUM", "SUMIF", "SUMIFS")
            } else {
                ("AVERAGE", "AVERAGEIF", "AVERAGEIFS")
            };
            match conds.len() {
                0 => format!("={plain}({})", rng(t)?),
                1 => {
                    let c = &conds[0];
                    let cc = c.col?;
                    if cc == t {
                        format!("={one}({},{})", rng(cc)?, criteria(c))
                    } else {
                        format!("={one}({},{},{})", rng(cc)?, criteria(c), rng(t)?)
                    }
                }
                _ => {
                    let mut s = format!("={many}({}", rng(t)?);
                    for c in &conds {
                        s.push_str(&format!(",{},{}", rng(c.col?)?, criteria(c)));
                    }
                    s.push(')');
                    s
                }
            }
        }
        Agg::Max | Agg::Min | Agg::Median => {
            let t = target?;
            if !conds.is_empty() {
                confidence -= 0.45;
                notes.push("the condition was not applied (conditional max/min/median is not supported yet)".into());
            }
            let f = match agg {
                Agg::Max => "MAX",
                Agg::Min => "MIN",
                _ => "MEDIAN",
            };
            format!("={f}({})", rng(t)?)
        }
    };

    // Validate by parsing.
    if parse(&formula, req.active, Dialect::calc_a1_en_us(), resolver).is_err() {
        return None;
    }

    let applied: &[String] = if matches!(agg, Agg::Max | Agg::Min | Agg::Median) {
        &[]
    } else {
        &cond_texts
    };
    let mut explanation = explain(agg, target_label.as_deref(), applied);
    if !notes.is_empty() {
        explanation.push_str(&format!(" ({})", notes.join("; ")));
    }
    let title = explain(agg, target_label.as_deref(), applied);
    explanation.push_str(&format!(
        ". Uses rows {} to {} of the data in {}.",
        reg.body_first_row() + 1,
        reg.range.r1 + 1,
        range_a1(reg)
    ));

    let money = agg != Agg::Count && target.is_some_and(|t| header_is_money(reg, t));
    let preview = req
        .preview
        .and_then(|f| f(&formula))
        .and_then(|v| format_preview(&v, money));

    Some(Draft {
        s: Suggestion {
            title,
            formula: Some(formula),
            explanation,
            preview,
            confidence: confidence.clamp(0.05, 0.99),
            action: None,
            kind: SuggestionKind::InsertFormula,
            action_args: Vec::new(),
        },
    })
}

fn range_a1(reg: &Region) -> String {
    let r = reg.range;
    format!(
        "{}{}:{}{}",
        col_to_letters(r.c0),
        u64::from(r.r0) + 1,
        col_to_letters(r.c1),
        u64::from(r.r1) + 1
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_formatting() {
        let f = |x: f64, m: bool| format_preview(&Value::Number(x), m).unwrap_or_default();
        assert_eq!(f(12480.0, true), "$12,480");
        assert_eq!(f(1234567.891, false), "1,234,567.89");
        assert_eq!(f(0.5, false), "0.5");
        assert_eq!(f(-1500.25, true), "-$1,500.25");
        assert_eq!(f(999.0, false), "999");
        assert_eq!(f(0.0, false), "0");
        assert_eq!(f(-0.001, false), "0");
        assert_eq!(format_preview(&Value::Empty, false), None);
    }

    #[test]
    fn explain_texts() {
        assert_eq!(
            explain(Agg::Sum, Some("Total"), &["Status is not Paid".into()]),
            "Sum of Total where Status is not Paid"
        );
        assert_eq!(
            explain(Agg::Average, Some("Price"), &[]),
            "Average of Price"
        );
        assert_eq!(
            explain(Agg::Count, None, &["Status is Overdue".into()]),
            "Count of rows where Status is Overdue"
        );
    }

    #[test]
    fn criteria_quoting() {
        let c = |cmp, operand| Cond {
            col: Some(0),
            cmp,
            operand,
            fuzzy: false,
            pos: 0,
        };
        assert_eq!(
            criteria(&c(Cmp::Ne, Operand::Text("Paid".into()))),
            "\"<>Paid\""
        );
        assert_eq!(
            criteria(&c(Cmp::Eq, Operand::Text("A\"B".into()))),
            "\"A\"\"B\""
        );
        assert_eq!(criteria(&c(Cmp::Gt, Operand::Num(10.0))), "\">10\"");
        assert_eq!(criteria(&c(Cmp::Eq, Operand::Num(2.5))), "2.5");
    }
}
