//! `write_ods`: [`WorkbookData`] → ODS package.
//!
//! Entry order: `mimetype` (stored, first), `META-INF/manifest.xml`,
//! `content.xml`, `styles.xml`, `meta.xml`, `settings.xml`. Entry names are
//! fixed constants (§5.4 requirement 4).

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::io::{Seek, Write};

use tally_model::{
    AutoFilterData, CellContentData, CellData, CellRangeLocal, DimRun, FilterOpData, HAlign,
    NameData, SheetData, StyleData, StyleRun, VAlign, WorkbookData,
};
use tally_types::{col_to_letters, CriteriaMode, Value, MAX_COLS, MAX_ROWS};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use crate::numfmt::code_to_xml;
use crate::units::{
    border_to_string, color_hex, fmt_f64, general_display, hmm_to_cm, null_date_for,
};
use crate::xml::{escape_attr, escape_text};
use crate::{OdsError, WriteReport, DEFAULT_COL_WIDTH_HMM, DEFAULT_ROW_HEIGHT_HMM, MIMETYPE};

/// Flush the in-memory content.xml buffer to the zip in chunks this big.
const FLUSH_AT: usize = 1 << 20;

const NS: &str = concat!(
    r#" xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0""#,
    r#" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0""#,
    r#" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0""#,
    r#" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0""#,
    r#" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0""#,
    r#" xmlns:number="urn:oasis:names:tc:opendocument:xmlns:datastyle:1.0""#,
    r#" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0""#,
    r#" xmlns:of="urn:oasis:names:tc:opendocument:xmlns:of:1.2""#,
    r#" xmlns:xlink="http://www.w3.org/1999/xlink""#,
    r#" xmlns:dc="http://purl.org/dc/elements/1.1/""#,
    r#" xmlns:meta="urn:oasis:names:tc:opendocument:xmlns:meta:1.0""#,
    r#" xmlns:config="urn:oasis:names:tc:opendocument:xmlns:config:1.0""#,
    r#" xmlns:tableooo="http://openoffice.org/2009/table""#,
    r#" xmlns:calcext="urn:org:documentfoundation:names:experimental:calc:xmlns:calcext:1.0""#,
    r#" xmlns:loext="urn:org:documentfoundation:names:experimental:office:xmlns:loext:1.0""#,
    r#" xmlns:tally="urn:tally:names:experimental:1.0""#,
);

const XML_DECL: &str = r#"<?xml version="1.0" encoding="UTF-8"?>"#;

pub(crate) fn write<W: Write + Seek>(
    data: &WorkbookData,
    writer: W,
) -> Result<WriteReport, OdsError> {
    let mut report = WriteReport::default();
    let mut zip = ZipWriter::new(writer);
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    zip.start_file("mimetype", stored)?;
    zip.write_all(MIMETYPE.as_bytes())?;

    zip.start_file("META-INF/manifest.xml", deflated)?;
    zip.write_all(manifest().as_bytes())?;

    let plan = StylePlan::new(data, &mut report);
    zip.start_file("content.xml", deflated)?;
    write_content(data, &plan, &mut zip, &mut report)?;

    zip.start_file("styles.xml", deflated)?;
    zip.write_all(styles_xml(data, &plan).as_bytes())?;

    zip.start_file("meta.xml", deflated)?;
    zip.write_all(meta_xml(data).as_bytes())?;

    zip.start_file("settings.xml", deflated)?;
    zip.write_all(settings_xml(data).as_bytes())?;

    zip.finish()?;
    Ok(report)
}

