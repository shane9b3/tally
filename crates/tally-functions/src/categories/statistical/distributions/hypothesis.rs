//! CHITEST, FTEST, TTEST, ZTEST.
//!
//! Algorithms ported from Apache OpenOffice (interpr3.cxx), Apache License 2.0.

use std::collections::BTreeMap;

use tally_types::{ErrorKind, Value};

use super::floor_num;
use super::special::{chi_dist, f_dist, gauss, t_dist};
use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry};
use crate::stats_util::SparseData;
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const CHITEST_PARAMS: &[Param] = &[
    Param::required("data_b", ParamKind::Array),
    Param::required("data_e", ParamKind::Array),
];

/// `CHITEST(data_b; data_e)`.
pub const CHITEST: FunctionDef =
    FunctionDef::new("CHITEST", Category::Statistical, CHITEST_PARAMS, chitest)
        .odf("LEGACY.CHITEST")
        .help("Returns the chi square independence test.");

const FTEST_PARAMS: &[Param] = &[
    Param::required("data_1", ParamKind::Array),
    Param::required("data_2", ParamKind::Array),
];

/// `FTEST(data_1; data_2)`.
pub const FTEST: FunctionDef =
    FunctionDef::new("FTEST", Category::Statistical, FTEST_PARAMS, ftest)
        .help("Calculates the F test.");

const TTEST_PARAMS: &[Param] = &[
    Param::required("data_1", ParamKind::Array),
    Param::required("data_2", ParamKind::Array),
    Param::required("mode", ParamKind::Integer),
    Param::required("type", ParamKind::Integer),
];

/// `TTEST(data_1; data_2; mode; type)`.
pub const TTEST: FunctionDef =
    FunctionDef::new("TTEST", Category::Statistical, TTEST_PARAMS, ttest)
        .help("Calculates the t-Test.");

const ZTEST_PARAMS: &[Param] = &[
    Param::required("data", ParamKind::Reference),
    Param::required("mu", ParamKind::Number),
    Param::optional("sigma", ParamKind::Number),
];

/// `ZTEST(data; mu; [sigma])`.
pub const ZTEST: FunctionDef =
    FunctionDef::new("ZTEST", Category::Statistical, ZTEST_PARAMS, ztest)
        .help("Calculates the z-test with standard normal distribution.");

/// A non-string matrix element: a number (booleans included) or an error.
/// Text and empty elements are AOO "strings" and are not stored.
#[derive(Copy, Clone, Debug)]
enum Elem {
    Num(f64),
    Err(ErrorKind),
}

fn elem(v: &Value) -> Option<Elem> {
    match v {
        Value::Number(x) => Some(Elem::Num(*x)),
        Value::Bool(b) => Some(Elem::Num(if *b { 1.0 } else { 0.0 })),
        Value::Error(e) => Some(Elem::Err(*e)),
        _ => None,
    }
}

/// An argument as AOO `GetMatrix` sees it, stored sparsely by
/// column-major position (`col * rows + row`).
struct Mat {
    rows: u64,
    cols: u64,
    cells: BTreeMap<u64, Elem>,
}

impl Mat {
    fn get(&self, i: u64) -> Option<Elem> {
        self.cells.get(&i).copied()
    }

    /// Numbers in column-major order; the first error element is returned
    /// as the error (AOO keeps the first error it reads).
    fn numbers(&self) -> Result<Vec<f64>, ErrorKind> {
        self.cells
            .values()
            .map(|e| match e {
                Elem::Num(x) => Ok(*x),
                Elem::Err(e) => Err(*e),
            })
            .collect()
    }
}

/// AOO `GetMatrix`: a reference or array as is; a scalar is a 1×1 matrix
/// (a missing argument reads as 0, text is a string element).
fn matrix(a: &mut Args<'_>, i: usize) -> Mat {
    let arg = a.any(i);
    let d = if arg.is_missing() {
        SparseData::scalar(Value::Number(0.0))
    } else {
        SparseData::read(a.ctx(), arg)
    };
    let cells = d
        .items
        .iter()
        .filter_map(|(k, v)| elem(v).map(|e| (*k as u64, e)))
        .collect();
    Mat {
        rows: d.rows as u64,
        cols: d.cols as u64,
        cells,
    }
}

