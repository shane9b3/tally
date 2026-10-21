//! Scenarios (wave 5, W5-2): a base sheet keeps named sets of values for
//! its scenario ranges; showing one writes its values into the sheet
//! (with copy-back of the current values into the scenario shown before,
//! when that one allows it).
//!
//! The model is `SheetMeta::scenarios` (`tally_model::ScenarioData`,
//! which also carries the cells as `CellData` with OpenFormula texts).
//! ODS stores each scenario as a hidden sheet after the base sheet
//! (`table:scenario`); XLSX as `scenarios` on the base sheet. The IO
//! crates convert. Undo: the `Scenarios` meta part plus a snapshot of the
//! scenario ranges.
//!
//! Styles are not copied with a scenario (`copy_styles` is kept for the
//! file but has no effect); `prevent_changes` is a file flag too.

use std::sync::Arc;

use tally_api::{ApiError, RangeDto, ScenarioDto};
use tally_formula::Dialect;
use tally_model::{CellContentData, CellData, CellRangeLocal, ScenarioData};
use tally_types::{CellAddr, CellRange, SheetId, Value};

use crate::ops::{check_sheet, to_range};
use crate::ops_v03::with_meta;
use crate::protection::check_edit;
use crate::storage::{CellContent, ColumnStore};
use crate::structure::{span_after, Shape};
use crate::undo::{capture_range, clear_range_content, Change, Effects};
use crate::workbook::Workbook;

/// A scenario with its sheet-local id.
#[derive(Clone, Debug, PartialEq)]
pub struct Scenario {
    /// Id from `SheetMeta::alloc_id`.
    pub id: u32,
    /// The scenario.
    pub data: ScenarioData,
}

/// Most scenarios per sheet.
pub const MAX_SCENARIOS: usize = 1_000;

/// Most cells one scenario may hold.
pub const MAX_SCENARIO_CELLS: u64 = 1_000_000;

/// Loads a sheet's scenarios from the interchange model.
pub fn from_data(wb: &mut Workbook, sheet: SheetId, scenarios: &[ScenarioData]) {
    let Some(s) = wb.sheet_mut(sheet) else {
        return;
    };
    for sc in scenarios.iter().take(MAX_SCENARIOS) {
        let id = s.meta.alloc_id();
        let mut data = sc.clone();
        data.ranges.retain(|r| r.r0 <= r.r1 && r.c0 <= r.c1);
        std::sync::Arc::make_mut(&mut s.meta.scenarios)
            .push(std::sync::Arc::new(Scenario { id, data }));
    }
}

/// The interchange form of a sheet's scenarios.
pub fn to_data(wb: &Workbook, sheet: SheetId) -> Vec<ScenarioData> {
    wb.sheet(sheet)
        .map(|s| s.meta.scenarios.iter().map(|sc| sc.data.clone()).collect())
        .unwrap_or_default()
}

fn range_dto(r: &CellRangeLocal) -> RangeDto {
    RangeDto {
        r0: r.r0,
        c0: r.c0,
        r1: r.r1,
        c1: r.c1,
    }
}

/// The API form of a scenario.
pub fn to_api(sc: &Scenario) -> ScenarioDto {
    ScenarioDto {
        id: sc.id,
        name: sc.data.name.clone(),
        comment: sc.data.comment.clone(),
        ranges: sc.data.ranges.iter().map(range_dto).collect(),
        border_color: sc.data.border_color,
        display_border: sc.data.display_border,
        copy_back: sc.data.copy_back,
        copy_styles: sc.data.copy_styles,
        prevent_changes: sc.data.prevent_changes,
        active: sc.data.active,
    }
}

/// The scenarios of a sheet.
pub fn list(wb: &Workbook, sheet: SheetId) -> Result<Vec<ScenarioDto>, ApiError> {
    let s = wb
        .sheet(sheet)
        .ok_or_else(|| ApiError::not_found(format!("no sheet {sheet}")))?;
    Ok(s.meta.scenarios.iter().map(|sc| to_api(sc)).collect())
}

