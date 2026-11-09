//! AutoFilter (`<autoFilter>` of a worksheet): the area, per-column value
//! lists (`filters`), one custom condition (`customFilters`) and top/bottom
//! N (`top10`). Written through the patches pass (rust_xlsxwriter has no
//! `top10` filter and hides rows itself); the rows the filter hides are
//! written as hidden rows, as Excel does, and read back as filtered rows.

use std::io::BufRead;

use tally_model::{
    AutoFilterData, DimRun, FilterColumnData, FilterConditionData, FilterOpData, SheetData,
    WorkbookData,
};
use tally_types::{col_to_letters, MAX_COLS};

use crate::sheet::parse_range;
use crate::write_v03::{Patches, Warn};
use crate::xml::{Elem, Ev, XmlReader};
use crate::XlsxError;

/// Most `filter` values read per column (as `Op::SetColumnFilter` allows).
const MAX_VALUES: usize = 100_000;
/// Most bytes of `filter` value text per column (as the engine allows).
const MAX_VALUE_BYTES: usize = 16 * 1024 * 1024;
/// Most `filter` values per sheet, over all its columns.
const MAX_SHEET_VALUES: usize = 1_000_000;
/// Most bytes of `filter` value text per sheet.
const MAX_SHEET_VALUE_BYTES: usize = 64 * 1024 * 1024;
/// Longest value text kept from a `filter` or `customFilter`.
const MAX_VALUE_LEN: usize = 32_767;

/// Escapes an attribute value. Tab, LF and CR become character references
/// (attribute normalisation would turn them into spaces); other control
/// characters, which XML 1.0 cannot hold, become OOXML's `_xHHHH_` (and a
/// literal `_xHHHH_` gets its `_` escaped as `_x005F_`), as Excel writes
/// them.
fn xml_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(ch) = rest.chars().next() {
        if ch == '_' && is_x_escape(rest) {
            out.push_str("_x005F_");
            rest = &rest[1..];
            continue;
        }
        match ch {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\t' => out.push_str("&#9;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            c if (c as u32) < 0x20 || c == '\u{FFFE}' || c == '\u{FFFF}' => {
                out.push_str(&format!("_x{:04X}_", c as u32));
            }
            c => out.push(c),
        }
        rest = &rest[ch.len_utf8()..];
    }
    out
}

/// Escapes element text.
fn xml_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// `true` if `s` starts with `_xHHHH_`.
pub(crate) fn is_x_escape(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 7
        && b[0] == b'_'
        && (b[1] == b'x' || b[1] == b'X')
        && b[2..6].iter().all(u8::is_ascii_hexdigit)
        && b[6] == b'_'
}

/// Decodes OOXML's `_xHHHH_` escapes.
pub(crate) fn decode_x_escapes(s: &str) -> String {
    if !s.contains("_x") && !s.contains("_X") {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(ch) = rest.chars().next() {
        if ch == '_' && is_x_escape(rest) {
            let code = u32::from_str_radix(&rest[2..6], 16).unwrap_or(0xFFFD);
            out.push(char::from_u32(code).unwrap_or('\u{FFFD}'));
            rest = &rest[7..];
        } else {
            out.push(ch);
            rest = &rest[ch.len_utf8()..];
        }
    }
    out
}

/// Escapes Excel's wildcards (`*`, `?`, `~`) in a custom filter value.
fn escape_wildcards(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if matches!(ch, '*' | '?' | '~') {
            out.push('~');
        }
        out.push(ch);
    }
    out
}

/// One piece of an Excel wildcard pattern.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Wild {
    Lit(char),
    Star,
    Any,
}

/// Splits a custom filter value into literals and wildcards (`~` escapes
/// the next character).
fn wild_tokens(s: &str) -> Vec<Wild> {
    let mut out = Vec::new();
    let mut chars = s.chars();
    while let Some(ch) = chars.next() {
        out.push(match ch {
            '~' => Wild::Lit(chars.next().unwrap_or('~')),
            '*' => Wild::Star,
            '?' => Wild::Any,
            c => Wild::Lit(c),
        });
    }
    out
}

