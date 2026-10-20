//! Plain interchange model for Tally workbooks.
//!
//! [`WorkbookData`] is what file readers produce and writers consume
//! (`tally-io-ods`, `tally-io-csv`), and what the engine converts to and from
//! with `Workbook::from_data` / `to_data`. It has no engine dependency and no
//! behaviour: just data with serde derives.
//!
//! This is a frozen WP0 contract; WP5 may add fields through a lead-reviewed
//! PR (see `docs/contributing.md`).
//!
//! Conventions:
//! - rows and columns are 0-based;
//! - colours are `0xRRGGBBAA` packed into a `u32`;
//! - lengths are in 1/100 mm (`_hmm`), like ODF.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tally_types::{CalcSettings, Value};

pub mod charts;
pub mod pivot;
pub mod tools;

pub use charts::*;
pub use pivot::*;
pub use tools::*;

/// Longest number format code kept (characters). Readers replace longer
/// codes with General and add a warning; the engine refuses longer codes
/// in `SetNumberFormat` (security review 2, HIGH-4: a 1 MB code made every
/// visible cell format to a 1 MB string). Excel's own limit is 255.
pub const MAX_NUMBER_FORMAT_LEN: usize = 4096;

/// The warning readers give when they drop an over-long number format.
pub fn long_number_format_warning() -> String {
    format!(
        "Number formats longer than {MAX_NUMBER_FORMAT_LEN} characters were replaced by General."
    )
}

/// A whole workbook.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct WorkbookData {
    /// Sheets in tab order.
    pub sheets: Vec<SheetData>,
    /// Style table; [`CellData::style`] indexes into it.
    pub styles: Vec<StyleData>,
    /// Named expressions.
    pub names: Vec<NameData>,
    /// Document settings.
    pub settings: DocSettings,
    /// Document metadata.
    pub meta: DocMeta,
    /// What a reader dropped or changed to keep the file within its limits
    /// (shown to the user once when the file opens). Writers ignore it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    /// Named cell styles with their parent chains (v0.4). Empty means only
    /// the built-in `Default`.
    #[serde(default)]
    pub cell_styles: Vec<CellStyleData>,
}

/// One sheet.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct SheetData {
    /// Sheet (tab) name.
    pub name: String,
    /// Non-empty or styled cells, sorted row-major (by row, then column).
    pub cells: Vec<CellData>,
    /// Column widths that differ from the default.
    pub col_widths: Vec<DimRun>,
    /// Row heights that differ from the default.
    pub row_heights: Vec<DimRun>,
    /// Merged areas.
    pub merges: Vec<CellRangeLocal>,
    /// Frozen panes as `(rows, cols)` frozen at the top/left.
    pub frozen: Option<(u32, u32)>,
    /// Tab colour (`0xRRGGBBAA`).
    pub tab_color: Option<u32>,
    /// Whether the sheet is hidden.
    pub hidden: bool,
    /// Long vertical style runs over empty cells (formatting of blank areas
    /// such as a formatted whole column), kept as runs instead of styled
    /// empty [`cells`](Self::cells). A cell listed in `cells` uses its own
    /// style; positions without a cell take the run's style. Runs apply in
    /// order, so a later run overrides an earlier one where they overlap
    /// (the ODS reader lists column default styles first); the engine
    /// writes them sorted by column, then start row, without overlaps.
    #[serde(default)]
    pub style_runs: Vec<StyleRun>,
    /// Array (Ctrl+Shift+Enter) formula blocks. The anchor cell (`r0`,
    /// `c0`) holds the formula in [`cells`](Self::cells); the other cells of
    /// the block hold their cached result values (ODS writes them as plain
    /// value cells) and are replaced by the block on load.
    #[serde(default)]
    pub array_formulas: Vec<CellRangeLocal>,
    /// The sheet's AutoFilter, if any (ODS: an anonymous
    /// `table:database-range` with `table:display-filter-buttons`).
    #[serde(default)]
    pub auto_filter: Option<AutoFilterData>,
    /// Conditional formats, in priority order (v0.3).
    #[serde(default)]
    pub conditional_formats: Vec<ConditionalFormatData>,
    /// Data validation rules (v0.3).
    #[serde(default)]
    pub validations: Vec<ValidationData>,
    /// Cell notes (comments), sorted row-major (v0.3).
    #[serde(default)]
    pub notes: Vec<NoteData>,
    /// Cell hyperlinks (a cell attribute, separate from `HYPERLINK()`
    /// formulas), sorted row-major (v0.3).
    #[serde(default)]
    pub links: Vec<LinkData>,
    /// Sheet protection; `None` when the sheet is not protected.
    #[serde(default)]
    pub protection: Option<SheetProtectionData>,
    /// Page setup; `None` for the defaults ([`PageSetupData::default`]).
    #[serde(default)]
    pub page_setup: Option<PageSetupData>,
    /// Manual page breaks before these rows (sorted).
    #[serde(default)]
    pub row_breaks: Vec<u32>,
    /// Manual page breaks before these columns (sorted).
    #[serde(default)]
    pub col_breaks: Vec<u32>,
    /// Charts anchored on this sheet (v0.4, W5-1).
    #[serde(default)]
    pub charts: Vec<ChartData>,
    /// Pictures anchored on this sheet (v0.4, W5-1).
    #[serde(default)]
    pub images: Vec<ImageData>,
    /// DataPilot tables whose output is on this sheet (v0.5, W5-2).
    #[serde(default)]
    pub pivots: Vec<PivotData>,
    /// Row outline groups (v0.5).
    #[serde(default)]
    pub row_groups: Vec<OutlineGroupData>,
    /// Column outline groups (v0.5).
    #[serde(default)]
    pub col_groups: Vec<OutlineGroupData>,
    /// Scenarios of this (base) sheet (v0.5).
    #[serde(default)]
    pub scenarios: Vec<ScenarioData>,
    /// Named database ranges on this sheet (v0.5).
    #[serde(default)]
    pub database_ranges: Vec<DatabaseRangeData>,
    /// The standard/advanced filter of the anonymous database range (the
    /// AutoFilter area), if one is set (v0.5).
    #[serde(default)]
    pub standard_filter: Option<StandardFilterData>,
    /// The Data ▸ Subtotals definition, kept for "Remove" (v0.5).
    #[serde(default)]
    pub subtotals: Option<SubtotalsData>,
    /// View toggles; `None` for the defaults (v0.5).
    #[serde(default)]
    pub view_options: Option<ViewOptionsData>,
    /// Window ▸ Split position (v0.5).
    #[serde(default)]
    pub split: Option<SplitData>,
}

// ---------------------------------------------------------------------------
// Conditional formatting
// ---------------------------------------------------------------------------

/// A conditional format: rules applied to one or more ranges.
///
/// Formulas are OpenFormula text (`of:=[.A1]>5`) relative to `base`, a
/// cell on the same sheet (ODS `calcext:base-cell-address`; normally the
/// top-left cell of the first range).
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct ConditionalFormatData {
    /// The ranges the rules apply to.
    pub ranges: Vec<CellRangeLocal>,
    /// Base cell `(row, col)` of the relative references in the formulas.
    pub base: (u32, u32),
    /// Rules in priority order (the first matching style wins per
    /// property; colour scales, data bars and icon sets combine).
    pub rules: Vec<CfRuleData>,
}

