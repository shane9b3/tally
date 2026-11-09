//! Database ranges, the sheet's standard filter and its subtotal rules
//! follow row/column inserts and deletes, cell shifts and block moves,
//! with the columns their parameters name (AOO
//! `ScDBCollection::UpdateReference`, dbcolect.cxx:818-860, and
//! `ScDBData::MoveTo`, dbcolect.cxx:399-437).
//!
//! AOO moves the query, sort and subtotal fields by the change of the
//! range's first column only; here each named column is moved on its own,
//! so a column inserted or deleted inside the range keeps the parameters
//! on their columns. A parameter whose column is deleted is dropped (AOO
//! turns off the entries that fall past the range's end). A database
//! range whose rows (or columns) are all deleted stays as one row (column)
//! at the deletion point, as `ScRefUpdate::Update` leaves it
//! (refupdat.cxx:301-311: `theRow2 < theRow1` → `theRow2 = theRow1`).

use std::sync::Arc;

use tally_model::{
    CellRangeLocal, DatabaseRangeData, DbSortData, StandardFilterData, SubtotalsData,
};

use crate::storage::SheetMeta;

/// How areas move.
pub(crate) struct AreaMove<'a> {
    /// The area after the change (`None`: deleted).
    pub range: &'a dyn Fn(&CellRangeLocal) -> Option<CellRangeLocal>,
    /// Where column `c` of the area goes (`None`: deleted).
    pub col: &'a dyn Fn(&CellRangeLocal, u32) -> Option<u32>,
    /// A deleted database range's one-row (one-column) remainder, or
    /// `None` to remove it.
    pub collapse: &'a dyn Fn(&CellRangeLocal) -> Option<CellRangeLocal>,
}

/// Moves a filter's area and condition columns; `false` when no condition
/// is left (an advanced filter keeps its criteria block).
fn move_filter(
    f: &mut StandardFilterData,
    old: &CellRangeLocal,
    range: CellRangeLocal,
    m: &AreaMove,
) -> bool {
    f.range = range;
    let had = !f.conditions.is_empty();
    f.conditions.retain_mut(|c| match (m.col)(old, c.col) {
        Some(n) => {
            c.col = n;
            true
        }
        None => false,
    });
    f.criteria.is_some() || !had || !f.conditions.is_empty()
}

/// Moves the subtotal area and group / total columns; `false` when no group is left.
fn move_subtotals(
    st: &mut SubtotalsData,
    old: &CellRangeLocal,
    range: CellRangeLocal,
    m: &AreaMove,
) -> bool {
    st.range = range;
    st.groups.retain_mut(|g| match (m.col)(old, g.group_by) {
        Some(n) => {
            g.group_by = n;
            g.columns = g.columns.iter().filter_map(|&c| (m.col)(old, c)).collect();
            true
        }
        None => false,
    });
    !st.groups.is_empty()
}

/// Moves the kept sort keys (offsets from the first column); `false` when none is left.
fn move_sort(
    so: &mut DbSortData,
    old: &CellRangeLocal,
    range: &CellRangeLocal,
    m: &AreaMove,
) -> bool {
    so.keys = so
        .keys
        .iter()
        .filter_map(|&(f, asc)| {
            let n = (m.col)(old, old.c0.checked_add(f)?)?;
            (range.c0 <= n && n <= range.c1).then(|| (n - range.c0, asc))
        })
        .collect();
    !so.keys.is_empty()
}

fn move_db(d: &mut DatabaseRangeData, m: &AreaMove) -> bool {
    let old = d.range;
    let Some(range) = (m.range)(&old).or_else(|| (m.collapse)(&old)) else {
        return false;
    };
    if range == old {
        return true;
    }
    d.range = range;
    if let Some(f) = d.filter.as_mut() {
        if !move_filter(f, &old, range, m) {
            d.filter = None;
        }
    }
    if let Some(st) = d.subtotals.as_mut() {
        // The subtotal area starts where the database range does.
        let st_old = st.range;
        let st_new = (m.range)(&st_old).unwrap_or(range);
        if !move_subtotals(st, &st_old, st_new, m) {
            d.subtotals = None;
        }
    }
    if let Some(so) = d.sort.as_mut() {
        if !move_sort(so, &old, &range, m) {
            d.sort = None;
        }
    }
    true
}

/// Moves the database ranges, the standard filter and the subtotal rules
/// of `meta`; collections that do not change keep their `Arc`.
pub(crate) fn adjust(meta: &mut SheetMeta, m: &AreaMove) {
    if meta
        .database_ranges
        .iter()
        .any(|d| (m.range)(&d.range) != Some(d.range))
    {
        Arc::make_mut(&mut meta.database_ranges).retain_mut(|d| move_db(d, m));
    }
    if let Some(sf) = meta.standard_filter.as_ref() {
        let old = sf.range;
        match (m.range)(&old) {
            Some(r) if r == old => {}
            Some(r) => {
                let mut n = (**sf).clone();
                meta.standard_filter = move_filter(&mut n, &old, r, m).then(|| Arc::new(n));
            }
            None => meta.standard_filter = None,
        }
    }
    if let Some(st) = meta.subtotals.as_ref() {
        let old = st.range;
        match (m.range)(&old) {
            Some(r) if r == old => {}
            Some(r) => {
                let mut n = (**st).clone();
                meta.subtotals = move_subtotals(&mut n, &old, r, m).then(|| Arc::new(n));
            }
            None => meta.subtotals = None,
        }
    }
}

/// `true` when every column a database range's parameters name lies in
/// its area (Refresh Range refuses the range otherwise).
pub(crate) fn params_inside(d: &DatabaseRangeData) -> bool {
    let r = d.range;
    let inside = |c: u32| r.c0 <= c && c <= r.c1;
    let width = r.c1 - r.c0;
    d.filter
        .as_ref()
        .is_none_or(|f| f.conditions.iter().all(|c| inside(c.col)))
        && d.subtotals.as_ref().is_none_or(|st| {
            st.groups
                .iter()
                .all(|g| inside(g.group_by) && g.columns.iter().all(|&c| inside(c)))
        })
        && d.sort
            .as_ref()
            .is_none_or(|so| so.keys.iter().all(|&(f, _)| f <= width))
}
