//! Day-count conventions and coupon-date helpers of the Analysis add-in.
//!
//! Ported from Apache OpenOffice scaddins/analysis (analysishelper.cxx,
//! financial.cxx), Apache License 2.0: `DateToDays`, `DaysToDate`,
//! `GetDiffDate360`, `GetDiffParam`, `GetDiffDate`, `GetYearDiff`,
//! `GetDaysInYear`, `GetYearFrac`, `ScaDate` and the `COUP*` helpers.
//!
//! Dates are AOO "day numbers": days since 0001-01-00 on the proleptic
//! Gregorian calendar (`DateToDays(1, 1, 1) == 1`). A document serial `s`
//! is the day number `null_date + s`.
//!
//! `basis`: 0 = US (NASD) 30/360, 1 = actual/actual, 2 = actual/360,
//! 3 = actual/365, 4 = European 30/360.

use tally_numfmt::date::{civil_from_days, days_in_month, epoch_offset, is_leap_year};
use tally_types::{DateEpoch, ErrorKind};

/// AOO `THROW_IAE`: an illegal argument, `Err:502`.
pub(super) const IAE: ErrorKind = ErrorKind::InvalidArgument;

/// Day number of 1970-01-01.
const UNIX_DAY: i64 = 719_163;

/// Day number of serial 0 in `epoch` (AOO `GetNullDate`).
pub(super) fn null_date(epoch: DateEpoch) -> i64 {
    epoch_offset(epoch) + UNIX_DAY
}

fn leap(y: i32) -> bool {
    is_leap_year(i64::from(y))
}

/// AOO `DaysInMonth` (month 1..=12).
pub(super) fn dim(month: u32, year: i32) -> u32 {
    days_in_month(i64::from(year), month)
}

/// AOO `DateToDays`: days from 0001-01-00 to `day.month.year`. The day is
/// not validated (Feb 29 of a common year is Mar 1), as in AOO.
pub(super) fn date_to_days(day: u32, month: u32, year: i32) -> i64 {
    let y = i64::from(year) - 1;
    let mut n = y * 365 + y / 4 - y / 100 + y / 400;
    for m in 1..month {
        n += i64::from(dim(m, year));
    }
    n + i64::from(day)
}

/// AOO `DaysToDate`: `(day, month, year)` of a day number; day numbers
/// below 1 or past year 32767 are illegal.
pub(super) fn days_to_date(n: i64) -> Result<(u32, u32, i32), ErrorKind> {
    if n < 1 {
        return Err(IAE);
    }
    let (y, m, d) = civil_from_days(n - UNIX_DAY);
    if y > 0x7FFF {
        return Err(IAE);
    }
    Ok((d, m, y as i32))
}

/// AOO `GetDiffDate360` on split dates.
#[allow(clippy::too_many_arguments)]
fn diff_360_parts(
    mut d1: u32,
    m1: u32,
    y1: i32,
    leap1: bool,
    mut d2: u32,
    mut m2: u32,
    mut y2: i32,
    us: bool,
) -> i64 {
    if d1 == 31 {
        d1 -= 1;
    } else if us && m1 == 2 && (d1 == 29 || (d1 == 28 && !leap1)) {
        d1 = 30;
    }
    if d2 == 31 {
        if us && d1 != 30 {
            d2 = 1;
            if m2 == 12 {
                y2 += 1;
                m2 = 1;
            } else {
                m2 += 1;
            }
        } else {
            d2 = 30;
        }
    }
    i64::from(d2) + i64::from(m2) * 30 + i64::from(y2) * 360
        - i64::from(d1)
        - i64::from(m1) * 30
        - i64::from(y1) * 360
}

/// AOO `GetDiffDate360(nNullDate, nDate1, nDate2, bUSAMethod)`.
pub(super) fn diff_date_360(nd: i64, date1: i64, date2: i64, us: bool) -> Result<i64, ErrorKind> {
    let (d1, m1, y1) = days_to_date(date1 + nd)?;
    let (d2, m2, y2) = days_to_date(date2 + nd)?;
    Ok(diff_360_parts(d1, m1, y1, leap(y1), d2, m2, y2, us))
}

