//! Test doubles: [`MockEvalContext`] (a `HashMap`-backed sheet set) and
//! [`MockArgs`] (an argument list that records which arguments were
//! evaluated). Available under `cfg(test)` and the `test-support` feature.

use std::collections::{HashMap, HashSet};
use std::ops::ControlFlow;

use tally_types::{CalcSettings, CellAddr, CellRange, SheetId, Value};

use crate::context::{ArgList, CellFormatInfo, EvalContext, EvalValue};

/// An in-memory [`EvalContext`].
///
/// Sheets are `0..sheet_count` (ids equal tab positions). The origin
/// defaults to `A1` on sheet 0; `now_serial` is fixed and `rand` is a
/// deterministic LCG.
#[derive(Clone, Debug)]
pub struct MockEvalContext {
    cells: HashMap<CellAddr, Value>,
    formats: HashMap<CellAddr, CellFormatInfo>,
    /// Calculation settings (mutable for tests).
    pub settings: CalcSettings,
    /// The formula cell.
    pub origin: CellAddr,
    /// What [`EvalContext::in_array_context`] returns.
    pub array_context: bool,
    /// What [`EvalContext::now_serial`] returns (default 2024-01-15 12:00).
    pub now: f64,
    /// Number of sheets.
    pub sheets: u32,
    /// Ranges passed to [`EvalContext::record_dynamic_dep`].
    pub dynamic_deps: Vec<CellRange>,
    /// Cells read through [`EvalContext::cell_value`] (in order).
    pub reads: Vec<CellAddr>,
    /// Rows reported by [`EvalContext::hidden_row`], as `(sheet, row)`.
    pub hidden_rows: HashSet<(SheetId, u32)>,
    rng: u64,
}

impl Default for MockEvalContext {
    fn default() -> Self {
        Self::new()
    }
}

fn addr(sheet: SheetId, a1: &str) -> CellAddr {
    CellAddr::from_a1(sheet, a1).unwrap_or_else(|| panic!("bad A1 address {a1:?}"))
}

impl MockEvalContext {
    /// An empty one-sheet workbook with default settings.
    pub fn new() -> Self {
        Self {
            cells: HashMap::new(),
            formats: HashMap::new(),
            settings: CalcSettings::default(),
            origin: CellAddr::new(0, 0, 0),
            array_context: false,
            now: 45_306.5,
            sheets: 1,
            dynamic_deps: Vec::new(),
            reads: Vec::new(),
            hidden_rows: HashSet::new(),
            rng: 0x2545_F491_4F6C_DD1D,
        }
    }

    /// Sets a cell (`"B3"` on `sheet`); `Value::Empty` clears it.
    /// Panics on a malformed address (test helper).
    pub fn set(&mut self, sheet: SheetId, a1: &str, value: impl Into<Value>) -> &mut Self {
        let a = addr(sheet, a1);
        let value = value.into();
        if value.is_empty() {
            self.cells.remove(&a);
        } else {
            self.cells.insert(a, value);
        }
        if sheet >= self.sheets {
            self.sheets = sheet + 1;
        }
        self
    }

    /// Reads a cell back (no recording).
    pub fn get(&self, sheet: SheetId, a1: &str) -> Value {
        self.cells
            .get(&addr(sheet, a1))
            .cloned()
            .unwrap_or_default()
    }

    /// Sets the formula cell.
    pub fn set_origin(&mut self, sheet: SheetId, a1: &str) -> &mut Self {
        self.origin = addr(sheet, a1);
        self
    }

    /// Sets the format info returned for a cell.
    pub fn set_format(&mut self, sheet: SheetId, a1: &str, info: CellFormatInfo) -> &mut Self {
        self.formats.insert(addr(sheet, a1), info);
        self
    }

    /// A reference argument, e.g. `ctx.range(0, "A1:B3")`.
    /// Panics on malformed text (test helper).
    pub fn range(&self, sheet: SheetId, a1: &str) -> EvalValue {
        EvalValue::Range(
            CellRange::from_a1(sheet, a1).unwrap_or_else(|| panic!("bad A1 range {a1:?}")),
        )
    }

    fn used(&self, sheet: SheetId) -> Option<CellRange> {
        let mut it = self.cells.keys().filter(|a| a.sheet == sheet);
        let first = *it.next()?;
        let (mut r0, mut c0, mut r1, mut c1) = (first.row, first.col, first.row, first.col);
        for a in it {
            r0 = r0.min(a.row);
            c0 = c0.min(a.col);
            r1 = r1.max(a.row);
            c1 = c1.max(a.col);
        }
        Some(CellRange::new(sheet, r0, c0, r1, c1))
    }
}

impl EvalContext for MockEvalContext {
    fn origin(&self) -> CellAddr {
        self.origin
    }

    fn settings(&self) -> &CalcSettings {
        &self.settings
    }

