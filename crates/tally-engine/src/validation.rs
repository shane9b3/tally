//! Data validation (Calc "Validity"): rules over ranges, conversion to
//! and from the API and the model, list resolution and the check run on
//! typed input and pastes.
//!
//! A value is checked after it is written: `Stop` alerts make the op fail
//! with [`VALIDATION`] (the op is reverted), `Warning` and `Info` keep the
//! value and report a notice in `ApplyResult.warnings`. Clearing a cell is
//! never checked, and a rule without an error alert accepts anything.

use std::collections::{HashMap, HashSet};
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
use crate::rangeindex::RangeIndex;
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
    let ranges = ranges_from_api(&v.ranges, crate::condfmt::MAX_RANGES)?;
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

/// Most cells one op's validation check evaluates; the rest are accepted
/// with a notice (security review 3, HIGH-4).
pub const MAX_VALIDATION_CELLS_PER_OP: u64 = 2_000_000;
/// Work budget of one op's validation check (element operations of the
/// rule formulas plus a fixed charge per cell).
pub const MAX_VALIDATION_WORK_PER_OP: u64 = 50_000_000;
/// Fixed charge per checked cell.
const CELL_COST: u64 = 16;

/// A list source resolved once: the keys a value may match.
#[derive(Debug, Default)]
struct ListSet {
    /// Numbers (and booleans as 0/1) by bit pattern, `-0` as `0`.
    nums: HashSet<u64>,
    /// Lowercased text items (text values compare case-insensitively).
    texts: HashSet<String>,
    /// Lowercased, trimmed text items (a number matches its general text).
    trimmed: HashSet<String>,
}

fn num_bits(n: f64) -> u64 {
    if n == 0.0 {
        0
    } else {
        n.to_bits()
    }
}

impl ListSet {
    fn from_values(items: &[Value]) -> Self {
        let mut set = ListSet::default();
        for item in items {
            let item = match item {
                Value::Array(m) => match m.get(0, 0) {
                    Some(v) => v,
                    None => continue,
                },
                v => v,
            };
            match item {
                Value::Number(n) => {
                    set.nums.insert(num_bits(*n));
                }
                Value::Bool(b) => {
                    set.nums.insert(num_bits(f64::from(u8::from(*b))));
                }
                Value::Text(t) => {
                    set.texts.insert(t.to_lowercase());
                    set.trimmed.insert(t.trim().to_lowercase());
                }
                Value::Empty | Value::Error(_) | Value::Array(_) => {}
            }
        }
        set
    }

    /// Mirrors `compare(v, Eq, item)` plus the number-as-text match of
    /// the single-cell check.
    fn contains(&self, v: &Value, locale: &tally_numfmt::Locale) -> bool {
        match v {
            Value::Number(n) => {
                self.nums.contains(&num_bits(*n))
                    || (!self.trimmed.is_empty()
                        && self.trimmed.contains(
                            &tally_numfmt::general_string(*n, None, locale).to_lowercase(),
                        ))
            }
            Value::Bool(b) => self.nums.contains(&num_bits(f64::from(u8::from(*b)))),
            Value::Text(t) => self.texts.contains(&t.to_lowercase()),
            Value::Empty => self.texts.contains(""),
            Value::Array(m) => m.get(0, 0).is_some_and(|x| self.contains(x, locale)),
            Value::Error(_) => false,
        }
    }
}

/// What a shared list set is keyed by: the resolved source range (so
/// validations with the same absolute source share one set) or, for
/// literal lists, the validation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum ListKey {
    Validation(u32),
    Range(CellRange),
}

/// The per-op state of a validation check: list sets and folded formulas
/// resolved once per validation, and the budget.
pub struct Checker {
    lists: HashMap<ListKey, Arc<ListSet>>,
    folded: HashMap<usize, Arc<tally_formula::Formula>>,
    cells: u64,
    work: u64,
    work_budget: u64,
    exhausted: bool,
}

