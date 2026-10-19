//! The call path: arity check, argument preparation per [`ParamKind`],
//! implicit intersection and array lifting, then the function itself.

use std::sync::Arc;

use tally_types::{CellRange, ErrorKind, Matrix, Value};

use crate::args::{check_arity, sanitize};
use crate::coerce::{intersect, scalar_value};
use crate::context::{ArgList, EvalContext, EvalValue};
use crate::registry::{FnFlags, FuncId, FunctionDef, ParamKind, Registry};

/// Upper bound on the cells of one array-lifted call (2^21).
const LIFT_LIMIT: u64 = 1 << 21;

/// Arguments after preparation: prepared slots, falling back to the
/// engine's list for lazy (`None`) slots.
struct Prepared<'a> {
    slots: Vec<Option<EvalValue>>,
    inner: &'a mut dyn ArgList,
}

impl ArgList for Prepared<'_> {
    fn len(&self) -> usize {
        self.slots.len()
    }

    fn is_missing(&self, i: usize) -> bool {
        match self.slots.get(i) {
            None => true,
            Some(Some(v)) => v.is_missing(),
            Some(None) => self.inner.is_missing(i),
        }
    }

    fn eval(&mut self, i: usize, ctx: &mut dyn EvalContext) -> EvalValue {
        match self.slots.get(i) {
            None => EvalValue::Missing,
            Some(Some(v)) => v.clone(),
            Some(None) => self.inner.eval(i, ctx),
        }
    }
}

fn is_multi_cell(v: &EvalValue) -> bool {
    match v {
        EvalValue::Range(r) => !r.is_single(),
        EvalValue::Array(m) => m.rows * m.cols != 1,
        EvalValue::Scalar(Value::Array(m)) => m.rows * m.cols != 1,
        _ => false,
    }
}

fn intersect_arg(r: &CellRange, ctx: &mut dyn EvalContext) -> EvalValue {
    match intersect(r, ctx.origin()) {
        Ok(addr) => EvalValue::from_value(ctx.cell_value(addr)),
        Err(e) => EvalValue::error(e),
    }
}

fn materialize(v: &EvalValue, ctx: &mut dyn EvalContext) -> Result<Arc<Matrix>, ErrorKind> {
    match v {
        EvalValue::Array(m) | EvalValue::Scalar(Value::Array(m)) => Ok(m.clone()),
        EvalValue::Range(r) => {
            if r.cell_count() > LIFT_LIMIT {
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
        other => Ok(Arc::new(Matrix::new_filled(
            1,
            1,
            match other {
                EvalValue::Scalar(v) => v.clone(),
                _ => Value::Empty,
            },
        ))),
    }
}

/// OOo matrix-jump element pick: a single row or column is replicated,
/// positions outside a larger result are `#N/A`.
fn pick(m: &Matrix, r: u32, c: u32) -> Value {
    let rr = if m.rows == 1 { 0 } else { r };
    let cc = if m.cols == 1 { 0 } else { c };
    m.get(rr, cc)
        .cloned()
        .unwrap_or(Value::Error(ErrorKind::NA))
}

fn invoke(
    def: &FunctionDef,
    ctx: &mut dyn EvalContext,
    inner: &mut dyn ArgList,
    slots: Vec<Option<EvalValue>>,
) -> EvalValue {
    let mut prepared = Prepared { slots, inner };
    sanitize((def.eval)(ctx, &mut prepared))
}

/// Calls `def` with `args`: checks arity (`Err:511` too few, `Err:504` too
/// many), evaluates every non-[`ParamKind::Lazy`] argument once, applies
/// implicit intersection to ranges given to scalar params, lifts liftable
/// params elementwise when `ctx.in_array_context()` (never for
/// [`FnFlags::LAZY_ARGS`] functions), then runs the implementation.
/// Non-finite numeric results become `#NUM!`.
///
/// The result may be a reference (from [`FnFlags::RETURNS_REF`] functions):
/// the engine passes it on to an enclosing function, or reduces it with
/// [`crate::coerce::scalar_value`] when it is a cell's final result.
pub fn call_def(def: &FunctionDef, ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let n = args.len();
    if let Err(e) = check_arity(def, n) {
        return EvalValue::error(e);
    }
    let mut slots: Vec<Option<EvalValue>> = Vec::with_capacity(n);
    for i in 0..n {
        let slot = if def.param_kind(i) == ParamKind::Lazy {
            None
        } else if args.is_missing(i) {
            Some(EvalValue::Missing)
        } else {
            Some(args.eval(i, ctx))
        };
        slots.push(slot);
    }

    let lifted: Vec<usize> = if ctx.in_array_context() && !def.flags.contains(FnFlags::LAZY_ARGS) {
        (0..n)
            .filter(|&i| {
                def.param_kind(i).is_liftable() && slots[i].as_ref().is_some_and(is_multi_cell)
            })
            .collect()
    } else {
        Vec::new()
    };

    for (i, slot) in slots.iter_mut().enumerate() {
        if lifted.contains(&i) || !def.param_kind(i).is_scalar() {
            continue;
        }
        if let Some(EvalValue::Range(r)) = slot {
            let r = *r;
            *slot = Some(intersect_arg(&r, ctx));
        }
    }

    if lifted.is_empty() {
        return invoke(def, ctx, args, slots);
    }
    lift(def, ctx, args, slots, &lifted)
}

fn lift(
    def: &FunctionDef,
    ctx: &mut dyn EvalContext,
    args: &mut dyn ArgList,
    slots: Vec<Option<EvalValue>>,
    lifted: &[usize],
) -> EvalValue {
    let mut mats: Vec<(usize, Arc<Matrix>)> = Vec::with_capacity(lifted.len());
    for &i in lifted {
        let Some(v) = &slots[i] else { continue };
        match materialize(v, ctx) {
            Ok(m) => mats.push((i, m)),
            Err(e) => return EvalValue::error(e),
        }
    }
    let rows = mats.iter().map(|(_, m)| m.rows).max().unwrap_or(1);
    let cols = mats.iter().map(|(_, m)| m.cols).max().unwrap_or(1);
    if u64::from(rows) * u64::from(cols) > LIFT_LIMIT {
        return EvalValue::error(ErrorKind::Value);
    }
    let mut out = Matrix::new_filled(rows, cols, Value::Empty);
    for r in 0..rows {
        for c in 0..cols {
            let mut call_slots = slots.clone();
            for (i, m) in &mats {
                call_slots[*i] = Some(EvalValue::Scalar(pick(m, r, c)));
            }
            let v = scalar_value(&invoke(def, ctx, args, call_slots), ctx);
            out.set(r, c, v);
        }
    }
    EvalValue::Array(Arc::new(out))
}

impl Registry {
    /// Calls the function `func` (see [`call_def`]); an unknown id is `#NAME?`.
    pub fn eval_call(
        &self,
        ctx: &mut dyn EvalContext,
        func: FuncId,
        args: &mut dyn ArgList,
    ) -> EvalValue {
        match self.get(func) {
            Some(def) => call_def(def, ctx, args),
            None => EvalValue::error(ErrorKind::Name),
        }
    }
}
