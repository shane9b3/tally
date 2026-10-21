//! Data tools and editing assists (wave 5, W5-2 / W5-3): the standard and
//! advanced filter, Subtotals, Consolidate, Text to Columns, Remove
//! Duplicates, Insert/Delete Cells with shift, Fill Series, Delete
//! Contents, sheet extras, names extras, database ranges, view options,
//! split, custom sort lists and the shared aggregation used by the
//! DataPilot, Subtotals and Consolidate.
//!
//! AutoInput lives in [`crate::autocomplete`], the Navigator and the
//! document statistics in [`crate::navigator`], AutoFormat in
//! [`crate::cellstyles`].
//!
//! TODO(W5-3): `precedents`/`dependents` from `DepIndex`, `solver`,
//! `set_calc_settings`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use tally_api::{
    AggFn, ApiError, CsvColumnType, CsvImportOptionsDto, DatabaseRangeDto, DateUnit,
    DeleteContentsFlags, Direction, NameLabelFlags, Op, RangeDto, SeriesKind, SheetCell,
    SheetRange, SplitDto, StandardFilterDto, StdCondition, StdConnector, StdFilterOp,
    StdFilterOptions, SubtotalGroup, SubtotalOptions, ViewOptionsDto,
};
use tally_model::{
    AggFnData, CellRangeLocal, DatabaseRangeData, StandardFilterData, StdConditionData,
    StdConnectorData, StdFilterOpData, StdFilterOptionsData, SubtotalGroupData, SubtotalsData,
};
use tally_numfmt::date::{days_in_month, serial_from_ymd, weekday, ymd_from_serial};
use tally_numfmt::RecognizeContext;
use tally_types::{CellAddr, CellRange, ErrorKind, SheetId, Value, MAX_COLS, MAX_ROWS};

use crate::ops::{check_sheet, map_styles, to_range, MAX_OP_CELLS};
use crate::ops_v03::with_meta;
use crate::protection::{check_edit, check_format, check_sheet_action, check_structure};
use crate::service::view::display_text;
use crate::storage::{CellContent, ColumnStore, MetaKind, Spans, StyleId};
use crate::structure::{shift_cells, Band};
use crate::undo::{capture_range, clear_range_content, Change, Effects};
use crate::workbook::{SnapContent, Workbook};

/// Most conditions of a standard filter (AOO's dialog has 4 rows).
pub const MAX_STD_CONDITIONS: usize = 4;
/// Most subtotal groups (AOO's dialog has 3 tabs).
pub const MAX_SUBTOTAL_GROUPS: usize = 3;
/// Most AutoInput candidates returned.
pub const MAX_AUTOCOMPLETE_ITEMS: usize = 1_000;
/// Most `table_op` sandbox evaluations per recalc.
pub const MAX_TABLE_OPS: u32 = 10_000;
/// Most database ranges per document.
pub const MAX_DATABASE_RANGES: usize = 10_000;
/// Most custom sort lists, and most entries per list.
pub const MAX_SORT_LISTS: usize = 1_000;
/// Most rows a filter, Subtotals or Remove Duplicates scans.
pub const MAX_TOOL_ROWS: u64 = MAX_OP_CELLS;

/// AOO's AutoFormat presets, in dialog order.
pub const AUTOFORMAT_PRESETS: [&str; 17] = [
    "Default",
    "3D",
    "Black 1",
    "Black 2",
    "Blue",
    "Brown",
    "Currency",
    "Currency 3D",
    "Currency Gray",
    "Currency Lavender",
    "Currency Turquoise",
    "Gray",
    "Green",
    "Lavender",
    "Red",
    "Turquoise",
    "Yellow",
];

// ---------------------------------------------------------------------------
// Aggregation (shared by the DataPilot, Subtotals and Consolidate)
// ---------------------------------------------------------------------------

/// API → model aggregation.
pub fn agg_to_model(f: AggFn) -> AggFnData {
    match f {
        AggFn::Sum => AggFnData::Sum,
        AggFn::Count => AggFnData::Count,
        AggFn::Average => AggFnData::Average,
        AggFn::Max => AggFnData::Max,
        AggFn::Min => AggFnData::Min,
        AggFn::Product => AggFnData::Product,
        AggFn::CountNums => AggFnData::CountNums,
        AggFn::StDev => AggFnData::StDev,
        AggFn::StDevP => AggFnData::StDevP,
        AggFn::Var => AggFnData::Var,
        AggFn::VarP => AggFnData::VarP,
    }
}

/// Model → API aggregation.
pub fn agg_to_api(f: AggFnData) -> AggFn {
    match f {
        AggFnData::Sum => AggFn::Sum,
        AggFnData::Count => AggFn::Count,
        AggFnData::Average => AggFn::Average,
        AggFnData::Max => AggFn::Max,
        AggFnData::Min => AggFn::Min,
        AggFnData::Product => AggFn::Product,
        AggFnData::CountNums => AggFn::CountNums,
        AggFnData::StDev => AggFn::StDev,
        AggFnData::StDevP => AggFn::StDevP,
        AggFnData::Var => AggFn::Var,
        AggFnData::VarP => AggFn::VarP,
    }
}

/// The caption AOO uses for an aggregation (`Sum - Revenue`, `Apple Sum`).
pub fn agg_caption(f: AggFnData) -> &'static str {
    match f {
        AggFnData::Sum => "Sum",
        AggFnData::Count => "Count",
        AggFnData::Average => "Average",
        AggFnData::Max => "Max",
        AggFnData::Min => "Min",
        AggFnData::Product => "Product",
        AggFnData::CountNums => "Count (Numbers only)",
        AggFnData::StDev => "StDev (Sample)",
        AggFnData::StDevP => "StDevP (Population)",
        AggFnData::Var => "Var (Sample)",
        AggFnData::VarP => "VarP (Population)",
    }
}

/// The `SUBTOTAL` function number of an aggregation.
pub fn subtotal_code(f: AggFnData) -> u32 {
    match f {
        AggFnData::Average => 1,
        AggFnData::CountNums => 2,
        AggFnData::Count => 3,
        AggFnData::Max => 4,
        AggFnData::Min => 5,
        AggFnData::Product => 6,
        AggFnData::StDev => 7,
        AggFnData::StDevP => 8,
        AggFnData::Sum => 9,
        AggFnData::Var => 10,
        AggFnData::VarP => 11,
    }
}

/// The spreadsheet function of an aggregation (Consolidate with links).
pub fn agg_function_name(f: AggFnData) -> &'static str {
    match f {
        AggFnData::Sum => "SUM",
        AggFnData::Count => "COUNTA",
        AggFnData::Average => "AVERAGE",
        AggFnData::Max => "MAX",
        AggFnData::Min => "MIN",
        AggFnData::Product => "PRODUCT",
        AggFnData::CountNums => "COUNT",
        AggFnData::StDev => "STDEV",
        AggFnData::StDevP => "STDEVP",
        AggFnData::Var => "VAR",
        AggFnData::VarP => "VARP",
    }
}

/// A running aggregation with AOO semantics: Count = non-empty values,
/// CountNums = numbers; Average of nothing and StDev/Var with fewer than
/// two numbers give `#DIV/0!`; an error value makes the result that
/// error.
#[derive(Clone, Debug, Default)]
pub struct Agg {
    count: u64,
    nums: u64,
    sum: f64,
    sum_sq: f64,
    product: f64,
    max: Option<f64>,
    min: Option<f64>,
    error: Option<ErrorKind>,
    // Welford's running mean and M2 for stable variances.
    mean: f64,
    m2: f64,
}

impl Agg {
    /// Adds a value.
    pub fn push(&mut self, v: &Value) {
        match v {
            Value::Empty => {}
            Value::Error(e) => {
                self.count += 1;
                self.error.get_or_insert(*e);
            }
            Value::Number(n) => {
                self.count += 1;
                self.push_num(*n);
            }
            Value::Bool(_) | Value::Text(_) | Value::Array(_) => self.count += 1,
        }
    }

    fn push_num(&mut self, n: f64) {
        self.nums += 1;
        self.sum += n;
        self.sum_sq += n * n;
        self.product = if self.nums == 1 { n } else { self.product * n };
        self.max = Some(self.max.map_or(n, |m| m.max(n)));
        self.min = Some(self.min.map_or(n, |m| m.min(n)));
        let delta = n - self.mean;
        self.mean += delta / self.nums as f64;
        self.m2 += delta * (n - self.mean);
    }

    /// `true` if nothing was added.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// The result for `f` (`Value::Empty` when nothing was added and the
    /// function is not a count).
    pub fn result(&self, f: AggFnData) -> Value {
        if let Some(e) = self.error {
            return Value::Error(e);
        }
        let n = self.nums as f64;
        let div0 = Value::Error(ErrorKind::Div0);
        match f {
            AggFnData::Count => Value::Number(self.count as f64),
            AggFnData::CountNums => Value::Number(n),
            _ if self.count == 0 => Value::Empty,
            AggFnData::Sum => Value::Number(self.sum),
            AggFnData::Average => {
                if self.nums == 0 {
                    div0
                } else {
                    Value::Number(self.sum / n)
                }
            }
            AggFnData::Max => self.max.map_or(Value::Number(0.0), Value::Number),
            AggFnData::Min => self.min.map_or(Value::Number(0.0), Value::Number),
            AggFnData::Product => Value::Number(if self.nums == 0 { 0.0 } else { self.product }),
            AggFnData::StDev => {
                if self.nums < 2 {
                    div0
                } else {
                    Value::Number((self.m2 / (n - 1.0)).max(0.0).sqrt())
                }
            }
            AggFnData::StDevP => {
                if self.nums < 1 {
                    div0
                } else {
                    Value::Number((self.m2 / n).max(0.0).sqrt())
                }
            }
            AggFnData::Var => {
                if self.nums < 2 {
                    div0
                } else {
                    Value::Number(self.m2 / (n - 1.0))
                }
            }
            AggFnData::VarP => {
                if self.nums < 1 {
                    div0
                } else {
                    Value::Number(self.m2 / n)
                }
            }
        }
    }
}

/// Aggregates `values` with `f`.
pub fn aggregate(f: AggFnData, values: impl IntoIterator<Item = Value>) -> Value {
    let mut a = Agg::default();
    for v in values {
        a.push(&v);
    }
    a.result(f)
}

// ---------------------------------------------------------------------------
// Standard and advanced filter
// ---------------------------------------------------------------------------

fn std_op_to_model(op: StdFilterOp) -> StdFilterOpData {
    match op {
        StdFilterOp::Eq => StdFilterOpData::Eq,
        StdFilterOp::Ne => StdFilterOpData::Ne,
        StdFilterOp::Lt => StdFilterOpData::Lt,
        StdFilterOp::Gt => StdFilterOpData::Gt,
        StdFilterOp::Le => StdFilterOpData::Le,
        StdFilterOp::Ge => StdFilterOpData::Ge,
        StdFilterOp::Largest => StdFilterOpData::Largest,
        StdFilterOp::Smallest => StdFilterOpData::Smallest,
        StdFilterOp::LargestPct => StdFilterOpData::LargestPct,
        StdFilterOp::SmallestPct => StdFilterOpData::SmallestPct,
        StdFilterOp::Contains => StdFilterOpData::Contains,
        StdFilterOp::NotContains => StdFilterOpData::NotContains,
        StdFilterOp::BeginsWith => StdFilterOpData::BeginsWith,
        StdFilterOp::NotBeginsWith => StdFilterOpData::NotBeginsWith,
        StdFilterOp::EndsWith => StdFilterOpData::EndsWith,
        StdFilterOp::NotEndsWith => StdFilterOpData::NotEndsWith,
    }
}

