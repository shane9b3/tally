//! Special functions shared by the distribution functions: gamma and log-gamma
//! (Lanczos), beta, the regularized incomplete beta and gamma functions,
//! the t, F and chi-square tails, the binomial mass function and the
//! inverse-iteration solver used by every `*INV` function.
//!
//! Algorithms ported from Apache OpenOffice (interpr3.cxx), Apache License 2.0.
//! Also from AOO `sc/source/core/tool/interpr6.cxx` (incomplete gamma),
//! Apache License 2.0. The normal distribution and erf/erfc are in
//! [`crate::stats_util`].

// The coefficients are copied verbatim from the AOO sources (some carry more
// digits than an f64 holds) so they can be checked against the original.
#![allow(clippy::excessive_precision)]

use tally_types::ErrorKind;

use crate::coerce::approx_floor;

/// Largest argument for which Γ(x) does not overflow (AOO `fMaxGammaArgument`).
pub(super) const MAX_GAMMA_ARGUMENT: f64 = 171.624376956302;

const MACH_EPS: f64 = f64::EPSILON;
const HALF_MACH_EPS: f64 = 0.5 * f64::EPSILON;
/// The Lanczos `g` of the lanczos13m53 approximation.
const LANCZOS_G: f64 = 6.024_680_040_776_729_583_740_234_375;

// Normal distribution and erf/erfc live in `crate::stats_util` (shared with
// GAUSS/PHI and the add-in ERF/ERFC); re-exported for this family.
pub(super) use crate::stats_util::{gauss, gaussinv, integral_phi, phi};

// ---------------------------------------------------------------------------
// Gamma and beta functions
// ---------------------------------------------------------------------------

/// Lanczos sum with a rational function (boost lanczos13m53). `z > 0`.
fn lanczos_sum(z: f64) -> f64 {
    const NUM: [f64; 13] = [
        23_531_376_880.410_759_688_572_007_674_451_636_754_73,
        42_919_803_642.649_098_768_957_899_047_001_988_850_93,
        35_711_959_237.355_668_049_440_185_451_547_166_705_96,
        17_921_034_426.037_209_699_919_755_754_458_931_112_67,
        6_039_542_586.352_028_005_064_291_644_307_297_921_07,
        1_439_720_407.311_721_673_663_223_072_794_912_393_972,
        248_874_557.862_054_156_511_460_386_413_229_423_216_3,
        31_426_415.585_400_194_380_614_231_628_318_205_362_87,
        2_876_370.628_935_372_441_225_409_051_620_849_613_599,
        186_056.265_395_223_495_040_294_989_716_045_699_282_2,
        8_071.672_002_365_816_210_638_002_902_272_250_613_822,
        210.824_277_751_579_345_872_509_733_920_713_362_711_7,
        2.506_628_274_631_000_270_164_908_177_133_837_338_626,
    ];
    const DENOM: [f64; 13] = [
        0.0,
        39_916_800.0,
        120_543_840.0,
        150_917_976.0,
        105_258_076.0,
        45_995_730.0,
        13_339_535.0,
        2_637_558.0,
        357_423.0,
        32_670.0,
        1_925.0,
        66.0,
        1.0,
    ];
    let (mut num, mut den);
    if z <= 1.0 {
        num = NUM[12];
        den = DENOM[12];
        for i in (0..12).rev() {
            num = num * z + NUM[i];
            den = den * z + DENOM[i];
        }
    } else {
        // Cancel down with z^12; Horner scheme with reversed coefficients.
        let zinv = 1.0 / z;
        num = NUM[0];
        den = DENOM[0];
        for i in 1..13 {
            num = num * zinv + NUM[i];
            den = den * zinv + DENOM[i];
        }
    }
    num / den
}

