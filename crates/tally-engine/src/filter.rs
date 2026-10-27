//! AutoFilter (inventory §11): one filtered area per sheet; per-column
//! criteria (a list of formatted values to keep and/or one condition);
//! non-matching data rows are hidden as *filtered* rows, separate from
//! rows the user hid.
//!
//! The filtered rows are a function of the criteria and the cell values.
//! As in AOO, they are recomputed ([`Workbook::reapply_filter`]) only when
//! the criteria change (and when that change is undone or redone), on a
//! row/column change of the sheet, on a hard recalc and on an explicit
//! `Op::RefreshFilter`, not after every edit: editing a filtered column
//! leaves the visible rows alone until the filter is refreshed.

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashSet};

use tally_api::{
    AutoFilterInfo, ColumnFilter, FilterColumnInfo, FilterCondition, FilterOp, RangeDto,
};
use tally_model::{
    AutoFilterData, CellRangeLocal, FilterColumnData, FilterConditionData, FilterOpData,
};
use tally_types::{CellAddr, CellRange, SheetId, Value};

use crate::service::view::display_text;
use crate::storage::{AutoFilter, ColumnCriteria, ColumnStore, Spans};
use crate::tools::{passes_threshold, top_threshold, TopKind};
use crate::workbook::Workbook;

/// Most distinct values listed per column by [`filter_info`].
pub const MAX_UNIQUE: usize = 10_000;

/// Most values one column's value list may hold (`Op::SetColumnFilter`).
pub const MAX_FILTER_VALUES: usize = 100_000;

/// Most bytes of value text in one column's value list.
pub const MAX_FILTER_VALUE_BYTES: usize = 16 * 1024 * 1024;

/// API → model operator.
pub fn op_to_model(op: FilterOp) -> FilterOpData {
    match op {
        FilterOp::Eq => FilterOpData::Eq,
        FilterOp::Ne => FilterOpData::Ne,
        FilterOp::Gt => FilterOpData::Gt,
        FilterOp::Ge => FilterOpData::Ge,
        FilterOp::Lt => FilterOpData::Lt,
        FilterOp::Le => FilterOpData::Le,
        FilterOp::Contains => FilterOpData::Contains,
        FilterOp::BeginsWith => FilterOpData::BeginsWith,
        FilterOp::EndsWith => FilterOpData::EndsWith,
        FilterOp::Empty => FilterOpData::Empty,
        FilterOp::NotEmpty => FilterOpData::NotEmpty,
        FilterOp::TopValues => FilterOpData::TopValues,
        FilterOp::BottomValues => FilterOpData::BottomValues,
        FilterOp::TopPercent => FilterOpData::TopPercent,
        FilterOp::BottomPercent => FilterOpData::BottomPercent,
    }
}

/// Model → API operator.
pub fn op_to_api(op: FilterOpData) -> FilterOp {
    match op {
        FilterOpData::Eq => FilterOp::Eq,
        FilterOpData::Ne => FilterOp::Ne,
        FilterOpData::Gt => FilterOp::Gt,
        FilterOpData::Ge => FilterOp::Ge,
        FilterOpData::Lt => FilterOp::Lt,
        FilterOpData::Le => FilterOp::Le,
        FilterOpData::Contains => FilterOp::Contains,
        FilterOpData::BeginsWith => FilterOp::BeginsWith,
        FilterOpData::EndsWith => FilterOp::EndsWith,
        FilterOpData::Empty => FilterOp::Empty,
        FilterOpData::NotEmpty => FilterOp::NotEmpty,
        FilterOpData::TopValues => FilterOp::TopValues,
        FilterOpData::BottomValues => FilterOp::BottomValues,
        FilterOpData::TopPercent => FilterOp::TopPercent,
        FilterOpData::BottomPercent => FilterOp::BottomPercent,
    }
}

/// API filter → engine criteria.
pub fn criteria_from_api(f: &ColumnFilter) -> ColumnCriteria {
    ColumnCriteria {
        values: f.values.clone(),
        condition: f
            .condition
            .as_ref()
            .map(|c| (op_to_model(c.op), c.value.clone())),
    }
}

