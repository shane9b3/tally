//! LINEST, LOGEST, TREND, GROWTH.
//!
//! Least squares after Apache OpenOffice `sc/source/core/tool/interpr3.cxx`
//! (`CalculateRGPRKP`, `CalculateTrendGrowth`; Apache-2.0): the data is
//! centred when a constant term is fitted, factored by Householder QR, and
//! `R b = Q^T y` is solved by back substitution. The statistics block is the
//! 5-row layout (padded with `#N/A`). A rank-deficient X, too few samples or
//! bad shapes are `Err:502`, as AOO's `PushIllegalArgument`.

use std::sync::Arc;

use tally_types::{ErrorKind, Matrix, Value};

use super::mat::{dense, Dense};
use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry, ReturnKind};
use crate::{run_ref, ArgList, Args, EvalContext, EvalValue};

const ILLEGAL: ErrorKind = ErrorKind::InvalidArgument;

const LINEST_PARAMS: &[Param] = &[
    Param::required("data_Y", ParamKind::Array),
    Param::optional("data_X", ParamKind::Array),
    Param::optional("Linear_type", ParamKind::Bool),
    Param::optional("stats", ParamKind::Bool),
];
const LOGEST_PARAMS: &[Param] = &[
    Param::required("data_Y", ParamKind::Array),
    Param::optional("data_X", ParamKind::Array),
    Param::optional("Function_type", ParamKind::Bool),
    Param::optional("stats", ParamKind::Bool),
];
const TREND_PARAMS: &[Param] = &[
    Param::required("data_Y", ParamKind::Array),
    Param::optional("data_X", ParamKind::Array),
    Param::optional("new data_X", ParamKind::Array),
    Param::optional("Linear_type", ParamKind::Bool),
];
const GROWTH_PARAMS: &[Param] = &[
    Param::required("data_Y", ParamKind::Array),
    Param::optional("data_X", ParamKind::Array),
    Param::optional("new_data_X", ParamKind::Array),
    Param::optional("Function_type", ParamKind::Bool),
];

/// `LINEST(data_Y; data_X; Linear_type; stats)`.
pub const LINEST: FunctionDef = FunctionDef::new("LINEST", Category::Array, LINEST_PARAMS, linest)
    .returns(ReturnKind::Array)
    .help("Returns the parameters of a linear regression trend.");
/// `LOGEST(data_Y; data_X; Function_type; stats)`.
pub const LOGEST: FunctionDef = FunctionDef::new("LOGEST", Category::Array, LOGEST_PARAMS, logest)
    .returns(ReturnKind::Array)
    .help("Returns the parameters of an exponential regression trend.");
/// `TREND(data_Y; data_X; new data_X; Linear_type)`.
pub const TREND: FunctionDef = FunctionDef::new("TREND", Category::Array, TREND_PARAMS, trend)
    .returns(ReturnKind::Array)
    .help("Returns values along a linear trend.");
/// `GROWTH(data_Y; data_X; new_data_X; Function_type)`.
pub const GROWTH: FunctionDef = FunctionDef::new("GROWTH", Category::Array, GROWTH_PARAMS, growth)
    .returns(ReturnKind::Array)
    .help("Returns values along an exponential trend.");

/// Regression statistics (NaN where undefined).
struct Stats {
    se: Vec<f64>,
    se_intercept: Option<f64>,
    r2: f64,
    se_y: f64,
    f: f64,
    df: f64,
    ss_reg: f64,
    ss_resid: f64,
}

struct Fit {
    coef: Vec<f64>,
    intercept: f64,
    stats: Stats,
}

