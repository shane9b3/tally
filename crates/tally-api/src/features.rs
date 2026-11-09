//! v0.3 DTOs: conditional formatting, data validation, notes, protection,
//! page setup, document properties, goal seek and locales.
//!
//! Same serde conventions as [`crate::dto`]. Formulas in these DTOs are in
//! the UI dialect (CalcA1, `,` separators) without the leading `=` (a
//! leading `=` is accepted on input). Relative references are relative to
//! the top-left cell of the first range (Calc and Excel do the same): a
//! rule `B2>5` on `B2:B9` tests every cell of the range against 5.

use serde::{Deserialize, Serialize};
use tally_types::SheetId;
use ts_rs::TS;

use crate::dto::{BordersDto, CellPos, DisplayDto, RangeDto};

// ---------------------------------------------------------------------------
// Conditional formatting
// ---------------------------------------------------------------------------

/// A conditional format of a sheet.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ConditionalFormatDto {
    /// Id (stable while the document is open; ignored on add).
    pub id: u32,
    /// Target ranges (the first range's top-left is the formula base).
    pub ranges: Vec<RangeDto>,
    /// Rules in priority order.
    pub rules: Vec<CfRuleDto>,
}

/// One rule.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CfRuleDto {
    /// What it tests.
    pub kind: CfRuleKind,
    /// The style applied to matching cells (not used by colour scales,
    /// data bars and icon sets).
    pub style: CfStyleDto,
    /// Stop evaluating lower-priority rules when this one matches.
    #[serde(default)]
    pub stop_if_true: bool,
}

/// A differential style; only set fields apply.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CfStyleDto {
    /// Bold.
    pub bold: Option<bool>,
    /// Italic.
    pub italic: Option<bool>,
    /// Underline.
    pub underline: Option<bool>,
    /// Strikethrough.
    pub strike: Option<bool>,
    /// Text colour.
    pub color: Option<u32>,
    /// Background fill.
    pub fill: Option<u32>,
    /// Borders (edges left `None` are unchanged).
    pub borders: Option<BordersDto>,
    /// Number format code.
    pub number_format: Option<String>,
    /// Chip display.
    pub display: Option<DisplayDto>,
}

/// Comparison of cell-value rules and validations.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum CmpOp {
    /// Between two values (inclusive).
    Between,
    /// Not between.
    NotBetween,
    /// Equal.
    Eq,
    /// Not equal.
    Ne,
    /// Greater.
    Gt,
    /// Greater or equal.
    Ge,
    /// Less.
    Lt,
    /// Less or equal.
    Le,
}

/// Text tests.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum CfTextOp {
    /// Contains.
    Contains,
    /// Does not contain.
    NotContains,
    /// Begins with.
    BeginsWith,
    /// Ends with.
    EndsWith,
}

/// Date periods relative to today.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum CfDatePeriod {
    /// Today.
    Today,
    /// Yesterday.
    Yesterday,
    /// Tomorrow.
    Tomorrow,
    /// The last 7 days (today included).
    Last7Days,
    /// This week (Sunday–Saturday).
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

/// How a threshold is computed.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum CfValueKind {
    /// Lowest value.
    Min,
    /// Highest value.
    Max,
    /// `value` percent between min and max.
    Percent,
    /// The `value`th percentile.
    Percentile,
    /// The number `value`.
    Number,
    /// The formula `value`.
    Formula,
    /// Data bars: min(min, 0).
    AutoMin,
    /// Data bars: max(max, 0).
    AutoMax,
}

/// A threshold.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CfThreshold {
    /// Kind.
    pub kind: CfValueKind,
    /// A number, a formula (for `formula`) or `""`.
    pub value: String,
}

/// A colour scale stop.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CfColorStop {
    /// Where.
    pub threshold: CfThreshold,
    /// Colour.
    pub color: u32,
}

