//! Row and column outline groups (wave 5, W5-2): Group/Ungroup,
//! collapse/expand (hides the rows/columns), level buttons, AutoOutline
//! from the formula structure, and how groups follow row/column changes.
//!
//! The model is `SheetMeta::row_groups`/`col_groups`
//! (`tally_model::OutlineGroupData`, at most [`MAX_LEVELS`] levels). A
//! collapsed group hides its rows/columns through `Sheet::outline_rows` /
//! `outline_cols` (derived, recomputed by [`reapply`]): a third hidden
//! source next to user-hidden and filtered rows, so expanding a group
//! shows exactly what collapsing hid. Files carry the hidden flag on the
//! rows as AOO does ([`export_hidden`] / [`after_load`]).
//!
//! Nesting follows AOO's `ScOutlineArray::Insert`: a new group that
//! contains existing groups pushes them one level deeper; a group inside
//! an existing one is one level deeper than it; a group that partially
//! overlaps another is refused. ODS writes
//! `table:table-row-group`/`table:table-column-group table:display`;
//! XLSX `outlineLevel`/`collapsed`/`hidden`.

use std::sync::Arc;

use tally_api::{ApiError, OutlineAxis, OutlineDto, OutlineGroupDto, RangeDto};
use tally_model::OutlineGroupData;
use tally_types::{CellRange, SheetId, MAX_COLS, MAX_ROWS};

use crate::ops::check_sheet;
use crate::ops_v03::with_meta;
use crate::protection::check_sheet_action;
use crate::storage::{SheetMeta, Spans};
use crate::structure::{span_after, Shape};
use crate::undo::{Change, Effects};
use crate::workbook::Workbook;

/// Deepest outline level (AOO and Excel).
pub const MAX_LEVELS: u8 = 8;

/// Most groups per axis and sheet.
pub const MAX_GROUPS: usize = 100_000;

fn rows_of(axis: OutlineAxis) -> bool {
    matches!(axis, OutlineAxis::Rows)
}

fn limit_of(rows: bool) -> u32 {
    if rows {
        MAX_ROWS
    } else {
        MAX_COLS
    }
}

fn groups_mut(meta: &mut SheetMeta, rows: bool) -> &mut Vec<OutlineGroupData> {
    if rows {
        Arc::make_mut(&mut meta.row_groups)
    } else {
        Arc::make_mut(&mut meta.col_groups)
    }
}

fn groups_of(meta: &SheetMeta, rows: bool) -> &[OutlineGroupData] {
    if rows {
        &meta.row_groups
    } else {
        &meta.col_groups
    }
}

fn check_span(rows: bool, start: u32, end: u32) -> Result<(), ApiError> {
    if start > end || end >= limit_of(rows) {
        return Err(ApiError::invalid_argument(format!(
            "bad {} range {start}..={end}",
            if rows { "row" } else { "column" }
        )));
    }
    Ok(())
}

fn sort_groups(groups: &mut [OutlineGroupData]) {
    groups.sort_by_key(|g| (g.level, g.start, g.end));
}

/// The hidden spans of a group list: the rows/columns of every collapsed
/// group.
pub fn hidden_spans(groups: &[OutlineGroupData]) -> Spans {
    Spans::from_spans(
        groups
            .iter()
            .filter(|g| g.collapsed)
            .map(|g| (g.start, g.end))
            .collect(),
    )
}

/// Recomputes the rows/columns hidden by collapsed groups of `sheet`.
/// Returns `true` if they changed.
pub fn reapply(wb: &mut Workbook, sheet: SheetId) -> bool {
    let Some(s) = wb.sheet_mut(sheet) else {
        return false;
    };
    let rows = hidden_spans(&s.meta.row_groups);
    let cols = hidden_spans(&s.meta.col_groups);
    let changed = s.outline_rows != rows || s.outline_cols != cols;
    s.outline_rows = rows;
    s.outline_cols = cols;
    changed
}

