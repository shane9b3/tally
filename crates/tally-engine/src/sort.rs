//! Sorting rows (inventory §10).
//!
//! - Stable, multi-key, rows top to bottom; with `has_header` the first row
//!   stays in place.
//! - Order within a key: numbers (booleans as 1/0) < text < errors, empty
//!   cells always last whatever the direction (AOO). Text compares
//!   case-insensitively by default; case-sensitive puts lowercase first.
//! - Rows move whole: contents, formulas and styles.
//! - Formula references keep their relative offsets, whatever they point
//!   at (AOO `ScColumn::SwapRow`): `=A2*B2` in row 2 becomes `=A5*B5`
//!   when its row moves to 5, and a relative reference outside the range
//!   (`=C1` from row 2 → `=C4` from row 5) moves too; absolute references
//!   (`$E$1`) are unaffected.
//! - Ranges containing merges or (part of) array formulas are refused.
//! - Undo restores a snapshot of the sorted rows.

use std::cmp::Ordering;

use tally_api::{ApiError, SortKey};
use tally_types::{CellAddr, CellRange, Value};

use crate::ops::{array_block_error, MAX_OP_CELLS, MERGED};
use crate::storage::StyleId;
use crate::undo::{capture_range, clear_range_content, clear_range_styles, Change};
use crate::workbook::{SnapContent, Workbook};

/// Most sort keys.
pub const MAX_KEYS: usize = 64;

fn class(v: &Value) -> u8 {
    match v {
        Value::Number(_) | Value::Bool(_) => 0,
        Value::Text(_) => 1,
        Value::Error(_) | Value::Array(_) => 2,
        Value::Empty => 3,
    }
}

fn swap_case(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_lowercase() {
                c.to_uppercase().next().unwrap_or(c)
            } else if c.is_uppercase() {
                c.to_lowercase().next().unwrap_or(c)
            } else {
                c
            }
        })
        .collect()
}

/// Compares two cells for sorting in ascending order (empties excluded).
fn compare_values(a: &Value, b: &Value, case_sensitive: bool) -> Ordering {
    let (ca, cb) = (class(a), class(b));
    if ca != cb {
        return ca.cmp(&cb);
    }
    match (a, b) {
        (Value::Text(x), Value::Text(y)) => {
            let o = x.to_lowercase().cmp(&y.to_lowercase());
            if case_sensitive && o == Ordering::Equal {
                swap_case(x).cmp(&swap_case(y))
            } else {
                o
            }
        }
        (Value::Error(x), Value::Error(y)) => x.code().cmp(&y.code()),
        _ => {
            let n = |v: &Value| match v {
                Value::Number(n) => *n,
                Value::Bool(b) => f64::from(u8::from(*b)),
                _ => 0.0,
            };
            let (x, y) = (n(a), n(b));
            if tally_types::approx_equal(x, y) {
                Ordering::Equal
            } else {
                x.partial_cmp(&y).unwrap_or(Ordering::Equal)
            }
        }
    }
}

/// Compares for one key: empties last regardless of direction.
pub fn compare_key(a: &Value, b: &Value, ascending: bool, case_sensitive: bool) -> Ordering {
    match (a.is_empty(), b.is_empty()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        _ => {
            let o = compare_values(a, b, case_sensitive);
            if ascending {
                o
            } else {
                o.reverse()
            }
        }
    }
}

/// `true` if the first row of `range` looks like labels: some column has
/// text in the first row over a non-empty, non-text cell in the second.
pub fn detect_header(wb: &Workbook, range: &CellRange) -> bool {
    if range.r0 >= range.r1 {
        return false;
    }
    (range.c0..=range.c1).any(|c| {
        let top = wb.value(CellAddr::new(range.sheet, range.r0, c));
        let next = wb.value(CellAddr::new(range.sheet, range.r0 + 1, c));
        matches!(top, Value::Text(_)) && !next.is_empty() && !matches!(next, Value::Text(_))
    })
}

