//! PERMUT, PERMUTATIONA, PROB.

use tally_types::{approx_equal, ErrorKind, Value};

use super::util::data;
use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const PERMUT_PARAMS: &[Param] = &[
    Param::required("count_1", ParamKind::Integer),
    Param::required("count_2", ParamKind::Integer),
];
const PROB_PARAMS: &[Param] = &[
    Param::required("data", ParamKind::Array),
    Param::required("probability", ParamKind::Array),
    Param::required("start", ParamKind::Number),
    Param::optional("end", ParamKind::Number),
];

/// `PERMUT(count_1; count_2)`.
pub const PERMUT: FunctionDef =
    FunctionDef::new("PERMUT", Category::Statistical, PERMUT_PARAMS, permut)
        .help("Returns the number of permutations without repetition.");
/// `PERMUTATIONA(count_1; count_2)`.
pub const PERMUTATIONA: FunctionDef = FunctionDef::new(
    "PERMUTATIONA",
    Category::Statistical,
    PERMUT_PARAMS,
    permutationa,
)
.help("Returns the number of permutations with repetition.");
/// `PROB(data; probability; start; [end])`.
pub const PROB: FunctionDef = FunctionDef::new("PROB", Category::Statistical, PROB_PARAMS, prob)
    .help("Returns the probability that values fall between two limits.");

fn permut(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let n = a.int(0)?;
        let k = a.int(1)?;
        if n < 0 || k < 0 || k > n {
            return Err(ErrorKind::InvalidArgument);
        }
        let mut r = 1.0;
        for i in 0..k {
            r *= (n - i) as f64;
        }
        Ok(Value::Number(r))
    })
}

fn permutationa(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let n = a.int(0)?;
        let k = a.int(1)?;
        if n < 0 || k < 0 {
            return Err(ErrorKind::InvalidArgument);
        }
        Ok(Value::Number((n as f64).powi(k as i32)))
    })
}

fn prob(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let xs = data(&mut a, 0)?;
        let ps = data(&mut a, 1)?;
        let lo = a.num(2)?;
        let hi = if a.is_missing(3) { lo } else { a.num(3)? };
        if (xs.rows, xs.cols) != (ps.rows, ps.cols) {
            return Err(ErrorKind::NA);
        }
        let xs = xs.dense_numbers()?;
        let ps = ps.dense_numbers()?;
        let (lo, hi) = if lo > hi { (hi, lo) } else { (lo, hi) };
        let (mut total, mut hit) = (0.0, 0.0);
        for (x, p) in xs.iter().zip(&ps) {
            if *p <= 0.0 || *p > 1.0 {
                return Err(ErrorKind::Value);
            }
            total += p;
            if (*x >= lo && *x <= hi) || approx_equal(*x, lo) || approx_equal(*x, hi) {
                hit += p;
            }
        }
        if (total - 1.0).abs() > 1.0e-7 {
            return Err(ErrorKind::Value);
        }
        Ok(Value::Number(hit))
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(PERMUT);
    r.register(PERMUTATIONA);
    r.register(PROB);
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{ErrorKind, Value};

    fn n(x: f64) -> EvalValue {
        EvalValue::number(x)
    }

    #[test]
    fn permutations() {
        let mut c = MockEvalContext::new();
        assert_eq!(
            call(&mut c, "PERMUT", vec![n(5.0), n(2.0)]),
            Value::Number(20.0)
        );
        assert_eq!(
            call(&mut c, "PERMUT", vec![n(5.0), n(0.0)]),
            Value::Number(1.0)
        );
        assert_eq!(
            call(&mut c, "PERMUT", vec![n(2.0), n(3.0)]),
            Value::Error(ErrorKind::InvalidArgument)
        );
        assert_eq!(
            call(&mut c, "PERMUTATIONA", vec![n(3.0), n(2.0)]),
            Value::Number(9.0)
        );
        assert_eq!(
            call(&mut c, "PERMUTATIONA", vec![n(0.0), n(0.0)]),
            Value::Number(1.0)
        );
        assert_eq!(
            call(&mut c, "PERMUTATIONA", vec![n(-1.0), n(2.0)]),
            Value::Error(ErrorKind::InvalidArgument)
        );
    }

    #[test]
    fn prob() {
        let mut c = MockEvalContext::new();
        for (i, (x, p)) in [(0.0, 0.2), (1.0, 0.3), (2.0, 0.1), (3.0, 0.4)]
            .iter()
            .enumerate()
        {
            c.set(0, &format!("A{}", i + 1), *x);
            c.set(0, &format!("B{}", i + 1), *p);
        }
        let x = c.range(0, "A1:A4");
        let p = c.range(0, "B1:B4");
        let v = call(&mut c, "PROB", vec![x.clone(), p.clone(), n(1.0), n(2.0)]);
        assert!(matches!(v, Value::Number(r) if (r - 0.4).abs() < 1e-12));
        assert_eq!(
            call(&mut c, "PROB", vec![x.clone(), p.clone(), n(3.0)]),
            Value::Number(0.4)
        );
        let short = c.range(0, "B1:B3");
        assert_eq!(
            call(&mut c, "PROB", vec![x, short, n(1.0)]),
            Value::Error(ErrorKind::NA)
        );
    }
}