/// Inserts a group into `groups` with AOO nesting rules; returns the
/// group's level.
pub fn insert_group(
    groups: &mut Vec<OutlineGroupData>,
    start: u32,
    end: u32,
) -> Result<u8, ApiError> {
    if groups.len() >= MAX_GROUPS {
        return Err(ApiError::invalid_argument(format!(
            "at most {MAX_GROUPS} groups per sheet"
        )));
    }
    let mut level: u8 = 1;
    for g in groups.iter() {
        let contains = g.start <= start && end <= g.end;
        let inside = start <= g.start && g.end <= end;
        if contains && inside {
            // Identical range: nest the new group inside the old one.
            level = level.max(g.level + 1);
        } else if contains {
            level = level.max(g.level + 1);
        } else if inside {
            // Pushed deeper below.
        } else if g.end < start || end < g.start {
            // Disjoint.
        } else {
            return Err(ApiError::invalid_argument(
                "the group would partially overlap another group",
            ));
        }
    }
    let deepest_inside = groups
        .iter()
        .filter(|g| start <= g.start && g.end <= end && !(g.start == start && g.end == end))
        .map(|g| g.level)
        .max()
        .unwrap_or(0);
    // Groups inside the new one move one level deeper; levels are counted
    // from the new group's level.
    if level > MAX_LEVELS || deepest_inside.saturating_add(1) > MAX_LEVELS {
        return Err(ApiError::invalid_argument(format!(
            "at most {MAX_LEVELS} outline levels"
        )));
    }
    for g in groups.iter_mut() {
        let inside = start <= g.start && g.end <= end && !(g.start == start && g.end == end);
        if inside && g.level >= level {
            g.level += 1;
            if g.level > MAX_LEVELS {
                return Err(ApiError::invalid_argument(format!(
                    "at most {MAX_LEVELS} outline levels"
                )));
            }
        }
    }
    groups.push(OutlineGroupData {
        start,
        end,
        level,
        collapsed: false,
    });
    sort_groups(groups);
    Ok(level)
}

/// The index of the innermost group covering `start..=end` (an exact
/// match wins).
fn innermost(groups: &[OutlineGroupData], start: u32, end: u32) -> Option<usize> {
    groups
        .iter()
        .enumerate()
        .filter(|(_, g)| g.start <= start && end <= g.end)
        .max_by_key(|(_, g)| (g.level, g.start == start && g.end == end))
        .map(|(i, _)| i)
}

/// Removes the innermost group covering `start..=end`; nested groups come
/// up one level.
pub fn remove_group(
    groups: &mut Vec<OutlineGroupData>,
    start: u32,
    end: u32,
) -> Result<OutlineGroupData, ApiError> {
    let i = innermost(groups, start, end)
        .ok_or_else(|| ApiError::not_found("no group covers the selection"))?;
    let g = groups.remove(i);
    for x in groups.iter_mut() {
        if g.start <= x.start && x.end <= g.end && x.level > g.level {
            x.level -= 1;
        }
    }
    sort_groups(groups);
    Ok(g)
}

/// The API form of a sheet's outline (`None` without groups).
pub fn to_api(meta: &SheetMeta) -> Option<OutlineDto> {
    if meta.row_groups.is_empty() && meta.col_groups.is_empty() {
        return None;
    }
    let conv = |g: &OutlineGroupData| OutlineGroupDto {
        start: g.start,
        end: g.end,
        level: g.level,
        collapsed: g.collapsed,
    };
    Some(OutlineDto {
        rows: meta.row_groups.iter().map(conv).collect(),
        cols: meta.col_groups.iter().map(conv).collect(),
        row_levels: meta.row_groups.iter().map(|g| g.level).max().unwrap_or(0),
        col_levels: meta.col_groups.iter().map(|g| g.level).max().unwrap_or(0),
    })
}

fn finish(
    wb: &mut Workbook,
    sheet: SheetId,
    fx: &mut Effects,
    changes: Vec<Change>,
) -> Vec<Change> {
    if reapply(wb, sheet) {
        // SUBTOTAL 101–111 and AGGREGATE see the hidden rows.
        wb.dirty_visibility_dependents(&[sheet]);
    }
    wb.pending
        .display_only
        .push(CellRange::new(sheet, 0, 0, MAX_ROWS - 1, MAX_COLS - 1));
    fx.layout(sheet);
    changes
}

/// `Op::Group`.
pub fn group(
    wb: &mut Workbook,
    sheet: SheetId,
    axis: OutlineAxis,
    start: u32,
    end: u32,
    fx: &mut Effects,
) -> Result<Vec<Change>, ApiError> {
    check_sheet(wb, sheet)?;
    let rows = rows_of(axis);
    check_span(rows, start, end)?;
    check_sheet_action(wb, sheet, |p| p.format_cells)?;
    let ch = with_meta(wb, sheet, fx, |_, meta| {
        insert_group(groups_mut(meta, rows), start, end).map(|_| ())
    })?;
    Ok(finish(wb, sheet, fx, ch))
}

/// `Op::Ungroup`.
pub fn ungroup(
    wb: &mut Workbook,
    sheet: SheetId,
    axis: OutlineAxis,
    start: u32,
    end: u32,
    fx: &mut Effects,
) -> Result<Vec<Change>, ApiError> {
    check_sheet(wb, sheet)?;
    let rows = rows_of(axis);
    check_span(rows, start, end)?;
    check_sheet_action(wb, sheet, |p| p.format_cells)?;
    let ch = with_meta(wb, sheet, fx, |_, meta| {
        remove_group(groups_mut(meta, rows), start, end).map(|_| ())
    })?;
    Ok(finish(wb, sheet, fx, ch))
}

