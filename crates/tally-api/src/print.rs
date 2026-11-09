//! v0.6 DTOs: printing, PDF export, calculation settings, the solver and
//! external-link policy warnings (wave 5, W5-3).
//!
//! Same serde and ts-rs conventions as [`crate::dto`].

use serde::{Deserialize, Serialize};
use tally_types::SheetId;
use ts_rs::TS;

use crate::dto::RangeDto;
use crate::tools::{SheetCell, SheetRange};

// ---------------------------------------------------------------------------
// Print layout and jobs
// ---------------------------------------------------------------------------

/// Which sheets a print covers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum PrintSheets {
    /// The active sheet.
    Active,
    /// Every visible sheet.
    All,
    /// These sheets, in tab order.
    Some {
        /// Sheet ids.
        ids: Vec<SheetId>,
    },
}

/// Options of `print_layout` and `prepare_print_job`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PrintOptionsDto {
    /// Sheets.
    pub sheets: PrintSheets,
    /// Print only these selected ranges of the active sheet (AOO's
    /// "Selected cells"), each as its own print range, in this order. A
    /// whole-row or whole-column range ends at the sheet's data.
    pub selection: Option<Vec<RangeDto>>,
    /// Skip pages without content (AOO default `true`).
    pub suppress_empty_pages: bool,
    /// First page to print (1-based), or all.
    pub page_from: Option<u32>,
    /// Last page to print (1-based).
    pub page_to: Option<u32>,
    /// The sheet the user is on. [`PrintSheets::Active`] and `selection`
    /// resolve against it; `None` uses the workbook's stored active sheet,
    /// which only follows sheet switches that went through an edit.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub active_sheet: Option<SheetId>,
    /// Pages to print as inclusive 1-based `[first, last]` ranges over the
    /// job ("1-3,5" is `[[1, 3], [5, 5]]`), or all. Pages keep their job
    /// order and numbering; ranges past the end select nothing.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub page_ranges: Option<Vec<[u32; 2]>>,
}

impl Default for PrintOptionsDto {
    /// The active sheet, every page, empty pages suppressed.
    fn default() -> Self {
        Self {
            sheets: PrintSheets::Active,
            selection: None,
            suppress_empty_pages: true,
            page_from: None,
            page_to: None,
            active_sheet: None,
            page_ranges: None,
        }
    }
}

/// A chart rendered to SVG by the UI, for the print job (≤ 4 MB each).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ChartSvgDto {
    /// Sheet.
    pub sheet: SheetId,
    /// Chart id.
    pub id: u32,
    /// The SVG document.
    pub svg: String,
}

/// One printed page.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PrintPageDto {
    /// 0-based index in the job.
    pub index: u32,
    /// The printed page number (`&[PAGE]`, honours `first_page_number`).
    pub number: u32,
    /// Sheet.
    pub sheet: SheetId,
    /// Rows `[first, last]` on the page (repeat rows excluded).
    pub rows: [u32; 2],
    /// Columns `[first, last]`.
    pub cols: [u32; 2],
    /// Repeated rows printed above `rows`.
    pub repeat_rows: Option<[u32; 2]>,
    /// Repeated columns printed left of `cols`.
    pub repeat_cols: Option<[u32; 2]>,
    /// Scale applied, percent.
    pub scale_pct: u32,
    /// Paper `[width, height]` in millimetres (after orientation).
    pub paper_mm: [f32; 2],
    /// Where the body starts `[x, y]` in millimetres (margins, centring).
    pub origin_mm: [f32; 2],
    /// Resolved header `[left, center, right]`.
    pub header: Option<[String; 3]>,
    /// Resolved footer `[left, center, right]`.
    pub footer: Option<[String; 3]>,
}

/// Result of `print_layout`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PrintLayoutDto {
    /// Pages in print order.
    pub pages: Vec<PrintPageDto>,
    /// Problems (e.g. an unknown font).
    pub warnings: Vec<String>,
}

/// One page break for Page Break Preview.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BreakInfo {
    /// The break is above this row / left of this column.
    pub at: u32,
    /// Manual (`true`) or automatic.
    pub manual: bool,
}

/// Result of `page_breaks`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PageBreaksDto {
    /// The printed area (`None`: nothing to print).
    pub area: Option<RangeDto>,
    /// Row breaks, ascending.
    pub row_breaks: Vec<BreakInfo>,
    /// Column breaks, ascending.
    pub col_breaks: Vec<BreakInfo>,
}

/// A prepared print job (`prepare_print_job`); the engine keeps an LRU of
/// 4 jobs per process.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PrintJobInfo {
    /// Job handle for `print_job_page_png` and `print_job_pdf`.
    pub job: u32,
    /// Its layout.
    pub layout: PrintLayoutDto,
}