impl Default for Checker {
    fn default() -> Self {
        Self::new()
    }
}

impl Checker {
    /// A fresh checker (one per op).
    pub fn new() -> Self {
        Self::with_budget(MAX_VALIDATION_WORK_PER_OP)
    }

    /// A checker with a custom work budget (tests).
    pub fn with_budget(work_budget: u64) -> Self {
        Self {
            lists: HashMap::new(),
            folded: HashMap::new(),
            cells: 0,
            work: 0,
            work_budget,
            exhausted: false,
        }
    }

    /// `true` once the budget ran out (later cells are accepted).
    pub fn exhausted(&self) -> bool {
        self.exhausted
    }

    /// Cells checked so far.
    pub fn cells_checked(&self) -> u64 {
        self.cells
    }

    fn charge(&mut self, n: u64) -> bool {
        self.work = self.work.saturating_add(n);
        if self.work > self.work_budget || self.cells > MAX_VALIDATION_CELLS_PER_OP {
            self.exhausted = true;
        }
        !self.exhausted
    }

    /// `f` with its absolute parts folded (once per formula per op), then
    /// evaluated freshly at `addr`; `None` when the budget is spent.
    fn eval(&mut self, wb: &mut Workbook, f: &FExpr, addr: CellAddr) -> Option<Value> {
        if f.invalid.is_some() {
            return Some(Value::Error(tally_types::ErrorKind::Name));
        }
        let key = Arc::as_ptr(&f.formula) as usize;
        let formula = match self.folded.get(&key) {
            Some(x) => x.clone(),
            None => {
                let folded = f
                    .fold_absolute_fresh(wb, addr)
                    .unwrap_or_else(|| f.formula.clone());
                self.charge(CELL_COST);
                self.folded.insert(key, folded.clone());
                folded
            }
        };
        let (v, work) = crate::fexpr::eval_formula_fresh_counted(wb, &formula, addr);
        self.charge(work);
        Some(v)
    }

    /// The list set of `val` (shared across cells when the source does
    /// not depend on the cell).
    fn list(
        &mut self,
        wb: &mut Workbook,
        val: &Validation,
        src: &ListSrc,
        addr: CellAddr,
    ) -> Arc<ListSet> {
        let key = match src {
            ListSrc::Values(_) => Some(ListKey::Validation(val.id)),
            ListSrc::Expr(f) if crate::fexpr::is_absolute(f) => {
                let e = crate::names::resolve_chain(&wb.names, &f.formula.expr, addr.sheet);
                match e {
                    Expr::Ref(r) => match r.resolve(addr) {
                        ResolvedRef::Range(cr) => Some(ListKey::Range(cr)),
                        _ => Some(ListKey::Validation(val.id)),
                    },
                    _ => Some(ListKey::Validation(val.id)),
                }
            }
            ListSrc::Expr(_) => None,
        };
        if let Some(k) = key {
            if let Some(s) = self.lists.get(&k) {
                return s.clone();
            }
        }
        let items = list_values(wb, src, addr);
        self.charge(items.len() as u64);
        let set = Arc::new(ListSet::from_values(&items));
        if let Some(k) = key {
            self.lists.insert(k, set.clone());
        }
        set
    }

