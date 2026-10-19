//! Lengths, colours, dates and durations as written in ODF attributes.

use tally_types::DateEpoch;

/// Proleptic Gregorian days since 1970-01-01 (Howard Hinnant's algorithm).
pub(crate) fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = i64::from(m);
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The civil date of day zero of `epoch`'s serial numbers.
pub(crate) fn epoch_base(epoch: DateEpoch) -> i64 {
    match epoch {
        DateEpoch::D1899 => days_from_civil(1899, 12, 30),
        DateEpoch::D1900 => days_from_civil(1899, 12, 31),
        DateEpoch::D1904 => days_from_civil(1904, 1, 1),
    }
}

/// Maps a `table:null-date` value to an epoch (unknown dates keep the default).
pub(crate) fn epoch_from_null_date(s: &str) -> DateEpoch {
    match parse_iso_date(s).map(|(y, m, d, _)| (y, m, d)) {
        Some((1904, 1, 1)) => DateEpoch::D1904,
        Some((1899, 12, 31)) | Some((1900, 1, 1)) => DateEpoch::D1900,
        _ => DateEpoch::D1899,
    }
}

/// The `table:null-date` value for an epoch.
pub(crate) fn null_date_for(epoch: DateEpoch) -> &'static str {
    match epoch {
        DateEpoch::D1899 => "1899-12-30",
        DateEpoch::D1900 => "1899-12-31",
        DateEpoch::D1904 => "1904-01-01",
    }
}

/// Largest year magnitude [`parse_iso_date`] accepts.
pub(crate) const MAX_YEAR: i64 = 99_999;

/// Parses `[-]YYYY-MM-DD[THH:MM[:SS[.fff]]][Z|±HH:MM]` into
/// `(year, month, day, seconds_into_day)`. Time zones are ignored.
pub(crate) fn parse_iso_date(s: &str) -> Option<(i64, u32, u32, f64)> {
    let s = s.trim();
    let (neg, body) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s),
    };
    let (date, time) = match body.find('T') {
        Some(i) => (&body[..i], Some(&body[i + 1..])),
        None => (body, None),
    };
    let mut parts = date.split('-');
    let y: i64 = parts.next()?.parse().ok()?;
    let m: u32 = parts.next()?.parse().ok()?;
    let d: u32 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let y = if neg { -y } else { y };
    // Far beyond any spreadsheet date (AOO: -32768..=32767); keeps the
    // day arithmetic in `days_from_civil` from overflowing.
    if !(-MAX_YEAR..=MAX_YEAR).contains(&y) {
        return None;
    }
    let secs = match time {
        None => 0.0,
        Some(t) => {
            let t = t
                .trim_end_matches('Z')
                .split(['+', '-'])
                .next()
                .unwrap_or_default();
            let mut it = t.split(':');
            let h: f64 = it.next()?.parse().ok()?;
            let mi: f64 = it.next().map_or(Some(0.0), |x| x.parse().ok())?;
            let se: f64 = it.next().map_or(Some(0.0), |x| x.parse().ok())?;
            h * 3600.0 + mi * 60.0 + se
        }
    };
    Some((y, m, d, secs))
}

/// `office:date-value` → serial number relative to `epoch`.
pub(crate) fn date_value_to_serial(s: &str, epoch: DateEpoch) -> Option<f64> {
    let (y, m, d, secs) = parse_iso_date(s)?;
    let days = days_from_civil(y, m, d) - epoch_base(epoch);
    Some(days as f64 + secs / 86_400.0)
}

/// `office:time-value` (ISO 8601 duration such as `PT12H30M15.5S` or
/// `-P1DT2H`) → fraction of a day.
pub(crate) fn duration_to_days(s: &str) -> Option<f64> {
    let s = s.trim();
    let (neg, s) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s),
    };
    let s = s.strip_prefix('P')?;
    let mut secs = 0.0f64;
    let mut num = String::new();
    let mut in_time = false;
    let mut any = false;
    for ch in s.chars() {
        match ch {
            'T' => in_time = true,
            '0'..='9' | '.' | ',' => num.push(if ch == ',' { '.' } else { ch }),
            unit => {
                let v: f64 = num.parse().ok()?;
                num.clear();
                any = true;
                secs += v * match (unit, in_time) {
                    ('Y', false) => 365.0 * 86_400.0,
                    ('M', false) => 30.0 * 86_400.0,
                    ('W', false) => 7.0 * 86_400.0,
                    ('D', false) => 86_400.0,
                    ('H', true) => 3600.0,
                    ('M', true) => 60.0,
                    ('S', true) => 1.0,
                    _ => return None,
                };
            }
        }
    }
    if !num.is_empty() || !any {
        return None;
    }
    let days = secs / 86_400.0;
    Some(if neg { -days } else { days })
}