/// One conditional formatting rule.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CfRuleData {
    /// What the rule tests.
    pub kind: CfKindData,
    /// The style applied where the rule matches (unused by colour scales,
    /// data bars and icon sets).
    #[serde(default)]
    pub style: CfStyleData,
    /// Excel's "stop if true".
    #[serde(default)]
    pub stop_if_true: bool,
}

/// A differential style: only the fields that are set apply.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct CfStyleData {
    /// Bold.
    pub bold: Option<bool>,
    /// Italic.
    pub italic: Option<bool>,
    /// Underline.
    pub underline: Option<bool>,
    /// Strikethrough.
    pub strike: Option<bool>,
    /// Text colour (`0xRRGGBBAA`).
    pub color: Option<u32>,
    /// Background fill (`0xRRGGBBAA`).
    pub fill: Option<u32>,
    /// Borders to draw (`None` edges are unchanged).
    pub borders: Option<BordersData>,
    /// Number format code.
    pub number_format: Option<String>,
    /// Chip display.
    pub display: Option<DisplayData>,
}

impl CfStyleData {
    /// `true` if nothing is set.
    pub fn is_empty(&self) -> bool {
        *self == CfStyleData::default()
    }
}

/// Comparison operators of cell-value rules and validations.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CmpOpData {
    /// Between the two values (inclusive).
    Between,
    /// Outside the two values.
    NotBetween,
    /// `=`
    Eq,
    /// `<>`
    Ne,
    /// `>`
    Gt,
    /// `>=`
    Ge,
    /// `<`
    Lt,
    /// `<=`
    Le,
}

/// Text tests of [`CfKindData::Text`].
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CfTextOpData {
    /// Contains the text.
    Contains,
    /// Does not contain the text.
    NotContains,
    /// Begins with the text.
    BeginsWith,
    /// Ends with the text.
    EndsWith,
}

/// Date periods of [`CfKindData::DateIs`] (ODS `calcext:date`).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CfDatePeriodData {
    /// Today.
    Today,
    /// Yesterday.
    Yesterday,
    /// Tomorrow.
    Tomorrow,
    /// The last 7 days including today.
    Last7Days,
    /// This week (Sunday to Saturday).
    ThisWeek,
    /// Last week.
    LastWeek,
    /// Next week.
    NextWeek,
    /// This month.
    ThisMonth,
    /// Last month.
    LastMonth,
    /// Next month.
    NextMonth,
    /// This year.
    ThisYear,
    /// Last year.
    LastYear,
    /// Next year.
    NextYear,
}

/// How a colour scale stop, data bar end or icon threshold is computed.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CfValueKindData {
    /// The lowest value in the range.
    Min,
    /// The highest value in the range.
    Max,
    /// `value` percent of the way from the minimum to the maximum.
    Percent,
    /// The `value`th percentile of the range's numbers.
    Percentile,
    /// The number `value`.
    Number,
    /// The result of the formula `value`.
    Formula,
    /// Data bars: the minimum or 0, whichever is lower.
    AutoMin,
    /// Data bars: the maximum or 0, whichever is higher.
    AutoMax,
}

/// A threshold: a kind and its operand (a number as text, or an
/// OpenFormula `of:=…` for [`CfValueKindData::Formula`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CfThresholdData {
    /// The kind.
    pub kind: CfValueKindData,
    /// The operand (`""` for min/max).
    pub value: String,
}

/// A colour scale stop.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CfColorStopData {
    /// Where the stop is.
    pub threshold: CfThresholdData,
    /// Its colour (`0xRRGGBBAA`).
    pub color: u32,
}

/// Icon sets (the names are the OOXML/ODS `iconSet` names).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum IconSetData {
    /// `3Arrows`
    ThreeArrows,
    /// `3ArrowsGray`
    ThreeArrowsGray,
    /// `3Flags`
    ThreeFlags,
    /// `3TrafficLights1`
    ThreeTrafficLights,
    /// `3TrafficLights2`
    ThreeTrafficLightsRimmed,
    /// `3Signs`
    ThreeSigns,
    /// `3Symbols`
    ThreeSymbols,
    /// `3Symbols2`
    ThreeSymbolsUncircled,
    /// `3Stars`
    ThreeStars,
    /// `3Triangles`
    ThreeTriangles,
    /// `4Arrows`
    FourArrows,
    /// `4ArrowsGray`
    FourArrowsGray,
    /// `4RedToBlack`
    FourRedToBlack,
    /// `4Rating`
    FourRating,
    /// `4TrafficLights`
    FourTrafficLights,
    /// `5Arrows`
    FiveArrows,
    /// `5ArrowsGray`
    FiveArrowsGray,
    /// `5Rating`
    FiveRating,
    /// `5Quarters`
    FiveQuarters,
    /// `5Boxes`
    FiveBoxes,
}

impl IconSetData {
    /// Every icon set.
    pub const ALL: [IconSetData; 20] = [
        IconSetData::ThreeArrows,
        IconSetData::ThreeArrowsGray,
        IconSetData::ThreeFlags,
        IconSetData::ThreeTrafficLights,
        IconSetData::ThreeTrafficLightsRimmed,
        IconSetData::ThreeSigns,
        IconSetData::ThreeSymbols,
        IconSetData::ThreeSymbolsUncircled,
        IconSetData::ThreeStars,
        IconSetData::ThreeTriangles,
        IconSetData::FourArrows,
        IconSetData::FourArrowsGray,
        IconSetData::FourRedToBlack,
        IconSetData::FourRating,
        IconSetData::FourTrafficLights,
        IconSetData::FiveArrows,
        IconSetData::FiveArrowsGray,
        IconSetData::FiveRating,
        IconSetData::FiveQuarters,
        IconSetData::FiveBoxes,
    ];

    /// The OOXML / ODS name, e.g. `3Arrows`.
    pub fn file_name(self) -> &'static str {
        match self {
            IconSetData::ThreeArrows => "3Arrows",
            IconSetData::ThreeArrowsGray => "3ArrowsGray",
            IconSetData::ThreeFlags => "3Flags",
            IconSetData::ThreeTrafficLights => "3TrafficLights1",
            IconSetData::ThreeTrafficLightsRimmed => "3TrafficLights2",
            IconSetData::ThreeSigns => "3Signs",
            IconSetData::ThreeSymbols => "3Symbols",
            IconSetData::ThreeSymbolsUncircled => "3Symbols2",
            IconSetData::ThreeStars => "3Stars",
            IconSetData::ThreeTriangles => "3Triangles",
            IconSetData::FourArrows => "4Arrows",
            IconSetData::FourArrowsGray => "4ArrowsGray",
            IconSetData::FourRedToBlack => "4RedToBlack",
            IconSetData::FourRating => "4Rating",
            IconSetData::FourTrafficLights => "4TrafficLights",
            IconSetData::FiveArrows => "5Arrows",
            IconSetData::FiveArrowsGray => "5ArrowsGray",
            IconSetData::FiveRating => "5Rating",
            IconSetData::FiveQuarters => "5Quarters",
            IconSetData::FiveBoxes => "5Boxes",
        }
    }

    /// The set named `name` (`3TrafficLights` is accepted for
    /// `3TrafficLights1`).
    pub fn from_file_name(name: &str) -> Option<IconSetData> {
        if name == "3TrafficLights" {
            return Some(IconSetData::ThreeTrafficLights);
        }
        IconSetData::ALL.into_iter().find(|s| s.file_name() == name)
    }

    /// Number of icons (3, 4 or 5).
    pub fn icon_count(self) -> usize {
        match self.file_name().as_bytes().first() {
            Some(b'4') => 4,
            Some(b'5') => 5,
            _ => 3,
        }
    }
}