/// Fits `y ~ X` (`xs` holds the K columns of X). Householder QR on the
/// centred data when `constant`.
fn fit(y: &[f64], xs: &[Vec<f64>], constant: bool) -> Result<Fit, ErrorKind> {
    let n = y.len();
    let k = xs.len();
    if (constant && n < k + 1) || (!constant && n < k) || n < 1 || k < 1 {
        return Err(ILLEGAL);
    }
    let nf = n as f64;
    let mean_y = if constant {
        y.iter().sum::<f64>() / nf
    } else {
        0.0
    };
    let means: Vec<f64> = xs
        .iter()
        .map(|c| {
            if constant {
                c.iter().sum::<f64>() / nf
            } else {
                0.0
            }
        })
        .collect();
    let xc: Vec<Vec<f64>> = xs
        .iter()
        .zip(&means)
        .map(|(c, m)| c.iter().map(|v| v - m).collect())
        .collect();
    let yc: Vec<f64> = y.iter().map(|v| v - mean_y).collect();

    // Householder QR; `a` holds columns, `qty` is Q^T y.
    let mut a = xc.clone();
    let mut qty = yc.clone();
    let orig: Vec<f64> = a
        .iter()
        .map(|c| c.iter().map(|v| v * v).sum::<f64>().sqrt())
        .collect();
    for j in 0..k {
        let norm = a[j][j..].iter().map(|v| v * v).sum::<f64>().sqrt();
        if orig[j] == 0.0 || norm <= orig[j] * 1e-12 {
            return Err(ILLEGAL);
        }
        let alpha = if a[j][j] > 0.0 { -norm } else { norm };
        let mut v: Vec<f64> = a[j][j..].to_vec();
        v[0] -= alpha;
        let vv: f64 = v.iter().map(|t| t * t).sum();
        if vv > 0.0 {
            for col in a.iter_mut().skip(j + 1).chain(std::iter::once(&mut qty)) {
                let dot: f64 = v.iter().zip(&col[j..]).map(|(p, q)| p * q).sum();
                let f = 2.0 * dot / vv;
                for (t, p) in col[j..].iter_mut().zip(&v) {
                    *t -= f * p;
                }
            }
        }
        a[j][j] = alpha;
    }
    // R[i][l] = a[l][i] for i <= l. Solve R b = qty[..k].
    let mut b = vec![0.0; k];
    for i in (0..k).rev() {
        let mut s = qty[i];
        for l in i + 1..k {
            s -= a[l][i] * b[l];
        }
        b[i] = s / a[i][i];
    }
    let intercept = if constant {
        mean_y - b.iter().zip(&means).map(|(c, m)| c * m).sum::<f64>()
    } else {
        0.0
    };

    // Residual and regression sums of squares.
    let mut ss_reg = 0.0;
    let mut ss_resid = 0.0;
    for i in 0..n {
        let fitted: f64 = (0..k).map(|j| b[j] * xc[j][i]).sum();
        ss_reg += fitted * fitted;
        let e = yc[i] - fitted;
        ss_resid += e * e;
    }
    let ss_total: f64 = yc.iter().map(|v| v * v).sum();
    let df = (n - k - usize::from(constant)) as f64;
    let mse = ss_resid / df;

    // Rinv (upper triangular), so (X'X)^-1 = Rinv Rinv^T.
    let mut rinv = vec![vec![0.0; k]; k];
    for j in 0..k {
        rinv[j][j] = 1.0 / a[j][j];
        for i in (0..j).rev() {
            let mut s = 0.0;
            for l in i + 1..=j {
                s += a[l][i] * rinv[l][j];
            }
            rinv[i][j] = -s / a[i][i];
        }
    }
    let se: Vec<f64> = (0..k)
        .map(|j| (mse * (j..k).map(|l| rinv[j][l] * rinv[j][l]).sum::<f64>()).sqrt())
        .collect();
    let se_intercept = constant.then(|| {
        let q: f64 = (0..k)
            .map(|l| {
                let t: f64 = (0..=l).map(|j| means[j] * rinv[j][l]).sum();
                t * t
            })
            .sum();
        (mse * (1.0 / nf + q)).sqrt()
    });
    Ok(Fit {
        coef: b,
        intercept,
        stats: Stats {
            se,
            se_intercept,
            r2: ss_reg / ss_total,
            se_y: mse.sqrt(),
            f: (ss_reg / k as f64) / mse,
            df,
            ss_reg,
            ss_resid,
        },
    })
}

