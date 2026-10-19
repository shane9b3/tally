//! MMULT, TRANSPOSE, MINVERSE, MDETERM, MUNIT.
//!
//! Semantics after Apache OpenOffice `sc/source/core/tool/interpr5.cxx`
//! (Apache-2.0). Text in a matrix operand is `#VALUE!`, empties are 0.

use std::sync::Arc;

use tally_types::{ErrorKind, Matrix, Value};

use super::mat::{dense, values, Dense};
use crate::registry::{Category, FnFlags, FunctionDef, Param, ParamKind, Registry, ReturnKind};
use crate::{run, run_ref, ArgList, Args, EvalContext, EvalValue};

const ONE: &[Param] = &[Param::required("array", ParamKind::Array)];
const TWO: &[Param] = &[
    Param::required("array", ParamKind::Array),
    Param::required("array", ParamKind::Array),
];
const MUNIT_PARAMS: &[Param] = &[Param::required("Dimensions", ParamKind::Integer)];

/// `MMULT(array; array)`.
pub const MMULT: FunctionDef = FunctionDef::new("MMULT", Category::Array, TWO, mmult)
    .returns(ReturnKind::Array)
    .flags(FnFlags::FORCE_ARRAY)
    .help("Returns the matrix product of two arrays.");

/// `TRANSPOSE(array)`.
pub const TRANSPOSE: FunctionDef = FunctionDef::new("TRANSPOSE", Category::Array, ONE, transpose)
    .returns(ReturnKind::Array)
    .flags(FnFlags::FORCE_ARRAY)
    .help("Transposes the rows and columns of an array.");

/// `MINVERSE(array)`.
pub const MINVERSE: FunctionDef = FunctionDef::new("MINVERSE", Category::Array, ONE, minverse)
    .returns(ReturnKind::Array)
    .flags(FnFlags::FORCE_ARRAY)
    .help("Returns the inverse matrix.");

/// `MDETERM(array)`.
pub const MDETERM: FunctionDef = FunctionDef::new("MDETERM", Category::Array, ONE, mdeterm)
    .flags(FnFlags::FORCE_ARRAY)
    .help("Returns the determinant of a square matrix.");

/// `MUNIT(dimensions)`.
pub const MUNIT: FunctionDef = FunctionDef::new("MUNIT", Category::Array, MUNIT_PARAMS, munit)
    .returns(ReturnKind::Array)
    .help("Returns the unitary square array of a certain size.");

/// Runs a body producing a matrix result.
fn run_matrix(body: impl FnOnce() -> Result<Dense, ErrorKind>) -> EvalValue {
    run_ref(|| body().map(Dense::into_eval))
}

fn mmult(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run_matrix(|| {
        let x = a.any(0);
        let x = dense(a.ctx(), x, ErrorKind::Value)?;
        let y = a.any(1);
        let y = dense(a.ctx(), y, ErrorKind::Value)?;
        if x.cols != y.rows {
            return Err(ErrorKind::Value);
        }
        let mut out = Dense::zeros(x.rows, y.cols);
        for i in 0..x.rows {
            for j in 0..y.cols {
                let mut s = 0.0;
                for k in 0..x.cols {
                    s += x.at(i, k) * y.at(k, j);
                }
                out.set(i, j, s);
            }
        }
        Ok(out)
    })
}

fn transpose(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run_ref(|| {
        let x = a.any(0);
        let m = values(a.ctx(), x)?;
        let mut out = Matrix::new_filled(m.cols, m.rows, Value::Number(0.0));
        for r in 0..m.rows {
            for c in 0..m.cols {
                let v = match m.get(r, c) {
                    Some(Value::Empty) | None => Value::Number(0.0),
                    Some(v) => v.clone(),
                };
                out.set(c, r, v);
            }
        }
        Ok(EvalValue::Array(Arc::new(out)))
    })
}

/// Square matrix operand.
fn square(ctx: &mut dyn EvalContext, arg: EvalValue) -> Result<Dense, ErrorKind> {
    let m = dense(ctx, arg, ErrorKind::Value)?;
    if m.rows != m.cols || m.rows == 0 {
        return Err(ErrorKind::Value);
    }
    Ok(m)
}

/// Gauss-Jordan inversion with partial pivoting. `None` if singular.
fn invert(m: &Dense) -> Option<Dense> {
    let n = m.rows;
    let mut a = m.clone();
    let mut inv = Dense::zeros(n, n);
    for i in 0..n {
        inv.set(i, i, 1.0);
    }
    let scale = m.d.iter().fold(0.0f64, |s, x| s.max(x.abs()));
    let tiny = (scale * f64::EPSILON).max(f64::MIN_POSITIVE);
    for col in 0..n {
        let mut p = col;
        for r in col + 1..n {
            if a.at(r, col).abs() > a.at(p, col).abs() {
                p = r;
            }
        }
        if a.at(p, col).abs() <= tiny {
            return None;
        }
        if p != col {
            for c in 0..n {
                a.d.swap(p * n + c, col * n + c);
                inv.d.swap(p * n + c, col * n + c);
            }
        }
        let piv = a.at(col, col);
        for c in 0..n {
            a.set(col, c, a.at(col, c) / piv);
            inv.set(col, c, inv.at(col, c) / piv);
        }
        for r in 0..n {
            if r == col {
                continue;
            }
            let f = a.at(r, col);
            if f != 0.0 {
                for c in 0..n {
                    a.set(r, c, a.at(r, c) - f * a.at(col, c));
                    inv.set(r, c, inv.at(r, c) - f * inv.at(col, c));
                }
            }
        }
    }
    Some(inv)
}