/// How notes are printed (`PageSetupDto::print_notes`).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum NotesPrint {
    /// Not printed.
    None,
    /// On a page at the end.
    AtEnd,
}

// ---------------------------------------------------------------------------
// Calculation settings
// ---------------------------------------------------------------------------

/// Day zero of date serial numbers.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum DateEpochDto {
    /// 1899-12-30 (the OpenOffice default).
    D1899,
    /// 1900-01-01 (StarCalc 1.0).
    D1900,
    /// 1904-01-01 (classic Mac Excel).
    D1904,
}

/// How criteria strings match text.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum CriteriaModeDto {
    /// `*`, `?` and `~` wildcards.
    Wildcards,
    /// Regular expressions (the AOO default).
    Regex,
    /// Literal comparison.
    Literal,
}

/// Text-to-number conversion in arithmetic.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum TextToNumberDto {
    /// Any text yields `#VALUE!`.
    GenerateError,
    /// Any text counts as 0.
    TreatAsZero,
    /// Plain numbers and ISO dates convert.
    ConvertUnambiguous,
    /// Convert with the document locale.
    ConvertLocale,
}

/// Iterative calculation of circular references.
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct IterationsDto {
    /// Maximum rounds (AOO default 100).
    pub steps: u32,
    /// Stop when no result changes by more than this (default 0.001).
    pub min_change: f64,
}

/// Calculation settings (Tools ▸ Options ▸ Calculate; `calc_settings`
/// and `Op::SetCalcSettings`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CalcSettingsDto {
    /// Iterations; `None` = cycles give `Err:522`.
    pub iterations: Option<IterationsDto>,
    /// Date epoch.
    pub date_epoch: DateEpochDto,
    /// Case-sensitive text comparison.
    pub case_sensitive: bool,
    /// Precision as shown.
    pub precision_as_shown: bool,
    /// Search criteria must apply to whole cells.
    pub whole_cell_match: bool,
    /// Criteria matching.
    pub criteria_mode: CriteriaModeDto,
    /// Text to number conversion.
    pub text_to_number: TextToNumberDto,
    /// First year of the two-digit year window (1583–9857; ODS
    /// `table:null-year`). `None`: the document does not set it yet (the
    /// application option applies; `SetCalcSettings` then keeps the
    /// current value).
    #[serde(default)]
    #[ts(optional = nullable)]
    pub two_digit_year: Option<u16>,
    /// "Limit decimals for general number format" (0–20); `None` =
    /// unlimited.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub standard_decimals: Option<u8>,
}

// ---------------------------------------------------------------------------
// Solver
// ---------------------------------------------------------------------------

/// What the solver optimises.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum SolverGoal {
    /// Maximise the objective.
    Maximize,
    /// Minimise the objective.
    Minimize,
    /// Reach a value.
    Value {
        /// Target.
        value: f64,
    },
}

/// Constraint operators.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum SolverOp {
    /// `<=`
    Le,
    /// `>=`
    Ge,
    /// `=`
    Eq,
    /// The cells must be integers (`rhs` ignored).
    Integer,
    /// The cells must be 0 or 1 (`rhs` ignored).
    Binary,
}

/// One constraint: every cell of `lhs` against `rhs` (a number or a
/// reference in the UI dialect).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SolverConstraint {
    /// Left side.
    pub lhs: SheetRange,
    /// Operator.
    pub op: SolverOp,
    /// Right side.
    pub rhs: String,
}

/// Solver options.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SolverOptions {
    /// Variables are ≥ 0.
    pub assume_non_negative: bool,
    /// All variables are integers.
    pub integer: bool,
    /// Tolerance for equality and integrality.
    pub epsilon: f64,
    /// Branch-and-bound node limit.
    pub max_iterations: u32,
    /// Wall-clock limit in milliseconds.
    pub timeout_ms: u32,
}

impl Default for SolverOptions {
    fn default() -> Self {
        Self {
            assume_non_negative: false,
            integer: false,
            epsilon: 1e-9,
            max_iterations: 10_000,
            timeout_ms: 10_000,
        }
    }
}

/// A solver request (linear: two-phase simplex plus branch and bound).
/// Linearity is checked by finite differences; a non-linear model is
/// `invalid_argument`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SolverRequest {
    /// The objective (formula) cell.
    pub objective: SheetCell,
    /// Goal.
    pub goal: SolverGoal,
    /// Variable cells (constants, not formulas).
    pub variables: Vec<SheetRange>,
    /// Constraints.
    pub constraints: Vec<SolverConstraint>,
    /// Options.
    pub options: SolverOptions,
}

/// Solver outcome.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum SolverStatus {
    /// An optimum was found.
    Optimal,
    /// No feasible point.
    Infeasible,
    /// The objective is unbounded.
    Unbounded,
    /// The iteration or time limit was hit (`values` is the best so far).
    Limit,
    /// The model is not linear in the variable cells (the solver is
    /// linear only; `values` are the current values) (v0.6).
    NotLinear,
}

