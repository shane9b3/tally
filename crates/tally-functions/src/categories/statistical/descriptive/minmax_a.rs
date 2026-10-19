//! MAXA and MINA.

use tally_types::Value;

use super::util::seq_numbers;
use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const PARAMS: &[Param] = &[Param::repeat("value", ParamKind::ValueSeq)];

/// `MAXA(value; ...)`.
pub const MAXA: FunctionDef = FunctionDef::new("MAXA", Category::Statistical, PARAMS, maxa)
    .help("Returns the maximum value in a list, text counted as 0.");
/// `MINA(value; ...)`.
pub const MINA: FunctionDef = FunctionDef::new("MINA", Category::Statistical, PARAMS, mina)
    .help("Returns the minimum value in a list, text counted as 0.");

fn extreme(ctx: &mut dyn EvalContext, args: &mut dyn ArgList, want_max: bool) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let v = seq_numbers(&mut a, true)?;
        let best = v
            .iter()
            .copied()
            .reduce(|b, x| if want_max { b.max(x) } else { b.min(x) });
        // AOO: no values at all gives 0.
        Ok(Value::Number(best.unwrap_or(0.0)))
    })
}

fn maxa(c: &mut dyn EvalContext, a: &mut dyn ArgList) -> EvalValue {
    extreme(c, a, true)
}
fn mina(c: &mut dyn EvalContext, a: &mut dyn ArgList) -> EvalValue {
    extreme(c, a, false)
}

pub(super) fn register(r: &mut Registry) {
    r.register(MAXA);
    r.register(MINA);
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{ErrorKind, Value};

    #[test]
    fn maxa_mina() {
        let mut c = MockEvalContext::new();
        c.set(0, "A1", -3.0)
            .set(0, "A2", "x")
            .set(0, "A3", true)
            .set(0, "B1", Value::Error(ErrorKind::NA));
        let r = c.range(0, "A1:A3");
        assert_eq!(call(&mut c, "MAXA", vec![r.clone()]), Value::Number(1.0));
        assert_eq!(call(&mut c, "MINA", vec![r]), Value::Number(-3.0));
        let r = c.range(0, "A2");
        assert_eq!(
            call(&mut c, "MINA", vec![r, EvalValue::number(4.0)]),
            Value::Number(0.0)
        );
        let r = c.range(0, "B1");
        assert_eq!(call(&mut c, "MAXA", vec![r]), Value::Error(ErrorKind::NA));
        let r = c.range(0, "C1:C2");
        assert_eq!(call(&mut c, "MAXA", vec![r]), Value::Number(0.0));
    }
}
