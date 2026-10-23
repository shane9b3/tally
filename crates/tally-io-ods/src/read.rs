//! `read_ods`: package → [`WorkbookData`].

use std::collections::HashMap;
use std::io::{BufRead, Read, Seek};

use tally_model::{DocMeta, WorkbookData};

use crate::archive::Package;
use crate::content::parse_content;
use crate::styles::{Origin, StyleSheet};
use crate::xml::{Ev, XmlLimits, XmlReader};
use crate::{Limits, OdsError, MIMETYPE};

const MAX_META_TEXT: usize = 64 * 1024;

pub(crate) fn read(reader: impl Read + Seek, limits: &Limits) -> Result<WorkbookData, OdsError> {
    read_with(reader, limits, false)
}

fn xml_reader(r: &mut dyn BufRead, xl: XmlLimits, legacy: bool) -> XmlReader<&mut dyn BufRead> {
    if legacy {
        XmlReader::new_legacy(r, xl)
    } else {
        XmlReader::new(r, xl)
    }
}

/// [`read`] with `legacy` selecting the OOo 1.x (`.sxc`) mapping.
pub(crate) fn read_with(
    reader: impl Read + Seek,
    limits: &Limits,
    legacy: bool,
) -> Result<WorkbookData, OdsError> {
    let _attr_budget = crate::xml::AttrBudget::install(limits.max_attr_bytes);
    let mut pkg = Package::open(reader, limits)?;
    if !pkg.has("content.xml") {
        return Err(OdsError::NotOds);
    }
    if let Some(mt) = pkg.read_small("mimetype", 256)? {
        let mt = String::from_utf8_lossy(&mt);
        let mt = mt.trim();
        let ok = if legacy {
            mt.starts_with(crate::sxc::MIMETYPE)
        } else {
            mt.starts_with(MIMETYPE)
        };
        if !ok {
            return Err(OdsError::NotOds);
        }
    }

    let xl = XmlLimits {
        max_depth: limits.max_xml_depth,
        max_attrs: limits.max_attrs_per_element,
    };
    let other = limits.max_entry_bytes;

    let manifest = pkg.with_entry("META-INF/manifest.xml", other, |r| {
        parse_manifest(&mut xml_reader(r, xl, legacy))
    })?;
    if manifest.as_ref().is_some_and(|m| m.content_encrypted) {
        return Err(OdsError::Unsupported(
            "encrypted (password-protected) documents".into(),
        ));
    }

    let mut styles = StyleSheet::default();
    pkg.with_entry("styles.xml", other, |r| {
        parse_styles_xml(&mut xml_reader(r, xl, legacy), &mut styles)
    })?;
    let mut content = pkg
        .with_entry("content.xml", limits.max_content_xml_bytes, |r| {
            parse_content(&mut xml_reader(r, xl, legacy), &mut styles, limits)
        })?
        .ok_or(OdsError::NotOds)?;
    if !content.is_spreadsheet {
        return Err(OdsError::NotOds);
    }
    let settings = pkg
        .with_entry("settings.xml", other, |r| {
            parse_settings(&mut XmlReader::new(r, xl))
        })?
        .unwrap_or_default();
    let meta = pkg
        .with_entry("meta.xml", other, |r| {
            parse_meta(&mut XmlReader::new(r, xl))
        })?
        .unwrap_or_default();

    let frames = std::mem::take(&mut content.frames);
    let mut sheets = content.sheets;
    for (sheet, extra) in sheets.iter_mut().zip(&content.extras) {
        let name = extra.master_page.as_deref().unwrap_or("Default");
        let mut p = match styles.master_pages.get(name) {
            Some(m) => crate::v03::page_setup_of(m, styles.page_layout(&m.layout)),
            None => tally_model::PageSetupData::default(),
        };
        p.print_range = extra.print_range;
        p.repeat_rows = extra.repeat_rows;
        p.repeat_cols = extra.repeat_cols;
        if p != tally_model::PageSetupData::default() {
            sheet.page_setup = Some(p);
        }
        if sheet.conditional_formats.is_empty() {
            legacy_maps(sheet, &styles);
        }
    }
    let locale = styles
        .default_locale
        .clone()
        .filter(|l| !l.eq_ignore_ascii_case("en-US"));
    let named_styles = styles.named_styles();
    let default_font = styles.default_font();
    let mut wb = WorkbookData {
        sheets,
        styles: styles.styles,
        names: content.names,
        meta,
        warnings: {
            let mut w = std::mem::take(&mut styles.warnings);
            w.extend(content.warnings);
            w
        },
        ..WorkbookData::default()
    };
    wb.settings.locale = locale;
    wb.settings.default_font = default_font;
    wb.settings.structure_protection = content.structure_protection;
    if let Some(calc) = content.calc {
        wb.settings.calc_settings = calc;
    }
    if let Some(active) = settings.active {
        if let Some(i) = wb.sheets.iter().position(|s| s.name == active) {
            wb.settings.active_sheet = i as u32;
        }
    }
    for sheet in &mut wb.sheets {
        if let Some(&(rows, cols)) = settings.frozen.get(&sheet.name) {
            if rows > 0 || cols > 0 {
                sheet.frozen = Some((rows, cols));
            }
        }
        if let Some(items) = settings.views.get(&sheet.name) {
            for (k, v) in items {
                crate::tools::apply_view_item(sheet, k, v);
            }
        }
        if sheet.frozen.is_none() {
            if let Some(split) = settings.splits.get(&sheet.name) {
                sheet.split = Some(*split);
            }
        }
    }
    // Wave 5 (W5-2): named cell styles (`office:styles`).
    wb.cell_styles = named_styles;
    // Wave 5 (W5-1): chart objects and pictures the frames refer to.
    if !frames.is_empty() {
        let media = manifest.map(|m| m.media).unwrap_or_default();
        crate::frame::attach_objects(&mut pkg, &mut wb, frames, &media, limits)?;
    }
    Ok(wb)
}

