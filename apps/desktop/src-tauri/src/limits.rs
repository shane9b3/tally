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
    ApiError, CellPos, ChartDto, ChartSvgDto, FindQuery, GoalSeekRequest, Op, PastePayload,
    PivotDto, PivotSource, PrintOptionsDto, RangeDto, SelectionDto, SheetCell, SheetRange,
    SolverRequest, Transaction,
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
/// Most series in one chart (`addChart`, `updateChart`).
pub const MAX_CHART_SERIES: usize = 255;
/// Most fields (row + column + page + data) in one pivot table.
pub const MAX_PIVOT_FIELDS: usize = 64;
/// Most constraints (and variable ranges) in one solver request.
pub const MAX_SOLVER_CONSTRAINTS: usize = 1_000;
/// Highest page number a print job may address (`page_from`, `page_to`, `page`).
pub const MAX_PRINT_PAGES: u32 = 10_000;
/// Largest SVG payload per chart in `prepare_print_job`, in bytes (4 MB).
pub const MAX_SVG_BYTES: usize = 4 * 1024 * 1024;
/// Most chart SVGs in one print job.
pub const MAX_PRINT_CHARTS: usize = 1_000;
/// Most chart SVG bytes in one print job (all charts together).
pub const MAX_SVG_TOTAL_BYTES: usize = 64 * 1024 * 1024;
/// Longest solver wall-clock limit the shell passes on (ms).
pub const MAX_SOLVER_TIMEOUT_MS: u32 = 60_000;
/// Most branch-and-bound nodes the shell passes on.
pub const MAX_SOLVER_ITERATIONS: u32 = 100_000;
/// Consolidation sources (the engine's limit).
pub const MAX_CONSOLIDATE_SOURCES: usize = 255;
/// Subtotal columns per group (the engine's limit).
pub const MAX_SUBTOTAL_COLUMNS: usize = 1_024;
/// Ops of the "expensive structural" kinds (outline groups, sheet copies, cell
/// insertion, name definitions, scenarios) in one transaction.
pub const MAX_STRUCTURAL_OPS: usize = 1_000;
/// Highest raster resolution of `print_job_page_png`.
pub const MAX_PRINT_DPI: u32 = 600;
/// Longest AutoInput prefix in characters.
pub const MAX_AUTOCOMPLETE_CHARS: usize = 1_024;

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

// ----- wave 5 (v0.4–v0.6) ---------------------------------------------------

/// A chart DTO (`addChart`, `updateChart`): at most [`MAX_CHART_SERIES`]
/// series, a valid anchor cell, finite non-negative anchor sizes and every
/// reference text at most [`MAX_TEXT`] bytes (the structural walk covers
/// the rest).
pub fn chart(what: &str, chart: &ChartDto) -> CmdResult<()> {
    if chart.series.len() > MAX_CHART_SERIES {
        return Err(bad(format!(
            "{what}: too many series ({}; the limit is {MAX_CHART_SERIES})",
            chart.series.len()
        )));
    }
    anchor(what, &chart.anchor)?;
    name(what, &chart.name)?;
    for s in &chart.series {
        text(what, &s.values)?;
    }
    Ok(())
}

/// A chart/image anchor: the cell is inside the sheet and the sizes are
/// finite and not negative.
pub fn anchor(what: &str, a: &tally_api::ChartAnchor) -> CmdResult<()> {
    pos(what, a.r, a.c)?;
    for v in [a.dx_mm, a.dy_mm, a.width_mm, a.height_mm] {
        if !v.is_finite() {
            return Err(bad(format!("{what}: the anchor is not a number")));
        }
    }
    if a.width_mm < 0.0 || a.height_mm < 0.0 {
        return Err(bad(format!("{what}: the size is negative")));
    }
    Ok(())
}

/// A pivot source: its range or database-range name.
pub fn pivot_source(what: &str, source: &PivotSource) -> CmdResult<()> {
    match source {
        PivotSource::Range { range: rg, .. } => range(what, rg),
        PivotSource::DatabaseRange { name: n } => name(what, n),
    }
}

