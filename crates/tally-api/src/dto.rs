//! Data transfer objects shared by the Rust engine and the TypeScript UI.
//!
//! Every type derives [`ts_rs::TS`] with `#[ts(export)]`; `cargo test`
//! writes one `.ts` file per type into `TS_RS_EXPORT_DIR` (defaulted to
//! `packages/bridge/src/generated` by `.cargo/config.toml`).
//!
//! Serde conventions (mirrored by ts-rs):
//! - struct fields are `camelCase`;
//! - enum variants are `camelCase` strings (`"ods"`, `"up"`, `"bold"` …);
//! - enums with data are internally tagged with `"type"`, so TypeScript
//!   gets discriminated unions (`{ type: "setCellInput", sheet, r, c, input }`);
//! - `Option` fields are `T | null` and are always present in JSON;
//! - `u64` fields are exported as `number` (values stay below 2^53);
//! - colours are `0xRRGGBBAA` packed into a `u32`; sizes are CSS pixels at
//!   zoom 100%.

use serde::{Deserialize, Serialize};
use tally_types::SheetId;
use ts_rs::TS;

use crate::features::{
    BreakAxis, CfRuleDto, DataBarView, DocPropertiesDto, IconView, NoteDto, PageSetupDto,
    ProtectionOptions, ValidationDto,
};

// ---------------------------------------------------------------------------
// Documents and sheets
// ---------------------------------------------------------------------------

/// Handle of an open document.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DocId(pub u32);

/// Summary of an open workbook.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WorkbookInfo {
    /// Document handle.
    pub doc: DocId,
    /// Window title (file name or "Untitled").
    pub title: String,
    /// File path, if the document has been saved or opened from disk.
    pub path: Option<String>,
    /// Sheets in tab order.
    pub sheets: Vec<SheetInfo>,
    /// The active sheet.
    pub active_sheet: SheetId,
    /// Unsaved changes exist.
    pub dirty: bool,
    /// Monotonic document version; bumps on every change.
    #[ts(type = "number")]
    pub version: u64,
    /// Things the user should know about the document, shown once as toasts
    /// (e.g. "Recalculation stopped after N formulas…" when opening a file
    /// hit the recalc watchdog). Empty in most results.
    #[serde(default)]
    pub warnings: Vec<String>,
}

/// One sheet tab.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SheetInfo {
    /// Stable sheet id (not the tab index).
    pub id: SheetId,
    /// Tab name.
    pub name: String,
    /// Tab colour as a CSS colour string, e.g. `#3366cc`.
    pub tab_color: Option<String>,
    /// Hidden sheets have no tab.
    pub hidden: bool,
}

/// Geometry of a sheet, for pixel ↔ index mapping in the grid.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SheetLayout {
    /// The sheet.
    pub sheet: SheetId,
    /// Bounding box of non-empty cells, or `None` for an empty sheet.
    pub used: Option<RangeDto>,
    /// Width of columns without an explicit size.
    pub default_col_width_px: f32,
    /// Height of rows without an explicit size.
    pub default_row_height_px: f32,
    /// Columns that differ from the default, sorted, non-overlapping.
    pub col_runs: Vec<DimRunPx>,
    /// Rows that differ from the default, sorted, non-overlapping.
    pub row_runs: Vec<DimRunPx>,
    /// Frozen rows/columns (top-left of the scrollable pane), if any.
    pub frozen: Option<CellPos>,
    /// Merged areas.
    pub merges: Vec<RangeDto>,
}

/// A run of rows or columns `start..=end` sharing one size.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DimRunPx {
    /// First index (inclusive).
    pub start: u32,
    /// Last index (inclusive).
    pub end: u32,
    /// Size in pixels.
    pub size: f32,
    /// Hidden rows/columns have size 0 on screen.
    pub hidden: bool,
    /// `true` when the rows are hidden by an AutoFilter (as opposed to
    /// hidden by the user). Absent or `null` means not filtered.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub filtered: Option<bool>,
}

/// An inclusive range on an implied sheet.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RangeDto {
    /// First row.
    pub r0: u32,
    /// First column.
    pub c0: u32,
    /// Last row.
    pub r1: u32,
    /// Last column.
    pub c1: u32,
}

/// A cell position on an implied sheet.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CellPos {
    /// Row.
    pub r: u32,
    /// Column.
    pub c: u32,
}

// ---------------------------------------------------------------------------
// Cells and styles
// ---------------------------------------------------------------------------

/// The cells of one requested range.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CellBlock {
    /// The sheet.
    pub sheet: SheetId,
    /// The requested range.
    pub range: RangeDto,
    /// Document version the block was read at.
    #[ts(type = "number")]
    pub version: u64,
    /// Only non-empty or styled cells; missing cells are empty and default-styled.
    pub cells: Vec<CellView>,
}

/// Flag bits for [`CellView::flags`].
pub mod cell_flags {
    /// The cell contains a formula.
    pub const FORMULA: u8 = 1;
    /// The value is an error.
    pub const ERROR: u8 = 2;
    /// The cell is part of an array result.
    pub const ARRAY: u8 = 4;
    /// The cell has a comment.
    pub const COMMENT: u8 = 8;
    /// The cell has a list validation with a dropdown.
    pub const VALIDATION_DROPDOWN: u8 = 16;
    /// The cell has a hyperlink (attribute or `HYPERLINK()` result).
    pub const LINK: u8 = 32;
}

