//! Data validation (Calc "Validity"): rules over ranges, conversion to
//! and from the API and the model, list resolution and the check run on
//! typed input and pastes.
//!
//! A value is checked after it is written: `Stop` alerts make the op fail
//! with [`VALIDATION`] (the op is reverted), `Warning` and `Info` keep the
//! value and report a notice in `ApplyResult.warnings`. Clearing a cell is
//! never checked, and a rule without an error alert accepts anything.

use std::sync::Arc;

use tally_api::{
    ApiError, ListSource, ValidationAlert, ValidationDto, ValidationErrorStyle, ValidationInfo,
    ValidationKind, ValidationMessage,
};
use tally_formula::{Expr, ResolvedRef};
use tally_model::{
    CellRangeLocal, CmpOpData, ListSourceData, ValidationData, ValidationErrorData,
    ValidationErrorStyleData, ValidationKindData, ValidationMessageData,
};
use tally_types::{CellAddr, CellRange, SheetId, Value, MAX_COLS, MAX_ROWS};

use crate::condfmt::{cmp_from_api, cmp_to_api, compare, operand_count, ranges_from_api};
use crate::fexpr::{number_of, truthy, FExpr};
use crate::storage::ColumnStore;
use crate::workbook::Workbook;

/// Error code of rejected input.
pub const VALIDATION: &str = "validation";
/// Most list items resolved.
pub const MAX_LIST_ITEMS: usize = 10_000;
/// Most validations per sheet.
pub const MAX_VALIDATIONS: usize = 10_000;

/// What numbers a comparison rule tests.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum NumKind {
    /// Whole numbers.
    Whole,
    /// Any number.
    Decimal,
    /// Dates (serials).
    Date,
    /// Times (day fractions).
    Time,
    /// The length of the text.
    TextLength,
}

/// List items.
#[derive(Clone, Debug, PartialEq)]
pub enum ListSrc {
    /// Literal items.
    Values(Vec<String>),
    /// A reference or expression.
    Expr(FExpr),
}

/// What a validation allows.
#[derive(Clone, Debug, PartialEq)]
pub enum VKind {
    /// Anything.
    Any,
    /// A list.
    List(ListSrc),
    /// A comparison.
    Compare {
        /// What is compared.
        what: NumKind,
        /// Operator.
        op: CmpOpData,
        /// Operands.
        operands: Vec<FExpr>,
    },
    /// A formula.
    Custom(FExpr),
}

/// A validation.
#[derive(Clone, Debug, PartialEq)]
pub struct Validation {
    /// Id, unique per sheet.
    pub id: u32,
    /// Ranges.
    pub ranges: Vec<CellRangeLocal>,
    /// Origin of the formulas' offsets (see [`crate::condfmt::CondFormat`]).
    pub origin: CellAddr,
    /// Rule.
    pub kind: VKind,
    /// Empty is valid.
    pub allow_blank: bool,
    /// Show the dropdown.
    pub show_dropdown: bool,
    /// Input help.
    pub input_help: Option<ValidationMessageData>,
    /// Error alert.
    pub error: Option<ValidationErrorData>,
}

impl Validation {
    /// The top-left cell of the first range.
    pub fn anchor(&self, sheet: SheetId) -> CellAddr {
        let r = self.ranges.first().copied().unwrap_or_default();
        CellAddr::new(sheet, r.r0, r.c0)
    }

    /// `true` if a range contains the cell.
    pub fn contains(&self, row: u32, col: u32) -> bool {
        self.ranges
            .iter()
            .any(|r| r.r0 <= row && row <= r.r1 && r.c0 <= col && col <= r.c1)
    }

    /// The formulas, mutably.
    pub fn formulas_mut(&mut self) -> Vec<&mut FExpr> {
        match &mut self.kind {
            VKind::Any | VKind::List(ListSrc::Values(_)) => Vec::new(),
            VKind::List(ListSrc::Expr(f)) | VKind::Custom(f) => vec![f],
            VKind::Compare { operands, .. } => operands.iter_mut().collect(),
        }
    }

    /// `true` for a list rule with a dropdown.
    pub fn has_dropdown(&self) -> bool {
        self.show_dropdown && matches!(self.kind, VKind::List(_))
    }
}

