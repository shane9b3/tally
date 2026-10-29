//! Shared helpers for the matrix functions: argument to matrix conversion
//! and a small dense row-major `f64` matrix.

use std::sync::Arc;

use tally_types::{ErrorKind, Matrix, Value};

use crate::{EvalContext, EvalValue};

/// Upper bound on the cells read from one range argument.
const MAX_CELLS: u64 = 1 << 22;

/// A dense row-major matrix of numbers.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Dense {
    pub rows: usize,
    pub cols: usize,
    pub d: Vec<f64>,
}

impl Dense {
    pub fn zeros(rows: usize, cols: usize) -> Self {
        Self {
            rows,
            cols,
            d: vec![0.0; rows * cols],
        }
    }

    pub fn at(&self, r: usize, c: usize) -> f64 {
        self.d[r * self.cols + c]
    }

    pub fn set(&mut self, r: usize, c: usize, v: f64) {
        self.d[r * self.cols + c] = v;
    }

    /// The matrix as an array result.
    pub fn into_eval(self) -> EvalValue {
        let data = self.d.into_iter().map(Value::Number).collect();
        match Matrix::from_vec(self.rows as u32, self.cols as u32, data) {
            Some(m) => EvalValue::Array(Arc::new(m)),
            None => EvalValue::error(ErrorKind::Value),
        }
    }
}

/// Argument as a matrix of values (ranges are read with empties kept as
/// `Value::Empty`). A scalar error propagates; an omitted argument or a
/// reference union (no rectangular shape) is `#VALUE!`.
pub(super) fn values(ctx: &mut dyn EvalContext, arg: EvalValue) -> Result<Arc<Matrix>, ErrorKind> {
    match arg {
        EvalValue::Missing => Err(ErrorKind::Value),
        EvalValue::Array(m) | EvalValue::Scalar(Value::Array(m)) => Ok(m),
        EvalValue::Scalar(Value::Error(e)) => Err(e),
        EvalValue::Scalar(v) => Ok(Arc::new(Matrix::new_filled(1, 1, v))),
        EvalValue::Range(r) => {
            if r.cell_count() > MAX_CELLS {
                return Err(ErrorKind::Value);
            }
            let mut m = Matrix::new_filled(r.rows(), r.cols(), Value::Empty);
            let (r0, c0) = (r.r0, r.c0);
            ctx.for_each_in_range(&r, true, &mut |addr, v| {
                m.set(addr.row - r0, addr.col - c0, v.clone());
                std::ops::ControlFlow::Continue(())
            });
            Ok(Arc::new(m))
        }
        // AOO `GetMatrix` → `CreateMatrixFromDoubleRef`: not a 2D matrix.
        #[allow(unreachable_patterns)]
        v if v.is_3d() => Err(ErrorKind::ParamList),
        #[allow(unreachable_patterns)]
        _ => Err(ErrorKind::Value),
    }
}

/// Argument as a numeric matrix: numbers and booleans as is, empties 0,
/// text `text_err`, errors propagate.
pub(super) fn dense(
    ctx: &mut dyn EvalContext,
    arg: EvalValue,
    text_err: ErrorKind,
) -> Result<Dense, ErrorKind> {
    let m = values(ctx, arg)?;
    let mut d = Vec::with_capacity(m.data.len());
    for v in &m.data {
        d.push(match v {
            Value::Number(x) => *x,
            Value::Bool(b) => f64::from(u8::from(*b)),
            Value::Empty => 0.0,
            Value::Error(e) => return Err(*e),
            _ => return Err(text_err),
        });
    }
    Ok(Dense {
        rows: m.rows as usize,
        cols: m.cols as usize,
        d,
    })
}
