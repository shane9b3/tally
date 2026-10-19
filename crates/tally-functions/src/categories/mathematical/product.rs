//! PRODUCT.

use tally_types::Value;

use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const PRODUCT_PARAMS: &[Param] = &[Param::repeat("number", ParamKind::NumberSeq)];

/// `PRODUCT(number; ...)`.
pub const PRODUCT: FunctionDef =
    FunctionDef::new("PRODUCT", Category::Mathematical, PRODUCT_PARAMS, product)
        .help("Multiplies the arguments.");

/// Multiplies every number (SUM visiting rules). With no numbers at all the
/// result is 0, as in AOO (`ScInterpreter::ScProduct`).
fn product(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let mut p = 1.0;
        let mut count = 0u64;
        a.for_each_number_all(|x| {
            p *= x;
            count += 1;
        })?;
        Ok(Value::Number(if count == 0 { 0.0 } else { p }))
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(PRODUCT);
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{ErrorKind, Value};

    #[test]
    fn product() {
        let mut c = MockEvalContext::new();
        c.set(0, "A1", 2.0)
            .set(0, "A2", "x")
            .set(0, "A3", true)
            .set(0, "A4", 4.0)
            .set(0, "C1", "t")
            .set(0, "D1", Value::Error(ErrorKind::Div0));
        let r = |c: &MockEvalContext, s: &str| c.range(0, s);
        let a = r(&c, "A1:A4");
        assert_eq!(call(&mut c, "PRODUCT", vec![a]), Value::Number(8.0));
        let a = r(&c, "C1:C3");
        assert_eq!(call(&mut c, "PRODUCT", vec![a]), Value::Number(0.0));
        let a = r(&c, "B1:B3");
        assert_eq!(call(&mut c, "PRODUCT", vec![a]), Value::Number(0.0));
        let a = r(&c, "D1");
        assert_eq!(
            call(&mut c, "PRODUCT", vec![a]),
            Value::Error(ErrorKind::Div0)
        );
        let a = r(&c, "A1:A4");
        assert_eq!(
            call(&mut c, "PRODUCT", vec![a, EvalValue::number(0.5)]),
            Value::Number(4.0)
        );
        assert_eq!(
            call(
                &mut c,
                "PRODUCT",
                vec![EvalValue::text("3"), EvalValue::bool(true)]
            ),
            Value::Number(3.0)
        );
        assert_eq!(
            call(&mut c, "PRODUCT", vec![EvalValue::text("x")]),
            Value::Error(ErrorKind::Value)
        );
        assert_eq!(
            call(&mut c, "PRODUCT", vec![]),
            Value::Error(ErrorKind::MissingVariable2)
        );
    }
}
