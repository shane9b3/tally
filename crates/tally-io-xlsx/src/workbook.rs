//! Package-level parts: `[Content_Types].xml`, relationships and
//! `xl/workbook.xml`.

use std::io::BufRead;

use crate::units::parse_bool;
use crate::xml::{Ev, XmlReader};
use crate::XlsxError;

/// Longest defined-name text we keep (longer names are dropped).
const MAX_NAME_TEXT: usize = 64 * 1024;

/// One relationship of a part.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Rel {
    pub id: String,
    /// Package path of the target (`xl/worksheets/sheet1.xml`), already
    /// resolved against the source part's directory.
    pub target: String,
    /// Relationship type URI.
    pub rtype: String,
}

impl Rel {
    /// `true` if the type URI ends with `/kind` (`worksheet`, `styles`…).
    pub fn is(&self, kind: &str) -> bool {
        self.rtype
            .rsplit('/')
            .next()
            .is_some_and(|k| k.eq_ignore_ascii_case(kind))
    }
}

/// Resolves a relationship target against `base_dir` (`xl`): absolute
/// targets (`/xl/x.xml`) are package-rooted, `..` components are folded.
/// The result may still be unsafe; the package refuses such names.
pub(crate) fn resolve_target(base_dir: &str, target: &str) -> String {
    let target = target.replace('\\', "/");
    let joined = if let Some(abs) = target.strip_prefix('/') {
        abs.to_string()
    } else if base_dir.is_empty() {
        target
    } else {
        format!("{base_dir}/{target}")
    };
    let mut parts: Vec<&str> = Vec::new();
    for p in joined.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    // Escapes the package root: keep `..` so the lookup fails.
                    parts.push("..");
                }
            }
            other => parts.push(other),
        }
    }
    parts.join("/")
}

/// The directory of a part path (`xl/workbook.xml` → `xl`).
pub(crate) fn part_dir(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(d, _)| d)
}

/// The relationships part of a part (`xl/workbook.xml` →
/// `xl/_rels/workbook.xml.rels`).
pub(crate) fn rels_path(path: &str) -> String {
    match path.rsplit_once('/') {
        Some((d, f)) => format!("{d}/_rels/{f}.rels"),
        None => format!("_rels/{path}.rels"),
    }
}

/// Parses a `.rels` part. External targets (`TargetMode="External"`) are
/// dropped: we never follow links out of the package.
pub(crate) fn parse_rels<R: BufRead>(
    xr: &mut XmlReader<R>,
    base_dir: &str,
) -> Result<Vec<Rel>, XlsxError> {
    let mut out = Vec::new();
    loop {
        match xr.next()? {
            Ev::Start(e) if e.local() == "Relationship" => {
                let external = e
                    .attr_local("TargetMode")
                    .is_some_and(|m| m.eq_ignore_ascii_case("External"));
                if let (Some(id), Some(target), false) =
                    (e.attr_local("Id"), e.attr_local("Target"), external)
                {
                    out.push(Rel {
                        id: id.to_string(),
                        target: resolve_target(base_dir, target),
                        rtype: e.attr_local("Type").unwrap_or("").to_string(),
                    });
                }
                xr.skip()?;
            }
            Ev::Eof => return Ok(out),
            _ => {}
        }
    }
}

/// What `[Content_Types].xml` says about the package.
#[derive(Clone, Debug, Default)]
pub(crate) struct ContentTypes {
    /// Part names with a spreadsheet main content type.
    pub main_parts: Vec<String>,
    /// A binary (`.xlsb`) workbook part is declared.
    pub binary: bool,
}

pub(crate) fn parse_content_types<R: BufRead>(
    xr: &mut XmlReader<R>,
) -> Result<ContentTypes, XlsxError> {
    let mut ct = ContentTypes::default();
    loop {
        match xr.next()? {
            Ev::Start(e) if e.local() == "Override" || e.local() == "Default" => {
                let t = e.attr_local("ContentType").unwrap_or("");
                if t.contains("sheet.binary") {
                    ct.binary = true;
                }
                if t.contains("spreadsheetml.sheet.main+xml")
                    || t.contains("spreadsheetml.template.main+xml")
                    || t.contains("ms-excel.sheet.macroEnabled.main+xml")
                    || t.contains("ms-excel.template.macroEnabled.main+xml")
                {
                    if let Some(p) = e.attr_local("PartName") {
                        ct.main_parts.push(p.trim_start_matches('/').to_string());
                    }
                }
                xr.skip()?;
            }
            Ev::Eof => return Ok(ct),
            _ => {}
        }
    }
}

/// One `<sheet>` of `xl/workbook.xml`.
#[derive(Clone, Debug, Default)]
pub(crate) struct SheetEntry {
    pub name: String,
    pub rid: String,
    pub hidden: bool,
}

/// One `<definedName>`.
#[derive(Clone, Debug, Default)]
pub(crate) struct DefinedName {
    pub name: String,
    pub local_sheet: Option<u32>,
    pub text: String,
}

