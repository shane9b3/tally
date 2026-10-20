//! Conditional formatting: the engine form of the rules, conversion to and
//! from the API and the model, and evaluation for the cells the grid asks
//! for.
//!
//! Evaluation is lazy and viewport-bound: `cells()` evaluates the rules of
//! the requested cells only. Rules that need the whole range (colour
//! scales, data bars, icon sets, top/bottom N, averages, duplicates) use
//! per-format aggregates cached per document version in [`Transient`].
//! Rule formulas are [`FExpr`]s written for the top-left cell of the first
//! range and evaluated at each cell (relative references move with it).
//!
//! Cost controls (security review 3, CRITICAL-1), all per document
//! version in [`Transient`]: a [`RangeIndex`] per sheet finds the formats
//! covering a cell in O(log n + k); the absolute subexpressions of a rule
//! formula (`SUM($A:$A)`) are folded to literals once
//! ([`FExpr::fold_absolute`]); the outcome of every evaluated cell is
//! cached; and each `cells()` call has a work budget
//! ([`MAX_CF_WORK_PER_CALL`]) past which the remaining cells are left
//! unformatted and the block reports `cf_truncated`.
//!
//! Matching style rules combine by priority: the first rule (in format
//! order, then rule order) that sets a property wins for that property.
//! Colour scales set the fill, if no higher rule did; the first data bar
//! and the first icon set apply. `stop_if_true` ends the evaluation.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tally_api::{
    ApiError, CfColorStop, CfDatePeriod, CfRuleDto, CfRuleKind, CfStyleDto, CfTextOp, CfThreshold,
    CfValueKind, CmpOp, ConditionalFormatDto, DataBarView, IconSetKind, IconView, RangeDto,
};
use tally_model::{
    BordersData, CellRangeLocal, CfColorStopData, CfDatePeriodData, CfKindData, CfRuleData,
    CfStyleData, CfTextOpData, CfThresholdData, CfValueKindData, CmpOpData, ConditionalFormatData,
    DisplayData, IconSetData, StyleData,
};
use tally_numfmt::{general_string, parse_format, FormatAst};
use tally_types::{CellAddr, CellRange, SheetId, Value, MAX_COLS, MAX_ROWS};

use crate::fexpr::{eval_formula_counted, number_of, truthy, FExpr};
use crate::rangeindex::RangeIndex;
use crate::storage::{ColumnStore, StyleId};
use crate::workbook::Workbook;

/// Style ids at or above this are synthesized conditional styles.
pub const CF_STYLE_BASE: u32 = 0x4000_0000;
/// Most conditional formats per sheet (security review 3, CRITICAL-1:
/// 10,000 formats covering one viewport took a minute per fetch; the
/// readers and `AddConditionalFormat` drop extras with a warning).
pub const MAX_CF_PER_SHEET: usize = 2_048;
/// Most rules per conditional format.
pub const MAX_RULES: usize = 64;
/// Most ranges per conditional format.
pub const MAX_CF_RANGES: usize = 64;
/// Most ranges per validation.
pub const MAX_RANGES: usize = 1024;
/// Work budget of one `cells()` call for conditional formats, in the
/// evaluator's element operations plus [`RULE_COST`] per rule evaluated.
/// Past it the remaining cells of the call are not decorated and the
/// block carries `cf_truncated` (CRITICAL-1 (2)).
pub const MAX_CF_WORK_PER_CALL: u64 = 8_000_000;
/// Fixed charge per rule evaluation (context set-up and the call itself;
/// a simple rule such as `A1>0` costs about 200 ns in a release build,
/// so the budget bounds one call at roughly 80 ms of rule evaluation).
pub const RULE_COST: u64 = 20;
/// Most cached cell outcomes per document version.
const MAX_CACHED_OUTCOMES: usize = 1 << 18;

/// A conditional format.
#[derive(Clone, Debug, PartialEq)]
pub struct CondFormat {
    /// Id, unique per sheet.
    pub id: u32,
    /// Target ranges (non-empty).
    pub ranges: Vec<CellRangeLocal>,
    /// The cell the formulas' relative offsets were written for; kept up
    /// to date by row/column changes (used to adjust references).
    pub origin: CellAddr,
    /// Rules.
    pub rules: Vec<CfRule>,
}

/// One rule.
#[derive(Clone, Debug, PartialEq)]
pub struct CfRule {
    /// The test.
    pub kind: CfKind,
    /// The differential style.
    pub style: CfStyleData,
    /// Stop after a match.
    pub stop_if_true: bool,
}

/// A threshold with its operand.
#[derive(Clone, Debug, PartialEq)]
pub struct Threshold {
    /// Kind.
    pub kind: CfValueKindData,
    /// Operand (`None` for min/max).
    pub value: Option<FExpr>,
}

/// The test of a rule.
#[derive(Clone, Debug, PartialEq)]
#[allow(missing_docs)]
pub enum CfKind {
    CellValue {
        op: CmpOpData,
        operands: Vec<FExpr>,
    },
    Formula(FExpr),
    ColorScale(Vec<(Threshold, u32)>),
    DataBar {
        min: Threshold,
        max: Threshold,
        color: u32,
        negative_color: Option<u32>,
        show_value: bool,
        gradient: bool,
    },
    IconSet {
        set: IconSetData,
        thresholds: Vec<Threshold>,
        show_value: bool,
        reverse: bool,
    },
    Duplicate,
    Unique,
    Top {
        rank: u32,
        percent: bool,
        bottom: bool,
    },
    Average {
        above: bool,
        equal: bool,
    },
    Text {
        op: CfTextOpData,
        text: String,
    },
    Blank,
    NotBlank,
    Error,
    NotError,
    DateIs(CfDatePeriodData),
}

impl CfKind {
    /// Rules that read the whole range (need [`Agg`]).
    fn needs_agg(&self) -> bool {
        matches!(
            self,
            CfKind::ColorScale(_)
                | CfKind::DataBar { .. }
                | CfKind::IconSet { .. }
                | CfKind::Duplicate
                | CfKind::Unique
                | CfKind::Top { .. }
                | CfKind::Average { .. }
        )
    }
}

impl CondFormat {
    /// The top-left cell of the first range (the base of the UI text).
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

    /// Every formula of the rules, mutably (for reference adjustment).
    pub fn formulas_mut(&mut self) -> Vec<&mut FExpr> {
        let mut out: Vec<&mut FExpr> = Vec::new();
        for rule in &mut self.rules {
            match &mut rule.kind {
                CfKind::CellValue { operands, .. } => out.extend(operands.iter_mut()),
                CfKind::Formula(f) => out.push(f),
                CfKind::ColorScale(stops) => {
                    out.extend(stops.iter_mut().filter_map(|(t, _)| t.value.as_mut()));
                }
                CfKind::DataBar { min, max, .. } => {
                    out.extend(min.value.as_mut());
                    out.extend(max.value.as_mut());
                }
                CfKind::IconSet { thresholds, .. } => {
                    out.extend(thresholds.iter_mut().filter_map(|t| t.value.as_mut()));
                }
                _ => {}
            }
        }
        out
    }
}

// ---------------------------------------------------------------------------
// Enum conversions (API ⇄ model)
// ---------------------------------------------------------------------------

/// API → model comparison.
pub fn cmp_from_api(op: CmpOp) -> CmpOpData {
    match op {
        CmpOp::Between => CmpOpData::Between,
        CmpOp::NotBetween => CmpOpData::NotBetween,
        CmpOp::Eq => CmpOpData::Eq,
        CmpOp::Ne => CmpOpData::Ne,
        CmpOp::Gt => CmpOpData::Gt,
        CmpOp::Ge => CmpOpData::Ge,
        CmpOp::Lt => CmpOpData::Lt,
        CmpOp::Le => CmpOpData::Le,
    }
}

/// Model → API comparison.
pub fn cmp_to_api(op: CmpOpData) -> CmpOp {
    match op {
        CmpOpData::Between => CmpOp::Between,
        CmpOpData::NotBetween => CmpOp::NotBetween,
        CmpOpData::Eq => CmpOp::Eq,
        CmpOpData::Ne => CmpOp::Ne,
        CmpOpData::Gt => CmpOp::Gt,
        CmpOpData::Ge => CmpOp::Ge,
        CmpOpData::Lt => CmpOp::Lt,
        CmpOpData::Le => CmpOp::Le,
    }
}

