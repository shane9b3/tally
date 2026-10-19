//! FIND and SEARCH.
//!
//! Positions are 1-based counts of Unicode scalar values. A start position
//! below 1 or past the end of the text, and a missing match, are `#VALUE!`
//! (AOO `ScInterpreter::ScFind` / `ScSearch`).

use regex::RegexBuilder;
use tally_types::{CriteriaMode, ErrorKind, Value};

use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry, ReturnKind};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const PARAMS: &[Param] = &[
    Param::required("find_text", ParamKind::Text),
    Param::required("text", ParamKind::Text),
    Param::optional("position", ParamKind::Integer),
];

/// `FIND(find_text; text [; position])`: case-sensitive, no wildcards.
pub const FIND: FunctionDef = FunctionDef::new("FIND", Category::Text, PARAMS, find)
    .returns(ReturnKind::Number)
    .help("Finds a text within another, case-sensitively.");
/// `SEARCH(find_text; text [; position])`: case-insensitive, wildcards or
/// regular expressions per the document's criteria mode.
pub const SEARCH: FunctionDef = FunctionDef::new("SEARCH", Category::Text, PARAMS, search)
    .returns(ReturnKind::Number)
    .help("Finds a text within another, ignoring case.");

/// Validates the start position and returns it as a 0-based char index
/// together with its byte offset in `text`.
fn start_offset(text: &str, pos: i64) -> Result<(usize, usize), ErrorKind> {
    let len = text.chars().count();
    let pos = usize::try_from(pos).map_err(|_| ErrorKind::Value)?;
    if pos < 1 || pos > len.max(1) {
        return Err(ErrorKind::Value);
    }
    let byte = text
        .char_indices()
        .nth(pos - 1)
        .map_or(text.len(), |(b, _)| b);
    Ok((pos - 1, byte))
}

fn find(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let needle = a.text(0)?;
        let text = a.text(1)?;
        let pos = a.int_opt(2, 1)?;
        let (char_start, byte_start) = start_offset(&text, pos)?;
        let rel = text[byte_start..].find(&*needle).ok_or(ErrorKind::Value)?;
        let chars_before = text[byte_start..byte_start + rel].chars().count();
        Ok(Value::Number((char_start + chars_before + 1) as f64))
    })
}

/// Translates a `*` / `?` / `~` wildcard pattern to a regular expression.
fn wildcard_to_regex(p: &str) -> String {
    let mut out = String::new();
    let mut it = p.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '*' => out.push_str(".*"),
            '?' => out.push('.'),
            '~' => match it.peek() {
                Some(&n) if matches!(n, '*' | '?' | '~') => {
                    out.push_str(&regex::escape(&n.to_string()));
                    it.next();
                }
                _ => out.push_str(&regex::escape("~")),
            },
            other => out.push_str(&regex::escape(&other.to_string())),
        }
    }
    out
}

fn search(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let needle = a.text(0)?;
        let text = a.text(1)?;
        let pos = a.int_opt(2, 1)?;
        let (char_start, byte_start) = start_offset(&text, pos)?;
        let pattern = match a.ctx().settings().criteria_mode {
            CriteriaMode::Wildcards => wildcard_to_regex(&needle),
            CriteriaMode::Regex => needle.to_string(),
            CriteriaMode::Literal => regex::escape(&needle),
        };
        let re = RegexBuilder::new(&pattern)
            .case_insensitive(true)
            .dot_matches_new_line(true)
            .size_limit(1 << 20)
            .dfa_size_limit(1 << 20)
            .build()
            .map_err(|_| ErrorKind::Value)?;
        let m = re.find_at(&text, byte_start).ok_or(ErrorKind::Value)?;
        let chars_before = text[byte_start..m.start()].chars().count();
        Ok(Value::Number((char_start + chars_before + 1) as f64))
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(FIND);
    r.register(SEARCH);
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{CriteriaMode, ErrorKind, Value};

    fn s(x: &str) -> EvalValue {
        EvalValue::text(x)
    }
    fn n(x: f64) -> EvalValue {
        EvalValue::number(x)
    }
    fn t(name: &str, args: Vec<EvalValue>) -> Value {
        call(&mut MockEvalContext::new(), name, args)
    }
    const NV: Value = Value::Error(ErrorKind::Value);

    #[test]
    fn find() {
        assert_eq!(t("FIND", vec![s("l"), s("hello")]), Value::Number(3.0));
        assert_eq!(
            t("FIND", vec![s("l"), s("hello"), n(4.0)]),
            Value::Number(4.0)
        );
        assert_eq!(t("FIND", vec![s("L"), s("hello")]), NV);
        assert_eq!(t("FIND", vec![s("z"), s("hello")]), NV);
        assert_eq!(t("FIND", vec![s("*"), s("a*b")]), Value::Number(2.0));
        assert_eq!(t("FIND", vec![s(""), s("abc")]), Value::Number(1.0));
        assert_eq!(t("FIND", vec![s("l"), s("hello"), n(0.0)]), NV);
        assert_eq!(t("FIND", vec![s("l"), s("hello"), n(6.0)]), NV);
        assert_eq!(t("FIND", vec![s("b"), s("é😀b")]), Value::Number(3.0));
        assert_eq!(t("FIND", vec![n(2.0), s("a12")]), Value::Number(3.0));
    }

    #[test]
    fn search() {
        assert_eq!(t("SEARCH", vec![s("L"), s("hello")]), Value::Number(3.0));
        assert_eq!(t("SEARCH", vec![s("h?o"), s("ahello")]), NV);
        assert_eq!(t("SEARCH", vec![s("e?l"), s("hello")]), Value::Number(2.0));
        assert_eq!(t("SEARCH", vec![s("l*o"), s("hello")]), Value::Number(3.0));
        assert_eq!(t("SEARCH", vec![s("~*"), s("a*b")]), Value::Number(2.0));
        assert_eq!(
            t("SEARCH", vec![s("a.c"), s("abc a.c")]),
            Value::Number(5.0)
        );
        assert_eq!(
            t("SEARCH", vec![s("l"), s("hello"), n(4.0)]),
            Value::Number(4.0)
        );
        assert_eq!(t("SEARCH", vec![s("q"), s("hello")]), NV);
    }

    #[test]
    fn search_modes() {
        let mut c = MockEvalContext::new();
        c.settings.criteria_mode = CriteriaMode::Regex;
        let v = call(&mut c, "SEARCH", vec![s("l+o"), s("hello")]);
        assert_eq!(v, Value::Number(3.0));
        assert_eq!(call(&mut c, "SEARCH", vec![s("("), s("hello")]), NV);
        c.settings.criteria_mode = CriteriaMode::Literal;
        assert_eq!(
            call(&mut c, "SEARCH", vec![s("*"), s("a*b")]),
            Value::Number(2.0)
        );
    }

    #[test]
    fn errors() {
        assert_eq!(
            t("FIND", vec![EvalValue::error(ErrorKind::NA), s("a")]),
            Value::Error(ErrorKind::NA)
        );
        assert_eq!(
            t("SEARCH", vec![s("a")]),
            Value::Error(ErrorKind::MissingVariable2)
        );
    }
}
