//! Date serial round trips and anchors.

use proptest::prelude::*;
use tally_numfmt::date::{
    civil_from_days, days_from_civil, days_in_month, hms_from_serial, is_leap_year, iso_weekday,
    serial_from_hms, serial_from_ymd, serial_from_ymd_normalized, weekday, ymd_from_serial,
};
use tally_types::DateEpoch;

const EPOCHS: [DateEpoch; 3] = [DateEpoch::D1899, DateEpoch::D1900, DateEpoch::D1904];

/// Every day from 1583-01-01 to 9999-12-31, in all three epochs.
#[test]
fn every_day_1583_to_9999() {
    for epoch in EPOCHS {
        let mut expected_serial = serial_from_ymd(epoch, 1583, 1, 1).unwrap();
        let mut count = 0u32;
        for y in 1583..=9999 {
            for m in 1..=12 {
                for d in 1..=days_in_month(i64::from(y), m) {
                    let s = serial_from_ymd(epoch, y, m, d).unwrap();
                    assert_eq!(s, expected_serial, "{y}-{m}-{d} not consecutive");
                    assert_eq!(ymd_from_serial(epoch, s), Some((y, m, d)));
                    // Any time of day maps to the same date.
                    assert_eq!(ymd_from_serial(epoch, s + 0.999), Some((y, m, d)));
                    expected_serial += 1.0;
                    count += 1;
                }
            }
        }
        let span = days_from_civil(10_000, 1, 1) - days_from_civil(1583, 1, 1);
        assert_eq!(i64::from(count), span);
    }
}

#[test]
fn anchors() {
    let e = DateEpoch::D1899;
    assert_eq!(serial_from_ymd(e, 1899, 12, 30), Some(0.0));
    assert_eq!(serial_from_ymd(e, 1899, 12, 31), Some(1.0));
    assert_eq!(serial_from_ymd(e, 1900, 1, 1), Some(2.0));
    assert_eq!(serial_from_ymd(e, 1900, 2, 28), Some(60.0));
    assert_eq!(serial_from_ymd(e, 1900, 3, 1), Some(61.0));
    // No fictitious 1900-02-29.
    assert_eq!(serial_from_ymd(e, 1900, 2, 29), None);
    assert_eq!(serial_from_ymd(e, 2000, 1, 1), Some(36526.0));
    assert_eq!(serial_from_ymd(e, 2026, 1, 1), Some(46023.0));
    // 2026-10-19 is 46314 (Excel/LibreOffice agree; 46023 + 291 days).
    assert_eq!(serial_from_ymd(e, 2026, 10, 19), Some(46314.0));
    assert_eq!(serial_from_ymd(e, 1, 1, 1), Some(-693_593.0));
    assert_eq!(serial_from_ymd(e, 9999, 12, 31), Some(2_958_465.0));
    assert_eq!(serial_from_ymd(DateEpoch::D1900, 1900, 1, 1), Some(0.0));
    assert_eq!(
        serial_from_ymd(DateEpoch::D1900, 2026, 10, 19),
        Some(46312.0)
    );
    assert_eq!(serial_from_ymd(DateEpoch::D1904, 1904, 1, 1), Some(0.0));
    assert_eq!(
        serial_from_ymd(DateEpoch::D1904, 2026, 10, 19),
        Some(44852.0)
    );
    // Range limits.
    assert_eq!(serial_from_ymd(e, 0, 12, 31), None);
    assert_eq!(serial_from_ymd(e, 10000, 1, 1), None);
    assert_eq!(serial_from_ymd(e, 2026, 13, 1), None);
    assert_eq!(serial_from_ymd(e, 2026, 4, 31), None);
    assert_eq!(serial_from_ymd(e, 2026, 4, 0), None);
    assert_eq!(ymd_from_serial(e, -693_594.0), None);
    assert_eq!(ymd_from_serial(e, 2_958_466.0), None);
    assert_eq!(ymd_from_serial(e, f64::NAN), None);
    assert_eq!(ymd_from_serial(e, -0.5), Some((1899, 12, 29)));
    // Weekdays: 2026-10-19 is a Monday; 1899-12-30 was a Saturday.
    assert_eq!(weekday(e, 46314.0), Some(2));
    assert_eq!(iso_weekday(e, 46314.0), Some(1));
    assert_eq!(weekday(e, 0.0), Some(7));
    assert_eq!(weekday(DateEpoch::D1904, 0.0), Some(6)); // 1904-01-01 Friday
                                                         // Leap years.
    assert!(is_leap_year(2000) && is_leap_year(2024) && !is_leap_year(1900));
    assert_eq!(days_in_month(2024, 2), 29);
    assert_eq!(days_in_month(2023, 2), 28);
    assert_eq!(days_in_month(2023, 13), 0);
}

