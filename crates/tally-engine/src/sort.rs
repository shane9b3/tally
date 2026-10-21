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
//! - v0.5 options ([`sort_range_with`]): left-to-right, copy results to,
//!   a custom order for the first key, natural order, include formats.
//!   Conditional formats and validations keep their ranges (M4: AOO moves
//!   only the cell attributes with `bIncludePattern`).

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

/// Natural text order: digit runs compare as numbers (`a2` < `a10`).
fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (a, b) = (a.to_lowercase(), b.to_lowercase());
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(ca), Some(cb)) if ca.is_ascii_digit() && cb.is_ascii_digit() => {
                let mut na = String::new();
                while let Some(c) = x.peek().copied().filter(char::is_ascii_digit) {
                    na.push(c);
                    x.next();
                }
                let mut nb = String::new();
                while let Some(c) = y.peek().copied().filter(char::is_ascii_digit) {
                    nb.push(c);
                    y.next();
                }
                let (ta, tb) = (na.trim_start_matches('0'), nb.trim_start_matches('0'));
                let o = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if o != Ordering::Equal {
                    return o;
                }
            }
            (Some(ca), Some(cb)) => {
                let o = ca.cmp(&cb);
                if o != Ordering::Equal {
                    return o;
                }
                x.next();
                y.next();
            }
        }
    }
}

/// Position of a text in a custom sort list (case-insensitive).
fn list_rank(list: &[String], v: &Value) -> Option<usize> {
    let Value::Text(t) = v else {
        return None;
    };
    list.iter().position(|e| e.eq_ignore_ascii_case(t))
}

/// Compares two cells for one key with the v0.5 options: a custom order
/// for the first key (list entries first, in list order), natural text
/// order, then [`compare_key`].
pub fn compare_key_with(
    a: &Value,
    b: &Value,
    ascending: bool,
    case_sensitive: bool,
    natural: bool,
    custom: Option<&[String]>,
) -> Ordering {
    if let Some(list) = custom {
        let (ra, rb) = (list_rank(list, a), list_rank(list, b));
        let o = match (ra, rb) {
            (Some(x), Some(y)) => x.cmp(&y),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        };
        if o != Ordering::Equal {
            return if ascending { o } else { o.reverse() };
        }
    }
    if natural {
        if let (Value::Text(x), Value::Text(y)) = (a, b) {
            let o = natural_cmp(x, y);
            if o != Ordering::Equal {
                return if ascending { o } else { o.reverse() };
            }
        }
    }
    compare_key(a, b, ascending, case_sensitive)
}

/// Sorts the rows of `range` by `keys` (the v0.2 behaviour: formats move
/// with the rows).
pub fn sort_range(
    wb: &mut Workbook,
    range: CellRange,
    keys: &[SortKey],
    has_header: bool,
    case_sensitive: bool,
) -> Result<Vec<Change>, ApiError> {
    sort_range_with(wb, range, keys, has_header, case_sensitive, None)
}

/// Checks the keys and the range; returns the data area without the
/// header line.
fn check_sort(
    wb: &Workbook,
    range: CellRange,
    keys: &[SortKey],
    has_header: bool,
    left_to_right: bool,
) -> Result<Option<CellRange>, ApiError> {
    if keys.is_empty() || keys.len() > MAX_KEYS {
        return Err(ApiError::invalid_argument(format!(
            "between 1 and {MAX_KEYS} sort keys are needed"
        )));
    }
    let (lo, hi) = if left_to_right {
        (range.r0, range.r1)
    } else {
        (range.c0, range.c1)
    };
    if keys.iter().any(|k| k.col < lo || k.col > hi) {
        return Err(ApiError::invalid_argument(
            "a sort key is outside the range",
        ));
    }
    if range.cell_count() > MAX_OP_CELLS {
        return Err(ApiError::invalid_argument("the range is too large to sort"));
    }
    let data = if left_to_right {
        let c0 = range.c0 + u32::from(has_header);
        if c0 >= range.c1 {
            return Ok(None);
        }
        CellRange::new(range.sheet, range.r0, c0, range.r1, range.c1)
    } else {
        let r0 = range.r0 + u32::from(has_header);
        if r0 >= range.r1 {
            return Ok(None);
        }
        CellRange::new(range.sheet, r0, range.c0, range.r1, range.c1)
    };
    check_movable(wb, &data)?;
    Ok(Some(data))
}