fn manifest() -> String {
    let mut s = String::new();
    s.push_str(XML_DECL);
    s.push_str(r#"<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.2">"#);
    let _ = write!(
        s,
        r#"<manifest:file-entry manifest:full-path="/" manifest:version="1.2" manifest:media-type="{MIMETYPE}"/>"#
    );
    for f in ["content.xml", "styles.xml", "meta.xml", "settings.xml"] {
        let _ = write!(
            s,
            r#"<manifest:file-entry manifest:full-path="{f}" manifest:media-type="text/xml"/>"#
        );
    }
    s.push_str("</manifest:manifest>");
    s
}

fn styles_xml(data: &WorkbookData, plan: &StylePlan) -> String {
    let mut s = String::new();
    s.push_str(XML_DECL);
    let _ = write!(s, r#"<office:document-styles{NS} office:version="1.2">"#);
    s.push_str("<office:styles>");
    s.push_str(r#"<style:default-style style:family="table-cell"><style:paragraph-properties style:tab-stop-distance="1.25cm"/>"#);
    if let Some(tag) = &data.settings.locale {
        let mut parts = tag.splitn(2, '-');
        let lang = parts.next().unwrap_or("en");
        let country = parts.next();
        s.push_str(r#"<style:text-properties fo:language=""#);
        escape_attr(lang, &mut s);
        s.push('"');
        if let Some(c) = country {
            s.push_str(r#" fo:country=""#);
            escape_attr(c, &mut s);
            s.push('"');
        }
        s.push_str("/>");
    }
    s.push_str("</style:default-style>");
    s.push_str(r#"<style:style style:name="Default" style:family="table-cell"/>"#);
    s.push_str(&plan.cf_data_xml);
    for (st, name, ds) in &plan.cf_styles {
        crate::v03::write_cf_style(&mut s, name, st, ds.as_deref());
    }
    s.push_str("</office:styles><office:automatic-styles>");
    for (i, (p, _)) in plan.pages.iter().enumerate() {
        crate::v03::write_page_layout(&mut s, &format!("Mpm{}", i + 1), p);
    }
    s.push_str("</office:automatic-styles><office:master-styles>");
    for (i, (p, name)) in plan.pages.iter().enumerate() {
        crate::v03::write_master_page(&mut s, name, &format!("Mpm{}", i + 1), p);
    }
    s.push_str("</office:master-styles></office:document-styles>");
    s
}

fn meta_xml(data: &WorkbookData) -> String {
    let mut s = String::new();
    s.push_str(XML_DECL);
    let _ = write!(
        s,
        r#"<office:document-meta{NS} office:version="1.2"><office:meta><meta:generator>Tally/0.1</meta:generator>"#
    );
    let m = &data.meta;
    for k in &m.keywords {
        s.push_str("<meta:keyword>");
        escape_text(k, &mut s);
        s.push_str("</meta:keyword>");
    }
    for (tag, v) in [
        ("dc:title", &m.title),
        ("dc:subject", &m.subject),
        ("dc:description", &m.description),
        ("meta:initial-creator", &m.author),
        ("dc:creator", &m.last_author),
        ("meta:creation-date", &m.created),
        ("dc:date", &m.modified),
    ] {
        if let Some(v) = v {
            let _ = write!(s, "<{tag}>");
            escape_text(v, &mut s);
            let _ = write!(s, "</{tag}>");
        }
    }
    s.push_str("</office:meta></office:document-meta>");
    s
}

fn config_item(s: &mut String, name: &str, ty: &str, value: &str) {
    let _ = write!(
        s,
        r#"<config:config-item config:name="{name}" config:type="{ty}">"#
    );
    escape_text(value, s);
    s.push_str("</config:config-item>");
}

fn settings_xml(data: &WorkbookData) -> String {
    let mut s = String::new();
    s.push_str(XML_DECL);
    let _ = write!(
        s,
        r#"<office:document-settings{NS} office:version="1.2"><office:settings>"#
    );
    s.push_str(r#"<config:config-item-set config:name="ooo:view-settings">"#);
    s.push_str(
        r#"<config:config-item-map-indexed config:name="Views"><config:config-item-map-entry>"#,
    );
    config_item(&mut s, "ViewId", "string", "view1");
    s.push_str(r#"<config:config-item-map-named config:name="Tables">"#);
    for sheet in &data.sheets {
        s.push_str(r#"<config:config-item-map-entry config:name=""#);
        escape_attr(&sheet.name, &mut s);
        s.push_str("\">");
        let (rows, cols) = sheet.frozen.unwrap_or((0, 0));
        let mode = |n: u32| if n > 0 { "2" } else { "0" };
        config_item(&mut s, "HorizontalSplitMode", "short", mode(cols));
        config_item(&mut s, "VerticalSplitMode", "short", mode(rows));
        config_item(&mut s, "HorizontalSplitPosition", "int", &cols.to_string());
        config_item(&mut s, "VerticalSplitPosition", "int", &rows.to_string());
        if sheet.frozen.is_some() {
            config_item(&mut s, "ActiveSplitRange", "short", "2");
            config_item(&mut s, "PositionRight", "int", &cols.to_string());
            config_item(&mut s, "PositionBottom", "int", &rows.to_string());
        }
        s.push_str("</config:config-item-map-entry>");
    }
    s.push_str("</config:config-item-map-named>");
    if let Some(active) = data.sheets.get(data.settings.active_sheet as usize) {
        config_item(&mut s, "ActiveTable", "string", &active.name);
    }
    s.push_str(
        "</config:config-item-map-entry></config:config-item-map-indexed></config:config-item-set>",
    );
    s.push_str("</office:settings></office:document-settings>");
    s
}

/// The page-style part of a setup (what a master page holds).
fn page_style_key(p: &tally_model::PageSetupData) -> tally_model::PageSetupData {
    tally_model::PageSetupData {
        print_range: None,
        repeat_rows: None,
        repeat_cols: None,
        ..p.clone()
    }
}

/// (hidden, tab colour, master page) of a table style.
type TableKey = (bool, Option<u32>, String);

/// An interval of one column: (first row, last row, style, validation).
type ColInterval = (u32, u32, Option<u32>, Option<usize>);

/// Names of the automatic styles we will emit.
struct StylePlan {
    /// data.styles index → (cell style name, XML).
    cell_xml: String,
    data_xml: String,
    fonts: Vec<String>,
    /// (width, break before) → name.
    col_styles: Vec<((u32, bool), String)>,
    /// (height, break before) → name.
    row_styles: Vec<((u32, bool), String)>,
    /// (hidden, tab colour, master page) → name.
    table_styles: Vec<(TableKey, String)>,
    /// Conditional styles: (style, name, data style).
    cf_styles: Vec<(tally_model::CfStyleData, String, Option<String>)>,
    /// Data styles of the conditional styles (styles.xml).
    cf_data_xml: String,
    /// Page styles: (setup, master page name); the first is "Default".
    pages: Vec<(tally_model::PageSetupData, String)>,
    /// (tab, row, col) → derived cell style with `style:map`s.
    derived: HashMap<(usize, u32, u32), String>,
}

impl StylePlan {
    fn new(data: &WorkbookData, report: &mut WriteReport) -> Self {
        let mut plan = StylePlan {
            cell_xml: String::new(),
            data_xml: String::new(),
            fonts: Vec::new(),
            col_styles: vec![((DEFAULT_COL_WIDTH_HMM, false), "co1".into())],
            row_styles: vec![((DEFAULT_ROW_HEIGHT_HMM, false), "ro1".into())],
            table_styles: Vec::new(),
            cf_styles: Vec::new(),
            cf_data_xml: String::new(),
            pages: vec![(tally_model::PageSetupData::default(), "Default".into())],
            derived: HashMap::new(),
        };
        let mut codes: HashMap<&str, Option<String>> = HashMap::new();
        let mut warned_display = false;
        let mut warned_indent = false;
        let mut data_names: Vec<Option<String>> = Vec::with_capacity(data.styles.len());
        for (i, st) in data.styles.iter().enumerate() {
            let data_name = match st.number_format.as_deref() {
                None => None,
                Some(code) if code.eq_ignore_ascii_case("General") || code.is_empty() => None,
                Some(code) => {
                    let next = codes.len();
                    codes
                        .entry(code)
                        .or_insert_with(|| {
                            let name = format!("N{next}");
                            let mut xml = String::new();
                            match code_to_xml(code, &name, &mut xml) {
                                Ok(()) => {
                                    plan.data_xml.push_str(&xml);
                                    Some(name)
                                }
                                Err(why) => {
                                    report.warnings.push(format!(
                                        "number format {code:?} written as General: {why}"
                                    ));
                                    None
                                }
                            }
                        })
                        .clone()
                }
            };
            if st.display.is_some() && !warned_display {
                warned_display = true;
                report
                    .warnings
                    .push("chip display styles are not written in this version".into());
            }
            if st.align.indent > 0 && !warned_indent {
                warned_indent = true;
                report
                    .warnings
                    .push("indent levels are not written in this version".into());
            }
            if let Some(f) = &st.font.family {
                if !plan.fonts.contains(f) {
                    plan.fonts.push(f.clone());
                }
            }
            cell_style_xml(
                &mut plan.cell_xml,
                &format!("ce{}", i + 1),
                st,
                data_name.as_deref(),
            );
            data_names.push(data_name);
        }
        let row_break = |sheet: &SheetData, r: u32| sheet.row_breaks.binary_search(&r).is_ok();
        for sheet in &data.sheets {
            for run in &sheet.col_widths {
                let key = (run.size_hmm, false);
                if !plan.col_styles.iter().any(|(w, _)| *w == key) {
                    let name = format!("co{}", plan.col_styles.len() + 1);
                    plan.col_styles.push((key, name));
                }
            }
            for &c in &sheet.col_breaks {
                let w = sheet
                    .col_widths
                    .iter()
                    .find(|r| r.start <= c && c <= r.end)
                    .map_or(DEFAULT_COL_WIDTH_HMM, |r| r.size_hmm);
                if !plan.col_styles.iter().any(|(k, _)| *k == (w, true)) {
                    let name = format!("co{}", plan.col_styles.len() + 1);
                    plan.col_styles.push(((w, true), name));
                }
            }
            for run in &sheet.row_heights {
                let key = (run.size_hmm, false);
                if !plan.row_styles.iter().any(|(h, _)| *h == key) {
                    let name = format!("ro{}", plan.row_styles.len() + 1);
                    plan.row_styles.push((key, name));
                }
            }
            for &r in &sheet.row_breaks {
                debug_assert!(row_break(sheet, r));
                let h = sheet
                    .row_heights
                    .iter()
                    .find(|x| x.start <= r && r <= x.end)
                    .map_or(DEFAULT_ROW_HEIGHT_HMM, |x| x.size_hmm);
                if !plan.row_styles.iter().any(|(k, _)| *k == (h, true)) {
                    let name = format!("ro{}", plan.row_styles.len() + 1);
                    plan.row_styles.push(((h, true), name));
                }
            }
            let master = plan.master_for(sheet);
            let key = (sheet.hidden, sheet.tab_color, master);
            if !plan.table_styles.iter().any(|(k, _)| *k == key) {
                let name = format!("ta{}", plan.table_styles.len() + 1);
                plan.table_styles.push((key, name));
            }
            for cf in &sheet.conditional_formats {
                for rule in &cf.rules {
                    plan.cf_style_name(&rule.style, report);
                }
            }
        }
        plan.plan_derived(data, &data_names);
        plan
    }

    /// The master page of a sheet (registering a new page style).
    fn master_for(&mut self, sheet: &SheetData) -> String {
        let key = sheet
            .page_setup
            .as_ref()
            .map_or_else(tally_model::PageSetupData::default, page_style_key);
        if let Some((_, n)) = self.pages.iter().find(|(p, _)| *p == key) {
            return n.clone();
        }
        let name = format!("PageStyle_{}", self.pages.len());
        self.pages.push((key, name.clone()));
        name
    }

    /// The named style for a conditional style (registered on first use).
    fn cf_style_name(&mut self, st: &tally_model::CfStyleData, report: &mut WriteReport) -> String {
        if let Some((_, n, _)) = self.cf_styles.iter().find(|(x, _, _)| x == st) {
            return n.clone();
        }
        let i = self.cf_styles.len() + 1;
        let name = format!("Tally_CF_{i}");
        let ds = st.number_format.as_deref().and_then(|code| {
            let dn = format!("NCF{i}");
            let mut xml = String::new();
            match code_to_xml(code, &dn, &mut xml) {
                Ok(()) => {
                    self.cf_data_xml.push_str(&xml);
                    Some(dn)
                }
                Err(why) => {
                    report
                        .warnings
                        .push(format!("conditional number format {code:?} dropped: {why}"));
                    None
                }
            }
        });
        self.cf_styles.push((st.clone(), name.clone(), ds));
        name
    }

    fn cf_style_known(&self, st: &tally_model::CfStyleData) -> String {
        self.cf_styles
            .iter()
            .find(|(x, _, _)| x == st)
            .map_or_else(|| "Default".to_string(), |(_, n, _)| n.clone())
    }

    /// Derived cell styles carrying `style:map`s (OpenOffice's conditional
    /// formatting) for the cells in conditional formats with value or
    /// formula rules. Empty cells are not covered.
    fn plan_derived(&mut self, data: &WorkbookData, data_names: &[Option<String>]) {
        let mut by_key: HashMap<(Option<u32>, Vec<usize>), String> = HashMap::new();
        for (tab, sheet) in data.sheets.iter().enumerate() {
            let mappable: Vec<(usize, &tally_model::ConditionalFormatData)> = sheet
                .conditional_formats
                .iter()
                .enumerate()
                .filter(|(_, cf)| {
                    cf.rules
                        .iter()
                        .any(|r| crate::v03::map_condition(&r.kind).is_some())
                })
                .collect();
            if mappable.is_empty() {
                continue;
            }
            for cell in &sheet.cells {
                let ids: Vec<usize> = mappable
                    .iter()
                    .filter(|(_, cf)| {
                        cf.ranges.iter().any(|r| {
                            r.r0 <= cell.row
                                && cell.row <= r.r1
                                && r.c0 <= cell.col
                                && cell.col <= r.c1
                        })
                    })
                    .map(|(i, _)| *i)
                    .collect();
                if ids.is_empty() {
                    continue;
                }
                let base = cell.style.filter(|i| (*i as usize) < data.styles.len());
                let key = (
                    base,
                    ids.iter().map(|i| tab * 100_000 + i).collect::<Vec<_>>(),
                );
                let name = match by_key.get(&key) {
                    Some(n) => n.clone(),
                    None => {
                        let name = format!("cm{}", by_key.len() + 1);
                        let default = StyleData::default();
                        let st = base.map_or(&default, |i| &data.styles[i as usize]);
                        let ds = base.and_then(|i| data_names[i as usize].clone());
                        let mut xml = String::new();
                        cell_style_xml(&mut xml, &name, st, ds.as_deref());
                        let close = "</style:style>";
                        if xml.ends_with(close) {
                            xml.truncate(xml.len() - close.len());
                        }
                        let sheet_name = odf_sheet_name(&sheet.name);
                        for &i in &ids {
                            let cf = &sheet.conditional_formats[i];
                            for rule in &cf.rules {
                                let Some(c) = crate::v03::map_condition(&rule.kind) else {
                                    continue;
                                };
                                xml.push_str(r#"<style:map style:condition=""#);
                                escape_attr(&c, &mut xml);
                                let _ = write!(
                                    xml,
                                    r#"" style:apply-style-name="{}" style:base-cell-address=""#,
                                    self.cf_style_known(&rule.style)
                                );
                                escape_attr(
                                    &crate::v03::base_address(&sheet_name, cf.base.0, cf.base.1),
                                    &mut xml,
                                );
                                xml.push_str("\"/>");
                            }
                        }
                        xml.push_str(close);
                        self.cell_xml.push_str(&xml);
                        by_key.insert(key, name.clone());
                        name
                    }
                };
                self.derived.insert((tab, cell.row, cell.col), name);
            }
        }
    }

    fn col_style(&self, w: u32, brk: bool) -> &str {
        self.col_styles
            .iter()
            .find(|(x, _)| *x == (w, brk))
            .map_or("co1", |(_, n)| n.as_str())
    }

    fn row_style(&self, h: u32, brk: bool) -> &str {
        self.row_styles
            .iter()
            .find(|(x, _)| *x == (h, brk))
            .map_or("ro1", |(_, n)| n.as_str())
    }

    fn table_style(&self, sheet: &SheetData) -> &str {
        let master = sheet
            .page_setup
            .as_ref()
            .map_or_else(tally_model::PageSetupData::default, page_style_key);
        let master = self
            .pages
            .iter()
            .find(|(p, _)| *p == master)
            .map_or("Default", |(_, n)| n.as_str());
        self.table_styles
            .iter()
            .find(|((h, t, m), _)| *h == sheet.hidden && *t == sheet.tab_color && m == master)
            .map_or("ta1", |(_, n)| n.as_str())
    }

    fn automatic_styles(&self, out: &mut String) {
        out.push_str(&self.data_xml);
        for ((w, brk), name) in &self.col_styles {
            let _ = write!(
                out,
                r#"<style:style style:name="{name}" style:family="table-column"><style:table-column-properties fo:break-before="{}" style:column-width="{}"/></style:style>"#,
                if *brk { "page" } else { "auto" },
                hmm_to_cm(*w)
            );
        }
        for ((h, brk), name) in &self.row_styles {
            let optimal = if name == "ro1" { "true" } else { "false" };
            let _ = write!(
                out,
                r#"<style:style style:name="{name}" style:family="table-row"><style:table-row-properties style:row-height="{}" fo:break-before="{}" style:use-optimal-row-height="{optimal}"/></style:style>"#,
                hmm_to_cm(*h),
                if *brk { "page" } else { "auto" }
            );
        }
        for ((hidden, tab, master), name) in &self.table_styles {
            let _ = write!(
                out,
                r#"<style:style style:name="{name}" style:family="table" style:master-page-name="{master}"><style:table-properties table:display="{}" style:writing-mode="lr-tb""#,
                !hidden
            );
            if let Some(c) = tab {
                let _ = write!(out, r#" tableooo:tab-color="{}""#, color_hex(*c));
            }
            out.push_str("/></style:style>");
        }
        out.push_str(&self.cell_xml);
    }
}

fn cell_style_xml(out: &mut String, name: &str, st: &StyleData, data_style: Option<&str>) {
    let _ = write!(
        out,
        r#"<style:style style:name="{name}" style:family="table-cell" style:parent-style-name="Default""#
    );
    if let Some(ds) = data_style {
        let _ = write!(out, r#" style:data-style-name="{ds}""#);
    }
    out.push('>');
    let mut cell_props = String::new();
    if let Some(fill) = st.fill {
        let _ = write!(cell_props, r#" fo:background-color="{}""#, color_hex(fill));
    }
    let b = &st.borders;
    let sides = [&b.top, &b.bottom, &b.left, &b.right];
    if sides.iter().all(|x| x.is_some() && **x == b.top) {
        if let Some(l) = &b.top {
            let _ = write!(cell_props, r#" fo:border="{}""#, border_to_string(l));
        }
    } else {
        for (name, side) in ["top", "bottom", "left", "right"].iter().zip(sides) {
            if let Some(l) = side {
                let _ = write!(cell_props, r#" fo:border-{name}="{}""#, border_to_string(l));
            }
        }
    }
    if st.align.wrap {
        cell_props.push_str(r#" fo:wrap-option="wrap""#);
    }
    if let Some(p) = st.protection {
        let v = match (p.hidden, p.locked, p.hidden_formula) {
            (true, _, _) => "hidden-and-protected",
            (false, true, true) => "protected formula-hidden",
            (false, true, false) => "protected",
            (false, false, true) => "formula-hidden",
            (false, false, false) => "none",
        };
        let _ = write!(cell_props, r#" style:cell-protect="{v}""#);
        if p.hide_print {
            cell_props.push_str(r#" style:print-content="false""#);
        }
    }
    match st.align.v {
        VAlign::Bottom => {}
        VAlign::Middle => cell_props.push_str(r#" style:vertical-align="middle""#),
        VAlign::Top => cell_props.push_str(r#" style:vertical-align="top""#),
    }
    let text_align = match st.align.h {
        HAlign::General => None,
        HAlign::Left => Some("start"),
        HAlign::Center => Some("center"),
        HAlign::Right => Some("end"),
        HAlign::Justify => Some("justify"),
    };
    if text_align.is_some() {
        cell_props.push_str(r#" style:text-align-source="fix" style:repeat-content="false""#);
    }
    if !cell_props.is_empty() {
        let _ = write!(out, "<style:table-cell-properties{cell_props}/>");
    }
    if let Some(a) = text_align {
        let _ = write!(out, r#"<style:paragraph-properties fo:text-align="{a}"/>"#);
    }
    let f = &st.font;
    let mut tp = String::new();
    if f.bold {
        tp.push_str(r#" fo:font-weight="bold" style:font-weight-asian="bold" style:font-weight-complex="bold""#);
    }
    if f.italic {
        tp.push_str(r#" fo:font-style="italic" style:font-style-asian="italic" style:font-style-complex="italic""#);
    }
    if f.underline {
        tp.push_str(r#" style:text-underline-style="solid" style:text-underline-width="auto" style:text-underline-color="font-color""#);
    }
    if f.strike {
        tp.push_str(
            r#" style:text-line-through-style="solid" style:text-line-through-type="single""#,
        );
    }
    if let Some(c) = f.color {
        let _ = write!(tp, r#" fo:color="{}""#, color_hex(c));
    }
    if let Some(sz) = f.size_pt.filter(|s| s.is_finite() && *s > 0.0) {
        let _ = write!(
            tp,
            r#" fo:font-size="{sz}pt" style:font-size-asian="{sz}pt" style:font-size-complex="{sz}pt""#
        );
    }
    if let Some(fam) = &f.family {
        tp.push_str(r#" style:font-name=""#);
        escape_attr(fam, &mut tp);
        tp.push('"');
    }
    if !tp.is_empty() {
        let _ = write!(out, "<style:text-properties{tp}/>");
    }
    out.push_str("</style:style>");
}

fn flush<W: Write + Seek>(buf: &mut String, zip: &mut ZipWriter<W>) -> Result<(), OdsError> {
    zip.write_all(buf.as_bytes())?;
    buf.clear();
    Ok(())
}

fn write_content<W: Write + Seek>(
    data: &WorkbookData,
    plan: &StylePlan,
    zip: &mut ZipWriter<W>,
    report: &mut WriteReport,
) -> Result<(), OdsError> {
    let mut s = String::with_capacity(FLUSH_AT + 4096);
    s.push_str(XML_DECL);
    let _ = write!(s, r#"<office:document-content{NS} office:version="1.2">"#);
    s.push_str("<office:font-face-decls>");
    for f in &plan.fonts {
        s.push_str(r#"<style:font-face style:name=""#);
        escape_attr(f, &mut s);
        s.push_str(r#"" svg:font-family=""#);
        escape_attr(f, &mut s);
        s.push_str(r#""/>"#);
    }
    s.push_str("</office:font-face-decls><office:automatic-styles>");
    plan.automatic_styles(&mut s);
    s.push_str("</office:automatic-styles><office:body><office:spreadsheet");
    if let Some(p) = &data.settings.structure_protection {
        crate::v03::write_protection_attrs(&mut s, "table:structure-protected", p);
    }
    s.push('>');

    let c = &data.settings.calc_settings;
    let (regex, wild) = match c.criteria_mode {
        CriteriaMode::Wildcards => (false, true),
        CriteriaMode::Regex => (true, false),
        CriteriaMode::Literal => (false, false),
    };
    let _ = write!(
        s,
        r#"<table:calculation-settings table:case-sensitive="{}" table:precision-as-shown="{}" table:search-criteria-must-apply-to-whole-cell="{}" table:automatic-find-labels="false" table:use-regular-expressions="{regex}" table:use-wildcards="{wild}">"#,
        c.case_sensitive, c.precision_as_shown, c.whole_cell_match
    );
    let _ = write!(
        s,
        r#"<table:null-date table:date-value="{}"/>"#,
        null_date_for(c.date_epoch)
    );
    if let Some(it) = c.iterations {
        let _ = write!(
            s,
            r#"<table:iteration table:status="enable" table:steps="{}" table:minimum-difference="{}"/>"#,
            it.steps,
            fmt_f64(it.min_change)
        );
    }
    s.push_str("</table:calculation-settings>");
    let vals: Vec<(String, &[tally_model::ValidationData])> = data
        .sheets
        .iter()
        .map(|sh| (odf_sheet_name(&sh.name), sh.validations.as_slice()))
        .collect();
    crate::v03::write_validations(&mut s, &vals);

    for (tab, sheet) in data.sheets.iter().enumerate() {
        write_sheet(sheet, tab, data, plan, &mut s, zip, report)?;
    }
    let global: Vec<&NameData> = data.names.iter().filter(|n| n.sheet.is_none()).collect();
    write_named_expressions(&mut s, &global, data);
    write_database_ranges(&mut s, data);
    s.push_str("</office:spreadsheet></office:body></office:document-content>");
    flush(&mut s, zip)
}

/// The run containing `i`, or the first run starting after it.
fn run_at(runs: &[DimRun], i: u32) -> (Option<&DimRun>, Option<u32>) {
    for r in runs {
        if r.start <= i && i <= r.end {
            return (Some(r), None);
        }
        if r.start > i {
            return (None, Some(r.start));
        }
    }
    (None, None)
}

/// Per-cell extras of a sheet: validations, notes, links, derived styles.
struct Extras<'a> {
    tab: usize,
    /// (ranges, validation name).
    vals: Vec<(&'a [CellRangeLocal], String)>,
    notes: HashMap<(u32, u32), &'a tally_model::NoteData>,
    links: HashMap<(u32, u32), &'a str>,
    derived: &'a HashMap<(usize, u32, u32), String>,
}

impl Extras<'_> {
    fn validation(&self, r: u32, c: u32) -> Option<&str> {
        self.vals
            .iter()
            .find(|(rs, _)| {
                rs.iter()
                    .any(|x| x.r0 <= r && r <= x.r1 && x.c0 <= c && c <= x.c1)
            })
            .map(|(_, n)| n.as_str())
    }
}

/// A vertical run of empty cells with a style and/or a validation.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct ERun {
    col: u32,
    start_row: u32,
    end_row: u32,
    style: Option<u32>,
    val: Option<usize>,
}

/// Merges, anchors and matrix spans of one row, for writing its cells.
struct RowCtx<'a> {
    /// Covered column intervals (inclusive), sorted by start.
    covered: Vec<(u32, u32)>,
    /// Merge anchors in this row: (col, cols spanned, rows spanned).
    anchors: Vec<(u32, u32, u32)>,
    /// Array formula anchors of the sheet: (row, col) → (cols, rows).
    matrix: &'a HashMap<(u32, u32), (u32, u32)>,
    row: u32,
    extras: &'a Extras<'a>,
}

impl RowCtx<'_> {
    fn is_plain(&self) -> bool {
        self.covered.is_empty() && self.anchors.is_empty()
    }

    fn covered(&self, c: u32) -> bool {
        let i = self.covered.partition_point(|&(a, _)| a <= c);
        i > 0 && self.covered[i - 1].1 >= c
    }

    fn anchor(&self, c: u32) -> Option<(u32, u32)> {
        let i = self.anchors.partition_point(|a| a.0 < c);
        self.anchors
            .get(i)
            .filter(|a| a.0 == c)
            .map(|&(_, cs, rs)| (cs, rs))
    }

    /// The first column after `c` where the covered/anchor status changes.
    fn next_boundary(&self, c: u32) -> u32 {
        let mut next = u32::MAX;
        let i = self.covered.partition_point(|&(a, _)| a <= c);
        if i > 0 && self.covered[i - 1].1 >= c {
            next = next.min(self.covered[i - 1].1.saturating_add(1));
        }
        if let Some(&(a, _)) = self.covered.get(i) {
            next = next.min(a);
        }
        let j = self.anchors.partition_point(|a| a.0 <= c);
        if j > 0 && self.anchors[j - 1].0 == c {
            next = next.min(c.saturating_add(1));
        }
        if let Some(&(a, _, _)) = self.anchors.get(j) {
            next = next.min(a);
        }
        next
    }

    /// One past the last column that needs an element (covered or anchor).
    fn end(&self) -> u32 {
        let a = self.covered.iter().map(|&(_, b)| b.saturating_add(1));
        let b = self.anchors.iter().map(|&(c, _, _)| c.saturating_add(1));
        a.chain(b).max().unwrap_or(0)
    }
}

/// Writes empty cells for columns `from..from+n` (styled with `style` and
/// validated by `val` if set), as covered cells where merges cover them
/// and with spans at merge anchors.
fn write_segmented(
    s: &mut String,
    ctx: &RowCtx<'_>,
    from: u32,
    n: u32,
    style: Option<u32>,
    val: Option<&str>,
) {
    let end = from.saturating_add(n);
    let mut c = from;
    while c < end {
        let stop = ctx.next_boundary(c).min(end);
        let k = stop - c;
        let anchor = ctx.anchor(c);
        let tag = if anchor.is_none() && ctx.covered(c) {
            "table:covered-table-cell"
        } else {
            "table:table-cell"
        };
        let _ = write!(s, "<{tag}");
        if let Some(st) = style {
            let _ = write!(s, r#" table:style-name="ce{}""#, st + 1);
        }
        if let Some(v) = val {
            let _ = write!(s, r#" table:content-validation-name="{v}""#);
        }
        if let Some((cs, rs)) = anchor {
            write_spans(s, cs, rs);
        } else if k > 1 {
            let _ = write!(s, r#" table:number-columns-repeated="{k}""#);
        }
        s.push_str("/>");
        c = stop;
    }
}

fn write_spans(s: &mut String, cs: u32, rs: u32) {
    let _ = write!(
        s,
        r#" table:number-columns-spanned="{cs}" table:number-rows-spanned="{rs}""#
    );
}

/// Sorted, merged inclusive spans.
fn merge_spans(spans: &[(u32, u32)]) -> Vec<(u32, u32)> {
    let mut v: Vec<(u32, u32)> = spans.iter().copied().filter(|(a, b)| a <= b).collect();
    v.sort_unstable();
    let mut out: Vec<(u32, u32)> = Vec::with_capacity(v.len());
    for (a, b) in v {
        if let Some(last) = out.last_mut() {
            if a <= last.1.saturating_add(1) {
                last.1 = last.1.max(b);
                continue;
            }
        }
        out.push((a, b));
    }
    out
}

/// A sheet name as written in ODF cell addresses (quoted when needed).
pub(crate) fn odf_sheet_name(name: &str) -> String {
    let plain = !name.is_empty()
        && name.chars().all(|c| c.is_alphanumeric() || c == '_')
        && !name.starts_with(|c: char| c.is_ascii_digit());
    if plain {
        name.to_string()
    } else {
        format!("'{}'", name.replace('\'', "''"))
    }
}

/// Splits style runs and validation rectangles into disjoint vertical
/// runs per column (`col_default` columns keep their default style: their
/// validation-only runs carry no style).
fn empty_runs(sheet: &SheetData, vruns: &[StyleRun]) -> Vec<ERun> {
    let mut cols: BTreeMap<u32, Vec<ColInterval>> = BTreeMap::new();
    for r in vruns {
        cols.entry(r.col)
            .or_default()
            .push((r.start_row, r.end_row, Some(r.style), None));
    }
    for (i, v) in sheet.validations.iter().enumerate() {
        for rg in &v.ranges {
            if rg.r1 >= MAX_ROWS || rg.c1 >= MAX_COLS || rg.r0 > rg.r1 || rg.c0 > rg.c1 {
                continue;
            }
            for c in rg.c0..=rg.c1 {
                cols.entry(c)
                    .or_default()
                    .push((rg.r0, rg.r1, None, Some(i)));
            }
        }
    }
    let mut out = Vec::new();
    for (col, ivs) in cols {
        if ivs.iter().all(|x| x.3.is_none()) {
            // Style runs only: they are already disjoint.
            for (a, b, st, _) in ivs {
                out.push(ERun {
                    col,
                    start_row: a,
                    end_row: b,
                    style: st,
                    val: None,
                });
            }
            continue;
        }
        let mut points: Vec<u32> = Vec::with_capacity(ivs.len() * 2);
        for &(a, b, _, _) in &ivs {
            points.push(a);
            points.push(b.saturating_add(1));
        }
        points.sort_unstable();
        points.dedup();
        for w in points.windows(2) {
            let (a, b) = (w[0], w[1] - 1);
            // Later entries win (a later validation replaces an earlier one).
            let style = ivs
                .iter()
                .rev()
                .find(|x| x.2.is_some() && x.0 <= a && a <= x.1)
                .and_then(|x| x.2);
            let val = ivs
                .iter()
                .rev()
                .find(|x| x.3.is_some() && x.0 <= a && a <= x.1)
                .and_then(|x| x.3);
            if style.is_none() && val.is_none() {
                continue;
            }
            match out.last_mut() {
                Some(ERun {
                    col: c,
                    end_row,
                    style: s2,
                    val: v2,
                    ..
                }) if *c == col
                    && end_row.checked_add(1) == Some(a)
                    && *s2 == style
                    && *v2 == val =>
                {
                    *end_row = b;
                }
                _ => out.push(ERun {
                    col,
                    start_row: a,
                    end_row: b,
                    style,
                    val,
                }),
            }
        }
    }
    out.sort_by_key(|r| (r.start_row, r.col));
    out
}

fn write_sheet<W: Write + Seek>(
    sheet: &SheetData,
    tab: usize,
    data: &WorkbookData,
    plan: &StylePlan,
    s: &mut String,
    zip: &mut ZipWriter<W>,
    report: &mut WriteReport,
) -> Result<(), OdsError> {
    let sheet_name = odf_sheet_name(&sheet.name);
    s.push_str(r#"<table:table table:name=""#);
    escape_attr(&sheet.name, s);
    let _ = write!(s, r#"" table:style-name="{}""#, plan.table_style(sheet));
    if let Some(p) = &sheet.protection {
        crate::v03::write_protection_attrs(s, "table:protected", p);
    }
    if let Some(r) = sheet.page_setup.as_ref().and_then(|p| p.print_range) {
        s.push_str(r#" table:print-ranges=""#);
        escape_attr(&crate::v03::range_list(&sheet_name, &[r]), s);
        s.push('"');
    }
    s.push('>');
    if let Some(p) = &sheet.protection {
        crate::v03::write_protection_options(s, p);
    }
    let repeat_rows = sheet.page_setup.as_ref().and_then(|p| p.repeat_rows);
    let repeat_cols = sheet.page_setup.as_ref().and_then(|p| p.repeat_cols);

    let extras = Extras {
        tab,
        vals: sheet
            .validations
            .iter()
            .enumerate()
            .rev()
            .map(|(i, v)| (v.ranges.as_slice(), crate::v03::validation_name(tab, i)))
            .collect(),
        notes: sheet.notes.iter().map(|n| ((n.row, n.col), n)).collect(),
        links: sheet
            .links
            .iter()
            .map(|l| ((l.row, l.col), l.url.as_str()))
            .collect(),
        derived: &plan.derived,
    };

    let mut cells: Vec<&CellData> = sheet.cells.iter().collect();
    cells.sort_by_key(|c| (c.row, c.col));
    cells.dedup_by_key(|c| (c.row, c.col));
    // Notes and links on cells that do not exist get an empty cell.
    let style_at = |r: u32, c: u32| {
        sheet
            .style_runs
            .iter()
            .rev()
            .find(|x| x.col == c && x.start_row <= r && r <= x.end_row)
            .map(|x| x.style)
    };
    let mut synth: Vec<CellData> = Vec::new();
    {
        let have: std::collections::HashSet<(u32, u32)> =
            cells.iter().map(|c| (c.row, c.col)).collect();
        let mut extra_pos: Vec<(u32, u32)> = extras
            .notes
            .keys()
            .chain(extras.links.keys())
            .copied()
            .filter(|p| !have.contains(p) && p.0 < MAX_ROWS && p.1 < MAX_COLS)
            .collect();
        extra_pos.sort_unstable();
        extra_pos.dedup();
        for (r, c) in extra_pos {
            synth.push(CellData {
                row: r,
                col: c,
                content: CellContentData::Value(Value::Empty),
                style: style_at(r, c),
            });
        }
    }
    if !synth.is_empty() {
        cells.extend(synth.iter());
        cells.sort_by_key(|c| (c.row, c.col));
    }
    let mut cols = sheet.col_widths.clone();
    cols.sort_by_key(|r| r.start);
    let mut rows = sheet.row_heights.clone();
    rows.sort_by_key(|r| r.start);

    let mut merges: Vec<CellRangeLocal> = sheet
        .merges
        .iter()
        .copied()
        .filter(|m| {
            m.r0 <= m.r1
                && m.c0 <= m.c1
                && m.r1 < MAX_ROWS
                && m.c1 < MAX_COLS
                && (m.r0 != m.r1 || m.c0 != m.c1)
        })
        .collect();
    merges.sort_by_key(|m| (m.r0, m.c0));
    let matrix: HashMap<(u32, u32), (u32, u32)> = sheet
        .array_formulas
        .iter()
        .filter(|a| a.r0 <= a.r1 && a.c0 <= a.c1)
        .map(|a| ((a.r0, a.c0), (a.c1 - a.c0 + 1, a.r1 - a.r0 + 1)))
        .collect();
    let filtered = merge_spans(
        sheet
            .auto_filter
            .as_ref()
            .map_or(&[][..], |f| f.filtered_rows.as_slice()),
    );

    // Style runs: whole columns become the column's default cell style;
    // the others are written as (repeated) rows of styled empty cells.
    let valid_style = |i: u32| (i as usize) < data.styles.len();
    let mut col_default: BTreeMap<u32, u32> = BTreeMap::new();
    let mut style_vruns: Vec<StyleRun> = Vec::new();
    for run in &sheet.style_runs {
        if !valid_style(run.style) || run.start_row > run.end_row || run.end_row >= MAX_ROWS {
            continue;
        }
        if run.start_row == 0 && run.end_row == MAX_ROWS - 1 {
            col_default.insert(run.col, run.style);
        } else {
            style_vruns.push(*run);
        }
    }
    let vruns = empty_runs(sheet, &style_vruns);
    let col_breaks: std::collections::BTreeSet<u32> = sheet.col_breaks.iter().copied().collect();
    let row_breaks: std::collections::BTreeSet<u32> = sheet.row_breaks.iter().copied().collect();

    // Columns.
    let max_cell_col = cells.iter().map(|c| c.col).max();
    let max_run_col = cols.iter().map(|r| r.end).max();
    let max_style_col = col_default
        .keys()
        .copied()
        .chain(vruns.iter().map(|r| r.col))
        .chain(merges.iter().map(|m| m.c1))
        .chain(col_breaks.iter().copied())
        .chain(repeat_cols.map(|r| r.1))
        .max();
    let end_col = max_cell_col
        .max(max_run_col)
        .max(max_style_col)
        .map_or(1, |c| c.saturating_add(1))
        .min(MAX_COLS);
    let mut c = 0;
    while c < end_col {
        if let Some((a, _)) = repeat_cols {
            if c == a {
                s.push_str("<table:table-header-columns>");
            }
        }
        let (run, next) = run_at(&cols, c);
        let (width, hidden, mut stop) = match run {
            Some(r) => (r.size_hmm, r.hidden, r.end.saturating_add(1).min(end_col)),
            None => (
                DEFAULT_COL_WIDTH_HMM,
                false,
                next.unwrap_or(end_col).min(end_col),
            ),
        };
        let brk = col_breaks.contains(&c);
        if brk {
            stop = c + 1;
        }
        if let Some(&b) = col_breaks.range(c + 1..).next() {
            stop = stop.min(b);
        }
        if let Some((a, b)) = repeat_cols {
            for x in [a, b.saturating_add(1)] {
                if x > c {
                    stop = stop.min(x);
                }
            }
        }
        let default = col_default.get(&c).copied();
        let mut n = 1;
        while c + n < stop && col_default.get(&(c + n)).copied() == default {
            n += 1;
        }
        let _ = write!(
            s,
            r#"<table:table-column table:style-name="{}""#,
            plan.col_style(width, brk)
        );
        if hidden {
            s.push_str(r#" table:visibility="collapse""#);
        }
        if n > 1 {
            let _ = write!(s, r#" table:number-columns-repeated="{n}""#);
        }
        match default {
            Some(i) => {
                let _ = write!(s, r#" table:default-cell-style-name="ce{}"/>"#, i + 1);
            }
            None => s.push_str(r#" table:default-cell-style-name="Default"/>"#),
        }
        c += n;
        if let Some((_, b)) = repeat_cols {
            if c == b.saturating_add(1) {
                s.push_str("</table:table-header-columns>");
            }
        }
    }

    // Rows.
    let max_cell_row = cells.last().map(|c| c.row);
    let max_run_row = rows.iter().map(|r| r.end).max();
    let max_style_row = vruns
        .iter()
        .map(|r| r.end_row)
        .chain(merges.iter().map(|m| m.r1))
        .chain(filtered.last().map(|f| f.1))
        .chain(row_breaks.iter().copied())
        .chain(repeat_rows.map(|r| r.1))
        .max();
    let end_row = max_cell_row
        .max(max_run_row)
        .max(max_style_row)
        .map_or(1, |r| r.saturating_add(1))
        .min(MAX_ROWS);
    let mut r = 0;
    let mut ci = 0;
    let mut vi = 0;
    let mut mi = 0;
    let mut fi = 0;
    // Runs covering the current row, sorted by column.
    let mut active: Vec<ERun> = Vec::new();
    // Merges covering the current row.
    let mut active_m: Vec<CellRangeLocal> = Vec::new();
    while r < end_row {
        if let Some((a, _)) = repeat_rows {
            if r == a {
                s.push_str("<table:table-header-rows>");
            }
        }
        active.retain(|a| a.end_row >= r);
        while vi < vruns.len() && vruns[vi].start_row <= r {
            let run = vruns[vi];
            if run.end_row >= r {
                let at = active.partition_point(|a| a.col < run.col);
                active.insert(at, run);
            }
            vi += 1;
        }
        active_m.retain(|m| m.r1 >= r);
        while mi < merges.len() && merges[mi].r0 <= r {
            if merges[mi].r1 >= r {
                active_m.push(merges[mi]);
            }
            mi += 1;
        }
        while fi < filtered.len() && filtered[fi].1 < r {
            fi += 1;
        }
        let is_filtered = filtered.get(fi).is_some_and(|f| f.0 <= r);
        let next_filter = match filtered.get(fi) {
            Some(f) if f.0 <= r => f.1.saturating_add(1),
            Some(f) => f.0,
            None => u32::MAX,
        };
        // The next row where the set of covered cells changes.
        let next_merge = active_m
            .iter()
            .map(|m| {
                if m.r0 == r {
                    r + 1
                } else {
                    m.r1.saturating_add(1)
                }
            })
            .chain(merges.get(mi).map(|m| m.r0))
            .min()
            .unwrap_or(u32::MAX);
        let mut ctx = RowCtx {
            covered: Vec::new(),
            anchors: Vec::new(),
            matrix: &matrix,
            row: r,
            extras: &extras,
        };
        for m in &active_m {
            if m.r0 == r {
                ctx.anchors.push((m.c0, m.c1 - m.c0 + 1, m.r1 - m.r0 + 1));
                if m.c1 > m.c0 {
                    ctx.covered.push((m.c0 + 1, m.c1));
                }
            } else {
                ctx.covered.push((m.c0, m.c1));
            }
        }
        // Sorted and disjoint, so the per-cell lookups are binary searches
        // (review 2, LOW-1: linear scans made saving 8,000 merges per row
        // quadratic).
        ctx.covered = merge_spans(&ctx.covered);
        ctx.anchors.sort_unstable();
        // The next row where the set of active runs changes.
        let next_style = active
            .iter()
            .map(|a| a.end_row.saturating_add(1))
            .chain(vruns.get(vi).map(|v| v.start_row))
            .min()
            .unwrap_or(u32::MAX);
        let brk = row_breaks.contains(&r);
        let next_break = row_breaks
            .range(r + 1..)
            .next()
            .copied()
            .unwrap_or(u32::MAX);
        let next_header = repeat_rows.map_or(u32::MAX, |(a, b)| {
            [a, b.saturating_add(1)]
                .into_iter()
                .filter(|x| *x > r)
                .min()
                .unwrap_or(u32::MAX)
        });
        let (run, next) = run_at(&rows, r);
        let (height, hidden, run_stop) = match run {
            Some(x) => (x.size_hmm, x.hidden, x.end.saturating_add(1)),
            None => (DEFAULT_ROW_HEIGHT_HMM, false, next.unwrap_or(u32::MAX)),
        };
        let _ = write!(
            s,
            r#"<table:table-row table:style-name="{}""#,
            plan.row_style(height, brk)
        );
        // A row hidden by the user stays "collapse" even when the filter
        // also hides it, so it stays hidden when the filter is removed.
        if hidden {
            s.push_str(r#" table:visibility="collapse""#);
        } else if is_filtered {
            s.push_str(r#" table:visibility="filter""#);
        }
        let row_start = ci;
        while ci < cells.len() && cells[ci].row == r {
            ci += 1;
        }
        let row_cells = &cells[row_start..ci];
        let n = if row_cells.is_empty() && !brk {
            let next_cell = cells.get(ci).map_or(end_row, |c| c.row);
            next_cell
                .min(run_stop)
                .min(next_style)
                .min(next_merge)
                .min(next_filter)
                .min(next_break)
                .min(next_header)
                .min(end_row)
                .max(r + 1)
                - r
        } else {
            1
        };
        if n > 1 {
            let _ = write!(s, r#" table:number-rows-repeated="{n}""#);
        }
        if row_cells.is_empty() && active.is_empty() && ctx.is_plain() {
            s.push_str("><table:table-cell/></table:table-row>");
        } else {
            s.push('>');
            write_row_cells(s, row_cells, &active, data, report, &ctx);
            s.push_str("</table:table-row>");
        }
        r += n;
        if let Some((_, b)) = repeat_rows {
            if r == b.saturating_add(1) {
                s.push_str("</table:table-header-rows>");
            }
        }
        if s.len() >= FLUSH_AT {
            flush(s, zip)?;
        }
    }
    let local: Vec<&NameData> = data
        .names
        .iter()
        .filter(|n| n.sheet == Some(tab as u32))
        .collect();
    write_named_expressions(s, &local, data);
    crate::v03::write_cond_formats(s, &sheet_name, &sheet.conditional_formats, &|st| {
        plan.cf_style_known(st)
    });
    s.push_str("</table:table>");
    Ok(())
}

/// Writes `table:named-expressions` for `names` (nothing when empty).
fn write_named_expressions(s: &mut String, names: &[&NameData], data: &WorkbookData) {
    if names.is_empty() {
        return;
    }
    let base = format!(
        "${}.$A$1",
        odf_sheet_name(data.sheets.first().map_or("Sheet1", |s| s.name.as_str()))
    );
    s.push_str("<table:named-expressions>");
    for n in names {
        let body = n.formula.strip_prefix("of:").unwrap_or(&n.formula);
        let range = body
            .strip_prefix("=[")
            .and_then(|b| b.strip_suffix(']'))
            .filter(|inner| !inner.contains(['[', ']']));
        match range {
            Some(inner) => {
                s.push_str(r#"<table:named-range table:name=""#);
                escape_attr(&n.name, s);
                s.push_str(r#"" table:base-cell-address=""#);
                escape_attr(&base, s);
                s.push_str(r#"" table:cell-range-address=""#);
                escape_attr(inner, s);
                s.push_str(r#""/>"#);
            }
            None => {
                s.push_str(r#"<table:named-expression table:name=""#);
                escape_attr(&n.name, s);
                s.push_str(r#"" table:base-cell-address=""#);
                escape_attr(&base, s);
                s.push_str(r#"" table:expression=""#);
                let expr = if n.formula.starts_with("of:") || n.formula.starts_with("oooc:") {
                    n.formula.clone()
                } else {
                    format!("of:{}", n.formula)
                };
                escape_attr(&expr, s);
                s.push_str(r#""/>"#);
            }
        }
    }
    s.push_str("</table:named-expressions>");
}

fn filter_operator(op: FilterOpData) -> &'static str {
    match op {
        FilterOpData::Eq => "=",
        FilterOpData::Ne => "!=",
        FilterOpData::Gt => ">",
        FilterOpData::Ge => ">=",
        FilterOpData::Lt => "<",
        FilterOpData::Le => "<=",
        FilterOpData::Contains => "contains",
        FilterOpData::BeginsWith => "begins",
        FilterOpData::EndsWith => "ends",
        FilterOpData::Empty => "empty",
        FilterOpData::NotEmpty => "!empty",
    }
}

fn write_condition(s: &mut String, field: u32, op: &str, value: &str) {
    let ty = if value.trim().parse::<f64>().is_ok() {
        "number"
    } else {
        "text"
    };
    let _ = write!(
        s,
        r#"<table:filter-condition table:field-number="{field}" table:value=""#
    );
    escape_attr(value, s);
    let _ = write!(
        s,
        r#"" table:operator="{}" table:data-type="{ty}"/>"#,
        escape_op(op)
    );
}

fn escape_op(op: &str) -> String {
    let mut out = String::new();
    escape_attr(op, &mut out);
    out
}

/// Writes `table:database-ranges` for the sheets' AutoFilters.
fn write_database_ranges(s: &mut String, data: &WorkbookData) {
    let filters: Vec<(usize, &SheetData, &AutoFilterData)> = data
        .sheets
        .iter()
        .enumerate()
        .filter_map(|(i, sh)| sh.auto_filter.as_ref().map(|f| (i, sh, f)))
        .filter(|(_, _, f)| f.range.r0 <= f.range.r1 && f.range.c0 <= f.range.c1)
        .collect();
    if filters.is_empty() {
        return;
    }
    s.push_str("<table:database-ranges>");
    for (tab, sheet, f) in filters {
        let name = odf_sheet_name(&sheet.name);
        let a = |r: u32, c: u32| format!("{name}.{}{}", col_to_letters(c), r + 1);
        let _ = write!(
            s,
            r#"<table:database-range table:name="__Anonymous_Sheet_DB__{tab}" table:target-range-address=""#
        );
        escape_attr(
            &format!(
                "{}:{}",
                a(f.range.r0, f.range.c0),
                a(f.range.r1, f.range.c1)
            ),
            s,
        );
        s.push_str(r#"" table:display-filter-buttons="true">"#);
        let mut items: Vec<String> = Vec::new();
        for col in &f.columns {
            if col.col < f.range.c0 || col.col > f.range.c1 {
                continue;
            }
            let field = col.col - f.range.c0;
            if let Some(values) = &col.values {
                let mut x = String::from("<table:filter-or>");
                for v in values {
                    if v.is_empty() {
                        write_condition(&mut x, field, "empty", "");
                    } else {
                        write_condition(&mut x, field, "=", v);
                    }
                }
                x.push_str("</table:filter-or>");
                items.push(x);
            }
            if let Some(c) = &col.condition {
                let mut x = String::new();
                write_condition(&mut x, field, filter_operator(c.op), &c.value);
                items.push(x);
            }
        }
        if !items.is_empty() {
            s.push_str("<table:filter>");
            if items.len() == 1 {
                s.push_str(&items[0]);
            } else {
                s.push_str("<table:filter-and>");
                for it in &items {
                    s.push_str(it);
                }
                s.push_str("</table:filter-and>");
            }
            s.push_str("</table:filter>");
        }
        s.push_str("</table:database-range>");
    }
    s.push_str("</table:database-ranges>");
}

/// Writes one row: its cells merged with styled empty cells for the style
/// and validation runs covering the row (`active`, sorted by column),
/// covered cells and merge anchors from `ctx`.
fn write_row_cells(
    s: &mut String,
    row_cells: &[&CellData],
    active: &[ERun],
    data: &WorkbookData,
    report: &mut WriteReport,
    ctx: &RowCtx<'_>,
) {
    let mut col = 0;
    let mut ai = 0;
    for cell in row_cells {
        while ai < active.len() && active[ai].col < cell.col {
            col = write_styled_empty(s, ctx, col, &active[ai..], cell.col, &mut ai);
        }
        if ai < active.len() && active[ai].col == cell.col {
            ai += 1;
        }
        if cell.col > col {
            write_segmented(s, ctx, col, cell.col - col, None, None);
        }
        write_cell(s, cell, data, report, ctx);
        col = cell.col.saturating_add(1);
    }
    while ai < active.len() {
        col = write_styled_empty(s, ctx, col, &active[ai..], u32::MAX, &mut ai);
    }
    let end = ctx.end();
    if end > col {
        write_segmented(s, ctx, col, end - col, None, None);
    }
}

/// Writes the empty cells of `runs[0]` (after any blanks from `col`),
/// grouping following runs of the same style and validation in adjacent
/// columns below `limit`. Returns the next column and advances `ai`.
fn write_styled_empty(
    s: &mut String,
    ctx: &RowCtx<'_>,
    col: u32,
    runs: &[ERun],
    limit: u32,
    ai: &mut usize,
) -> u32 {
    let first = runs[0];
    if first.col > col {
        write_segmented(s, ctx, col, first.col - col, None, None);
    }
    let mut n = 1;
    while (n as usize) < runs.len()
        && runs[n as usize].col == first.col + n
        && runs[n as usize].col < limit
        && runs[n as usize].style == first.style
        && runs[n as usize].val == first.val
    {
        n += 1;
    }
    let name = first
        .val
        .map(|i| crate::v03::validation_name(ctx.extras.tab, i));
    write_segmented(s, ctx, first.col, n, first.style, name.as_deref());
    *ai += n as usize;
    first.col + n
}

fn write_cell(
    s: &mut String,
    cell: &CellData,
    data: &WorkbookData,
    report: &mut WriteReport,
    ctx: &RowCtx<'_>,
) {
    let x = ctx.extras;
    let anchor = ctx.anchor(cell.col);
    let tag = if anchor.is_none() && ctx.covered(cell.col) {
        "table:covered-table-cell"
    } else {
        "table:table-cell"
    };
    let _ = write!(s, "<{tag}");
    match x.derived.get(&(x.tab, cell.row, cell.col)) {
        Some(name) => {
            let _ = write!(s, r#" table:style-name="{name}""#);
        }
        None => {
            if let Some(i) = cell.style {
                if (i as usize) < data.styles.len() {
                    let _ = write!(s, r#" table:style-name="ce{}""#, i + 1);
                }
            }
        }
    }
    if let Some(v) = x.validation(cell.row, cell.col) {
        let _ = write!(s, r#" table:content-validation-name="{v}""#);
    }
    if let Some((cs, rs)) = anchor {
        write_spans(s, cs, rs);
    }
    let (value, is_formula) = match &cell.content {
        CellContentData::Value(v) => (Some(v), false),
        CellContentData::Formula { text, cached } => {
            s.push_str(r#" table:formula=""#);
            escape_attr(text, s);
            s.push('"');
            if let Some((cols, rows)) = ctx.matrix.get(&(ctx.row, cell.col)) {
                let _ = write!(
                    s,
                    r#" table:number-matrix-columns-spanned="{cols}" table:number-matrix-rows-spanned="{rows}""#
                );
            }
            (cached.as_ref(), true)
        }
        // Only readers produce shared members (the engine resolves them);
        // without a formula printer here, keep the cached value.
        CellContentData::SharedFormula { cached, .. } => {
            let w = "shared formula members written as their cached values".to_string();
            if !report.warnings.contains(&w) {
                report.warnings.push(w);
            }
            (cached.as_ref(), false)
        }
    };
    let mut value = value.cloned().unwrap_or(Value::Empty);
    if let Value::Array(m) = &value {
        report.warnings.push(format!(
            "array value at row {} col {} written as its top-left element",
            cell.row + 1,
            cell.col + 1
        ));
        value = m.data.first().cloned().unwrap_or(Value::Empty);
    }
    if let Value::Number(n) = value {
        if !n.is_finite() {
            value = Value::Error(tally_types::ErrorKind::Num);
        }
    }
    // Attributes, then the paragraphs (wrapped in a link) after any note.
    let text: Option<String> = match &value {
        Value::Empty | Value::Array(_) => None,
        Value::Number(n) => {
            let _ = write!(
                s,
                r#" office:value-type="float" office:value="{}" calcext:value-type="float""#,
                fmt_f64(*n)
            );
            Some(general_display(*n))
        }
        Value::Bool(b) => {
            let _ = write!(
                s,
                r#" office:value-type="boolean" office:boolean-value="{b}" calcext:value-type="boolean""#
            );
            Some(if *b { "TRUE" } else { "FALSE" }.to_string())
        }
        Value::Text(t) => {
            s.push_str(r#" office:value-type="string""#);
            if is_formula || needs_string_value(t) {
                s.push_str(r#" office:string-value=""#);
                escape_attr(t, s);
                s.push('"');
            }
            s.push_str(r#" calcext:value-type="string""#);
            Some(t.to_string())
        }
        Value::Error(e) => {
            s.push_str(
                r#" office:value-type="string" office:string-value="" calcext:value-type="error""#,
            );
            Some(e.display().into_owned())
        }
    };
    let note = x.notes.get(&(cell.row, cell.col));
    let link = x.links.get(&(cell.row, cell.col));
    if text.is_none() && note.is_none() && link.is_none() {
        s.push_str("/>");
        return;
    }
    s.push('>');
    if let Some(n) = note {
        crate::v03::write_annotation(s, n);
    }
    match (text, link) {
        (Some(t), None) => write_paragraphs(s, &t),
        (Some(t), Some(url)) => {
            let mut inner = String::new();
            write_paragraphs(&mut inner, &t);
            // Wrap each paragraph's content in the link.
            let mut href = String::new();
            escape_attr(url, &mut href);
            let open = format!(r#"<text:a xlink:href="{href}" xlink:type="simple">"#);
            let wrapped = inner
                .replace("<text:p/>", "<text:p></text:p>")
                .replace("<text:p>", &format!("<text:p>{open}"))
                .replace("</text:p>", "</text:a></text:p>");
            s.push_str(&wrapped);
        }
        (None, Some(url)) => {
            s.push_str(r#"<text:p><text:a xlink:href=""#);
            escape_attr(url, s);
            s.push_str(r#"" xlink:type="simple"/></text:p>"#);
        }
        (None, None) => {}
    }
    let _ = write!(s, "</{tag}>");
}

/// Text whose `text:p` form would not survive white-space collapsing exactly.
fn needs_string_value(t: &str) -> bool {
    t.contains('\r')
}

/// Writes `t` as one `text:p` per line, encoding spaces and tabs so ODF
/// white-space collapsing reproduces the exact string.
pub(crate) fn write_paragraphs(s: &mut String, t: &str) {
    for line in t.split('\n') {
        if line.is_empty() {
            s.push_str("<text:p/>");
            continue;
        }
        s.push_str("<text:p>");
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            match chars[i] {
                ' ' => {
                    let start = i;
                    while i < chars.len() && chars[i] == ' ' {
                        i += 1;
                    }
                    let n = i - start;
                    let at_edge = start == 0 || i == chars.len();
                    let extra = if at_edge {
                        n
                    } else {
                        s.push(' ');
                        n - 1
                    };
                    match extra {
                        0 => {}
                        1 => s.push_str("<text:s/>"),
                        k => {
                            let _ = write!(s, r#"<text:s text:c="{k}"/>"#);
                        }
                    }
                    continue;
                }
                '\t' => s.push_str("<text:tab/>"),
                '\r' => {}
                c => {
                    let mut tmp = [0u8; 4];
                    escape_text(c.encode_utf8(&mut tmp), s);
                }
            }
            i += 1;
        }
        s.push_str("</text:p>");
    }
}