/// A pivot DTO (`createPivot`, `updatePivot`): at most [`MAX_PIVOT_FIELDS`]
/// fields over all areas, a valid source and target.
pub fn pivot(what: &str, p: &PivotDto) -> CmdResult<()> {
    let fields =
        p.row_fields.len() + p.col_fields.len() + p.page_fields.len() + p.data_fields.len();
    if fields > MAX_PIVOT_FIELDS {
        return Err(bad(format!(
            "{what}: too many fields ({fields}; the limit is {MAX_PIVOT_FIELDS})"
        )));
    }
    pivot_source(what, &p.source)?;
    cell(what, &p.target)?;
    name(what, &p.name)
}

/// A sheet-qualified cell.
pub fn sheet_cell(what: &str, c: &SheetCell) -> CmdResult<()> {
    pos(what, c.r, c.c)
}

/// A sheet-qualified range.
pub fn sheet_range(what: &str, r: &SheetRange) -> CmdResult<()> {
    range(what, &r.range)
}

/// A solver request: at most [`MAX_SOLVER_CONSTRAINTS`] constraints and
/// variable ranges, every cell inside the sheet, finite options.
pub fn solver_request(req: &SolverRequest) -> CmdResult<()> {
    if req.constraints.len() > MAX_SOLVER_CONSTRAINTS {
        return Err(bad(format!(
            "too many constraints ({}; the limit is {MAX_SOLVER_CONSTRAINTS})",
            req.constraints.len()
        )));
    }
    if req.variables.len() > MAX_SOLVER_CONSTRAINTS {
        return Err(bad(format!(
            "too many variable ranges ({}; the limit is {MAX_SOLVER_CONSTRAINTS})",
            req.variables.len()
        )));
    }
    sheet_cell("the objective cell", &req.objective)?;
    req.variables
        .iter()
        .try_for_each(|v| sheet_range("a variable range", v))?;
    for c in &req.constraints {
        sheet_range("a constraint", &c.lhs)?;
        text("a constraint value", &c.rhs)?;
    }
    if !req.options.epsilon.is_finite() {
        return Err(bad("the solver epsilon is not a number"));
    }
    if req.options.timeout_ms > MAX_SOLVER_TIMEOUT_MS {
        return Err(bad(format!(
            "the solver time limit is too long ({} ms; the limit is {MAX_SOLVER_TIMEOUT_MS})",
            req.options.timeout_ms
        )));
    }
    if req.options.max_iterations > MAX_SOLVER_ITERATIONS {
        return Err(bad(format!(
            "too many solver iterations ({}; the limit is {MAX_SOLVER_ITERATIONS})",
            req.options.max_iterations
        )));
    }
    Ok(())
}

/// A page number (1-based or 0-based, the engine decides): at most
/// [`MAX_PRINT_PAGES`].
pub fn print_page(what: &str, page: u32) -> CmdResult<()> {
    if page > MAX_PRINT_PAGES {
        return Err(bad(format!(
            "{what}: page {page} is beyond the limit of {MAX_PRINT_PAGES}"
        )));
    }
    Ok(())
}

/// Print options: the selection range and the page window.
pub fn print_options(opts: &PrintOptionsDto) -> CmdResult<()> {
    if let Some(sel) = &opts.selection {
        range("the print selection", sel)?;
    }
    for (what, p) in [("page_from", opts.page_from), ("page_to", opts.page_to)] {
        if let Some(p) = p {
            print_page(what, p)?;
        }
    }
    if let (Some(a), Some(b)) = (opts.page_from, opts.page_to) {
        if a > b {
            return Err(bad("the page range is reversed"));
        }
    }
    Ok(())
}

/// Chart SVGs for a print job: at most [`MAX_PRINT_CHARTS`], each at most
/// [`MAX_SVG_BYTES`].
pub fn chart_svgs(charts: &[ChartSvgDto]) -> CmdResult<()> {
    if charts.len() > MAX_PRINT_CHARTS {
        return Err(bad(format!(
            "too many chart images ({}; the limit is {MAX_PRINT_CHARTS})",
            charts.len()
        )));
    }
    charts
        .iter()
        .try_for_each(|c| text_max("a chart image", &c.svg, MAX_SVG_BYTES))?;
    let total: usize = charts.iter().map(|c| c.svg.len()).sum();
    if total > MAX_SVG_TOTAL_BYTES {
        return Err(bad(format!(
            "the chart images are too large together ({total} bytes; the limit is {MAX_SVG_TOTAL_BYTES})"
        )));
    }
    Ok(())
}

