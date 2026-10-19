//! RANDBETWEEN (Analysis add-in).

use tally_types::{ErrorKind, Value};

use crate::coerce::approx_floor;
use crate::registry::{Category, FnFlags, FunctionDef, Param, ParamKind, Registry};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const PARAMS: &[Param] = &[
    Param::required("bottom", ParamKind::Number),
    Param::required("top", ParamKind::Number),
];

/// `RANDBETWEEN(bottom; top)`: volatile.
pub const RANDBETWEEN: FunctionDef =
    FunctionDef::new("RANDBETWEEN", Category::AddIn, PARAMS, randbetween)
        .flags(FnFlags::VOLATILE)
        .help("Returns a random integer between the numbers you specify");

/// A uniform integer in `[ceil(bottom), floor(top)]`; `bottom > top`, or no
/// integer in the interval, is `Err:502`.
fn randbetween(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let bottom = a.num(0)?;
        let top = a.num(1)?;
        if bottom > top {
            return Err(ErrorKind::InvalidArgument);
        }
        let lo = -approx_floor(-bottom);
        let hi = approx_floor(top);
        if lo > hi {
            return Err(ErrorKind::InvalidArgument);
        }
        let r = a.ctx().rand();
        let x = (lo + (r * (hi - lo + 1.0)).floor()).min(hi);
        Ok(Value::Number(x))
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(RANDBETWEEN);
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{ErrorKind, Value};

    fn n(x: f64) -> EvalValue {
        EvalValue::number(x)
    }

    #[test]
    fn range_and_errors() {
        let mut c = MockEvalContext::new();
        let mut seen = [false; 5];
        for _ in 0..300 {
            match call(&mut c, "RANDBETWEEN", vec![n(-2.0), n(2.0)]) {
                Value::Number(x) => {
                    assert_eq!(x, x.floor());
                    assert!((-2.0..=2.0).contains(&x));
                    seen[(x + 2.0) as usize] = true;
                }
                other => panic!("{other:?}"),
            }
        }
        assert!(seen.iter().all(|s| *s), "every value should occur");
        assert_eq!(
            call(&mut c, "RANDBETWEEN", vec![n(3.0), n(3.0)]),
            Value::Number(3.0)
        );
        assert_eq!(
            call(&mut c, "RANDBETWEEN", vec![n(5.0), n(1.0)]),
            Value::Error(ErrorKind::InvalidArgument)
        );
        assert_eq!(
            call(&mut c, "RANDBETWEEN", vec![n(1.2), n(1.8)]),
            Value::Error(ErrorKind::InvalidArgument)
        );
        assert_eq!(
            call(&mut c, "RANDBETWEEN", vec![n(0.5), n(1.5)]),
            Value::Number(1.0)
        );
        assert_eq!(
            call(&mut c, "RANDBETWEEN", vec![EvalValue::text("x"), n(1.0)]),
            Value::Error(ErrorKind::Value)
        );
    }
}