/// What the grid needs to draw one cell.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CellView {
    /// Row.
    pub r: u32,
    /// Column.
    pub c: u32,
    /// Formatted display text.
    pub text: String,
    /// Kind of the underlying value.
    pub kind: ValueKind,
    /// The raw number, so General can shrink to fit the column width.
    pub num: Option<f64>,
    /// Style id (`0` is the default style); resolve with `styles()`.
    pub style: u32,
    /// Text colour from the number format (e.g. `[RED]`), overriding the style.
    pub color: Option<u32>,
    /// Bit set of [`cell_flags`].
    pub flags: u8,
    /// Style id to draw instead of `style` because a conditional format
    /// matches (a synthesized id; resolve it with `styles()` like any
    /// other). Absent or `null` when no conditional style applies.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub cf_style: Option<u32>,
    /// A conditional-format data bar to draw.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub cf_bar: Option<DataBarView>,
    /// A conditional-format icon to draw.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub cf_icon: Option<IconView>,
    /// The cell's hyperlink (the cell attribute, or the target of a
    /// `HYPERLINK()` formula).
    #[serde(default)]
    #[ts(optional = nullable)]
    pub link: Option<String>,
}

/// Kind of a cell value.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ValueKind {
    /// Empty.
    Empty,
    /// Number (including dates).
    Number,
    /// Text.
    Text,
    /// Boolean.
    Bool,
    /// Error.
    Error,
}

/// A resolved cell style.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StyleDto {
    /// Style id.
    pub id: u32,
    /// Bold.
    pub bold: bool,
    /// Italic.
    pub italic: bool,
    /// Underline.
    pub underline: bool,
    /// Strikethrough.
    pub strike: bool,
    /// Font family, or `None` for the theme font.
    pub font_family: Option<String>,
    /// Font size in points.
    pub font_size_pt: f32,
    /// Text colour, or `None` for the theme colour.
    pub color: Option<u32>,
    /// Background fill, or `None`.
    pub fill: Option<u32>,
    /// Horizontal alignment.
    pub h_align: HAlign,
    /// Vertical alignment.
    pub v_align: VAlign,
    /// Wrap text.
    pub wrap: bool,
    /// Indent level.
    pub indent: u8,
    /// Borders.
    pub borders: BordersDto,
    /// Number format code (`"General"` for the default).
    pub number_format: String,
    /// Special display treatment.
    pub display: Option<DisplayDto>,
}

/// Horizontal alignment.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum HAlign {
    /// Numbers right, text left, booleans and errors centred.
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
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum VAlign {
    /// Bottom.
    Bottom,
    /// Middle.
    Middle,
    /// Top.
    Top,
}

/// The four borders of a cell; `None` means no border.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BordersDto {
    /// Top.
    pub top: Option<BorderLineDto>,
    /// Bottom.
    pub bottom: Option<BorderLineDto>,
    /// Left.
    pub left: Option<BorderLineDto>,
    /// Right.
    pub right: Option<BorderLineDto>,
}

/// One border line.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BorderLineDto {
    /// Line style.
    pub style: BorderStyleDto,
    /// Width in pixels.
    pub width_px: f32,
    /// Colour.
    pub color: u32,
}

/// Line style of a border.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum BorderStyleDto {
    /// Solid.
    Solid,
    /// Dashed.
    Dashed,
    /// Dotted.
    Dotted,
    /// Double.
    Double,
}

/// Special display treatment of a cell.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum DisplayDto {
    /// A rounded pill ("Paid", "Overdue") with these colours.
    Chip {
        /// Background colour.
        bg: u32,
        /// Text colour.
        fg: u32,
    },
}

// ---------------------------------------------------------------------------
// Mutations
// ---------------------------------------------------------------------------

