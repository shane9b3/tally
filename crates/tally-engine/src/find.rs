//! Find & Replace (inventory §23).
//!
//! - Values mode searches the displayed (formatted) text of every non-empty
//!   cell; formulas mode searches the formula text (`=…`) of formula cells
//!   and the edit text of constants.
//! - Literal, wildcard (`*`, `?`, `~` escapes) and regular-expression
//!   patterns, all compiled to a size-limited `regex`; `match_case` and
//!   `whole_cell` apply to every mode.
//! - Results are row-major per sheet, sheets in tab order, at most
//!   [`MAX_HITS`]; `from` rotates them so the first hit follows that cell.
//! - Replace All rewrites constants through the input recognizer and
//!   formulas through the parser; a replacement that would not parse keeps
//!   the old formula and is reported as an `OpError` warning. Array
//!   formula cells are skipped (reported). Values mode never touches
//!   formulas. Undo restores every changed cell.

use regex::{Regex, RegexBuilder};
use tally_api::{ApiError, FindHit, FindMode, FindQuery, FindResult};
use tally_types::{CellAddr, SheetId};

use crate::service::view::{cell_edit_info, display_text};
use crate::storage::{CellContent, ColumnStore};
use crate::undo::{capture_range, Change, Effects};
use crate::workbook::Workbook;

/// Most hits returned by [`find`].
pub const MAX_HITS: usize = 10_000;
/// Most cells one Replace All rewrites.
pub const MAX_REPLACE: usize = 1_000_000;
/// Compiled-regex size limit (bytes).
pub const REGEX_SIZE_LIMIT: usize = 1 << 20;
/// Longest find pattern or replacement accepted (bytes), checked before
/// compiling (security review 2, LOW-3: a 1 MB pattern used 640 MB while
/// the regex compiler rejected it).
pub const MAX_PATTERN_BYTES: usize = 64 * 1024;
/// Most cells the Replace All ops of one transaction rewrite together (the
/// undo record holds every old cell).
pub const MAX_REPLACE_TX: u64 = 10_000_000;
/// Longest text a replacement may produce in one cell (Excel's cell limit).
pub const MAX_REPLACED_CELL_CHARS: usize = 32_767;
/// Most bytes of new text one Replace All may produce in total.
pub const MAX_REPLACE_BYTES: usize = 256 * 1024 * 1024;

fn wildcard_to_regex(p: &str) -> String {
    let mut out = String::with_capacity(p.len() * 2);
    let mut chars = p.chars();
    while let Some(c) = chars.next() {
        match c {
            '*' => out.push_str(".*"),
            '?' => out.push('.'),
            '~' => match chars.next() {
                Some(n) => out.push_str(&regex::escape(&n.to_string())),
                None => out.push_str(&regex::escape("~")),
            },
            other => out.push_str(&regex::escape(&other.to_string())),
        }
    }
    out
}

/// Compiles the query's pattern.
pub fn build_regex(q: &FindQuery) -> Result<Regex, ApiError> {
    if q.text.is_empty() {
        return Err(ApiError::invalid_argument("nothing to find"));
    }
    if q.text.len() > MAX_PATTERN_BYTES {
        return Err(ApiError::invalid_argument(format!(
            "the search text is longer than {} KiB",
            MAX_PATTERN_BYTES / 1024
        )));
    }
    let p = match q.mode {
        FindMode::Literal => regex::escape(&q.text),
        FindMode::Wildcards => wildcard_to_regex(&q.text),
        FindMode::Regex => q.text.clone(),
    };
    let p = if q.whole_cell {
        format!("^(?:{p})$")
    } else {
        p
    };
    RegexBuilder::new(&p)
        .case_insensitive(!q.match_case)
        .size_limit(REGEX_SIZE_LIMIT)
        .dfa_size_limit(REGEX_SIZE_LIMIT)
        .build()
        .map_err(|e| ApiError::invalid_argument(format!("bad pattern: {e}")))
}

