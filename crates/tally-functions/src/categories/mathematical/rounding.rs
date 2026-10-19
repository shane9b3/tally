//! ROUND (later: ROUNDUP, ROUNDDOWN, TRUNC, INT, MROUND, EVEN, ODD).

use tally_types::{ErrorKind, Value};

use crate::coerce::approx_floor;
use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const ROUND_PARAMS: &[Param] = &[
    Param::required("number", ParamKind::Number),
    Param::optional("count", ParamKind::Integer),
];

/// `ROUND(number; [count])`.
pub const ROUND: FunctionDef =
    FunctionDef::new("ROUND", Category::Mathematical, ROUND_PARAMS, round)
        .help("Rounds a number to a predefined accuracy.");

/// 10^1 ..= 10^20 and 10^-1 ..= 10^-20 as literals (AOO `n10s`), so the
/// scale factors are the correctly rounded constants.
const POW10: [f64; 21] = [
    1e0, 1e1, 1e2, 1e3, 1e4, 1e5, 1e6, 1e7, 1e8, 1e9, 1e10, 1e11, 1e12, 1e13, 1e14, 1e15, 1e16,
    1e17, 1e18, 1e19, 1e20,
];
const POW10_NEG: [f64; 21] = [
    1e0, 1e-1, 1e-2, 1e-3, 1e-4, 1e-5, 1e-6, 1e-7, 1e-8, 1e-9, 1e-10, 1e-11, 1e-12, 1e-13, 1e-14,
    1e-15, 1e-16, 1e-17, 1e-18, 1e-19, 1e-20,
];

/// Correction added before flooring, indexed by `15 - exponent` (AOO `nKorrVal`).
const KORR: [f64; 16] = [
    0.0, 9e-1, 9e-2, 9e-3, 9e-4, 9e-5, 9e-6, 9e-7, 9e-8, 9e-9, 9e-10, 9e-11, 9e-12, 9e-13, 9e-14,
    9e-15,
];

/// Rounds half away from zero to `places` decimals (negative: tens,
/// hundreds, …), with AOO's binary-representation correction so that
/// `2.675 → 2.68` and `1.005 → 1.01`. `places` outside `-20..=20` returns
/// `x` unchanged.
///
/// Ported from Apache OpenOffice `sal/rtl/source/math.cxx`
/// (`rtl_math_round`, `rtl_math_RoundingMode_Corrected`), Apache-2.0.
pub(crate) fn round_corrected(x: f64, places: i32) -> f64 {
    if x == 0.0 || !x.is_finite() {
        return x;
    }
    let neg = x.is_sign_negative();
    let mut v = x.abs();
    let fac = match places {
        0 => 1.0,
        1..=20 => POW10[places as usize],
        -20..=-1 => POW10_NEG[places.unsigned_abs() as usize],
        _ => return x,
    };
    if places != 0 {
        if v > f64::MAX / 1e20 {
            return x;
        }
        v *= fac;
    }
    let exp = if v > 0.0 { v.log10().floor() as i32 } else { 0 };
    let index = (15 - exp).clamp(0, 15);
    let index = if index <= 1 { 0 } else { index as usize };
    v = (v + 0.5 + KORR[index]).floor();
    if places != 0 {
        v /= fac;
    }
    if neg {
        -v
    } else {
        v
    }
}

fn round(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let x = a.num(0)?;
        // AOO ScInterpreter::RoundNumber: count = approxFloor(count), and
        // anything outside -20..=20 is an illegal argument.
        let places = approx_floor(a.num_opt(1, 0.0)?);
        if !(-20.0..=20.0).contains(&places) {
            return Err(ErrorKind::InvalidArgument);
        }
        Ok(Value::Number(round_corrected(x, places as i32)))
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(ROUND);
}

#[cfg(test)]
mod tests {
    use super::round_corrected;

    #[test]
    fn corrected_rounding() {
        assert_eq!(round_corrected(2.675, 2), 2.68);
        assert_eq!(round_corrected(1.005, 2), 1.01);
        assert_eq!(round_corrected(-2.5, 0), -3.0);
        assert_eq!(round_corrected(2.5, 0), 3.0);
        assert_eq!(round_corrected(2.4999, 0), 2.0);
        assert_eq!(round_corrected(1234.5678, -2), 1200.0);
        assert_eq!(round_corrected(-1250.0, -2), -1300.0);
        assert_eq!(round_corrected(0.125, 2), 0.13);
        assert_eq!(round_corrected(1e300, 5), 1e300);
        assert_eq!(round_corrected(5.0, -1), 10.0);
        assert_eq!(round_corrected(4.0, -1), 0.0);
    }
}