/// Y and X reshaped for fitting.
struct Layout {
    ys: Vec<f64>,
    xs: Vec<Vec<f64>>,
    /// 1: X same shape as Y; 2: Y column, X has K columns; 3: Y row, X has K rows.
    case: u8,
}

fn col_major(d: &Dense) -> Vec<f64> {
    let mut v = Vec::with_capacity(d.d.len());
    for c in 0..d.cols {
        for r in 0..d.rows {
            v.push(d.at(r, c));
        }
    }
    v
}

/// AOO's shape rules for Y and X (X defaults to 1..N).
fn layout(y: &Dense, x: Option<&Dense>) -> Result<Layout, ErrorKind> {
    let ys = col_major(y);
    let Some(x) = x else {
        let xs = vec![(1..=ys.len()).map(|i| i as f64).collect()];
        return Ok(Layout { ys, xs, case: 1 });
    };
    if x.rows == y.rows && x.cols == y.cols {
        return Ok(Layout {
            ys,
            xs: vec![col_major(x)],
            case: 1,
        });
    }
    if y.cols == 1 {
        if x.rows != y.rows {
            return Err(ILLEGAL);
        }
        let xs = (0..x.cols)
            .map(|c| (0..x.rows).map(|r| x.at(r, c)).collect())
            .collect();
        return Ok(Layout { ys, xs, case: 2 });
    }
    if y.rows == 1 {
        if x.cols != y.cols {
            return Err(ILLEGAL);
        }
        let xs = (0..x.rows)
            .map(|r| (0..x.cols).map(|c| x.at(r, c)).collect())
            .collect();
        return Ok(Layout { ys, xs, case: 3 });
    }
    Err(ILLEGAL)
}

fn ln_all(ys: &mut [f64]) -> Result<(), ErrorKind> {
    for v in ys {
        if *v <= 0.0 {
            return Err(ILLEGAL);
        }
        *v = v.ln();
    }
    Ok(())
}

fn opt_dense(a: &mut Args, i: usize) -> Result<Option<Dense>, ErrorKind> {
    if a.is_missing(i) {
        return Ok(None);
    }
    let v = a.any(i);
    dense(a.ctx(), v, ILLEGAL).map(Some)
}

/// Builds the LINEST/LOGEST result block.
fn est_matrix(f: &Fit, stats: bool, log: bool) -> Matrix {
    let k = f.coef.len();
    let rows = if stats { 5 } else { 1 };
    let mut m = Matrix::new_filled(rows, k as u32 + 1, Value::Error(ErrorKind::NA));
    let g = |x: f64| if log { x.exp() } else { x };
    for j in 0..k {
        m.set(0, (k - 1 - j) as u32, Value::Number(g(f.coef[j])));
    }
    m.set(0, k as u32, Value::Number(g(f.intercept)));
    if stats {
        let s = &f.stats;
        for j in 0..k {
            m.set(1, (k - 1 - j) as u32, Value::Number(s.se[j]));
        }
        if let Some(se) = s.se_intercept {
            m.set(1, k as u32, Value::Number(se));
        }
        m.set(2, 0, Value::Number(s.r2));
        m.set(2, 1, Value::Number(s.se_y));
        m.set(3, 0, Value::Number(s.f));
        m.set(3, 1, Value::Number(s.df));
        m.set(4, 0, Value::Number(s.ss_reg));
        m.set(4, 1, Value::Number(s.ss_resid));
    }
    m
}

