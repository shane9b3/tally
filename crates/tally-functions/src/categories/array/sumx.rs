//! SUMX2MY2, SUMX2PY2, SUMXMY2.
//!
//! After AOO `interpr5.cxx` (`ScSumX2MY2`, `CalculateSumX2MY2SumX2PY2SumXMY2`;
//! Apache-2.0): the two arrays must have the same dimensions (`#VALUE!`);
//! pairs where either element is text or empty are skipped.

use tally_types::{ErrorKind, Value};

use super::mat::values;
use crate::registry::{Category, FnFlags, FunctionDef, Param, ParamKind, Registry};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const PARAMS: &[Param] = &[
    Param::required("array_x", ParamKind::Array),
    Param::required("array_y", ParamKind::Array),
];

/// `SUMX2MY2(array_x; array_y)`.
pub const SUMX2MY2: FunctionDef = FunctionDef::new("SUMX2MY2", Category::Array, PARAMS, x2my2)
    .flags(FnFlags::FORCE_ARRAY)
    .help("Returns the sum of the difference of squares of corresponding values in two arrays.");
/// `SUMX2PY2(array_x; array_y)`.
pub const SUMX2PY2: FunctionDef = FunctionDef::new("SUMX2PY2", Category::Array, PARAMS, x2py2)
    .flags(FnFlags::FORCE_ARRAY)
    .help("Returns the sum of the sum of squares of corresponding values in two arrays.");
/// `SUMXMY2(array_x; array_y)`.
pub const SUMXMY2: FunctionDef = FunctionDef::new("SUMXMY2", Category::Array, PARAMS, xmy2)
    .flags(FnFlags::FORCE_ARRAY)
    .help("Returns the sum of squares of differences of corresponding values in two arrays.");

fn pairwise(
    ctx: &mut dyn EvalContext,
    args: &mut dyn ArgList,
    f: fn(f64, f64) -> f64,
) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let x = a.any(0);
        let x = values(a.ctx(), x)?;
        let y = a.any(1);
        let y = values(a.ctx(), y)?;
        if x.rows != y.rows || x.cols != y.cols {
            return Err(ErrorKind::Value);
        }
        let num = |v: &Value| -> Result<Option<f64>, ErrorKind> {
            match v {
                Value::Number(n) => Ok(Some(*n)),
                Value::Bool(b) => Ok(Some(f64::from(u8::from(*b)))),
                Value::Error(e) => Err(*e),
                _ => Ok(None),
            }
        };
        let mut sum = 0.0;
        // Column-major, so the first error found matches AOO.
        for c in 0..x.cols {
            for r in 0..x.rows {
                let (p, q) = (x.get(r, c), y.get(r, c));
                if let (Some(p), Some(q)) = (p, q) {
                    let (p, q) = (num(p)?, num(q)?);
                    if let (Some(p), Some(q)) = (p, q) {
                        sum += f(p, q);
                    }
                }
            }
        }
        Ok(Value::Number(sum))
    })
}

fn x2my2(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    pairwise(ctx, args, |x, y| x * x - y * y)
}
fn x2py2(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    pairwise(ctx, args, |x, y| x * x + y * y)
}
fn xmy2(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    pairwise(ctx, args, |x, y| (x - y) * (x - y))
}

pub(super) fn register(r: &mut Registry) {
    r.register(SUMX2MY2);
    r.register(SUMX2PY2);
    r.register(SUMXMY2);
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use tally_types::{ErrorKind, Value};

    #[test]
    fn sums() {
        let mut c = MockEvalContext::new();
        for (i, (x, y)) in [(2.0, 1.0), (3.0, 2.0), (4.0, 5.0)].iter().enumerate() {
            c.set(0, &format!("A{}", i + 1), *x);
            c.set(0, &format!("B{}", i + 1), *y);
        }
        let (a, b) = (c.range(0, "A1:A3"), c.range(0, "B1:B3"));
        let n = |c: &mut MockEvalContext, f: &str| call(c, f, vec![a.clone(), b.clone()]);
        assert_eq!(n(&mut c, "SUMX2MY2"), Value::Number(3.0 + 5.0 - 9.0));
        assert_eq!(n(&mut c, "SUMX2PY2"), Value::Number(5.0 + 13.0 + 41.0));
        assert_eq!(n(&mut c, "SUMXMY2"), Value::Number(1.0 + 1.0 + 1.0));
        let short = c.range(0, "B1:B2");
        assert_eq!(
            call(&mut c, "SUMXMY2", vec![a, short]),
            Value::Error(ErrorKind::Value)
        );
    }

    #[test]
    fn text_pairs_skipped() {
        let mut c = MockEvalContext::new();
        c.set(0, "A1", 3.0)
            .set(0, "A2", "x")
            .set(0, "B1", 1.0)
            .set(0, "B2", 9.0);
        let (a, b) = (c.range(0, "A1:A2"), c.range(0, "B1:B2"));
        assert_eq!(call(&mut c, "SUMX2MY2", vec![a, b]), Value::Number(8.0));
    }
}
