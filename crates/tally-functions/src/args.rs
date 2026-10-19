//! Ergonomic helpers for function authors.
//!
//! ```ignore
//! fn eval_round(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
//!     let mut a = Args::new(ctx, args);
//!     run(|| {
//!         let x = a.num(0)?;
//!         let places = a.int_opt(1, 0)?;
//!         Ok(Value::Number(round(x, places)))
//!     })
//! }
//! ```

use std::sync::Arc;

use tally_types::{CellRange, ErrorKind, Value};

use crate::coerce::{self, SeqSource};
use crate::context::{ArgList, EvalContext, EvalValue};
use crate::criteria::Criterion;
use crate::registry::FunctionDef;

/// Longest text (in bytes) a formula may produce: 4 MiB. Longer results of
/// operators and functions are `Err:513` (string overflow), so nested text
/// functions cannot build gigabyte strings.
pub const MAX_TEXT_BYTES: usize = 1 << 22;

/// `Err:513` if a text of `len` bytes would exceed [`MAX_TEXT_BYTES`].
/// Functions that can grow text call this *before* allocating.
pub fn check_text_len(len: usize) -> Result<(), ErrorKind> {
    if len > MAX_TEXT_BYTES {
        Err(ErrorKind::StringOverflow)
    } else {
        Ok(())
    }
}

/// Runs a function body that produces a value, turning `Err(e)` into
/// `Value::Error(e)` and non-finite numbers into `#NUM!`.
pub fn run(body: impl FnOnce() -> Result<Value, ErrorKind>) -> EvalValue {
    sanitize(EvalValue::from_value(body().unwrap_or_else(Value::Error)))
}

/// Like [`run`] for bodies that may return a reference (IF, INDEX, OFFSET).
pub fn run_ref(body: impl FnOnce() -> Result<EvalValue, ErrorKind>) -> EvalValue {
    sanitize(body().unwrap_or_else(EvalValue::error))
}

/// Maps NaN and infinities to `#NUM!` (AOO never stores them) and text
/// longer than [`MAX_TEXT_BYTES`] to `Err:513`, including inside arrays.
/// Every function result passes through here, so the text cap holds for
/// each level of nested calls.
pub fn sanitize(v: EvalValue) -> EvalValue {
    fn fix(v: &Value) -> Option<Value> {
        match v {
            Value::Number(n) if !n.is_finite() => Some(Value::Error(ErrorKind::Num)),
            Value::Text(t) if t.len() > MAX_TEXT_BYTES => {
                Some(Value::Error(ErrorKind::StringOverflow))
            }
            _ => None,
        }
    }
    match v {
        EvalValue::Scalar(ref s) => fix(s).map_or(v, EvalValue::Scalar),
        EvalValue::Array(m) if m.data.iter().any(|x| fix(x).is_some()) => {
            let mut m = (*m).clone();
            for x in &mut m.data {
                if let Some(f) = fix(x) {
                    *x = f;
                }
            }
            EvalValue::Array(Arc::new(m))
        }
        other => other,
    }
}

/// Checks an argument count against a definition: too few is `Err:511`,
/// too many is `Err:504`.
pub fn check_arity(def: &FunctionDef, n: usize) -> Result<(), ErrorKind> {
    if n < usize::from(def.min_args) {
        return Err(ErrorKind::MissingVariable2);
    }
    if !def.is_variadic() && n > usize::from(def.max_args) {
        return Err(ErrorKind::ParamList);
    }
    Ok(())
}

/// The context and arguments of one call, with coercing accessors.
///
/// Each argument is evaluated at most once (results are cached), so lazy
/// arguments can be read repeatedly. Indices past the end are missing.
pub struct Args<'a> {
    ctx: &'a mut dyn EvalContext,
    list: &'a mut dyn ArgList,
    cache: Vec<Option<EvalValue>>,
}

impl<'a> Args<'a> {
    /// Wraps a call's context and argument list.
    pub fn new(ctx: &'a mut dyn EvalContext, list: &'a mut dyn ArgList) -> Self {
        let n = list.len();
        Self {
            ctx,
            list,
            cache: vec![None; n],
        }
    }

    /// The evaluation context.
    pub fn ctx(&mut self) -> &mut dyn EvalContext {
        &mut *self.ctx
    }

    /// Number of arguments written in the call.
    pub fn count(&self) -> usize {
        self.cache.len()
    }

    /// `true` if argument `i` is omitted (empty or past the end).
    pub fn is_missing(&self, i: usize) -> bool {
        i >= self.cache.len() || self.list.is_missing(i)
    }

