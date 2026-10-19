//! Addin functions: dates family (Analysis and Date add-ins).

use crate::registry::Registry;

mod datefunc;
mod workdays;
mod yearfrac;

use tally_numfmt::date::{civil_from_days, epoch_offset, unix_days_from_serial};
use tally_types::{DateEpoch, ErrorKind};

/// Day number (days since 1970-01-01) of `floor(serial)`; out-of-range
/// dates are `Err:502`, as in AOO's add-ins.
pub(crate) fn day_of(epoch: DateEpoch, serial: f64) -> Result<i64, ErrorKind> {
    unix_days_from_serial(epoch, serial).ok_or(ErrorKind::InvalidArgument)
}

/// Serial of a day number, `Err:502` if the year leaves 1..=9999.
pub(crate) fn serial_of(epoch: DateEpoch, days: i64) -> Result<f64, ErrorKind> {
    let (y, _, _) = civil_from_days(days);
    if !(1..=9999).contains(&y) {
        return Err(ErrorKind::InvalidArgument);
    }
    Ok((days - epoch_offset(epoch)) as f64)
}

/// Registers the dates functions.
pub fn register(r: &mut Registry) {
    workdays::register(r);
    yearfrac::register(r);
    datefunc::register(r);
}