/// One mutation. Every op is undoable as part of its [`Transaction`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum Op {
    /// Raw user text; the engine recognises formula, number, date, bool or text.
    SetCellInput {
        /// Sheet.
        sheet: SheetId,
        /// Row.
        r: u32,
        /// Column.
        c: u32,
        /// What the user typed.
        input: String,
    },
    /// A block of raw inputs with its top-left at (`r`, `c`).
    SetRangeInputs {
        /// Sheet.
        sheet: SheetId,
        /// Top row.
        r: u32,
        /// Left column.
        c: u32,
        /// Rows of inputs.
        rows: Vec<Vec<String>>,
    },
    /// Clears contents, formats or both.
    Clear {
        /// Sheet.
        sheet: SheetId,
        /// Ranges to clear.
        ranges: Vec<RangeDto>,
        /// What to clear.
        what: ClearWhat,
    },
    /// Applies a style patch. Toggle semantics are resolved by the UI.
    ApplyStyle {
        /// Sheet.
        sheet: SheetId,
        /// Target ranges.
        ranges: Vec<RangeDto>,
        /// Fields to set; `None` leaves a field unchanged.
        patch: StylePatch,
    },
    /// Sets the number format code.
    SetNumberFormat {
        /// Sheet.
        sheet: SheetId,
        /// Target ranges.
        ranges: Vec<RangeDto>,
        /// OOo format code.
        code: String,
    },
    /// Inserts `count` rows before row `at`.
    InsertRows {
        /// Sheet.
        sheet: SheetId,
        /// Insert position.
        at: u32,
        /// Number of rows.
        count: u32,
    },
    /// Deletes `count` rows starting at row `at`.
    DeleteRows {
        /// Sheet.
        sheet: SheetId,
        /// First row.
        at: u32,
        /// Number of rows.
        count: u32,
    },
    /// Inserts `count` columns before column `at`.
    InsertCols {
        /// Sheet.
        sheet: SheetId,
        /// Insert position.
        at: u32,
        /// Number of columns.
        count: u32,
    },
    /// Deletes `count` columns starting at column `at`.
    DeleteCols {
        /// Sheet.
        sheet: SheetId,
        /// First column.
        at: u32,
        /// Number of columns.
        count: u32,
    },
    /// Sets the width of columns `start..=end`.
    SetColWidth {
        /// Sheet.
        sheet: SheetId,
        /// First column.
        start: u32,
        /// Last column.
        end: u32,
        /// Width in pixels.
        px: f32,
    },
    /// Sets the height of rows `start..=end`.
    SetRowHeight {
        /// Sheet.
        sheet: SheetId,
        /// First row.
        start: u32,
        /// Last row.
        end: u32,
        /// Height in pixels.
        px: f32,
    },
    /// Adds a sheet.
    AddSheet {
        /// Name, or `None` for the next free "SheetN".
        name: Option<String>,
        /// Tab index, or `None` to append.
        index: Option<u32>,
    },
    /// Renames a sheet.
    RenameSheet {
        /// Sheet.
        sheet: SheetId,
        /// New name.
        name: String,
    },
    /// Deletes a sheet (the last visible sheet cannot be deleted).
    DeleteSheet {
        /// Sheet.
        sheet: SheetId,
    },
    /// Moves a sheet to tab index `to`.
    MoveSheet {
        /// Sheet.
        sheet: SheetId,
        /// New tab index.
        to: u32,
    },
    /// Pastes clipboard content with its top-left at `target`.
    Paste {
        /// Sheet.
        sheet: SheetId,
        /// Target range (paste repeats to fill a larger multiple).
        target: RangeDto,
        /// What to paste.
        payload: PastePayload,
    },
    /// Defines (or redefines) a named range or expression.
    DefineName {
        /// The name: `[A-Za-z_][A-Za-z0-9_.]*`, not a cell reference;
        /// unique per scope, case-insensitively.
        name: String,
        /// Sheet scope, or `None` for the workbook.
        scope: Option<SheetId>,
        /// What it refers to, in CalcA1 syntax without `=`, e.g.
        /// `$Sheet1.$A$1:$B$5` or `0.07`.
        refers_to: String,
    },
    /// Deletes a name (formulas using it then show `#NAME?`).
    DeleteName {
        /// The name.
        name: String,
        /// Its scope.
        scope: Option<SheetId>,
    },
    /// Enters an array (Ctrl+Shift+Enter) formula over `range`.
    SetArrayFormula {
        /// Sheet.
        sheet: SheetId,
        /// The block; the formula's result matrix is spread over it.
        range: RangeDto,
        /// The formula, starting with `=` (CalcA1).
        input: String,
    },
    /// Cut and paste: moves `from` so its top-left lands at `to`, rewriting
    /// references that point into the moved block.
    MoveRange {
        /// Sheet.
        sheet: SheetId,
        /// Source block.
        from: RangeDto,
        /// Destination top-left.
        to: CellPos,
    },
    /// Merges `range`; only the top-left cell keeps its content (the UI
    /// should warn when other cells are non-empty).
    MergeCells {
        /// Sheet.
        sheet: SheetId,
        /// Area to merge.
        range: RangeDto,
    },
    /// Removes every merge intersecting `range`.
    UnmergeCells {
        /// Sheet.
        sheet: SheetId,
        /// Area.
        range: RangeDto,
    },
    /// Freezes the top `rows` rows and left `cols` columns (`0, 0` unfreezes).
    SetFrozen {
        /// Sheet.
        sheet: SheetId,
        /// Frozen rows.
        rows: u32,
        /// Frozen columns.
        cols: u32,
    },
    /// Sorts the rows of `range` (stable, multi-key).
    SortRange {
        /// Sheet.
        sheet: SheetId,
        /// The area; with `has_header` its first row stays in place.
        range: RangeDto,
        /// Keys in priority order.
        keys: Vec<SortKey>,
        /// The first row holds column labels.
        has_header: bool,
        /// Compare text case-sensitively (lowercase first).
        case_sensitive: bool,
    },
    /// Sets (or with `None` removes) the sheet's AutoFilter area.
    SetAutoFilter {
        /// Sheet.
        sheet: SheetId,
        /// The area including its header row.
        range: Option<RangeDto>,
    },
    /// Sets (or with `None` clears) the filter of one AutoFilter column.
    SetColumnFilter {
        /// Sheet.
        sheet: SheetId,
        /// Absolute column inside the AutoFilter area.
        col: u32,
        /// The filter.
        filter: Option<ColumnFilter>,
    },
    /// Hides or shows rows (user-hidden, independent of filters).
    SetRowsHidden {
        /// Sheet.
        sheet: SheetId,
        /// First row.
        start: u32,
        /// Last row.
        end: u32,
        /// Hide (`true`) or show.
        hidden: bool,
    },
    /// Hides or shows columns.
    SetColsHidden {
        /// Sheet.
        sheet: SheetId,
        /// First column.
        start: u32,
        /// Last column.
        end: u32,
        /// Hide (`true`) or show.
        hidden: bool,
    },
    /// Replaces every match of `query` (in `sheet`, or every sheet).
    ReplaceAll {
        /// Sheet to search, or `None` for all sheets (`query.sheet` is ignored).
        sheet: Option<SheetId>,
        /// What to find.
        query: FindQuery,
        /// Replacement (regex mode: `$1`, `&` for the whole match).
        replace: String,
    },
    /// Paste with options (values only, formats only, transpose, skip
    /// empty cells, arithmetic with the target).
    PasteSpecial {
        /// Sheet.
        sheet: SheetId,
        /// Target range (its top-left is the paste position).
        target: RangeDto,
        /// What to paste.
        payload: PastePayload,
        /// How.
        options: PasteOptions,
    },
    /// Fill handle / Fill Series: extends `source` over `target`.
    FillRange {
        /// Sheet.
        sheet: SheetId,
        /// The cells that define the series.
        source: RangeDto,
        /// The area to fill, including `source`; it extends `source` in
        /// exactly one direction (down, up, right or left).
        target: RangeDto,
        /// Series type.
        mode: FillMode,
    },
    // ----- v0.3 ----------------------------------------------------------
    /// Adds a conditional format (its id is assigned by the engine).
    AddConditionalFormat {
        /// Sheet.
        sheet: SheetId,
        /// Target ranges.
        ranges: Vec<RangeDto>,
        /// Rules in priority order.
        rules: Vec<CfRuleDto>,
    },
    /// Replaces a conditional format's ranges and rules.
    UpdateConditionalFormat {
        /// Sheet.
        sheet: SheetId,
        /// Id from `conditional_formats`.
        id: u32,
        /// Target ranges.
        ranges: Vec<RangeDto>,
        /// Rules.
        rules: Vec<CfRuleDto>,
    },
    /// Deletes a conditional format.
    DeleteConditionalFormat {
        /// Sheet.
        sheet: SheetId,
        /// Id.
        id: u32,
    },
    /// Adds a data validation (`validation.id` is ignored).
    AddValidation {
        /// Sheet.
        sheet: SheetId,
        /// The rule and its ranges.
        validation: ValidationDto,
    },
    /// Replaces a data validation.
    UpdateValidation {
        /// Sheet.
        sheet: SheetId,
        /// Id from `validations`.
        id: u32,
        /// The new rule and ranges.
        validation: ValidationDto,
    },
    /// Deletes a data validation.
    DeleteValidation {
        /// Sheet.
        sheet: SheetId,
        /// Id.
        id: u32,
    },
    /// Sets (or replaces) the note of a cell (`note.r`/`note.c` are ignored).
    SetNote {
        /// Sheet.
        sheet: SheetId,
        /// Row.
        r: u32,
        /// Column.
        c: u32,
        /// The note.
        note: NoteDto,
    },
    /// Deletes the note of a cell.
    DeleteNote {
        /// Sheet.
        sheet: SheetId,
        /// Row.
        r: u32,
        /// Column.
        c: u32,
    },
    /// Sets a cell's hyperlink.
    SetLink {
        /// Sheet.
        sheet: SheetId,
        /// Row.
        r: u32,
        /// Column.
        c: u32,
        /// URL (`http(s):`, `mailto:`, `file:` or `#Sheet.A1`).
        url: String,
    },
    /// Removes the hyperlinks of every cell in `ranges`.
    ClearLink {
        /// Sheet.
        sheet: SheetId,
        /// Ranges.
        ranges: Vec<RangeDto>,
    },
    /// Protects a sheet (locked cells can no longer be edited).
    ProtectSheet {
        /// Sheet.
        sheet: SheetId,
        /// Optional password.
        password: Option<String>,
        /// Allowed actions (`None`: the defaults).
        options: Option<ProtectionOptions>,
    },
    /// Removes sheet protection (`OpError { code: "protected" }` on a wrong
    /// password).
    UnprotectSheet {
        /// Sheet.
        sheet: SheetId,
        /// The password, if one was set.
        password: Option<String>,
    },
    /// Sets the cell protection attributes of `ranges`.
    SetCellProtection {
        /// Sheet.
        sheet: SheetId,
        /// Ranges.
        ranges: Vec<RangeDto>,
        /// Locked (`None`: unchanged).
        locked: Option<bool>,
        /// Hide formulas (`None`: unchanged).
        hidden_formula: Option<bool>,
    },
    /// Protects the document structure (sheets cannot be added, deleted,
    /// renamed or moved).
    ProtectDocument {
        /// Optional password.
        password: Option<String>,
    },
    /// Removes document structure protection.
    UnprotectDocument {
        /// The password, if one was set.
        password: Option<String>,
    },
    /// Replaces a sheet's page setup (breaks in `setup` are ignored).
    SetPageSetup {
        /// Sheet.
        sheet: SheetId,
        /// The setup.
        setup: PageSetupDto,
    },
    /// Adds or removes a manual page break.
    SetPageBreak {
        /// Sheet.
        sheet: SheetId,
        /// Row or column break.
        axis: BreakAxis,
        /// The break is above this row / left of this column.
        at: u32,
        /// Add (`true`) or remove.
        present: bool,
    },
    /// Sets title, subject, keywords, description and author.
    SetDocumentProperties {
        /// The properties (read-only fields are ignored).
        props: DocPropertiesDto,
    },
    /// Re-applies the sheet's AutoFilter to the current values (filters
    /// are not re-applied after edits, as in AOO).
    RefreshFilter {
        /// Sheet.
        sheet: SheetId,
    },
    /// Sets rows `start..=end` to the height their wrapped or large-font
    /// content needs (rows with neither keep their height). The new
    /// heights are returned in `ApplyResult.fitted_rows`.
    AutoFitRows {
        /// Sheet.
        sheet: SheetId,
        /// First row.
        start: u32,
        /// Last row.
        end: u32,
    },
}