/// Engine criteria → API filter.
pub fn criteria_to_api(c: &ColumnCriteria) -> ColumnFilter {
    ColumnFilter {
        values: c.values.clone(),
        condition: c.condition.as_ref().map(|(op, v)| FilterCondition {
            op: op_to_api(*op),
            value: v.clone(),
        }),
    }
}

/// Model → engine AutoFilter (takes the model's value lists without
/// copying them).
pub fn from_model(d: AutoFilterData) -> AutoFilter {
    let range = d.range;
    let mut columns = BTreeMap::new();
    for c in d.columns {
        if c.col < range.c0 || c.col > range.c1 {
            continue;
        }
        columns.insert(
            c.col,
            ColumnCriteria {
                values: c.values,
                condition: c.condition.map(|x| (x.op, x.value)),
            },
        );
    }
    AutoFilter { range, columns }
}

/// An area that reaches the sheet's last row (or column), a whole-column,
/// whole-row or whole-sheet ref, cut down in that direction to the sheet's
/// cells (keeping its header row and first column), as AOO's
/// `GetPrintArea`-style trimming. Any other area is kept exactly as
/// written, so `A1:D1000` with data to row 500 stays `A1:D1000` and rows
/// typed later inside it are filtered.
pub fn clamp_to_cells(r: CellRangeLocal, cells: &[tally_model::CellData]) -> CellRangeLocal {
    let whole_rows = r.r1 >= tally_types::MAX_ROWS - 1;
    let whole_cols = r.c1 >= tally_types::MAX_COLS - 1;
    if !whole_rows && !whole_cols {
        return r;
    }
    let (mut r1, mut c1) = (r.r0, r.c0);
    for c in cells {
        r1 = r1.max(c.row);
        c1 = c1.max(c.col);
    }
    CellRangeLocal {
        r1: if whole_rows { r.r1.min(r1) } else { r.r1 },
        c1: if whole_cols { r.c1.min(c1) } else { r.c1 },
        ..r
    }
}

/// Engine → model AutoFilter.
pub fn to_model(f: &AutoFilter, filtered: &Spans) -> AutoFilterData {
    AutoFilterData {
        range: f.range,
        columns: f
            .columns
            .iter()
            .map(|(col, c)| FilterColumnData {
                col: *col,
                values: c.values.clone(),
                condition: c.condition.as_ref().map(|(op, v)| FilterConditionData {
                    op: *op,
                    value: v.clone(),
                }),
            })
            .collect(),
        filtered_rows: filtered.spans().to_vec(),
    }
}

/// `true` if a cell (its value and display text) passes `c`.
pub fn passes(c: &ColumnCriteria, v: &Value, text: &str) -> bool {
    Matcher::new(c).passes(v, text)
}

/// Column criteria prepared once per filter application: the value list as
/// a set of lowercased texts, so a row is one hash lookup instead of a
/// lowercase-and-compare per listed value (security review 2, HIGH-5:
/// 10,000 values × 100k rows took 12 s).
pub struct Matcher<'a> {
    values: Option<HashSet<String>>,
    condition: Option<(FilterOpData, &'a str)>,
    /// `(ge, threshold)` of a top/bottom-N condition ([`Matcher::for_column`]).
    threshold: Option<(bool, f64)>,
}

/// The top/bottom-N kind of an operator (`None` for the others).
fn top_kind(op: FilterOpData) -> Option<TopKind> {
    Some(match op {
        FilterOpData::TopValues => TopKind::Largest,
        FilterOpData::BottomValues => TopKind::Smallest,
        FilterOpData::TopPercent => TopKind::LargestPct,
        FilterOpData::BottomPercent => TopKind::SmallestPct,
        _ => return None,
    })
}

fn number_of(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => Some(*n),
        Value::Bool(b) => Some(f64::from(u8::from(*b))),
        _ => None,
    }
}

impl<'a> Matcher<'a> {
    /// Prepares `c` (a top/bottom-N condition passes every cell; use
    /// [`Matcher::for_column`]).
    pub fn new(c: &'a ColumnCriteria) -> Self {
        Self {
            values: c
                .values
                .as_ref()
                .map(|v| v.iter().map(|x| x.to_lowercase()).collect()),
            condition: c.condition.as_ref().map(|(op, o)| (*op, o.as_str())),
            threshold: None,
        }
    }