/// Count, sum and sum of squares of a sample.
fn moments(v: &[f64]) -> (f64, f64, f64) {
    let mut sum = 0.0;
    let mut sum_sq = 0.0;
    for &x in v {
        sum += x;
        sum_sq += x * x;
    }
    (v.len() as f64, sum, sum_sq)
}

/// AOO `ScChiTest`: both matrices must have the same dimensions (`Err:502`);
/// every position must hold a number in both (`Err:502` otherwise). A
/// single row or column has `n - 1` degrees of freedom (`#VALUE!` for one
/// cell), a table `(rows - 1)(cols - 1)`.
fn chitest(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let m1 = matrix(&mut a, 0);
        let m2 = matrix(&mut a, 1);
        if m1.rows != m2.rows || m1.cols != m2.cols {
            return Err(ErrorKind::InvalidArgument);
        }
        let mut first_err = None;
        let mut chi = 0.0;
        for i in 0..m1.rows * m1.cols {
            let (Some(x), Some(e)) = (m1.get(i), m2.get(i)) else {
                return Err(first_err.unwrap_or(ErrorKind::InvalidArgument));
            };
            match (x, e) {
                (Elem::Num(x), Elem::Num(e)) => chi += (x - e) * (x - e) / e,
                (Elem::Err(err), _) | (_, Elem::Err(err)) => {
                    first_err.get_or_insert(err);
                }
            }
        }
        if let Some(e) = first_err {
            return Err(e);
        }
        let df = if m1.cols == 1 || m1.rows == 1 {
            let df = (m1.cols * m1.rows - 1) as f64;
            if df == 0.0 {
                return Err(ErrorKind::Value);
            }
            df
        } else {
            (m1.cols - 1) as f64 * (m1.rows - 1) as f64
        };
        Ok(Value::Number(chi_dist(chi, df)?))
    })
}

/// AOO `ScFTest`: two-tailed probability that the variances differ.
/// Fewer than two numbers in either sample, or a zero variance, is `#VALUE!`.
fn ftest(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let m1 = matrix(&mut a, 0);
        let m2 = matrix(&mut a, 1);
        let (c1, s1, q1) = moments(&m1.numbers()?);
        let (c2, s2, q2) = moments(&m2.numbers()?);
        if c1 < 2.0 || c2 < 2.0 {
            return Err(ErrorKind::Value);
        }
        let v1 = (q1 - s1 * s1 / c1) / (c1 - 1.0);
        let v2 = (q2 - s2 * s2 / c2) / (c2 - 1.0);
        if v1 == 0.0 || v2 == 0.0 {
            return Err(ErrorKind::Value);
        }
        let (f, f1, f2) = if v1 > v2 {
            (v1 / v2, c1 - 1.0, c2 - 1.0)
        } else {
            (v2 / v1, c2 - 1.0, c1 - 1.0)
        };
        Ok(Value::Number(2.0 * f_dist(f, f1, f2)))
    })
}