/// Number of operands an operator takes.
pub fn operand_count(op: CmpOpData) -> usize {
    match op {
        CmpOpData::Between | CmpOpData::NotBetween => 2,
        _ => 1,
    }
}

fn text_op_from_api(op: CfTextOp) -> CfTextOpData {
    match op {
        CfTextOp::Contains => CfTextOpData::Contains,
        CfTextOp::NotContains => CfTextOpData::NotContains,
        CfTextOp::BeginsWith => CfTextOpData::BeginsWith,
        CfTextOp::EndsWith => CfTextOpData::EndsWith,
    }
}

fn text_op_to_api(op: CfTextOpData) -> CfTextOp {
    match op {
        CfTextOpData::Contains => CfTextOp::Contains,
        CfTextOpData::NotContains => CfTextOp::NotContains,
        CfTextOpData::BeginsWith => CfTextOp::BeginsWith,
        CfTextOpData::EndsWith => CfTextOp::EndsWith,
    }
}

macro_rules! map_enum {
    ($from:ident, $to:ident, $a:ty, $b:ty, [$($v:ident),* $(,)?]) => {
        fn $from(x: $a) -> $b {
            match x { $(<$a>::$v => <$b>::$v,)* }
        }
        fn $to(x: $b) -> $a {
            match x { $(<$b>::$v => <$a>::$v,)* }
        }
    };
}

map_enum!(
    period_from_api,
    period_to_api,
    CfDatePeriod,
    CfDatePeriodData,
    [
        Today, Yesterday, Tomorrow, Last7Days, ThisWeek, LastWeek, NextWeek, ThisMonth, LastMonth,
        NextMonth, ThisYear, LastYear, NextYear
    ]
);

map_enum!(
    vkind_from_api,
    vkind_to_api,
    CfValueKind,
    CfValueKindData,
    [Min, Max, Percent, Percentile, Number, Formula, AutoMin, AutoMax]
);

map_enum!(
    icons_from_api,
    icons_to_api,
    IconSetKind,
    IconSetData,
    [
        ThreeArrows,
        ThreeArrowsGray,
        ThreeFlags,
        ThreeTrafficLights,
        ThreeTrafficLightsRimmed,
        ThreeSigns,
        ThreeSymbols,
        ThreeSymbolsUncircled,
        ThreeStars,
        ThreeTriangles,
        FourArrows,
        FourArrowsGray,
        FourRedToBlack,
        FourRating,
        FourTrafficLights,
        FiveArrows,
        FiveArrowsGray,
        FiveRating,
        FiveQuarters,
        FiveBoxes
    ]
);

/// API icon set → model.
pub fn icon_set_from_api(s: IconSetKind) -> IconSetData {
    icons_from_api(s)
}

fn style_from_api(s: &CfStyleDto) -> Result<CfStyleData, ApiError> {
    if let Some(code) = &s.number_format {
        if code.chars().count() > tally_model::MAX_NUMBER_FORMAT_LEN {
            return Err(ApiError::invalid_argument(format!(
                "number format is longer than {} characters",
                tally_model::MAX_NUMBER_FORMAT_LEN
            )));
        }
        parse_format(code)
            .map_err(|e| ApiError::invalid_argument(format!("bad number format: {e}")))?;
    }
    Ok(CfStyleData {
        bold: s.bold,
        italic: s.italic,
        underline: s.underline,
        strike: s.strike,
        color: s.color,
        fill: s.fill,
        borders: s.borders.as_ref().map(|b| BordersData {
            top: b.top.as_ref().map(crate::styles::border_from_dto),
            bottom: b.bottom.as_ref().map(crate::styles::border_from_dto),
            left: b.left.as_ref().map(crate::styles::border_from_dto),
            right: b.right.as_ref().map(crate::styles::border_from_dto),
            diagonal_down: b.diagonal_down.as_ref().map(crate::styles::border_from_dto),
            diagonal_up: b.diagonal_up.as_ref().map(crate::styles::border_from_dto),
        }),
        number_format: s.number_format.clone(),
        display: s.display.as_ref().map(|d| match d {
            tally_api::DisplayDto::Chip { bg, fg } => DisplayData::Chip { bg: *bg, fg: *fg },
        }),
    })
}

fn style_to_api(s: &CfStyleData) -> CfStyleDto {
    CfStyleDto {
        bold: s.bold,
        italic: s.italic,
        underline: s.underline,
        strike: s.strike,
        color: s.color,
        fill: s.fill,
        borders: s.borders.as_ref().map(|b| tally_api::BordersDto {
            top: b.top.as_ref().map(crate::styles::border_dto),
            bottom: b.bottom.as_ref().map(crate::styles::border_dto),
            left: b.left.as_ref().map(crate::styles::border_dto),
            right: b.right.as_ref().map(crate::styles::border_dto),
            diagonal_down: b.diagonal_down.as_ref().map(crate::styles::border_dto),
            diagonal_up: b.diagonal_up.as_ref().map(crate::styles::border_dto),
        }),
        number_format: s.number_format.clone(),
        display: s.display.as_ref().map(|d| match d {
            DisplayData::Chip { bg, fg } => tally_api::DisplayDto::Chip { bg: *bg, fg: *fg },
        }),
    }
}

// ---------------------------------------------------------------------------
// API ⇄ engine
// ---------------------------------------------------------------------------

/// Validates DTO ranges (at least one, at most `max`, inside the grid).
pub fn ranges_from_api(ranges: &[RangeDto], max: usize) -> Result<Vec<CellRangeLocal>, ApiError> {
    if ranges.is_empty() {
        return Err(ApiError::invalid_argument("at least one range is needed"));
    }
    if ranges.len() > max {
        return Err(ApiError::invalid_argument(format!(
            "too many ranges (at most {max})"
        )));
    }
    ranges
        .iter()
        .map(|r| {
            if r.r0 > r.r1 || r.c0 > r.c1 || r.r1 >= MAX_ROWS || r.c1 >= MAX_COLS {
                Err(ApiError::invalid_argument("a range is outside the grid"))
            } else {
                Ok(CellRangeLocal {
                    r0: r.r0,
                    c0: r.c0,
                    r1: r.r1,
                    c1: r.c1,
                })
            }
        })
        .collect()
}

/// Model ranges → DTOs.
pub fn ranges_to_api(ranges: &[CellRangeLocal]) -> Vec<RangeDto> {
    ranges
        .iter()
        .map(|r| RangeDto {
            r0: r.r0,
            c0: r.c0,
            r1: r.r1,
            c1: r.c1,
        })
        .collect()
}

fn threshold_from_api(
    wb: &Workbook,
    t: &CfThreshold,
    origin: CellAddr,
) -> Result<Threshold, ApiError> {
    let kind = vkind_from_api(t.kind);
    let value = match kind {
        CfValueKindData::Min
        | CfValueKindData::Max
        | CfValueKindData::AutoMin
        | CfValueKindData::AutoMax => None,
        _ => Some(FExpr::parse_ui(wb, &t.value, origin)?),
    };
    Ok(Threshold { kind, value })
}

fn threshold_to_api(wb: &Workbook, t: &Threshold, at: CellAddr) -> CfThreshold {
    CfThreshold {
        kind: vkind_to_api(t.kind),
        value: t
            .value
            .as_ref()
            .map_or_else(String::new, |f| f.print_ui(wb, at)),
    }
}

fn operands_from_api(
    wb: &Workbook,
    op: CmpOpData,
    formulas: &[String],
    origin: CellAddr,
) -> Result<Vec<FExpr>, ApiError> {
    let n = operand_count(op);
    if formulas.len() < n {
        return Err(ApiError::invalid_argument(format!(
            "the comparison needs {n} value(s)"
        )));
    }
    formulas[..n]
        .iter()
        .map(|f| FExpr::parse_operand(wb, f, origin))
        .collect()
}

