//! DataPilot (wave 5, W5-2): the pivot cache, aggregation with AOO
//! semantics, "show as", grouping and the tabular layout.
//!
//! A pivot is stored on its target sheet (`SheetMeta::pivots`). Its
//! output is written into the sheet as plain values with direct styles
//! (bold headers, a light fill on totals); the roles of the output cells
//! (`PivotCellRole`, field and item) are kept on the engine's
//! [`PivotTable`] for `pivot_at`, `PivotDrillDown` and GETPIVOTDATA
//! (`EvalContext::pivot_data`, see `docs/functions/engine-hooks.md`).
//!
//! Layout (AOO 4.1 tabular output):
//! - an optional `Filter` button row, one `Field | - all -` row per page
//!   field and a blank row;
//! - the header block: the corner caption (`Sum - Revenue`, or `Data`
//!   with several data fields) and the column field names on the first
//!   row, the items of every column field on the next rows, the row field
//!   names on the last header row;
//! - one row per row-field item combination, `<item> Result` subtotal
//!   rows after the children of an outer item, `Total Result` at the end;
//!   the same across the columns.
//! - Aggregation: Count = non-empty, CountNums = numbers, StDev/Var with
//!   fewer than two numbers = `#DIV/0!`; item order numbers before text,
//!   case-insensitive.

use std::collections::HashMap;
use std::sync::Arc;

use tally_api::{
    ApiError, AutoShow, BaseItem, DataLayout, DatePart, PivotCellRole, PivotDataFieldDto, PivotDto,
    PivotFieldDto, PivotFieldItemsDto, PivotGroupDto, PivotHit, PivotItemDto, PivotLayout,
    PivotSort, PivotSource, PivotSubtotals, RangeDto, ShowAs, ShowAsKind,
};
use tally_model::{
    AggFnData, AutoShowData, BaseItemData, CellRangeLocal, DataLayoutData, DatePartData, PivotData,
    PivotDataFieldData, PivotFieldData, PivotGroupData, PivotLayoutData, PivotSortData,
    PivotSourceData, PivotSubtotalsData, ShowAsData, ShowAsKindData,
};
use tally_numfmt::date::ymd_from_serial;
use tally_types::{CellAddr, CellRange, ErrorKind, SheetId, Value, MAX_COLS, MAX_ROWS};

use crate::ops::{check_sheet, map_styles, to_range};
use crate::ops_v03::with_meta;
use crate::protection::check_edit;
use crate::service::view::display_text;
use crate::structure::{point_after, span_after, Shape};
use crate::tools::{agg_caption, agg_to_api, agg_to_model, Agg};
use crate::undo::{capture_range, clear_range_content, clear_range_styles, Change, Effects};
use crate::workbook::Workbook;

/// One cell of a written output, with its role.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutCell {
    /// Row.
    pub r: u32,
    /// Column.
    pub c: u32,
    /// Role.
    pub role: PivotCellRole,
    /// Field (source column), if any.
    pub field: Option<u32>,
    /// Item text, if any.
    pub item: Option<String>,
    /// For data cells: the row items, column items and data field.
    pub key: Option<(Vec<String>, Vec<String>, u32)>,
}

/// The lookup table of a computed pivot (GETPIVOTDATA and drill-down).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Lookup {
    /// `(row items, column items, data field index)` → value.
    pub values: HashMap<(Vec<String>, Vec<String>, u32), Value>,
    /// Field names in source column order.
    pub field_names: Vec<String>,
    /// Data field captions.
    pub captions: Vec<String>,
}

/// A pivot table with its sheet-local id (stored on its target sheet).
#[derive(Clone, Debug, PartialEq)]
pub struct PivotTable {
    /// Id from `SheetMeta::alloc_id`.
    pub id: u32,
    /// The definition.
    pub data: PivotData,
    /// The roles of the last written output (empty after a load).
    pub cells: Arc<Vec<OutCell>>,
    /// The values of the last computation (for GETPIVOTDATA).
    pub lookup: Arc<Lookup>,
}

impl PivotTable {
    fn new(id: u32, data: PivotData) -> Self {
        Self {
            id,
            data,
            cells: Arc::new(Vec::new()),
            lookup: Arc::new(Lookup::default()),
        }
    }
}

/// Most pivot tables per document.
pub const MAX_PIVOTS: usize = 10_000;
/// Most source cells one pivot reads (`unsupported` past it).
pub const MAX_SOURCE_CELLS: u64 = 10_000_000;
/// Most distinct items listed per field.
pub const MAX_ITEMS: usize = 10_000;
/// Most output cells.
pub const MAX_OUTPUT_CELLS: u64 = 4_000_000;

/// Loads a sheet's pivots from the interchange model.
pub fn from_data(wb: &mut Workbook, sheet: SheetId, pivots: &[PivotData]) {
    let Some(s) = wb.sheet_mut(sheet) else {
        return;
    };
    for p in pivots.iter().take(MAX_PIVOTS) {
        let id = s.meta.alloc_id();
        Arc::make_mut(&mut s.meta.pivots).push(Arc::new(PivotTable::new(id, p.clone())));
    }
}