/// What a conditional formatting rule tests.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum CfKindData {
    /// Compare the cell value (1 formula, 2 for between).
    CellValue {
        /// Comparison.
        op: CmpOpData,
        /// Operands as OpenFormula (`of:=5`, `of:=[.B1]`, `of:="x"`).
        formulas: Vec<String>,
    },
    /// A formula that is true for matching cells.
    Formula {
        /// OpenFormula text.
        formula: String,
    },
    /// A 2- or 3-colour scale.
    ColorScale {
        /// The stops, lowest first.
        stops: Vec<CfColorStopData>,
    },
    /// A data bar.
    DataBar {
        /// The value of an empty bar.
        min: CfThresholdData,
        /// The value of a full bar.
        max: CfThresholdData,
        /// Bar colour.
        color: u32,
        /// Colour of negative bars, if different.
        negative_color: Option<u32>,
        /// Show the cell value next to the bar.
        show_value: bool,
        /// Gradient fill.
        gradient: bool,
    },
    /// An icon set.
    IconSet {
        /// Which icons.
        set: IconSetData,
        /// One threshold per icon, lowest first (the first is ignored:
        /// icon 0 is used below threshold 1).
        thresholds: Vec<CfThresholdData>,
        /// Show the cell value next to the icon.
        show_value: bool,
        /// Reverse the icon order.
        reverse: bool,
    },
    /// Values that occur more than once in the ranges.
    Duplicate,
    /// Values that occur once.
    Unique,
    /// The top or bottom N (or N percent) values.
    Top {
        /// N.
        rank: u32,
        /// N is a percentage.
        percent: bool,
        /// Bottom instead of top.
        bottom: bool,
    },
    /// Above or below the average.
    Average {
        /// Above (`true`) or below.
        above: bool,
        /// Include values equal to the average.
        equal: bool,
    },
    /// A text test (case-insensitive).
    Text {
        /// Test.
        op: CfTextOpData,
        /// The text.
        text: String,
    },
    /// The cell is empty (or holds only spaces).
    Blank,
    /// The cell is not empty.
    NotBlank,
    /// The cell holds an error.
    Error,
    /// The cell does not hold an error.
    NotError,
    /// A date in a period relative to today.
    DateIs {
        /// The period.
        period: CfDatePeriodData,
    },
}

// ---------------------------------------------------------------------------
// Data validation
// ---------------------------------------------------------------------------

/// A data validation rule over one or more ranges. Formulas are OpenFormula
/// relative to `base` (see [`ConditionalFormatData`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ValidationData {
    /// The ranges.
    pub ranges: Vec<CellRangeLocal>,
    /// Base cell `(row, col)` of relative references.
    pub base: (u32, u32),
    /// What is allowed.
    pub kind: ValidationKindData,
    /// Empty cells are valid.
    pub allow_blank: bool,
    /// Show the list dropdown (list rules).
    pub show_dropdown: bool,
    /// Help shown when the cell is selected.
    pub input_help: Option<ValidationMessageData>,
    /// The alert for invalid input; `None` accepts invalid input silently.
    pub error: Option<ValidationErrorData>,
}

/// An input help message.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct ValidationMessageData {
    /// Title.
    pub title: String,
    /// Text.
    pub text: String,
}

/// Alert styles for invalid input.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ValidationErrorStyleData {
    /// Reject the input.
    #[default]
    Stop,
    /// Warn; the user may keep the input.
    Warning,
    /// Inform; the input is kept.
    Info,
}

/// An error alert.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct ValidationErrorData {
    /// Style.
    pub style: ValidationErrorStyleData,
    /// Title.
    pub title: String,
    /// Text.
    pub text: String,
}

/// Where list items come from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ListSourceData {
    /// Literal items.
    Values(Vec<String>),
    /// A range (or any expression giving the items) as OpenFormula.
    Range(String),
}

/// What a validation allows.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ValidationKindData {
    /// Anything.
    Any,
    /// One of a list.
    List {
        /// The items.
        source: ListSourceData,
    },
    /// Whole numbers.
    Whole {
        /// Comparison.
        op: CmpOpData,
        /// Operands (OpenFormula).
        formulas: Vec<String>,
    },
    /// Decimal numbers.
    Decimal {
        /// Comparison.
        op: CmpOpData,
        /// Operands.
        formulas: Vec<String>,
    },
    /// Dates.
    Date {
        /// Comparison.
        op: CmpOpData,
        /// Operands.
        formulas: Vec<String>,
    },
    /// Times.
    Time {
        /// Comparison.
        op: CmpOpData,
        /// Operands.
        formulas: Vec<String>,
    },
    /// Text whose length compares.
    TextLength {
        /// Comparison.
        op: CmpOpData,
        /// Operands.
        formulas: Vec<String>,
    },
    /// A formula that must be true.
    Custom {
        /// OpenFormula.
        formula: String,
    },
}

// ---------------------------------------------------------------------------
// Notes, links, protection, page setup
// ---------------------------------------------------------------------------

/// A cell note (comment).
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct NoteData {
    /// Row.
    pub row: u32,
    /// Column.
    pub col: u32,
    /// Plain text (lines separated by `\n`).
    pub text: String,
    /// Author.
    pub author: Option<String>,
    /// Creation time (ISO 8601).
    pub created: Option<String>,
    /// Always shown (not only on hover).
    pub shown: bool,
}

/// A hyperlink on a cell.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct LinkData {
    /// Row.
    pub row: u32,
    /// Column.
    pub col: u32,
    /// The URL (or `#Sheet1.A1` for an in-document target). Shared, so a
    /// link that a file applies to a whole range costs one allocation.
    pub url: std::sync::Arc<str>,
}

/// Well-known values of [`SheetProtectionData::algorithm`].
pub mod hash_algorithms {
    /// SHA-256 of the UTF-8 password, base64 (ODF 1.2, LibreOffice).
    pub const SHA256: &str = "sha256";
    /// SHA-1 of the UTF-8 password, base64 (ODF 1.0/1.1).
    pub const SHA1: &str = "sha1";
    /// Excel's 16-bit legacy hash as 4 hex digits (OOXML `password`).
    pub const XLSX_LEGACY: &str = "xlsx-legacy";
}