/// A rule from the API (formulas relative to `origin`).
pub fn rule_from_api(wb: &Workbook, r: &CfRuleDto, origin: CellAddr) -> Result<CfRule, ApiError> {
    let kind = match &r.kind {
        CfRuleKind::CellValue { op, formulas } => {
            let op = cmp_from_api(*op);
            CfKind::CellValue {
                op,
                operands: operands_from_api(wb, op, formulas, origin)?,
            }
        }
        CfRuleKind::Formula { formula } => CfKind::Formula(FExpr::parse_ui(wb, formula, origin)?),
        CfRuleKind::ColorScale { stops } => {
            if !(2..=3).contains(&stops.len()) {
                return Err(ApiError::invalid_argument(
                    "a colour scale needs 2 or 3 stops",
                ));
            }
            CfKind::ColorScale(
                stops
                    .iter()
                    .map(|s| Ok((threshold_from_api(wb, &s.threshold, origin)?, s.color)))
                    .collect::<Result<_, ApiError>>()?,
            )
        }
        CfRuleKind::DataBar {
            min,
            max,
            color,
            negative_color,
            show_value,
            gradient,
        } => CfKind::DataBar {
            min: threshold_from_api(wb, min, origin)?,
            max: threshold_from_api(wb, max, origin)?,
            color: *color,
            negative_color: *negative_color,
            show_value: *show_value,
            gradient: *gradient,
        },
        CfRuleKind::IconSet {
            set,
            thresholds,
            show_value,
            reverse,
        } => {
            let set = icons_from_api(*set);
            if thresholds.len() != set.icon_count() {
                return Err(ApiError::invalid_argument(format!(
                    "this icon set needs {} thresholds",
                    set.icon_count()
                )));
            }
            CfKind::IconSet {
                set,
                thresholds: thresholds
                    .iter()
                    .map(|t| threshold_from_api(wb, t, origin))
                    .collect::<Result<_, _>>()?,
                show_value: *show_value,
                reverse: *reverse,
            }
        }
        CfRuleKind::Duplicate => CfKind::Duplicate,
        CfRuleKind::Unique => CfKind::Unique,
        CfRuleKind::Top {
            rank,
            percent,
            bottom,
        } => {
            if *rank == 0 || (*percent && *rank > 100) {
                return Err(ApiError::invalid_argument("bad rank"));
            }
            CfKind::Top {
                rank: *rank,
                percent: *percent,
                bottom: *bottom,
            }
        }
        CfRuleKind::Average { above, equal } => CfKind::Average {
            above: *above,
            equal: *equal,
        },
        CfRuleKind::Text { op, text } => CfKind::Text {
            op: text_op_from_api(*op),
            text: text.clone(),
        },
        CfRuleKind::Blank => CfKind::Blank,
        CfRuleKind::NotBlank => CfKind::NotBlank,
        CfRuleKind::Error => CfKind::Error,
        CfRuleKind::NotError => CfKind::NotError,
        CfRuleKind::DateIs { period } => CfKind::DateIs(period_from_api(*period)),
    };
    Ok(CfRule {
        kind,
        style: style_from_api(&r.style)?,
        stop_if_true: r.stop_if_true,
    })
}

/// A conditional format from the API.
pub fn from_api(
    wb: &Workbook,
    sheet: SheetId,
    id: u32,
    ranges: &[RangeDto],
    rules: &[CfRuleDto],
) -> Result<CondFormat, ApiError> {
    let ranges = ranges_from_api(ranges, MAX_CF_RANGES)?;
    if rules.is_empty() || rules.len() > MAX_RULES {
        return Err(ApiError::invalid_argument(format!(
            "a conditional format needs 1 to {MAX_RULES} rules"
        )));
    }
    let origin = CellAddr::new(sheet, ranges[0].r0, ranges[0].c0);
    let rules = rules
        .iter()
        .map(|r| rule_from_api(wb, r, origin))
        .collect::<Result<_, _>>()?;
    Ok(CondFormat {
        id,
        ranges,
        origin,
        rules,
    })
}

/// The API view of a conditional format.
pub fn to_api(wb: &Workbook, sheet: SheetId, cf: &CondFormat) -> ConditionalFormatDto {
    let at = cf.anchor(sheet);
    let rules = cf
        .rules
        .iter()
        .map(|r| {
            let kind = match &r.kind {
                CfKind::CellValue { op, operands } => CfRuleKind::CellValue {
                    op: cmp_to_api(*op),
                    formulas: operands.iter().map(|f| f.print_ui(wb, at)).collect(),
                },
                CfKind::Formula(f) => CfRuleKind::Formula {
                    formula: f.print_ui(wb, at),
                },
                CfKind::ColorScale(stops) => CfRuleKind::ColorScale {
                    stops: stops
                        .iter()
                        .map(|(t, c)| CfColorStop {
                            threshold: threshold_to_api(wb, t, at),
                            color: *c,
                        })
                        .collect(),
                },
                CfKind::DataBar {
                    min,
                    max,
                    color,
                    negative_color,
                    show_value,
                    gradient,
                } => CfRuleKind::DataBar {
                    min: threshold_to_api(wb, min, at),
                    max: threshold_to_api(wb, max, at),
                    color: *color,
                    negative_color: *negative_color,
                    show_value: *show_value,
                    gradient: *gradient,
                },
                CfKind::IconSet {
                    set,
                    thresholds,
                    show_value,
                    reverse,
                } => CfRuleKind::IconSet {
                    set: icons_to_api(*set),
                    thresholds: thresholds
                        .iter()
                        .map(|t| threshold_to_api(wb, t, at))
                        .collect(),
                    show_value: *show_value,
                    reverse: *reverse,
                },
                CfKind::Duplicate => CfRuleKind::Duplicate,
                CfKind::Unique => CfRuleKind::Unique,
                CfKind::Top {
                    rank,
                    percent,
                    bottom,
                } => CfRuleKind::Top {
                    rank: *rank,
                    percent: *percent,
                    bottom: *bottom,
                },
                CfKind::Average { above, equal } => CfRuleKind::Average {
                    above: *above,
                    equal: *equal,
                },
                CfKind::Text { op, text } => CfRuleKind::Text {
                    op: text_op_to_api(*op),
                    text: text.clone(),
                },
                CfKind::Blank => CfRuleKind::Blank,
                CfKind::NotBlank => CfRuleKind::NotBlank,
                CfKind::Error => CfRuleKind::Error,
                CfKind::NotError => CfRuleKind::NotError,
                CfKind::DateIs(p) => CfRuleKind::DateIs {
                    period: period_to_api(*p),
                },
            };
            CfRuleDto {
                kind,
                style: style_to_api(&r.style),
                stop_if_true: r.stop_if_true,
            }
        })
        .collect();
    ConditionalFormatDto {
        id: cf.id,
        ranges: ranges_to_api(&cf.ranges),
        rules,
    }
}

// ---------------------------------------------------------------------------
// Model ⇄ engine
// ---------------------------------------------------------------------------

fn threshold_from_model(wb: &Workbook, t: &CfThresholdData, base: CellAddr) -> Threshold {
    let value = match t.kind {
        CfValueKindData::Min
        | CfValueKindData::Max
        | CfValueKindData::AutoMin
        | CfValueKindData::AutoMax => None,
        _ => Some(match t.value.trim().parse::<f64>() {
            Ok(n) if n.is_finite() && t.kind != CfValueKindData::Formula => FExpr::number(n),
            _ => FExpr::parse_of(wb, &t.value, base),
        }),
    };
    Threshold {
        kind: t.kind,
        value,
    }
}

fn threshold_to_model(wb: &Workbook, t: &Threshold, base: CellAddr) -> CfThresholdData {
    let value = match &t.value {
        None => String::new(),
        Some(f) => match (t.kind, f.as_number()) {
            (CfValueKindData::Formula, _) | (_, None) => f.print_of(wb, base),
            (_, Some(n)) => format!("{n}"),
        },
    };
    CfThresholdData {
        kind: t.kind,
        value,
    }
}