/// Γ(z) for `0 < z <= MAX_GAMMA_ARGUMENT` (boost tgamma).
fn gamma_helper(z: f64) -> f64 {
    let mut g = lanczos_sum(z);
    let zgh = z + LANCZOS_G - 0.5;
    // Avoid intermediate overflow.
    let half_power = zgh.powf(z / 2.0 - 0.25);
    g *= half_power;
    g /= zgh.exp();
    g *= half_power;
    if z <= 20.0 && z == approx_floor(z) {
        g = g.round();
    }
    g
}

/// ln Γ(z) for `z > 0`.
fn log_gamma_helper(z: f64) -> f64 {
    let zgh = z + LANCZOS_G - 0.5;
    lanczos_sum(z).ln() + (z - 0.5) * zgh.ln() - zgh
}

/// Γ(z) (AOO `GetGamma`). Overflow is `#NUM!`; the caller must reject
/// non-positive integers.
pub(super) fn gamma(z: f64) -> Result<f64, ErrorKind> {
    let log_pi = std::f64::consts::PI.ln();
    let log_dbl_max = f64::MAX.ln();
    if z > MAX_GAMMA_ARGUMENT {
        return Err(ErrorKind::Num);
    }
    if z >= 1.0 {
        return Ok(gamma_helper(z));
    }
    if z >= 0.5 {
        return Ok(gamma_helper(z + 1.0) / z);
    }
    if z >= -0.5 {
        let log_test = log_gamma_helper(z + 2.0) - (z + 1.0).ln() - z.abs().ln();
        if log_test >= log_dbl_max {
            return Err(ErrorKind::Num);
        }
        return Ok(gamma_helper(z + 2.0) / (z + 1.0) / z);
    }
    // Euler's reflection formula: Γ(x) = π / (Γ(1-x) sin(πx)).
    let s = (std::f64::consts::PI * z).sin();
    let log_divisor = log_gamma_helper(1.0 - z) + s.abs().ln();
    if log_divisor - log_pi >= log_dbl_max {
        return Ok(0.0); // underflow
    }
    if log_divisor < 0.0 && log_pi - log_divisor > log_dbl_max {
        return Err(ErrorKind::Num);
    }
    Ok((log_pi - log_divisor).exp() * if s < 0.0 { -1.0 } else { 1.0 })
}

/// ln Γ(z) for `z > 0` (AOO `GetLogGamma`).
pub(super) fn log_gamma(z: f64) -> f64 {
    if z >= MAX_GAMMA_ARGUMENT {
        return log_gamma_helper(z);
    }
    if z >= 1.0 {
        return gamma_helper(z).ln();
    }
    if z >= 0.5 {
        return (gamma_helper(z + 1.0) / z).ln();
    }
    log_gamma_helper(z + 2.0) - (z + 1.0).ln() - z.ln()
}

/// B(a, b) for `a, b > 0` (AOO `GetBeta`).
fn beta(alpha: f64, beta: f64) -> f64 {
    let (a, b) = if alpha > beta {
        (alpha, beta)
    } else {
        (beta, alpha)
    };
    if a + b < MAX_GAMMA_ARGUMENT {
        // Arguments are positive and below the overflow limit.
        return gamma_helper_any(a) / gamma_helper_any(a + b) * gamma_helper_any(b);
    }
    let gm = LANCZOS_G - 0.5;
    let mut lanczos = lanczos_sum(a);
    lanczos /= lanczos_sum(a + b);
    lanczos *= lanczos_sum(b);
    let abgm = a + b + gm;
    lanczos *= ((abgm / (a + gm)) / (b + gm)).sqrt();
    let temp_a = b / (a + gm);
    let temp_b = a / (b + gm);
    let result = (-a * temp_a.ln_1p() - b * temp_b.ln_1p() - gm).exp();
    result * lanczos
}

/// [`gamma`] for a positive argument below [`MAX_GAMMA_ARGUMENT`] (cannot fail).
fn gamma_helper_any(z: f64) -> f64 {
    gamma(z).unwrap_or(f64::INFINITY)
}

