//! Inserting and deleting rows and columns (plan §3.5: references are
//! rewritten by `tally_formula::adjust_for_structure_change`; cells, style
//! runs and dimensions shift; formulas in deleted cells go away).

use std::sync::Arc;

use tally_formula::{adjust_for_structure_change, shifted_origin, StructureChange};
use tally_model::CellRangeLocal;
use tally_types::{CellAddr, CellRange, SheetId, MAX_COLS, MAX_ROWS};

use crate::formula_cells::FormulaCellId;
use crate::storage::{shift_set_delete, shift_set_insert, CellContent, Column, ColumnStore};
use crate::workbook::{SnapContent, Workbook};

/// The parts of a row/column change.
#[derive(Copy, Clone, Debug)]
pub struct Shape {
    /// Sheet.
    pub sheet: SheetId,
    /// Rows (`true`) or columns.
    pub rows: bool,
    /// First index.
    pub at: u32,
    /// Number of rows/columns.
    pub count: u32,
    /// Insert (`true`) or delete.
    pub insert: bool,
}

impl Shape {
    /// Decomposes a row/column change (`None` for sheet changes).
    pub fn of(change: &StructureChange) -> Option<Shape> {
        Some(match *change {
            StructureChange::InsertRows { sheet, at, count } => Shape {
                sheet,
                rows: true,
                at,
                count,
                insert: true,
            },
            StructureChange::DeleteRows { sheet, at, count } => Shape {
                sheet,
                rows: true,
                at,
                count,
                insert: false,
            },
            StructureChange::InsertCols { sheet, at, count } => Shape {
                sheet,
                rows: false,
                at,
                count,
                insert: true,
            },
            StructureChange::DeleteCols { sheet, at, count } => Shape {
                sheet,
                rows: false,
                at,
                count,
                insert: false,
            },
            _ => return None,
        })
    }

    /// The inverse change.
    pub fn inverse(&self) -> StructureChange {
        let (sheet, at, count) = (self.sheet, self.at, self.count);
        match (self.rows, self.insert) {
            (true, true) => StructureChange::DeleteRows { sheet, at, count },
            (true, false) => StructureChange::InsertRows { sheet, at, count },
            (false, true) => StructureChange::DeleteCols { sheet, at, count },
            (false, false) => StructureChange::InsertCols { sheet, at, count },
        }
    }

    /// The axis length.
    pub fn limit(&self) -> u32 {
        if self.rows {
            MAX_ROWS
        } else {
            MAX_COLS
        }
    }

    /// The cells that move or disappear.
    pub fn affected(&self) -> CellRange {
        if self.rows {
            CellRange::new(self.sheet, self.at, 0, MAX_ROWS - 1, MAX_COLS - 1)
        } else {
            CellRange::new(self.sheet, 0, self.at, MAX_ROWS - 1, MAX_COLS - 1)
        }
    }

    /// The deleted cells (delete) or the cells pushed off the end (insert).
    pub fn lost(&self) -> CellRange {
        let (a, b) = if self.insert {
            (self.limit() - self.count, self.limit() - 1)
        } else {
            (self.at, self.at + self.count - 1)
        };
        if self.rows {
            CellRange::new(self.sheet, a, 0, b, MAX_COLS - 1)
        } else {
            CellRange::new(self.sheet, 0, a, MAX_ROWS - 1, b)
        }
    }

    fn deletes(&self, addr: CellAddr) -> bool {
        if self.insert || addr.sheet != self.sheet {
            return false;
        }
        let i = if self.rows { addr.row } else { addr.col };
        i >= self.at && i < self.at.saturating_add(self.count)
    }
}

fn formula_ids(contents: impl IntoIterator<Item = (u32, CellContent)>) -> Vec<FormulaCellId> {
    contents
        .into_iter()
        .filter_map(|(_, c)| match c {
            CellContent::Formula(id) => Some(id),
            CellContent::Value(_) | CellContent::ArrayPart(_) => None,
        })
        .collect()
}

