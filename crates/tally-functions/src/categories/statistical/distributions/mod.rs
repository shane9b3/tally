//! Statistical functions: distributions family.
//!
//! Algorithms ported from Apache OpenOffice (interpr3.cxx), Apache License 2.0.

use tally_types::ErrorKind;

use crate::coerce::approx_floor;
use crate::registry::Registry;
use crate::Args;

mod chi_f_t;
mod discrete;
mod gamma_beta;
mod hypothesis;
mod misc;
mod normal;
mod special;

/// Registers the distributions functions.
pub fn register(r: &mut Registry) {
    normal::register(r);
    gamma_beta::register(r);
    chi_f_t::register(r);
    discrete::register(r);
    misc::register(r);
    hypothesis::register(r);
}

/// Optional number with AOO parameter-count semantics: an argument that is
/// written but empty (`f(1;;2)`, `f(1;)`) counts as present and reads as 0;
/// only arguments past the end of the list take `default`.
fn opt_num(a: &mut Args<'_>, i: usize, default: f64) -> Result<f64, ErrorKind> {
    if i < a.count() {
        a.num(i)
    } else {
        Ok(default)
    }
}

/// Optional boolean with AOO parameter-count semantics (see [`opt_num`]):
/// a written but empty argument is FALSE.
fn opt_bool(a: &mut Args<'_>, i: usize, default: bool) -> Result<bool, ErrorKind> {
    if i < a.count() {
        a.bool(i)
    } else {
        Ok(default)
    }
}

/// Argument `i` as a number passed through `rtl::math::approxFloor`
/// (AOO reads integer parameters of these functions with `GetDouble`, so
/// values beyond the 32-bit range are accepted).
fn floor_num(a: &mut Args<'_>, i: usize) -> Result<f64, ErrorKind> {
    Ok(approx_floor(a.num(i)?))
}