/// ln B(a, b) for `a, b > 0` (AOO `GetLogBeta`).
fn log_beta(alpha: f64, beta: f64) -> f64 {
    let (a, b) = if alpha > beta {
        (alpha, beta)
    } else {
        (beta, alpha)
    };
    let gm = LANCZOS_G - 0.5;
    let mut lanczos = lanczos_sum(a);
    lanczos /= lanczos_sum(a + b);
    lanczos *= lanczos_sum(b);
    let mut log_lanczos = lanczos.ln();
    let abgm = a + b + gm;
    log_lanczos += 0.5 * (abgm.ln() - (a + gm).ln() - (b + gm).ln());
    let temp_a = b / (a + gm);
    let temp_b = a / (b + gm);
    -a * temp_a.ln_1p() - b * temp_b.ln_1p() - gm + log_lanczos
}

/// Beta density on [0, 1] (AOO `GetBetaDistPDF`). A pole at an end point
/// is `Err:502`.
pub(super) fn beta_dist_pdf(x: f64, a: f64, b: f64) -> Result<f64, ErrorKind> {
    if a == 1.0 {
        // b (1-x)^(b-1)
        if b == 1.0 {
            return Ok(1.0);
        }
        if b == 2.0 {
            return Ok(-2.0 * x + 2.0);
        }
        if x == 1.0 && b < 1.0 {
            return Err(ErrorKind::InvalidArgument);
        }
        if x <= 0.01 {
            return Ok(b + b * ((b - 1.0) * (-x).ln_1p()).exp_m1());
        }
        return Ok(b * (0.5 - x + 0.5).powf(b - 1.0));
    }
    if b == 1.0 {
        // a x^(a-1)
        if a == 2.0 {
            return Ok(a * x);
        }
        if x == 0.0 && a < 1.0 {
            return Err(ErrorKind::InvalidArgument);
        }
        return Ok(a * x.powf(a - 1.0));
    }
    if x <= 0.0 {
        if a < 1.0 && x == 0.0 {
            return Err(ErrorKind::InvalidArgument);
        }
        return Ok(0.0);
    }
    if x >= 1.0 {
        if b < 1.0 && x == 1.0 {
            return Err(ErrorKind::InvalidArgument);
        }
        return Ok(0.0);
    }
    // x^(a-1) (1-x)^(b-1) / B(a,b)
    let log_dbl_max = f64::MAX.ln();
    let log_dbl_min = f64::MIN_POSITIVE.ln();
    let log_y = if x < 0.1 {
        (-x).ln_1p()
    } else {
        (0.5 - x + 0.5).ln()
    };
    let log_x = x.ln();
    let am1_log_x = (a - 1.0) * log_x;
    let bm1_log_y = (b - 1.0) * log_y;
    let lb = log_beta(a, b);
    let inside = |v: f64| v < log_dbl_max && v > log_dbl_min;
    if inside(am1_log_x) && inside(bm1_log_y) && inside(lb) && inside(am1_log_x + bm1_log_y) {
        Ok(x.powf(a - 1.0) * (0.5 - x + 0.5).powf(b - 1.0) / beta(a, b))
    } else {
        Ok((am1_log_x + bm1_log_y - lb).exp())
    }
}

