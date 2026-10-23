//! The formula evaluator.
//!
//! It is written against `&mut dyn EvalContext` only, so it runs the same
//! way inside the engine and over `MockEvalContext`. Operators are evaluated
//! with an explicit work stack and value stack (no Rust recursion over the
//! AST). Function arguments are evaluated on the same stack before the call,
//! except [`ParamKind::Lazy`] ones (IF branches…), which the function asks
//! for on demand; that is the only re-entry, and it is bounded by
//! [`MAX_EVAL_DEPTH`] (deeper gives Err:512).
//!
//! Semantics (AOO 4.1 unless noted):
//! - `+ - * /` coerce text per `settings.text_to_number`, empty → 0, bool →
//!   1/0; `x/0` is `#DIV/0!`; non-finite results are `#NUM!`; `+`/`-` use
//!   `approx_add` so `0.1+0.2-0.3` is 0.
//! - `^` is evaluated as parsed (left-associative); `0^0` = 1.
//! - `&` concatenates, rendering numbers like General.
//! - Comparisons: numbers (and booleans, as 1/0) numerically with
//!   `approx_equal`; text case-insensitively unless `settings.case_sensitive`;
//!   a number is less than any text; empty equals 0 and "".
//! - Unary `+` is the identity (AOO keeps text), unary `-` negates, `%` divides by 100.
//! - `:` between references gives their bounding box (`#VALUE!` otherwise
//!   or across sheets), `!` intersects (`#NULL!` when disjoint), `~` (union)
//!   joins two references into one `EvalValue::Union` operand (AOO
//!   `svRefList`): sequence parameters, `SUBTOTAL`, `AREAS`, `COUNTIF`,
//!   `INDEX` and `ISREF` accept it, anything that needs one rectangle or a
//!   scalar gives `#VALUE!` (a union as a cell's final value too).
//! - A multi-cell reference used as an operand is implicitly intersected
//!   with the formula cell, except in array context (array-forcing
//!   parameters such as SUMPRODUCT's), where references become matrices and
//!   operators apply elementwise. Inline arrays always operate elementwise.
//! - A 3-D reference (`Sheet1.A1:Sheet3.A1`) evaluates to `#VALUE!` in
//!   Phase A (`EvalValue` has no multi-sheet variant).

use std::cell::{Cell, RefCell};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::Arc;

use tally_formula::{ArrayLit, BinOp, Expr, FuncRef, RefExpr, ResolvedRef, UnOp};
use tally_functions::coerce::{scalar_value, value_to_number, value_to_text};
use tally_functions::numeric::approx_add;
use tally_functions::{
    check_arity, registry, ArgList, CellFormatInfo, EvalContext, EvalValue, FuncId, FunctionDef,
    ParamKind,
};
use tally_numfmt::{general_string, EN_US};
use tally_types::{
    approx_equal, CalcSettings, CellAddr, CellRange, ErrorKind, Matrix, SheetId, Value,
};

/// Maximum re-entry depth (lazy arguments evaluated by functions other
/// than IF, which is evaluated on the work stack). Sized so a debug build
/// stays within a 256 KB stack.
pub const MAX_EVAL_DEPTH: u32 = 40;

/// Upper bound on the cells of a reference turned into a matrix (2^21).
const MATRIX_LIMIT: u64 = 1 << 21;

/// The work budget of one formula evaluation, in element operations: every
/// evaluation step costs 1, an operator or function call over arrays or
/// ranges costs the number of cells it touches, and the matrix functions
/// their arithmetic (`MMULT` rows × inner × columns, `MINVERSE`/`MDETERM`
/// n³). A formula that would exceed it gives Err:512 ("formula overflow":
/// too complex to evaluate) without doing the work (security review 2,
/// MEDIUM-3). A 1000-row `SUMPRODUCT` over 20 whole columns stays far
/// below it; `MMULT` of two 1448² matrices (3·10⁹) does not.
pub const MAX_FORMULA_WORK: u64 = 50_000_000;

/// Per-evaluation state: the definitions of the names the formula uses,
/// their memoized values, the names being evaluated (cycle detection) and
/// the work budget.
///
/// A name is evaluated by reference: its own parsed expression is evaluated
/// where it is used, at the using cell (so relative references in names
/// keep OpenOffice semantics), once per (name, array context) per cell
/// evaluation. One env covers exactly one cell evaluation, so a memoized
/// value never outlives the state it was computed from.
#[derive(Debug)]
pub struct EvalEnv {
    names: crate::names::NameDefs,
    memo: RefCell<HashMap<(u32, bool), EvalValue>>,
    active: RefCell<Vec<u32>>,
    work: Cell<u64>,
    limit: u64,
}

impl Default for EvalEnv {
    fn default() -> Self {
        Self::new(crate::names::NameDefs::default())
    }
}

impl EvalEnv {
    /// An env over the given name definitions with the default budget.
    pub fn new(names: crate::names::NameDefs) -> Self {
        Self {
            names,
            memo: RefCell::new(HashMap::new()),
            active: RefCell::new(Vec::new()),
            work: Cell::new(0),
            limit: MAX_FORMULA_WORK,
        }
    }