    /// Prepares `c` for column `col` of the data rows `r0..=r1` of `sheet`:
    /// a top/bottom-N condition gets its threshold, once, from the numbers
    /// among the column's cells in those rows (AOO `ScTable::TopTenQuery`:
    /// the whole query area, not just the rows other columns leave
    /// visible; ties pass).
    pub fn for_column(
        wb: &Workbook,
        sheet: SheetId,
        r0: u32,
        r1: u32,
        col: u32,
        c: &'a ColumnCriteria,
    ) -> Self {
        let mut m = Self::new(c);
        if let Some((op, operand)) = m.condition {
            if let Some(kind) = top_kind(op) {
                let vals: Vec<f64> = match wb.sheet(sheet).and_then(|s| s.columns.get(col as usize))
                {
                    Some(column) if r0 <= r1 => column
                        .cells
                        .iter_range(r0, r1)
                        .filter_map(|(r, _)| number_of(&wb.value(CellAddr::new(sheet, r, col))))
                        .collect(),
                    _ => Vec::new(),
                };
                m.threshold = Some(top_threshold(vals, kind, operand));
            }
        }
        m
    }

    /// `true` if a cell (its value and display text) passes.
    pub fn passes(&self, v: &Value, text: &str) -> bool {
        if let Some(values) = &self.values {
            if !values.contains(&text.to_lowercase()) {
                return false;
            }
        }
        if let Some((op, operand)) = self.condition {
            if top_kind(op).is_some() {
                if let Some((ge, t)) = self.threshold {
                    if !passes_threshold(number_of(v), ge, t) {
                        return false;
                    }
                }
            } else if !condition(op, operand, v, text) {
                return false;
            }
        }
        true
    }
}

/// Checks the size of a column filter from the API.
pub fn check_filter_size(f: &ColumnFilter) -> Result<(), tally_api::ApiError> {
    if let Some(v) = &f.values {
        let bytes: usize = v.iter().map(String::len).sum();
        if v.len() > MAX_FILTER_VALUES || bytes > MAX_FILTER_VALUE_BYTES {
            return Err(tally_api::ApiError::invalid_argument(format!(
                "a column filter may list at most {MAX_FILTER_VALUES} values \
                 ({} MiB of text)",
                MAX_FILTER_VALUE_BYTES >> 20
            )));
        }
    }
    Ok(())
}

fn condition(op: FilterOpData, operand: &str, v: &Value, text: &str) -> bool {
    let empty = v.is_empty() || matches!(v, Value::Text(t) if t.is_empty());
    match op {
        FilterOpData::Empty => return empty,
        FilterOpData::NotEmpty => return !empty,
        _ => {}
    }
    let t = text.to_lowercase();
    let o = operand.to_lowercase();
    match op {
        FilterOpData::Contains => return t.contains(&o),
        FilterOpData::BeginsWith => return t.starts_with(&o),
        FilterOpData::EndsWith => return t.ends_with(&o),
        _ => {}
    }
    let num = match v {
        Value::Number(n) => Some(*n),
        Value::Bool(b) => Some(f64::from(u8::from(*b))),
        _ => None,
    };
    let ord = match (num, operand.trim().parse::<f64>().ok()) {
        (Some(a), Some(b)) => {
            if tally_types::approx_equal(a, b) {
                Ordering::Equal
            } else {
                a.partial_cmp(&b).unwrap_or(Ordering::Equal)
            }
        }
        // A number against text, or text: compare the displayed text, but
        // numbers never equal text (and sort before it).
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) if !empty => Ordering::Greater,
        _ => t.cmp(&o),
    };
    match op {
        FilterOpData::Eq => ord == Ordering::Equal,
        FilterOpData::Ne => ord != Ordering::Equal,
        FilterOpData::Gt => ord == Ordering::Greater,
        FilterOpData::Ge => ord != Ordering::Less,
        FilterOpData::Lt => ord == Ordering::Less,
        FilterOpData::Le => ord != Ordering::Greater,
        _ => true,
    }
}

/// The local range of an AutoFilter as a sheet range.
pub fn area(sheet: SheetId, r: &CellRangeLocal) -> CellRange {
    CellRange::new(sheet, r.r0, r.c0, r.r1, r.c1)
}