/// Sheet (or document structure) protection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SheetProtectionData {
    /// The password hash, or `None` without a password.
    pub password_hash: Option<String>,
    /// The hash algorithm: one of [`hash_algorithms`], an OOXML
    /// `algorithmName` such as `SHA-512`, or an ODF algorithm URI.
    pub algorithm: Option<String>,
    /// OOXML salt (base64).
    pub salt: Option<String>,
    /// OOXML spin count.
    pub spin_count: Option<u32>,
    /// A second hash of the same password in Excel's legacy 16-bit form
    /// (4 hex digits), kept so that a password set in Tally survives an
    /// XLSX save when the main hash is an ODF one.
    #[serde(default)]
    pub xlsx_legacy_hash: Option<String>,
    /// Locked cells can be selected.
    pub select_locked: bool,
    /// Unlocked cells can be selected.
    pub select_unlocked: bool,
    /// Rows may be inserted.
    pub insert_rows: bool,
    /// Columns may be inserted.
    pub insert_cols: bool,
    /// Rows may be deleted.
    pub delete_rows: bool,
    /// Columns may be deleted.
    pub delete_cols: bool,
    /// Cells may be formatted.
    pub format_cells: bool,
    /// Ranges may be sorted.
    pub sort: bool,
    /// AutoFilters may be used.
    pub auto_filter: bool,
}

impl Default for SheetProtectionData {
    fn default() -> Self {
        Self {
            password_hash: None,
            algorithm: None,
            salt: None,
            spin_count: None,
            xlsx_legacy_hash: None,
            select_locked: true,
            select_unlocked: true,
            insert_rows: false,
            insert_cols: false,
            delete_rows: false,
            delete_cols: false,
            format_cells: false,
            sort: false,
            auto_filter: false,
        }
    }
}

/// Cell protection attributes of a style (`style:cell-protect`, OOXML
/// `protection`). The default (no [`StyleData::protection`]) is locked and
/// not hidden, as in OOo and Excel.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CellProtectionData {
    /// Locked: cannot be edited while the sheet is protected.
    pub locked: bool,
    /// The formula is hidden while the sheet is protected.
    pub hidden_formula: bool,
    /// The whole cell is hidden (ODS `hidden-and-protected`).
    pub hidden: bool,
    /// Not printed (ODS `style:print-content="false"`).
    pub hide_print: bool,
}

impl Default for CellProtectionData {
    fn default() -> Self {
        Self {
            locked: true,
            hidden_formula: false,
            hidden: false,
            hide_print: false,
        }
    }
}

/// Page orientation.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum OrientationData {
    /// Portrait.
    #[default]
    Portrait,
    /// Landscape.
    Landscape,
}

/// Paper sizes (portrait width × height).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum PaperData {
    /// 210 × 297 mm.
    #[default]
    A4,
    /// 297 × 420 mm.
    A3,
    /// 148 × 210 mm.
    A5,
    /// 8.5 × 11 in.
    Letter,
    /// 8.5 × 14 in.
    Legal,
    /// 7.25 × 10.5 in.
    Executive,
    /// 11 × 17 in.
    Tabloid,
    /// 176 × 250 mm.
    B5,
    /// Any other size, in 1/100 mm.
    Custom {
        /// Width.
        width_hmm: u32,
        /// Height.
        height_hmm: u32,
    },
}

impl PaperData {
    /// Portrait `(width, height)` in 1/100 mm.
    pub fn size_hmm(self) -> (u32, u32) {
        match self {
            PaperData::A4 => (21000, 29700),
            PaperData::A3 => (29700, 42000),
            PaperData::A5 => (14800, 21000),
            PaperData::Letter => (21590, 27940),
            PaperData::Legal => (21590, 35560),
            PaperData::Executive => (18415, 26670),
            PaperData::Tabloid => (27940, 43180),
            PaperData::B5 => (17600, 25000),
            PaperData::Custom {
                width_hmm,
                height_hmm,
            } => (width_hmm, height_hmm),
        }
    }

    /// The named paper with this portrait size (±1 mm), else `Custom`.
    pub fn from_size_hmm(w: u32, h: u32) -> PaperData {
        let (w, h) = (w.min(h), w.max(h));
        for p in [
            PaperData::A4,
            PaperData::A3,
            PaperData::A5,
            PaperData::Letter,
            PaperData::Legal,
            PaperData::Executive,
            PaperData::Tabloid,
            PaperData::B5,
        ] {
            let (pw, ph) = p.size_hmm();
            if pw.abs_diff(w) <= 100 && ph.abs_diff(h) <= 100 {
                return p;
            }
        }
        PaperData::Custom {
            width_hmm: w,
            height_hmm: h,
        }
    }
}

/// Print scaling.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PageScaleData {
    /// A zoom percentage (10..=400).
    Percent(u32),
    /// Fit the print range into `width × height` pages (`0` = any number).
    FitTo {
        /// Pages wide.
        width: u32,
        /// Pages tall.
        height: u32,
    },
    /// Fit the print range into this many pages in total (ODS
    /// `style:scale-to-pages`; v0.4).
    FitPages(u32),
}

impl Default for PageScaleData {
    fn default() -> Self {
        PageScaleData::Percent(100)
    }
}

/// Page margins in 1/100 mm.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MarginsData {
    /// Top.
    pub top: u32,
    /// Bottom.
    pub bottom: u32,
    /// Left.
    pub left: u32,
    /// Right.
    pub right: u32,
    /// Header (distance of the header from the top edge).
    pub header: u32,
    /// Footer.
    pub footer: u32,
}

impl Default for MarginsData {
    /// LibreOffice's defaults: 2 cm all round, 0.75 cm header/footer.
    fn default() -> Self {
        Self {
            top: 2000,
            bottom: 2000,
            left: 2000,
            right: 2000,
            header: 750,
            footer: 750,
        }
    }
}

/// A header or footer: three regions of text with placeholders `&[PAGE]`,
/// `&[PAGES]`, `&[SHEET]`, `&[FILE]`, `&[DATE]`, `&[TIME]`.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct HeaderFooterData {
    /// Left region.
    pub left: String,
    /// Centre region.
    pub center: String,
    /// Right region.
    pub right: String,
}

impl HeaderFooterData {
    /// `true` if all regions are empty.
    pub fn is_empty(&self) -> bool {
        self.left.is_empty() && self.center.is_empty() && self.right.is_empty()
    }
}

/// Print order of pages.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum PageOrderData {
    /// Down, then over.
    #[default]
    TopToBottom,
    /// Over, then down.
    LeftToRight,
}

/// Page setup of one sheet.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PageSetupData {
    /// Orientation.
    pub orientation: OrientationData,
    /// Paper.
    pub paper: PaperData,
    /// Margins.
    pub margins: MarginsData,
    /// Scaling.
    pub scale: PageScaleData,
    /// The print range (`None`: the used area).
    pub print_range: Option<CellRangeLocal>,
    /// Rows repeated at the top of each page, inclusive.
    pub repeat_rows: Option<(u32, u32)>,
    /// Columns repeated at the left of each page, inclusive.
    pub repeat_cols: Option<(u32, u32)>,
    /// Header (`None`: no header).
    pub header: Option<HeaderFooterData>,
    /// Footer (`None`: no footer).
    pub footer: Option<HeaderFooterData>,
    /// Print cell gridlines.
    pub print_gridlines: bool,
    /// Print row and column headers.
    pub print_headers: bool,
    /// Centre horizontally.
    pub center_h: bool,
    /// Centre vertically.
    pub center_v: bool,
    /// Page order.
    pub page_order: PageOrderData,
    /// First page number (`None`: continue / 1).
    pub first_page_number: Option<u32>,
    /// Print charts and pictures (v0.4; AOO default `true`).
    #[serde(default = "default_true")]
    pub print_charts: bool,
    /// How notes are printed (v0.4).
    #[serde(default)]
    pub print_notes: NotesPrintData,
    /// Print zero values (v0.4; default `true`).
    #[serde(default = "default_true")]
    pub print_zero_values: bool,
    /// Print formulas instead of results (v0.4).
    #[serde(default)]
    pub print_formulas: bool,
}

