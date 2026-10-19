//! B, BINOMDIST, CRITBINOM, HYPGEOMDIST, NEGBINOMDIST, POISSON.
//!
//! Algorithms ported from Apache OpenOffice (interpr3.cxx), Apache License 2.0.

use tally_types::{ErrorKind, Value};

use super::special::{beta_dist, binom_dist_pmf, binom_dist_range, log_gamma, up_reg_igamma};
use super::{floor_num, opt_bool};
use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const B_PARAMS: &[Param] = &[
    Param::required("trials", ParamKind::Integer),
    Param::required("sp", ParamKind::Number),
    Param::required("t_1", ParamKind::Integer),
    Param::optional("t_2", ParamKind::Integer),
];

/// `B(trials; sp; t_1; [t_2])`.
pub const B: FunctionDef = FunctionDef::new("B", Category::Statistical, B_PARAMS, b)
    .help("Returns the probability of a trial result using binomial distribution.");

const BINOMDIST_PARAMS: &[Param] = &[
    Param::required("x", ParamKind::Integer),
    Param::required("trials", ParamKind::Integer),
    Param::required("sp", ParamKind::Number),
    Param::required("c", ParamKind::Bool),
];

/// `BINOMDIST(x; trials; sp; c)`.
pub const BINOMDIST: FunctionDef = FunctionDef::new(
    "BINOMDIST",
    Category::Statistical,
    BINOMDIST_PARAMS,
    binomdist,
)
.help("Values of the binomial distribution.");

const CRITBINOM_PARAMS: &[Param] = &[
    Param::required("trials", ParamKind::Integer),
    Param::required("sp", ParamKind::Number),
    Param::required("alpha", ParamKind::Number),
];

/// `CRITBINOM(trials; sp; alpha)`.
pub const CRITBINOM: FunctionDef =
    FunctionDef::new("CRITBINOM", Category::Statistical, CRITBINOM_PARAMS, critbinom)
        .help("Returns the smallest value for which the cumulative binomial distribution is greater than or equal to a criterion value.");

const HYPGEOMDIST_PARAMS: &[Param] = &[
    Param::required("x", ParamKind::Integer),
    Param::required("n_sample", ParamKind::Integer),
    Param::required("successes", ParamKind::Integer),
    Param::required("n_population", ParamKind::Integer),
];

/// `HYPGEOMDIST(x; n_sample; successes; n_population)`.
pub const HYPGEOMDIST: FunctionDef = FunctionDef::new(
    "HYPGEOMDIST",
    Category::Statistical,
    HYPGEOMDIST_PARAMS,
    hypgeomdist,
)
.help("Values of the hypergeometric distribution.");

const NEGBINOMDIST_PARAMS: &[Param] = &[
    Param::required("x", ParamKind::Integer),
    Param::required("r", ParamKind::Integer),
    Param::required("sp", ParamKind::Number),
];

/// `NEGBINOMDIST(x; r; sp)`.
pub const NEGBINOMDIST: FunctionDef = FunctionDef::new(
    "NEGBINOMDIST",
    Category::Statistical,
    NEGBINOMDIST_PARAMS,
    negbinomdist,
)
.help("Values of the negative binomial distribution.");

const POISSON_PARAMS: &[Param] = &[
    Param::required("number", ParamKind::Integer),
    Param::required("mean", ParamKind::Number),
    Param::optional("cumulative", ParamKind::Bool),
];

/// `POISSON(number; mean; [cumulative])`.
pub const POISSON: FunctionDef =
    FunctionDef::new("POISSON", Category::Statistical, POISSON_PARAMS, poisson)
        .help("Returns the Poisson distribution.");

