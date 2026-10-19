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
    /// Sum of numbers, or `None` if there are none.
    pub sum: Option<f64>,
    /// Non-empty cells.
    #[ts(type = "number")]
    pub count: u64,
    /// Numeric cells.
    #[ts(type = "number")]
    pub count_numbers: u64,
    /// Average of numbers.
    pub average: Option<f64>,
    /// Minimum number.
    pub min: Option<f64>,
    /// Maximum number.
    pub max: Option<f64>,
    /// `sum` formatted with the active cell's number format.
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
    }
}
