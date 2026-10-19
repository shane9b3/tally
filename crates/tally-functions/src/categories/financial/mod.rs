//! Financial functions (AOO built-ins).
//!
//! - [`tvm`]: time value of money (PV, FV, PMT, NPER, RATE, IPMT, PPMT,
//!   CUMIPMT, CUMPRINC, ISPMT, EFFECTIVE, NOMINAL, DURATION, ZGZ);
//! - [`depreciation`]: SLN, SYD, DB, DDB, VDB;
//! - [`cashflow`]: NPV, IRR, MIRR.
//!
//! Ported from Apache OpenOffice `sc/source/core/tool/interpr2.cxx`
//! (Apache License 2.0).

use tally_types::ErrorKind;

use crate::registry::Registry;
use crate::Args;

mod cashflow;
mod depreciation;
pub(crate) mod tvm;

/// Registers the Financial functions.
pub fn register(r: &mut Registry) {
    tvm::register(r);
    depreciation::register(r);
    cashflow::register(r);
}

/// AOO's handling of optional numeric parameters: an argument written in
/// the call (even an empty one, `f(1;2;3;)`) is read with `GetDouble`, so an
/// empty slot counts as 0; only arguments past the end take `default`
/// (AOO tests `nParamCount`, not `IsMissing`).
pub(super) fn opt_num(a: &mut Args<'_>, i: usize, default: f64) -> Result<f64, ErrorKind> {
    if i < a.count() {
        a.num(i)
    } else {
        Ok(default)
    }
}