impl Workbook {
    /// Recomputes the rows hidden by `sheet`'s AutoFilter. Returns `true`
    /// if they changed.
    pub fn reapply_filter(&mut self, sheet: SheetId) -> bool {
        let Some(s) = self.sheet(sheet) else {
            return false;
        };
        let new = match &s.meta.auto_filter {
            None => Spans::default(),
            Some(af) if af.columns.is_empty() => Spans::default(),
            Some(af) => {
                // Only the filtered columns' own cells are read; an empty cell
                // is decided once per column (review: a file's 16,384
                // filtered columns over a million rows took minutes per
                // refresh). A row is hidden when a cell fails its column, or
                // when it is empty in a column whose criteria refuse empty.
                let (body0, body1) = (af.range.r0.saturating_add(1), af.range.r1);
                let mut fails_empty = 0u32;
                // Row → (filled cells in empty-refusing columns, a cell failed).
                let mut rows: std::collections::HashMap<u32, (u32, bool)> =
                    std::collections::HashMap::new();
                for (col, c) in &af.columns {
                    let m = Matcher::for_column(self, sheet, body0, body1, *col, c);
                    let refuses_empty = !m.passes(&Value::Empty, "");
                    fails_empty += u32::from(refuses_empty);
                    let Some(column) = s.columns.get(*col as usize) else {
                        continue;
                    };
                    if body0 > body1 {
                        continue;
                    }
                    for (r, _) in column.cells.iter_range(body0, body1) {
                        let addr = CellAddr::new(sheet, r, *col);
                        let v = self.value(addr);
                        let text = display_text(self, addr);
                        let e = rows.entry(r).or_insert((0, false));
                        if refuses_empty {
                            e.0 += 1;
                        }
                        if !m.passes(&v, &text) {
                            e.1 = true;
                        }
                    }
                }
                if fails_empty == 0 {
                    let mut hidden: Vec<u32> =
                        rows.iter().filter(|(_, x)| x.1).map(|(r, _)| *r).collect();
                    hidden.sort_unstable();
                    Spans::from_sorted_rows(hidden)
                } else {
                    // Every row is hidden except those filled in all the
                    // empty-refusing columns and passing everywhere.
                    let mut kept: Vec<u32> = rows
                        .iter()
                        .filter(|(_, x)| !x.1 && x.0 == fails_empty)
                        .map(|(r, _)| *r)
                        .collect();
                    kept.sort_unstable();
                    let mut spans = Vec::new();
                    let mut next = body0;
                    for r in kept {
                        if r > next {
                            spans.push((next, r - 1));
                        }
                        next = r + 1;
                    }
                    if next <= body1 {
                        spans.push((next, body1));
                    }
                    Spans::from_spans(spans)
                }
            }
        };
        // The standard/advanced filter (wave 5) hides rows the same way.
        let new = match s.meta.standard_filter.as_deref() {
            Some(sf) => {
                let extra = crate::tools::std_filter_hidden(self, sheet, sf);
                if extra.is_empty() {
                    new
                } else {
                    let mut v = new.spans().to_vec();
                    v.extend_from_slice(extra.spans());
                    Spans::from_spans(v)
                }
            }
            None => new,
        };
        match self.sheet_mut(sheet) {
            Some(s) if s.filtered_rows != new => {
                s.filtered_rows = new;
                true
            }
            _ => false,
        }
    }

    /// Re-applies the AutoFilters whose area intersects `changed`; returns
    /// the sheets whose filtered rows changed.
    pub fn reapply_filters_touching(&mut self, changed: &[CellRange]) -> Vec<SheetId> {
        let mut sheets: Vec<SheetId> = Vec::new();
        for r in changed {
            let Some(af) = self
                .sheet(r.sheet)
                .and_then(|s| s.meta.auto_filter.as_ref())
            else {
                continue;
            };
            if area(r.sheet, &af.range).intersects(r) && !sheets.contains(&r.sheet) {
                sheets.push(r.sheet);
            }
        }
        sheets.retain(|s| self.reapply_filter(*s));
        sheets
    }
}