/// AOO `GetDiffParam`: whole years, remaining days and days per year.
fn diff_param(nd: i64, start: i64, end: i64, mode: i32) -> Result<(i64, i64, i64), ErrorKind> {
    let (start, end) = if start > end {
        (end, start)
    } else {
        (start, end)
    };
    let date1 = start + nd;
    let date2 = end + nd;
    let (d1, m1, y1) = days_to_date(date1)?;
    let (d2, m2, y2) = days_to_date(date2)?;
    Ok(match mode {
        0 | 4 => {
            let years = i64::from(y2 - y1);
            let diff = diff_360_parts(d1, m1, y1, leap(y1), d2, m2, y2, mode == 0) - years * 360;
            (years, diff, 360)
        }
        1 => {
            let mut years = i64::from(y2 - y1);
            let diy = if leap(y1) { 366 } else { 365 };
            if years != 0 && (m1 > m2 || (m1 == m2 && d1 > d2)) {
                years -= 1;
            }
            let mut diff = if years != 0 {
                date2 - date_to_days(d1, m1, y2)
            } else {
                date2 - date1
            };
            if diff < 0 {
                diff += diy;
            }
            (years, diff, diy)
        }
        2 => ((date2 - date1) / 360, (date2 - date1) % 360, 360),
        3 => ((date2 - date1) / 365, (date2 - date1) % 365, 365),
        _ => return Err(IAE),
    })
}

/// AOO `GetDiffDate`: signed day count and days in the first year.
fn diff_date(nd: i64, start: i64, end: i64, mode: i32) -> Result<(i64, i64), ErrorKind> {
    let neg = start > end;
    let (start, end) = if neg { (end, start) } else { (start, end) };
    let (ret, days1) = match mode {
        0 | 4 => {
            let (d1, m1, y1) = days_to_date(start + nd)?;
            let (d2, m2, y2) = days_to_date(end + nd)?;
            let months = i64::from(m2) - i64::from(m1) + i64::from(y2 - y1) * 12;
            let days = i64::from(d2) - i64::from(d1);
            let mut ret = months * 30 + days;
            if mode == 0 && m1 == 2 && m2 != 2 && y1 == y2 {
                ret -= if leap(y1) { 1 } else { 2 };
            }
            (ret, 360)
        }
        1 => {
            let (_, _, y) = days_to_date(start + nd)?;
            (end - start, if leap(y) { 366 } else { 365 })
        }
        2 => (end - start, 360),
        3 => (end - start, 365),
        _ => return Err(IAE),
    };
    Ok((if neg { -ret } else { ret }, days1))
}

/// AOO `GetYearDiff`: day count over days of the first year.
pub(super) fn year_diff(nd: i64, start: i64, end: i64, mode: i32) -> Result<f64, ErrorKind> {
    let (total, days1) = diff_date(nd, start, end, mode)?;
    Ok(total as f64 / days1 as f64)
}

/// AOO `GetDaysInYear`.
pub(super) fn days_in_year(nd: i64, date: i64, mode: i32) -> Result<i64, ErrorKind> {
    match mode {
        0 | 2 | 4 => Ok(360),
        1 => {
            let (_, _, y) = days_to_date(date + nd)?;
            Ok(if leap(y) { 366 } else { 365 })
        }
        3 => Ok(365),
        _ => Err(IAE),
    }
}

/// AOO `GetYearFrac` (YEARFRAC semantics, always non-negative).
pub(super) fn year_frac(nd: i64, start: i64, end: i64, mode: i32) -> Result<f64, ErrorKind> {
    if start == end {
        return Ok(0.0);
    }
    let (years, diff, diy) = diff_param(nd, start, end, mode)?;
    Ok(years as f64 + diff as f64 / diy as f64)
}

