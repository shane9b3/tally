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
//! - Aggregation as AOO `ScDPAggData` (`dptabres.cxx:415-597`): Count
//!   counts strings and errors, every other function ignores strings, is
//!   an error after an error value and empty without values; StDev/Var
//!   need two values; errors are output as `#VALUE!`. Items are
//!   identified by value (numbers, dates, booleans) or case-insensitive
//!   text; numbers sort before text, the empty item `(empty)` last.
//! - Budgets: at most [`MAX_FIELDS`] fields (also for pivots read from
//!   files), [`MAX_AGG_WORK`] aggregation updates, [`MAX_OUTPUT_CELLS`]
//!   result cells; the row and column trees are bounded by the sheet size
//!   before the output is laid out.

use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasherDefault, Hasher};
use std::sync::Arc;

use tally_api::{
    ApiError, AutoShow, BaseItem, DataLayout, DatePart, PivotCellRole, PivotDataFieldDto, PivotDto,
    PivotFieldDto, PivotFieldItemsDto, PivotGroupDto, PivotHit, PivotItemDto, PivotLayout,
    PivotSort, PivotSource, PivotSourceFilterDto, PivotSubtotals, RangeDto, ShowAs, ShowAsKind,
};
use tally_model::{
    AggFnData, AutoShowData, BaseItemData, CellRangeLocal, DataLayoutData, DatePartData, PivotData,
    PivotDataFieldData, PivotFieldData, PivotGroupData, PivotLayoutData, PivotSortData,
    PivotSourceData, PivotSourceFilterData, PivotSubtotalsData, ShowAsData, ShowAsKindData,
    StandardFilterData, StdConditionData, StdFilterOptionsData,
};
use tally_numfmt::date::ymd_from_serial;
use tally_numfmt::{recognize_input_with, Locale, RecognizeContext};
use tally_types::{CellAddr, CellRange, DateEpoch, ErrorKind, SheetId, Value, MAX_COLS, MAX_ROWS};

use crate::ops::{check_sheet, map_styles, to_range};
use crate::ops_v03::with_meta;
use crate::protection::check_edit;
use crate::service::view::display_text;
use crate::structure::{point_after, span_after, Shape};
use crate::tools::{agg_caption, agg_to_api, agg_to_model};
use crate::undo::{capture_range, clear_range_content, clear_range_styles, Change, Effects};
use crate::workbook::Workbook;

/// One label cell of a written output (headers, items, totals), with
/// its role. Data cells are described by [`Grid`].
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
}

/// One row or column line of the data area.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridLine {
    /// The row or column tree node of the line.
    pub node: u32,
    /// A subtotal or the grand total.
    pub total: bool,
    /// A blank spacer line.
    pub blank: bool,
    /// Index into the lookup's value grid.
    pub line: u32,
}

/// The data area of a written output: the sheet row/column of every line.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Grid {
    /// `(sheet row, line)`, ascending.
    pub rows: Vec<(u32, GridLine)>,
    /// `(sheet column, line)`, ascending.
    pub cols: Vec<(u32, GridLine)>,
}

/// One axis field of the lookup.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FieldLookup {
    /// Lowercased display name and source name.
    pub names: (String, String),
    /// Hash of the lowercased item caption → item index.
    pub by_text: FxMap<u64, u32>,
    /// Captions shared by several items → all of them.
    pub dups: FxMap<u64, Vec<u32>>,
    /// Hashes of the item identities (drill-down).
    pub keys: Vec<u64>,
}

/// One axis of the lookup: the tree of item paths and the line of every
/// node.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AxisLookup {
    /// Fields per level.
    pub fields: Vec<FieldLookup>,
    /// Parent of every node (node 0 is the root).
    pub parent: Vec<u32>,
    /// Item of every node.
    pub item: Vec<u32>,
    /// `(parent, item)` → node.
    pub edges: FxMap<(u32, u32), u32>,
    /// `(node, data field or u16::MAX)` → line.
    pub line_of: FxMap<(u32, u16), u32>,
}

/// A page field of the lookup.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PageLookup {
    /// Lowercased display name and source name.
    pub names: (String, String),
    /// Lowercased selected item.
    pub selected: Option<String>,
}

/// The lookup table of a computed pivot (GETPIVOTDATA and drill-down):
/// indexed by item name, so one call costs O(fields).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Lookup {
    /// Per data field, the lowercased names GETPIVOTDATA accepts: the
    /// caption, the source name, AOO's given name and the long form.
    pub df_names: Vec<Vec<String>>,
    /// Row axis.
    pub rows: AxisLookup,
    /// Column axis.
    pub cols: AxisLookup,
    /// Page fields.
    pub pages: Vec<PageLookup>,
    /// Displayed values, row line × column line.
    pub values: Vec<Res>,
    /// Column lines.
    pub ncols: usize,
    /// The Data pseudo-field is on the rows / columns.
    pub data_rows: bool,
    /// See `data_rows`.
    pub data_cols: bool,
}

/// A pivot table with its sheet-local id (stored on its target sheet).
#[derive(Clone, Debug, PartialEq)]
pub struct PivotTable {
    /// Id from `SheetMeta::alloc_id`.
    pub id: u32,
    /// The definition.
    pub data: PivotData,
    /// The label roles of the last written output (empty after a load).
    pub cells: Arc<Vec<OutCell>>,
    /// The data area of the last written output.
    pub grid: Arc<Grid>,
    /// The values of the last computation (for GETPIVOTDATA).
    pub lookup: Arc<Lookup>,
}