impl Workbook {
    /// Applies a row/column insertion or deletion. Returns the previous
    /// content of every formula whose references were rewritten (at its
    /// address before the change), for undo.
    pub fn structure_change(
        &mut self,
        change: &StructureChange,
    ) -> Vec<(CellAddr, Option<SnapContent>)> {
        let Some(sh) = Shape::of(change) else {
            return Vec::new();
        };
        if self.sheet(sh.sheet).is_none() || sh.count == 0 || sh.at >= sh.limit() {
            return Vec::new();
        }
        // 1. Rewrite references.
        let mut old = Vec::new();
        let mut rewritten: Vec<(FormulaCellId, Arc<tally_formula::Formula>)> = Vec::new();
        for (id, fc) in &self.formulas {
            if sh.deletes(fc.addr) || fc.invalid_text.is_some() {
                continue;
            }
            let mut f = (*fc.formula).clone();
            if adjust_for_structure_change(&mut f, fc.addr, change) {
                old.push((
                    fc.addr,
                    Some(SnapContent::Formula {
                        formula: fc.formula.clone(),
                        invalid_text: None,
                        array: fc.array,
                    }),
                ));
                rewritten.push((id, Arc::new(f)));
            }
        }
        // 2. Shift storage.
        let mut gone: Vec<FormulaCellId> = Vec::new();
        if let Some(sheet) = self.sheet_mut(sh.sheet) {
            if sh.rows {
                for col in &mut sheet.columns {
                    let lost = if sh.insert {
                        col.cells.insert_rows(sh.at, sh.count)
                    } else {
                        col.cells.delete_rows(sh.at, sh.count)
                    };
                    gone.extend(formula_ids(lost));
                    if sh.insert {
                        shift_set_insert(&mut col.formula_rows, sh.at, sh.count, MAX_ROWS);
                        col.styles.insert_rows(sh.at, sh.count);
                    } else {
                        shift_set_delete(&mut col.formula_rows, sh.at, sh.count);
                        col.styles.delete_rows(sh.at, sh.count);
                    }
                }
                if sh.insert {
                    sheet.row_dims.insert(sh.at, sh.count, MAX_ROWS);
                } else {
                    sheet.row_dims.delete(sh.at, sh.count);
                }
            } else {
                let at = sh.at as usize;
                if sh.insert {
                    if at < sheet.columns.len() {
                        let fill = (0..sh.count).map(|_| Column::default());
                        sheet.columns.splice(at..at, fill);
                        if sheet.columns.len() > MAX_COLS as usize {
                            for col in sheet.columns.drain(MAX_COLS as usize..) {
                                gone.extend(formula_ids(
                                    col.cells
                                        .iter_range(0, MAX_ROWS - 1)
                                        .map(|(r, c)| (r, c.clone())),
                                ));
                            }
                        }
                    }
                    sheet.col_dims.insert(sh.at, sh.count, MAX_COLS);
                } else {
                    let end = (at + sh.count as usize).min(sheet.columns.len());
                    if at < end {
                        for col in sheet.columns.drain(at..end) {
                            gone.extend(formula_ids(
                                col.cells
                                    .iter_range(0, MAX_ROWS - 1)
                                    .map(|(r, c)| (r, c.clone())),
                            ));
                        }
                    }
                    sheet.col_dims.delete(sh.at, sh.count);
                }
            }
        }
        for id in gone {
            self.deps.forget(id);
            self.formulas.remove(id);
        }
        // 3. Move formula cells and install rewritten ASTs.
        let mut touched: Vec<FormulaCellId> = Vec::new();
        for (id, fc) in self.formulas.iter_mut() {
            if fc.addr.sheet == sh.sheet {
                let n = shifted_origin(fc.addr, change);
                if n != fc.addr {
                    fc.addr = n;
                    touched.push(id);
                }
            }
        }
        for (id, f) in rewritten {
            if let Some(fc) = self.formulas.get_mut(id) {
                fc.formula = f;
                touched.push(id);
            }
        }
        touched.sort();
        touched.dedup();
        // 4. Re-register precedents and dirty.
        for id in touched {
            if let Some(fc) = self.formulas.get_mut(id) {
                fc.dynamic_deps.clear();
            }
            self.refresh_formula(id);
        }
        self.pending.ranges.push(sh.affected());
        old
    }

