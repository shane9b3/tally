//! DELTA, GESTEP, ERF, ERFC, FACTDOUBLE, SQRTPI, MULTINOMIAL, SERIESSUM,
//! MROUND, QUOTIENT, GCD_ADD, LCM_ADD, ISEVEN_ADD, ISODD_ADD (Analysis add-in).
//!
//! Semantics follow Apache OpenOffice `scaddins/source/analysis/analysis.cxx`
//! and `analysishelper.cxx` (Apache-2.0). ERF/ERFC use a series and a
//! continued fraction instead of AOO's `rtl::math::erf` so both are accurate
//! to about 1e-16 relative.

use tally_types::{ErrorKind, Value};

use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry, ReturnKind};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const TWO_NUM: &[Param] = &[
    Param::required("number1", ParamKind::Number),
    Param::optional("number2", ParamKind::Number),
];
const ONE_NUM: &[Param] = &[Param::required("number", ParamKind::Number)];
const ONE_INT: &[Param] = &[Param::required("number", ParamKind::Integer)];
const SEQ: &[Param] = &[Param::repeat("number", ParamKind::NumberSeq)];
const SERIES: &[Param] = &[
    Param::required("x", ParamKind::Number),
    Param::required("n", ParamKind::Number),
    Param::required("m", ParamKind::Number),
    Param::required("coefficients", ParamKind::NumberSeq),
];
const MROUND_P: &[Param] = &[
    Param::required("number", ParamKind::Number),
    Param::required("multiple", ParamKind::Number),
];
const QUOT_P: &[Param] = &[
    Param::required("numerator", ParamKind::Number),
    Param::required("denominator", ParamKind::Number),
];
const ERF_P: &[Param] = &[
    Param::required("lower_limit", ParamKind::Number),
    Param::optional("upper_limit", ParamKind::Number),
];

/// `DELTA(a [; b])`: 1 if equal.
pub const DELTA: FunctionDef = FunctionDef::new("DELTA", Category::AddIn, TWO_NUM, delta)
    .help("1 if the two numbers are equal, else 0.");
/// `GESTEP(number [; step])`.
pub const GESTEP: FunctionDef = FunctionDef::new("GESTEP", Category::AddIn, TWO_NUM, gestep)
    .help("1 if number >= step, else 0.");
/// `ERF(lower [; upper])`.
pub const ERF: FunctionDef = FunctionDef::new("ERF", Category::AddIn, ERF_P, erf_fn)
    .help("Error function integrated between the limits.");
/// `ERFC(x)`.
pub const ERFC: FunctionDef = FunctionDef::new("ERFC", Category::AddIn, ONE_NUM, erfc_fn)
    .help("Complementary error function.");
/// `FACTDOUBLE(n)`.
pub const FACTDOUBLE: FunctionDef =
    FunctionDef::new("FACTDOUBLE", Category::AddIn, ONE_INT, factdouble)
        .help("Double factorial n!!.");
/// `SQRTPI(x)`.
pub const SQRTPI: FunctionDef =
    FunctionDef::new("SQRTPI", Category::AddIn, ONE_NUM, sqrtpi).help("Square root of x times pi.");
/// `MULTINOMIAL(n1; n2; ...)`.
pub const MULTINOMIAL: FunctionDef =
    FunctionDef::new("MULTINOMIAL", Category::AddIn, SEQ, multinomial)
        .help("Multinomial coefficient of the numbers.");
/// `SERIESSUM(x; n; m; coefficients)`.
pub const SERIESSUM: FunctionDef =
    FunctionDef::new("SERIESSUM", Category::AddIn, SERIES, seriessum)
        .help("Sum of the power series c_i * x^(n + i*m).");
/// `MROUND(number; multiple)`.
pub const MROUND: FunctionDef = FunctionDef::new("MROUND", Category::AddIn, MROUND_P, mround)
    .help("Rounds to the nearest multiple.");