    /// The same with a different work budget (tests).
    pub fn with_limit(mut self, limit: u64) -> Self {
        self.limit = limit;
        self
    }

    /// Work spent so far.
    pub fn work(&self) -> u64 {
        self.work.get()
    }

    /// Charges `n` units of work; `false` once the budget is spent.
    pub fn charge(&self, n: u64) -> bool {
        let w = self.work.get().saturating_add(n);
        self.work.set(w);
        w <= self.limit
    }

    /// `true` once the budget is spent.
    pub fn exhausted(&self) -> bool {
        self.work.get() > self.limit
    }

    fn enter(&self, id: u32) -> bool {
        let mut a = self.active.borrow_mut();
        if a.len() >= crate::names::MAX_NAME_DEPTH as usize || a.contains(&id) {
            return false;
        }
        a.push(id);
        true
    }

    fn leave(&self, id: u32, array: bool, v: Option<&EvalValue>) {
        let mut a = self.active.borrow_mut();
        if let Some(p) = a.iter().rposition(|x| *x == id) {
            a.truncate(p);
        }
        if let Some(v) = v {
            self.memo.borrow_mut().insert((id, array), v.clone());
        }
    }

    fn memoized(&self, id: u32, array: bool) -> Option<EvalValue> {
        self.memo.borrow().get(&(id, array)).cloned()
    }
}

/// Evaluates an expression as a cell formula (not in array context).
pub fn eval_expr(e: &Expr, ctx: &mut dyn EvalContext) -> EvalValue {
    eval_with(e, ctx, false, &EvalEnv::default())
}

/// Evaluates an expression in array context: references used as operands
/// become matrices and operators apply elementwise.
pub fn eval_in_array_context(e: &Expr, ctx: &mut dyn EvalContext) -> EvalValue {
    eval_with(e, ctx, true, &EvalEnv::default())
}

/// Evaluates an expression with names and a work budget (`array` selects
/// array context). Err:512 if the budget runs out.
pub fn eval_with(e: &Expr, ctx: &mut dyn EvalContext, array: bool, env: &EvalEnv) -> EvalValue {
    let v = eval_at_depth(e, ctx, array, 0, env);
    if env.exhausted() {
        return EvalValue::error(ErrorKind::FormulaOverflow);
    }
    v
}

/// Reduces an evaluation result to the value a cell stores: references by
/// implicit intersection, arrays to their top-left element; an empty result
/// (a reference to an empty cell) becomes 0, as in AOO.
pub fn cell_result(v: &EvalValue, ctx: &mut dyn EvalContext) -> Value {
    match scalar_value(v, ctx) {
        Value::Empty => Value::Number(0.0),
        Value::Number(n) if !n.is_finite() => Value::Error(ErrorKind::Num),
        Value::Array(m) => m
            .get(0, 0)
            .cloned()
            .unwrap_or(Value::Error(ErrorKind::Value)),
        other => other,
    }
}

/// Reduces an array-context result to a value, keeping matrices.
pub fn array_result(v: &EvalValue, ctx: &mut dyn EvalContext) -> Value {
    match v {
        EvalValue::Array(m) => Value::Array(m.clone()),
        EvalValue::Range(r) if !r.is_single() => match materialize(r, ctx) {
            Ok(m) => Value::Array(m),
            Err(e) => Value::Error(e),
        },
        other => cell_result(other, ctx),
    }
}