/// `Op::SetOutlineCollapsed`.
pub fn set_collapsed(
    wb: &mut Workbook,
    sheet: SheetId,
    axis: OutlineAxis,
    start: u32,
    end: u32,
    collapsed: bool,
    fx: &mut Effects,
) -> Result<Vec<Change>, ApiError> {
    check_sheet(wb, sheet)?;
    let rows = rows_of(axis);
    check_span(rows, start, end)?;
    let ch = with_meta(wb, sheet, fx, |_, meta| {
        let list = groups_of(meta, rows);
        let i = innermost(list, start, end)
            .ok_or_else(|| ApiError::not_found("no group covers the selection"))?;
        if list[i].collapsed == collapsed {
            return Ok(());
        }
        groups_mut(meta, rows)[i].collapsed = collapsed;
        Ok(())
    })?;
    Ok(finish(wb, sheet, fx, ch))
}

/// `Op::ShowOutlineLevel`: groups deeper than `level` collapse, the rest
/// expand (`0` collapses everything, `8` expands everything).
pub fn show_level(
    wb: &mut Workbook,
    sheet: SheetId,
    axis: OutlineAxis,
    level: u8,
    fx: &mut Effects,
) -> Result<Vec<Change>, ApiError> {
    check_sheet(wb, sheet)?;
    let rows = rows_of(axis);
    let ch = with_meta(wb, sheet, fx, |_, meta| {
        let need = groups_of(meta, rows)
            .iter()
            .any(|g| g.collapsed != (g.level > level));
        if need {
            for g in groups_mut(meta, rows).iter_mut() {
                g.collapsed = g.level > level;
            }
        }
        Ok(())
    })?;
    Ok(finish(wb, sheet, fx, ch))
}

/// `Op::ClearOutline`.
pub fn clear(
    wb: &mut Workbook,
    sheet: SheetId,
    axis: Option<OutlineAxis>,
    fx: &mut Effects,
) -> Result<Vec<Change>, ApiError> {
    check_sheet(wb, sheet)?;
    check_sheet_action(wb, sheet, |p| p.format_cells)?;
    let ch = with_meta(wb, sheet, fx, |_, meta| {
        if axis.is_none_or(rows_of) && !meta.row_groups.is_empty() {
            meta.row_groups = Arc::new(Vec::new());
        }
        if axis.is_none_or(|a| !rows_of(a)) && !meta.col_groups.is_empty() {
            meta.col_groups = Arc::new(Vec::new());
        }
        Ok(())
    })?;
    Ok(finish(wb, sheet, fx, ch))
}

/// Detected row and column spans.
pub type DetectedGroups = (Vec<(u32, u32)>, Vec<(u32, u32)>);

/// The groups AutoOutline detects in `range`: a formula whose reference
/// covers a contiguous run of cells directly above it (same column) makes
/// a row group of those rows; one covering cells directly to its left
/// (same row) makes a column group (AOO `ScTable::DoAutoOutline`).
pub fn detect_groups(wb: &Workbook, range: &CellRange) -> DetectedGroups {
    let mut rows: Vec<(u32, u32)> = Vec::new();
    let mut cols: Vec<(u32, u32)> = Vec::new();
    for (_, f) in &wb.formulas {
        let a = f.addr;
        if a.sheet != range.sheet || !range.contains(a) {
            continue;
        }
        for p in &f.precedents {
            if p.sheet != a.sheet {
                continue;
            }
            if p.c0 == a.col && p.c1 == a.col && p.r1.checked_add(1) == Some(a.row) && p.r0 < p.r1 {
                rows.push((p.r0, p.r1));
            }
            if p.r0 == a.row && p.r1 == a.row && p.c1.checked_add(1) == Some(a.col) && p.c0 < p.c1 {
                cols.push((p.c0, p.c1));
            }
        }
    }
    rows.sort_unstable();
    rows.dedup();
    cols.sort_unstable();
    cols.dedup();
    (rows, cols)
}

