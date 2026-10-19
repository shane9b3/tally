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
use crate::storage::{AutoFilter, ColumnCriteria, Spans};
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

/// Model → engine AutoFilter.
pub fn from_model(d: &AutoFilterData) -> AutoFilter {
    let mut columns = BTreeMap::new();
    for c in &d.columns {
        if c.col < d.range.c0 || c.col > d.range.c1 {
            continue;
        }
        columns.insert(
            c.col,
            ColumnCriteria {
                values: c.values.clone(),
                condition: c.condition.as_ref().map(|x| (x.op, x.value.clone())),
            },
        );
    }
    AutoFilter {
        range: d.range,
        columns,
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
}

impl<'a> Matcher<'a> {
    /// Prepares `c`.
    pub fn new(c: &'a ColumnCriteria) -> Self {
        Self {
            values: c
                .values
                .as_ref()
                .map(|v| v.iter().map(|x| x.to_lowercase()).collect()),
            condition: c.condition.as_ref().map(|(op, o)| (*op, o.as_str())),
        }
    }

    /// `true` if a cell (its value and display text) passes.
    pub fn passes(&self, v: &Value, text: &str) -> bool {
        if let Some(values) = &self.values {
            if !values.contains(&text.to_lowercase()) {
                return false;
            }
        }
        if let Some((op, operand)) = self.condition {
            if !condition(op, operand, v, text) {
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
                let matchers: Vec<(u32, Matcher<'_>)> = af
                    .columns
                    .iter()
                    .map(|(col, c)| (*col, Matcher::new(c)))
                    .collect();
                let mut hidden = Vec::new();
                for r in af.range.r0.saturating_add(1)..=af.range.r1 {
                    let keep = matchers.iter().all(|(col, m)| {
                        let addr = CellAddr::new(sheet, r, *col);
                        let v = self.value(addr);
                        let text = display_text(self, addr);
                        m.passes(&v, &text)
                    });
                    if !keep {
                        hidden.push(r);
                    }
                }
                Spans::from_sorted_rows(hidden)
            }
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
        for row in r.r0.saturating_add(1)..=r.r1 {
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