/// `QUOTIENT(numerator; denominator)`.
pub const QUOTIENT: FunctionDef = FunctionDef::new("QUOTIENT", Category::AddIn, QUOT_P, quotient)
    .help("Integer part of a division.");
/// `GCD(n1; ...)` of the Analysis add-in; plain GCD belongs to Mathematical.
pub const GCD_ADD: FunctionDef = FunctionDef::new("GCD_ADD", Category::AddIn, SEQ, gcd_fn)
    .odf("COM.SUN.STAR.SHEET.ADDIN.ANALYSIS.GETGCD")
    .help("Greatest common divisor.");
/// `LCM(n1; ...)` of the Analysis add-in; plain LCM belongs to Mathematical.
pub const LCM_ADD: FunctionDef = FunctionDef::new("LCM_ADD", Category::AddIn, SEQ, lcm_fn)
    .odf("COM.SUN.STAR.SHEET.ADDIN.ANALYSIS.GETLCM")
    .help("Least common multiple.");
/// `ISEVEN(number)` of the Analysis add-in.
pub const ISEVEN_ADD: FunctionDef =
    FunctionDef::new("ISEVEN_ADD", Category::AddIn, ONE_INT, iseven)
        .odf("COM.SUN.STAR.SHEET.ADDIN.ANALYSIS.GETISEVEN")
        .returns(ReturnKind::Bool)
        .help("TRUE if the integer part is even.");
/// `ISODD(number)` of the Analysis add-in.
pub const ISODD_ADD: FunctionDef = FunctionDef::new("ISODD_ADD", Category::AddIn, ONE_INT, isodd)
    .odf("COM.SUN.STAR.SHEET.ADDIN.ANALYSIS.GETISODD")
    .returns(ReturnKind::Bool)
    .help("TRUE if the integer part is odd.");

fn delta(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let (x, y) = (a.num(0)?, a.num_opt(1, 0.0)?);
        Ok(Value::Number(if x == y { 1.0 } else { 0.0 }))
    })
}

fn gestep(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let (x, s) = (a.num(0)?, a.num_opt(1, 0.0)?);
        Ok(Value::Number(if x >= s { 1.0 } else { 0.0 }))
    })
}

// ERF/ERFC: AOO's add-in calls `rtl::math::erf`/`erfc`, ported once in
// `crate::stats_util`.
use crate::stats_util::{erf, erfc};

fn erf_fn(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let lo = a.num(0)?;
        if a.is_missing(1) {
            Ok(Value::Number(erf(lo)))
        } else {
            let hi = a.num(1)?;
            Ok(Value::Number(erf(hi) - erf(lo)))
        }
    })
}

fn erfc_fn(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| Ok(Value::Number(erfc(a.num(0)?))))
}

fn factdouble(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let n = a.int(0)?;
        if !(0..=300).contains(&n) {
            return if n < 0 {
                Err(ErrorKind::InvalidArgument)
            } else {
                Err(ErrorKind::Num)
            };
        }
        let mut r = 1.0_f64;
        let mut k = n;
        while k > 1 {
            r *= k as f64;
            k -= 2;
        }
        Ok(Value::Number(r))
    })
}

fn sqrtpi(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let x = a.num(0)?;
        if x < 0.0 {
            return Err(ErrorKind::InvalidArgument);
        }
        Ok(Value::Number((x * std::f64::consts::PI).sqrt()))
    })
}

fn multinomial(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let mut items = Vec::new();
        a.for_each_number_all(|v| items.push(v.trunc()))?;
        if items.iter().any(|v| *v < 0.0) {
            return Err(ErrorKind::InvalidArgument);
        }
        // Product of binomials C(running_sum, n_i): exact in f64 while the result stays below 2^53.
        let mut result = 1.0_f64;
        let mut total = 0.0_f64;
        for n in items {
            total += n;
            if total > 1e6 {
                return Err(ErrorKind::Num);
            }
            let k = n.min(total - n);
            let mut c = 1.0_f64;
            let mut j = 1.0;
            while j <= k {
                c = c * (total - k + j) / j;
                j += 1.0;
            }
            result *= c.round();
        }
        Ok(Value::Number(result))
    })
}

