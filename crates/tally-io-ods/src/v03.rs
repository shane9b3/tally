//! v0.3 ODF features: conditional formats (`calcext:conditional-formats`
//! and the legacy `style:map`), content validations, annotations,
//! protection, page layouts / master pages and document metadata helpers.
//!
//! Condition strings follow LibreOffice's export
//! (`sc/source/filter/xml/xmlexprt.cxx`, `XMLStylesExportHelper.cxx`):
//! expressions are OpenFormula without the `of:` prefix (`[.A1]>0`),
//! relative to `base-cell-address`.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::BufRead;

use tally_model::{
    CellRangeLocal, CfColorStopData, CfDatePeriodData, CfKindData, CfRuleData, CfStyleData,
    CfTextOpData, CfThresholdData, CfValueKindData, CmpOpData, ConditionalFormatData,
    HeaderFooterData, HfAreaData, IconSetData, ListSourceData, MarginsData, NoteData,
    OrientationData, PageOrderData, PageScaleData, PageSetupData, PaperData, SheetProtectionData,
    ValidationData, ValidationErrorData, ValidationErrorStyleData, ValidationKindData,
    ValidationMessageData,
};
use tally_types::{col_to_letters, MAX_COLS, MAX_ROWS};

use crate::units::{color_hex, hmm_to_cm, length_to_hmm, parse_color};
use crate::xml::{escape_attr, escape_text, Elem, Ev, XmlReader};
use crate::OdsError;

/// Most conditional formats, validations or notes read per document part.
pub(crate) const MAX_ITEMS: usize = 100_000;
const MAX_TEXT: usize = 64 * 1024;
/// Most conditional formats per sheet (the engine's cap).
pub(crate) const MAX_CF_PER_SHEET: usize = 2_048;
/// Most ranges per conditional format.
pub(crate) const MAX_CF_RANGES: usize = 64;
/// Longest note text kept (the engine's cap; longer notes are truncated).
pub(crate) const MAX_NOTE_TEXT: usize = 32 * 1024;
/// Longest `table:protection-key` kept.
const MAX_PROTECTION_KEY: usize = 1024;

pub(crate) const URI_SHA256: &str = "http://www.w3.org/2000/09/xmldsig#sha256";
pub(crate) const URI_SHA1: &str = "http://www.w3.org/2000/09/xmldsig#sha1";
pub(crate) const URI_XLS_LEGACY: &str =
    "http://docs.oasis-open.org/office/ns/table/legacy-hash-excel";

// ---------------------------------------------------------------------------
// Small parsing helpers
// ---------------------------------------------------------------------------

/// Splits `s` at the first top-level `sep` (outside quotes, brackets and
/// parentheses); returns `(before, after)`.
pub(crate) fn split_top(s: &str, sep: char) -> Option<(&str, &str)> {
    let mut depth = 0i32;
    let mut quoted = false;
    let mut squoted = false;
    for (i, c) in s.char_indices() {
        match c {
            '"' if !squoted => quoted = !quoted,
            '\'' if !quoted => squoted = !squoted,
            '(' | '[' | '{' if !quoted && !squoted => depth += 1,
            ')' | ']' | '}' if !quoted && !squoted => depth -= 1,
            c if c == sep && depth == 0 && !quoted && !squoted => {
                return Some((&s[..i], &s[i + c.len_utf8()..]));
            }
            _ => {}
        }
    }
    None
}

/// The argument text of `name(…)` at the start of `s` (balanced).
pub(crate) fn call_args<'a>(s: &'a str, name: &str) -> Option<&'a str> {
    let rest = s.trim().strip_prefix(name)?.trim_start();
    let inner = rest.strip_prefix('(')?;
    // Find the matching close parenthesis.
    let mut depth = 1i32;
    let mut quoted = false;
    for (i, c) in inner.char_indices() {
        match c {
            '"' => quoted = !quoted,
            '(' if !quoted => depth += 1,
            ')' if !quoted => {
                depth -= 1;
                if depth == 0 {
                    return Some(&inner[..i]);
                }
            }
            _ => {}
        }
    }
    None
}

/// An ODF expression as model OpenFormula (`of:=…`).
pub(crate) fn of(expr: &str) -> String {
    let e = expr.trim();
    let e = e.strip_prefix("of:").unwrap_or(e);
    format!("of:={}", e.strip_prefix('=').unwrap_or(e))
}

/// A model OpenFormula as an ODF expression (no `of:=`).
pub(crate) fn expr(of: &str) -> &str {
    let t = of.trim();
    let t = t
        .strip_prefix("of:")
        .or_else(|| t.strip_prefix("oooc:"))
        .unwrap_or(t);
    t.strip_prefix('=').unwrap_or(t)
}

/// `Sheet1.A1` / `$Sheet1.$A$1` → `(row, col)` (the sheet is ignored).
pub(crate) fn parse_base(s: &str) -> Option<(u32, u32)> {
    let cell = match s.rfind('.') {
        Some(i) => &s[i + 1..],
        None => s,
    };
    let t: String = cell.chars().filter(|c| *c != '$').collect();
    let split = t.find(|c: char| c.is_ascii_digit())?;
    let (letters, digits) = t.split_at(split);
    let col = tally_types::letters_to_col(letters)?;
    let row: u32 = digits.parse().ok()?;
    ((1..=MAX_ROWS).contains(&row) && col < MAX_COLS).then(|| (row - 1, col))
}

/// `Sheet.A1` for a base cell.
pub(crate) fn base_address(sheet: &str, row: u32, col: u32) -> String {
    format!("{sheet}.{}{}", col_to_letters(col), u64::from(row) + 1)
}

/// A space-separated `table:target-range-address` list → ranges.
pub(crate) fn parse_range_list(s: &str) -> Vec<CellRangeLocal> {
    let mut out = Vec::new();
    let mut quoted = false;
    let mut cur = String::new();
    let flush = |cur: &mut String, out: &mut Vec<CellRangeLocal>| {
        if cur.is_empty() {
            return;
        }
        let (a, b) = split_top(cur, ':').unwrap_or((cur.as_str(), cur.as_str()));
        if let (Some((r0, c0)), Some((r1, c1))) = (parse_base(a), parse_base(b)) {
            out.push(CellRangeLocal {
                r0: r0.min(r1),
                c0: c0.min(c1),
                r1: r0.max(r1),
                c1: c0.max(c1),
            });
        }
        cur.clear();
    };
    for ch in s.trim().chars() {
        if ch == '\'' {
            quoted = !quoted;
        }
        if ch == ' ' && !quoted {
            flush(&mut cur, &mut out);
        } else {
            cur.push(ch);
        }
    }
    flush(&mut cur, &mut out);
    out.truncate(1024);
    out
}