/// Icon sets.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum IconSetKind {
    /// ↓ → ↑ coloured.
    ThreeArrows,
    /// ↓ → ↑ grey.
    ThreeArrowsGray,
    /// Flags.
    ThreeFlags,
    /// Traffic lights.
    ThreeTrafficLights,
    /// Traffic lights with rims.
    ThreeTrafficLightsRimmed,
    /// Signs.
    ThreeSigns,
    /// Circled symbols.
    ThreeSymbols,
    /// Symbols.
    ThreeSymbolsUncircled,
    /// Stars.
    ThreeStars,
    /// Triangles.
    ThreeTriangles,
    /// 4 arrows.
    FourArrows,
    /// 4 grey arrows.
    FourArrowsGray,
    /// Red to black.
    FourRedToBlack,
    /// 4 ratings.
    FourRating,
    /// 4 traffic lights.
    FourTrafficLights,
    /// 5 arrows.
    FiveArrows,
    /// 5 grey arrows.
    FiveArrowsGray,
    /// 5 ratings.
    FiveRating,
    /// Quarters.
    FiveQuarters,
    /// Boxes.
    FiveBoxes,
}

/// What a rule tests.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum CfRuleKind {
    /// Compare the cell value with 1 (2 for between) formulas.
    CellValue {
        /// Comparison.
        op: CmpOp,
        /// Operands, e.g. `5`, `B1`, `"Paid"`.
        formulas: Vec<String>,
    },
    /// A formula, true for matching cells.
    Formula {
        /// The formula.
        formula: String,
    },
    /// 2 or 3 stops.
    ColorScale {
        /// Stops, lowest first.
        stops: Vec<CfColorStop>,
    },
    /// A data bar.
    DataBar {
        /// Empty-bar value.
        min: CfThreshold,
        /// Full-bar value.
        max: CfThreshold,
        /// Bar colour.
        color: u32,
        /// Negative bar colour.
        negative_color: Option<u32>,
        /// Show the value next to the bar.
        show_value: bool,
        /// Gradient fill.
        gradient: bool,
    },
    /// An icon set.
    IconSet {
        /// Icons.
        set: IconSetKind,
        /// One threshold per icon (the first is ignored).
        thresholds: Vec<CfThreshold>,
        /// Show the value next to the icon.
        show_value: bool,
        /// Reverse the icon order.
        reverse: bool,
    },
    /// Values occurring more than once.
    Duplicate,
    /// Values occurring once.
    Unique,
    /// Top/bottom N (or N%).
    Top {
        /// N.
        rank: u32,
        /// Percent.
        percent: bool,
        /// Bottom instead of top.
        bottom: bool,
    },
    /// Above/below average.
    Average {
        /// Above.
        above: bool,
        /// Include equal.
        equal: bool,
    },
    /// Text test (case-insensitive).
    Text {
        /// Test.
        op: CfTextOp,
        /// Text.
        text: String,
    },
    /// Empty cells.
    Blank,
    /// Non-empty cells.
    NotBlank,
    /// Errors.
    Error,
    /// Non-errors.
    NotError,
    /// Dates in a period.
    DateIs {
        /// Period.
        period: CfDatePeriod,
    },
}

/// A data bar to draw in a cell ([`crate::CellView::cf_bar`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DataBarView {
    /// Bar length, `0..=1` of the cell width (from the axis when `axis`
    /// is set).
    pub fraction: f32,
    /// Colour.
    pub color: u32,
    /// The value is negative (drawn leftwards from `axis`).
    pub negative: bool,
    /// Axis position `0..=1` when the range has negative values.
    pub axis: Option<f32>,
    /// Gradient fill.
    pub gradient: bool,
}

/// An icon to draw in a cell ([`crate::CellView::cf_icon`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct IconView {
    /// The set.
    pub set: IconSetKind,
    /// Index in the set, 0 = lowest (already reversed when `reverse`).
    pub index: u32,
}

// ---------------------------------------------------------------------------
// Data validation
// ---------------------------------------------------------------------------