enum Work<'e> {
    Eval(&'e Expr, bool),
    Unary(UnOp, bool),
    Binary(BinOp, bool),
    Percent(bool),
    Array(&'e ArrayLit),
    Call {
        id: FuncId,
        def: &'static FunctionDef,
        args: &'e [Expr],
        array: bool,
    },
    /// IF outside array context: the test is on the value stack; push the branch.
    IfBranch {
        args: &'e [Expr],
        array: bool,
    },
    /// A name's expression was evaluated (its value is on the stack).
    NameDone(u32, bool),
}

fn pre_evaluated(def: &FunctionDef, i: usize, arg: &Expr) -> bool {
    def.param_kind(i) != ParamKind::Lazy && !matches!(arg, Expr::Missing)
}

/// The evaluator loop. `array` is the context of the root expression.
pub fn eval_at_depth<'e>(
    root: &'e Expr,
    ctx: &mut dyn EvalContext,
    array: bool,
    depth: u32,
    env: &'e EvalEnv,
) -> EvalValue {
    if depth > MAX_EVAL_DEPTH {
        return EvalValue::error(ErrorKind::FormulaOverflow);
    }
    let reg = registry();
    let mut work: Vec<Work<'e>> = vec![Work::Eval(root, array)];
    let mut vals: Vec<EvalValue> = Vec::new();
    while let Some(w) = work.pop() {
        if !env.charge(1) {
            return EvalValue::error(ErrorKind::FormulaOverflow);
        }
        match w {
            Work::Eval(e, arr) => match e {
                Expr::Num(n) => vals.push(EvalValue::number(*n)),
                Expr::Str(s) => vals.push(EvalValue::Scalar(Value::Text(s.clone()))),
                Expr::Bool(b) => vals.push(EvalValue::bool(*b)),
                Expr::Err(k) => vals.push(EvalValue::error(*k)),
                Expr::Missing => vals.push(EvalValue::Missing),
                Expr::Ref(r) => vals.push(resolve_ref(r, ctx.origin())),
                Expr::Name(n) => name_step(n, arr, env, &mut work, &mut vals),
                Expr::Paren(x) => work.push(Work::Eval(x, arr)),
                Expr::Unary(op, x) => {
                    work.push(Work::Unary(*op, arr));
                    work.push(Work::Eval(x, arr));
                }
                Expr::Postfix(_, x) => {
                    work.push(Work::Percent(arr));
                    work.push(Work::Eval(x, arr));
                }
                Expr::Binary(op, l, r) => {
                    work.push(Work::Binary(*op, arr));
                    work.push(Work::Eval(r, arr));
                    work.push(Work::Eval(l, arr));
                }
                Expr::ArrayLit(a) => {
                    work.push(Work::Array(a));
                    for item in a.items.iter().rev() {
                        work.push(Work::Eval(item, arr));
                    }
                }
                Expr::Call { func, args } => {
                    let def_id = match func {
                        FuncRef::Known(id) => {
                            let fid = FuncId(id.0);
                            reg.get(fid).map(|d| (fid, d))
                        }
                        FuncRef::Unknown(_) => None,
                    };
                    let Some((id, def)) = def_id else {
                        vals.push(EvalValue::error(ErrorKind::Name));
                        continue;
                    };
                    if let Err(e) = check_arity(def, args.len()) {
                        vals.push(EvalValue::error(e));
                        continue;
                    }
                    if !arr && def.name == "IF" && !args.is_empty() {
                        // Same result as calling IF, without re-entering
                        // the evaluator for the chosen branch.
                        work.push(Work::IfBranch { args, array: arr });
                        work.push(Work::Eval(&args[0], arr));
                        continue;
                    }
                    work.push(Work::Call {
                        id,
                        def,
                        args,
                        array: arr,
                    });
                    for (i, a) in args.iter().enumerate().rev() {
                        if pre_evaluated(def, i, a) {
                            work.push(Work::Eval(a, arr || def.param_forces_array(i)));
                        }
                    }
                }
            },
            Work::NameDone(id, arr) => env.leave(id, arr, vals.last()),
            Work::Unary(op, arr) => {
                let v = vals.pop().unwrap_or(EvalValue::Missing);
                if !env.charge(value_cells(&v, arr)) {
                    return EvalValue::error(ErrorKind::FormulaOverflow);
                }
                vals.push(apply_unary(op, v, arr, ctx));
            }
            Work::Percent(arr) => {
                let v = vals.pop().unwrap_or(EvalValue::Missing);
                if !env.charge(value_cells(&v, arr)) {
                    return EvalValue::error(ErrorKind::FormulaOverflow);
                }
                vals.push(map_unary(
                    v,
                    arr,
                    ctx,
                    &|x, s| match value_to_number(x, s) {
                        Ok(n) => Value::Number(n / 100.0),
                        Err(e) => Value::Error(e),
                    },
                ));
            }
            Work::Binary(op, arr) => {
                let r = vals.pop().unwrap_or(EvalValue::Missing);
                let l = vals.pop().unwrap_or(EvalValue::Missing);
                if !env.charge(value_cells(&l, arr).max(value_cells(&r, arr))) {
                    return EvalValue::error(ErrorKind::FormulaOverflow);
                }
                vals.push(apply_binary(op, l, r, arr, ctx));
            }
            Work::Array(a) => {
                let _ = env.charge(a.items.len() as u64);
                let n = a.items.len().min(vals.len());
                let items = vals.split_off(vals.len() - n);
                let data: Vec<Value> = items.iter().map(|v| scalar_value(v, ctx)).collect();
                vals.push(match Matrix::from_vec(a.rows, a.cols, data) {
                    Some(m) => EvalValue::Array(Arc::new(m)),
                    None => EvalValue::error(ErrorKind::Value),
                });
            }
            Work::IfBranch { args, array } => {
                let test = vals.pop().unwrap_or(EvalValue::Missing);
                let test = scalar_value(&test, ctx);
                match tally_functions::coerce::value_to_bool(&test) {
                    Err(e) => vals.push(EvalValue::error(e)),
                    Ok(t) => {
                        let branch = if t { 1 } else { 2 };
                        match args.get(branch) {
                            None => vals.push(EvalValue::bool(t)),
                            Some(Expr::Missing) => vals.push(EvalValue::number(0.0)),
                            Some(e) => work.push(Work::Eval(e, array)),
                        }
                    }
                }
            }
            Work::Call {
                id,
                def,
                args,
                array,
            } => {
                let n_pre = args
                    .iter()
                    .enumerate()
                    .filter(|(i, a)| pre_evaluated(def, *i, a))
                    .count()
                    .min(vals.len());
                let mut pre = vals.split_off(vals.len() - n_pre).into_iter();
                let slots: Vec<Option<EvalValue>> = args
                    .iter()
                    .enumerate()
                    .map(|(i, a)| {
                        if pre_evaluated(def, i, a) {
                            pre.next()
                        } else {
                            None
                        }
                    })
                    .collect();
                if !env.charge(call_cost(def, &slots, ctx, array)) {
                    return EvalValue::error(ErrorKind::FormulaOverflow);
                }
                let mut list = ExprArgs {
                    args,
                    slots,
                    def,
                    array,
                    depth,
                    env,
                };
                let mut actx = ArrayCtx { inner: ctx, array };
                vals.push(reg.eval_call(&mut actx, id, &mut list));
            }
        }
    }
    vals.pop().unwrap_or(EvalValue::Missing)
}

