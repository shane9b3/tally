//! FACT, COMBIN, COMBINA, GCD, LCM, SUMSQ, ISEVEN, ISODD.
//!
//! Ported in behaviour from Apache OpenOffice `ScInterpreter::ScFact`,
//! `ScBinomial`, `ScGCD`, `ScLCM`, `ScSumSQ` (sc/source/core/tool/interpr*.cxx).

use tally_types::{ErrorKind, Value};

use crate::coerce::approx_floor;
use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry, ReturnKind};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const FACT_PARAMS: &[Param] = &[Param::required("number", ParamKind::Integer)];
const PAIR_PARAMS: &[Param] = &[
    Param::required("number_1", ParamKind::Integer),
    Param::required("number_2", ParamKind::Integer),
];
const LIST_PARAMS: &[Param] = &[Param::repeat("integer", ParamKind::NumberSeq)];
const SUMSQ_PARAMS: &[Param] = &[Param::repeat("number", ParamKind::NumberSeq)];
const PARITY_PARAMS: &[Param] = &[Param::required("value", ParamKind::Number)];

/// `FACT(number)`.
pub const FACT: FunctionDef = FunctionDef::new("FACT", Category::Mathematical, FACT_PARAMS, fact)
    .help("Returns the factorial of a number.");
/// `COMBIN(number_1; number_2)`.
pub const COMBIN: FunctionDef =
    FunctionDef::new("COMBIN", Category::Mathematical, PAIR_PARAMS, combin)
        .help("Returns the number of combinations for elements without repetition.");
/// `COMBINA(number_1; number_2)`.
pub const COMBINA: FunctionDef =
    FunctionDef::new("COMBINA", Category::Mathematical, PAIR_PARAMS, combina)
        .help("Returns the number of combinations for a given number of elements with repetition.");
/// `GCD(integer; ...)`.
pub const GCD: FunctionDef = FunctionDef::new("GCD", Category::Mathematical, LIST_PARAMS, gcd)
    .help("Returns the greatest common divisor of the given integers.");
/// `LCM(integer; ...)`.
pub const LCM: FunctionDef = FunctionDef::new("LCM", Category::Mathematical, LIST_PARAMS, lcm)
    .help("Returns the least common multiple of the given integers.");
/// `SUMSQ(number; ...)`.
pub const SUMSQ: FunctionDef =
    FunctionDef::new("SUMSQ", Category::Mathematical, SUMSQ_PARAMS, sumsq)
        .help("Returns the sum of the squares of the arguments.");
/// `ISEVEN(value)`.
pub const ISEVEN: FunctionDef =
    FunctionDef::new("ISEVEN", Category::Mathematical, PARITY_PARAMS, iseven)
        .returns(ReturnKind::Bool)
        .help("Returns TRUE if value is an even integer.");
/// `ISODD(value)`.
pub const ISODD: FunctionDef =
    FunctionDef::new("ISODD", Category::Mathematical, PARITY_PARAMS, isodd)
        .returns(ReturnKind::Bool)
        .help("Returns TRUE if value is an odd integer.");

/// `n!` for `0 <= n <= 170` as the plain running product (AOO `Fakultaet`).
fn factorial(n: u32) -> f64 {
    (2..=n).fold(1.0, |acc, i| acc * f64::from(i))
}

/// `C(n, k)` for non-negative integral `n >= k`. Stops as soon as the
/// running value overflows, so huge arguments cannot loop long.
fn binomial(n: f64, k: f64) -> f64 {
    let k = k.min(n - k);
    let mut r = 1.0;
    let mut i = 1.0;
    while i <= k {
        r = r * (n - k + i) / i;
        if r.is_infinite() {
            break;
        }
        i += 1.0;
    }
    r.round()
}

fn fact(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        // AOO: floor first, then 0 <= n <= 170.
        let n = approx_floor(a.num(0)?);
        if !(0.0..=170.0).contains(&n) {
            return Err(ErrorKind::InvalidArgument);
        }
        Ok(Value::Number(factorial(n as u32)))
    })
}

fn combin(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let n = approx_floor(a.num(0)?);
        let k = approx_floor(a.num(1)?);
        if k < 0.0 || n < k || n < 0.0 {
            return Err(ErrorKind::InvalidArgument);
        }
        Ok(Value::Number(binomial(n, k)))
    })
}

fn combina(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let n = approx_floor(a.num(0)?);
        let k = approx_floor(a.num(1)?);
        if k < 0.0 || n < 0.0 || (n == 0.0 && k > 0.0) {
            return Err(ErrorKind::InvalidArgument);
        }
        if k == 0.0 {
            return Ok(Value::Number(1.0));
        }
        Ok(Value::Number(binomial(n + k - 1.0, k)))
    })
}

