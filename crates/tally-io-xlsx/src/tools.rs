//! Wave 5 (W5-2) XLSX pieces: outline groups, scenarios, the pivot
//! cache/table parts (written as a post-pass patch of the package, read
//! from the sheet relationships) and the named-style patch of
//! `xl/styles.xml`.

use std::fmt::Write as _;
use std::io::BufRead;

use rust_xlsxwriter::Worksheet;
use tally_model::{
    AggFnData, CellContentData, CellData, CellRangeLocal, DataLayoutData, OutlineGroupData,
    PivotData, PivotDataFieldData, PivotFieldData, PivotSourceData, PivotSubtotalsData,
    ScenarioData, SheetData, WorkbookData,
};
use tally_types::{col_to_letters, Value, MAX_COLS, MAX_ROWS};

use crate::sheet::parse_range;
use crate::write_v03::{Patches, Warn};
use crate::xml::{Elem, Ev, XmlReader};
use crate::XlsxError;

const NS_MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const NS_REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
/// Relationship type of a pivot table part (sheet → table).
pub(crate) const REL_PIVOT_TABLE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/pivotTable";
/// Relationship type of a pivot cache definition (workbook/table → cache).
pub(crate) const REL_PIVOT_CACHE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/pivotCacheDefinition";
const CT_PIVOT_CACHE: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.pivotCacheDefinition+xml";
const CT_PIVOT_TABLE: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.pivotTable+xml";

