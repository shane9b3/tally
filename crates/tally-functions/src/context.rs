//! The traits the engine implements ([`EvalContext`], [`ArgList`]) and the
//! evaluation-time value type [`EvalValue`].
//!
//! Functions never see the engine directly: everything they need from the
//! workbook goes through these two traits (plan §3.7).

use std::ops::ControlFlow;
use std::sync::Arc;

use tally_types::{CalcSettings, CellAddr, CellRange, Matrix, SheetId, Value};

/// An argument as the evaluator hands it to a function.
///
/// References stay unexpanded (`Range`) so functions can iterate them
/// sparsely through [`EvalContext::for_each_in_range`].
#[derive(Clone, PartialEq, Debug)]
pub enum EvalValue {
    /// The argument was omitted (`=ROUND(1;)` or past the end of the list).
    Missing,
    /// A single value. Never holds [`Value::Array`]; use [`EvalValue::from_value`].
    Scalar(Value),
    /// A reference (a single cell is a 1×1 range).
    Range(CellRange),
    /// An inline or computed array.
    Array(Arc<Matrix>),
}

impl EvalValue {
    /// Wraps a value, moving [`Value::Array`] into [`EvalValue::Array`].
    pub fn from_value(v: Value) -> Self {
        match v {
            Value::Array(m) => EvalValue::Array(m),
            other => EvalValue::Scalar(other),
        }
    }

    /// Shorthand for `Scalar(Value::Number(n))`.
    pub fn number(n: f64) -> Self {
        EvalValue::Scalar(Value::Number(n))
    }

    /// Shorthand for `Scalar(Value::Text(s))`.
    pub fn text(s: &str) -> Self {
        EvalValue::Scalar(Value::text(s))
    }

    /// Shorthand for `Scalar(Value::Bool(b))`.
    pub fn bool(b: bool) -> Self {
        EvalValue::Scalar(Value::Bool(b))
    }

    /// Shorthand for `Scalar(Value::Error(e))`.
    pub fn error(e: tally_types::ErrorKind) -> Self {
        EvalValue::Scalar(Value::Error(e))
    }

    /// `true` for [`EvalValue::Missing`].
    pub const fn is_missing(&self) -> bool {
        matches!(self, EvalValue::Missing)
    }

    /// The error if this is a scalar error value.
    pub const fn as_error(&self) -> Option<tally_types::ErrorKind> {
        match self {
            EvalValue::Scalar(Value::Error(e)) => Some(*e),
            _ => None,
        }
    }
}

impl From<Value> for EvalValue {
    fn from(v: Value) -> Self {
        EvalValue::from_value(v)
    }
}

impl From<CellRange> for EvalValue {
    fn from(r: CellRange) -> Self {
        EvalValue::Range(r)
    }
}

/// Formatting and formula facts about a cell (for CELL, ISFORMULA, FORMULA).
///
/// Non-exhaustive so fields can be added; build it from
/// [`CellFormatInfo::default`] and assign fields.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
#[non_exhaustive]
pub struct CellFormatInfo {
    /// The cell holds a formula.
    pub has_formula: bool,
    /// The formula text in the UI dialect, if any (FORMULA()).
    pub formula: Option<Arc<str>>,
    /// The number format code, if not General.
    pub format_code: Option<Arc<str>>,
    /// The cell is protected.
    pub protected: bool,
}

/// What a function may ask of the engine. Implemented by `tally-engine`
/// (and by `mock::MockEvalContext` for tests).
pub trait EvalContext {
    /// The cell containing the formula being evaluated (for implicit
    /// intersection, ROW(), COLUMN()).
    fn origin(&self) -> CellAddr;

    /// The document's calculation settings.
    fn settings(&self) -> &CalcSettings;

    /// The value of one cell, evaluating it first if it is a dirty formula.
    /// Out-of-grid or deleted cells yield `Value::Error(ErrorKind::Ref)`.
    fn cell_value(&mut self, addr: CellAddr) -> Value;

    /// Visits the cells of `range` **column by column** (top to bottom
    /// within a column, columns left to right), matching AOO's value
    /// iterator so the first error found is the same. With `skip_empty`,
    /// empty cells are not visited (the engine should iterate sparsely).
    /// Stops when `f` returns `ControlFlow::Break`.
    fn for_each_in_range(
        &mut self,
        range: &CellRange,
        skip_empty: bool,
        f: &mut dyn FnMut(CellAddr, &Value) -> ControlFlow<()>,
    );