/// One sort key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SortKey {
    /// Absolute column.
    pub col: u32,
    /// Ascending (`true`) or descending.
    pub ascending: bool,
}

/// The filter of one AutoFilter column; a row is shown when its cell
/// passes every part that is set.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ColumnFilter {
    /// Formatted texts to keep (`""` keeps empty cells).
    pub values: Option<Vec<String>>,
    /// A condition to match.
    pub condition: Option<FilterCondition>,
}

/// A filter condition.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FilterCondition {
    /// Comparison.
    pub op: FilterOp,
    /// Operand; numbers compare numerically, text case-insensitively.
    pub value: String,
}

/// Filter comparisons.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum FilterOp {
    /// Equal.
    Eq,
    /// Not equal.
    Ne,
    /// Greater than.
    Gt,
    /// Greater or equal.
    Ge,
    /// Less than.
    Lt,
    /// Less or equal.
    Le,
    /// Contains the text.
    Contains,
    /// Starts with the text.
    BeginsWith,
    /// Ends with the text.
    EndsWith,
    /// The cell is empty.
    Empty,
    /// The cell is not empty.
    NotEmpty,
}

/// Options of [`Op::PasteSpecial`].
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PasteOptions {
    /// Paste values (formula results), not formulas or formats.
    pub values_only: bool,
    /// Paste formats only.
    pub formats_only: bool,
    /// Swap rows and columns.
    pub transpose: bool,
    /// Empty source cells leave the target unchanged.
    pub skip_empty: bool,
    /// Combine with the target's numbers (internal payloads only).
    pub operation: PasteOperation,
}