/// The text a query looks at in one cell (`None` to skip the cell).
fn searched_text(
    wb: &Workbook,
    addr: CellAddr,
    content: &CellContent,
    in_formulas: bool,
) -> Option<String> {
    if in_formulas {
        // Hidden formulas on a protected sheet are not searchable
        // (security review 3, LOW-9), as they are not shown or copied.
        let hidden = || {
            crate::protection::sheet_protection(wb, addr.sheet).is_some()
                && wb.sheet(addr.sheet).is_some_and(|s| {
                    crate::protection::cell_protection(wb, s.style(addr.row, addr.col))
                        .hidden_formula
                })
        };
        match content {
            CellContent::Formula(_) if hidden() => None,
            CellContent::Formula(id) => wb.formula_text(*id),
            CellContent::ArrayPart(_) => None,
            CellContent::Value(_) => cell_edit_info(wb, addr.sheet, addr.row, addr.col)
                .ok()
                .map(|i| i.input),
        }
    } else {
        let t = display_text(wb, addr);
        (!t.is_empty()).then_some(t)
    }
}

/// Every matching cell in `sheets` order, row-major, at most `cap`.
fn scan(
    wb: &Workbook,
    sheets: &[SheetId],
    re: &Regex,
    in_formulas: bool,
    cap: usize,
) -> (Vec<FindHit>, bool) {
    let mut hits = Vec::new();
    for &sid in sheets {
        let Some(sheet) = wb.sheet(sid) else {
            continue;
        };
        let mut cells: Vec<(u32, u32)> = Vec::new();
        for (ci, col) in sheet.columns.iter().enumerate() {
            cells.extend(
                col.cells
                    .iter_range(0, tally_types::MAX_ROWS - 1)
                    .map(|(r, _)| (r, ci as u32)),
            );
        }
        cells.sort_unstable();
        for (r, c) in cells {
            let Some(content) = sheet.get(r, c) else {
                continue;
            };
            let addr = CellAddr::new(sid, r, c);
            if let Some(t) = searched_text(wb, addr, content, in_formulas) {
                if re.is_match(&t) {
                    if hits.len() >= cap {
                        return (hits, true);
                    }
                    hits.push(FindHit { sheet: sid, r, c });
                }
            }
        }
    }
    (hits, false)
}

fn sheets_for(wb: &Workbook, sheet: Option<SheetId>) -> Vec<SheetId> {
    match sheet {
        Some(s) => vec![s],
        None => wb.sheets.iter().map(|s| s.id).collect(),
    }
}

/// Runs a find query.
pub fn find(wb: &Workbook, q: &FindQuery) -> Result<FindResult, ApiError> {
    if let Some(s) = q.sheet {
        if wb.sheet(s).is_none() {
            return Err(ApiError::not_found(format!("no sheet {s}")));
        }
    }
    let re = build_regex(q)?;
    let sheets = sheets_for(wb, q.sheet);
    let (mut hits, mut truncated) = scan(wb, &sheets, &re, q.in_formulas, MAX_REPLACE);
    if let Some(from) = q.from {
        let fs = q.sheet.unwrap_or(wb.active_sheet);
        let tab = |s: SheetId| wb.tab_of(s).unwrap_or(usize::MAX);
        let ft = tab(fs);
        let split = hits.iter().position(|h| {
            let t = tab(h.sheet);
            t > ft || (t == ft && (h.r, h.c) > (from.r, from.c))
        });
        if let Some(i) = split {
            hits.rotate_left(i);
        }
    }
    if hits.len() > MAX_HITS {
        hits.truncate(MAX_HITS);
        truncated = true;
    }
    Ok(FindResult { hits, truncated })
}

