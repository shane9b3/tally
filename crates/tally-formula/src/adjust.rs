//! Reference adjustment for structural changes, and relocation.
//!
//! References are stored relative to the formula's cell, so a structural
//! change has to account for two movements: the referenced cells move, and
//! the formula's own cell may move. [`adjust_for_structure_change`] takes the
//! formula's address *before* the change and rewrites the AST so that
//! printing or evaluating it at the cell's address *after* the change yields
//! the adjusted references.
//!
//! Rules (OpenOffice defaults, "expand references" off):
//! - inserting at or before a reference shifts it; inserting strictly inside
//!   an area (after its first row/column, up to and including its last) grows
//!   the area; inserting right after an area leaves it alone;
//! - deleting cells a single reference points at makes it `#REF!`; deleting
//!   part of an area shrinks it; deleting all of it makes it `#REF!`;
//! - whole-column refs are unaffected by row changes and vice versa;
//! - deleting a sheet turns references to it into [`SheetRef::Deleted`];
//!   renaming is a no-op because references hold sheet ids.

use tally_types::{CellAddr, SheetId, MAX_COLS, MAX_ROWS};

use crate::ast::{CellRefExpr, Formula, RefCoord, RefExpr, SheetRef};

/// A structural edit of the workbook.
#[derive(Clone, Copy, Eq, PartialEq, Hash, Debug)]
pub enum StructureChange {
    /// Insert `count` rows before 0-based row `at` on `sheet`.
    InsertRows {
        /// The sheet.
        sheet: SheetId,
        /// First new row.
        at: u32,
        /// Number of rows.
        count: u32,
    },
    /// Delete rows `at..at+count` on `sheet`.
    DeleteRows {
        /// The sheet.
        sheet: SheetId,
        /// First deleted row.
        at: u32,
        /// Number of rows.
        count: u32,
    },
    /// Insert `count` columns before 0-based column `at` on `sheet`.
    InsertCols {
        /// The sheet.
        sheet: SheetId,
        /// First new column.
        at: u32,
        /// Number of columns.
        count: u32,
    },
    /// Delete columns `at..at+count` on `sheet`.
    DeleteCols {
        /// The sheet.
        sheet: SheetId,
        /// First deleted column.
        at: u32,
        /// Number of columns.
        count: u32,
    },
    /// Delete a sheet.
    DeleteSheet {
        /// The sheet.
        sheet: SheetId,
    },
    /// Rename a sheet: a no-op for ASTs, which reference sheets by id.
    RenameSheet {
        /// The sheet.
        sheet: SheetId,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Axis {
    Row,
    Col,
}

#[derive(Clone, Copy)]
enum Edit {
    Insert { at: u32, count: u32 },
    Delete { at: u32, count: u32 },
}

#[derive(Clone, Copy)]
struct LineChange {
    sheet: SheetId,
    axis: Axis,
    edit: Edit,
}

impl LineChange {
    fn of(ch: &StructureChange) -> Option<Self> {
        let (sheet, axis, edit) = match *ch {
            StructureChange::InsertRows { sheet, at, count } => {
                (sheet, Axis::Row, Edit::Insert { at, count })
            }
            StructureChange::DeleteRows { sheet, at, count } => {
                (sheet, Axis::Row, Edit::Delete { at, count })
            }
            StructureChange::InsertCols { sheet, at, count } => {
                (sheet, Axis::Col, Edit::Insert { at, count })
            }
            StructureChange::DeleteCols { sheet, at, count } => {
                (sheet, Axis::Col, Edit::Delete { at, count })
            }
            StructureChange::DeleteSheet { .. } | StructureChange::RenameSheet { .. } => {
                return None
            }
        };
        Some(Self { sheet, axis, edit })
    }

    fn limit(self) -> u32 {
        match self.axis {
            Axis::Row => MAX_ROWS,
            Axis::Col => MAX_COLS,
        }
    }

    /// New position of a single line, or `None` if deleted / pushed off.
    fn point(self, p: u32) -> Option<u32> {
        match self.edit {
            Edit::Insert { at, count } => {
                if p >= at {
                    let n = p.checked_add(count)?;
                    (n < self.limit()).then_some(n)
                } else {
                    Some(p)
                }
            }
            Edit::Delete { at, count } => {
                let end = at.saturating_add(count);
                if p < at {
                    Some(p)
                } else if p < end {
                    None
                } else {
                    Some(p - count)
                }
            }
        }
    }

    /// New span of an inclusive interval `lo..=hi`.
    fn span(self, lo: u32, hi: u32) -> Option<(u32, u32)> {
        let max = self.limit() - 1;
        match self.edit {
            Edit::Insert { at, count } => {
                if at <= lo {
                    let nlo = lo.saturating_add(count);
                    if nlo > max {
                        return None;
                    }
                    Some((nlo, hi.saturating_add(count).min(max)))
                } else if at <= hi {
                    Some((lo, hi.saturating_add(count).min(max)))
                } else {
                    Some((lo, hi))
                }
            }
            Edit::Delete { at, count } => {
                if count == 0 {
                    return Some((lo, hi));
                }
                let d1 = at.saturating_add(count - 1);
                if d1 < lo {
                    Some((lo - count, hi - count))
                } else if at > hi {
                    Some((lo, hi))
                } else if at <= lo && d1 >= hi {
                    None
                } else {
                    let nlo = lo.min(at);
                    let nhi = if hi > d1 { hi - count } else { at - 1 };
                    Some((nlo, nhi))
                }
            }
        }
    }
}

fn shift_origin(origin: CellAddr, ch: &StructureChange) -> CellAddr {
    let Some(lc) = LineChange::of(ch) else {
        return origin;
    };
    if lc.sheet != origin.sheet {
        return origin;
    }
    let mut o = origin;
    match lc.axis {
        Axis::Row => o.row = lc.point(o.row).unwrap_or(o.row),
        Axis::Col => o.col = lc.point(o.col).unwrap_or(o.col),
    }
    o
}

fn sheet_id(s: &SheetRef, current: SheetId) -> Option<SheetId> {
    match s {
        SheetRef::Current => Some(current),
        SheetRef::Id { id, .. } => Some(*id),
        _ => None,
    }
}

struct Ctx {
    old: CellAddr,
    new: CellAddr,
    lc: LineChange,
}

impl Ctx {
    fn origin_on(&self, axis: Axis, o: CellAddr) -> u32 {
        match axis {
            Axis::Row => o.row,
            Axis::Col => o.col,
        }
    }

    fn limit(axis: Axis) -> u32 {
        match axis {
            Axis::Row => MAX_ROWS,
            Axis::Col => MAX_COLS,
        }
    }

    /// Re-encodes a coordinate from the old origin to the new one, moving the
    /// absolute position through `f`. `None` means the reference is gone.
    fn recode(&self, c: RefCoord, axis: Axis, moved: Option<u32>) -> Option<RefCoord> {
        let pos = moved?;
        Some(RefCoord::encode(pos, c.abs, self.origin_on(axis, self.new)))
    }

    fn pos(&self, c: RefCoord, axis: Axis) -> Option<u32> {
        c.resolve(self.origin_on(axis, self.old), Self::limit(axis))
    }

    fn cell(&self, c: &CellRefExpr, on_target: bool) -> Option<CellRefExpr> {
        let row = self.pos(c.row, Axis::Row)?;
        let col = self.pos(c.col, Axis::Col)?;
        let (nrow, ncol) = if on_target {
            match self.lc.axis {
                Axis::Row => (self.lc.point(row), Some(col)),
                Axis::Col => (Some(row), self.lc.point(col)),
            }
        } else {
            (Some(row), Some(col))
        };
        Some(CellRefExpr {
            sheet: c.sheet.clone(),
            row: self.recode(c.row, Axis::Row, nrow)?,
            col: self.recode(c.col, Axis::Col, ncol)?,
        })
    }

    /// Moves an interval written as (a, b) along `axis`, keeping the
    /// written orientation.
    fn interval(&self, a: u32, b: u32, on_target: bool, axis: Axis) -> Option<(u32, u32)> {
        if !on_target || self.lc.axis != axis {
            return Some((a, b));
        }
        let (lo, hi) = (a.min(b), a.max(b));
        let (nlo, nhi) = self.lc.span(lo, hi)?;
        Some(if a <= b { (nlo, nhi) } else { (nhi, nlo) })
    }

    fn adjust(&self, r: &RefExpr) -> Option<RefExpr> {
        let cur = self.old.sheet;
        match r {
            RefExpr::Invalid => Some(RefExpr::Invalid),
            RefExpr::Cell(c) => {
                let on = sheet_id(&c.sheet, cur) == Some(self.lc.sheet);
                Some(RefExpr::Cell(self.cell(c, on)?))
            }
            RefExpr::Area(a, b) => {
                let sa = sheet_id(&a.sheet, cur);
                let sb = match b.sheet {
                    SheetRef::Current => sa,
                    _ => sheet_id(&b.sheet, cur),
                };
                let on = sa == Some(self.lc.sheet) || sb == Some(self.lc.sheet);
                let (ar, br) = (self.pos(a.row, Axis::Row)?, self.pos(b.row, Axis::Row)?);
                let (ac, bc) = (self.pos(a.col, Axis::Col)?, self.pos(b.col, Axis::Col)?);
                let (ar, br) = self.interval(ar, br, on, Axis::Row)?;
                let (ac, bc) = self.interval(ac, bc, on, Axis::Col)?;
                Some(RefExpr::Area(
                    CellRefExpr {
                        sheet: a.sheet.clone(),
                        row: self.recode(a.row, Axis::Row, Some(ar))?,
                        col: self.recode(a.col, Axis::Col, Some(ac))?,
                    },
                    CellRefExpr {
                        sheet: b.sheet.clone(),
                        row: self.recode(b.row, Axis::Row, Some(br))?,
                        col: self.recode(b.col, Axis::Col, Some(bc))?,
                    },
                ))
            }
            RefExpr::Cols { sheet, start, end } => {
                let on = sheet_id(sheet, cur) == Some(self.lc.sheet);
                let (s, e) = (self.pos(*start, Axis::Col)?, self.pos(*end, Axis::Col)?);
                let (s, e) = self.interval(s, e, on, Axis::Col)?;
                Some(RefExpr::Cols {
                    sheet: sheet.clone(),
                    start: self.recode(*start, Axis::Col, Some(s))?,
                    end: self.recode(*end, Axis::Col, Some(e))?,
                })
            }
            RefExpr::Rows { sheet, start, end } => {
                let on = sheet_id(sheet, cur) == Some(self.lc.sheet);
                let (s, e) = (self.pos(*start, Axis::Row)?, self.pos(*end, Axis::Row)?);
                let (s, e) = self.interval(s, e, on, Axis::Row)?;
                Some(RefExpr::Rows {
                    sheet: sheet.clone(),
                    start: self.recode(*start, Axis::Row, Some(s))?,
                    end: self.recode(*end, Axis::Row, Some(e))?,
                })
            }
        }
    }

    /// References already out of the grid are left untouched.
    fn resolvable(&self, r: &RefExpr) -> bool {
        let p = |c: RefCoord, axis| self.pos(c, axis).is_some();
        match r {
            RefExpr::Invalid => false,
            RefExpr::Cell(c) => p(c.row, Axis::Row) && p(c.col, Axis::Col),
            RefExpr::Area(a, b) => {
                p(a.row, Axis::Row)
                    && p(a.col, Axis::Col)
                    && p(b.row, Axis::Row)
                    && p(b.col, Axis::Col)
            }
            RefExpr::Cols { start, end, .. } => p(*start, Axis::Col) && p(*end, Axis::Col),
            RefExpr::Rows { start, end, .. } => p(*start, Axis::Row) && p(*end, Axis::Row),
        }
    }
}

fn delete_sheet(r: &mut RefExpr, sheet: SheetId) -> bool {
    let hit = |s: &SheetRef| matches!(s, SheetRef::Id { id, .. } if *id == sheet);
    let mut changed = false;
    let mut mark = |s: &mut SheetRef| {
        if hit(s) {
            *s = SheetRef::Deleted;
            changed = true;
        }
    };
    match r {
        RefExpr::Cell(c) => mark(&mut c.sheet),
        RefExpr::Area(a, b) => {
            mark(&mut a.sheet);
            mark(&mut b.sheet);
        }
        RefExpr::Cols { sheet: s, .. } | RefExpr::Rows { sheet: s, .. } => mark(s),
        RefExpr::Invalid => {}
    }
    changed
}

/// Rewrites the references of `f` (a formula at `origin`, its address before
/// the change) for a structural change. Returns `true` if the AST changed.
///
/// The formula's own cell moves with the change (the caller stores it at the
/// shifted address); relative offsets are re-encoded for that new address.
pub fn adjust_for_structure_change(
    f: &mut Formula,
    origin: CellAddr,
    change: &StructureChange,
) -> bool {
    let mut changed = false;
    match change {
        StructureChange::RenameSheet { .. } => {}
        StructureChange::DeleteSheet { sheet } => {
            f.expr.walk_refs_mut(|r| changed |= delete_sheet(r, *sheet));
        }
        _ => {
            let Some(lc) = LineChange::of(change) else {
                return false;
            };
            let ctx = Ctx {
                old: origin,
                new: shift_origin(origin, change),
                lc,
            };
            f.expr.walk_refs_mut(|r| {
                if !ctx.resolvable(r) {
                    return;
                }
                let new = ctx.adjust(r).unwrap_or(RefExpr::Invalid);
                if new != *r {
                    *r = new;
                    changed = true;
                }
            });
        }
    }
    changed
}

/// The cell a formula at `origin` lives at after `change` (unchanged if the
/// change does not move it; a deleted origin keeps its address).
pub fn shifted_origin(origin: CellAddr, change: &StructureChange) -> CellAddr {
    shift_origin(origin, change)
}

/// Relocates a formula from `from` to `to` for copy/paste or fill.
///
/// Because references are stored relative to the formula's cell, this is the
/// identity on the AST: the copy is simply printed (or evaluated) at `to`.
/// Relative references that fall outside the sheet at `to` print as
/// `#REF!`. The addresses are accepted only to document intent.
pub fn relocate(f: &Formula, from: CellAddr, to: CellAddr) -> Formula {
    let _ = (from, to);
    f.clone()
}
