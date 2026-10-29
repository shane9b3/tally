//! Test doubles: [`MockEvalContext`] (a `HashMap`-backed sheet set) and
//! [`MockArgs`] (an argument list that records which arguments were
//! evaluated). Available under `cfg(test)` and the `test-support` feature.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::ops::ControlFlow;
use std::sync::Arc;

use tally_types::{CalcSettings, CellAddr, CellRange, SheetId, Value};

use crate::context::{ArgList, CellFormatInfo, EvalContext, EvalValue};

/// A [`MockEvalContext::set_pivot_data`] hook: `(anchor, data field, filters)`.
pub type PivotDataFn = dyn Fn(CellAddr, &str, &[(String, String)]) -> Option<Value>;

/// A [`MockEvalContext::set_table_op`] hook: `(formula cell, substitutions)`.
pub type TableOpFn = dyn Fn(CellAddr, &[(CellAddr, Value)]) -> Option<Value>;

/// One recorded [`EvalContext::pivot_data`] request: `(anchor, data field, filters)`.
pub type PivotCall = (CellAddr, String, Vec<(String, String)>);

/// One recorded [`EvalContext::table_op`] request: `(formula cell, substitutions)`.
pub type TableOpCall = (CellAddr, Vec<(CellAddr, Value)>);

/// An optional shared closure with a `Debug` impl, so the context can keep
/// deriving `Clone` and `Debug`.
struct Hook<F: ?Sized>(Option<Arc<F>>);

impl<F: ?Sized> Clone for Hook<F> {
    fn clone(&self) -> Self {
        Hook(self.0.clone())
    }
}

impl<F: ?Sized> Default for Hook<F> {
    fn default() -> Self {
        Hook(None)
    }
}