/// The permutation of lanes (rows, or columns when `left_to_right`) that
/// sorts `data`: `perm[i]` is the source lane of destination lane `i`.
fn permutation(
    wb: &Workbook,
    data: &CellRange,
    keys: &[SortKey],
    case_sensitive: bool,
    left_to_right: bool,
    natural: bool,
    custom: Option<&[String]>,
) -> Vec<u32> {
    let n = if left_to_right {
        data.cols()
    } else {
        data.rows()
    };
    let at = |lane: u32, key: u32| {
        if left_to_right {
            CellAddr::new(data.sheet, key, data.c0 + lane)
        } else {
            CellAddr::new(data.sheet, data.r0 + lane, key)
        }
    };
    let key_values: Vec<Vec<Value>> = keys
        .iter()
        .map(|k| (0..n).map(|i| wb.value(at(i, k.col))).collect())
        .collect();
    let mut perm: Vec<u32> = (0..n).collect();
    perm.sort_by(|&a, &b| {
        for (ki, (k, vals)) in keys.iter().zip(&key_values).enumerate() {
            let o = compare_key_with(
                &vals[a as usize],
                &vals[b as usize],
                k.ascending,
                case_sensitive,
                natural,
                custom.filter(|_| ki == 0),
            );
            if o != Ordering::Equal {
                return o;
            }
        }
        Ordering::Equal
    });
    perm
}

/// Sorts `range` with the v0.5 options: left-to-right, copy results to,
/// custom order, include formats (`None` = the v0.2 behaviour, formats
/// included). Conditional formats and validations keep their ranges
/// (only the cells' own formats, notes and links move), as AOO does.
pub fn sort_range_with(
    wb: &mut Workbook,
    range: CellRange,
    keys: &[SortKey],
    has_header: bool,
    case_sensitive: bool,
    options: Option<&tally_api::SortOptions>,
) -> Result<Vec<Change>, ApiError> {
    let ltr = options.is_some_and(|o| o.left_to_right);
    let include_formats = options.is_none_or(|o| o.include_formats);
    let natural = options.is_some_and(|o| o.natural);
    let custom: Option<&[String]> = options.and_then(|o| o.custom_order.as_deref());
    let Some(data) = check_sort(wb, range, keys, has_header, ltr)? else {
        return Ok(Vec::new());
    };
    let perm = permutation(wb, &data, keys, case_sensitive, ltr, natural, custom);
    if let Some(dest) = options.and_then(|o| o.copy_to) {
        return copy_sorted(
            wb,
            range,
            &data,
            &perm,
            dest,
            ltr,
            include_formats,
            has_header,
        );
    }
    if perm.iter().enumerate().all(|(i, p)| i as u32 == *p) {
        return Ok(Vec::new());
    }
    Ok(permute_data(wb, data, &perm, ltr, include_formats))
}

/// Refuses an area holding merges or array blocks (rows cannot move
/// whole through them).
pub(crate) fn check_movable(wb: &Workbook, data: &CellRange) -> Result<(), ApiError> {
    let sheet = wb
        .sheet(data.sheet)
        .ok_or_else(|| ApiError::not_found("no sheet"))?;
    if sheet
        .meta
        .merges
        .iter()
        .any(|m| CellRange::new(data.sheet, m.r0, m.c0, m.r1, m.c1).intersects(data))
    {
        return Err(ApiError::new(
            MERGED,
            "A range containing merged cells cannot be sorted.",
        ));
    }
    if !wb.array_blocks_in(data).is_empty() {
        return Err(array_block_error());
    }
    Ok(())
}

