//! Formula cells: a slot map of [`FormulaCell`]s referenced from the sheet
//! storage by [`FormulaCellId`].

use std::sync::Arc;

use slotmap::{new_key_type, SlotMap};
use tally_formula::Formula;
use tally_types::{CellAddr, CellRange, ErrorKind, Matrix, Value};

new_key_type! {
    /// Stable handle of a formula cell.
    pub struct FormulaCellId;
}

/// Evaluation state of a formula cell.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CellState {
    /// `cached` is current.
    Clean,
    /// Needs evaluation.
    Dirty,
    /// Being evaluated (meeting it again means a cycle).
    Evaluating,
}

/// A formula and its cached result.
#[derive(Clone, Debug)]
pub struct FormulaCell {
    /// Where the formula lives.
    pub addr: CellAddr,
    /// The parsed formula (relative references, shared between copies).
    pub formula: Arc<Formula>,
    /// Last result (always a scalar).
    pub cached: Value,
    /// Evaluation state.
    pub state: CellState,
    /// Contains a volatile function; dirtied on every recalc.
    pub volatile: bool,
    /// For a formula that failed to parse: the text the user typed. The
    /// formula is then `Expr::Err(code)` and the cell shows that error.
    pub invalid_text: Option<Arc<str>>,
    /// Static precedents resolved at `addr` (3-D spans expanded by tab order).
    pub precedents: Vec<CellRange>,
    /// The precedents whose values are always read, for cycle detection:
    /// `precedents` without the references inside lazily evaluated
    /// arguments (IF/IFERROR/IFNA branches) and the address-only arguments
    /// of ROW and COLUMN. `None` when that is all of `precedents`.
    pub strict_precedents: Option<Vec<CellRange>>,
    /// References recorded during the last evaluation (INDIRECT, OFFSET).
    pub dynamic_deps: Vec<CellRange>,
    /// Has a 3-D reference, whose precedents depend on the tab order.
    pub has_3d: bool,
    /// Uses named ranges (evaluated with names expanded).
    pub has_names: bool,
    /// For an array formula anchor: the block size `(rows, cols)`.
    pub array: Option<(u32, u32)>,
    /// For an array formula anchor: the last result matrix (`None` when
    /// the result was a scalar, which fills the whole block).
    pub array_result: Option<Arc<Matrix>>,
}

impl FormulaCell {
    /// The array block of an anchor.
    pub fn array_range(&self) -> Option<CellRange> {
        let (rows, cols) = self.array?;
        Some(CellRange::new(
            self.addr.sheet,
            self.addr.row,
            self.addr.col,
            self.addr.row + rows.saturating_sub(1),
            self.addr.col + cols.saturating_sub(1),
        ))
    }

    /// The cells this formula writes: its block, or its own cell.
    pub fn extent(&self) -> CellRange {
        self.array_range()
            .unwrap_or_else(|| CellRange::single(self.addr))
    }

    /// The value shown at offset (`dr`, `dc`) of an array block: the result
    /// matrix element, a single row/column repeated, `#N/A` outside a
    /// larger matrix; a scalar result fills the block.
    pub fn array_value(&self, dr: u32, dc: u32) -> Value {
        match &self.array_result {
            None => self.cached.clone(),
            Some(m) => {
                let r = if m.rows == 1 { 0 } else { dr };
                let c = if m.cols == 1 { 0 } else { dc };
                match m.get(r, c) {
                    Some(Value::Empty) => Value::Number(0.0),
                    Some(Value::Number(n)) if !n.is_finite() => Value::Error(ErrorKind::Num),
                    Some(v) => v.clone(),
                    None => Value::Error(ErrorKind::NA),
                }
            }
        }
    }
}

/// The slot map of formula cells.
pub type FormulaCells = SlotMap<FormulaCellId, FormulaCell>;