impl<F: ?Sized> fmt::Debug for Hook<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(if self.0.is_some() {
            "Hook(set)"
        } else {
            "Hook(none)"
        })
    }
}

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
    /// The document locale tag reported by [`EvalContext::locale_tag`]
    /// (default `en-US`; see [`MockEvalContext::set_locale`]).
    pub locale: &'static str,
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
    /// Rows hidden by an AutoFilter (reported by both
    /// [`EvalContext::filtered_row`] and [`EvalContext::hidden_row`]).
    pub filtered_rows: HashSet<(SheetId, u32)>,
    /// Every [`EvalContext::pivot_data`] request.
    pub pivot_calls: Vec<PivotCall>,
    /// Every [`EvalContext::table_op`] request.
    pub table_op_calls: Vec<TableOpCall>,
    pivot_hook: Hook<PivotDataFn>,
    table_op_hook: Hook<TableOpFn>,
    rng: u64,
    /// Answer [`EvalContext::lookup_index`] (see [`MockEvalContext::set_lookup_index`]).
    lookup_index: bool,
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
            locale: "en-US",
            origin: CellAddr::new(0, 0, 0),
            array_context: false,
            now: 45_306.5,
            sheets: 1,
            dynamic_deps: Vec::new(),
            reads: Vec::new(),
            hidden_rows: HashSet::new(),
            filtered_rows: HashSet::new(),
            pivot_calls: Vec::new(),
            table_op_calls: Vec::new(),
            pivot_hook: Hook::default(),
            table_op_hook: Hook::default(),
            rng: 0x2545_F491_4F6C_DD1D,
            lookup_index: false,
        }
    }

    /// With `on`, [`EvalContext::lookup_index`] builds a fresh
    /// [`crate::LookupIndex`] for every line of any size (default off:
    /// lookups scan), so tests can compare both paths.
    pub fn set_lookup_index(&mut self, on: bool) -> &mut Self {
        self.lookup_index = on;
        self
    }

    /// Installs the pivot lookup behind [`EvalContext::pivot_data`]
    /// (GETPIVOTDATA). The hook receives the top-left cell of the table
    /// reference, the data field text and the `(field, item)` filters, and
    /// returns `None` when no pivot table answers. Without a hook every
    /// request is `None`.
    pub fn set_pivot_data(
        &mut self,
        f: impl Fn(CellAddr, &str, &[(String, String)]) -> Option<Value> + 'static,
    ) -> &mut Self {
        self.pivot_hook = Hook(Some(Arc::new(f)));
        self
    }

    /// Installs the what-if evaluator behind [`EvalContext::table_op`]
    /// (MULTIPLE.OPERATIONS). The hook receives the formula cell and the
    /// `(input cell, replacement value)` substitutions. Without a hook
    /// every request is `None`.
    pub fn set_table_op(
        &mut self,
        f: impl Fn(CellAddr, &[(CellAddr, Value)]) -> Option<Value> + 'static,
    ) -> &mut Self {
        self.table_op_hook = Hook(Some(Arc::new(f)));
        self
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

    /// Sets the document locale by tag (`"de-DE"`), as the engine does from
    /// `DocSettings.locale`. Panics on a tag `tally_numfmt::locale_by_tag`
    /// does not know (test helper); unknown tags are the engine's business.
    pub fn set_locale(&mut self, tag: &str) -> &mut Self {
        let l = tally_numfmt::locale_by_tag(tag)
            .unwrap_or_else(|| panic!("unknown locale tag {tag:?}"));
        self.locale = l.tag;
        self
    }

    /// A reference union argument from A1 ranges, e.g.
    /// `ctx.union(0, &["A1:A2", "B1:B2"])` (see [`EvalValue::union`]).
    /// Panics on malformed text (test helper).
    pub fn union(&self, sheet: SheetId, a1s: &[&str]) -> EvalValue {
        EvalValue::union(
            a1s.iter()
                .map(|a1| {
                    CellRange::from_a1(sheet, a1).unwrap_or_else(|| panic!("bad A1 range {a1:?}"))
                })
                .collect(),
        )
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

    fn locale_tag(&self) -> &str {
        self.locale
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

    fn sheet_at(&self, index: u32) -> Option<SheetId> {
        (index < self.sheets).then_some(index)
    }

    fn sheet_count(&self) -> u32 {
        self.sheets
    }

    fn cell_format_info(&self, addr: CellAddr) -> CellFormatInfo {
        self.formats.get(&addr).cloned().unwrap_or_default()
    }

    fn hidden_row(&self, sheet: SheetId, row: u32) -> bool {
        self.hidden_rows.contains(&(sheet, row)) || self.filtered_rows.contains(&(sheet, row))
    }

    fn filtered_row(&self, sheet: SheetId, row: u32) -> bool {
        self.filtered_rows.contains(&(sheet, row))
    }

    fn in_array_context(&self) -> bool {
        self.array_context
    }

    fn pivot_data(
        &mut self,
        at: CellAddr,
        field: &str,
        filters: &[(String, String)],
    ) -> Option<Value> {
        self.pivot_calls
            .push((at, field.to_owned(), filters.to_vec()));
        self.pivot_hook
            .0
            .as_ref()
            .and_then(|f| f(at, field, filters))
    }

    fn table_op(&mut self, formula: CellAddr, subs: &[(CellAddr, Value)]) -> Option<Value> {
        self.table_op_calls.push((formula, subs.to_vec()));
        self.table_op_hook.0.as_ref().and_then(|f| f(formula, subs))
    }

    fn lookup_index(&mut self, line: &CellRange) -> Option<Arc<crate::LookupIndex>> {
        if !self.lookup_index {
            return None;
        }
        let mut items = Vec::new();
        let (r0, c0) = (line.r0, line.c0);
        self.for_each_in_range(line, true, &mut |a, v| {
            items.push((a.row - r0 + a.col - c0, v.clone()));
            ControlFlow::Continue(())
        });
        Some(Arc::new(crate::LookupIndex::build(
            items,
            self.settings.case_sensitive,
        )))
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
