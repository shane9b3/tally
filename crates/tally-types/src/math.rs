//! Numeric helpers with OpenOffice semantics.

/// Relative tolerance used by [`approx_equal`]: 2^-48, as in
/// `rtl::math::approxEqual` (`1.0 / (16777216.0 * 16777216.0)`).
const APPROX_EPS: f64 = 1.0 / (16_777_216.0 * 16_777_216.0);

/// Approximate equality as used by spreadsheet comparisons.
///
/// Follows OpenOffice's `rtl::math::approxEqual`: two numbers are equal when
/// they are identical or their difference is below 2^-48 relative to their
/// magnitude. We use the smaller magnitude, which makes the test symmetric.
///
/// - `+0.0` and `-0.0` are equal.
/// - A non-zero number never equals zero.
/// - `NaN` never equals anything, including itself.
/// - Infinities are equal only to themselves.
pub fn approx_equal(a: f64, b: f64) -> bool {
    if a == b {
        return true;
    }
    if !a.is_finite() || !b.is_finite() {
        return false;
    }
    (a - b).abs() < a.abs().min(b.abs()) * APPROX_EPS
}

/// Rounds to 15 significant decimal digits, the precision spreadsheets
/// display (`0.1 + 0.2` becomes `0.3`).
///
/// Zero, `NaN` and infinities are returned unchanged.
pub fn round_to_15_digits(x: f64) -> f64 {
    if x == 0.0 || !x.is_finite() {
        return x;
    }
    // `{:.14e}` yields 15 significant digits, correctly rounded from the
    // exact binary value; parsing it back gives the nearest f64.
    // Values near f64::MAX can round up past it; keep the original then.
    match format!("{x:.14e}").parse::<f64>() {
        Ok(r) if r.is_finite() => r,
        _ => x,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approx_equal_cases() {
        assert!(approx_equal(0.1 + 0.2, 0.3));
        assert!(!approx_equal(1.0, 1.0 + 1e-10));
        assert!(approx_equal(1.0, 1.0));
        assert!(approx_equal(0.0, -0.0));
        assert!(!approx_equal(0.0, 1e-300));
        assert!(!approx_equal(f64::NAN, f64::NAN));
        assert!(!approx_equal(f64::NAN, 1.0));
        assert!(approx_equal(f64::INFINITY, f64::INFINITY));
        assert!(!approx_equal(f64::INFINITY, f64::NEG_INFINITY));
        assert!(!approx_equal(f64::INFINITY, f64::MAX));
        assert!(approx_equal(-1e300, -1e300 * (1.0 + 1e-16)));
        assert!(!approx_equal(-1.0, 1.0));
        // Symmetric.
        let (a, b) = (1e20, 1e20 + 1e5);
        assert_eq!(approx_equal(a, b), approx_equal(b, a));
    }

    #[test]
    fn round_to_15_digits_cases() {
        assert_eq!(round_to_15_digits(0.1 + 0.2), 0.3);
        assert_eq!(round_to_15_digits(1.0 - 0.9), 0.1);
        assert_eq!(round_to_15_digits(1.0 / 3.0), 0.333_333_333_333_333);
        assert_eq!(round_to_15_digits(2.0 / 3.0 * 1e10), 6_666_666_666.666_67);
        assert_eq!(round_to_15_digits(-2.5), -2.5);
        assert_eq!(round_to_15_digits(1e-320), 1e-320);
        assert_eq!(round_to_15_digits(0.0).to_bits(), 0.0f64.to_bits());
        assert_eq!(round_to_15_digits(-0.0).to_bits(), (-0.0f64).to_bits());
        assert!(round_to_15_digits(f64::NAN).is_nan());
        assert_eq!(round_to_15_digits(f64::INFINITY), f64::INFINITY);
        assert_eq!(
            round_to_15_digits(f64::MAX),
            f64::MAX,
            "no overflow to infinity"
        );
    }
}