impl PivotTable {
    fn new(id: u32, data: PivotData) -> Self {
        Self {
            id,
            data,
            cells: Arc::new(Vec::new()),
            grid: Arc::new(Grid::default()),
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
/// Most fields (row, column, page and data) of one pivot.
pub const MAX_FIELDS: usize = 256;
/// Most distinct items of all columns of one source read.
pub const MAX_CACHE_ITEMS: usize = 2_000_000;

/// Why a definition read from a file is refused, if it is.
fn file_definition_error(p: &PivotData) -> Option<String> {
    let fields =
        p.row_fields.len() + p.col_fields.len() + p.page_fields.len() + p.data_fields.len();
    if fields > MAX_FIELDS {
        return Some(format!("it has {fields} fields (at most {MAX_FIELDS})"));
    }
    if p.data_fields.is_empty() {
        return Some("it has no data field".into());
    }
    if p.target.0 >= MAX_ROWS || p.target.1 >= MAX_COLS {
        return Some("its target is outside the grid".into());
    }
    None
}

/// Loads a sheet's pivots from the interchange model. Range sources name
/// their sheet by tab index in the file and by [`SheetId`] in the engine
/// (so inserting, moving or copying sheets keeps them on their sheet); an
/// index past the last sheet becomes `u32::MAX` ("the source sheet is
/// gone").
pub fn from_data(wb: &mut Workbook, sheet: SheetId, pivots: &[PivotData]) {
    let ids: Vec<SheetId> = wb.sheets.iter().map(|s| s.id).collect();
    let Some(s) = wb.sheet_mut(sheet) else {
        return;
    };
    let mut warnings = Vec::new();
    for p in pivots.iter().take(MAX_PIVOTS) {
        // Definitions from files get the API's checks (security review 4,
        // H7): too many fields would make every refresh allocate without
        // bound.
        if let Some(why) = file_definition_error(p) {
            warnings.push(format!("Pivot table {:?} was not loaded: {why}.", p.name));
            continue;
        }
        let id = s.meta.alloc_id();
        let mut data = p.clone();
        for f in data
            .row_fields
            .iter_mut()
            .chain(data.col_fields.iter_mut())
            .chain(data.page_fields.iter_mut())
        {
            f.hidden_items.truncate(MAX_ITEMS);
            if let PivotSortData::Manual { order } = &mut f.sort {
                order.truncate(MAX_ITEMS);
            }
        }
        if let PivotSourceData::Range { sheet, .. } = &mut data.source {
            *sheet = ids.get(*sheet as usize).copied().unwrap_or(u32::MAX);
        }
        // The source filter within the API's limits (so the pivot can be
        // edited and sent back).
        if let Some(f) = &mut data.source_filter {
            f.conditions.retain(|c| c.value.len() <= 4096);
            f.conditions.truncate(MAX_SOURCE_FILTER_CONDITIONS);
            if f.conditions.is_empty() {
                data.source_filter = None;
            }
        }
        Arc::make_mut(&mut s.meta.pivots).push(Arc::new(PivotTable::new(id, data)));
    }
    for w in warnings {
        if !wb.import_warnings.contains(&w) {
            wb.import_warnings.push(w);
        }
    }
}

/// The interchange form of a sheet's pivots (source sheet ids become tab
/// indices).
pub fn to_data(wb: &Workbook, sheet: SheetId) -> Vec<PivotData> {
    wb.sheet(sheet)
        .map(|s| {
            s.meta
                .pivots
                .iter()
                .map(|p| {
                    let mut data = p.data.clone();
                    if let PivotSourceData::Range { sheet, .. } = &mut data.source {
                        *sheet = wb.tab_of(*sheet).map_or(u32::MAX, |t| t as u32);
                    }
                    data
                })
                .collect()
        })
        .unwrap_or_default()
}

/// For `CopySheet`: pivots copied with the sheet's meta stay on the copy
/// (their target and output are sheet-local); a range source on the
/// copied sheet now reads the copy, as for charts.
pub fn clone_for_sheet(
    meta: &mut crate::storage::SheetMeta,
    old_sheet: SheetId,
    new_sheet: SheetId,
) {
    if meta.pivots.is_empty() {
        return;
    }
    for p in Arc::make_mut(&mut meta.pivots).iter_mut() {
        if let PivotSourceData::Range { sheet, .. } = &p.data.source {
            if *sheet == old_sheet {
                let p = Arc::make_mut(p);
                if let PivotSourceData::Range { sheet, .. } = &mut p.data.source {
                    *sheet = new_sheet;
                }
            }
        }
    }
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
                sheet: *sheet,
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
        > MAX_FIELDS
    {
        return Err(ApiError::invalid_argument("too many fields"));
    }
    for f in dto
        .row_fields
        .iter()
        .chain(&dto.col_fields)
        .chain(&dto.page_fields)
    {
        let manual = match &f.sort {
            PivotSort::Manual { order } => order.len(),
            _ => 0,
        };
        if f.hidden_items.len() > MAX_ITEMS || manual > MAX_ITEMS {
            return Err(ApiError::invalid_argument(format!(
                "at most {MAX_ITEMS} hidden or ordered items per field"
            )));
        }
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
        source_filter: source_filter_from_api(wb, &dto.source, dto.source_filter.as_ref())?,
    })
}

/// Most conditions of a source filter (AOO's dialog has 3; files may
/// hold a few more).
pub const MAX_SOURCE_FILTER_CONDITIONS: usize = 8;

fn source_filter_from_api(
    wb: &Workbook,
    source: &PivotSource,
    f: Option<&PivotSourceFilterDto>,
) -> Result<Option<PivotSourceFilterData>, ApiError> {
    let Some(f) = f else { return Ok(None) };
    if f.conditions.is_empty() {
        return Ok(None);
    }
    if f.conditions.len() > MAX_SOURCE_FILTER_CONDITIONS {
        return Err(ApiError::invalid_argument(format!(
            "at most {MAX_SOURCE_FILTER_CONDITIONS} filter conditions"
        )));
    }
    let src = api_source_range(wb, source)?;
    if f.conditions
        .iter()
        .any(|c| c.col >= src.cols() || c.value.len() > 4096)
    {
        return Err(ApiError::invalid_argument(
            "a filter condition column is outside the source or its value is too long",
        ));
    }
    let model = crate::tools::std_filter_to_model(
        CellRangeLocal::default(),
        &f.conditions,
        &tally_api::StdFilterOptions::default(),
        None,
        None,
    );
    Ok(Some(PivotSourceFilterData {
        conditions: model.conditions,
        case_sensitive: f.case_sensitive,
        regex: f.regex,
        no_duplicates: f.no_duplicates,
    }))
}

fn source_filter_to_api(f: &PivotSourceFilterData) -> PivotSourceFilterDto {
    PivotSourceFilterDto {
        conditions: crate::tools::std_conditions_to_api(&f.conditions),
        case_sensitive: f.case_sensitive,
        regex: f.regex,
        no_duplicates: f.no_duplicates,
    }
}

/// Model → API definition.
pub fn to_api(sheet: SheetId, p: &PivotTable) -> PivotDto {
    let d = &p.data;
    PivotDto {
        id: p.id,
        name: d.name.clone(),
        source: match &d.source {
            PivotSourceData::Range { sheet, range } => PivotSource::Range {
                sheet: *sheet,
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
        source_filter: d
            .source_filter
            .as_ref()
            .filter(|f| !f.conditions.is_empty())
            .map(source_filter_to_api),
    }
}

/// Every pivot of the document.
pub fn list(wb: &Workbook) -> Vec<PivotDto> {
    let mut out = Vec::new();
    for s in &wb.sheets {
        for p in s.meta.pivots.iter() {
            out.push(to_api(s.id, p));
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
                .sheet(*sheet)
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

/// AOO's caption of the empty item (`STR_EMPTYDATA`, `globstr.src`;
/// `dptabres.cxx:1414-1415`).
pub const EMPTY_CAPTION: &str = "(empty)";

/// A small, fast hasher for the integer keys of the pivot maps (node ids,
/// item indices).
#[derive(Clone, Copy, Debug, Default)]
pub struct Fx(u64);

impl Fx {
    fn add(&mut self, w: u64) {
        self.0 = (self.0.rotate_left(5) ^ w).wrapping_mul(0x517c_c1b7_2722_0a95);
    }
}

impl Hasher for Fx {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.add(u64::from(*b));
        }
    }
    fn write_u16(&mut self, i: u16) {
        self.add(u64::from(i));
    }
    fn write_u32(&mut self, i: u32) {
        self.add(u64::from(i));
    }
    fn write_u64(&mut self, i: u64) {
        self.add(i);
    }
    fn write_usize(&mut self, i: usize) {
        self.add(i as u64);
    }
}

/// A hash map with [`Fx`].
pub type FxMap<K, V> = HashMap<K, V, BuildHasherDefault<Fx>>;

/// The identity of an item. AOO `ScDPItemData::Compare`
/// (`dptablecache.cxx:265-295`): values compare by value (TRUE is the
/// value 1), strings case-insensitively (`global.cxx:1993`), and a value
/// never equals a string; group items compare by their name.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ItemKey {
    /// The empty item.
    Empty,
    /// A value: the bits of the value rounded to 15 significant digits
    /// (AOO compares values with `approxEqual`).
    Num(u64),
    /// A string, lowercased.
    Text(String),
    /// A numeric or date group, by name.
    Group(String),
}

/// One distinct item of a field.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// Caption: the display text of the first occurrence, `(empty)`, or
    /// the group name.
    pub text: String,
    /// Value (numbers, dates, booleans) or the sort value of a group.
    pub num: Option<f64>,
    /// The empty item (sorted last).
    pub empty: bool,
    /// The caption is written as a number (AOO
    /// `MemberResultFlags::NUMERIC`, `dpoutput.cxx:619-629`): ungrouped
    /// values only (`dptabres.cxx:1390-1395`).
    pub numeric: bool,
    /// Identity.
    pub key: ItemKey,
}

/// A source value as the aggregation sees it (AOO `ScDPValueData`): TRUE
/// and FALSE are values, as in the DataPilot cache.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cv {
    /// An empty cell (ignored by every function).
    Empty,
    /// A value.
    Num(f64),
    /// A string (counted by Count only).
    Str,
    /// An error (counted by Count; an error for every other function).
    Err,
}

/// The source read once, column-major: field names, per needed column the
/// distinct items and the item of every kept row, per data column the
/// values.
pub struct Cache {
    /// Field names (header row).
    pub names: Vec<String>,
    /// Items per column (empty for columns no field uses).
    pub items: Vec<Vec<Item>>,
    /// Per column and row: the item index.
    pub item_of: Vec<Vec<u32>>,
    /// Per column and row: the value (data columns only).
    pub values: Vec<Vec<Cv>>,
    /// Source row numbers (for drill-down).
    pub source_rows: Vec<u32>,
}

/// Rounds to 15 significant digits (AOO `rtl::math::approxValue`).
fn round15(x: f64) -> f64 {
    if x == 0.0 {
        return 0.0;
    }
    if !x.is_finite() {
        return x;
    }
    format!("{x:.14e}").parse().unwrap_or(x)
}

fn text_key(i: &Item) -> Cow<'_, str> {
    match &i.key {
        ItemKey::Text(t) => Cow::Borrowed(t.as_str()),
        _ => Cow::Owned(i.text.to_lowercase()),
    }
}

/// Item order: values before strings (case-insensitive), the empty item
/// last (AOO `ScDPItemData::Compare`).
fn item_order(a: &Item, b: &Item) -> Ordering {
    match (a.empty, b.empty) {
        (true, false) => return Ordering::Greater,
        (false, true) => return Ordering::Less,
        (true, true) => return Ordering::Equal,
        _ => {}
    }
    match (a.num, b.num) {
        (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(Ordering::Equal),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => text_key(a)
            .cmp(&text_key(b))
            .then_with(|| a.text.cmp(&b.text)),
    }
}

/// Distinct items of one column, keyed by identity, with fast paths for
/// exact repeats.
#[derive(Default)]
struct ItemIndex {
    exact_num: FxMap<u64, u32>,
    num: FxMap<u64, u32>,
    exact_text: HashMap<Arc<str>, u32>,
    text: HashMap<String, u32>,
    empty: Option<u32>,
}

impl ItemIndex {
    fn push(items: &mut Vec<Item>, item: Item) -> u32 {
        items.push(item);
        (items.len() - 1) as u32
    }

    fn empty(&mut self, items: &mut Vec<Item>) -> u32 {
        *self.empty.get_or_insert_with(|| {
            Self::push(
                items,
                Item {
                    text: EMPTY_CAPTION.to_string(),
                    num: None,
                    empty: true,
                    numeric: false,
                    key: ItemKey::Empty,
                },
            )
        })
    }

    fn text_item(&mut self, items: &mut Vec<Item>, text: String) -> u32 {
        let lower = text.to_lowercase();
        if let Some(i) = self.text.get(&lower) {
            return *i;
        }
        let i = Self::push(
            items,
            Item {
                text,
                num: None,
                empty: false,
                numeric: false,
                key: ItemKey::Text(lower.clone()),
            },
        );
        self.text.insert(lower, i);
        i
    }

    fn get(&mut self, items: &mut Vec<Item>, wb: &Workbook, a: CellAddr, v: &Value) -> u32 {
        match v {
            Value::Empty => self.empty(items),
            Value::Number(_) | Value::Bool(_) => {
                let n = match v {
                    Value::Number(n) => *n,
                    Value::Bool(b) => f64::from(u8::from(*b)),
                    _ => 0.0,
                };
                let bits = n.to_bits();
                if let Some(i) = self.exact_num.get(&bits) {
                    return *i;
                }
                let k = round15(n).to_bits();
                let i = match self.num.get(&k) {
                    Some(i) => *i,
                    None => {
                        let i = Self::push(
                            items,
                            Item {
                                text: display_text(wb, a),
                                num: Some(n),
                                empty: false,
                                numeric: true,
                                key: ItemKey::Num(k),
                            },
                        );
                        self.num.insert(k, i);
                        i
                    }
                };
                self.exact_num.insert(bits, i);
                i
            }
            Value::Text(t) => {
                if t.is_empty() {
                    return self.empty(items);
                }
                if let Some(i) = self.exact_text.get(t) {
                    return *i;
                }
                let i = self.text_item(items, t.to_string());
                self.exact_text.insert(t.clone(), i);
                i
            }
            Value::Error(_) | Value::Array(_) => {
                let t = display_text(wb, a);
                self.text_item(items, t)
            }
        }
    }
}

fn cv_of(v: &Value) -> Cv {
    match v {
        Value::Empty => Cv::Empty,
        Value::Number(n) => Cv::Num(*n),
        Value::Bool(b) => Cv::Num(f64::from(u8::from(*b))),
        Value::Text(t) if t.is_empty() => Cv::Empty,
        Value::Text(_) | Value::Array(_) => Cv::Str,
        Value::Error(_) => Cv::Err,
    }
}

/// Reads the source (header row = field names) with `ignore_empty_rows`.
/// Only the columns flagged in `need_items` get items and only those in
/// `need_values` keep their values; display texts are formatted once per
/// distinct item.
pub fn read_source(
    wb: &Workbook,
    src: &CellRange,
    ignore_empty: bool,
    need_items: &[bool],
    need_values: &[bool],
) -> Result<Cache, ApiError> {
    read_source_rows(wb, src, ignore_empty, need_items, need_values, None)
}

/// [`read_source`] over the data rows in `keep` only (the source filter;
/// `None` reads every row).
fn read_source_rows(
    wb: &Workbook,
    src: &CellRange,
    ignore_empty: bool,
    need_items: &[bool],
    need_values: &[bool],
    keep: Option<&HashSet<u32>>,
) -> Result<Cache, ApiError> {
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
    let want_items = |fi: usize| need_items.get(fi).copied().unwrap_or(false);
    let want_values = |fi: usize| need_values.get(fi).copied().unwrap_or(false);
    let mut items: Vec<Vec<Item>> = vec![Vec::new(); ncols];
    let mut index: Vec<ItemIndex> = (0..ncols).map(|_| ItemIndex::default()).collect();
    let mut item_of: Vec<Vec<u32>> = vec![Vec::new(); ncols];
    let mut values: Vec<Vec<Cv>> = vec![Vec::new(); ncols];
    let mut source_rows = Vec::new();
    let mut row: Vec<Value> = Vec::with_capacity(ncols);
    let mut total_items = 0usize;
    for r in src.r0 + 1..=src.r1 {
        if keep.is_some_and(|k| !k.contains(&r)) {
            continue;
        }
        row.clear();
        let mut all_empty = true;
        for (fi, c) in (src.c0..=src.c1).enumerate() {
            let v = if ignore_empty || want_items(fi) || want_values(fi) {
                wb.value(CellAddr::new(sheet, r, c))
            } else {
                Value::Empty
            };
            all_empty &= v.is_empty();
            row.push(v);
        }
        if ignore_empty && all_empty {
            continue;
        }
        for (fi, v) in row.iter().enumerate() {
            if want_items(fi) {
                let a = CellAddr::new(sheet, r, src.c0 + fi as u32);
                let before = items[fi].len();
                let i = index[fi].get(&mut items[fi], wb, a, v);
                item_of[fi].push(i);
                total_items += items[fi].len() - before;
                if total_items > MAX_CACHE_ITEMS {
                    return Err(ApiError::unsupported(format!(
                        "the pivot source has more than {MAX_CACHE_ITEMS} distinct items"
                    )));
                }
            }
            if want_values(fi) {
                values[fi].push(cv_of(v));
            }
        }
        source_rows.push(r);
    }
    Ok(Cache {
        names,
        items,
        item_of,
        values,
        source_rows,
    })
}

/// The columns a definition needs items (axis and page fields) and
/// values (data fields) of.
fn needs(p: &PivotData, ncols: usize) -> (Vec<bool>, Vec<bool>) {
    let mut items = vec![false; ncols];
    let mut values = vec![false; ncols];
    for f in p
        .row_fields
        .iter()
        .chain(&p.col_fields)
        .chain(&p.page_fields)
    {
        if let Some(x) = items.get_mut(f.source_col as usize) {
            *x = true;
        }
    }
    for d in &p.data_fields {
        if let Some(x) = values.get_mut(d.source_col as usize) {
            *x = true;
        }
    }
    (items, values)
}

/// Reads the source of a definition.
fn read_for(wb: &Workbook, p: &PivotData) -> Result<(CellRange, Cache), ApiError> {
    let src = source_range(wb, &p.source)?;
    let (items, values) = needs(p, src.cols() as usize);
    let keep = p
        .source_filter
        .as_ref()
        .filter(|f| !f.conditions.is_empty())
        .map(|f| filtered_rows(wb, &src, f));
    let cache = read_source_rows(
        wb,
        &src,
        p.ignore_empty_rows,
        &items,
        &values,
        keep.as_ref(),
    )?;
    Ok((src, cache))
}

/// The source data rows the source filter keeps, as AOO's
/// `ScDPTableDataCache::ValidQuery` (`dptablecache.cxx:634`): the
/// standard filter's tests, AND binding tighter than OR; "No
/// duplications" is not applied (AOO's DataPilot ignores `bDuplicate`).
fn filtered_rows(wb: &Workbook, src: &CellRange, f: &PivotSourceFilterData) -> HashSet<u32> {
    let data = StandardFilterData {
        range: CellRangeLocal {
            r0: src.r0,
            c0: src.c0,
            r1: src.r1,
            c1: src.c1,
        },
        conditions: f
            .conditions
            .iter()
            .map(|c| StdConditionData {
                col: src.c0.saturating_add(c.col).min(src.c1),
                ..c.clone()
            })
            .collect(),
        options: StdFilterOptionsData {
            case_sensitive: f.case_sensitive,
            regex: f.regex,
            has_header: true,
            ..StdFilterOptionsData::default()
        },
        criteria: None,
    };
    crate::tools::visible_rows(wb, src.sheet, &data)
        .into_iter()
        .collect()
}

/// `pivot_field_items`.
pub fn field_items(
    wb: &Workbook,
    source: &PivotSource,
) -> Result<Vec<PivotFieldItemsDto>, ApiError> {
    let src = api_source_range(wb, source)?;
    let ncols = src.cols() as usize;
    let mut out = Vec::with_capacity(ncols);
    // One column at a time: the items budget applies per column and only
    // one column's items are held.
    for fi in 0..ncols {
        let mut need = vec![false; ncols];
        need[fi] = true;
        let cache = read_source(wb, &src, false, &need, &[])?;
        let name = &cache.names[fi];
        let mut counts: Vec<u32> = vec![0; cache.items[fi].len()];
        for it in &cache.item_of[fi] {
            counts[*it as usize] += 1;
        }
        let mut order: Vec<usize> = (0..cache.items[fi].len()).collect();
        order.sort_by(|a, b| item_order(&cache.items[fi][*a], &cache.items[fi][*b]));
        let non_empty: Vec<&Item> = cache.items[fi].iter().filter(|i| !i.empty).collect();
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
// Grouping (AOO dpgroup.cxx)
// ---------------------------------------------------------------------------

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// Locale and epoch for group names.
#[derive(Clone, Copy)]
struct Gx {
    epoch: DateEpoch,
    locale: &'static Locale,
}

/// `rtl::math::approxEqual`.
fn approx_eq(a: f64, b: f64) -> bool {
    if a == b {
        return true;
    }
    (a - b).abs() < a.abs() * (1.0 / (16_777_216.0 * 16_777_216.0))
}

/// `rtl::math::approxFloor`.
fn approx_floor(x: f64) -> f64 {
    round15(x).floor()
}

fn is_integer(x: f64) -> bool {
    approx_eq(x, approx_floor(x))
}

/// A number as `rtl::math::doubleToUString(.., Automatic, Max, .., true)`.
fn fmt_num(n: f64) -> String {
    let r = round15(n);
    if r == 0.0 {
        "0".into()
    } else {
        format!("{r}")
    }
}

/// A date as the standard date format's input-line string
/// (`lcl_AppendDateStr`, `dpgroup.cxx:153-159`).
fn date_str(serial: f64, gx: Gx) -> String {
    match tally_numfmt::parse_format(gx.locale.short_date) {
        Ok(ast) => {
            tally_numfmt::format_value_with(
                &Value::Number(serial.floor()),
                &ast,
                gx.locale,
                None,
                gx.epoch,
            )
            .text
        }
        Err(_) => fmt_num(serial),
    }
}

/// `ScDPNumGroupInfo` after `ScDPNumGroupDimension::GetNumEntries`
/// (`dpgroup.cxx:934-1016`): automatic start/end from the source values,
/// non-integer detection, date rounding.
struct NumInfo {
    start: f64,
    end: f64,
    step: f64,
    auto_end: bool,
    dates: bool,
    non_integer: bool,
}

impl NumInfo {
    fn new(start: Option<f64>, end: Option<f64>, step: f64, dates: bool, src: &[Item]) -> Self {
        let step = if step > 0.0 && step.is_finite() {
            step
        } else {
            1.0
        };
        let mut non_integer = start.is_some_and(|s| !is_integer(s))
            || end.is_some_and(|e| !is_integer(e))
            || !is_integer(step);
        let mut min = 0.0f64;
        let mut max = 0.0f64;
        let mut first = true;
        for it in src.iter().filter(|i| !i.empty) {
            let Some(v) = it.num else { continue };
            if first {
                min = v;
                max = v;
                first = false;
            } else {
                min = min.min(v);
                max = max.max(v);
            }
            if !non_integer && !is_integer(v) {
                non_integer = true;
            }
        }
        if dates {
            non_integer = false;
            min = approx_floor(min);
            max = approx_floor(max) + 1.0;
        }
        Self {
            start: start.filter(|s| s.is_finite()).unwrap_or(min),
            end: end.filter(|e| e.is_finite()).unwrap_or(max),
            step,
            auto_end: end.is_none(),
            dates,
            non_integer,
        }
    }

    /// `lcl_GetNumGroupName` (`dpgroup.cxx:161-200`).
    fn name(&self, lo: f64, gx: Gx) -> String {
        let mut hi = lo + self.step;
        if !self.non_integer && (self.dates || !approx_eq(hi, self.end)) {
            hi -= 1.0;
        }
        if hi > self.end && !self.auto_end {
            hi = self.end;
        }
        if self.dates {
            format!("{} - {}", date_str(lo, gx), date_str(hi, gx))
        } else {
            format!("{}-{}", fmt_num(lo), fmt_num(hi))
        }
    }

    fn special(&self, v: f64, first: bool, gx: Gx) -> String {
        let c = if first { '<' } else { '>' };
        if self.dates {
            format!("{c}{}", date_str(v, gx))
        } else {
            format!("{c}{}", fmt_num(v))
        }
    }

    /// `lcl_GetNumGroupForValue` (`dpgroup.cxx:226-268`): the group name
    /// and its sort value.
    fn group(&self, v: f64, gx: Gx) -> (String, f64) {
        if v < self.start && !approx_eq(v, self.start) {
            return (self.special(self.start, true, gx), self.start - self.step);
        }
        if v > self.end && !approx_eq(v, self.end) {
            return (self.special(self.end, false, gx), self.end + self.step);
        }
        let mut div = approx_floor((v - self.start) / self.step);
        let mut gs = self.start + div * self.step;
        if approx_eq(gs, self.end) && !approx_eq(gs, self.start) {
            if self.dates {
                return (self.special(self.end, false, gx), self.end + self.step);
            }
            // A group of only the end value joins the previous group.
            div -= 1.0;
            gs = self.start + div * self.step;
        }
        (self.name(gs, gx), gs)
    }
}

/// The date-part group of a value (`lcl_GetDatePartValue`,
/// `lcl_GetDateGroupName`, `dpgroup.cxx:439-536`): `<start` / `>end` for
/// values outside explicit limits, else the parts' names joined.
fn date_part_group(
    parts: &[DatePartData],
    start: Option<f64>,
    end: Option<f64>,
    n: f64,
    gx: Gx,
) -> Option<(String, f64)> {
    if let Some(s) = start.filter(|s| s.is_finite()) {
        if n < s && !approx_eq(n, s) {
            return Some((format!("<{}", date_str(s, gx)), f64::NEG_INFINITY));
        }
    }
    if let Some(e) = end.filter(|e| e.is_finite()) {
        if n > e && !approx_eq(n, e) {
            return Some((format!(">{}", date_str(e, gx)), f64::INFINITY));
        }
    }
    let (y, m, d) = ymd_from_serial(gx.epoch, n)?;
    let secs = ((n - approx_floor(n)) * 86_400.0 + 0.5).floor() as u32;
    let (hh, mm, ss) = (secs / 3600, (secs / 60) % 60, secs % 60);
    let mut ordered = parts.to_vec();
    ordered.sort_by_key(|p| match p {
        DatePartData::Years => 0,
        DatePartData::Quarters => 1,
        DatePartData::Months => 2,
        DatePartData::Days(_) => 3,
        DatePartData::Hours => 4,
        DatePartData::Minutes => 5,
        DatePartData::Seconds => 6,
    });
    ordered.dedup_by(|a, b| std::mem::discriminant(a) == std::mem::discriminant(b));
    let mut text_parts: Vec<String> = Vec::new();
    let mut key = 0.0;
    for p in &ordered {
        match p {
            DatePartData::Years => {
                text_parts.push(y.to_string());
                key = key * 100_000.0 + f64::from(y);
            }
            DatePartData::Quarters => {
                let q = (m - 1) / 3 + 1;
                text_parts.push(format!("Q{q}"));
                key = key * 10.0 + f64::from(q);
            }
            DatePartData::Months => {
                text_parts.push(MONTHS[(m - 1) as usize % 12].to_string());
                key = key * 100.0 + f64::from(m);
            }
            DatePartData::Days(_) => {
                // Day of the year, 1..366 counted as in a leap year; the
                // name is `DD-MMM` (NF_DATE_SYS_DDMMM).
                let jan1 = tally_numfmt::date::serial_from_ymd(gx.epoch, y, 1, 1).unwrap_or(n);
                let mut doy = (approx_floor(n) - jan1) + 1.0;
                if doy >= 60.0 && !tally_numfmt::date::is_leap_year(i64::from(y)) {
                    doy += 1.0;
                }
                text_parts.push(format!("{d:02}-{}", MONTHS[(m - 1) as usize % 12]));
                key = key * 1000.0 + doy;
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
        return None;
    }
    Some((text_parts.join(" ").replace(" :", ":"), key))
}

/// The grouped items of a field and, per source item, its group.
/// Strings and the empty item stay as they are.
fn group_items(g: &PivotGroupData, src: &[Item], gx: Gx) -> (Vec<Item>, Vec<u32>) {
    let mut out: Vec<Item> = Vec::new();
    let mut index: HashMap<ItemKey, u32> = HashMap::new();
    fn add(out: &mut Vec<Item>, index: &mut HashMap<ItemKey, u32>, it: Item) -> u32 {
        if let Some(i) = index.get(&it.key) {
            return *i;
        }
        let i = out.len() as u32;
        index.insert(it.key.clone(), i);
        out.push(it);
        i
    }
    let group = |name: String, sort: f64| Item {
        text: name.clone(),
        num: Some(sort),
        empty: false,
        numeric: false,
        key: ItemKey::Group(name),
    };
    // Days alone with a step: numeric grouping of date values (AOO
    // `dbfunc3.cxx:1083-1093`).
    let num_info = match g {
        PivotGroupData::Numeric { start, end, step } => {
            Some(NumInfo::new(*start, *end, *step, false, src))
        }
        PivotGroupData::Date { start, end, parts } => match parts.as_slice() {
            [DatePartData::Days(n)] if *n >= 2 => {
                Some(NumInfo::new(*start, *end, f64::from(*n), true, src))
            }
            _ => None,
        },
    };
    let map = src
        .iter()
        .map(|it| {
            let Some(v) = it.num.filter(|_| !it.empty) else {
                return add(&mut out, &mut index, it.clone());
            };
            let grouped = match (&num_info, g) {
                (Some(info), _) => {
                    let (name, sort) = info.group(v, gx);
                    Some(group(name, sort))
                }
                (None, PivotGroupData::Date { start, end, parts }) => {
                    date_part_group(parts, *start, *end, v, gx).map(|(n, s)| group(n, s))
                }
                (None, PivotGroupData::Numeric { .. }) => None,
            };
            add(&mut out, &mut index, grouped.unwrap_or_else(|| it.clone()))
        })
        .collect();
    (out, map)
}

// ---------------------------------------------------------------------------
// Computation
// ---------------------------------------------------------------------------

/// A result cell: empty, a value or an error (AOO
/// `SC_DPAGG_RESULT_EMPTY/VALID/ERROR`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Res {
    /// No data.
    Empty,
    /// A value.
    Num(f64),
    /// An error (output as `#VALUE!`, `dpoutput.cxx:573-575`).
    Err,
}

impl Res {
    fn value(self) -> Value {
        match self {
            Res::Empty => Value::Empty,
            Res::Num(n) => Value::Number(n),
            Res::Err => Value::Error(ErrorKind::Value),
        }
    }
    fn num_or_zero(self) -> f64 {
        match self {
            Res::Num(n) => n,
            _ => 0.0,
        }
    }
}

/// The running aggregate of one result cell, able to answer every
/// function (AOO `ScDPAggData::Update/Calculate`, `dptabres.cxx:415-597`).
#[derive(Clone, Copy, Debug, Default)]
struct Acc {
    /// Non-empty values, strings and errors (Count = CNT2).
    all: u64,
    /// Values.
    n: u64,
    /// An error was seen.
    err: bool,
    sum: f64,
    prod: f64,
    min: f64,
    max: f64,
    mean: f64,
    m2: f64,
}

impl Acc {
    fn push(&mut self, v: Cv) {
        match v {
            Cv::Empty => {}
            Cv::Str => self.all += 1,
            Cv::Err => {
                self.all += 1;
                self.err = true;
            }
            Cv::Num(x) => {
                self.all += 1;
                self.n += 1;
                self.sum += x;
                if self.n == 1 {
                    self.prod = x;
                    self.min = x;
                    self.max = x;
                } else {
                    self.prod *= x;
                    self.min = self.min.min(x);
                    self.max = self.max.max(x);
                }
                let delta = x - self.mean;
                self.mean += delta / self.n as f64;
                self.m2 += delta * (x - self.mean);
            }
        }
    }

    /// The result for `f`: Count counts strings and errors; every other
    /// function ignores strings, is an error after an error value, and
    /// is empty without values; StDev/Var need two values.
    fn result(&self, f: AggFnData) -> Res {
        let fin = |x: f64| {
            if x.is_finite() {
                Res::Num(x)
            } else {
                Res::Err
            }
        };
        if f == AggFnData::Count {
            return if self.all == 0 {
                Res::Empty
            } else {
                Res::Num(self.all as f64)
            };
        }
        if self.err {
            return Res::Err;
        }
        if self.n == 0 {
            return Res::Empty;
        }
        let n = self.n as f64;
        match f {
            AggFnData::Count | AggFnData::CountNums => Res::Num(n),
            AggFnData::Sum => fin(self.sum),
            AggFnData::Product => fin(self.prod),
            AggFnData::Average => fin(self.sum / n),
            AggFnData::Max => fin(self.max),
            AggFnData::Min => fin(self.min),
            AggFnData::StDev if self.n < 2 => Res::Err,
            AggFnData::Var if self.n < 2 => Res::Err,
            AggFnData::StDev => fin((self.m2 / (n - 1.0)).max(0.0).sqrt()),
            AggFnData::Var => fin(self.m2 / (n - 1.0)),
            AggFnData::StDevP => fin((self.m2 / n).max(0.0).sqrt()),
            AggFnData::VarP => fin(self.m2 / n),
        }
    }
}

/// An axis or page field after grouping.
struct AxisField<'a> {
    source_col: usize,
    name: String,
    items: Cow<'a, [Item]>,
    /// Per source row: the item index.
    item_of: Cow<'a, [u32]>,
    def: &'a PivotFieldData,
    /// Per item: hidden by the definition.
    hidden: Vec<bool>,
    /// Per item: position in the manual order.
    manual: Option<Vec<usize>>,
}

/// The tree of item paths of one axis: node 0 is the root, every other
/// node one item under its parent.
#[derive(Clone, Debug)]
struct Trie {
    parent: Vec<u32>,
    item: Vec<u32>,
    level: Vec<u16>,
    edges: FxMap<(u32, u32), u32>,
}

/// Child lists of every node in one flat vector (compressed rows).
#[derive(Clone, Debug, Default)]
struct Csr {
    start: Vec<u32>,
    len: Vec<u32>,
    flat: Vec<u32>,
}

impl Csr {
    fn get(&self, n: u32) -> &[u32] {
        match (self.start.get(n as usize), self.len.get(n as usize)) {
            (Some(s), Some(l)) => &self.flat[*s as usize..(*s + *l) as usize],
            _ => &[],
        }
    }
}

impl Trie {
    fn new() -> Self {
        Self {
            parent: vec![u32::MAX],
            item: vec![u32::MAX],
            level: vec![0],
            edges: FxMap::default(),
        }
    }

    fn len(&self) -> usize {
        self.parent.len()
    }

    fn child(&self, p: u32, it: u32) -> Option<u32> {
        self.edges.get(&(p, it)).copied()
    }

    fn child_or_insert(&mut self, p: u32, it: u32) -> u32 {
        if let Some(n) = self.child(p, it) {
            return n;
        }
        let n = self.parent.len() as u32;
        let lvl = self.level[p as usize] + 1;
        self.parent.push(p);
        self.item.push(it);
        self.level.push(lvl);
        self.edges.insert((p, it), n);
        n
    }

    fn level(&self, n: u32) -> usize {
        self.level[n as usize] as usize
    }

    fn path(&self, mut n: u32) -> Vec<u32> {
        let mut v = Vec::with_capacity(self.level(n));
        while n != 0 {
            v.push(self.item[n as usize]);
            n = self.parent[n as usize];
        }
        v.reverse();
        v
    }

    /// The children of every node, in creation order.
    fn kids(&self) -> Csr {
        let n = self.len();
        let mut len = vec![0u32; n];
        for p in self.parent.iter().skip(1) {
            len[*p as usize] += 1;
        }
        let mut start = vec![0u32; n];
        let mut acc = 0u32;
        for (s, l) in start.iter_mut().zip(&len) {
            *s = acc;
            acc += l;
        }
        let mut fill = start.clone();
        let mut flat = vec![0u32; acc as usize];
        for (c, p) in self.parent.iter().enumerate().skip(1) {
            let f = &mut fill[*p as usize];
            flat[*f as usize] = c as u32;
            *f += 1;
        }
        Csr { start, len, flat }
    }

    fn ancestor(&self, mut n: u32, level: usize) -> u32 {
        while self.level(n) > level {
            n = self.parent[n as usize];
        }
        n
    }
}

/// One axis: its fields, the tree and the display children per node.
struct Axis<'a> {
    fields: Vec<AxisField<'a>>,
    trie: Trie,
    disp: Csr,
}

type Cells = FxMap<(u32, u32, u16), Acc>;

struct Computed<'a> {
    rows: Axis<'a>,
    cols: Axis<'a>,
    pages: Vec<AxisField<'a>>,
    /// `(row node, col node, data field)` → aggregate.
    cells: Cells,
    /// Source rows kept after page filters, hidden and auto-hidden items.
    kept: Vec<u32>,
}

/// Most row × field updates of one aggregation.
const MAX_AGG_WORK: u64 = 100_000_000;
/// Most nodes of one axis tree (with AutoShow, before members are hidden).
const MAX_NODES: usize = 4_000_000;

fn axis_field<'a>(
    cache: &'a Cache,
    f: &'a PivotFieldData,
    gx: Gx,
) -> Result<AxisField<'a>, ApiError> {
    let sc = f.source_col as usize;
    if sc >= cache.names.len() {
        return Err(ApiError::invalid_argument(format!(
            "field column {sc} is outside the source"
        )));
    }
    let (items, item_of): (Cow<'a, [Item]>, Cow<'a, [u32]>) = match &f.group {
        None => (
            Cow::Borrowed(&cache.items[sc][..]),
            Cow::Borrowed(&cache.item_of[sc][..]),
        ),
        Some(g) => {
            let (items, map) = group_items(g, &cache.items[sc], gx);
            let of: Vec<u32> = cache.item_of[sc].iter().map(|i| map[*i as usize]).collect();
            (Cow::Owned(items), Cow::Owned(of))
        }
    };
    let hidden_names: HashSet<&str> = f.hidden_items.iter().map(String::as_str).collect();
    let hidden: Vec<bool> = items
        .iter()
        .map(|i| hidden_names.contains(i.text.as_str()) || (i.empty && hidden_names.contains("")))
        .collect();
    let manual = match &f.sort {
        PivotSortData::Manual { order } => {
            let mut pos: HashMap<String, usize> = HashMap::with_capacity(order.len());
            for (k, o) in order.iter().enumerate() {
                pos.entry(o.to_lowercase()).or_insert(k);
            }
            Some(
                items
                    .iter()
                    .map(|i| pos.get(text_key(i).as_ref()).copied().unwrap_or(usize::MAX))
                    .collect(),
            )
        }
        _ => None,
    };
    Ok(AxisField {
        source_col: sc,
        name: if f.name.is_empty() {
            cache.names[sc].clone()
        } else {
            f.name.clone()
        },
        items,
        item_of,
        def: f,
        hidden,
        manual,
    })
}