fn std_op_to_api(op: StdFilterOpData) -> StdFilterOp {
    match op {
        StdFilterOpData::Eq => StdFilterOp::Eq,
        StdFilterOpData::Ne => StdFilterOp::Ne,
        StdFilterOpData::Lt => StdFilterOp::Lt,
        StdFilterOpData::Gt => StdFilterOp::Gt,
        StdFilterOpData::Le => StdFilterOp::Le,
        StdFilterOpData::Ge => StdFilterOp::Ge,
        StdFilterOpData::Largest => StdFilterOp::Largest,
        StdFilterOpData::Smallest => StdFilterOp::Smallest,
        StdFilterOpData::LargestPct => StdFilterOp::LargestPct,
        StdFilterOpData::SmallestPct => StdFilterOp::SmallestPct,
        StdFilterOpData::Contains => StdFilterOp::Contains,
        StdFilterOpData::NotContains => StdFilterOp::NotContains,
        StdFilterOpData::BeginsWith => StdFilterOp::BeginsWith,
        StdFilterOpData::NotBeginsWith => StdFilterOp::NotBeginsWith,
        StdFilterOpData::EndsWith => StdFilterOp::EndsWith,
        StdFilterOpData::NotEndsWith => StdFilterOp::NotEndsWith,
    }
}

/// API filter → model.
pub fn std_filter_to_model(
    range: CellRangeLocal,
    conditions: &[StdCondition],
    options: &StdFilterOptions,
    criteria: Option<(u32, CellRangeLocal)>,
    copy_to_tab: Option<(u32, u32, u32)>,
) -> StandardFilterData {
    StandardFilterData {
        range,
        conditions: conditions
            .iter()
            .map(|c| StdConditionData {
                connector: match c.connector {
                    StdConnector::And => StdConnectorData::And,
                    StdConnector::Or => StdConnectorData::Or,
                },
                col: c.col,
                op: std_op_to_model(c.op),
                value: c.value.clone(),
            })
            .collect(),
        options: StdFilterOptionsData {
            case_sensitive: options.case_sensitive,
            regex: options.regex,
            has_header: options.has_header,
            no_duplicates: options.no_duplicates,
            copy_to: copy_to_tab,
            keep_criteria: options.keep_criteria,
        },
        criteria,
    }
}

/// Model filter → API.
pub fn std_filter_to_api(wb: &Workbook, f: &StandardFilterData) -> StandardFilterDto {
    let sheet_of = |tab: u32| wb.sheets.get(tab as usize).map(|s| s.id);
    StandardFilterDto {
        range: range_dto(&f.range),
        conditions: f
            .conditions
            .iter()
            .map(|c| StdCondition {
                connector: match c.connector {
                    StdConnectorData::And => StdConnector::And,
                    StdConnectorData::Or => StdConnector::Or,
                },
                col: c.col,
                op: std_op_to_api(c.op),
                value: c.value.clone(),
            })
            .collect(),
        options: StdFilterOptions {
            case_sensitive: f.options.case_sensitive,
            regex: f.options.regex,
            has_header: f.options.has_header,
            no_duplicates: f.options.no_duplicates,
            copy_to: f.options.copy_to.and_then(|(t, r, c)| {
                Some(SheetCell {
                    sheet: sheet_of(t)?,
                    r,
                    c,
                })
            }),
            keep_criteria: f.options.keep_criteria,
        },
        criteria: f.criteria.and_then(|(t, r)| {
            Some(SheetRange {
                sheet: sheet_of(t)?,
                range: range_dto(&r),
            })
        }),
    }
}

fn range_dto(r: &CellRangeLocal) -> RangeDto {
    RangeDto {
        r0: r.r0,
        c0: r.c0,
        r1: r.r1,
        c1: r.c1,
    }
}

fn local(r: &CellRange) -> CellRangeLocal {
    CellRangeLocal {
        r0: r.r0,
        c0: r.c0,
        r1: r.r1,
        c1: r.c1,
    }
}

/// One prepared condition.
enum Test {
    Cmp {
        op: StdFilterOpData,
        number: Option<f64>,
        text: String,
    },
    Regex {
        op: StdFilterOpData,
        re: Box<regex::Regex>,
    },
    /// Rows whose value is in the set (top/bottom N, computed up front).
    TopSet(HashSet<u32>),
}

fn fold(s: &str, case_sensitive: bool) -> String {
    if case_sensitive {
        s.to_string()
    } else {
        s.to_lowercase()
    }
}

/// Builds the regex of a condition: `=` and `<>` match the whole cell,
/// the text conditions search in it (`BeginsWith` anchors at the start).
fn build_regex(op: StdFilterOpData, pat: &str, case_sensitive: bool) -> Option<regex::Regex> {
    let body = match op {
        StdFilterOpData::Eq | StdFilterOpData::Ne => format!("^(?:{pat})$"),
        StdFilterOpData::BeginsWith | StdFilterOpData::NotBeginsWith => format!("^(?:{pat})"),
        StdFilterOpData::EndsWith | StdFilterOpData::NotEndsWith => format!("(?:{pat})$"),
        _ => pat.to_string(),
    };
    regex::RegexBuilder::new(&body)
        .case_insensitive(!case_sensitive)
        .size_limit(1 << 20)
        .dfa_size_limit(1 << 20)
        .build()
        .ok()
}

fn prepare_tests(
    wb: &Workbook,
    sheet: SheetId,
    rows: std::ops::RangeInclusive<u32>,
    f: &StandardFilterData,
) -> Vec<(StdConnectorData, Test)> {
    let cs = f.options.case_sensitive;
    let ctx = RecognizeContext::now(wb.settings.date_epoch);
    f.conditions
        .iter()
        .map(|c| {
            let test = match c.op {
                StdFilterOpData::Largest
                | StdFilterOpData::Smallest
                | StdFilterOpData::LargestPct
                | StdFilterOpData::SmallestPct => {
                    let n = c.value.trim().parse::<f64>().unwrap_or(0.0).max(0.0);
                    let mut vals: Vec<(f64, u32)> = rows
                        .clone()
                        .filter_map(|r| match wb.value(CellAddr::new(sheet, r, c.col)) {
                            Value::Number(x) => Some((x, r)),
                            _ => None,
                        })
                        .collect();
                    let largest =
                        matches!(c.op, StdFilterOpData::Largest | StdFilterOpData::LargestPct);
                    vals.sort_by(|a, b| {
                        let o = a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal);
                        if largest {
                            o.reverse()
                        } else {
                            o
                        }
                    });
                    let take = if matches!(
                        c.op,
                        StdFilterOpData::LargestPct | StdFilterOpData::SmallestPct
                    ) {
                        ((vals.len() as f64) * n / 100.0).round() as usize
                    } else {
                        n as usize
                    };
                    Test::TopSet(vals.iter().take(take).map(|(_, r)| *r).collect())
                }
                op if f.options.regex
                    && !matches!(
                        op,
                        StdFilterOpData::Lt
                            | StdFilterOpData::Gt
                            | StdFilterOpData::Le
                            | StdFilterOpData::Ge
                    ) =>
                {
                    match build_regex(op, &c.value, cs) {
                        Some(re) => Test::Regex {
                            op,
                            re: Box::new(re),
                        },
                        None => Test::Cmp {
                            op,
                            number: None,
                            text: fold(&c.value, cs),
                        },
                    }
                }
                op => {
                    let rec = tally_numfmt::recognize_input_with(&c.value, wb.locale, &ctx);
                    let number = match rec.value {
                        Value::Number(n) => Some(n),
                        Value::Bool(b) => Some(f64::from(u8::from(b))),
                        _ => None,
                    };
                    Test::Cmp {
                        op,
                        number,
                        text: fold(&c.value, cs),
                    }
                }
            };
            (c.connector, test)
        })
        .collect()
}

fn test_passes(t: &Test, row: u32, v: &Value, text: &str, cs: bool) -> bool {
    use std::cmp::Ordering;
    match t {
        Test::TopSet(set) => set.contains(&row),
        Test::Regex { op, re } => {
            let hit = re.is_match(text);
            match op {
                StdFilterOpData::Ne
                | StdFilterOpData::NotContains
                | StdFilterOpData::NotBeginsWith
                | StdFilterOpData::NotEndsWith => !hit,
                _ => hit,
            }
        }
        Test::Cmp {
            op,
            number,
            text: o,
        } => {
            let t = fold(text, cs);
            match op {
                StdFilterOpData::Contains => return t.contains(o.as_str()),
                StdFilterOpData::NotContains => return !t.contains(o.as_str()),
                StdFilterOpData::BeginsWith => return t.starts_with(o.as_str()),
                StdFilterOpData::NotBeginsWith => return !t.starts_with(o.as_str()),
                StdFilterOpData::EndsWith => return t.ends_with(o.as_str()),
                StdFilterOpData::NotEndsWith => return !t.ends_with(o.as_str()),
                _ => {}
            }
            let num = match v {
                Value::Number(n) => Some(*n),
                Value::Bool(b) => Some(f64::from(u8::from(*b))),
                _ => None,
            };
            let empty = v.is_empty();
            let ord = match (num, number) {
                (Some(a), Some(b)) => {
                    if tally_types::approx_equal(a, *b) {
                        Ordering::Equal
                    } else {
                        a.partial_cmp(b).unwrap_or(Ordering::Equal)
                    }
                }
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) if !empty => Ordering::Greater,
                _ => t.cmp(o),
            };
            match op {
                StdFilterOpData::Eq => ord == Ordering::Equal,
                StdFilterOpData::Ne => ord != Ordering::Equal,
                StdFilterOpData::Gt => ord == Ordering::Greater,
                StdFilterOpData::Ge => ord != Ordering::Less,
                StdFilterOpData::Lt => ord == Ordering::Less,
                StdFilterOpData::Le => ord != Ordering::Greater,
                _ => true,
            }
        }
    }
}

/// The data rows of a filter that stay visible.
fn visible_rows(wb: &Workbook, sheet: SheetId, f: &StandardFilterData) -> Vec<u32> {
    let r0 = f.range.r0 + u32::from(f.options.has_header);
    if r0 > f.range.r1 {
        return Vec::new();
    }
    let rows = r0..=f.range.r1;
    let tests = prepare_tests(wb, sheet, rows.clone(), f);
    let cs = f.options.case_sensitive;
    let criteria = f
        .criteria
        .and_then(|(tab, r)| Some((wb.sheets.get(tab as usize)?.id, r)))
        .map(|(csheet, r)| advanced_criteria(wb, sheet, csheet, &r, f));
    let mut out = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for row in rows {
        // AND binds tighter than OR: OR starts a new group.
        let mut any = false;
        let mut group = true;
        let mut first = true;
        for (i, c) in f.conditions.iter().enumerate() {
            let (_, test) = &tests[i];
            let addr = CellAddr::new(sheet, row, c.col);
            let v = wb.value(addr);
            let text = display_text(wb, addr);
            let ok = test_passes(test, row, &v, &text, cs);
            if first || c.connector == StdConnectorData::And {
                group &= ok;
            } else {
                any |= group;
                group = ok;
            }
            first = false;
        }
        let mut keep = if f.conditions.is_empty() {
            true
        } else {
            any || group
        };
        if let Some(crit) = &criteria {
            keep &= crit.iter().any(|and_row| {
                and_row
                    .iter()
                    .all(|(col, cr)| cr.matches(&wb.value(CellAddr::new(sheet, row, *col))))
            });
        }
        if keep && f.options.no_duplicates {
            let key: Vec<String> = (f.range.c0..=f.range.c1)
                .map(|c| display_text(wb, CellAddr::new(sheet, row, c)).to_lowercase())
                .collect();
            if !seen.insert(key.join("\u{1}")) {
                keep = false;
            }
        }
        if keep {
            out.push(row);
        }
    }
    out
}