/// A length such as `2.258cm`, `0.889in`, `12pt` → 1/100 mm.
pub(crate) fn length_to_hmm(s: &str) -> Option<u32> {
    let s = s.trim();
    let split = s.find(|c: char| c.is_ascii_alphabetic()).unwrap_or(s.len());
    let (num, unit) = s.split_at(split);
    let v: f64 = num.trim().parse().ok()?;
    let factor = match unit {
        "cm" => 1000.0,
        "mm" => 100.0,
        "in" | "inch" => 2540.0,
        "pt" => 2540.0 / 72.0,
        "pc" => 2540.0 / 6.0,
        "px" => 2540.0 / 96.0,
        _ => return None,
    };
    let hmm = (v * factor).round();
    (hmm.is_finite() && hmm >= 0.0 && hmm <= f64::from(u32::MAX)).then_some(hmm as u32)
}

/// An ODF border value (`0.74pt solid #000000`, tokens in any order) →
/// `Some(None)` for `none`/`hidden`, `Some(Some(line))` for a line, `None`
/// when unparsable.
pub(crate) fn parse_border(s: &str) -> Option<Option<tally_model::BorderLine>> {
    use tally_model::{BorderLine, BorderStyle};
    let t = s.trim();
    if t.is_empty() || t.eq_ignore_ascii_case("none") || t.eq_ignore_ascii_case("hidden") {
        return Some(None);
    }
    let mut line = BorderLine {
        style: BorderStyle::Solid,
        width_hmm: 26,
        color: 0x0000_00ff,
    };
    let mut any = false;
    for tok in t.split_ascii_whitespace() {
        let lower = tok.to_ascii_lowercase();
        if let Some(c) = parse_color(tok) {
            line.color = c;
            any = true;
        } else if let Some(w) = length_to_hmm(tok) {
            line.width_hmm = w;
            any = true;
        } else {
            line.style = match lower.as_str() {
                "none" | "hidden" => return Some(None),
                "solid" => BorderStyle::Solid,
                "double" | "double-thin" => BorderStyle::Double,
                "dotted" => BorderStyle::Dotted,
                "dashed" | "fine-dashed" | "dash-dot" | "dash-dot-dot" | "dot-dash" => {
                    BorderStyle::Dashed
                }
                "thin" | "medium" | "thick" => {
                    line.width_hmm = match lower.as_str() {
                        "thin" => 26,
                        "medium" => 88,
                        _ => 176,
                    };
                    line.style
                }
                _ => BorderStyle::Solid,
            };
            any = true;
        }
    }
    any.then_some(Some(line))
}

/// A border line as an ODF border value, e.g. `0.74pt solid #000000`.
pub(crate) fn border_to_string(b: &tally_model::BorderLine) -> String {
    use tally_model::BorderStyle;
    let pt = (f64::from(b.width_hmm) * 72.0 / 2540.0 * 100.0).round() / 100.0;
    let style = match b.style {
        BorderStyle::Solid => "solid",
        BorderStyle::Dashed => "dashed",
        BorderStyle::Dotted => "dotted",
        BorderStyle::Double => "double",
    };
    format!("{pt}pt {style} {}", color_hex(b.color))
}

/// 1/100 mm → `cm` string with up to 3 decimals (exact for whole 1/100 mm).
pub(crate) fn hmm_to_cm(hmm: u32) -> String {
    let whole = hmm / 1000;
    let frac = hmm % 1000;
    if frac == 0 {
        format!("{whole}cm")
    } else {
        let f = format!("{frac:03}");
        format!("{whole}.{}cm", f.trim_end_matches('0'))
    }
}

/// A font size in points, e.g. `10pt`.
pub(crate) fn font_size_pt(s: &str) -> Option<f32> {
    let n = s.trim().strip_suffix("pt")?;
    let v: f32 = n.trim().parse().ok()?;
    (v.is_finite() && v > 0.0).then_some(v)
}

/// `#rrggbb` → `0xRRGGBBff`.
pub(crate) fn parse_color(s: &str) -> Option<u32> {
    let hex = s.trim().strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let rgb = u32::from_str_radix(hex, 16).ok()?;
    Some((rgb << 8) | 0xff)
}

/// `0xRRGGBBAA` → `#rrggbb` (alpha dropped; ODF has no alpha here).
pub(crate) fn color_hex(c: u32) -> String {
    format!("#{:06x}", c >> 8)
}

/// Shortest round-trip text for an `f64` attribute value.
pub(crate) fn fmt_f64(v: f64) -> String {
    let a = v.abs();
    if a != 0.0 && !(1e-5..1e16).contains(&a) {
        format!("{v:e}")
    } else {
        format!("{v}")
    }
}