    fn cell_value(&mut self, addr: CellAddr) -> Value {
        self.reads.push(addr);
        if !addr.is_valid() || addr.sheet >= self.sheets {
            return Value::Error(tally_types::ErrorKind::Ref);
        }
        self.cells.get(&addr).cloned().unwrap_or_default()
    }

    fn for_each_in_range(
        &mut self,
        range: &CellRange,
        skip_empty: bool,
        f: &mut dyn FnMut(CellAddr, &Value) -> ControlFlow<()>,
    ) {
        if skip_empty {
            let mut hits: Vec<(CellAddr, Value)> = self
                .cells
                .iter()
                .filter(|(a, _)| range.contains(**a))
                .map(|(a, v)| (*a, v.clone()))
                .collect();
            hits.sort_by_key(|(a, _)| (a.col, a.row));
            for (a, v) in hits {
                if f(a, &v).is_break() {
                    return;
                }
            }
        } else {
            for col in range.c0..=range.c1 {
                for row in range.r0..=range.r1 {
                    let a = CellAddr::new(range.sheet, row, col);
                    let v = self.cells.get(&a).cloned().unwrap_or_default();
                    if f(a, &v).is_break() {
                        return;
                    }
                }
            }
        }
    }

    fn used_bounds(&self, range: &CellRange) -> Option<CellRange> {
        self.used(range.sheet)?.intersect(range)
    }

    fn parse_reference_text(&self, text: &str, a1: bool) -> Option<CellRange> {
        if !a1 {
            return None;
        }
        CellRange::from_a1(self.origin.sheet, text)
    }

    fn record_dynamic_dep(&mut self, range: &CellRange) {
        self.dynamic_deps.push(*range);
    }

    fn now_serial(&self) -> f64 {
        self.now
    }

    fn rand(&mut self) -> f64 {
        // xorshift64*: deterministic, good enough for tests.
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        let x = self.rng.wrapping_mul(0x2545_F491_4F6C_DD1D);
        (x >> 11) as f64 / (1u64 << 53) as f64
    }

    fn sheet_index(&self, id: SheetId) -> Option<u32> {
        (id < self.sheets).then_some(id)
    }

    fn sheet_count(&self) -> u32 {
        self.sheets
    }

    fn cell_format_info(&self, addr: CellAddr) -> CellFormatInfo {
        self.formats.get(&addr).cloned().unwrap_or_default()
    }

    fn hidden_row(&self, sheet: SheetId, row: u32) -> bool {
        self.hidden_rows.contains(&(sheet, row))
    }

    fn in_array_context(&self) -> bool {
        self.array_context
    }
}

/// An [`ArgList`] over pre-built values that records evaluation order.
#[derive(Clone, Debug, Default)]
pub struct MockArgs {
    values: Vec<EvalValue>,
    /// Indices passed to [`ArgList::eval`], in order.
    pub evaluated: Vec<usize>,
}

impl MockArgs {
    /// Arguments from values; use [`EvalValue::Missing`] for `f(1;;3)` gaps.
    pub fn new(values: Vec<EvalValue>) -> Self {
        Self {
            values,
            evaluated: Vec::new(),
        }
    }

    /// `true` if argument `i` was evaluated at least once.
    pub fn was_evaluated(&self, i: usize) -> bool {
        self.evaluated.contains(&i)
    }
}

impl ArgList for MockArgs {
    fn len(&self) -> usize {
        self.values.len()
    }

    fn is_missing(&self, i: usize) -> bool {
        self.values.get(i).is_none_or(EvalValue::is_missing)
    }

    fn eval(&mut self, i: usize, _ctx: &mut dyn EvalContext) -> EvalValue {
        self.evaluated.push(i);
        self.values.get(i).cloned().unwrap_or(EvalValue::Missing)
    }
}

/// Calls a registered function by name and reduces the result as a cell
/// would (a returned reference is implicitly intersected; arrays come back
/// as `Value::Array`). Panics if the name is unknown (test helper).
pub fn call(ctx: &mut MockEvalContext, name: &str, args: Vec<EvalValue>) -> Value {
    call_with(ctx, name, &mut MockArgs::new(args))
}

/// Like [`call`] but keeps the [`MockArgs`] so the caller can inspect
/// [`MockArgs::evaluated`].
pub fn call_with(ctx: &mut MockEvalContext, name: &str, args: &mut MockArgs) -> Value {
    let raw = call_raw(ctx, name, args);
    match raw {
        EvalValue::Array(m) => Value::Array(m),
        other => crate::coerce::scalar_value(&other, ctx),
    }
}

/// Calls a registered function by name and returns the raw [`EvalValue`]
/// (references are not reduced). Panics if the name is unknown.
pub fn call_raw(ctx: &mut MockEvalContext, name: &str, args: &mut MockArgs) -> EvalValue {
    let reg = crate::registry();
    let id = reg
        .lookup(name)
        .unwrap_or_else(|| panic!("unknown function {name}"));
    reg.eval_call(ctx, id, args)
}