// ---------------------------------------------------------------------------
// API ⇄ engine ⇄ model
// ---------------------------------------------------------------------------

fn msg_from_api(m: &ValidationMessage) -> ValidationMessageData {
    ValidationMessageData {
        title: m.title.clone(),
        text: m.text.clone(),
    }
}

fn msg_to_api(m: &ValidationMessageData) -> ValidationMessage {
    ValidationMessage {
        title: m.title.clone(),
        text: m.text.clone(),
    }
}

fn alert_from_api(a: &ValidationAlert) -> ValidationErrorData {
    ValidationErrorData {
        style: match a.style {
            ValidationErrorStyle::Stop => ValidationErrorStyleData::Stop,
            ValidationErrorStyle::Warning => ValidationErrorStyleData::Warning,
            ValidationErrorStyle::Info => ValidationErrorStyleData::Info,
        },
        title: a.title.clone(),
        text: a.text.clone(),
    }
}

fn alert_to_api(a: &ValidationErrorData) -> ValidationAlert {
    ValidationAlert {
        style: match a.style {
            ValidationErrorStyleData::Stop => ValidationErrorStyle::Stop,
            ValidationErrorStyleData::Warning => ValidationErrorStyle::Warning,
            ValidationErrorStyleData::Info => ValidationErrorStyle::Info,
        },
        title: a.title.clone(),
        text: a.text.clone(),
    }
}

/// A validation from the API.
pub fn from_api(
    wb: &Workbook,
    sheet: SheetId,
    id: u32,
    v: &ValidationDto,
) -> Result<Validation, ApiError> {
    let ranges = ranges_from_api(&v.ranges)?;
    let origin = CellAddr::new(sheet, ranges[0].r0, ranges[0].c0);
    let cmp = |what: NumKind, op: tally_api::CmpOp, f: &[String]| -> Result<VKind, ApiError> {
        let op = cmp_from_api(op);
        let n = operand_count(op);
        if f.len() < n {
            return Err(ApiError::invalid_argument(format!(
                "the comparison needs {n} value(s)"
            )));
        }
        Ok(VKind::Compare {
            what,
            op,
            operands: f[..n]
                .iter()
                .map(|x| FExpr::parse_ui(wb, x, origin))
                .collect::<Result<_, _>>()?,
        })
    };
    let kind = match &v.kind {
        ValidationKind::Any => VKind::Any,
        ValidationKind::List { source } => VKind::List(match source {
            ListSource::Values { items } => {
                if items.len() > MAX_LIST_ITEMS {
                    return Err(ApiError::invalid_argument("too many list items"));
                }
                ListSrc::Values(items.clone())
            }
            ListSource::Range { reference } => {
                ListSrc::Expr(FExpr::parse_ui(wb, reference, origin)?)
            }
        }),
        ValidationKind::Whole { op, formulas } => cmp(NumKind::Whole, *op, formulas)?,
        ValidationKind::Decimal { op, formulas } => cmp(NumKind::Decimal, *op, formulas)?,
        ValidationKind::Date { op, formulas } => cmp(NumKind::Date, *op, formulas)?,
        ValidationKind::Time { op, formulas } => cmp(NumKind::Time, *op, formulas)?,
        ValidationKind::TextLength { op, formulas } => cmp(NumKind::TextLength, *op, formulas)?,
        ValidationKind::Custom { formula } => VKind::Custom(FExpr::parse_ui(wb, formula, origin)?),
    };
    Ok(Validation {
        id,
        ranges,
        origin,
        kind,
        allow_blank: v.allow_blank,
        show_dropdown: v.show_dropdown,
        input_help: v.input_help.as_ref().map(msg_from_api),
        error: v.error.as_ref().map(alert_from_api),
    })
}

fn kind_to_api(wb: &Workbook, k: &VKind, at: CellAddr) -> ValidationKind {
    match k {
        VKind::Any => ValidationKind::Any,
        VKind::List(ListSrc::Values(items)) => ValidationKind::List {
            source: ListSource::Values {
                items: items.clone(),
            },
        },
        VKind::List(ListSrc::Expr(f)) => ValidationKind::List {
            source: ListSource::Range {
                reference: f.print_ui(wb, at),
            },
        },
        VKind::Compare { what, op, operands } => {
            let op = cmp_to_api(*op);
            let formulas = operands.iter().map(|f| f.print_ui(wb, at)).collect();
            match what {
                NumKind::Whole => ValidationKind::Whole { op, formulas },
                NumKind::Decimal => ValidationKind::Decimal { op, formulas },
                NumKind::Date => ValidationKind::Date { op, formulas },
                NumKind::Time => ValidationKind::Time { op, formulas },
                NumKind::TextLength => ValidationKind::TextLength { op, formulas },
            }
        }
        VKind::Custom(f) => ValidationKind::Custom {
            formula: f.print_ui(wb, at),
        },
    }
}

