//! AVERAGE (later: AVERAGEA).

use tally_types::{ErrorKind, Value};

use crate::numeric::ApproxSum;
use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const AVERAGE_PARAMS: &[Param] = &[Param::repeat("number", ParamKind::NumberSeq)];

/// `AVERAGE(number; ...)`.
pub const AVERAGE: FunctionDef =
    FunctionDef::new("AVERAGE", Category::Statistical, AVERAGE_PARAMS, average)
        .help("Returns the average of a sample.");

fn average(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let mut acc = ApproxSum::new();
        a.for_each_number_all(|x| acc.add(x))?;
        if acc.count() == 0 {
            return Err(ErrorKind::Div0);
        }
        Ok(Value::Number(acc.total() / acc.count() as f64))
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(AVERAGE);
}
