//! Locale data used by formatting and input recognition.
//!
//! A [`Locale`] is plain data (`&'static str` tables), so adding a locale
//! means adding another constant like [`EN_US`]. v0.3 ships en-US, en-GB,
//! de-DE, fr-FR, es-ES and ja-JP ([`LOCALES`], [`locale_by_tag`]).

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
    /// The currency symbol follows the amount (`12,50 €`).
    pub currency_after: bool,
    /// Short date format code (`MM/DD/YYYY` in en-US).
    pub short_date: &'static str,
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
    currency_after: false,
    short_date: "MM/DD/YYYY",
};

const EN_MONTHS: [&str; 12] = [
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
];
const EN_SHORT_MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const EN_DAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
const EN_SHORT_DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

/// The en-GB locale (day-month-year, pound sterling).
pub const EN_GB: Locale = Locale {
    tag: "en-GB",
    decimal_sep: '.',
    group_sep: ',',
    list_sep: ',',
    date_order: DateOrder::DMY,
    date_sep: '/',
    time_sep: ':',
    currency_symbol: "£",
    currency_code: "GBP",
    lcid_hex: "809",
    month_names: EN_MONTHS,
    short_month_names: EN_SHORT_MONTHS,
    day_names: EN_DAYS,
    short_day_names: EN_SHORT_DAYS,
    am_pm: ("AM", "PM"),
    two_digit_year_pivot: 1930,
    true_name: "TRUE",
    false_name: "FALSE",
    long_date_day_of_week_sep: ", ",
    quarter_abbrev: ["Q1", "Q2", "Q3", "Q4"],
    quarter_names: ["1st quarter", "2nd quarter", "3rd quarter", "4th quarter"],
    era_abbrev: ("BC", "AD"),
    era_names: ("Before Christ", "Anno Domini"),
    currency_after: false,
    short_date: "DD/MM/YYYY",
};

/// The de-DE locale.
pub const DE_DE: Locale = Locale {
    tag: "de-DE",
    decimal_sep: ',',
    group_sep: '.',
    list_sep: ';',
    date_order: DateOrder::DMY,
    date_sep: '.',
    time_sep: ':',
    currency_symbol: "€",
    currency_code: "EUR",
    lcid_hex: "407",
    month_names: [
        "Januar",
        "Februar",
        "März",
        "April",
        "Mai",
        "Juni",
        "Juli",
        "August",
        "September",
        "Oktober",
        "November",
        "Dezember",
    ],
    short_month_names: [
        "Jan", "Feb", "Mär", "Apr", "Mai", "Jun", "Jul", "Aug", "Sep", "Okt", "Nov", "Dez",
    ],
    day_names: [
        "Sonntag",
        "Montag",
        "Dienstag",
        "Mittwoch",
        "Donnerstag",
        "Freitag",
        "Samstag",
    ],
    short_day_names: ["So", "Mo", "Di", "Mi", "Do", "Fr", "Sa"],
    am_pm: ("AM", "PM"),
    two_digit_year_pivot: 1930,
    true_name: "WAHR",
    false_name: "FALSCH",
    long_date_day_of_week_sep: ", ",
    quarter_abbrev: ["Q1", "Q2", "Q3", "Q4"],
    quarter_names: ["1. Quartal", "2. Quartal", "3. Quartal", "4. Quartal"],
    era_abbrev: ("v. Chr.", "n. Chr."),
    era_names: ("vor Christus", "nach Christus"),
    currency_after: true,
    short_date: "DD.MM.YYYY",
};

/// The fr-FR locale (the grouping separator is a no-break space).
pub const FR_FR: Locale = Locale {
    tag: "fr-FR",
    decimal_sep: ',',
    group_sep: '\u{a0}',
    list_sep: ';',
    date_order: DateOrder::DMY,
    date_sep: '/',
    time_sep: ':',
    currency_symbol: "€",
    currency_code: "EUR",
    lcid_hex: "40C",
    month_names: [
        "janvier",
        "février",
        "mars",
        "avril",
        "mai",
        "juin",
        "juillet",
        "août",
        "septembre",
        "octobre",
        "novembre",
        "décembre",
    ],
    short_month_names: [
        "janv.", "févr.", "mars", "avr.", "mai", "juin", "juil.", "août", "sept.", "oct.", "nov.",
        "déc.",
    ],
    day_names: [
        "dimanche", "lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi",
    ],
    short_day_names: ["dim.", "lun.", "mar.", "mer.", "jeu.", "ven.", "sam."],
    am_pm: ("AM", "PM"),
    two_digit_year_pivot: 1930,
    true_name: "VRAI",
    false_name: "FAUX",
    long_date_day_of_week_sep: " ",
    quarter_abbrev: ["T1", "T2", "T3", "T4"],
    quarter_names: [
        "1er trimestre",
        "2e trimestre",
        "3e trimestre",
        "4e trimestre",
    ],
    era_abbrev: ("av. J.-C.", "ap. J.-C."),
    era_names: ("avant Jésus-Christ", "après Jésus-Christ"),
    currency_after: true,
    short_date: "DD/MM/YYYY",
};