/// The API view.
pub fn to_api(wb: &Workbook, sheet: SheetId, v: &Validation) -> ValidationDto {
    ValidationDto {
        id: v.id,
        ranges: crate::condfmt::ranges_to_api(&v.ranges),
        kind: kind_to_api(wb, &v.kind, v.anchor(sheet)),
        allow_blank: v.allow_blank,
        show_dropdown: v.show_dropdown,
        input_help: v.input_help.as_ref().map(msg_to_api),
        error: v.error.as_ref().map(alert_to_api),
    }
}

/// From the model (`None` when it has no valid range).
pub fn from_model(
    wb: &Workbook,
    sheet: SheetId,
    id: u32,
    d: &ValidationData,
) -> Option<Validation> {
    let ranges: Vec<CellRangeLocal> = d
        .ranges
        .iter()
        .copied()
        .filter(|r| r.r0 <= r.r1 && r.c0 <= r.c1 && r.r1 < MAX_ROWS && r.c1 < MAX_COLS)
        .take(crate::condfmt::MAX_RANGES)
        .collect();
    if ranges.is_empty() {
        return None;
    }
    let base = CellAddr::new(
        sheet,
        d.base.0.min(MAX_ROWS - 1),
        d.base.1.min(MAX_COLS - 1),
    );
    let cmp = |what: NumKind, op: CmpOpData, f: &[String]| -> Option<VKind> {
        let n = operand_count(op);
        (f.len() >= n).then(|| VKind::Compare {
            what,
            op,
            operands: f[..n]
                .iter()
                .map(|x| FExpr::parse_of(wb, x, base))
                .collect(),
        })
    };
    let kind = match &d.kind {
        ValidationKindData::Any => VKind::Any,
        ValidationKindData::List { source } => VKind::List(match source {
            ListSourceData::Values(v) => {
                ListSrc::Values(v.iter().take(MAX_LIST_ITEMS).cloned().collect())
            }
            ListSourceData::Range(f) => ListSrc::Expr(FExpr::parse_of(wb, f, base)),
        }),
        ValidationKindData::Whole { op, formulas } => cmp(NumKind::Whole, *op, formulas)?,
        ValidationKindData::Decimal { op, formulas } => cmp(NumKind::Decimal, *op, formulas)?,
        ValidationKindData::Date { op, formulas } => cmp(NumKind::Date, *op, formulas)?,
        ValidationKindData::Time { op, formulas } => cmp(NumKind::Time, *op, formulas)?,
        ValidationKindData::TextLength { op, formulas } => cmp(NumKind::TextLength, *op, formulas)?,
        ValidationKindData::Custom { formula } => VKind::Custom(FExpr::parse_of(wb, formula, base)),
    };
    Some(Validation {
        id,
        ranges,
        origin: base,
        kind,
        allow_blank: d.allow_blank,
        show_dropdown: d.show_dropdown,
        input_help: d.input_help.clone(),
        error: d.error.clone(),
    })
}

/// To the model (formulas relative to the top-left cell of the first range).
pub fn to_model(wb: &Workbook, sheet: SheetId, v: &Validation) -> ValidationData {
    let base = v.anchor(sheet);
    let kind = match &v.kind {
        VKind::Any => ValidationKindData::Any,
        VKind::List(ListSrc::Values(items)) => ValidationKindData::List {
            source: ListSourceData::Values(items.clone()),
        },
        VKind::List(ListSrc::Expr(f)) => ValidationKindData::List {
            source: ListSourceData::Range(f.print_of(wb, base)),
        },
        VKind::Compare { what, op, operands } => {
            let op = *op;
            let formulas = operands.iter().map(|f| f.print_of(wb, base)).collect();
            match what {
                NumKind::Whole => ValidationKindData::Whole { op, formulas },
                NumKind::Decimal => ValidationKindData::Decimal { op, formulas },
                NumKind::Date => ValidationKindData::Date { op, formulas },
                NumKind::Time => ValidationKindData::Time { op, formulas },
                NumKind::TextLength => ValidationKindData::TextLength { op, formulas },
            }
        }
        VKind::Custom(f) => ValidationKindData::Custom {
            formula: f.print_of(wb, base),
        },
    };
    ValidationData {
        ranges: v.ranges.clone(),
        base: (base.row, base.col),
        kind,
        allow_blank: v.allow_blank,
        show_dropdown: v.show_dropdown,
        input_help: v.input_help.clone(),
        error: v.error.clone(),
    }
}