/// Moves the lanes (rows, or columns when `ltr`) of `data` so that
/// destination lane `i` holds source lane `perm[i]`: contents (formula
/// ASTs unchanged, as `ScColumn::SwapRow`), notes, links and (with
/// `include_formats`) styles. One snapshot of `data` for undo. Shared by
/// sorting and Remove Duplicates.
pub(crate) fn permute_data(
    wb: &mut Workbook,
    data: CellRange,
    perm: &[u32],
    ltr: bool,
    include_formats: bool,
) -> Vec<Change> {
    let sheet = data.sheet;
    let snap = capture_range(wb, data, true, true);
    let n = perm.len() as u32;
    // Notes and hyperlinks travel with their lanes.
    let meta = crate::undo::meta_parts(
        wb,
        sheet,
        &[
            crate::storage::MetaKind::Notes,
            crate::storage::MetaKind::Links,
        ],
    );
    if let Some(s) = wb.sheet_mut(sheet) {
        for which in 0..2 {
            let has = if which == 0 {
                crate::annotations::entries_in(&s.meta.notes, &data)
                    .next()
                    .is_some()
            } else {
                crate::annotations::entries_in(&s.meta.links, &data)
                    .next()
                    .is_some()
            };
            if !has {
                continue;
            }
            if which == 0 {
                let map = std::sync::Arc::make_mut(&mut s.meta.notes);
                permute_lanes(map, &data, perm, ltr);
            } else {
                let map = std::sync::Arc::make_mut(&mut s.meta.links);
                permute_lanes(map, &data, perm, ltr);
            }
        }
    }
    // Styles per cell, before clearing.
    let mut styles: Vec<Vec<StyleId>> = Vec::new();
    if let Some(s) = wb.sheet(sheet) {
        if ltr {
            for r in data.r0..=data.r1 {
                styles.push((0..n).map(|i| s.style(r, data.c0 + i)).collect());
            }
        } else {
            for c in data.c0..=data.c1 {
                styles.push((0..n).map(|i| s.style(data.r0 + i, c)).collect());
            }
        }
    }
    // Contents by source lane.
    let mut by_lane: Vec<Vec<(u32, SnapContent)>> = vec![Vec::new(); n as usize];
    for (r, c, content) in snap.contents.iter().flatten() {
        if ltr {
            by_lane[(c - data.c0) as usize].push((*r, content.clone()));
        } else {
            by_lane[(r - data.r0) as usize].push((*c, content.clone()));
        }
    }
    clear_range_content(wb, data);
    if include_formats {
        clear_range_styles(wb, data);
    }
    for (dst_i, &src_i) in perm.iter().enumerate() {
        let dst = dst_i as u32;
        for (other, content) in &by_lane[src_i as usize] {
            // AOO (`ScColumn::SwapRow`) keeps every relative reference
            // relative: the AST (relative offsets) moves unchanged.
            let new = if ltr {
                CellAddr::new(sheet, *other, data.c0 + dst)
            } else {
                CellAddr::new(sheet, data.r0 + dst, *other)
            };
            wb.restore_content(new, Some(content.clone()));
        }
        if include_formats {
            if let Some(s) = wb.sheet_mut(sheet) {
                for (li, lane_styles) in styles.iter().enumerate() {
                    let st = lane_styles[src_i as usize];
                    if st == StyleId::DEFAULT {
                        continue;
                    }
                    let (row, col) = if ltr {
                        (data.r0 + li as u32, data.c0 + dst)
                    } else {
                        (data.r0 + dst, data.c0 + li as u32)
                    };
                    if let Some(c) = s.column_mut(col) {
                        c.styles.set(row, row, st);
                    }
                }
            }
        }
    }
    wb.pending.ranges.push(data);
    let mut out: Vec<Change> = meta.into_iter().collect();
    out.push(Change::Range(snap));
    out
}

