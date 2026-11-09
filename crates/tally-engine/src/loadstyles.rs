//! Load Styles: copies the cell styles and the page format of another
//! document into this one (AOO's `XStyleLoader::loadStylesFromURL`,
//! `ScDocShell::LoadStylesArgs` in `sc/source/ui/docshell/docsh6.cxx`).
//!
//! As in AOO:
//! - every source style of the chosen families is looked up by name; a
//!   new name is added, an existing one is replaced only with
//!   `overwrite` (built-in styles always exist here);
//! - all styles are created before any is filled, so a copied style keeps
//!   its parent by name, and a parent that was not replaced is this
//!   document's own: the attributes the source style set itself come from
//!   the source, the inherited ones from the parent here;
//! - cells keep their style by name, so cells based on a replaced style
//!   show the new definition with their direct formatting on top
//!   (`UpdStlShtPtrsFrmNms`), and so do this document's styles whose
//!   parent was replaced;
//! - number formats travel as format codes (Tally stores codes, so the
//!   source's formats need no merging into a formatter table).
//!
//! Tally keeps one page setup per sheet instead of named page styles: the
//! source's page style is its first sheet's page format, and with
//! `overwrite` it replaces the page-style part (paper, margins, scaling,
//! headers and footers, print options) of every sheet here. Print ranges,
//! repeated rows / columns and manual breaks are sheet attributes in AOO
//! and stay. Without `overwrite` the page format is not copied: every
//! document has the `Default` page style already.
//!
//! Unlike AOO (whose `LoadStylesArgs` records no undo action) the whole
//! load is one undo step.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use tally_api::{ApiError, LoadStylesOptions};
use tally_model::{CellStyleData, PageSetupData, StyleData, WorkbookData};
use tally_types::SheetId;

use crate::cellstyles::{
    after_table_change, builtin_data, check_name, is_builtin, rebase, table_of, BUILTIN,
    MAX_CELL_STYLES,
};
use crate::undo::{Change, Effects};
use crate::workbook::Workbook;

/// Longest parent chain followed (deeper chains are cut at the root).
const MAX_DEPTH: usize = 64;

/// The styles another document offers.
#[derive(Clone, Debug, Default)]
pub struct StyleSource {
    /// Its stored cell styles (built-ins it did not redefine are AOO's).
    pub cell_styles: Vec<CellStyleData>,
    /// Its page style: the first sheet's page setup.
    pub page_setup: Option<PageSetupData>,
}

impl StyleSource {
    /// The styles of a decoded file.
    pub fn from_data(data: &WorkbookData) -> Self {
        Self {
            cell_styles: data.cell_styles.clone(),
            page_setup: data
                .sheets
                .first()
                .map(|s| s.page_setup.clone().unwrap_or_default()),
        }
    }

    /// The styles of a workbook (a built-in template).
    pub fn from_workbook(wb: &Workbook) -> Self {
        Self {
            cell_styles: wb.cell_styles.clone(),
            page_setup: wb.sheets.first().map(|s| (*s.meta.page_setup).clone()),
        }
    }
}

/// A style list with a name index (lists hold up to [`MAX_CELL_STYLES`]
/// entries, so linear lookups would make the load quadratic).
struct Indexed {
    list: Vec<CellStyleData>,
    at: HashMap<String, usize>,
}

impl Indexed {
    fn new(list: Vec<CellStyleData>) -> Self {
        let mut at = HashMap::with_capacity(list.len());
        for (i, s) in list.iter().enumerate() {
            at.entry(s.name.clone()).or_insert(i);
        }
        Self { list, at }
    }

    fn get(&self, name: &str) -> Option<&CellStyleData> {
        self.at.get(name).map(|&i| &self.list[i])
    }

    fn lookup(&self, name: &str) -> Option<StyleData> {
        self.get(name)
            .map(|s| s.style.clone())
            .or_else(|| builtin_data(name).map(|b| b.style))
    }

    fn parent_of(&self, name: &str) -> Option<&str> {
        self.get(name).and_then(|s| s.parent.as_deref())
    }

    /// Parent-chain depth of `name` (cut at [`MAX_DEPTH`]).
    fn depth(&self, name: &str) -> usize {
        let mut d = 0;
        let mut cur = name;
        while let Some(p) = self.parent_of(cur) {
            d += 1;
            if d >= MAX_DEPTH || p == name {
                break;
            }
            cur = p;
        }
        d
    }

    /// `true` when `parent`'s chain reaches `name`.
    fn loops(&self, name: &str, parent: &str) -> bool {
        let mut cur = Some(parent);
        for _ in 0..MAX_DEPTH {
            match cur {
                Some(c) if c == name => return true,
                Some(c) => cur = self.parent_of(c),
                None => return false,
            }
        }
        true
    }

    fn put(&mut self, entry: CellStyleData) {
        match self.at.get(&entry.name) {
            Some(&i) => self.list[i] = entry,
            None => {
                self.at.insert(entry.name.clone(), self.list.len());
                self.list.push(entry);
            }
        }
    }
}

/// This document's stored styles in their stored order, then the
/// built-ins it does not store (so they can follow a replaced parent;
/// unchanged ones are dropped again before the comparison with the old
/// table, so a load that changes nothing leaves the table as it was).
fn work_of(old: &[CellStyleData]) -> Vec<CellStyleData> {
    let mut work = old.to_vec();
    for b in BUILTIN {
        if !old.iter().any(|s| s.name == b) {
            work.extend(builtin_data(b));
        }
    }
    work
}

