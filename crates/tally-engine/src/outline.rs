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
use tally_types::{CellAddr, CellRange, SheetId, MAX_COLS, MAX_ROWS};

use crate::ops::check_sheet;
use crate::ops_v03::with_meta;
use crate::protection::check_sheet_action;
use crate::storage::{SheetMeta, Spans};
use crate::structure::{span_after, Shape};
use crate::undo::{Change, Effects};
use crate::workbook::Workbook;

/// Deepest outline level: AOO `SC_OL_MAXDEPTH` (`sc/inc/olinetab.hxx:31`,
/// enforced in `ScOutlineArray::Insert`, `olinetab.cxx:240-243`); Excel's
/// limit is also 7 (correctness review 4, M5).
pub const MAX_LEVELS: u8 = 7;

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

/// The index range of the groups of `level` in a sorted list.
fn level_range(groups: &[OutlineGroupData], level: u8) -> std::ops::Range<usize> {
    groups.partition_point(|g| g.level < level)..groups.partition_point(|g| g.level <= level)
}

/// The groups of one level (sorted by start, disjoint) that overlap
/// `start..=end`, as indices into `groups`.
fn overlapping(
    groups: &[OutlineGroupData],
    level: u8,
    start: u32,
    end: u32,
) -> std::ops::Range<usize> {
    let r = level_range(groups, level);
    let slice = &groups[r.clone()];
    let a = slice.partition_point(|g| g.end < start);
    let b = slice.partition_point(|g| g.start <= end);
    r.start + a..r.start + b.max(a)
}

/// Inserts a group into `groups` (sorted by `(level, start, end)`) with
/// AOO nesting rules; returns the group's level. Binary searches per
/// level, so building a large outline is not quadratic (security review
/// 4, HIGH-T5).
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
    let deepest = groups.last().map_or(0, |g| g.level);
    let mut level: u8 = 1;
    let mut deepest_inside: u8 = 0;
    for l in 1..=deepest {
        for g in &groups[overlapping(groups, l, start, end)] {
            let contains = g.start <= start && end <= g.end;
            let inside = start <= g.start && g.end <= end;
            if contains {
                // Also an identical range: nest the new group inside.
                level = level.max(g.level + 1);
            } else if inside {
                deepest_inside = deepest_inside.max(g.level);
            } else {
                return Err(ApiError::invalid_argument(
                    "the group would partially overlap another group",
                ));
            }
        }
    }
    // Groups inside the new one move one level deeper; levels are counted
    // from the new group's level.
    if level > MAX_LEVELS || deepest_inside.saturating_add(1) > MAX_LEVELS {
        return Err(ApiError::invalid_argument(format!(
            "at most {MAX_LEVELS} outline levels"
        )));
    }
    let inside: Vec<usize> = (level..=deepest)
        .flat_map(|l| overlapping(groups, l, start, end))
        .filter(|i| {
            let g = &groups[*i];
            start <= g.start && g.end <= end && !(g.start == start && g.end == end)
        })
        .collect();
    let pushed = !inside.is_empty();
    for i in inside {
        groups[i].level += 1;
    }
    let g = OutlineGroupData {
        start,
        end,
        level,
        collapsed: false,
    };
    if pushed {
        groups.push(g);
        sort_groups(groups);
    } else {
        let at = groups.partition_point(|x| (x.level, x.start, x.end) < (level, start, end));
        groups.insert(at, g);
    }
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

/// The single range a formula refers to, as AOO's
/// `ScFormulaCell::HasRefListExpressibleAsOneReference`
/// (`sc/source/core/data/cell2.cxx:665-713`): exactly one reference, or
/// several references that are the only arguments of one function and
/// tile one rectangle (`lcl_refListFormsOneRange`, `cell2.cxx:333-380`).
pub fn single_reference(f: &tally_formula::Formula, origin: CellAddr) -> Option<CellRange> {
    use tally_formula::{Expr, ResolvedRef};
    let mut refs = Vec::new();
    f.expr.walk_refs(|r| refs.push(r.resolve(origin)));
    let rect = |r: &ResolvedRef| match r {
        ResolvedRef::Range(c) => Some(*c),
        _ => None,
    };
    if refs.len() == 1 {
        return rect(&refs[0]);
    }
    if refs.len() < 2 {
        return None;
    }
    // One call holds every reference, and only references.
    let mut ok = false;
    f.expr.walk(|e| {
        if let Expr::Call { args, .. } = e {
            if args.len() == refs.len()
                && args.iter().all(|a| {
                    let mut a = a;
                    while let Expr::Paren(x) = a {
                        a = x;
                    }
                    matches!(a, Expr::Ref(_))
                })
            {
                ok = true;
            }
        }
    });
    if !ok {
        return None;
    }
    let mut rs: Vec<CellRange> = refs.iter().map(rect).collect::<Option<_>>()?;
    let sheet = rs[0].sheet;
    if rs.iter().any(|r| r.sheet != sheet) {
        return None;
    }
    let same_rows = rs.iter().all(|r| r.r0 == rs[0].r0 && r.r1 == rs[0].r1);
    let same_cols = rs.iter().all(|r| r.c0 == rs[0].c0 && r.c1 == rs[0].c1);
    if same_rows {
        rs.sort_by_key(|r| r.c0);
        if rs
            .windows(2)
            .all(|w| w[0].c1.checked_add(1) == Some(w[1].c0))
        {
            let last = rs[rs.len() - 1];
            return Some(CellRange::new(sheet, rs[0].r0, rs[0].c0, last.r1, last.c1));
        }
    } else if same_cols {
        rs.sort_by_key(|r| r.r0);
        if rs
            .windows(2)
            .all(|w| w[0].r1.checked_add(1) == Some(w[1].r0))
        {
            let last = rs[rs.len() - 1];
            return Some(CellRange::new(sheet, rs[0].r0, rs[0].c0, last.r1, last.c1));
        }
    }
    None
}

