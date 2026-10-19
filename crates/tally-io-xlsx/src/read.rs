//! `read_xlsx`: package → [`WorkbookData`].

use std::io::{Read, Seek};
use std::sync::Arc;

use tally_model::{NameData, SheetData, WorkbookData};
use tally_types::{CellAddr, DateEpoch, IterSettings};

use crate::archive::Package;
use crate::formula::{excel_name_to_of, XlsxResolver};
use crate::sheet::{parse_sheet, Budget, SheetCtx};
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
        .unwrap_or_default();
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
        .unwrap_or_default();
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
    let styles_path = rel_target("styles", "styles.xml");
    let styles: StyleTable = pkg
        .with_entry(&styles_path, other, |r| {
            parse_styles(&mut XmlReader::new(r, xl))
        })?
        .unwrap_or_default();

    let names: Vec<String> = info.sheets.iter().map(|s| s.name.clone()).collect();
    let resolver = XlsxResolver::new(&names);
    let mut budget = Budget::default();
    let mut sheets = Vec::with_capacity(info.sheets.len());
    for (i, entry) in info.sheets.iter().enumerate() {
        let sheet_id = u32::try_from(i).unwrap_or(u32::MAX);
        let target = rels
            .iter()
            .find(|r| r.id == entry.rid)
            .map(|r| r.target.clone());
        let ctx = SheetCtx {
            sheet: sheet_id,
            sst: &sst,
            styles: &styles,
            date1904: info.date1904,
            resolver: &resolver,
            limits,
        };
        let parsed = match target {
            Some(t) => pkg.with_entry(&t, big, |r| {
                parse_sheet(&mut XmlReader::new(r, xl), &ctx, &mut budget)
            })?,
            None => None,
        };
        // A sheet whose part is missing (or a chartsheet) opens empty.
        let mut sheet = parsed.unwrap_or_default();
        sheet.name = entry.name.clone();
        sheet.hidden = entry.hidden;
        sheets.push(sheet);
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
        ..WorkbookData::default()
    };
    wb.names = defined_names(&info.names, &resolver, wb.sheets.len());
    if info.date1904 {
        wb.settings.calc_settings.date_epoch = DateEpoch::D1904;
    }
    if let Some((steps, min_change)) = info.iterate {
        wb.settings.calc_settings.iterations = Some(IterSettings { steps, min_change });
    }
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

/// `definedName`s → model names. Built-in `_xlnm.*` names are dropped
/// except `_xlnm.Print_Area` (kept as `Print_Area`, sheet-local);
/// untranslatable names are dropped.
fn defined_names(
    raw: &[crate::workbook::DefinedName],
    resolver: &XlsxResolver,
    sheet_count: usize,
) -> Vec<NameData> {
    let mut out = Vec::new();
    for n in raw {
        let name = match n.name.strip_prefix("_xlnm.") {
            Some("Print_Area") => "Print_Area".to_string(),
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
            name,
            sheet,
            formula,
        });
    }
    out
}
