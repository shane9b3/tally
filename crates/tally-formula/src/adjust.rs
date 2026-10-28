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

use tally_types::{CellAddr, CellRange, SheetId, MAX_COLS, MAX_ROWS};

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

/// Decides whether a structural change on several sheets at once covers
/// every sheet of a 3D reference `first..last` (by tab order).
pub type Covers<'a> = &'a dyn Fn(SheetId, SheetId) -> bool;

struct Ctx<'a> {
    old: CellAddr,
    new: CellAddr,
    lc: LineChange,
    /// `None`: a one-sheet change (3D references stay). `Some`: the pass
    /// for a change made on several sheets at once, which moves only the
    /// 3D references it covers.
    three_d: Option<Covers<'a>>,
}

impl Ctx<'_> {
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
                let on = self.three_d.is_none() && sheet_id(&c.sheet, cur) == Some(self.lc.sheet);
                Some(RefExpr::Cell(self.cell(c, on)?))
            }
            RefExpr::Area(a, b) => {
                let sa = sheet_id(&a.sheet, cur);
                let sb = match b.sheet {
                    SheetRef::Current => sa,
                    _ => sheet_id(&b.sheet, cur),
                };
                // AOO `ScRefUpdate::Update`: a reference over several
                // sheets moves only when the change covers all of them.
                let on = match (sa, sb, self.three_d) {
                    (Some(x), Some(y), covers) if x != y => covers.is_some_and(|f| f(x, y)),
                    (_, _, Some(_)) => false,
                    _ => sa == Some(self.lc.sheet) || sb == Some(self.lc.sheet),
                };
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
                let on = self.three_d.is_none() && sheet_id(sheet, cur) == Some(self.lc.sheet);
                let (s, e) = (self.pos(*start, Axis::Col)?, self.pos(*end, Axis::Col)?);
                let (s, e) = self.interval(s, e, on, Axis::Col)?;
                Some(RefExpr::Cols {
                    sheet: sheet.clone(),
                    start: self.recode(*start, Axis::Col, Some(s))?,
                    end: self.recode(*end, Axis::Col, Some(e))?,
                })
            }
            RefExpr::Rows { sheet, start, end } => {
                let on = self.three_d.is_none() && sheet_id(sheet, cur) == Some(self.lc.sheet);
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
                three_d: None,
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

/// The second half of a row/column change made on several sheets at once
/// (a multi-sheet selection): after each sheet's own
/// [`adjust_for_structure_change`], which leaves references over several
/// sheets alone, this moves the 3D references whose sheets `covers` (all
/// inside the changed set), once. `origin` is the formula's current cell
/// (already moved); the `sheet` of `change` is ignored. Returns `true` if
/// the AST changed.
pub fn adjust_3d_for_structure_change(
    f: &mut Formula,
    origin: CellAddr,
    change: &StructureChange,
    covers: Covers<'_>,
) -> bool {
    let Some(lc) = LineChange::of(change) else {
        return false;
    };
    let ctx = Ctx {
        old: origin,
        new: origin,
        lc,
        three_d: Some(covers),
    };
    let mut changed = false;
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

// ---------------------------------------------------------------------------
// Moves (cut and paste) and re-anchoring (sort)
// ---------------------------------------------------------------------------

/// A block of cells moved on one sheet (cut and paste): `from` (whose
/// `sheet` is the sheet) goes to `from` shifted by (`dr`, `dc`).
#[derive(Clone, Copy, Eq, PartialEq, Hash, Debug)]
pub struct BlockMove {
    /// The source block.
    pub from: CellRange,
    /// Row offset of the destination.
    pub dr: i64,
    /// Column offset of the destination.
    pub dc: i64,
}

impl BlockMove {
    /// The destination block, if it fits on the grid.
    pub fn to(&self) -> Option<CellRange> {
        let sh = |v: u32, d: i64, limit: u32| -> Option<u32> {
            let x = i64::from(v) + d;
            (0..i64::from(limit)).contains(&x).then_some(x as u32)
        };
        Some(CellRange::new(
            self.from.sheet,
            sh(self.from.r0, self.dr, MAX_ROWS)?,
            sh(self.from.c0, self.dc, MAX_COLS)?,
            sh(self.from.r1, self.dr, MAX_ROWS)?,
            sh(self.from.c1, self.dc, MAX_COLS)?,
        ))
    }
}

/// What happens to one resolved reference target.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Action {
    /// Leave the AST as written (relative parts keep their offsets).
    Literal,
    /// Keep the absolute target (re-encode relative parts for the new origin).
    Keep,
    /// Move the target by (rows, cols).
    Shift(i64, i64),
    /// The target is gone: `#REF!`.
    Invalid,
}

/// The resolved absolute target of a reference: `(sheet, r0, c0, r1, c1)`;
/// `None` for whole rows/columns, 3-D spans and unresolvable references.
fn target_of(r: &RefExpr, origin: CellAddr) -> Option<CellRange> {
    let cell = |c: &CellRefExpr, sheet: SheetId| -> Option<(SheetId, u32, u32)> {
        let s = sheet_id(&c.sheet, sheet)?;
        Some((
            s,
            c.row.resolve(origin.row, MAX_ROWS)?,
            c.col.resolve(origin.col, MAX_COLS)?,
        ))
    };
    match r {
        RefExpr::Cell(c) => {
            let (s, row, col) = cell(c, origin.sheet)?;
            Some(CellRange::new(s, row, col, row, col))
        }
        RefExpr::Area(a, b) => {
            let (sa, ra, ca) = cell(a, origin.sheet)?;
            let (sb, rb, cb) = cell(b, sa)?;
            (sa == sb).then(|| CellRange::new(sa, ra.min(rb), ca.min(cb), ra.max(rb), ca.max(cb)))
        }
        _ => None,
    }
}

fn recode_coord(
    c: RefCoord,
    origin_old: u32,
    origin_new: u32,
    limit: u32,
    d: i64,
) -> Option<RefCoord> {
    let pos = i64::from(c.resolve(origin_old, limit)?) + d;
    if !(0..i64::from(limit)).contains(&pos) {
        return None;
    }
    Some(RefCoord::encode(pos as u32, c.abs, origin_new))
}

/// Applies `action` to `r` (a reference of a formula moving from `old` to `new`).
fn apply_action(r: &RefExpr, old: CellAddr, new: CellAddr, action: Action) -> RefExpr {
    let (dr, dc) = match action {
        Action::Literal => return r.clone(),
        Action::Invalid => return RefExpr::Invalid,
        Action::Keep => (0, 0),
        Action::Shift(dr, dc) => (dr, dc),
    };
    let rc = |c: &CellRefExpr| -> Option<CellRefExpr> {
        Some(CellRefExpr {
            sheet: c.sheet.clone(),
            row: recode_coord(c.row, old.row, new.row, MAX_ROWS, dr)?,
            col: recode_coord(c.col, old.col, new.col, MAX_COLS, dc)?,
        })
    };
    let out = match r {
        RefExpr::Invalid => Some(RefExpr::Invalid),
        RefExpr::Cell(c) => rc(c).map(RefExpr::Cell),
        RefExpr::Area(a, b) => match (rc(a), rc(b)) {
            (Some(a), Some(b)) => Some(RefExpr::Area(a, b)),
            _ => None,
        },
        RefExpr::Cols { sheet, start, end } => {
            match (
                recode_coord(*start, old.col, new.col, MAX_COLS, dc),
                recode_coord(*end, old.col, new.col, MAX_COLS, dc),
            ) {
                (Some(start), Some(end)) => Some(RefExpr::Cols {
                    sheet: sheet.clone(),
                    start,
                    end,
                }),
                _ => None,
            }
        }
        RefExpr::Rows { sheet, start, end } => {
            match (
                recode_coord(*start, old.row, new.row, MAX_ROWS, dr),
                recode_coord(*end, old.row, new.row, MAX_ROWS, dr),
            ) {
                (Some(start), Some(end)) => Some(RefExpr::Rows {
                    sheet: sheet.clone(),
                    start,
                    end,
                }),
                _ => None,
            }
        }
    };
    out.unwrap_or(RefExpr::Invalid)
}

/// `true` if every coordinate of `r` resolves at `origin` (references
/// already off the grid are left as written).
fn fully_resolvable(r: &RefExpr, origin: CellAddr) -> bool {
    let p = |c: RefCoord, o: u32, l: u32| c.resolve(o, l).is_some();
    match r {
        RefExpr::Invalid => false,
        RefExpr::Cell(c) => p(c.row, origin.row, MAX_ROWS) && p(c.col, origin.col, MAX_COLS),
        RefExpr::Area(a, b) => {
            p(a.row, origin.row, MAX_ROWS)
                && p(a.col, origin.col, MAX_COLS)
                && p(b.row, origin.row, MAX_ROWS)
                && p(b.col, origin.col, MAX_COLS)
        }
        RefExpr::Cols { start, end, .. } => {
            p(*start, origin.col, MAX_COLS) && p(*end, origin.col, MAX_COLS)
        }
        RefExpr::Rows { start, end, .. } => {
            p(*start, origin.row, MAX_ROWS) && p(*end, origin.row, MAX_ROWS)
        }
    }
}

fn remap(
    f: &mut Formula,
    old: CellAddr,
    new: CellAddr,
    mut decide: impl FnMut(Option<CellRange>) -> Action,
) -> bool {
    let mut changed = false;
    f.expr.walk_refs_mut(|r| {
        let action = if fully_resolvable(r, old) {
            decide(target_of(r, old))
        } else {
            Action::Literal
        };
        let n = apply_action(r, old, new, action);
        if n != *r {
            *r = n;
            changed = true;
        }
    });
    changed
}

/// Rewrites the references of a formula for a block move (cut and paste,
/// OpenOffice `URM_MOVE` semantics).
///
/// `old` is the formula's address before the move and `new` its address
/// after (equal for formulas outside the moved block). For each cell or
/// single-sheet area reference:
/// - a target entirely inside the moved block follows it;
/// - otherwise, a target entirely inside the destination (cells that the
///   move overwrites) becomes `#REF!`;
/// - otherwise the absolute target is kept (relative parts are re-encoded
///   for `new`, so a moved formula still points at the same outside cells).
///
/// Whole rows/columns and 3-D spans keep their absolute targets. Returns
/// `true` if the AST changed.
pub fn move_refs(f: &mut Formula, old: CellAddr, new: CellAddr, mv: &BlockMove) -> bool {
    let to = mv.to();
    remap(f, old, new, |t| match t {
        Some(t) if mv.from.contains_range(&t) => Action::Shift(mv.dr, mv.dc),
        Some(t) if to.is_some_and(|d| d.contains_range(&t)) => Action::Invalid,
        _ => Action::Keep,
    })
}

/// Re-anchors a formula moving from `old` to `new` so that its references
/// keep pointing at the same absolute cells, except references whose
/// target lies entirely inside `literal` (e.g. the sorted range), which
/// keep their relative offsets and so move with the formula. Returns
/// `true` if the AST changed.
pub fn reanchor(
    f: &mut Formula,
    old: CellAddr,
    new: CellAddr,
    literal: Option<&CellRange>,
) -> bool {
    remap(f, old, new, |t| match (t, literal) {
        (Some(t), Some(l)) if l.contains_range(&t) => Action::Literal,
        _ => Action::Keep,
    })
}

#[cfg(test)]
mod move_tests {
    use super::*;
    use crate::{parse, print, Dialect, SimpleResolver};

    fn a(s: &str) -> CellAddr {
        CellAddr::from_a1(0, s).unwrap()
    }

    fn moved(src: &str, old: &str, new: &str, from: &str, dr: i64, dc: i64) -> String {
        let r = SimpleResolver::with_default_functions();
        let mut f = parse(src, a(old), Dialect::calc_a1_en_us(), &r).unwrap();
        let mv = BlockMove {
            from: CellRange::from_a1(0, from).unwrap(),
            dr,
            dc,
        };
        move_refs(&mut f, a(old), a(new), &mv);
        print(&f, a(new), Dialect::calc_a1_en_us(), &r)
    }

    #[test]
    fn outside_formula_follows_moved_cells() {
        // C1 refers to A1, which moves to E5.
        assert_eq!(moved("=A1+B1", "C1", "C1", "A1:A2", 4, 4), "=E5+B1");
        assert_eq!(
            moved("=SUM(A1:A2)", "C1", "C1", "A1:A2", 4, 4),
            "=SUM(E5:E6)"
        );
        // Partially inside: unchanged.
        assert_eq!(
            moved("=SUM(A1:A3)", "C1", "C1", "A1:A2", 4, 4),
            "=SUM(A1:A3)"
        );
        assert_eq!(moved("=$A$1", "C1", "C1", "A1", 1, 0), "=$A$2");
    }

    #[test]
    fn moved_formula_keeps_outside_targets() {
        // A2 = B1 (outside) moves to D10: still B1; refs inside the block move.
        assert_eq!(moved("=Z1+A1", "A2", "D11", "A1:A2", 9, 3), "=Z1+D10");
    }

    #[test]
    fn references_to_overwritten_cells_become_ref_errors() {
        assert_eq!(moved("=E5", "Z1", "Z1", "A1", 4, 4), "=#REF!");
        assert_eq!(moved("=E5+1", "A1", "E5", "A1", 4, 4), "=#REF!+1");
    }

    #[test]
    fn whole_columns_and_other_sheets_are_kept() {
        assert_eq!(moved("=SUM(A:A)", "C1", "C1", "A1:A2", 4, 4), "=SUM(A:A)");
        assert_eq!(moved("=SUM(B:B)", "A1", "B2", "A1", 1, 1), "=SUM(B:B)");
    }

    #[test]
    fn reanchor_keeps_targets_except_literal() {
        let r = SimpleResolver::with_default_functions();
        let mut f = parse("=A2*$E$1+F9", a("C2"), Dialect::calc_a1_en_us(), &r).unwrap();
        let lit = CellRange::from_a1(0, "A2:C5").unwrap();
        reanchor(&mut f, a("C2"), a("C5"), Some(&lit));
        assert_eq!(
            print(&f, a("C5"), Dialect::calc_a1_en_us(), &r),
            "=A5*$E$1+F9"
        );
        let mut g = parse("=A2", a("C2"), Dialect::calc_a1_en_us(), &r).unwrap();
        reanchor(&mut g, a("C2"), a("C5"), None);
        assert_eq!(print(&g, a("C5"), Dialect::calc_a1_en_us(), &r), "=A2");
    }
}