fn sign(a: u32, b: u32) -> std::cmp::Ordering {
    a.cmp(&b)
}

/// Builds the outline AOO's `ScTable::DoAutoOutline` builds for `range`
/// (`sc/source/core/data/table2.cxx:3055-3131`) into empty row and column
/// lists:
/// - rows, top to bottom: the first formula of the row (left to right)
///   whose single reference ([`single_reference`]) lies in its own
///   column, entirely above or entirely below it, inserts a row group
///   over the referenced rows (the `DiffSign` rule);
/// - columns, left to right: the first formula of the column (top to
///   bottom) whose reference lies in its own row, entirely left or right
///   of it, inserts a column group.
///
/// A group that cannot be inserted (partial overlap, too deep) is
/// skipped and the next formula of the row/column is tried.
pub fn auto_groups(
    wb: &Workbook,
    range: &CellRange,
) -> (Vec<OutlineGroupData>, Vec<OutlineGroupData>) {
    let mut cells: Vec<(u32, u32, CellRange)> = Vec::new();
    for (_, f) in &wb.formulas {
        let a = f.addr;
        if a.sheet != range.sheet || !range.contains(a) || f.invalid_text.is_some() {
            continue;
        }
        if let Some(r) = single_reference(&f.formula, a) {
            if r.sheet == a.sheet {
                cells.push((a.row, a.col, r));
            }
        }
    }
    let mut rows: Vec<OutlineGroupData> = Vec::new();
    cells.sort_unstable_by_key(|(r, c, _)| (*r, *c));
    let mut done: Option<u32> = None;
    for (row, col, r) in &cells {
        if done == Some(*row) {
            continue;
        }
        if r.c0 == *col
            && r.c1 == *col
            && sign(r.r0, *row) == sign(r.r1, *row)
            && insert_group(&mut rows, r.r0, r.r1).is_ok()
        {
            done = Some(*row);
        }
    }
    let mut cols: Vec<OutlineGroupData> = Vec::new();
    cells.sort_unstable_by_key(|(r, c, _)| (*c, *r));
    let mut done: Option<u32> = None;
    for (row, col, r) in &cells {
        if done == Some(*col) {
            continue;
        }
        if r.r0 == *row
            && r.r1 == *row
            && sign(r.c0, *col) == sign(r.c1, *col)
            && insert_group(&mut cols, r.c0, r.c1).is_ok()
        {
            done = Some(*col);
        }
    }
    (rows, cols)
}

/// `Op::AutoOutline`: like AOO (`ScOutlineDocFunc::AutoOutline`,
/// `sc/source/ui/docshell/olinefun.cxx:300-324`) the existing outline is
/// removed first (its rows/columns shown), then rebuilt from the
/// formulas ([`auto_groups`]).
pub fn auto_outline(
    wb: &mut Workbook,
    sheet: SheetId,
    range: Option<RangeDto>,
    fx: &mut Effects,
) -> Result<Vec<Change>, ApiError> {
    check_sheet(wb, sheet)?;
    check_sheet_action(wb, sheet, |p| p.format_cells)?;
    let range = match range {
        Some(r) => Some(crate::ops::to_range(sheet, &r)?),
        None => wb.sheet(sheet).and_then(|s| s.used_range()),
    };
    let (rows, cols) = match &range {
        Some(r) => auto_groups(wb, r),
        None => (Vec::new(), Vec::new()),
    };
    let ch = with_meta(wb, sheet, fx, |_, meta| {
        if *meta.row_groups != rows {
            meta.row_groups = Arc::new(rows);
        }
        if *meta.col_groups != cols {
            meta.col_groups = Arc::new(cols);
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
        for i in 0..7u32 {
            insert_group(&mut g, i, 20 - i).unwrap();
        }
        assert!(insert_group(&mut g, 9, 11).is_err());
    }
}