/// The es-ES locale.
pub const ES_ES: Locale = Locale {
    tag: "es-ES",
    decimal_sep: ',',
    group_sep: '.',
    list_sep: ';',
    date_order: DateOrder::DMY,
    date_sep: '/',
    time_sep: ':',
    currency_symbol: "€",
    currency_code: "EUR",
    lcid_hex: "C0A",
    month_names: [
        "enero",
        "febrero",
        "marzo",
        "abril",
        "mayo",
        "junio",
        "julio",
        "agosto",
        "septiembre",
        "octubre",
        "noviembre",
        "diciembre",
    ],
    short_month_names: [
        "ene", "feb", "mar", "abr", "may", "jun", "jul", "ago", "sep", "oct", "nov", "dic",
    ],
    day_names: [
        "domingo",
        "lunes",
        "martes",
        "miércoles",
        "jueves",
        "viernes",
        "sábado",
    ],
    short_day_names: ["dom", "lun", "mar", "mié", "jue", "vie", "sáb"],
    am_pm: ("a. m.", "p. m."),
    two_digit_year_pivot: 1930,
    true_name: "VERDADERO",
    false_name: "FALSO",
    long_date_day_of_week_sep: ", ",
    quarter_abbrev: ["T1", "T2", "T3", "T4"],
    quarter_names: [
        "1.er trimestre",
        "2.º trimestre",
        "3.er trimestre",
        "4.º trimestre",
    ],
    era_abbrev: ("a. C.", "d. C."),
    era_names: ("antes de Cristo", "después de Cristo"),
    currency_after: true,
    short_date: "DD/MM/YYYY",
};

/// The ja-JP locale.
pub const JA_JP: Locale = Locale {
    tag: "ja-JP",
    decimal_sep: '.',
    group_sep: ',',
    list_sep: ',',
    date_order: DateOrder::YMD,
    date_sep: '/',
    time_sep: ':',
    currency_symbol: "¥",
    currency_code: "JPY",
    lcid_hex: "411",
    month_names: [
        "1月", "2月", "3月", "4月", "5月", "6月", "7月", "8月", "9月", "10月", "11月", "12月",
    ],
    short_month_names: [
        "1月", "2月", "3月", "4月", "5月", "6月", "7月", "8月", "9月", "10月", "11月", "12月",
    ],
    day_names: [
        "日曜日",
        "月曜日",
        "火曜日",
        "水曜日",
        "木曜日",
        "金曜日",
        "土曜日",
    ],
    short_day_names: ["日", "月", "火", "水", "木", "金", "土"],
    am_pm: ("午前", "午後"),
    two_digit_year_pivot: 1930,
    true_name: "TRUE",
    false_name: "FALSE",
    long_date_day_of_week_sep: " ",
    quarter_abbrev: ["Q1", "Q2", "Q3", "Q4"],
    quarter_names: ["第1四半期", "第2四半期", "第3四半期", "第4四半期"],
    era_abbrev: ("紀元前", "西暦"),
    era_names: ("紀元前", "西暦"),
    currency_after: false,
    short_date: "YYYY/MM/DD",
};

/// Every built-in locale, en-US first.
pub const LOCALES: [&Locale; 6] = [&EN_US, &EN_GB, &DE_DE, &FR_FR, &ES_ES, &JA_JP];

/// The built-in locale with this BCP 47 tag (case-insensitive; `de`,
/// `de_DE` and `de-AT` fall back to the language's locale).
pub fn locale_by_tag(tag: &str) -> Option<&'static Locale> {
    let t = tag.trim().replace('_', "-");
    if let Some(l) = LOCALES.iter().find(|l| l.tag.eq_ignore_ascii_case(&t)) {
        return Some(l);
    }
    let lang = t.split('-').next()?.to_ascii_lowercase();
    LOCALES
        .iter()
        .find(|l| l.tag.split('-').next().is_some_and(|x| x == lang))
        .copied()
}

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

    /// Looks up a month by full or abbreviated name (case-insensitive; a
    /// trailing `.` of an abbreviation is optional). English names are
    /// accepted in every locale. Returns 1..=12 and whether the name was
    /// the full one.
    pub fn month_from_name(&self, name: &str) -> Option<(u32, bool)> {
        let key = name.trim_end_matches('.').to_lowercase();
        if key.is_empty() {
            return None;
        }
        let tables = [
            (&self.month_names, &self.short_month_names),
            (&EN_US.month_names, &EN_US.short_month_names),
        ];
        for (longs, shorts) in tables {
            for (i, (long, short)) in longs.iter().zip(shorts.iter()).enumerate() {
                if key == long.to_lowercase() {
                    return Some((i as u32 + 1, true));
                }
                if key == short.trim_end_matches('.').to_lowercase() {
                    return Some((i as u32 + 1, false));
                }
            }
        }
        None
    }

    /// `true` if `c` is this locale's grouping separator (for a locale
    /// that groups with a space, any space character counts).
    pub fn is_group_char(&self, c: char) -> bool {
        c == self.group_sep
            || (self.group_sep.is_whitespace() && matches!(c, ' ' | '\u{a0}' | '\u{202f}'))
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
    fn lookup_by_tag() {
        assert_eq!(locale_by_tag("de-DE").map(|l| l.tag), Some("de-DE"));
        assert_eq!(locale_by_tag("fr").map(|l| l.tag), Some("fr-FR"));
        assert_eq!(locale_by_tag("es_MX").map(|l| l.tag), Some("es-ES"));
        assert_eq!(locale_by_tag("EN-gb").map(|l| l.tag), Some("en-GB"));
        assert!(locale_by_tag("xx-YY").is_none());
        assert_eq!(DE_DE.month_from_name("märz"), Some((3, true)));
        assert_eq!(FR_FR.month_from_name("févr."), Some((2, false)));
        assert_eq!(FR_FR.month_from_name("févr"), Some((2, false)));
        assert_eq!(
            ES_ES.month_from_name("Oct"),
            Some((10, false)),
            "English too"
        );
        assert!(FR_FR.is_group_char(' '));
        assert!(!DE_DE.is_group_char(' '));
    }

    #[test]
    fn month_lookup() {
        let l = Locale::en_us();
        assert_eq!(l.month_from_name("oct"), Some((10, false)));
        assert_eq!(l.month_from_name("OCTOBER"), Some((10, true)));
        assert_eq!(l.month_from_name("Octo"), None);
    }
}