/// Evaluates a name by reference: its memoized value, or its expression
/// pushed on the work stack (`#NAME?` if undefined, self-referential or
/// nested deeper than [`crate::names::MAX_NAME_DEPTH`]).
#[inline(never)]
fn name_step<'e>(
    n: &tally_formula::NameRef,
    arr: bool,
    env: &'e EvalEnv,
    work: &mut Vec<Work<'e>>,
    vals: &mut Vec<EvalValue>,
) {
    match env.names.resolve(n) {
        None => vals.push(EvalValue::error(ErrorKind::Name)),
        Some((id, f)) => {
            if let Some(v) = env.memoized(id, arr) {
                vals.push(v);
            } else if env.enter(id) {
                work.push(Work::NameDone(id, arr));
                work.push(Work::Eval(&f.expr, arr));
            } else {
                vals.push(EvalValue::error(ErrorKind::Name));
            }
        }
    }
}

/// Cells an operand stands for (what an elementwise operation touches).
fn value_cells(v: &EvalValue, array: bool) -> u64 {
    match v {
        EvalValue::Array(m) | EvalValue::Scalar(Value::Array(m)) => {
            u64::from(m.rows) * u64::from(m.cols)
        }
        EvalValue::Range(r) if array => r.cell_count(),
        _ => 1,
    }
}

/// The work a call is charged before it runs: the cells of its array and
/// range arguments (the used part of ranges a function only scans), plus
/// the arithmetic of the matrix functions.
#[inline(never)]
fn call_cost(
    def: &FunctionDef,
    slots: &[Option<EvalValue>],
    ctx: &dyn EvalContext,
    array: bool,
) -> u64 {
    let mut total: u64 = 1;
    let mut dims: Vec<(u64, u64)> = Vec::new();
    for (i, s) in slots.iter().enumerate() {
        let Some(v) = s else {
            continue;
        };
        let (cells, d) = match v {
            EvalValue::Array(m) | EvalValue::Scalar(Value::Array(m)) => {
                let d = (u64::from(m.rows), u64::from(m.cols));
                (d.0 * d.1, d)
            }
            EvalValue::Range(r) if !r.is_single() => {
                let d = (u64::from(r.rows()), u64::from(r.cols()));
                let dense = array
                    || def.param_forces_array(i)
                    || matches!(def.param_kind(i), ParamKind::Array);
                let cells = if dense {
                    r.cell_count()
                } else {
                    ctx.used_bounds(r).map_or(0, |u| u.cell_count())
                };
                (cells, d)
            }
            _ => (1, (1, 1)),
        };
        total = total.saturating_add(cells);
        dims.push(d);
    }
    let extra = match def.name {
        "MMULT" if dims.len() >= 2 => dims[0]
            .0
            .saturating_mul(dims[0].1)
            .saturating_mul(dims[1].1),
        "MINVERSE" | "MDETERM" if !dims.is_empty() => {
            let n = dims[0].0.max(dims[0].1);
            n.saturating_mul(n).saturating_mul(n)
        }
        "MUNIT" => match slots.first() {
            Some(Some(EvalValue::Scalar(Value::Number(n)))) if *n > 0.0 => {
                let n = n.min(1e9) as u64;
                n.saturating_mul(n)
            }
            _ => 0,
        },
        _ => 0,
    };
    total.saturating_add(extra)
}

/// Resolves a reference at `origin`.
fn resolve_ref(r: &RefExpr, origin: CellAddr) -> EvalValue {
    match r.resolve(origin) {
        ResolvedRef::Range(cr) => EvalValue::Range(cr),
        ResolvedRef::Span3d { .. } => EvalValue::error(ErrorKind::Value),
        ResolvedRef::Invalid => EvalValue::error(ErrorKind::Ref),
    }
}

/// The arguments of one call: pre-evaluated slots, lazy ones evaluated on demand.
struct ExprArgs<'e> {
    args: &'e [Expr],
    slots: Vec<Option<EvalValue>>,
    def: &'static FunctionDef,
    array: bool,
    depth: u32,
    env: &'e EvalEnv,
}

impl ArgList for ExprArgs<'_> {
    fn len(&self) -> usize {
        self.args.len()
    }

    fn is_missing(&self, i: usize) -> bool {
        self.args.get(i).is_none_or(|a| matches!(a, Expr::Missing))
    }

    fn eval(&mut self, i: usize, ctx: &mut dyn EvalContext) -> EvalValue {
        match (self.slots.get(i), self.args.get(i)) {
            (Some(Some(v)), _) => v.clone(),
            (_, None) | (_, Some(Expr::Missing)) => EvalValue::Missing,
            (_, Some(e)) => {
                let arr = self.array || self.def.param_forces_array(i);
                eval_at_depth(e, ctx, arr, self.depth + 1, self.env)
            }
        }
    }
}