/// Arithmetic for [`PasteOptions::operation`].
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum PasteOperation {
    /// Replace the target.
    #[default]
    None,
    /// target + source.
    Add,
    /// target − source.
    Subtract,
    /// target × source.
    Multiply,
    /// target ÷ source.
    Divide,
}

/// Series type of [`Op::FillRange`].
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum FillMode {
    /// Detect: numbers, dates, day/month names and numbered text form
    /// series; other text and formulas are copied.
    Auto,
    /// Repeat the source.
    Copy,
    /// Linear series (step from the source, or 1).
    LinearSeries,
    /// Growth series (ratio from the source, or 1).
    GrowthSeries,
    /// Date series (step in days, or months when month-aligned).
    Date,
}

/// What [`Op::Clear`] removes.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ClearWhat {
    /// Values and formulas.
    Contents,
    /// Styles and number formats.
    Formats,
    /// Both.
    All,
}

/// A partial style; `None` fields are left unchanged.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StylePatch {
    /// Bold.
    pub bold: Option<bool>,
    /// Italic.
    pub italic: Option<bool>,
    /// Underline.
    pub underline: Option<bool>,
    /// Strikethrough.
    pub strike: Option<bool>,
    /// Font family.
    pub font_family: Option<String>,
    /// Font size in points.
    pub font_size_pt: Option<f32>,
    /// Text colour.
    pub color: Option<u32>,
    /// Background fill.
    pub fill: Option<u32>,
    /// Horizontal alignment.
    pub h_align: Option<HAlign>,
    /// Vertical alignment.
    pub v_align: Option<VAlign>,
    /// Wrap text.
    pub wrap: Option<bool>,
    /// Number format code.
    pub number_format: Option<String>,
    /// Border edges to set or remove; edges left `None` are unchanged.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub borders: Option<BordersPatch>,
}

/// Border edits for [`StylePatch::borders`], relative to each target range:
/// `top`/`bottom`/`left`/`right` apply to the outer edges of the range,
/// `inner_h`/`inner_v` to the lines between its rows/columns.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BordersPatch {
    /// Top edge of the range.
    pub top: Option<BorderEdgePatch>,
    /// Bottom edge of the range.
    pub bottom: Option<BorderEdgePatch>,
    /// Left edge of the range.
    pub left: Option<BorderEdgePatch>,
    /// Right edge of the range.
    pub right: Option<BorderEdgePatch>,
    /// Horizontal lines between the rows of the range.
    pub inner_h: Option<BorderEdgePatch>,
    /// Vertical lines between the columns of the range.
    pub inner_v: Option<BorderEdgePatch>,
}

/// One border edge edit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BorderEdgePatch {
    /// The line to draw, or `None` to remove the border.
    pub line: Option<BorderLineDto>,
}

/// Clipboard content for [`Op::Paste`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum PastePayload {
    /// The engine's own last copy (from [`ClipboardOut::token`]): formulas are
    /// re-anchored and styles kept.
    Internal {
        /// Token from the copy.
        token: String,
    },
    /// Tab-separated text from another application; each cell goes through
    /// the input recognizer.
    Tsv {
        /// The TSV text.
        text: String,
    },
}

/// A group of ops applied and undone together.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Transaction {
    /// Ops in order.
    pub ops: Vec<Op>,
    /// Undo menu label, e.g. "Bold".
    pub label: Option<String>,
    /// Selection to restore on undo.
    pub selection_before: Option<SelectionDto>,
    /// Selection to restore on redo.
    pub selection_after: Option<SelectionDto>,
}

/// Outcome of `apply`, `undo` or `redo`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ApplyResult {
    /// Document version after the change.
    #[ts(type = "number")]
    pub version: u64,
    /// Changed cells (including recalculated dependents), coalesced into rectangles.
    pub changed: Vec<SheetRanges>,
    /// Sheets whose row/column layout changed.
    pub layout_changed: Vec<SheetId>,
    /// Sheets were added, removed, renamed or reordered.
    pub sheets_changed: bool,
    /// Ops that failed (the rest of the transaction still applies).
    pub errors: Vec<OpError>,
    /// Undo/redo availability after the change.
    pub undo: UndoState,
    /// Selection the UI should show (from the transaction or undo record).
    pub selection: Option<SelectionDto>,
    /// Fresh workbook info, present when `sheets_changed`.
    pub info: Option<WorkbookInfo>,
    /// Problems of ops that still applied (validation warnings and
    /// information messages, code `"validation"`). Absent or `null` when
    /// there are none.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub warnings: Option<Vec<OpError>>,
    /// Row heights computed by `Op::AutoFitRows`, as pixel runs.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub fitted_rows: Option<Vec<DimRunPx>>,
    /// Number of cells `Op::ReplaceAll` changed (present when the
    /// transaction had a ReplaceAll op).
    #[serde(default)]
    #[ts(optional = nullable)]
    pub replaced: Option<u32>,
}