fn ranges_of(sheet: SheetId, dto: &ScenarioDto) -> Result<Vec<CellRange>, ApiError> {
    if dto.ranges.is_empty() {
        return Err(ApiError::invalid_argument("a scenario needs a range"));
    }
    dto.ranges.iter().map(|r| to_range(sheet, r)).collect()
}

fn local(r: &CellRange) -> CellRangeLocal {
    CellRangeLocal {
        r0: r.r0,
        c0: r.c0,
        r1: r.r1,
        c1: r.c1,
    }
}

/// Snapshots the cells of `ranges` as model cells (values, formulas as
/// OpenFormula text).
pub fn snapshot_cells(wb: &Workbook, sheet: SheetId, ranges: &[CellRange]) -> Vec<CellData> {
    let mut out = Vec::new();
    let Some(s) = wb.sheet(sheet) else {
        return out;
    };
    for r in ranges {
        for c in r.c0..=r.c1 {
            let Some(col) = s.column(c) else {
                continue;
            };
            for (row, content) in col.cells.iter_range(r.r0, r.r1) {
                let addr = CellAddr::new(sheet, row, c);
                let content = match content {
                    CellContent::Value(v) => CellContentData::Value(v.clone()),
                    CellContent::Formula(id) => match wb.formulas.get(*id) {
                        Some(f) if f.array.is_none() => {
                            let text = match &f.invalid_text {
                                Some(t) => format!("of:={t}"),
                                None => tally_formula::print(
                                    &f.formula,
                                    addr,
                                    Dialect::open_formula(),
                                    wb,
                                ),
                            };
                            CellContentData::Formula {
                                text: Arc::from(text.as_str()),
                                cached: Some(wb.value(addr)),
                            }
                        }
                        _ => CellContentData::Value(wb.value(addr)),
                    },
                    CellContent::ArrayPart(_) => CellContentData::Value(wb.value(addr)),
                };
                out.push(CellData {
                    row,
                    col: c,
                    content,
                    style: None,
                });
            }
        }
    }
    out
}

/// Writes scenario cells into `ranges` (the ranges are cleared first).
fn write_cells(wb: &mut Workbook, sheet: SheetId, ranges: &[CellRange], cells: &[CellData]) {
    for r in ranges {
        clear_range_content(wb, *r);
    }
    for c in cells {
        let addr = CellAddr::new(sheet, c.row, c.col);
        if !ranges.iter().any(|r| r.contains(addr)) {
            continue;
        }
        match &c.content {
            CellContentData::Value(Value::Empty) => {}
            CellContentData::Value(v) => wb.set_value(addr, v.clone()),
            CellContentData::Formula { text, .. } | CellContentData::SharedFormula { text, .. } => {
                let t: &str = text;
                match tally_formula::parse(
                    t,
                    addr,
                    Dialect::open_formula(),
                    &crate::workbook::OdfResolver(wb),
                ) {
                    Ok(f) => wb.set_formula(addr, Arc::new(f), None),
                    Err(_) => wb.set_value(addr, Value::text(t.trim_start_matches("of:"))),
                }
            }
        }
    }
    for r in ranges {
        wb.pending.ranges.push(*r);
    }
}

fn union_snapshot(wb: &Workbook, ranges: &[CellRange]) -> Vec<Change> {
    ranges
        .iter()
        .map(|r| Change::Range(capture_range(wb, *r, true, false)))
        .collect()
}

fn settings_from(dto: &ScenarioDto, ranges: &[CellRange], base: &ScenarioData) -> ScenarioData {
    ScenarioData {
        name: dto.name.trim().to_string(),
        comment: dto.comment.clone(),
        ranges: ranges.iter().map(local).collect(),
        cells: base.cells.clone(),
        border_color: dto.border_color,
        display_border: dto.display_border,
        copy_back: dto.copy_back,
        copy_styles: dto.copy_styles,
        prevent_changes: dto.prevent_changes,
        active: base.active,
    }
}

