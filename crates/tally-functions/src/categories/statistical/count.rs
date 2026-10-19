//! COUNT (later: COUNTA, COUNTBLANK lives with the COUNTIF family).

use tally_types::Value;

use crate::coerce::{parse_unambiguous, SeqSource};
use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const COUNT_PARAMS: &[Param] = &[Param::repeat("value", ParamKind::ValueSeq)];

/// `COUNT(value; ...)`.
pub const COUNT: FunctionDef =
    FunctionDef::new("COUNT", Category::Statistical, COUNT_PARAMS, count)
        .help("Counts how many numbers are in the list of arguments.");

/// Counts numbers and booleans (AOO stores booleans as numbers) anywhere;
/// direct text counts when it is an unambiguous number or ISO date; a
/// missing direct argument counts (as 0); errors are never counted and
/// never propagate.
fn count(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    let epoch = a.ctx().settings().date_epoch;
    run(|| {
        let mut n = 0u64;
        for i in 0..a.count() {
            a.for_each_value(i, |v, src| match (v, src) {
                (Value::Number(_) | Value::Bool(_), _) => n += 1,
                (Value::Empty, SeqSource::Direct) => n += 1,
                (Value::Text(s), SeqSource::Direct) if parse_unambiguous(s, epoch).is_some() => {
                    n += 1
                }
                _ => {}
            });
        }
        Ok(Value::Number(n as f64))
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(COUNT);
}