/// Ranges on one sheet.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SheetRanges {
    /// The sheet.
    pub sheet: SheetId,
    /// Ranges.
    pub ranges: Vec<RangeDto>,
}

/// A failed op inside a transaction.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct OpError {
    /// Index into [`Transaction::ops`].
    pub op_index: u32,
    /// Machine-readable code (see [`ApiError`] codes).
    pub code: String,
    /// Human-readable message.
    pub message: String,
}

/// Undo/redo availability.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UndoState {
    /// Undo is possible.
    pub can_undo: bool,
    /// Redo is possible.
    pub can_redo: bool,
    /// Label of the next undo.
    pub undo_label: Option<String>,
    /// Label of the next redo.
    pub redo_label: Option<String>,
}

// ---------------------------------------------------------------------------
// Selection, editing and assistance
// ---------------------------------------------------------------------------

/// A selection on one sheet.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SelectionDto {
    /// The sheet.
    pub sheet: SheetId,
    /// Selected ranges (Phase A: exactly one).
    pub ranges: Vec<RangeDto>,
    /// The active cell.
    pub active: CellPos,
}

/// Status bar statistics for a selection.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SelectionStats {
    /// Sum of numbers (booleans count as 1 and 0), or `None` if there are
    /// none or the selection contains an error.
    pub sum: Option<f64>,
    /// Non-empty cells.
    #[ts(type = "number")]
    pub count: u64,
    /// Numeric cells (booleans included).
    #[ts(type = "number")]
    pub count_numbers: u64,
    /// Average of numbers.
    pub average: Option<f64>,
    /// Minimum number.
    pub min: Option<f64>,
    /// Maximum number.
    pub max: Option<f64>,
    /// `sum` formatted with the active cell's number format; when the
    /// selection contains an error, that error's text (and `sum`,
    /// `average`, `min` and `max` are `None`), as AOO's status bar shows.
    pub formatted_sum: Option<String>,
}

/// What the formula bar shows for a cell.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CellEditInfo {
    /// Text to show in the editor (formulas in the UI dialect).
    pub input: String,
    /// Plain-English explanation of a formula, e.g. "A1 × B1".
    pub hint: Option<String>,
    /// The cell is part of an array (Ctrl+Shift+Enter) formula block; `input`
    /// is then shown as `{=…}` and edits must cover the whole block.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub array: Option<bool>,
    /// For array cells: the whole block.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub array_range: Option<RangeDto>,
}

/// A spreadsheet function, for autocomplete and help.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FunctionInfo {
    /// Canonical name, e.g. "SUMIF".
    pub name: String,
    /// Category, e.g. "Mathematical".
    pub category: String,
    /// Signature in the UI dialect, e.g. "SUMIF(range, criterion, [sum_range])".
    pub signature: String,
    /// One-line description.
    pub description: String,
}

/// A ⌘K palette suggestion.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Suggestion {
    /// Title shown in the list.
    pub title: String,
    /// Formula to insert, if any.
    pub formula: Option<String>,
    /// Why this suggestion fits.
    pub explanation: String,
    /// Formatted preview of the result.
    pub preview: Option<String>,
    /// 0..=1.
    pub confidence: f32,
    /// Command id to run instead of inserting a formula.
    pub action: Option<String>,
    /// Arguments for `action`, e.g. `column` = `G`, `range` = `F2:F15`
    /// (empty for formula suggestions).
    pub action_args: Vec<ActionArg>,
}

/// One named argument of a [`Suggestion`] action.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ActionArg {
    /// Argument name, e.g. "column".
    pub key: String,
    /// Argument value, e.g. "G".
    pub value: String,
}

/// Result of `copy`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ClipboardOut {
    /// `text/plain` content.
    pub tsv: String,
    /// `text/html` content with the token embedded.
    pub html: String,
    /// Token identifying the engine's internal snapshot.
    pub token: String,
}

/// A defined name.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NameInfo {
    /// The name as defined.
    pub name: String,
    /// Sheet scope, or `None` for the workbook.
    pub scope: Option<SheetId>,
    /// What it refers to, in CalcA1 syntax without `=`.
    pub refers_to: String,
    /// Name of the scope sheet.
    pub sheet_name: Option<String>,
}

/// The AutoFilter of a sheet.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AutoFilterInfo {
    /// The area including its header row, or `None` without an AutoFilter.
    pub range: Option<RangeDto>,
    /// One entry per column of the area.
    pub columns: Vec<FilterColumnInfo>,
}

/// One AutoFilter column.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FilterColumnInfo {
    /// Absolute column.
    pub col: u32,
    /// Distinct formatted values of the data rows, sorted (numbers first,
    /// then text case-insensitively; `""` for empty cells last), at most
    /// 10,000.
    pub unique: Vec<String>,
    /// How many data rows hold each entry of `unique`.
    pub counts: Vec<u32>,
    /// The active filter.
    pub active: Option<ColumnFilter>,
}

/// How [`FindQuery::text`] is interpreted.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum FindMode {
    /// Plain text.
    #[default]
    Literal,
    /// `*` any run, `?` one character, `~` escapes.
    Wildcards,
    /// A regular expression (Rust `regex` syntax, size-limited).
    Regex,
}