fn check_dto(dto: &ScenarioDto) -> Result<(), ApiError> {
    if dto.name.trim().is_empty() || dto.name.chars().count() > 255 {
        return Err(ApiError::invalid_argument("bad scenario name"));
    }
    if dto.comment.len() > 64 * 1024 {
        return Err(ApiError::invalid_argument("the comment is too long"));
    }
    Ok(())
}

/// `Op::CreateScenario`: the current values of the ranges become the
/// scenario, which is then the one shown.
pub fn create(
    wb: &mut Workbook,
    sheet: SheetId,
    dto: &ScenarioDto,
    fx: &mut Effects,
) -> Result<Vec<Change>, ApiError> {
    check_sheet(wb, sheet)?;
    check_dto(dto)?;
    let ranges = ranges_of(sheet, dto)?;
    check_edit(wb, &ranges)?;
    let cells: u64 = ranges.iter().map(|r| r.cell_count()).sum();
    if cells > MAX_SCENARIO_CELLS {
        return Err(ApiError::invalid_argument(
            "the scenario ranges are too large",
        ));
    }
    let count = wb.sheet(sheet).map_or(0, |s| s.meta.scenarios.len());
    if count >= MAX_SCENARIOS {
        return Err(ApiError::invalid_argument(format!(
            "at most {MAX_SCENARIOS} scenarios per sheet"
        )));
    }
    if wb.sheet(sheet).is_some_and(|s| {
        s.meta
            .scenarios
            .iter()
            .any(|x| x.data.name.eq_ignore_ascii_case(dto.name.trim()))
    }) {
        return Err(ApiError::invalid_argument(format!(
            "a scenario named {:?} already exists",
            dto.name.trim()
        )));
    }
    let snap = snapshot_cells(wb, sheet, &ranges);
    let mut created = 0;
    let ch = with_meta(wb, sheet, fx, |_, meta| {
        let id = meta.alloc_id();
        created = id;
        let list = Arc::make_mut(&mut meta.scenarios);
        for sc in list.iter_mut() {
            if sc.data.active {
                Arc::make_mut(sc).data.active = false;
            }
        }
        let mut data = settings_from(dto, &ranges, &ScenarioData::default());
        data.cells = snap;
        data.active = true;
        list.push(Arc::new(Scenario { id, data }));
        Ok(())
    })?;
    fx.created(u32::MAX, created);
    Ok(ch)
}

/// `Op::UpdateScenario`: new settings, the values are kept (cells outside
/// new ranges are dropped).
pub fn update(
    wb: &mut Workbook,
    sheet: SheetId,
    id: u32,
    dto: &ScenarioDto,
    fx: &mut Effects,
) -> Result<Vec<Change>, ApiError> {
    check_sheet(wb, sheet)?;
    check_dto(dto)?;
    let ranges = ranges_of(sheet, dto)?;
    with_meta(wb, sheet, fx, |_, meta| {
        let list = Arc::make_mut(&mut meta.scenarios);
        let pos = list
            .iter()
            .position(|s| s.id == id)
            .ok_or_else(|| ApiError::not_found(format!("no scenario {id}")))?;
        if list
            .iter()
            .any(|x| x.id != id && x.data.name.eq_ignore_ascii_case(dto.name.trim()))
        {
            return Err(ApiError::invalid_argument(format!(
                "a scenario named {:?} already exists",
                dto.name.trim()
            )));
        }
        let old = &list[pos].data;
        let mut data = settings_from(dto, &ranges, old);
        data.cells.retain(|c| {
            ranges
                .iter()
                .any(|r| r.contains(CellAddr::new(sheet, c.row, c.col)))
        });
        list[pos] = Arc::new(Scenario { id, data });
        Ok(())
    })
}