/// Continued fraction of the incomplete beta function:
/// `I_x(a,b) = x^a (1-x)^b / (a B(a,b)) * cf`.
fn beta_helper_cont_frac(x: f64, a: f64, b: f64) -> f64 {
    let (mut a1, mut b1) = (1.0, 1.0);
    let mut b2 = 1.0 - (a + b) / (a + 1.0) * x;
    let (mut a2, mut fnorm, mut cf);
    if b2 == 0.0 {
        a2 = 0.0;
        fnorm = 1.0;
        cf = 1.0;
    } else {
        a2 = 1.0;
        fnorm = 1.0 / b2;
        cf = a2 * fnorm;
    }
    let mut cfnew = 1.0;
    let mut rm = 1.0;
    const MAX_ITER: f64 = 50000.0;
    loop {
        let apl2m = a + 2.0 * rm;
        let d2m = rm * (b - rm) * x / ((apl2m - 1.0) * apl2m);
        let d2m1 = -(a + rm) * (a + b + rm) * x / (apl2m * (apl2m + 1.0));
        a1 = (a2 + d2m * a1) * fnorm;
        b1 = (b2 + d2m * b1) * fnorm;
        a2 = a1 + d2m1 * a2 * fnorm;
        b2 = b1 + d2m1 * b2 * fnorm;
        let mut finished = false;
        if b2 != 0.0 {
            fnorm = 1.0 / b2;
            cfnew = a2 * fnorm;
            finished = (cf - cfnew).abs() < cf.abs() * MACH_EPS;
        }
        cf = cfnew;
        rm += 1.0;
        if rm >= MAX_ITER || finished {
            break;
        }
    }
    cf
}

/// Regularized incomplete beta function `I_x(a, b)` (AOO `GetBetaDist`).
pub(super) fn beta_dist(x_in: f64, alpha: f64, beta_p: f64) -> f64 {
    if x_in <= 0.0 {
        return 0.0;
    }
    if x_in >= 1.0 {
        return 1.0;
    }
    if beta_p == 1.0 {
        return x_in.powf(alpha);
    }
    if alpha == 1.0 {
        return -(beta_p * (-x_in).ln_1p()).exp_m1();
    }
    let mut y = (0.5 - x_in) + 0.5;
    let mut ln_y = (-x_in).ln_1p();
    let mut x = x_in;
    let mut ln_x = x_in.ln();
    let (mut a, mut b) = (alpha, beta_p);
    let reflect = x_in > alpha / (alpha + beta_p);
    if reflect {
        a = beta_p;
        b = alpha;
        x = y;
        y = x_in;
        ln_x = ln_y;
        ln_y = x_in.ln();
    }
    let mut result = beta_helper_cont_frac(x, a, b) / a;
    let p = a / (a + b);
    let q = b / (a + b);
    let temp = if a > 1.0 && b > 1.0 && p < 0.97 && q < 0.97 {
        // a, b > 1 and 0 < x < 1: no pole, cannot fail.
        beta_dist_pdf(x, a, b).unwrap_or(f64::NAN) * x * y
    } else {
        (a * ln_x + b * ln_y - log_beta(a, b)).exp()
    };
    result *= temp;
    if reflect {
        result = 0.5 - result + 0.5;
    }
    result.clamp(0.0, 1.0)
}

// ---------------------------------------------------------------------------
// Incomplete gamma (interpr6.cxx, after Cephes)
// ---------------------------------------------------------------------------

/// Continued fraction for the upper incomplete gamma; valid for `x > a + 1`.
fn gamma_cont_fraction(a: f64, x: f64) -> Result<f64, ErrorKind> {
    let big_inv = f64::EPSILON;
    let big = 1.0 / big_inv;
    let mut count = 0.0;
    let mut y = 1.0 - a;
    let mut denom = x + 2.0 - a;
    let mut pkm1 = x + 1.0;
    let mut pkm2 = 1.0;
    let mut qkm1 = denom * x;
    let mut qkm2 = x;
    let mut approx = pkm1 / qkm1;
    let mut finished = false;
    while !finished && count < 10000.0 {
        count += 1.0;
        y += 1.0;
        let num = y * count;
        denom += 2.0;
        let pk = pkm1 * denom - pkm2 * num;
        let qk = qkm1 * denom - qkm2 * num;
        if qk != 0.0 {
            let r = pk / qk;
            finished = ((approx - r) / r).abs() <= HALF_MACH_EPS;
            approx = r;
        }
        pkm2 = pkm1;
        pkm1 = pk;
        qkm2 = qkm1;
        qkm1 = qk;
        if pk.abs() > big {
            pkm2 *= big_inv;
            pkm1 *= big_inv;
            qkm2 *= big_inv;
            qkm1 *= big_inv;
        }
    }
    if finished {
        Ok(approx)
    } else {
        Err(ErrorKind::NoConvergence)
    }
}

