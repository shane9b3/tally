//! Addin functions: complex family (Analysis add-in).

use crate::registry::Registry;

/// Defines a one-argument complex function: the `FunctionDef` constant and
/// its evaluator, from a closure-like operation on [`cplx::Cx`].
macro_rules! unary_complex {
    ($konst:ident, $name:literal, $help:literal, $op:path) => {
        pub const $konst: crate::registry::FunctionDef = crate::registry::FunctionDef::new(
            $name,
            crate::registry::Category::AddIn,
            super::cplx::UNARY_PARAMS,
            |ctx: &mut dyn crate::EvalContext, args: &mut dyn crate::ArgList| {
                super::cplx::unary(ctx, args, $op)
            },
        )
        .returns(crate::registry::ReturnKind::Text)
        .help($help);
    };
}

mod arith;
mod bessel;
mod cplx;
mod exp_log;
mod parts;
mod trig;

/// Registers the complex functions.
pub fn register(r: &mut Registry) {
    parts::register(r);
    arith::register(r);
    exp_log::register(r);
    trig::register(r);
    bessel::register(r);
}