/// Copies the source's cell styles (see the module docs); the inverse
/// change, or none when nothing changed.
fn load_cell_styles(
    wb: &mut Workbook,
    source: &[CellStyleData],
    overwrite: bool,
) -> Result<Vec<Change>, ApiError> {
    let src = Indexed::new(table_of(source));
    let old = wb.cell_styles.clone();
    let old_names: HashSet<&str> = old.iter().map(|s| s.name.as_str()).collect();
    let exists = |name: &str| is_builtin(name) || old_names.contains(name);
    let mut picked: Vec<&CellStyleData> = src
        .list
        .iter()
        .filter(|s| check_name(&s.name).is_ok() && s.name.trim() == s.name)
        .filter(|s| overwrite || !exists(&s.name))
        .collect();
    let added = picked.iter().filter(|s| !exists(&s.name)).count();
    if old.len() + added > MAX_CELL_STYLES {
        return Err(ApiError::invalid_argument(format!(
            "at most {MAX_CELL_STYLES} cell styles"
        )));
    }
    // Parents before children, so a copied parent is final when its
    // children are re-based on it.
    picked.sort_by_cached_key(|s| src.depth(&s.name));
    let names: HashSet<&str> = picked.iter().map(|s| s.name.as_str()).collect();
    let before = Indexed::new(work_of(&old));
    let mut work = Indexed::new(before.list.clone());
    for s in &picked {
        let name = s.name.clone();
        let parent = if name == "Default" {
            None
        } else {
            let p = s
                .parent
                .clone()
                .filter(|p| *p != name && (src.lookup(p).is_some() || work.lookup(p).is_some()))
                .unwrap_or_else(|| "Default".into());
            Some(if work.loops(&name, &p) {
                "Default".to_string()
            } else {
                p
            })
        };
        let mut style = match parent.as_deref() {
            Some(p) => match (src.lookup(p), work.lookup(p)) {
                (Some(theirs), Some(ours)) => rebase(&theirs, &s.style, &ours),
                _ => s.style.clone(),
            },
            None => s.style.clone(),
        };
        style.font.family = crate::fonts::own_family(style.font.family, wb.default_font.as_deref());
        style.style_name = Some(name.clone());
        work.put(CellStyleData {
            name,
            parent,
            style,
        });
    }
    // This document's other styles follow a replaced parent.
    let mut rest: Vec<(usize, usize)> = work
        .list
        .iter()
        .enumerate()
        .filter(|(_, s)| !names.contains(s.name.as_str()))
        .map(|(i, s)| (work.depth(&s.name), i))
        .collect();
    rest.sort_by_key(|&(d, _)| d);
    for (_, i) in rest {
        let child = work.list[i].clone();
        let Some(p) = child.parent.as_deref() else {
            continue;
        };
        let (Some(was), Some(now)) = (before.lookup(p), work.lookup(p)) else {
            continue;
        };
        if was == now {
            continue;
        }
        let mut style = rebase(&was, &child.style, &now);
        style.style_name = Some(child.name.clone());
        work.put(CellStyleData { style, ..child });
    }
    // Built-ins are stored only when this document had them stored or
    // they now differ from AOO's.
    let mut work = work.list;
    work.retain(|s| {
        !is_builtin(&s.name)
            || old_names.contains(s.name.as_str())
            || builtin_data(&s.name).as_ref() != Some(s)
    });
    if work == old {
        return Ok(Vec::new());
    }
    wb.cell_styles = work;
    after_table_change(wb, &old);
    Ok(vec![Change::CellStyles(old)])
}

/// The page-style part of `src` on a sheet whose setup is `cur`.
fn page_style_on(src: &PageSetupData, cur: &PageSetupData) -> PageSetupData {
    PageSetupData {
        print_range: cur.print_range,
        repeat_rows: cur.repeat_rows,
        repeat_cols: cur.repeat_cols,
        excel_margins: None,
        ..src.clone()
    }
}

/// Copies the source's page format onto every sheet (sheets whose
/// protection forbids formatting are skipped with a warning).
fn load_page_style(
    wb: &mut Workbook,
    src: &PageSetupData,
    fx: &mut Effects,
) -> Result<Vec<Change>, ApiError> {
    let ids: Vec<SheetId> = wb.sheets.iter().map(|s| s.id).collect();
    let mut out = Vec::new();
    let mut skipped = 0usize;
    for id in ids {
        if crate::protection::check_sheet_action(wb, id, |p| p.format_cells).is_err() {
            skipped += 1;
            continue;
        }
        let Some(cur) = wb.sheet(id).map(|s| s.meta.page_setup.clone()) else {
            continue;
        };
        let new = page_style_on(src, &cur);
        if new == *cur {
            continue;
        }
        out.extend(crate::ops_v03::with_meta(wb, id, fx, |_, meta| {
            meta.page_setup = Arc::new(new);
            Ok(())
        })?);
        fx.layout(id);
    }
    if skipped > 0 {
        fx.warnings.push(ApiError::new(
            crate::protection::PROTECTED,
            format!("The page format was not changed on {skipped} protected sheet(s)."),
        ));
    }
    Ok(out)
}

/// Load Styles (see the module docs): the inverse changes, in order.
/// Nothing is changed when an error is returned.
pub fn load(
    wb: &mut Workbook,
    source: &StyleSource,
    opts: &LoadStylesOptions,
    fx: &mut Effects,
) -> Result<Vec<Change>, ApiError> {
    let mut changes = Vec::new();
    if opts.cell_styles {
        changes.extend(load_cell_styles(wb, &source.cell_styles, opts.overwrite)?);
    }
    if opts.page_styles && opts.overwrite {
        if let Some(p) = &source.page_setup {
            match load_page_style(wb, p, fx) {
                Ok(c) => changes.extend(c),
                Err(e) => {
                    for ch in changes.into_iter().rev() {
                        let _ = crate::undo::apply_change(wb, ch, fx);
                    }
                    return Err(e);
                }
            }
        }
    }
    Ok(changes)
}
