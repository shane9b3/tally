//! AVEDEV, AVERAGEA, DEVSQ, GEOMEAN, HARMEAN, KURT, SKEW, STANDARDIZE, TRIMMEAN.

use tally_types::{ErrorKind, Value};

use super::util::{data, mean, seq_numbers, sum_sq_dev};
use crate::coerce::approx_floor;
use crate::numeric::ApproxSum;
use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const SEQ: &[Param] = &[Param::repeat("number", ParamKind::NumberSeq)];
const SEQ_A: &[Param] = &[Param::repeat("value", ParamKind::ValueSeq)];
const STANDARDIZE_PARAMS: &[Param] = &[
    Param::required("number", ParamKind::Number),
    Param::required("mean", ParamKind::Number),
    Param::required("stdev", ParamKind::Number),
];
const TRIMMEAN_PARAMS: &[Param] = &[
    Param::required("data", ParamKind::Array),
    Param::required("alpha", ParamKind::Number),
];

/// `AVEDEV(number; ...)`.
pub const AVEDEV: FunctionDef = FunctionDef::new("AVEDEV", Category::Statistical, SEQ, avedev)
    .help("Returns the average of the absolute deviations of data points from their mean.");
/// `AVERAGEA(value; ...)`.
pub const AVERAGEA: FunctionDef =
    FunctionDef::new("AVERAGEA", Category::Statistical, SEQ_A, averagea)
        .help("Returns the average of a sample, counting text as 0.");
/// `DEVSQ(number; ...)`.
pub const DEVSQ: FunctionDef = FunctionDef::new("DEVSQ", Category::Statistical, SEQ, devsq)
    .help("Returns the sum of squares of deviations from the sample mean.");
/// `GEOMEAN(number; ...)`.
pub const GEOMEAN: FunctionDef = FunctionDef::new("GEOMEAN", Category::Statistical, SEQ, geomean)
    .help("Returns the geometric mean of a sample.");
/// `HARMEAN(number; ...)`.
pub const HARMEAN: FunctionDef = FunctionDef::new("HARMEAN", Category::Statistical, SEQ, harmean)
    .help("Returns the harmonic mean of a data set.");
/// `KURT(number; ...)`.
pub const KURT: FunctionDef = FunctionDef::new("KURT", Category::Statistical, SEQ, kurt)
    .help("Returns the kurtosis of a data set.");
/// `SKEW(number; ...)`.
pub const SKEW: FunctionDef = FunctionDef::new("SKEW", Category::Statistical, SEQ, skew)
    .help("Returns the skewness of a distribution.");
/// `STANDARDIZE(number; mean; stdev)`.
pub const STANDARDIZE: FunctionDef = FunctionDef::new(
    "STANDARDIZE",
    Category::Statistical,
    STANDARDIZE_PARAMS,
    standardize,
)
.help("Converts a random variable to a normalized value.");
/// `TRIMMEAN(data; alpha)`.
pub const TRIMMEAN: FunctionDef =
    FunctionDef::new("TRIMMEAN", Category::Statistical, TRIMMEAN_PARAMS, trimmean)
        .help("Returns the mean of a data set without the extreme values.");

fn avedev(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let v = seq_numbers(&mut a, false)?;
        // AOO ScAveDev / LibreOffice: no numbers is NoValue.
        if v.is_empty() {
            return Err(ErrorKind::Value);
        }
        let m = mean(&v);
        Ok(Value::Number(
            v.iter().map(|x| (x - m).abs()).sum::<f64>() / v.len() as f64,
        ))
    })
}

fn averagea(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let v = seq_numbers(&mut a, true)?;
        let mut acc = ApproxSum::new();
        for x in &v {
            acc.add(*x);
        }
        if v.is_empty() {
            return Err(ErrorKind::Div0);
        }
        Ok(Value::Number(acc.total() / v.len() as f64))
    })
}

fn devsq(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let v = seq_numbers(&mut a, false)?;
        if v.is_empty() {
            return Err(ErrorKind::Value);
        }
        Ok(Value::Number(sum_sq_dev(&v, mean(&v))))
    })
}

fn geomean(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let v = seq_numbers(&mut a, false)?;
        if v.is_empty() {
            return Err(ErrorKind::Value);
        }
        if v.iter().any(|x| *x <= 0.0) {
            return Err(ErrorKind::InvalidArgument);
        }
        let s: f64 = v.iter().map(|x| x.ln()).sum();
        Ok(Value::Number((s / v.len() as f64).exp()))
    })
}

fn harmean(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let v = seq_numbers(&mut a, false)?;
        if v.is_empty() {
            return Err(ErrorKind::Value);
        }
        if v.iter().any(|x| *x <= 0.0) {
            return Err(ErrorKind::InvalidArgument);
        }
        let s: f64 = v.iter().map(|x| 1.0 / x).sum();
        Ok(Value::Number(v.len() as f64 / s))
    })
}

/// Sample standard deviation of `v` (n >= 2), or `#DIV/0!` when it is 0.
fn z_scores(v: &[f64]) -> Result<Vec<f64>, ErrorKind> {
    let m = mean(v);
    let sd = (sum_sq_dev(v, m) / (v.len() as f64 - 1.0)).sqrt();
    if sd == 0.0 {
        return Err(ErrorKind::Div0);
    }
    Ok(v.iter().map(|x| (x - m) / sd).collect())
}

fn skew(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let v = seq_numbers(&mut a, false)?;
        if v.len() < 3 {
            return Err(ErrorKind::Div0);
        }
        let n = v.len() as f64;
        let s: f64 = z_scores(&v)?.iter().map(|z| z * z * z).sum();
        Ok(Value::Number(s * n / ((n - 1.0) * (n - 2.0))))
    })
}

