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

use std::collections::HashSet;
use std::sync::Arc;

use tally_api::{ApiError, LoadStylesOptions};
use tally_model::{CellStyleData, PageSetupData, StyleData, WorkbookData};
use tally_types::SheetId;

use crate::cellstyles::{
    after_table_change, builtin_data, check_name, is_builtin, rebase, table_of, MAX_CELL_STYLES,
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

fn lookup(list: &[CellStyleData], name: &str) -> Option<StyleData> {
    list.iter()
        .find(|s| s.name == name)
        .map(|s| s.style.clone())
        .or_else(|| builtin_data(name).map(|b| b.style))
}

fn parent_of<'a>(list: &'a [CellStyleData], name: &str) -> Option<&'a str> {
    list.iter()
        .find(|s| s.name == name)
        .and_then(|s| s.parent.as_deref())
}

/// Parent-chain depth of `name` in `list` (cut at [`MAX_DEPTH`]).
fn depth(list: &[CellStyleData], name: &str) -> usize {
    let mut d = 0;
    let mut cur = name;
    while let Some(p) = parent_of(list, cur) {
        d += 1;
        if d >= MAX_DEPTH || p == name {
            break;
        }
        cur = p;
    }
    d
}

/// `true` when `parent`'s chain in `list` reaches `name`.
fn loops(list: &[CellStyleData], name: &str, parent: &str) -> bool {
    let mut cur = Some(parent);
    for _ in 0..MAX_DEPTH {
        match cur {
            Some(c) if c == name => return true,
            Some(c) => cur = parent_of(list, c),
            None => return false,
        }
    }
    true
}

fn put(list: &mut Vec<CellStyleData>, entry: CellStyleData) {
    match list.iter_mut().find(|s| s.name == entry.name) {
        Some(s) => *s = entry,
        None => list.push(entry),
    }
}

/// Copies the source's cell styles (see the module docs); the inverse
/// change, or none when nothing changed.
fn load_cell_styles(
    wb: &mut Workbook,
    source: &[CellStyleData],
    overwrite: bool,
) -> Result<Vec<Change>, ApiError> {
    let src = table_of(source);
    let old = wb.cell_styles.clone();
    let exists = |name: &str| is_builtin(name) || old.iter().any(|s| s.name == name);
    let mut picked: Vec<&CellStyleData> = src
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
    picked.sort_by_key(|s| depth(&src, &s.name));
    let names: HashSet<&str> = picked.iter().map(|s| s.name.as_str()).collect();
    let mut work = table_of(&old);
    for s in &picked {
        let name = s.name.clone();
        let parent = if name == "Default" {
            None
        } else {
            let p = s
                .parent
                .clone()
                .filter(|p| *p != name && (lookup(&src, p).is_some() || lookup(&work, p).is_some()))
                .unwrap_or_else(|| "Default".into());
            Some(if loops(&work, &name, &p) {
                "Default".to_string()
            } else {
                p
            })
        };
        let mut style = match parent.as_deref() {
            Some(p) => match (lookup(&src, p), lookup(&work, p)) {
                (Some(theirs), Some(ours)) => rebase(&theirs, &s.style, &ours),
                _ => s.style.clone(),
            },
            None => s.style.clone(),
        };
        style.font.family = crate::fonts::own_family(style.font.family, wb.default_font.as_deref());
        style.style_name = Some(name.clone());
        put(
            &mut work,
            CellStyleData {
                name,
                parent,
                style,
            },
        );
    }
    // This document's other styles follow a replaced parent.
    let before = table_of(&old);
    let mut rest: Vec<String> = work
        .iter()
        .filter(|s| !names.contains(s.name.as_str()))
        .map(|s| s.name.clone())
        .collect();
    rest.sort_by_key(|n| depth(&work, n));
    for n in rest {
        let Some(child) = work.iter().find(|s| s.name == n).cloned() else {
            continue;
        };
        let Some(p) = child.parent.as_deref() else {
            continue;
        };
        let (Some(was), Some(now)) = (lookup(&before, p), lookup(&work, p)) else {
            continue;
        };
        if was == now {
            continue;
        }
        let mut style = rebase(&was, &child.style, &now);
        style.style_name = Some(child.name.clone());
        put(&mut work, CellStyleData { style, ..child });
    }
    // Built-ins are stored only when this document had them stored or
    // they now differ from AOO's.
    work.retain(|s| {
        !is_builtin(&s.name)
            || old.iter().any(|o| o.name == s.name)
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