/// AOO `ScB`: with three arguments the mass function at `t_1`, with four
/// the probability of `t_1..=t_2` successes. Invalid counts or `p` outside
/// [0, 1] are `Err:502`. A written but empty `t_2` reads as 0.
fn b(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let n = floor_num(&mut a, 0)?;
        let p = a.num(1)?;
        if a.count() < 4 {
            let x = floor_num(&mut a, 2)?;
            if n < 0.0 || x < 0.0 || x > n || !(0.0..=1.0).contains(&p) {
                return Err(ErrorKind::InvalidArgument);
            }
            if p == 0.0 {
                return Ok(Value::Number(if x == 0.0 { 1.0 } else { 0.0 }));
            }
            if p == 1.0 {
                return Ok(Value::Number(if x == n { 1.0 } else { 0.0 }));
            }
            return Ok(Value::Number(binom_dist_pmf(x, n, p)?));
        }
        let xs = floor_num(&mut a, 2)?;
        let xe = floor_num(&mut a, 3)?;
        let q = (0.5 - p) + 0.5;
        let valid_x = 0.0 <= xs && xs <= xe && xe <= n;
        if !valid_x {
            return Err(ErrorKind::InvalidArgument);
        }
        if !(0.0 < p && p < 1.0) {
            return if p == 0.0 {
                Ok(Value::Number(if xs == 0.0 { 1.0 } else { 0.0 }))
            } else if p == 1.0 {
                Ok(Value::Number(if xe == n { 1.0 } else { 0.0 }))
            } else {
                Err(ErrorKind::InvalidArgument)
            };
        }
        if xs == xe {
            return Ok(Value::Number(binom_dist_pmf(xs, n, p)?));
        }
        let factor = q.powf(n);
        if factor > f64::MIN_POSITIVE {
            return Ok(Value::Number(binom_dist_range(n, xs, xe, factor, p, q)));
        }
        let factor = p.powf(n);
        if factor > f64::MIN_POSITIVE {
            // sum_{j=xs}^{xe} C(n,j) p^j q^(n-j) = sum_{i=n-xe}^{n-xs} C(n,i) q^i p^(n-i)
            return Ok(Value::Number(binom_dist_range(
                n,
                n - xe,
                n - xs,
                factor,
                q,
                p,
            )));
        }
        Ok(Value::Number(
            beta_dist(q, n - xe, xe + 1.0) - beta_dist(q, n - xs + 1.0, xs),
        ))
    })
}

/// AOO `ScBinomDist`: counts floored; `x` outside [0, n] or `p` outside
/// [0, 1] is `Err:502`.
fn binomdist(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let x = floor_num(&mut a, 0)?;
        let n = floor_num(&mut a, 1)?;
        let p = a.num(2)?;
        let cumulative = a.bool(3)?;
        let q = (0.5 - p) + 0.5; // one bit more for p near 1
        if n < 0.0 || x < 0.0 || x > n || !(0.0..=1.0).contains(&p) {
            return Err(ErrorKind::InvalidArgument);
        }
        if p == 0.0 {
            return Ok(Value::Number(if x == 0.0 || cumulative {
                1.0
            } else {
                0.0
            }));
        }
        if p == 1.0 {
            return Ok(Value::Number(if x == n { 1.0 } else { 0.0 }));
        }
        if !cumulative {
            return Ok(Value::Number(binom_dist_pmf(x, n, p)?));
        }
        if x == n {
            return Ok(Value::Number(1.0));
        }
        let mut factor = q.powf(n);
        if x == 0.0 {
            return Ok(Value::Number(factor));
        }
        if factor > f64::MIN_POSITIVE {
            return Ok(Value::Number(binom_dist_range(n, 0.0, x, factor, p, q)));
        }
        factor = p.powf(n);
        if factor <= f64::MIN_POSITIVE {
            return Ok(Value::Number(beta_dist(q, n - x, x + 1.0)));
        }
        if factor > f64::EPSILON {
            let mut sum = 1.0 - factor;
            let max = ((n - x) as u32).saturating_sub(1);
            let mut i = 0u32;
            while i < max && factor > 0.0 {
                let fi = f64::from(i);
                factor *= (n - fi) / (fi + 1.0) * q / p;
                sum -= factor;
                i += 1;
            }
            return Ok(Value::Number(sum.max(0.0)));
        }
        Ok(Value::Number(binom_dist_range(n, n - x, n, factor, q, p)))
    })
}

/// AOO `ScCritBinom`: `n` floored; `n < 0`, `alpha` outside (0, 1) or `p`
/// outside [0, 1] is `Err:502`; both tails underflowing is `#VALUE!`.
fn critbinom(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let n = floor_num(&mut a, 0)?;
        let p = a.num(1)?;
        let alpha = a.num(2)?;
        if n < 0.0 || alpha <= 0.0 || alpha >= 1.0 || !(0.0..=1.0).contains(&p) {
            return Err(ErrorKind::InvalidArgument);
        }
        let q = 1.0 - p;
        let mut factor = q.powf(n);
        let max = n as u64;
        if factor == 0.0 {
            factor = p.powf(n);
            if factor == 0.0 {
                return Err(ErrorKind::Value);
            }
            let mut sum = 1.0 - factor;
            let mut i = 0u64;
            while i < max && sum >= alpha {
                let fi = i as f64;
                factor *= (n - fi) / (fi + 1.0) * q / p;
                sum -= factor;
                i += 1;
            }
            return Ok(Value::Number(n - i as f64));
        }
        let mut sum = factor;
        let mut i = 0u64;
        while i < max && sum < alpha {
            let fi = i as f64;
            factor *= (n - fi) / (fi + 1.0) * p / q;
            sum += factor;
            i += 1;
        }
        Ok(Value::Number(i as f64))
    })
}