/// Aggregates every (row prefix, column prefix, data field) combination
/// of the kept rows; prefixes are tree nodes.
fn aggregate(
    rows: &[AxisField<'_>],
    cols: &[AxisField<'_>],
    p: &PivotData,
    cache: &Cache,
    kept: &[u32],
) -> Result<(Trie, Trie, Cells), ApiError> {
    let limit = |fields: &[AxisField<'_>], sheet: u32| {
        if fields.iter().any(|f| f.def.auto_show.is_some()) {
            MAX_NODES
        } else {
            sheet as usize + 1
        }
    };
    let limits = (limit(rows, MAX_ROWS), limit(cols, MAX_COLS));
    let mut rt = Trie::new();
    let mut ct = Trie::new();
    let mut cells = Cells::default();
    let mut rp: Vec<u32> = Vec::with_capacity(rows.len() + 1);
    let mut cp: Vec<u32> = Vec::with_capacity(cols.len() + 1);
    let dcols: Vec<&[Cv]> = p
        .data_fields
        .iter()
        .map(|d| &cache.values[d.source_col as usize][..])
        .collect();
    for &r in kept {
        let r = r as usize;
        rp.clear();
        rp.push(0);
        let mut n = 0;
        for f in rows {
            n = rt.child_or_insert(n, f.item_of[r]);
            rp.push(n);
        }
        cp.clear();
        cp.push(0);
        let mut n = 0;
        for f in cols {
            n = ct.child_or_insert(n, f.item_of[r]);
            cp.push(n);
        }
        for (di, col) in dcols.iter().enumerate() {
            let v = col[r];
            for &a in &rp {
                for &b in &cp {
                    cells.entry((a, b, di as u16)).or_default().push(v);
                }
            }
        }
        // Without AutoShow every node is shown, so an axis tree larger
        // than the sheet can stop here (security review 4, H7).
        if cells.len() as u64 > MAX_OUTPUT_CELLS || rt.len() > limits.0 || ct.len() > limits.1 {
            return Err(ApiError::unsupported("the pivot result is too large"));
        }
    }
    Ok((rt, ct, cells))
}

fn res_at(cells: &Cells, r: u32, c: u32, di: usize, f: AggFnData) -> Res {
    cells
        .get(&(r, c, di as u16))
        .map_or(Res::Empty, |a| a.result(f))
}

/// The members AutoShow hides under each parent (AOO
/// `ScDPResultDimension::DoAutoShow`, `dptabres.cxx:3136-3190`): the
/// first `count` by the data field's total plus every member equal to the
/// last included one.
fn auto_hidden(
    trie: &Trie,
    fields: &[AxisField<'_>],
    cells: &Cells,
    p: &PivotData,
    rows_axis: bool,
) -> Vec<bool> {
    let mut hidden = vec![false; trie.len()];
    let all_kids = trie.kids();
    for n in 0..trie.len() {
        let lvl = trie.level[n] as usize;
        let Some(a) = fields.get(lvl).and_then(|f| f.def.auto_show.as_ref()) else {
            continue;
        };
        let kids = all_kids.get(n as u32);
        let count = a.count as usize;
        if count == 0 || count >= kids.len() || p.data_fields.is_empty() {
            continue;
        }
        let df = (a.data_field as usize).min(p.data_fields.len() - 1);
        let fun = p.data_fields[df].function;
        let val = |k: u32| -> Option<f64> {
            let r = if rows_axis {
                res_at(cells, k, 0, df, fun)
            } else {
                res_at(cells, 0, k, df, fun)
            };
            match r {
                Res::Num(x) => Some(x),
                _ => None,
            }
        };
        let mut ranked: Vec<(u32, Option<f64>)> = kids.iter().map(|k| (*k, val(*k))).collect();
        ranked.sort_by(|x, y| match (x.1, y.1) {
            (Some(a1), Some(b1)) => {
                let o = a1.partial_cmp(&b1).unwrap_or(Ordering::Equal);
                if a.top {
                    o.reverse()
                } else {
                    o
                }
            }
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        });
        let mut incl = count;
        while incl < ranked.len() && ranked[incl].1 == ranked[count - 1].1 {
            incl += 1;
        }
        for (k, _) in &ranked[incl..] {
            hidden[*k as usize] = true;
        }
    }
    hidden
}

fn path_hidden(trie: &Trie, fields: &[AxisField<'_>], r: usize, hidden: &[bool]) -> bool {
    let mut n = 0;
    for f in fields {
        match trie.child(n, f.item_of[r]) {
            Some(c) => n = c,
            None => return false,
        }
        if hidden[n as usize] {
            return true;
        }
    }
    false
}

/// The display children of every node: the items with data (or every
/// visible item with "show items without data"), sorted. Creating more
/// than `budget` nodes is refused before anything is laid out.
fn build_display(
    trie: &mut Trie,
    fields: &[AxisField<'_>],
    cells: &Cells,
    p: &PivotData,
    rows_axis: bool,
    budget: usize,
) -> Result<Csr, ApiError> {
    let data_kids = trie.kids();
    let mut disp = Csr::default();
    let mut created = 0usize;
    let mut stack: Vec<u32> = vec![0];
    while let Some(node) = stack.pop() {
        let lvl = trie.level(node);
        let Some(f) = fields.get(lvl) else { continue };
        let mut present: Vec<u32> = data_kids
            .get(node)
            .iter()
            .map(|k| trie.item[*k as usize])
            .collect();
        if f.def.show_empty {
            let visible = f.hidden.iter().filter(|h| !**h).count();
            let auto_limits = f
                .def
                .auto_show
                .as_ref()
                .is_some_and(|a| (a.count as usize) < visible);
            if !auto_limits {
                let mut have = vec![false; f.items.len()];
                for it in &present {
                    have[*it as usize] = true;
                }
                for (it, h) in f.hidden.iter().enumerate() {
                    if !have[it] && !*h {
                        present.push(it as u32);
                    }
                }
            }
        }
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
                let df = (*data_field as usize).min(p.data_fields.len().saturating_sub(1));
                let fun = p.data_fields.get(df).map_or(AggFnData::Sum, |d| d.function);
                let mut keyed: Vec<(u32, f64)> = present
                    .iter()
                    .map(|it| {
                        let v = trie.child(node, *it).map_or(0.0, |c| {
                            if rows_axis {
                                res_at(cells, c, 0, df, fun).num_or_zero()
                            } else {
                                res_at(cells, 0, c, df, fun).num_or_zero()
                            }
                        });
                        (*it, v)
                    })
                    .collect();
                keyed.sort_by(|a, b| {
                    let o = a.1.partial_cmp(&b.1).unwrap_or(Ordering::Equal);
                    if *ascending {
                        o
                    } else {
                        o.reverse()
                    }
                });
                present = keyed.into_iter().map(|(it, _)| it).collect();
            }
            PivotSortData::Manual { .. } => {
                if let Some(pos) = &f.manual {
                    present.sort_by_key(|it| pos[*it as usize]);
                }
            }
        }
        let first = disp.flat.len();
        for it in present {
            disp.flat.push(trie.child_or_insert(node, it));
            created += 1;
            if created > budget {
                return Err(ApiError::unsupported("the pivot output is too large"));
            }
        }
        if disp.start.len() <= node as usize {
            disp.start.resize(node as usize + 1, 0);
            disp.len.resize(node as usize + 1, 0);
        }
        disp.start[node as usize] = first as u32;
        disp.len[node as usize] = (disp.flat.len() - first) as u32;
        stack.extend(disp.flat[first..].iter().rev().copied());
    }
    Ok(disp)
}

fn compute<'a>(
    wb: &Workbook,
    p: &'a PivotData,
    cache: &'a Cache,
) -> Result<Computed<'a>, ApiError> {
    let gx = Gx {
        epoch: wb.settings.date_epoch,
        locale: wb.locale,
    };
    if p.data_fields.is_empty() {
        return Err(ApiError::invalid_argument("a pivot needs a data field"));
    }
    for d in &p.data_fields {
        if d.source_col as usize >= cache.names.len() {
            return Err(ApiError::invalid_argument(
                "a data field is outside the source",
            ));
        }
    }
    let rows: Vec<AxisField<'a>> = p
        .row_fields
        .iter()
        .map(|f| axis_field(cache, f, gx))
        .collect::<Result<_, _>>()?;
    let cols: Vec<AxisField<'a>> = p
        .col_fields
        .iter()
        .map(|f| axis_field(cache, f, gx))
        .collect::<Result<_, _>>()?;
    let pages: Vec<AxisField<'a>> = p
        .page_fields
        .iter()
        .map(|f| axis_field(cache, f, gx))
        .collect::<Result<_, _>>()?;
    // Page filters and hidden items.
    let page_sel: Vec<Option<Vec<bool>>> = pages
        .iter()
        .map(|pf| {
            pf.def.page_item.as_ref().map(|s| {
                pf.items
                    .iter()
                    .map(|i| i.text == *s || (i.empty && s.is_empty()))
                    .collect()
            })
        })
        .collect();
    let n = cache.source_rows.len();
    let mut kept: Vec<u32> = (0..n)
        .filter(|&r| {
            pages
                .iter()
                .zip(&page_sel)
                .all(|(pf, sel)| sel.as_ref().is_none_or(|s| s[pf.item_of[r] as usize]))
                && rows
                    .iter()
                    .chain(&cols)
                    .all(|f| !f.hidden[f.item_of[r] as usize])
        })
        .map(|r| r as u32)
        .collect();
    let work = kept.len() as u64
        * (rows.len() as u64 + 1)
        * (cols.len() as u64 + 1)
        * p.data_fields.len() as u64;
    if work > MAX_AGG_WORK {
        return Err(ApiError::unsupported(
            "the pivot table is too large to compute",
        ));
    }
    let (mut rt, mut ct, mut cells) = aggregate(&rows, &cols, p, cache, &kept)?;
    // AutoShow: hide the members, then compute again without them so the
    // totals exclude them (AOO `dptabsrc.cxx:986-995`).
    if rows.iter().chain(&cols).any(|f| f.def.auto_show.is_some()) {
        let rh = auto_hidden(&rt, &rows, &cells, p, true);
        let ch = auto_hidden(&ct, &cols, &cells, p, false);
        if rh.iter().chain(&ch).any(|h| *h) {
            kept.retain(|&r| {
                !path_hidden(&rt, &rows, r as usize, &rh)
                    && !path_hidden(&ct, &cols, r as usize, &ch)
            });
            let (rt2, ct2, cells2) = aggregate(&rows, &cols, p, cache, &kept)?;
            rt = rt2;
            ct = ct2;
            cells = cells2;
        }
    }
    let rdisp = build_display(&mut rt, &rows, &cells, p, true, MAX_ROWS as usize)?;
    let cdisp = build_display(&mut ct, &cols, &cells, p, false, MAX_COLS as usize)?;
    Ok(Computed {
        rows: Axis {
            fields: rows,
            trie: rt,
            disp: rdisp,
        },
        cols: Axis {
            fields: cols,
            trie: ct,
            disp: cdisp,
        },
        pages,
        cells,
        kept,
    })
}

// ---------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------

/// What an axis line is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// An innermost item (or the single line of an axis without fields).
    Detail,
    /// The n-th subtotal of the item at the node.
    Sub(u8),
    /// The grand total.
    Grand,
    /// A blank spacer.
    Blank,
    /// The title row of an item in outline layout (its children start on
    /// the next row; no values).
    Title,
}