    /// Argument `i`, evaluated on first use ([`EvalValue::Missing`] if omitted).
    pub fn any(&mut self, i: usize) -> EvalValue {
        if self.is_missing(i) {
            return EvalValue::Missing;
        }
        if let Some(Some(v)) = self.cache.get(i) {
            return v.clone();
        }
        let v = self.list.eval(i, &mut *self.ctx);
        if let Some(slot) = self.cache.get_mut(i) {
            *slot = Some(v.clone());
        }
        v
    }

    /// Argument `i` reduced to one value (see [`coerce::scalar_value`]);
    /// errors are returned as values, not propagated.
    pub fn value(&mut self, i: usize) -> Value {
        let v = self.any(i);
        coerce::scalar_value(&v, &mut *self.ctx)
    }

    /// Argument `i` as a number (missing → 0).
    pub fn num(&mut self, i: usize) -> Result<f64, ErrorKind> {
        let v = self.any(i);
        coerce::to_number(&v, &mut *self.ctx)
    }

    /// Argument `i` as a number, or `default` if omitted.
    pub fn num_opt(&mut self, i: usize, default: f64) -> Result<f64, ErrorKind> {
        if self.is_missing(i) {
            Ok(default)
        } else {
            self.num(i)
        }
    }

    /// Argument `i` as an integer (see [`coerce::to_integer`]).
    pub fn int(&mut self, i: usize) -> Result<i64, ErrorKind> {
        let v = self.any(i);
        coerce::to_integer(&v, &mut *self.ctx)
    }

    /// Argument `i` as an integer, or `default` if omitted.
    pub fn int_opt(&mut self, i: usize, default: i64) -> Result<i64, ErrorKind> {
        if self.is_missing(i) {
            Ok(default)
        } else {
            self.int(i)
        }
    }

    /// Argument `i` as text (missing → `""`).
    pub fn text(&mut self, i: usize) -> Result<Arc<str>, ErrorKind> {
        let v = self.any(i);
        coerce::to_text(&v, &mut *self.ctx)
    }

    /// Argument `i` as text, or `default` if omitted.
    pub fn text_opt(&mut self, i: usize, default: &str) -> Result<Arc<str>, ErrorKind> {
        if self.is_missing(i) {
            Ok(default.into())
        } else {
            self.text(i)
        }
    }

    /// Argument `i` as a boolean (missing → `false`).
    pub fn bool(&mut self, i: usize) -> Result<bool, ErrorKind> {
        let v = self.any(i);
        coerce::to_bool(&v, &mut *self.ctx)
    }

    /// Argument `i` as a boolean, or `default` if omitted.
    pub fn bool_opt(&mut self, i: usize, default: bool) -> Result<bool, ErrorKind> {
        if self.is_missing(i) {
            Ok(default)
        } else {
            self.bool(i)
        }
    }

    /// Argument `i` as a reference. A scalar error propagates; anything
    /// else that is not a reference is `#VALUE!`.
    pub fn range(&mut self, i: usize) -> Result<CellRange, ErrorKind> {
        match self.any(i) {
            EvalValue::Range(r) => Ok(r),
            EvalValue::Scalar(Value::Error(e)) => Err(e),
            _ => Err(ErrorKind::Value),
        }
    }

    /// Argument `i` parsed as a criterion; an error value propagates.
    pub fn criterion(&mut self, i: usize) -> Result<Criterion, ErrorKind> {
        let v = self.value(i);
        if let Value::Error(e) = v {
            return Err(e);
        }
        Ok(Criterion::parse(&v, self.ctx.settings()))
    }

    /// SUM semantics over argument `i` (see [`coerce::number_seq`]).
    pub fn for_each_number(&mut self, i: usize, f: impl FnMut(f64)) -> Result<(), ErrorKind> {
        let v = self.any(i);
        coerce::number_seq(&v, &mut *self.ctx, f)
    }

    /// SUM semantics over every argument, in order.
    pub fn for_each_number_all(&mut self, mut f: impl FnMut(f64)) -> Result<(), ErrorKind> {
        for i in 0..self.count() {
            self.for_each_number(i, &mut f)?;
        }
        Ok(())
    }

    /// COUNTA-style visit of argument `i` (see [`coerce::values_seq`]).
    pub fn for_each_value(&mut self, i: usize, f: impl FnMut(&Value, SeqSource)) {
        let v = self.any(i);
        coerce::values_seq(&v, &mut *self.ctx, f);
    }
}