fn default_true() -> bool {
    true
}

impl Default for PageSetupData {
    fn default() -> Self {
        Self {
            orientation: OrientationData::Portrait,
            paper: PaperData::A4,
            margins: MarginsData::default(),
            scale: PageScaleData::Percent(100),
            print_range: None,
            repeat_rows: None,
            repeat_cols: None,
            header: Some(HeaderFooterData {
                center: "&[SHEET]".into(),
                ..HeaderFooterData::default()
            }),
            footer: Some(HeaderFooterData {
                center: "Page &[PAGE]".into(),
                ..HeaderFooterData::default()
            }),
            print_gridlines: false,
            print_headers: false,
            center_h: false,
            center_v: false,
            page_order: PageOrderData::TopToBottom,
            first_page_number: None,
            print_charts: true,
            print_notes: NotesPrintData::None,
            print_zero_values: true,
            print_formulas: false,
        }
    }
}

/// An AutoFilter: the filtered area (header row first), the per-column
/// criteria and the rows they currently hide.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct AutoFilterData {
    /// The area including its header row (`r0`).
    pub range: CellRangeLocal,
    /// Active column filters (absolute column numbers inside `range`).
    pub columns: Vec<FilterColumnData>,
    /// Rows hidden by the filter, as inclusive `(first, last)` spans,
    /// sorted. These are distinct from user-hidden rows
    /// ([`DimRun::hidden`]); ODS writes them as `table:visibility="filter"`.
    pub filtered_rows: Vec<(u32, u32)>,
}

/// The filter of one AutoFilter column. A row is kept when its cell passes
/// both parts that are set.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct FilterColumnData {
    /// Absolute 0-based column.
    pub col: u32,
    /// Keep only cells whose formatted text is one of these (`""` = empty).
    pub values: Option<Vec<String>>,
    /// Keep only cells matching this condition.
    pub condition: Option<FilterConditionData>,
}

/// One filter condition.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FilterConditionData {
    /// Comparison.
    pub op: FilterOpData,
    /// Operand (numbers compare numerically when both sides are numbers).
    pub value: String,
}

/// Filter comparison operators (ODS `table:operator`).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FilterOpData {
    /// `=`
    Eq,
    /// `!=`
    Ne,
    /// `>`
    Gt,
    /// `>=`
    Ge,
    /// `<`
    Lt,
    /// `<=`
    Le,
    /// `contains`
    Contains,
    /// `begins`
    BeginsWith,
    /// `ends`
    EndsWith,
    /// `empty`
    Empty,
    /// `!empty`
    NotEmpty,
}

/// A vertical run of one style in one column, rows `start_row..=end_row`.
///
/// A run with `start_row == 0` and `end_row == MAX_ROWS - 1` is a whole
/// column style (ODS: the column's `table:default-cell-style-name`).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StyleRun {
    /// 0-based column.
    pub col: u32,
    /// First row (0-based).
    pub start_row: u32,
    /// Last row (inclusive).
    pub end_row: u32,
    /// Index into [`WorkbookData::styles`].
    pub style: u32,
}

/// One cell.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CellData {
    /// 0-based row.
    pub row: u32,
    /// 0-based column.
    pub col: u32,
    /// Value or formula.
    pub content: CellContentData,
    /// Index into [`WorkbookData::styles`], or `None` for the default style.
    pub style: Option<u32>,
}

/// The content of a cell.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum CellContentData {
    /// A constant value ([`Value::Empty`] for a styled but empty cell).
    Value(Value),
    /// A formula.
    Formula {
        /// Formula text in the OpenFormula dialect, e.g. `of:=SUM([.A1:.A3])`.
        ///
        /// Shared (`Arc<str>`) so that a repeated formula cell read from a
        /// file costs one allocation, not one per materialized cell.
        text: Arc<str>,
        /// The cached result stored in the file, if any.
        cached: Option<Value>,
    },
    /// A member of a shared formula group (XLSX `t="shared"`): the formula
    /// of the group's anchor cell, moved to this cell (relative references
    /// shift with it, as when the anchor is filled or copied here).
    ///
    /// Every member shares the anchor's `text` allocation, so a group of a
    /// million cells costs one formula text, and the engine parses it once
    /// (security review 2, MEDIUM-1). Only readers produce this variant;
    /// the engine resolves it on load and never exports it.
    SharedFormula {
        /// The anchor's formula text (OpenFormula, as for
        /// [`CellContentData::Formula`]), valid at (`anchor_row`, `anchor_col`).
        text: Arc<str>,
        /// The anchor cell's row (0-based, same sheet).
        anchor_row: u32,
        /// The anchor cell's column (0-based).
        anchor_col: u32,
        /// The cached result stored in the file, if any.
        cached: Option<Value>,
    },
}

impl Default for CellContentData {
    fn default() -> Self {
        CellContentData::Value(Value::Empty)
    }
}

/// A cell style.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct StyleData {
    /// Font attributes.
    pub font: FontData,
    /// Background fill (`0xRRGGBBAA`), or `None` for no fill.
    pub fill: Option<u32>,
    /// Alignment.
    pub align: AlignData,
    /// Cell borders.
    pub borders: BordersData,
    /// OOo number format code, or `None` for General.
    pub number_format: Option<String>,
    /// Special display treatment (chips).
    pub display: Option<DisplayData>,
    /// Cell protection; `None` is the default (locked, formula visible).
    #[serde(default)]
    pub protection: Option<CellProtectionData>,
    /// The named cell style this style is based on (`None` = `Default`);
    /// see [`WorkbookData::cell_styles`] (v0.4).
    #[serde(default)]
    pub style_name: Option<String>,
}

/// Font attributes.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct FontData {
    /// Bold.
    pub bold: bool,
    /// Italic.
    pub italic: bool,
    /// Single underline.
    pub underline: bool,
    /// Strikethrough.
    pub strike: bool,
    /// Font family, or `None` for the default.
    pub family: Option<String>,
    /// Size in points, or `None` for the default.
    pub size_pt: Option<f32>,
    /// Text colour (`0xRRGGBBAA`), or `None` for automatic.
    pub color: Option<u32>,
    /// Underline style, used when `underline` is set (v0.4).
    #[serde(default)]
    pub underline_style: UnderlineStyleData,
}

/// Horizontal alignment.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum HAlign {
    /// Numbers right, text left, booleans and errors centred.
    #[default]
    General,
    /// Left.
    Left,
    /// Centre.
    Center,
    /// Right.
    Right,
    /// Justified.
    Justify,
}

/// Vertical alignment.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum VAlign {
    /// Bottom (the spreadsheet default).
    #[default]
    Bottom,
    /// Middle.
    Middle,
    /// Top.
    Top,
}