#[test]
fn times() {
    assert_eq!(hms_from_serial(0.0), (0, 0, 0, 0));
    assert_eq!(hms_from_serial(0.75), (18, 0, 0, 0));
    assert_eq!(hms_from_serial(46314.5), (12, 0, 0, 0));
    assert_eq!(hms_from_serial(52_199.999_9 / 86400.0), (14, 30, 0, 0));
    assert_eq!(hms_from_serial(86_399.999 / 86400.0), (23, 59, 59, 999));
    assert_eq!(hms_from_serial(86_399.999_6 / 86400.0), (0, 0, 0, 0));
    assert_eq!(serial_from_hms(18, 0, 0, 0), 0.75);
    assert_eq!(serial_from_hms(25, 30, 0, 0), 25.5 / 24.0);
    for ms in (0..86_400_000u32).step_by(997) {
        let s = serial_from_hms(
            ms / 3_600_000,
            (ms / 60_000) % 60,
            (ms / 1000) % 60,
            ms % 1000,
        );
        let (h, m, sec, milli) = hms_from_serial(s);
        assert_eq!(h * 3_600_000 + m * 60_000 + sec * 1000 + milli, ms);
    }
}

#[test]
fn normalized_dates() {
    let e = DateEpoch::D1899;
    assert_eq!(
        serial_from_ymd_normalized(e, 2026, 22, 1),
        serial_from_ymd(e, 2027, 10, 1)
    );
    assert_eq!(
        serial_from_ymd_normalized(e, 2024, 3, 0),
        serial_from_ymd(e, 2024, 2, 29)
    );
    assert_eq!(
        serial_from_ymd_normalized(e, 2026, -1, 1),
        serial_from_ymd(e, 2025, 11, 1)
    );
    assert_eq!(
        serial_from_ymd_normalized(e, 2026, 1, -30),
        serial_from_ymd(e, 2025, 12, 1)
    );
    assert_eq!(
        serial_from_ymd_normalized(e, 2026, 1, 365),
        serial_from_ymd(e, 2026, 12, 31)
    );
    assert_eq!(serial_from_ymd_normalized(e, 0, 1, 1), None);
    assert_eq!(
        serial_from_ymd_normalized(e, 0, 13, 1),
        serial_from_ymd(e, 1, 1, 1)
    );
}

proptest! {
    #[test]
    fn days_roundtrip(z in -3_000_000i64..3_000_000) {
        let (y, m, d) = civil_from_days(z);
        prop_assert_eq!(days_from_civil(y, m, d), z);
    }

    #[test]
    fn serial_roundtrip(y in 1i32..=9999, m in 1u32..=12, d in 1u32..=31, e in 0usize..3) {
        let epoch = EPOCHS[e];
        prop_assume!(d <= days_in_month(i64::from(y), m));
        let s = serial_from_ymd(epoch, y, m, d).unwrap();
        prop_assert_eq!(ymd_from_serial(epoch, s), Some((y, m, d)));
        prop_assert_eq!(serial_from_ymd_normalized(epoch, i64::from(y), i64::from(m), i64::from(d)), Some(s));
    }

    #[test]
    fn serial_to_ymd_to_serial(s in -693_593i64..=2_958_465, e in 0usize..3, frac in 0.0f64..1.0) {
        let epoch = EPOCHS[e];
        let x = s as f64 + frac;
        if let Some((y, m, d)) = ymd_from_serial(epoch, x) {
            prop_assert_eq!(serial_from_ymd(epoch, y, m, d), Some(s as f64));
        }
    }
}