/// The literal text of `t`, `None` if it holds a wildcard.
fn literal(t: &[Wild]) -> Option<String> {
    t.iter()
        .map(|w| match w {
            Wild::Lit(c) => Some(*c),
            _ => None,
        })
        .collect()
}

/// The `<filterColumn>` children of one column filter (`None` when the
/// column cannot be written).
fn column_xml(c: &FilterColumnData) -> Option<String> {
    if let Some(values) = &c.values {
        let blank = values.iter().any(String::is_empty);
        let mut s = String::from(if blank {
            r#"<filters blank="1">"#
        } else {
            "<filters>"
        });
        for v in values.iter().filter(|v| !v.is_empty()) {
            s.push_str(&format!(r#"<filter val="{}"/>"#, xml_attr(v)));
        }
        s.push_str("</filters>");
        return Some(s);
    }
    let cond = c.condition.as_ref()?;
    let v = cond.value.trim();
    let custom = |op: Option<&str>, val: String| {
        let op = op.map_or(String::new(), |o| format!(r#" operator="{o}""#));
        format!(
            r#"<customFilters><customFilter{op} val="{}"/></customFilters>"#,
            xml_attr(&val)
        )
    };
    let top10 = |top: bool, percent: bool| {
        let n = v.parse::<f64>().ok().filter(|n| n.is_finite() && *n >= 1.0);
        let n = n.map_or(10, |n| n.min(500.0) as u32);
        format!(
            r#"<top10 top="{}" val="{n}" percent="{}"/>"#,
            u8::from(top),
            u8::from(percent)
        )
    };
    Some(match cond.op {
        FilterOpData::Eq => custom(None, escape_wildcards(v)),
        FilterOpData::Ne => custom(Some("notEqual"), escape_wildcards(v)),
        FilterOpData::Gt => custom(Some("greaterThan"), v.to_string()),
        FilterOpData::Ge => custom(Some("greaterThanOrEqual"), v.to_string()),
        FilterOpData::Lt => custom(Some("lessThan"), v.to_string()),
        FilterOpData::Le => custom(Some("lessThanOrEqual"), v.to_string()),
        FilterOpData::Contains => custom(None, format!("*{}*", escape_wildcards(v))),
        FilterOpData::BeginsWith => custom(None, format!("{}*", escape_wildcards(v))),
        FilterOpData::EndsWith => custom(None, format!("*{}", escape_wildcards(v))),
        FilterOpData::Empty => r#"<filters blank="1"/>"#.to_string(),
        FilterOpData::NotEmpty => custom(Some("notEqual"), " ".to_string()),
        FilterOpData::TopValues => top10(true, false),
        FilterOpData::BottomValues => top10(false, false),
        FilterOpData::TopPercent => top10(true, true),
        FilterOpData::BottomPercent => top10(false, true),
    })
}

/// The `<autoFilter>` element of a sheet.
fn autofilter_xml(af: &AutoFilterData, sheet: &str, report: &mut dyn Warn) -> String {
    let r = &af.range;
    let mut s = format!(
        r#"<autoFilter ref="{}{}:{}{}">"#,
        col_to_letters(r.c0),
        u64::from(r.r0) + 1,
        col_to_letters(r.c1),
        u64::from(r.r1) + 1
    );
    let mut cols: Vec<&FilterColumnData> = af
        .columns
        .iter()
        .filter(|c| c.col >= r.c0 && c.col <= r.c1)
        .collect();
    cols.sort_by_key(|c| c.col);
    for c in cols {
        if c.values.is_some() && c.condition.is_some() {
            report.warn(format!(
                "sheet {sheet:?}: an AutoFilter column with both a value list and a condition \
                 was saved with its value list only (Excel keeps one filter per column)"
            ));
        }
        if let Some(inner) = column_xml(c) {
            s.push_str(&format!(r#"<filterColumn colId="{}">"#, c.col - r.c0));
            s.push_str(&inner);
            s.push_str("</filterColumn>");
        }
    }
    s.push_str("</autoFilter>");
    s
}

/// Adds each sheet's `<autoFilter>` through the patches pass.
pub(crate) fn autofilter_patches(
    data: &WorkbookData,
    patches: &mut Patches,
    report: &mut dyn Warn,
) {
    for (i, sheet) in data.sheets.iter().enumerate() {
        if let Some(af) = &sheet.auto_filter {
            if af.range.r0 > af.range.r1 || af.range.c0 > af.range.c1 || af.range.c1 >= MAX_COLS {
                continue;
            }
            patches.sheet_inserts.push((
                format!("xl/worksheets/sheet{}.xml", i + 1),
                autofilter_xml(af, &sheet.name, report),
            ));
            // Excel's hidden per-sheet name for the filter area.
            let r = &af.range;
            patches.defined_names.push(format!(
                r#"<definedName name="_xlnm._FilterDatabase" localSheetId="{i}" hidden="1">{}</definedName>"#,
                xml_text(&format!(
                    "'{}'!${}${}:${}${}",
                    sheet.name.replace('\'', "''"),
                    col_to_letters(r.c0),
                    u64::from(r.r0) + 1,
                    col_to_letters(r.c1),
                    u64::from(r.r1) + 1
                ))
            ));
        }
    }
}

/// The filter condition of a `customFilter` (Excel's wildcards become
/// contains / begins with / ends with).
fn custom_condition(op: &str, val: &str) -> Option<FilterConditionData> {
    let val: String = val.chars().take(MAX_VALUE_LEN).collect();
    let cond = |op, value: String| Some(FilterConditionData { op, value });
    match op {
        "equal" => {
            let t = wild_tokens(&val);
            if let Some(v) = literal(&t) {
                return cond(FilterOpData::Eq, v);
            }
            let (head, tail) = (
                t.first() == Some(&Wild::Star),
                t.last() == Some(&Wild::Star),
            );
            let inner = &t[usize::from(head)..t.len() - usize::from(tail && t.len() > 1)];
            let v = literal(inner)?;
            match (head, tail && t.len() > 1) {
                (true, true) => cond(FilterOpData::Contains, v),
                (false, true) => cond(FilterOpData::BeginsWith, v),
                (true, false) => cond(FilterOpData::EndsWith, v),
                (false, false) => None,
            }
        }
        "notEqual" if val.trim().is_empty() => cond(FilterOpData::NotEmpty, String::new()),
        "notEqual" => cond(FilterOpData::Ne, literal(&wild_tokens(&val))?),
        "greaterThan" => cond(FilterOpData::Gt, val),
        "greaterThanOrEqual" => cond(FilterOpData::Ge, val),
        "lessThan" => cond(FilterOpData::Lt, val),
        "lessThanOrEqual" => cond(FilterOpData::Le, val),
        _ => None,
    }
}

/// Parses `<autoFilter>` (its `Start` was just read).
pub(crate) fn parse_autofilter<R: BufRead>(
    xr: &mut XmlReader<R>,
    e: &Elem,
    warnings: &mut Vec<String>,
) -> Result<Option<AutoFilterData>, XlsxError> {
    let Some(range) = e.attr_local("ref").and_then(parse_range) else {
        xr.skip()?;
        return Ok(None);
    };
    let mut af = AutoFilterData {
        range,
        columns: Vec::new(),
        filtered_rows: Vec::new(),
    };
    let d = xr.depth();
    let mut col: Option<FilterColumnData> = None;
    // `filters blank="1"`: the empty entry goes last, as Tally lists it.
    let mut blank = false;
    // Value budgets: this column's bytes, the sheet's values and bytes.
    let mut col_bytes = 0usize;
    let (mut sheet_values, mut sheet_bytes) = (0usize, 0usize);
    let mut dropped_values = false;
    // `customFilter`s of the current column (Tally keeps one condition).
    let mut customs = 0u32;
    let mut two_conditions = 0u32;
    loop {
        match xr.next()? {
            Ev::Start(c) => match c.local() {
                "filterColumn" => {
                    let id = c
                        .attr_local("colId")
                        .and_then(|v| v.trim().parse::<u32>().ok());
                    col = id
                        .and_then(|id| range.c0.checked_add(id))
                        .filter(|c| *c <= range.c1)
                        .map(|col| FilterColumnData {
                            col,
                            values: None,
                            condition: None,
                        });
                    col_bytes = 0;
                    customs = 0;
                    if col.is_none() {
                        xr.skip()?;
                    }
                }
                "filters" => {
                    if let Some(fc) = col.as_mut() {
                        blank = c.attr_local("blank").is_some_and(crate::units::parse_bool);
                        fc.values = Some(Vec::new());
                    }
                }
                "filter" => {
                    if let (Some(fc), Some(v)) = (col.as_mut(), c.attr_local("val")) {
                        if let Some(list) = fc.values.as_mut() {
                            let v: String =
                                decode_x_escapes(v).chars().take(MAX_VALUE_LEN).collect();
                            if list.len() < MAX_VALUES
                                && col_bytes + v.len() <= MAX_VALUE_BYTES
                                && sheet_values < MAX_SHEET_VALUES
                                && sheet_bytes + v.len() <= MAX_SHEET_VALUE_BYTES
                            {
                                col_bytes += v.len();
                                sheet_values += 1;
                                sheet_bytes += v.len();
                                list.push(v);
                            } else {
                                dropped_values = true;
                            }
                        }
                    }
                    xr.skip()?;
                }
                "customFilter" => {
                    if let Some(fc) = col.as_mut() {
                        customs += 1;
                        if fc.condition.is_none() {
                            let op = c.attr_local("operator").unwrap_or("equal");
                            let val = decode_x_escapes(c.attr_local("val").unwrap_or_default());
                            fc.condition = custom_condition(op, &val);
                        }
                    }
                    xr.skip()?;
                }
                "top10" => {
                    if let Some(fc) = col.as_mut() {
                        let flag = |k: &str, default: bool| {
                            c.attr_local(k).map_or(default, crate::units::parse_bool)
                        };
                        let op = match (flag("top", true), flag("percent", false)) {
                            (true, false) => FilterOpData::TopValues,
                            (false, false) => FilterOpData::BottomValues,
                            (true, true) => FilterOpData::TopPercent,
                            (false, true) => FilterOpData::BottomPercent,
                        };
                        let n = c
                            .attr_local("val")
                            .and_then(|v| v.trim().parse::<f64>().ok())
                            .filter(|n| n.is_finite() && *n >= 1.0)
                            .map_or(10, |n| n.min(1e9) as u64);
                        fc.condition = Some(FilterConditionData {
                            op,
                            value: n.to_string(),
                        });
                    }
                    xr.skip()?;
                }
                "customFilters" => {}
                _ => xr.skip()?,
            },
            Ev::End if xr.depth() < d => break,
            Ev::End => {
                // The end of a `filterColumn` (depth d + 1 → d).
                if xr.depth() == d {
                    if let Some(mut fc) = col.take() {
                        if std::mem::take(&mut blank) {
                            if let Some(list) = fc.values.as_mut() {
                                list.push(String::new());
                            }
                        }
                        if customs > 1 {
                            // Two conditions (`and`/or): Tally's AutoFilter
                            // column holds one, so the column is not filtered
                            // rather than filtered differently.
                            two_conditions += 1;
                        } else if (fc.values.is_some() || fc.condition.is_some())
                            && !af.columns.iter().any(|x| x.col == fc.col)
                        {
                            af.columns.push(fc);
                        }
                    }
                }
            }
            Ev::Eof => break,
            Ev::Text(_) => {}
        }
    }
    if dropped_values {
        warnings.push(format!(
            "An AutoFilter lists more values than Tally keeps ({MAX_VALUES} or {} MiB per \
             column, {MAX_SHEET_VALUES} per sheet); the rest were dropped.",
            MAX_VALUE_BYTES >> 20
        ));
    }
    if two_conditions > 0 {
        warnings.push(format!(
            "{two_conditions} AutoFilter column(s) with two conditions are not supported and \
             were opened unfiltered (use Data ▸ More Filters ▸ Standard Filter)."
        ));
    }
    Ok(Some(af))
}

/// Rows Excel hid inside a filtered AutoFilter area become the filter's
/// rows (not user-hidden rows), as ODS `table:visibility="filter"` does.
pub(crate) fn attach_filtered_rows(sheet: &mut SheetData) {
    let Some(af) = sheet.auto_filter.as_mut() else {
        return;
    };
    if af.columns.is_empty() || af.range.r0 >= af.range.r1 {
        return;
    }
    let (b0, b1) = (af.range.r0 + 1, af.range.r1);
    let mut runs: Vec<DimRun> = Vec::with_capacity(sheet.row_heights.len());
    let mut spans: Vec<(u32, u32)> = Vec::new();
    for run in sheet.row_heights.drain(..) {
        if !run.hidden || run.end < b0 || run.start > b1 {
            runs.push(run);
            continue;
        }
        let (s, e) = (run.start.max(b0), run.end.min(b1));
        if run.start < s {
            runs.push(DimRun {
                end: s - 1,
                ..run.clone()
            });
        }
        runs.push(DimRun {
            start: s,
            end: e,
            hidden: false,
            ..run.clone()
        });
        if run.end > e {
            runs.push(DimRun {
                start: e + 1,
                ..run.clone()
            });
        }
        match spans.last_mut() {
            Some(last) if s <= last.1.saturating_add(1) => last.1 = last.1.max(e),
            _ => spans.push((s, e)),
        }
    }
    sheet.row_heights = runs;
    af.filtered_rows = spans;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_filter_wildcards() {
        let c = |op, v| custom_condition(op, v).map(|c| (c.op, c.value));
        assert_eq!(
            c("equal", "*ab*"),
            Some((FilterOpData::Contains, "ab".into()))
        );
        assert_eq!(
            c("equal", "ab*"),
            Some((FilterOpData::BeginsWith, "ab".into()))
        );
        assert_eq!(
            c("equal", "*ab"),
            Some((FilterOpData::EndsWith, "ab".into()))
        );
        assert_eq!(c("equal", "a~*b"), Some((FilterOpData::Eq, "a*b".into())));
        assert_eq!(c("equal", "a?b"), None);
        assert_eq!(
            c("notEqual", " "),
            Some((FilterOpData::NotEmpty, String::new()))
        );
        assert_eq!(c("greaterThan", "3"), Some((FilterOpData::Gt, "3".into())));
        assert_eq!(escape_wildcards("a*b?c~"), "a~*b~?c~~");
        // An escaped final `*` is part of the text (review L2).
        assert_eq!(
            c("equal", "*ab~*"),
            Some((FilterOpData::EndsWith, "ab*".into()))
        );
        assert_eq!(
            c("equal", "~*ab*"),
            Some((FilterOpData::BeginsWith, "*ab".into()))
        );
        assert_eq!(
            c("equal", "*a~~*"),
            Some((FilterOpData::Contains, "a~".into()))
        );
        assert_eq!(c("equal", "*a*b*"), None);
    }

    #[test]
    fn control_characters_are_escaped_and_decoded() {
        let s = "a\tb\nc\rd\u{1}e_x0041_f&<\"";
        let x = xml_attr(s);
        assert_eq!(x, "a&#9;b&#10;c&#13;d_x0001_e_x005F_x0041_f&amp;&lt;&quot;");
        // The reader unescapes the entities, then `_xHHHH_`.
        let unentity = x
            .replace("&#9;", "\t")
            .replace("&#10;", "\n")
            .replace("&#13;", "\r")
            .replace("&amp;", "&")
            .replace("&lt;", "<")
            .replace("&quot;", "\"");
        assert_eq!(decode_x_escapes(&unentity), s);
    }
}