/// Converts an AOO-style replacement (`$1`, `&` = whole match, `\&` = `&`)
/// for `regex`; literal and wildcard modes replace literally.
fn replacement(mode: FindMode, rep: &str) -> String {
    if mode != FindMode::Regex {
        return rep.replace('$', "$$");
    }
    let mut out = String::with_capacity(rep.len() + 8);
    let mut chars = rep.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'&') => {
                chars.next();
                out.push('&');
            }
            '&' => out.push_str("${0}"),
            '$' if chars.peek().is_some_and(char::is_ascii_digit) => {
                // `$1abc` would name group "1abc" in `regex`; brace it.
                let mut digits = String::new();
                while let Some(d) = chars.peek().copied().filter(char::is_ascii_digit) {
                    digits.push(d);
                    chars.next();
                }
                out.push_str(&format!("${{{digits}}}"));
            }
            other => out.push(other),
        }
    }
    out
}

/// Replaces every match (see the module docs).
pub fn replace_all(
    wb: &mut Workbook,
    sheet: Option<SheetId>,
    query: &FindQuery,
    replace: &str,
    fx: &mut Effects,
) -> Result<Vec<Change>, ApiError> {
    let re = build_regex(query)?;
    if replace.len() > MAX_PATTERN_BYTES {
        return Err(ApiError::invalid_argument(format!(
            "the replacement is longer than {} KiB",
            MAX_PATTERN_BYTES / 1024
        )));
    }
    let done = u64::from(fx.replaced.unwrap_or(0));
    let left = MAX_REPLACE_TX.saturating_sub(done).min(MAX_REPLACE as u64) as usize;
    if left == 0 {
        return Err(ApiError::unsupported(format!(
            "one step may replace at most {MAX_REPLACE_TX} cells"
        )));
    }
    let rep = replacement(query.mode, replace);
    let sheets = sheets_for(wb, sheet);
    let (hits, _) = scan(wb, &sheets, &re, query.in_formulas, left);
    let mut changes = Vec::new();
    let mut failed = 0usize;
    let mut rejected = 0usize;
    let mut skipped_arrays = 0usize;
    let mut too_long = 0usize;
    let mut bytes = 0usize;
    for h in hits {
        let addr = CellAddr::new(h.sheet, h.r, h.c);
        let Some(content) = wb.sheet(h.sheet).and_then(|s| s.get(h.r, h.c)).cloned() else {
            continue;
        };
        let old_text = match &content {
            CellContent::ArrayPart(_) => {
                skipped_arrays += 1;
                continue;
            }
            CellContent::Formula(id) => {
                if !query.in_formulas {
                    continue;
                }
                if wb.formulas.get(*id).is_some_and(|f| f.array.is_some()) {
                    skipped_arrays += 1;
                    continue;
                }
                wb.formula_text(*id).unwrap_or_default()
            }
            CellContent::Value(_) => cell_edit_info(wb, h.sheet, h.r, h.c)
                .map(|i| i.input)
                .unwrap_or_default(),
        };
        let new_text = re.replace_all(&old_text, rep.as_str()).into_owned();
        if new_text == old_text {
            continue;
        }
        if new_text.chars().count() > MAX_REPLACED_CELL_CHARS {
            too_long += 1;
            continue;
        }
        bytes += new_text.len();
        if bytes > MAX_REPLACE_BYTES {
            fx.warnings.push(ApiError::unsupported(format!(
                "Replace All stopped after {} cells: the replacements would hold more \
                 than {} MiB of text",
                changes.len(),
                MAX_REPLACE_BYTES >> 20
            )));
            break;
        }
        if new_text.starts_with('=')
            && new_text.len() > 1
            && wb.parse_formula(&new_text, addr).is_err()
        {
            failed += 1;
            if failed <= 20 {
                fx.warnings.push(ApiError::invalid_argument(format!(
                    "{}: {new_text:?} is not a valid formula; the cell was left unchanged",
                    addr.to_a1()
                )));
            }
            continue;
        }
        let range = tally_types::CellRange::single(addr);
        let snapshot = capture_range(wb, range, true, true);
        wb.set_input(addr, &new_text);
        // Validation: a Stop rule refuses this cell (the rest of the
        // replace goes on), Warning/Info accept it with a notice (M12).
        if let Some(val) = crate::validation::at(wb, h.sheet, h.r, h.c).cloned() {
            if let Some(err) = &val.error {
                let v = crate::validation::current_value(wb, addr);
                let ok = v.is_empty() && val.allow_blank
                    || !v.is_empty()
                        && crate::validation::is_valid_input(wb, &val, addr, &v, Some(&new_text));
                if !ok {
                    let text = if err.text.trim().is_empty() {
                        "The value you entered is not valid.".to_string()
                    } else {
                        err.text.clone()
                    };
                    match err.style {
                        tally_model::ValidationErrorStyleData::Stop => {
                            crate::undo::restore_range(wb, snapshot.clone());
                            rejected += 1;
                            if rejected <= 20 {
                                fx.notices.push(ApiError::new(
                                    crate::validation::VALIDATION,
                                    format!("{}: {text} (not replaced)", addr.to_a1()),
                                ));
                            }
                            continue;
                        }
                        _ => fx.notices.push(ApiError::new(
                            crate::validation::VALIDATION,
                            format!("{}: {text}", addr.to_a1()),
                        )),
                    }
                }
            }
        }
        changes.push(Change::Range(snapshot));
    }
    if rejected > 20 {
        fx.notices.push(ApiError::new(
            crate::validation::VALIDATION,
            format!(
                "{} more cells were not replaced (validation)",
                rejected - 20
            ),
        ));
    }
    if failed > 20 {
        fx.warnings.push(ApiError::invalid_argument(format!(
            "{} more replacements were not valid formulas",
            failed - 20
        )));
    }
    if too_long > 0 {
        fx.warnings.push(ApiError::invalid_argument(format!(
            "{too_long} cells were left unchanged: the result would be longer than \
             {MAX_REPLACED_CELL_CHARS} characters"
        )));
    }
    if skipped_arrays > 0 {
        fx.warnings.push(ApiError::new(
            crate::ops::ARRAY_BLOCK,
            format!("{skipped_arrays} cells of array formulas were skipped"),
        ));
    }
    *fx.replaced.get_or_insert(0) += u32::try_from(changes.len()).unwrap_or(u32::MAX);
    Ok(changes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(text: &str, mode: FindMode) -> FindQuery {
        FindQuery {
            text: text.into(),
            sheet: None,
            in_formulas: false,
            match_case: false,
            whole_cell: false,
            mode,
            from: None,
        }
    }

    #[test]
    fn patterns() {
        let re = build_regex(&q("a*c", FindMode::Wildcards)).unwrap();
        assert!(re.is_match("xABCx"));
        let re = build_regex(&FindQuery {
            whole_cell: true,
            ..q("a?c", FindMode::Wildcards)
        })
        .unwrap();
        assert!(re.is_match("abc") && !re.is_match("abcd"));
        let re = build_regex(&q("1.5", FindMode::Literal)).unwrap();
        assert!(!re.is_match("125"));
        let re = build_regex(&q("~*", FindMode::Wildcards)).unwrap();
        assert!(re.is_match("a*b") && !re.is_match("ab"));
        assert!(build_regex(&q("(", FindMode::Regex)).is_err());
        assert!(build_regex(&q("", FindMode::Literal)).is_err());
        // Pathological patterns hit the size limit instead of exploding.
        assert!(build_regex(&q("(a{1000}){1000}", FindMode::Regex)).is_err());
    }

    #[test]
    fn replacements() {
        assert_eq!(replacement(FindMode::Literal, "$1"), "$$1");
        assert_eq!(replacement(FindMode::Regex, "[&]\\&$1x"), "[${0}]&${1}x");
    }
}