/// Builds a conditional format from the model (invalid or empty ones give
/// `None`).
pub fn from_model(
    wb: &Workbook,
    sheet: SheetId,
    id: u32,
    d: &ConditionalFormatData,
) -> Option<CondFormat> {
    let ranges: Vec<CellRangeLocal> = d
        .ranges
        .iter()
        .copied()
        .filter(|r| r.r0 <= r.r1 && r.c0 <= r.c1 && r.r1 < MAX_ROWS && r.c1 < MAX_COLS)
        .take(MAX_CF_RANGES)
        .collect();
    if ranges.is_empty() {
        return None;
    }
    let base = CellAddr::new(
        sheet,
        d.base.0.min(MAX_ROWS - 1),
        d.base.1.min(MAX_COLS - 1),
    );
    let mut rules = Vec::new();
    for r in d.rules.iter().take(MAX_RULES) {
        let kind = match &r.kind {
            CfKindData::CellValue { op, formulas } => {
                if formulas.len() < operand_count(*op) {
                    continue;
                }
                CfKind::CellValue {
                    op: *op,
                    operands: formulas[..operand_count(*op)]
                        .iter()
                        .map(|f| FExpr::parse_of(wb, f, base))
                        .collect(),
                }
            }
            CfKindData::Formula { formula } => CfKind::Formula(FExpr::parse_of(wb, formula, base)),
            CfKindData::ColorScale { stops } => {
                if !(2..=3).contains(&stops.len()) {
                    continue;
                }
                CfKind::ColorScale(
                    stops
                        .iter()
                        .map(|s| (threshold_from_model(wb, &s.threshold, base), s.color))
                        .collect(),
                )
            }
            CfKindData::DataBar {
                min,
                max,
                color,
                negative_color,
                show_value,
                gradient,
            } => CfKind::DataBar {
                min: threshold_from_model(wb, min, base),
                max: threshold_from_model(wb, max, base),
                color: *color,
                negative_color: *negative_color,
                show_value: *show_value,
                gradient: *gradient,
            },
            CfKindData::IconSet {
                set,
                thresholds,
                show_value,
                reverse,
            } => {
                let mut th: Vec<Threshold> = thresholds
                    .iter()
                    .take(set.icon_count())
                    .map(|t| threshold_from_model(wb, t, base))
                    .collect();
                // Missing thresholds: evenly spaced percents.
                let n = set.icon_count();
                while th.len() < n {
                    let i = th.len();
                    th.push(Threshold {
                        kind: CfValueKindData::Percent,
                        value: Some(FExpr::number((100 * i / n) as f64)),
                    });
                }
                CfKind::IconSet {
                    set: *set,
                    thresholds: th,
                    show_value: *show_value,
                    reverse: *reverse,
                }
            }
            CfKindData::Duplicate => CfKind::Duplicate,
            CfKindData::Unique => CfKind::Unique,
            CfKindData::Top {
                rank,
                percent,
                bottom,
            } => CfKind::Top {
                rank: (*rank).max(1),
                percent: *percent,
                bottom: *bottom,
            },
            CfKindData::Average { above, equal } => CfKind::Average {
                above: *above,
                equal: *equal,
            },
            CfKindData::Text { op, text } => CfKind::Text {
                op: *op,
                text: text.clone(),
            },
            CfKindData::Blank => CfKind::Blank,
            CfKindData::NotBlank => CfKind::NotBlank,
            CfKindData::Error => CfKind::Error,
            CfKindData::NotError => CfKind::NotError,
            CfKindData::DateIs { period } => CfKind::DateIs(*period),
        };
        rules.push(CfRule {
            kind,
            style: r.style.clone(),
            stop_if_true: r.stop_if_true,
        });
    }
    if rules.is_empty() {
        return None;
    }
    Some(CondFormat {
        id,
        ranges,
        origin: base,
        rules,
    })
}

/// The model form (formulas relative to the top-left cell of the first
/// range).
pub fn to_model(wb: &Workbook, sheet: SheetId, cf: &CondFormat) -> ConditionalFormatData {
    let base = cf.anchor(sheet);
    let rules = cf
        .rules
        .iter()
        .map(|r| {
            let kind = match &r.kind {
                CfKind::CellValue { op, operands } => CfKindData::CellValue {
                    op: *op,
                    formulas: operands.iter().map(|f| f.print_of(wb, base)).collect(),
                },
                CfKind::Formula(f) => CfKindData::Formula {
                    formula: f.print_of(wb, base),
                },
                CfKind::ColorScale(stops) => CfKindData::ColorScale {
                    stops: stops
                        .iter()
                        .map(|(t, c)| CfColorStopData {
                            threshold: threshold_to_model(wb, t, base),
                            color: *c,
                        })
                        .collect(),
                },
                CfKind::DataBar {
                    min,
                    max,
                    color,
                    negative_color,
                    show_value,
                    gradient,
                } => CfKindData::DataBar {
                    min: threshold_to_model(wb, min, base),
                    max: threshold_to_model(wb, max, base),
                    color: *color,
                    negative_color: *negative_color,
                    show_value: *show_value,
                    gradient: *gradient,
                },
                CfKind::IconSet {
                    set,
                    thresholds,
                    show_value,
                    reverse,
                } => CfKindData::IconSet {
                    set: *set,
                    thresholds: thresholds
                        .iter()
                        .map(|t| threshold_to_model(wb, t, base))
                        .collect(),
                    show_value: *show_value,
                    reverse: *reverse,
                },
                CfKind::Duplicate => CfKindData::Duplicate,
                CfKind::Unique => CfKindData::Unique,
                CfKind::Top {
                    rank,
                    percent,
                    bottom,
                } => CfKindData::Top {
                    rank: *rank,
                    percent: *percent,
                    bottom: *bottom,
                },
                CfKind::Average { above, equal } => CfKindData::Average {
                    above: *above,
                    equal: *equal,
                },
                CfKind::Text { op, text } => CfKindData::Text {
                    op: *op,
                    text: text.clone(),
                },
                CfKind::Blank => CfKindData::Blank,
                CfKind::NotBlank => CfKindData::NotBlank,
                CfKind::Error => CfKindData::Error,
                CfKind::NotError => CfKindData::NotError,
                CfKind::DateIs(p) => CfKindData::DateIs { period: *p },
            };
            CfRuleData {
                kind,
                style: r.style.clone(),
                stop_if_true: r.stop_if_true,
            }
        })
        .collect();
    ConditionalFormatData {
        ranges: cf.ranges.clone(),
        base: (base.row, base.col),
        rules,
    }
}

// ---------------------------------------------------------------------------
// Transient state: aggregates and synthesized styles
// ---------------------------------------------------------------------------

/// A value as a hash key for duplicate counting.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum ValKey {
    Num(u64),
    Text(String),
    Bool(bool),
    Err(String),
}

fn val_key(v: &Value) -> Option<ValKey> {
    Some(match v {
        Value::Empty => return None,
        Value::Number(n) => ValKey::Num(if *n == 0.0 { 0 } else { n.to_bits() }),
        Value::Text(t) => ValKey::Text(t.to_lowercase()),
        Value::Bool(b) => ValKey::Bool(*b),
        Value::Error(e) => ValKey::Err(e.display().into_owned()),
        Value::Array(m) => return m.get(0, 0).and_then(val_key),
    })
}

/// Whole-range statistics of one conditional format.
#[derive(Debug, Default)]
pub struct Agg {
    /// The numbers of the ranges, ascending.
    nums: Vec<f64>,
    counts: HashMap<ValKey, u32>,
}

impl Agg {
    fn compute(wb: &Workbook, sheet: SheetId, cf: &CondFormat) -> Agg {
        let mut agg = Agg::default();
        let Some(s) = wb.sheet(sheet) else {
            return agg;
        };
        let want_counts = cf
            .rules
            .iter()
            .any(|r| matches!(r.kind, CfKind::Duplicate | CfKind::Unique));
        let mut seen: std::collections::HashSet<(u32, u32)> = std::collections::HashSet::new();
        let multi = cf.ranges.len() > 1;
        for r in &cf.ranges {
            let c1 = r.c1.min(s.columns.len().saturating_sub(1) as u32);
            if s.columns.is_empty() {
                continue;
            }
            for c in r.c0..=c1 {
                let Some(col) = s.column(c) else {
                    continue;
                };
                for (row, _) in col.cells.iter_range(r.r0, r.r1) {
                    if multi && !seen.insert((row, c)) {
                        continue;
                    }
                    let v = wb.value(CellAddr::new(sheet, row, c));
                    if let Value::Number(n) = v {
                        agg.nums.push(n);
                    }
                    if want_counts {
                        if let Some(k) = val_key(&v) {
                            *agg.counts.entry(k).or_insert(0) += 1;
                        }
                    }
                }
            }
        }
        agg.nums.sort_by(f64::total_cmp);
        agg
    }

