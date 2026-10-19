//! TODAY and NOW.

use tally_types::Value;

use crate::registry::{Category, FnFlags, FunctionDef, Registry, ReturnKind};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

/// `TODAY()`: the current date (integer serial).
pub const TODAY: FunctionDef = FunctionDef::new("TODAY", Category::DateTime, &[], today)
    .returns(ReturnKind::Number)
    .flags(FnFlags::VOLATILE)
    .help("Current date.");
/// `NOW()`: the current date and time.
pub const NOW: FunctionDef = FunctionDef::new("NOW", Category::DateTime, &[], now)
    .returns(ReturnKind::Number)
    .flags(FnFlags::VOLATILE)
    .help("Current date and time.");

fn today(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| Ok(Value::Number(a.ctx().now_serial().floor())))
}

fn now(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| Ok(Value::Number(a.ctx().now_serial())))
}

pub(super) fn register(r: &mut Registry) {
    r.register(TODAY);
    r.register(NOW);
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use crate::{registry, EvalValue, FnFlags};
    use tally_types::{ErrorKind, Value};

    #[test]
    fn today_now() {
        let mut c = MockEvalContext::new();
        c.now = 46314.75;
        assert_eq!(call(&mut c, "NOW", vec![]), Value::Number(46314.75));
        assert_eq!(call(&mut c, "TODAY", vec![]), Value::Number(46314.0));
        assert_eq!(
            call(&mut c, "NOW", vec![EvalValue::number(1.0)]),
            Value::Error(ErrorKind::ParamList)
        );
        let reg = registry();
        for name in ["NOW", "TODAY"] {
            let def = reg.def_by_name(name);
            assert!(def.is_some_and(|d| d.flags.contains(FnFlags::VOLATILE)));
        }
    }
}
