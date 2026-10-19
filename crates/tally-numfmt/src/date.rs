//! Date serial numbers on the proleptic Gregorian calendar.
//!
//! A serial is a day count (`f64`) relative to the document's epoch, with the
//! time of day as the fractional part. The three epochs are:
//!
//! | [`DateEpoch`] | serial 0   | notes |
//! |---|---|---|
//! | `D1899` | 1899-12-30 | OpenOffice / LibreOffice default |
//! | `D1900` | 1900-01-01 | StarCalc 1.0 convention (AOO "01/01/1900") |
//! | `D1904` | 1904-01-01 | classic Mac |
//!
//! There is no Julian switch (ADR-0001) and no fictitious 1900-02-29.
//! Valid years are 1..=9999; functions return `None` outside that range so
//! callers can raise `Err:502`.
//!
//! The civil-date algorithms are Howard Hinnant's `days_from_civil` /
//! `civil_from_days` (public domain), counting days from 1970-01-01.

use tally_types::DateEpoch;

/// Smallest supported year.
pub const MIN_YEAR: i32 = 1;
/// Largest supported year.
pub const MAX_YEAR: i32 = 9999;

/// Days from 1970-01-01 to the given proleptic Gregorian date.
///
/// Works for any year; `m` must be 1..=12 and `d` is not validated (it is
/// simply added), so callers validate with [`days_in_month`] first.
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let m = i64::from(m);
    let d = i64::from(d);
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400; // [0, 399]
    let mp = (m + 9) % 12; // March = 0
    let doy = (153 * mp + 2) / 5 + d - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe - 719_468
}

/// The proleptic Gregorian date `(year, month, day)` that is `z` days after
/// 1970-01-01. Inverse of [`days_from_civil`].
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Whether `y` is a leap year in the proleptic Gregorian calendar.
pub fn is_leap_year(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

/// Number of days in month `m` (1..=12) of year `y`; 0 for an invalid month.
pub fn days_in_month(y: i64, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(y) => 29,
        2 => 28,
        _ => 0,
    }
}

/// Days from 1970-01-01 to serial 0 of `epoch`.
pub fn epoch_offset(epoch: DateEpoch) -> i64 {
    match epoch {
        DateEpoch::D1899 => -25_569, // 1899-12-30
        DateEpoch::D1900 => -25_567, // 1900-01-01
        DateEpoch::D1904 => -24_107, // 1904-01-01
    }
}

/// Serial of a valid date, or `None` if the year is outside 1..=9999 or the
/// month/day are invalid.
pub fn serial_from_ymd(epoch: DateEpoch, y: i32, m: u32, d: u32) -> Option<f64> {
    if !(MIN_YEAR..=MAX_YEAR).contains(&y) || !(1..=12).contains(&m) {
        return None;
    }
    if d == 0 || d > days_in_month(i64::from(y), m) {
        return None;
    }
    Some((days_from_civil(i64::from(y), m, d) - epoch_offset(epoch)) as f64)
}

/// Serial of a date with month and day overflow normalised, as AOO's
/// `DATE()` does: month 13 is January of the next year, month 0 is December
/// of the previous year, day 0 is the last day of the previous month and
/// day 32 of January is February 1.
///
/// Returns `None` if the resulting date's year is outside 1..=9999. The
/// year itself is used as given (two-digit-year expansion is the caller's
/// job).
pub fn serial_from_ymd_normalized(epoch: DateEpoch, y: i64, m: i64, d: i64) -> Option<f64> {
    if y.abs() > 100_000 || m.abs() > 1_200_000 || d.abs() > 100_000_000 {
        return None;
    }
    let m0 = m - 1;
    let y = y + m0.div_euclid(12);
    let m = (m0.rem_euclid(12) + 1) as u32;
    let days = days_from_civil(y, m, 1) + (d - 1);
    let (ry, _, _) = civil_from_days(days);
    if !(i64::from(MIN_YEAR)..=i64::from(MAX_YEAR)).contains(&ry) {
        return None;
    }
    Some((days - epoch_offset(epoch)) as f64)
}

/// Day number (days since 1970-01-01) of the integer part of `serial`, or
/// `None` if the serial is not finite or its year is outside 1..=9999.
pub fn unix_days_from_serial(epoch: DateEpoch, serial: f64) -> Option<i64> {
    if !serial.is_finite() || serial.abs() > 1.0e8 {
        return None;
    }
    let days = serial.floor() as i64 + epoch_offset(epoch);
    let (y, _, _) = civil_from_days(days);
    if (i64::from(MIN_YEAR)..=i64::from(MAX_YEAR)).contains(&y) {
        Some(days)
    } else {
        None
    }
}

/// The date of `floor(serial)`, or `None` if the year falls outside
/// 1..=9999 (callers report `Err:502`). Negative serials are allowed.
pub fn ymd_from_serial(epoch: DateEpoch, serial: f64) -> Option<(i32, u32, u32)> {
    let days = unix_days_from_serial(epoch, serial)?;
    let (y, m, d) = civil_from_days(days);
    Some((y as i32, m, d))
}