/// Alignment attributes.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct AlignData {
    /// Horizontal alignment.
    pub h: HAlign,
    /// Vertical alignment.
    pub v: VAlign,
    /// Wrap text.
    pub wrap: bool,
    /// Indent level.
    pub indent: u8,
    /// Text rotation in degrees counter-clockwise, `-90..=90` (ODS
    /// `style:rotation-angle`, XLSX `textRotation`); `0` = none (v0.4).
    #[serde(default)]
    pub rotation_deg: i16,
    /// Vertically stacked characters (v0.4).
    #[serde(default)]
    pub stacked: bool,
    /// Shrink the text to fit the cell (v0.4).
    #[serde(default)]
    pub shrink_to_fit: bool,
}

/// Line style of a border.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum BorderStyle {
    /// Solid line.
    #[default]
    Solid,
    /// Dashed line.
    Dashed,
    /// Dotted line.
    Dotted,
    /// Double line.
    Double,
}

/// One border line.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct BorderLine {
    /// Line style.
    pub style: BorderStyle,
    /// Width in 1/100 mm.
    pub width_hmm: u32,
    /// Colour (`0xRRGGBBAA`).
    pub color: u32,
}

/// The four borders of a cell; `None` means no border.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct BordersData {
    /// Top border.
    pub top: Option<BorderLine>,
    /// Bottom border.
    pub bottom: Option<BorderLine>,
    /// Left border.
    pub left: Option<BorderLine>,
    /// Right border.
    pub right: Option<BorderLine>,
    /// Diagonal from the top-left to the bottom-right corner (ODS
    /// `style:diagonal-tl-br`, XLSX `diagonalDown`; v0.4).
    #[serde(default)]
    pub diagonal_down: Option<BorderLine>,
    /// Diagonal from the bottom-left to the top-right corner (ODS
    /// `style:diagonal-bl-tr`, XLSX `diagonalUp`; v0.4).
    #[serde(default)]
    pub diagonal_up: Option<BorderLine>,
}

/// Special display styles.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum DisplayData {
    /// Render the value as a rounded "pill" with these colours (`0xRRGGBBAA`).
    /// Persisted to ODS as an extension attribute; other apps see a fill.
    Chip {
        /// Background colour.
        bg: u32,
        /// Text colour.
        fg: u32,
    },
}

/// A named expression.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct NameData {
    /// The name.
    pub name: String,
    /// Index of the sheet the name is local to, or `None` for workbook scope.
    pub sheet: Option<u32>,
    /// Expression in the OpenFormula dialect, e.g. `of:=[$Sheet1.$A$1:.$A$10]`.
    ///
    /// Relative references are relative to cell A1 of the first sheet (the
    /// model has no base cell; ODS `table:base-cell-address` is ignored on
    /// read and written as `$<first sheet>.$A$1`).
    pub formula: String,
}

/// Document-level settings.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct DocSettings {
    /// Index of the active sheet in [`WorkbookData::sheets`].
    pub active_sheet: u32,
    /// Calculation settings.
    pub calc_settings: CalcSettings,
    /// UI locale tag (`en-US`, `de-DE`, …) used for number formatting
    /// and input recognition; `None` for the application default.
    #[serde(default)]
    pub locale: Option<String>,
    /// Document structure protection (sheets cannot be added, removed,
    /// renamed, moved or hidden); `None` when not protected.
    #[serde(default)]
    pub structure_protection: Option<SheetProtectionData>,
    /// Custom sort lists (Tools ▸ Options ▸ Sort Lists), each a list of
    /// entries in order (v0.5).
    #[serde(default)]
    pub custom_sort_lists: Vec<Vec<String>>,
}

/// Document metadata. Timestamps are ISO 8601 strings as stored in the file.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct DocMeta {
    /// Document title.
    pub title: Option<String>,
    /// Application that wrote the file.
    pub generator: Option<String>,
    /// Creation time.
    pub created: Option<String>,
    /// Last modification time.
    pub modified: Option<String>,
    /// Subject.
    #[serde(default)]
    pub subject: Option<String>,
    /// Keywords.
    #[serde(default)]
    pub keywords: Vec<String>,
    /// Description (comments).
    #[serde(default)]
    pub description: Option<String>,
    /// Initial author.
    #[serde(default)]
    pub author: Option<String>,
    /// Author of the last modification.
    #[serde(default)]
    pub last_author: Option<String>,
}

/// A run of rows or columns `start..=end` sharing one size.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DimRun {
    /// First index (inclusive).
    pub start: u32,
    /// Last index (inclusive).
    pub end: u32,
    /// Size in 1/100 mm.
    pub size_hmm: u32,
    /// Hidden (also used for filtered rows).
    pub hidden: bool,
}