/// The criteria block of an advanced filter as OR rows of AND
/// `(column, criterion)` pairs (D-function rules: the first row holds the
/// labels, matched against the range's header row).
fn advanced_criteria(
    wb: &Workbook,
    sheet: SheetId,
    csheet: SheetId,
    crit: &CellRangeLocal,
    f: &StandardFilterData,
) -> Vec<Vec<(u32, tally_functions::criteria::Criterion)>> {
    let mut headers: HashMap<String, u32> = HashMap::new();
    if f.options.has_header {
        for c in f.range.c0..=f.range.c1 {
            let t = display_text(wb, CellAddr::new(sheet, f.range.r0, c)).to_lowercase();
            headers.entry(t).or_insert(c);
        }
    }
    let mut out = Vec::new();
    if crit.r0 >= crit.r1 {
        return out;
    }
    for r in crit.r0 + 1..=crit.r1 {
        let mut and_row = Vec::new();
        let mut blank = true;
        for c in crit.c0..=crit.c1 {
            let label = display_text(wb, CellAddr::new(csheet, crit.r0, c)).to_lowercase();
            let Some(col) = headers.get(&label).copied() else {
                continue;
            };
            let v = wb.value(CellAddr::new(csheet, r, c));
            if v.is_empty() {
                continue;
            }
            blank = false;
            and_row.push((
                col,
                tally_functions::criteria::Criterion::parse_in(&v, &wb.settings, wb.locale),
            ));
        }
        if !blank {
            out.push(and_row);
        }
    }
    out
}

/// The rows a standard/advanced filter hides (for `Workbook::reapply_filter`).
pub fn std_filter_hidden(wb: &Workbook, sheet: SheetId, f: &StandardFilterData) -> Spans {
    if f.options.copy_to.is_some() {
        return Spans::default();
    }
    let visible: HashSet<u32> = visible_rows(wb, sheet, f).into_iter().collect();
    let r0 = f.range.r0 + u32::from(f.options.has_header);
    if r0 > f.range.r1 {
        return Spans::default();
    }
    Spans::from_sorted_rows((r0..=f.range.r1).filter(|r| !visible.contains(r)))
}

fn check_filter_args(
    wb: &Workbook,
    sheet: SheetId,
    range: &RangeDto,
    conditions: &[StdCondition],
    options: &StdFilterOptions,
) -> Result<CellRange, ApiError> {
    check_sheet(wb, sheet)?;
    let range = to_range(sheet, range)?;
    check_sheet_action(wb, sheet, |p| p.auto_filter)?;
    if conditions.len() > MAX_STD_CONDITIONS {
        return Err(ApiError::invalid_argument(format!(
            "at most {MAX_STD_CONDITIONS} conditions"
        )));
    }
    if conditions
        .iter()
        .any(|c| c.col < range.c0 || c.col > range.c1 || c.value.len() > 4096)
    {
        return Err(ApiError::invalid_argument(
            "a condition column is outside the range or its value is too long",
        ));
    }
    if range.cell_count() > MAX_TOOL_ROWS {
        return Err(ApiError::invalid_argument(
            "the range is too large to filter",
        ));
    }
    if let Some(d) = options.copy_to {
        check_sheet(wb, d.sheet)?;
        if d.r >= MAX_ROWS || d.c >= MAX_COLS {
            return Err(ApiError::invalid_argument("bad copy target"));
        }
    }
    Ok(range)
}

fn install_filter(
    wb: &mut Workbook,
    sheet: SheetId,
    data: StandardFilterData,
    fx: &mut Effects,
) -> Result<Vec<Change>, ApiError> {
    let old_rows = wb
        .sheet(sheet)
        .map(|s| s.filtered_rows.clone())
        .unwrap_or_default();
    let mut changes = with_meta(wb, sheet, fx, |_, meta| {
        meta.standard_filter = Some(Arc::new(data.clone()));
        // The filter is also kept on the database range holding the area.
        if let Some(i) = meta
            .database_ranges
            .iter()
            .position(|d| d.range == data.range)
        {
            Arc::make_mut(&mut meta.database_ranges)[i].filter = Some(data.clone());
        }
        Ok(())
    })?;
    wb.reapply_filter(sheet);
    changes.push(Change::FilteredRows {
        sheet,
        rows: old_rows,
    });
    wb.dirty_visibility_dependents(&[sheet]);
    Ok(changes)
}

/// A copied cell: column, content and style.
type SrcCell = (u32, Option<SnapContent>, StyleId);

/// Copies the visible rows (header included) of a filter to `dest`.
fn copy_filtered(
    wb: &mut Workbook,
    sheet: SheetId,
    f: &StandardFilterData,
    dest: SheetCell,
) -> Result<Vec<Change>, ApiError> {
    let mut rows: Vec<u32> = Vec::new();
    if f.options.has_header {
        rows.push(f.range.r0);
    }
    rows.extend(visible_rows(wb, sheet, f));
    let h = rows.len() as u64;
    let w = u64::from(f.range.c1 - f.range.c0) + 1;
    if h == 0 {
        return Ok(Vec::new());
    }
    if u64::from(dest.r) + h > u64::from(MAX_ROWS) || u64::from(dest.c) + w > u64::from(MAX_COLS) {
        return Err(ApiError::invalid_argument(
            "the copy does not fit on the sheet",
        ));
    }
    let target = CellRange::new(
        dest.sheet,
        dest.r,
        dest.c,
        dest.r + h as u32 - 1,
        dest.c + w as u32 - 1,
    );
    check_edit(wb, &[target])?;
    crate::ops::check_array_blocks(wb, &[target])?;
    let inv = capture_range(wb, target, true, true);
    let src: Vec<(u32, Vec<SrcCell>)> = rows
        .iter()
        .map(|r| {
            let cells = (f.range.c0..=f.range.c1)
                .map(|c| {
                    let a = CellAddr::new(sheet, *r, c);
                    (c, wb.snapshot_content(a), wb.style_at(a))
                })
                .collect();
            (*r, cells)
        })
        .collect();
    clear_range_content(wb, target);
    crate::undo::clear_range_styles(wb, target);
    for (i, (_, cells)) in src.iter().enumerate() {
        for (c, content, style) in cells {
            let a = CellAddr::new(dest.sheet, dest.r + i as u32, dest.c + (c - f.range.c0));
            if let Some(content) = content {
                // Formulas keep their relative offsets (as a copy does).
                wb.restore_content(a, Some(content.clone()));
            }
            if *style != StyleId::DEFAULT {
                if let Some(col) = wb.sheet_mut(dest.sheet).and_then(|s| s.column_mut(a.col)) {
                    col.styles.set(a.row, a.row, *style);
                }
            }
        }
    }
    wb.pending.ranges.push(target);
    Ok(vec![Change::Range(inv)])
}

/// `Op::StandardFilter`.
pub fn standard_filter(
    wb: &mut Workbook,
    sheet: SheetId,
    range: &RangeDto,
    conditions: &[StdCondition],
    options: &StdFilterOptions,
    fx: &mut Effects,
) -> Result<Vec<Change>, ApiError> {
    let range = check_filter_args(wb, sheet, range, conditions, options)?;
    let copy_to = options
        .copy_to
        .and_then(|d| Some((wb.tab_of(d.sheet)? as u32, d.r, d.c)));
    let data = std_filter_to_model(local(&range), conditions, options, None, copy_to);
    if let Some(dest) = options.copy_to {
        let mut out = copy_filtered(wb, sheet, &data, dest)?;
        if options.keep_criteria {
            out.extend(install_filter(wb, sheet, data, fx)?);
        }
        return Ok(out);
    }
    install_filter(wb, sheet, data, fx)
}

/// `Op::AdvancedFilter`.
pub fn advanced_filter(
    wb: &mut Workbook,
    sheet: SheetId,
    range: &RangeDto,
    criteria: &SheetRange,
    options: &StdFilterOptions,
    fx: &mut Effects,
) -> Result<Vec<Change>, ApiError> {
    let range = check_filter_args(wb, sheet, range, &[], options)?;
    check_sheet(wb, criteria.sheet)?;
    let crit = to_range(criteria.sheet, &criteria.range)?;
    if crit.cell_count() > 100_000 {
        return Err(ApiError::invalid_argument(
            "the criteria range is too large",
        ));
    }
    let tab = wb.tab_of(criteria.sheet).unwrap_or(0) as u32;
    let copy_to = options
        .copy_to
        .and_then(|d| Some((wb.tab_of(d.sheet)? as u32, d.r, d.c)));
    let mut opts = options.clone();
    opts.has_header = true;
    let data = std_filter_to_model(
        local(&range),
        &[],
        &opts,
        Some((tab, local(&crit))),
        copy_to,
    );
    if let Some(dest) = options.copy_to {
        let mut out = copy_filtered(wb, sheet, &data, dest)?;
        if options.keep_criteria {
            out.extend(install_filter(wb, sheet, data, fx)?);
        }
        return Ok(out);
    }
    install_filter(wb, sheet, data, fx)
}

/// `Op::ResetFilter`: removes the standard/advanced filter and shows its
/// rows (the AutoFilter buttons stay).
pub fn reset_filter(
    wb: &mut Workbook,
    sheet: SheetId,
    fx: &mut Effects,
) -> Result<Vec<Change>, ApiError> {
    check_sheet(wb, sheet)?;
    let old_rows = wb
        .sheet(sheet)
        .map(|s| s.filtered_rows.clone())
        .unwrap_or_default();
    let mut changes = with_meta(wb, sheet, fx, |_, meta| {
        meta.standard_filter = None;
        if meta.database_ranges.iter().any(|d| d.filter.is_some()) {
            for d in Arc::make_mut(&mut meta.database_ranges) {
                d.filter = None;
            }
        }
        if let Some(af) = meta.auto_filter.as_ref() {
            if !af.columns.is_empty() {
                let mut n = (**af).clone();
                n.columns.clear();
                meta.auto_filter = Some(Arc::new(n));
            }
        }
        Ok(())
    })?;
    wb.reapply_filter(sheet);
    changes.push(Change::FilteredRows {
        sheet,
        rows: old_rows,
    });
    wb.dirty_visibility_dependents(&[sheet]);
    Ok(changes)
}

// ---------------------------------------------------------------------------
// Subtotals
// ---------------------------------------------------------------------------

fn sub_group_to_model(g: &SubtotalGroup) -> SubtotalGroupData {
    SubtotalGroupData {
        group_by: g.group_by,
        columns: g.columns.clone(),
        function: agg_to_model(g.function),
    }
}

/// Runs the op `op` through the regular dispatcher (used for the row
/// inserts/deletes and sorts the tools are built from).
fn run(wb: &mut Workbook, op: Op, fx: &mut Effects) -> Result<Vec<Change>, ApiError> {
    crate::ops::apply_op(wb, &op, None, fx)
}