/// Delegating context that reports the array-context flag of a call.
struct ArrayCtx<'a> {
    inner: &'a mut dyn EvalContext,
    array: bool,
}

impl EvalContext for ArrayCtx<'_> {
    fn origin(&self) -> CellAddr {
        self.inner.origin()
    }
    fn settings(&self) -> &CalcSettings {
        self.inner.settings()
    }
    fn locale_tag(&self) -> &str {
        self.inner.locale_tag()
    }
    fn cell_value(&mut self, addr: CellAddr) -> Value {
        self.inner.cell_value(addr)
    }
    fn for_each_in_range(
        &mut self,
        range: &CellRange,
        skip_empty: bool,
        f: &mut dyn FnMut(CellAddr, &Value) -> std::ops::ControlFlow<()>,
    ) {
        self.inner.for_each_in_range(range, skip_empty, f);
    }
    fn used_bounds(&self, range: &CellRange) -> Option<CellRange> {
        self.inner.used_bounds(range)
    }
    fn parse_reference_text(&self, text: &str, a1: bool) -> Option<CellRange> {
        self.inner.parse_reference_text(text, a1)
    }
    fn record_dynamic_dep(&mut self, range: &CellRange) {
        self.inner.record_dynamic_dep(range);
    }
    fn now_serial(&self) -> f64 {
        self.inner.now_serial()
    }
    fn rand(&mut self) -> f64 {
        self.inner.rand()
    }
    fn sheet_index(&self, id: SheetId) -> Option<u32> {
        self.inner.sheet_index(id)
    }
    fn sheet_count(&self) -> u32 {
        self.inner.sheet_count()
    }
    fn cell_format_info(&self, addr: CellAddr) -> CellFormatInfo {
        self.inner.cell_format_info(addr)
    }
    fn in_array_context(&self) -> bool {
        self.array
    }
    fn lookup_index(
        &mut self,
        line: &CellRange,
    ) -> Option<std::sync::Arc<tally_functions::LookupIndex>> {
        self.inner.lookup_index(line)
    }
    fn hidden_row(&self, sheet: SheetId, row: u32) -> bool {
        self.inner.hidden_row(sheet, row)
    }
    fn filtered_row(&self, sheet: SheetId, row: u32) -> bool {
        self.inner.filtered_row(sheet, row)
    }
    fn table_op(&mut self, formula: CellAddr, subs: &[(CellAddr, Value)]) -> Option<Value> {
        self.inner.table_op(formula, subs)
    }
    fn pivot_data(
        &mut self,
        at: CellAddr,
        field: &str,
        filters: &[(String, String)],
    ) -> Option<Value> {
        self.inner.pivot_data(at, field, filters)
    }
    fn pivot_data_excel(
        &mut self,
        at: CellAddr,
        words: &str,
        filters: &[(String, String)],
    ) -> Option<Value> {
        self.inner.pivot_data_excel(at, words, filters)
    }
}

// ---------------------------------------------------------------------------
// Operands
// ---------------------------------------------------------------------------

enum Operand {
    Scalar(Value),
    Matrix(Arc<Matrix>),
}

/// Turns a reference into a matrix (empty cells stay `Empty`).
pub fn materialize(r: &CellRange, ctx: &mut dyn EvalContext) -> Result<Arc<Matrix>, ErrorKind> {
    if r.cell_count() > MATRIX_LIMIT {
        return Err(ErrorKind::Value);
    }
    let mut m = Matrix::new_filled(r.rows(), r.cols(), Value::Empty);
    let (r0, c0) = (r.r0, r.c0);
    ctx.for_each_in_range(r, true, &mut |addr, val| {
        m.set(addr.row - r0, addr.col - c0, val.clone());
        std::ops::ControlFlow::Continue(())
    });
    Ok(Arc::new(m))
}

fn operand(v: EvalValue, array: bool, ctx: &mut dyn EvalContext) -> Operand {
    match v {
        EvalValue::Missing => Operand::Scalar(Value::Empty),
        EvalValue::Scalar(Value::Array(m)) | EvalValue::Array(m) => Operand::Matrix(m),
        EvalValue::Scalar(v) => Operand::Scalar(v),
        EvalValue::Range(r) if array && !r.is_single() => match materialize(&r, ctx) {
            Ok(m) => Operand::Matrix(m),
            Err(e) => Operand::Scalar(Value::Error(e)),
        },
        // A single-cell range, an implicitly intersected one, or a union
        // (which has no single cell to intersect with: `#VALUE!`).
        other => Operand::Scalar(scalar_value(&other, ctx)),
    }
}

/// Matrix element pick: a single row or column is replicated, positions
/// outside a larger result are `#N/A` (the result takes the larger
/// extent). This is the Excel/LibreOffice rule, a deliberate deviation from
/// AOO 4.1, which takes the smaller extent (see docs/adr/0001-stack.md).
fn pick(m: &Matrix, r: u32, c: u32) -> Value {
    let rr = if m.rows == 1 { 0 } else { r };
    let cc = if m.cols == 1 { 0 } else { c };
    m.get(rr, cc)
        .cloned()
        .unwrap_or(Value::Error(ErrorKind::NA))
}

