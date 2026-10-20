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

use crate::charts::{ChartAnchor, ChartDto, ChartRef, ImageDto};
use crate::features::{
    BreakAxis, CfRuleDto, DataBarView, DocPropertiesDto, IconView, NoteDto, PageSetupDto,
    ProtectionOptions, ValidationDto,
};
use crate::pivot::{AggFn, PivotDto};
use crate::print::{CalcSettingsDto, ExternalLinkWarning};
use crate::tools::{
    AutoFormatOptions, CellStyleDto, CsvImportOptionsDto, DateUnit, DeleteContentsFlags,
    NameLabelFlags, OutlineAxis, ScenarioDto, SeriesKind, SheetCell, SheetRange, ShiftDir,
    SortOptions, SplitDto, StdCondition, StdFilterOptions, SubtotalGroup, SubtotalOptions,
    UnderlineStyle, ViewOptionsDto,
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
    /// External links the reader disabled by policy (DDE, external
    /// references, linked sheets); absent or `null` when there are none
    /// (v0.6).
    #[serde(default)]
    #[ts(optional = nullable)]
    pub external_links: Option<Vec<ExternalLinkWarning>>,
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
    /// Text rotation in degrees counter-clockwise, `-90..=90`; absent or
    /// `null` = 0 (v0.4).
    #[serde(default)]
    #[ts(optional = nullable)]
    pub rotation_deg: Option<i16>,
    /// Vertically stacked characters (v0.4).
    #[serde(default)]
    #[ts(optional = nullable)]
    pub stacked: Option<bool>,
    /// Shrink the text to fit the cell (v0.4).
    #[serde(default)]
    #[ts(optional = nullable)]
    pub shrink_to_fit: Option<bool>,
    /// Underline style, when `underline` is set; absent = single (v0.4).
    #[serde(default)]
    #[ts(optional = nullable)]
    pub underline_style: Option<UnderlineStyle>,
    /// The named cell style the cell is based on; absent = `Default`
    /// (v0.4).
    #[serde(default)]
    #[ts(optional = nullable)]
    pub style_name: Option<String>,
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
    /// Diagonal from the top-left to the bottom-right corner (v0.4).
    #[serde(default)]
    #[ts(optional = nullable)]
    pub diagonal_down: Option<BorderLineDto>,
    /// Diagonal from the bottom-left to the top-right corner (v0.4).
    #[serde(default)]
    #[ts(optional = nullable)]
    pub diagonal_up: Option<BorderLineDto>,
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
        /// v0.5 options (left-to-right, copy results to, custom order,
        /// include formats); absent or `null` = the v0.2 behaviour.
        #[serde(default)]
        #[ts(optional = nullable)]
        options: Option<SortOptions>,
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
    // ----- v0.4: charts and pictures (W5-1) ------------------------------
    /// Adds a chart (`chart.id` and `chart.name` are ignored). When
    /// `chart.series` is empty and `chart.source` is set, the engine
    /// derives the series. The id comes back in `ApplyResult.created`.
    /// Refused on a protected sheet (AOO protects objects with the sheet).
    AddChart {
        /// Sheet.
        sheet: SheetId,
        /// The chart.
        chart: ChartDto,
    },
    /// Replaces a chart; the id is kept.
    UpdateChart {
        /// Sheet.
        sheet: SheetId,
        /// Id from `charts`.
        id: u32,
        /// The new definition.
        chart: ChartDto,
    },
    /// Moves and/or resizes a chart.
    MoveResizeChart {
        /// Sheet.
        sheet: SheetId,
        /// Id.
        id: u32,
        /// The new placement.
        anchor: ChartAnchor,
    },
    /// Deletes a chart.
    DeleteChart {
        /// Sheet.
        sheet: SheetId,
        /// Id.
        id: u32,
    },
    /// Moves a chart to another sheet (same anchor cell and size).
    MoveChartToSheet {
        /// Sheet.
        sheet: SheetId,
        /// Id.
        id: u32,
        /// Destination sheet.
        to: SheetId,
    },
    /// Adds a picture (`image.id`/`image.name` are ignored; `image.data`
    /// is base64 PNG or JPEG, ≤ 16 MB decoded). The id comes back in
    /// `ApplyResult.created`.
    AddImage {
        /// Sheet.
        sheet: SheetId,
        /// The picture.
        image: ImageDto,
    },
    /// Deletes a picture.
    DeleteImage {
        /// Sheet.
        sheet: SheetId,
        /// Id.
        id: u32,
    },
    /// Moves and/or resizes a picture.
    MoveResizeImage {
        /// Sheet.
        sheet: SheetId,
        /// Id.
        id: u32,
        /// The new placement.
        anchor: ChartAnchor,
    },
    // ----- v0.5: DataPilot (W5-2) ----------------------------------------
    /// Creates a pivot table (`pivot.id` is ignored) and writes its
    /// output. When the target area is not empty, the op fails with
    /// `OpError { code: "target_not_empty" }` unless `overwrite`.
    CreatePivot {
        /// The definition.
        pivot: PivotDto,
        /// Overwrite non-empty target cells.
        overwrite: bool,
    },
    /// Replaces a pivot's definition and rewrites its output.
    UpdatePivot {
        /// Id from `pivots`.
        id: u32,
        /// The new definition.
        pivot: PivotDto,
        /// Overwrite non-empty target cells outside the previous output.
        overwrite: bool,
    },
    /// Recomputes a pivot from its source (AOO never refreshes
    /// automatically).
    RefreshPivot {
        /// Id.
        id: u32,
    },
    /// Deletes a pivot.
    DeletePivot {
        /// Id.
        id: u32,
        /// Also clear its output cells.
        clear_output: bool,
    },
    /// Inserts a sheet holding the source rows behind a data cell.
    PivotDrillDown {
        /// Id.
        id: u32,
        /// Row of the data cell.
        r: u32,
        /// Column of the data cell.
        c: u32,
    },
    // ----- v0.5: outline (W5-2) ------------------------------------------
    /// Groups rows or columns `start..=end` (one level deeper; at most 8).
    Group {
        /// Sheet.
        sheet: SheetId,
        /// Rows or columns.
        axis: OutlineAxis,
        /// First row/column.
        start: u32,
        /// Last row/column.
        end: u32,
    },
    /// Removes the innermost group covering `start..=end`.
    Ungroup {
        /// Sheet.
        sheet: SheetId,
        /// Rows or columns.
        axis: OutlineAxis,
        /// First row/column.
        start: u32,
        /// Last row/column.
        end: u32,
    },
    /// Collapses or expands the group covering `start..=end` (hides or
    /// shows its rows/columns).
    SetOutlineCollapsed {
        /// Sheet.
        sheet: SheetId,
        /// Rows or columns.
        axis: OutlineAxis,
        /// First row/column.
        start: u32,
        /// Last row/column.
        end: u32,
        /// Collapse (`true`) or expand.
        collapsed: bool,
    },
    /// Shows groups up to `level` (the gutter's level buttons; `0` shows
    /// everything collapsed, `8` everything expanded).
    ShowOutlineLevel {
        /// Sheet.
        sheet: SheetId,
        /// Rows or columns.
        axis: OutlineAxis,
        /// Level.
        level: u8,
    },
    /// Builds groups from the formula structure of `range` (or the used
    /// area).
    AutoOutline {
        /// Sheet.
        sheet: SheetId,
        /// Area, or `None` for the used area.
        range: Option<RangeDto>,
    },
    /// Removes every group (of one axis, or both).
    ClearOutline {
        /// Sheet.
        sheet: SheetId,
        /// Axis, or `None` for both.
        axis: Option<OutlineAxis>,
    },
    // ----- v0.5: filters, subtotals, data tools (W5-2) --------------------
    /// Applies a standard filter (up to 4 conditions joined by AND/OR) to
    /// `range`; rows are hidden, or copied to `options.copy_to`.
    StandardFilter {
        /// Sheet.
        sheet: SheetId,
        /// The area (header row first when `options.has_header`).
        range: RangeDto,
        /// Conditions.
        conditions: Vec<StdCondition>,
        /// Options.
        options: StdFilterOptions,
    },
    /// Applies an advanced filter: `criteria` holds a criteria block in
    /// the D-function form (labels, then one OR row per line).
    AdvancedFilter {
        /// Sheet.
        sheet: SheetId,
        /// The area.
        range: RangeDto,
        /// The criteria block.
        criteria: SheetRange,
        /// Options.
        options: StdFilterOptions,
    },
    /// Removes the standard/advanced filter and shows its rows (the
    /// AutoFilter buttons stay).
    ResetFilter {
        /// Sheet.
        sheet: SheetId,
    },
    /// Inserts subtotal rows (`SUBTOTAL(n;…)` formulas, `X Result` and
    /// `Grand Total` labels) and builds an outline.
    Subtotals {
        /// Sheet.
        sheet: SheetId,
        /// The area including its header row.
        range: RangeDto,
        /// Groups (at most 3).
        groups: Vec<SubtotalGroup>,
        /// Options.
        options: SubtotalOptions,
    },
    /// Removes the subtotal rows and outline of `range`.
    RemoveSubtotals {
        /// Sheet.
        sheet: SheetId,
        /// The area.
        range: RangeDto,
    },
    /// Consolidates `sources` into `dest`, by position or by labels.
    Consolidate {
        /// Function.
        function: AggFn,
        /// Source areas.
        sources: Vec<SheetRange>,
        /// Destination top-left.
        dest: SheetCell,
        /// Match row labels.
        by_row_labels: bool,
        /// Match column labels.
        by_col_labels: bool,
        /// Write formulas linking to the sources instead of values.
        link: bool,
    },
    /// Splits the text of the column `range` into the columns to its
    /// right (overwriting them).
    TextToColumns {
        /// Sheet.
        sheet: SheetId,
        /// One column of cells.
        range: RangeDto,
        /// Separators and column types.
        options: CsvImportOptionsDto,
    },
    /// Removes rows of `range` whose `cols` repeat an earlier row (the
    /// first occurrence stays).
    RemoveDuplicates {
        /// Sheet.
        sheet: SheetId,
        /// The area.
        range: RangeDto,
        /// Columns compared (absolute; empty = all).
        cols: Vec<u32>,
        /// The first row holds labels.
        has_header: bool,
    },
    /// Shifts cells of `range` down or right, inserting empty cells.
    InsertCells {
        /// Sheet.
        sheet: SheetId,
        /// The area.
        range: RangeDto,
        /// `down` or `right`.
        shift: ShiftDir,
    },
    /// Deletes the cells of `range`, shifting the rest up or left.
    DeleteCells {
        /// Sheet.
        sheet: SheetId,
        /// The area.
        range: RangeDto,
        /// `up` or `left`.
        shift: ShiftDir,
    },
    /// Delete Contents dialog: removes only the chosen kinds.
    DeleteContents {
        /// Sheet.
        sheet: SheetId,
        /// Ranges.
        ranges: Vec<RangeDto>,
        /// What to remove.
        what: DeleteContentsFlags,
    },
    /// Fill Series dialog: fills `range` from its first cell in
    /// `direction`.
    FillSeries {
        /// Sheet.
        sheet: SheetId,
        /// The area to fill.
        range: RangeDto,
        /// Direction.
        direction: Direction,
        /// Series type.
        kind: SeriesKind,
        /// Time unit (date series).
        unit: Option<DateUnit>,
        /// Start value as typed (`None`: the first cell's value).
        start: Option<String>,
        /// End value as typed (`None`: fill the whole range).
        end: Option<String>,
        /// Increment as typed (`None`: 1).
        increment: Option<String>,
    },
    // ----- v0.5: scenarios, cell styles, AutoFormat (W5-2 / W5-6) ---------
    /// Creates a scenario from the current values of `scenario.ranges`
    /// (`scenario.id` is ignored). The id comes back in
    /// `ApplyResult.created`.
    CreateScenario {
        /// Base sheet.
        sheet: SheetId,
        /// The scenario.
        scenario: ScenarioDto,
    },
    /// Replaces a scenario's settings (its values are kept).
    UpdateScenario {
        /// Base sheet.
        sheet: SheetId,
        /// Id.
        id: u32,
        /// The new settings.
        scenario: ScenarioDto,
    },
    /// Shows a scenario (its values replace the ranges' cells; with
    /// `copy_back`, the current values are stored into the scenario shown
    /// before).
    ShowScenario {
        /// Base sheet.
        sheet: SheetId,
        /// Id.
        id: u32,
    },
    /// Deletes a scenario.
    DeleteScenario {
        /// Base sheet.
        sheet: SheetId,
        /// Id.
        id: u32,
    },
    /// Defines or redefines a named cell style; cells using it update.
    DefineCellStyle {
        /// The style (`style.builtin` is ignored).
        style: CellStyleDto,
    },
    /// Deletes a named style; cells using it fall back to its parent.
    DeleteCellStyle {
        /// Name.
        name: String,
    },
    /// Applies a named style to `ranges` (direct formatting is cleared).
    ApplyCellStyle {
        /// Sheet.
        sheet: SheetId,
        /// Ranges.
        ranges: Vec<RangeDto>,
        /// Style name.
        name: String,
    },
    /// Applies one of AOO's 17 AutoFormat presets to `range`.
    ApplyAutoFormat {
        /// Sheet.
        sheet: SheetId,
        /// The area.
        range: RangeDto,
        /// Preset name (`Default`, `3D`, `Blue`, `Currency` …).
        preset: String,
        /// Which parts to apply.
        options: AutoFormatOptions,
    },
    // ----- v0.5: sheets, names, view, settings ----------------------------
    /// Copies a sheet (formulas in the copy point at the copy).
    CopySheet {
        /// Sheet.
        sheet: SheetId,
        /// Name of the copy, or `None` for "Name_2".
        name: Option<String>,
        /// Tab index of the copy, or `None` for after the source.
        index: Option<u32>,
    },
    /// Hides or shows a sheet (the last visible sheet cannot be hidden).
    SetSheetHidden {
        /// Sheet.
        sheet: SheetId,
        /// Hide (`true`) or show.
        hidden: bool,
    },
    /// Sets or clears a sheet's tab colour.
    SetTabColor {
        /// Sheet.
        sheet: SheetId,
        /// `0xRRGGBBAA`, or `None` for no colour.
        color: Option<u32>,
    },
    /// Creates names from the labels on the edges of `range`.
    CreateNamesFromLabels {
        /// Sheet.
        sheet: SheetId,
        /// The area including its labels.
        range: RangeDto,
        /// Which edges hold labels.
        from: NameLabelFlags,
    },
    /// Defines (or redefines) a named database range.
    DefineDatabaseRange {
        /// Name.
        name: String,
        /// Sheet.
        sheet: SheetId,
        /// The area.
        range: RangeDto,
        /// The first row holds labels.
        has_header: bool,
    },
    /// Deletes a named database range (its cells are kept).
    DeleteDatabaseRange {
        /// Name.
        name: String,
    },
    /// Sets a sheet's view options (persisted, undoable).
    SetViewOptions {
        /// Sheet.
        sheet: SheetId,
        /// The options.
        options: ViewOptionsDto,
    },
    /// Sets (or with `None` removes) the Window ▸ Split position.
    SetSplit {
        /// Sheet.
        sheet: SheetId,
        /// The split.
        split: Option<SplitDto>,
    },
    /// Replaces the custom sort lists.
    SetCustomSortLists {
        /// The lists.
        lists: Vec<Vec<String>>,
    },
    /// Sets the calculation settings (undoable; followed by a hard
    /// recalc).
    SetCalcSettings {
        /// The settings.
        settings: CalcSettingsDto,
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
    /// Text rotation in degrees counter-clockwise, `-90..=90` (v0.4).
    #[serde(default)]
    #[ts(optional = nullable)]
    pub rotation_deg: Option<i16>,
    /// Vertically stacked characters (v0.4).
    #[serde(default)]
    #[ts(optional = nullable)]
    pub stacked: Option<bool>,
    /// Shrink to fit (v0.4).
    #[serde(default)]
    #[ts(optional = nullable)]
    pub shrink_to_fit: Option<bool>,
    /// Indent level (v0.4).
    #[serde(default)]
    #[ts(optional = nullable)]
    pub indent: Option<u8>,
    /// Underline style (v0.4; also sets `underline` when not `None`).
    #[serde(default)]
    #[ts(optional = nullable)]
    pub underline_style: Option<UnderlineStyle>,
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
    /// Diagonal top-left to bottom-right, per cell (v0.4).
    #[serde(default)]
    #[ts(optional = nullable)]
    pub diagonal_down: Option<BorderEdgePatch>,
    /// Diagonal bottom-left to top-right, per cell (v0.4).
    #[serde(default)]
    #[ts(optional = nullable)]
    pub diagonal_up: Option<BorderEdgePatch>,
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
    /// Charts whose resolved data changed (cells in a series range
    /// changed, recalculated, or became hidden/visible). The UI re-fetches
    /// `chart_data` for the visible ones (v0.4).
    #[serde(default)]
    #[ts(optional = nullable)]
    pub charts_changed: Option<Vec<ChartRef>>,
    /// Sheets whose object list changed (a chart or picture was added,
    /// removed, moved or updated). Re-fetch `charts()` / `images()` (v0.4).
    #[serde(default)]
    #[ts(optional = nullable)]
    pub objects_changed: Option<Vec<SheetId>>,
    /// Ids allocated by `Add*`/`Create*` ops, in op order (v0.4).
    #[serde(default)]
    #[ts(optional = nullable)]
    pub created: Option<Vec<CreatedId>>,
    /// The pivot table list changed (create, update, delete, refresh)
    /// (v0.5).
    #[serde(default)]
    #[ts(optional = nullable)]
    pub pivots_changed: Option<bool>,
}

/// An id allocated by an `Add*`/`Create*` op (`ApplyResult.created`).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CreatedId {
    /// Index into `Transaction::ops`.
    pub op_index: u32,
    /// The new id.
    pub id: u32,
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
        // v0.4–v0.6 (wave 5)
        crate::charts::ChartDataDto::export_all(&cfg).unwrap();
        crate::charts::ImageDto::export_all(&cfg).unwrap();
        crate::pivot::PivotFieldItemsDto::export_all(&cfg).unwrap();
        crate::pivot::PivotHit::export_all(&cfg).unwrap();
        crate::print::PrintOptionsDto::export_all(&cfg).unwrap();
        crate::print::ChartSvgDto::export_all(&cfg).unwrap();
        crate::print::PrintJobInfo::export_all(&cfg).unwrap();
        crate::print::PageBreaksDto::export_all(&cfg).unwrap();
        crate::print::SolverRequest::export_all(&cfg).unwrap();
        crate::print::SolverResult::export_all(&cfg).unwrap();
        crate::tools::OutlineGroupDto::export_all(&cfg).unwrap();
        crate::tools::StandardFilterDto::export_all(&cfg).unwrap();
        crate::tools::DatabaseRangeDto::export_all(&cfg).unwrap();
        crate::tools::AutocompleteDto::export_all(&cfg).unwrap();
        crate::tools::TraceDto::export_all(&cfg).unwrap();
        crate::tools::NavigatorDto::export_all(&cfg).unwrap();
        crate::tools::DocumentStatsDto::export_all(&cfg).unwrap();
    }

    #[test]
    fn v04_shapes() {
        use crate::charts::*;
        use crate::features::PageScale;
        use crate::pivot::*;
        use crate::print::*;
        use crate::tools::*;
        // Charts.
        let op = Op::MoveResizeChart {
            sheet: 1,
            id: 3,
            anchor: ChartAnchor {
                r: 0,
                c: 4,
                dx_mm: 1.0,
                dy_mm: 0.0,
                width_mm: 160.0,
                height_mm: 90.0,
            },
        };
        let v = serde_json::to_value(&op).unwrap();
        assert_eq!(v["type"], "moveResizeChart");
        assert_eq!(v["anchor"]["widthMm"], 160.0);
        let back: Op = serde_json::from_value(v).unwrap();
        assert_eq!(back, op);
        // DataPilot.
        let op = Op::DeletePivot {
            id: 1,
            clear_output: true,
        };
        assert_eq!(
            serde_json::to_value(&op).unwrap(),
            json!({ "type": "deletePivot", "id": 1, "clearOutput": true })
        );
        // Outline and data tools.
        let op = Op::Group {
            sheet: 1,
            axis: OutlineAxis::Rows,
            start: 2,
            end: 5,
        };
        assert_eq!(
            serde_json::to_value(&op).unwrap(),
            json!({ "type": "group", "sheet": 1, "axis": "rows", "start": 2, "end": 5 })
        );
        let op: Op = serde_json::from_value(json!({
            "type": "standardFilter", "sheet": 1,
            "range": { "r0": 0, "c0": 0, "r1": 9, "c1": 2 },
            "conditions": [{ "connector": "and", "col": 1, "op": "gt", "value": "5" }],
            "options": { "caseSensitive": false, "regex": false, "hasHeader": true,
                         "noDuplicates": false, "copyTo": null, "keepCriteria": true }
        }))
        .unwrap();
        assert!(matches!(op, Op::StandardFilter { ref conditions, .. } if conditions.len() == 1));
        let op = Op::Consolidate {
            function: AggFn::Average,
            sources: vec![SheetRange {
                sheet: 1,
                range: RangeDto {
                    r0: 0,
                    c0: 0,
                    r1: 3,
                    c1: 1,
                },
            }],
            dest: SheetCell {
                sheet: 2,
                r: 0,
                c: 0,
            },
            by_row_labels: true,
            by_col_labels: false,
            link: false,
        };
        assert_eq!(serde_json::to_value(&op).unwrap()["function"], "average");
        let op = Op::InsertCells {
            sheet: 1,
            range: RangeDto {
                r0: 1,
                c0: 1,
                r1: 2,
                c1: 2,
            },
            shift: ShiftDir::Right,
        };
        assert_eq!(serde_json::to_value(&op).unwrap()["shift"], "right");
        let op = Op::FillSeries {
            sheet: 1,
            range: RangeDto {
                r0: 0,
                c0: 0,
                r1: 9,
                c1: 0,
            },
            direction: Direction::Down,
            kind: SeriesKind::Date,
            unit: Some(DateUnit::Month),
            start: Some("2026-01-01".into()),
            end: None,
            increment: Some("1".into()),
        };
        let v = serde_json::to_value(&op).unwrap();
        assert_eq!(v["unit"], "month");
        assert_eq!(v["kind"], "date");
        // Settings.
        let op = Op::SetCalcSettings {
            settings: CalcSettingsDto {
                iterations: None,
                date_epoch: DateEpochDto::D1899,
                case_sensitive: false,
                precision_as_shown: false,
                whole_cell_match: true,
                criteria_mode: CriteriaModeDto::Wildcards,
                text_to_number: TextToNumberDto::ConvertUnambiguous,
            },
        };
        assert_eq!(
            serde_json::to_value(&op).unwrap()["settings"]["criteriaMode"],
            "wildcards"
        );
        // SortRange without the v0.5 options (an older client) still parses.
        let sort: Op = serde_json::from_value(json!({
            "type": "sortRange", "sheet": 1,
            "range": { "r0": 0, "c0": 0, "r1": 9, "c1": 2 },
            "keys": [{ "col": 1, "ascending": false }],
            "hasHeader": true, "caseSensitive": false
        }))
        .unwrap();
        assert!(matches!(sort, Op::SortRange { options: None, .. }));
        // ApplyResult without the v0.4 fields still parses; with them it
        // round-trips.
        let r: ApplyResult = serde_json::from_value(json!({
            "version": 1, "changed": [], "layoutChanged": [], "sheetsChanged": false,
            "errors": [], "undo": { "canUndo": false, "canRedo": false,
            "undoLabel": null, "redoLabel": null }, "selection": null, "info": null
        }))
        .unwrap();
        assert!(r.created.is_none() && r.charts_changed.is_none());
        let r = ApplyResult {
            charts_changed: Some(vec![ChartRef { sheet: 1, id: 2 }]),
            objects_changed: Some(vec![1]),
            created: Some(vec![CreatedId { op_index: 0, id: 2 }]),
            pivots_changed: Some(true),
            ..r
        };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["created"][0], json!({ "opIndex": 0, "id": 2 }));
        assert_eq!(v["chartsChanged"][0], json!({ "sheet": 1, "id": 2 }));
        // Style extras are optional in both directions.
        let p: StylePatch = serde_json::from_value(json!({
            "bold": null, "italic": null, "underline": null, "strike": null,
            "fontFamily": null, "fontSizePt": null, "color": null, "fill": null,
            "hAlign": null, "vAlign": null, "wrap": null, "numberFormat": null
        }))
        .unwrap();
        assert_eq!(p.rotation_deg, None);
        assert_eq!(
            serde_json::to_value(PageScale::FitPages { pages: 2 }).unwrap(),
            json!({ "type": "fitPages", "pages": 2 })
        );
        let b: BordersDto = serde_json::from_value(json!({
            "top": null, "bottom": null, "left": null, "right": null
        }))
        .unwrap();
        assert_eq!(b.diagonal_down, None);
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
            options: None,
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