fn kurt(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let v = seq_numbers(&mut a, false)?;
        if v.len() < 4 {
            return Err(ErrorKind::Div0);
        }
        let n = v.len() as f64;
        let s: f64 = z_scores(&v)?.iter().map(|z| z * z * z * z).sum();
        let k = s * n * (n + 1.0) / ((n - 1.0) * (n - 2.0) * (n - 3.0))
            - 3.0 * (n - 1.0) * (n - 1.0) / ((n - 2.0) * (n - 3.0));
        Ok(Value::Number(k))
    })
}

fn standardize(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let x = a.num(0)?;
        let m = a.num(1)?;
        let sd = a.num(2)?;
        if sd <= 0.0 {
            return Err(ErrorKind::InvalidArgument);
        }
        Ok(Value::Number((x - m) / sd))
    })
}

fn trimmean(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let mut v = data(&mut a, 0)?.numbers()?;
        let alpha = a.num(1)?;
        if !(0.0..1.0).contains(&alpha) {
            return Err(ErrorKind::InvalidArgument);
        }
        if v.is_empty() {
            return Err(ErrorKind::Value);
        }
        v.sort_by(f64::total_cmp);
        let n = v.len();
        let k = (approx_floor(n as f64 * alpha / 2.0) as usize).min(n / 2);
        let kept = &v[k..n - k];
        if kept.is_empty() {
            return Err(ErrorKind::Div0);
        }
        Ok(Value::Number(mean(kept)))
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(AVEDEV);
    r.register(AVERAGEA);
    r.register(DEVSQ);
    r.register(GEOMEAN);
    r.register(HARMEAN);
    r.register(KURT);
    r.register(SKEW);
    r.register(STANDARDIZE);
    r.register(TRIMMEAN);
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{ErrorKind, Value};

    fn nums(xs: &[f64]) -> Vec<EvalValue> {
        xs.iter().map(|x| EvalValue::number(*x)).collect()
    }

    fn close(v: Value, want: f64) {
        match v {
            Value::Number(n) => assert!((n - want).abs() < 1e-12, "{n} vs {want}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn moments() {
        let mut c = MockEvalContext::new();
        close(
            call(
                &mut c,
                "AVEDEV",
                nums(&[4.0, 5.0, 8.0, 7.0, 11.0, 4.0, 3.0]),
            ),
            2.2857142857142856,
        );
        close(
            call(&mut c, "DEVSQ", nums(&[4.0, 5.0, 8.0, 7.0, 11.0, 4.0, 3.0])),
            48.0,
        );
        close(call(&mut c, "GEOMEAN", nums(&[4.0, 9.0])), 6.0);
        close(call(&mut c, "HARMEAN", nums(&[2.0, 3.0, 6.0])), 3.0);
        close(
            call(
                &mut c,
                "SKEW",
                nums(&[3.0, 4.0, 5.0, 2.0, 3.0, 4.0, 5.0, 6.0, 4.0, 7.0]),
            ),
            0.3595430714067974,
        );
        close(
            call(
                &mut c,
                "KURT",
                nums(&[3.0, 4.0, 5.0, 2.0, 3.0, 4.0, 5.0, 6.0, 4.0, 7.0]),
            ),
            -0.15179963720841272,
        );
        close(
            call(&mut c, "STANDARDIZE", nums(&[42.0, 40.0, 1.5])),
            1.3333333333333333,
        );
    }

    #[test]
    fn errors() {
        let mut c = MockEvalContext::new();
        assert_eq!(
            call(&mut c, "GEOMEAN", nums(&[1.0, -1.0])),
            Value::Error(ErrorKind::InvalidArgument)
        );
        assert_eq!(
            call(&mut c, "SKEW", nums(&[1.0, 2.0])),
            Value::Error(ErrorKind::Div0)
        );
        assert_eq!(
            call(&mut c, "KURT", nums(&[1.0, 1.0, 1.0, 1.0])),
            Value::Error(ErrorKind::Div0)
        );
        assert_eq!(
            call(&mut c, "STANDARDIZE", nums(&[1.0, 1.0, 0.0])),
            Value::Error(ErrorKind::InvalidArgument)
        );
    }

    #[test]
    fn averagea_and_trimmean() {
        let mut c = MockEvalContext::new();
        c.set(0, "A1", 1.0)
            .set(0, "A2", "x")
            .set(0, "A3", true)
            .set(0, "A4", 5.0);
        let r = c.range(0, "A1:A4");
        close(call(&mut c, "AVERAGEA", vec![r]), 7.0 / 4.0);
        let r = c.range(0, "A5:A6");
        assert_eq!(
            call(&mut c, "AVERAGEA", vec![r]),
            Value::Error(ErrorKind::Div0)
        );
        let xs: Vec<f64> = vec![4.0, 5.0, 6.0, 7.0, 2.0, 3.0, 4.0, 5.0, 1.0, 2.0, 3.0, 100.0];
        for (i, x) in xs.iter().enumerate() {
            c.set(0, &format!("C{}", i + 1), *x);
        }
        let r = c.range(0, "C1:C12");
        close(
            call(&mut c, "TRIMMEAN", vec![r.clone(), EvalValue::number(0.2)]),
            4.1,
        );
        assert_eq!(
            call(&mut c, "TRIMMEAN", vec![r, EvalValue::number(1.0)]),
            Value::Error(ErrorKind::InvalidArgument)
        );
    }
}