/// Power series for the lower incomplete gamma; valid for `x <= a + 1`.
fn gamma_series(a: f64, x: f64) -> Result<f64, ErrorKind> {
    let mut denom_factor = a;
    let mut summand = 1.0 / a;
    let mut sum = summand;
    let mut count = 1;
    loop {
        denom_factor += 1.0;
        summand = summand * x / denom_factor;
        sum += summand;
        count += 1;
        if !(summand / sum > HALF_MACH_EPS && count <= 10000) {
            break;
        }
    }
    if count > 10000 {
        Err(ErrorKind::NoConvergence)
    } else {
        Ok(sum)
    }
}

/// Regularized lower incomplete gamma `P(a, x)`, `a, x > 0`.
pub(super) fn low_reg_igamma(a: f64, x: f64) -> Result<f64, ErrorKind> {
    let factor = (a * x.ln() - x - log_gamma(a)).exp();
    if x > a + 1.0 {
        Ok(1.0 - factor * gamma_cont_fraction(a, x)?)
    } else {
        Ok(factor * gamma_series(a, x)?)
    }
}

/// Regularized upper incomplete gamma `Q(a, x)`, `a, x > 0`.
pub(super) fn up_reg_igamma(a: f64, x: f64) -> Result<f64, ErrorKind> {
    let factor = (a * x.ln() - x - log_gamma(a)).exp();
    if x > a + 1.0 {
        Ok(factor * gamma_cont_fraction(a, x)?)
    } else {
        Ok(1.0 - factor * gamma_series(a, x)?)
    }
}

/// Gamma density with shape `alpha` and scale `lambda` (both > 0).
pub(super) fn gamma_dist_pdf(x: f64, alpha: f64, lambda: f64) -> Result<f64, ErrorKind> {
    if x <= 0.0 {
        return Ok(0.0);
    }
    let xr = x / lambda;
    if xr > 1.0 {
        let log_dbl_max = f64::MAX.ln();
        if xr.ln() * (alpha - 1.0) < log_dbl_max && alpha < MAX_GAMMA_ARGUMENT {
            Ok(xr.powf(alpha - 1.0) * (-xr).exp() / lambda / gamma(alpha)?)
        } else {
            Ok(((alpha - 1.0) * xr.ln() - xr - lambda.ln() - log_gamma(alpha)).exp())
        }
    } else if alpha < MAX_GAMMA_ARGUMENT {
        Ok(xr.powf(alpha - 1.0) * (-xr).exp() / lambda / gamma(alpha)?)
    } else {
        Ok(xr.powf(alpha - 1.0) * (-xr).exp() / lambda / log_gamma(alpha).exp())
    }
}

/// Gamma cumulative distribution with shape `alpha` and scale `lambda`.
pub(super) fn gamma_dist(x: f64, alpha: f64, lambda: f64) -> Result<f64, ErrorKind> {
    if x <= 0.0 {
        Ok(0.0)
    } else {
        low_reg_igamma(alpha, x / lambda)
    }
}

// ---------------------------------------------------------------------------
// t, F and chi-square
// ---------------------------------------------------------------------------

/// Right tail of the F distribution (AOO `GetFDist`).
pub(super) fn f_dist(x: f64, f1: f64, f2: f64) -> f64 {
    beta_dist(f2 / (f2 + f1 * x), f2 / 2.0, f1 / 2.0)
}

/// One right tail of Student's t distribution (AOO `GetTDist`).
pub(super) fn t_dist(t: f64, df: f64) -> f64 {
    0.5 * beta_dist(df / (df + t * t), df / 2.0, 0.5)
}

/// Right tail of the chi-square distribution (LEGACY.CHIDIST); `df > 0`.
pub(super) fn chi_dist(x: f64, df: f64) -> Result<f64, ErrorKind> {
    if x <= 0.0 {
        Ok(1.0)
    } else {
        up_reg_igamma(df / 2.0, x / 2.0)
    }
}