#[derive(Debug, Default)]
struct Manifest {
    content_encrypted: bool,
    /// `full-path` → `media-type` (for the chart objects and pictures).
    media: HashMap<String, String>,
}

fn parse_manifest<R: BufRead>(xml: &mut XmlReader<R>) -> Result<Manifest, OdsError> {
    let mut m = Manifest::default();
    let mut current: Option<String> = None;
    loop {
        match xml.next()? {
            Ev::Start(e) => match e.name.as_str() {
                "manifest:file-entry" => {
                    current = e.attr("manifest:full-path").map(str::to_string);
                    if let (Some(p), Some(t)) = (current.as_ref(), e.attr("manifest:media-type")) {
                        if m.media.len() < 20_000 && p.len() <= 256 {
                            m.media.insert(p.clone(), t.chars().take(128).collect());
                        }
                    }
                }
                "manifest:encryption-data" => {
                    if current.as_deref() == Some("content.xml") {
                        m.content_encrypted = true;
                    }
                    xml.skip()?;
                }
                _ => {}
            },
            Ev::Eof => return Ok(m),
            _ => {}
        }
    }
}

fn parse_styles_xml<R: BufRead>(
    xml: &mut XmlReader<R>,
    styles: &mut StyleSheet,
) -> Result<(), OdsError> {
    loop {
        match xml.next()? {
            Ev::Start(e) => match e.name.as_str() {
                "office:document-styles" => {}
                "office:font-face-decls" => styles.parse_font_faces(xml)?,
                "office:styles" | "office:automatic-styles" => {
                    styles.parse_block(xml, Origin::Styles)?;
                }
                "office:master-styles" => {
                    styles.master_pages = crate::v03::parse_master_styles(xml)?;
                }
                _ => xml.skip()?,
            },
            Ev::Eof => return Ok(()),
            _ => {}
        }
    }
}

#[derive(Debug, Default)]
struct Settings {
    active: Option<String>,
    /// Sheet name → (frozen rows, frozen cols).
    frozen: HashMap<String, (u32, u32)>,
    /// Sheet name → view option items (wave 5).
    views: HashMap<String, Vec<(String, String)>>,
    /// Sheet name → Window ▸ Split position in 1/100 mm (`SplitMode` 1,
    /// whose positions are pixels at 100 %).
    splits: HashMap<String, tally_model::SplitData>,
}

#[derive(Default)]
struct SplitInfo {
    h_mode: i64,
    h_pos: u32,
    v_mode: i64,
    v_pos: u32,
}