fn gcd2(mut a: f64, mut b: f64) -> f64 {
    while b != 0.0 {
        (a, b) = (b, a % b);
    }
    a
}

/// Folds every number of every argument (floored, negatives are `Err:502`).
fn fold_ints(
    a: &mut Args,
    init: f64,
    mut step: impl FnMut(f64, f64) -> f64,
) -> Result<f64, ErrorKind> {
    let mut acc = init;
    let mut bad = false;
    a.for_each_number_all(|x| {
        let x = approx_floor(x);
        if x < 0.0 {
            bad = true;
        } else {
            acc = step(acc, x);
        }
    })?;
    if bad {
        Err(ErrorKind::InvalidArgument)
    } else {
        Ok(acc)
    }
}

fn gcd(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| Ok(Value::Number(fold_ints(&mut a, 0.0, gcd2)?)))
}

fn lcm(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        // 1 is the identity, but no numbers at all gives 0 like GCD.
        let mut any = false;
        let r = fold_ints(&mut a, 1.0, |acc, x| {
            any = true;
            if acc == 0.0 || x == 0.0 {
                0.0
            } else {
                acc / gcd2(acc, x) * x
            }
        })?;
        // An overflow to infinity becomes #NUM! in `run`.
        Ok(Value::Number(if any { r } else { 0.0 }))
    })
}

fn sumsq(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let mut s = 0.0;
        a.for_each_number_all(|x| s += x * x)?;
        Ok(Value::Number(s))
    })
}

fn parity(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| Ok(Value::Bool(a.num(0)?.trunc() % 2.0 == 0.0)))
}

fn iseven(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    parity(ctx, args)
}

fn isodd(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| Ok(Value::Bool(a.num(0)?.trunc() % 2.0 != 0.0)))
}

pub(super) fn register(r: &mut Registry) {
    for d in [FACT, COMBIN, COMBINA, GCD, LCM, SUMSQ, ISEVEN, ISODD] {
        r.register(d);
    }
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{ErrorKind, Value};

    fn f(name: &str, args: &[f64]) -> Value {
        call(
            &mut MockEvalContext::new(),
            name,
            args.iter().map(|x| EvalValue::number(*x)).collect(),
        )
    }
    fn num(x: f64) -> Value {
        Value::Number(x)
    }
    fn e502() -> Value {
        Value::Error(ErrorKind::InvalidArgument)
    }

    #[test]
    fn factorial_and_combinations() {
        assert_eq!(f("FACT", &[5.0]), num(120.0));
        assert_eq!(f("FACT", &[0.0]), num(1.0));
        assert_eq!(f("FACT", &[5.9]), num(120.0));
        assert_eq!(f("FACT", &[-1.0]), e502());
        assert_eq!(f("FACT", &[171.0]), e502());
        assert!(matches!(f("FACT", &[170.0]), Value::Number(x) if x > 7e306));
        assert_eq!(f("COMBIN", &[5.0, 2.0]), num(10.0));
        assert_eq!(f("COMBIN", &[5.0, 0.0]), num(1.0));
        assert_eq!(f("COMBIN", &[5.0, 6.0]), e502());
        assert_eq!(f("COMBIN", &[50.0, 25.0]), num(126_410_606_437_752.0));
        assert_eq!(f("COMBIN", &[2e9, 1e9]), Value::Error(ErrorKind::Num));
        assert_eq!(f("COMBINA", &[3.0, 2.0]), num(6.0));
        assert_eq!(f("COMBINA", &[0.0, 0.0]), num(1.0));
        assert_eq!(f("COMBINA", &[0.0, 1.0]), e502());
    }

    #[test]
    fn gcd_lcm() {
        assert_eq!(f("GCD", &[12.0, 18.0]), num(6.0));
        assert_eq!(f("GCD", &[0.0, 0.0]), num(0.0));
        assert_eq!(f("GCD", &[-1.0, 2.0]), e502());
        assert_eq!(f("LCM", &[4.0, 6.0]), num(12.0));
        assert_eq!(f("LCM", &[0.0, 6.0]), num(0.0));
        assert_eq!(
            f("LCM", &[f64::MAX, 2f64.powi(1023)]),
            Value::Error(ErrorKind::Num)
        );
    }

    #[test]
    fn sumsq_parity() {
        assert_eq!(f("SUMSQ", &[3.0, 4.0]), num(25.0));
        assert_eq!(f("ISEVEN", &[2.9]), Value::Bool(true));
        assert_eq!(f("ISEVEN", &[-3.0]), Value::Bool(false));
        assert_eq!(f("ISODD", &[-3.5]), Value::Bool(true));
        assert_eq!(f("ISODD", &[0.0]), Value::Bool(false));
    }
}
