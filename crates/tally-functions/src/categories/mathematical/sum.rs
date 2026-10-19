//! SUM (later: SUMSQ, PRODUCT live in their own family files).

use tally_types::Value;

use crate::numeric::ApproxSum;
use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const SUM_PARAMS: &[Param] = &[Param::repeat("number", ParamKind::NumberSeq)];

/// `SUM(number; ...)`.
pub const SUM: FunctionDef = FunctionDef::new("SUM", Category::Mathematical, SUM_PARAMS, sum)
    .help("Returns the sum of all arguments.");

fn sum(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let mut acc = ApproxSum::new();
        a.for_each_number_all(|x| acc.add(x))?;
        Ok(Value::Number(acc.total()))
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(SUM);
}