/// Time of day `(hours, minutes, seconds, milliseconds)` of the fractional
/// part of `serial`, rounded to the nearest millisecond so that values like
/// 14:29:59.99999 come out as 14:30:00.000. A time that rounds up to 24:00
/// wraps to 00:00:00.000. Non-finite input gives all zeros.
pub fn hms_from_serial(serial: f64) -> (u32, u32, u32, u32) {
    if !serial.is_finite() {
        return (0, 0, 0, 0);
    }
    let frac = serial - serial.floor();
    let mut ms = (frac * 86_400_000.0).round() as i64;
    if ms >= 86_400_000 {
        ms -= 86_400_000;
    }
    let ms = ms.max(0) as u32;
    (
        ms / 3_600_000,
        (ms / 60_000) % 60,
        (ms / 1000) % 60,
        ms % 1000,
    )
}

/// Fraction of a day for a time of day. Values are not range-checked, so
/// `serial_from_hms(25, 0, 0, 0)` is `25/24`.
pub fn serial_from_hms(h: u32, m: u32, s: u32, millis: u32) -> f64 {
    let total_ms = f64::from(h) * 3_600_000.0
        + f64::from(m) * 60_000.0
        + f64::from(s) * 1000.0
        + f64::from(millis);
    total_ms / 86_400_000.0
}

/// Weekday of `floor(serial)` with Sunday = 1 … Saturday = 7 (AOO
/// `WEEKDAY(x; 1)`), or `None` out of range.
pub fn weekday(epoch: DateEpoch, serial: f64) -> Option<u32> {
    let days = unix_days_from_serial(epoch, serial)?;
    Some(weekday_sun0(days) + 1)
}

/// ISO weekday of `floor(serial)` with Monday = 1 … Sunday = 7, or `None`
/// out of range.
pub fn iso_weekday(epoch: DateEpoch, serial: f64) -> Option<u32> {
    let days = unix_days_from_serial(epoch, serial)?;
    Some((weekday_sun0(days) + 6) % 7 + 1)
}

/// Weekday 0 = Sunday … 6 = Saturday of a day number (days since 1970-01-01,
/// which was a Thursday).
pub(crate) fn weekday_sun0(unix_days: i64) -> u32 {
    (unix_days + 4).rem_euclid(7) as u32
}

/// ISO 8601 week number (1..=53) of a day number (days since 1970-01-01).
pub fn iso_week_number(unix_days: i64) -> u32 {
    let iso_wd = i64::from((weekday_sun0(unix_days) + 6) % 7); // Monday = 0
                                                               // The Thursday of this week decides the year.
    let thursday = unix_days - iso_wd + 3;
    let (ty, _, _) = civil_from_days(thursday);
    let jan1 = days_from_civil(ty, 1, 1);
    ((thursday - jan1) / 7 + 1) as u32
}

/// The current local date and time as a serial in `epoch` (for `NOW()`;
/// `TODAY()` is its floor).
pub fn now_serial(epoch: DateEpoch) -> f64 {
    use chrono::{Datelike, Timelike};
    let now = chrono::Local::now().naive_local();
    let days = days_from_civil(i64::from(now.year()), now.month(), now.day());
    let secs = f64::from(now.num_seconds_from_midnight())
        + f64::from(now.nanosecond() % 1_000_000_000) / 1e9;
    (days - epoch_offset(epoch)) as f64 + secs / 86_400.0
}

/// Today's local date `(year, month, day)`.
pub fn today_ymd() -> (i32, u32, u32) {
    use chrono::Datelike;
    let now = chrono::Local::now().naive_local();
    (now.year(), now.month(), now.day())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchors() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(1899, 12, 30), -25_569);
        assert_eq!(days_from_civil(1900, 1, 1), -25_567);
        assert_eq!(days_from_civil(1904, 1, 1), -24_107);
        assert_eq!(civil_from_days(-25_569), (1899, 12, 30));
    }

    #[test]
    fn normalized() {
        let e = DateEpoch::D1899;
        assert_eq!(
            serial_from_ymd_normalized(e, 2025, 13, 1),
            serial_from_ymd(e, 2026, 1, 1)
        );
        assert_eq!(
            serial_from_ymd_normalized(e, 2026, 3, 0),
            serial_from_ymd(e, 2026, 2, 28)
        );
        assert_eq!(
            serial_from_ymd_normalized(e, 2026, 0, 1),
            serial_from_ymd(e, 2025, 12, 1)
        );
        assert_eq!(
            serial_from_ymd_normalized(e, 2026, 1, 32),
            serial_from_ymd(e, 2026, 2, 1)
        );
        assert_eq!(serial_from_ymd_normalized(e, 10000, 1, 1), None);
    }

    #[test]
    fn hms() {
        assert_eq!(hms_from_serial(0.5), (12, 0, 0, 0));
        assert_eq!(hms_from_serial(14.5 / 24.0), (14, 30, 0, 0));
        assert_eq!(hms_from_serial(0.999_999_999_9), (0, 0, 0, 0));
        assert_eq!(hms_from_serial(-0.25), (18, 0, 0, 0));
        assert_eq!(serial_from_hms(12, 0, 0, 0), 0.5);
    }

    #[test]
    fn iso_week() {
        // 2026-01-01 is a Thursday: week 1. 2027-01-01 (Friday) is week 53 of 2026.
        assert_eq!(iso_week_number(days_from_civil(2026, 1, 1)), 1);
        assert_eq!(iso_week_number(days_from_civil(2027, 1, 1)), 53);
        assert_eq!(iso_week_number(days_from_civil(2026, 10, 19)), 43);
    }
}
