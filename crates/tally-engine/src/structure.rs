//! Inserting and deleting rows and columns (plan §3.5: references are
//! rewritten by `tally_formula::adjust_for_structure_change`; cells, style
//! runs and dimensions shift; formulas in deleted cells go away).

use std::sync::Arc;

use tally_formula::{adjust_for_structure_change, shifted_origin, StructureChange};
use tally_types::{CellAddr, CellRange, SheetId, MAX_COLS, MAX_ROWS};

use crate::deps::DepIndex;
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
            CellContent::Value(_) => None,
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
            self.deps.remove(id);
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
            let Some(fc) = self.formulas.get(id) else {
                continue;
            };
            let p = self.precedents_of(&fc.formula, fc.addr);
            let vol = fc.volatile;
            self.deps.remove(id);
            self.deps.add(id, &p);
            self.deps.set_volatile(id, vol);
            if let Some(fc) = self.formulas.get_mut(id) {
                fc.precedents = p;
                fc.dynamic_deps.clear();
            }
            self.pending.dirty.push(id);
        }
        self.pending.ranges.push(sh.affected());
        old
    }
}