/// One line of an axis.
#[derive(Clone, Copy, Debug)]
struct Line {
    node: u32,
    kind: Kind,
    /// Data field when the Data pseudo-field is on this axis.
    df: Option<u16>,
    /// A subtotal shown in the item's outline title row ("subtotals at
    /// the top" with a single subtotal row, AOO
    /// `ScDPResultMember::IsSubTotalInTitle`): captioned with the item
    /// itself and styled as an item row.
    title: bool,
}

impl Line {
    /// A styled total line (a subtotal in a title row is not one).
    fn total(&self) -> bool {
        matches!(self.kind, Kind::Sub(_) | Kind::Grand) && !self.title
    }

    /// A line without values.
    fn empty(&self) -> bool {
        matches!(self.kind, Kind::Blank | Kind::Title)
    }
}

/// Most subtotal functions per field.
const MAX_SUBTOTAL_FNS: usize = 16;

fn subtotal_fns(f: &PivotFieldData) -> Vec<Option<AggFnData>> {
    match &f.subtotals {
        PivotSubtotalsData::None => Vec::new(),
        // Auto: the data field's own function, labelled "Result".
        PivotSubtotalsData::Auto => vec![None],
        PivotSubtotalsData::Custom(fns) => {
            if fns.is_empty() {
                vec![None]
            } else {
                fns.iter()
                    .take(MAX_SUBTOTAL_FNS)
                    .map(|f| Some(*f))
                    .collect()
            }
        }
    }
}

