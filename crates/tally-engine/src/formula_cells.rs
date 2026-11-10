//! Formula cells: a slot map of [`FormulaCell`]s referenced from the sheet
//! storage by [`FormulaCellId`].

use std::sync::Arc;

use slotmap::{new_key_type, SlotMap};
use tally_formula::Formula;
use tally_types::{CellAddr, CellRange, ErrorKind, Matrix, SheetId, Value};

/// A 3-D precedent kept unexpanded: `rect` on every sheet from `first` to
/// `last` in the current tab order (either order). Which sheets it covers
/// is decided by tab position when it is matched, so the dependency index
/// stays one entry per reference however many sheets it spans.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Span3d {
    /// Sheet of the start endpoint.
    pub first: SheetId,
    /// Sheet of the end endpoint.
    pub last: SheetId,
    /// The rectangle (its `sheet` is `first`).
    pub rect: CellRange,
}

/// The 3-D precedents of a formula cell.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SpanSet {
    /// Static 3-D precedents.
    pub spans: Vec<Span3d>,
    /// The static ones whose values are always read (see
    /// `FormulaCell::strict_precedents`).
    pub strict: Vec<Span3d>,
    /// 3-D references built during the last evaluation (the `:` operator
    /// across sheets, INDIRECT).
    pub dynamic: Vec<Span3d>,
}

impl SpanSet {
    /// `None` when all three lists are empty.
    pub fn boxed(
        spans: Vec<Span3d>,
        strict: Vec<Span3d>,
        dynamic: Vec<Span3d>,
    ) -> Option<Box<Self>> {
        if spans.is_empty() && strict.is_empty() && dynamic.is_empty() {
            None
        } else {
            Some(Box::new(Self {
                spans,
                strict,
                dynamic,
            }))
        }
    }
}

impl Span3d {
    /// The rectangle on `sheet`.
    pub fn on(&self, sheet: SheetId) -> CellRange {
        CellRange { sheet, ..self.rect }
    }

    /// Whether the rectangles overlap, ignoring sheets.
    pub fn rect_hits(&self, r: &CellRange) -> bool {
        self.rect.r0 <= r.r1 && r.r0 <= self.rect.r1 && self.rect.c0 <= r.c1 && r.c0 <= self.rect.c1
    }
}

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
    /// Static single-sheet precedents resolved at `addr` (3-D references
    /// are in `spans`).
    pub precedents: Vec<CellRange>,
    /// 3-D precedents, unexpanded (`None` for the common formula without
    /// any, which keeps `FormulaCell` small).
    pub three_d: Option<Box<SpanSet>>,
    /// The precedents whose values are always read, for cycle detection:
    /// `precedents` without the references inside lazily evaluated
    /// arguments (IF/IFERROR/IFNA branches) and the address-only arguments
    /// of ROW and COLUMN. `None` when that is all of `precedents`.
    pub strict_precedents: Option<Box<Vec<CellRange>>>,
    /// References recorded during the last evaluation (INDIRECT, OFFSET).
    pub dynamic_deps: Vec<CellRange>,
    /// Has a 3-D reference, whose precedents depend on the tab order.
    pub has_3d: bool,
    /// Calls SHEET or SHEETS, whose results depend on the tab order:
    /// recalculated whenever sheets are added, removed or moved.
    pub tab_order_dependent: bool,
    /// Uses named ranges (evaluated with names expanded).
    pub has_names: bool,
    /// Calls a function whose result depends on row visibility (SUBTOTAL,
    /// AGGREGATE): dirtied when rows are hidden, shown or filtered.
    pub visibility_dependent: bool,
    /// For an array formula anchor: the block size `(rows, cols)`.
    pub array: Option<(u32, u32)>,
    /// For an array formula anchor: the last result matrix (`None` when
    /// the result was a scalar, which fills the whole block).
    pub array_result: Option<Arc<Matrix>>,
}

impl FormulaCell {
    /// Static 3-D precedents.
    pub fn spans(&self) -> &[Span3d] {
        self.three_d.as_ref().map_or(&[], |t| &t.spans)
    }

    /// Always-read static 3-D precedents.
    pub fn strict_spans(&self) -> &[Span3d] {
        self.three_d.as_ref().map_or(&[], |t| &t.strict)
    }

    /// 3-D references recorded during the last evaluation.
    pub fn dynamic_spans(&self) -> &[Span3d] {
        self.three_d.as_ref().map_or(&[], |t| &t.dynamic)
    }

    /// Static and dynamic 3-D precedents (what the dependency index holds).
    pub fn all_spans(&self) -> Vec<Span3d> {
        let mut v = self.spans().to_vec();
        v.extend_from_slice(self.dynamic_spans());
        v
    }

    /// Replaces the static 3-D precedents, keeping the dynamic ones.
    pub fn set_static_spans(&mut self, spans: Vec<Span3d>, strict: Vec<Span3d>) {
        let dynamic = self.three_d.take().map(|t| t.dynamic).unwrap_or_default();
        self.three_d = SpanSet::boxed(spans, strict, dynamic);
    }

    /// Replaces the dynamic 3-D precedents.
    pub fn set_dynamic_spans(&mut self, dynamic: Vec<Span3d>) {
        let (spans, strict) = self
            .three_d
            .take()
            .map(|t| (t.spans, t.strict))
            .unwrap_or_default();
        self.three_d = SpanSet::boxed(spans, strict, dynamic);
    }

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
        array_value_of(&self.cached, self.array_result.as_ref(), dr, dc)
    }
}

/// [`FormulaCell::array_value`] for a result `cached` / `matrix` that is
/// not stored in a cell yet (the parallel recalc pre-pass).
pub fn array_value_of(cached: &Value, matrix: Option<&Arc<Matrix>>, dr: u32, dc: u32) -> Value {
    match matrix {
        None => cached.clone(),
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

/// The slot map of formula cells.
pub type FormulaCells = SlotMap<FormulaCellId, FormulaCell>;