fn estimate(ctx: &mut dyn EvalContext, args: &mut dyn ArgList, log: bool) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run_ref(|| {
        let constant = a.bool_opt(2, true)?;
        let stats = a.bool_opt(3, false)?;
        let y = a.any(0);
        let y = dense(a.ctx(), y, ILLEGAL)?;
        let x = opt_dense(&mut a, 1)?;
        let mut l = layout(&y, x.as_ref())?;
        if log {
            ln_all(&mut l.ys)?;
        }
        let f = fit(&l.ys, &l.xs, constant)?;
        Ok(EvalValue::Array(Arc::new(est_matrix(&f, stats, log))))
    })
}

fn linest(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    estimate(ctx, args, false)
}

fn logest(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    estimate(ctx, args, true)
}

/// TREND / GROWTH (AOO `CalculateTrendGrowth`).
fn predict(ctx: &mut dyn EvalContext, args: &mut dyn ArgList, log: bool) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run_ref(|| {
        let constant = a.bool_opt(3, true)?;
        let y = a.any(0);
        let y = dense(a.ctx(), y, ILLEGAL)?;
        let x = match opt_dense(&mut a, 1)? {
            Some(x) => x,
            None => {
                // Default X: 1..N, shaped like Y.
                let mut d = Dense::zeros(y.rows, y.cols);
                for (i, v) in d.d.iter_mut().enumerate() {
                    *v = (i + 1) as f64;
                }
                // Row-major fill equals column-major for vectors; for a
                // general matrix use column-major order as AOO.
                if y.rows > 1 && y.cols > 1 {
                    let mut n = 0.0;
                    for c in 0..y.cols {
                        for r in 0..y.rows {
                            n += 1.0;
                            d.set(r, c, n);
                        }
                    }
                }
                d
            }
        };
        let new_x = match opt_dense(&mut a, 2)? {
            Some(x) => x,
            None => x.clone(),
        };
        let mut l = layout(&y, Some(&x))?;
        if log {
            ln_all(&mut l.ys)?;
        }
        let f = fit(&l.ys, &l.xs, constant)?;
        let k = l.xs.len();
        // Points of the new X and the shape of the result.
        let (points, rows, cols): (Vec<Vec<f64>>, usize, usize) = match l.case {
            1 => {
                let pts = col_major(&new_x).into_iter().map(|v| vec![v]).collect();
                (pts, new_x.rows, new_x.cols)
            }
            2 => {
                if new_x.cols != k {
                    return Err(ILLEGAL);
                }
                let pts = (0..new_x.rows)
                    .map(|r| (0..k).map(|c| new_x.at(r, c)).collect())
                    .collect();
                (pts, new_x.rows, 1)
            }
            _ => {
                if new_x.rows != k {
                    return Err(ILLEGAL);
                }
                let pts = (0..new_x.cols)
                    .map(|c| (0..k).map(|r| new_x.at(r, c)).collect())
                    .collect();
                (pts, 1, new_x.cols)
            }
        };
        let mut out = Dense::zeros(rows, cols);
        for (i, p) in points.iter().enumerate() {
            let mut v = f.intercept + p.iter().zip(&f.coef).map(|(x, b)| x * b).sum::<f64>();
            if log {
                v = v.exp();
            }
            // Case 1 points are column-major over the new X shape.
            if l.case == 1 {
                out.set(i % rows, i / rows, v);
            } else {
                out.d[i] = v;
            }
        }
        Ok(out.into_eval())
    })
}

fn trend(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    predict(ctx, args, false)
}

fn growth(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    predict(ctx, args, true)
}