/// A find / replace query.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FindQuery {
    /// What to look for.
    pub text: String,
    /// Sheet to search, or `None` for every sheet in tab order.
    pub sheet: Option<SheetId>,
    /// Search formula text (and constants as typed) instead of displayed values.
    pub in_formulas: bool,
    /// Case-sensitive.
    pub match_case: bool,
    /// The whole cell must match.
    pub whole_cell: bool,
    /// Pattern syntax.
    pub mode: FindMode,
    /// Hits after this cell (row-major, on `sheet` or the active sheet) come
    /// first, wrapping around.
    pub from: Option<CellPos>,
}

/// Find results.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FindResult {
    /// Matching cells (at most 10,000).
    pub hits: Vec<FindHit>,
    /// More matches exist than were returned.
    pub truncated: bool,
}

/// One matching cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FindHit {
    /// Sheet.
    pub sheet: SheetId,
    /// Row.
    pub r: u32,
    /// Column.
    pub c: u32,
}

/// File format for saving.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum FileFormat {
    /// OpenDocument Spreadsheet.
    Ods,
    /// Office Open XML (not in Phase A).
    Xlsx,
    /// Comma-separated values (active sheet only).
    Csv,
}

/// Direction for Ctrl+arrow navigation.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Direction {
    /// Towards row 0.
    Up,
    /// Towards the last row.
    Down,
    /// Towards column 0.
    Left,
    /// Towards the last column.
    Right,
}

// ---------------------------------------------------------------------------
// Events and errors
// ---------------------------------------------------------------------------

/// Payload of the `doc-changed` event (changes the UI did not initiate).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DocChangedEvent {
    /// The document.
    pub doc: DocId,
    /// What changed.
    pub result: ApplyResult,
}

/// Payload of the `menu-action` event (native menu item chosen).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MenuActionEvent {
    /// Command id, e.g. "edit.undo".
    pub id: String,
}

/// Error returned by every fallible API call (and by Tauri commands as
/// `{ code, message }`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, thiserror::Error)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
#[error("{code}: {message}")]
pub struct ApiError {
    /// Machine-readable code, one of the `ApiError::*` constants.
    pub code: String,
    /// Human-readable message.
    pub message: String,
}

impl ApiError {
    /// Unknown document, sheet or other id.
    pub const NOT_FOUND: &'static str = "not_found";
    /// An argument is out of range or malformed.
    pub const INVALID_ARGUMENT: &'static str = "invalid_argument";
    /// Reading or writing a file failed.
    pub const IO: &'static str = "io";
    /// The file is corrupt, malicious or exceeds limits.
    pub const BAD_FILE: &'static str = "bad_file";
    /// The feature or format is not supported (yet).
    pub const UNSUPPORTED: &'static str = "unsupported";
    /// A bug.
    pub const INTERNAL: &'static str = "internal";
    /// The user dismissed a native dialog (shell only; the UI ignores it).
    pub const CANCELLED: &'static str = "cancelled";