/// `Op::ShowScenario`: the scenario's values replace the ranges' cells;
/// the scenario shown before stores the current values first when it has
/// `copy_back`.
pub fn show(
    wb: &mut Workbook,
    sheet: SheetId,
    id: u32,
    fx: &mut Effects,
) -> Result<Vec<Change>, ApiError> {
    check_sheet(wb, sheet)?;
    let s = wb
        .sheet(sheet)
        .ok_or_else(|| ApiError::not_found(format!("no sheet {sheet}")))?;
    let target = s
        .meta
        .scenarios
        .iter()
        .find(|x| x.id == id)
        .cloned()
        .ok_or_else(|| ApiError::not_found(format!("no scenario {id}")))?;
    let previous = s.meta.scenarios.iter().find(|x| x.data.active).cloned();
    let target_ranges: Vec<CellRange> = target
        .data
        .ranges
        .iter()
        .map(|r| crate::filter::area(sheet, r))
        .collect();
    let prev_ranges: Vec<CellRange> = previous
        .iter()
        .flat_map(|p| p.data.ranges.iter().map(|r| crate::filter::area(sheet, r)))
        .collect();
    let mut all = target_ranges.clone();
    all.extend(prev_ranges.iter().copied());
    check_edit(wb, &all)?;
    let mut changes = union_snapshot(wb, &all);
    let copy_back = previous
        .as_ref()
        .filter(|p| p.data.copy_back && p.id != id)
        .map(|p| (p.id, snapshot_cells(wb, sheet, &prev_ranges)));
    let meta = with_meta(wb, sheet, fx, |_, meta| {
        let list = Arc::make_mut(&mut meta.scenarios);
        for sc in list.iter_mut() {
            let was = sc.data.active;
            let mut new_cells = None;
            if let Some((pid, cells)) = &copy_back {
                if sc.id == *pid {
                    new_cells = Some(cells.clone());
                }
            }
            let active = sc.id == id;
            if was != active || new_cells.is_some() {
                let m = Arc::make_mut(sc);
                m.data.active = active;
                if let Some(c) = new_cells {
                    m.data.cells = c;
                }
            }
        }
        Ok(())
    })?;
    changes.extend(meta);
    write_cells(wb, sheet, &target_ranges, &target.data.cells);
    Ok(changes)
}

/// `Op::DeleteScenario`.
pub fn delete(
    wb: &mut Workbook,
    sheet: SheetId,
    id: u32,
    fx: &mut Effects,
) -> Result<Vec<Change>, ApiError> {
    check_sheet(wb, sheet)?;
    with_meta(wb, sheet, fx, |_, meta| {
        if !meta.scenarios.iter().any(|s| s.id == id) {
            return Err(ApiError::not_found(format!("no scenario {id}")));
        }
        Arc::make_mut(&mut meta.scenarios).retain(|s| s.id != id);
        Ok(())
    })
}

/// Moves scenario ranges and cells for a row/column change.
pub fn adjust_structure(wb: &mut Workbook, sh: &Shape) {
    let Some(s) = wb.sheet_mut(sh.sheet) else {
        return;
    };
    if s.meta.scenarios.is_empty() {
        return;
    }
    let list = Arc::make_mut(&mut s.meta.scenarios);
    for sc in list.iter_mut() {
        let d = &sc.data;
        let ranges: Vec<CellRangeLocal> = d
            .ranges
            .iter()
            .filter_map(|r| {
                if sh.rows {
                    span_after(sh, r.r0, r.r1).map(|(r0, r1)| CellRangeLocal { r0, r1, ..*r })
                } else {
                    span_after(sh, r.c0, r.c1).map(|(c0, c1)| CellRangeLocal { c0, c1, ..*r })
                }
            })
            .collect();
        let cells: Vec<CellData> = d
            .cells
            .iter()
            .filter_map(|c| {
                let p = if sh.rows { c.row } else { c.col };
                let n = crate::structure::point_after(sh, p)?;
                let mut c = c.clone();
                if sh.rows {
                    c.row = n;
                } else {
                    c.col = n;
                }
                Some(c)
            })
            .collect();
        if ranges != d.ranges || cells != d.cells {
            let m = Arc::make_mut(sc);
            m.data.ranges = ranges;
            m.data.cells = cells;
        }
    }
}
