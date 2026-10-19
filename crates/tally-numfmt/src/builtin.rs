//! Built-in format codes behind toolbar buttons and shortcuts.

use crate::locale::Locale;

/// A quick format applied from the toolbar, a shortcut or the recognizer.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum QuickFormat {
    /// `General`.
    General,
    /// `0`.
    Integer,
    /// `0.00` (Ctrl+Shift+1).
    Number2,
    /// `#,##0`.
    Thousands,
    /// `#,##0.00`.
    Thousands2,
    /// `0%`.
    Percent0,
    /// `0.00%` (Ctrl+Shift+5, toolbar %).
    Percent,
    /// `[$$-409]#,##0.00;[RED]-[$$-409]#,##0.00` (Ctrl+Shift+4, toolbar $).
    Currency,
    /// `MM/DD/YYYY` (Ctrl+Shift+3).
    Date,
    /// `YYYY-MM-DD`.
    IsoDate,
    /// `DD MMM YYYY`.
    LongDate,
    /// `HH:MM`.
    Time,
    /// `HH:MM:SS`.
    TimeSeconds,
    /// `HH:MM AM/PM`.
    Time12,
    /// `[HH]:MM:SS`.
    Duration,
    /// `MM/DD/YYYY HH:MM`.
    DateTime,
    /// `0.00E+00` (Ctrl+Shift+2).
    Scientific,
    /// `@`.
    Text,
}

impl QuickFormat {
    /// Every quick format.
    pub const ALL: [QuickFormat; 18] = [
        QuickFormat::General,
        QuickFormat::Integer,
        QuickFormat::Number2,
        QuickFormat::Thousands,
        QuickFormat::Thousands2,
        QuickFormat::Percent0,
        QuickFormat::Percent,
        QuickFormat::Currency,
        QuickFormat::Date,
        QuickFormat::IsoDate,
        QuickFormat::LongDate,
        QuickFormat::Time,
        QuickFormat::TimeSeconds,
        QuickFormat::Time12,
        QuickFormat::Duration,
        QuickFormat::DateTime,
        QuickFormat::Scientific,
        QuickFormat::Text,
    ];
}

/// The format code for `kind` in `locale`.
///
/// Codes always use the OOo English keywords; only the currency symbol and
/// locale id come from the locale (en-US: `[$$-409]`).
pub fn builtin_format(kind: QuickFormat, locale: &Locale) -> String {
    match kind {
        QuickFormat::General => "General".into(),
        QuickFormat::Integer => "0".into(),
        QuickFormat::Number2 => "0.00".into(),
        QuickFormat::Thousands => "#,##0".into(),
        QuickFormat::Thousands2 => "#,##0.00".into(),
        QuickFormat::Percent0 => "0%".into(),
        QuickFormat::Percent => "0.00%".into(),
        QuickFormat::Currency => {
            let tag = format!("[${}-{}]", locale.currency_symbol, locale.lcid_hex);
            format!("{tag}#,##0.00;[RED]-{tag}#,##0.00")
        }
        QuickFormat::Date => "MM/DD/YYYY".into(),
        QuickFormat::IsoDate => "YYYY-MM-DD".into(),
        QuickFormat::LongDate => "DD MMM YYYY".into(),
        QuickFormat::Time => "HH:MM".into(),
        QuickFormat::TimeSeconds => "HH:MM:SS".into(),
        QuickFormat::Time12 => "HH:MM AM/PM".into(),
        QuickFormat::Duration => "[HH]:MM:SS".into(),
        QuickFormat::DateTime => "MM/DD/YYYY HH:MM".into(),
        QuickFormat::Scientific => "0.00E+00".into(),
        QuickFormat::Text => "@".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_parse() {
        let l = Locale::en_us();
        for k in QuickFormat::ALL {
            let code = builtin_format(k, &l);
            let ast = crate::parse_format(&code).unwrap();
            assert_eq!(crate::print_format(&ast), code);
        }
        assert_eq!(
            builtin_format(QuickFormat::Currency, &l),
            "[$$-409]#,##0.00;[RED]-[$$-409]#,##0.00"
        );
    }
}