/// Flattens an axis into lines (AOO `ScDPResultMember::FillMemberResults`
/// and `GetSize`, `dptabres.cxx:1205-1240,1369-1543`):
///
/// - no subtotals for the innermost field, even when only the Data
///   pseudo-field follows it (`lcl_TestSubTotal`, `dptabsrc.cxx:229-245`);
/// - the layout options (outline, empty lines) apply to row fields only,
///   and not to the innermost row level, which is the Data pseudo-field
///   when it is on the rows (`ScDPSource::CreateRes_Impl`,
///   `dptabsrc.cxx:690,935`; `ScDPLevel::IsOutlineLayout`);
/// - outline layout: a title row with the item, its children from the
///   next row on; "subtotals at the top" with exactly one subtotal row
///   (one function, one data field on this axis) puts the subtotal in the
///   title row, otherwise the subtotals follow the children
///   (`IsSubTotalInTitle`, `dptabres.cxx:1183-1203`);
/// - the empty line comes after the item's subtotals.
///
/// Refused once more than `limit` lines would be built (the output
/// cannot hold them anyway), so title and empty lines stay within the
/// output budget before anything else is materialized.
fn axis_lines(
    ax: &Axis<'_>,
    ndata: usize,
    data_here: bool,
    grand: bool,
    rows_axis: bool,
    limit: usize,
) -> Result<Vec<Line>, ApiError> {
    struct Ctx<'x, 'a> {
        ax: &'x Axis<'a>,
        ndata: usize,
        data_here: bool,
        rows_axis: bool,
        limit: usize,
        out: Vec<Line>,
    }
    impl Ctx<'_, '_> {
        fn one(
            &mut self,
            node: u32,
            kind: Kind,
            df: Option<u16>,
            title: bool,
        ) -> Result<(), ApiError> {
            if self.out.len() >= self.limit {
                return Err(ApiError::unsupported("the pivot output is too large"));
            }
            self.out.push(Line {
                node,
                kind,
                df,
                title,
            });
            Ok(())
        }
        /// One line, or one per data field when the Data field is here.
        fn push(&mut self, node: u32, kind: Kind, title: bool) -> Result<(), ApiError> {
            if self.data_here {
                for di in 0..self.ndata {
                    self.one(node, kind, Some(di as u16), title)?;
                }
                Ok(())
            } else {
                self.one(node, kind, None, title)
            }
        }
        fn walk(&mut self, node: u32) -> Result<(), ApiError> {
            let ax = self.ax;
            let nf = ax.fields.len();
            let lvl = ax.trie.level(node);
            if lvl == nf {
                return self.push(node, Kind::Detail, false);
            }
            let f = &ax.fields[lvl];
            let subs = if lvl + 1 < nf {
                subtotal_fns(f.def).len()
            } else {
                0
            };
            let enabled = self.rows_axis && (lvl + 1 < nf || self.data_here);
            let outline = enabled && f.def.layout != PivotLayoutData::Tabular;
            let sub_rows = subs * if self.data_here { self.ndata } else { 1 };
            let in_title = outline && f.def.layout == PivotLayoutData::OutlineTop && sub_rows == 1;
            let empty_line = enabled && f.def.empty_line_after;
            for &child in ax.disp.get(node) {
                if in_title {
                    self.push(child, Kind::Sub(0), true)?;
                } else if outline {
                    self.one(child, Kind::Title, None, false)?;
                }
                self.walk(child)?;
                if !in_title {
                    for i in 0..subs {
                        self.push(child, Kind::Sub(i as u8), false)?;
                    }
                }
                if empty_line {
                    self.one(child, Kind::Blank, None, false)?;
                }
            }
            Ok(())
        }
    }
    let mut cx = Ctx {
        ax,
        ndata,
        data_here,
        rows_axis,
        limit,
        out: Vec::new(),
    };
    if ax.fields.is_empty() {
        let kind = if grand && data_here {
            Kind::Grand
        } else {
            Kind::Detail
        };
        cx.push(0, kind, false)?;
        return Ok(cx.out);
    }
    cx.walk(0)?;
    if grand {
        cx.push(0, Kind::Grand, false)?;
    }
    Ok(cx.out)
}

/// The function name in output captions (AOO `nFuncStrIds`,
/// `dptabres.cxx:71-85`, en-US strings `globstr.src:695-730`): Count and
/// Count (numbers only) are both "Count", the sample and population
/// variants "StDev" / "Var". The dialog's function list keeps the long
/// names ([`agg_caption`]). The output is not localised (as AOO en-US).
pub fn measure_fn_name(f: AggFnData) -> &'static str {
    match f {
        AggFnData::Sum => "Sum",
        AggFnData::Count | AggFnData::CountNums => "Count",
        AggFnData::Average => "Average",
        AggFnData::Max => "Max",
        AggFnData::Min => "Min",
        AggFnData::Product => "Product",
        AggFnData::StDev | AggFnData::StDevP => "StDev",
        AggFnData::Var | AggFnData::VarP => "Var",
    }
}

/// `<function> - <source field>` with `fname` naming the function.
fn fn_field(fname: &str, d: &PivotDataFieldData, names: &[String]) -> String {
    format!(
        "{fname} - {}",
        names.get(d.source_col as usize).map_or("", String::as_str)
    )
}

/// The caption of a data field (AOO `ScDPResultData::GetMeasureString`:
/// its layout name, else "Sum - Revenue").
pub fn caption(d: &PivotDataFieldData, names: &[String]) -> String {
    match &d.caption {
        Some(c) if !c.trim().is_empty() => c.clone(),
        _ => fn_field(measure_fn_name(d.function), d, names),
    }
}

/// A label cell of the output.
struct Label {
    r: u32,
    c: u32,
    text: String,
    /// Written as a number (numeric item).
    numeric: bool,
    role: PivotCellRole,
    field: Option<u32>,
    item: Option<String>,
    /// The item is the text (not stored twice).
    item_is_text: bool,
}

struct Layout {
    labels: Vec<Label>,
    rows: u32,
    cols: u32,
    first_data_row: u32,
    data_col0: u32,
    row_lines: Vec<Line>,
    col_lines: Vec<Line>,
    /// Displayed values, row line × column line.
    values: Vec<Res>,
}

/// The function of a subtotal line forced by custom subtotals.
fn forced(ax: &Axis<'_>, l: &Line) -> Option<AggFnData> {
    let Kind::Sub(i) = l.kind else { return None };
    let lvl = ax.trie.level(l.node);
    match &ax.fields.get(lvl.checked_sub(1)?)?.def.subtotals {
        PivotSubtotalsData::Custom(v) if !v.is_empty() => v.get(i as usize).copied(),
        _ => None,
    }
}

/// The data field and function of a cell; `None` when the row and column
/// subtotals force different functions (AOO `ScDPAggData::Update`: no
/// value then).
fn cell_fn(p: &PivotData, comp: &Computed<'_>, rl: &Line, cl: &Line) -> (usize, Option<AggFnData>) {
    let di = rl.df.or(cl.df).unwrap_or(0) as usize;
    let own = p.data_fields[di].function;
    let f = match (forced(&comp.rows, rl), forced(&comp.cols, cl)) {
        (Some(a), Some(b)) if a != b => None,
        (Some(a), _) => Some(a),
        (_, Some(b)) => Some(b),
        _ => Some(own),
    };
    (di, f)
}

/// The items of `node`'s path on levels `0..levels` that the previous
/// labelled line `prev` did not already write: `(level, ancestor node)`.
fn unwritten(
    trie: &Trie,
    node: u32,
    levels: usize,
    prev: Option<u32>,
) -> impl Iterator<Item = (usize, u32)> + '_ {
    (0..levels).filter_map(move |lvl| {
        let anc = trie.ancestor(node, lvl + 1);
        let same = prev.is_some_and(|pn| trie.level(pn) > lvl && trie.ancestor(pn, lvl + 1) == anc);
        (!same).then_some((lvl, anc))
    })
}

fn item_text<'b, 'a>(ax: &'b Axis<'a>, node: u32) -> (&'b Item, &'b AxisField<'a>) {
    let lvl = ax.trie.level(node) - 1;
    let f = &ax.fields[lvl];
    (&f.items[ax.trie.item[node as usize] as usize], f)
}