/// AOO `CalculateTest`: the t statistic and degrees of freedom of the
/// homoscedastic (`welch = false`) or Welch (`welch = true`) two-sample test.
fn two_sample(m1: &Mat, m2: &Mat, welch: bool) -> Result<(f64, f64), ErrorKind> {
    let (c1, s1, q1) = moments(&m1.numbers()?);
    let (c2, s2, q2) = moments(&m2.numbers()?);
    if c1 < 2.0 || c2 < 2.0 {
        return Err(ErrorKind::Value);
    }
    if welch {
        let v1 = (q1 - s1 * s1 / c1) / (c1 - 1.0) / c1;
        let v2 = (q2 - s2 * s2 / c2) / (c2 - 1.0) / c2;
        if v1 + v2 == 0.0 {
            return Err(ErrorKind::Value);
        }
        let t = (s1 / c1 - s2 / c2).abs() / (v1 + v2).sqrt();
        let c = v1 / (v1 + v2);
        // Non-integral degrees of freedom are fine for GetTDist (#52406#).
        let f = 1.0 / (c * c / (c1 - 1.0) + (1.0 - c) * (1.0 - c) / (c2 - 1.0));
        Ok((t, f))
    } else {
        let v1 = (q1 - s1 * s1 / c1) / (c1 - 1.0);
        let v2 = (q2 - s2 * s2 / c2) / (c2 - 1.0);
        let t = (s1 / c1 - s2 / c2).abs() / ((c1 - 1.0) * v1 + (c2 - 1.0) * v2).sqrt()
            * (c1 * c2 * (c1 + c2 - 2.0) / (c1 + c2)).sqrt();
        Ok((t, c1 + c2 - 2.0))
    }
}

/// AOO `ScTTest`: `mode` (tails) and `type` are floored; tails other than
/// 1 or 2 and types other than 1 (paired), 2 (equal variance) or 3
/// (Welch) are `Err:502`. Paired data must have equal dimensions
/// (`Err:502`); only positions numeric in both count.
fn ttest(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let m1 = matrix(&mut a, 0);
        let m2 = matrix(&mut a, 1);
        let tails = floor_num(&mut a, 2)?;
        let kind = floor_num(&mut a, 3)?;
        if tails != 1.0 && tails != 2.0 {
            return Err(ErrorKind::InvalidArgument);
        }
        let (t, f) = if kind == 1.0 {
            if m1.rows != m2.rows || m1.cols != m2.cols {
                return Err(ErrorKind::InvalidArgument);
            }
            let mut first_err = None;
            let (mut count, mut sum1, mut sum2, mut sum_sq_d) = (0.0f64, 0.0, 0.0, 0.0);
            for (&i, &x) in &m1.cells {
                let Some(y) = m2.get(i) else { continue };
                match (x, y) {
                    (Elem::Num(x), Elem::Num(y)) => {
                        sum1 += x;
                        sum2 += y;
                        sum_sq_d += (x - y) * (x - y);
                        count += 1.0;
                    }
                    (Elem::Err(e), _) | (_, Elem::Err(e)) => {
                        first_err.get_or_insert(e);
                        count += 1.0;
                    }
                }
            }
            if let Some(e) = first_err {
                return Err(e);
            }
            if count < 1.0 {
                return Err(ErrorKind::Value);
            }
            let t = (count - 1.0).sqrt() * (sum1 - sum2).abs()
                / (count * sum_sq_d - (sum1 - sum2) * (sum1 - sum2)).sqrt();
            (t, count - 1.0)
        } else if kind == 2.0 {
            two_sample(&m1, &m2, false)?
        } else if kind == 3.0 {
            two_sample(&m1, &m2, true)?
        } else {
            return Err(ErrorKind::InvalidArgument);
        };
        let r = t_dist(t, f);
        Ok(Value::Number(if tails == 1.0 { r } else { 2.0 * r }))
    })
}

