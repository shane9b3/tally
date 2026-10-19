//! Formula cells: a slot map of [`FormulaCell`]s referenced from the sheet
//! storage by [`FormulaCellId`].

use std::sync::Arc;

use slotmap::{new_key_type, SlotMap};
use tally_formula::Formula;
use tally_types::{CellAddr, CellRange, Value};

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
    /// References recorded during the last evaluation (INDIRECT, OFFSET).
    pub dynamic_deps: Vec<CellRange>,
    /// Has a 3-D reference, whose precedents depend on the tab order.
    pub has_3d: bool,
}

/// The slot map of formula cells.
pub type FormulaCells = SlotMap<FormulaCellId, FormulaCell>;