fn map_unary(
    v: EvalValue,
    array: bool,
    ctx: &mut dyn EvalContext,
    f: &dyn Fn(&Value, &CalcSettings) -> Value,
) -> EvalValue {
    match operand(v, array, ctx) {
        Operand::Scalar(x) => EvalValue::Scalar(f(&x, ctx.settings())),
        Operand::Matrix(m) => {
            let s = ctx.settings();
            let data = m.data.iter().map(|x| f(x, s)).collect();
            match Matrix::from_vec(m.rows, m.cols, data) {
                Some(out) => EvalValue::Array(Arc::new(out)),
                None => EvalValue::error(ErrorKind::Value),
            }
        }
    }
}

fn apply_unary(op: UnOp, v: EvalValue, array: bool, ctx: &mut dyn EvalContext) -> EvalValue {
    match op {
        UnOp::Plus => v,
        UnOp::Neg => map_unary(v, array, ctx, &|x, s| match value_to_number(x, s) {
            Ok(n) => Value::Number(-n),
            Err(e) => Value::Error(e),
        }),
    }
}

fn apply_binary(
    op: BinOp,
    l: EvalValue,
    r: EvalValue,
    array: bool,
    ctx: &mut dyn EvalContext,
) -> EvalValue {
    match op {
        BinOp::Range => return range_op(&l, &r),
        BinOp::Intersect => return intersect_op(&l, &r),
        BinOp::Union => return union_op(&l, &r),
        _ => {}
    }
    let a = operand(l, array, ctx);
    let b = operand(r, array, ctx);
    let s = ctx.settings();
    match (a, b) {
        (Operand::Scalar(x), Operand::Scalar(y)) => EvalValue::Scalar(scalar_op(op, &x, &y, s)),
        (a, b) => {
            let (ra, ca) = dims(&a);
            let (rb, cb) = dims(&b);
            let rows = ra.max(rb);
            let cols = ca.max(cb);
            if u64::from(rows) * u64::from(cols) > MATRIX_LIMIT {
                return EvalValue::error(ErrorKind::Value);
            }
            let mut data = Vec::with_capacity(rows as usize * cols as usize);
            for i in 0..rows {
                for j in 0..cols {
                    let x = elem(&a, i, j);
                    let y = elem(&b, i, j);
                    data.push(scalar_op(op, &x, &y, s));
                }
            }
            match Matrix::from_vec(rows, cols, data) {
                Some(m) => EvalValue::Array(Arc::new(m)),
                None => EvalValue::error(ErrorKind::Value),
            }
        }
    }
}

fn dims(o: &Operand) -> (u32, u32) {
    match o {
        Operand::Scalar(_) => (1, 1),
        Operand::Matrix(m) => (m.rows, m.cols),
    }
}

fn elem(o: &Operand, r: u32, c: u32) -> Value {
    match o {
        Operand::Scalar(v) => v.clone(),
        Operand::Matrix(m) => pick(m, r, c),
    }
}

fn range_op(l: &EvalValue, r: &EvalValue) -> EvalValue {
    if let Some(e) = l.as_error().or_else(|| r.as_error()) {
        return EvalValue::error(e);
    }
    match (l, r) {
        (EvalValue::Range(a), EvalValue::Range(b)) if a.sheet == b.sheet => {
            EvalValue::Range(CellRange::new(
                a.sheet,
                a.r0.min(b.r0),
                a.c0.min(b.c0),
                a.r1.max(b.r1),
                a.c1.max(b.c1),
            ))
        }
        _ => EvalValue::error(ErrorKind::Value),
    }
}

/// `a~b`: the areas of both sides, in formula order (nested unions
/// concatenate). Anything that is not a reference gives `#VALUE!`.
fn union_op(l: &EvalValue, r: &EvalValue) -> EvalValue {
    if let Some(e) = l.as_error().or_else(|| r.as_error()) {
        return EvalValue::error(e);
    }
    match (l.areas(), r.areas()) {
        (Some(a), Some(b)) => {
            let mut areas = Vec::with_capacity(a.len() + b.len());
            areas.extend_from_slice(a);
            areas.extend_from_slice(b);
            EvalValue::union(areas)
        }
        _ => EvalValue::error(ErrorKind::Value),
    }
}

fn intersect_op(l: &EvalValue, r: &EvalValue) -> EvalValue {
    if let Some(e) = l.as_error().or_else(|| r.as_error()) {
        return EvalValue::error(e);
    }
    match (l, r) {
        (EvalValue::Range(a), EvalValue::Range(b)) => match a.intersect(b) {
            Some(x) => EvalValue::Range(x),
            None => EvalValue::error(ErrorKind::Null),
        },
        _ => EvalValue::error(ErrorKind::Value),
    }
}

fn finite(x: f64) -> Value {
    if x.is_finite() {
        Value::Number(x)
    } else {
        Value::Error(ErrorKind::Num)
    }
}