/// Inserts the subtotal rows of one group level inside `rows` and returns
/// the new last row of the area.
#[allow(clippy::too_many_arguments)]
fn subtotal_level(
    wb: &mut Workbook,
    sheet: SheetId,
    groups: &[SubtotalGroupData],
    level: usize,
    r0: u32,
    r1: u32,
    c0: u32,
    c1: u32,
    case_sensitive: bool,
    fx: &mut Effects,
    changes: &mut Vec<Change>,
    subtotal_rows: &mut Vec<u32>,
) -> Result<u32, ApiError> {
    let g = &groups[level];
    let key = |wb: &Workbook, r: u32| {
        let t = display_text(wb, CellAddr::new(sheet, r, g.group_by));
        if case_sensitive {
            t
        } else {
            t.to_lowercase()
        }
    };
    let mut end = r1;
    let mut start = r0;
    while start <= end {
        let k = key(wb, start);
        let mut last = start;
        while last < end && key(wb, last + 1) == k {
            last += 1;
        }
        // Inner levels first: they grow the run.
        let mut run_end = last;
        if level + 1 < groups.len() {
            let grown = subtotal_level(
                wb,
                sheet,
                groups,
                level + 1,
                start,
                last,
                c0,
                c1,
                case_sensitive,
                fx,
                changes,
                subtotal_rows,
            )?;
            end += grown - last;
            run_end = grown;
        }
        // The subtotal row after the run.
        let at = run_end + 1;
        if at >= MAX_ROWS {
            return Err(ApiError::invalid_argument("no room for subtotal rows"));
        }
        changes.extend(run(
            wb,
            Op::InsertRows {
                sheet,
                at,
                count: 1,
            },
            fx,
        )?);
        end += 1;
        for r in subtotal_rows.iter_mut() {
            if *r >= at {
                *r += 1;
            }
        }
        let label_text = display_text(wb, CellAddr::new(sheet, start, g.group_by));
        let label = format!("{label_text} {}", agg_caption(g.function));
        let row_range = CellRange::new(sheet, at, c0, at, c1);
        changes.push(Change::Range(capture_range(wb, row_range, true, true)));
        wb.set_value(CellAddr::new(sheet, at, g.group_by), Value::text(label));
        for col in &g.columns {
            if *col < c0 || *col > c1 {
                continue;
            }
            let letters = tally_types::col_to_letters(*col);
            let text = format!(
                "=SUBTOTAL({};{}{}:{}{})",
                subtotal_code(g.function),
                letters,
                start + 1,
                letters,
                run_end + 1
            );
            wb.set_input(CellAddr::new(sheet, at, *col), &text);
        }
        map_styles(wb, row_range, |table, old| {
            let mut d = table.get(old).data.clone();
            d.font.bold = true;
            table.intern(d)
        });
        subtotal_rows.push(at);
        // Outline group over the run's rows (inner subtotal rows included).
        let _ = with_meta(wb, sheet, fx, |_, meta| {
            let list = Arc::make_mut(&mut meta.row_groups);
            if !list.iter().any(|x| x.start == start && x.end == run_end) {
                let _ = crate::outline::insert_group(list, start, run_end);
            }
            Ok(())
        })
        .map(|c| changes.extend(c));
        start = at + 1;
    }
    Ok(end)
}

/// `Op::Subtotals`.
pub fn subtotals(
    wb: &mut Workbook,
    sheet: SheetId,
    range: &RangeDto,
    groups: &[SubtotalGroup],
    options: &SubtotalOptions,
    fx: &mut Effects,
) -> Result<Vec<Change>, ApiError> {
    check_sheet(wb, sheet)?;
    let range = to_range(sheet, range)?;
    check_sheet_action(wb, sheet, |p| p.insert_rows)?;
    check_edit(wb, &[range])?;
    if groups.is_empty() || groups.len() > MAX_SUBTOTAL_GROUPS {
        return Err(ApiError::invalid_argument(format!(
            "between 1 and {MAX_SUBTOTAL_GROUPS} subtotal groups"
        )));
    }
    if groups
        .iter()
        .any(|g| g.group_by < range.c0 || g.group_by > range.c1 || g.columns.len() > 1024)
    {
        return Err(ApiError::invalid_argument(
            "a group column is outside the range",
        ));
    }
    if range.cell_count() > MAX_TOOL_ROWS || range.rows() < 2 {
        return Err(ApiError::invalid_argument("bad range for subtotals"));
    }
    let model: Vec<SubtotalGroupData> = groups.iter().map(sub_group_to_model).collect();
    let mut changes: Vec<Change> = Vec::new();
    // Existing subtotals go first.
    if wb.sheet(sheet).is_some_and(|s| s.meta.subtotals.is_some()) {
        changes.extend(remove_subtotals(wb, sheet, &range_dto(&local(&range)), fx)?);
    }
    let r0 = range.r0 + 1;
    let mut r1 = range.r1;
    if options.presort {
        let keys: Vec<tally_api::SortKey> = model
            .iter()
            .map(|g| tally_api::SortKey {
                col: g.group_by,
                ascending: options.ascending,
            })
            .collect();
        changes.extend(run(
            wb,
            Op::SortRange {
                sheet,
                range: range_dto(&local(&range)),
                keys,
                has_header: true,
                case_sensitive: options.case_sensitive,
                options: Some(tally_api::SortOptions {
                    include_formats: options.include_formats,
                    ..tally_api::SortOptions::default()
                }),
            },
            fx,
        )?);
    }
    let mut subtotal_rows = Vec::new();
    let end = subtotal_level(
        wb,
        sheet,
        &model,
        0,
        r0,
        r1,
        range.c0,
        range.c1,
        options.case_sensitive,
        fx,
        &mut changes,
        &mut subtotal_rows,
    )?;
    r1 = end;
    // Grand total.
    let at = r1 + 1;
    if at >= MAX_ROWS {
        return Err(ApiError::invalid_argument("no room for the grand total"));
    }
    changes.extend(run(
        wb,
        Op::InsertRows {
            sheet,
            at,
            count: 1,
        },
        fx,
    )?);
    let row_range = CellRange::new(sheet, at, range.c0, at, range.c1);
    changes.push(Change::Range(capture_range(wb, row_range, true, true)));
    wb.set_value(
        CellAddr::new(sheet, at, model[0].group_by),
        Value::text("Grand Total"),
    );
    let mut done: HashSet<u32> = HashSet::new();
    for g in &model {
        for col in &g.columns {
            if *col < range.c0 || *col > range.c1 || !done.insert(*col) {
                continue;
            }
            let letters = tally_types::col_to_letters(*col);
            let text = format!(
                "=SUBTOTAL({};{}{}:{}{})",
                subtotal_code(g.function),
                letters,
                r0 + 1,
                letters,
                r1 + 1
            );
            wb.set_input(CellAddr::new(sheet, at, *col), &text);
        }
    }
    map_styles(wb, row_range, |table, old| {
        let mut d = table.get(old).data.clone();
        d.font.bold = true;
        table.intern(d)
    });
    if options.page_breaks {
        for r in &subtotal_rows {
            if let Some(b) = r.checked_add(1) {
                changes.extend(run(
                    wb,
                    Op::SetPageBreak {
                        sheet,
                        axis: tally_api::BreakAxis::Row,
                        at: b,
                        present: true,
                    },
                    fx,
                )?);
            }
        }
    }
    let data = SubtotalsData {
        range: CellRangeLocal {
            r0: range.r0,
            c0: range.c0,
            r1: at,
            c1: range.c1,
        },
        groups: model,
        page_breaks: options.page_breaks,
        case_sensitive: options.case_sensitive,
        presort: options.presort,
        ascending: options.ascending,
        include_formats: options.include_formats,
    };
    changes.extend(with_meta(wb, sheet, fx, |_, meta| {
        meta.subtotals = Some(Arc::new(data.clone()));
        if let Some(i) = meta
            .database_ranges
            .iter()
            .position(|d| d.range.r0 == range.r0 && d.range.c0 == range.c0)
        {
            Arc::make_mut(&mut meta.database_ranges)[i].subtotals = Some(data.clone());
        }
        Ok(())
    })?);
    Ok(changes)
}

/// `true` if the row holds a subtotal (a `SUBTOTAL(` formula in one of
/// its cells and a label text).
fn is_subtotal_row(wb: &Workbook, sheet: SheetId, r: u32, c0: u32, c1: u32) -> bool {
    let Some(s) = wb.sheet(sheet) else {
        return false;
    };
    let mut has_formula = false;
    let mut has_text = false;
    for c in c0..=c1.min(s.columns.len().saturating_sub(1) as u32) {
        match s.get(r, c) {
            Some(CellContent::Formula(id)) => {
                if wb
                    .formula_text(*id)
                    .is_some_and(|t| t.to_uppercase().starts_with("=SUBTOTAL("))
                {
                    has_formula = true;
                }
            }
            Some(CellContent::Value(Value::Text(t))) => {
                let t = t.as_ref();
                if t == "Grand Total"
                    || [
                        AggFnData::Sum,
                        AggFnData::Count,
                        AggFnData::Average,
                        AggFnData::Max,
                        AggFnData::Min,
                        AggFnData::Product,
                        AggFnData::CountNums,
                        AggFnData::StDev,
                        AggFnData::StDevP,
                        AggFnData::Var,
                        AggFnData::VarP,
                    ]
                    .iter()
                    .any(|f| t.ends_with(&format!(" {}", agg_caption(*f))))
                {
                    has_text = true;
                }
            }
            _ => {}
        }
    }
    has_formula && has_text
}

/// `Op::RemoveSubtotals`.
pub fn remove_subtotals(
    wb: &mut Workbook,
    sheet: SheetId,
    range: &RangeDto,
    fx: &mut Effects,
) -> Result<Vec<Change>, ApiError> {
    check_sheet(wb, sheet)?;
    let mut range = to_range(sheet, range)?;
    check_sheet_action(wb, sheet, |p| p.delete_rows)?;
    if let Some(st) = wb.sheet(sheet).and_then(|s| s.meta.subtotals.clone()) {
        let r = crate::filter::area(sheet, &st.range);
        if r.intersects(&range) {
            range = CellRange::new(
                sheet,
                range.r0.min(r.r0),
                range.c0.min(r.c0),
                range.r1.max(r.r1),
                range.c1.max(r.c1),
            );
        }
    }
    let rows: Vec<u32> = (range.r0..=range.r1)
        .filter(|r| is_subtotal_row(wb, sheet, *r, range.c0, range.c1))
        .collect();
    let mut changes = Vec::new();
    // Bottom to top, in runs.
    let mut i = rows.len();
    while i > 0 {
        let end = rows[i - 1];
        let mut start = end;
        while i > 1 && rows[i - 2] + 1 == start {
            start = rows[i - 2];
            i -= 1;
        }
        i -= 1;
        changes.extend(run(
            wb,
            Op::DeleteRows {
                sheet,
                at: start,
                count: end - start + 1,
            },
            fx,
        )?);
    }
    changes.extend(with_meta(wb, sheet, fx, |_, meta| {
        meta.subtotals = None;
        let before = meta.row_groups.len();
        if before > 0 {
            Arc::make_mut(&mut meta.row_groups).retain(|g| g.end < range.r0 || g.start > range.r1);
        }
        if meta.database_ranges.iter().any(|d| d.subtotals.is_some()) {
            for d in Arc::make_mut(&mut meta.database_ranges) {
                d.subtotals = None;
            }
        }
        Ok(())
    })?);
    crate::outline::reapply(wb, sheet);
    Ok(changes)
}

// ---------------------------------------------------------------------------
// Consolidate
// ---------------------------------------------------------------------------