fn lay_out(p: &PivotData, cache: &Cache, comp: &Computed<'_>) -> Result<Layout, ApiError> {
    let ndata = p.data_fields.len();
    let data_in_rows = ndata > 1 && p.data_layout == DataLayoutData::Row;
    let data_in_cols = ndata > 1 && p.data_layout == DataLayoutData::Column;
    let row_lines = axis_lines(
        &comp.rows,
        ndata,
        data_in_rows,
        p.row_totals,
        true,
        MAX_ROWS as usize,
    )?;
    let col_lines = axis_lines(
        &comp.cols,
        ndata,
        data_in_cols,
        p.col_totals,
        false,
        MAX_COLS as usize,
    )?;
    let nr = comp.rows.fields.len() as u32 + u32::from(data_in_rows);
    let nc = comp.cols.fields.len() as u32 + u32::from(data_in_cols);
    let captions: Vec<String> = p
        .data_fields
        .iter()
        .map(|d| caption(d, &cache.names))
        .collect();
    let mut cells: Vec<Label> = Vec::new();
    let mut r = 0u32;
    let push = |cells: &mut Vec<Label>,
                r: u32,
                c: u32,
                text: &str,
                role: PivotCellRole,
                field: Option<u32>,
                item: Option<String>| {
        cells.push(Label {
            r,
            c,
            text: text.to_string(),
            numeric: false,
            role,
            field,
            item,
            item_is_text: false,
        });
    };
    let push_item = |cells: &mut Vec<Label>, r: u32, c: u32, it: &Item, f: &AxisField<'_>| {
        cells.push(Label {
            r,
            c,
            text: it.text.clone(),
            numeric: it.numeric,
            role: PivotCellRole::RowItem,
            field: Some(f.source_col as u32),
            item: None,
            item_is_text: true,
        });
    };
    // The measure part of a total caption (AOO
    // `ScDPResultData::GetMeasureString`, `dptabres.cxx:867-903`): the
    // measure is the line's data field when the Data field is on this
    // axis, the only data field when there is one, else "any". "Any", or a
    // single data field with the automatic function, gives "Result" (or
    // the forced function's name); otherwise the data field's caption with
    // the forced function: "East Sum - Revenue", "East Max - Revenue".
    let measure = |data_here: bool, df: Option<u16>, force: Option<AggFnData>| -> String {
        let m = if data_here {
            df.map(usize::from)
        } else if ndata == 1 {
            Some(0)
        } else {
            None
        };
        match m.and_then(|m| p.data_fields.get(m)) {
            Some(d) if !(ndata == 1 && force.is_none()) => match &d.caption {
                Some(c) if !c.trim().is_empty() => c.clone(),
                _ => fn_field(
                    measure_fn_name(force.unwrap_or(d.function)),
                    d,
                    &cache.names,
                ),
            },
            _ => force.map_or("Result", measure_fn_name).to_string(),
        }
    };
    let subtotal_text = |ax: &Axis<'_>, l: &Line, data_here: bool| -> (String, String, u32) {
        let (it, f) = item_text(ax, l.node);
        (
            format!("{} {}", it.text, measure(data_here, l.df, forced(ax, l))),
            it.text.clone(),
            f.source_col as u32,
        )
    };
    // The grand total: the root member "Total" with the measure ("Total
    // Result", or "Total Sum - Revenue" per data field on this axis).
    let grand_text = |ax: &Axis<'_>, l: &Line, data_here: bool| -> String {
        if ax.fields.is_empty() {
            "Total Result".to_string()
        } else {
            format!("Total {}", measure(data_here, l.df, None))
        }
    };
    // Filter button and page fields.
    let mut had_top = false;
    if p.show_filter_button {
        // AOO `lcl_DoFilterButton` (`dpoutput.cxx:692`): the `Filter`
        // caption with the button flag; the grid draws it as a push
        // button that opens the source filter (`ScGridWindow::DoPushButton`).
        push(
            &mut cells,
            r,
            0,
            "Filter",
            PivotCellRole::FilterButton,
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
            &name,
            PivotCellRole::PageField,
            Some(pf.source_col),
            None,
        );
        let sel = pf.page_item.clone().unwrap_or_else(|| "- all -".into());
        push(
            &mut cells,
            r,
            1,
            &sel,
            PivotCellRole::PageField,
            Some(pf.source_col),
            Some(sel.clone()),
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
    let corner = if ndata == 1 {
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
        &corner,
        PivotCellRole::Header,
        None,
        None,
    );
    let data_col0 = nr.max(1);
    for (k, f) in comp.cols.fields.iter().enumerate() {
        push(
            &mut cells,
            top,
            data_col0 + k as u32,
            &f.name,
            PivotCellRole::Header,
            Some(f.source_col as u32),
            None,
        );
    }
    if data_in_cols {
        push(
            &mut cells,
            top,
            data_col0 + comp.cols.fields.len() as u32,
            "Data",
            PivotCellRole::Header,
            None,
            None,
        );
    }
    let last_header = top + header_rows - 1;
    for (k, f) in comp.rows.fields.iter().enumerate() {
        push(
            &mut cells,
            last_header,
            k as u32,
            &f.name,
            PivotCellRole::Header,
            Some(f.source_col as u32),
            None,
        );
    }
    if data_in_rows {
        push(
            &mut cells,
            last_header,
            comp.rows.fields.len() as u32,
            "Data",
            PivotCellRole::Header,
            None,
            None,
        );
    }
    // Size check before anything per line is built.
    let first_data_row = top + header_rows;
    let rows = first_data_row as u64 + row_lines.len() as u64;
    let cols = (data_col0 as u64 + col_lines.len() as u64).max(2);
    if rows > u64::from(MAX_ROWS) || cols > u64::from(MAX_COLS) || rows * cols > MAX_OUTPUT_CELLS {
        return Err(ApiError::unsupported("the pivot output is too large"));
    }
    let (rows, cols) = (rows as u32, cols as u32);
    // Column items: level k on header row top + 1 + k. Every item is
    // written once, on the first line of its span, whatever that line is
    // (an item, or a subtotal of an inner item that has no children with
    // data); later lines do not repeat it (AOO: HASMEMBER at the member's
    // first position, CONTINUE after it, `dptabres.cxx:1427-1434`).
    let ct = &comp.cols.trie;
    let mut prev: Option<u32> = None;
    for (ci, line) in col_lines.iter().enumerate() {
        let c = data_col0 + ci as u32;
        match line.kind {
            Kind::Blank | Kind::Title => continue,
            Kind::Grand => {
                let rr = if nc == 0 { top } else { top + 1 };
                let text = grand_text(&comp.cols, line, data_in_cols);
                push(&mut cells, rr, c, &text, PivotCellRole::Total, None, None);
            }
            Kind::Sub(_) => {
                let (text, item, field) = subtotal_text(&comp.cols, line, data_in_cols);
                let lvl = ct.level(line.node) - 1;
                for (l, anc) in unwritten(ct, line.node, lvl, prev) {
                    let (it, f) = item_text(&comp.cols, anc);
                    push_item(&mut cells, top + 1 + l as u32, c, it, f);
                    if let Some(x) = cells.last_mut() {
                        x.role = PivotCellRole::ColItem;
                    }
                }
                prev = Some(line.node);
                push(
                    &mut cells,
                    top + 1 + lvl as u32,
                    c,
                    &text,
                    PivotCellRole::Total,
                    Some(field),
                    Some(item),
                );
            }
            Kind::Detail => {
                for (lvl, anc) in unwritten(ct, line.node, ct.level(line.node), prev) {
                    let (it, f) = item_text(&comp.cols, anc);
                    push_item(&mut cells, top + 1 + lvl as u32, c, it, f);
                    if let Some(l) = cells.last_mut() {
                        l.role = PivotCellRole::ColItem;
                    }
                }
                prev = Some(line.node);
            }
        }
        // Total lines carry the data field in their own caption; the Data
        // level stays empty there (AOO sets only the member name,
        // `dptabres.cxx:1514-1534`).
        let data_label = line.kind == Kind::Detail || comp.cols.fields.is_empty();
        if let (Some(di), true) = (line.df, data_label) {
            let cap = &captions[di as usize];
            push(
                &mut cells,
                top + nc,
                c,
                cap,
                PivotCellRole::ColItem,
                None,
                Some(cap.clone()),
            );
        }
    }
    // Row items.
    let rt = &comp.rows.trie;
    let mut prev: Option<u32> = None;
    for (ri, line) in row_lines.iter().enumerate() {
        let rr = first_data_row + ri as u32;
        match line.kind {
            Kind::Blank => continue,
            Kind::Grand => {
                let text = grand_text(&comp.rows, line, data_in_rows);
                push(&mut cells, rr, 0, &text, PivotCellRole::Total, None, None);
            }
            Kind::Sub(_) if !line.title => {
                let (text, item, field) = subtotal_text(&comp.rows, line, data_in_rows);
                let lvl = rt.level(line.node) - 1;
                for (l, anc) in unwritten(rt, line.node, lvl, prev) {
                    let (it, f) = item_text(&comp.rows, anc);
                    push_item(&mut cells, rr, l as u32, it, f);
                }
                prev = Some(line.node);
                push(
                    &mut cells,
                    rr,
                    lvl as u32,
                    &text,
                    PivotCellRole::Total,
                    Some(field),
                    Some(item),
                );
            }
            // Items, outline title rows and subtotals shown in a title row:
            // the item (and any outer item not written yet).
            Kind::Detail | Kind::Title | Kind::Sub(_) => {
                for (lvl, anc) in unwritten(rt, line.node, rt.level(line.node), prev) {
                    let (it, f) = item_text(&comp.rows, anc);
                    push_item(&mut cells, rr, lvl as u32, it, f);
                }
                prev = Some(line.node);
            }
        }
        let data_label = line.kind == Kind::Detail || comp.rows.fields.is_empty();
        if let (Some(di), true) = (line.df, data_label) {
            let cap = &captions[di as usize];
            push(
                &mut cells,
                rr,
                nr - 1,
                cap,
                PivotCellRole::RowItem,
                None,
                Some(cap.clone()),
            );
        }
    }
    // Data values.
    let ncl = col_lines.len();
    let mut values: Vec<Res> = Vec::with_capacity(row_lines.len() * ncl);
    for rl in &row_lines {
        for cl in &col_lines {
            if rl.empty() || cl.empty() {
                values.push(Res::Empty);
                continue;
            }
            let (di, f) = cell_fn(p, comp, rl, cl);
            values.push(match f {
                Some(f) => res_at(&comp.cells, rl.node, cl.node, di, f),
                None => Res::Empty,
            });
        }
    }
    show_as(p, comp, &row_lines, &col_lines, &mut values);
    Ok(Layout {
        labels: cells,
        rows,
        cols,
        first_data_row,
        data_col0,
        row_lines,
        col_lines,
        values,
    })
}

/// How a data field's reference ("show as" relative to a base field) is
/// resolved.
#[derive(Clone, Copy)]
enum Base {
    /// The base field is not a row or column field.
    Invalid,
    /// Running total.
    Running,
    /// A named item (its index; `None`: no such item).
    Named(Option<u32>),
    Previous,
    Next,
}

#[derive(Clone, Copy)]
struct RefInfo {
    rows_axis: bool,
    lvl: usize,
    base: Base,
}

fn ref_info(sa: &ShowAsData, comp: &Computed<'_>) -> RefInfo {
    let invalid = RefInfo {
        rows_axis: true,
        lvl: 0,
        base: Base::Invalid,
    };
    let Some(bc) = sa.base_field else {
        return invalid;
    };
    let found = comp
        .rows
        .fields
        .iter()
        .position(|f| f.source_col as u32 == bc)
        .map(|l| (true, l))
        .or_else(|| {
            comp.cols
                .fields
                .iter()
                .position(|f| f.source_col as u32 == bc)
                .map(|l| (false, l))
        });
    let Some((rows_axis, lvl)) = found else {
        return invalid;
    };
    let ax = if rows_axis { &comp.rows } else { &comp.cols };
    let base = match (sa.kind, &sa.base_item) {
        (ShowAsKindData::RunningTotal, _) => Base::Running,
        (_, Some(BaseItemData::Previous)) => Base::Previous,
        (_, Some(BaseItemData::Next)) => Base::Next,
        (_, Some(BaseItemData::Named(name))) => Base::Named(
            ax.fields[lvl]
                .items
                .iter()
                .position(|i| i.text == *name)
                .map(|i| i as u32),
        ),
        (_, None) => Base::Named(None),
    };
    RefInfo {
        rows_axis,
        lvl,
        base,
    }
}

/// Applies the "show as" modes of the data fields to every cell, totals
/// included (AOO `ScDPDataMember::UpdateRunningTotals`,
/// `dptabres.cxx:2250-2580`).
fn show_as(
    p: &PivotData,
    comp: &Computed<'_>,
    row_lines: &[Line],
    col_lines: &[Line],
    values: &mut [Res],
) {
    if p.data_fields.iter().all(|d| d.show_as.is_none()) {
        return;
    }
    let refs: Vec<Option<RefInfo>> = p
        .data_fields
        .iter()
        .map(|d| d.show_as.as_ref().map(|sa| ref_info(sa, comp)))
        .collect();
    // Position of every node among its parent's display children.
    let pos_of = |ax: &Axis<'_>| -> Vec<u32> {
        let mut pos = vec![0u32; ax.trie.len()];
        for n in 0..ax.trie.len() {
            for (i, k) in ax.disp.get(n as u32).iter().enumerate() {
                pos[*k as usize] = i as u32;
            }
        }
        pos
    };
    let rpos = pos_of(&comp.rows);
    let cpos = pos_of(&comp.cols);
    // Running totals so far: (node, other line, data field) → total.
    let mut running: FxMap<(u32, u32, u16), f64> = FxMap::default();
    let cells = &comp.cells;
    let ncl = col_lines.len();
    for (ri, rl) in row_lines.iter().enumerate() {
        for (ci, cl) in col_lines.iter().enumerate() {
            if rl.empty() || cl.empty() {
                continue;
            }
            let (di, fun) = cell_fn(p, comp, rl, cl);
            let (Some(sa), Some(info), Some(fun)) = (&p.data_fields[di].show_as, refs[di], fun)
            else {
                continue;
            };
            let idx = ri * ncl + ci;
            let v = values[idx];
            let own = p.data_fields[di].function;
            let out = match sa.kind {
                ShowAsKindData::PercentOfRow
                | ShowAsKindData::PercentOfColumn
                | ShowAsKindData::PercentOfTotal
                | ShowAsKindData::Index => {
                    let Res::Num(x) = v else { continue };
                    let row_total =
                        res_at(cells, rl.node, 0, di, forced(&comp.rows, rl).unwrap_or(own))
                            .num_or_zero();
                    let col_total =
                        res_at(cells, 0, cl.node, di, forced(&comp.cols, cl).unwrap_or(own))
                            .num_or_zero();
                    let grand = res_at(cells, 0, 0, di, own).num_or_zero();
                    let div = |t: f64| if t == 0.0 { Res::Err } else { Res::Num(x / t) };
                    match sa.kind {
                        ShowAsKindData::PercentOfRow => div(row_total),
                        ShowAsKindData::PercentOfColumn => div(col_total),
                        ShowAsKindData::PercentOfTotal => div(grand),
                        _ => {
                            if row_total == 0.0 || col_total == 0.0 {
                                Res::Err
                            } else {
                                Res::Num(x * grand / (row_total * col_total))
                            }
                        }
                    }
                }
                _ => {
                    let (ax, this, other, other_idx, pos) = if info.rows_axis {
                        (&comp.rows, rl, cl, ci, &rpos)
                    } else {
                        (&comp.cols, cl, rl, ri, &cpos)
                    };
                    reference(
                        sa.kind,
                        info,
                        ax,
                        this,
                        other,
                        other_idx as u32,
                        pos,
                        cells,
                        di,
                        fun,
                        v,
                        &mut running,
                    )
                }
            };
            values[idx] = out;
        }
    }
}