/// AOO `ScaDate`: a date with day-count-aware month arithmetic.
#[derive(Copy, Clone, Debug)]
pub(super) struct ScaDate {
    orig_day: u32,
    day: u32,
    month: u32,
    year: i32,
    last_day_mode: bool,
    last_day: bool,
    days30: bool,
    us_mode: bool,
}

impl ScaDate {
    /// `ScaDate(nNullDate, nDate, nBase)`.
    pub(super) fn new(nd: i64, date: i64, base: i32) -> Result<Self, ErrorKind> {
        let (d, m, y) = days_to_date(nd + date)?;
        let mut s = ScaDate {
            orig_day: d,
            day: d,
            month: m,
            year: y,
            last_day_mode: base != 5,
            last_day: d >= dim(m, y),
            days30: base == 0 || base == 4,
            us_mode: base == 0,
        };
        s.set_day();
        Ok(s)
    }

    fn set_day(&mut self) {
        if self.days30 {
            self.day = self.orig_day.min(30);
            if self.last_day || self.day >= dim(self.month, self.year) {
                self.day = 30;
            }
        } else {
            let last = dim(self.month, self.year);
            self.day = if self.last_day {
                last
            } else {
                self.orig_day.min(last)
            };
        }
    }

    fn days_in_month_of(&self, m: u32) -> i64 {
        if self.days30 {
            30
        } else {
            i64::from(dim(m, self.year))
        }
    }

    fn days_in_month_range(&self, from: u32, to: u32) -> i64 {
        if from > to {
            return 0;
        }
        if self.days30 {
            i64::from(to - from + 1) * 30
        } else {
            (from..=to).map(|m| self.days_in_month_of(m)).sum()
        }
    }

    fn days_in_year_range(&self, from: i32, to: i32) -> i64 {
        if from > to {
            return 0;
        }
        if self.days30 {
            i64::from(to - from + 1) * 360
        } else {
            (from..=to).map(|y| if leap(y) { 366 } else { 365 }).sum()
        }
    }

    fn do_add_years(&mut self, count: i64) -> Result<(), ErrorKind> {
        let ny = count + i64::from(self.year);
        if !(0..=0x7FFF).contains(&ny) {
            return Err(IAE);
        }
        self.year = ny as i32;
        Ok(())
    }

    /// `addYears`.
    pub(super) fn add_years(&mut self, count: i64) -> Result<(), ErrorKind> {
        self.do_add_years(count)?;
        self.set_day();
        Ok(())
    }

    /// `setYear`.
    pub(super) fn set_year(&mut self, y: i32) {
        self.year = y;
        self.set_day();
    }

    /// `addMonths`.
    pub(super) fn add_months(&mut self, count: i64) -> Result<(), ErrorKind> {
        let nm = count + i64::from(self.month);
        if nm > 12 {
            let nm = nm - 1;
            self.do_add_years(nm / 12)?;
            self.month = (nm % 12) as u32 + 1;
        } else if nm < 1 {
            self.do_add_years(nm / 12 - 1)?;
            self.month = (nm % 12 + 12) as u32;
        } else {
            self.month = nm as u32;
        }
        self.set_day();
        Ok(())
    }

    /// The year.
    pub(super) fn year(&self) -> i32 {
        self.year
    }

    /// The month.
    pub(super) fn month(&self) -> u32 {
        self.month
    }

    /// `getDate(nNullDate)`: back to a serial.
    pub(super) fn date(&self, nd: i64) -> i64 {
        let last = dim(self.month, self.year);
        let real = if self.last_day_mode && self.last_day {
            last
        } else {
            last.min(self.orig_day)
        };
        date_to_days(real, self.month, self.year) - nd
    }

    fn lt(&self, o: &ScaDate) -> bool {
        if self.year != o.year {
            return self.year < o.year;
        }
        if self.month != o.month {
            return self.month < o.month;
        }
        if self.day != o.day {
            return self.day < o.day;
        }
        if self.last_day || o.last_day {
            return !self.last_day && o.last_day;
        }
        self.orig_day < o.orig_day
    }

