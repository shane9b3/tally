//! Excel date serials ↔ Tally date serials.
//!
//! Excel's 1900 date system counts 1900-01-01 as serial 1 and, copying a
//! Lotus 1-2-3 bug, treats 1900 as a leap year: serial 60 is the
//! nonexistent 1900-02-29 and serial 61 is 1900-03-01. Tally's default epoch
//! (D1899, serial 0 = 1899-12-30, as OpenOffice) agrees with Excel from
//! serial 61 on.
//!
//! | Date | Excel 1900 | Tally D1899 | Tally D1900 | Excel 1904 = Tally D1904 |
//! |---|---|---|---|---|
//! | 1899-12-30 | – | 0 | −2 | −1462 |
//! | 1899-12-31 | (0 = "1900-01-00") | 1 | −1 | −1461 |
//! | 1900-01-01 | 1 | 2 | 0 | −1460 |
//! | 1900-02-28 | 59 | 60 | 58 | −1403 |
//! | (1900-02-29) | 60 | – | – | – |
//! | 1900-03-01 | 61 | 61 | 59 | −1402 |
//! | 1904-01-01 | 1462 | 1462 | 1460 | 0 |
//! | 2026-10-19 | 46314 | 46314 | 46312 | 44852 |
//!
//! Only cells whose number format shows a date (day, month or year) are
//! serials; every other number is left alone (a duration `[h]:mm` of 1.5
//! days must stay 1.5).
//!
//! - Import, 1900 system → D1899: serials 1 ≤ s < 60 gain one day so the
//!   *calendar date* Excel displays is kept; the fictitious day 60 becomes
//!   1900-02-28 (D1899 60, keeping its time of day); everything else is
//!   unchanged.
//! - Import, 1904 system: the workbook epoch becomes D1904 and serials are
//!   kept as they are (LibreOffice does the same), so formulas that mix
//!   dates and plain numbers keep working.
//! - Export from D1899: 1 ≤ s < 61 loses one day (1899-12-31 has no Excel
//!   date and is written as Excel 0); from D1900: +1 before 1900-03-01 and
//!   +2 from then on; D1904 writes the `date1904` flag and keeps serials.

use tally_types::DateEpoch;

/// Converts a date-formatted Excel serial to Tally's serial: 1900 system →
/// [`DateEpoch::D1899`] with the leap-year bug removed; `date1904` serials
/// are returned unchanged (the workbook epoch becomes
/// [`DateEpoch::D1904`]).
pub fn excel_serial_to_tally(serial: f64, date1904: bool) -> f64 {
    if date1904 || !serial.is_finite() {
        return serial;
    }
    if (1.0..60.0).contains(&serial) {
        serial + 1.0
    } else {
        // 60.x (the fictitious 1900-02-29) maps onto 1900-02-28 = D1899 60.
        serial
    }
}

/// Converts a date-formatted Tally serial in `epoch` to the Excel serial to
/// store: 1900 system for [`DateEpoch::D1899`] and [`DateEpoch::D1900`],
/// unchanged for [`DateEpoch::D1904`] (written with `date1904="1"`).
pub fn tally_serial_to_excel(serial: f64, epoch: DateEpoch) -> f64 {
    if !serial.is_finite() {
        return serial;
    }
    match epoch {
        DateEpoch::D1899 => {
            if (1.0..61.0).contains(&serial) {
                serial - 1.0
            } else {
                serial
            }
        }
        DateEpoch::D1900 => {
            if serial < 59.0 {
                serial + 1.0
            } else {
                serial + 2.0
            }
        }
        DateEpoch::D1904 => serial,
    }
}