    fn min(&self) -> Option<f64> {
        self.nums.first().copied()
    }

    fn max(&self) -> Option<f64> {
        self.nums.last().copied()
    }

    fn mean(&self) -> Option<f64> {
        (!self.nums.is_empty()).then(|| self.nums.iter().sum::<f64>() / self.nums.len() as f64)
    }

    /// Linear-interpolated percentile (`p` in 0..=100), as PERCENTILE.
    fn percentile(&self, p: f64) -> Option<f64> {
        let n = self.nums.len();
        if n == 0 {
            return None;
        }
        let p = (p / 100.0).clamp(0.0, 1.0);
        let x = p * (n - 1) as f64;
        let i = x.floor() as usize;
        let f = x - i as f64;
        let a = self.nums[i];
        let b = self.nums[(i + 1).min(n - 1)];
        Some(a + (b - a) * f)
    }
}

/// A synthesized style (base style + conditional patch).
#[derive(Clone, Debug)]
pub struct CfStyleEntry {
    /// The full style.
    pub data: StyleData,
    /// Its number format code.
    pub format_code: Arc<str>,
    /// Parsed number format.
    pub format: Arc<FormatAst>,
}

/// What the conditional formats do to one cell, as the grid needs it
/// (the synthesized style id instead of the patch).
#[derive(Clone, Debug, PartialEq)]
pub struct CfDecor {
    /// The synthesized style (`CF_STYLE_BASE + i`), if a style applies.
    pub style: Option<u32>,
    /// The style sets a number format (the view text is re-rendered).
    pub number_format: bool,
    /// A data bar.
    pub bar: Option<DataBarView>,
    /// An icon.
    pub icon: Option<IconView>,
    /// The value is hidden.
    pub hide_value: bool,
}

#[derive(Debug, Default)]
struct TransientInner {
    version: u64,
    aggs: HashMap<(SheetId, u32), Arc<Agg>>,
    styles: Vec<CfStyleEntry>,
    style_index: HashMap<String, u32>,
    /// Per sheet: the index of the format ranges (format position → id).
    index: HashMap<SheetId, Arc<RangeIndex<u32>>>,
    /// Folded rule formulas by the address of the original `Formula`
    /// (stable within a version: the meta is immutable between versions).
    folded: HashMap<usize, Option<Arc<tally_formula::Formula>>>,
    /// Evaluated cells: `None` when nothing applies.
    outcomes: HashMap<(SheetId, u32, u32), Option<CfDecor>>,
}

impl TransientInner {
    fn at_version(&mut self, version: u64) {
        if self.version != version {
            self.version = version;
            self.aggs.clear();
            self.index.clear();
            self.folded.clear();
            self.outcomes.clear();
        }
    }
}

/// Per-document caches that are not document state: conditional-format
/// aggregates (valid for one document version) and the synthesized style
/// table. Cloning gives an empty cache.
#[derive(Default)]
pub struct Transient {
    inner: Mutex<TransientInner>,
}

impl Clone for Transient {
    fn clone(&self) -> Self {
        Transient::default()
    }
}

impl std::fmt::Debug for Transient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Transient")
    }
}

/// Most synthesized styles kept (the table is cleared beyond this).
const MAX_CF_STYLES: usize = 65_536;

impl Transient {
    fn lock(&self) -> std::sync::MutexGuard<'_, TransientInner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn agg(&self, wb: &Workbook, version: u64, sheet: SheetId, cf: &CondFormat) -> Arc<Agg> {
        {
            let mut g = self.lock();
            g.at_version(version);
            if let Some(a) = g.aggs.get(&(sheet, cf.id)) {
                return a.clone();
            }
        }
        let a = Arc::new(Agg::compute(wb, sheet, cf));
        let mut g = self.lock();
        g.at_version(version);
        g.aggs.insert((sheet, cf.id), a.clone());
        a
    }

    /// The range index of `sheet`'s formats (positions in
    /// `meta.cond_formats`), built once per version.
    fn index(&self, wb: &Workbook, version: u64, sheet: SheetId) -> Arc<RangeIndex<u32>> {
        {
            let mut g = self.lock();
            g.at_version(version);
            if let Some(i) = g.index.get(&sheet) {
                return i.clone();
            }
        }
        let items: Vec<(CellRangeLocal, u32)> = wb
            .sheet(sheet)
            .map(|s| {
                s.meta
                    .cond_formats
                    .iter()
                    .enumerate()
                    .flat_map(|(i, cf)| cf.ranges.iter().map(move |r| (*r, i as u32)))
                    .collect()
            })
            .unwrap_or_default();
        let idx = Arc::new(RangeIndex::build(items));
        let mut g = self.lock();
        g.at_version(version);
        g.index.insert(sheet, idx.clone());
        idx
    }

    /// `f` with its absolute subexpressions folded at `at` (cached per
    /// version by the formula's identity); the work spent folding.
    fn folded(
        &self,
        wb: &Workbook,
        version: u64,
        f: &FExpr,
        at: CellAddr,
    ) -> (Arc<tally_formula::Formula>, u64) {
        let key = Arc::as_ptr(&f.formula) as usize;
        {
            let mut g = self.lock();
            g.at_version(version);
            if let Some(cached) = g.folded.get(&key) {
                return (cached.clone().unwrap_or_else(|| f.formula.clone()), 0);
            }
        }
        let folded = f.fold_absolute(wb, at);
        let out = folded.clone().unwrap_or_else(|| f.formula.clone());
        let mut g = self.lock();
        g.at_version(version);
        if g.folded.len() < MAX_CACHED_OUTCOMES {
            g.folded.insert(key, folded);
        }
        // Folding evaluated the absolute parts once; charge it as one
        // rule evaluation.
        (out, RULE_COST)
    }

    fn cached_outcome(&self, version: u64, key: (SheetId, u32, u32)) -> Option<Option<CfDecor>> {
        let mut g = self.lock();
        g.at_version(version);
        g.outcomes.get(&key).cloned()
    }

    fn remember_outcome(&self, version: u64, key: (SheetId, u32, u32), d: Option<CfDecor>) {
        let mut g = self.lock();
        g.at_version(version);
        if g.outcomes.len() < MAX_CACHED_OUTCOMES {
            g.outcomes.insert(key, d);
        }
    }

    /// The synthesized style id for `base` with `patch` applied.
    pub fn style_for(&self, base: &StyleData, patch: &CfStyleData) -> (u32, CfStyleEntry) {
        let key = format!("{base:?}|{patch:?}");
        let mut g = self.lock();
        if let Some(&i) = g.style_index.get(&key) {
            if let Some(e) = g.styles.get(i as usize) {
                return (CF_STYLE_BASE + i, e.clone());
            }
        }
        if g.styles.len() >= MAX_CF_STYLES {
            g.styles.clear();
            g.style_index.clear();
        }
        let mut data = base.clone();
        apply_patch(&mut data, patch);
        let code: Arc<str> = data.number_format.as_deref().unwrap_or("General").into();
        let format = Arc::new(
            parse_format(&code)
                .or_else(|_| parse_format("General"))
                .unwrap_or(FormatAst {
                    sections: Vec::new(),
                }),
        );
        let entry = CfStyleEntry {
            data,
            format_code: code,
            format,
        };
        let i = g.styles.len() as u32;
        g.styles.push(entry.clone());
        g.style_index.insert(key, i);
        (CF_STYLE_BASE + i, entry)
    }

    /// A synthesized style by id.
    pub fn style(&self, id: u32) -> Option<CfStyleEntry> {
        let i = id.checked_sub(CF_STYLE_BASE)?;
        self.lock().styles.get(i as usize).cloned()
    }
}