/// Permutes the entries of a `(row, col)` map inside `data` by lanes.
fn permute_lanes<T>(
    map: &mut std::collections::BTreeMap<(u32, u32), T>,
    data: &CellRange,
    perm: &[u32],
    ltr: bool,
) {
    if !ltr {
        crate::annotations::permute_rows(map, data.r0, data.c0, data.c1, perm);
        return;
    }
    let keys: Vec<(u32, u32)> = map
        .range((data.r0, 0)..=(data.r1, u32::MAX))
        .map(|(k, _)| *k)
        .filter(|(_, c)| *c >= data.c0 && *c <= data.c1)
        .collect();
    let mut taken: Vec<((u32, u32), T)> = Vec::with_capacity(keys.len());
    for k in keys {
        if let Some(v) = map.remove(&k) {
            taken.push((k, v));
        }
    }
    let mut inv = vec![0u32; perm.len()];
    for (dst, src) in perm.iter().enumerate() {
        inv[*src as usize] = dst as u32;
    }
    for ((r, c), v) in taken {
        let lane = (c - data.c0) as usize;
        map.insert((r, data.c0 + inv[lane]), v);
    }
}

/// Writes the sorted copy of `range` at `dest`; the range is unchanged.
#[allow(clippy::too_many_arguments)]
fn copy_sorted(
    wb: &mut Workbook,
    range: CellRange,
    data: &CellRange,
    perm: &[u32],
    dest: tally_api::SheetCell,
    ltr: bool,
    include_formats: bool,
    has_header: bool,
) -> Result<Vec<Change>, ApiError> {
    crate::ops::check_sheet(wb, dest.sheet)?;
    let h = range.rows() as u64;
    let w = range.cols() as u64;
    if u64::from(dest.r) + h > u64::from(tally_types::MAX_ROWS)
        || u64::from(dest.c) + w > u64::from(tally_types::MAX_COLS)
    {
        return Err(ApiError::invalid_argument(
            "the copy does not fit on the sheet",
        ));
    }
    let target = CellRange::new(
        dest.sheet,
        dest.r,
        dest.c,
        dest.r + range.rows() - 1,
        dest.c + range.cols() - 1,
    );
    crate::protection::check_edit(wb, &[target])?;
    crate::ops::check_array_blocks(wb, &[target])?;
    let src = capture_range(wb, range, true, true);
    let inv = capture_range(wb, target, true, true);
    clear_range_content(wb, target);
    if include_formats {
        clear_range_styles(wb, target);
    }
    // Source lane → destination lane (header lane stays in place).
    let lane_of = |i: u32| -> u32 {
        let base = if ltr { data.c0 } else { data.r0 };
        if i < base {
            return i - if ltr { range.c0 } else { range.r0 };
        }
        let src_lane = i - base;
        let dst_lane = perm
            .iter()
            .position(|p| *p == src_lane)
            .map_or(src_lane, |x| x as u32);
        dst_lane + u32::from(has_header)
    };
    let mut styles: Vec<(u32, u32, StyleId)> = Vec::new();
    if let Some(s) = wb.sheet(range.sheet) {
        for c in range.c0..=range.c1 {
            for r in range.r0..=range.r1 {
                let st = s.style(r, c);
                if st != StyleId::DEFAULT {
                    styles.push((r, c, st));
                }
            }
        }
    }
    let place = |r: u32, c: u32| -> CellAddr {
        let (nr, nc) = if ltr {
            (r - range.r0, lane_of(c))
        } else {
            (lane_of(r), c - range.c0)
        };
        CellAddr::new(dest.sheet, dest.r + nr, dest.c + nc)
    };
    for (r, c, content) in src.contents.iter().flatten() {
        wb.restore_content(place(*r, *c), Some(content.clone()));
    }
    if include_formats {
        for (r, c, st) in styles {
            let a = place(r, c);
            if let Some(col) = wb.sheet_mut(dest.sheet).and_then(|s| s.column_mut(a.col)) {
                col.styles.set(a.row, a.row, st);
            }
        }
    }
    wb.pending.ranges.push(target);
    Ok(vec![Change::Range(inv)])
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