fn minverse(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run_matrix(|| {
        let x = a.any(0);
        let m = square(a.ctx(), x)?;
        // AOO reports a singular matrix as an illegal argument (LibreOffice and
        // Excel give #VALUE! / #NUM!).
        invert(&m).ok_or(ErrorKind::InvalidArgument)
    })
}

/// Determinant by LU decomposition with partial pivoting.
fn determinant(m: &Dense) -> f64 {
    let n = m.rows;
    let mut a = m.clone();
    let mut det = 1.0;
    for col in 0..n {
        let mut p = col;
        for r in col + 1..n {
            if a.at(r, col).abs() > a.at(p, col).abs() {
                p = r;
            }
        }
        if a.at(p, col) == 0.0 {
            return 0.0;
        }
        if p != col {
            for c in 0..n {
                a.d.swap(p * n + c, col * n + c);
            }
            det = -det;
        }
        let piv = a.at(col, col);
        det *= piv;
        for r in col + 1..n {
            let f = a.at(r, col) / piv;
            for c in col + 1..n {
                a.set(r, c, a.at(r, c) - f * a.at(col, c));
            }
        }
    }
    det
}

fn mdeterm(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let x = a.any(0);
        let m = square(a.ctx(), x)?;
        Ok(Value::Number(determinant(&m)))
    })
}

fn munit(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run_matrix(|| {
        let n = a.int(0)?;
        if n < 1 {
            return Err(ErrorKind::InvalidArgument);
        }
        if n > 1448 {
            return Err(ErrorKind::Value);
        }
        let n = n as usize;
        let mut m = Dense::zeros(n, n);
        for i in 0..n {
            m.set(i, i, 1.0);
        }
        Ok(m)
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(MMULT);
    r.register(TRANSPOSE);
    r.register(MINVERSE);
    r.register(MDETERM);
    r.register(MUNIT);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::{call, MockEvalContext};
    use tally_types::approx_equal;

    fn arr(rows: u32, cols: u32, data: &[f64]) -> EvalValue {
        let v = data.iter().map(|x| Value::Number(*x)).collect();
        EvalValue::Array(Arc::new(
            Matrix::from_vec(rows, cols, v)
                .unwrap_or_else(|| Matrix::new_filled(rows, cols, Value::Empty)),
        ))
    }

    fn nums(v: &Value) -> Vec<f64> {
        match v {
            Value::Array(m) => m.data.iter().filter_map(Value::as_number).collect(),
            other => vec![other.as_number().unwrap_or(f64::NAN)],
        }
    }

    fn close(v: &Value, want: &[f64]) {
        let got = nums(v);
        assert_eq!(got.len(), want.len(), "{v:?}");
        for (g, w) in got.iter().zip(want) {
            assert!((g - w).abs() < 1e-12, "{g} vs {w}");
        }
    }

    #[test]
    fn mmult() {
        let mut c = MockEvalContext::new();
        let a = arr(2, 2, &[1.0, 2.0, 3.0, 4.0]);
        let b = arr(2, 2, &[5.0, 6.0, 7.0, 8.0]);
        let r = call(&mut c, "MMULT", vec![a.clone(), b]);
        close(&r, &[19.0, 22.0, 43.0, 50.0]);
        let bad = arr(3, 1, &[1.0, 2.0, 3.0]);
        assert_eq!(
            call(&mut c, "MMULT", vec![a, bad]),
            Value::Error(ErrorKind::Value)
        );
    }

    #[test]
    fn transpose_shape() {
        let mut c = MockEvalContext::new();
        let r = call(
            &mut c,
            "TRANSPOSE",
            vec![arr(2, 3, &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0])],
        );
        match r {
            Value::Array(m) => {
                assert_eq!((m.rows, m.cols), (3, 2));
                close(&Value::Array(m), &[1.0, 4.0, 2.0, 5.0, 3.0, 6.0]);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn inverse_and_det() {
        let mut c = MockEvalContext::new();
        let r = call(&mut c, "MINVERSE", vec![arr(2, 2, &[4.0, 7.0, 2.0, 6.0])]);
        close(&r, &[0.6, -0.7, -0.2, 0.4]);
        let sing = arr(2, 2, &[1.0, 2.0, 2.0, 4.0]);
        assert_eq!(
            call(&mut c, "MINVERSE", vec![sing.clone()]),
            Value::Error(ErrorKind::InvalidArgument)
        );
        assert_eq!(call(&mut c, "MDETERM", vec![sing]), Value::Number(0.0));
        let d = call(&mut c, "MDETERM", vec![arr(2, 2, &[1.0, 2.0, 3.0, 4.0])]);
        assert!(approx_equal(d.as_number().unwrap_or(0.0), -2.0));
        let d3 = call(
            &mut c,
            "MDETERM",
            vec![arr(3, 3, &[2.0, 0.0, 1.0, 1.0, 3.0, 2.0, 1.0, 1.0, 1.0])],
        );
        assert!(
            (d3.as_number().unwrap_or(0.0) - (-0.0 + 2.0 * (3.0 - 2.0) + 1.0 * (1.0 - 3.0))).abs()
                < 1e-12
        );
        assert_eq!(
            call(&mut c, "MDETERM", vec![arr(1, 2, &[1.0, 2.0])]),
            Value::Error(ErrorKind::Value)
        );
    }

    #[test]
    fn munit() {
        let mut c = MockEvalContext::new();
        close(
            &call(&mut c, "MUNIT", vec![EvalValue::number(2.0)]),
            &[1.0, 0.0, 0.0, 1.0],
        );
        assert_eq!(
            call(&mut c, "MUNIT", vec![EvalValue::number(0.0)]),
            Value::Error(ErrorKind::InvalidArgument)
        );
    }
}