/// Left tail of the chi-square distribution (CHISQDIST); `df > 0`.
pub(super) fn chi_sq_dist_cdf(x: f64, df: f64) -> Result<f64, ErrorKind> {
    if x <= 0.0 {
        Ok(0.0)
    } else {
        low_reg_igamma(df / 2.0, x / 2.0)
    }
}

/// Chi-square density; `df` must be a positive integer.
pub(super) fn chi_sq_dist_pdf(x: f64, df: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if df * x > 1_391_000.0 {
        // Intermediate values would be invalid; use logarithms.
        return ((0.5 * df - 1.0) * (x * 0.5).ln() - 0.5 * x - 2f64.ln() - log_gamma(0.5 * df))
            .exp();
    }
    let (mut value, mut count) = if df % 2.0 < 0.5 {
        (0.5, 2.0)
    } else {
        (1.0 / (x * 2.0 * std::f64::consts::PI).sqrt(), 1.0)
    };
    while count < df {
        value *= x / count;
        count += 2.0;
    }
    if x >= 1425.0 {
        // e^(-x/2) underflows
        (value.ln() - x / 2.0).exp()
    } else {
        value * (-x / 2.0).exp()
    }
}

// ---------------------------------------------------------------------------
// Binomial
// ---------------------------------------------------------------------------

/// Binomial mass function; `0 <= x <= n` integral, `0 < p < 1`.
pub(super) fn binom_dist_pmf(x: f64, n: f64, p: f64) -> Result<f64, ErrorKind> {
    let q = (0.5 - p) + 0.5;
    let mut factor = q.powf(n);
    if factor <= f64::MIN_POSITIVE {
        factor = p.powf(n);
        if factor <= f64::MIN_POSITIVE {
            return Ok(beta_dist_pdf(p, x + 1.0, n - x + 1.0)? / (n + 1.0));
        }
        let max = (n - x) as u32;
        let mut i = 0u32;
        while i < max && factor > 0.0 {
            let fi = f64::from(i);
            factor *= (n - fi) / (fi + 1.0) * q / p;
            i += 1;
        }
        return Ok(factor);
    }
    let max = x as u32;
    let mut i = 0u32;
    while i < max && factor > 0.0 {
        let fi = f64::from(i);
        factor *= (n - fi) / (fi + 1.0) * p / q;
        i += 1;
    }
    Ok(factor)
}

/// Sum of binomial terms `xs..=xe` starting from `factor = q^n`
/// (AOO `lcl_GetBinomDistRange`); `0 <= xs < xe <= n` integral.
pub(super) fn binom_dist_range(n: f64, xs: f64, xe: f64, mut factor: f64, p: f64, q: f64) -> f64 {
    let nxs = xs as u32;
    let mut i = 1u32;
    while i <= nxs && factor > 0.0 {
        let fi = f64::from(i);
        factor *= (n - fi + 1.0) / fi * p / q;
        i += 1;
    }
    let mut sum = factor;
    let nxe = xe as u32;
    let mut i = nxs.saturating_add(1);
    while i <= nxe && factor > 0.0 {
        let fi = f64::from(i);
        factor *= (n - fi + 1.0) / fi * p / q;
        sum += factor;
        i += 1;
    }
    sum.min(1.0)
}

// ---------------------------------------------------------------------------
// Inverse iteration
// ---------------------------------------------------------------------------

fn has_change_of_sign(u: f64, w: f64) -> bool {
    (u < 0.0 && w > 0.0) || (u > 0.0 && w < 0.0)
}