/// `Op::Consolidate`.
#[allow(clippy::too_many_arguments)]
pub fn consolidate(
    wb: &mut Workbook,
    function: AggFn,
    sources: &[SheetRange],
    dest: SheetCell,
    by_row_labels: bool,
    by_col_labels: bool,
    link: bool,
    fx: &mut Effects,
) -> Result<Vec<Change>, ApiError> {
    let _ = fx;
    if sources.is_empty() || sources.len() > 255 {
        return Err(ApiError::invalid_argument(
            "between 1 and 255 source ranges",
        ));
    }
    check_sheet(wb, dest.sheet)?;
    if dest.r >= MAX_ROWS || dest.c >= MAX_COLS {
        return Err(ApiError::invalid_argument("bad destination"));
    }
    let mut srcs: Vec<CellRange> = Vec::with_capacity(sources.len());
    for s in sources {
        check_sheet(wb, s.sheet)?;
        let r = to_range(s.sheet, &s.range)?;
        if r.cell_count() > MAX_OP_CELLS {
            return Err(ApiError::invalid_argument("a source range is too large"));
        }
        srcs.push(r);
    }
    let f = agg_to_model(function);
    // Row and column keys: labels (first column/row of each source) or
    // positions.
    let mut row_keys: Vec<String> = Vec::new();
    let mut col_keys: Vec<String> = Vec::new();
    let mut row_index: HashMap<String, usize> = HashMap::new();
    let mut col_index: HashMap<String, usize> = HashMap::new();
    // (row key, col key) → the source cells.
    let mut cells: BTreeMap<(usize, usize), Vec<CellAddr>> = BTreeMap::new();
    for s in &srcs {
        let r_start = s.r0 + u32::from(by_col_labels);
        let c_start = s.c0 + u32::from(by_row_labels);
        for r in r_start..=s.r1 {
            let rk = if by_row_labels {
                display_text(wb, CellAddr::new(s.sheet, r, s.c0))
            } else {
                (r - r_start).to_string()
            };
            let ri = *row_index.entry(rk.clone()).or_insert_with(|| {
                row_keys.push(rk.clone());
                row_keys.len() - 1
            });
            for c in c_start..=s.c1 {
                let ck = if by_col_labels {
                    display_text(wb, CellAddr::new(s.sheet, s.r0, c))
                } else {
                    (c - c_start).to_string()
                };
                let ci = *col_index.entry(ck.clone()).or_insert_with(|| {
                    col_keys.push(ck.clone());
                    col_keys.len() - 1
                });
                cells
                    .entry((ri, ci))
                    .or_default()
                    .push(CellAddr::new(s.sheet, r, c));
            }
        }
        if cells.len() as u64 > MAX_OP_CELLS {
            return Err(ApiError::invalid_argument("the consolidation is too large"));
        }
    }
    let h = row_keys.len() as u64 + u64::from(by_col_labels);
    let w = col_keys.len() as u64 + u64::from(by_row_labels);
    if h == 0 || w == 0 {
        return Ok(Vec::new());
    }
    if u64::from(dest.r) + h > u64::from(MAX_ROWS) || u64::from(dest.c) + w > u64::from(MAX_COLS) {
        return Err(ApiError::invalid_argument(
            "the result does not fit on the sheet",
        ));
    }
    let target = CellRange::new(
        dest.sheet,
        dest.r,
        dest.c,
        dest.r + h as u32 - 1,
        dest.c + w as u32 - 1,
    );
    check_edit(wb, &[target])?;
    crate::ops::check_array_blocks(wb, &[target])?;
    let inv = capture_range(wb, target, true, false);
    clear_range_content(wb, target);
    let r_off = dest.r + u32::from(by_col_labels);
    let c_off = dest.c + u32::from(by_row_labels);
    if by_col_labels {
        for (ci, k) in col_keys.iter().enumerate() {
            wb.set_value(
                CellAddr::new(dest.sheet, dest.r, c_off + ci as u32),
                Value::text(k.as_str()),
            );
        }
    }
    if by_row_labels {
        for (ri, k) in row_keys.iter().enumerate() {
            wb.set_value(
                CellAddr::new(dest.sheet, r_off + ri as u32, dest.c),
                Value::text(k.as_str()),
            );
        }
    }
    for ((ri, ci), addrs) in &cells {
        let at = CellAddr::new(dest.sheet, r_off + *ri as u32, c_off + *ci as u32);
        if link {
            let refs: Vec<String> = addrs
                .iter()
                .map(|a| {
                    let name = wb
                        .sheet(a.sheet)
                        .map(|s| s.name.to_string())
                        .unwrap_or_default();
                    let quoted = if name.contains([' ', '\'', '.', '-']) {
                        format!("'{}'", name.replace('\'', "''"))
                    } else {
                        name
                    };
                    format!("${}.{}", quoted, a.to_a1())
                })
                .collect();
            let text = format!("={}({})", agg_function_name(f), refs.join(";"));
            wb.set_input(at, &text);
        } else {
            let v = aggregate(f, addrs.iter().map(|a| wb.value(*a)));
            if !v.is_empty() {
                wb.set_value(at, v);
            }
        }
    }
    wb.pending.ranges.push(target);
    Ok(vec![Change::Range(inv)])
}

// ---------------------------------------------------------------------------
// Text to columns
// ---------------------------------------------------------------------------

/// Splits `text` per the CSV options (separators, merged separators,
/// quotes, trimming, or fixed widths).
pub fn split_fields(text: &str, o: &CsvImportOptionsDto) -> Vec<String> {
    if let Some(widths) = &o.fixed_widths {
        let chars: Vec<char> = text.chars().collect();
        let mut starts: Vec<usize> = widths.iter().map(|w| *w as usize).collect();
        starts.sort_unstable();
        starts.dedup();
        if starts.first() != Some(&0) {
            starts.insert(0, 0);
        }
        let mut out = Vec::new();
        for (i, s) in starts.iter().enumerate() {
            let e = starts.get(i + 1).copied().unwrap_or(chars.len());
            let s = (*s).min(chars.len());
            let e = e.min(chars.len()).max(s);
            let f: String = chars[s..e].iter().collect();
            out.push(if o.trim { f.trim().to_string() } else { f });
        }
        return out;
    }
    let seps: Vec<char> = o.delimiters.chars().collect();
    let quote = o.quote.chars().next();
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quote = false;
    let mut chars = text.chars().peekable();
    let mut last_was_sep = false;
    while let Some(c) = chars.next() {
        if Some(c) == quote {
            if in_quote && chars.peek() == quote.as_ref() {
                cur.push(c);
                chars.next();
            } else {
                in_quote = !in_quote;
            }
            last_was_sep = false;
            continue;
        }
        if !in_quote && seps.contains(&c) {
            if o.merge_delimiters && last_was_sep {
                continue;
            }
            out.push(std::mem::take(&mut cur));
            last_was_sep = true;
            continue;
        }
        last_was_sep = false;
        cur.push(c);
    }
    out.push(cur);
    if o.trim {
        for f in &mut out {
            *f = f.trim().to_string();
        }
    }
    out
}

/// Parses a date field in the given day/month/year order.
fn parse_date(field: &str, order: CsvColumnType, epoch: tally_types::DateEpoch) -> Option<f64> {
    let parts: Vec<u32> = field
        .split(|c: char| !c.is_ascii_digit())
        .filter(|p| !p.is_empty())
        .map(|p| p.parse::<u32>().ok())
        .collect::<Option<_>>()?;
    if parts.len() != 3 {
        return None;
    }
    let (d, m, y) = match order {
        CsvColumnType::DateDmy => (parts[0], parts[1], parts[2]),
        CsvColumnType::DateMdy => (parts[1], parts[0], parts[2]),
        _ => (parts[2], parts[1], parts[0]),
    };
    let y = if y < 100 {
        if y < 30 {
            2000 + y
        } else {
            1900 + y
        }
    } else {
        y
    };
    serial_from_ymd(epoch, i32::try_from(y).ok()?, m, d)
}

/// `Op::TextToColumns`.
pub fn text_to_columns(
    wb: &mut Workbook,
    sheet: SheetId,
    range: &RangeDto,
    options: &CsvImportOptionsDto,
    fx: &mut Effects,
) -> Result<Vec<Change>, ApiError> {
    let _ = fx;
    check_sheet(wb, sheet)?;
    let range = to_range(sheet, range)?;
    if range.c0 != range.c1 {
        return Err(ApiError::invalid_argument("select one column"));
    }
    if range.rows() as u64 > MAX_TOOL_ROWS {
        return Err(ApiError::invalid_argument("the range is too large"));
    }
    if options.delimiters.is_empty() && options.fixed_widths.is_none() {
        return Err(ApiError::invalid_argument("no separator"));
    }
    let rows: Vec<(u32, Vec<String>)> = (range.r0..=range.r1)
        .filter_map(|r| {
            let a = CellAddr::new(sheet, r, range.c0);
            let v = wb.value(a);
            let text = match &v {
                Value::Text(t) => t.to_string(),
                Value::Empty => return None,
                _ => display_text(wb, a),
            };
            Some((r, split_fields(&text, options)))
        })
        .collect();
    let width = rows.iter().map(|(_, f)| f.len()).max().unwrap_or(0) as u32;
    if width == 0 {
        return Ok(Vec::new());
    }
    let c1 = range
        .c0
        .checked_add(width - 1)
        .filter(|c| *c < MAX_COLS)
        .ok_or_else(|| ApiError::invalid_argument("the columns do not fit on the sheet"))?;
    let target = CellRange::new(sheet, range.r0, range.c0, range.r1, c1);
    check_edit(wb, &[target])?;
    crate::ops::check_array_blocks(wb, &[target])?;
    let inv = capture_range(wb, target, true, true);
    let epoch = wb.settings.date_epoch;
    for (r, fields) in rows {
        let mut out_col = range.c0;
        for (i, field) in fields.iter().enumerate() {
            let kind = options
                .column_types
                .get(i)
                .copied()
                .unwrap_or(CsvColumnType::Standard);
            if kind == CsvColumnType::Hide {
                continue;
            }
            let a = CellAddr::new(sheet, r, out_col);
            out_col += 1;
            wb.clear_content(a);
            if field.is_empty() {
                continue;
            }
            match kind {
                CsvColumnType::Text => wb.set_value(a, Value::text(field.as_str())),
                CsvColumnType::DateDmy | CsvColumnType::DateMdy | CsvColumnType::DateYmd => {
                    match parse_date(field, kind, epoch) {
                        Some(serial) => {
                            wb.set_value(a, Value::Number(serial));
                            map_styles(
                                wb,
                                CellRange::new(sheet, r, out_col - 1, r, out_col - 1),
                                |t, old| {
                                    if t.get(old).format.is_date() {
                                        old
                                    } else {
                                        t.with_format(old, "YYYY-MM-DD")
                                    }
                                },
                            );
                        }
                        None => wb.set_value(a, Value::text(field.as_str())),
                    }
                }
                _ if options.detect_numbers => wb.set_constant_input(a, field),
                _ => wb.set_value(a, Value::text(field.as_str())),
            }
        }
        // Columns past the fields of this row are cleared.
        for c in out_col..=c1 {
            wb.clear_content(CellAddr::new(sheet, r, c));
        }
    }
    wb.pending.ranges.push(target);
    Ok(vec![Change::Range(inv)])
}

// ---------------------------------------------------------------------------
// Remove duplicates, delete contents
// ---------------------------------------------------------------------------

/// `Op::RemoveDuplicates`: rows of `range` whose key columns repeat an
/// earlier row are removed (the cells below shift up inside the range,
/// as Excel does).
pub fn remove_duplicates(
    wb: &mut Workbook,
    sheet: SheetId,
    range: &RangeDto,
    cols: &[u32],
    has_header: bool,
    fx: &mut Effects,
) -> Result<Vec<Change>, ApiError> {
    check_sheet(wb, sheet)?;
    let range = to_range(sheet, range)?;
    check_edit(wb, &[range])?;
    if range.cell_count() > MAX_TOOL_ROWS {
        return Err(ApiError::invalid_argument("the range is too large"));
    }
    let keys: Vec<u32> = if cols.is_empty() {
        (range.c0..=range.c1).collect()
    } else {
        cols.to_vec()
    };
    if keys.iter().any(|c| *c < range.c0 || *c > range.c1) {
        return Err(ApiError::invalid_argument(
            "a key column is outside the range",
        ));
    }
    let r0 = range.r0 + u32::from(has_header);
    let mut seen: HashSet<String> = HashSet::new();
    let mut dups: Vec<u32> = Vec::new();
    for r in r0..=range.r1 {
        let key: Vec<String> = keys
            .iter()
            .map(|c| display_text(wb, CellAddr::new(sheet, r, *c)).to_lowercase())
            .collect();
        if !seen.insert(key.join("\u{1}")) {
            dups.push(r);
        }
    }
    let mut changes = Vec::new();
    let mut i = dups.len();
    while i > 0 {
        let end = dups[i - 1];
        let mut start = end;
        while i > 1 && dups[i - 2] + 1 == start {
            start = dups[i - 2];
            i -= 1;
        }
        i -= 1;
        let band = Band {
            sheet,
            rows: true,
            lo: range.c0,
            hi: range.c1,
            at: start,
            count: end - start + 1,
            insert: false,
        };
        changes.extend(shift_cells(wb, &band, fx)?);
    }
    if !dups.is_empty() {
        fx.replaced = Some(dups.len() as u32);
    }
    Ok(changes)
}

