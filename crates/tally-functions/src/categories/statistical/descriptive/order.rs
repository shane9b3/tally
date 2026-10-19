//! LARGE, SMALL, RANK, PERCENTILE, PERCENTRANK, QUARTILE, MODE.

use tally_types::{ErrorKind, Value};

use super::util::{data, seq_numbers};
use crate::coerce::approx_floor;
use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const SEQ: &[Param] = &[Param::repeat("number", ParamKind::NumberSeq)];
const KTH: &[Param] = &[
    Param::required("data", ParamKind::Array),
    Param::required("rank_c", ParamKind::Integer),
];
const RANK_PARAMS: &[Param] = &[
    Param::required("value", ParamKind::Number),
    Param::required("data", ParamKind::Array),
    Param::optional("type", ParamKind::Integer),
];
const PERCENTILE_PARAMS: &[Param] = &[
    Param::required("data", ParamKind::Array),
    Param::required("alpha", ParamKind::Number),
];
const QUARTILE_PARAMS: &[Param] = &[
    Param::required("data", ParamKind::Array),
    Param::required("type", ParamKind::Integer),
];
const PERCENTRANK_PARAMS: &[Param] = &[
    Param::required("data", ParamKind::Array),
    Param::required("value", ParamKind::Number),
];

/// `LARGE(data; rank_c)`.
pub const LARGE: FunctionDef = FunctionDef::new("LARGE", Category::Statistical, KTH, large)
    .help("Returns the k-th largest value in a data set.");
/// `SMALL(data; rank_c)`.
pub const SMALL: FunctionDef = FunctionDef::new("SMALL", Category::Statistical, KTH, small)
    .help("Returns the k-th smallest value in a data set.");
/// `RANK(value; data; [type])`.
pub const RANK: FunctionDef = FunctionDef::new("RANK", Category::Statistical, RANK_PARAMS, rank)
    .help("Returns the rank of a number in a sample.");
/// `PERCENTILE(data; alpha)`.
pub const PERCENTILE: FunctionDef = FunctionDef::new(
    "PERCENTILE",
    Category::Statistical,
    PERCENTILE_PARAMS,
    percentile,
)
.help("Returns the alpha-percentile of data values in an array.");
/// `PERCENTRANK(data; value)`.
pub const PERCENTRANK: FunctionDef = FunctionDef::new(
    "PERCENTRANK",
    Category::Statistical,
    PERCENTRANK_PARAMS,
    percentrank,
)
.help("Returns the percentage rank of a value in a sample.");
/// `QUARTILE(data; type)`.
pub const QUARTILE: FunctionDef =
    FunctionDef::new("QUARTILE", Category::Statistical, QUARTILE_PARAMS, quartile)
        .help("Returns the quartile of a data set.");
/// `MODE(number; ...)`.
pub const MODE: FunctionDef = FunctionDef::new("MODE", Category::Statistical, SEQ, mode)
    .help("Returns the most common value in a data set.");

fn sorted(a: &mut Args, i: usize) -> Result<Vec<f64>, ErrorKind> {
    let mut v = data(a, i)?.numbers()?;
    v.sort_by(f64::total_cmp);
    Ok(v)
}

fn kth(ctx: &mut dyn EvalContext, args: &mut dyn ArgList, largest: bool) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let v = sorted(&mut a, 0)?;
        let k = a.int(1)?;
        // AOO: k < 1 or k > count is Err:502 (also for an empty data set).
        if k < 1 || k as usize > v.len() {
            return Err(ErrorKind::InvalidArgument);
        }
        let k = k as usize;
        Ok(Value::Number(if largest {
            v[v.len() - k]
        } else {
            v[k - 1]
        }))
    })
}

fn large(c: &mut dyn EvalContext, a: &mut dyn ArgList) -> EvalValue {
    kth(c, a, true)
}
fn small(c: &mut dyn EvalContext, a: &mut dyn ArgList) -> EvalValue {
    kth(c, a, false)
}

fn rank(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let x = a.num(0)?;
        let v = data(&mut a, 1)?.numbers()?;
        let ascending = a.int_opt(2, 0)? != 0;
        if !v.contains(&x) {
            return Err(ErrorKind::NA);
        }
        let ahead = v
            .iter()
            .filter(|y| if ascending { **y < x } else { **y > x })
            .count();
        Ok(Value::Number((ahead + 1) as f64))
    })
}

/// Linear interpolation between closest ranks (AOO `GetPercentile`).
fn interpolate(v: &[f64], alpha: f64) -> f64 {
    let n = v.len();
    if n == 1 {
        return v[0];
    }
    let pos = alpha * (n as f64 - 1.0);
    let lo = approx_floor(pos);
    let frac = pos - lo;
    let lo = (lo as usize).min(n - 1);
    if lo + 1 >= n || frac == 0.0 {
        v[lo]
    } else {
        v[lo] + frac * (v[lo + 1] - v[lo])
    }
}

fn percentile_of(v: &[f64], alpha: f64) -> Result<Value, ErrorKind> {
    if v.is_empty() || !(0.0..=1.0).contains(&alpha) {
        return Err(ErrorKind::InvalidArgument);
    }
    Ok(Value::Number(interpolate(v, alpha)))
}

fn percentile(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let v = sorted(&mut a, 0)?;
        let alpha = a.num(1)?;
        percentile_of(&v, alpha)
    })
}