pub(super) fn register(r: &mut Registry) {
    r.register(LINEST);
    r.register(LOGEST);
    r.register(TREND);
    r.register(GROWTH);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::{call, MockEvalContext};

    fn mat(v: &Value) -> Vec<Vec<Value>> {
        match v {
            Value::Array(m) => (0..m.rows)
                .map(|r| (0..m.cols).filter_map(|c| m.get(r, c).cloned()).collect())
                .collect(),
            other => vec![vec![other.clone()]],
        }
    }

    fn n(v: &Value, r: usize, c: usize) -> f64 {
        mat(v)[r][c].as_number().unwrap_or(f64::NAN)
    }

    fn setup(c: &mut MockEvalContext, y: &[f64], x: &[f64]) {
        for (i, v) in y.iter().enumerate() {
            c.set(0, &format!("A{}", i + 1), *v);
        }
        for (i, v) in x.iter().enumerate() {
            c.set(0, &format!("B{}", i + 1), *v);
        }
    }

    #[test]
    fn linest_simple() {
        let mut c = MockEvalContext::new();
        setup(&mut c, &[3.0, 5.0, 7.0, 9.0], &[1.0, 2.0, 3.0, 4.0]);
        let (y, x) = (c.range(0, "A1:A4"), c.range(0, "B1:B4"));
        let r = call(&mut c, "LINEST", vec![y.clone(), x.clone()]);
        assert!((n(&r, 0, 0) - 2.0).abs() < 1e-12);
        assert!((n(&r, 0, 1) - 1.0).abs() < 1e-12);
        let r = call(&mut c, "LINEST", vec![y, x, EvalValue::bool(false)]);
        // Through the origin: slope = sum(xy)/sum(x^2) = 70/30.
        assert!((n(&r, 0, 0) - 70.0 / 30.0).abs() < 1e-12);
        assert_eq!(n(&r, 0, 1), 0.0);
    }

    #[test]
    fn linest_stats_known() {
        // Excel documentation example.
        let mut c = MockEvalContext::new();
        setup(&mut c, &[1.0, 9.0, 5.0, 7.0], &[0.0, 4.0, 2.0, 3.0]);
        let (y, x) = (c.range(0, "A1:A4"), c.range(0, "B1:B4"));
        let r = call(
            &mut c,
            "LINEST",
            vec![y, x, EvalValue::bool(true), EvalValue::bool(true)],
        );
        assert!((n(&r, 0, 0) - 2.0).abs() < 1e-12);
        assert!((n(&r, 0, 1) - 1.0).abs() < 1e-12);
        assert!((n(&r, 4, 1) - 0.0).abs() < 1e-12);
        assert_eq!(mat(&r)[2][1].as_number().map(|v| v.is_nan()), Some(false));
        assert!((n(&r, 3, 1) - 2.0).abs() < 1e-12, "df");
    }

    #[test]
    fn linest_noisy_stats() {
        let mut c = MockEvalContext::new();
        setup(
            &mut c,
            &[1.0, 3.0, 2.0, 5.0, 4.0],
            &[1.0, 2.0, 3.0, 4.0, 5.0],
        );
        let (y, x) = (c.range(0, "A1:A5"), c.range(0, "B1:B5"));
        let r = call(
            &mut c,
            "LINEST",
            vec![y, x, EvalValue::bool(true), EvalValue::bool(true)],
        );
        // slope 0.8, intercept 0.6, ss_resid 3.6, ss_reg 6.4
        assert!((n(&r, 0, 0) - 0.8).abs() < 1e-12);
        assert!((n(&r, 0, 1) - 0.6).abs() < 1e-12);
        assert!((n(&r, 2, 0) - 0.64).abs() < 1e-12);
        assert!((n(&r, 4, 0) - 6.4).abs() < 1e-12);
        assert!((n(&r, 4, 1) - 3.6).abs() < 1e-12);
        let se_slope = (3.6f64 / 3.0 / 10.0).sqrt();
        assert!((n(&r, 1, 0) - se_slope).abs() < 1e-12);
        let se_b = (3.6f64 / 3.0 * (0.2 + 9.0 / 10.0)).sqrt();
        assert!((n(&r, 1, 1) - se_b).abs() < 1e-12);
        assert!((n(&r, 3, 0) - 6.4 / (3.6 / 3.0)).abs() < 1e-10);
    }

    #[test]
    fn linest_multi_and_collinear() {
        let mut c = MockEvalContext::new();
        // y = 1 + 2*x1 + 3*x2
        let x1 = [1.0, 2.0, 3.0, 4.0, 5.0];
        let x2 = [2.0, 1.0, 4.0, 3.0, 7.0];
        for i in 0..5 {
            c.set(0, &format!("A{}", i + 1), 1.0 + 2.0 * x1[i] + 3.0 * x2[i]);
            c.set(0, &format!("B{}", i + 1), x1[i]);
            c.set(0, &format!("C{}", i + 1), x2[i]);
            c.set(0, &format!("D{}", i + 1), 2.0 * x1[i]);
        }
        let (y, x) = (c.range(0, "A1:A5"), c.range(0, "B1:C5"));
        let r = call(&mut c, "LINEST", vec![y.clone(), x]);
        assert!((n(&r, 0, 0) - 3.0).abs() < 1e-10);
        assert!((n(&r, 0, 1) - 2.0).abs() < 1e-10);
        assert!((n(&r, 0, 2) - 1.0).abs() < 1e-10);
        let col = c.range(0, "B1:B5");
        let dup = c.range(0, "B1:D5");
        let _ = col;
        assert_eq!(
            call(&mut c, "LINEST", vec![y, dup]),
            Value::Error(ErrorKind::InvalidArgument)
        );
    }

    #[test]
    fn logest_growth_trend() {
        let mut c = MockEvalContext::new();
        // y = 2 * 3^x
        setup(&mut c, &[6.0, 18.0, 54.0, 162.0], &[1.0, 2.0, 3.0, 4.0]);
        let (y, x) = (c.range(0, "A1:A4"), c.range(0, "B1:B4"));
        let r = call(&mut c, "LOGEST", vec![y.clone(), x.clone()]);
        assert!((n(&r, 0, 0) - 3.0).abs() < 1e-9);
        assert!((n(&r, 0, 1) - 2.0).abs() < 1e-9);
        let nx = arr_col(&[5.0, 6.0]);
        let g = call(&mut c, "GROWTH", vec![y.clone(), x.clone(), nx.clone()]);
        assert!((n(&g, 0, 0) - 486.0).abs() < 1e-8);
        assert!((n(&g, 1, 0) - 1458.0).abs() < 1e-7);
        let t = call(&mut c, "TREND", vec![y, x, nx]);
        // Linear fit of 6,18,54,162: slope 50.4, intercept -62.4... check shape.
        match &t {
            Value::Array(m) => assert_eq!((m.rows, m.cols), (2, 1)),
            other => panic!("{other:?}"),
        }
        assert!((n(&t, 1, 0) - n(&t, 0, 0) - 50.4).abs() < 1e-9);
    }

    fn arr_col(v: &[f64]) -> EvalValue {
        let d = v.iter().map(|x| Value::Number(*x)).collect();
        EvalValue::Array(Arc::new(
            Matrix::from_vec(v.len() as u32, 1, d)
                .unwrap_or_else(|| Matrix::new_filled(1, 1, Value::Empty)),
        ))
    }

    #[test]
    fn errors() {
        let mut c = MockEvalContext::new();
        setup(&mut c, &[1.0, -2.0, 3.0], &[1.0, 2.0, 3.0]);
        let (y, x) = (c.range(0, "A1:A3"), c.range(0, "B1:B3"));
        assert_eq!(
            call(&mut c, "LOGEST", vec![y.clone(), x.clone()]),
            Value::Error(ErrorKind::InvalidArgument)
        );
        c.set(0, "A2", "t");
        let y2 = c.range(0, "A1:A3");
        assert_eq!(
            call(&mut c, "LINEST", vec![y2, x]),
            Value::Error(ErrorKind::InvalidArgument)
        );
        let one = c.range(0, "A1:A1");
        assert_eq!(
            call(&mut c, "LINEST", vec![one]),
            Value::Error(ErrorKind::InvalidArgument)
        );
    }
}