/// One cell of a relative reference mode (difference, percentage,
/// percentage difference from a base item, running total).
#[allow(clippy::too_many_arguments)]
fn reference(
    kind: ShowAsKindData,
    info: RefInfo,
    ax: &Axis<'_>,
    this: &Line,
    other: &Line,
    other_idx: u32,
    pos: &[u32],
    cells: &Cells,
    di: usize,
    fun: AggFnData,
    v: Res,
    running: &mut FxMap<(u32, u32, u16), f64>,
) -> Res {
    if matches!(info.base, Base::Invalid) {
        // A base field outside the layout is an error (`:2409-2410`).
        return Res::Err;
    }
    let lvl = info.lvl;
    let node = this.node;
    // A total in the reference dimension (no item at its level) is empty
    // (`:2343-2363,2522-2525`).
    if ax.trie.level(node) <= lvl {
        return Res::Empty;
    }
    let path = ax.trie.path(node);
    let anc = ax.trie.ancestor(node, lvl);
    let me = ax.trie.ancestor(node, lvl + 1);
    let suffix = &path[lvl + 1..];
    let descend = |sib: u32| -> Option<u32> {
        let mut n = sib;
        for s in suffix {
            n = ax.trie.child(n, *s)?;
        }
        Some(n)
    };
    let value_of = |n: u32| -> f64 {
        if info.rows_axis {
            res_at(cells, n, other.node, di, fun).num_or_zero()
        } else {
            res_at(cells, other.node, n, di, fun).num_or_zero()
        }
    };
    let this_v = v.num_or_zero();
    let sibs = ax.disp.get(anc);
    let my_pos = pos[me as usize] as usize;
    let key = |n: u32| (n, other_idx, di as u16);
    match info.base {
        Base::Running => {
            // The total of the previous existing member plus this value.
            let mut total = this_v;
            for k in (0..my_pos).rev() {
                if let Some(c) = descend(sibs[k]) {
                    total += running.get(&key(c)).copied().unwrap_or_else(|| value_of(c));
                    break;
                }
            }
            running.insert(key(node), total);
            Res::Num(total)
        }
        Base::Previous | Base::Next => {
            let found = if matches!(info.base, Base::Previous) {
                (0..my_pos).rev().find_map(|k| descend(sibs[k]))
            } else {
                (my_pos + 1..sibs.len()).find_map(|k| descend(sibs[k]))
            };
            match found {
                // Previous/next not found: empty (`:2540-2542`).
                None => Res::Empty,
                Some(b) => relative(kind, this_v, value_of(b), false),
            }
        }
        // A named item that does not exist: error (`:2546-2547`).
        Base::Named(None) => Res::Err,
        Base::Named(Some(item)) => {
            let is_self = ax.trie.item[me as usize] == item;
            let b = ax
                .trie
                .child(anc, item)
                .and_then(descend)
                .map_or(0.0, value_of);
            relative(kind, this_v, b, is_self)
        }
        Base::Invalid => Res::Err,
    }
}