    /// Clamps `range` to the sheet's used area (so `A:A` costs what is
    /// filled); `None` if the intersection is empty.
    fn used_bounds(&self, range: &CellRange) -> Option<CellRange>;

    /// Parses reference text for INDIRECT/ADDRESS (`a1`: A1 vs R1C1 syntax),
    /// relative to [`EvalContext::origin`].
    fn parse_reference_text(&self, text: &str, a1: bool) -> Option<CellRange>;

    /// Records a reference discovered during evaluation (INDIRECT, OFFSET)
    /// as a dynamic dependency of the current formula.
    fn record_dynamic_dep(&mut self, range: &CellRange);

    /// The current date-time as a serial number in the document's epoch,
    /// fixed for the duration of one recalc.
    fn now_serial(&self) -> f64;

    /// A uniform random number in `[0, 1)`.
    fn rand(&mut self) -> f64;

    /// 0-based tab position of a sheet, `None` if it does not exist.
    fn sheet_index(&self, id: SheetId) -> Option<u32>;

    /// Number of sheets.
    fn sheet_count(&self) -> u32;

    /// Formatting and formula facts about a cell.
    fn cell_format_info(&self, addr: CellAddr) -> CellFormatInfo;

    /// `true` while evaluating an array formula or an argument of a
    /// [`crate::FnFlags::FORCE_ARRAY`] function / [`crate::ParamKind::Array`]
    /// parameter. Scalar parameters are then lifted elementwise.
    fn in_array_context(&self) -> bool {
        false
    }

    /// `true` if `row` (0-based) of `sheet` is hidden, manually or by a
    /// filter. SUBTOTAL codes 101 to 111 skip such rows. The default (no
    /// hidden rows) keeps implementations that do not track visibility
    /// correct for every other function.
    fn hidden_row(&self, sheet: SheetId, row: u32) -> bool {
        let _ = (sheet, row);
        false
    }

    /// `true` if `row` (0-based) of `sheet` is hidden by an AutoFilter (a
    /// subset of [`EvalContext::hidden_row`]). SUBTOTAL codes 1 to 11 skip
    /// such rows, as in AOO. The default reports no filtered rows.
    fn filtered_row(&self, sheet: SheetId, row: u32) -> bool {
        let _ = (sheet, row);
        false
    }

    /// MULTIPLE.OPERATIONS (wave 5): the value of the formula cell
    /// `formula` when each cell in `subs` temporarily holds the paired
    /// value. The engine re-evaluates the precedent chain from the
    /// substituted cells to `formula` in a sandbox, without changing the
    /// document (capped at 10,000 calls per recalc). The default, `None`,
    /// means the host has no table-operation sandbox; the function then
    /// returns `Err:504`.
    fn table_op(&mut self, formula: CellAddr, subs: &[(CellAddr, Value)]) -> Option<Value> {
        let _ = (formula, subs);
        None
    }

    /// GETPIVOTDATA (wave 5): the value of data field `field` of the pivot
    /// table whose output contains `at`, restricted by `filters`
    /// (`(field name, item name)` pairs). `None` when no pivot output
    /// contains `at` or the host has no pivot tables; the function then
    /// returns `#REF!`.
    fn pivot_data(
        &mut self,
        at: CellAddr,
        field: &str,
        filters: &[(String, String)],
    ) -> Option<Value> {
        let _ = (at, field, filters);
        None
    }
}

/// The arguments of one call, evaluated on demand by the engine.
pub trait ArgList {
    /// Number of arguments written in the call (including empty ones in `f(1;;3)`).
    fn len(&self) -> usize;

    /// `true` if there are no arguments.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// `true` if argument `i` is syntactically empty or `i >= len()`.
    fn is_missing(&self, i: usize) -> bool;

    /// Evaluates argument `i`. Returns [`EvalValue::Missing`] for missing
    /// arguments. The engine evaluates argument `i` in array context when
    /// [`crate::FunctionDef::param_forces_array`] says so.
    fn eval(&mut self, i: usize, ctx: &mut dyn EvalContext) -> EvalValue;
}
