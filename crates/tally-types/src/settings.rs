//! Per-document calculation settings.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Day zero of date serial numbers.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum DateEpoch {
    /// Serial 0 is 1899-12-30 (OpenOffice / LibreOffice default).
    #[default]
    D1899,
    /// Serial 0 is 1900-01-01 (StarCalc 1.0 convention; no fictitious
    /// 1900-02-29). XLSX import/export must shift Excel serials 1..60.
    D1900,
    /// Serial 0 is 1904-01-01 (classic Mac Excel).
    D1904,
}

/// How text is converted when used where a number is expected.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum TextToNumber {
    /// Any text yields `#VALUE!`.
    GenerateError,
    /// Any text counts as 0.
    TreatAsZero,
    /// Plain numbers and ISO 8601 dates convert; anything else is `#VALUE!`.
    #[default]
    ConvertUnambiguous,
    /// Convert using the document locale's number and date recognition.
    ConvertLocale,
}

/// How criteria strings (`COUNTIF`, `SUMIF`, `MATCH` type 0, …) match text.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum CriteriaMode {
    /// `*`, `?` and `~` wildcards (LibreOffice / Excel default).
    #[default]
    Wildcards,
    /// Regular expressions (the Apache OpenOffice default).
    Regex,
    /// Plain literal comparison.
    Literal,
}

/// Iterative calculation settings for circular references.
#[derive(Copy, Clone, PartialEq, Debug)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct IterSettings {
    /// Maximum number of iterations.
    pub steps: u32,
    /// Stop when no result changes by more than this.
    pub min_change: f64,
}

impl Default for IterSettings {
    /// OpenOffice's defaults: 100 steps, minimum change 0.001.
    fn default() -> Self {
        Self {
            steps: 100,
            min_change: 0.001,
        }
    }
}

/// Settings that change formula results. Stored per document.
#[derive(Clone, PartialEq, Debug)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct CalcSettings {
    /// Day zero of date serials.
    pub date_epoch: DateEpoch,
    /// Text-to-number conversion in arithmetic.
    pub text_to_number: TextToNumber,
    /// How criteria strings match.
    pub criteria_mode: CriteriaMode,
    /// Whether text comparisons are case-sensitive (default `false`; AOO's
    /// default is `true`, a deliberate deviation, see docs/adr/0001-stack.md).
    pub case_sensitive: bool,
    /// Whether `=` and `<>` criteria must match whole cells (default `true`).
    pub whole_cell_match: bool,
    /// Iterative calculation; `None` (default) means cycles are `Err:522`.
    pub iterations: Option<IterSettings>,
    /// Round values to their displayed precision before use (default `false`).
    pub precision_as_shown: bool,
    /// First year of the two-digit year window: `yy` is read as a year in
    /// `start..=start + 99` (AOO `ScDocOptions::nYear2000`, ODS
    /// `table:null-year`, default 1930). `None` until the document sets it:
    /// new documents and files without `table:calculation-settings` take
    /// the application option (AOO Tools ▸ Options ▸ General ▸ "Year (two
    /// digits)", `ScDocShell::InitOptions`); see [`Self::year2000`].
    #[cfg_attr(feature = "serde", serde(default))]
    pub two_digit_year: Option<u16>,
    /// Decimals of the General format (AOO Tools ▸ Options ▸ Calc ▸
    /// Calculate ▸ "Limit decimals for general number format", 0..=20,
    /// `ScDocOptions::nPrecStandardFormat`; ODS `style:decimal-places` of
    /// the default cell style). `None` = unlimited (the default).
    #[cfg_attr(feature = "serde", serde(default))]
    pub standard_decimals: Option<u8>,
}

/// AOO's default two-digit year window start (`Office.Common/DateFormat/TwoDigitYear`).
pub const DEFAULT_TWO_DIGIT_YEAR: u16 = 1930;
/// Range of the two-digit year option (AOO's `NF_YEARVALUE` field, optgdlg.src).
pub const TWO_DIGIT_YEAR_RANGE: std::ops::RangeInclusive<u16> = 1583..=9857;
/// Most decimals of the General format limit (AOO's `ED_PREC` field, optdlg.src).
pub const MAX_STANDARD_DECIMALS: u8 = 20;

impl CalcSettings {
    /// The two-digit year window start in effect (1930 when unset).
    pub fn year2000(&self) -> u16 {
        self.two_digit_year.unwrap_or(DEFAULT_TWO_DIGIT_YEAR)
    }
}

/// Expands a two-digit year into the window starting at `start` (AOO
/// `SvNumberFormatter::ExpandTwoDigitYear`, zforlist.hxx): with 1930,
/// 30..=99 → 1930..=1999 and 0..=29 → 2000..=2029. Other years are
/// returned unchanged.
pub fn expand_two_digit_year(yy: i32, start: u16) -> i32 {
    if !(0..100).contains(&yy) {
        return yy;
    }
    let start = i32::from(start);
    if yy < start % 100 {
        yy + (start / 100 + 1) * 100
    } else {
        yy + (start / 100) * 100
    }
}

impl Default for CalcSettings {
    fn default() -> Self {
        Self {
            date_epoch: DateEpoch::default(),
            text_to_number: TextToNumber::default(),
            criteria_mode: CriteriaMode::default(),
            case_sensitive: false,
            whole_cell_match: true,
            iterations: None,
            precision_as_shown: false,
            two_digit_year: None,
            standard_decimals: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_adr() {
        let s = CalcSettings::default();
        assert_eq!(s.date_epoch, DateEpoch::D1899);
        assert_eq!(s.text_to_number, TextToNumber::ConvertUnambiguous);
        assert_eq!(s.criteria_mode, CriteriaMode::Wildcards);
        assert!(!s.case_sensitive);
        assert!(s.whole_cell_match);
        assert_eq!(s.iterations, None);
        assert!(!s.precision_as_shown);
        let it = IterSettings::default();
        assert_eq!((it.steps, it.min_change), (100, 0.001));
        assert_eq!(s.year2000(), 1930);
        assert_eq!(s.standard_decimals, None);
    }

    #[test]
    fn two_digit_year_windows() {
        // AOO ExpandTwoDigitYear: the window is start..=start + 99.
        assert_eq!(expand_two_digit_year(45, 1930), 1945);
        assert_eq!(expand_two_digit_year(29, 1930), 2029);
        assert_eq!(expand_two_digit_year(30, 1930), 1930);
        assert_eq!(expand_two_digit_year(45, 1950), 2045);
        assert_eq!(expand_two_digit_year(50, 1950), 1950);
        assert_eq!(expand_two_digit_year(49, 1950), 2049);
        assert_eq!(expand_two_digit_year(0, 2000), 2000);
        assert_eq!(expand_two_digit_year(99, 2000), 2099);
        assert_eq!(expand_two_digit_year(5, 1583), 1605);
        assert_eq!(expand_two_digit_year(83, 1583), 1583);
        assert_eq!(expand_two_digit_year(1945, 1950), 1945);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_round_trip() {
        let s = CalcSettings {
            date_epoch: DateEpoch::D1904,
            criteria_mode: CriteriaMode::Regex,
            iterations: Some(IterSettings::default()),
            ..CalcSettings::default()
        };
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(serde_json::from_str::<CalcSettings>(&json).unwrap(), s);
    }
}