fn relative(kind: ShowAsKindData, v: f64, b: f64, is_self: bool) -> Res {
    // The reference item itself is empty for difference and percentage
    // difference (`:2491-2495`).
    if is_self && kind != ShowAsKindData::PercentOf {
        return Res::Empty;
    }
    match kind {
        ShowAsKindData::Difference => Res::Num(v - b),
        ShowAsKindData::PercentOf => {
            if b == 0.0 {
                Res::Err
            } else {
                Res::Num(v / b)
            }
        }
        _ => {
            if b == 0.0 {
                Res::Err
            } else {
                Res::Num((v - b) / b)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Lookup (GETPIVOTDATA, drill-down)
// ---------------------------------------------------------------------------

fn lower(s: &str) -> String {
    s.trim().to_lowercase()
}

/// A stable 64-bit hash (the lookup indexes item names and identities by
/// hash, so a large pivot keeps no second copy of its item texts).
fn hash_of(v: &impl std::hash::Hash) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    v.hash(&mut h);
    h.finish()
}

fn lookup_field(f: &AxisField<'_>, source: &str) -> FieldLookup {
    let mut by_text: FxMap<u64, u32> = FxMap::default();
    by_text.reserve(f.items.len());
    let mut dups: FxMap<u64, Vec<u32>> = FxMap::default();
    for (i, it) in f.items.iter().enumerate() {
        let t = hash_of(&lower(&it.text));
        match by_text.get(&t) {
            Some(first) => dups.entry(t).or_insert_with(|| vec![*first]).push(i as u32),
            None => {
                by_text.insert(t, i as u32);
            }
        }
    }
    FieldLookup {
        names: (lower(&f.name), lower(source)),
        by_text,
        dups,
        keys: f.items.iter().map(|i| hash_of(&i.key)).collect(),
    }
}

fn lookup_axis(ax: &Axis<'_>, lines: &[Line], names: &[String]) -> AxisLookup {
    let mut line_of: FxMap<(u32, u16), u32> = FxMap::default();
    for (i, l) in lines.iter().enumerate() {
        // An outline title row has no values; a subtotal shown in the
        // title row is the item's subtotal (AOO 4.1 answers #REF! there,
        // a deliberate deviation, parity-deviations T5).
        if matches!(l.kind, Kind::Blank | Kind::Title | Kind::Sub(1..)) {
            continue;
        }
        line_of
            .entry((l.node, l.df.unwrap_or(u16::MAX)))
            .or_insert(i as u32);
    }
    AxisLookup {
        fields: ax
            .fields
            .iter()
            .map(|f| lookup_field(f, names.get(f.source_col).map_or("", String::as_str)))
            .collect(),
        parent: ax.trie.parent.clone(),
        item: ax.trie.item.clone(),
        edges: ax.trie.edges.clone(),
        line_of,
    }
}

fn build_lookup(p: &PivotData, cache: &Cache, comp: &Computed<'_>, layout: &mut Layout) -> Lookup {
    let rows = lookup_axis(&comp.rows, &layout.row_lines, &cache.names);
    let cols = lookup_axis(&comp.cols, &layout.col_lines, &cache.names);
    let pages: Vec<PageLookup> = comp
        .pages
        .iter()
        .map(|f| PageLookup {
            names: (
                lower(&f.name),
                lower(cache.names.get(f.source_col).map_or("", String::as_str)),
            ),
            selected: f.def.page_item.as_ref().map(|s| lower(s)),
        })
        .collect();
    Lookup {
        df_names: p
            .data_fields
            .iter()
            .map(|d| {
                // The caption, the source name and AOO's given name
                // (`lcl_GetDataFieldName`, `dpoutput.cxx:1668-1700`: the
                // short function name, ignoring a custom caption), plus
                // the long function name captions used before 2026-11-13
                // so formulas written then keep resolving.
                let mut v = vec![
                    lower(&caption(d, &cache.names)),
                    lower(
                        cache
                            .names
                            .get(d.source_col as usize)
                            .map_or("", String::as_str),
                    ),
                    lower(&fn_field(measure_fn_name(d.function), d, &cache.names)),
                    lower(&fn_field(agg_caption(d.function), d, &cache.names)),
                ];
                v.dedup();
                v
            })
            .collect(),
        rows,
        cols,
        pages,
        values: std::mem::take(&mut layout.values),
        ncols: layout.col_lines.len(),
        data_rows: layout.row_lines.iter().any(|l| l.df.is_some()),
        data_cols: layout.col_lines.iter().any(|l| l.df.is_some()),
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
/// the label roles, the data grid, the lookup table and the inverse
/// changes.
type Written = (
    PivotData,
    Arc<Vec<OutCell>>,
    Arc<Grid>,
    Arc<Lookup>,
    Vec<Change>,
);

/// Computes `data` and writes its output at the target; returns the
/// written table and the changes (the caller records the meta).
fn write_output(
    wb: &mut Workbook,
    sheet: SheetId,
    data: &PivotData,
    old_output: Option<CellRangeLocal>,
    overwrite: bool,
) -> Result<Written, ApiError> {
    let (src, cache) = read_for(wb, data)?;
    let comp = compute(wb, data, &cache)?;
    let mut layout = lay_out(data, &cache, &comp)?;
    let lookup = build_lookup(data, &cache, &comp, &mut layout);
    drop(comp);
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
    // AOO `lcl_EmptyExcept` / STR_PIVOT_NOTEMPTY (`dbdocfun.cxx:1349-1371`).
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
    let mut roles: Vec<OutCell> = Vec::with_capacity(layout.labels.len());
    let ctx = RecognizeContext::for_settings(&wb.settings);
    for l in layout.labels {
        let a = CellAddr::new(sheet, tr + l.r, tc + l.c);
        // Numeric captions are entered as typed (AOO `ScDPOutput::HeaderCell`
        // writes them without the text apostrophe).
        let (value, format) = if l.numeric {
            let rec = recognize_input_with(&l.text, wb.locale, &ctx);
            (rec.value, rec.implied_format)
        } else {
            (Value::text(l.text.as_str()), None)
        };
        if !value.is_empty() {
            wb.set_value(a, value);
        }
        let cell = CellRange::new(sheet, a.row, a.col, a.row, a.col);
        let total = l.role == PivotCellRole::Total;
        map_styles(wb, cell, |t, old| {
            let mut d = t.get(old).data.clone();
            d.font.bold = true;
            if total {
                d.fill = Some(0xeeee_eeff);
            }
            let id = t.intern(d);
            match &format {
                Some(code) => t.with_format(id, code),
                None => id,
            }
        });
        roles.push(OutCell {
            r: a.row,
            c: a.col,
            role: l.role,
            field: l.field,
            item: if l.item_is_text { Some(l.text) } else { l.item },
        });
    }
    roles.sort_by_key(|o| (o.r, o.c));
    roles.dedup_by_key(|o| (o.r, o.c));
    // Data cells.
    let ncl = layout.col_lines.len();
    let r0 = tr + layout.first_data_row;
    let c0 = tc + layout.data_col0;
    for (ri, rl) in layout.row_lines.iter().enumerate() {
        if rl.empty() {
            continue;
        }
        for (ci, cl) in layout.col_lines.iter().enumerate() {
            if cl.empty() {
                continue;
            }
            let v = lookup.values[ri * ncl + ci].value();
            if !v.is_empty() {
                wb.set_value(CellAddr::new(sheet, r0 + ri as u32, c0 + ci as u32), v);
            }
        }
    }
    // Total styles, one strip per total line.
    let total_style = |t: &mut crate::styles::StyleTable, old| {
        let mut d = t.get(old).data.clone();
        d.font.bold = true;
        d.fill = Some(0xeeee_eeff);
        t.intern(d)
    };
    if ncl > 0 {
        for (ri, rl) in layout.row_lines.iter().enumerate() {
            if rl.total() {
                let r = r0 + ri as u32;
                map_styles(
                    wb,
                    CellRange::new(sheet, r, c0, r, c0 + ncl as u32 - 1),
                    total_style,
                );
            }
        }
    }
    if !layout.row_lines.is_empty() {
        let r1 = r0 + layout.row_lines.len() as u32 - 1;
        for (ci, cl) in layout.col_lines.iter().enumerate() {
            if cl.total() {
                let c = c0 + ci as u32;
                map_styles(wb, CellRange::new(sheet, r0, c, r1, c), total_style);
            }
        }
    }
    let grid_line = |i: usize, l: &Line| GridLine {
        node: l.node,
        total: l.total(),
        blank: l.empty(),
        line: i as u32,
    };
    let grid = Grid {
        rows: layout
            .row_lines
            .iter()
            .enumerate()
            .map(|(i, l)| (r0 + i as u32, grid_line(i, l)))
            .collect(),
        cols: layout
            .col_lines
            .iter()
            .enumerate()
            .map(|(i, l)| (c0 + i as u32, grid_line(i, l)))
            .collect(),
    };
    wb.pending.ranges.push(out);
    let mut written = data.clone();
    written.output = Some(CellRangeLocal {
        r0: out.r0,
        c0: out.c0,
        r1: out.r1,
        c1: out.c1,
    });
    Ok((
        written,
        Arc::new(roles),
        Arc::new(grid),
        Arc::new(lookup),
        changes,
    ))
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

fn table(id: u32, w: &Written) -> Arc<PivotTable> {
    Arc::new(PivotTable {
        id,
        data: w.0.clone(),
        cells: w.1.clone(),
        grid: w.2.clone(),
        lookup: w.3.clone(),
    })
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
    let mut w = write_output(wb, sheet, &data, None, overwrite)?;
    let mut changes = std::mem::take(&mut w.4);
    let mut created = 0;
    changes.extend(with_meta(wb, sheet, fx, |_, meta| {
        let id = meta.alloc_id();
        created = id;
        Arc::make_mut(&mut meta.pivots).push(table(id, &w));
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
    let mut w = write_output(wb, target_sheet, &data, old_output, overwrite)?;
    changes.append(&mut w.4);
    changes.extend(with_meta(wb, target_sheet, fx, |_, meta| {
        let list = Arc::make_mut(&mut meta.pivots);
        let t = table(id, &w);
        match list.iter().position(|p| p.id == id) {
            Some(i) => list[i] = t,
            None => list.push(t),
        }
        Ok(())
    })?);
    fx.pivots_changed = Some(true);
    Ok(changes)
}

/// `Op::RefreshPivot`. When the new output grows over non-empty cells
/// outside the previous output, the op fails with `target_not_empty`
/// unless `overwrite` (AOO asks STR_PIVOT_NOTEMPTY, `dbdocfun.cxx:1349-1371`).
pub fn refresh(
    wb: &mut Workbook,
    id: u32,
    overwrite: bool,
    fx: &mut Effects,
) -> Result<Vec<Change>, ApiError> {
    let holder = sheet_of_pivot(wb, id)
        .ok_or_else(|| ApiError::not_found(format!("no pivot table {id}")))?;
    let old = pivot_of(wb, holder, id)?;
    let mut w = write_output(wb, holder, &old.data, old.data.output, overwrite)?;
    let mut changes = std::mem::take(&mut w.4);
    changes.extend(with_meta(wb, holder, fx, |_, meta| {
        let list = Arc::make_mut(&mut meta.pivots);
        if let Some(i) = list.iter().position(|p| p.id == id) {
            list[i] = table(id, &w);
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

/// The data-grid lines of an output cell.
fn grid_at(g: &Grid, r: u32, c: u32) -> Option<(GridLine, GridLine)> {
    let ri = g.rows.binary_search_by_key(&r, |x| x.0).ok()?;
    let ci = g.cols.binary_search_by_key(&c, |x| x.0).ok()?;
    Some((g.rows[ri].1, g.cols[ci].1))
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
        if let Some(cell) = hit {
            return Some(PivotHit {
                pivot: p.id,
                role: cell.role,
                field: cell.field,
                item: cell.item.clone(),
            });
        }
        let role = match grid_at(&p.grid, r, c) {
            Some((a, b)) if a.total || b.total => PivotCellRole::Total,
            _ => PivotCellRole::Data,
        };
        return Some(PivotHit {
            pivot: p.id,
            role,
            field: None,
            item: None,
        });
    }
    None
}

/// The item keys of a node's path.
fn node_keys(ax: &AxisLookup, mut n: u32) -> Option<Vec<u64>> {
    let mut items = Vec::new();
    while n != 0 {
        items.push(*ax.item.get(n as usize)?);
        n = *ax.parent.get(n as usize)?;
    }
    items.reverse();
    items
        .iter()
        .enumerate()
        .map(|(l, it)| ax.fields.get(l)?.keys.get(*it as usize).copied())
        .collect()
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
    let not_data = || ApiError::invalid_argument("not a data cell of the pivot");
    if p.cells
        .binary_search_by_key(&(r, c), |x| (x.r, x.c))
        .is_ok()
    {
        return Err(not_data());
    }
    let (rl, cl) = grid_at(&p.grid, r, c).ok_or_else(not_data)?;
    if rl.blank || cl.blank {
        return Err(not_data());
    }
    let rk = node_keys(&p.lookup.rows, rl.node).ok_or_else(not_data)?;
    let ck = node_keys(&p.lookup.cols, cl.node).ok_or_else(not_data)?;
    crate::protection::check_structure(wb)?;
    let (src, cache) = read_for(wb, &p.data)?;
    let comp = compute(wb, &p.data, &cache)?;
    let want = |fields: &[AxisField<'_>], keys: &[u64]| -> Option<Vec<u32>> {
        keys.iter()
            .enumerate()
            .map(|(l, k)| {
                fields
                    .get(l)?
                    .items
                    .iter()
                    .position(|i| hash_of(&i.key) == *k)
                    .map(|i| i as u32)
            })
            .collect()
    };
    let matches: Vec<usize> = match (want(&comp.rows.fields, &rk), want(&comp.cols.fields, &ck)) {
        (Some(ri), Some(ci)) => comp
            .kept
            .iter()
            .map(|r| *r as usize)
            .filter(|&row| {
                ri.iter()
                    .enumerate()
                    .all(|(l, it)| comp.rows.fields[l].item_of[row] == *it)
                    && ci
                        .iter()
                        .enumerate()
                        .all(|(l, it)| comp.cols.fields[l].item_of[row] == *it)
            })
            .collect(),
        _ => Vec::new(),
    };
    drop(comp);
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

/// Resolves the data field text of the Excel form (AOO
/// `ScDPObject::ParseFilters`, `dpobject.cxx:1320-1530`): from left to
/// right, the longest word run naming a data field, else the longest
/// naming an item of exactly one field (which becomes a filter); a
/// single data field may be omitted.
fn parse_excel_words(
    lk: &Lookup,
    text: &str,
    filters: &mut Vec<(String, String)>,
) -> Option<usize> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut df: Option<usize> = None;
    let mut i = 0;
    'outer: while i < words.len() {
        for j in (i + 1..=words.len()).rev() {
            let cand = lower(&words[i..j].join(" "));
            if let Some(d) = lk.df_names.iter().position(|n| n.contains(&cand)) {
                df = Some(d);
                i = j;
                continue 'outer;
            }
            // An item of exactly one field (the same name in several
            // fields is an error).
            let mut owners = lk
                .rows
                .fields
                .iter()
                .chain(&lk.cols.fields)
                .filter(|f| f.by_text.contains_key(&hash_of(&cand)))
                .map(|f| &f.names.0)
                .chain(
                    lk.pages
                        .iter()
                        .filter(|pf| pf.selected.as_deref() == Some(cand.as_str()))
                        .map(|pf| &pf.names.0),
                );
            if let Some(owner) = owners.next() {
                if owners.next().is_some() {
                    return None;
                }
                filters.push((owner.clone(), cand));
                i = j;
                continue 'outer;
            }
        }
        return None;
    }
    match df {
        Some(d) => Some(d),
        None if lk.df_names.len() == 1 => Some(0),
        None => None,
    }
}

/// The node of an axis selected by per-level item names (a prefix; an
/// unspecified outer field with a specified inner one has no cell). An
/// item name shared by several items that are all shown is ambiguous.
fn select_node(ax: &AxisLookup, sel: &[Option<String>]) -> Option<u32> {
    let mut n = 0u32;
    let mut ended = false;
    for (lvl, s) in sel.iter().enumerate() {
        match s {
            None => ended = true,
            Some(_) if ended => return None,
            Some(t) => {
                let f = ax.fields.get(lvl)?;
                let h = hash_of(t);
                let one = [*f.by_text.get(&h)?];
                let cands: &[u32] = f.dups.get(&h).map_or(&one, Vec::as_slice);
                let mut found = cands
                    .iter()
                    .filter_map(|it| ax.edges.get(&(n, *it)).copied());
                let first = found.next()?;
                if found.next().is_some() {
                    return None;
                }
                n = first;
            }
        }
    }
    Some(n)
}

/// `EvalContext::pivot_data`: the displayed value of the pivot whose
/// output contains `at`, for data field `field` restricted by `filters`
/// (AOO `ScDPOutput::GetPivotData`, `dpoutput.cxx:1728-1828`). `excel`:
/// the Excel 2000 form, whose field text is parsed into a data field and
/// item names; in the AOO form `field` must name a data field. `None`
/// (`#REF!`) for an unknown field or item, a page field not set to the
/// filtered item, an error result or no such cell.
pub fn pivot_data(
    wb: &Workbook,
    at: CellAddr,
    field: &str,
    filters: &[(String, String)],
    excel: bool,
) -> Option<Value> {
    let s = wb.sheet(at.sheet)?;
    let p = s.meta.pivots.iter().find(|p| {
        p.data
            .output
            .is_some_and(|o| at.row >= o.r0 && at.row <= o.r1 && at.col >= o.c0 && at.col <= o.c1)
    })?;
    let lk = &p.lookup;
    if lk.df_names.is_empty() {
        return None;
    }
    let mut filters: Vec<(String, String)> = filters.to_vec();
    let df = if excel {
        parse_excel_words(lk, field, &mut filters)?
    } else {
        let t = lower(field);
        lk.df_names.iter().position(|n| n.contains(&t))?
    };
    let mut rsel: Vec<Option<String>> = vec![None; lk.rows.fields.len()];
    let mut csel: Vec<Option<String>> = vec![None; lk.cols.fields.len()];
    let named = |names: &(String, String), f: &str| names.0 == f || names.1 == f;
    for (fname, item) in &filters {
        let fl = lower(fname);
        let il = lower(item);
        if let Some(l) = lk.rows.fields.iter().position(|f| named(&f.names, &fl)) {
            rsel[l] = Some(il);
        } else if let Some(l) = lk.cols.fields.iter().position(|f| named(&f.names, &fl)) {
            csel[l] = Some(il);
        } else if let Some(pf) = lk.pages.iter().find(|f| named(&f.names, &fl)) {
            // Only the selected page item is answerable (`lcl_CheckPageField`).
            if pf.selected.as_deref() != Some(il.as_str()) {
                return None;
            }
        } else {
            return None;
        }
    }
    let rn = select_node(&lk.rows, &rsel)?;
    let cn = select_node(&lk.cols, &csel)?;
    let dkey = |on_axis: bool| if on_axis { df as u16 } else { u16::MAX };
    let ri = *lk.rows.line_of.get(&(rn, dkey(lk.data_rows)))? as usize;
    let ci = *lk.cols.line_of.get(&(cn, dkey(lk.data_cols)))? as usize;
    match lk.values.get(ri * lk.ncols + ci)? {
        Res::Num(n) => Some(Value::Number(*n)),
        Res::Empty => Some(Value::Number(0.0)),
        Res::Err => None,
    }
}

/// Moves pivot sources, targets and outputs for a row/column change.
pub fn adjust_structure(wb: &mut Workbook, sh: &Shape) {
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
                if *sheet == sh.sheet {
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
                let (cells, grid): (Vec<OutCell>, Grid) = if on_sheet {
                    let cells = p
                        .cells
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
                        .collect();
                    let moved = |v: &[(u32, GridLine)], axis_rows: bool| -> Vec<(u32, GridLine)> {
                        v.iter()
                            .filter_map(|(x, l)| {
                                let x = if sh.rows == axis_rows {
                                    point_after(sh, *x)?
                                } else {
                                    *x
                                };
                                Some((x, *l))
                            })
                            .collect()
                    };
                    (
                        cells,
                        Grid {
                            rows: moved(&p.grid.rows, true),
                            cols: moved(&p.grid.cols, false),
                        },
                    )
                } else {
                    ((*p.cells).clone(), (*p.grid).clone())
                };
                *p = Arc::new(PivotTable {
                    id: p.id,
                    data: d,
                    cells: Arc::new(cells),
                    grid: Arc::new(grid),
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

    fn gx() -> Gx {
        Gx {
            epoch: DateEpoch::D1899,
            locale: &tally_numfmt::EN_US,
        }
    }

    fn num_item(n: f64) -> Item {
        Item {
            text: fmt_num(n),
            num: Some(n),
            empty: false,
            numeric: true,
            key: ItemKey::Num(round15(n).to_bits()),
        }
    }

    fn groups(g: &PivotGroupData, vals: &[f64]) -> Vec<String> {
        let src: Vec<Item> = vals.iter().map(|v| num_item(*v)).collect();
        let (items, map) = group_items(g, &src, gx());
        map.iter()
            .map(|i| items[*i as usize].text.clone())
            .collect()
    }

    #[test]
    fn numeric_groups_follow_dpgroup() {
        // Integers, explicit start: "10-19".
        let g = PivotGroupData::Numeric {
            start: Some(0.0),
            end: None,
            step: 10.0,
        };
        assert_eq!(groups(&g, &[17.0, 3.0]), ["10-19", "0-9"]);
        // The end value joins the last group, shown including the end.
        let g = PivotGroupData::Numeric {
            start: Some(0.0),
            end: Some(20.0),
            step: 10.0,
        };
        assert_eq!(
            groups(&g, &[20.0, 12.0, 30.0, -4.0]),
            ["10-20", "10-20", ">20", "<0"]
        );
        // The last group is clamped to the end.
        let g = PivotGroupData::Numeric {
            start: Some(0.0),
            end: Some(25.0),
            step: 10.0,
        };
        assert_eq!(groups(&g, &[22.0]), ["20-25"]);
        // Automatic start/end from the values; non-integers show lo-hi.
        let g = PivotGroupData::Numeric {
            start: None,
            end: None,
            step: 5.0,
        };
        assert_eq!(groups(&g, &[-4.0, 2.5, 30.0]), ["-4-1", "1-6", "26-31"]);
    }

    #[test]
    fn date_groups_follow_dpgroup() {
        let s =
            |y, m, d| tally_numfmt::date::serial_from_ymd(DateEpoch::D1899, y, m, d).unwrap_or(0.0);
        let d = PivotGroupData::Date {
            start: None,
            end: None,
            parts: vec![DatePartData::Months, DatePartData::Years],
        };
        assert_eq!(groups(&d, &[s(2024, 3, 15)]), ["2024 Mar"]);
        let d = PivotGroupData::Date {
            start: None,
            end: None,
            parts: vec![DatePartData::Days(1)],
        };
        assert_eq!(groups(&d, &[s(2024, 1, 5)]), ["05-Jan"]);
        // Explicit limits.
        let d = PivotGroupData::Date {
            start: Some(s(2024, 2, 1)),
            end: Some(s(2024, 6, 30)),
            parts: vec![DatePartData::Months],
        };
        assert_eq!(
            groups(&d, &[s(2024, 1, 20), s(2024, 4, 11), s(2024, 12, 31)]),
            ["<02/01/2024", "Apr", ">06/30/2024"]
        );
        // Days with a step: numeric date groups from the first date.
        let d = PivotGroupData::Date {
            start: None,
            end: None,
            parts: vec![DatePartData::Days(7)],
        };
        assert_eq!(
            groups(&d, &[s(2024, 1, 5), s(2024, 1, 12), s(2024, 1, 20)]),
            [
                "01/05/2024 - 01/11/2024",
                "01/12/2024 - 01/18/2024",
                "01/19/2024 - 01/25/2024"
            ]
        );
    }

    #[test]
    fn aggregation_follows_scdpaggdata() {
        let mut a = Acc::default();
        a.push(Cv::Str);
        a.push(Cv::Str);
        assert_eq!(a.result(AggFnData::Sum), Res::Empty);
        assert_eq!(a.result(AggFnData::Min), Res::Empty);
        assert_eq!(a.result(AggFnData::Count), Res::Num(2.0));
        let mut e = Acc::default();
        e.push(Cv::Err);
        e.push(Cv::Num(3.0));
        assert_eq!(e.result(AggFnData::Sum), Res::Err);
        assert_eq!(e.result(AggFnData::Count), Res::Num(2.0));
        assert_eq!(e.result(AggFnData::CountNums), Res::Err);
        let mut one = Acc::default();
        one.push(Cv::Num(5.0));
        assert_eq!(one.result(AggFnData::StDev), Res::Err);
        assert_eq!(one.result(AggFnData::StDevP), Res::Num(0.0));
    }
}