    /// Rewrites the rule formulas of a sheet's conditional formats and
    /// validations for a structural change on another sheet, copying only
    /// the formats and validations whose formulas change.
    pub(crate) fn adjust_other_sheet_rules(
        meta: &mut crate::storage::SheetMeta,
        change: &StructureChange,
    ) {
        fn adjusted(
            f: &crate::fexpr::FExpr,
            origin: CellAddr,
            change: &StructureChange,
        ) -> Option<crate::fexpr::FExpr> {
            let mut n = f.clone();
            n.adjust(origin, change);
            (n.formula != f.formula).then_some(n)
        }
        for i in 0..meta.cond_formats.len() {
            let cf = &meta.cond_formats[i];
            let origin = cf.origin;
            let mut probe = (**cf).clone();
            let changed = probe
                .formulas_mut()
                .into_iter()
                .any(|f| adjusted(f, origin, change).is_some());
            if changed {
                let cf = Arc::make_mut(&mut Arc::make_mut(&mut meta.cond_formats)[i]);
                for f in cf.formulas_mut() {
                    f.adjust(origin, change);
                }
            }
        }
        for i in 0..meta.validations.len() {
            let v = &meta.validations[i];
            let origin = v.origin;
            let mut probe = (**v).clone();
            let changed = probe
                .formulas_mut()
                .into_iter()
                .any(|f| adjusted(f, origin, change).is_some());
            if changed {
                let v = Arc::make_mut(&mut Arc::make_mut(&mut meta.validations)[i]);
                for f in v.formulas_mut() {
                    f.adjust(origin, change);
                }
            }
        }
    }

    /// Adjusts what is not cells for a row/column change: the sheet's
    /// merges and AutoFilter (they grow, shrink or go away like area
    /// references) and every name's references. Callers record
    /// [`crate::undo::sheet_meta`] and [`crate::undo::names_snapshot`]
    /// first; [`Self::structure_change`] (and so undo's
    /// `Change::Structure`) leaves them alone.
    pub fn adjust_meta_for_structure(&mut self, change: &StructureChange) {
        if let Some(sh) = Shape::of(change) {
            if let Some(sheet) = self.sheet_mut(sh.sheet) {
                let span = |lo: u32, hi: u32| span_after(&sh, lo, hi);
                let mut merges = Vec::with_capacity(sheet.meta.merges.len());
                for m in sheet.meta.merges.iter() {
                    let n = if sh.rows {
                        span(m.r0, m.r1).map(|(r0, r1)| CellRangeLocal { r0, r1, ..*m })
                    } else {
                        span(m.c0, m.c1).map(|(c0, c1)| CellRangeLocal { c0, c1, ..*m })
                    };
                    if let Some(n) = n.filter(|n| (n.r0, n.c0) != (n.r1, n.c1)) {
                        merges.push(n);
                    }
                }
                if *sheet.meta.merges != merges {
                    sheet.meta.merges = Arc::new(merges);
                }
                if let Some(af) = sheet.meta.auto_filter.take() {
                    let af = Arc::unwrap_or_clone(af);
                    let r = af.range;
                    let n = if sh.rows {
                        span(r.r0, r.r1).map(|(r0, r1)| CellRangeLocal { r0, r1, ..r })
                    } else {
                        span(r.c0, r.c1).map(|(c0, c1)| CellRangeLocal { c0, c1, ..r })
                    };
                    sheet.meta.auto_filter = n.map(|range| {
                        let columns = if sh.rows {
                            af.columns
                        } else {
                            af.columns
                                .into_iter()
                                .filter_map(|(c, f)| point_after(&sh, c).map(|nc| (nc, f)))
                                .collect()
                        };
                        Arc::new(crate::storage::AutoFilter { range, columns })
                    });
                }
                let adjust_ranges = |ranges: &[CellRangeLocal]| -> Vec<CellRangeLocal> {
                    ranges
                        .iter()
                        .filter_map(|r| {
                            if sh.rows {
                                span(r.r0, r.r1).map(|(r0, r1)| CellRangeLocal { r0, r1, ..*r })
                            } else {
                                span(r.c0, r.c1).map(|(c0, c1)| CellRangeLocal { c0, c1, ..*r })
                            }
                        })
                        .collect()
                };
                if !sheet.meta.cond_formats.is_empty() {
                    let list = Arc::make_mut(&mut sheet.meta.cond_formats);
                    for cf in list.iter_mut() {
                        let cf = Arc::make_mut(cf);
                        cf.ranges = adjust_ranges(&cf.ranges);
                        let origin = cf.origin;
                        for f in cf.formulas_mut() {
                            f.adjust(origin, change);
                        }
                        cf.origin = shifted_origin(origin, change);
                    }
                    list.retain(|cf| !cf.ranges.is_empty());
                }
                if !sheet.meta.validations.is_empty() {
                    let list = Arc::make_mut(&mut sheet.meta.validations);
                    for v in list.iter_mut() {
                        let v = Arc::make_mut(v);
                        v.ranges = adjust_ranges(&v.ranges);
                        let origin = v.origin;
                        for f in v.formulas_mut() {
                            f.adjust(origin, change);
                        }
                        v.origin = shifted_origin(origin, change);
                    }
                    list.retain(|v| !v.ranges.is_empty());
                }
                if !sheet.meta.notes.is_empty() {
                    crate::annotations::shift_for_structure(
                        Arc::make_mut(&mut sheet.meta.notes),
                        &sh,
                    );
                }
                if !sheet.meta.links.is_empty() {
                    crate::annotations::shift_for_structure(
                        Arc::make_mut(&mut sheet.meta.links),
                        &sh,
                    );
                }
                crate::pagesetup::adjust_for_structure(&mut sheet.meta, &sh);
            }
            // Wave 5 objects (anchors and ranges on this and other sheets).
            crate::charts::adjust_structure(self, &sh);
            crate::outline::adjust_structure(self, &sh);
            crate::pivot::adjust_structure(self, &sh);
            crate::scenarios::adjust_structure(self, &sh);
            // Rule formulas on other sheets that point at the changed sheet.
            for other in &mut self.sheets {
                if other.id == sh.sheet {
                    continue;
                }
                // Only formulas that actually change are rewritten, so
                // untouched sheets keep sharing their collections.
                Self::adjust_other_sheet_rules(&mut other.meta, change);
            }
            let sheet = sh.sheet;
            self.reapply_filter(sheet);
        }
        let mut renamed = false;
        for def in self.names.defs_mut() {
            let mut f = (*def.formula).clone();
            if adjust_for_structure_change(&mut f, def.base, change) {
                def.formula = Arc::new(f);
                renamed = true;
            }
            def.base = shifted_origin(def.base, change);
        }
        if renamed {
            self.refresh_all_name_dependents();
        }
    }
}