/// The parts of `xl/workbook.xml` we use.
#[derive(Clone, Debug, Default)]
pub(crate) struct WorkbookInfo {
    pub sheets: Vec<SheetEntry>,
    pub names: Vec<DefinedName>,
    pub date1904: bool,
    pub active_tab: u32,
    /// `calcPr iterate="1"`: (count, delta).
    pub iterate: Option<(u32, f64)>,
}

pub(crate) fn parse_workbook<R: BufRead>(
    xr: &mut XmlReader<R>,
    max_sheets: usize,
) -> Result<WorkbookInfo, XlsxError> {
    let mut wb = WorkbookInfo::default();
    loop {
        match xr.next()? {
            Ev::Start(e) => match e.local() {
                "workbook" | "sheets" | "definedNames" | "bookViews" => {}
                "workbookPr" => {
                    wb.date1904 = e.attr_local("date1904").is_some_and(parse_bool);
                    xr.skip()?;
                }
                "workbookView" => {
                    if let Some(t) = e
                        .attr_local("activeTab")
                        .and_then(|v| v.trim().parse().ok())
                    {
                        wb.active_tab = t;
                    }
                    xr.skip()?;
                }
                "calcPr" => {
                    if e.attr_local("iterate").is_some_and(parse_bool) {
                        let count = e
                            .attr_local("iterateCount")
                            .and_then(|v| v.trim().parse().ok())
                            .unwrap_or(100);
                        let delta = e
                            .attr_local("iterateDelta")
                            .and_then(|v| v.trim().parse::<f64>().ok())
                            .filter(|d| d.is_finite() && *d >= 0.0)
                            .unwrap_or(0.001);
                        wb.iterate = Some((count, delta));
                    }
                    xr.skip()?;
                }
                "sheet" => {
                    if wb.sheets.len() >= max_sheets {
                        return Err(XlsxError::LimitExceeded {
                            what: "sheets",
                            limit: max_sheets as u64,
                        });
                    }
                    let state = e.attr_local("state").unwrap_or("visible");
                    wb.sheets.push(SheetEntry {
                        name: e.attr_local("name").unwrap_or("").to_string(),
                        rid: e.attr_local("id").unwrap_or("").to_string(),
                        hidden: state == "hidden" || state == "veryHidden",
                    });
                    xr.skip()?;
                }
                "definedName" => {
                    let name = e.attr_local("name").unwrap_or("").to_string();
                    let local_sheet = e
                        .attr_local("localSheetId")
                        .and_then(|v| v.trim().parse().ok());
                    match xr.text_content(MAX_NAME_TEXT) {
                        Ok(text) => wb.names.push(DefinedName {
                            name,
                            local_sheet,
                            text,
                        }),
                        // An oversized name is dropped, not fatal.
                        Err(XlsxError::LimitExceeded { .. }) => xr.skip()?,
                        Err(other) => return Err(other),
                    }
                }
                _ => xr.skip()?,
            },
            Ev::Eof => return Ok(wb),
            _ => {}
        }
    }
}

/// `docProps/core.xml` / `docProps/app.xml` metadata.
pub(crate) fn parse_doc_props<R: BufRead>(
    xr: &mut XmlReader<R>,
    meta: &mut tally_model::DocMeta,
) -> Result<(), XlsxError> {
    const MAX: usize = 64 * 1024;
    loop {
        match xr.next()? {
            Ev::Start(e) => {
                let slot = match e.local() {
                    "coreProperties" | "Properties" => continue,
                    "title" => &mut meta.title,
                    "created" => &mut meta.created,
                    "modified" => &mut meta.modified,
                    "Application" => &mut meta.generator,
                    _ => {
                        xr.skip()?;
                        continue;
                    }
                };
                match xr.text_content(MAX) {
                    Ok(t) if !t.trim().is_empty() => *slot = Some(t.trim().to_string()),
                    Ok(_) => {}
                    Err(XlsxError::LimitExceeded { .. }) => xr.skip()?,
                    Err(other) => return Err(other),
                }
            }
            Ev::Eof => return Ok(()),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets() {
        assert_eq!(
            resolve_target("xl", "worksheets/sheet1.xml"),
            "xl/worksheets/sheet1.xml"
        );
        assert_eq!(
            resolve_target("xl", "/xl/worksheets/s.xml"),
            "xl/worksheets/s.xml"
        );
        assert_eq!(
            resolve_target("xl/worksheets", "../styles.xml"),
            "xl/styles.xml"
        );
        assert_eq!(resolve_target("xl", "../../etc/passwd"), "../etc/passwd");
        assert_eq!(resolve_target("", "xl/workbook.xml"), "xl/workbook.xml");
        assert_eq!(rels_path("xl/workbook.xml"), "xl/_rels/workbook.xml.rels");
        assert_eq!(part_dir("xl/workbook.xml"), "xl");
    }
}