/// The interchange form of a sheet's pivots.
pub fn to_data(wb: &Workbook, sheet: SheetId) -> Vec<PivotData> {
    wb.sheet(sheet)
        .map(|s| s.meta.pivots.iter().map(|p| p.data.clone()).collect())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// API conversions
// ---------------------------------------------------------------------------

fn range_dto(r: &CellRangeLocal) -> RangeDto {
    RangeDto {
        r0: r.r0,
        c0: r.c0,
        r1: r.r1,
        c1: r.c1,
    }
}

fn sort_from_api(s: &PivotSort) -> PivotSortData {
    match s {
        PivotSort::None => PivotSortData::None,
        PivotSort::Name { ascending } => PivotSortData::Name {
            ascending: *ascending,
        },
        PivotSort::Data {
            data_field,
            ascending,
        } => PivotSortData::Data {
            data_field: *data_field,
            ascending: *ascending,
        },
        PivotSort::Manual { order } => PivotSortData::Manual {
            order: order.clone(),
        },
    }
}

fn sort_to_api(s: &PivotSortData) -> PivotSort {
    match s {
        PivotSortData::None => PivotSort::None,
        PivotSortData::Name { ascending } => PivotSort::Name {
            ascending: *ascending,
        },
        PivotSortData::Data {
            data_field,
            ascending,
        } => PivotSort::Data {
            data_field: *data_field,
            ascending: *ascending,
        },
        PivotSortData::Manual { order } => PivotSort::Manual {
            order: order.clone(),
        },
    }
}

fn group_from_api(g: &PivotGroupDto) -> PivotGroupData {
    match g {
        PivotGroupDto::Numeric { start, end, step } => PivotGroupData::Numeric {
            start: *start,
            end: *end,
            step: *step,
        },
        PivotGroupDto::Date { start, end, parts } => PivotGroupData::Date {
            start: *start,
            end: *end,
            parts: parts
                .iter()
                .map(|p| match p {
                    DatePart::Seconds => DatePartData::Seconds,
                    DatePart::Minutes => DatePartData::Minutes,
                    DatePart::Hours => DatePartData::Hours,
                    DatePart::Days { n } => DatePartData::Days(*n),
                    DatePart::Months => DatePartData::Months,
                    DatePart::Quarters => DatePartData::Quarters,
                    DatePart::Years => DatePartData::Years,
                })
                .collect(),
        },
    }
}

fn group_to_api(g: &PivotGroupData) -> PivotGroupDto {
    match g {
        PivotGroupData::Numeric { start, end, step } => PivotGroupDto::Numeric {
            start: *start,
            end: *end,
            step: *step,
        },
        PivotGroupData::Date { start, end, parts } => PivotGroupDto::Date {
            start: *start,
            end: *end,
            parts: parts
                .iter()
                .map(|p| match p {
                    DatePartData::Seconds => DatePart::Seconds,
                    DatePartData::Minutes => DatePart::Minutes,
                    DatePartData::Hours => DatePart::Hours,
                    DatePartData::Days(n) => DatePart::Days { n: *n },
                    DatePartData::Months => DatePart::Months,
                    DatePartData::Quarters => DatePart::Quarters,
                    DatePartData::Years => DatePart::Years,
                })
                .collect(),
        },
    }
}

fn field_from_api(f: &PivotFieldDto) -> PivotFieldData {
    PivotFieldData {
        source_col: f.source_col,
        name: f.name.clone(),
        subtotals: match &f.subtotals {
            PivotSubtotals::Auto => PivotSubtotalsData::Auto,
            PivotSubtotals::None => PivotSubtotalsData::None,
            PivotSubtotals::Custom { fns } => {
                PivotSubtotalsData::Custom(fns.iter().map(|f| agg_to_model(*f)).collect())
            }
        },
        show_empty: f.show_empty,
        sort: sort_from_api(&f.sort),
        hidden_items: f.hidden_items.clone(),
        page_item: f.page_item.clone(),
        layout: match f.layout {
            PivotLayout::Tabular => PivotLayoutData::Tabular,
            PivotLayout::OutlineTop => PivotLayoutData::OutlineTop,
            PivotLayout::OutlineBottom => PivotLayoutData::OutlineBottom,
        },
        empty_line_after: f.empty_line_after,
        auto_show: f.auto_show.as_ref().map(|a| AutoShowData {
            top: a.top,
            count: a.count,
            data_field: a.data_field,
        }),
        group: f.group.as_ref().map(group_from_api),
    }
}

fn field_to_api(f: &PivotFieldData) -> PivotFieldDto {
    PivotFieldDto {
        source_col: f.source_col,
        name: f.name.clone(),
        subtotals: match &f.subtotals {
            PivotSubtotalsData::Auto => PivotSubtotals::Auto,
            PivotSubtotalsData::None => PivotSubtotals::None,
            PivotSubtotalsData::Custom(fns) => PivotSubtotals::Custom {
                fns: fns.iter().map(|f| agg_to_api(*f)).collect(),
            },
        },
        show_empty: f.show_empty,
        sort: sort_to_api(&f.sort),
        hidden_items: f.hidden_items.clone(),
        page_item: f.page_item.clone(),
        layout: match f.layout {
            PivotLayoutData::Tabular => PivotLayout::Tabular,
            PivotLayoutData::OutlineTop => PivotLayout::OutlineTop,
            PivotLayoutData::OutlineBottom => PivotLayout::OutlineBottom,
        },
        empty_line_after: f.empty_line_after,
        auto_show: f.auto_show.as_ref().map(|a| AutoShow {
            top: a.top,
            count: a.count,
            data_field: a.data_field,
        }),
        group: f.group.as_ref().map(group_to_api),
    }
}

fn show_as_from_api(s: &ShowAs) -> ShowAsData {
    ShowAsData {
        kind: match s.kind {
            ShowAsKind::Difference => ShowAsKindData::Difference,
            ShowAsKind::PercentOf => ShowAsKindData::PercentOf,
            ShowAsKind::PercentDifference => ShowAsKindData::PercentDifference,
            ShowAsKind::RunningTotal => ShowAsKindData::RunningTotal,
            ShowAsKind::PercentOfRow => ShowAsKindData::PercentOfRow,
            ShowAsKind::PercentOfColumn => ShowAsKindData::PercentOfColumn,
            ShowAsKind::PercentOfTotal => ShowAsKindData::PercentOfTotal,
            ShowAsKind::Index => ShowAsKindData::Index,
        },
        base_field: s.base_field,
        base_item: s.base_item.as_ref().map(|b| match b {
            BaseItem::Named { name } => BaseItemData::Named(name.clone()),
            BaseItem::Previous => BaseItemData::Previous,
            BaseItem::Next => BaseItemData::Next,
        }),
    }
}

fn show_as_to_api(s: &ShowAsData) -> ShowAs {
    ShowAs {
        kind: match s.kind {
            ShowAsKindData::Difference => ShowAsKind::Difference,
            ShowAsKindData::PercentOf => ShowAsKind::PercentOf,
            ShowAsKindData::PercentDifference => ShowAsKind::PercentDifference,
            ShowAsKindData::RunningTotal => ShowAsKind::RunningTotal,
            ShowAsKindData::PercentOfRow => ShowAsKind::PercentOfRow,
            ShowAsKindData::PercentOfColumn => ShowAsKind::PercentOfColumn,
            ShowAsKindData::PercentOfTotal => ShowAsKind::PercentOfTotal,
            ShowAsKindData::Index => ShowAsKind::Index,
        },
        base_field: s.base_field,
        base_item: s.base_item.as_ref().map(|b| match b {
            BaseItemData::Named(name) => BaseItem::Named { name: name.clone() },
            BaseItemData::Previous => BaseItem::Previous,
            BaseItemData::Next => BaseItem::Next,
        }),
    }
}

fn data_field_from_api(d: &PivotDataFieldDto) -> PivotDataFieldData {
    PivotDataFieldData {
        source_col: d.source_col,
        function: agg_to_model(d.function),
        caption: d.caption.clone(),
        show_as: d.show_as.as_ref().map(show_as_from_api),
    }
}

fn data_field_to_api(d: &PivotDataFieldData) -> PivotDataFieldDto {
    PivotDataFieldDto {
        source_col: d.source_col,
        function: agg_to_api(d.function),
        caption: d.caption.clone(),
        show_as: d.show_as.as_ref().map(show_as_to_api),
    }
}

/// API → model definition (the target sheet is resolved by the caller).
pub fn from_api(wb: &Workbook, dto: &PivotDto) -> Result<PivotData, ApiError> {
    let source = match &dto.source {
        PivotSource::Range { sheet, range } => {
            check_sheet(wb, *sheet)?;
            let r = to_range(*sheet, range)?;
            PivotSourceData::Range {
                sheet: wb.tab_of(*sheet).unwrap_or(0) as u32,
                range: CellRangeLocal {
                    r0: r.r0,
                    c0: r.c0,
                    r1: r.r1,
                    c1: r.c1,
                },
            }
        }
        PivotSource::DatabaseRange { name } => {
            crate::tools::find_database_range(wb, name)
                .ok_or_else(|| ApiError::not_found(format!("no database range {name:?}")))?;
            PivotSourceData::DatabaseRange { name: name.clone() }
        }
    };
    if dto.target.r >= MAX_ROWS || dto.target.c >= MAX_COLS {
        return Err(ApiError::invalid_argument("the target is outside the grid"));
    }
    if dto.data_fields.is_empty() {
        return Err(ApiError::invalid_argument("a pivot needs a data field"));
    }
    if dto.row_fields.len() + dto.col_fields.len() + dto.page_fields.len() + dto.data_fields.len()
        > 256
    {
        return Err(ApiError::invalid_argument("too many fields"));
    }
    Ok(PivotData {
        name: dto.name.trim().to_string(),
        source,
        target: (dto.target.r, dto.target.c),
        output: None,
        row_fields: dto.row_fields.iter().map(field_from_api).collect(),
        col_fields: dto.col_fields.iter().map(field_from_api).collect(),
        page_fields: dto.page_fields.iter().map(field_from_api).collect(),
        data_fields: dto.data_fields.iter().map(data_field_from_api).collect(),
        data_layout: match dto.data_layout {
            DataLayout::Row => DataLayoutData::Row,
            DataLayout::Column => DataLayoutData::Column,
        },
        row_totals: dto.row_totals,
        col_totals: dto.col_totals,
        ignore_empty_rows: dto.ignore_empty_rows,
        identify_categories: dto.identify_categories,
        show_filter_button: dto.show_filter_button,
        drill_down: dto.drill_down,
    })
}

/// Model → API definition.
pub fn to_api(wb: &Workbook, sheet: SheetId, p: &PivotTable) -> PivotDto {
    let d = &p.data;
    PivotDto {
        id: p.id,
        name: d.name.clone(),
        source: match &d.source {
            PivotSourceData::Range { sheet, range } => PivotSource::Range {
                sheet: wb.sheets.get(*sheet as usize).map_or(0, |s| s.id),
                range: range_dto(range),
            },
            PivotSourceData::DatabaseRange { name } => {
                PivotSource::DatabaseRange { name: name.clone() }
            }
        },
        target_sheet: sheet,
        target: tally_api::CellPos {
            r: d.target.0,
            c: d.target.1,
        },
        output: d.output.as_ref().map(range_dto),
        row_fields: d.row_fields.iter().map(field_to_api).collect(),
        col_fields: d.col_fields.iter().map(field_to_api).collect(),
        page_fields: d.page_fields.iter().map(field_to_api).collect(),
        data_fields: d.data_fields.iter().map(data_field_to_api).collect(),
        data_layout: match d.data_layout {
            DataLayoutData::Row => DataLayout::Row,
            DataLayoutData::Column => DataLayout::Column,
        },
        row_totals: d.row_totals,
        col_totals: d.col_totals,
        ignore_empty_rows: d.ignore_empty_rows,
        identify_categories: d.identify_categories,
        show_filter_button: d.show_filter_button,
        drill_down: d.drill_down,
    }
}

/// Every pivot of the document.
pub fn list(wb: &Workbook) -> Vec<PivotDto> {
    let mut out = Vec::new();
    for s in &wb.sheets {
        for p in s.meta.pivots.iter() {
            out.push(to_api(wb, s.id, p));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Source reading and items
// ---------------------------------------------------------------------------

/// The sheet and range of a pivot source.
pub fn source_range(wb: &Workbook, src: &PivotSourceData) -> Result<CellRange, ApiError> {
    match src {
        PivotSourceData::Range { sheet, range } => {
            let id = wb
                .sheets
                .get(*sheet as usize)
                .map(|s| s.id)
                .ok_or_else(|| ApiError::not_found("the source sheet is gone"))?;
            if range.r0 > range.r1
                || range.c0 > range.c1
                || range.r1 >= MAX_ROWS
                || range.c1 >= MAX_COLS
            {
                return Err(ApiError::invalid_argument("bad source range"));
            }
            Ok(CellRange::new(id, range.r0, range.c0, range.r1, range.c1))
        }
        PivotSourceData::DatabaseRange { name } => {
            let (sheet, r, _) = crate::tools::find_database_range(wb, name)
                .ok_or_else(|| ApiError::not_found(format!("no database range {name:?}")))?;
            Ok(CellRange::new(sheet, r.r0, r.c0, r.r1, r.c1))
        }
    }
}

fn api_source_range(wb: &Workbook, src: &PivotSource) -> Result<CellRange, ApiError> {
    match src {
        PivotSource::Range { sheet, range } => {
            check_sheet(wb, *sheet)?;
            to_range(*sheet, range)
        }
        PivotSource::DatabaseRange { name } => {
            let (sheet, r, _) = crate::tools::find_database_range(wb, name)
                .ok_or_else(|| ApiError::not_found(format!("no database range {name:?}")))?;
            Ok(CellRange::new(sheet, r.r0, r.c0, r.r1, r.c1))
        }
    }
}

/// One distinct item of a field.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// Display text.
    pub text: String,
    /// Numeric value (numbers and dates), for ordering and grouping.
    pub num: Option<f64>,
}

/// The source read once: field names, per field the distinct items, per
/// row the item index of every field and the values of every column.
pub struct Cache {
    /// Field names (header row).
    pub names: Vec<String>,
    /// Items per field.
    pub items: Vec<Vec<Item>>,
    /// Per row: item index per field.
    pub rows: Vec<Vec<u32>>,
    /// Per row and column: the value (for the data fields).
    pub values: Vec<Vec<Value>>,
    /// Source row numbers (for drill-down).
    pub source_rows: Vec<u32>,
}

fn item_order(a: &Item, b: &Item) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (a.text.is_empty(), b.text.is_empty()) {
        (true, false) => return Ordering::Greater,
        (false, true) => return Ordering::Less,
        _ => {}
    }
    match (a.num, b.num) {
        (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(Ordering::Equal),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => a
            .text
            .to_lowercase()
            .cmp(&b.text.to_lowercase())
            .then_with(|| a.text.cmp(&b.text)),
    }
}

/// Reads the source (header row = field names) with `ignore_empty_rows`.
pub fn read_source(wb: &Workbook, src: &CellRange, ignore_empty: bool) -> Result<Cache, ApiError> {
    if src.cell_count() > MAX_SOURCE_CELLS {
        return Err(ApiError::unsupported(format!(
            "the pivot source has more than {MAX_SOURCE_CELLS} cells"
        )));
    }
    if src.r0 >= src.r1 {
        return Err(ApiError::invalid_argument(
            "the source needs a header row and at least one data row",
        ));
    }
    let sheet = src.sheet;
    let ncols = src.cols() as usize;
    let mut names: Vec<String> = Vec::with_capacity(ncols);
    for c in src.c0..=src.c1 {
        let t = display_text(wb, CellAddr::new(sheet, src.r0, c));
        let t = if t.is_empty() {
            format!("Column{}", c - src.c0 + 1)
        } else {
            t
        };
        names.push(t);
    }
    let mut items: Vec<Vec<Item>> = vec![Vec::new(); ncols];
    let mut index: Vec<HashMap<String, u32>> = vec![HashMap::new(); ncols];
    let mut rows: Vec<Vec<u32>> = Vec::new();
    let mut values: Vec<Vec<Value>> = Vec::new();
    let mut source_rows = Vec::new();
    for r in src.r0 + 1..=src.r1 {
        let mut row_items = Vec::with_capacity(ncols);
        let mut row_values = Vec::with_capacity(ncols);
        let mut all_empty = true;
        for (fi, c) in (src.c0..=src.c1).enumerate() {
            let a = CellAddr::new(sheet, r, c);
            let v = wb.value(a);
            all_empty &= v.is_empty();
            let text = if v.is_empty() {
                String::new()
            } else {
                display_text(wb, a)
            };
            let idx = match index[fi].get(&text) {
                Some(i) => *i,
                None => {
                    let i = items[fi].len() as u32;
                    items[fi].push(Item {
                        text: text.clone(),
                        num: match v {
                            Value::Number(n) => Some(n),
                            Value::Bool(b) => Some(f64::from(u8::from(b))),
                            _ => None,
                        },
                    });
                    index[fi].insert(text, i);
                    i
                }
            };
            row_items.push(idx);
            row_values.push(v);
        }
        if ignore_empty && all_empty {
            continue;
        }
        rows.push(row_items);
        values.push(row_values);
        source_rows.push(r);
    }
    Ok(Cache {
        names,
        items,
        rows,
        values,
        source_rows,
    })
}

/// `pivot_field_items`.
pub fn field_items(
    wb: &Workbook,
    source: &PivotSource,
) -> Result<Vec<PivotFieldItemsDto>, ApiError> {
    let src = api_source_range(wb, source)?;
    let cache = read_source(wb, &src, false)?;
    let mut out = Vec::with_capacity(cache.names.len());
    for (fi, name) in cache.names.iter().enumerate() {
        let mut counts: Vec<u32> = vec![0; cache.items[fi].len()];
        for row in &cache.rows {
            counts[row[fi] as usize] += 1;
        }
        let mut order: Vec<usize> = (0..cache.items[fi].len()).collect();
        order.sort_by(|a, b| item_order(&cache.items[fi][*a], &cache.items[fi][*b]));
        let non_empty: Vec<&Item> = cache.items[fi]
            .iter()
            .filter(|i| !i.text.is_empty())
            .collect();
        let numeric = !non_empty.is_empty() && non_empty.iter().all(|i| i.num.is_some());
        let date = numeric
            && (src.r0 + 1..=src.r1).take(64).all(|r| {
                wb.styles
                    .get(wb.style_at(CellAddr::new(src.sheet, r, src.c0 + fi as u32)))
                    .format
                    .is_date()
            });
        out.push(PivotFieldItemsDto {
            source_col: fi as u32,
            name: name.clone(),
            numeric,
            date,
            items: order
                .iter()
                .take(MAX_ITEMS)
                .map(|i| PivotItemDto {
                    text: cache.items[fi][*i].text.clone(),
                    count: counts[*i],
                })
                .collect(),
        });
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Grouping
// ---------------------------------------------------------------------------

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

fn fmt_num(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

/// The grouped item of a value.
fn group_item(g: &PivotGroupData, item: &Item, epoch: tally_types::DateEpoch) -> Item {
    let Some(n) = item.num else {
        return item.clone();
    };
    match g {
        PivotGroupData::Numeric { start, end, step } => {
            let step = if *step > 0.0 && step.is_finite() {
                *step
            } else {
                1.0
            };
            let s = start.unwrap_or(0.0);
            if let Some(e) = end {
                if n > *e {
                    return Item {
                        text: format!(">{}", fmt_num(*e)),
                        num: Some(f64::INFINITY),
                    };
                }
            }
            if n < s {
                return Item {
                    text: format!("<{}", fmt_num(s)),
                    num: Some(f64::NEG_INFINITY),
                };
            }
            let k = ((n - s) / step).floor();
            let lo = s + k * step;
            let hi = lo + step;
            let text = if step.fract() == 0.0 && lo.fract() == 0.0 {
                format!("{}-{}", fmt_num(lo), fmt_num(hi - 1.0))
            } else {
                format!("{}-{}", fmt_num(lo), fmt_num(hi))
            };
            Item {
                text,
                num: Some(lo),
            }
        }
        PivotGroupData::Date { parts, .. } => {
            let Some((y, m, d)) = ymd_from_serial(epoch, n) else {
                return item.clone();
            };
            let frac = n.fract();
            let secs = (frac * 86_400.0).round() as u32;
            let (hh, mm, ss) = (secs / 3600, (secs / 60) % 60, secs % 60);
            let mut text_parts: Vec<String> = Vec::new();
            let mut key = 0.0;
            let mut ordered = parts.clone();
            ordered.sort_by_key(|p| match p {
                DatePartData::Years => 0,
                DatePartData::Quarters => 1,
                DatePartData::Months => 2,
                DatePartData::Days(_) => 3,
                DatePartData::Hours => 4,
                DatePartData::Minutes => 5,
                DatePartData::Seconds => 6,
            });
            ordered.dedup();
            for p in &ordered {
                match p {
                    DatePartData::Years => {
                        text_parts.push(y.to_string());
                        key = key * 10_000.0 + f64::from(y);
                    }
                    DatePartData::Quarters => {
                        let q = (m - 1) / 3 + 1;
                        text_parts.push(format!("Q{q}"));
                        key = key * 10.0 + f64::from(q);
                    }
                    DatePartData::Months => {
                        text_parts.push(MONTHS[(m - 1) as usize].to_string());
                        key = key * 100.0 + f64::from(m);
                    }
                    DatePartData::Days(step) => {
                        let step = (*step).max(1);
                        if step == 1 {
                            text_parts.push(format!("{y:04}-{m:02}-{d:02}"));
                            key = key * 1_000_000.0 + n.floor();
                        } else {
                            let base = n.floor();
                            let k = (base / f64::from(step)).floor() * f64::from(step);
                            let (ky, km, kd) = ymd_from_serial(epoch, k).unwrap_or((y, m, d));
                            text_parts.push(format!("{ky:04}-{km:02}-{kd:02}"));
                            key = key * 1_000_000.0 + k;
                        }
                    }
                    DatePartData::Hours => {
                        text_parts.push(format!("{hh:02}"));
                        key = key * 100.0 + f64::from(hh);
                    }
                    DatePartData::Minutes => {
                        text_parts.push(format!(":{mm:02}"));
                        key = key * 100.0 + f64::from(mm);
                    }
                    DatePartData::Seconds => {
                        text_parts.push(format!(":{ss:02}"));
                        key = key * 100.0 + f64::from(ss);
                    }
                }
            }
            if text_parts.is_empty() {
                return item.clone();
            }
            Item {
                text: text_parts.join(" ").replace(" :", ":"),
                num: Some(key),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Computation
// ---------------------------------------------------------------------------

/// An axis field after grouping: its items in display order and, per
/// source row, the item index.
struct AxisField {
    source_col: usize,
    name: String,
    items: Vec<Item>,
    row_item: Vec<u32>,
    def: PivotFieldData,
}

/// A node of the row or column tree.
struct Node {
    /// Item index per level, root = empty.
    key: Vec<u32>,
    children: Vec<Node>,
}

struct Computed {
    row_fields: Vec<AxisField>,
    col_fields: Vec<AxisField>,
    /// `(row key, col key, data field)` → aggregation.
    cells: HashMap<(Vec<u32>, Vec<u32>, usize), Agg>,
    row_tree: Node,
    col_tree: Node,
    /// Source rows kept after the page filters.
    kept: Vec<usize>,
}

fn axis_field(
    cache: &Cache,
    f: &PivotFieldData,
    epoch: tally_types::DateEpoch,
) -> Result<AxisField, ApiError> {
    let sc = f.source_col as usize;
    if sc >= cache.names.len() {
        return Err(ApiError::invalid_argument(format!(
            "field column {sc} is outside the source"
        )));
    }
    let (items, row_item) = match &f.group {
        None => (
            cache.items[sc].clone(),
            cache.rows.iter().map(|r| r[sc]).collect(),
        ),
        Some(g) => {
            let mut items: Vec<Item> = Vec::new();
            let mut index: HashMap<String, u32> = HashMap::new();
            let map: Vec<u32> = cache.items[sc]
                .iter()
                .map(|it| {
                    let gi = group_item(g, it, epoch);
                    match index.get(&gi.text) {
                        Some(i) => *i,
                        None => {
                            let i = items.len() as u32;
                            index.insert(gi.text.clone(), i);
                            items.push(gi);
                            i
                        }
                    }
                })
                .collect();
            (
                items,
                cache.rows.iter().map(|r| map[r[sc] as usize]).collect(),
            )
        }
    };
    Ok(AxisField {
        source_col: sc,
        name: if f.name.is_empty() {
            cache.names[sc].clone()
        } else {
            f.name.clone()
        },
        items,
        row_item,
        def: f.clone(),
    })
}

fn build_tree(
    fields: &[AxisField],
    kept: &[usize],
    cells: &HashMap<(Vec<u32>, Vec<u32>, usize), Agg>,
    other_axis_rows: bool,
    data_fields: &[PivotDataFieldData],
) -> Node {
    fn build(
        level: usize,
        prefix: Vec<u32>,
        rows: &[usize],
        fields: &[AxisField],
        cells: &HashMap<(Vec<u32>, Vec<u32>, usize), Agg>,
        rows_axis: bool,
        data_fields: &[PivotDataFieldData],
    ) -> Node {
        if level >= fields.len() {
            return Node {
                key: prefix,
                children: Vec::new(),
            };
        }
        let f = &fields[level];
        let mut present: Vec<u32> = Vec::new();
        let mut seen = vec![false; f.items.len()];
        let mut by_item: Vec<Vec<usize>> = vec![Vec::new(); f.items.len()];
        for &r in rows {
            let it = f.row_item[r] as usize;
            if !seen[it] {
                seen[it] = true;
                present.push(it as u32);
            }
            by_item[it].push(r);
        }
        if f.def.show_empty {
            present = (0..f.items.len() as u32).collect();
        }
        present.retain(|i| !f.def.hidden_items.contains(&f.items[*i as usize].text));
        // Order.
        let total_of = |item: u32, df: usize| -> f64 {
            let mut key = prefix.clone();
            key.push(item);
            let k = if rows_axis {
                (key, Vec::new(), df)
            } else {
                (Vec::new(), key, df)
            };
            cells
                .get(&k)
                .map(|a| {
                    match a.result(data_fields.get(df).map_or(AggFnData::Sum, |d| d.function)) {
                        Value::Number(n) => n,
                        _ => 0.0,
                    }
                })
                .unwrap_or(0.0)
        };
        match &f.def.sort {
            PivotSortData::None => {}
            PivotSortData::Name { ascending } => {
                present.sort_by(|a, b| {
                    let o = item_order(&f.items[*a as usize], &f.items[*b as usize]);
                    if *ascending {
                        o
                    } else {
                        o.reverse()
                    }
                });
            }
            PivotSortData::Data {
                data_field,
                ascending,
            } => {
                let df = *data_field as usize;
                present.sort_by(|a, b| {
                    let o = total_of(*a, df)
                        .partial_cmp(&total_of(*b, df))
                        .unwrap_or(std::cmp::Ordering::Equal);
                    if *ascending {
                        o
                    } else {
                        o.reverse()
                    }
                });
            }
            PivotSortData::Manual { order } => {
                let pos = |i: u32| {
                    order
                        .iter()
                        .position(|o| o.eq_ignore_ascii_case(&f.items[i as usize].text))
                        .unwrap_or(usize::MAX)
                };
                present.sort_by_key(|i| pos(*i));
            }
        }
        if let Some(a) = &f.def.auto_show {
            let df = a.data_field as usize;
            let mut ranked = present.clone();
            ranked.sort_by(|x, y| {
                let o = total_of(*x, df)
                    .partial_cmp(&total_of(*y, df))
                    .unwrap_or(std::cmp::Ordering::Equal);
                if a.top {
                    o.reverse()
                } else {
                    o
                }
            });
            let keep: Vec<u32> = ranked.into_iter().take(a.count as usize).collect();
            present.retain(|i| keep.contains(i));
        }
        let children = present
            .into_iter()
            .map(|it| {
                let mut key = prefix.clone();
                key.push(it);
                build(
                    level + 1,
                    key,
                    &by_item[it as usize],
                    fields,
                    cells,
                    rows_axis,
                    data_fields,
                )
            })
            .collect();
        Node {
            key: prefix,
            children,
        }
    }
    build(
        0,
        Vec::new(),
        kept,
        fields,
        cells,
        other_axis_rows,
        data_fields,
    )
}

fn compute(wb: &Workbook, p: &PivotData, cache: &Cache) -> Result<Computed, ApiError> {
    let epoch = wb.settings.date_epoch;
    let row_fields: Vec<AxisField> = p
        .row_fields
        .iter()
        .map(|f| axis_field(cache, f, epoch))
        .collect::<Result<_, _>>()?;
    let col_fields: Vec<AxisField> = p
        .col_fields
        .iter()
        .map(|f| axis_field(cache, f, epoch))
        .collect::<Result<_, _>>()?;
    let page_fields: Vec<AxisField> = p
        .page_fields
        .iter()
        .map(|f| axis_field(cache, f, epoch))
        .collect::<Result<_, _>>()?;
    for d in &p.data_fields {
        if d.source_col as usize >= cache.names.len() {
            return Err(ApiError::invalid_argument(
                "a data field is outside the source",
            ));
        }
    }
    // Page filters and hidden items.
    let kept: Vec<usize> = (0..cache.rows.len())
        .filter(|&r| {
            page_fields.iter().all(|pf| match &pf.def.page_item {
                None => true,
                Some(sel) => pf.items[pf.row_item[r] as usize].text == *sel,
            }) && row_fields.iter().chain(&col_fields).all(|f| {
                !f.def
                    .hidden_items
                    .contains(&f.items[f.row_item[r] as usize].text)
            })
        })
        .collect();
    // Aggregate every prefix combination.
    let mut cells: HashMap<(Vec<u32>, Vec<u32>, usize), Agg> = HashMap::new();
    for &r in &kept {
        let rkey: Vec<u32> = row_fields.iter().map(|f| f.row_item[r]).collect();
        let ckey: Vec<u32> = col_fields.iter().map(|f| f.row_item[r]).collect();
        for (di, d) in p.data_fields.iter().enumerate() {
            let v = &cache.values[r][d.source_col as usize];
            for rl in 0..=rkey.len() {
                for cl in 0..=ckey.len() {
                    cells
                        .entry((rkey[..rl].to_vec(), ckey[..cl].to_vec(), di))
                        .or_default()
                        .push(v);
                }
            }
        }
        if cells.len() as u64 > MAX_OUTPUT_CELLS {
            return Err(ApiError::unsupported("the pivot result is too large"));
        }
    }
    let row_tree = build_tree(&row_fields, &kept, &cells, true, &p.data_fields);
    let col_tree = build_tree(&col_fields, &kept, &cells, false, &p.data_fields);
    Ok(Computed {
        row_fields,
        col_fields,
        cells,
        row_tree,
        col_tree,
        kept,
    })
}

// ---------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------

/// One line of an axis: its key and what it is.
#[derive(Clone, Debug)]
struct Line {
    /// Item index per level (shorter for subtotals and the grand total).
    key: Vec<u32>,
    /// Subtotal line of the item at `key.len() - 1`, with the function.
    subtotal: Option<AggFnData>,
    /// Grand total.
    grand: bool,
    /// Blank spacer line.
    blank: bool,
    /// Data field index when the Data pseudo-field is on this axis.
    data_field: Option<usize>,
}

fn subtotal_fns(f: &PivotFieldData, data: &[PivotDataFieldData]) -> Vec<Option<AggFnData>> {
    match &f.subtotals {
        PivotSubtotalsData::None => Vec::new(),
        // Auto: the data field's own function, labelled "Result".
        PivotSubtotalsData::Auto => vec![None],
        PivotSubtotalsData::Custom(fns) => {
            if fns.is_empty() {
                vec![None]
            } else {
                let _ = data;
                fns.iter().map(|f| Some(*f)).collect()
            }
        }
    }
}

/// Flattens a tree into lines (tabular: subtotal lines after the
/// children of outer items; the Data pseudo-field as the innermost
/// level when it is on this axis).
fn lines(
    node: &Node,
    fields: &[AxisField],
    data: &[PivotDataFieldData],
    data_here: bool,
    grand: bool,
    out: &mut Vec<Line>,
) {
    fn walk(
        node: &Node,
        fields: &[AxisField],
        data: &[PivotDataFieldData],
        data_here: bool,
        out: &mut Vec<Line>,
    ) {
        let level = node.key.len();
        if level == fields.len() {
            if data_here {
                for di in 0..data.len() {
                    out.push(Line {
                        key: node.key.clone(),
                        subtotal: None,
                        grand: false,
                        blank: false,
                        data_field: Some(di),
                    });
                }
            } else {
                out.push(Line {
                    key: node.key.clone(),
                    subtotal: None,
                    grand: false,
                    blank: false,
                    data_field: None,
                });
            }
            return;
        }
        for child in &node.children {
            walk(child, fields, data, data_here, out);
            let f = &fields[level];
            let inner_levels = fields.len() - level - 1;
            // Subtotals only for outer fields (with more than one child
            // line beneath), as AOO.
            if inner_levels > 0 || (data_here && data.len() > 1) {
                for fun in subtotal_fns(&f.def, data) {
                    if data_here {
                        for (di, d) in data.iter().enumerate() {
                            out.push(Line {
                                key: child.key.clone(),
                                subtotal: Some(fun.unwrap_or(d.function)),
                                grand: false,
                                blank: false,
                                data_field: Some(di),
                            });
                        }
                    } else {
                        out.push(Line {
                            key: child.key.clone(),
                            subtotal: Some(fun.unwrap_or(AggFnData::Sum)),
                            grand: false,
                            blank: false,
                            data_field: None,
                        });
                    }
                }
            }
            if f.def.empty_line_after {
                out.push(Line {
                    key: child.key.clone(),
                    subtotal: None,
                    grand: false,
                    blank: true,
                    data_field: None,
                });
            }
        }
    }
    walk(node, fields, data, data_here, out);
    if grand && (data_here || !fields.is_empty()) {
        if data_here {
            for di in 0..data.len() {
                out.push(Line {
                    key: Vec::new(),
                    subtotal: None,
                    grand: true,
                    blank: false,
                    data_field: Some(di),
                });
            }
        } else {
            out.push(Line {
                key: Vec::new(),
                subtotal: None,
                grand: true,
                blank: false,
                data_field: None,
            });
        }
    }
    if fields.is_empty() && out.is_empty() {
        // No fields on this axis: a single total line.
        if data_here {
            for di in 0..data.len() {
                out.push(Line {
                    key: Vec::new(),
                    subtotal: None,
                    grand: false,
                    blank: false,
                    data_field: Some(di),
                });
            }
        } else {
            out.push(Line {
                key: Vec::new(),
                subtotal: None,
                grand: false,
                blank: false,
                data_field: None,
            });
        }
    }
}

/// The caption of a data field.
pub fn caption(d: &PivotDataFieldData, names: &[String]) -> String {
    match &d.caption {
        Some(c) if !c.trim().is_empty() => c.clone(),
        _ => format!(
            "{} - {}",
            agg_caption(d.function),
            names
                .get(d.source_col as usize)
                .cloned()
                .unwrap_or_default()
        ),
    }
}

/// The previous column line at a level: key, subtotal, grand, data field.
type PrevCol = (Vec<u32>, Option<AggFnData>, bool, Option<usize>);

/// A written output cell before the show-as pass.
struct Out {
    r: u32,
    c: u32,
    value: Value,
    role: PivotCellRole,
    field: Option<u32>,
    item: Option<String>,
    key: Option<(Vec<String>, Vec<String>, u32)>,
}

struct Layout {
    cells: Vec<Out>,
    rows: u32,
    cols: u32,
    lookup: Lookup,
}

fn key_texts(fields: &[AxisField], key: &[u32]) -> Vec<String> {
    key.iter()
        .enumerate()
        .map(|(i, it)| fields[i].items[*it as usize].text.clone())
        .collect()
}

fn lay_out(p: &PivotData, cache: &Cache, comp: &Computed) -> Result<Layout, ApiError> {
    let data_in_rows = p.data_fields.len() > 1 && p.data_layout == DataLayoutData::Row;
    let data_in_cols = p.data_fields.len() > 1 && p.data_layout == DataLayoutData::Column;
    let mut row_lines = Vec::new();
    lines(
        &comp.row_tree,
        &comp.row_fields,
        &p.data_fields,
        data_in_rows,
        p.row_totals,
        &mut row_lines,
    );
    let mut col_lines = Vec::new();
    lines(
        &comp.col_tree,
        &comp.col_fields,
        &p.data_fields,
        data_in_cols,
        p.col_totals,
        &mut col_lines,
    );
    let nr = comp.row_fields.len() as u32 + u32::from(data_in_rows);
    let nc = comp.col_fields.len() as u32 + u32::from(data_in_cols);
    let captions: Vec<String> = p
        .data_fields
        .iter()
        .map(|d| caption(d, &cache.names))
        .collect();
    let mut cells: Vec<Out> = Vec::new();
    let mut r = 0u32;
    let push = |cells: &mut Vec<Out>,
                r: u32,
                c: u32,
                v: Value,
                role: PivotCellRole,
                field: Option<u32>,
                item: Option<String>| {
        cells.push(Out {
            r,
            c,
            value: v,
            role,
            field,
            item,
            key: None,
        });
    };
    // Filter button and page fields.
    let mut had_top = false;
    if p.show_filter_button {
        push(
            &mut cells,
            r,
            0,
            Value::text("Filter"),
            PivotCellRole::Header,
            None,
            None,
        );
        r += 1;
        had_top = true;
    }
    for pf in &p.page_fields {
        let name = if pf.name.is_empty() {
            cache
                .names
                .get(pf.source_col as usize)
                .cloned()
                .unwrap_or_default()
        } else {
            pf.name.clone()
        };
        push(
            &mut cells,
            r,
            0,
            Value::text(name),
            PivotCellRole::PageField,
            Some(pf.source_col),
            None,
        );
        let sel = pf.page_item.clone().unwrap_or_else(|| "- all -".into());
        push(
            &mut cells,
            r,
            1,
            Value::text(sel.as_str()),
            PivotCellRole::PageField,
            Some(pf.source_col),
            Some(sel),
        );
        r += 1;
        had_top = true;
    }
    if had_top {
        r += 1;
    }
    // Header block.
    let header_rows = nc + 1;
    let top = r;
    let corner = if p.data_fields.len() == 1 {
        captions[0].clone()
    } else {
        "Data".to_string()
    };
    // Without column fields the single header row holds the row field
    // names and the caption above the data column (AOO: `Region | Sum -
    // Revenue`); otherwise the caption is the corner cell.
    let corner_col = if nc == 0 { nr.max(1) } else { 0 };
    push(
        &mut cells,
        top,
        corner_col,
        Value::text(corner.as_str()),
        PivotCellRole::Header,
        None,
        None,
    );
    // Column field names on the first header row, after the corner.
    let data_col0 = nr.max(1);
    for (k, f) in comp.col_fields.iter().enumerate() {
        push(
            &mut cells,
            top,
            data_col0 + k as u32,
            Value::text(f.name.as_str()),
            PivotCellRole::Header,
            Some(f.source_col as u32),
            None,
        );
    }
    if data_in_cols {
        push(
            &mut cells,
            top,
            data_col0 + comp.col_fields.len() as u32,
            Value::text("Data"),
            PivotCellRole::Header,
            None,
            None,
        );
    }
    // Row field names on the last header row.
    let last_header = top + header_rows - 1;
    for (k, f) in comp.row_fields.iter().enumerate() {
        push(
            &mut cells,
            last_header,
            k as u32,
            Value::text(f.name.as_str()),
            PivotCellRole::Header,
            Some(f.source_col as u32),
            None,
        );
    }
    if data_in_rows {
        push(
            &mut cells,
            last_header,
            comp.row_fields.len() as u32,
            Value::text("Data"),
            PivotCellRole::Header,
            None,
            None,
        );
    }
    // Column items: level k on header row top + 1 + k; an outer item is
    // written on the first column of its span.
    let mut prev: Vec<Option<PrevCol>> = vec![None; nc as usize];
    for (ci, line) in col_lines.iter().enumerate() {
        let c = data_col0 + ci as u32;
        if line.blank {
            prev.iter_mut().for_each(|x| *x = None);
            continue;
        }
        if line.grand {
            let (text, role) = ("Total Result".to_string(), PivotCellRole::Total);
            let rr = if nc == 0 { top } else { top + 1 };
            push(
                &mut cells,
                rr,
                c,
                Value::text(text.as_str()),
                role,
                None,
                None,
            );
            if let Some(di) = line.data_field {
                push(
                    &mut cells,
                    top + nc,
                    c,
                    Value::text(captions[di].as_str()),
                    PivotCellRole::ColItem,
                    None,
                    Some(captions[di].clone()),
                );
            }
            prev.iter_mut().for_each(|x| *x = None);
            continue;
        }
        if let Some(fun) = line.subtotal {
            let lvl = line.key.len() - 1;
            let text = format!(
                "{} {}",
                comp.col_fields[lvl].items[line.key[lvl] as usize].text,
                match comp.col_fields[lvl].def.subtotals {
                    PivotSubtotalsData::Custom(ref v) if !v.is_empty() =>
                        agg_caption(fun).to_string(),
                    _ => "Result".to_string(),
                }
            );
            push(
                &mut cells,
                top + 1 + lvl as u32,
                c,
                Value::text(text.as_str()),
                PivotCellRole::Total,
                Some(comp.col_fields[lvl].source_col as u32),
                Some(
                    comp.col_fields[lvl].items[line.key[lvl] as usize]
                        .text
                        .clone(),
                ),
            );
            if let Some(di) = line.data_field {
                push(
                    &mut cells,
                    top + nc,
                    c,
                    Value::text(captions[di].as_str()),
                    PivotCellRole::ColItem,
                    None,
                    Some(captions[di].clone()),
                );
            }
            prev.iter_mut().for_each(|x| *x = None);
            continue;
        }
        for (lvl, it) in line.key.iter().enumerate() {
            let same = prev[lvl].as_ref().is_some_and(|(k, st, g, _)| {
                k.len() > lvl && k[..=lvl] == line.key[..=lvl] && st.is_none() && !g
            });
            if !same {
                let f = &comp.col_fields[lvl];
                let text = f.items[*it as usize].text.clone();
                push(
                    &mut cells,
                    top + 1 + lvl as u32,
                    c,
                    Value::text(text.as_str()),
                    PivotCellRole::ColItem,
                    Some(f.source_col as u32),
                    Some(text),
                );
            }
            prev[lvl] = Some((line.key.clone(), line.subtotal, line.grand, line.data_field));
        }
        if let Some(di) = line.data_field {
            push(
                &mut cells,
                top + nc,
                c,
                Value::text(captions[di].as_str()),
                PivotCellRole::ColItem,
                None,
                Some(captions[di].clone()),
            );
        }
    }
    // Row items and data.
    let first_data_row = top + header_rows;
    let mut lookup = Lookup {
        values: HashMap::new(),
        field_names: cache.names.clone(),
        captions: captions.clone(),
    };
    let mut prev_row: Vec<Option<Vec<u32>>> = vec![None; nr as usize];
    let mut rr = first_data_row;
    let total_cols = data_col0 + col_lines.len() as u32;
    for line in &row_lines {
        if line.blank {
            prev_row.iter_mut().for_each(|x| *x = None);
            rr += 1;
            continue;
        }
        // Labels.
        if line.grand {
            push(
                &mut cells,
                rr,
                0,
                Value::text("Total Result"),
                PivotCellRole::Total,
                None,
                None,
            );
            if let Some(di) = line.data_field {
                push(
                    &mut cells,
                    rr,
                    nr - 1,
                    Value::text(captions[di].as_str()),
                    PivotCellRole::RowItem,
                    None,
                    Some(captions[di].clone()),
                );
            }
            prev_row.iter_mut().for_each(|x| *x = None);
        } else if let Some(fun) = line.subtotal {
            let lvl = line.key.len() - 1;
            let f = &comp.row_fields[lvl];
            let item = f.items[line.key[lvl] as usize].text.clone();
            let text = format!(
                "{} {}",
                item,
                match f.def.subtotals {
                    PivotSubtotalsData::Custom(ref v) if !v.is_empty() =>
                        agg_caption(fun).to_string(),
                    _ => "Result".to_string(),
                }
            );
            push(
                &mut cells,
                rr,
                lvl as u32,
                Value::text(text.as_str()),
                PivotCellRole::Total,
                Some(f.source_col as u32),
                Some(item),
            );
            if let Some(di) = line.data_field {
                push(
                    &mut cells,
                    rr,
                    nr - 1,
                    Value::text(captions[di].as_str()),
                    PivotCellRole::RowItem,
                    None,
                    Some(captions[di].clone()),
                );
            }
            prev_row.iter_mut().for_each(|x| *x = None);
        } else {
            for (lvl, it) in line.key.iter().enumerate() {
                let same = prev_row[lvl]
                    .as_ref()
                    .is_some_and(|k| k.len() > lvl && k[..=lvl] == line.key[..=lvl]);
                if !same {
                    let f = &comp.row_fields[lvl];
                    let text = f.items[*it as usize].text.clone();
                    push(
                        &mut cells,
                        rr,
                        lvl as u32,
                        Value::text(text.as_str()),
                        PivotCellRole::RowItem,
                        Some(f.source_col as u32),
                        Some(text),
                    );
                }
                prev_row[lvl] = Some(line.key.clone());
            }
            if let Some(di) = line.data_field {
                push(
                    &mut cells,
                    rr,
                    nr - 1,
                    Value::text(captions[di].as_str()),
                    PivotCellRole::RowItem,
                    None,
                    Some(captions[di].clone()),
                );
            }
        }
        // Data cells.
        for (ci, cl) in col_lines.iter().enumerate() {
            if cl.blank {
                continue;
            }
            let c = data_col0 + ci as u32;
            let di = line.data_field.or(cl.data_field).unwrap_or(0);
            // Subtotal lines use the custom function when one was chosen,
            // else the data field's own function.
            let own = p.data_fields[di].function;
            let custom = line.subtotal.or(cl.subtotal).filter(|_| {
                comp.row_fields
                    .iter()
                    .chain(&comp.col_fields)
                    .any(|f| matches!(f.def.subtotals, PivotSubtotalsData::Custom(ref v) if !v.is_empty()))
            });
            let fun = custom.unwrap_or(own);
            let v = comp
                .cells
                .get(&(line.key.clone(), cl.key.clone(), di))
                .map(|a| a.result(fun))
                .unwrap_or(Value::Empty);
            let role = if line.grand || cl.grand || line.subtotal.is_some() || cl.subtotal.is_some()
            {
                PivotCellRole::Total
            } else {
                PivotCellRole::Data
            };
            let rk = key_texts(&comp.row_fields, &line.key);
            let ck = key_texts(&comp.col_fields, &cl.key);
            if role == PivotCellRole::Data
                || line.grand
                || cl.grand
                || (line.subtotal.is_some() || cl.subtotal.is_some())
            {
                lookup
                    .values
                    .entry((rk.clone(), ck.clone(), di as u32))
                    .or_insert_with(|| v.clone());
            }
            cells.push(Out {
                r: rr,
                c,
                value: v,
                role,
                field: None,
                item: None,
                key: Some((rk, ck, di as u32)),
            });
        }
        rr += 1;
    }
    let rows = rr;
    let cols = total_cols.max(2);
    if u64::from(rows) * u64::from(cols) > MAX_OUTPUT_CELLS {
        return Err(ApiError::unsupported("the pivot output is too large"));
    }
    let mut layout = Layout {
        cells,
        rows,
        cols,
        lookup,
    };
    show_as(
        p,
        comp,
        &mut layout,
        first_data_row,
        data_col0,
        &row_lines,
        &col_lines,
    );
    Ok(layout)
}

/// Applies the "show as" transformations of the data fields.
#[allow(clippy::too_many_arguments)]
fn show_as(
    p: &PivotData,
    comp: &Computed,
    layout: &mut Layout,
    first_data_row: u32,
    data_col0: u32,
    row_lines: &[Line],
    col_lines: &[Line],
) {
    if p.data_fields.iter().all(|d| d.show_as.is_none()) {
        return;
    }
    let num = |v: &Value| match v {
        Value::Number(n) => Some(*n),
        _ => None,
    };
    // Index the data cells by (row line, col line).
    let mut grid: HashMap<(usize, usize), usize> = HashMap::new();
    let mut row_of: HashMap<u32, usize> = HashMap::new();
    let mut rr = first_data_row;
    for (i, l) in row_lines.iter().enumerate() {
        row_of.insert(rr, i);
        let _ = l;
        rr += 1;
    }
    for (idx, o) in layout.cells.iter().enumerate() {
        if o.key.is_none() {
            continue;
        }
        if let Some(ri) = row_of.get(&o.r) {
            let ci = (o.c - data_col0) as usize;
            grid.insert((*ri, ci), idx);
        }
    }
    let value_at = |layout: &Layout, ri: usize, ci: usize| -> Option<f64> {
        grid.get(&(ri, ci))
            .and_then(|i| num(&layout.cells[*i].value))
    };
    let grand_row = row_lines.iter().position(|l| l.grand);
    let grand_col = col_lines.iter().position(|l| l.grand);
    let mut updates: Vec<(usize, Value)> = Vec::new();
    for (ri, rl) in row_lines.iter().enumerate() {
        for (ci, cl) in col_lines.iter().enumerate() {
            let Some(&idx) = grid.get(&(ri, ci)) else {
                continue;
            };
            let di = rl.data_field.or(cl.data_field).unwrap_or(0);
            let Some(sa) = &p.data_fields[di].show_as else {
                continue;
            };
            let Some(v) = num(&layout.cells[idx].value) else {
                continue;
            };
            let same_df = |l: &Line| l.data_field.is_none_or(|d| d == di);
            let row_total = grand_col
                .filter(|_| same_df(rl))
                .and_then(|gc| value_at(layout, ri, gc));
            let col_total = grand_row
                .filter(|_| same_df(cl))
                .and_then(|gr| value_at(layout, gr, ci));
            let grand = grand_row
                .zip(grand_col)
                .and_then(|(gr, gc)| value_at(layout, gr, gc));
            let pct = |a: f64, b: Option<f64>| match b {
                Some(b) if b != 0.0 => Value::Number(a / b),
                _ => Value::Error(ErrorKind::Div0),
            };
            let out = match sa.kind {
                ShowAsKindData::PercentOfRow => pct(v, row_total),
                ShowAsKindData::PercentOfColumn => pct(v, col_total),
                ShowAsKindData::PercentOfTotal => pct(v, grand),
                ShowAsKindData::Index => match (row_total, col_total, grand) {
                    (Some(r), Some(c), Some(g)) if r != 0.0 && c != 0.0 => {
                        Value::Number(v * g / (r * c))
                    }
                    _ => Value::Error(ErrorKind::Div0),
                },
                ShowAsKindData::RunningTotal
                | ShowAsKindData::Difference
                | ShowAsKindData::PercentOf
                | ShowAsKindData::PercentDifference => {
                    // The base field is a row or column field; its level.
                    let base_col = sa.base_field.unwrap_or(0);
                    let row_lvl = comp
                        .row_fields
                        .iter()
                        .position(|f| f.source_col as u32 == base_col);
                    let col_lvl = comp
                        .col_fields
                        .iter()
                        .position(|f| f.source_col as u32 == base_col);
                    let (along_rows, lvl) = match (row_lvl, col_lvl) {
                        (Some(l), _) => (true, l),
                        (None, Some(l)) => (false, l),
                        _ => continue,
                    };
                    let this = if along_rows { rl } else { cl };
                    if this.grand || this.subtotal.is_some() || this.key.len() <= lvl {
                        continue;
                    }
                    let fields = if along_rows {
                        &comp.row_fields
                    } else {
                        &comp.col_fields
                    };
                    let lines_all = if along_rows { row_lines } else { col_lines };
                    // Sibling lines: same prefix above `lvl`, same deeper key
                    // shape (same data field), ordinary lines.
                    let siblings: Vec<usize> = lines_all
                        .iter()
                        .enumerate()
                        .filter(|(_, l)| {
                            !l.grand
                                && !l.blank
                                && l.subtotal.is_none()
                                && l.key.len() == this.key.len()
                                && l.key[..lvl] == this.key[..lvl]
                                && l.key[lvl + 1..] == this.key[lvl + 1..]
                                && l.data_field == this.data_field
                        })
                        .map(|(i, _)| i)
                        .collect();
                    let me = if along_rows { ri } else { ci };
                    let pos = siblings.iter().position(|s| *s == me).unwrap_or(0);
                    let get = |layout: &Layout, i: usize| {
                        if along_rows {
                            value_at(layout, i, ci)
                        } else {
                            value_at(layout, ri, i)
                        }
                    };
                    if sa.kind == ShowAsKindData::RunningTotal {
                        let sum: f64 = siblings[..=pos]
                            .iter()
                            .filter_map(|i| get(layout, *i))
                            .sum();
                        Value::Number(sum)
                    } else {
                        let base_idx = match &sa.base_item {
                            Some(BaseItemData::Previous) => pos.checked_sub(1).map(|i| siblings[i]),
                            Some(BaseItemData::Next) => siblings.get(pos + 1).copied(),
                            Some(BaseItemData::Named(name)) => siblings.iter().copied().find(|i| {
                                let l = &lines_all[*i];
                                fields[lvl].items[l.key[lvl] as usize].text == *name
                            }),
                            None => None,
                        };
                        let is_base = base_idx == Some(me);
                        match (sa.kind, base_idx.and_then(|b| get(layout, b))) {
                            (ShowAsKindData::Difference, Some(b)) => {
                                if is_base {
                                    Value::Empty
                                } else {
                                    Value::Number(v - b)
                                }
                            }
                            (ShowAsKindData::PercentOf, Some(b)) => pct(v, Some(b)),
                            (ShowAsKindData::PercentDifference, Some(b)) => {
                                if is_base {
                                    Value::Empty
                                } else {
                                    pct(v - b, Some(b))
                                }
                            }
                            (
                                ShowAsKindData::Difference | ShowAsKindData::PercentDifference,
                                None,
                            ) => Value::Empty,
                            (_, None) => Value::Error(ErrorKind::NA),
                            _ => Value::Number(v),
                        }
                    }
                }
            };
            updates.push((idx, out));
        }
    }
    for (idx, v) in updates {
        layout.cells[idx].value = v;
    }
}

// ---------------------------------------------------------------------------
// Writing the output
// ---------------------------------------------------------------------------

fn pivot_of(wb: &Workbook, sheet: SheetId, id: u32) -> Result<Arc<PivotTable>, ApiError> {
    wb.sheet(sheet)
        .and_then(|s| s.meta.pivots.iter().find(|p| p.id == id).cloned())
        .ok_or_else(|| ApiError::not_found(format!("no pivot table {id}")))
}

/// Finds the sheet holding pivot `id`.
pub fn sheet_of_pivot(wb: &Workbook, id: u32) -> Option<SheetId> {
    wb.sheets
        .iter()
        .find(|s| s.meta.pivots.iter().any(|p| p.id == id))
        .map(|s| s.id)
}

/// `true` if `range` holds any cell outside `except`.
fn has_content_outside(wb: &Workbook, range: &CellRange, except: Option<&CellRange>) -> bool {
    let snap = capture_range(wb, *range, true, false);
    snap.contents.iter().flatten().any(|(r, c, _)| {
        let a = CellAddr::new(range.sheet, *r, *c);
        !except.is_some_and(|e| e.contains(a))
    })
}

/// What [`write_output`] produced: the definition with its output range,
/// the cell roles, the lookup table and the inverse changes.
type Written = (PivotData, Arc<Vec<OutCell>>, Arc<Lookup>, Vec<Change>);

/// Computes `data` and writes its output at the target; returns the
/// written table and the changes (the caller records the meta).
fn write_output(
    wb: &mut Workbook,
    sheet: SheetId,
    data: &PivotData,
    old_output: Option<CellRangeLocal>,
    overwrite: bool,
) -> Result<Written, ApiError> {
    let src = source_range(wb, &data.source)?;
    let cache = read_source(wb, &src, data.ignore_empty_rows)?;
    let comp = compute(wb, data, &cache)?;
    let layout = lay_out(data, &cache, &comp)?;
    let (tr, tc) = data.target;
    if u64::from(tr) + u64::from(layout.rows) > u64::from(MAX_ROWS)
        || u64::from(tc) + u64::from(layout.cols) > u64::from(MAX_COLS)
    {
        return Err(ApiError::invalid_argument(
            "the pivot output does not fit on the sheet",
        ));
    }
    let out = CellRange::new(
        sheet,
        tr,
        tc,
        tr + layout.rows.saturating_sub(1),
        tc + layout.cols.saturating_sub(1),
    );
    if src.sheet == sheet && src.intersects(&out) {
        return Err(ApiError::invalid_argument(
            "the pivot output would overlap its source",
        ));
    }
    let old = old_output.map(|o| CellRange::new(sheet, o.r0, o.c0, o.r1, o.c1));
    if !overwrite && has_content_outside(wb, &out, old.as_ref()) {
        return Err(ApiError::new(
            "target_not_empty",
            "Destination range is not empty. Overwrite?",
        ));
    }
    let mut ranges = vec![out];
    if let Some(o) = old {
        ranges.push(o);
    }
    check_edit(wb, &ranges)?;
    crate::ops::check_array_blocks(wb, &ranges)?;
    let mut changes = Vec::new();
    for r in &ranges {
        changes.push(Change::Range(capture_range(wb, *r, true, true)));
    }
    if let Some(o) = old {
        clear_range_content(wb, o);
        clear_range_styles(wb, o);
    }
    clear_range_content(wb, out);
    clear_range_styles(wb, out);
    let mut roles: Vec<OutCell> = Vec::with_capacity(layout.cells.len());
    for o in layout.cells {
        let a = CellAddr::new(sheet, tr + o.r, tc + o.c);
        if !o.value.is_empty() {
            wb.set_value(a, o.value);
        }
        let cell = CellRange::new(sheet, a.row, a.col, a.row, a.col);
        match o.role {
            PivotCellRole::Header
            | PivotCellRole::RowItem
            | PivotCellRole::ColItem
            | PivotCellRole::PageField => {
                map_styles(wb, cell, |t, old| {
                    let mut d = t.get(old).data.clone();
                    d.font.bold = true;
                    t.intern(d)
                });
            }
            PivotCellRole::Total => {
                map_styles(wb, cell, |t, old| {
                    let mut d = t.get(old).data.clone();
                    d.font.bold = true;
                    d.fill = Some(0xeeee_eeff);
                    t.intern(d)
                });
            }
            PivotCellRole::Data => {}
        }
        roles.push(OutCell {
            r: a.row,
            c: a.col,
            role: o.role,
            field: o.field,
            item: o.item,
            key: o.key,
        });
    }
    roles.sort_by_key(|o| (o.r, o.c));
    wb.pending.ranges.push(out);
    let mut written = data.clone();
    written.output = Some(CellRangeLocal {
        r0: out.r0,
        c0: out.c0,
        r1: out.r1,
        c1: out.c1,
    });
    Ok((written, Arc::new(roles), Arc::new(layout.lookup), changes))
}

fn next_name(wb: &Workbook) -> String {
    let mut n = 1;
    loop {
        let name = format!("DataPilot{n}");
        let taken = wb.sheets.iter().any(|s| {
            s.meta
                .pivots
                .iter()
                .any(|p| p.data.name.eq_ignore_ascii_case(&name))
        });
        if !taken {
            return name;
        }
        n += 1;
    }
}

/// `Op::CreatePivot`.
pub fn create(
    wb: &mut Workbook,
    dto: &PivotDto,
    overwrite: bool,
    fx: &mut Effects,
) -> Result<Vec<Change>, ApiError> {
    let sheet = dto.target_sheet;
    check_sheet(wb, sheet)?;
    let total: usize = wb.sheets.iter().map(|s| s.meta.pivots.len()).sum();
    if total >= MAX_PIVOTS {
        return Err(ApiError::invalid_argument(format!(
            "at most {MAX_PIVOTS} pivot tables"
        )));
    }
    let mut data = from_api(wb, dto)?;
    if data.name.is_empty() {
        data.name = next_name(wb);
    }
    let (written, roles, lookup, mut changes) = write_output(wb, sheet, &data, None, overwrite)?;
    let mut created = 0;
    changes.extend(with_meta(wb, sheet, fx, |_, meta| {
        let id = meta.alloc_id();
        created = id;
        Arc::make_mut(&mut meta.pivots).push(Arc::new(PivotTable {
            id,
            data: written.clone(),
            cells: roles.clone(),
            lookup: lookup.clone(),
        }));
        Ok(())
    })?);
    fx.created(u32::MAX, created);
    fx.pivots_changed = Some(true);
    Ok(changes)
}

/// `Op::UpdatePivot`.
pub fn update(
    wb: &mut Workbook,
    id: u32,
    dto: &PivotDto,
    overwrite: bool,
    fx: &mut Effects,
) -> Result<Vec<Change>, ApiError> {
    let holder = sheet_of_pivot(wb, id)
        .ok_or_else(|| ApiError::not_found(format!("no pivot table {id}")))?;
    let old = pivot_of(wb, holder, id)?;
    let mut data = from_api(wb, dto)?;
    if data.name.is_empty() {
        data.name = old.data.name.clone();
    }
    let target_sheet = dto.target_sheet;
    check_sheet(wb, target_sheet)?;
    let mut changes = Vec::new();
    // Moving to another sheet: the old output is cleared there.
    let old_output = if target_sheet == holder {
        old.data.output
    } else {
        None
    };
    if target_sheet != holder {
        if let Some(o) = old.data.output {
            let r = CellRange::new(holder, o.r0, o.c0, o.r1, o.c1);
            check_edit(wb, &[r])?;
            changes.push(Change::Range(capture_range(wb, r, true, true)));
            clear_range_content(wb, r);
            clear_range_styles(wb, r);
        }
        changes.extend(with_meta(wb, holder, fx, |_, meta| {
            Arc::make_mut(&mut meta.pivots).retain(|p| p.id != id);
            Ok(())
        })?);
    }
    let (written, roles, lookup, ch) =
        write_output(wb, target_sheet, &data, old_output, overwrite)?;
    changes.extend(ch);
    changes.extend(with_meta(wb, target_sheet, fx, |_, meta| {
        let list = Arc::make_mut(&mut meta.pivots);
        let table = Arc::new(PivotTable {
            id,
            data: written.clone(),
            cells: roles.clone(),
            lookup: lookup.clone(),
        });
        match list.iter().position(|p| p.id == id) {
            Some(i) => list[i] = table,
            None => list.push(table),
        }
        Ok(())
    })?);
    fx.pivots_changed = Some(true);
    Ok(changes)
}

/// `Op::RefreshPivot`.
pub fn refresh(wb: &mut Workbook, id: u32, fx: &mut Effects) -> Result<Vec<Change>, ApiError> {
    let holder = sheet_of_pivot(wb, id)
        .ok_or_else(|| ApiError::not_found(format!("no pivot table {id}")))?;
    let old = pivot_of(wb, holder, id)?;
    let (written, roles, lookup, mut changes) =
        write_output(wb, holder, &old.data, old.data.output, true)?;
    changes.extend(with_meta(wb, holder, fx, |_, meta| {
        let list = Arc::make_mut(&mut meta.pivots);
        if let Some(i) = list.iter().position(|p| p.id == id) {
            list[i] = Arc::new(PivotTable {
                id,
                data: written.clone(),
                cells: roles.clone(),
                lookup: lookup.clone(),
            });
        }
        Ok(())
    })?);
    fx.pivots_changed = Some(true);
    Ok(changes)
}

/// `Op::DeletePivot`.
pub fn delete(
    wb: &mut Workbook,
    id: u32,
    clear_output: bool,
    fx: &mut Effects,
) -> Result<Vec<Change>, ApiError> {
    let holder = sheet_of_pivot(wb, id)
        .ok_or_else(|| ApiError::not_found(format!("no pivot table {id}")))?;
    let old = pivot_of(wb, holder, id)?;
    let mut changes = Vec::new();
    if clear_output {
        if let Some(o) = old.data.output {
            let r = CellRange::new(holder, o.r0, o.c0, o.r1, o.c1);
            check_edit(wb, &[r])?;
            changes.push(Change::Range(capture_range(wb, r, true, true)));
            clear_range_content(wb, r);
            clear_range_styles(wb, r);
        }
    }
    changes.extend(with_meta(wb, holder, fx, |_, meta| {
        Arc::make_mut(&mut meta.pivots).retain(|p| p.id != id);
        Ok(())
    })?);
    fx.pivots_changed = Some(true);
    Ok(changes)
}

/// `pivot_at`.
pub fn pivot_at(wb: &Workbook, sheet: SheetId, r: u32, c: u32) -> Option<PivotHit> {
    let s = wb.sheet(sheet)?;
    for p in s.meta.pivots.iter() {
        let Some(o) = p.data.output else { continue };
        if r < o.r0 || r > o.r1 || c < o.c0 || c > o.c1 {
            continue;
        }
        let hit = p
            .cells
            .binary_search_by_key(&(r, c), |x| (x.r, x.c))
            .ok()
            .map(|i| &p.cells[i]);
        return Some(match hit {
            Some(cell) => PivotHit {
                pivot: p.id,
                role: cell.role,
                field: cell.field,
                item: cell.item.clone(),
            },
            None => PivotHit {
                pivot: p.id,
                role: PivotCellRole::Data,
                field: None,
                item: None,
            },
        });
    }
    None
}

/// `Op::PivotDrillDown`: a new sheet with the source rows behind a data
/// cell.
pub fn drill_down(
    wb: &mut Workbook,
    id: u32,
    r: u32,
    c: u32,
    fx: &mut Effects,
) -> Result<Vec<Change>, ApiError> {
    let holder = sheet_of_pivot(wb, id)
        .ok_or_else(|| ApiError::not_found(format!("no pivot table {id}")))?;
    let p = pivot_of(wb, holder, id)?;
    let cell = p
        .cells
        .binary_search_by_key(&(r, c), |x| (x.r, x.c))
        .ok()
        .map(|i| p.cells[i].clone())
        .filter(|x| matches!(x.role, PivotCellRole::Data | PivotCellRole::Total))
        .ok_or_else(|| ApiError::invalid_argument("not a data cell of the pivot"))?;
    let Some((rk, ck, _)) = cell.key else {
        return Err(ApiError::invalid_argument("not a data cell of the pivot"));
    };
    crate::protection::check_structure(wb)?;
    let src = source_range(wb, &p.data.source)?;
    let cache = read_source(wb, &src, p.data.ignore_empty_rows)?;
    let comp = compute(wb, &p.data, &cache)?;
    let matches: Vec<usize> = comp
        .kept
        .iter()
        .copied()
        .filter(|&row| {
            rk.iter().enumerate().all(|(l, t)| {
                comp.row_fields[l].items[comp.row_fields[l].row_item[row] as usize].text == *t
            }) && ck.iter().enumerate().all(|(l, t)| {
                comp.col_fields[l].items[comp.col_fields[l].row_item[row] as usize].text == *t
            })
        })
        .collect();
    if matches.len() as u64 + 1 > u64::from(MAX_ROWS) {
        return Err(ApiError::invalid_argument("too many rows"));
    }
    let name = wb.unique_sheet_name(&format!("{}_Data", p.data.name), None);
    let tab = wb.tab_of(holder).unwrap_or(0) + 1;
    let new = wb.add_sheet(Some(&name), Some(tab));
    for (ci, n) in cache.names.iter().enumerate() {
        wb.set_value(CellAddr::new(new, 0, ci as u32), Value::text(n.as_str()));
    }
    for (i, row) in matches.iter().enumerate() {
        let sr = cache.source_rows[*row];
        for (ci, col) in (src.c0..=src.c1).enumerate() {
            let from = CellAddr::new(src.sheet, sr, col);
            let v = wb.value(from);
            let st = wb.style_at(from);
            let to = CellAddr::new(new, i as u32 + 1, ci as u32);
            if !v.is_empty() {
                wb.set_value(to, v);
            }
            if st != crate::storage::StyleId::DEFAULT {
                if let Some(c) = wb.sheet_mut(new).and_then(|s| s.column_mut(to.col)) {
                    c.styles.set(to.row, to.row, st);
                }
            }
        }
    }
    wb.refresh_all_precedents();
    fx.sheets_changed = true;
    fx.created(u32::MAX, new);
    Ok(vec![Change::RemoveSheet { id: new }])
}

/// `EvalContext::pivot_data`: the value of the pivot whose output
/// contains `at`, for data field `field` restricted by `filters`.
pub fn pivot_data(
    wb: &Workbook,
    at: CellAddr,
    field: &str,
    filters: &[(String, String)],
) -> Option<Value> {
    let s = wb.sheet(at.sheet)?;
    let p = s.meta.pivots.iter().find(|p| {
        p.data
            .output
            .is_some_and(|o| at.row >= o.r0 && at.row <= o.r1 && at.col >= o.c0 && at.col <= o.c1)
    })?;
    let lk = &p.lookup;
    let lower = |s: &str| s.trim().to_lowercase();
    // The data field: an exact caption, a field name, or (Excel form) the
    // leading words, with the remaining words item names of any field.
    let mut filters: Vec<(String, String)> = filters.to_vec();
    let find_df = |text: &str| -> Option<usize> {
        let t = lower(text);
        if let Some(i) = lk.captions.iter().position(|c| lower(c) == t) {
            return Some(i);
        }
        p.data.data_fields.iter().position(|d| {
            lk.field_names
                .get(d.source_col as usize)
                .is_some_and(|n| lower(n) == t)
        })
    };
    let df = match find_df(field) {
        Some(i) => i,
        None => {
            // Greedy: the longest leading word run that names a data field;
            // the rest are item names.
            let words: Vec<&str> = field.split_whitespace().collect();
            let mut found: Option<(usize, usize)> = None;
            for n in (1..=words.len()).rev() {
                if let Some(i) = find_df(&words[..n].join(" ")) {
                    found = Some((i, n));
                    break;
                }
            }
            let (i, n) = match found {
                Some(x) => x,
                None if p.data.data_fields.len() == 1 => (0, 0),
                None => return None,
            };
            let rest = words[n..].join(" ");
            if !rest.trim().is_empty() {
                // An item name of some row/column field.
                let item = rest.trim().to_string();
                let owner = p
                    .data
                    .row_fields
                    .iter()
                    .chain(&p.data.col_fields)
                    .find(|f| {
                        lk.values.keys().any(|(rk, ck, _)| {
                            let pos_r = p
                                .data
                                .row_fields
                                .iter()
                                .position(|x| x.source_col == f.source_col);
                            let pos_c = p
                                .data
                                .col_fields
                                .iter()
                                .position(|x| x.source_col == f.source_col);
                            pos_r.is_some_and(|l| {
                                rk.get(l).is_some_and(|t| lower(t) == lower(&item))
                            }) || pos_c.is_some_and(|l| {
                                ck.get(l).is_some_and(|t| lower(t) == lower(&item))
                            })
                        })
                    })?;
                let name = lk
                    .field_names
                    .get(owner.source_col as usize)
                    .cloned()
                    .unwrap_or_default();
                filters.push((name, item));
            }
            i
        }
    };
    let field_name = |f: &PivotFieldData| -> String {
        if f.name.is_empty() {
            lk.field_names
                .get(f.source_col as usize)
                .cloned()
                .unwrap_or_default()
        } else {
            f.name.clone()
        }
    };
    let mut rk: Vec<Option<String>> = vec![None; p.data.row_fields.len()];
    let mut ck: Vec<Option<String>> = vec![None; p.data.col_fields.len()];
    for (fname, item) in &filters {
        let fl = lower(fname);
        if let Some(l) = p
            .data
            .row_fields
            .iter()
            .position(|f| lower(&field_name(f)) == fl)
        {
            rk[l] = Some(item.clone());
        } else if let Some(l) = p
            .data
            .col_fields
            .iter()
            .position(|f| lower(&field_name(f)) == fl)
        {
            ck[l] = Some(item.clone());
        } else if let Some(pf) = p
            .data
            .page_fields
            .iter()
            .find(|f| lower(&field_name(f)) == fl)
        {
            // A page field: only its selected item (or all) is answerable.
            if pf
                .page_item
                .as_ref()
                .is_some_and(|sel| lower(sel) != lower(item))
            {
                return None;
            }
        } else {
            return None;
        }
    }
    // Keys are prefixes: an unspecified outer field with a specified inner
    // one has no cell.
    let prefix = |v: Vec<Option<String>>| -> Option<Vec<String>> {
        let mut out = Vec::new();
        let mut ended = false;
        for x in v {
            match x {
                Some(t) if !ended => out.push(t),
                Some(_) => return None,
                None => ended = true,
            }
        }
        Some(out)
    };
    let rk = prefix(rk)?;
    let ck = prefix(ck)?;
    // Items are matched case-insensitively against the display text.
    let hit = lk.values.iter().find(|((r, c, d), _)| {
        *d as usize == df
            && r.len() == rk.len()
            && c.len() == ck.len()
            && r.iter().zip(&rk).all(|(a, b)| lower(a) == lower(b))
            && c.iter().zip(&ck).all(|(a, b)| lower(a) == lower(b))
    });
    hit.map(|(_, v)| match v {
        Value::Empty => Value::Number(0.0),
        other => other.clone(),
    })
}

/// Moves pivot sources, targets and outputs for a row/column change.
pub fn adjust_structure(wb: &mut Workbook, sh: &Shape) {
    let changed_tab = wb.tab_of(sh.sheet).map(|t| t as u32);
    for s in wb.sheets.iter_mut() {
        if s.meta.pivots.is_empty() {
            continue;
        }
        let on_sheet = s.id == sh.sheet;
        let mut any = false;
        let mut list: Vec<Arc<PivotTable>> = s.meta.pivots.iter().cloned().collect();
        for p in list.iter_mut() {
            let mut d = p.data.clone();
            if let PivotSourceData::Range { sheet, range } = &mut d.source {
                if Some(*sheet) == changed_tab {
                    let n = if sh.rows {
                        span_after(sh, range.r0, range.r1).map(|(r0, r1)| CellRangeLocal {
                            r0,
                            r1,
                            ..*range
                        })
                    } else {
                        span_after(sh, range.c0, range.c1).map(|(c0, c1)| CellRangeLocal {
                            c0,
                            c1,
                            ..*range
                        })
                    };
                    if let Some(n) = n {
                        *range = n;
                    }
                }
            }
            if on_sheet {
                let (tr, tc) = d.target;
                let np = if sh.rows {
                    point_after(sh, tr).map(|r| (r, tc))
                } else {
                    point_after(sh, tc).map(|c| (tr, c))
                };
                if let Some(np) = np {
                    d.target = np;
                }
                if let Some(o) = d.output {
                    d.output = if sh.rows {
                        span_after(sh, o.r0, o.r1).map(|(r0, r1)| CellRangeLocal { r0, r1, ..o })
                    } else {
                        span_after(sh, o.c0, o.c1).map(|(c0, c1)| CellRangeLocal { c0, c1, ..o })
                    };
                }
            }
            if d != p.data {
                any = true;
                let cells: Vec<OutCell> = if on_sheet {
                    p.cells
                        .iter()
                        .filter_map(|c| {
                            let (r, col) = if sh.rows {
                                (point_after(sh, c.r)?, c.c)
                            } else {
                                (c.r, point_after(sh, c.c)?)
                            };
                            Some(OutCell {
                                r,
                                c: col,
                                ..c.clone()
                            })
                        })
                        .collect()
                } else {
                    (*p.cells).clone()
                };
                *p = Arc::new(PivotTable {
                    id: p.id,
                    data: d,
                    cells: Arc::new(cells),
                    lookup: p.lookup.clone(),
                });
            }
        }
        if any {
            s.meta.pivots = Arc::new(list);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_and_date_groups() {
        let g = PivotGroupData::Numeric {
            start: Some(0.0),
            end: None,
            step: 10.0,
        };
        let it = Item {
            text: "17".into(),
            num: Some(17.0),
        };
        assert_eq!(
            group_item(&g, &it, tally_types::DateEpoch::D1899).text,
            "10-19"
        );
        let d = PivotGroupData::Date {
            start: None,
            end: None,
            parts: vec![DatePartData::Months, DatePartData::Years],
        };
        let serial =
            tally_numfmt::date::serial_from_ymd(tally_types::DateEpoch::D1899, 2024, 3, 15)
                .unwrap();
        let it = Item {
            text: "x".into(),
            num: Some(serial),
        };
        assert_eq!(
            group_item(&d, &it, tally_types::DateEpoch::D1899).text,
            "2024 Mar"
        );
    }
}