/// Simple "General" display text for a cached number in `text:p`.
pub(crate) fn general_display(v: f64) -> String {
    if v.is_finite() && v == v.trunc() && v.abs() < 1e15 {
        return format!("{v:.0}");
    }
    let a = v.abs();
    if a != 0.0 && !(1e-5..1e15).contains(&a) {
        let s = format!("{v:E}");
        // `1.5E300` → `1.5E+300` like spreadsheet apps.
        return match s.find('E') {
            Some(i) if !s[i + 1..].starts_with('-') => format!("{}E+{}", &s[..i], &s[i + 1..]),
            _ => s,
        };
    }
    format!("{v}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_days() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
        assert_eq!(
            date_value_to_serial("1899-12-30", DateEpoch::D1899),
            Some(0.0)
        );
        assert_eq!(
            date_value_to_serial("2026-10-19", DateEpoch::D1899),
            Some(46_314.0)
        );
        assert_eq!(
            date_value_to_serial("1900-03-01", DateEpoch::D1899),
            Some(61.0)
        );
        assert_eq!(
            date_value_to_serial("2026-10-19T18:00:00", DateEpoch::D1899),
            Some(46_314.75)
        );
        assert_eq!(
            date_value_to_serial("1904-01-02", DateEpoch::D1904),
            Some(1.0)
        );
        assert_eq!(
            date_value_to_serial("1800-01-01", DateEpoch::D1899),
            Some(-36_522.0)
        );
        assert_eq!(date_value_to_serial("2026-13-01", DateEpoch::D1899), None);
    }

    #[test]
    fn durations() {
        assert_eq!(duration_to_days("PT12H00M00S"), Some(0.5));
        assert_eq!(duration_to_days("PT36H"), Some(1.5));
        assert_eq!(duration_to_days("P1DT6H"), Some(1.25));
        assert_eq!(duration_to_days("-PT6H"), Some(-0.25));
        assert!((duration_to_days("PT00H00M01.5S").unwrap() - 1.5 / 86_400.0).abs() < 1e-15);
        assert_eq!(duration_to_days("PT"), None);
        assert_eq!(duration_to_days("12:00"), None);
    }

    #[test]
    fn lengths_and_colours() {
        assert_eq!(length_to_hmm("2.258cm"), Some(2258));
        assert_eq!(length_to_hmm("0.889in"), Some(2258));
        assert_eq!(length_to_hmm("22.58mm"), Some(2258));
        assert_eq!(length_to_hmm("72pt"), Some(2540));
        assert_eq!(length_to_hmm("-1cm"), None);
        assert_eq!(hmm_to_cm(2258), "2.258cm");
        assert_eq!(hmm_to_cm(3000), "3cm");
        assert_eq!(hmm_to_cm(2250), "2.25cm");
        assert_eq!(parse_color("#FF0000"), Some(0xff00_00ff));
        assert_eq!(color_hex(0x1122_33ff), "#112233");
        assert_eq!(font_size_pt("10.5pt"), Some(10.5));
    }

    #[test]
    fn numbers() {
        for v in [0.1, 1.0 / 3.0, 1e300, -2.5e-9, 123456789.0, 0.0] {
            assert_eq!(fmt_f64(v).parse::<f64>().unwrap(), v);
        }
        assert_eq!(general_display(3.0), "3");
        assert_eq!(general_display(-0.5), "-0.5");
        assert_eq!(general_display(1.5e300), "1.5E+300");
    }
}

#[cfg(test)]
mod border_tests {
    use super::*;
    use tally_model::{BorderLine, BorderStyle};

    #[test]
    fn border_strings_round_trip() {
        let b = parse_border("0.74pt solid #000000").unwrap().unwrap();
        assert_eq!(b.width_hmm, 26);
        assert_eq!(b.style, BorderStyle::Solid);
        assert_eq!(b.color, 0x0000_00ff);
        assert_eq!(border_to_string(&b), "0.74pt solid #000000");
        let b = parse_border("#ff0000 double 0.06cm").unwrap().unwrap();
        assert_eq!(
            (b.width_hmm, b.style, b.color),
            (60, BorderStyle::Double, 0xff00_00ff)
        );
        assert_eq!(parse_border("none"), Some(None));
        assert_eq!(parse_border("hidden"), Some(None));
        assert_eq!(
            parse_border("1pt dot-dash #00ff00").unwrap().unwrap().style,
            BorderStyle::Dashed
        );
        for w in [2u32, 26, 53, 88, 176] {
            let b = BorderLine {
                style: BorderStyle::Dotted,
                width_hmm: w,
                color: 0x1234_56ff,
            };
            assert_eq!(parse_border(&border_to_string(&b)), Some(Some(b)));
        }
    }
}
