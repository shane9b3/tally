//! RAND.

use tally_types::Value;

use crate::registry::{Category, FnFlags, FunctionDef, Registry};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

/// `RAND()`: volatile.
pub const RAND: FunctionDef = FunctionDef::new("RAND", Category::Mathematical, &[], rand)
    .flags(FnFlags::VOLATILE)
    .help("Returns a random number between 0 and 1.");

fn rand(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| Ok(Value::Number(a.ctx().rand())))
}

pub(super) fn register(r: &mut Registry) {
    r.register(RAND);
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use crate::{registry, EvalValue, FnFlags};
    use tally_types::{ErrorKind, Value};

    #[test]
    fn rand_in_unit_interval_and_volatile() {
        let mut c = MockEvalContext::new();
        let mut seen = Vec::new();
        for _ in 0..50 {
            match call(&mut c, "RAND", vec![]) {
                Value::Number(x) => {
                    assert!((0.0..1.0).contains(&x));
                    seen.push(x);
                }
                other => panic!("not a number: {other:?}"),
            }
        }
        assert!(seen.windows(2).any(|w| w[0] != w[1]));
        let reg = registry();
        let def = reg.def_by_name("RAND");
        assert!(def.is_some_and(|d| d.flags.contains(FnFlags::VOLATILE)));
        assert_eq!(
            call(&mut c, "RAND", vec![EvalValue::number(1.0)]),
            Value::Error(ErrorKind::ParamList)
        );
    }
}