/// An inclusive range on an implied sheet (the sheet that contains it).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct CellRangeLocal {
    /// First row.
    pub r0: u32,
    /// First column.
    pub c0: u32,
    /// Last row.
    pub r1: u32,
    /// Last column.
    pub c1: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tally_types::{ErrorKind, IterSettings, Matrix};

    fn sample() -> WorkbookData {
        let bold = StyleData {
            font: FontData {
                bold: true,
                family: Some("Inter".into()),
                size_pt: Some(11.0),
                color: Some(0x1122_33ff),
                ..FontData::default()
            },
            fill: Some(0xffee_ddff),
            align: AlignData {
                h: HAlign::Right,
                v: VAlign::Middle,
                wrap: true,
                indent: 1,
                rotation_deg: 45,
                stacked: false,
                shrink_to_fit: true,
            },
            borders: BordersData {
                bottom: Some(BorderLine {
                    style: BorderStyle::Double,
                    width_hmm: 26,
                    color: 0x0000_00ff,
                }),
                diagonal_down: Some(BorderLine {
                    style: BorderStyle::Dashed,
                    width_hmm: 26,
                    color: 0xff00_00ff,
                }),
                ..BordersData::default()
            },
            number_format: Some("#,##0.00".into()),
            display: Some(DisplayData::Chip {
                bg: 0xd1fa_e5ff,
                fg: 0x0657_4fff,
            }),
            protection: Some(CellProtectionData {
                locked: false,
                ..CellProtectionData::default()
            }),
            style_name: Some("Heading".into()),
        };
        let sheet1 =
            SheetData {
                name: "Orders".into(),
                cells: vec![
                    CellData {
                        row: 0,
                        col: 0,
                        content: CellContentData::Value(Value::Number(2.0)),
                        style: Some(0),
                    },
                    CellData {
                        row: 0,
                        col: 1,
                        content: CellContentData::Value(Value::text("Paid")),
                        style: None,
                    },
                    CellData {
                        row: 1,
                        col: 0,
                        content: CellContentData::Formula {
                            text: "of:=SUM([.A1:.A1])".into(),
                            cached: Some(Value::Number(2.0)),
                        },
                        style: None,
                    },
                    CellData {
                        row: 1,
                        col: 1,
                        content: CellContentData::Formula {
                            text: "of:=1/0".into(),
                            cached: Some(Value::Error(ErrorKind::Div0)),
                        },
                        style: None,
                    },
                    CellData {
                        row: 2,
                        col: 0,
                        content: CellContentData::Value(Value::Array(Arc::new(
                            Matrix::new_filled(1, 2, Value::Bool(true)),
                        ))),
                        style: None,
                    },
                ],
                col_widths: vec![DimRun {
                    start: 0,
                    end: 2,
                    size_hmm: 2258,
                    hidden: false,
                }],
                row_heights: vec![DimRun {
                    start: 5,
                    end: 5,
                    size_hmm: 452,
                    hidden: true,
                }],
                merges: vec![CellRangeLocal {
                    r0: 3,
                    c0: 0,
                    r1: 3,
                    c1: 2,
                }],
                frozen: Some((1, 0)),
                tab_color: Some(0x3366_ccff),
                hidden: false,
                style_runs: vec![StyleRun {
                    col: 4,
                    start_row: 10,
                    end_row: 20_000,
                    style: 0,
                }],
                array_formulas: vec![CellRangeLocal {
                    r0: 6,
                    c0: 0,
                    r1: 7,
                    c1: 1,
                }],
                auto_filter: Some(AutoFilterData {
                    range: CellRangeLocal {
                        r0: 0,
                        c0: 0,
                        r1: 9,
                        c1: 2,
                    },
                    columns: vec![FilterColumnData {
                        col: 1,
                        values: Some(vec!["Paid".into(), String::new()]),
                        condition: Some(FilterConditionData {
                            op: FilterOpData::BeginsWith,
                            value: "P".into(),
                        }),
                    }],
                    filtered_rows: vec![(3, 4)],
                }),
                conditional_formats: vec![ConditionalFormatData {
                    ranges: vec![CellRangeLocal {
                        r0: 0,
                        c0: 0,
                        r1: 9,
                        c1: 0,
                    }],
                    base: (0, 0),
                    rules: vec![
                        CfRuleData {
                            kind: CfKindData::CellValue {
                                op: CmpOpData::Between,
                                formulas: vec!["of:=1".into(), "of:=[.B1]".into()],
                            },
                            style: CfStyleData {
                                bold: Some(true),
                                fill: Some(0xff00_00ff),
                                ..CfStyleData::default()
                            },
                            stop_if_true: false,
                        },
                        CfRuleData {
                            kind: CfKindData::IconSet {
                                set: IconSetData::ThreeArrows,
                                thresholds: vec![
                                    CfThresholdData {
                                        kind: CfValueKindData::Percent,
                                        value: "0".into(),
                                    },
                                    CfThresholdData {
                                        kind: CfValueKindData::Percent,
                                        value: "33".into(),
                                    },
                                ],
                                show_value: true,
                                reverse: false,
                            },
                            style: CfStyleData::default(),
                            stop_if_true: false,
                        },
                    ],
                }],
                validations: vec![ValidationData {
                    ranges: vec![CellRangeLocal::default()],
                    base: (0, 0),
                    kind: ValidationKindData::List {
                        source: ListSourceData::Values(vec!["Paid".into(), "Due".into()]),
                    },
                    allow_blank: true,
                    show_dropdown: true,
                    input_help: Some(ValidationMessageData {
                        title: "Status".into(),
                        text: "Pick one".into(),
                    }),
                    error: Some(ValidationErrorData::default()),
                }],
                notes: vec![NoteData {
                    row: 1,
                    col: 1,
                    text: "Check".into(),
                    author: Some("Ana".into()),
                    created: None,
                    shown: false,
                }],
                links: vec![LinkData {
                    row: 0,
                    col: 1,
                    url: "https://example.com".into(),
                }],
                protection: Some(SheetProtectionData::default()),
                page_setup: Some(PageSetupData {
                    orientation: OrientationData::Landscape,
                    scale: PageScaleData::FitPages(3),
                    print_formulas: true,
                    print_notes: NotesPrintData::AtEnd,
                    ..PageSetupData::default()
                }),
                row_breaks: vec![20],
                col_breaks: vec![],
                charts: vec![ChartData {
                    name: "Object 1".into(),
                    kind: ChartKindData::Line,
                    stacking: StackingData::Percent,
                    title: Some("Sales".into()),
                    categories: Some(ChartRangeData {
                        areas: vec![(
                            0,
                            CellRangeLocal {
                                r0: 1,
                                c0: 0,
                                r1: 5,
                                c1: 0,
                            },
                        )],
                        invalid: false,
                    }),
                    series: vec![ChartSeriesData {
                        name: Some(SeriesNameData::Text("Q1".into())),
                        values: ChartRangeData {
                            areas: vec![(
                                0,
                                CellRangeLocal {
                                    r0: 1,
                                    c0: 1,
                                    r1: 5,
                                    c1: 1,
                                },
                            )],
                            invalid: false,
                        },
                        trend: Some(TrendLineData {
                            kind: TrendKindData::Exponential,
                            show_equation: true,
                            show_r2: true,
                            name: None,
                        }),
                        error_bars: Some(ErrorBarsData {
                            kind: ErrorBarKindData::StdDev { k: 1.5 },
                            direction: ErrorBarDirData::Plus,
                        }),
                        ..ChartSeriesData::default()
                    }],
                    y2_axis: Some(ChartAxisData {
                        log: true,
                        min: Some(1.0),
                        ..ChartAxisData::default()
                    }),
                    ..ChartData::default()
                }],
                images: vec![ImageData {
                    name: "Image 1".into(),
                    anchor: ChartAnchorData {
                        row: 2,
                        col: 3,
                        width_hmm: 1000,
                        height_hmm: 500,
                        ..ChartAnchorData::default()
                    },
                    mime: "image/png".into(),
                    bytes: vec![0x89, 0x50, 0x4e, 0x47],
                }],
                pivots: vec![PivotData {
                    name: "DataPilot1".into(),
                    source: PivotSourceData::Range {
                        sheet: 0,
                        range: CellRangeLocal {
                            r0: 0,
                            c0: 0,
                            r1: 24,
                            c1: 5,
                        },
                    },
                    target: (30, 0),
                    row_fields: vec![PivotFieldData {
                        source_col: 0,
                        name: "Region".into(),
                        sort: PivotSortData::Data {
                            data_field: 0,
                            ascending: false,
                        },
                        group: Some(PivotGroupData::Date {
                            start: None,
                            end: None,
                            parts: vec![DatePartData::Months, DatePartData::Days(7)],
                        }),
                        ..PivotFieldData::default()
                    }],
                    data_fields: vec![PivotDataFieldData {
                        source_col: 5,
                        function: AggFnData::StDevP,
                        caption: None,
                        show_as: Some(ShowAsData {
                            kind: ShowAsKindData::PercentOfColumn,
                            base_field: None,
                            base_item: Some(BaseItemData::Previous),
                        }),
                    }],
                    ..PivotData::default()
                }],
                row_groups: vec![OutlineGroupData {
                    start: 1,
                    end: 4,
                    level: 1,
                    collapsed: true,
                }],
                col_groups: vec![],
                scenarios: vec![ScenarioData {
                    name: "Best case".into(),
                    ranges: vec![CellRangeLocal {
                        r0: 0,
                        c0: 0,
                        r1: 0,
                        c1: 0,
                    }],
                    cells: vec![CellData {
                        row: 0,
                        col: 0,
                        content: CellContentData::Value(Value::Number(9.0)),
                        style: None,
                    }],
                    border_color: Some(0x00ff_00ff),
                    active: true,
                    ..ScenarioData::default()
                }],
                database_ranges: vec![DatabaseRangeData {
                    name: "Orders".into(),
                    range: CellRangeLocal {
                        r0: 0,
                        c0: 0,
                        r1: 9,
                        c1: 2,
                    },
                    has_header: true,
                    filter_buttons: false,
                    filter: None,
                    subtotals: Some(SubtotalsData {
                        range: CellRangeLocal {
                            r0: 0,
                            c0: 0,
                            r1: 9,
                            c1: 2,
                        },
                        groups: vec![SubtotalGroupData {
                            group_by: 0,
                            columns: vec![2],
                            function: AggFnData::Sum,
                        }],
                        ..SubtotalsData::default()
                    }),
                }],
                standard_filter: Some(StandardFilterData {
                    range: CellRangeLocal {
                        r0: 0,
                        c0: 0,
                        r1: 9,
                        c1: 2,
                    },
                    conditions: vec![StdConditionData {
                        connector: StdConnectorData::Or,
                        col: 1,
                        op: StdFilterOpData::LargestPct,
                        value: "10".into(),
                    }],
                    options: StdFilterOptionsData {
                        regex: true,
                        copy_to: Some((1, 0, 0)),
                        ..StdFilterOptionsData::default()
                    },
                    criteria: None,
                }),
                subtotals: None,
                view_options: Some(ViewOptionsData {
                    show_formulas: true,
                    ..ViewOptionsData::default()
                }),
                split: Some(SplitData {
                    x_hmm: 5000,
                    y_hmm: 0,
                }),
            };
        WorkbookData {
            sheets: vec![
                sheet1,
                SheetData {
                    name: "Empty".into(),
                    hidden: true,
                    ..SheetData::default()
                },
            ],
            styles: vec![bold],
            names: vec![NameData {
                name: "Total".into(),
                sheet: None,
                formula: "of:=[$Orders.$A$1:.$A$2]".into(),
            }],
            settings: DocSettings {
                active_sheet: 1,
                calc_settings: CalcSettings {
                    iterations: Some(IterSettings::default()),
                    ..CalcSettings::default()
                },
                locale: Some("de-DE".into()),
                structure_protection: None,
                custom_sort_lists: vec![vec!["Q1".into(), "Q2".into()]],
            },
            meta: DocMeta {
                title: Some("Sample".into()),
                generator: Some("Tally/0.1.0".into()),
                created: Some("2026-10-19T08:00:00".into()),
                modified: None,
                subject: Some("Orders".into()),
                keywords: vec!["a".into(), "b".into()],
                description: None,
                author: Some("Ana".into()),
                last_author: None,
            },
            warnings: Vec::new(),
            cell_styles: vec![CellStyleData {
                name: "Heading".into(),
                parent: Some("Default".into()),
                style: StyleData {
                    font: FontData {
                        bold: true,
                        underline: true,
                        underline_style: UnderlineStyleData::Double,
                        ..FontData::default()
                    },
                    ..StyleData::default()
                },
            }],
        }
    }

    #[test]
    fn workbook_json_round_trip() {
        let wb = sample();
        let json = serde_json::to_string_pretty(&wb).unwrap();
        let back: WorkbookData = serde_json::from_str(&json).unwrap();
        assert_eq!(back, wb);
    }

    #[test]
    fn icon_sets_and_paper() {
        for s in IconSetData::ALL {
            assert_eq!(IconSetData::from_file_name(s.file_name()), Some(s));
        }
        assert_eq!(IconSetData::FiveQuarters.icon_count(), 5);
        assert_eq!(IconSetData::ThreeFlags.icon_count(), 3);
        assert_eq!(PaperData::from_size_hmm(29700, 21000), PaperData::A4);
        assert_eq!(PaperData::from_size_hmm(21590, 27940), PaperData::Letter);
        assert!(matches!(
            PaperData::from_size_hmm(1000, 2000),
            PaperData::Custom { .. }
        ));
    }

    #[test]
    fn old_json_still_loads() {
        // A v0.2 sheet without any of the v0.3 fields.
        let s: SheetData = serde_json::from_str(
            r#"{"name":"A","cells":[],"col_widths":[],"row_heights":[],"merges":[],
                "frozen":null,"tab_color":null,"hidden":false}"#,
        )
        .unwrap();
        assert!(s.notes.is_empty() && s.protection.is_none());
        // A v0.3 sheet and style without any of the v0.4 fields.
        assert!(s.charts.is_empty() && s.pivots.is_empty() && s.split.is_none());
        let st: StyleData = serde_json::from_str(
            r#"{"font":{"bold":false,"italic":false,"underline":false,"strike":false,
                "family":null,"size_pt":null,"color":null},"fill":null,
                "align":{"h":"General","v":"Bottom","wrap":false,"indent":0},
                "borders":{"top":null,"bottom":null,"left":null,"right":null},
                "number_format":null,"display":null}"#,
        )
        .unwrap();
        assert_eq!(st, StyleData::default());
        let p: PageSetupData = serde_json::from_value(serde_json::json!({
            "orientation": "Portrait", "paper": "A4",
            "margins": MarginsData::default(), "scale": {"Percent": 100},
            "print_range": null, "repeat_rows": null, "repeat_cols": null,
            "header": null, "footer": null, "print_gridlines": false,
            "print_headers": false, "center_h": false, "center_v": false,
            "page_order": "TopToBottom", "first_page_number": null
        }))
        .unwrap();
        assert!(p.print_charts && p.print_zero_values && !p.print_formulas);
        assert_eq!(p.print_notes, NotesPrintData::None);
    }

    #[test]
    fn v04_fields_round_trip_and_default_off() {
        let wb = sample();
        let s = &wb.sheets[0];
        assert_eq!(s.charts.len(), 1);
        let json = serde_json::to_string(&wb).unwrap();
        let back: WorkbookData = serde_json::from_str(&json).unwrap();
        assert_eq!(back.sheets[0].charts, s.charts);
        assert_eq!(back.sheets[0].pivots, s.pivots);
        assert_eq!(back.sheets[0].scenarios, s.scenarios);
        assert_eq!(back.cell_styles, wb.cell_styles);
        assert_eq!(
            back.settings.custom_sort_lists,
            wb.settings.custom_sort_lists
        );
        assert_eq!(back.styles[0].align.rotation_deg, 45);
        assert!(back.styles[0].borders.diagonal_down.is_some());
        // Chart defaults match AOO: 16 × 9 cm, legend right, Y major grid.
        let c = ChartData::default();
        assert_eq!((c.anchor.width_hmm, c.anchor.height_hmm), (16_000, 9_000));
        assert_eq!(c.legend, LegendPosData::Right);
        assert!(c.y_axis.major_grid && !c.x_axis.major_grid);
        assert!(ChartRangeData::default().is_unset());
        // Older SheetData JSON without any v0.4 field still loads.
        let sd: SheetData = serde_json::from_str(r#"{"name":"A","cells":[],"col_widths":[],"row_heights":[],"merges":[],"frozen":null,"tab_color":null,"hidden":false}"#).unwrap();
        assert_eq!(sd.view_options, None);
    }

    #[test]
    fn defaults() {
        let wb = WorkbookData::default();
        assert!(wb.sheets.is_empty());
        assert_eq!(wb.settings.active_sheet, 0);
        assert_eq!(
            CellContentData::default(),
            CellContentData::Value(Value::Empty)
        );
        let s = StyleData::default();
        assert_eq!(s.align.h, HAlign::General);
        assert_eq!(s.align.v, VAlign::Bottom);
        assert!(s.number_format.is_none());
    }
}