/// A data validation rule.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ValidationDto {
    /// Id (ignored on add).
    pub id: u32,
    /// Target ranges.
    pub ranges: Vec<RangeDto>,
    /// What is allowed.
    pub kind: ValidationKind,
    /// Empty cells are valid.
    pub allow_blank: bool,
    /// Show the dropdown for list rules.
    pub show_dropdown: bool,
    /// List rules: sort the dropdown entries ascending (AOO "Sort entries
    /// ascending"; numbers first, then text in code-point order).
    #[serde(default)]
    pub sort_ascending: bool,
    /// Input help.
    pub input_help: Option<ValidationMessage>,
    /// Error alert (`None`: invalid input is accepted silently).
    pub error: Option<ValidationAlert>,
}

/// Input help.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ValidationMessage {
    /// Title.
    pub title: String,
    /// Text.
    pub text: String,
}

/// Alert styles.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ValidationErrorStyle {
    /// Reject the input (`OpError { code: "validation" }`).
    Stop,
    /// Accept with a warning (`ApplyResult.warnings`).
    Warning,
    /// Accept with an information message (`ApplyResult.warnings`).
    Info,
}

/// An error alert.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ValidationAlert {
    /// Style.
    pub style: ValidationErrorStyle,
    /// Title.
    pub title: String,
    /// Text (the `OpError` message; a default text is used when empty).
    pub text: String,
}

/// List item source.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum ListSource {
    /// Literal items.
    Values {
        /// Items.
        items: Vec<String>,
    },
    /// A cell range (or formula), e.g. `$Lists.$A$1:$A$9`.
    Range {
        /// The reference.
        reference: String,
    },
}

/// What a validation allows.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum ValidationKind {
    /// Anything.
    Any,
    /// One of a list.
    List {
        /// Items.
        source: ListSource,
    },
    /// Whole numbers.
    Whole {
        /// Comparison.
        op: CmpOp,
        /// Operands.
        formulas: Vec<String>,
    },
    /// Decimals.
    Decimal {
        /// Comparison.
        op: CmpOp,
        /// Operands.
        formulas: Vec<String>,
    },
    /// Dates.
    Date {
        /// Comparison.
        op: CmpOp,
        /// Operands.
        formulas: Vec<String>,
    },
    /// Times.
    Time {
        /// Comparison.
        op: CmpOp,
        /// Operands.
        formulas: Vec<String>,
    },
    /// Text length.
    TextLength {
        /// Comparison.
        op: CmpOp,
        /// Operands.
        formulas: Vec<String>,
    },
    /// A formula that must be true.
    Custom {
        /// The formula.
        formula: String,
    },
}

/// The validation of one cell, as the editor needs it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ValidationInfo {
    /// The rule's id.
    pub id: u32,
    /// List items (resolved from the source range; at most 10,000), empty
    /// for non-list rules.
    pub list_items: Vec<String>,
    /// Show the dropdown.
    pub show_dropdown: bool,
    /// Input help.
    pub input_help: Option<ValidationMessage>,
    /// Error alert.
    pub error: Option<ValidationAlert>,
    /// The rule.
    pub kind: ValidationKind,
}

// ---------------------------------------------------------------------------
// Notes
// ---------------------------------------------------------------------------

/// A cell note (comment).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NoteDto {
    /// Row.
    pub r: u32,
    /// Column.
    pub c: u32,
    /// Text.
    pub text: String,
    /// Author.
    pub author: Option<String>,
    /// Creation time (ISO 8601); set by the engine when `None` on input.
    pub created: Option<String>,
    /// Always shown.
    pub shown: bool,
}

// ---------------------------------------------------------------------------
// Protection
// ---------------------------------------------------------------------------

/// What users may still do on a protected sheet.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProtectionOptions {
    /// Select locked cells.
    pub select_locked: bool,
    /// Select unlocked cells.
    pub select_unlocked: bool,
    /// Insert rows.
    pub insert_rows: bool,
    /// Insert columns.
    pub insert_cols: bool,
    /// Delete rows.
    pub delete_rows: bool,
    /// Delete columns.
    pub delete_cols: bool,
    /// Format cells.
    pub format_cells: bool,
    /// Sort.
    pub sort: bool,
    /// Use AutoFilter.
    pub auto_filter: bool,
}

