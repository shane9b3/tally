//! Locale data used by formatting and input recognition.
//!
//! A [`Locale`] is plain data (`&'static str` tables), so adding a locale
//! means adding another constant like [`EN_US`]. Phase A ships en-US only.

/// Order of day, month and year in short numeric dates.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum DateOrder {
    /// Month, day, year (en-US: `10/19/2026`).
    MDY,
    /// Day, month, year (most of Europe: `19/10/2026`).
    DMY,
    /// Year, month, day (ISO-like locales: `2026/10/19`).
    YMD,
}

/// Number, date and text conventions for one locale.
#[derive(Clone, PartialEq, Debug)]
pub struct Locale {
    /// BCP 47 tag, e.g. `en-US`.
    pub tag: &'static str,
    /// Decimal separator (`.` in en-US).
    pub decimal_sep: char,
    /// Thousands grouping separator (`,` in en-US).
    pub group_sep: char,
    /// List / function argument separator (`,` in en-US).
    pub list_sep: char,
    /// Order of the fields in short numeric dates.
    pub date_order: DateOrder,
    /// Separator in short numeric dates (`/` in en-US).
    pub date_sep: char,
    /// Separator between hours, minutes and seconds (`:`).
    pub time_sep: char,
    /// Currency symbol (`$`).
    pub currency_symbol: &'static str,
    /// ISO 4217 currency code (`USD`).
    pub currency_code: &'static str,
    /// Windows LCID in hex as used in `[$$-409]` format tags.
    pub lcid_hex: &'static str,
    /// Full month names, January first.
    pub month_names: [&'static str; 12],
    /// Abbreviated month names, January first.
    pub short_month_names: [&'static str; 12],
    /// Full weekday names, Sunday first.
    pub day_names: [&'static str; 7],
    /// Abbreviated weekday names, Sunday first.
    pub short_day_names: [&'static str; 7],
    /// The 12-hour clock markers (`("AM", "PM")`).
    pub am_pm: (&'static str, &'static str),
    /// First year of the two-digit-year window. With 1930 (the AOO
    /// default), `00..=29` means 2000..=2029 and `30..=99` means 1930..=1999.
    pub two_digit_year_pivot: i32,
    /// Text shown for `TRUE`.
    pub true_name: &'static str,
    /// Text shown for `FALSE`.
    pub false_name: &'static str,
    /// Separator after the weekday name in long dates (`NNNN`): `", "`.
    pub long_date_day_of_week_sep: &'static str,
    /// Short quarter names for the `Q` keyword.
    pub quarter_abbrev: [&'static str; 4],
    /// Long quarter names for the `QQ` keyword.
    pub quarter_names: [&'static str; 4],
    /// Short era names (before, after the epoch of the Gregorian calendar).
    pub era_abbrev: (&'static str, &'static str),
    /// Full era names.
    pub era_names: (&'static str, &'static str),
}

/// The en-US locale.
pub const EN_US: Locale = Locale {
    tag: "en-US",
    decimal_sep: '.',
    group_sep: ',',
    list_sep: ',',
    date_order: DateOrder::MDY,
    date_sep: '/',
    time_sep: ':',
    currency_symbol: "$",
    currency_code: "USD",
    lcid_hex: "409",
    month_names: [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ],
    short_month_names: [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ],
    day_names: [
        "Sunday",
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
    ],
    short_day_names: ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"],
    am_pm: ("AM", "PM"),
    two_digit_year_pivot: 1930,
    true_name: "TRUE",
    false_name: "FALSE",
    long_date_day_of_week_sep: ", ",
    quarter_abbrev: ["Q1", "Q2", "Q3", "Q4"],
    quarter_names: ["1st quarter", "2nd quarter", "3rd quarter", "4th quarter"],
    era_abbrev: ("BC", "AD"),
    era_names: ("Before Christ", "Anno Domini"),
};

impl Locale {
    /// Returns the en-US locale.
    pub fn en_us() -> Locale {
        EN_US.clone()
    }

    /// Expands a two-digit year using [`Locale::two_digit_year_pivot`].
    ///
    /// Years outside `0..=99` are returned unchanged.
    pub fn expand_two_digit_year(&self, yy: i32) -> i32 {
        if !(0..=99).contains(&yy) {
            return yy;
        }
        let pivot = self.two_digit_year_pivot;
        let mut year = pivot - pivot.rem_euclid(100) + yy;
        if year < pivot {
            year += 100;
        }
        year
    }

    /// Looks up a month by full or abbreviated name (ASCII case-insensitive).
    /// Returns 1..=12.
    pub fn month_from_name(&self, name: &str) -> Option<(u32, bool)> {
        for (i, (long, short)) in self
            .month_names
            .iter()
            .zip(self.short_month_names.iter())
            .enumerate()
        {
            if name.eq_ignore_ascii_case(long) {
                return Some((i as u32 + 1, true));
            }
            if name.eq_ignore_ascii_case(short) {
                return Some((i as u32 + 1, false));
            }
        }
        None
    }
}

impl Default for Locale {
    fn default() -> Self {
        Self::en_us()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pivot() {
        let l = Locale::en_us();
        assert_eq!(l.expand_two_digit_year(0), 2000);
        assert_eq!(l.expand_two_digit_year(29), 2029);
        assert_eq!(l.expand_two_digit_year(30), 1930);
        assert_eq!(l.expand_two_digit_year(99), 1999);
        assert_eq!(l.expand_two_digit_year(2026), 2026);
    }

    #[test]
    fn month_lookup() {
        let l = Locale::en_us();
        assert_eq!(l.month_from_name("oct"), Some((10, false)));
        assert_eq!(l.month_from_name("OCTOBER"), Some((10, true)));
        assert_eq!(l.month_from_name("Octo"), None);
    }
}