/// `Op::AutoOutline`.
pub fn auto_outline(
    wb: &mut Workbook,
    sheet: SheetId,
    range: Option<RangeDto>,
    fx: &mut Effects,
) -> Result<Vec<Change>, ApiError> {
    check_sheet(wb, sheet)?;
    check_sheet_action(wb, sheet, |p| p.format_cells)?;
    let range = match range {
        Some(r) => crate::ops::to_range(sheet, &r)?,
        None => match wb.sheet(sheet).and_then(|s| s.used_range()) {
            Some(r) => r,
            None => return Ok(Vec::new()),
        },
    };
    let (rows, cols) = detect_groups(wb, &range);
    if rows.is_empty() && cols.is_empty() {
        return Ok(Vec::new());
    }
    let ch = with_meta(wb, sheet, fx, |_, meta| {
        // Widest first, so inner groups nest under outer ones.
        let mut rows = rows;
        rows.sort_by_key(|(a, b)| (std::cmp::Reverse(b - a), *a));
        for (a, b) in rows {
            let list = groups_mut(meta, true);
            if list.iter().any(|g| g.start == a && g.end == b) {
                continue;
            }
            let _ = insert_group(list, a, b);
        }
        let mut cols = cols;
        cols.sort_by_key(|(a, b)| (std::cmp::Reverse(b - a), *a));
        for (a, b) in cols {
            let list = groups_mut(meta, false);
            if list.iter().any(|g| g.start == a && g.end == b) {
                continue;
            }
            let _ = insert_group(list, a, b);
        }
        Ok(())
    })?;
    Ok(finish(wb, sheet, fx, ch))
}

/// Moves outline groups for a row/column change on `sh.sheet` (a group
/// grows, shrinks or goes away like an area reference).
pub fn adjust_structure(wb: &mut Workbook, sh: &Shape) {
    let Some(s) = wb.sheet_mut(sh.sheet) else {
        return;
    };
    let list = if sh.rows {
        &mut s.meta.row_groups
    } else {
        &mut s.meta.col_groups
    };
    if list.is_empty() {
        return;
    }
    let moved: Vec<OutlineGroupData> = list
        .iter()
        .filter_map(|g| {
            span_after(sh, g.start, g.end).map(|(start, end)| OutlineGroupData { start, end, ..*g })
        })
        .collect();
    if *list.as_ref() != moved {
        let mut moved = moved;
        sort_groups(&mut moved);
        *list = Arc::new(moved);
    }
    reapply(wb, sh.sheet);
}

/// For files: rows/columns of collapsed groups are written hidden, as
/// AOO and Excel keep them.
pub fn export_hidden(wb: &Workbook, sheet: SheetId, sd: &mut tally_model::SheetData) {
    let Some(s) = wb.sheet(sheet) else {
        return;
    };
    let default_row = crate::units::px_to_hmm(crate::units::DEFAULT_ROW_HEIGHT_PX);
    let default_col = crate::units::px_to_hmm(crate::units::DEFAULT_COL_WIDTH_PX);
    if !s.outline_rows.is_empty() {
        let mut dims = s.row_dims.clone();
        for (a, b) in s.outline_rows.spans() {
            dims.set_hidden(*a, *b, true, default_row);
        }
        sd.row_heights = dims.runs().to_vec();
    }
    if !s.outline_cols.is_empty() {
        let mut dims = s.col_dims.clone();
        for (a, b) in s.outline_cols.spans() {
            dims.set_hidden(*a, *b, true, default_col);
        }
        sd.col_widths = dims.runs().to_vec();
    }
}

/// After a load: rows/columns hidden by a file inside collapsed groups
/// are the groups' doing, so the user-hidden flag is cleared there and
/// the outline spans take over.
pub fn after_load(wb: &mut Workbook, sheet: SheetId) {
    let Some(s) = wb.sheet_mut(sheet) else {
        return;
    };
    let default_row = crate::units::px_to_hmm(crate::units::DEFAULT_ROW_HEIGHT_PX);
    let default_col = crate::units::px_to_hmm(crate::units::DEFAULT_COL_WIDTH_PX);
    for (a, b) in hidden_spans(&s.meta.row_groups).spans() {
        s.row_dims.set_hidden(*a, *b, false, default_row);
    }
    for (a, b) in hidden_spans(&s.meta.col_groups).spans() {
        s.col_dims.set_hidden(*a, *b, false, default_col);
    }
    reapply(wb, sheet);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nesting_rules() {
        let mut g = Vec::new();
        assert_eq!(insert_group(&mut g, 2, 10).unwrap(), 1);
        assert_eq!(insert_group(&mut g, 3, 5).unwrap(), 2);
        assert_eq!(insert_group(&mut g, 0, 20).unwrap(), 1, "outer group");
        assert_eq!(g.iter().map(|x| x.level).collect::<Vec<_>>(), vec![1, 2, 3]);
        assert!(insert_group(&mut g, 4, 12).is_err(), "partial overlap");
        let r = remove_group(&mut g, 3, 5).unwrap();
        assert_eq!((r.start, r.end), (3, 5));
        assert_eq!(g.len(), 2);
        let r = remove_group(&mut g, 2, 10).unwrap();
        assert_eq!(r.level, 2);
        assert_eq!(g[0].level, 1);
    }

    #[test]
    fn level_limit() {
        let mut g = Vec::new();
        for i in 0..8u32 {
            insert_group(&mut g, i, 20 - i).unwrap();
        }
        assert!(insert_group(&mut g, 9, 11).is_err());
    }
}