impl Default for ProtectionOptions {
    fn default() -> Self {
        Self {
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

/// Protection state of a sheet and of the document structure.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProtectionInfo {
    /// The sheet is protected.
    pub sheet_protected: bool,
    /// Unprotecting the sheet needs a password.
    pub sheet_has_password: bool,
    /// Allowed actions while protected.
    pub options: ProtectionOptions,
    /// The document structure is protected.
    pub structure_protected: bool,
    /// Unprotecting the structure needs a password.
    pub structure_has_password: bool,
}

// ---------------------------------------------------------------------------
// Page setup
// ---------------------------------------------------------------------------

/// Page orientation.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Orientation {
    /// Portrait.
    Portrait,
    /// Landscape.
    Landscape,
}

/// Paper size.
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum Paper {
    /// A4.
    A4,
    /// A3.
    A3,
    /// A5.
    A5,
    /// US Letter.
    Letter,
    /// US Legal.
    Legal,
    /// Executive.
    Executive,
    /// Tabloid.
    Tabloid,
    /// B5 (ISO).
    B5,
    /// Custom size (portrait), millimetres.
    Custom {
        /// Width.
        width_mm: f32,
        /// Height.
        height_mm: f32,
    },
}

/// Print scaling.
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum PageScale {
    /// A zoom (10–400 %).
    Percent {
        /// Percent.
        percent: u32,
    },
    /// Fit into pages (`0` = unlimited).
    FitTo {
        /// Pages wide.
        width: u32,
        /// Pages tall.
        height: u32,
    },
    /// Fit the print range into this many pages in total (AOO "Fit print
    /// range(s) on number of pages"; v0.4).
    FitPages {
        /// Pages.
        pages: u32,
    },
}

/// Margins in millimetres.
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Margins {
    /// Top.
    pub top_mm: f32,
    /// Bottom.
    pub bottom_mm: f32,
    /// Left.
    pub left_mm: f32,
    /// Right.
    pub right_mm: f32,
    /// Header area height, its spacing included (the minimum with AutoFit
    /// height; AOO's dialog shows "Height" as this minus the spacing).
    pub header_mm: f32,
    /// Footer area height, its spacing included.
    pub footer_mm: f32,
}

/// Header or footer area layout (AOO Format ▸ Page ▸ Header / Footer).
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HeaderFooterArea {
    /// "Header on" / "Footer on". Off keeps the texts but prints nothing
    /// and gives the area's room to the body.
    pub on: bool,
    /// Spacing between the text and the body, part of the area's height.
    pub spacing_mm: f32,
    /// AutoFit height: the area grows to fit its lines (4.94 mm each) plus
    /// the spacing.
    pub autofit: bool,
    /// Left margin of the area inside the page margins.
    pub left_mm: f32,
    /// Right margin of the area inside the page margins.
    pub right_mm: f32,
}

/// A header or footer; placeholders `&[PAGE]`, `&[PAGES]`, `&[SHEET]`,
/// `&[FILE]`, `&[DATE]`, `&[TIME]`.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HeaderFooter {
    /// Left.
    pub left: String,
    /// Centre.
    pub center: String,
    /// Right.
    pub right: String,
}

/// Page order.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum PageOrder {
    /// Down, then over.
    TopToBottom,
    /// Over, then down.
    LeftToRight,
}