/// A number as Excel's General format prints it (enough for scenario
/// values and shared items).
fn general_number(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

fn xml_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn cell_ref(r: u32, c: u32) -> String {
    format!("{}{}", col_to_letters(c), u64::from(r) + 1)
}

fn range_ref(r: &CellRangeLocal) -> String {
    format!("{}:{}", cell_ref(r.r0, r.c0), cell_ref(r.r1, r.c1))
}

// ---------------------------------------------------------------------------
// Outline
// ---------------------------------------------------------------------------

/// Writes the row and column outline groups (`outlineLevel`, `collapsed`,
/// `hidden`); groups that rust_xlsxwriter refuses (adjacent groups of one
/// level) are reported and skipped.
pub(crate) fn write_outline(
    ws: &mut Worksheet,
    sheet: &SheetData,
    report: &mut dyn Warn,
) -> Result<(), XlsxError> {
    let mut rows: Vec<&OutlineGroupData> = sheet.row_groups.iter().collect();
    rows.sort_by_key(|g| (g.level, g.start));
    for g in rows {
        if g.start > g.end || g.end >= MAX_ROWS || g.level > 7 {
            continue;
        }
        let r = if g.collapsed {
            ws.group_rows_collapsed(g.start, g.end)
        } else {
            ws.group_rows(g.start, g.end)
        };
        if let Err(e) = r {
            report.warn(format!(
                "sheet {:?}: row group {}..{} skipped ({e})",
                sheet.name,
                g.start + 1,
                g.end + 1
            ));
        }
    }
    let mut cols: Vec<&OutlineGroupData> = sheet.col_groups.iter().collect();
    cols.sort_by_key(|g| (g.level, g.start));
    for g in cols {
        if g.start > g.end || g.end >= MAX_COLS || g.level > 7 {
            continue;
        }
        let (a, b) = (
            u16::try_from(g.start).unwrap_or(u16::MAX),
            u16::try_from(g.end).unwrap_or(u16::MAX),
        );
        let r = if g.collapsed {
            ws.group_columns_collapsed(a, b)
        } else {
            ws.group_columns(a, b)
        };
        if let Err(e) = r {
            report.warn(format!(
                "sheet {:?}: column group {}..{} skipped ({e})",
                sheet.name,
                g.start + 1,
                g.end + 1
            ));
        }
    }
    Ok(())
}

/// Builds groups from per-index outline levels: for every level `k`, a
/// maximal run of indices with level ≥ `k` is a group; it is collapsed
/// when every index of the run is hidden or the index after it carries
/// `collapsed`.
pub(crate) fn groups_from_levels(levels: &[(u32, u8, bool, bool)]) -> Vec<OutlineGroupData> {
    // levels: (index, level, hidden, collapsed-marker on this index)
    let mut out = Vec::new();
    let max = levels.iter().map(|l| l.1).max().unwrap_or(0).min(8);
    for k in 1..=max {
        let mut run: Option<(u32, u32, bool)> = None; // start, end, all hidden
        let mut i = 0;
        while i < levels.len() {
            let (idx, lvl, hidden, _) = levels[i];
            let contiguous = run.is_some_and(|(_, e, _)| e + 1 == idx);
            if lvl >= k {
                run = match run {
                    Some((s, _, h)) if contiguous => Some((s, idx, h && hidden)),
                    Some((s, e, h)) => {
                        let after_collapsed = levels.iter().any(|l| l.0 == e + 1 && l.3);
                        out.push(OutlineGroupData {
                            start: s,
                            end: e,
                            level: k,
                            collapsed: h || after_collapsed,
                        });
                        let _ = s;
                        Some((idx, idx, hidden))
                    }
                    None => Some((idx, idx, hidden)),
                };
            } else if let Some((s, e, h)) = run.take() {
                let after_collapsed = levels.iter().any(|l| l.0 == e + 1 && l.3);
                out.push(OutlineGroupData {
                    start: s,
                    end: e,
                    level: k,
                    collapsed: h || after_collapsed,
                });
            }
            i += 1;
        }
        if let Some((s, e, h)) = run {
            let after_collapsed = levels.iter().any(|l| l.0 == e + 1 && l.3);
            out.push(OutlineGroupData {
                start: s,
                end: e,
                level: k,
                collapsed: h || after_collapsed,
            });
        }
    }
    out.truncate(100_000);
    out
}

// ---------------------------------------------------------------------------
// Scenarios
// ---------------------------------------------------------------------------

fn scenario_value_text(c: &CellData) -> Option<String> {
    let v = match &c.content {
        CellContentData::Value(v) => v.clone(),
        CellContentData::Formula { cached, .. } | CellContentData::SharedFormula { cached, .. } => {
            cached.clone()?
        }
    };
    Some(match v {
        Value::Empty => return None,
        Value::Number(n) => general_number(n),
        Value::Text(t) => t.to_string(),
        Value::Bool(b) => if b { "TRUE" } else { "FALSE" }.to_string(),
        Value::Error(e) => e.code().to_string(),
        Value::Array(_) => return None,
    })
}

/// The `<scenarios>` element of a sheet (`None` without scenarios).
pub(crate) fn scenarios_xml(sheet: &SheetData) -> Option<String> {
    if sheet.scenarios.is_empty() {
        return None;
    }
    let current = sheet.scenarios.iter().position(|s| s.active).unwrap_or(0);
    let mut s = format!(r#"<scenarios current="{current}" show="{current}">"#);
    for sc in sheet.scenarios.iter().take(1_000) {
        let cells: Vec<(String, String)> = sc
            .cells
            .iter()
            .filter(|c| {
                sc.ranges
                    .iter()
                    .any(|r| r.r0 <= c.row && c.row <= r.r1 && r.c0 <= c.col && c.col <= r.c1)
            })
            .filter_map(|c| Some((cell_ref(c.row, c.col), scenario_value_text(c)?)))
            .take(32)
            .collect();
        let _ = write!(
            s,
            r#"<scenario name="{}" locked="{}" hidden="0" count="{}""#,
            xml_attr(&sc.name),
            u8::from(sc.prevent_changes),
            cells.len()
        );
        if !sc.comment.is_empty() {
            let _ = write!(s, r#" comment="{}""#, xml_attr(&sc.comment));
        }
        s.push('>');
        for (r, v) in cells {
            let _ = write!(s, r#"<inputCells r="{r}" val="{}"/>"#, xml_attr(&v));
        }
        s.push_str("</scenario>");
    }
    s.push_str("</scenarios>");
    Some(s)
}

/// Adds the view options rust_xlsxwriter cannot set (`showFormulas`,
/// `showZeros`, `showRowColHeaders`) to the sheet views.
pub(crate) fn view_patches(data: &WorkbookData, patches: &mut Patches) {
    for (i, sheet) in data.sheets.iter().enumerate() {
        let Some(v) = &sheet.view_options else {
            continue;
        };
        let mut attrs: Vec<&str> = Vec::new();
        if v.show_formulas {
            attrs.push(r#"showFormulas="1""#);
        }
        if !v.show_zero_values {
            attrs.push(r#"showZeros="0""#);
        }
        if !v.show_headers {
            attrs.push(r#"showRowColHeaders="0""#);
        }
        if !attrs.is_empty() {
            patches
                .sheet_view_attrs
                .push((format!("xl/worksheets/sheet{}.xml", i + 1), attrs.join(" ")));
        }
    }
}

/// Adds `<scenarios>` to the sheet XML through the patches pass.
pub(crate) fn scenario_patches(data: &WorkbookData, patches: &mut Patches, report: &mut dyn Warn) {
    for (i, sheet) in data.sheets.iter().enumerate() {
        if let Some(xml) = scenarios_xml(sheet) {
            patches
                .sheet_inserts
                .push((format!("xl/worksheets/sheet{}.xml", i + 1), xml));
            if sheet.scenarios.iter().any(|sc| sc.cells.len() > 32) {
                report.warn(format!(
                    "sheet {:?}: Excel keeps at most 32 changing cells per scenario; the rest were dropped",
                    sheet.name
                ));
            }
        }
    }
}

/// Parses `<scenarios>` into the sheet's scenarios.
pub(crate) fn parse_scenarios<R: BufRead>(
    xr: &mut XmlReader<R>,
    e: &Elem,
    out: &mut Vec<ScenarioData>,
) -> Result<(), XlsxError> {
    let current: Option<usize> = e.attr_local("current").and_then(|v| v.trim().parse().ok());
    let mut idx = 0usize;
    loop {
        match xr.next()? {
            Ev::Start(c) if c.local() == "scenario" => {
                let mut sc = ScenarioData {
                    name: c.attr_local("name").unwrap_or("Scenario").to_string(),
                    comment: c.attr_local("comment").unwrap_or_default().to_string(),
                    prevent_changes: c.attr_local("locked").is_some_and(crate::units::parse_bool),
                    active: current == Some(idx),
                    ..ScenarioData::default()
                };
                loop {
                    match xr.next()? {
                        Ev::Start(ic) if ic.local() == "inputCells" => {
                            if let Some((r, col)) =
                                ic.attr_local("r").and_then(crate::sheet::parse_cell_ref)
                            {
                                let text = ic.attr_local("val").unwrap_or_default();
                                let v = match text.trim().parse::<f64>() {
                                    Ok(n) if n.is_finite() => Value::Number(n),
                                    _ if text.eq_ignore_ascii_case("TRUE") => Value::Bool(true),
                                    _ if text.eq_ignore_ascii_case("FALSE") => Value::Bool(false),
                                    _ => Value::text(text),
                                };
                                if sc.cells.len() < 1_000 {
                                    sc.ranges.push(CellRangeLocal {
                                        r0: r,
                                        c0: col,
                                        r1: r,
                                        c1: col,
                                    });
                                    sc.cells.push(CellData {
                                        row: r,
                                        col,
                                        content: CellContentData::Value(v),
                                        style: None,
                                    });
                                }
                            }
                            xr.skip()?;
                        }
                        Ev::Start(_) => xr.skip()?,
                        Ev::End => break,
                        Ev::Eof => break,
                        _ => {}
                    }
                }
                if out.len() < 1_000 {
                    out.push(sc);
                }
                idx += 1;
            }
            Ev::Start(_) => xr.skip()?,
            Ev::End => return Ok(()),
            Ev::Eof => return Ok(()),
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Pivot tables (write)
// ---------------------------------------------------------------------------

fn agg_subtotal(f: AggFnData) -> &'static str {
    match f {
        AggFnData::Sum => "sum",
        AggFnData::Count => "count",
        AggFnData::Average => "average",
        AggFnData::Max => "max",
        AggFnData::Min => "min",
        AggFnData::Product => "product",
        AggFnData::CountNums => "countNums",
        AggFnData::StDev => "stdDev",
        AggFnData::StDevP => "stdDevp",
        AggFnData::Var => "var",
        AggFnData::VarP => "varp",
    }
}

fn agg_parse(s: &str) -> AggFnData {
    match s {
        "count" => AggFnData::Count,
        "average" => AggFnData::Average,
        "max" => AggFnData::Max,
        "min" => AggFnData::Min,
        "product" => AggFnData::Product,
        "countNums" => AggFnData::CountNums,
        "stdDev" => AggFnData::StDev,
        "stdDevp" => AggFnData::StDevP,
        "var" => AggFnData::Var,
        "varp" => AggFnData::VarP,
        _ => AggFnData::Sum,
    }
}

fn cell_text(c: &CellData) -> Option<String> {
    scenario_value_text(c)
}

/// The source of a pivot: sheet index and range.
fn source_of(data: &WorkbookData, p: &PivotData) -> Option<(usize, CellRangeLocal)> {
    match &p.source {
        PivotSourceData::Range { sheet, range } => Some((*sheet as usize, *range)),
        PivotSourceData::DatabaseRange { name } => {
            data.sheets.iter().enumerate().find_map(|(i, s)| {
                s.database_ranges
                    .iter()
                    .find(|d| d.name.eq_ignore_ascii_case(name))
                    .map(|d| (i, d.range))
            })
        }
    }
}

/// Adds the pivot cache and pivot table parts through the patches pass.
pub(crate) fn pivot_patches(data: &WorkbookData, patches: &mut Patches, report: &mut dyn Warn) {
    let mut n = 0u32;
    for (tab, sheet) in data.sheets.iter().enumerate() {
        for p in &sheet.pivots {
            let Some((src_tab, src)) = source_of(data, p) else {
                report.warn(format!(
                    "pivot table {:?}: its source is gone; not written",
                    p.name
                ));
                continue;
            };
            let Some(src_sheet) = data.sheets.get(src_tab) else {
                continue;
            };
            if p.data_fields.is_empty() || src.r0 >= src.r1 {
                continue;
            }
            n += 1;
            let names: Vec<String> = (src.c0..=src.c1)
                .map(|c| {
                    src_sheet
                        .cells
                        .iter()
                        .find(|x| x.row == src.r0 && x.col == c)
                        .and_then(cell_text)
                        .filter(|t| !t.is_empty())
                        .unwrap_or_else(|| format!("Column{}", c - src.c0 + 1))
                })
                .collect();
            // Distinct items per axis field (the cache's shared items).
            let axis_cols: Vec<u32> = p
                .row_fields
                .iter()
                .chain(&p.col_fields)
                .chain(&p.page_fields)
                .map(|f| f.source_col)
                .collect();
            let mut items: Vec<Vec<String>> = vec![Vec::new(); names.len()];
            for (ci, list) in items.iter_mut().enumerate() {
                if !axis_cols.contains(&(ci as u32)) {
                    continue;
                }
                let col = src.c0 + ci as u32;
                let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
                for cell in src_sheet
                    .cells
                    .iter()
                    .filter(|x| x.col == col && x.row > src.r0 && x.row <= src.r1)
                {
                    if let Some(t) = cell_text(cell) {
                        if seen.insert(t.clone()) && list.len() < 10_000 {
                            list.push(t);
                        }
                    }
                }
            }
            let records = src.r1 - src.r0;
            // Cache definition.
            let mut cache = format!(
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<pivotCacheDefinition xmlns="{NS_MAIN}" xmlns:r="{NS_REL}" saveData="0" refreshOnLoad="1" refreshedBy="Tally" createdVersion="3" refreshedVersion="3" minRefreshableVersion="3" recordCount="{records}"><cacheSource type="worksheet"><worksheetSource ref="{}" sheet="{}"/></cacheSource><cacheFields count="{}">"#,
                range_ref(&src),
                xml_attr(&src_sheet.name),
                names.len()
            );
            for (ci, name) in names.iter().enumerate() {
                let _ = write!(
                    cache,
                    r#"<cacheField name="{}" numFmtId="0">"#,
                    xml_attr(name)
                );
                if items[ci].is_empty() {
                    cache.push_str(r#"<sharedItems/>"#);
                } else {
                    let _ = write!(cache, r#"<sharedItems count="{}">"#, items[ci].len());
                    for it in &items[ci] {
                        let _ = write!(cache, r#"<s v="{}"/>"#, xml_attr(it));
                    }
                    cache.push_str("</sharedItems>");
                }
                cache.push_str("</cacheField>");
            }
            cache.push_str("</cacheFields></pivotCacheDefinition>");
            // Table definition.
            let output = p.output.unwrap_or(CellRangeLocal {
                r0: p.target.0,
                c0: p.target.1,
                r1: p.target.0,
                c1: p.target.1,
            });
            let data_in_rows = p.data_fields.len() > 1 && p.data_layout == DataLayoutData::Row;
            let data_in_cols = p.data_fields.len() > 1 && p.data_layout == DataLayoutData::Column;
            let first_data_col = p.row_fields.len() as u32 + u32::from(data_in_rows);
            let mut table = format!(
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<pivotTableDefinition xmlns="{NS_MAIN}" name="{}" cacheId="{n}" applyNumberFormats="0" applyBorderFormats="0" applyFontFormats="0" applyPatternFormats="0" applyAlignmentFormats="0" applyWidthHeightFormats="1" dataCaption="Data" updatedVersion="3" minRefreshableVersion="3" useAutoFormatting="1" itemPrintTitles="1" createdVersion="3" indent="0" outline="0" outlineData="0" compact="0" compactData="0" gridDropZones="1" multipleFieldFilters="0" rowGrandTotals="{}" colGrandTotals="{}"><location ref="{}" firstHeaderRow="1" firstDataRow="{}" firstDataCol="{}" rowPageCount="{}" colPageCount="{}"/>"#,
                xml_attr(if p.name.is_empty() {
                    "DataPilot"
                } else {
                    &p.name
                }),
                u8::from(p.row_totals),
                u8::from(p.col_totals),
                range_ref(&output),
                p.col_fields.len() as u32 + 1 + u32::from(data_in_cols),
                first_data_col.max(1),
                u8::from(!p.page_fields.is_empty()),
                u8::from(!p.page_fields.is_empty()),
            );
            let _ = write!(table, r#"<pivotFields count="{}">"#, names.len());
            for (ci, field_items) in items.iter().enumerate() {
                let c = ci as u32;
                let axis = if p.row_fields.iter().any(|f| f.source_col == c) {
                    Some("axisRow")
                } else if p.col_fields.iter().any(|f| f.source_col == c) {
                    Some("axisCol")
                } else if p.page_fields.iter().any(|f| f.source_col == c) {
                    Some("axisPage")
                } else {
                    None
                };
                let is_data = p.data_fields.iter().any(|d| d.source_col == c);
                table.push_str("<pivotField");
                if let Some(a) = axis {
                    let _ = write!(table, r#" axis="{a}""#);
                }
                if is_data {
                    table.push_str(r#" dataField="1""#);
                }
                let subtotal_off = p
                    .row_fields
                    .iter()
                    .chain(&p.col_fields)
                    .chain(&p.page_fields)
                    .find(|f| f.source_col == c)
                    .is_some_and(|f| matches!(f.subtotals, PivotSubtotalsData::None));
                if subtotal_off {
                    table.push_str(r#" defaultSubtotal="0""#);
                }
                table.push_str(r#" showAll="0""#);
                if axis.is_some() {
                    let _ = write!(
                        table,
                        r#"><items count="{}">"#,
                        field_items.len() + usize::from(!subtotal_off)
                    );
                    for i in 0..field_items.len() {
                        let _ = write!(table, r#"<item x="{i}"/>"#);
                    }
                    if !subtotal_off {
                        table.push_str(r#"<item t="default"/>"#);
                    }
                    table.push_str("</items></pivotField>");
                } else {
                    table.push_str("/>");
                }
            }
            table.push_str("</pivotFields>");
            let axis_fields =
                |table: &mut String, tag: &str, fields: &[PivotFieldData], data_here: bool| {
                    let count = fields.len() + usize::from(data_here);
                    if count == 0 {
                        return;
                    }
                    let _ = write!(table, r#"<{tag} count="{count}">"#);
                    for f in fields {
                        let _ = write!(table, r#"<field x="{}"/>"#, f.source_col);
                    }
                    if data_here {
                        table.push_str(r#"<field x="-2"/>"#);
                    }
                    let _ = write!(table, "</{tag}>");
                };
            axis_fields(&mut table, "rowFields", &p.row_fields, data_in_rows);
            table.push_str(r#"<rowItems count="1"><i/></rowItems>"#);
            axis_fields(&mut table, "colFields", &p.col_fields, data_in_cols);
            table.push_str(r#"<colItems count="1"><i/></colItems>"#);
            if !p.page_fields.is_empty() {
                let _ = write!(table, r#"<pageFields count="{}">"#, p.page_fields.len());
                for f in &p.page_fields {
                    let _ = write!(table, r#"<pageField fld="{}" hier="-1"/>"#, f.source_col);
                }
                table.push_str("</pageFields>");
            }
            let _ = write!(table, r#"<dataFields count="{}">"#, p.data_fields.len());
            for d in &p.data_fields {
                let name = d.caption.clone().unwrap_or_else(|| {
                    format!(
                        "{} of {}",
                        match d.function {
                            AggFnData::Sum => "Sum",
                            AggFnData::Count => "Count",
                            AggFnData::Average => "Average",
                            AggFnData::Max => "Max",
                            AggFnData::Min => "Min",
                            AggFnData::Product => "Product",
                            AggFnData::CountNums => "Count",
                            AggFnData::StDev => "StdDev",
                            AggFnData::StDevP => "StdDevp",
                            AggFnData::Var => "Var",
                            AggFnData::VarP => "Varp",
                        },
                        names
                            .get(d.source_col as usize)
                            .cloned()
                            .unwrap_or_default()
                    )
                });
                let _ = write!(
                    table,
                    r#"<dataField name="{}" fld="{}" subtotal="{}" baseField="0" baseItem="0"/>"#,
                    xml_attr(&name),
                    d.source_col,
                    agg_subtotal(d.function)
                );
            }
            table.push_str("</dataFields>");
            table.push_str(r#"<pivotTableStyleInfo name="PivotStyleLight16" showRowHeaders="1" showColHeaders="1" showRowStripes="0" showColStripes="0" showLastColumn="1"/></pivotTableDefinition>"#);
            let cache_path = format!("xl/pivotCache/pivotCacheDefinition{n}.xml");
            let table_path = format!("xl/pivotTables/pivotTable{n}.xml");
            patches.extra_parts.push((cache_path.clone(), cache));
            patches.extra_parts.push((table_path.clone(), table));
            patches
                .content_types
                .push((format!("/{cache_path}"), CT_PIVOT_CACHE.into()));
            patches
                .content_types
                .push((format!("/{table_path}"), CT_PIVOT_TABLE.into()));
            patches.rels.push((
                "xl/_rels/workbook.xml.rels".into(),
                vec![(
                    format!("rIdPc{n}"),
                    REL_PIVOT_CACHE.into(),
                    format!("pivotCache/pivotCacheDefinition{n}.xml"),
                )],
            ));
            patches.rels.push((
                format!("xl/worksheets/_rels/sheet{}.xml.rels", tab + 1),
                vec![(
                    format!("rIdPt{n}"),
                    REL_PIVOT_TABLE.into(),
                    format!("../pivotTables/pivotTable{n}.xml"),
                )],
            ));
            patches.rels.push((
                format!("xl/pivotTables/_rels/pivotTable{n}.xml.rels"),
                vec![(
                    "rId1".into(),
                    REL_PIVOT_CACHE.into(),
                    format!("../pivotCache/pivotCacheDefinition{n}.xml"),
                )],
            ));
            patches
                .workbook_inserts
                .push(format!(r#"<pivotCache cacheId="{n}" r:id="rIdPc{n}"/>"#));
        }
    }
}

// ---------------------------------------------------------------------------
// Pivot tables (read)
// ---------------------------------------------------------------------------

/// A pivot cache definition as read: the source and the field names.
#[derive(Debug, Default, Clone)]
pub(crate) struct CacheInfo {
    pub sheet: Option<String>,
    pub range: Option<CellRangeLocal>,
    pub name: Option<String>,
    pub fields: Vec<String>,
}

/// Parses `xl/pivotCache/pivotCacheDefinitionN.xml`.
pub(crate) fn parse_cache<R: BufRead>(xr: &mut XmlReader<R>) -> Result<CacheInfo, XlsxError> {
    let mut info = CacheInfo::default();
    loop {
        match xr.next()? {
            Ev::Start(e) => match e.local() {
                "pivotCacheDefinition" | "cacheSource" | "cacheFields" => {}
                "worksheetSource" => {
                    info.sheet = e.attr_local("sheet").map(str::to_string);
                    info.range = e.attr_local("ref").and_then(parse_range);
                    info.name = e.attr_local("name").map(str::to_string);
                    xr.skip()?;
                }
                "cacheField" => {
                    if info.fields.len() < 16_384 {
                        info.fields
                            .push(e.attr_local("name").unwrap_or_default().to_string());
                    }
                    xr.skip()?;
                }
                _ => xr.skip()?,
            },
            Ev::Eof => return Ok(info),
            _ => {}
        }
    }
}

/// Parses `xl/pivotTables/pivotTableN.xml` into a definition (the source
/// comes from `cache`); `None` when nothing usable was found.
pub(crate) fn parse_table<R: BufRead>(
    xr: &mut XmlReader<R>,
    cache: &CacheInfo,
    sheet_index: &dyn Fn(&str) -> Option<u32>,
    db_range: &dyn Fn(&str) -> Option<(u32, CellRangeLocal)>,
) -> Result<Option<PivotData>, XlsxError> {
    let mut p = PivotData::default();
    let mut field_axis: Vec<Option<&'static str>> = Vec::new();
    let mut no_subtotal: Vec<bool> = Vec::new();
    let mut row_x: Vec<i64> = Vec::new();
    let mut col_x: Vec<i64> = Vec::new();
    let mut page_x: Vec<i64> = Vec::new();
    loop {
        match xr.next()? {
            Ev::Start(e) => match e.local() {
                "pivotTableDefinition" => {
                    p.name = e.attr_local("name").unwrap_or("DataPilot").to_string();
                    p.row_totals = e
                        .attr_local("rowGrandTotals")
                        .is_none_or(crate::units::parse_bool);
                    p.col_totals = e
                        .attr_local("colGrandTotals")
                        .is_none_or(crate::units::parse_bool);
                }
                "location" => {
                    if let Some(r) = e.attr_local("ref").and_then(parse_range) {
                        p.target = (r.r0, r.c0);
                        p.output = Some(r);
                    }
                    xr.skip()?;
                }
                "pivotFields" => {}
                "pivotField" => {
                    field_axis.push(match e.attr_local("axis") {
                        Some("axisRow") => Some("row"),
                        Some("axisCol") => Some("col"),
                        Some("axisPage") => Some("page"),
                        _ => None,
                    });
                    no_subtotal.push(e.attr_local("defaultSubtotal") == Some("0"));
                    xr.skip()?;
                }
                "rowFields" | "colFields" => {
                    let is_row = e.local() == "rowFields";
                    let mut xs = Vec::new();
                    loop {
                        match xr.next()? {
                            Ev::Start(f) => {
                                if f.local() == "field" {
                                    if let Some(x) =
                                        f.attr_local("x").and_then(|v| v.trim().parse::<i64>().ok())
                                    {
                                        xs.push(x);
                                    }
                                }
                                xr.skip()?;
                            }
                            Ev::End => break,
                            Ev::Eof => break,
                            _ => {}
                        }
                    }
                    if is_row {
                        row_x = xs;
                    } else {
                        col_x = xs;
                    }
                }
                "pageFields" => loop {
                    match xr.next()? {
                        Ev::Start(f) => {
                            if f.local() == "pageField" {
                                if let Some(x) = f
                                    .attr_local("fld")
                                    .and_then(|v| v.trim().parse::<i64>().ok())
                                {
                                    page_x.push(x);
                                }
                            }
                            xr.skip()?;
                        }
                        Ev::End => break,
                        Ev::Eof => break,
                        _ => {}
                    }
                },
                "dataFields" => loop {
                    match xr.next()? {
                        Ev::Start(f) => {
                            if f.local() == "dataField" {
                                if let Some(x) = f
                                    .attr_local("fld")
                                    .and_then(|v| v.trim().parse::<u32>().ok())
                                {
                                    p.data_fields.push(PivotDataFieldData {
                                        source_col: x,
                                        function: agg_parse(
                                            f.attr_local("subtotal").unwrap_or("sum"),
                                        ),
                                        caption: f.attr_local("name").map(str::to_string),
                                        show_as: None,
                                    });
                                }
                            }
                            xr.skip()?;
                        }
                        Ev::End => break,
                        Ev::Eof => break,
                        _ => {}
                    }
                },
                _ => xr.skip()?,
            },
            Ev::Eof => break,
            _ => {}
        }
    }
    let source = match (&cache.sheet, cache.range, &cache.name) {
        (Some(sheet), Some(range), _) => {
            sheet_index(sheet).map(|s| PivotSourceData::Range { sheet: s, range })
        }
        (_, _, Some(name)) => {
            db_range(name).map(|_| PivotSourceData::DatabaseRange { name: name.clone() })
        }
        _ => None,
    };
    let Some(source) = source else {
        return Ok(None);
    };
    p.source = source;
    let field = |x: i64| -> Option<PivotFieldData> {
        let i = usize::try_from(x).ok()?;
        Some(PivotFieldData {
            source_col: i as u32,
            name: cache.fields.get(i).cloned().unwrap_or_default(),
            subtotals: if no_subtotal.get(i).copied().unwrap_or(false) {
                PivotSubtotalsData::None
            } else {
                PivotSubtotalsData::Auto
            },
            ..PivotFieldData::default()
        })
    };
    for x in &row_x {
        if *x == -2 {
            p.data_layout = DataLayoutData::Row;
        } else if let Some(f) = field(*x) {
            p.row_fields.push(f);
        }
    }
    for x in &col_x {
        if *x == -2 {
            p.data_layout = DataLayoutData::Column;
        } else if let Some(f) = field(*x) {
            p.col_fields.push(f);
        }
    }
    for x in &page_x {
        if let Some(f) = field(*x) {
            p.page_fields.push(f);
        }
    }
    let _ = field_axis;
    if p.data_fields.is_empty() {
        return Ok(None);
    }
    Ok(Some(p))
}

// ---------------------------------------------------------------------------
// Named styles (write patch)
// ---------------------------------------------------------------------------

/// Adds the named cell styles to the patches (the names round-trip; the
/// attributes stay on the cells, which carry them materialized).
pub(crate) fn style_patches(data: &WorkbookData, patches: &mut Patches) {
    for st in &data.cell_styles {
        if st.name == "Default" || st.name == "Normal" {
            continue;
        }
        patches.style_names.push(st.name.clone());
    }
}

/// Patches `xl/styles.xml` with named styles: one `cellStyleXfs` entry
/// (the default look) and one `cellStyle` per name.
pub(crate) fn patch_styles_xml(xml: &str, names: &[String]) -> String {
    if names.is_empty() {
        return xml.to_string();
    }
    let mut out = xml.to_string();
    // cellStyleXfs: append entries and bump the count.
    let base = count_of(&out, "<cellStyleXfs count=\"");
    let entries: String = names
        .iter()
        .map(|_| r#"<xf numFmtId="0" fontId="0" fillId="0" borderId="0"/>"#)
        .collect();
    if let Some(i) = out.find("</cellStyleXfs>") {
        out.insert_str(i, &entries);
        out = replace_count(&out, "<cellStyleXfs count=\"", base + names.len());
    }
    let mut styles = String::new();
    for (k, n) in names.iter().enumerate() {
        let _ = write!(
            styles,
            r#"<cellStyle name="{}" xfId="{}"/>"#,
            xml_attr(n),
            base + k
        );
    }
    if let Some(i) = out.find("</cellStyles>") {
        let n = count_of(&out, "<cellStyles count=\"");
        out.insert_str(i, &styles);
        out = replace_count(&out, "<cellStyles count=\"", n + names.len());
    } else if let Some(i) = out.find("<dxfs") {
        let el = format!(
            r#"<cellStyles count="{}">{styles}</cellStyles>"#,
            names.len()
        );
        out.insert_str(i, &el);
    }
    out
}

fn count_of(xml: &str, prefix: &str) -> usize {
    xml.find(prefix)
        .and_then(|i| {
            let rest = &xml[i + prefix.len()..];
            let end = rest.find('"')?;
            rest[..end].parse().ok()
        })
        .unwrap_or(0)
}

fn replace_count(xml: &str, prefix: &str, n: usize) -> String {
    match xml.find(prefix) {
        Some(i) => {
            let rest = &xml[i + prefix.len()..];
            match rest.find('"') {
                Some(end) => format!("{}{prefix}{n}{}", &xml[..i], &rest[end..]),
                None => xml.to_string(),
            }
        }
        None => xml.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_to_groups() {
        // rows 1..3 level 1, rows 2..3 level 2 (hidden), row 4 collapsed marker
        let levels = vec![
            (1, 1, false, false),
            (2, 2, true, false),
            (3, 2, true, false),
            (4, 0, false, true),
        ];
        let g = groups_from_levels(&levels);
        assert_eq!(g.len(), 2);
        assert_eq!(
            (g[0].start, g[0].end, g[0].level, g[0].collapsed),
            (1, 3, 1, true)
        );
        assert_eq!(
            (g[1].start, g[1].end, g[1].level, g[1].collapsed),
            (2, 3, 2, true)
        );
    }

    #[test]
    fn styles_patch_adds_names() {
        let xml = r#"<styleSheet><cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs><cellXfs count="1"><xf/></cellXfs><cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles><dxfs count="0"/></styleSheet>"#;
        let out = patch_styles_xml(xml, &["Heading".into()]);
        assert!(out.contains(r#"<cellStyleXfs count="2">"#));
        assert!(out.contains(r#"<cellStyle name="Heading" xfId="1"/>"#));
        assert!(out.contains(r#"<cellStyles count="2">"#));
    }
}