/// `Op::DeleteContents`.
pub fn delete_contents(
    wb: &mut Workbook,
    sheet: SheetId,
    ranges: &[RangeDto],
    what: &DeleteContentsFlags,
    fx: &mut Effects,
) -> Result<Vec<Change>, ApiError> {
    check_sheet(wb, sheet)?;
    let ranges: Vec<CellRange> = ranges
        .iter()
        .map(|r| to_range(sheet, r))
        .collect::<Result<_, _>>()?;
    let any_content = what.text || what.numbers || what.dates || what.formulas;
    if any_content {
        check_edit(wb, &ranges)?;
        crate::ops::check_array_blocks(wb, &ranges)?;
    }
    if what.formats {
        check_format(wb, &ranges)?;
    }
    let mut changes = Vec::new();
    for r in &ranges {
        if r.cell_count() > MAX_OP_CELLS && (any_content || what.notes) {
            return Err(ApiError::invalid_argument("the range is too large"));
        }
        if any_content {
            changes.push(Change::Range(capture_range(wb, *r, true, false)));
            let mut to_clear: Vec<CellAddr> = Vec::new();
            if let Some(s) = wb.sheet(sheet) {
                for c in r.c0..=r.c1.min(s.columns.len().saturating_sub(1) as u32) {
                    let Some(col) = s.column(c) else { continue };
                    for (row, content) in col.cells.iter_range(r.r0, r.r1) {
                        let a = CellAddr::new(sheet, row, c);
                        let del = match content {
                            CellContent::Formula(_) | CellContent::ArrayPart(_) => what.formulas,
                            CellContent::Value(Value::Text(_)) => what.text,
                            CellContent::Value(Value::Number(_)) => {
                                let date = wb.styles.get(s.style(row, c)).format.is_date();
                                if date {
                                    what.dates
                                } else {
                                    what.numbers
                                }
                            }
                            CellContent::Value(Value::Bool(_)) => what.numbers,
                            CellContent::Value(_) => what.text || what.numbers,
                        };
                        if del {
                            to_clear.push(a);
                        }
                    }
                }
            }
            for a in to_clear {
                wb.clear_content(a);
            }
            wb.pending.ranges.push(*r);
        }
        if what.formats {
            changes.push(Change::Range(capture_range(wb, *r, false, true)));
            crate::undo::clear_range_styles(wb, *r);
        }
    }
    if what.notes {
        changes.extend(with_meta(wb, sheet, fx, |_, meta| {
            for r in &ranges {
                if crate::annotations::entries_in(&meta.notes, r)
                    .next()
                    .is_some()
                {
                    crate::annotations::clear_in(Arc::make_mut(&mut meta.notes), r);
                }
            }
            Ok(())
        })?);
    }
    if what.objects {
        changes.extend(with_meta(wb, sheet, fx, |_, meta| {
            let inside = |row: u32, col: u32| {
                ranges
                    .iter()
                    .any(|r| r.contains(CellAddr::new(sheet, row, col)))
            };
            if meta
                .charts
                .iter()
                .any(|c| inside(c.data.anchor.row, c.data.anchor.col))
            {
                Arc::make_mut(&mut meta.charts)
                    .retain(|c| !inside(c.data.anchor.row, c.data.anchor.col));
            }
            if meta
                .images
                .iter()
                .any(|c| inside(c.data.anchor.row, c.data.anchor.col))
            {
                Arc::make_mut(&mut meta.images)
                    .retain(|c| !inside(c.data.anchor.row, c.data.anchor.col));
            }
            Ok(())
        })?);
        fx.objects(sheet);
    }
    Ok(changes)
}

// ---------------------------------------------------------------------------
// Fill series
// ---------------------------------------------------------------------------

fn parse_number(wb: &Workbook, text: &str) -> Option<f64> {
    let ctx = RecognizeContext::now(wb.settings.date_epoch);
    match tally_numfmt::recognize_input_with(text.trim(), wb.locale, &ctx).value {
        Value::Number(n) => Some(n),
        Value::Bool(b) => Some(f64::from(u8::from(b))),
        _ => None,
    }
}

fn add_months(epoch: tally_types::DateEpoch, serial: f64, months: i64) -> Option<f64> {
    let (y, m, d) = ymd_from_serial(epoch, serial)?;
    let total = i64::from(y) * 12 + i64::from(m) - 1 + months;
    let ny = total.div_euclid(12);
    let nm = (total.rem_euclid(12) + 1) as u32;
    let nd = d.min(days_in_month(ny, nm));
    let base = serial_from_ymd(epoch, i32::try_from(ny).ok()?, nm, nd)?;
    Some(base + serial.fract())
}

/// `Op::FillSeries` (the Fill Series dialog).
#[allow(clippy::too_many_arguments)]
pub fn fill_series(
    wb: &mut Workbook,
    sheet: SheetId,
    range: &RangeDto,
    direction: Direction,
    kind: SeriesKind,
    unit: Option<DateUnit>,
    start: Option<&str>,
    end: Option<&str>,
    increment: Option<&str>,
    fx: &mut Effects,
) -> Result<Vec<Change>, ApiError> {
    let _ = fx;
    check_sheet(wb, sheet)?;
    let range = to_range(sheet, range)?;
    check_edit(wb, &[range])?;
    crate::ops::check_array_blocks(wb, &[range])?;
    if range.cell_count() > MAX_OP_CELLS {
        return Err(ApiError::invalid_argument("the range is too large"));
    }
    if kind == SeriesKind::AutoFill {
        let (source, target) = match direction {
            Direction::Down => (
                CellRange::new(sheet, range.r0, range.c0, range.r0, range.c1),
                range,
            ),
            Direction::Up => (
                CellRange::new(sheet, range.r1, range.c0, range.r1, range.c1),
                range,
            ),
            Direction::Right => (
                CellRange::new(sheet, range.r0, range.c0, range.r1, range.c0),
                range,
            ),
            Direction::Left => (
                CellRange::new(sheet, range.r0, range.c1, range.r1, range.c1),
                range,
            ),
        };
        return crate::fill::fill_range(wb, source, target, tally_api::FillMode::Auto);
    }
    let epoch = wb.settings.date_epoch;
    let step = match increment {
        Some(t) => {
            parse_number(wb, t).ok_or_else(|| ApiError::invalid_argument("bad increment"))?
        }
        None => 1.0,
    };
    let end_v = match end {
        Some(t) => {
            Some(parse_number(wb, t).ok_or_else(|| ApiError::invalid_argument("bad end value"))?)
        }
        None => None,
    };
    let vertical = matches!(direction, Direction::Down | Direction::Up);
    let lanes: Vec<u32> = if vertical {
        (range.c0..=range.c1).collect()
    } else {
        (range.r0..=range.r1).collect()
    };
    let inv = capture_range(wb, range, true, true);
    for lane in lanes {
        let mut idx: Vec<u32> = if vertical {
            (range.r0..=range.r1).collect()
        } else {
            (range.c0..=range.c1).collect()
        };
        if matches!(direction, Direction::Up | Direction::Left) {
            idx.reverse();
        }
        let at = |i: u32| {
            if vertical {
                CellAddr::new(sheet, i, lane)
            } else {
                CellAddr::new(sheet, lane, i)
            }
        };
        let first = at(idx[0]);
        let start_v = match start {
            Some(t) => {
                parse_number(wb, t).ok_or_else(|| ApiError::invalid_argument("bad start value"))?
            }
            None => match wb.value(first) {
                Value::Number(n) => n,
                _ => continue,
            },
        };
        let first_style = wb.style_at(first);
        let needs_date_format =
            kind == SeriesKind::Date && !wb.styles.get(first_style).format.is_date();
        let mut v = start_v;
        for (k, i) in idx.iter().enumerate() {
            if k > 0 {
                let next = match kind {
                    SeriesKind::Linear => Some(v + step),
                    SeriesKind::Growth => Some(v * step),
                    SeriesKind::Date => match unit.unwrap_or(DateUnit::Day) {
                        DateUnit::Day => Some(v + step),
                        DateUnit::Weekday => {
                            let mut d = v;
                            let mut left = step.round() as i64;
                            let dir = if left < 0 { -1.0 } else { 1.0 };
                            left = left.abs();
                            while left > 0 {
                                d += dir;
                                let wd = weekday(epoch, d).unwrap_or(2);
                                // 1 = Sunday, 7 = Saturday.
                                if wd != 1 && wd != 7 {
                                    left -= 1;
                                }
                            }
                            Some(d)
                        }
                        // Months count from the start, so the day clamps
                        // per month (Jan 31 → Feb 29 → Mar 31).
                        DateUnit::Month => {
                            add_months(epoch, start_v, (step.round() as i64) * k as i64)
                        }
                        DateUnit::Year => {
                            add_months(epoch, start_v, (step.round() as i64) * 12 * k as i64)
                        }
                    },
                    SeriesKind::AutoFill => None,
                };
                let Some(n) = next else { break };
                v = n;
                if let Some(e) = end_v {
                    let past = if step >= 0.0 && kind != SeriesKind::Growth
                        || kind == SeriesKind::Growth && step >= 1.0
                    {
                        v > e + 1e-9
                    } else {
                        v < e - 1e-9
                    };
                    if past {
                        break;
                    }
                }
            } else if start.is_none() {
                continue;
            }
            let a = at(*i);
            wb.set_value(a, Value::Number(v));
            let cell = CellRange::new(sheet, a.row, a.col, a.row, a.col);
            if k > 0 && first_style != StyleId::DEFAULT {
                map_styles(wb, cell, |_, _| first_style);
            }
            if needs_date_format {
                map_styles(wb, cell, |t, old| t.with_format(old, "YYYY-MM-DD"));
            }
        }
    }
    wb.pending.ranges.push(range);
    Ok(vec![Change::Range(inv)])
}

// ---------------------------------------------------------------------------
// Insert/delete cells
// ---------------------------------------------------------------------------

/// `Op::InsertCells` / `Op::DeleteCells`.
pub fn shift_range(
    wb: &mut Workbook,
    sheet: SheetId,
    range: &RangeDto,
    shift: tally_api::ShiftDir,
    fx: &mut Effects,
) -> Result<Vec<Change>, ApiError> {
    check_sheet(wb, sheet)?;
    let r = to_range(sheet, range)?;
    let (rows, insert) = match shift {
        tally_api::ShiftDir::Down => (true, true),
        tally_api::ShiftDir::Up => (true, false),
        tally_api::ShiftDir::Right => (false, true),
        tally_api::ShiftDir::Left => (false, false),
    };
    check_sheet_action(wb, sheet, |p| match (rows, insert) {
        (true, true) => p.insert_rows,
        (true, false) => p.delete_rows,
        (false, true) => p.insert_cols,
        (false, false) => p.delete_cols,
    })?;
    let band = if rows {
        Band {
            sheet,
            rows: true,
            lo: r.c0,
            hi: r.c1,
            at: r.r0,
            count: r.rows(),
            insert,
        }
    } else {
        Band {
            sheet,
            rows: false,
            lo: r.r0,
            hi: r.r1,
            at: r.c0,
            count: r.cols(),
            insert,
        }
    };
    shift_cells(wb, &band, fx)
}