/// Applies a binary operator to two scalars.
pub fn scalar_op(op: BinOp, a: &Value, b: &Value, s: &CalcSettings) -> Value {
    if let Value::Error(e) = a {
        return Value::Error(*e);
    }
    if let Value::Error(e) = b {
        return Value::Error(*e);
    }
    match op {
        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Pow => {
            let x = match value_to_number(a, s) {
                Ok(x) => x,
                Err(e) => return Value::Error(e),
            };
            let y = match value_to_number(b, s) {
                Ok(y) => y,
                Err(e) => return Value::Error(e),
            };
            match op {
                BinOp::Add => finite(approx_add(x, y)),
                BinOp::Sub => finite(approx_add(x, -y)),
                BinOp::Mul => finite(x * y),
                BinOp::Div => {
                    if y == 0.0 {
                        Value::Error(ErrorKind::Div0)
                    } else {
                        finite(x / y)
                    }
                }
                _ => {
                    if x == 0.0 && y == 0.0 {
                        Value::Number(1.0)
                    } else {
                        // As POWER (AOO ScPow / GetDoubleErrorValue): NaN
                        // (a negative base with a fractional exponent) is
                        // #VALUE!, an infinite result #NUM!.
                        let r = x.powf(y);
                        if r.is_nan() {
                            Value::Error(ErrorKind::Value)
                        } else {
                            finite(r)
                        }
                    }
                }
            }
        }
        BinOp::Concat => {
            let x = match text_of(a) {
                Ok(t) => t,
                Err(e) => return Value::Error(e),
            };
            let y = match text_of(b) {
                Ok(t) => t,
                Err(e) => return Value::Error(e),
            };
            // Text results are capped like function results (Err:513).
            if x.len().saturating_add(y.len()) > tally_functions::MAX_TEXT_BYTES {
                return Value::Error(ErrorKind::StringOverflow);
            }
            let mut out = String::with_capacity(x.len() + y.len());
            out.push_str(&x);
            out.push_str(&y);
            Value::text(out)
        }
        BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
            let ord = compare(a, b, s.case_sensitive);
            Value::Bool(match op {
                BinOp::Eq => ord == Ordering::Equal,
                BinOp::Ne => ord != Ordering::Equal,
                BinOp::Lt => ord == Ordering::Less,
                BinOp::Le => ord != Ordering::Greater,
                BinOp::Gt => ord == Ordering::Greater,
                _ => ord != Ordering::Less,
            })
        }
        BinOp::Range | BinOp::Intersect | BinOp::Union => Value::Error(ErrorKind::Value),
    }
}

fn text_of(v: &Value) -> Result<Arc<str>, ErrorKind> {
    match v {
        Value::Number(n) => Ok(general_string(*n, None, &EN_US).into()),
        other => value_to_text(other),
    }
}

enum Cmp<'a> {
    Num(f64),
    Text(&'a str),
}