/// New position of index `p` after a row/column change, `None` if deleted.
pub fn point_after(sh: &Shape, p: u32) -> Option<u32> {
    if sh.insert {
        if p >= sh.at {
            let n = p.checked_add(sh.count)?;
            (n < sh.limit()).then_some(n)
        } else {
            Some(p)
        }
    } else {
        let end = sh.at.saturating_add(sh.count);
        if p < sh.at {
            Some(p)
        } else if p < end {
            None
        } else {
            Some(p - sh.count)
        }
    }
}

/// New span of `lo..=hi` after a row/column change (area semantics:
/// inserting inside grows it, deleting part shrinks it, deleting all of it
/// gives `None`).
pub fn span_after(sh: &Shape, lo: u32, hi: u32) -> Option<(u32, u32)> {
    let max = sh.limit() - 1;
    if sh.insert {
        if sh.at <= lo {
            let nlo = lo.checked_add(sh.count)?;
            if nlo > max {
                return None;
            }
            Some((nlo, hi.saturating_add(sh.count).min(max)))
        } else if sh.at <= hi {
            Some((lo, hi.saturating_add(sh.count).min(max)))
        } else {
            Some((lo, hi))
        }
    } else {
        let d1 = sh.at.saturating_add(sh.count - 1);
        if d1 < lo {
            Some((lo - sh.count, hi - sh.count))
        } else if sh.at > hi {
            Some((lo, hi))
        } else if sh.at <= lo && d1 >= hi {
            None
        } else {
            let nlo = lo.min(sh.at);
            let nhi = if hi > d1 { hi - sh.count } else { sh.at - 1 };
            Some((nlo, nhi))
        }
    }
}

/// `true` if the change would split the array block `b` (insert strictly
/// inside it, or delete part of it).
pub fn splits_block(sh: &Shape, b: &CellRange) -> bool {
    if b.sheet != sh.sheet {
        return false;
    }
    let (lo, hi) = if sh.rows { (b.r0, b.r1) } else { (b.c0, b.c1) };
    if sh.insert {
        sh.at > lo && sh.at <= hi
    } else {
        let d1 = sh.at.saturating_add(sh.count - 1);
        let disjoint = d1 < lo || sh.at > hi;
        let covers = sh.at <= lo && d1 >= hi;
        !disjoint && !covers
    }
}