/// Ranges as a `target-range-address` list (`Sheet1.A1:Sheet1.B5 …`).
pub(crate) fn range_list(sheet: &str, ranges: &[CellRangeLocal]) -> String {
    ranges
        .iter()
        .map(|r| {
            format!(
                "{}:{}",
                base_address(sheet, r.r0, r.c0),
                base_address(sheet, r.r1, r.c1)
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn quote_text(t: &str) -> String {
    format!("\"{}\"", t.replace('"', "\"\""))
}

fn unquote_text(t: &str) -> String {
    let t = t.trim();
    match t.strip_prefix('"').and_then(|x| x.strip_suffix('"')) {
        Some(inner) => inner.replace("\"\"", "\""),
        None => t.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Conditional formats: calcext
// ---------------------------------------------------------------------------

fn cmp_token(op: CmpOpData) -> &'static str {
    match op {
        CmpOpData::Eq => "=",
        CmpOpData::Ne => "!=",
        CmpOpData::Gt => ">",
        CmpOpData::Ge => ">=",
        CmpOpData::Lt => "<",
        CmpOpData::Le => "<=",
        CmpOpData::Between => "between",
        CmpOpData::NotBetween => "not-between",
    }
}

/// The blank tests have no LibreOffice token: they are written as
/// formulas and recognised again on read.
fn blank_formula(base: (u32, u32), not: bool) -> String {
    format!(
        "LEN(TRIM([.{}{}])){}0",
        col_to_letters(base.1),
        u64::from(base.0) + 1,
        if not { ">" } else { "=" }
    )
}

/// The `calcext:value` of a style-applying rule (`None` for kinds that are
/// written as their own element).
pub(crate) fn condition_value(kind: &CfKindData, base: (u32, u32)) -> Option<String> {
    Some(match kind {
        CfKindData::CellValue { op, formulas } => {
            let a = formulas.first().map_or("", |f| expr(f));
            match op {
                CmpOpData::Between | CmpOpData::NotBetween => format!(
                    "{}({},{})",
                    cmp_token(*op),
                    a,
                    formulas.get(1).map_or("", |f| expr(f))
                ),
                _ => format!("{}{}", cmp_token(*op), a),
            }
        }
        CfKindData::Formula { formula } => format!("formula-is({})", expr(formula)),
        CfKindData::Duplicate => "duplicate".into(),
        CfKindData::Unique => "unique".into(),
        CfKindData::Top {
            rank,
            percent,
            bottom,
        } => format!(
            "{}-{}({rank})",
            if *bottom { "bottom" } else { "top" },
            if *percent { "percent" } else { "elements" }
        ),
        CfKindData::Average { above, equal } => match (above, equal) {
            (true, false) => "above-average".into(),
            (true, true) => "above-equal-average".into(),
            (false, false) => "below-average".into(),
            (false, true) => "below-equal-average".into(),
        },
        CfKindData::Text { op, text } => format!(
            "{}({})",
            match op {
                CfTextOpData::Contains => "contains-text",
                CfTextOpData::NotContains => "not-contains-text",
                CfTextOpData::BeginsWith => "begins-with",
                CfTextOpData::EndsWith => "ends-with",
            },
            quote_text(text)
        ),
        CfKindData::Blank => format!("formula-is({})", blank_formula(base, false)),
        CfKindData::NotBlank => format!("formula-is({})", blank_formula(base, true)),
        CfKindData::Error => "is-error".into(),
        CfKindData::NotError => "is-no-error".into(),
        CfKindData::ColorScale { .. }
        | CfKindData::DataBar { .. }
        | CfKindData::IconSet { .. }
        | CfKindData::DateIs { .. } => return None,
    })
}

/// Parses a `calcext:value` (LibreOffice `GetConditionData`).
pub(crate) fn parse_condition_value(v: &str, base: (u32, u32)) -> Option<CfKindData> {
    let v = v.trim();
    let args2 = |name: &str| -> Option<(String, String)> {
        let a = call_args(v, name)?;
        let (x, y) = split_top(a, ',')?;
        Some((of(x), of(y)))
    };
    // Longer tokens first (`not-between` before `between`, …).
    if let Some((a, b)) = args2("not-between") {
        return Some(CfKindData::CellValue {
            op: CmpOpData::NotBetween,
            formulas: vec![a, b],
        });
    }
    if let Some((a, b)) = args2("between") {
        return Some(CfKindData::CellValue {
            op: CmpOpData::Between,
            formulas: vec![a, b],
        });
    }
    if let Some(f) = call_args(v, "formula-is") {
        let f = f.trim();
        if f == blank_formula(base, false) {
            return Some(CfKindData::Blank);
        }
        if f == blank_formula(base, true) {
            return Some(CfKindData::NotBlank);
        }
        return Some(CfKindData::Formula { formula: of(f) });
    }
    for (name, bottom, percent) in [
        ("top-elements", false, false),
        ("bottom-elements", true, false),
        ("top-percent", false, true),
        ("bottom-percent", true, true),
    ] {
        if let Some(a) = call_args(v, name) {
            let rank = a
                .trim()
                .parse::<f64>()
                .ok()
                .map_or(10, |n| n.max(1.0) as u32);
            return Some(CfKindData::Top {
                rank,
                percent,
                bottom,
            });
        }
    }
    for (name, op) in [
        ("not-contains-text", CfTextOpData::NotContains),
        ("contains-text", CfTextOpData::Contains),
        ("begins-with", CfTextOpData::BeginsWith),
        ("ends-with", CfTextOpData::EndsWith),
    ] {
        if let Some(a) = call_args(v, name) {
            return Some(CfKindData::Text {
                op,
                text: unquote_text(a),
            });
        }
    }
    let simple = |op: CmpOpData, rest: &str| CfKindData::CellValue {
        op,
        formulas: vec![of(rest)],
    };
    Some(match v {
        "duplicate" => CfKindData::Duplicate,
        "unique" => CfKindData::Unique,
        "above-average" => CfKindData::Average {
            above: true,
            equal: false,
        },
        "above-equal-average" => CfKindData::Average {
            above: true,
            equal: true,
        },
        "below-average" => CfKindData::Average {
            above: false,
            equal: false,
        },
        "below-equal-average" => CfKindData::Average {
            above: false,
            equal: true,
        },
        "is-error" => CfKindData::Error,
        "is-no-error" => CfKindData::NotError,
        _ => {
            if let Some(r) = v.strip_prefix("<=") {
                simple(CmpOpData::Le, r)
            } else if let Some(r) = v.strip_prefix(">=") {
                simple(CmpOpData::Ge, r)
            } else if let Some(r) = v.strip_prefix("!=") {
                simple(CmpOpData::Ne, r)
            } else if let Some(r) = v.strip_prefix('<') {
                simple(CmpOpData::Lt, r)
            } else if let Some(r) = v.strip_prefix('=') {
                simple(CmpOpData::Eq, r)
            } else if let Some(r) = v.strip_prefix('>') {
                simple(CmpOpData::Gt, r)
            } else {
                return None;
            }
        }
    })
}

fn entry_type(t: &CfThresholdData, first: bool) -> &'static str {
    match t.kind {
        CfValueKindData::Min => "minimum",
        CfValueKindData::Max => "maximum",
        CfValueKindData::Percent => "percent",
        CfValueKindData::Percentile => "percentile",
        CfValueKindData::Number => "number",
        CfValueKindData::Formula => "formula",
        CfValueKindData::AutoMin => "auto-minimum",
        CfValueKindData::AutoMax if first => "auto-minimum",
        CfValueKindData::AutoMax => "auto-maximum",
    }
}

fn entry_value(t: &CfThresholdData) -> String {
    match t.kind {
        CfValueKindData::Min
        | CfValueKindData::Max
        | CfValueKindData::AutoMin
        | CfValueKindData::AutoMax => "0".into(),
        CfValueKindData::Formula => expr(&t.value).to_string(),
        _ if t.value.starts_with("of:") => expr(&t.value).to_string(),
        _ => {
            let v = t.value.trim();
            if v.is_empty() {
                "0".into()
            } else {
                v.to_string()
            }
        }
    }
}

fn parse_entry(e: &Elem) -> CfThresholdData {
    let value = e.attr("calcext:value").unwrap_or("0").to_string();
    let kind = match e.attr("calcext:type").unwrap_or("number") {
        "minimum" => CfValueKindData::Min,
        "maximum" => CfValueKindData::Max,
        "percent" => CfValueKindData::Percent,
        "percentile" => CfValueKindData::Percentile,
        "formula" => CfValueKindData::Formula,
        "auto-minimum" => CfValueKindData::AutoMin,
        "auto-maximum" => CfValueKindData::AutoMax,
        _ => CfValueKindData::Number,
    };
    let value = match kind {
        CfValueKindData::Min
        | CfValueKindData::Max
        | CfValueKindData::AutoMin
        | CfValueKindData::AutoMax => String::new(),
        CfValueKindData::Formula => of(&value),
        _ if value.trim().parse::<f64>().is_ok() => value.trim().to_string(),
        _ => of(&value),
    };
    CfThresholdData { kind, value }
}

fn period_token(p: CfDatePeriodData) -> &'static str {
    match p {
        CfDatePeriodData::Today => "today",
        CfDatePeriodData::Yesterday => "yesterday",
        CfDatePeriodData::Tomorrow => "tomorrow",
        CfDatePeriodData::Last7Days => "last-7-days",
        CfDatePeriodData::ThisWeek => "this-week",
        CfDatePeriodData::LastWeek => "last-week",
        CfDatePeriodData::NextWeek => "next-week",
        CfDatePeriodData::ThisMonth => "this-month",
        CfDatePeriodData::LastMonth => "last-month",
        CfDatePeriodData::NextMonth => "next-month",
        CfDatePeriodData::ThisYear => "this-year",
        CfDatePeriodData::LastYear => "last-year",
        CfDatePeriodData::NextYear => "next-year",
    }
}

fn period_from(s: &str) -> Option<CfDatePeriodData> {
    use CfDatePeriodData as P;
    [
        P::Today,
        P::Yesterday,
        P::Tomorrow,
        P::Last7Days,
        P::ThisWeek,
        P::LastWeek,
        P::NextWeek,
        P::ThisMonth,
        P::LastMonth,
        P::NextMonth,
        P::ThisYear,
        P::LastYear,
        P::NextYear,
    ]
    .into_iter()
    .find(|p| period_token(*p) == s)
}

/// Writes `calcext:conditional-formats` for a sheet. `style_name` gives
/// the named style of a rule's differential style.
pub(crate) fn write_cond_formats(
    s: &mut String,
    sheet: &str,
    cfs: &[ConditionalFormatData],
    style_name: &dyn Fn(&CfStyleData) -> String,
) {
    if cfs.is_empty() {
        return;
    }
    s.push_str("<calcext:conditional-formats>");
    for cf in cfs {
        if cf.ranges.is_empty() {
            continue;
        }
        s.push_str(r#"<calcext:conditional-format calcext:target-range-address=""#);
        escape_attr(&range_list(sheet, &cf.ranges), s);
        s.push_str("\">");
        let base = base_address(sheet, cf.base.0, cf.base.1);
        for rule in &cf.rules {
            match &rule.kind {
                CfKindData::ColorScale { stops } => {
                    s.push_str("<calcext:color-scale>");
                    for st in stops {
                        let _ = write!(s, r#"<calcext:color-scale-entry calcext:value=""#);
                        escape_attr(&entry_value(&st.threshold), s);
                        let _ = write!(
                            s,
                            r#"" calcext:type="{}" calcext:color="{}"/>"#,
                            entry_type(&st.threshold, true),
                            color_hex(st.color)
                        );
                    }
                    s.push_str("</calcext:color-scale>");
                }
                CfKindData::DataBar {
                    min,
                    max,
                    color,
                    negative_color,
                    show_value,
                    gradient,
                } => {
                    s.push_str("<calcext:data-bar");
                    if !gradient {
                        s.push_str(r#" calcext:gradient="false""#);
                    }
                    if !show_value {
                        s.push_str(r#" calcext:show-value="false""#);
                    }
                    if let Some(n) = negative_color {
                        let _ = write!(s, r#" calcext:negative-color="{}""#, color_hex(*n));
                    }
                    let _ = write!(
                        s,
                        r##" calcext:positive-color="{}" calcext:axis-color="#000000">"##,
                        color_hex(*color)
                    );
                    for (t, first) in [(min, true), (max, false)] {
                        s.push_str(r#"<calcext:formatting-entry calcext:value=""#);
                        escape_attr(&entry_value(t), s);
                        let _ = write!(s, r#"" calcext:type="{}"/>"#, entry_type(t, first));
                    }
                    s.push_str("</calcext:data-bar>");
                }
                CfKindData::IconSet {
                    set,
                    thresholds,
                    show_value,
                    reverse,
                } => {
                    let _ = write!(
                        s,
                        r#"<calcext:icon-set calcext:icon-set-type="{}""#,
                        set.file_name()
                    );
                    if !show_value {
                        s.push_str(r#" calcext:show-value="false""#);
                    }
                    if *reverse {
                        // Not a LibreOffice attribute; ignored by other readers.
                        s.push_str(r#" tally:reverse="true""#);
                    }
                    s.push('>');
                    for t in thresholds {
                        s.push_str(r#"<calcext:formatting-entry calcext:value=""#);
                        escape_attr(&entry_value(t), s);
                        let _ = write!(s, r#"" calcext:type="{}"/>"#, entry_type(t, true));
                    }
                    s.push_str("</calcext:icon-set>");
                }
                CfKindData::DateIs { period } => {
                    s.push_str(r#"<calcext:date-is calcext:style=""#);
                    escape_attr(&style_name(&rule.style), s);
                    let _ = write!(s, r#"" calcext:date="{}"/>"#, period_token(*period));
                }
                other => {
                    let Some(v) = condition_value(other, cf.base) else {
                        continue;
                    };
                    s.push_str(r#"<calcext:condition calcext:apply-style-name=""#);
                    escape_attr(&style_name(&rule.style), s);
                    s.push_str(r#"" calcext:value=""#);
                    escape_attr(&v, s);
                    s.push_str(r#"" calcext:base-cell-address=""#);
                    escape_attr(&base, s);
                    s.push_str("\"/>");
                }
            }
        }
        s.push_str("</calcext:conditional-format>");
    }
    s.push_str("</calcext:conditional-formats>");
}

/// Parses `calcext:conditional-formats`; `style` resolves a named style.
pub(crate) fn parse_cond_formats<R: BufRead>(
    xml: &mut XmlReader<R>,
    style: &mut dyn FnMut(&str) -> CfStyleData,
) -> Result<Vec<ConditionalFormatData>, OdsError> {
    let mut out: Vec<ConditionalFormatData> = Vec::new();
    let d = xml.depth();
    loop {
        match xml.next()? {
            Ev::Start(e) if e.name == "calcext:conditional-format" => {
                let mut ranges =
                    parse_range_list(e.attr("calcext:target-range-address").unwrap_or(""));
                ranges.truncate(MAX_CF_RANGES);
                let mut base: Option<(u32, u32)> = None;
                let mut rules: Vec<CfRuleData> = Vec::new();
                let dd = xml.depth();
                loop {
                    match xml.next()? {
                        Ev::Start(c) => {
                            let rule = parse_rule(xml, &c, &mut base, style)?;
                            if let Some(r) = rule {
                                if rules.len() < 64 {
                                    rules.push(r);
                                }
                            }
                        }
                        Ev::End if xml.depth() < dd => break,
                        Ev::Eof => break,
                        _ => {}
                    }
                }
                if !ranges.is_empty() && !rules.is_empty() && out.len() < MAX_CF_PER_SHEET {
                    let b = base.unwrap_or((ranges[0].r0, ranges[0].c0));
                    out.push(ConditionalFormatData {
                        ranges,
                        base: b,
                        rules,
                    });
                }
            }
            Ev::Start(_) => xml.skip()?,
            Ev::End if xml.depth() < d => return Ok(out),
            Ev::Eof => return Ok(out),
            _ => {}
        }
    }
}

fn entries<R: BufRead>(
    xml: &mut XmlReader<R>,
) -> Result<Vec<(CfThresholdData, Option<u32>)>, OdsError> {
    let mut v = Vec::new();
    let d = xml.depth();
    loop {
        match xml.next()? {
            Ev::Start(c) => {
                if matches!(
                    c.name.as_str(),
                    "calcext:formatting-entry" | "calcext:color-scale-entry"
                ) && v.len() < 8
                {
                    v.push((
                        parse_entry(&c),
                        c.attr("calcext:color").and_then(parse_color),
                    ));
                }
                xml.skip()?;
            }
            Ev::End if xml.depth() < d => return Ok(v),
            Ev::Eof => return Ok(v),
            _ => {}
        }
    }
}

fn parse_rule<R: BufRead>(
    xml: &mut XmlReader<R>,
    c: &Elem,
    base: &mut Option<(u32, u32)>,
    style: &mut dyn FnMut(&str) -> CfStyleData,
) -> Result<Option<CfRuleData>, OdsError> {
    let rule = |kind: CfKindData, st: CfStyleData| CfRuleData {
        kind,
        style: st,
        stop_if_true: false,
    };
    Ok(match c.name.as_str() {
        "calcext:condition" => {
            xml.skip()?;
            if base.is_none() {
                *base = c.attr("calcext:base-cell-address").and_then(parse_base);
            }
            let b = base.unwrap_or((0, 0));
            let st = style(c.attr("calcext:apply-style-name").unwrap_or(""));
            c.attr("calcext:value")
                .and_then(|v| parse_condition_value(v, b))
                .map(|k| rule(k, st))
        }
        "calcext:date-is" => {
            xml.skip()?;
            let st = style(c.attr("calcext:style").unwrap_or(""));
            c.attr("calcext:date")
                .and_then(period_from)
                .map(|period| rule(CfKindData::DateIs { period }, st))
        }
        "calcext:color-scale" => {
            let es = entries(xml)?;
            let stops: Vec<CfColorStopData> = es
                .into_iter()
                .map(|(t, col)| CfColorStopData {
                    threshold: t,
                    color: col.unwrap_or(0x0000_00ff),
                })
                .collect();
            (2..=3)
                .contains(&stops.len())
                .then(|| rule(CfKindData::ColorScale { stops }, CfStyleData::default()))
        }
        "calcext:data-bar" => {
            let gradient = c.attr("calcext:gradient") != Some("false");
            let show_value = c.attr("calcext:show-value") != Some("false");
            let color = c
                .attr("calcext:positive-color")
                .and_then(parse_color)
                .unwrap_or(0x2a60_99ff);
            let negative_color = c.attr("calcext:negative-color").and_then(parse_color);
            let es = entries(xml)?;
            (es.len() >= 2).then(|| {
                rule(
                    CfKindData::DataBar {
                        min: es[0].0.clone(),
                        max: es[1].0.clone(),
                        color,
                        negative_color,
                        show_value,
                        gradient,
                    },
                    CfStyleData::default(),
                )
            })
        }
        "calcext:icon-set" => {
            let set = c
                .attr("calcext:icon-set-type")
                .and_then(IconSetData::from_file_name);
            let show_value = c.attr("calcext:show-value") != Some("false");
            let reverse = c.attr("tally:reverse") == Some("true");
            let es = entries(xml)?;
            set.and_then(|set| {
                let thresholds: Vec<CfThresholdData> = es
                    .into_iter()
                    .map(|(t, _)| t)
                    .take(set.icon_count())
                    .collect();
                (thresholds.len() == set.icon_count()).then(|| {
                    rule(
                        CfKindData::IconSet {
                            set,
                            thresholds,
                            show_value,
                            reverse,
                        },
                        CfStyleData::default(),
                    )
                })
            })
        }
        _ => {
            xml.skip()?;
            None
        }
    })
}

// ---------------------------------------------------------------------------
// Conditional formats: legacy style:map (OpenOffice)
// ---------------------------------------------------------------------------

/// A `style:map` of a cell style.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RawMap {
    pub condition: String,
    pub apply_style: String,
    pub base: Option<(u32, u32)>,
}

/// A `style:map` condition (`cell-content()>5`, `cell-content-is-between(1,5)`,
/// `is-true-formula(…)`) → rule kind.
pub(crate) fn parse_map_condition(c: &str) -> Option<CfKindData> {
    let c = c.trim();
    let c = c.strip_prefix("of:").unwrap_or(c);
    let c = c.strip_prefix("oooc:").unwrap_or(c);
    for (name, op) in [
        ("cell-content-is-not-between", CmpOpData::NotBetween),
        ("cell-content-is-between", CmpOpData::Between),
    ] {
        if let Some(a) = call_args(c, name) {
            let (x, y) = split_top(a, ',')?;
            return Some(CfKindData::CellValue {
                op,
                formulas: vec![of(x), of(y)],
            });
        }
    }
    if let Some(f) = call_args(c, "is-true-formula") {
        return Some(CfKindData::Formula { formula: of(f) });
    }
    let rest = c.strip_prefix("cell-content()")?.trim_start();
    let (op, r) = if let Some(r) = rest.strip_prefix("<=") {
        (CmpOpData::Le, r)
    } else if let Some(r) = rest.strip_prefix(">=") {
        (CmpOpData::Ge, r)
    } else if let Some(r) = rest.strip_prefix("!=") {
        (CmpOpData::Ne, r)
    } else if let Some(r) = rest.strip_prefix("<>") {
        (CmpOpData::Ne, r)
    } else if let Some(r) = rest.strip_prefix('<') {
        (CmpOpData::Lt, r)
    } else if let Some(r) = rest.strip_prefix('>') {
        (CmpOpData::Gt, r)
    } else if let Some(r) = rest.strip_prefix('=') {
        (CmpOpData::Eq, r)
    } else {
        return None;
    };
    Some(CfKindData::CellValue {
        op,
        formulas: vec![of(r)],
    })
}

/// The `style:condition` of a rule kind that OpenOffice understands.
pub(crate) fn map_condition(kind: &CfKindData) -> Option<String> {
    Some(match kind {
        CfKindData::CellValue { op, formulas } => {
            let a = expr(formulas.first()?);
            match op {
                CmpOpData::Between => {
                    format!("cell-content-is-between({a},{})", expr(formulas.get(1)?))
                }
                CmpOpData::NotBetween => format!(
                    "cell-content-is-not-between({a},{})",
                    expr(formulas.get(1)?)
                ),
                other => format!("cell-content(){}{a}", cmp_token(*other)),
            }
        }
        CfKindData::Formula { formula } => format!("is-true-formula({})", expr(formula)),
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// Content validations
// ---------------------------------------------------------------------------

fn validation_condition(v: &ValidationData) -> Option<String> {
    let cmp = |prefix: &str, op: &CmpOpData, f: &[String], text_len: bool| -> Option<String> {
        let a = expr(f.first()?);
        Some(match op {
            CmpOpData::Between | CmpOpData::NotBetween => {
                let b = expr(f.get(1)?);
                let name = match (text_len, op) {
                    (true, CmpOpData::Between) => "cell-content-text-length-is-between",
                    (true, _) => "cell-content-text-length-is-not-between",
                    (false, CmpOpData::Between) => "cell-content-is-between",
                    (false, _) => "cell-content-is-not-between",
                };
                if prefix.is_empty() {
                    format!("{name}({a},{b})")
                } else {
                    format!("{prefix} and {name}({a},{b})")
                }
            }
            other => {
                if text_len {
                    format!("cell-content-text-length(){}{a}", cmp_token(*other))
                } else {
                    format!("{prefix} and cell-content(){}{a}", cmp_token(*other))
                }
            }
        })
    };
    let c = match &v.kind {
        ValidationKindData::Any => return None,
        ValidationKindData::List { source } => match source {
            ListSourceData::Values(items) => format!(
                "cell-content-is-in-list({})",
                items
                    .iter()
                    .map(|i| quote_text(i))
                    .collect::<Vec<_>>()
                    .join(";")
            ),
            ListSourceData::Range(f) => format!("cell-content-is-in-list({})", expr(f)),
        },
        ValidationKindData::Whole { op, formulas } => {
            cmp("cell-content-is-whole-number()", op, formulas, false)?
        }
        ValidationKindData::Decimal { op, formulas } => {
            cmp("cell-content-is-decimal-number()", op, formulas, false)?
        }
        ValidationKindData::Date { op, formulas } => {
            cmp("cell-content-is-date()", op, formulas, false)?
        }
        ValidationKindData::Time { op, formulas } => {
            cmp("cell-content-is-time()", op, formulas, false)?
        }
        ValidationKindData::TextLength { op, formulas } => cmp("", op, formulas, true)?,
        ValidationKindData::Custom { formula } => format!("is-true-formula({})", expr(formula)),
    };
    Some(format!("of:{c}"))
}

fn parse_cmp_tail(s: &str, text_len: bool) -> Option<(CmpOpData, Vec<String>)> {
    let s = s.trim();
    let names: [(&str, CmpOpData); 2] = if text_len {
        [
            (
                "cell-content-text-length-is-not-between",
                CmpOpData::NotBetween,
            ),
            ("cell-content-text-length-is-between", CmpOpData::Between),
        ]
    } else {
        [
            ("cell-content-is-not-between", CmpOpData::NotBetween),
            ("cell-content-is-between", CmpOpData::Between),
        ]
    };
    for (name, op) in names {
        if let Some(a) = call_args(s, name) {
            let (x, y) = split_top(a, ',')?;
            return Some((op, vec![of(x), of(y)]));
        }
    }
    let head = if text_len {
        "cell-content-text-length()"
    } else {
        "cell-content()"
    };
    let rest = s.strip_prefix(head)?.trim_start();
    let (op, r) = if let Some(r) = rest.strip_prefix("<=") {
        (CmpOpData::Le, r)
    } else if let Some(r) = rest.strip_prefix(">=") {
        (CmpOpData::Ge, r)
    } else if let Some(r) = rest.strip_prefix("!=") {
        (CmpOpData::Ne, r)
    } else if let Some(r) = rest.strip_prefix('<') {
        (CmpOpData::Lt, r)
    } else if let Some(r) = rest.strip_prefix('>') {
        (CmpOpData::Gt, r)
    } else if let Some(r) = rest.strip_prefix('=') {
        (CmpOpData::Eq, r)
    } else {
        return None;
    };
    Some((op, vec![of(r)]))
}

/// A `table:condition` → validation kind.
pub(crate) fn parse_validation_condition(c: &str) -> Option<ValidationKindData> {
    let c = c.trim();
    let c = c
        .strip_prefix("of:")
        .or_else(|| c.strip_prefix("oooc:"))
        .unwrap_or(c)
        .trim();
    if let Some(a) = call_args(c, "cell-content-is-in-list") {
        let a = a.trim();
        // Literal items are quoted strings separated by `;`.
        if a.starts_with('"') {
            let mut items = Vec::new();
            let mut rest = a;
            loop {
                match split_top(rest, ';') {
                    Some((x, y)) => {
                        items.push(unquote_text(x));
                        rest = y;
                    }
                    None => {
                        items.push(unquote_text(rest));
                        break;
                    }
                }
                if items.len() > 10_000 {
                    break;
                }
            }
            return Some(ValidationKindData::List {
                source: ListSourceData::Values(items),
            });
        }
        return Some(ValidationKindData::List {
            source: ListSourceData::Range(of(a)),
        });
    }
    if let Some(f) = call_args(c, "is-true-formula") {
        return Some(ValidationKindData::Custom { formula: of(f) });
    }
    if c.starts_with("cell-content-text-length") {
        let (op, formulas) = parse_cmp_tail(c, true)?;
        return Some(ValidationKindData::TextLength { op, formulas });
    }
    for (head, mk) in [
        (
            "cell-content-is-whole-number()",
            (|op, formulas| ValidationKindData::Whole { op, formulas })
                as fn(CmpOpData, Vec<String>) -> ValidationKindData,
        ),
        ("cell-content-is-decimal-number()", |op, formulas| {
            ValidationKindData::Decimal { op, formulas }
        }),
        ("cell-content-is-date()", |op, formulas| {
            ValidationKindData::Date { op, formulas }
        }),
        ("cell-content-is-time()", |op, formulas| {
            ValidationKindData::Time { op, formulas }
        }),
    ] {
        if let Some(rest) = c.strip_prefix(head) {
            let rest = rest.trim_start();
            let tail = rest.strip_prefix("and").map(str::trim_start);
            return match tail.and_then(|t| parse_cmp_tail(t, false)) {
                Some((op, f)) => Some(mk(op, f)),
                // A type test alone (no comparison): any number.
                None => Some(mk(CmpOpData::Ge, vec!["of:=-1E+307".into()])),
            };
        }
    }
    None
}

fn message<R: BufRead>(
    xml: &mut XmlReader<R>,
    e: &Elem,
) -> Result<(String, String, bool), OdsError> {
    let title = e.attr("table:title").unwrap_or("").to_string();
    let display = e.attr("table:display") == Some("true");
    let mut lines: Vec<String> = Vec::new();
    let d = xml.depth();
    loop {
        match xml.next()? {
            Ev::Start(c) if c.name == "text:p" => {
                let t = xml.text_content(MAX_TEXT)?;
                if lines.len() < 256 {
                    lines.push(t);
                }
            }
            Ev::Start(_) => xml.skip()?,
            Ev::End if xml.depth() < d => break,
            Ev::Eof => break,
            _ => {}
        }
    }
    Ok((title, lines.join("\n"), display))
}

/// Parses `table:content-validations` into name → validation (without
/// ranges; cells refer to them by name).
pub(crate) fn parse_validations<R: BufRead>(
    xml: &mut XmlReader<R>,
) -> Result<HashMap<String, ValidationData>, OdsError> {
    let mut out = HashMap::new();
    let d = xml.depth();
    loop {
        match xml.next()? {
            Ev::Start(e) if e.name == "table:content-validation" => {
                let name = e.attr("table:name").unwrap_or("").to_string();
                let kind = e
                    .attr("table:condition")
                    .and_then(parse_validation_condition)
                    .unwrap_or(ValidationKindData::Any);
                let base = e
                    .attr("table:base-cell-address")
                    .and_then(parse_base)
                    .unwrap_or((0, 0));
                let mut v = ValidationData {
                    ranges: Vec::new(),
                    base,
                    kind,
                    allow_blank: e.attr("table:allow-empty-cell") != Some("false"),
                    show_dropdown: false,
                    sort_ascending: false,
                    input_help: None,
                    error: None,
                };
                let dd = xml.depth();
                loop {
                    match xml.next()? {
                        Ev::Start(c) if c.name == "table:help-message" => {
                            let (title, text, display) = message(xml, &c)?;
                            if display {
                                v.input_help = Some(ValidationMessageData { title, text });
                            }
                        }
                        Ev::Start(c) if c.name == "table:error-message" => {
                            let style = match c.attr("table:message-type").unwrap_or("stop") {
                                "warning" => ValidationErrorStyleData::Warning,
                                "information" => ValidationErrorStyleData::Info,
                                _ => ValidationErrorStyleData::Stop,
                            };
                            let (title, text, display) = message(xml, &c)?;
                            if display {
                                v.error = Some(ValidationErrorData { style, title, text });
                            }
                        }
                        Ev::Start(_) => xml.skip()?,
                        Ev::End if xml.depth() < dd => break,
                        Ev::Eof => break,
                        _ => {}
                    }
                }
                // AOO xmlcvali.cxx:261-275: `no` hides the list,
                // `unsorted` (the default) and `sorted-ascending` show it.
                // ODF 1.2 spells the hidden value `none`; accept both.
                let display = e.attr("table:display-list");
                v.show_dropdown = matches!(v.kind, ValidationKindData::List { .. })
                    && !matches!(display, Some("no" | "none"));
                // ODF 1.2 and LibreOffice spell it `sort-ascending`.
                v.sort_ascending = v.show_dropdown
                    && matches!(display, Some("sorted-ascending" | "sort-ascending"));
                if !name.is_empty() && out.len() < MAX_ITEMS {
                    out.insert(name, v);
                }
            }
            Ev::Start(_) => xml.skip()?,
            Ev::End if xml.depth() < d => return Ok(out),
            Ev::Eof => return Ok(out),
            _ => {}
        }
    }
}

fn write_message(s: &mut String, tag: &str, extra: &str, title: &str, text: &str, display: bool) {
    let _ = write!(s, "<table:{tag}{extra} table:title=\"");
    escape_attr(title, s);
    let _ = write!(s, "\" table:display=\"{display}\">");
    for line in text.split('\n') {
        if line.is_empty() && text.is_empty() {
            continue;
        }
        s.push_str("<text:p>");
        escape_text(line, s);
        s.push_str("</text:p>");
    }
    let _ = write!(s, "</table:{tag}>");
}

/// Writes `table:content-validations` (names `val{tab}_{i}`).
pub(crate) fn write_validations(s: &mut String, sheets: &[(String, &[ValidationData])]) {
    if sheets.iter().all(|(_, v)| v.is_empty()) {
        return;
    }
    s.push_str("<table:content-validations>");
    for (tab, (sheet, vals)) in sheets.iter().enumerate() {
        for (i, v) in vals.iter().enumerate() {
            let _ = write!(
                s,
                r#"<table:content-validation table:name="{}""#,
                validation_name(tab, i)
            );
            if let Some(c) = validation_condition(v) {
                s.push_str(r#" table:condition=""#);
                escape_attr(&c, s);
                s.push('"');
            }
            let _ = write!(s, r#" table:allow-empty-cell="{}""#, v.allow_blank);
            if matches!(v.kind, ValidationKindData::List { .. }) {
                // AOO XMLStylesExportHelper.cxx:373-383.
                s.push_str(if !v.show_dropdown {
                    r#" table:display-list="no""#
                } else if v.sort_ascending {
                    r#" table:display-list="sorted-ascending""#
                } else {
                    r#" table:display-list="unsorted""#
                });
            }
            s.push_str(r#" table:base-cell-address=""#);
            escape_attr(&base_address(sheet, v.base.0, v.base.1), s);
            s.push_str("\">");
            if let Some(h) = &v.input_help {
                write_message(s, "help-message", "", &h.title, &h.text, true);
            }
            match &v.error {
                Some(e) => {
                    let t = match e.style {
                        ValidationErrorStyleData::Stop => "stop",
                        ValidationErrorStyleData::Warning => "warning",
                        ValidationErrorStyleData::Info => "information",
                    };
                    write_message(
                        s,
                        "error-message",
                        &format!(r#" table:message-type="{t}""#),
                        &e.title,
                        &e.text,
                        true,
                    );
                }
                None => write_message(
                    s,
                    "error-message",
                    r#" table:message-type="stop""#,
                    "",
                    "",
                    false,
                ),
            }
            s.push_str("</table:content-validation>");
        }
    }
    s.push_str("</table:content-validations>");
}

/// The name of validation `i` of sheet `tab`.
pub(crate) fn validation_name(tab: usize, i: usize) -> String {
    format!("val{}_{}", tab + 1, i + 1)
}

/// Merges rectangles of one validation (vertical runs of the same columns
/// are joined).
pub(crate) fn merge_rects(mut rects: Vec<CellRangeLocal>) -> Vec<CellRangeLocal> {
    rects.sort_by_key(|r| (r.c0, r.c1, r.r0));
    let mut out: Vec<CellRangeLocal> = Vec::with_capacity(rects.len());
    for r in rects {
        if let Some(last) = out.last_mut() {
            if last.c0 == r.c0 && last.c1 == r.c1 && last.r1.checked_add(1) == Some(r.r0) {
                last.r1 = r.r1;
                continue;
            }
        }
        out.push(r);
    }
    // Join horizontally adjacent equal row spans.
    out.sort_by_key(|r| (r.r0, r.r1, r.c0));
    let mut joined: Vec<CellRangeLocal> = Vec::with_capacity(out.len());
    for r in out {
        if let Some(last) = joined.last_mut() {
            if last.r0 == r.r0 && last.r1 == r.r1 && last.c1.checked_add(1) == Some(r.c0) {
                last.c1 = r.c1;
                continue;
            }
        }
        joined.push(r);
    }
    joined.truncate(1024);
    joined
}

// ---------------------------------------------------------------------------
// Annotations, protection
// ---------------------------------------------------------------------------

/// Parses `office:annotation` (row/column are filled in by the caller).
pub(crate) fn parse_annotation<R: BufRead>(
    xml: &mut XmlReader<R>,
    e: &Elem,
) -> Result<NoteData, OdsError> {
    let mut n = NoteData {
        shown: e.attr("office:display") == Some("true"),
        ..NoteData::default()
    };
    let mut lines: Vec<String> = Vec::new();
    let mut total = 0usize;
    let d = xml.depth();
    loop {
        match xml.next()? {
            Ev::Start(c) => match c.name.as_str() {
                "dc:creator" => n.author = Some(xml.text_content(MAX_TEXT)?),
                "dc:date" => n.created = Some(xml.text_content(MAX_TEXT)?),
                "text:p" => {
                    let t = xml.text_content(MAX_TEXT)?;
                    // Notes are capped at MAX_NOTE_TEXT (the engine's
                    // limit): text past it is not kept (MEDIUM-8).
                    if lines.len() < 4096 && total < MAX_NOTE_TEXT {
                        total += t.len() + 1;
                        lines.push(t);
                    }
                }
                _ => xml.skip()?,
            },
            Ev::End if xml.depth() < d => break,
            Ev::Eof => break,
            _ => {}
        }
    }
    n.text = lines.join("\n");
    if n.text.len() > MAX_NOTE_TEXT {
        let mut end = MAX_NOTE_TEXT;
        while !n.text.is_char_boundary(end) {
            end -= 1;
        }
        n.text.truncate(end);
    }
    n.author = n.author.filter(|a| !a.is_empty());
    Ok(n)
}

/// Writes `office:annotation` for a note.
pub(crate) fn write_annotation(s: &mut String, n: &NoteData) {
    let _ = write!(s, r#"<office:annotation office:display="{}">"#, n.shown);
    if let Some(a) = &n.author {
        s.push_str("<dc:creator>");
        escape_text(a, s);
        s.push_str("</dc:creator>");
    }
    if let Some(d) = &n.created {
        s.push_str("<dc:date>");
        escape_text(d, s);
        s.push_str("</dc:date>");
    }
    crate::write::write_paragraphs(s, &n.text);
    s.push_str("</office:annotation>");
}

/// The protection attributes of `table:table` / `office:spreadsheet`.
pub(crate) fn parse_protection_attrs(e: &Elem, flag: &str) -> Option<SheetProtectionData> {
    if e.attr(flag) != Some("true") {
        return None;
    }
    let mut p = SheetProtectionData::default();
    if let Some(k) = e
        .attr("table:protection-key")
        .filter(|k| !k.is_empty() && k.len() <= MAX_PROTECTION_KEY)
    {
        p.password_hash = Some(k.to_string());
        p.algorithm = Some(
            match e
                .attr("table:protection-key-digest-algorithm")
                .unwrap_or(URI_SHA1)
            {
                URI_SHA256 | "http://www.w3.org/2001/04/xmlenc#sha256" => {
                    tally_model::hash_algorithms::SHA256.to_string()
                }
                URI_SHA1 => tally_model::hash_algorithms::SHA1.to_string(),
                // Our own 4-hex-digit Excel hash (LibreOffice re-hashes it
                // with SHA-1, which cannot be verified: kept as the URI).
                URI_XLS_LEGACY if k.len() == 4 && k.chars().all(|c| c.is_ascii_hexdigit()) => {
                    tally_model::hash_algorithms::XLSX_LEGACY.to_string()
                }
                other => other.to_string(),
            },
        );
    }
    Some(p)
}

/// Writes protection attributes (`flag="true"`, key, algorithm).
pub(crate) fn write_protection_attrs(s: &mut String, flag: &str, p: &SheetProtectionData) {
    let _ = write!(s, r#" {flag}="true""#);
    let Some(hash) = p.password_hash.as_deref().filter(|h| !h.is_empty()) else {
        return;
    };
    let uri = match p.algorithm.as_deref() {
        Some(tally_model::hash_algorithms::SHA256) => Some(URI_SHA256),
        Some(tally_model::hash_algorithms::SHA1) | None => Some(URI_SHA1),
        Some(tally_model::hash_algorithms::XLSX_LEGACY) => None,
        Some(u) if u.starts_with("http") => {
            s.push_str(r#" table:protection-key=""#);
            escape_attr(hash, s);
            s.push_str(r#"" table:protection-key-digest-algorithm=""#);
            escape_attr(u, s);
            s.push('"');
            return;
        }
        // OOXML agile hashes have no ODF form: the password is dropped.
        Some(_) => return,
    };
    match uri {
        Some(u) => {
            s.push_str(r#" table:protection-key=""#);
            escape_attr(hash, s);
            let _ = write!(s, r#"" table:protection-key-digest-algorithm="{u}""#);
        }
        None => {
            // LibreOffice stores the Excel hash re-hashed with SHA-1; we keep
            // the 4-digit form under the legacy URI so it round-trips.
            s.push_str(r#" table:protection-key=""#);
            escape_attr(hash, s);
            let _ = write!(
                s,
                r#"" table:protection-key-digest-algorithm="{URI_XLS_LEGACY}""#
            );
        }
    }
}

/// `loext:table-protection` options.
pub(crate) fn parse_protection_options(e: &Elem, p: &mut SheetProtectionData) {
    let f = |k: &str| e.attr(k) == Some("true");
    p.select_locked = f("loext:select-protected-cells");
    p.select_unlocked = f("loext:select-unprotected-cells");
    p.insert_cols = f("loext:insert-columns");
    p.insert_rows = f("loext:insert-rows");
    p.delete_cols = f("loext:delete-columns");
    p.delete_rows = f("loext:delete-rows");
    p.auto_filter = f("loext:use-autofilter");
    p.format_cells = f("tally:format-cells");
    p.sort = f("tally:sort");
}

/// Writes `loext:table-protection`.
pub(crate) fn write_protection_options(s: &mut String, p: &SheetProtectionData) {
    s.push_str("<loext:table-protection");
    for (k, v) in [
        ("loext:select-protected-cells", p.select_locked),
        ("loext:select-unprotected-cells", p.select_unlocked),
        ("loext:insert-columns", p.insert_cols),
        ("loext:insert-rows", p.insert_rows),
        ("loext:delete-columns", p.delete_cols),
        ("loext:delete-rows", p.delete_rows),
        ("loext:use-autofilter", p.auto_filter),
        ("tally:format-cells", p.format_cells),
        ("tally:sort", p.sort),
    ] {
        if v {
            let _ = write!(s, r#" {k}="true""#);
        }
    }
    s.push_str("/>");
}

// ---------------------------------------------------------------------------
// Page layouts and master pages
// ---------------------------------------------------------------------------

/// `style:page-layout` as read.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct PageLayout {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub landscape: bool,
    pub margins: MarginsData,
    pub grid: bool,
    pub headers: bool,
    /// The other `style:print` tokens, when the attribute is present:
    /// (charts, annotations, zero-values, formulas).
    pub content: Option<(bool, bool, bool, bool)>,
    pub scale: Option<PageScaleData>,
    pub center_h: bool,
    pub center_v: bool,
    pub ltr: bool,
    pub first_page: Option<u32>,
    pub header_height: Option<u32>,
    pub footer_height: Option<u32>,
    /// Header area layout (`on` unused: the master page decides).
    pub header_area: HfAreaData,
    /// Footer area layout.
    pub footer_area: HfAreaData,
}

/// `style:master-page` as read.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct MasterPage {
    pub layout: String,
    /// Header texts (also of a header switched off).
    pub header: Option<HeaderFooterData>,
    pub footer: Option<HeaderFooterData>,
    /// `style:header` present and not `style:display="false"`.
    pub header_on: bool,
    pub footer_on: bool,
}

/// Parses `style:page-layout` (the element just started).
pub(crate) fn parse_page_layout<R: BufRead>(
    xml: &mut XmlReader<R>,
) -> Result<PageLayout, OdsError> {
    let mut p = PageLayout {
        margins: MarginsData {
            header: 0,
            footer: 0,
            ..MarginsData::default()
        },
        ..PageLayout::default()
    };
    let d = xml.depth();
    loop {
        match xml.next()? {
            Ev::Start(c) => {
                match c.name.as_str() {
                    "style:page-layout-properties" => {
                        let len = |k: &str| c.attr(k).and_then(length_to_hmm);
                        p.width = len("fo:page-width");
                        p.height = len("fo:page-height");
                        p.landscape = c.attr("style:print-orientation") == Some("landscape");
                        if let Some(m) = len("fo:margin") {
                            p.margins.top = m;
                            p.margins.bottom = m;
                            p.margins.left = m;
                            p.margins.right = m;
                        }
                        for (k, slot) in [
                            ("fo:margin-top", &mut p.margins.top),
                            ("fo:margin-bottom", &mut p.margins.bottom),
                            ("fo:margin-left", &mut p.margins.left),
                            ("fo:margin-right", &mut p.margins.right),
                        ] {
                            if let Some(v) = len(k) {
                                *slot = v;
                            }
                        }
                        let print = c.attr("style:print").unwrap_or("");
                        p.grid = print.split_whitespace().any(|t| t == "grid");
                        p.headers = print.split_whitespace().any(|t| t == "headers");
                        if c.attr("style:print").is_some() {
                            let has = |name: &str| print.split_whitespace().any(|t| t == name);
                            p.content = Some((
                                has("charts"),
                                has("annotations"),
                                has("zero-values"),
                                has("formulas"),
                            ));
                        }
                        let pct = c
                            .attr("style:scale-to")
                            .and_then(|v| v.trim().trim_end_matches('%').parse::<f64>().ok());
                        let x = c
                            .attr("loext:scale-to-X")
                            .or_else(|| c.attr("style:scale-to-X"))
                            .and_then(|v| v.trim().parse::<u32>().ok());
                        let y = c
                            .attr("loext:scale-to-Y")
                            .or_else(|| c.attr("style:scale-to-Y"))
                            .and_then(|v| v.trim().parse::<u32>().ok());
                        let pages = c
                            .attr("style:scale-to-pages")
                            .and_then(|v| v.trim().parse::<u32>().ok());
                        p.scale = if x.is_some() || y.is_some() {
                            Some(PageScaleData::FitTo {
                                width: x.unwrap_or(0),
                                height: y.unwrap_or(0),
                            })
                        } else if let Some(n) = pages {
                            Some(PageScaleData::FitPages(n.clamp(1, 1000)))
                        } else {
                            pct.map(|v| PageScaleData::Percent((v.round() as u32).clamp(10, 400)))
                        };
                        p.center_h =
                            matches!(c.attr("style:table-centering"), Some("horizontal" | "both"));
                        p.center_v =
                            matches!(c.attr("style:table-centering"), Some("vertical" | "both"));
                        p.ltr = c.attr("style:print-page-order") == Some("ltr");
                        p.first_page = c
                            .attr("style:first-page-number")
                            .and_then(|v| v.trim().parse().ok());
                    }
                    "style:header-style" | "style:footer-style" => {
                        let header = c.name == "style:header-style";
                        let dd = xml.depth();
                        loop {
                            match xml.next()? {
                                Ev::Start(hp) => {
                                    if hp.name == "style:header-footer-properties" {
                                        let len = |k: &str| hp.attr(k).and_then(length_to_hmm);
                                        // AutoFit height writes a minimum
                                        // height, a fixed one `svg:height`.
                                        let (h, autofit) = match len("fo:min-height") {
                                            Some(h) => (h, true),
                                            None => match len("svg:height") {
                                                Some(h) => (h, false),
                                                None => (0, true),
                                            },
                                        };
                                        let side = len("fo:margin");
                                        let area = HfAreaData {
                                            on: true,
                                            // The spacing is the margin
                                            // towards the body.
                                            spacing: len(if header {
                                                "fo:margin-bottom"
                                            } else {
                                                "fo:margin-top"
                                            })
                                            .or(side)
                                            .unwrap_or(0),
                                            autofit,
                                            left: len("fo:margin-left").or(side).unwrap_or(0),
                                            right: len("fo:margin-right").or(side).unwrap_or(0),
                                        };
                                        if header {
                                            p.header_height = Some(h);
                                            p.header_area = area;
                                        } else {
                                            p.footer_height = Some(h);
                                            p.footer_area = area;
                                        }
                                    }
                                    xml.skip()?;
                                }
                                Ev::End if xml.depth() < dd => break,
                                Ev::Eof => break,
                                _ => {}
                            }
                        }
                        continue;
                    }
                    _ => {}
                }
                xml.skip()?;
            }
            Ev::End if xml.depth() < d => return Ok(p),
            Ev::Eof => return Ok(p),
            _ => {}
        }
    }
}

/// Header/footer paragraph text with fields as placeholders.
fn hf_paragraphs<R: BufRead>(xml: &mut XmlReader<R>) -> Result<String, OdsError> {
    let mut out = String::new();
    let mut paras = 0;
    let d = xml.depth();
    loop {
        match xml.next()? {
            Ev::Start(c) => {
                let field = match c.name.as_str() {
                    "text:p" => {
                        if paras > 0 {
                            out.push('\n');
                        }
                        paras += 1;
                        None
                    }
                    "text:page-number" => Some("&[PAGE]"),
                    "text:page-count" => Some("&[PAGES]"),
                    "text:sheet-name" => Some("&[SHEET]"),
                    "text:file-name" | "text:title" => Some("&[FILE]"),
                    "text:date" => Some("&[DATE]"),
                    "text:time" => Some("&[TIME]"),
                    "text:s" => {
                        let n = c
                            .attr("text:c")
                            .and_then(|v| v.parse::<usize>().ok())
                            .unwrap_or(1)
                            .min(100);
                        out.extend(std::iter::repeat_n(' ', n));
                        xml.skip()?;
                        continue;
                    }
                    "text:tab" => {
                        out.push('\t');
                        xml.skip()?;
                        continue;
                    }
                    // Shift+Enter in AOO's header editor.
                    "text:line-break" => {
                        out.push('\n');
                        xml.skip()?;
                        continue;
                    }
                    _ => None,
                };
                if let Some(f) = field {
                    out.push_str(f);
                    xml.skip()?;
                }
            }
            Ev::Text(t) => {
                if out.len() + t.len() > MAX_TEXT {
                    return Err(OdsError::LimitExceeded {
                        what: "header text",
                        limit: MAX_TEXT as u64,
                    });
                }
                out.push_str(&t);
            }
            Ev::End if xml.depth() < d => return Ok(out),
            Ev::Eof => return Ok(out),
            _ => {}
        }
    }
}

/// A `style:header` / `style:footer`: its texts and whether it is shown
/// (`style:display="false"` switches it off but keeps the texts, AOO 3+).
fn parse_hf<R: BufRead>(
    xml: &mut XmlReader<R>,
    e: &Elem,
) -> Result<(HeaderFooterData, bool), OdsError> {
    let shown = e.attr("style:display") != Some("false");
    let mut h = HeaderFooterData::default();
    let d = xml.depth();
    loop {
        match xml.next()? {
            Ev::Start(c) => match c.name.as_str() {
                "style:region-left" => h.left = hf_paragraphs(xml)?,
                "style:region-center" => h.center = hf_paragraphs(xml)?,
                "style:region-right" => h.right = hf_paragraphs(xml)?,
                "text:p" => {
                    // A header without regions: the paragraph is centred.
                    let mut t = hf_paragraphs(xml)?;
                    if !h.center.is_empty() {
                        t = format!("{}\n{t}", h.center);
                    }
                    h.center = t;
                }
                _ => xml.skip()?,
            },
            Ev::End if xml.depth() < d => break,
            Ev::Eof => break,
            _ => {}
        }
    }
    Ok((h, shown))
}

/// Parses `office:master-styles` → name → master page.
pub(crate) fn parse_master_styles<R: BufRead>(
    xml: &mut XmlReader<R>,
) -> Result<HashMap<String, MasterPage>, OdsError> {
    let mut out = HashMap::new();
    let d = xml.depth();
    loop {
        match xml.next()? {
            Ev::Start(e) if e.name == "style:master-page" => {
                let name = e.attr("style:name").unwrap_or("").to_string();
                let mut m = MasterPage {
                    layout: e.attr("style:page-layout-name").unwrap_or("").to_string(),
                    ..MasterPage::default()
                };
                let dd = xml.depth();
                loop {
                    match xml.next()? {
                        Ev::Start(c) if c.name == "style:header" => {
                            let (h, on) = parse_hf(xml, &c)?;
                            m.header_on = on;
                            m.header = (on || !h.is_empty()).then_some(h);
                        }
                        Ev::Start(c) if c.name == "style:footer" => {
                            let (f, on) = parse_hf(xml, &c)?;
                            m.footer_on = on;
                            m.footer = (on || !f.is_empty()).then_some(f);
                        }
                        Ev::Start(_) => xml.skip()?,
                        Ev::End if xml.depth() < dd => break,
                        Ev::Eof => break,
                        _ => {}
                    }
                }
                if out.len() < 10_000 {
                    out.insert(name, m);
                }
            }
            Ev::Start(_) => xml.skip()?,
            Ev::End if xml.depth() < d => return Ok(out),
            Ev::Eof => return Ok(out),
            _ => {}
        }
    }
}

/// The model page setup of a master page.
pub(crate) fn page_setup_of(m: &MasterPage, l: Option<&PageLayout>) -> PageSetupData {
    let mut p = PageSetupData {
        header: m.header.clone(),
        footer: m.footer.clone(),
        ..PageSetupData::default()
    };
    if let Some(l) = l {
        p.header_area = l.header_area;
        p.footer_area = l.footer_area;
        p.orientation = if l.landscape {
            OrientationData::Landscape
        } else {
            OrientationData::Portrait
        };
        if let (Some(w), Some(h)) = (l.width, l.height) {
            p.paper = PaperData::from_size_hmm(w, h);
        }
        p.margins = MarginsData {
            header: l.header_height.unwrap_or(MarginsData::default().header),
            footer: l.footer_height.unwrap_or(MarginsData::default().footer),
            ..l.margins
        };
        if let Some(s) = l.scale {
            p.scale = s;
        }
        p.print_gridlines = l.grid;
        p.print_headers = l.headers;
        if let Some((charts, notes, zero, formulas)) = l.content {
            p.print_charts = charts;
            p.print_notes = if notes {
                tally_model::NotesPrintData::AtEnd
            } else {
                tally_model::NotesPrintData::None
            };
            p.print_zero_values = zero;
            p.print_formulas = formulas;
        }
        p.center_h = l.center_h;
        p.center_v = l.center_v;
        p.page_order = if l.ltr {
            PageOrderData::LeftToRight
        } else {
            PageOrderData::TopToBottom
        };
        p.first_page_number = l.first_page;
    }
    // An absent `style:header` is no header (ODF); `display="false"` is a
    // header switched off.
    p.header_area.on = m.header_on;
    p.footer_area.on = m.footer_on;
    p
}

fn write_hf_region(s: &mut String, tag: &str, text: &str) {
    let _ = write!(s, "<style:{tag}>");
    for line in text.split('\n') {
        s.push_str("<text:p>");
        let mut rest = line;
        while !rest.is_empty() {
            let next = rest.find("&[");
            let (lit, tail) = match next {
                Some(i) => (&rest[..i], &rest[i..]),
                None => (rest, ""),
            };
            escape_text(lit, s);
            if tail.is_empty() {
                break;
            }
            let fields = [
                ("&[PAGES]", "<text:page-count>99</text:page-count>"),
                ("&[PAGE]", "<text:page-number>1</text:page-number>"),
                ("&[SHEET]", "<text:sheet-name>???</text:sheet-name>"),
                ("&[FILE]", "<text:title>???</text:title>"),
                ("&[DATE]", "<text:date>01/01/2026</text:date>"),
                ("&[TIME]", "<text:time>00:00:00</text:time>"),
            ];
            match fields.iter().find(|(k, _)| tail.starts_with(k)) {
                Some((k, xml)) => {
                    s.push_str(xml);
                    rest = &tail[k.len()..];
                }
                None => {
                    escape_text("&[", s);
                    rest = &tail[2..];
                }
            }
        }
        s.push_str("</text:p>");
    }
    let _ = write!(s, "</style:{tag}>");
}

/// A `style:header` / `style:footer`; one switched off keeps its texts
/// under `style:display="false"` (AOO 3+).
fn write_hf(s: &mut String, tag: &str, h: &Option<HeaderFooterData>, on: bool) {
    match h {
        None => {
            let _ = write!(s, r#"<style:{tag} style:display="false"/>"#);
        }
        Some(h) if !on && h.is_empty() => {
            let _ = write!(s, r#"<style:{tag} style:display="false"/>"#);
        }
        Some(h) => {
            if on {
                let _ = write!(s, "<style:{tag}>");
            } else {
                let _ = write!(s, r#"<style:{tag} style:display="false">"#);
            }
            write_hf_region(s, "region-left", &h.left);
            write_hf_region(s, "region-center", &h.center);
            write_hf_region(s, "region-right", &h.right);
            let _ = write!(s, "</style:{tag}>");
        }
    }
}

/// Writes a `style:page-layout` named `layout` for `p`.
pub(crate) fn write_page_layout(s: &mut String, layout: &str, p: &PageSetupData) {
    let (mut w, mut h) = p.paper.size_hmm();
    if p.orientation == OrientationData::Landscape {
        std::mem::swap(&mut w, &mut h);
    }
    let _ = write!(
        s,
        r#"<style:page-layout style:name="{layout}"><style:page-layout-properties fo:page-width="{}" fo:page-height="{}" style:print-orientation="{}" fo:margin-top="{}" fo:margin-bottom="{}" fo:margin-left="{}" fo:margin-right="{}""#,
        hmm_to_cm(w),
        hmm_to_cm(h),
        if p.orientation == OrientationData::Landscape {
            "landscape"
        } else {
            "portrait"
        },
        hmm_to_cm(p.margins.top),
        hmm_to_cm(p.margins.bottom),
        hmm_to_cm(p.margins.left),
        hmm_to_cm(p.margins.right),
    );
    // AOO writes every token of what the sheet prints (Format > Page >
    // Sheet > Print); drawings and objects have no switch of their own here
    // and follow "charts and pictures".
    let mut print = Vec::new();
    if p.print_charts {
        print.extend(["charts", "drawings", "objects"]);
    }
    if p.print_zero_values {
        print.push("zero-values");
    }
    if p.print_formulas {
        print.push("formulas");
    }
    if p.print_notes != tally_model::NotesPrintData::None {
        print.push("annotations");
    }
    if p.print_gridlines {
        print.push("grid");
    }
    if p.print_headers {
        print.push("headers");
    }
    let _ = write!(s, r#" style:print="{}""#, print.join(" "));
    match p.scale {
        PageScaleData::Percent(n) => {
            let _ = write!(s, r#" style:scale-to="{n}%""#);
        }
        PageScaleData::FitTo { width, height } => {
            let _ = write!(
                s,
                r#" loext:scale-to-X="{width}" loext:scale-to-Y="{height}""#
            );
        }
        PageScaleData::FitPages(pages) => {
            let _ = write!(s, r#" style:scale-to-pages="{pages}""#);
        }
    }
    let centering = match (p.center_h, p.center_v) {
        (true, true) => "both",
        (true, false) => "horizontal",
        (false, true) => "vertical",
        (false, false) => "none",
    };
    let _ = write!(
        s,
        r#" style:table-centering="{centering}" style:print-page-order="{}""#,
        if p.page_order == PageOrderData::LeftToRight {
            "ltr"
        } else {
            "ttb"
        }
    );
    match p.first_page_number {
        Some(n) => {
            let _ = write!(s, r#" style:first-page-number="{n}""#);
        }
        None => s.push_str(r#" style:first-page-number="continue""#),
    }
    s.push_str(" style:writing-mode=\"lr-tb\"/>");
    write_hf_style(
        s,
        "header",
        p.header_shown().is_some(),
        p.margins.header,
        &p.header_area,
    );
    write_hf_style(
        s,
        "footer",
        p.footer_shown().is_some(),
        p.margins.footer,
        &p.footer_area,
    );
    s.push_str("</style:page-layout>");
}

/// `style:header-style` / `style:footer-style`: the area's height
/// (`fo:min-height` with AutoFit height, else `svg:height`), its side
/// margins and its spacing towards the body. Like AOO, an area that is
/// switched off is written empty.
fn write_hf_style(s: &mut String, tag: &str, on: bool, height: u32, a: &HfAreaData) {
    if !on {
        let _ = write!(s, "<style:{tag}-style/>");
        return;
    }
    let _ = write!(
        s,
        r#"<style:{tag}-style><style:header-footer-properties {}="{}" fo:margin-left="{}" fo:margin-right="{}" {}="{}"/></style:{tag}-style>"#,
        if a.autofit {
            "fo:min-height"
        } else {
            "svg:height"
        },
        hmm_to_cm(height),
        hmm_to_cm(a.left),
        hmm_to_cm(a.right),
        if tag == "header" {
            "fo:margin-bottom"
        } else {
            "fo:margin-top"
        },
        hmm_to_cm(a.spacing),
    );
}

/// Writes a `style:master-page`.
pub(crate) fn write_master_page(s: &mut String, name: &str, layout: &str, p: &PageSetupData) {
    let _ = write!(
        s,
        r#"<style:master-page style:name="{name}" style:page-layout-name="{layout}">"#
    );
    write_hf(s, "header", &p.header, p.header_area.on);
    write_hf(s, "footer", &p.footer, p.footer_area.on);
    s.push_str("</style:master-page>");
}

/// Writes a named conditional style (`style:style` in `office:styles`).
pub(crate) fn write_cf_style(
    s: &mut String,
    name: &str,
    st: &CfStyleData,
    data_style: Option<&str>,
) {
    let _ = write!(
        s,
        r#"<style:style style:name="{name}" style:family="table-cell" style:parent-style-name="Default""#
    );
    if let Some(ds) = data_style {
        let _ = write!(s, r#" style:data-style-name="{ds}""#);
    }
    s.push('>');
    let (fill, color) = match &st.display {
        Some(tally_model::DisplayData::Chip { bg, fg }) => {
            (st.fill.or(Some(*bg)), st.color.or(Some(*fg)))
        }
        None => (st.fill, st.color),
    };
    let mut cp = String::new();
    if let Some(f) = fill {
        let _ = write!(cp, r#" fo:background-color="{}""#, color_hex(f));
    }
    if let Some(b) = &st.borders {
        for (side, l) in [
            ("top", &b.top),
            ("bottom", &b.bottom),
            ("left", &b.left),
            ("right", &b.right),
        ] {
            if let Some(l) = l {
                let _ = write!(
                    cp,
                    r#" fo:border-{side}="{}""#,
                    crate::units::border_to_string(l)
                );
            }
        }
    }
    if let Some(tally_model::DisplayData::Chip { bg, fg }) = &st.display {
        let _ = write!(cp, r#" tally:chip="{} {}""#, color_hex(*bg), color_hex(*fg));
    }
    if !cp.is_empty() {
        let _ = write!(s, "<style:table-cell-properties{cp}/>");
    }
    let mut tp = String::new();
    if let Some(b) = st.bold {
        let w = if b { "bold" } else { "normal" };
        let _ = write!(
            tp,
            r#" fo:font-weight="{w}" style:font-weight-asian="{w}" style:font-weight-complex="{w}""#
        );
    }
    if let Some(i) = st.italic {
        let w = if i { "italic" } else { "normal" };
        let _ = write!(
            tp,
            r#" fo:font-style="{w}" style:font-style-asian="{w}" style:font-style-complex="{w}""#
        );
    }
    if let Some(u) = st.underline {
        let _ = write!(
            tp,
            r#" style:text-underline-style="{}""#,
            if u { "solid" } else { "none" }
        );
    }
    if let Some(x) = st.strike {
        let _ = write!(
            tp,
            r#" style:text-line-through-style="{}""#,
            if x { "solid" } else { "none" }
        );
    }
    if let Some(c) = color {
        let _ = write!(tp, r#" fo:color="{}""#, color_hex(c));
    }
    if !tp.is_empty() {
        let _ = write!(s, "<style:text-properties{tp}/>");
    }
    s.push_str("</style:style>");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn condition_values_round_trip() {
        let kinds = [
            CfKindData::CellValue {
                op: CmpOpData::Ge,
                formulas: vec!["of:=[.B1]*2".into()],
            },
            CfKindData::CellValue {
                op: CmpOpData::NotBetween,
                formulas: vec!["of:=SUM([.A1];1)".into(), "of:=\"a,b\"".into()],
            },
            CfKindData::Formula {
                formula: "of:=MOD([.A1];2)=0".into(),
            },
            CfKindData::Top {
                rank: 5,
                percent: true,
                bottom: true,
            },
            CfKindData::Text {
                op: CfTextOpData::NotContains,
                text: "say \"hi\"".into(),
            },
            CfKindData::Blank,
            CfKindData::NotBlank,
            CfKindData::Unique,
            CfKindData::NotError,
            CfKindData::Average {
                above: false,
                equal: true,
            },
        ];
        for k in kinds {
            let v = condition_value(&k, (0, 0)).unwrap();
            assert_eq!(parse_condition_value(&v, (0, 0)), Some(k), "{v}");
        }
        assert_eq!(
            parse_map_condition("cell-content()<>[.B2]"),
            Some(CfKindData::CellValue {
                op: CmpOpData::Ne,
                formulas: vec!["of:=[.B2]".into()]
            })
        );
        assert_eq!(
            map_condition(&CfKindData::CellValue {
                op: CmpOpData::Between,
                formulas: vec!["of:=1".into(), "of:=5".into()]
            })
            .as_deref(),
            Some("cell-content-is-between(1,5)")
        );
    }

    #[test]
    fn validation_conditions_round_trip() {
        let mk = |kind| ValidationData {
            ranges: vec![],
            base: (0, 0),
            kind,
            allow_blank: true,
            show_dropdown: true,
            sort_ascending: false,
            input_help: None,
            error: None,
        };
        let kinds = [
            ValidationKindData::List {
                source: ListSourceData::Values(vec!["a".into(), "b;c".into(), "d\"e".into()]),
            },
            ValidationKindData::List {
                source: ListSourceData::Range("of:=[$Sheet1.$A$1:.$A$5]".into()),
            },
            ValidationKindData::Whole {
                op: CmpOpData::Between,
                formulas: vec!["of:=1".into(), "of:=[.$B$1]".into()],
            },
            ValidationKindData::Decimal {
                op: CmpOpData::Lt,
                formulas: vec!["of:=2.5".into()],
            },
            ValidationKindData::Date {
                op: CmpOpData::Ge,
                formulas: vec!["of:=TODAY()".into()],
            },
            ValidationKindData::TextLength {
                op: CmpOpData::Le,
                formulas: vec!["of:=10".into()],
            },
            ValidationKindData::TextLength {
                op: CmpOpData::NotBetween,
                formulas: vec!["of:=1".into(), "of:=3".into()],
            },
            ValidationKindData::Custom {
                formula: "of:=ISNUMBER([.A1])".into(),
            },
        ];
        for k in kinds {
            let c = validation_condition(&mk(k.clone())).unwrap();
            assert_eq!(parse_validation_condition(&c), Some(k), "{c}");
        }
        // LibreOffice's own forms.
        assert!(matches!(
            parse_validation_condition(
                "of:cell-content-is-whole-number() and cell-content()>=[.A1]"
            ),
            Some(ValidationKindData::Whole {
                op: CmpOpData::Ge,
                ..
            })
        ));
    }

    #[test]
    fn ranges_and_bases() {
        assert_eq!(parse_base("$Sheet1.$C$4"), Some((3, 2)));
        assert_eq!(parse_base("'My Sheet'.A1"), Some((0, 0)));
        let r = parse_range_list("Sheet1.A1:Sheet1.B5 'a b'.C3:'a b'.C3");
        assert_eq!(r.len(), 2);
        assert_eq!((r[0].r1, r[0].c1), (4, 1));
        assert_eq!(range_list("S", &r[..1]), "S.A1:S.B5");
        let merged = merge_rects(vec![
            CellRangeLocal {
                r0: 0,
                c0: 1,
                r1: 0,
                c1: 1,
            },
            CellRangeLocal {
                r0: 1,
                c0: 1,
                r1: 4,
                c1: 1,
            },
            CellRangeLocal {
                r0: 0,
                c0: 2,
                r1: 4,
                c1: 2,
            },
        ]);
        assert_eq!(
            merged,
            vec![CellRangeLocal {
                r0: 0,
                c0: 1,
                r1: 4,
                c1: 2
            }]
        );
    }
}