/// One variable's solved value.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SolverValue {
    /// The cell.
    pub cell: SheetCell,
    /// Its value.
    pub value: f64,
}

/// Solver result. Nothing is changed; apply `values` with `SetCellInput`
/// if the user accepts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SolverResult {
    /// Outcome.
    pub status: SolverStatus,
    /// The objective's value at `values`.
    pub objective: f64,
    /// Variable values.
    pub values: Vec<SolverValue>,
    /// Simplex pivots / nodes used.
    pub iterations: u32,
}

// ---------------------------------------------------------------------------
// External links policy
// ---------------------------------------------------------------------------

/// Kinds of external links a file may contain. All are disabled by policy
/// (docs/security.md): DDE gives `#N/A`, external references `#REF!`,
/// linked sheets are imported as values.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ExternalLinkKind {
    /// `=DDE(...)` formulas.
    Dde,
    /// `'file:...'#$Sheet.A1` references.
    ExternalReference,
    /// A sheet linked to an external file (`table:table-source`).
    LinkedSheet,
    /// A linked (not embedded) picture.
    LinkedImage,
}

/// What the reader disabled, reported in `WorkbookInfo.external_links`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExternalLinkWarning {
    /// Kind.
    pub kind: ExternalLinkKind,
    /// How many were found.
    pub count: u32,
    /// The first target (a file name or DDE topic), for the message.
    pub detail: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn print_and_settings_shapes() {
        assert_eq!(
            serde_json::to_value(PrintSheets::Some { ids: vec![1, 3] }).unwrap(),
            json!({ "type": "some", "ids": [1, 3] })
        );
        assert_eq!(
            serde_json::to_value(PrintSheets::Active).unwrap(),
            json!({ "type": "active" })
        );
        // The scope fields are optional on the wire (older callers).
        let o: PrintOptionsDto = serde_json::from_value(json!({
            "sheets": { "type": "active" }, "selection": null,
            "suppressEmptyPages": true, "pageFrom": null, "pageTo": null
        }))
        .unwrap();
        assert_eq!(o, PrintOptionsDto::default());
        let o: PrintOptionsDto = serde_json::from_value(json!({
            "sheets": { "type": "all" }, "selection": null,
            "suppressEmptyPages": true, "pageFrom": null, "pageTo": null,
            "activeSheet": 2, "pageRanges": [[1, 3], [5, 5]]
        }))
        .unwrap();
        assert_eq!(o.active_sheet, Some(2));
        assert_eq!(o.page_ranges, Some(vec![[1, 3], [5, 5]]));
        let s = CalcSettingsDto {
            iterations: Some(IterationsDto {
                steps: 100,
                min_change: 0.001,
            }),
            date_epoch: DateEpochDto::D1904,
            case_sensitive: false,
            precision_as_shown: true,
            whole_cell_match: true,
            criteria_mode: CriteriaModeDto::Regex,
            text_to_number: TextToNumberDto::ConvertLocale,
            two_digit_year: Some(1950),
            standard_decimals: None,
        };
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["dateEpoch"], "d1904");
        assert_eq!(v["iterations"]["minChange"], 0.001);
        assert_eq!(v["textToNumber"], "convertLocale");
        assert_eq!(v["twoDigitYear"], 1950);
        // Older clients omit the new fields.
        let mut old = v.clone();
        old.as_object_mut().unwrap().remove("twoDigitYear");
        old.as_object_mut().unwrap().remove("standardDecimals");
        let old: CalcSettingsDto = serde_json::from_value(old).unwrap();
        assert_eq!((old.two_digit_year, old.standard_decimals), (None, None));
        let back: CalcSettingsDto = serde_json::from_value(v).unwrap();
        assert_eq!(back, s);
        assert_eq!(
            serde_json::to_value(SolverGoal::Value { value: 5.0 }).unwrap(),
            json!({ "type": "value", "value": 5.0 })
        );
        assert_eq!(serde_json::to_value(SolverOp::Ge).unwrap(), json!("ge"));
        assert_eq!(
            serde_json::to_value(ExternalLinkKind::ExternalReference).unwrap(),
            json!("externalReference")
        );
        let page = PrintPageDto {
            index: 0,
            number: 3,
            sheet: 1,
            rows: [0, 55],
            cols: [0, 6],
            repeat_rows: None,
            repeat_cols: None,
            scale_pct: 100,
            paper_mm: [210.0, 297.0],
            origin_mm: [20.0, 20.0],
            header: Some(["".into(), "Sheet1".into(), "".into()]),
            footer: None,
        };
        let v = serde_json::to_value(&page).unwrap();
        assert_eq!(v["paperMm"], json!([210.0, 297.0]));
        assert_eq!(v["header"][1], "Sheet1");
    }
}
