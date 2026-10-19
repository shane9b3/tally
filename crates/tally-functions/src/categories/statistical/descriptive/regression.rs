//! CORREL, COVAR, PEARSON, RSQ, SLOPE, INTERCEPT, STEYX, FORECAST.
//!
//! Two data arguments are paired by position; a pair is skipped unless both
//! cells hold a number or boolean (AOO matrix semantics: text, empty cells
//! and errors in a pair are skipped, not propagated). Arguments of different
//! shape are `#N/A`.

use tally_types::{ErrorKind, Value};

use super::util::{as_number, data, mean};
use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const PAIR: &[Param] = &[
    Param::required("data_1", ParamKind::Array),
    Param::required("data_2", ParamKind::Array),
];
const YX: &[Param] = &[
    Param::required("data_y", ParamKind::Array),
    Param::required("data_x", ParamKind::Array),
];
const FORECAST_PARAMS: &[Param] = &[
    Param::required("value", ParamKind::Number),
    Param::required("data_y", ParamKind::Array),
    Param::required("data_x", ParamKind::Array),
];

macro_rules! def {
    ($c:ident, $f:ident, $name:literal, $params:ident, $help:literal) => {
        #[doc = concat!("`", $name, "`.")]
        pub const $c: FunctionDef =
            FunctionDef::new($name, Category::Statistical, $params, $f).help($help);
    };
}

def!(
    CORREL,
    correl,
    "CORREL",
    PAIR,
    "Returns the correlation coefficient between two data sets."
);
def!(
    COVAR,
    covar,
    "COVAR",
    PAIR,
    "Returns the population covariance of two data sets."
);
def!(
    PEARSON,
    pearson,
    "PEARSON",
    PAIR,
    "Returns the Pearson product moment correlation coefficient."
);
def!(
    RSQ,
    rsq,
    "RSQ",
    YX,
    "Returns the square of the Pearson correlation coefficient."
);
def!(
    SLOPE,
    slope,
    "SLOPE",
    YX,
    "Returns the slope of the linear regression line."
);
def!(
    INTERCEPT,
    intercept,
    "INTERCEPT",
    YX,
    "Returns the y-intercept of the regression line."
);
def!(
    STEYX,
    steyx,
    "STEYX",
    YX,
    "Returns the standard error of the predicted y values."
);
def!(
    FORECAST,
    forecast,
    "FORECAST",
    FORECAST_PARAMS,
    "Extrapolates a future value along a linear trend."
);

/// The numeric pairs of arguments `i` and `j`.
fn pairs(a: &mut Args, i: usize, j: usize) -> Result<(Vec<f64>, Vec<f64>), ErrorKind> {
    let d1 = data(a, i)?;
    let d2 = data(a, j)?;
    if (d1.rows, d1.cols) != (d2.rows, d2.cols) {
        return Err(ErrorKind::NA);
    }
    let (mut xs, mut ys) = (Vec::new(), Vec::new());
    let (mut p, mut q) = (0, 0);
    while p < d1.items.len() && q < d2.items.len() {
        let (ip, vp) = &d1.items[p];
        let (iq, vq) = &d2.items[q];
        match ip.cmp(iq) {
            std::cmp::Ordering::Less => p += 1,
            std::cmp::Ordering::Greater => q += 1,
            std::cmp::Ordering::Equal => {
                if let (Some(x), Some(y)) = (as_number(vp), as_number(vq)) {
                    xs.push(x);
                    ys.push(y);
                }
                p += 1;
                q += 1;
            }
        }
    }
    Ok((xs, ys))
}

/// Centered sums: (n, mean_x, mean_y, Sxx, Syy, Sxy).
struct Sums {
    n: usize,
    mx: f64,
    my: f64,
    sxx: f64,
    syy: f64,
    sxy: f64,
}

fn sums(xs: &[f64], ys: &[f64]) -> Sums {
    let n = xs.len();
    let (mx, my) = if n == 0 {
        (0.0, 0.0)
    } else {
        (mean(xs), mean(ys))
    };
    let (mut sxx, mut syy, mut sxy) = (0.0, 0.0, 0.0);
    for (x, y) in xs.iter().zip(ys) {
        sxx += (x - mx) * (x - mx);
        syy += (y - my) * (y - my);
        sxy += (x - mx) * (y - my);
    }
    Sums {
        n,
        mx,
        my,
        sxx,
        syy,
        sxy,
    }
}

/// Runs `f` on the sums of the pairs of arguments (`first`, `first + 1`).
fn with_sums(
    ctx: &mut dyn EvalContext,
    args: &mut dyn ArgList,
    first: usize,
    f: impl FnOnce(&mut Args, Sums) -> Result<f64, ErrorKind>,
) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let (xs, ys) = pairs(&mut a, first, first + 1)?;
        Ok(Value::Number(f(&mut a, sums(&xs, &ys))?))
    })
}

fn pearson_r(s: &Sums) -> Result<f64, ErrorKind> {
    if s.n < 2 || s.sxx == 0.0 || s.syy == 0.0 {
        return Err(ErrorKind::Div0);
    }
    Ok(s.sxy / (s.sxx * s.syy).sqrt())
}