/// AOO `lcl_PutFactorialElements`.
fn put_factorial_elements(cn: &mut Vec<f64>, lower: f64, upper: f64, base: f64) {
    let mut i = lower;
    while i <= upper {
        let v = base - i;
        if v > 1.0 {
            cn.push(v);
        }
        i += 1.0;
    }
}

/// AOO `ScHypGeomDist` (Kohei Yoshida, #i47296#): all nine factorials are
/// expanded and overlapping ranges cancelled before multiplying, so large
/// arguments do not overflow. Invalid combinations are `Err:502`; more
/// than 500000 factors is `#VALUE!`.
fn hypgeomdist(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let x = floor_num(&mut a, 0)?;
        let n = floor_num(&mut a, 1)?;
        let m = floor_num(&mut a, 2)?;
        let nn = floor_num(&mut a, 3)?;
        Ok(Value::Number(hypgeom(x, n, m, nn)?))
    })
}

/// `x` successes in a sample of `n` from a population of `nn` with `mm` successes.
fn hypgeom(x: f64, n: f64, mm: f64, nn: f64) -> Result<f64, ErrorKind> {
    const MAX_ARRAY_SIZE: f64 = 500_000.0;
    if x < 0.0 || n < x || mm < x || nn < n || nn < mm || x < n - nn + mm {
        return Err(ErrorKind::InvalidArgument);
    }
    let est = x + n.min(mm);
    if est > MAX_ARRAY_SIZE {
        return Err(ErrorKind::Value);
    }
    let mut numer: Vec<f64> = Vec::with_capacity(est as usize + 10);
    let mut denom: Vec<f64> = Vec::with_capacity(est as usize + 10);

    // Trim coefficient C first.
    let mut c_num_var_upper = nn - n - mm + x - 1.0;
    let mut c_denom_var_lower = 1.0;
    if nn - n - mm + x >= mm - x + 1.0 {
        c_num_var_upper = mm - x - 1.0;
        c_denom_var_lower = nn - n - 2.0 * (mm - x) + 1.0;
    }
    let mut c_denom_upper = nn - n - mm + x + 1.0 - c_denom_var_lower;
    let mut d_num_var_lower = n - mm;

    if n >= mm + 1.0 {
        if nn - mm < n + 1.0 {
            // Case 1
            if nn - n < n + 1.0 {
                put_factorial_elements(&mut numer, 0.0, c_num_var_upper, nn - n);
                put_factorial_elements(&mut denom, 0.0, nn - n - 1.0, nn);
            } else {
                put_factorial_elements(&mut numer, nn - 2.0 * n, c_num_var_upper, nn - n);
                put_factorial_elements(&mut denom, 0.0, n - 1.0, nn);
            }
            if c_denom_upper < n - x + 1.0 {
                put_factorial_elements(&mut numer, 1.0, nn - mm - n + x, nn - mm + 1.0);
            } else {
                put_factorial_elements(&mut numer, 1.0, nn - mm - c_denom_upper, nn - mm + 1.0);
                c_denom_upper = n - x;
                c_denom_var_lower = nn - mm - 2.0 * (n - x) + 1.0;
            }
        } else {
            // Case 2
            if n > mm - 1.0 {
                put_factorial_elements(&mut numer, 0.0, c_num_var_upper, nn - n);
                put_factorial_elements(&mut denom, 0.0, mm - 1.0, nn);
            } else {
                put_factorial_elements(&mut numer, mm - n, c_num_var_upper, nn - n);
                put_factorial_elements(&mut denom, 0.0, n - 1.0, nn);
            }
            if c_denom_upper < n - x + 1.0 {
                put_factorial_elements(
                    &mut numer,
                    nn - mm - n + 1.0,
                    nn - mm - n + x,
                    nn - mm + 1.0,
                );
            } else {
                put_factorial_elements(
                    &mut numer,
                    nn - mm - n + 1.0,
                    nn - mm - c_denom_upper,
                    nn - mm + 1.0,
                );
                c_denom_upper = n - x;
                c_denom_var_lower = nn - mm - 2.0 * (n - x) + 1.0;
            }
        }
    } else {
        if nn - mm < mm + 1.0 {
            // Case 3
            if nn - n < mm + 1.0 {
                put_factorial_elements(&mut numer, 0.0, c_num_var_upper, nn - n);
                put_factorial_elements(&mut denom, 0.0, nn - mm - 1.0, nn);
            } else {
                put_factorial_elements(&mut numer, nn - n - mm, c_num_var_upper, nn - n);
                put_factorial_elements(&mut denom, 0.0, n - 1.0, nn);
            }
            if n - x + 1.0 > c_denom_upper {
                put_factorial_elements(&mut numer, 1.0, nn - mm - n + x, nn - mm + 1.0);
            } else {
                put_factorial_elements(&mut numer, 1.0, nn - mm - c_denom_upper, nn - mm + 1.0);
                c_denom_var_lower = nn - mm - 2.0 * (n - x) + 1.0;
                c_denom_upper = n - x;
            }
        } else {
            // Case 4
            if nn - n < nn - mm + 1.0 {
                put_factorial_elements(&mut numer, 0.0, c_num_var_upper, nn - n);
                put_factorial_elements(&mut denom, 0.0, mm - 1.0, nn);
            } else {
                put_factorial_elements(&mut numer, mm - n, c_num_var_upper, nn - n);
                put_factorial_elements(&mut denom, 0.0, n - 1.0, nn);
            }
            if n - x + 1.0 > c_denom_upper {
                put_factorial_elements(
                    &mut numer,
                    nn - 2.0 * mm + 1.0,
                    nn - mm - n + x,
                    nn - mm + 1.0,
                );
            } else if mm >= c_denom_upper {
                put_factorial_elements(
                    &mut numer,
                    nn - 2.0 * mm + 1.0,
                    nn - mm - c_denom_upper,
                    nn - mm + 1.0,
                );
                c_denom_upper = n - x;
                c_denom_var_lower = nn - mm - 2.0 * (n - x) + 1.0;
            } else {
                put_factorial_elements(
                    &mut denom,
                    c_denom_var_lower,
                    nn - n - 2.0 * mm + x,
                    nn - n - mm + x + 1.0,
                );
                c_denom_upper = n - x;
                c_denom_var_lower = nn - mm - 2.0 * (n - x) + 1.0;
            }
        }
        d_num_var_lower = 0.0;
    }

    let d_num_var_upper = if c_denom_upper < x + 1.0 {
        n - x - 1.0
    } else {
        n - c_denom_upper - 1.0
    };
    let d_denom_var_lower = if c_denom_upper < x + 1.0 {
        c_denom_var_lower
    } else {
        nn - n - mm + 1.0
    };
    put_factorial_elements(&mut numer, d_num_var_lower, d_num_var_upper, n);
    put_factorial_elements(
        &mut denom,
        d_denom_var_lower,
        nn - n - mm + x,
        nn - n - mm + x + 1.0,
    );

    numer.sort_by(f64::total_cmp);
    denom.sort_by(f64::total_cmp);
    let mut it1 = numer.iter().rev();
    let mut it2 = denom.iter().rev();
    let mut factor = 1.0;
    loop {
        let (e, d) = (it1.next(), it2.next());
        if e.is_none() && d.is_none() {
            break;
        }
        factor *= e.copied().unwrap_or(1.0) / d.copied().unwrap_or(1.0);
    }
    Ok(factor)
}

