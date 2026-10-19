//! SUMPRODUCT.

use tally_types::{ErrorKind, Value};

use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const SUMPRODUCT_PARAMS: &[Param] = &[Param::repeat("array", ParamKind::Array)];

/// `SUMPRODUCT(array; ...)`. At most 30 arrays (master list).
pub const SUMPRODUCT: FunctionDef =
    FunctionDef::new("SUMPRODUCT", Category::Array, SUMPRODUCT_PARAMS, sumproduct)
        .arity(1, 30)
        .help("(Inner products) Returns the sum of the products of array arguments.");

/// One argument as sparse numeric entries `(row, col, value)` plus its
/// declared dimensions. Everything not listed is 0.
struct Operand {
    rows: u32,
    cols: u32,
    entries: Vec<(u32, u32, f64)>,
}

fn element(v: &Value) -> Result<Option<f64>, ErrorKind> {
    match v {
        Value::Number(x) => Ok(Some(*x)),
        Value::Bool(b) => Ok(Some(if *b { 1.0 } else { 0.0 })),
        Value::Error(e) => Err(*e),
        // Text and empty elements count as 0 (AOO).
        _ => Ok(None),
    }
}

fn operand(ctx: &mut dyn EvalContext, arg: EvalValue) -> Result<Operand, ErrorKind> {
    match arg {
        EvalValue::Missing => Err(ErrorKind::Value),
        EvalValue::Scalar(Value::Array(m)) | EvalValue::Array(m) => {
            let mut entries = Vec::new();
            for c in 0..m.cols {
                for r in 0..m.rows {
                    if let Some(v) = m.get(r, c) {
                        if let Some(x) = element(v)? {
                            entries.push((r, c, x));
                        }
                    }
                }
            }
            Ok(Operand {
                rows: m.rows,
                cols: m.cols,
                entries,
            })
        }
        EvalValue::Scalar(v) => Ok(Operand {
            rows: 1,
            cols: 1,
            entries: element(&v)?.map(|x| (0, 0, x)).into_iter().collect(),
        }),
        EvalValue::Range(range) => {
            let mut entries = Vec::new();
            let mut failure = None;
            ctx.for_each_in_range(&range, true, &mut |addr, v| match element(v) {
                Ok(Some(x)) => {
                    entries.push((addr.row - range.r0, addr.col - range.c0, x));
                    std::ops::ControlFlow::Continue(())
                }
                Ok(None) => std::ops::ControlFlow::Continue(()),
                Err(e) => {
                    failure = Some(e);
                    std::ops::ControlFlow::Break(())
                }
            });
            if let Some(e) = failure {
                return Err(e);
            }
            Ok(Operand {
                rows: range.rows(),
                cols: range.cols(),
                entries,
            })
        }
    }
}

/// Elementwise product of all arguments, summed. All arguments must have the
/// same dimensions (`#VALUE!`). Work is proportional to the filled area, so
/// whole-column ranges are cheap.
fn sumproduct(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let mut ops = Vec::with_capacity(a.count());
        for i in 0..a.count() {
            let arg = a.any(i);
            ops.push(operand(a.ctx(), arg)?);
        }
        let Some(first) = ops.first() else {
            return Err(ErrorKind::MissingVariable2);
        };
        if ops
            .iter()
            .any(|o| o.rows != first.rows || o.cols != first.cols)
        {
            return Err(ErrorKind::Value);
        }
        // Dense over the bounding box of the filled cells.
        let rows = ops
            .iter()
            .flat_map(|o| o.entries.iter().map(|e| e.0 + 1))
            .max()
            .unwrap_or(0) as usize;
        let cols = ops
            .iter()
            .flat_map(|o| o.entries.iter().map(|e| e.1 + 1))
            .max()
            .unwrap_or(0) as usize;
        let mut product = vec![1.0f64; rows * cols];
        for o in &ops {
            let mut dense = vec![0.0f64; rows * cols];
            for &(r, c, x) in &o.entries {
                dense[r as usize * cols + c as usize] = x;
            }
            for (p, d) in product.iter_mut().zip(dense) {
                *p *= d;
            }
        }
        Ok(Value::Number(product.iter().sum()))
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(SUMPRODUCT);
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use std::sync::Arc;
    use tally_types::{ErrorKind, Matrix, Value};

    fn num(x: f64) -> Value {
        Value::Number(x)
    }

    fn arr(rows: u32, cols: u32, data: Vec<Value>) -> EvalValue {
        EvalValue::Array(Arc::new(
            Matrix::from_vec(rows, cols, data)
                .unwrap_or_else(|| Matrix::new_filled(rows, cols, Value::Empty)),
        ))
    }

    fn grid() -> MockEvalContext {
        let mut c = MockEvalContext::new();
        c.set(0, "A1", 1.0)
            .set(0, "A2", 2.0)
            .set(0, "A3", 3.0)
            .set(0, "B1", 4.0)
            .set(0, "B2", "x")
            .set(0, "B3", 6.0)
            .set(0, "C1", 7.0)
            .set(0, "D1", 8.0);
        c
    }

    #[test]
    fn two_ranges() {
        let mut c = grid();
        let (a, b) = (c.range(0, "A1:A3"), c.range(0, "B1:B3"));
        // text in B2 counts as 0: 1*4 + 2*0 + 3*6
        assert_eq!(call(&mut c, "SUMPRODUCT", vec![a.clone(), b]), num(22.0));
        assert_eq!(call(&mut c, "SUMPRODUCT", vec![a.clone()]), num(6.0));
        let row = c.range(0, "A1:C1");
        assert_eq!(
            call(&mut c, "SUMPRODUCT", vec![a.clone(), row]),
            Value::Error(ErrorKind::Value)
        );
        let d = c.range(0, "C1:C3");
        assert_eq!(call(&mut c, "SUMPRODUCT", vec![a, d]), num(7.0));
    }

    #[test]
    fn arrays_and_bools() {
        let mut c = grid();
        // The engine evaluates (A1:A3>1)*B1:B3 to an array like this.
        let mask = arr(3, 1, vec![num(0.0), num(2.0), num(3.0)]);
        assert_eq!(call(&mut c, "SUMPRODUCT", vec![mask]), num(5.0));
        let bools = arr(
            3,
            1,
            vec![Value::Bool(false), Value::Bool(true), Value::Bool(true)],
        );
        let b = c.range(0, "A1:A3");
        assert_eq!(call(&mut c, "SUMPRODUCT", vec![bools, b]), num(5.0));
        let withe = arr(2, 1, vec![num(1.0), Value::Error(ErrorKind::NA)]);
        assert_eq!(
            call(&mut c, "SUMPRODUCT", vec![withe]),
            Value::Error(ErrorKind::NA)
        );
        assert_eq!(
            call(
                &mut c,
                "SUMPRODUCT",
                vec![EvalValue::number(3.0), EvalValue::number(4.0)]
            ),
            num(12.0)
        );
    }

    #[test]
    fn whole_columns() {
        let mut c = grid();
        let (a, b) = (c.range(0, "A1:A1048576"), c.range(0, "B1:B1048576"));
        assert_eq!(call(&mut c, "SUMPRODUCT", vec![a, b]), num(22.0));
    }

    #[test]
    fn range_error_propagates() {
        let mut c = grid();
        c.set(0, "A2", Value::Error(ErrorKind::Div0));
        let (a, b) = (c.range(0, "A1:A3"), c.range(0, "B1:B3"));
        assert_eq!(
            call(&mut c, "SUMPRODUCT", vec![a, b]),
            Value::Error(ErrorKind::Div0)
        );
    }
}