/// Compares two scalars with the operator rules in the module docs.
pub fn compare(a: &Value, b: &Value, case_sensitive: bool) -> Ordering {
    fn class<'a>(v: &'a Value, other: &Value) -> Cmp<'a> {
        match v {
            Value::Number(n) => Cmp::Num(*n),
            Value::Bool(b) => Cmp::Num(if *b { 1.0 } else { 0.0 }),
            Value::Text(t) => Cmp::Text(t),
            // Empty takes the type of the other side.
            _ => match other {
                Value::Text(_) => Cmp::Text(""),
                _ => Cmp::Num(0.0),
            },
        }
    }
    match (class(a, b), class(b, a)) {
        (Cmp::Num(x), Cmp::Num(y)) => {
            if approx_equal(x, y) {
                Ordering::Equal
            } else {
                x.partial_cmp(&y).unwrap_or(Ordering::Equal)
            }
        }
        (Cmp::Text(x), Cmp::Text(y)) => {
            if case_sensitive {
                x.cmp(y)
            } else {
                x.to_lowercase().cmp(&y.to_lowercase())
            }
        }
        (Cmp::Num(_), Cmp::Text(_)) => Ordering::Less,
        (Cmp::Text(_), Cmp::Num(_)) => Ordering::Greater,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tally_formula::{parse, Dialect, SimpleResolver};
    use tally_functions::mock::MockEvalContext;

    fn resolver() -> SimpleResolver {
        let mut r = SimpleResolver::new();
        r.add_sheet(0, "Sheet1");
        r
    }

    fn eval_str(ctx: &mut MockEvalContext, src: &str) -> Value {
        let r = resolver();
        let f = parse(src, ctx.origin, Dialect::calc_a1_en_us(), &r).unwrap();
        let v = eval_expr(&f.expr, ctx);
        cell_result(&v, ctx)
    }

    #[test]
    fn arithmetic_and_errors() {
        let mut c = MockEvalContext::new();
        c.set(0, "A1", 2.0).set(0, "B1", 3.0).set(0, "C1", "x");
        assert_eq!(eval_str(&mut c, "=A1+B1*2"), Value::Number(8.0));
        assert_eq!(eval_str(&mut c, "=1/0"), Value::Error(ErrorKind::Div0));
        assert_eq!(eval_str(&mut c, "=C1+1"), Value::Error(ErrorKind::Value));
        assert_eq!(eval_str(&mut c, "=\"3\"+1"), Value::Number(4.0));
        assert_eq!(eval_str(&mut c, "=0^0"), Value::Number(1.0));
        assert_eq!(eval_str(&mut c, "=-2^2"), Value::Number(4.0));
        assert_eq!(eval_str(&mut c, "=2^3^2"), Value::Number(64.0));
        assert_eq!(eval_str(&mut c, "=10^400"), Value::Error(ErrorKind::Num));
        assert_eq!(eval_str(&mut c, "=50%"), Value::Number(0.5));
        assert_eq!(eval_str(&mut c, "=0.1+0.2-0.3"), Value::Number(0.0));
        assert_eq!(
            eval_str(&mut c, "=Z9"),
            Value::Number(0.0),
            "empty ref is 0"
        );
        assert_eq!(eval_str(&mut c, "=+\"abc\""), Value::text("abc"));
    }

    #[test]
    fn concat_and_compare() {
        let mut c = MockEvalContext::new();
        c.set(0, "A1", 1.5).set(0, "B1", "abc");
        assert_eq!(eval_str(&mut c, "=A1&\"x\""), Value::text("1.5x"));
        assert_eq!(eval_str(&mut c, "=1&TRUE()"), Value::text("1TRUE"));
        assert_eq!(eval_str(&mut c, "=B1=\"ABC\""), Value::Bool(true));
        assert_eq!(eval_str(&mut c, "=1<\"a\""), Value::Bool(true));
        assert_eq!(eval_str(&mut c, "=Z1=0"), Value::Bool(true));
        assert_eq!(eval_str(&mut c, "=Z1=\"\""), Value::Bool(true));
        assert_eq!(eval_str(&mut c, "=\"b\">\"A\""), Value::Bool(true));
        c.settings.case_sensitive = true;
        assert_eq!(eval_str(&mut c, "=B1=\"ABC\""), Value::Bool(false));
    }

    #[test]
    fn implicit_intersection_and_arrays() {
        let mut c = MockEvalContext::new();
        c.set(0, "A1", 1.0).set(0, "A2", 2.0).set(0, "A3", 3.0);
        c.origin = CellAddr::from_a1(0, "C2").unwrap();
        assert_eq!(eval_str(&mut c, "=A1:A3"), Value::Number(2.0));
        assert_eq!(eval_str(&mut c, "=A1:A3*10"), Value::Number(20.0));
        c.origin = CellAddr::from_a1(0, "C9").unwrap();
        assert_eq!(eval_str(&mut c, "=A1:A3"), Value::Error(ErrorKind::Value));
        assert_eq!(eval_str(&mut c, "={1;2;3}*2"), Value::Number(2.0));
        let r = resolver();
        let f = parse("=(A1:A3>1)*A1:A3", c.origin, Dialect::calc_a1_en_us(), &r).unwrap();
        let v = eval_in_array_context(&f.expr, &mut c);
        let m = match array_result(&v, &mut c) {
            Value::Array(m) => m,
            other => panic!("{other:?}"),
        };
        assert_eq!(
            m.data,
            vec![Value::Number(0.0), Value::Number(2.0), Value::Number(3.0)]
        );
    }

    #[test]
    fn reference_operators() {
        let mut c = MockEvalContext::new();
        c.set(0, "B2", 5.0);
        assert_eq!(eval_str(&mut c, "=A1:B2!B2:C3"), Value::Number(5.0));
        assert_eq!(
            eval_str(&mut c, "=A1:A2!C3:C4"),
            Value::Error(ErrorKind::Null)
        );
        assert_eq!(eval_str(&mut c, "=A1~B2"), Value::Error(ErrorKind::Value));
        assert_eq!(
            eval_str(&mut c, "=(-8)^(1/3)"),
            Value::Error(ErrorKind::Value)
        );
        assert_eq!(eval_str(&mut c, "=0^-1"), Value::Error(ErrorKind::Num));
        assert_eq!(eval_str(&mut c, "=10^400"), Value::Error(ErrorKind::Num));
        assert_eq!(eval_str(&mut c, "=(-8)^3"), Value::Number(-512.0));
    }

    #[test]
    fn deep_operator_chain_is_iterative() {
        // 1+1+...+1 with the parser's maximum tree depth, on a small stack.
        let src = format!("={}", vec!["1"; 1000].join("+"));
        let h = std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(move || {
                let mut c = MockEvalContext::new();
                eval_str(&mut c, &src)
            })
            .unwrap();
        assert_eq!(h.join().unwrap(), Value::Number(1000.0));
    }

    #[test]
    fn compare_rules() {
        assert_eq!(
            compare(&Value::Number(1.0), &Value::Bool(true), false),
            Ordering::Equal
        );
        assert_eq!(
            compare(&Value::Empty, &Value::Empty, false),
            Ordering::Equal
        );
        assert_eq!(
            compare(&Value::text("a"), &Value::Number(9.0), false),
            Ordering::Greater
        );
    }
}