/// AOO `ScZTest`: one-tailed probability `1 - Φ(z)` with `z` from the
/// sample mean and either the given `sigma` (`<= 0` is `Err:502`) or the
/// sample standard deviation. Numbers and booleans count, text and empty
/// cells are skipped; a text scalar is `Err:504`; fewer than two values
/// is `#DIV/0!`.
fn ztest(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let sigma = if a.count() >= 3 {
            let s = a.num(2)?;
            if s <= 0.0 {
                return Err(ErrorKind::InvalidArgument);
            }
            Some(s)
        } else {
            None
        };
        let mu = a.num(1)?;
        let values = match a.any(0) {
            EvalValue::Scalar(Value::Text(_)) | EvalValue::Missing => {
                return Err(ErrorKind::ParamList)
            }
            EvalValue::Scalar(Value::Empty) => Vec::new(),
            _ => matrix(&mut a, 0).numbers()?,
        };
        let (count, sum, sum_sq) = moments(&values);
        if count <= 1.0 {
            return Err(ErrorKind::Div0);
        }
        let mean = sum / count;
        Ok(Value::Number(match sigma {
            Some(s) => 0.5 - gauss((mean - mu) * count.sqrt() / s),
            None => {
                let var = (sum_sq - sum * sum / count) / (count - 1.0);
                0.5 - gauss((mean - mu) / (var / count).sqrt())
            }
        }))
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(CHITEST);
    r.register(FTEST);
    r.register(TTEST);
    r.register(ZTEST);
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{ErrorKind, Value};

    fn n(x: f64) -> EvalValue {
        EvalValue::number(x)
    }

    fn approx(v: Value, want: f64, rel: f64) {
        match v {
            Value::Number(x) => assert!(
                (x - want).abs() <= rel * want.abs(),
                "got {x:e}, want {want:e}"
            ),
            other => panic!("expected {want}, got {other:?}"),
        }
    }

    fn sheet() -> MockEvalContext {
        let mut c = MockEvalContext::new();
        for (i, (x, y)) in [(3.0, 5.0), (4.0, 7.0), (5.0, 6.0), (8.0, 9.0), (9.0, 13.0)]
            .iter()
            .enumerate()
        {
            c.set(0, &format!("A{}", i + 1), *x);
            c.set(0, &format!("B{}", i + 1), *y);
        }
        c
    }

    #[test]
    fn t_and_f() {
        let mut c = sheet();
        let (a, b) = (c.range(0, "A1:A5"), c.range(0, "B1:B5"));
        approx(
            call(&mut c, "TTEST", vec![a.clone(), b.clone(), n(2.0), n(1.0)]),
            0.019_554_212_720_304_595,
            1e-12,
        );
        approx(
            call(&mut c, "TTEST", vec![a.clone(), b.clone(), n(2.0), n(2.0)]),
            0.263_079_633_116_613_5,
            1e-12,
        );
        approx(
            call(&mut c, "TTEST", vec![a.clone(), b.clone(), n(1.0), n(3.0)]),
            0.132_186_792_158_304_03,
            1e-12,
        );
        assert_eq!(
            call(&mut c, "TTEST", vec![a.clone(), b.clone(), n(3.0), n(1.0)]),
            Value::Error(ErrorKind::InvalidArgument)
        );
        approx(
            call(&mut c, "FTEST", vec![a.clone(), b.clone()]),
            0.707_450_816_034_394_7,
            1e-12,
        );
        let one = c.range(0, "A1");
        assert_eq!(
            call(&mut c, "FTEST", vec![one, b]),
            Value::Error(ErrorKind::Value)
        );
    }

    #[test]
    fn chi_and_z() {
        let mut c = sheet();
        let (a, b) = (c.range(0, "A1:A5"), c.range(0, "B1:B5"));
        approx(
            call(&mut c, "CHITEST", vec![a.clone(), b.clone()]),
            0.463_691_172_774_899_13,
            1e-12,
        );
        let short = c.range(0, "B1:B4");
        assert_eq!(
            call(&mut c, "CHITEST", vec![a.clone(), short]),
            Value::Error(ErrorKind::InvalidArgument)
        );
        approx(
            call(&mut c, "ZTEST", vec![a.clone(), n(4.0)]),
            0.059_977_421_014_579_92,
            1e-9,
        );
        approx(
            call(&mut c, "ZTEST", vec![a.clone(), n(4.0), n(2.0)]),
            0.022_085_672_454_221_304,
            1e-9,
        );
        assert_eq!(
            call(&mut c, "ZTEST", vec![a, n(4.0), n(0.0)]),
            Value::Error(ErrorKind::InvalidArgument)
        );
        assert_eq!(
            call(&mut c, "ZTEST", vec![EvalValue::text("x"), n(4.0)]),
            Value::Error(ErrorKind::ParamList)
        );
    }
}