// ---------------------------------------------------------------------------
// Sheet extras
// ---------------------------------------------------------------------------

fn repoint_sheet(f: &mut tally_formula::Formula, from: SheetId, to: SheetId) -> bool {
    use tally_formula::{RefExpr, SheetRef};
    let mut changed = false;
    let mut fix = |s: &mut SheetRef| {
        if let SheetRef::Id { id, .. } = s {
            if *id == from {
                *id = to;
                changed = true;
            }
        }
    };
    f.expr.walk_refs_mut(|r| match r {
        RefExpr::Cell(c) => fix(&mut c.sheet),
        RefExpr::Area(a, b) => {
            fix(&mut a.sheet);
            fix(&mut b.sheet);
        }
        RefExpr::Cols { sheet, .. } | RefExpr::Rows { sheet, .. } => fix(sheet),
        RefExpr::Invalid => {}
    });
    changed
}

/// A formula copied to a new sheet: address, AST, raw text, array size.
type CopiedFormula = (
    CellAddr,
    Arc<tally_formula::Formula>,
    Option<Arc<str>>,
    Option<(u32, u32)>,
);

/// `Op::CopySheet`: duplicates a sheet (cells, styles, dimensions, meta
/// and sheet-scoped names); formulas in the copy that name the source
/// sheet point at the copy.
pub fn copy_sheet(
    wb: &mut Workbook,
    sheet: SheetId,
    name: Option<&str>,
    index: Option<u32>,
    fx: &mut Effects,
) -> Result<Vec<Change>, ApiError> {
    check_sheet(wb, sheet)?;
    check_structure(wb)?;
    if let Some(n) = name {
        crate::ops::check_sheet_name(n)?;
        if wb.sheet_by_name_ci(n).is_some() {
            return Err(ApiError::invalid_argument(format!(
                "a sheet named {n:?} already exists"
            )));
        }
    }
    if wb.sheets.len() >= crate::workbook::MAX_SHEETS {
        return Err(ApiError::invalid_argument(format!(
            "a workbook can have at most {} sheets",
            crate::workbook::MAX_SHEETS
        )));
    }
    let src_tab = wb.tab_of(sheet).unwrap_or(0);
    let src_name = wb
        .sheet(sheet)
        .map(|s| s.name.to_string())
        .unwrap_or_default();
    let new_name = match name {
        Some(n) => n.to_string(),
        None => wb.unique_sheet_name(&format!("{src_name}_2"), None),
    };
    let at = index.map_or(src_tab + 1, |i| i as usize);
    let new = wb.add_sheet(Some(&new_name), Some(at));
    // Cells and styles.
    let mut formulas: Vec<CopiedFormula> = Vec::new();
    let mut values: Vec<(CellAddr, Value)> = Vec::new();
    let (columns, row_dims, col_dims, hidden, tab_color, meta) = {
        let s = wb
            .sheet(sheet)
            .ok_or_else(|| ApiError::not_found("no sheet"))?;
        for (ci, col) in s.columns.iter().enumerate() {
            for (r, content) in col.cells.iter_range(0, MAX_ROWS - 1) {
                let a = CellAddr::new(new, r, ci as u32);
                match content {
                    CellContent::Value(v) => values.push((a, v.clone())),
                    CellContent::Formula(id) => {
                        if let Some(f) = wb.formulas.get(*id) {
                            formulas.push((a, f.formula.clone(), f.invalid_text.clone(), f.array));
                        }
                    }
                    CellContent::ArrayPart(_) => {}
                }
            }
        }
        let cols: Vec<crate::storage::StyleRuns> =
            s.columns.iter().map(|c| c.styles.clone()).collect();
        (
            cols,
            s.row_dims.clone(),
            s.col_dims.clone(),
            s.hidden,
            s.tab_color,
            s.meta.clone(),
        )
    };
    for (a, v) in values {
        wb.set_value(a, v);
    }
    // Sheet-scoped names first, so the copy's formulas resolve them.
    let names = crate::undo::names_snapshot(wb);
    let scoped: Vec<(String, crate::names::NameDef)> = wb
        .names
        .defined()
        .filter(|(_, e)| e.scope == Some(sheet))
        .filter_map(|(_, e)| Some((e.name.to_string(), e.def.clone()?)))
        .collect();
    for (n, def) in scoped {
        let mut f = (*def.formula).clone();
        repoint_sheet(&mut f, sheet, new);
        let mut base = def.base;
        if base.sheet == sheet {
            base.sheet = new;
        }
        wb.names.define(
            &n,
            Some(new),
            crate::names::NameDef {
                formula: Arc::new(f),
                base,
            },
        );
    }
    for (a, f, invalid, array) in formulas {
        let mut nf = (*f).clone();
        repoint_sheet(&mut nf, sheet, new);
        if invalid.is_none() && crate::names::has_names(&nf.expr) {
            // Names are stored by id: re-resolve them from the copy, where
            // the copied sheet-scoped names are visible.
            let text = tally_formula::print(&nf, a, wb.ui_dialect(), wb);
            if let Ok(re) = wb.parse_formula(&text, a) {
                nf = re;
            }
        }
        match array {
            Some((rows, cols)) => wb.set_array_formula(a, Arc::new(nf), rows, cols),
            None => wb.set_formula(a, Arc::new(nf), invalid),
        }
    }
    if let Some(s) = wb.sheet_mut(new) {
        for (ci, runs) in columns.into_iter().enumerate() {
            if runs.is_empty() {
                continue;
            }
            if let Some(col) = s.column_mut(ci as u32) {
                col.styles = runs;
            }
        }
        s.row_dims = row_dims;
        s.col_dims = col_dims;
        s.hidden = hidden;
        s.tab_color = tab_color;
        let mut m = meta;
        // Rule formulas and pivots are re-pointed; pivots keep their
        // source (a copy would write into the same output otherwise).
        if !m.cond_formats.is_empty() {
            for cf in Arc::make_mut(&mut m.cond_formats).iter_mut() {
                let cf = Arc::make_mut(cf);
                cf.origin.sheet = new;
                for f in cf.formulas_mut() {
                    let mut nf = (*f.formula).clone();
                    if repoint_sheet(&mut nf, sheet, new) {
                        f.formula = Arc::new(nf);
                    }
                }
            }
        }
        if !m.validations.is_empty() {
            for v in Arc::make_mut(&mut m.validations).iter_mut() {
                let v = Arc::make_mut(v);
                v.origin.sheet = new;
                for f in v.formulas_mut() {
                    let mut nf = (*f.formula).clone();
                    if repoint_sheet(&mut nf, sheet, new) {
                        f.formula = Arc::new(nf);
                    }
                }
            }
        }
        m.pivots = Arc::new(Vec::new());
        m.standard_filter = None;
        s.meta = m;
        s.filtered_rows = Spans::default();
    }
    wb.reapply_filter(new);
    crate::outline::reapply(wb, new);
    wb.refresh_all_precedents();
    wb.refresh_all_name_dependents();
    wb.pending
        .ranges
        .push(CellRange::new(new, 0, 0, MAX_ROWS - 1, MAX_COLS - 1));
    fx.sheets_changed = true;
    fx.layout(new);
    fx.objects(new);
    fx.created(u32::MAX, new);
    Ok(vec![names, Change::RemoveSheet { id: new }])
}

/// `Op::SetSheetHidden`.
pub fn set_sheet_hidden(
    wb: &mut Workbook,
    sheet: SheetId,
    hidden: bool,
    fx: &mut Effects,
) -> Result<Vec<Change>, ApiError> {
    check_sheet(wb, sheet)?;
    check_structure(wb)?;
    let (was, tab_color) = wb
        .sheet(sheet)
        .map(|s| (s.hidden, s.tab_color))
        .unwrap_or_default();
    if was == hidden {
        return Ok(Vec::new());
    }
    if hidden && wb.sheets.iter().filter(|s| !s.hidden).count() <= 1 {
        return Err(ApiError::invalid_argument(
            "the last visible sheet cannot be hidden",
        ));
    }
    if let Some(s) = wb.sheet_mut(sheet) {
        s.hidden = hidden;
    }
    if hidden && wb.active_sheet == sheet {
        if let Some(next) = wb.sheets.iter().find(|s| !s.hidden).map(|s| s.id) {
            wb.active_sheet = next;
        }
    }
    fx.sheets_changed = true;
    Ok(vec![Change::SheetProps {
        id: sheet,
        hidden: was,
        tab_color,
    }])
}

/// `Op::SetTabColor`.
pub fn set_tab_color(
    wb: &mut Workbook,
    sheet: SheetId,
    color: Option<u32>,
    fx: &mut Effects,
) -> Result<Vec<Change>, ApiError> {
    check_sheet(wb, sheet)?;
    let (hidden, was) = wb
        .sheet(sheet)
        .map(|s| (s.hidden, s.tab_color))
        .unwrap_or_default();
    if was == color {
        return Ok(Vec::new());
    }
    if let Some(s) = wb.sheet_mut(sheet) {
        s.tab_color = color;
    }
    fx.sheets_changed = true;
    Ok(vec![Change::SheetProps {
        id: sheet,
        hidden,
        tab_color: was,
    }])
}

// ---------------------------------------------------------------------------
// Names extras, database ranges
// ---------------------------------------------------------------------------