/// Finds a root of `f` (AOO `lcl_IterateInverse`): widens `[ax, bx]` until
/// the sign changes (never below 0), then inverse quadratic interpolation
/// with bisection fallback inside the bracket. `None` if no bracket is
/// found (no convergence).
pub(super) fn iterate_inverse(mut f: impl FnMut(f64) -> f64, ax: f64, bx: f64) -> Option<f64> {
    const Y_EPS: f64 = 1.0e-307;
    let x_eps = f64::EPSILON;
    let (mut ax, mut bx) = (ax, bx);

    // Find an enclosing interval.
    let mut ay = f(ax);
    let mut by = f(bx);
    let mut count = 0;
    while count < 1000 && !has_change_of_sign(ay, by) {
        if ay.abs() <= by.abs() {
            let temp = ax;
            ax += 2.0 * (ax - bx);
            if ax < 0.0 {
                ax = 0.0;
            }
            bx = temp;
            by = ay;
            ay = f(ax);
        } else {
            let temp = bx;
            bx += 2.0 * (bx - ax);
            ax = temp;
            ay = by;
            by = f(bx);
        }
        count += 1;
    }
    if ay == 0.0 {
        return Some(ax);
    }
    if by == 0.0 {
        return Some(bx);
    }
    if !has_change_of_sign(ay, by) {
        return None;
    }

    // Inverse quadratic interpolation with additional brackets.
    let (mut px, mut py) = (ax, ay);
    let (mut qx, mut qy) = (bx, by);
    let (mut rx, mut ry) = (ax, ay);
    let mut sx;
    let mut interpolate = true;
    let mut count = 0;
    while count < 500 && ry.abs() > Y_EPS && (bx - ax) > ax.abs().max(bx.abs()) * x_eps {
        sx = 0.5 * (ax + bx);
        if interpolate {
            if py != qy && qy != ry && ry != py {
                sx = px * ry * qy / (ry - py) / (qy - py)
                    + rx * qy * py / (qy - ry) / (py - ry)
                    + qx * py * ry / (py - qy) / (ry - qy);
                interpolate = ax < sx && sx < bx;
            } else {
                interpolate = false;
            }
        }
        if !interpolate {
            sx = 0.5 * (ax + bx);
            // Reset the points (AOO also resets P, which the shift below overwrites).
            qx = bx;
            qy = by;
            interpolate = true;
        }
        // Shift points for the next interpolation.
        px = qx;
        qx = rx;
        rx = sx;
        py = qy;
        qy = ry;
        ry = f(sx);
        // Update the brackets.
        if has_change_of_sign(ay, ry) {
            bx = rx;
            by = ry;
        } else {
            ax = rx;
            ay = ry;
        }
        // Too small an advance: bisect next time, for safety.
        interpolate = interpolate && ry.abs() * 2.0 <= qy.abs();
        count += 1;
    }
    Some(rx)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, rel: f64) -> bool {
        (a - b).abs() <= rel * b.abs().max(1e-300)
    }

    #[test]
    fn gamma_beta() {
        assert_eq!(gamma(5.0), Ok(24.0));
        assert!(close(
            gamma(0.5).unwrap_or(0.0),
            std::f64::consts::PI.sqrt(),
            1e-15
        ));
        assert!(close(
            gamma(-1.5).unwrap_or(0.0),
            2.363_271_801_207_355,
            1e-14
        ));
        assert_eq!(gamma(200.0), Err(ErrorKind::Num));
        assert!(close(log_gamma(10.0), 12.801_827_480_081_469, 1e-15));
        assert!(close(log_gamma(1000.0), 5_905.220_423_209_181, 1e-14));
        assert!(close(beta_dist(0.5, 2.0, 3.0), 0.6875, 1e-15));
        assert!(close(beta(2.0, 3.0), 1.0 / 12.0, 1e-15));
    }

    #[test]
    fn solver() {
        let r = iterate_inverse(|x| 2.0 - x * x, 0.5, 1.0).unwrap_or(0.0);
        assert!(close(r, std::f64::consts::SQRT_2, 1e-15));
        assert_eq!(iterate_inverse(|_| 1.0, 0.5, 1.0), None);
    }
}
