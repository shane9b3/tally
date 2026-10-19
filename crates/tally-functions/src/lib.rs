//! Function library for Tally (WP3).
//!
//! - [`registry()`]: the global [`Registry`] of [`FunctionDef`]s, looked up
//!   by canonical, ODF or OOXML name and addressed by [`FuncId`];
//! - [`EvalContext`] / [`ArgList`]: the traits the engine implements;
//! - [`eval_call`]: arity check, argument preparation, implicit
//!   intersection and array lifting, then the function;
//! - [`coerce`] and [`criteria`]: the only places coercion and criteria
//!   rules live;
//! - [`Args`] / [`run`]: helpers for function authors.
//!
//! Functions never depend on the engine. See `docs/architecture-plan.md`
//! §3.7 and `docs/contributing-functions.md`.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::sync::OnceLock;

mod args;
mod categories;
pub mod coerce;
mod context;
pub mod criteria;
mod eval;
#[cfg(any(test, feature = "test-support"))]
pub mod mock;
pub mod numeric;
mod registry;

pub use args::{check_arity, run, run_ref, sanitize, Args};
pub use context::{ArgList, CellFormatInfo, EvalContext, EvalValue};
pub use criteria::{CmpOp, Criterion};
pub use eval::call_def;
pub use registry::{
    Category, EvalFn, FnFlags, FuncId, FunctionDef, NameConflict, Param, ParamKind, Registry,
    ReturnKind, VARIADIC,
};

/// Builds a registry containing every built-in function.
pub fn build_registry() -> Registry {
    let mut r = Registry::new();
    categories::register_all(&mut r);
    r
}

/// The global registry, built on first use.
pub fn registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(build_registry)
}

/// Calls function `func` from the global registry; see [`call_def`] for
/// what happens before the implementation runs.
pub fn eval_call(ctx: &mut dyn EvalContext, func: FuncId, args: &mut dyn ArgList) -> EvalValue {
    registry().eval_call(ctx, func, args)
}