fn parse_settings<R: BufRead>(xml: &mut XmlReader<R>) -> Result<Settings, OdsError> {
    let mut s = Settings::default();
    // config:name of each open set / map / entry element (None for others).
    let mut stack: Vec<Option<String>> = Vec::new();
    let mut splits: HashMap<String, SplitInfo> = HashMap::new();
    loop {
        match xml.next()? {
            Ev::Start(e) => {
                if e.name == "config:config-item" {
                    let name = e.attr("config:name").unwrap_or_default().to_string();
                    let text = xml.text_content(MAX_META_TEXT)?;
                    let in_view = stack
                        .iter()
                        .flatten()
                        .next()
                        .is_some_and(|n| n == "ooo:view-settings");
                    if !in_view {
                        continue;
                    }
                    let depth = stack.len();
                    let in_tables = depth >= 2 && stack[depth - 2].as_deref() == Some("Tables");
                    if in_tables {
                        if let Some(Some(sheet)) = stack.last() {
                            let info = splits.entry(sheet.clone()).or_default();
                            let t = text.trim();
                            match name.as_str() {
                                "HorizontalSplitMode" => info.h_mode = t.parse().unwrap_or(0),
                                "HorizontalSplitPosition" => info.h_pos = t.parse().unwrap_or(0),
                                "VerticalSplitMode" => info.v_mode = t.parse().unwrap_or(0),
                                "VerticalSplitPosition" => info.v_pos = t.parse().unwrap_or(0),
                                "ShowFormulas"
                                | "ShowZeroValues"
                                | "ShowGrid"
                                | "HasColumnRowHeaders"
                                | "IsValueHighlightingEnabled" => {
                                    let v = s.views.entry(sheet.clone()).or_default();
                                    if v.len() < 16 {
                                        v.push((name.clone(), t.to_string()));
                                    }
                                }
                                _ => {}
                            }
                        }
                    } else if name == "ActiveTable" {
                        s.active = Some(text);
                    }
                    continue;
                }
                stack.push(e.attr("config:name").map(str::to_string));
            }
            Ev::End => {
                stack.pop();
            }
            Ev::Eof => break,
            _ => {}
        }
    }
    for (sheet, info) in splits {
        let cols = if info.h_mode == 2 { info.h_pos } else { 0 };
        let rows = if info.v_mode == 2 { info.v_pos } else { 0 };
        if info.h_mode == 1 || info.v_mode == 1 {
            let hmm = |mode: i64, px: u32| {
                if mode == 1 {
                    crate::tools::split_px_to_hmm(px)
                } else {
                    0
                }
            };
            let split = tally_model::SplitData {
                x_hmm: hmm(info.h_mode, info.h_pos),
                y_hmm: hmm(info.v_mode, info.v_pos),
            };
            if split.x_hmm > 0 || split.y_hmm > 0 {
                s.splits.insert(sheet.clone(), split);
            }
        }
        s.frozen.insert(sheet, (rows, cols));
    }
    Ok(s)
}

/// Turns the `style:map`s of the cell styles used on a sheet (OpenOffice's
/// conditional formatting) into conditional formats over those cells.
fn legacy_maps(sheet: &mut tally_model::SheetData, styles: &StyleSheet) {
    if styles.style_maps.is_empty() {
        return;
    }
    let mut cells: HashMap<u32, Vec<(u32, u32, u32)>> = HashMap::new();
    for c in &sheet.cells {
        if let Some(st) = c.style.filter(|s| styles.style_maps.contains_key(s)) {
            cells.entry(st).or_default().push((c.col, c.row, c.row));
        }
    }
    for r in &sheet.style_runs {
        if styles.style_maps.contains_key(&r.style) {
            cells
                .entry(r.style)
                .or_default()
                .push((r.col, r.start_row, r.end_row));
        }
    }
    let mut keys: Vec<u32> = cells.keys().copied().collect();
    keys.sort_unstable();
    for st in keys {
        let Some(maps) = styles.style_maps.get(&st) else {
            continue;
        };
        let rects: Vec<tally_model::CellRangeLocal> = cells[&st]
            .iter()
            .map(|&(c, r0, r1)| tally_model::CellRangeLocal {
                r0,
                c0: c,
                r1,
                c1: c,
            })
            .collect();
        let ranges = crate::v03::merge_rects(rects);
        let Some(first) = ranges.first().copied() else {
            continue;
        };
        let mut base = None;
        let rules: Vec<tally_model::CfRuleData> = maps
            .iter()
            .filter_map(|m| {
                base = base.or(m.base);
                // AOO applies the first matching entry only
                // (`ScConditionalFormat::GetCellStyle`, review 3, H1).
                crate::v03::parse_map_condition(&m.condition).map(|kind| tally_model::CfRuleData {
                    kind,
                    style: styles.cf_style(&m.apply_style),
                    stop_if_true: true,
                })
            })
            .collect();
        if !rules.is_empty() {
            sheet
                .conditional_formats
                .push(tally_model::ConditionalFormatData {
                    ranges,
                    base: base.unwrap_or((first.r0, first.c0)),
                    rules,
                });
        }
    }
}

fn parse_meta<R: BufRead>(xml: &mut XmlReader<R>) -> Result<DocMeta, OdsError> {
    let mut m = DocMeta::default();
    loop {
        match xml.next()? {
            Ev::Start(e) => {
                let slot = match e.name.as_str() {
                    "meta:keyword" => {
                        let k = xml.text_content(MAX_META_TEXT)?;
                        if !k.trim().is_empty() && m.keywords.len() < 1000 {
                            m.keywords.push(k.trim().to_string());
                        }
                        continue;
                    }
                    "meta:generator" => &mut m.generator,
                    "dc:title" => &mut m.title,
                    "dc:subject" => &mut m.subject,
                    "dc:description" => &mut m.description,
                    "meta:initial-creator" => &mut m.author,
                    "dc:creator" => &mut m.last_author,
                    "meta:creation-date" => &mut m.created,
                    "dc:date" => &mut m.modified,
                    "office:document-meta" | "office:meta" => continue,
                    _ => {
                        xml.skip()?;
                        continue;
                    }
                };
                *slot = Some(xml.text_content(MAX_META_TEXT)?);
            }
            Ev::Eof => return Ok(m),
            _ => {}
        }
    }
}
