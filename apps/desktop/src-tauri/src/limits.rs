//! Argument validation for the commands (WP7): every text, count and
//! coordinate the webview sends is checked here before the engine sees it,
//! and a violation rejects the whole command with `invalid_argument`
//! (docs/security.md, "Argument limits").
//!
//! - Find and replace text, regular expressions, cell input, formulas and
//!   any other string: at most [`MAX_TEXT`] bytes (64 KiB).
//! - Names (defined names, sheet names): at most [`MAX_NAME_CHARS`] chars.
//! - A TSV paste payload: at most [`MAX_TSV`] bytes (64 MiB); every string of
//!   one transaction together: at most [`MAX_TSV`] bytes.
//! - Ranges and cell positions: inside the sheet (`r0 ≤ r1 < MAX_ROWS`,
//!   `c0 ≤ c1 < MAX_COLS`), at most [`MAX_RANGES`] per list.
//!
//! Ops are checked twice: typed checks for the fields with their own
//! limits (names, row/column spans, TSV payloads) and a structural walk of
//! the serialized op that catches every string and range, so a new op
//! variant is covered without a change here.

use serde_json::Value;
use tally_api::{
    ApiError, CellPos, FindQuery, GoalSeekRequest, Op, PastePayload, RangeDto, SelectionDto,
    Transaction,
};
use tally_types::{MAX_COLS, MAX_ROWS};

use crate::commands::CmdResult;

/// Longest string argument in bytes (find text, regex, cell input, formulas).
pub const MAX_TEXT: usize = 64 * 1024;
/// Longest name (defined name, sheet name) in characters.
pub const MAX_NAME_CHARS: usize = 256;
/// Longest name box text in characters (a name, or a reference with a sheet name).
pub const MAX_NAME_BOX_CHARS: usize = 1024;
/// Largest TSV paste payload, and all strings of one transaction together, in bytes.
pub const MAX_TSV: usize = 64 * 1024 * 1024;
/// Most ranges in one argument or op.
pub const MAX_RANGES: usize = 10_000;
/// Most style ids in one `get_styles` call.
pub const MAX_STYLE_IDS: usize = 65_536;
/// Most ops in one transaction.
pub const MAX_OPS: usize = 100_000;
/// Most sort keys.
pub const MAX_SORT_KEYS: usize = 64;
/// Longest file path or template id.
pub const MAX_PATH: usize = 4096;
/// Longest locale tag.
pub const MAX_LOCALE: usize = 64;

fn bad(message: impl Into<String>) -> ApiError {
    ApiError::invalid_argument(message)
}

/// `text` is at most [`MAX_TEXT`] bytes.
pub fn text(what: &str, text: &str) -> CmdResult<()> {
    text_max(what, text, MAX_TEXT)
}

/// `text` is at most `max` bytes.
pub fn text_max(what: &str, text: &str, max: usize) -> CmdResult<()> {
    if text.len() > max {
        return Err(bad(format!(
            "{what} is too long ({} bytes; the limit is {max})",
            text.len()
        )));
    }
    Ok(())
}

/// A name (defined name, sheet name): at most [`MAX_NAME_CHARS`] characters.
pub fn name(what: &str, name: &str) -> CmdResult<()> {
    chars_max(what, name, MAX_NAME_CHARS)
}

/// At most `max` characters.
pub fn chars_max(what: &str, s: &str, max: usize) -> CmdResult<()> {
    // Cheap byte check first: `max` chars are at most `4 * max` bytes.
    if s.len() > max && (s.len() > 4 * max || s.chars().count() > max) {
        return Err(bad(format!(
            "{what} is too long (the limit is {max} characters)"
        )));
    }
    Ok(())
}

/// A row index.
pub fn row(what: &str, r: u32) -> CmdResult<()> {
    if r >= MAX_ROWS {
        return Err(bad(format!("{what}: row {r} is outside the sheet")));
    }
    Ok(())
}

/// A column index.
pub fn col(what: &str, c: u32) -> CmdResult<()> {
    if c >= MAX_COLS {
        return Err(bad(format!("{what}: column {c} is outside the sheet")));
    }
    Ok(())
}

/// A cell position.
pub fn pos(what: &str, r: u32, c: u32) -> CmdResult<()> {
    row(what, r)?;
    col(what, c)
}

/// A cell position DTO.
pub fn cell(what: &str, p: &CellPos) -> CmdResult<()> {
    pos(what, p.r, p.c)
}