fn quartile(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let v = sorted(&mut a, 0)?;
        let t = a.int(1)?;
        if !(0..=4).contains(&t) {
            return Err(ErrorKind::InvalidArgument);
        }
        percentile_of(&v, t as f64 * 0.25)
    })
}

fn percentrank(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let v = sorted(&mut a, 0)?;
        let x = a.num(1)?;
        let n = v.len();
        if n == 0 || x < v[0] || x > v[n - 1] {
            return Err(ErrorKind::NA);
        }
        if n == 1 {
            return Ok(Value::Number(1.0));
        }
        let denom = (n - 1) as f64;
        // First index with v[i] >= x.
        let i = v.partition_point(|y| *y < x);
        let r = if v[i] == x {
            i as f64 / denom
        } else {
            let (lo, hi) = (v[i - 1], v[i]);
            ((i - 1) as f64 + (x - lo) / (hi - lo)) / denom
        };
        // Default significance of 3 digits, truncated (AOO).
        Ok(Value::Number(approx_floor(r * 1000.0) / 1000.0))
    })
}

fn mode(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let mut v = seq_numbers(&mut a, false)?;
        v.sort_by(f64::total_cmp);
        let (mut best, mut best_n) = (0.0, 0usize);
        let mut i = 0;
        while i < v.len() {
            let mut j = i;
            while j < v.len() && v[j] == v[i] {
                j += 1;
            }
            if j - i > best_n && j - i >= 2 {
                best = v[i];
                best_n = j - i;
            }
            i = j;
        }
        // AOO ScMode: no repeated value is NoValue; ties give the smallest.
        if best_n < 2 {
            return Err(ErrorKind::Value);
        }
        Ok(Value::Number(best))
    })
}

pub(super) fn register(r: &mut Registry) {
    for d in [LARGE, SMALL, RANK, PERCENTILE, PERCENTRANK, QUARTILE, MODE] {
        r.register(d);
    }
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{ErrorKind, Value};

    fn n(x: f64) -> EvalValue {
        EvalValue::number(x)
    }

    fn sheet() -> MockEvalContext {
        let mut c = MockEvalContext::new();
        for (i, x) in [3.0, 1.0, 4.0, 1.0, 5.0, 9.0, 2.0, 6.0].iter().enumerate() {
            c.set(0, &format!("A{}", i + 1), *x);
        }
        c.set(0, "A9", "text");
        c
    }

    #[test]
    fn large_small() {
        let mut c = sheet();
        let r = c.range(0, "A1:A9");
        assert_eq!(
            call(&mut c, "LARGE", vec![r.clone(), n(2.0)]),
            Value::Number(6.0)
        );
        assert_eq!(
            call(&mut c, "SMALL", vec![r.clone(), n(2.0)]),
            Value::Number(1.0)
        );
        assert_eq!(
            call(&mut c, "SMALL", vec![r.clone(), n(9.0)]),
            Value::Error(ErrorKind::InvalidArgument)
        );
        assert_eq!(
            call(&mut c, "LARGE", vec![r, n(0.0)]),
            Value::Error(ErrorKind::InvalidArgument)
        );
    }

    #[test]
    fn rank_percentile() {
        let mut c = sheet();
        let r = c.range(0, "A1:A9");
        assert_eq!(
            call(&mut c, "RANK", vec![n(4.0), r.clone()]),
            Value::Number(4.0)
        );
        assert_eq!(
            call(&mut c, "RANK", vec![n(4.0), r.clone(), n(1.0)]),
            Value::Number(5.0)
        );
        assert_eq!(
            call(&mut c, "RANK", vec![n(7.0), r.clone()]),
            Value::Error(ErrorKind::NA)
        );
        assert_eq!(
            call(&mut c, "PERCENTILE", vec![r.clone(), n(0.5)]),
            Value::Number(3.5)
        );
        assert_eq!(
            call(&mut c, "QUARTILE", vec![r.clone(), n(1.0)]),
            Value::Number(1.75)
        );
        assert_eq!(
            call(&mut c, "QUARTILE", vec![r.clone(), n(5.0)]),
            Value::Error(ErrorKind::InvalidArgument)
        );
        assert_eq!(
            call(&mut c, "PERCENTILE", vec![r, n(1.5)]),
            Value::Error(ErrorKind::InvalidArgument)
        );
    }

    #[test]
    fn percentrank_mode() {
        let mut c = MockEvalContext::new();
        for (i, x) in [1.0, 2.0, 4.0].iter().enumerate() {
            c.set(0, &format!("A{}", i + 1), *x);
        }
        let r = c.range(0, "A1:A3");
        assert_eq!(
            call(&mut c, "PERCENTRANK", vec![r.clone(), n(3.0)]),
            Value::Number(0.75)
        );
        assert_eq!(
            call(&mut c, "PERCENTRANK", vec![r.clone(), n(2.0)]),
            Value::Number(0.5)
        );
        assert_eq!(
            call(&mut c, "PERCENTRANK", vec![r, n(5.0)]),
            Value::Error(ErrorKind::NA)
        );
        assert_eq!(
            call(&mut c, "MODE", vec![n(1.0), n(2.0), n(2.0), n(3.0), n(3.0)]),
            Value::Number(2.0)
        );
        assert_eq!(
            call(&mut c, "MODE", vec![n(1.0), n(2.0)]),
            Value::Error(ErrorKind::Value)
        );
    }
}