/// AOO `ScNegBinomDist`: `x` and `r` are not floored (the product runs
/// while `i < x`); `r < 0`, `x < 0` or `p` outside [0, 1] is `Err:502`.
fn negbinomdist(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let x = a.num(0)?;
        let r = a.num(1)?;
        let p = a.num(2)?;
        if r < 0.0 || x < 0.0 || !(0.0..=1.0).contains(&p) {
            return Err(ErrorKind::InvalidArgument);
        }
        let q = 1.0 - p;
        let mut factor = p.powf(r);
        let mut i = 0.0;
        // Stopping once the product is 0 only saves time.
        while i < x && factor != 0.0 && factor.is_finite() {
            factor *= (i + r) / (i + 1.0) * q;
            i += 1.0;
        }
        Ok(Value::Number(factor))
    })
}

/// AOO `ScPoissonDist`: `x` floored; `lambda < 0` or `x < 0` is `Err:502`.
/// Cumulative by default.
fn poisson(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let x = floor_num(&mut a, 0)?;
        let lambda = a.num(1)?;
        let cumulative = opt_bool(&mut a, 2, true)?;
        if lambda < 0.0 || x < 0.0 {
            return Err(ErrorKind::InvalidArgument);
        }
        if !cumulative {
            if lambda == 0.0 {
                return Ok(Value::Number(0.0));
            }
            if lambda > 712.0 {
                // exp(-lambda) underflows; accuracy about 11 digits.
                return Ok(Value::Number(
                    (x * lambda.ln() - lambda - log_gamma(x + 1.0)).exp(),
                ));
            }
            let mut var: f64 = 1.0;
            let mut f: f64 = 0.0;
            while f < x && var != 0.0 && var.is_finite() {
                var *= lambda / (f + 1.0);
                f += 1.0;
            }
            return Ok(Value::Number(var * (-lambda).exp()));
        }
        if lambda == 0.0 {
            return Ok(Value::Number(1.0));
        }
        if lambda > 712.0 {
            return Ok(Value::Number(up_reg_igamma(x + 1.0, lambda)?));
        }
        if x >= 936.0 {
            // Indistinguishable from 1.
            return Ok(Value::Number(1.0));
        }
        let mut summand = (-lambda).exp();
        let mut sum = summand;
        let end = x as i32;
        for i in 1..=end {
            summand = summand * lambda / f64::from(i);
            sum += summand;
        }
        Ok(Value::Number(sum))
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(B);
    r.register(BINOMDIST);
    r.register(CRITBINOM);
    r.register(HYPGEOMDIST);
    r.register(NEGBINOMDIST);
    r.register(POISSON);
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

    const BAD: Value = Value::Error(ErrorKind::InvalidArgument);

    #[test]
    fn binomial() {
        let mut c = MockEvalContext::new();
        approx(
            call(
                &mut c,
                "BINOMDIST",
                vec![n(3.0), n(10.0), n(0.5), EvalValue::bool(false)],
            ),
            0.117_187_5,
            1e-15,
        );
        approx(
            call(
                &mut c,
                "BINOMDIST",
                vec![n(3.0), n(10.0), n(0.5), EvalValue::bool(true)],
            ),
            0.171_875,
            1e-15,
        );
        assert_eq!(
            call(&mut c, "BINOMDIST", vec![n(11.0), n(10.0), n(0.5), n(0.0)]),
            BAD
        );
        approx(
            call(&mut c, "B", vec![n(10.0), n(0.5), n(3.0)]),
            0.117_187_5,
            1e-15,
        );
        approx(
            call(&mut c, "B", vec![n(10.0), n(0.5), n(0.0), n(3.0)]),
            0.171_875,
            1e-15,
        );
        assert_eq!(
            call(&mut c, "B", vec![n(10.0), n(0.5), n(4.0), n(3.0)]),
            BAD
        );
        assert_eq!(
            call(&mut c, "CRITBINOM", vec![n(10.0), n(0.5), n(0.75)]),
            Value::Number(6.0)
        );
        assert_eq!(
            call(&mut c, "CRITBINOM", vec![n(10.0), n(0.5), n(1.0)]),
            BAD
        );
        approx(
            call(
                &mut c,
                "BINOMDIST",
                vec![n(500.0), n(2000.0), n(0.25), n(1.0)],
            ),
            0.512_014_360_168_171_8,
            1e-12,
        );
    }

    #[test]
    fn others() {
        let mut c = MockEvalContext::new();
        approx(
            call(&mut c, "HYPGEOMDIST", vec![n(1.0), n(4.0), n(8.0), n(20.0)]),
            1760.0 / 4845.0,
            1e-14,
        );
        assert_eq!(
            call(&mut c, "HYPGEOMDIST", vec![n(5.0), n(4.0), n(8.0), n(20.0)]),
            BAD
        );
        approx(
            call(&mut c, "NEGBINOMDIST", vec![n(2.0), n(3.0), n(0.5)]),
            0.1875,
            1e-15,
        );
        approx(
            call(
                &mut c,
                "POISSON",
                vec![n(2.0), n(3.0), EvalValue::bool(false)],
            ),
            0.224_041_807_655_387_75,
            1e-14,
        );
        approx(
            call(&mut c, "POISSON", vec![n(2.0), n(3.0)]),
            0.423_190_081_126_843_5,
            1e-14,
        );
        assert_eq!(call(&mut c, "POISSON", vec![n(-1.0), n(3.0)]), BAD);
    }
}