/// A range inside the sheet with `r0 ≤ r1`, `c0 ≤ c1`.
pub fn range(what: &str, rg: &RangeDto) -> CmdResult<()> {
    if rg.r0 > rg.r1 || rg.c0 > rg.c1 {
        return Err(bad(format!("{what}: the range is reversed")));
    }
    pos(what, rg.r1, rg.c1)
}

/// A list of ranges: each valid, at most [`MAX_RANGES`].
pub fn ranges(what: &str, list: &[RangeDto]) -> CmdResult<()> {
    if list.len() > MAX_RANGES {
        return Err(bad(format!(
            "{what}: too many ranges ({}; the limit is {MAX_RANGES})",
            list.len()
        )));
    }
    list.iter().try_for_each(|r| range(what, r))
}

/// A row span `start..=end`.
fn row_span(what: &str, start: u32, end: u32) -> CmdResult<()> {
    if start > end {
        return Err(bad(format!("{what}: the span is reversed")));
    }
    row(what, end)
}

/// A column span `start..=end`.
fn col_span(what: &str, start: u32, end: u32) -> CmdResult<()> {
    if start > end {
        return Err(bad(format!("{what}: the span is reversed")));
    }
    col(what, end)
}

/// `count` rows or columns at `at` fit in the sheet (`at + count ≤ limit`).
fn insert_span(what: &str, at: u32, count: u32, limit: u32) -> CmdResult<()> {
    if u64::from(at) + u64::from(count) > u64::from(limit) {
        return Err(bad(format!("{what}: {count} at {at} is outside the sheet")));
    }
    Ok(())
}

/// A size in px: finite and not negative.
fn size_px(what: &str, px: f32) -> CmdResult<()> {
    if !px.is_finite() || px < 0.0 {
        return Err(bad(format!("{what}: invalid size")));
    }
    Ok(())
}

/// A selection: its ranges and active cell.
pub fn selection(what: &str, sel: &SelectionDto) -> CmdResult<()> {
    ranges(what, &sel.ranges)?;
    cell(what, &sel.active)
}

/// A find query (also inside `replaceAll`): the text (literal, wildcards or
/// a regular expression) at most [`MAX_TEXT`] bytes.
pub fn find_query(q: &FindQuery) -> CmdResult<()> {
    let what = match q.mode {
        tally_api::FindMode::Regex => "the regular expression",
        _ => "the search text",
    };
    text(what, &q.text)?;
    if let Some(from) = &q.from {
        cell("the search start", from)?;
    }
    Ok(())
}

/// A goal seek request.
pub fn goal_seek(req: &GoalSeekRequest) -> CmdResult<()> {
    cell("the formula cell", &req.formula_cell)?;
    cell("the variable cell", &req.variable_cell)?;
    if !req.target.is_finite() {
        return Err(bad("the target value is not a number"));
    }
    Ok(())
}

/// A transaction: op count, the typed per-op limits, then the structural
/// walk (strings, ranges, positions) and the total string size.
pub fn transaction(tx: &Transaction) -> CmdResult<()> {
    if tx.ops.len() > MAX_OPS {
        return Err(bad(format!(
            "too many ops in one transaction ({}; the limit is {MAX_OPS})",
            tx.ops.len()
        )));
    }
    if let Some(label) = &tx.label {
        chars_max("the undo label", label, MAX_NAME_CHARS)?;
    }
    for s in [&tx.selection_before, &tx.selection_after]
        .into_iter()
        .flatten()
    {
        selection("the selection", s)?;
    }
    let mut total = 0usize;
    for op in &tx.ops {
        typed_op(op)?;
        let v = serde_json::to_value(op)
            .map_err(|e| ApiError::new(ApiError::INTERNAL, format!("op: {e}")))?;
        walk(&v, false, &mut total)?;
    }
    Ok(())
}