// ---------------------------------------------------------------------------
// Lists and checks
// ---------------------------------------------------------------------------

/// The list items of `src` for the cell `at`, as values.
pub fn list_values(wb: &Workbook, src: &ListSrc, at: CellAddr) -> Vec<Value> {
    match src {
        ListSrc::Values(v) => v.iter().map(|s| Value::text(s.as_str())).collect(),
        ListSrc::Expr(f) => {
            let e = crate::names::resolve_chain(&wb.names, &f.formula.expr, at.sheet);
            if let Expr::Ref(r) = e {
                if let ResolvedRef::Range(cr) = r.resolve(at) {
                    return range_values(wb, &cr);
                }
                return Vec::new();
            }
            match f.eval(wb, at) {
                Value::Array(m) => m
                    .data
                    .iter()
                    .filter(|v| !v.is_empty())
                    .take(MAX_LIST_ITEMS)
                    .cloned()
                    .collect(),
                Value::Empty => Vec::new(),
                v => vec![v],
            }
        }
    }
}

fn range_values(wb: &Workbook, r: &CellRange) -> Vec<Value> {
    let mut out = Vec::new();
    let Some(s) = wb.sheet(r.sheet) else {
        return out;
    };
    // Column-major like Calc's list (a list range is normally one column).
    let mut cells: Vec<(u32, u32)> = Vec::new();
    for c in r.c0..=r.c1.min(s.columns.len().saturating_sub(1) as u32) {
        if let Some(col) = s.column(c) {
            for (row, _) in col.cells.iter_range(r.r0, r.r1) {
                cells.push((row, c));
                if cells.len() >= MAX_LIST_ITEMS {
                    break;
                }
            }
        }
        if cells.len() >= MAX_LIST_ITEMS || s.columns.is_empty() {
            break;
        }
    }
    cells.sort_unstable();
    for (row, c) in cells {
        let v = wb.value(CellAddr::new(r.sheet, row, c));
        if !v.is_empty() {
            out.push(v);
        }
    }
    out
}

/// The list items as display strings.
pub fn list_items(wb: &Workbook, src: &ListSrc, at: CellAddr) -> Vec<String> {
    match src {
        ListSrc::Values(v) => v.clone(),
        ListSrc::Expr(_) => list_values(wb, src, at)
            .iter()
            .map(|v| match v {
                Value::Number(n) => tally_numfmt::general_string(*n, None, wb.locale),
                Value::Text(t) => t.to_string(),
                Value::Bool(b) => if *b { "TRUE" } else { "FALSE" }.into(),
                Value::Error(e) => e.display().into_owned(),
                _ => String::new(),
            })
            .collect(),
    }
}

fn text_len(v: &Value) -> Option<f64> {
    Some(match v {
        Value::Text(t) => t.chars().count() as f64,
        Value::Number(n) => tally_numfmt::general_string(*n, None, &tally_numfmt::EN_US)
            .chars()
            .count() as f64,
        Value::Bool(b) => {
            if *b {
                4.0
            } else {
                5.0
            }
        }
        Value::Empty => 0.0,
        _ => return None,
    })
}