/// A label made into a name as AOO does: spaces and other characters
/// that are not letters, digits, `_` or `.` become `_`; a leading digit
/// gets a `_` in front.
pub fn name_from_label(label: &str) -> Option<String> {
    let t = label.trim();
    if t.is_empty() {
        return None;
    }
    let mut out = String::with_capacity(t.len() + 1);
    for c in t.chars() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '.' {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    if out.starts_with(|c: char| !c.is_ascii_alphabetic() && c != '_') {
        out.insert(0, '_');
    }
    Some(out)
}

fn sheet_ref_text(wb: &Workbook, sheet: SheetId) -> String {
    let name = wb
        .sheet(sheet)
        .map(|s| s.name.to_string())
        .unwrap_or_default();
    if name.contains([' ', '\'', '.', '-']) {
        format!("'{}'", name.replace('\'', "''"))
    } else {
        name
    }
}

/// `Op::CreateNamesFromLabels`.
pub fn create_names_from_labels(
    wb: &mut Workbook,
    sheet: SheetId,
    range: &RangeDto,
    from: &NameLabelFlags,
    fx: &mut Effects,
) -> Result<Vec<Change>, ApiError> {
    check_sheet(wb, sheet)?;
    check_structure(wb)?;
    let range = to_range(sheet, range)?;
    if !(from.top || from.left || from.bottom || from.right) {
        return Err(ApiError::invalid_argument("choose where the labels are"));
    }
    if range.cell_count() > MAX_OP_CELLS {
        return Err(ApiError::invalid_argument("the range is too large"));
    }
    let inner_r0 = range.r0 + u32::from(from.top);
    let inner_r1 = range.r1.saturating_sub(u32::from(from.bottom));
    let inner_c0 = range.c0 + u32::from(from.left);
    let inner_c1 = range.c1.saturating_sub(u32::from(from.right));
    let sheet_text = sheet_ref_text(wb, sheet);
    let abs = |r0: u32, c0: u32, r1: u32, c1: u32| {
        format!(
            "${}.${}${}:${}${}",
            sheet_text,
            tally_types::col_to_letters(c0),
            r0 + 1,
            tally_types::col_to_letters(c1),
            r1 + 1
        )
    };
    let mut defs: Vec<(String, String)> = Vec::new();
    let mut add = |label: String, refers: String| {
        if let Some(n) = name_from_label(&label) {
            if crate::names::validate_name(wb, &n).is_ok()
                && !defs.iter().any(|(x, _)| x.eq_ignore_ascii_case(&n))
            {
                defs.push((n, refers));
            }
        }
    };
    if inner_r0 <= inner_r1 && (from.top || from.bottom) {
        for c in inner_c0..=inner_c1.min(range.c1) {
            if from.top {
                add(
                    display_text(wb, CellAddr::new(sheet, range.r0, c)),
                    abs(inner_r0, c, inner_r1, c),
                );
            }
            if from.bottom {
                add(
                    display_text(wb, CellAddr::new(sheet, range.r1, c)),
                    abs(inner_r0, c, inner_r1, c),
                );
            }
        }
    }
    if inner_c0 <= inner_c1 && (from.left || from.right) {
        for r in inner_r0..=inner_r1.min(range.r1) {
            if from.left {
                add(
                    display_text(wb, CellAddr::new(sheet, r, range.c0)),
                    abs(r, inner_c0, r, inner_c1),
                );
            }
            if from.right {
                add(
                    display_text(wb, CellAddr::new(sheet, r, range.c1)),
                    abs(r, inner_c0, r, inner_c1),
                );
            }
        }
    }
    if defs.is_empty() {
        return Ok(Vec::new());
    }
    let mut changes = Vec::new();
    for (n, refers) in defs {
        changes.extend(run(
            wb,
            Op::DefineName {
                name: n,
                scope: None,
                refers_to: refers,
            },
            fx,
        )?);
    }
    Ok(changes)
}

/// The API form of a database range.
pub fn database_range_to_api(
    wb: &Workbook,
    sheet: SheetId,
    d: &DatabaseRangeData,
) -> DatabaseRangeDto {
    DatabaseRangeDto {
        name: d.name.clone(),
        sheet,
        range: range_dto(&d.range),
        has_header: d.has_header,
        filter: d.filter.as_ref().map(|f| std_filter_to_api(wb, f)),
    }
}

/// Every named database range.
pub fn database_ranges(wb: &Workbook) -> Vec<DatabaseRangeDto> {
    let mut out = Vec::new();
    for s in &wb.sheets {
        for d in s.meta.database_ranges.iter() {
            out.push(database_range_to_api(wb, s.id, d));
        }
    }
    out
}

fn check_db_name(name: &str) -> Result<(), ApiError> {
    let n = name.trim();
    if n.is_empty() || n.chars().count() > 255 {
        return Err(ApiError::invalid_argument("bad database range name"));
    }
    if n.starts_with("__Anonymous_Sheet_DB__") {
        return Err(ApiError::invalid_argument("that name is reserved"));
    }
    Ok(())
}

/// `Op::DefineDatabaseRange`.
pub fn define_database_range(
    wb: &mut Workbook,
    name: &str,
    sheet: SheetId,
    range: &RangeDto,
    has_header: bool,
    fx: &mut Effects,
) -> Result<Vec<Change>, ApiError> {
    check_sheet(wb, sheet)?;
    check_db_name(name)?;
    let range = to_range(sheet, range)?;
    let name = name.trim().to_string();
    let total: usize = wb.sheets.iter().map(|s| s.meta.database_ranges.len()).sum();
    // An existing range of the name (on any sheet) is redefined.
    let mut changes = Vec::new();
    let holders: Vec<SheetId> = wb
        .sheets
        .iter()
        .filter(|s| {
            s.id != sheet
                && s.meta
                    .database_ranges
                    .iter()
                    .any(|d| d.name.eq_ignore_ascii_case(&name))
        })
        .map(|s| s.id)
        .collect();
    for h in holders {
        changes.extend(with_meta(wb, h, fx, |_, meta| {
            Arc::make_mut(&mut meta.database_ranges)
                .retain(|d| !d.name.eq_ignore_ascii_case(&name));
            Ok(())
        })?);
    }
    changes.extend(with_meta(wb, sheet, fx, |_, meta| {
        let list = Arc::make_mut(&mut meta.database_ranges);
        match list.iter_mut().find(|d| d.name.eq_ignore_ascii_case(&name)) {
            Some(d) => {
                d.name = name.clone();
                d.range = local(&range);
                d.has_header = has_header;
            }
            None => {
                if total >= MAX_DATABASE_RANGES {
                    return Err(ApiError::invalid_argument(format!(
                        "at most {MAX_DATABASE_RANGES} database ranges"
                    )));
                }
                list.push(DatabaseRangeData {
                    name: name.clone(),
                    range: local(&range),
                    has_header,
                    filter_buttons: false,
                    filter: None,
                    subtotals: None,
                });
            }
        }
        Ok(())
    })?);
    Ok(changes)
}

/// `Op::DeleteDatabaseRange`.
pub fn delete_database_range(
    wb: &mut Workbook,
    name: &str,
    fx: &mut Effects,
) -> Result<Vec<Change>, ApiError> {
    let holder = wb
        .sheets
        .iter()
        .find(|s| {
            s.meta
                .database_ranges
                .iter()
                .any(|d| d.name.eq_ignore_ascii_case(name))
        })
        .map(|s| s.id)
        .ok_or_else(|| ApiError::not_found(format!("no database range {name:?}")))?;
    with_meta(wb, holder, fx, |_, meta| {
        Arc::make_mut(&mut meta.database_ranges).retain(|d| !d.name.eq_ignore_ascii_case(name));
        Ok(())
    })
}

/// The sheet and range of a named database range.
pub fn find_database_range(wb: &Workbook, name: &str) -> Option<(SheetId, CellRangeLocal, bool)> {
    wb.sheets.iter().find_map(|s| {
        s.meta
            .database_ranges
            .iter()
            .find(|d| d.name.eq_ignore_ascii_case(name))
            .map(|d| (s.id, d.range, d.has_header))
    })
}

// ---------------------------------------------------------------------------
// View options, split, custom sort lists
// ---------------------------------------------------------------------------

/// `Op::SetViewOptions`.
pub fn set_view_options(
    wb: &mut Workbook,
    sheet: SheetId,
    o: &ViewOptionsDto,
    fx: &mut Effects,
) -> Result<Vec<Change>, ApiError> {
    check_sheet(wb, sheet)?;
    let data = tally_model::ViewOptionsData {
        show_formulas: o.show_formulas,
        value_highlighting: o.value_highlighting,
        show_zero_values: o.show_zero_values,
        show_gridlines: o.show_gridlines,
        show_headers: o.show_headers,
    };
    with_meta(wb, sheet, fx, |_, meta| {
        meta.view_options =
            (data != tally_model::ViewOptionsData::default()).then(|| Arc::new(data.clone()));
        Ok(())
    })
}

/// The view options of a sheet.
pub fn view_options(wb: &Workbook, sheet: SheetId) -> Option<ViewOptionsDto> {
    let s = wb.sheet(sheet)?;
    let d = s.meta.view_options.as_deref().cloned().unwrap_or_default();
    Some(ViewOptionsDto {
        show_formulas: d.show_formulas,
        value_highlighting: d.value_highlighting,
        show_zero_values: d.show_zero_values,
        show_gridlines: d.show_gridlines,
        show_headers: d.show_headers,
        split: s.meta.split.map(|sp| SplitDto {
            x_px: crate::units::hmm_to_px(sp.x_hmm),
            y_px: crate::units::hmm_to_px(sp.y_hmm),
        }),
    })
}

/// `Op::SetSplit`.
pub fn set_split(
    wb: &mut Workbook,
    sheet: SheetId,
    split: Option<SplitDto>,
    fx: &mut Effects,
) -> Result<Vec<Change>, ApiError> {
    check_sheet(wb, sheet)?;
    let data = split.and_then(|s| {
        if !s.x_px.is_finite() || !s.y_px.is_finite() || s.x_px < 0.0 || s.y_px < 0.0 {
            return None;
        }
        let d = tally_model::SplitData {
            x_hmm: crate::units::px_to_hmm(s.x_px),
            y_hmm: crate::units::px_to_hmm(s.y_px),
        };
        (d.x_hmm > 0 || d.y_hmm > 0).then_some(d)
    });
    if split.is_some() && data.is_none() {
        return Err(ApiError::invalid_argument("bad split position"));
    }
    with_meta(wb, sheet, fx, |_, meta| {
        meta.split = data;
        Ok(())
    })
}

/// `Op::SetCustomSortLists`.
pub fn set_custom_sort_lists(
    wb: &mut Workbook,
    lists: &[Vec<String>],
) -> Result<Vec<Change>, ApiError> {
    if lists.len() > MAX_SORT_LISTS
        || lists
            .iter()
            .any(|l| l.len() > MAX_SORT_LISTS || l.iter().any(|e| e.len() > 1024))
    {
        return Err(ApiError::invalid_argument(format!(
            "at most {MAX_SORT_LISTS} lists of {MAX_SORT_LISTS} entries"
        )));
    }
    let old = std::mem::replace(&mut wb.custom_sort_lists, lists.to_vec());
    Ok(vec![Change::CustomSortLists(old)])
}

/// The data block around `(r, c)` in column `c`: the contiguous run of
/// non-empty rows (AutoInput looks in it).
pub fn data_block_rows(wb: &Workbook, sheet: SheetId, r: u32, c: u32) -> (u32, u32) {
    let Some(s) = wb.sheet(sheet) else {
        return (r, r);
    };
    let non_empty = |row: u32| s.is_non_empty(row, c);
    let mut top = r;
    while top > 0 && non_empty(top - 1) {
        top -= 1;
    }
    let mut bottom = r;
    while bottom + 1 < MAX_ROWS && non_empty(bottom + 1) {
        bottom += 1;
    }
    (top, bottom)
}

/// The meta kinds a sheet copy touches (for tests).
pub fn copied_meta_kinds() -> &'static [MetaKind] {
    &[MetaKind::Merges, MetaKind::Notes, MetaKind::Links]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregation_semantics() {
        let vals = [
            Value::Number(1.0),
            Value::Number(3.0),
            Value::text("x"),
            Value::Empty,
        ];
        assert_eq!(aggregate(AggFnData::Sum, vals.clone()), Value::Number(4.0));
        assert_eq!(
            aggregate(AggFnData::Count, vals.clone()),
            Value::Number(3.0)
        );
        assert_eq!(
            aggregate(AggFnData::CountNums, vals.clone()),
            Value::Number(2.0)
        );
        assert_eq!(
            aggregate(AggFnData::Average, vals.clone()),
            Value::Number(2.0)
        );
        assert_eq!(aggregate(AggFnData::Var, vals.clone()), Value::Number(2.0));
        assert_eq!(
            aggregate(AggFnData::StDev, [Value::Number(1.0)]),
            Value::Error(ErrorKind::Div0)
        );
        assert_eq!(aggregate(AggFnData::Sum, []), Value::Empty);
    }

    #[test]
    fn field_splitting() {
        let mut o = CsvImportOptionsDto {
            delimiters: ";".into(),
            ..CsvImportOptionsDto::default()
        };
        assert_eq!(split_fields("a;\"b;c\";d", &o), vec!["a", "b;c", "d"]);
        o.delimiters = " ".into();
        o.merge_delimiters = true;
        assert_eq!(split_fields("a   b c", &o), vec!["a", "b", "c"]);
        o.fixed_widths = Some(vec![0, 2, 5]);
        assert_eq!(split_fields("abcdefg", &o), vec!["ab", "cde", "fg"]);
    }

    #[test]
    fn labels_to_names() {
        assert_eq!(name_from_label("Net Sales").as_deref(), Some("Net_Sales"));
        assert_eq!(name_from_label("2024").as_deref(), Some("_2024"));
        assert_eq!(name_from_label("  "), None);
    }
}