/// A raster resolution: 1..=[`MAX_PRINT_DPI`].
pub fn dpi(dpi: u32) -> CmdResult<()> {
    if dpi == 0 || dpi > MAX_PRINT_DPI {
        return Err(bad(format!(
            "the resolution must be between 1 and {MAX_PRINT_DPI} dpi"
        )));
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
    let structural = tx
        .ops
        .iter()
        .filter(|o| {
            matches!(
                o,
                Op::Group { .. }
                    | Op::CopySheet { .. }
                    | Op::InsertCells { .. }
                    | Op::DefineName { .. }
                    | Op::CreateScenario { .. }
            )
        })
        .count();
    if structural > MAX_STRUCTURAL_OPS {
        return Err(bad(format!(
            "too many structural ops in one transaction ({structural}; the limit is {MAX_STRUCTURAL_OPS})"
        )));
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
        Op::AddChart { chart: ch, .. } | Op::UpdateChart { chart: ch, .. } => {
            chart("the chart", ch)
        }
        Op::MoveResizeChart { anchor: a, .. } | Op::MoveResizeImage { anchor: a, .. } => {
            anchor("the object", a)
        }
        Op::AddImage { image, .. } => {
            anchor("the picture", &image.anchor)?;
            name("the picture name", &image.name)?;
            // Base64 of at most MAX_IMAGE_BYTES (the engine's limit) plus slack.
            let max = usize::try_from(crate::images::MAX_IMAGE_BYTES.div_ceil(3) * 4 + 8)
                .unwrap_or(usize::MAX);
            text_max("the picture data", &image.data, max)
        }
        Op::Consolidate { sources, .. } if sources.len() > MAX_CONSOLIDATE_SOURCES => Err(bad(
            format!("too many consolidation sources (the limit is {MAX_CONSOLIDATE_SOURCES})"),
        )),
        Op::TextToColumns { options, .. } => {
            let widths = options.fixed_widths.as_ref().map_or(0, Vec::len);
            if widths > MAX_COLS as usize || options.column_types.len() > MAX_COLS as usize {
                return Err(bad("too many text-to-columns widths or column types"));
            }
            Ok(())
        }
        Op::RemoveDuplicates { cols, .. } if cols.len() > MAX_COLS as usize => {
            Err(bad("too many columns to compare"))
        }
        Op::Subtotals { groups, .. }
            if groups
                .iter()
                .any(|g| g.columns.len() > MAX_SUBTOTAL_COLUMNS) =>
        {
            Err(bad(format!(
                "too many subtotal columns (the limit is {MAX_SUBTOTAL_COLUMNS})"
            )))
        }
        Op::CreatePivot { pivot: p, .. } | Op::UpdatePivot { pivot: p, .. } => {
            pivot("the pivot table", p)
        }
        _ => Ok(()),
    }
}

/// Walks a serialized op: every string at most [`MAX_TEXT`] bytes (a TSV
/// payload's text and an `addImage` picture's `data` up to [`MAX_TSV`]),
/// every `{r0,c0,r1,c1}` a valid range,
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
            // Strings allowed up to MAX_TSV: a TSV payload's `text` and the
            // base64 `data` of an `addImage` op's `image`.
            let ty = map.get("type").and_then(Value::as_str);
            for (k, x) in map {
                let big = (ty == Some("tsv") && k == "text")
                    || (ty == Some("addImage") && k == "image")
                    || (tsv && k == "data");
                walk(x, big, total)?;
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

    /// Wave-5 caps: chart series, pivot fields, solver constraints, print
    /// pages, SVG payloads, and the `addImage` data allowance in the walk.
    #[test]
    fn wave5_caps() {
        use tally_api::{
            ChartAnchor, ChartAxisDto, ChartDto, ChartKind, ChartSeriesDto, ChartSvgDto,
            DataLabelsDto, ImageDto, LabelPos, LegendPos, PrintOptionsDto, PrintSheets, SheetCell,
            SolverConstraint, SolverGoal, SolverOp, SolverOptions, SolverRequest, Stacking,
        };
        let anchor = ChartAnchor {
            r: 0,
            c: 0,
            dx_mm: 0.0,
            dy_mm: 0.0,
            width_mm: 160.0,
            height_mm: 90.0,
        };
        let series = |n: usize| -> Vec<ChartSeriesDto> {
            (0..n)
                .map(|_| ChartSeriesDto {
                    name: None,
                    values: "A1:A5".into(),
                    x_values: None,
                    sizes: None,
                    kind: None,
                    secondary_axis: false,
                    color: None,
                    marker: None,
                    labels: DataLabelsDto {
                        value: false,
                        percent: false,
                        category: false,
                        series_name: false,
                        position: LabelPos::Outside,
                    },
                    trend: None,
                    error_bars: None,
                    mean_line: false,
                })
                .collect()
        };
        let axis = ChartAxisDto {
            visible: true,
            title: None,
            min: None,
            max: None,
            major: None,
            minor_count: None,
            log: false,
            reverse: false,
            major_grid: false,
            minor_grid: false,
            number_format: None,
        };
        let chart_with = |n: usize| ChartDto {
            id: 0,
            name: "Chart".into(),
            anchor,
            kind: ChartKind::Column,
            stacking: Stacking::None,
            lines: false,
            markers: false,
            smooth: false,
            hole: None,
            exploded: false,
            filled: false,
            stock: None,
            three_d: false,
            source: None,
            categories: None,
            series: series(n),
            title: None,
            subtitle: None,
            legend: LegendPos::Right,
            x_axis: axis.clone(),
            y_axis: axis.clone(),
            y2_axis: None,
            background: None,
            plot_background: None,
            include_hidden: false,
        };
        assert!(chart("c", &chart_with(MAX_CHART_SERIES)).is_ok());
        assert!(chart("c", &chart_with(MAX_CHART_SERIES + 1)).is_err());
        let add = |ch: ChartDto| Op::AddChart {
            sheet: 1,
            chart: ch,
        };
        assert!(transaction(&tx(vec![add(chart_with(MAX_CHART_SERIES + 1))])).is_err());
        let mut nan = chart_with(1);
        nan.anchor.width_mm = f32::NAN;
        assert!(chart("c", &nan).is_err());

        // A picture's base64 data may exceed MAX_TEXT (up to MAX_TSV) in the walk.
        let image = |n: usize| Op::AddImage {
            sheet: 1,
            image: ImageDto {
                id: 0,
                name: "p".into(),
                anchor,
                mime: "image/png".into(),
                data: "A".repeat(n),
            },
        };
        assert!(transaction(&tx(vec![image(MAX_TEXT * 4)])).is_ok());
        assert!(transaction(&tx(vec![image(MAX_TSV + 1)])).is_err());

        let cell = SheetCell {
            sheet: 1,
            r: 0,
            c: 0,
        };
        let constraint = |n: usize| -> Vec<SolverConstraint> {
            (0..n)
                .map(|_| SolverConstraint {
                    lhs: SheetRange {
                        sheet: 1,
                        range: RangeDto {
                            r0: 0,
                            c0: 0,
                            r1: 0,
                            c1: 0,
                        },
                    },
                    op: SolverOp::Le,
                    rhs: "10".into(),
                })
                .collect()
        };
        let req = |n: usize| SolverRequest {
            objective: cell,
            goal: SolverGoal::Maximize,
            variables: vec![],
            constraints: constraint(n),
            options: SolverOptions {
                assume_non_negative: true,
                integer: false,
                epsilon: 1e-6,
                max_iterations: 100,
                timeout_ms: 1000,
            },
        };
        assert!(solver_request(&req(MAX_SOLVER_CONSTRAINTS)).is_ok());
        assert!(solver_request(&req(MAX_SOLVER_CONSTRAINTS + 1)).is_err());
        let mut slow = req(1);
        slow.options.timeout_ms = MAX_SOLVER_TIMEOUT_MS;
        assert!(solver_request(&slow).is_ok());
        slow.options.timeout_ms = MAX_SOLVER_TIMEOUT_MS + 1;
        assert!(solver_request(&slow).is_err());
        slow.options.timeout_ms = 1000;
        slow.options.max_iterations = MAX_SOLVER_ITERATIONS + 1;
        assert!(solver_request(&slow).is_err());

        let opts = |from: Option<u32>, to: Option<u32>| PrintOptionsDto {
            sheets: PrintSheets::Active,
            selection: None,
            suppress_empty_pages: true,
            page_from: from,
            page_to: to,
        };
        assert!(print_options(&opts(Some(1), Some(MAX_PRINT_PAGES))).is_ok());
        assert!(print_options(&opts(None, Some(MAX_PRINT_PAGES + 1))).is_err());
        assert!(print_options(&opts(Some(5), Some(4))).is_err());
        assert!(print_page("p", MAX_PRINT_PAGES + 1).is_err());
        assert!(dpi(600).is_ok());
        assert!(dpi(0).is_err() && dpi(601).is_err());

        let svg = |n: usize| ChartSvgDto {
            sheet: 1,
            id: 1,
            svg: "<svg/>".repeat(n / 6 + 1),
        };
        assert!(chart_svgs(&[svg(1000)]).is_ok());
        assert!(chart_svgs(&[svg(MAX_SVG_BYTES + 6)]).is_err());
        assert!(chart_svgs(&vec![svg(1); MAX_PRINT_CHARTS + 1]).is_err());
        // 17 charts of 4 MB are each fine but exceed the 64 MiB total.
        assert!(chart_svgs(&vec![svg(MAX_SVG_BYTES - 6); 17]).is_err());
        assert!(chart_svgs(&vec![svg(MAX_SVG_BYTES - 6); 15]).is_ok());
    }

    #[test]
    fn tool_op_array_counts_are_capped() {
        use tally_api::SheetRange;
        let src = SheetRange {
            sheet: 1,
            range: RangeDto {
                r0: 0,
                c0: 0,
                r1: 0,
                c1: 0,
            },
        };
        let cons = |n: usize| Op::Consolidate {
            function: tally_api::AggFn::Sum,
            sources: vec![src; n],
            dest: SheetCell {
                sheet: 1,
                r: 0,
                c: 0,
            },
            by_row_labels: false,
            by_col_labels: false,
            link: false,
        };
        assert!(transaction(&tx(vec![cons(MAX_CONSOLIDATE_SOURCES)])).is_ok());
        assert!(transaction(&tx(vec![cons(MAX_CONSOLIDATE_SOURCES + 1)])).is_err());
        let dedupe = |n: usize| Op::RemoveDuplicates {
            sheet: 1,
            range: RangeDto {
                r0: 0,
                c0: 0,
                r1: 5,
                c1: 5,
            },
            cols: vec![0; n],
            has_header: false,
        };
        assert!(transaction(&tx(vec![dedupe(MAX_COLS as usize)])).is_ok());
        assert!(transaction(&tx(vec![dedupe(MAX_COLS as usize + 1)])).is_err());
        let names = |n: usize| {
            (0..n)
                .map(|i| Op::DefineName {
                    scope: None,
                    name: format!("N{i}"),
                    refers_to: "A1".into(),
                })
                .collect::<Vec<_>>()
        };
        assert!(transaction(&tx(names(MAX_STRUCTURAL_OPS))).is_ok());
        assert!(transaction(&tx(names(MAX_STRUCTURAL_OPS + 1))).is_err());
    }

    #[test]
    fn picture_data_is_capped_like_the_engine() {
        let max = usize::try_from(crate::images::MAX_IMAGE_BYTES.div_ceil(3) * 4 + 8).unwrap();
        let add = |n: usize| Op::AddImage {
            sheet: 1,
            image: tally_api::ImageDto {
                id: 0,
                name: "p".into(),
                mime: "image/png".into(),
                data: "A".repeat(n),
                anchor: tally_api::ChartAnchor {
                    r: 0,
                    c: 0,
                    dx_mm: 0.0,
                    dy_mm: 0.0,
                    width_mm: 10.0,
                    height_mm: 10.0,
                },
            },
        };
        assert!(transaction(&tx(vec![add(max)])).is_ok());
        assert!(transaction(&tx(vec![add(max + 1)])).is_err());
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