fn seriessum(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let (x, n, m) = (a.num(0)?, a.num(1)?, a.num(2)?);
        let mut coefs = Vec::new();
        a.for_each_number(3, |v| coefs.push(v))?;
        let mut sum = 0.0;
        for (i, c) in coefs.iter().enumerate() {
            sum += c * x.powf(n + m * i as f64);
        }
        Ok(Value::Number(sum))
    })
}

fn mround(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let (x, m) = (a.num(0)?, a.num(1)?);
        if m == 0.0 {
            return Ok(Value::Number(0.0));
        }
        if (x > 0.0 && m < 0.0) || (x < 0.0 && m > 0.0) {
            return Err(ErrorKind::InvalidArgument);
        }
        Ok(Value::Number((x / m).round() * m))
    })
}

fn quotient(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let (n, d) = (a.num(0)?, a.num(1)?);
        if d == 0.0 {
            return Err(ErrorKind::Div0);
        }
        Ok(Value::Number((n / d).trunc()))
    })
}

fn gcd2(mut a: f64, mut b: f64) -> f64 {
    while b != 0.0 {
        let t = a % b;
        a = b;
        b = t;
    }
    a
}

/// Collects the truncated, non-negative operands of GCD/LCM.
fn gcd_lcm_args(a: &mut Args) -> Result<Vec<f64>, ErrorKind> {
    let mut v = Vec::new();
    a.for_each_number_all(|x| v.push(x.trunc()))?;
    if v.iter().any(|x| *x < 0.0 || *x >= 9_007_199_254_740_992.0) {
        return Err(ErrorKind::InvalidArgument);
    }
    Ok(v)
}

fn gcd_fn(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let v = gcd_lcm_args(&mut a)?;
        Ok(Value::Number(v.into_iter().fold(0.0, gcd2)))
    })
}

fn lcm_fn(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let v = gcd_lcm_args(&mut a)?;
        let mut l = 1.0_f64;
        for x in v {
            if x == 0.0 {
                return Ok(Value::Number(0.0));
            }
            l = l / gcd2(l, x) * x;
            if l >= 9_007_199_254_740_992.0 {
                return Err(ErrorKind::Num);
            }
        }
        Ok(Value::Number(l))
    })
}

fn iseven(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| Ok(Value::Bool(a.int(0)? % 2 == 0)))
}

fn isodd(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| Ok(Value::Bool(a.int(0)? % 2 != 0)))
}

pub(super) fn register(r: &mut Registry) {
    for def in [
        DELTA,
        GESTEP,
        ERF,
        ERFC,
        FACTDOUBLE,
        SQRTPI,
        MULTINOMIAL,
        SERIESSUM,
        MROUND,
        QUOTIENT,
        GCD_ADD,
        LCM_ADD,
        ISEVEN_ADD,
        ISODD_ADD,
    ] {
        r.register(def);
    }
}

#[cfg(test)]
mod tests {
    use super::{erf, erfc};
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{ErrorKind, Value};