    /// `ScaDate::getDiff`: day count between two dates under their basis.
    pub(super) fn diff(from: &ScaDate, to: &ScaDate) -> Result<i64, ErrorKind> {
        if to.lt(from) {
            return Self::diff(to, from);
        }
        let mut a = *from;
        let mut b = *to;
        if to.days30 {
            if to.us_mode {
                if (from.month == 2 || from.day < 30) && b.orig_day == 31 {
                    b.day = 31;
                } else if b.month == 2 && b.last_day {
                    b.day = dim(2, b.year);
                }
            } else {
                if a.month == 2 && a.day == 30 {
                    a.day = dim(2, a.year);
                }
                if b.month == 2 && b.day == 30 {
                    b.day = dim(2, b.year);
                }
            }
        }
        let mut n: i64 = 0;
        if a.year < b.year || (a.year == b.year && a.month < b.month) {
            n = a.days_in_month_of(a.month) - i64::from(a.day) + 1;
            a.orig_day = 1;
            a.day = 1;
            a.last_day = false;
            a.add_months(1)?;
            if a.year < b.year {
                n += a.days_in_month_range(a.month, 12);
                a.add_months(13 - i64::from(a.month))?;
                n += a.days_in_year_range(a.year, b.year - 1);
                a.add_years(i64::from(b.year - a.year))?;
            }
            n += a.days_in_month_range(a.month, b.month - 1);
            a.add_months(i64::from(b.month) - i64::from(a.month))?;
        }
        n += i64::from(b.day) - i64::from(a.day);
        Ok(n.max(0))
    }
}

impl PartialEq for ScaDate {
    fn eq(&self, o: &Self) -> bool {
        !self.lt(o) && !o.lt(self)
    }
}

impl PartialOrd for ScaDate {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(if self.lt(o) {
            std::cmp::Ordering::Less
        } else if o.lt(self) {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        })
    }
}

/// AOO `CHK_Freq`: frequency must be 1, 2 or 4.
pub(super) fn check_freq(freq: i64) -> Result<(), ErrorKind> {
    if matches!(freq, 1 | 2 | 4) {
        Ok(())
    } else {
        Err(IAE)
    }
}

/// `lcl_GetCouppcd`: last coupon date on or before settlement.
fn couppcd_date(settle: &ScaDate, mat: &ScaDate, freq: i64) -> Result<ScaDate, ErrorKind> {
    let mut d = *mat;
    d.set_year(settle.year());
    if d < *settle {
        d.add_years(1)?;
    }
    while d > *settle {
        d.add_months(-12 / freq)?;
    }
    Ok(d)
}

/// `lcl_GetCoupncd`: first coupon date after settlement.
fn coupncd_date(settle: &ScaDate, mat: &ScaDate, freq: i64) -> Result<ScaDate, ErrorKind> {
    let mut d = *mat;
    d.set_year(settle.year());
    if d > *settle {
        d.add_years(-1)?;
    }
    while d <= *settle {
        d.add_months(12 / freq)?;
    }
    Ok(d)
}

/// Arguments shared by the `COUP*` helpers.
#[derive(Copy, Clone, Debug)]
pub(super) struct Coupon {
    /// Null date (day number of serial 0).
    pub nd: i64,
    /// Settlement serial.
    pub settle: i64,
    /// Maturity serial.
    pub mat: i64,
    /// Coupons per year (1, 2, 4).
    pub freq: i64,
    /// Day-count basis 0..=4.
    pub base: i32,
}

impl Coupon {
    fn check(&self) -> Result<(), ErrorKind> {
        if self.settle >= self.mat {
            return Err(IAE);
        }
        check_freq(self.freq)
    }

    fn dates(&self) -> Result<(ScaDate, ScaDate), ErrorKind> {
        Ok((
            ScaDate::new(self.nd, self.settle, self.base)?,
            ScaDate::new(self.nd, self.mat, self.base)?,
        ))
    }