/// Sort key for the distinct-value list: numbers (ascending), then text
/// (case-insensitive), then empty.
fn unique_order(a: &(Option<f64>, String), b: &(Option<f64>, String)) -> Ordering {
    match (a.1.is_empty(), b.1.is_empty()) {
        (true, false) => return Ordering::Greater,
        (false, true) => return Ordering::Less,
        _ => {}
    }
    match (a.0, b.0) {
        (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(Ordering::Equal),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => a.1.to_lowercase().cmp(&b.1.to_lowercase()),
    }
}

/// The AutoFilter of `sheet` with each column's distinct values.
pub fn filter_info(wb: &Workbook, sheet: SheetId) -> Option<AutoFilterInfo> {
    let s = wb.sheet(sheet)?;
    let Some(af) = &s.meta.auto_filter else {
        return Some(AutoFilterInfo {
            range: None,
            columns: Vec::new(),
        });
    };
    let r = af.range;
    let mut columns = Vec::new();
    for col in r.c0..=r.c1 {
        let mut seen: BTreeMap<String, (Option<f64>, String, u32)> = BTreeMap::new();
        // Only the column's cells are read; the empty rows count at once
        // (a whole-sheet area must not walk a million rows per column).
        let (b0, b1) = (r.r0.saturating_add(1), r.r1);
        let filled: Vec<u32> = match s.columns.get(col as usize) {
            Some(c) if b0 <= b1 => c.cells.iter_range(b0, b1).map(|(row, _)| row).collect(),
            _ => Vec::new(),
        };
        let empties = (b1 + 1)
            .saturating_sub(b0)
            .saturating_sub(filled.len() as u32);
        if empties > 0 {
            seen.insert(String::new(), (None, String::new(), empties));
        }
        for row in filled {
            let addr = CellAddr::new(sheet, row, col);
            let v = wb.value(addr);
            let text = if v.is_empty() {
                String::new()
            } else {
                display_text(wb, addr)
            };
            let key = text.to_lowercase();
            if seen.len() >= MAX_UNIQUE && !seen.contains_key(&key) {
                continue;
            }
            let num = match v {
                Value::Number(n) => Some(n),
                _ => None,
            };
            seen.entry(key).or_insert((num, text, 0)).2 += 1;
        }
        let mut list: Vec<(Option<f64>, String, u32)> = seen.into_values().collect();
        list.sort_by(|a, b| unique_order(&(a.0, a.1.clone()), &(b.0, b.1.clone())));
        columns.push(FilterColumnInfo {
            col,
            counts: list.iter().map(|x| x.2).collect(),
            unique: list.into_iter().map(|x| x.1).collect(),
            active: af.columns.get(&col).map(criteria_to_api),
        });
    }
    Some(AutoFilterInfo {
        range: Some(RangeDto {
            r0: r.r0,
            c0: r.c0,
            r1: r.r1,
            c1: r.c1,
        }),
        columns,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crit(op: FilterOpData, v: &str) -> ColumnCriteria {
        ColumnCriteria {
            values: None,
            condition: Some((op, v.into())),
        }
    }

    #[test]
    fn conditions() {
        let n = Value::Number(5.0);
        assert!(passes(&crit(FilterOpData::Gt, "4"), &n, "5"));
        assert!(!passes(&crit(FilterOpData::Gt, "5"), &n, "5"));
        assert!(passes(&crit(FilterOpData::Ge, "5"), &n, "5"));
        assert!(passes(&crit(FilterOpData::Eq, "5.0"), &n, "5"));
        let t = Value::text("Paid");
        assert!(passes(&crit(FilterOpData::Eq, "paid"), &t, "Paid"));
        assert!(passes(&crit(FilterOpData::BeginsWith, "pa"), &t, "Paid"));
        assert!(passes(&crit(FilterOpData::EndsWith, "ID"), &t, "Paid"));
        assert!(passes(&crit(FilterOpData::Contains, "ai"), &t, "Paid"));
        assert!(
            passes(&crit(FilterOpData::Gt, "3"), &t, "Paid"),
            "text sorts after numbers"
        );
        assert!(passes(&crit(FilterOpData::Empty, ""), &Value::Empty, ""));
        assert!(passes(&crit(FilterOpData::NotEmpty, ""), &t, "Paid"));
        let vals = ColumnCriteria {
            values: Some(vec!["paid".into(), String::new()]),
            condition: None,
        };
        assert!(passes(&vals, &t, "Paid"));
        assert!(passes(&vals, &Value::Empty, ""));
        assert!(!passes(&vals, &Value::text("Due"), "Due"));
    }
}