    fn c(name: &str, args: &[f64]) -> Value {
        call(
            &mut MockEvalContext::new(),
            name,
            args.iter().map(|x| EvalValue::number(*x)).collect(),
        )
    }
    fn num(name: &str, args: &[f64]) -> f64 {
        match c(name, args) {
            Value::Number(x) => x,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn erf_accuracy() {
        let cases = [
            (0.1, 0.1124629160182849),
            (0.5, 0.5204998778130465),
            (1.0, 0.8427007929497149),
            (2.0, 0.9953222650189527),
            (3.0, 0.9999779095030014),
        ];
        for (x, want) in cases {
            assert!(((erf(x) - want) / want).abs() < 1e-15, "erf({x})");
            assert!(((erf(-x) + want) / want).abs() < 1e-15);
        }
        let cc = [
            (0.5, 0.4795001221869535),
            (1.0, 0.1572992070502851),
            (2.0, 0.004677734981047266),
            (5.0, 1.5374597944280347e-12),
        ];
        for (x, want) in cc {
            assert!(
                ((erfc(x) - want) / want).abs() < 1e-14,
                "erfc({x}) = {}",
                erfc(x)
            );
        }
        assert_eq!(erf(0.0), 0.0);
        assert_eq!(erfc(0.0), 1.0);
        assert!((erfc(-1.0) - 1.8427007929497148).abs() < 1e-15);
    }

    #[test]
    fn simple_functions() {
        assert_eq!(c("DELTA", &[5.0, 5.0]), Value::Number(1.0));
        assert_eq!(c("DELTA", &[5.0]), Value::Number(0.0));
        assert_eq!(c("GESTEP", &[3.0, 4.0]), Value::Number(0.0));
        assert_eq!(c("GESTEP", &[-1.0]), Value::Number(0.0));
        assert_eq!(c("FACTDOUBLE", &[7.0]), Value::Number(105.0));
        assert_eq!(c("FACTDOUBLE", &[8.0]), Value::Number(384.0));
        assert_eq!(c("FACTDOUBLE", &[0.0]), Value::Number(1.0));
        assert_eq!(
            c("FACTDOUBLE", &[-1.0]),
            Value::Error(ErrorKind::InvalidArgument)
        );
        assert_eq!(c("FACTDOUBLE", &[400.0]), Value::Error(ErrorKind::Num));
        assert_eq!(c("MULTINOMIAL", &[2.0, 3.0, 4.0]), Value::Number(1260.0));
        assert_eq!(
            c("MULTINOMIAL", &[-1.0, 3.0]),
            Value::Error(ErrorKind::InvalidArgument)
        );
        assert!((num("SQRTPI", &[2.0]) - 2.5066282746310002).abs() < 1e-15);
        assert_eq!(
            c("SQRTPI", &[-1.0]),
            Value::Error(ErrorKind::InvalidArgument)
        );
    }

    #[test]
    fn rounding_and_division() {
        assert_eq!(c("MROUND", &[10.0, 3.0]), Value::Number(9.0));
        assert_eq!(c("MROUND", &[-10.0, -3.0]), Value::Number(-9.0));
        assert_eq!(
            c("MROUND", &[10.0, -3.0]),
            Value::Error(ErrorKind::InvalidArgument)
        );
        assert_eq!(c("MROUND", &[5.0, 0.0]), Value::Number(0.0));
        assert_eq!(c("MROUND", &[1.5, 1.0]), Value::Number(2.0));
        assert_eq!(c("QUOTIENT", &[7.0, 2.0]), Value::Number(3.0));
        assert_eq!(c("QUOTIENT", &[-7.0, 2.0]), Value::Number(-3.0));
        assert_eq!(c("QUOTIENT", &[1.0, 0.0]), Value::Error(ErrorKind::Div0));
    }

    #[test]
    fn gcd_lcm_parity() {
        assert_eq!(c("GCD_ADD", &[24.0, 36.0, 60.0]), Value::Number(12.0));
        assert_eq!(c("GCD_ADD", &[0.0, 0.0]), Value::Number(0.0));
        assert_eq!(
            c("GCD_ADD", &[-2.0, 4.0]),
            Value::Error(ErrorKind::InvalidArgument)
        );
        assert_eq!(c("LCM_ADD", &[4.0, 6.0, 10.0]), Value::Number(60.0));
        assert_eq!(c("LCM_ADD", &[4.0, 0.0]), Value::Number(0.0));
        assert_eq!(c("ISEVEN_ADD", &[4.0]), Value::Bool(true));
        assert_eq!(c("ISEVEN_ADD", &[-3.0]), Value::Bool(false));
        assert_eq!(c("ISODD_ADD", &[-3.0]), Value::Bool(true));
        assert_eq!(c("ISODD_ADD", &[2.9]), Value::Bool(false));
    }
}