/// Parses an ISO 8601 `YYYY-MM-DD[THH:MM[:SS[.fff]]]` value (strict OOXML
/// `t="d"` cells) into a serial in `epoch`.
pub(crate) fn iso_to_serial(s: &str, epoch: DateEpoch) -> Option<f64> {
    let s = s.trim();
    let (date, time) = match s.split_once('T') {
        Some((d, t)) => (d, Some(t)),
        None => (s, None),
    };
    let mut it = date.splitn(3, '-');
    let y: i32 = it.next()?.parse().ok()?;
    let m: u32 = it.next()?.parse().ok()?;
    let d: u32 = it.next()?.parse().ok()?;
    let mut serial = tally_numfmt::date::serial_from_ymd(epoch, y, m, d)?;
    if let Some(t) = time {
        let t = t.trim_end_matches('Z');
        let mut parts = t.splitn(3, ':');
        let h: f64 = parts.next()?.parse().ok()?;
        let mi: f64 = parts.next().unwrap_or("0").parse().ok()?;
        let sec: f64 = parts.next().unwrap_or("0").parse().ok()?;
        if !(0.0..24.0).contains(&h) || !(0.0..60.0).contains(&mi) || !(0.0..61.0).contains(&sec) {
            return None;
        }
        serial += (h * 3600.0 + mi * 60.0 + sec) / 86_400.0;
    }
    Some(serial)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tally_numfmt::date::serial_from_ymd;

    fn ymd(epoch: DateEpoch, y: i32, m: u32, d: u32) -> f64 {
        serial_from_ymd(epoch, y, m, d).unwrap_or(f64::NAN)
    }

    /// Excel 1900-system serials of the reference dates (from Excel).
    const EXCEL: [((i32, u32, u32), f64); 5] = [
        ((1900, 1, 1), 1.0),
        ((1900, 2, 28), 59.0),
        ((1900, 3, 1), 61.0),
        ((1904, 1, 1), 1462.0),
        ((2026, 10, 19), 46314.0),
    ];

    #[test]
    fn table_matches_numfmt() {
        use DateEpoch::*;
        assert_eq!(ymd(D1899, 1900, 1, 1), 2.0);
        assert_eq!(ymd(D1900, 1900, 1, 1), 0.0);
        assert_eq!(ymd(D1904, 1900, 1, 1), -1460.0);
        assert_eq!(ymd(D1899, 1900, 2, 28), 60.0);
        assert_eq!(ymd(D1900, 1900, 2, 28), 58.0);
        assert_eq!(ymd(D1899, 1900, 3, 1), 61.0);
        assert_eq!(ymd(D1900, 1900, 3, 1), 59.0);
        assert_eq!(ymd(D1899, 2026, 10, 19), 46314.0);
        assert_eq!(ymd(D1900, 2026, 10, 19), 46312.0);
        assert_eq!(ymd(D1904, 2026, 10, 19), 44852.0);
    }

    #[test]
    fn import_1900_system_keeps_calendar_dates() {
        for ((y, m, d), excel) in EXCEL {
            assert_eq!(
                excel_serial_to_tally(excel, false),
                ymd(DateEpoch::D1899, y, m, d),
                "{y}-{m}-{d}"
            );
        }
        // Times of day survive; the fictitious 1900-02-29 lands on 02-28.
        assert_eq!(excel_serial_to_tally(1.5, false), 2.5);
        assert_eq!(excel_serial_to_tally(60.25, false), 60.25);
        assert_eq!(excel_serial_to_tally(0.75, false), 0.75);
    }

    #[test]
    fn export_round_trips_every_epoch() {
        for epoch in [DateEpoch::D1899, DateEpoch::D1900] {
            for ((y, m, d), excel) in EXCEL {
                let s = ymd(epoch, y, m, d);
                assert_eq!(
                    tally_serial_to_excel(s, epoch),
                    excel,
                    "{epoch:?} {y}-{m}-{d}"
                );
                assert_eq!(tally_serial_to_excel(s + 0.5, epoch), excel + 0.5);
            }
        }
        // 1904: serials are kept (with the date1904 flag).
        for ((y, m, d), _) in EXCEL {
            let s = ymd(DateEpoch::D1904, y, m, d);
            assert_eq!(tally_serial_to_excel(s, DateEpoch::D1904), s);
            assert_eq!(excel_serial_to_tally(s, true), s);
        }
        // Import inverts export for D1899 everywhere except 1899-12-31.
        for s in [2.0, 2.25, 59.0, 60.0, 61.0, 100.5, 46314.0] {
            let e = tally_serial_to_excel(s, DateEpoch::D1899);
            assert_eq!(excel_serial_to_tally(e, false), s, "{s}");
        }
        assert_eq!(tally_serial_to_excel(1.0, DateEpoch::D1899), 0.0);
    }

    #[test]
    fn iso_dates() {
        assert_eq!(iso_to_serial("2026-10-19", DateEpoch::D1899), Some(46314.0));
        assert_eq!(
            iso_to_serial("2026-10-19T12:00:00Z", DateEpoch::D1899),
            Some(46314.5)
        );
        assert_eq!(iso_to_serial("2026-13-01", DateEpoch::D1899), None);
        assert_eq!(iso_to_serial("x", DateEpoch::D1899), None);
    }
}