    /// Creates an error.
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    /// Shorthand for a [`ApiError::NOT_FOUND`] error.
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(Self::NOT_FOUND, message)
    }

    /// Shorthand for an [`ApiError::INVALID_ARGUMENT`] error.
    pub fn invalid_argument(message: impl Into<String>) -> Self {
        Self::new(Self::INVALID_ARGUMENT, message)
    }

    /// Shorthand for an [`ApiError::UNSUPPORTED`] error.
    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::new(Self::UNSUPPORTED, message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn op_json_shape_is_a_discriminated_union() {
        let op = Op::SetCellInput {
            sheet: 1,
            r: 0,
            c: 2,
            input: "=A1*B1".into(),
        };
        assert_eq!(
            serde_json::to_value(&op).unwrap(),
            json!({ "type": "setCellInput", "sheet": 1, "r": 0, "c": 2, "input": "=A1*B1" })
        );
        let paste: Op = serde_json::from_value(json!({
            "type": "paste", "sheet": 0,
            "target": { "r0": 0, "c0": 0, "r1": 0, "c1": 0 },
            "payload": { "type": "tsv", "text": "1\t2" }
        }))
        .unwrap();
        assert!(matches!(
            paste,
            Op::Paste {
                payload: PastePayload::Tsv { .. },
                ..
            }
        ));
    }

    #[test]
    fn structs_are_camel_case() {
        let tx = Transaction {
            ops: vec![Op::AddSheet {
                name: None,
                index: Some(1),
            }],
            label: Some("Add sheet".into()),
            selection_before: None,
            selection_after: Some(SelectionDto {
                sheet: 0,
                ranges: vec![RangeDto {
                    r0: 0,
                    c0: 0,
                    r1: 0,
                    c1: 0,
                }],
                active: CellPos { r: 0, c: 0 },
            }),
        };
        let v = serde_json::to_value(&tx).unwrap();
        assert!(v.get("selectionBefore").is_some());
        assert!(v.get("selectionAfter").is_some());
        assert_eq!(
            v["ops"][0],
            json!({ "type": "addSheet", "name": null, "index": 1 })
        );
        let back: Transaction = serde_json::from_value(v).unwrap();
        assert_eq!(back, tx);
    }

    #[test]
    fn simple_enums_and_newtypes() {
        assert_eq!(serde_json::to_value(FileFormat::Ods).unwrap(), json!("ods"));
        assert_eq!(
            serde_json::to_value(Direction::Left).unwrap(),
            json!("left")
        );
        assert_eq!(serde_json::to_value(ClearWhat::All).unwrap(), json!("all"));
        assert_eq!(serde_json::to_value(DocId(7)).unwrap(), json!(7));
        assert_eq!(
            serde_json::to_value(DisplayDto::Chip { bg: 1, fg: 2 }).unwrap(),
            json!({ "type": "chip", "bg": 1, "fg": 2 })
        );
    }

    #[test]
    fn api_error_display_and_json() {
        let e = ApiError::not_found("no document 3");
        assert_eq!(e.to_string(), "not_found: no document 3");
        assert_eq!(
            serde_json::to_value(&e).unwrap(),
            json!({ "code": "not_found", "message": "no document 3" })
        );
    }

    /// Exports every DTO explicitly (ts-rs also generates one test per type).
    /// Writes into `TS_RS_EXPORT_DIR`.
    #[test]
    fn export_typescript_bindings() {
        let cfg = ts_rs::Config::from_env();
        WorkbookInfo::export_all(&cfg).unwrap();
        SheetLayout::export_all(&cfg).unwrap();
        CellBlock::export_all(&cfg).unwrap();
        StyleDto::export_all(&cfg).unwrap();
        Transaction::export_all(&cfg).unwrap();
        ApplyResult::export_all(&cfg).unwrap();
        SelectionStats::export_all(&cfg).unwrap();
        CellEditInfo::export_all(&cfg).unwrap();
        FunctionInfo::export_all(&cfg).unwrap();
        Suggestion::export_all(&cfg).unwrap();
        ActionArg::export_all(&cfg).unwrap();
        ClipboardOut::export_all(&cfg).unwrap();
        FileFormat::export_all(&cfg).unwrap();
        Direction::export_all(&cfg).unwrap();
        DocChangedEvent::export_all(&cfg).unwrap();
        MenuActionEvent::export_all(&cfg).unwrap();
        ApiError::export_all(&cfg).unwrap();
        NameInfo::export_all(&cfg).unwrap();
        AutoFilterInfo::export_all(&cfg).unwrap();
        FindQuery::export_all(&cfg).unwrap();
        FindResult::export_all(&cfg).unwrap();
        crate::features::ConditionalFormatDto::export_all(&cfg).unwrap();
        crate::features::ValidationInfo::export_all(&cfg).unwrap();
        crate::features::ProtectionInfo::export_all(&cfg).unwrap();
        crate::features::GoalSeekRequest::export_all(&cfg).unwrap();
        crate::features::GoalSeekResult::export_all(&cfg).unwrap();
        crate::features::LocaleInfo::export_all(&cfg).unwrap();
    }

    #[test]
    fn v03_shapes() {
        use crate::features::*;
        let op = Op::AddConditionalFormat {
            sheet: 1,
            ranges: vec![RangeDto {
                r0: 0,
                c0: 0,
                r1: 9,
                c1: 0,
            }],
            rules: vec![CfRuleDto {
                kind: CfRuleKind::CellValue {
                    op: CmpOp::Gt,
                    formulas: vec!["5".into()],
                },
                style: CfStyleDto {
                    bold: Some(true),
                    ..CfStyleDto::default()
                },
                stop_if_true: false,
            }],
        };
        let v = serde_json::to_value(&op).unwrap();
        assert_eq!(v["type"], "addConditionalFormat");
        assert_eq!(v["rules"][0]["kind"]["type"], "cellValue");
        assert_eq!(v["rules"][0]["kind"]["op"], "gt");
        assert_eq!(v["rules"][0]["stopIfTrue"], false);
        let back: Op = serde_json::from_value(v).unwrap();
        assert_eq!(back, op);
        // Old CellView JSON (no v0.3 fields) still parses.
        let cv: CellView = serde_json::from_value(json!({
            "r": 0, "c": 0, "text": "", "kind": "empty", "num": null,
            "style": 0, "color": null, "flags": 0
        }))
        .unwrap();
        assert_eq!(cv.cf_style, None);
        assert_eq!(
            serde_json::to_value(IconSetKind::ThreeTrafficLights).unwrap(),
            json!("threeTrafficLights")
        );
        assert_eq!(
            serde_json::to_value(PageScale::FitTo {
                width: 1,
                height: 0
            })
            .unwrap(),
            json!({ "type": "fitTo", "width": 1, "height": 0 })
        );
    }

    #[test]
    fn v02_shapes() {
        let op = Op::SortRange {
            sheet: 1,
            range: RangeDto {
                r0: 0,
                c0: 0,
                r1: 9,
                c1: 2,
            },
            keys: vec![SortKey {
                col: 1,
                ascending: false,
            }],
            has_header: true,
            case_sensitive: false,
        };
        let v = serde_json::to_value(&op).unwrap();
        assert_eq!(v["type"], "sortRange");
        assert_eq!(v["hasHeader"], true);
        assert_eq!(v["keys"][0], json!({ "col": 1, "ascending": false }));
        // New optional fields may be omitted by older clients.
        let p: StylePatch = serde_json::from_value(json!({
            "bold": null, "italic": null, "underline": null, "strike": null,
            "fontFamily": null, "fontSizePt": null, "color": null, "fill": null,
            "hAlign": null, "vAlign": null, "wrap": null, "numberFormat": null
        }))
        .unwrap();
        assert_eq!(p.borders, None);
        let fill: Op = serde_json::from_value(json!({
            "type": "fillRange", "sheet": 1,
            "source": { "r0": 0, "c0": 0, "r1": 0, "c1": 0 },
            "target": { "r0": 0, "c0": 0, "r1": 5, "c1": 0 },
            "mode": "auto"
        }))
        .unwrap();
        assert!(matches!(
            fill,
            Op::FillRange {
                mode: FillMode::Auto,
                ..
            }
        ));
        assert_eq!(
            serde_json::to_value(FilterOp::BeginsWith).unwrap(),
            json!("beginsWith")
        );
        assert_eq!(
            serde_json::to_value(FindMode::Regex).unwrap(),
            json!("regex")
        );
    }
}