/// Applies a differential style.
pub fn apply_patch(d: &mut StyleData, p: &CfStyleData) {
    if let Some(v) = p.bold {
        d.font.bold = v;
    }
    if let Some(v) = p.italic {
        d.font.italic = v;
    }
    if let Some(v) = p.underline {
        d.font.underline = v;
    }
    if let Some(v) = p.strike {
        d.font.strike = v;
    }
    if let Some(v) = p.color {
        d.font.color = Some(v);
    }
    if let Some(v) = p.fill {
        d.fill = Some(v);
    }
    if let Some(b) = &p.borders {
        for (slot, edge) in [
            (&mut d.borders.top, &b.top),
            (&mut d.borders.bottom, &b.bottom),
            (&mut d.borders.left, &b.left),
            (&mut d.borders.right, &b.right),
        ] {
            if edge.is_some() {
                slot.clone_from(edge);
            }
        }
    }
    if let Some(v) = &p.number_format {
        d.number_format = Some(v.clone());
    }
    if let Some(v) = &p.display {
        d.display = Some(v.clone());
    }
}

/// Fills the unset fields of `acc` from `p` (higher priority first).
fn merge_under(acc: &mut CfStyleData, p: &CfStyleData) {
    macro_rules! under {
        ($($f:ident),*) => { $( if acc.$f.is_none() { acc.$f.clone_from(&p.$f); } )* };
    }
    under!(
        bold,
        italic,
        underline,
        strike,
        color,
        fill,
        number_format,
        display
    );
    if let Some(b) = &p.borders {
        let a = acc.borders.get_or_insert_with(BordersData::default);
        for (slot, edge) in [
            (&mut a.top, &b.top),
            (&mut a.bottom, &b.bottom),
            (&mut a.left, &b.left),
            (&mut a.right, &b.right),
        ] {
            if slot.is_none() {
                slot.clone_from(edge);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Evaluation
// ---------------------------------------------------------------------------

/// What the conditional formats do to one cell.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CfOutcome {
    /// The combined differential style.
    pub style: CfStyleData,
    /// A data bar.
    pub bar: Option<DataBarView>,
    /// An icon.
    pub icon: Option<IconView>,
    /// The value is hidden (data bar / icon set without the value).
    pub hide_value: bool,
}

impl CfOutcome {
    /// `true` if nothing applies.
    pub fn is_empty(&self) -> bool {
        self.style.is_empty() && self.bar.is_none() && self.icon.is_none()
    }
}

/// Orders two values for comparisons: numbers (and booleans, and empty as
/// 0) before text; text case-insensitively; `None` for errors.
fn order(v: &Value, other: &Value) -> Option<std::cmp::Ordering> {
    use std::cmp::Ordering;
    let norm = |x: &Value, peer: &Value| -> Option<Value> {
        Some(match x {
            Value::Empty => match peer {
                Value::Text(_) => Value::text(""),
                _ => Value::Number(0.0),
            },
            Value::Bool(b) => Value::Number(f64::from(u8::from(*b))),
            Value::Error(_) => return None,
            Value::Array(m) => return m.get(0, 0).cloned(),
            other => other.clone(),
        })
    };
    let a = norm(v, other)?;
    let b = norm(other, v)?;
    match (&a, &b) {
        (Value::Number(x), Value::Number(y)) => x.partial_cmp(y),
        (Value::Text(x), Value::Text(y)) => Some(x.to_lowercase().cmp(&y.to_lowercase())),
        (Value::Number(_), Value::Text(_)) => Some(Ordering::Less),
        (Value::Text(_), Value::Number(_)) => Some(Ordering::Greater),
        _ => None,
    }
}

/// `true` if `v` passes `op` against the operands.
pub fn compare(v: &Value, op: CmpOpData, a: &Value, b: Option<&Value>) -> bool {
    use std::cmp::Ordering::*;
    let Some(oa) = order(v, a) else {
        return false;
    };
    match op {
        CmpOpData::Eq => oa == Equal,
        CmpOpData::Ne => oa != Equal,
        CmpOpData::Gt => oa == Greater,
        CmpOpData::Ge => oa != Less,
        CmpOpData::Lt => oa == Less,
        CmpOpData::Le => oa != Greater,
        CmpOpData::Between | CmpOpData::NotBetween => {
            let Some(b) = b else {
                return false;
            };
            let Some(ob) = order(v, b) else {
                return false;
            };
            // The bounds may be given in either order.
            let lo_hi = order(a, b).is_none_or(|o| o != Greater);
            let inside = if lo_hi {
                oa != Less && ob != Greater
            } else {
                ob != Less && oa != Greater
            };
            inside == (op == CmpOpData::Between)
        }
    }
}

fn value_text(v: &Value) -> Option<String> {
    Some(match v {
        Value::Empty => String::new(),
        Value::Number(n) => general_string(*n, None, &tally_numfmt::EN_US),
        Value::Text(t) => t.to_string(),
        Value::Bool(b) => if *b { "TRUE" } else { "FALSE" }.into(),
        Value::Error(_) => return None,
        Value::Array(m) => return m.get(0, 0).and_then(value_text),
    })
}

fn date_matches(wb: &Workbook, serial: f64, period: CfDatePeriodData) -> bool {
    use tally_numfmt::date::{weekday, ymd_from_serial};
    let epoch = wb.settings.date_epoch;
    let today = tally_numfmt::date::now_serial(epoch).floor();
    let d = serial.floor();
    let week_start = |t: f64| t - f64::from(weekday(epoch, t).unwrap_or(1) - 1);
    let ym = |s: f64| ymd_from_serial(epoch, s).map(|(y, m, _)| (i64::from(y), i64::from(m)));
    let month_index = |s: f64| ym(s).map(|(y, m)| y * 12 + m - 1);
    match period {
        CfDatePeriodData::Today => d == today,
        CfDatePeriodData::Yesterday => d == today - 1.0,
        CfDatePeriodData::Tomorrow => d == today + 1.0,
        CfDatePeriodData::Last7Days => d <= today && d > today - 7.0,
        CfDatePeriodData::ThisWeek => week_start(d) == week_start(today),
        CfDatePeriodData::LastWeek => week_start(d) == week_start(today) - 7.0,
        CfDatePeriodData::NextWeek => week_start(d) == week_start(today) + 7.0,
        CfDatePeriodData::ThisMonth | CfDatePeriodData::LastMonth | CfDatePeriodData::NextMonth => {
            let delta = match period {
                CfDatePeriodData::LastMonth => -1,
                CfDatePeriodData::NextMonth => 1,
                _ => 0,
            };
            match (month_index(d), month_index(today)) {
                (Some(a), Some(b)) => a == b + delta,
                _ => false,
            }
        }
        CfDatePeriodData::ThisYear | CfDatePeriodData::LastYear | CfDatePeriodData::NextYear => {
            let delta = match period {
                CfDatePeriodData::LastYear => -1,
                CfDatePeriodData::NextYear => 1,
                _ => 0,
            };
            match (ym(d), ym(today)) {
                (Some((a, _)), Some((b, _))) => a == b + delta,
                _ => false,
            }
        }
    }
}

/// Linear interpolation of two `0xRRGGBBAA` colours.
fn lerp_color(a: u32, b: u32, t: f64) -> u32 {
    let t = t.clamp(0.0, 1.0);
    let mut out = 0u32;
    for shift in [24, 16, 8, 0] {
        let x = f64::from((a >> shift) & 0xff);
        let y = f64::from((b >> shift) & 0xff);
        let v = (x + (y - x) * t).round().clamp(0.0, 255.0) as u32;
        out |= v << shift;
    }
    out
}

/// `true` if applying `p` under `acc` could change `acc` (every property
/// `p` sets is still unset in `acc`-wise). Rules that cannot contribute
/// are not evaluated.
fn can_contribute(p: &CfStyleData, acc: &CfStyleData) -> bool {
    macro_rules! any_new {
        ($($f:ident),*) => { false $(|| (p.$f.is_some() && acc.$f.is_none()))* };
    }
    if any_new!(
        bold,
        italic,
        underline,
        strike,
        color,
        fill,
        number_format,
        display
    ) {
        return true;
    }
    match (&p.borders, &acc.borders) {
        (Some(_), None) => true,
        (Some(b), Some(a)) => {
            (b.top.is_some() && a.top.is_none())
                || (b.bottom.is_some() && a.bottom.is_none())
                || (b.left.is_some() && a.left.is_none())
                || (b.right.is_some() && a.right.is_none())
        }
        (None, _) => false,
    }
}

/// Evaluates conditional formats for the cells of one sheet within one
/// `cells()` call: one work budget, the per-version caches of
/// [`Transient`], and the format index of the sheet.
pub struct CfEval<'a> {
    wb: &'a Workbook,
    sheet: SheetId,
    version: u64,
    index: Arc<RangeIndex<u32>>,
    spent: Cell<u64>,
    budget: u64,
    truncated: Cell<bool>,
    candidates: RefCell<Vec<u32>>,
}

impl<'a> CfEval<'a> {
    /// An evaluator for `sheet` at document `version` (aggregates, the
    /// index, folded formulas and outcomes are cached per version).
    pub fn new(wb: &'a Workbook, sheet: SheetId, version: u64) -> Self {
        Self::with_budget(wb, sheet, version, MAX_CF_WORK_PER_CALL)
    }

    /// [`Self::new`] with a custom work budget (tests).
    pub fn with_budget(wb: &'a Workbook, sheet: SheetId, version: u64, budget: u64) -> Self {
        let has = wb
            .sheet(sheet)
            .is_some_and(|s| !s.meta.cond_formats.is_empty());
        let index = if has {
            wb.transient.index(wb, version, sheet)
        } else {
            Arc::new(RangeIndex::default())
        };
        Self {
            wb,
            sheet,
            version,
            index,
            spent: Cell::new(0),
            budget,
            truncated: Cell::new(false),
            candidates: RefCell::new(Vec::new()),
        }
    }

    /// `true` once the call's work budget ran out: cells evaluated after
    /// that are left unformatted.
    pub fn truncated(&self) -> bool {
        self.truncated.get()
    }

    /// Work spent so far (tests).
    pub fn work(&self) -> u64 {
        self.spent.get()
    }

    /// The distinct rectangles of the sheet's formats intersecting `r`.
    pub fn rects_in(&self, r: &CellRangeLocal) -> Vec<CellRangeLocal> {
        self.index.intersections(r)
    }

    /// Charges `n`; `false` when the budget is spent.
    fn charge(&self, n: u64) -> bool {
        let w = self.spent.get().saturating_add(n);
        self.spent.set(w);
        if w > self.budget {
            self.truncated.set(true);
            return false;
        }
        true
    }

    /// Evaluates a rule formula at `addr`, folded and charged. `None`
    /// when the budget is spent.
    fn eval(&self, f: &FExpr, anchor: CellAddr, addr: CellAddr) -> Option<Value> {
        if f.invalid.is_some() {
            return self
                .charge(1)
                .then_some(Value::Error(tally_types::ErrorKind::Name));
        }
        let (formula, fold_cost) = self.wb.transient.folded(self.wb, self.version, f, anchor);
        if !self.charge(fold_cost + RULE_COST) {
            return None;
        }
        let (v, work) = eval_formula_counted(self.wb, &formula, addr);
        self.charge(work);
        Some(v)
    }

    fn threshold(&self, t: &Threshold, agg: &Agg, at: CellAddr) -> Option<f64> {
        let operand = || {
            t.value
                .as_ref()
                .and_then(|f| number_of(&self.eval(f, at, at)?))
        };
        match t.kind {
            CfValueKindData::Min => agg.min(),
            CfValueKindData::Max => agg.max(),
            CfValueKindData::AutoMin => agg.min().map(|m| m.min(0.0)),
            CfValueKindData::AutoMax => agg.max().map(|m| m.max(0.0)),
            CfValueKindData::Number | CfValueKindData::Formula => operand(),
            CfValueKindData::Percent => {
                let (lo, hi) = (agg.min()?, agg.max()?);
                Some(lo + (hi - lo) * operand()? / 100.0)
            }
            CfValueKindData::Percentile => agg.percentile(operand()?),
        }
    }

    /// Whether `rule` matches at `addr`; `None` when the budget is spent.
    fn rule_matches(
        &self,
        cf: &CondFormat,
        rule: &CfRule,
        anchor: CellAddr,
        addr: CellAddr,
        v: &Value,
    ) -> Option<bool> {
        let agg = || self.wb.transient.agg(self.wb, self.version, self.sheet, cf);
        Some(match &rule.kind {
            CfKind::CellValue { op, operands } => {
                let a = match operands.first() {
                    Some(f) => self.eval(f, anchor, addr)?,
                    None => return Some(false),
                };
                let b = match operands.get(1) {
                    Some(f) => Some(self.eval(f, anchor, addr)?),
                    None => None,
                };
                compare(v, *op, &a, b.as_ref())
            }
            CfKind::Formula(f) => truthy(&self.eval(f, anchor, addr)?),
            CfKind::Duplicate | CfKind::Unique => {
                if !self.charge(1) {
                    return None;
                }
                let Some(k) = val_key(v) else {
                    return Some(false);
                };
                let n = agg().counts.get(&k).copied().unwrap_or(0);
                if matches!(rule.kind, CfKind::Duplicate) {
                    n > 1
                } else {
                    n == 1
                }
            }
            CfKind::Top {
                rank,
                percent,
                bottom,
            } => {
                if !self.charge(1) {
                    return None;
                }
                let Value::Number(x) = v else {
                    return Some(false);
                };
                let a = agg();
                let n = a.nums.len();
                if n == 0 {
                    return Some(false);
                }
                let k = if *percent {
                    ((n as u64 * u64::from(*rank)) / 100).max(1) as usize
                } else {
                    *rank as usize
                }
                .min(n);
                if *bottom {
                    *x <= a.nums[k - 1]
                } else {
                    *x >= a.nums[n - k]
                }
            }
            CfKind::Average { above, equal } => {
                if !self.charge(1) {
                    return None;
                }
                let Value::Number(x) = v else {
                    return Some(false);
                };
                let Some(m) = agg().mean() else {
                    return Some(false);
                };
                match (above, equal) {
                    (true, false) => *x > m,
                    (true, true) => *x >= m,
                    (false, false) => *x < m,
                    (false, true) => *x <= m,
                }
            }
            CfKind::Text { op, text } => {
                if !self.charge(1) {
                    return None;
                }
                let Some(s) = value_text(v) else {
                    return Some(false);
                };
                let (s, t) = (s.to_lowercase(), text.to_lowercase());
                match op {
                    CfTextOpData::Contains => s.contains(&t),
                    CfTextOpData::NotContains => !s.contains(&t),
                    CfTextOpData::BeginsWith => s.starts_with(&t),
                    CfTextOpData::EndsWith => s.ends_with(&t),
                }
            }
            CfKind::Blank => {
                if !self.charge(1) {
                    return None;
                }
                value_text(v).is_some_and(|s| s.trim().is_empty())
            }
            CfKind::NotBlank => {
                if !self.charge(1) {
                    return None;
                }
                !value_text(v).is_some_and(|s| s.trim().is_empty())
            }
            CfKind::Error => {
                if !self.charge(1) {
                    return None;
                }
                matches!(v, Value::Error(_))
            }
            CfKind::NotError => {
                if !self.charge(1) {
                    return None;
                }
                !matches!(v, Value::Error(_))
            }
            CfKind::DateIs(p) => {
                if !self.charge(1) {
                    return None;
                }
                match v {
                    Value::Number(n) => date_matches(self.wb, *n, *p),
                    _ => false,
                }
            }
            CfKind::ColorScale(_) | CfKind::DataBar { .. } | CfKind::IconSet { .. } => false,
        })
    }

    /// The outcome for one cell (`None` when no format covers it, nothing
    /// applies, or the budget ran out before it was evaluated).
    pub fn outcome(&self, row: u32, col: u32) -> Option<CfOutcome> {
        let sheet = self.wb.sheet(self.sheet)?;
        if self.index.is_empty() || self.truncated.get() {
            return None;
        }
        let mut candidates = self.candidates.borrow_mut();
        self.index.covering_sorted(row, col, &mut candidates);
        if candidates.is_empty() {
            return None;
        }
        if !self.charge(candidates.len() as u64) {
            return None;
        }
        let addr = CellAddr::new(self.sheet, row, col);
        let mut out = CfOutcome::default();
        let v = self.wb.value(addr);
        'formats: for &i in candidates.iter() {
            let Some(cf) = sheet.meta.cond_formats.get(i as usize) else {
                continue;
            };
            let anchor = cf.anchor(self.sheet);
            for rule in &cf.rules {
                match &rule.kind {
                    CfKind::ColorScale(stops) => {
                        let Value::Number(x) = v else { continue };
                        if out.style.fill.is_some() {
                            continue;
                        }
                        let agg = self.wb.transient.agg(self.wb, self.version, self.sheet, cf);
                        let mut pts: Vec<(f64, u32)> = Vec::with_capacity(stops.len());
                        for (t, c) in stops {
                            if let Some(p) = self.threshold(t, &agg, anchor) {
                                pts.push((p, *c));
                            }
                        }
                        if self.truncated.get() {
                            return None;
                        }
                        if pts.len() != stops.len() || pts.is_empty() {
                            continue;
                        }
                        let color = if x <= pts[0].0 {
                            pts[0].1
                        } else if x >= pts[pts.len() - 1].0 {
                            pts[pts.len() - 1].1
                        } else {
                            let mut c = pts[0].1;
                            for w in pts.windows(2) {
                                let ((a, ca), (b, cb)) = (w[0], w[1]);
                                if x >= a && x <= b {
                                    c = if b > a {
                                        lerp_color(ca, cb, (x - a) / (b - a))
                                    } else {
                                        cb
                                    };
                                    break;
                                }
                            }
                            c
                        };
                        out.style.fill = Some(color);
                    }
                    CfKind::DataBar {
                        min,
                        max,
                        color,
                        negative_color,
                        show_value,
                        gradient,
                    } => {
                        let Value::Number(x) = v else { continue };
                        if out.bar.is_some() {
                            continue;
                        }
                        let agg = self.wb.transient.agg(self.wb, self.version, self.sheet, cf);
                        let lo = self.threshold(min, &agg, anchor);
                        let hi = self.threshold(max, &agg, anchor);
                        if self.truncated.get() {
                            return None;
                        }
                        let (Some(lo), Some(hi)) = (lo, hi) else {
                            continue;
                        };
                        let span = hi - lo;
                        let pos = if span > 0.0 {
                            ((x - lo) / span).clamp(0.0, 1.0)
                        } else {
                            1.0
                        };
                        let (fraction, axis) = if lo < 0.0 && hi > 0.0 {
                            let axis = (-lo / span).clamp(0.0, 1.0);
                            ((pos - axis).abs(), Some(axis as f32))
                        } else if hi <= 0.0 && lo < 0.0 {
                            (1.0 - pos, Some(1.0))
                        } else {
                            (pos, None)
                        };
                        let negative = x < 0.0;
                        out.bar = Some(DataBarView {
                            fraction: fraction as f32,
                            color: if negative {
                                negative_color.unwrap_or(*color)
                            } else {
                                *color
                            },
                            negative,
                            axis,
                            gradient: *gradient,
                        });
                        if !show_value {
                            out.hide_value = true;
                        }
                    }
                    CfKind::IconSet {
                        set,
                        thresholds,
                        show_value,
                        reverse,
                    } => {
                        let Value::Number(x) = v else { continue };
                        if out.icon.is_some() {
                            continue;
                        }
                        let agg = self.wb.transient.agg(self.wb, self.version, self.sheet, cf);
                        let mut index = 0usize;
                        for (i, t) in thresholds.iter().enumerate().skip(1) {
                            match self.threshold(t, &agg, anchor) {
                                Some(p) if x >= p => index = i,
                                _ => {}
                            }
                        }
                        if self.truncated.get() {
                            return None;
                        }
                        let n = set.icon_count();
                        let index = index.min(n - 1);
                        let index = if *reverse { n - 1 - index } else { index };
                        out.icon = Some(IconView {
                            set: icons_to_api(*set),
                            index: index as u32,
                        });
                        if !show_value {
                            out.hide_value = true;
                        }
                    }
                    _ => {
                        // A rule whose style adds nothing to what higher
                        // rules set cannot change the outcome unless it
                        // stops the evaluation.
                        if !rule.stop_if_true && !can_contribute(&rule.style, &out.style) {
                            continue;
                        }
                        let hit = self.rule_matches(cf, rule, anchor, addr, &v)?;
                        if hit {
                            merge_under(&mut out.style, &rule.style);
                            if rule.stop_if_true {
                                break 'formats;
                            }
                        }
                    }
                }
            }
        }
        (!out.is_empty()).then_some(out)
    }

    /// The decoration of one cell with base style `base`, from the
    /// per-version cache when the cell was evaluated before. `None` when
    /// nothing applies or the budget is spent (then nothing is cached).
    pub fn decor(&self, row: u32, col: u32, base: StyleId) -> Option<CfDecor> {
        if self.index.is_empty() {
            return None;
        }
        let key = (self.sheet, row, col);
        if let Some(d) = self.wb.transient.cached_outcome(self.version, key) {
            return d;
        }
        if self.truncated.get() {
            return None;
        }
        let o = self.outcome(row, col);
        if self.truncated.get() {
            return None;
        }
        let d = o.map(|o| {
            let (style, number_format) = if o.style.is_empty() {
                (None, false)
            } else {
                let base = &self.wb.styles.get(base).data;
                let (id, _) = self.wb.transient.style_for(base, &o.style);
                (Some(id), o.style.number_format.is_some())
            };
            CfDecor {
                style,
                number_format,
                bar: o.bar,
                icon: o.icon,
                hide_value: o.hide_value,
            }
        });
        self.wb
            .transient
            .remember_outcome(self.version, key, d.clone());
        d
    }
}

/// `true` if any conditional format on `sheet` intersects `range`.
pub fn any_in(wb: &Workbook, range: &CellRange) -> bool {
    wb.sheet(range.sheet).is_some_and(|s| {
        s.meta.cond_formats.iter().any(|cf| {
            cf.ranges.iter().any(|r| {
                r.r0 <= range.r1 && range.r0 <= r.r1 && r.c0 <= range.c1 && range.c0 <= r.c1
            })
        })
    })
}

/// Whether evaluation of `cf` reads the whole range (for tests/benches).
pub fn uses_aggregates(cf: &CondFormat) -> bool {
    cf.rules.iter().any(|r| r.kind.needs_agg())
}

/// The base style id `StyleId` of a cell as a [`StyleData`] reference.
pub fn base_style(wb: &Workbook, id: StyleId) -> &StyleData {
    &wb.styles.get(id).data
}

#[cfg(test)]
mod tests {
    use super::*;
    use tally_types::ErrorKind;

    #[test]
    fn comparisons() {
        let n = |x: f64| Value::Number(x);
        assert!(compare(&n(5.0), CmpOpData::Gt, &n(3.0), None));
        assert!(compare(&n(3.0), CmpOpData::Between, &n(5.0), Some(&n(1.0))));
        assert!(!compare(
            &n(6.0),
            CmpOpData::Between,
            &n(1.0),
            Some(&n(5.0))
        ));
        assert!(compare(
            &n(6.0),
            CmpOpData::NotBetween,
            &n(1.0),
            Some(&n(5.0))
        ));
        assert!(compare(
            &Value::text("paid"),
            CmpOpData::Eq,
            &Value::text("Paid"),
            None
        ));
        assert!(compare(&Value::Empty, CmpOpData::Lt, &n(5.0), None));
        assert!(!compare(
            &Value::Error(ErrorKind::NA),
            CmpOpData::Ne,
            &n(1.0),
            None
        ));
        assert!(compare(&Value::text("x"), CmpOpData::Gt, &n(1e9), None));
    }

    #[test]
    fn colors_interpolate() {
        assert_eq!(lerp_color(0x0000_00ff, 0xff00_00ff, 0.5), 0x8000_00ff);
        assert_eq!(lerp_color(0x0000_00ff, 0xff00_00ff, 2.0), 0xff00_00ff);
    }

    #[test]
    fn percentiles() {
        let a = Agg {
            nums: vec![1.0, 2.0, 3.0, 4.0, 5.0],
            counts: HashMap::new(),
        };
        assert_eq!(a.percentile(50.0), Some(3.0));
        assert_eq!(a.percentile(25.0), Some(2.0));
        assert_eq!(a.mean(), Some(3.0));
    }
}