/// `true` if `v` (the value of `addr`) passes the rule. `fresh` evaluates
/// with dirty precedents recalculated.
pub fn is_valid(wb: &mut Workbook, val: &Validation, addr: CellAddr, v: &Value) -> bool {
    if v.is_empty() {
        return val.allow_blank;
    }
    match &val.kind {
        VKind::Any => true,
        VKind::List(src) => {
            let items = list_values(wb, src, addr);
            items.iter().any(|item| {
                compare(v, CmpOpData::Eq, item, None)
                    || match (v, item) {
                        (Value::Number(n), Value::Text(t)) => {
                            tally_numfmt::general_string(*n, None, wb.locale)
                                .eq_ignore_ascii_case(t.trim())
                        }
                        _ => false,
                    }
            })
        }
        VKind::Compare { what, op, operands } => {
            let x = match what {
                NumKind::TextLength => text_len(v),
                _ => match v {
                    Value::Number(n) => Some(*n),
                    _ => None,
                },
            };
            let Some(x) = x else {
                return false;
            };
            if *what == NumKind::Whole && x.fract() != 0.0 {
                return false;
            }
            let ops: Vec<Value> = operands.iter().map(|f| f.eval_fresh(wb, addr)).collect();
            let num = |v: &Value| number_of(v).map_or(v.clone(), Value::Number);
            match ops.first() {
                Some(a) => compare(
                    &Value::Number(x),
                    *op,
                    &num(a),
                    ops.get(1).map(num).as_ref(),
                ),
                None => true,
            }
        }
        VKind::Custom(f) => truthy(&f.eval_fresh(wb, addr)),
    }
}

/// The first validation of `sheet` covering the cell.
pub fn at(wb: &Workbook, sheet: SheetId, row: u32, col: u32) -> Option<&Validation> {
    wb.sheet(sheet)?
        .meta
        .validations
        .iter()
        .find(|v| v.contains(row, col))
}

/// Checks the values just written to `cells`. `Err` for a `Stop` alert
/// (the caller reverts the op); notices for `Warning`/`Info`.
pub fn check_cells(
    wb: &mut Workbook,
    sheet: SheetId,
    cells: &[(u32, u32)],
) -> Result<Vec<ApiError>, ApiError> {
    let mut notices = Vec::new();
    let Some(s) = wb.sheet(sheet) else {
        return Ok(notices);
    };
    if s.meta.validations.is_empty() {
        return Ok(notices);
    }
    let vals: Vec<Arc<Validation>> = s
        .meta
        .validations
        .iter()
        .filter(|v| v.error.is_some())
        .map(|v| Arc::new(v.clone()))
        .collect();
    for &(r, c) in cells {
        let Some(val) = vals.iter().find(|v| v.contains(r, c)) else {
            continue;
        };
        let addr = CellAddr::new(sheet, r, c);
        let v = current_value(wb, addr);
        if v.is_empty() || is_valid(wb, val, addr, &v) {
            continue;
        }
        let Some(err) = &val.error else { continue };
        let text = if err.text.trim().is_empty() {
            "The value you entered is not valid.".to_string()
        } else {
            err.text.clone()
        };
        let e = ApiError::new(VALIDATION, text);
        match err.style {
            ValidationErrorStyleData::Stop => return Err(e),
            _ => {
                if notices.len() < 100 {
                    notices.push(e);
                }
            }
        }
    }
    Ok(notices)
}

/// The value of a cell, evaluating a just-entered (dirty) formula without
/// committing its result.
fn current_value(wb: &mut Workbook, addr: CellAddr) -> Value {
    let dirty = wb
        .formula_at(addr)
        .filter(|(_, f)| f.state != crate::formula_cells::CellState::Clean && f.array.is_none())
        .map(|(_, f)| FExpr {
            formula: f.formula.clone(),
            invalid: f.invalid_text.clone(),
        });
    match dirty {
        Some(f) => f.eval_fresh(wb, addr),
        None => wb.value(addr),
    }
}

/// The editor view of the validation at a cell.
pub fn info_at(wb: &Workbook, sheet: SheetId, row: u32, col: u32) -> Option<ValidationInfo> {
    let v = at(wb, sheet, row, col)?;
    let addr = CellAddr::new(sheet, row, col);
    Some(ValidationInfo {
        id: v.id,
        list_items: match &v.kind {
            VKind::List(src) => list_items(wb, src, addr),
            _ => Vec::new(),
        },
        show_dropdown: v.has_dropdown(),
        input_help: v.input_help.as_ref().map(msg_to_api),
        error: v.error.as_ref().map(alert_to_api),
        kind: kind_to_api(wb, &v.kind, v.anchor(sheet)),
    })
}