    /// `GetCouppcd`.
    pub(super) fn pcd(&self) -> Result<f64, ErrorKind> {
        self.check()?;
        let (s, m) = self.dates()?;
        Ok(couppcd_date(&s, &m, self.freq)?.date(self.nd) as f64)
    }

    /// `GetCoupncd`.
    pub(super) fn ncd(&self) -> Result<f64, ErrorKind> {
        self.check()?;
        let (s, m) = self.dates()?;
        Ok(coupncd_date(&s, &m, self.freq)?.date(self.nd) as f64)
    }

    /// `GetCoupdaybs`.
    pub(super) fn daybs(&self) -> Result<f64, ErrorKind> {
        self.check()?;
        let (s, m) = self.dates()?;
        let d = couppcd_date(&s, &m, self.freq)?;
        Ok(ScaDate::diff(&d, &s)? as f64)
    }

    /// `GetCoupdaysnc`.
    pub(super) fn daysnc(&self) -> Result<f64, ErrorKind> {
        self.check()?;
        if self.base != 0 && self.base != 4 {
            let (s, m) = self.dates()?;
            let d = coupncd_date(&s, &m, self.freq)?;
            return Ok(ScaDate::diff(&s, &d)? as f64);
        }
        Ok(self.days()? - self.daybs()?)
    }

    /// `GetCoupdays`.
    pub(super) fn days(&self) -> Result<f64, ErrorKind> {
        self.check()?;
        if self.base == 1 {
            let (s, m) = self.dates()?;
            let d = couppcd_date(&s, &m, self.freq)?;
            let mut next = d;
            next.add_months(12 / self.freq)?;
            return Ok(ScaDate::diff(&d, &next)? as f64);
        }
        Ok(days_in_year(0, 0, self.base)? as f64 / self.freq as f64)
    }

    /// `GetCoupnum`.
    pub(super) fn num(&self) -> Result<f64, ErrorKind> {
        self.check()?;
        let (s, m) = self.dates()?;
        let d = couppcd_date(&s, &m, self.freq)?;
        let months =
            i64::from(m.year() - d.year()) * 12 + i64::from(m.month()) - i64::from(d.month());
        Ok((months * self.freq / 12) as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ND: i64 = 693_594; // 1899-12-30

    #[test]
    fn day_numbers() {
        assert_eq!(date_to_days(1, 1, 1), 1);
        assert_eq!(null_date(DateEpoch::D1899), ND);
        assert_eq!(date_to_days(30, 12, 1899), ND);
        assert_eq!(days_to_date(ND + 39493), Ok((15, 2, 2008)));
        assert_eq!(days_to_date(0), Err(IAE));
    }

    #[test]
    fn year_fractions() {
        // 2008-02-15 .. 2008-05-15
        let (s, e) = (39493, 39583);
        assert_eq!(year_frac(ND, s, e, 2), Ok(0.25));
        assert_eq!(year_frac(ND, s, e, 0), Ok(0.25));
        assert_eq!(year_diff(ND, s, e, 3), Ok(90.0 / 365.0));
        assert_eq!(diff_date_360(ND, 39538, 39600, true), Ok(61));
    }

    #[test]
    fn coupons() {
        // 2007-01-25 .. 2008-11-15, semiannual, actual/actual.
        let c = Coupon {
            nd: ND,
            settle: 39107,
            mat: 39767,
            freq: 2,
            base: 1,
        };
        assert_eq!(c.num(), Ok(4.0));
        assert_eq!(c.days(), Ok(181.0));
        assert_eq!(c.daybs(), Ok(71.0));
        assert_eq!(c.daysnc(), Ok(110.0));
        assert_eq!(c.ncd(), Ok(39217.0)); // 2007-05-15
        assert_eq!(c.pcd(), Ok(39036.0)); // 2006-11-15
    }
}