/// The fields with limits of their own.
fn typed_op(op: &Op) -> CmdResult<()> {
    match op {
        Op::SetCellInput { r, c, input, .. } => {
            pos("the cell", *r, *c)?;
            text("the cell input", input)
        }
        Op::SetRangeInputs { r, c, rows, .. } => {
            pos("the block", *r, *c)?;
            let height = u32::try_from(rows.len()).unwrap_or(u32::MAX);
            let width = rows.iter().map(Vec::len).max().unwrap_or(0);
            let width = u32::try_from(width).unwrap_or(u32::MAX);
            insert_span("the block", *r, height, MAX_ROWS)?;
            insert_span("the block", *c, width, MAX_COLS)?;
            rows.iter()
                .flatten()
                .try_for_each(|s| text("the cell input", s))
        }
        Op::InsertRows { at, count, .. } | Op::DeleteRows { at, count, .. } => {
            insert_span("the rows", *at, *count, MAX_ROWS)
        }
        Op::InsertCols { at, count, .. } | Op::DeleteCols { at, count, .. } => {
            insert_span("the columns", *at, *count, MAX_COLS)
        }
        Op::SetColWidth { start, end, px, .. } => {
            col_span("the columns", *start, *end)?;
            size_px("the column width", *px)
        }
        Op::SetRowHeight { start, end, px, .. } => {
            row_span("the rows", *start, *end)?;
            size_px("the row height", *px)
        }
        Op::SetRowsHidden { start, end, .. } => row_span("the rows", *start, *end),
        Op::SetColsHidden { start, end, .. } => col_span("the columns", *start, *end),
        Op::AutoFitRows { start, end, .. } => row_span("the rows", *start, *end),
        Op::AddSheet { name: Some(n), .. } | Op::RenameSheet { name: n, .. } => {
            name("the sheet name", n)
        }
        Op::DefineName { name: n, .. } | Op::DeleteName { name: n, .. } => name("the name", n),
        Op::SetFrozen { rows, cols, .. } => {
            insert_span("the frozen rows", 0, *rows, MAX_ROWS)?;
            insert_span("the frozen columns", 0, *cols, MAX_COLS)
        }
        Op::SortRange { keys, .. } => {
            if keys.len() > MAX_SORT_KEYS {
                return Err(bad("too many sort keys"));
            }
            keys.iter().try_for_each(|k| col("the sort key", k.col))
        }
        Op::SetColumnFilter { col: c, .. } => col("the filter column", *c),
        Op::ReplaceAll { query, replace, .. } => {
            find_query(query)?;
            text("the replacement", replace)
        }
        Op::Paste { payload, .. } | Op::PasteSpecial { payload, .. } => match payload {
            PastePayload::Tsv { text: t } => text_max("the pasted text", t, MAX_TSV),
            PastePayload::Internal { token } => chars_max("the clipboard token", token, 256),
        },
        Op::SetPageBreak { axis, at, .. } => match axis {
            tally_api::BreakAxis::Row => row("the page break", *at),
            tally_api::BreakAxis::Col => col("the page break", *at),
        },
        Op::SetPageSetup { setup, .. } => {
            if let Some([a, b]) = setup.repeat_rows {
                row_span("the rows to repeat", a, b)?;
            }
            if let Some([a, b]) = setup.repeat_cols {
                col_span("the columns to repeat", a, b)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Walks a serialized op: every string at most [`MAX_TEXT`] bytes (a TSV
/// payload's text up to [`MAX_TSV`]), every `{r0,c0,r1,c1}` a valid range,
/// every `{r,c}` a valid position, every `ranges` list at most
/// [`MAX_RANGES`] long; `total` sums the string bytes (≤ [`MAX_TSV`]).
fn walk(v: &Value, tsv: bool, total: &mut usize) -> CmdResult<()> {
    match v {
        Value::String(s) => {
            *total += s.len();
            if *total > MAX_TSV {
                return Err(bad(format!(
                    "the transaction is too large (the limit is {MAX_TSV} bytes of text)"
                )));
            }
            text_max("a text argument", s, if tsv { MAX_TSV } else { MAX_TEXT })
        }
        Value::Array(items) => items.iter().try_for_each(|x| walk(x, false, total)),
        Value::Object(map) => {
            let num = |k: &str| map.get(k).and_then(Value::as_u64);
            if let (Some(r0), Some(c0), Some(r1), Some(c1)) =
                (num("r0"), num("c0"), num("r1"), num("c1"))
            {
                let fit = |x: u64| u32::try_from(x).unwrap_or(u32::MAX);
                range(
                    "a range",
                    &RangeDto {
                        r0: fit(r0),
                        c0: fit(c0),
                        r1: fit(r1),
                        c1: fit(c1),
                    },
                )?;
            } else if let (Some(r), Some(c)) = (num("r"), num("c")) {
                let fit = |x: u64| u32::try_from(x).unwrap_or(u32::MAX);
                pos("a cell", fit(r), fit(c))?;
            }
            if let Some(Value::Array(list)) = map.get("ranges") {
                if list.len() > MAX_RANGES {
                    return Err(bad(format!(
                        "too many ranges ({}; the limit is {MAX_RANGES})",
                        list.len()
                    )));
                }
            }
            let is_tsv = map.get("type").and_then(Value::as_str) == Some("tsv");
            for (k, x) in map {
                walk(x, is_tsv && k == "text", total)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tally_api::{FindMode, Op};

    fn tx(ops: Vec<Op>) -> Transaction {
        Transaction {
            ops,
            label: None,
            selection_before: None,
            selection_after: None,
        }
    }

    fn input(s: String) -> Op {
        Op::SetCellInput {
            sheet: 1,
            r: 0,
            c: 0,
            input: s,
        }
    }

    #[test]
    fn strings_and_names() {
        assert!(transaction(&tx(vec![input("x".repeat(MAX_TEXT))])).is_ok());
        let e = transaction(&tx(vec![input("x".repeat(MAX_TEXT + 1))])).unwrap_err();
        assert_eq!(e.code, ApiError::INVALID_ARGUMENT);
        let rename = |n: String| Op::RenameSheet { sheet: 1, name: n };
        assert!(transaction(&tx(vec![rename("é".repeat(256))])).is_ok());
        assert!(transaction(&tx(vec![rename("é".repeat(257))])).is_err());
        let define = Op::DefineName {
            name: "n".repeat(300),
            scope: None,
            refers_to: "1".into(),
        };
        assert!(transaction(&tx(vec![define])).is_err());
    }

    #[test]
    fn tsv_payload_may_be_large_but_not_huge() {
        let paste = |n: usize| Op::Paste {
            sheet: 1,
            target: RangeDto {
                r0: 0,
                c0: 0,
                r1: 0,
                c1: 0,
            },
            payload: PastePayload::Tsv {
                text: "a".repeat(n),
            },
        };
        assert!(transaction(&tx(vec![paste(1024 * 1024)])).is_ok());
        assert!(transaction(&tx(vec![paste(MAX_TSV + 1)])).is_err());
    }

    #[test]
    fn ranges_and_positions() {
        let clear = |rg: RangeDto| Op::Clear {
            sheet: 1,
            ranges: vec![rg],
            what: tally_api::ClearWhat::Contents,
        };
        let ok = RangeDto {
            r0: 0,
            c0: 0,
            r1: MAX_ROWS - 1,
            c1: MAX_COLS - 1,
        };
        assert!(transaction(&tx(vec![clear(ok)])).is_ok());
        assert!(transaction(&tx(vec![clear(RangeDto { r1: MAX_ROWS, ..ok })])).is_err());
        assert!(transaction(&tx(vec![clear(RangeDto { r0: 5, r1: 4, ..ok })])).is_err());
        let set = Op::SetCellInput {
            sheet: 1,
            r: 0,
            c: MAX_COLS,
            input: "1".into(),
        };
        assert!(transaction(&tx(vec![set])).is_err());
        let ins = Op::InsertRows {
            sheet: 1,
            at: MAX_ROWS - 1,
            count: 2,
        };
        assert!(transaction(&tx(vec![ins])).is_err());
        let note = Op::SetNote {
            sheet: 1,
            r: MAX_ROWS,
            c: 0,
            note: tally_api::NoteDto {
                r: MAX_ROWS,
                c: 0,
                text: "x".into(),
                author: None,
                created: None,
                shown: false,
            },
        };
        assert!(transaction(&tx(vec![note])).is_err());
    }

    #[test]
    fn find_queries() {
        let q = |text: String| FindQuery {
            text,
            sheet: None,
            in_formulas: false,
            match_case: false,
            whole_cell: false,
            mode: FindMode::Regex,
            from: None,
        };
        assert!(find_query(&q("a+".into())).is_ok());
        let e = find_query(&q("a".repeat(MAX_TEXT + 1))).unwrap_err();
        assert!(e.message.contains("regular expression"), "{}", e.message);
        let replace = Op::ReplaceAll {
            sheet: None,
            query: q("a".into()),
            replace: "b".repeat(MAX_TEXT + 1),
        };
        assert!(transaction(&tx(vec![replace])).is_err());
    }
}