    /// `true` if `v` (the value of `addr`) passes the rule, or the budget
    /// is spent (the value is accepted then).
    pub fn is_valid(
        &mut self,
        wb: &mut Workbook,
        val: &Validation,
        addr: CellAddr,
        v: &Value,
    ) -> bool {
        if v.is_empty() {
            return val.allow_blank;
        }
        if self.exhausted {
            return true;
        }
        self.cells += 1;
        if !self.charge(CELL_COST) {
            return true;
        }
        match &val.kind {
            VKind::Any => true,
            VKind::List(src) => {
                let set = self.list(wb, val, src, addr);
                set.contains(v, wb.locale)
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
                let mut ops: Vec<Value> = Vec::with_capacity(operands.len());
                for f in operands {
                    match self.eval(wb, f, addr) {
                        Some(v) => ops.push(v),
                        None => return true,
                    }
                }
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
            VKind::Custom(f) => match self.eval(wb, f, addr) {
                Some(v) => truthy(&v),
                None => true,
            },
        }
    }
}

/// `true` if `v` (the value of `addr`) passes the rule, evaluating with
/// dirty precedents recalculated (one-off check; `check_cells` shares the
/// work across cells).
pub fn is_valid(wb: &mut Workbook, val: &Validation, addr: CellAddr, v: &Value) -> bool {
    Checker::new().is_valid(wb, val, addr, v)
}

/// The first validation of `sheet` covering the cell.
pub fn at(wb: &Workbook, sheet: SheetId, row: u32, col: u32) -> Option<&Validation> {
    wb.sheet(sheet)?
        .meta
        .validations
        .iter()
        .find(|v| v.contains(row, col))
        .map(|v| &**v)
}

/// Checks the values just written to `cells`. `Err` for a `Stop` alert
/// (the caller reverts the op); notices for `Warning`/`Info`, and one
/// notice when the per-op budget ran out and the remaining cells were
/// accepted unchecked.
pub fn check_cells(
    wb: &mut Workbook,
    sheet: SheetId,
    cells: &[(u32, u32)],
) -> Result<Vec<ApiError>, ApiError> {
    check_cells_with_budget(wb, sheet, cells, MAX_VALIDATION_WORK_PER_OP)
}

/// [`check_cells`] with a custom work budget (tests).
pub fn check_cells_with_budget(
    wb: &mut Workbook,
    sheet: SheetId,
    cells: &[(u32, u32)],
    work_budget: u64,
) -> Result<Vec<ApiError>, ApiError> {
    let mut notices = Vec::new();
    let Some(s) = wb.sheet(sheet) else {
        return Ok(notices);
    };
    if s.meta.validations.is_empty() || cells.is_empty() {
        return Ok(notices);
    }
    // The collection is shared, not copied (Arc); the index maps each
    // cell to the first validation with an alert covering it.
    let vals: Arc<Vec<Arc<Validation>>> = s.meta.validations.clone();
    let with_alert: Vec<u32> = vals
        .iter()
        .enumerate()
        .filter(|(_, v)| v.error.is_some())
        .map(|(i, _)| i as u32)
        .collect();
    if with_alert.is_empty() {
        return Ok(notices);
    }
    let index = (cells.len() as u64 * with_alert.len() as u64 > 4096).then(|| {
        RangeIndex::build(
            with_alert
                .iter()
                .flat_map(|&i| vals[i as usize].ranges.iter().map(move |r| (*r, i))),
        )
    });
    let mut checker = Checker::with_budget(work_budget);
    let mut hits: Vec<u32> = Vec::new();
    let mut skipped = 0u64;
    for &(r, c) in cells {
        let pos = match &index {
            Some(idx) => {
                idx.covering_sorted(r, c, &mut hits);
                hits.first().copied()
            }
            None => with_alert
                .iter()
                .copied()
                .find(|&i| vals[i as usize].contains(r, c)),
        };
        let Some(pos) = pos else {
            continue;
        };
        let val = &vals[pos as usize];
        let addr = CellAddr::new(sheet, r, c);
        let v = current_value(wb, addr);
        if v.is_empty() {
            continue;
        }
        if checker.exhausted() {
            skipped += 1;
            continue;
        }
        if checker.is_valid(wb, val, addr, &v) {
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
    if skipped > 0 {
        notices.push(ApiError::new(
            VALIDATION,
            format!(
                "Validation checked {} cells and stopped; {skipped} more cells were \
                 accepted without checking (the rules are too costly for one step).",
                checker.cells_checked()
            ),
        ));
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