fn correl(c: &mut dyn EvalContext, a: &mut dyn ArgList) -> EvalValue {
    with_sums(c, a, 0, |_, s| pearson_r(&s))
}
fn pearson(c: &mut dyn EvalContext, a: &mut dyn ArgList) -> EvalValue {
    with_sums(c, a, 0, |_, s| pearson_r(&s))
}
fn covar(c: &mut dyn EvalContext, a: &mut dyn ArgList) -> EvalValue {
    with_sums(c, a, 0, |_, s| {
        if s.n < 1 {
            return Err(ErrorKind::Div0);
        }
        Ok(s.sxy / s.n as f64)
    })
}
fn rsq(c: &mut dyn EvalContext, a: &mut dyn ArgList) -> EvalValue {
    // Arguments are (y, x); the coefficient is symmetric.
    with_sums(c, a, 0, |_, s| pearson_r(&s).map(|r| r * r))
}

/// Slope of y on x; the sums are built with the pair as (y, x).
fn slope_of(s: &Sums) -> Result<f64, ErrorKind> {
    // `sums(xs, ys)` was called with xs = known_y, ys = known_x, so the
    // regressor's variance is `syy`.
    if s.n < 2 || s.syy == 0.0 {
        return Err(ErrorKind::Div0);
    }
    Ok(s.sxy / s.syy)
}
fn slope(c: &mut dyn EvalContext, a: &mut dyn ArgList) -> EvalValue {
    with_sums(c, a, 0, |_, s| slope_of(&s))
}
fn intercept(c: &mut dyn EvalContext, a: &mut dyn ArgList) -> EvalValue {
    with_sums(c, a, 0, |_, s| Ok(s.mx - slope_of(&s)? * s.my))
}
fn steyx(c: &mut dyn EvalContext, a: &mut dyn ArgList) -> EvalValue {
    with_sums(c, a, 0, |_, s| {
        if s.n < 3 || s.syy == 0.0 {
            return Err(ErrorKind::Div0);
        }
        // s.sxx is the spread of y, s.syy of x.
        let resid = (s.sxx - s.sxy * s.sxy / s.syy).max(0.0);
        Ok((resid / (s.n as f64 - 2.0)).sqrt())
    })
}
fn forecast(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let x = a.num(0)?;
        let (ys, xs) = pairs(&mut a, 1, 2)?;
        // Build with (xs = y values, ys = x values) like the others.
        let s = sums(&ys, &xs);
        let b = slope_of(&s)?;
        Ok(Value::Number(s.mx + b * (x - s.my)))
    })
}

pub(super) fn register(r: &mut Registry) {
    for d in [
        CORREL, COVAR, PEARSON, RSQ, SLOPE, INTERCEPT, STEYX, FORECAST,
    ] {
        r.register(d);
    }
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{ErrorKind, Value};

    fn close(v: Value, want: f64) {
        match v {
            Value::Number(n) => assert!((n - want).abs() < 1e-12, "{n} vs {want}"),
            other => panic!("{other:?}"),
        }
    }

    fn sheet() -> MockEvalContext {
        let mut c = MockEvalContext::new();
        let ys = [2.0, 4.0, 5.0, 4.0, 5.0];
        for (i, y) in ys.iter().enumerate() {
            c.set(0, &format!("A{}", i + 1), *y);
            c.set(0, &format!("B{}", i + 1), (i + 1) as f64);
        }
        c
    }

    #[test]
    fn regression() {
        let mut c = sheet();
        let y = c.range(0, "A1:A5");
        let x = c.range(0, "B1:B5");
        close(call(&mut c, "SLOPE", vec![y.clone(), x.clone()]), 0.6);
        close(call(&mut c, "INTERCEPT", vec![y.clone(), x.clone()]), 2.2);
        close(
            call(&mut c, "CORREL", vec![y.clone(), x.clone()]),
            0.7745966692414834,
        );
        close(
            call(&mut c, "PEARSON", vec![x.clone(), y.clone()]),
            0.7745966692414834,
        );
        close(call(&mut c, "RSQ", vec![y.clone(), x.clone()]), 0.6);
        close(call(&mut c, "COVAR", vec![y.clone(), x.clone()]), 1.2);
        close(
            call(&mut c, "STEYX", vec![y.clone(), x.clone()]),
            0.8944271909999159,
        );
        close(
            call(&mut c, "FORECAST", vec![EvalValue::number(6.0), y, x]),
            5.8,
        );
    }

    #[test]
    fn skipping_and_errors() {
        let mut c = sheet();
        c.set(0, "A3", "x");
        let y = c.range(0, "A1:A5");
        let x = c.range(0, "B1:B5");
        // pair 3 skipped: x = 1,2,4,5, y = 2,4,4,5
        close(call(&mut c, "SLOPE", vec![y.clone(), x.clone()]), 0.6);
        let short = c.range(0, "B1:B4");
        assert_eq!(
            call(&mut c, "SLOPE", vec![y, short]),
            Value::Error(ErrorKind::NA)
        );
        let one = c.range(0, "B1");
        assert_eq!(
            call(&mut c, "CORREL", vec![one.clone(), one]),
            Value::Error(ErrorKind::Div0)
        );
        let z = c.range(0, "C1:C3");
        assert_eq!(
            call(&mut c, "COVAR", vec![z.clone(), z]),
            Value::Error(ErrorKind::Div0)
        );
    }
}