/// Page setup of a sheet.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PageSetupDto {
    /// Orientation.
    pub orientation: Orientation,
    /// Paper.
    pub paper: Paper,
    /// Margins.
    pub margins: Margins,
    /// Scaling.
    pub scale: PageScale,
    /// Print range (`None`: the used area).
    pub print_range: Option<RangeDto>,
    /// Rows to repeat `[first, last]`.
    pub repeat_rows: Option<[u32; 2]>,
    /// Columns to repeat `[first, last]`.
    pub repeat_cols: Option<[u32; 2]>,
    /// Header.
    pub header: Option<HeaderFooter>,
    /// Footer.
    pub footer: Option<HeaderFooter>,
    /// Print gridlines.
    pub print_gridlines: bool,
    /// Print row/column headers.
    pub print_headers: bool,
    /// Centre horizontally.
    pub center_h: bool,
    /// Centre vertically.
    pub center_v: bool,
    /// Page order.
    pub page_order: PageOrder,
    /// First page number.
    pub first_page_number: Option<u32>,
    /// Manual row breaks (read-only here; see `Op::SetPageBreak`).
    #[serde(default)]
    pub row_breaks: Vec<u32>,
    /// Manual column breaks (read-only here).
    #[serde(default)]
    pub col_breaks: Vec<u32>,
    /// Print charts and pictures (absent = `true`; v0.4).
    #[serde(default)]
    #[ts(optional = nullable)]
    pub print_charts: Option<bool>,
    /// How notes are printed (absent = not printed; v0.4).
    #[serde(default)]
    #[ts(optional = nullable)]
    pub print_notes: Option<crate::print::NotesPrint>,
    /// Print zero values (absent = `true`; v0.4).
    #[serde(default)]
    #[ts(optional = nullable)]
    pub print_zero_values: Option<bool>,
    /// Print formulas instead of results (absent = `false`; v0.4).
    #[serde(default)]
    #[ts(optional = nullable)]
    pub print_formulas: Option<bool>,
    /// Header area (absent = on when `header` is set, 2.5 mm spacing,
    /// AutoFit).
    #[serde(default)]
    #[ts(optional = nullable)]
    pub header_area: Option<HeaderFooterArea>,
    /// Footer area (absent: as for the header).
    #[serde(default)]
    #[ts(optional = nullable)]
    pub footer_area: Option<HeaderFooterArea>,
}

/// Axis of a manual page break.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum BreakAxis {
    /// A break above a row.
    Row,
    /// A break left of a column.
    Col,
}

// ---------------------------------------------------------------------------
// Document properties, goal seek, locales
// ---------------------------------------------------------------------------

/// Document properties (File ▸ Properties).
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DocPropertiesDto {
    /// Title.
    pub title: Option<String>,
    /// Subject.
    pub subject: Option<String>,
    /// Keywords.
    pub keywords: Vec<String>,
    /// Description.
    pub description: Option<String>,
    /// Author.
    pub author: Option<String>,
    /// Creation time (read-only).
    pub created: Option<String>,
    /// Last modification time (read-only).
    pub modified: Option<String>,
    /// The application that wrote the file (read-only).
    pub generator: Option<String>,
}

/// A goal seek request: change `variable_cell` until `formula_cell`
/// equals `target`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GoalSeekRequest {
    /// Sheet of the formula cell.
    pub formula_sheet: SheetId,
    /// The formula cell.
    pub formula_cell: CellPos,
    /// Target value.
    pub target: f64,
    /// Sheet of the variable cell.
    pub variable_sheet: SheetId,
    /// The variable cell (a constant, not a formula).
    pub variable_cell: CellPos,
}

/// Goal seek outcome. Nothing is changed; apply `value` with
/// `SetCellInput` if the user accepts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GoalSeekResult {
    /// A value meeting the target (within 1e-9 relative) was found.
    pub found: bool,
    /// The best value for the variable cell.
    pub value: f64,
    /// The formula's result for `value`.
    pub result: f64,
    /// Iterations used (≤ 100).
    pub iterations: u32,
}

/// A supported locale.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LocaleInfo {
    /// BCP 47 tag, e.g. `de-DE`.
    pub tag: String,
    /// Decimal separator.
    pub decimal_sep: String,
    /// Grouping separator.
    pub group_sep: String,
    /// Function argument separator of the UI dialect.
    pub list_sep: String,
    /// Currency symbol.
    pub currency_symbol: String,
}