/// Sorts the rows of `range` by `keys`.
pub fn sort_range(
    wb: &mut Workbook,
    range: CellRange,
    keys: &[SortKey],
    has_header: bool,
    case_sensitive: bool,
) -> Result<Vec<Change>, ApiError> {
    if keys.is_empty() || keys.len() > MAX_KEYS {
        return Err(ApiError::invalid_argument(format!(
            "between 1 and {MAX_KEYS} sort keys are needed"
        )));
    }
    if keys.iter().any(|k| k.col < range.c0 || k.col > range.c1) {
        return Err(ApiError::invalid_argument(
            "a sort key is outside the range",
        ));
    }
    if range.cell_count() > MAX_OP_CELLS {
        return Err(ApiError::invalid_argument("the range is too large to sort"));
    }
    let r0 = range.r0 + u32::from(has_header);
    if r0 >= range.r1 {
        return Ok(Vec::new());
    }
    let data = CellRange::new(range.sheet, r0, range.c0, range.r1, range.c1);
    let sheet = wb
        .sheet(range.sheet)
        .ok_or_else(|| ApiError::not_found("no sheet"))?;
    if sheet
        .meta
        .merges
        .iter()
        .any(|m| CellRange::new(range.sheet, m.r0, m.c0, m.r1, m.c1).intersects(&data))
    {
        return Err(ApiError::new(
            MERGED,
            "A range containing merged cells cannot be sorted.",
        ));
    }
    if !wb.array_blocks_in(&data).is_empty() {
        return Err(array_block_error());
    }
    let n = data.rows();
    let key_values: Vec<Vec<Value>> = keys
        .iter()
        .map(|k| {
            (0..n)
                .map(|i| wb.value(CellAddr::new(range.sheet, r0 + i, k.col)))
                .collect()
        })
        .collect();
    let mut perm: Vec<u32> = (0..n).collect();
    perm.sort_by(|&a, &b| {
        for (k, vals) in keys.iter().zip(&key_values) {
            let o = compare_key(
                &vals[a as usize],
                &vals[b as usize],
                k.ascending,
                case_sensitive,
            );
            if o != Ordering::Equal {
                return o;
            }
        }
        Ordering::Equal
    });
    if perm.iter().enumerate().all(|(i, p)| i as u32 == *p) {
        return Ok(Vec::new());
    }
    let snap = capture_range(wb, data, true, true);
    // Notes and hyperlinks travel with their rows.
    let meta = crate::undo::meta_parts(
        wb,
        range.sheet,
        &[
            crate::storage::MetaKind::Notes,
            crate::storage::MetaKind::Links,
        ],
    );
    if let Some(s) = wb.sheet_mut(range.sheet) {
        if crate::annotations::entries_in(&s.meta.notes, &data)
            .next()
            .is_some()
        {
            let notes = std::sync::Arc::make_mut(&mut s.meta.notes);
            crate::annotations::permute_rows(notes, r0, data.c0, data.c1, &perm);
        }
        if crate::annotations::entries_in(&s.meta.links, &data)
            .next()
            .is_some()
        {
            let links = std::sync::Arc::make_mut(&mut s.meta.links);
            crate::annotations::permute_rows(links, r0, data.c0, data.c1, &perm);
        }
    }
    // Styles per cell, before clearing.
    let mut styles: Vec<Vec<StyleId>> = Vec::with_capacity(data.cols() as usize);
    if let Some(sheet) = wb.sheet(range.sheet) {
        for c in data.c0..=data.c1 {
            styles.push((0..n).map(|i| sheet.style(r0 + i, c)).collect());
        }
    }
    // Contents by source row.
    let mut by_row: Vec<Vec<(u32, SnapContent)>> = vec![Vec::new(); n as usize];
    for (r, c, content) in snap.contents.iter().flatten() {
        by_row[(r - r0) as usize].push((*c, content.clone()));
    }
    clear_range_content(wb, data);
    clear_range_styles(wb, data);
    for (dst_i, &src_i) in perm.iter().enumerate() {
        let dst_row = r0 + dst_i as u32;
        for (c, content) in &by_row[src_i as usize] {
            // AOO (`ScColumn::SwapRow`) keeps every relative reference
            // relative: the AST (relative offsets) moves unchanged.
            let new = CellAddr::new(range.sheet, dst_row, *c);
            let content = content.clone();
            wb.restore_content(new, Some(content));
        }
        if let Some(sheet) = wb.sheet_mut(range.sheet) {
            for (ci, col_styles) in styles.iter().enumerate() {
                let st = col_styles[src_i as usize];
                if st != StyleId::DEFAULT {
                    if let Some(col) = sheet.column_mut(data.c0 + ci as u32) {
                        col.styles.set(dst_row, dst_row, st);
                    }
                }
            }
        }
    }
    wb.pending.ranges.push(data);
    let mut out: Vec<Change> = meta.into_iter().collect();
    out.push(Change::Range(snap));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_order() {
        let n = |x: f64| Value::Number(x);
        let t = Value::text;
        let mut v = vec![
            t("b"),
            Value::Empty,
            n(2.0),
            t("A"),
            n(1.0),
            Value::Bool(true),
        ];
        v.sort_by(|a, b| compare_key(a, b, true, false));
        assert_eq!(
            v,
            vec![
                n(1.0),
                Value::Bool(true),
                n(2.0),
                t("A"),
                t("b"),
                Value::Empty
            ]
        );
        v.sort_by(|a, b| compare_key(a, b, false, false));
        assert_eq!(v.last(), Some(&Value::Empty), "empty last when descending");
        assert_eq!(v[0], t("b"));
        let mut c = vec![t("B"), t("b"), t("a"), t("A")];
        c.sort_by(|a, b| compare_key(a, b, true, true));
        assert_eq!(c, vec![t("a"), t("A"), t("b"), t("B")]);
    }
}
