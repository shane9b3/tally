//! `read_xlsx`: package → [`WorkbookData`].

use std::collections::HashSet;
use std::io::{Read, Seek};
use std::sync::Arc;

use tally_model::{NameData, SheetData, WorkbookData};
use tally_types::{CellAddr, DateEpoch, IterSettings};

use crate::archive::Package;
use crate::formula::{excel_name_to_of, XlsxResolver};
use crate::sheet::{parse_sheet, Budget, SheetCtx};
use crate::sheet_v03::MAX_LINKS_PER_SHEET;
use crate::sst::parse_sst;
use crate::styles::{parse_styles, StyleTable};
use crate::workbook::{
    parse_content_types, parse_doc_props, parse_rels, parse_workbook, part_dir, rels_path,
    ContentTypes, Rel,
};
use crate::xml::{XmlLimits, XmlReader};
use crate::{Limits, XlsxError};

const DEFAULT_WORKBOOK: &str = "xl/workbook.xml";

pub(crate) fn read(reader: impl Read + Seek, limits: &Limits) -> Result<WorkbookData, XlsxError> {
    let _attr_budget = crate::xml::AttrBudget::install(limits.max_attr_bytes);
    let mut pkg = Package::open(reader, limits)?;
    let xl = XmlLimits {
        max_depth: limits.max_xml_depth,
        max_attrs: limits.max_attrs_per_element,
    };
    let other = limits.max_entry_bytes;
    let big = limits.max_sheet_xml_bytes;

    // [Content_Types].xml is optional for us (Excel requires it).
    let ct: ContentTypes = pkg
        .with_entry("[Content_Types].xml", other, |r| {
            parse_content_types(&mut XmlReader::new(r, xl))
        })?
        .unwrap_or_default();
    if ct.binary && !pkg.has(DEFAULT_WORKBOOK) {
        return Err(XlsxError::Unsupported(
            "binary workbooks (.xlsb); save as .xlsx".into(),
        ));
    }

    // The workbook part: the package's officeDocument relationship, the
    // content types' main part, or the conventional path.
    let root_rels = pkg
        .with_entry("_rels/.rels", other, |r| {
            parse_rels(&mut XmlReader::new(r, xl), "")
        })?
        .unwrap_or_default()
        .list;
    let candidates: Vec<String> = root_rels
        .iter()
        .filter(|r| r.is("officeDocument"))
        .map(|r| r.target.clone())
        .chain(ct.main_parts.iter().cloned())
        .chain(std::iter::once(DEFAULT_WORKBOOK.to_string()))
        .collect();
    let Some(wb_path) = candidates.into_iter().find(|p| pkg.has(p)) else {
        return Err(XlsxError::NotXlsx);
    };
    let info = pkg
        .with_entry(&wb_path, other, |r| {
            parse_workbook(&mut XmlReader::new(r, xl), limits.max_sheets)
        })?
        .ok_or(XlsxError::NotXlsx)?;
    let dir = part_dir(&wb_path).to_string();
    let rels: Vec<Rel> = pkg
        .with_entry(&rels_path(&wb_path), other, |r| {
            parse_rels(&mut XmlReader::new(r, xl), &dir)
        })?
        .unwrap_or_default()
        .list;
    let rel_target = |kind: &str, default: &str| {
        rels.iter()
            .find(|r| r.is(kind))
            .map_or_else(|| format!("{dir}/{default}"), |r| r.target.clone())
    };

    let sst_path = rel_target("sharedStrings", "sharedStrings.xml");
    let sst: Vec<Arc<str>> = pkg
        .with_entry(&sst_path, big, |r| {
            parse_sst(&mut XmlReader::new(r, xl), limits)
        })?
        .unwrap_or_default();
    // A custom theme changes what `theme="N"` colours mean (item 28).
    let theme_path = rel_target("theme", "theme1.xml");
    let theme = pkg
        .with_entry(&theme_path, other, |r| {
            crate::colors::parse_theme(&mut XmlReader::new(r, xl))
        })?
        .flatten();
    crate::colors::set_active_theme(theme);
    let styles_path = rel_target("styles", "styles.xml");
    let mut styles: StyleTable = pkg
        .with_entry(&styles_path, other, |r| {
            parse_styles(&mut XmlReader::new(r, xl))
        })?
        .unwrap_or_default();

    let names: Vec<String> = info.sheets.iter().map(|s| s.name.clone()).collect();
    let resolver = XlsxResolver::new(&names);
    let cell_styles = std::mem::take(&mut styles.cell_styles);
    // Pivot tables are resolved once every sheet is read (their sources
    // may be later sheets): (target tab, table part path).
    let mut pivot_parts: Vec<(usize, String)> = Vec::new();
    let mut budget = Budget {
        warnings: std::mem::take(&mut styles.warnings),
        ..Budget::default()
    };
    let mut sheets = Vec::with_capacity(info.sheets.len());
    // Any sheet view without outline symbols hides them (AOO: per document).
    let mut hide_outline_symbols = false;
    let mut objects = crate::drawing_read::ObjectBudget::default();
    // Each worksheet part may back one sheet only: otherwise a tiny package
    // could make every sheet re-inflate and re-parse one big part.
    let mut seen_targets: HashSet<String> = HashSet::new();
    for (i, entry) in info.sheets.iter().enumerate() {
        let sheet_id = u32::try_from(i).unwrap_or(u32::MAX);
        let target = rels
            .iter()
            .find(|r| r.id == entry.rid)
            .map(|r| r.target.clone());
        if let Some(t) = &target {
            if !seen_targets.insert(t.clone()) {
                return Err(XlsxError::Xml(format!(
                    "two sheets use the same worksheet part {t:?}"
                )));
            }
        }
        let ctx = SheetCtx {
            sheet: sheet_id,
            sst: &sst,
            styles: &styles,
            date1904: info.date1904,
            resolver: &resolver,
            limits,
        };
        let parsed = match &target {
            Some(t) => pkg.with_entry(t, big, |r| {
                parse_sheet(&mut XmlReader::new(r, xl), &ctx, &mut budget)
            })?,
            None => None,
        };
        // A sheet whose part is missing (or a chartsheet) opens empty.
        let (mut sheet, extras) = parsed.unwrap_or_default();
        sheet.page_setup = extras.page.build();
        hide_outline_symbols |= extras.hide_outline_symbols;
        // The sheet's own relationships: hyperlink targets and comments.
        let sheet_rels = match &target {
            Some(t) => pkg
                .with_entry(&rels_path(t), other, |r| {
                    parse_rels(&mut XmlReader::new(r, xl), part_dir(t))
                })?
                .unwrap_or_default(),
            None => Default::default(),
        };
        if sheet_rels.dropped > 0 {
            budget.warn(format!(
                "Hyperlink targets longer than {} bytes were dropped.",
                crate::workbook::MAX_REL_TARGET
            ));
        }
        // Relationship id → external target (MEDIUM-6: one hash lookup
        // per link instead of a scan of every relationship).
        let external: std::collections::HashMap<&str, &str> = sheet_rels
            .list
            .iter()
            .filter(|r| r.external)
            .map(|r| (r.id.as_str(), r.target.as_str()))
            .collect();
        // Links are expanded to one entry per cell of their `ref`; every
        // cell counts against the cell budget, at most 10,000 per ref and
        // MAX_LINKS_PER_SHEET per sheet (HIGH-3), and the URL is shared.
        let mut seen: std::collections::HashSet<(u32, u32)> = std::collections::HashSet::new();
        let mut links_capped = false;
        'links: for (range, rid, location) in extras.links {
            let url: Option<Arc<str>> = match (&rid, &location) {
                (Some(id), None) => external.get(id.as_str()).map(|t| Arc::from(*t)),
                (Some(id), Some(loc)) => external
                    .get(id.as_str())
                    .map(|t| Arc::from(format!("{t}#{loc}"))),
                (None, Some(loc)) => Some(Arc::from(format!("#{}", loc.replacen('!', ".", 1)))),
                (None, None) => None,
            };
            let Some(url) = url else { continue };
            if url.len() > crate::workbook::MAX_REL_TARGET {
                links_capped = true;
                continue;
            }
            let cells = (u64::from(range.r1 - range.r0) + 1) * (u64::from(range.c1 - range.c0) + 1);
            if cells > 10_000 {
                continue;
            }
            // Expanded link cells count against the cell budget; past it
            // the remaining links are dropped (the file still opens).
            if budget.cells.saturating_add(cells) > limits.max_cells {
                links_capped = true;
                break;
            }
            budget.add_cells(cells, limits)?;
            for r in range.r0..=range.r1 {
                for c in range.c0..=range.c1 {
                    if sheet.links.len() >= MAX_LINKS_PER_SHEET {
                        links_capped = true;
                        break 'links;
                    }
                    if seen.insert((r, c)) {
                        sheet.links.push(tally_model::LinkData {
                            row: r,
                            col: c,
                            url: url.clone(),
                        });
                    }
                }
            }
        }
        if links_capped {
            budget.warn(format!(
                "Only the first {MAX_LINKS_PER_SHEET} hyperlinks of a sheet are kept."
            ));
        }
        sheet.links.sort_by_key(|l| (l.row, l.col));
        if let Some(c) = sheet_rels
            .list
            .iter()
            .find(|r| r.is("comments") && !r.external)
        {
            if let Some((notes, truncated)) = pkg.with_entry(&c.target, other, |r| {
                crate::sheet_v03::parse_comments(&mut XmlReader::new(r, xl))
            })? {
                if truncated > 0 {
                    budget.warn(format!(
                        "Comments are kept up to {} KiB of text; longer ones were truncated.",
                        crate::sheet_v03::MAX_NOTE_TEXT / 1024
                    ));
                }
                sheet.notes = notes;
            }
        }
        // Charts and pictures of the sheet's drawing part (W5-1).
        if let Some(rid) = &extras.drawing_rid {
            if let Some(d) = sheet_rels
                .list
                .iter()
                .find(|r| r.id == *rid && !r.external && r.is("drawing"))
            {
                crate::drawing_read::attach(
                    &mut pkg,
                    &d.target,
                    &mut sheet,
                    sheet_id,
                    &names,
                    &mut budget,
                    &mut objects,
                    xl,
                    other,
                )?;
            }
        }
        for r in sheet_rels
            .list
            .iter()
            .filter(|r| r.is("pivotTable") && !r.external)
        {
            if pivot_parts.len() < 10_000 {
                pivot_parts.push((i, r.target.clone()));
            }
        }
        sheet.name = entry.name.clone();
        sheet.hidden = entry.hidden;
        sheets.push(sheet);
    }
    // Wave 5: pivot table definitions (the output cells are in the sheet).
    for (tab, part) in pivot_parts {
        let cache_target = pkg
            .with_entry(&rels_path(&part), other, |r| {
                parse_rels(&mut XmlReader::new(r, xl), part_dir(&part))
            })?
            .and_then(|rels| {
                rels.list
                    .iter()
                    .find(|r| r.is("pivotCacheDefinition"))
                    .map(|r| r.target.clone())
            });
        let cache = match cache_target {
            Some(t) => pkg
                .with_entry(&t, other, |r| {
                    crate::tools::parse_cache(&mut XmlReader::new(r, xl))
                })?
                .unwrap_or_default(),
            None => Default::default(),
        };
        let sheet_names: Vec<String> = sheets.iter().map(|s| s.name.clone()).collect();
        let sheet_index =
            |n: &str| -> Option<u32> { sheet_names.iter().position(|x| x == n).map(|i| i as u32) };
        let db_range = |n: &str| -> Option<(u32, tally_model::CellRangeLocal)> {
            sheets.iter().enumerate().find_map(|(i, s)| {
                s.database_ranges
                    .iter()
                    .find(|d| d.name.eq_ignore_ascii_case(n))
                    .map(|d| (i as u32, d.range))
            })
        };
        let parsed = pkg.with_entry(&part, other, |r| {
            crate::tools::parse_table(&mut XmlReader::new(r, xl), &cache, &sheet_index, &db_range)
        })?;
        if let (Some(Some((p, compact))), Some(sheet)) = (parsed, sheets.get_mut(tab)) {
            if compact {
                budget.warn(format!(
                    "Pivot table {:?}: the compact form is shown as outline layout.",
                    p.name
                ));
            }
            if sheet.pivots.len() < 10_000 {
                sheet.pivots.push(p);
            }
        }
    }
    if objects.skipped > 0 {
        budget.warn(format!(
            "{} drawing object(s) that are not charts or pictures were skipped.",
            objects.skipped
        ));
    }
    if objects.series_cut {
        budget.warn(format!(
            "Charts past {} series per document were read without their extra series.",
            tally_model::MAX_SERIES_PER_DOCUMENT
        ));
    }
    if objects.literal_dropped {
        budget.warn(format!(
            "Chart cached values past {} points per document were not read.",
            tally_model::MAX_LITERAL_POINTS
        ));
    }
    if objects.capped {
        budget.warn(format!(
            "Only the first {} charts and {} pictures per sheet (256 MiB in all) were read.",
            crate::drawing_read::MAX_CHARTS,
            crate::drawing_read::MAX_IMAGES_PER_SHEET
        ));
    }
    if sheets.is_empty() {
        sheets.push(SheetData {
            name: "Sheet1".into(),
            ..SheetData::default()
        });
    }

    let mut wb = WorkbookData {
        sheets,
        styles: styles.styles,
        warnings: std::mem::take(&mut budget.warnings),
        ..WorkbookData::default()
    };
    wb.names = defined_names(&info.names, &resolver, &mut wb.sheets);
    wb.cell_styles = cell_styles;
    wb.settings.structure_protection = info.structure_protection.clone();
    wb.settings.default_font.clone_from(&styles.default_font);
    if info.date1904 {
        wb.settings.calc_settings.date_epoch = DateEpoch::D1904;
    }
    if let Some((steps, min_change)) = info.iterate {
        wb.settings.calc_settings.iterations = Some(IterSettings { steps, min_change });
    }
    wb.settings.view.sheet_tabs = !info.hide_sheet_tabs;
    wb.settings.view.outline_symbols = !hide_outline_symbols;
    if (info.active_tab as usize) < wb.sheets.len() {
        wb.settings.active_sheet = info.active_tab;
    }
    for (kind, default) in [
        ("core-properties", "docProps/core.xml"),
        ("extended-properties", "docProps/app.xml"),
    ] {
        let path = root_rels
            .iter()
            .find(|r| r.is(kind))
            .map_or_else(|| default.to_string(), |r| r.target.clone());
        pkg.with_entry(&path, other, |r| {
            parse_doc_props(&mut XmlReader::new(r, xl), &mut wb.meta)
        })?;
    }
    Ok(wb)
}

/// `definedName`s → model names. `_xlnm.Print_Area` becomes the sheet's
/// print range when it is a single area (otherwise the sheet-local name
/// `Print_Area`), `_xlnm.Print_Titles` its repeated rows/columns; other
/// built-in `_xlnm.*` names and untranslatable names are dropped.
/// `Sheet1!$A$1:$C$9` (one area) → its range.
fn single_area(text: &str) -> Option<tally_model::CellRangeLocal> {
    if text.contains(',') {
        return None;
    }
    let body = text.rsplit_once('!').map_or(text, |(_, r)| r);
    crate::sheet::parse_range(&body.replace('$', ""))
}

fn defined_names(
    raw: &[crate::workbook::DefinedName],
    resolver: &XlsxResolver,
    sheets: &mut [SheetData],
) -> Vec<NameData> {
    let sheet_count = sheets.len();
    let mut out = Vec::new();
    for n in raw {
        let local = n.local_sheet.filter(|s| (*s as usize) < sheet_count);
        let name = match n.name.strip_prefix("_xlnm.") {
            Some("Print_Area") => {
                if let (Some(i), Some(r)) = (local, single_area(n.text.trim())) {
                    let p = sheets[i as usize]
                        .page_setup
                        .get_or_insert_with(tally_model::PageSetupData::default);
                    p.print_range = Some(r);
                    continue;
                }
                "Print_Area".to_string()
            }
            Some("Print_Titles") => {
                if let Some(i) = local {
                    let (rows, cols) = crate::sheet_v03::print_titles(n.text.trim());
                    let p = sheets[i as usize]
                        .page_setup
                        .get_or_insert_with(tally_model::PageSetupData::default);
                    p.repeat_rows = rows.or(p.repeat_rows);
                    p.repeat_cols = cols.or(p.repeat_cols);
                }
                continue;
            }
            Some(_) => continue,
            None => n.name.clone(),
        };
        if name.is_empty() {
            continue;
        }
        let sheet = n.local_sheet.filter(|s| (*s as usize) < sheet_count);
        let origin = CellAddr::new(sheet.unwrap_or(0), 0, 0);
        let Some(formula) = excel_name_to_of(n.text.trim(), origin, resolver) else {
            continue;
        };
        out.push(NameData {
            base_cell: None,
            usage: Default::default(),
            name,
            sheet,
            formula,
        });
    }
    out
}
