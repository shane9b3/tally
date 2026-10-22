//! Named cell styles (wave 5, W5-2 / W5-6D): `Default`, `Result`,
//! `Result2`, `Heading`, `Heading1` plus user styles, each with a parent
//! chain. A cell's `StyleData::style_name` names the style it is based
//! on; direct formatting sits on top. Redefining a style re-interns every
//! cell style based on it (the direct formatting, taken as what differs
//! from the old definition, is re-applied over the new one), and so do
//! the styles whose parent it is.
//!
//! Storage: `Workbook::cell_styles` (`tally_model::CellStyleData`, fully
//! resolved attributes). ODS `office:styles` with
//! `style:parent-style-name`; XLSX `cellStyleXfs`/`cellStyles`. Undo:
//! [`crate::undo::Change::CellStyles`].
//!
//! AutoFormat ([`apply_auto_format`]) applies one of AOO's 17 presets
//! (header row, body, total row, left label column; borders, fills, fonts
//! and number formats) as direct formatting.

use std::collections::HashMap;

use tally_api::{ApiError, AutoFormatOptions, CellStyleDto, RangeDto, StyleDto};
use tally_model::{BorderLine, BorderStyle, CellStyleData, DisplayData, HAlign, StyleData, VAlign};
use tally_types::{CellRange, SheetId, MAX_COLS, MAX_ROWS};

use crate::ops::{check_sheet, map_styles, to_range};
use crate::protection::check_format;
use crate::storage::StyleId;
use crate::styles::{border_from_dto, underline_from_dto, StyleTable};
use crate::undo::{Change, Effects};
use crate::workbook::Workbook;

/// The built-in style names, in Styles panel order.
pub const BUILTIN: [&str; 5] = ["Default", "Result", "Result2", "Heading", "Heading1"];

/// Most named styles per document.
pub const MAX_CELL_STYLES: usize = 10_000;

/// `true` for a built-in style name (case-sensitive, as AOO).
pub fn is_builtin(name: &str) -> bool {
    BUILTIN.contains(&name)
}

/// The resolved attributes of the built-in styles (AOO 4.1 `sc` defaults).
fn builtin_data(name: &str) -> Option<CellStyleData> {
    let mut s = StyleData::default();
    let parent = match name {
        "Default" => None,
        "Result" => {
            s.font.bold = true;
            s.font.italic = true;
            s.font.underline = true;
            Some("Default")
        }
        "Result2" => {
            s.font.bold = true;
            s.font.italic = true;
            s.font.underline = true;
            s.number_format = Some("[$$-409]#,##0.00;[RED]-[$$-409]#,##0.00".into());
            Some("Result")
        }
        "Heading" => {
            s.font.bold = true;
            s.font.italic = true;
            s.font.size_pt = Some(16.0);
            s.align.h = HAlign::Center;
            Some("Default")
        }
        "Heading1" => {
            s.font.bold = true;
            s.font.italic = true;
            s.font.size_pt = Some(16.0);
            s.align.h = HAlign::Center;
            s.align.rotation_deg = 90;
            Some("Heading")
        }
        _ => return None,
    };
    s.style_name = Some(name.to_string());
    Some(CellStyleData {
        name: name.to_string(),
        parent: parent.map(str::to_string),
        style: s,
    })
}

/// The style table with the built-ins first (the stored table may hold
/// only user styles, or redefined built-ins).
pub fn table(wb: &Workbook) -> Vec<CellStyleData> {
    let mut out: Vec<CellStyleData> = Vec::with_capacity(wb.cell_styles.len() + 5);
    for b in BUILTIN {
        match wb.cell_styles.iter().find(|s| s.name == b) {
            Some(s) => out.push(s.clone()),
            None => out.extend(builtin_data(b)),
        }
    }
    for s in &wb.cell_styles {
        if !is_builtin(&s.name) && !out.iter().any(|x| x.name == s.name) {
            out.push(s.clone());
        }
    }
    out
}

/// The resolved attributes of `name`, if defined.
pub fn resolved(wb: &Workbook, name: &str) -> Option<StyleData> {
    wb.cell_styles
        .iter()
        .find(|s| s.name == name)
        .map(|s| s.style.clone())
        .or_else(|| builtin_data(name).map(|b| b.style))
        .map(|mut s| {
            s.style_name = Some(name.to_string());
            s
        })
}

/// The API form of the style table, `Default` first.
pub fn to_api(wb: &Workbook) -> Vec<CellStyleDto> {
    table(wb)
        .into_iter()
        .map(|s| {
            let code = s
                .style
                .number_format
                .clone()
                .unwrap_or_else(|| "General".into());
            CellStyleDto {
                builtin: is_builtin(&s.name),
                style: crate::styles::style_dto(0, &s.style, &code),
                name: s.name,
                parent: s.parent,
            }
        })
        .collect()
}

/// Model attributes from a (fully resolved) API style.
pub fn data_from_dto(d: &StyleDto) -> StyleData {
    let mut s = StyleData::default();
    s.font.bold = d.bold;
    s.font.italic = d.italic;
    s.font.underline = d.underline;
    s.font.strike = d.strike;
    s.font.family = d.font_family.clone();
    s.font.size_pt = (d.font_size_pt > 0.0
        && d.font_size_pt.is_finite()
        && d.font_size_pt != crate::styles::DEFAULT_FONT_SIZE_PT)
        .then_some(d.font_size_pt);
    s.font.color = d.color;
    s.font.underline_style = d
        .underline_style
        .map(underline_from_dto)
        .unwrap_or_default();
    s.fill = d.fill;
    s.align.h = match d.h_align {
        tally_api::HAlign::General => HAlign::General,
        tally_api::HAlign::Left => HAlign::Left,
        tally_api::HAlign::Center => HAlign::Center,
        tally_api::HAlign::Right => HAlign::Right,
        tally_api::HAlign::Justify => HAlign::Justify,
    };
    s.align.v = match d.v_align {
        tally_api::VAlign::Bottom => VAlign::Bottom,
        tally_api::VAlign::Middle => VAlign::Middle,
        tally_api::VAlign::Top => VAlign::Top,
    };
    s.align.wrap = d.wrap;
    s.align.indent = d.indent;
    s.align.rotation_deg = d.rotation_deg.unwrap_or(0);
    s.align.stacked = d.stacked.unwrap_or(false);
    s.align.shrink_to_fit = d.shrink_to_fit.unwrap_or(false);
    s.borders.top = d.borders.top.as_ref().map(border_from_dto);
    s.borders.bottom = d.borders.bottom.as_ref().map(border_from_dto);
    s.borders.left = d.borders.left.as_ref().map(border_from_dto);
    s.borders.right = d.borders.right.as_ref().map(border_from_dto);
    s.borders.diagonal_down = d.borders.diagonal_down.as_ref().map(border_from_dto);
    s.borders.diagonal_up = d.borders.diagonal_up.as_ref().map(border_from_dto);
    let nf = d.number_format.trim();
    s.number_format = (!nf.is_empty() && nf != "General").then(|| nf.to_string());
    s.display = d.display.as_ref().map(|x| match x {
        tally_api::DisplayDto::Chip { bg, fg } => DisplayData::Chip { bg: *bg, fg: *fg },
    });
    s
}

/// `cell` re-based from `old_base` onto `new_base`: every attribute the
/// cell shares with its old base follows the new base; the rest is direct
/// formatting and stays.
pub fn rebase(old_base: &StyleData, cell: &StyleData, new_base: &StyleData) -> StyleData {
    macro_rules! pick {
        ($($path:ident).+) => {
            if cell.$($path).+ == old_base.$($path).+ {
                new_base.$($path).+.clone()
            } else {
                cell.$($path).+.clone()
            }
        };
    }
    let mut s = StyleData::default();
    s.font.bold = pick!(font.bold);
    s.font.italic = pick!(font.italic);
    s.font.underline = pick!(font.underline);
    s.font.strike = pick!(font.strike);
    s.font.family = pick!(font.family);
    s.font.size_pt = pick!(font.size_pt);
    s.font.color = pick!(font.color);
    s.font.underline_style = pick!(font.underline_style);
    s.fill = pick!(fill);
    s.align.h = pick!(align.h);
    s.align.v = pick!(align.v);
    s.align.wrap = pick!(align.wrap);
    s.align.indent = pick!(align.indent);
    s.align.rotation_deg = pick!(align.rotation_deg);
    s.align.stacked = pick!(align.stacked);
    s.align.shrink_to_fit = pick!(align.shrink_to_fit);
    s.borders.top = pick!(borders.top);
    s.borders.bottom = pick!(borders.bottom);
    s.borders.left = pick!(borders.left);
    s.borders.right = pick!(borders.right);
    s.borders.diagonal_down = pick!(borders.diagonal_down);
    s.borders.diagonal_up = pick!(borders.diagonal_up);
    s.number_format = pick!(number_format);
    s.display = pick!(display);
    s.protection = pick!(protection);
    s.style_name = new_base.style_name.clone();
    s
}

/// Replaces the style of every cell of every sheet by `f(table, old)`.
pub fn remap_all(wb: &mut Workbook, mut f: impl FnMut(&mut StyleTable, StyleId) -> StyleId) {
    let Workbook { sheets, styles, .. } = wb;
    let mut cache: HashMap<StyleId, StyleId> = HashMap::new();
    let mut touched: Vec<SheetId> = Vec::new();
    for sheet in sheets.iter_mut() {
        let mut hit = false;
        for col in &mut sheet.columns {
            if col.styles.is_empty() {
                continue;
            }
            col.styles.map(0, MAX_ROWS - 1, |old| {
                let new = *cache.entry(old).or_insert_with(|| f(styles, old));
                hit |= new != old;
                new
            });
        }
        if hit {
            touched.push(sheet.id);
        }
    }
    for id in touched {
        wb.pending
            .display_only
            .push(CellRange::new(id, 0, 0, MAX_ROWS - 1, MAX_COLS - 1));
    }
}

/// Re-bases every cell style that is based on a style of `old` whose
/// definition differs in the current table (after a define, a delete or
/// an undo).
pub fn after_table_change(wb: &mut Workbook, old: &[CellStyleData]) {
    let lookup = |list: &[CellStyleData], name: &str| -> Option<StyleData> {
        list.iter()
            .find(|s| s.name == name)
            .map(|s| s.style.clone())
            .or_else(|| builtin_data(name).map(|b| b.style))
            .map(|mut s| {
                s.style_name = Some(name.to_string());
                s
            })
    };
    let mut names: Vec<String> = old.iter().map(|s| s.name.clone()).collect();
    names.extend(wb.cell_styles.iter().map(|s| s.name.clone()));
    names.extend(BUILTIN.iter().map(|s| s.to_string()));
    names.sort();
    names.dedup();
    let mut changed: HashMap<String, (StyleData, StyleData)> = HashMap::new();
    for n in names {
        let before = lookup(old, &n);
        let after = lookup(&wb.cell_styles, &n);
        match (before, after) {
            (Some(b), Some(a)) if b != a => {
                changed.insert(n, (b, a));
            }
            (Some(b), None) => {
                // Deleted: fall back to Default.
                let a = lookup(&wb.cell_styles, "Default").unwrap_or_default();
                changed.insert(n, (b, a));
            }
            _ => {}
        }
    }
    if changed.is_empty() {
        return;
    }
    remap_all(wb, |table, id| {
        let data = table.get(id).data.clone();
        let Some(name) = data.style_name.as_deref() else {
            return id;
        };
        let Some((b, a)) = changed.get(name) else {
            return id;
        };
        table.intern(rebase(b, &data, a))
    });
}

fn check_name(name: &str) -> Result<(), ApiError> {
    let n = name.trim();
    if n.is_empty() || n.chars().count() > 255 || n.contains(['\u{0}', '\n', '\r']) {
        return Err(ApiError::invalid_argument("bad style name"));
    }
    Ok(())
}

/// `Op::DefineCellStyle`: defines or redefines a named style; children
/// and cells based on it follow.
pub fn define(wb: &mut Workbook, dto: &CellStyleDto) -> Result<Vec<Change>, ApiError> {
    check_name(&dto.name)?;
    let name = dto.name.trim().to_string();
    if name == "Default" && dto.parent.is_some() {
        return Err(ApiError::invalid_argument("Default has no parent"));
    }
    if let Some(p) = dto.parent.as_deref() {
        if p == name {
            return Err(ApiError::invalid_argument(
                "a style cannot be its own parent",
            ));
        }
        if resolved(wb, p).is_none() {
            return Err(ApiError::not_found(format!("no style {p:?}")));
        }
        // No cycles through the parent chain.
        let mut cur = Some(p.to_string());
        let mut depth = 0;
        while let Some(c) = cur {
            if c == name {
                return Err(ApiError::invalid_argument("the parent chain would loop"));
            }
            depth += 1;
            if depth > 64 {
                break;
            }
            cur = table(wb)
                .iter()
                .find(|s| s.name == c)
                .and_then(|s| s.parent.clone());
        }
    }
    let exists = wb.cell_styles.iter().any(|s| s.name == name);
    if !exists && !is_builtin(&name) && wb.cell_styles.len() >= MAX_CELL_STYLES {
        return Err(ApiError::invalid_argument(format!(
            "at most {MAX_CELL_STYLES} cell styles"
        )));
    }
    let mut style = data_from_dto(&dto.style);
    style.font.family = crate::fonts::own_family(style.font.family, wb.default_font.as_deref());
    style.style_name = Some(name.clone());
    let parent = if name == "Default" {
        None
    } else {
        Some(dto.parent.clone().unwrap_or_else(|| "Default".into()))
    };
    let entry = CellStyleData {
        name: name.clone(),
        parent,
        style,
    };
    let old = wb.cell_styles.clone();
    match wb.cell_styles.iter_mut().find(|s| s.name == name) {
        Some(s) => *s = entry,
        None => wb.cell_styles.push(entry),
    }
    propagate_to_children(wb, &old);
    after_table_change(wb, &old);
    Ok(vec![Change::CellStyles(old)])
}

/// Children of redefined styles are re-based onto the new parent
/// (attributes they shared with the old parent follow it).
fn propagate_to_children(wb: &mut Workbook, old: &[CellStyleData]) {
    for _ in 0..16 {
        let current = table(wb);
        let mut updates: Vec<CellStyleData> = Vec::new();
        for child in &current {
            let Some(p) = child.parent.as_deref() else {
                continue;
            };
            let old_parent = old
                .iter()
                .find(|s| s.name == p)
                .map(|s| s.style.clone())
                .or_else(|| builtin_data(p).map(|b| b.style));
            let new_parent = current
                .iter()
                .find(|s| s.name == p)
                .map(|s| s.style.clone());
            let (Some(op), Some(np)) = (old_parent, new_parent) else {
                continue;
            };
            if op == np {
                continue;
            }
            let mut re = rebase(&op, &child.style, &np);
            re.style_name = Some(child.name.clone());
            if re != child.style {
                updates.push(CellStyleData {
                    name: child.name.clone(),
                    parent: child.parent.clone(),
                    style: re,
                });
            }
        }
        if updates.is_empty() {
            return;
        }
        for u in updates {
            match wb.cell_styles.iter_mut().find(|s| s.name == u.name) {
                Some(s) => *s = u,
                None => wb.cell_styles.push(u),
            }
        }
    }
}

/// `Op::DeleteCellStyle`: cells using the style fall back to its parent.
pub fn delete(wb: &mut Workbook, name: &str) -> Result<Vec<Change>, ApiError> {
    if is_builtin(name) {
        return Err(ApiError::invalid_argument(format!(
            "{name} is a built-in style and cannot be deleted"
        )));
    }
    let Some(pos) = wb.cell_styles.iter().position(|s| s.name == name) else {
        return Err(ApiError::not_found(format!("no style {name:?}")));
    };
    let old = wb.cell_styles.clone();
    let removed = wb.cell_styles.remove(pos);
    let parent = removed.parent.clone().unwrap_or_else(|| "Default".into());
    let parent_data = resolved(wb, &parent).unwrap_or_default();
    // Children of the deleted style move up to its parent.
    for s in wb.cell_styles.iter_mut() {
        if s.parent.as_deref() == Some(name) {
            s.parent = Some(parent.clone());
        }
    }
    let base = removed.style.clone();
    remap_all(wb, |table, id| {
        let data = table.get(id).data.clone();
        if data.style_name.as_deref() != Some(name) {
            return id;
        }
        table.intern(rebase(&base, &data, &parent_data))
    });
    Ok(vec![Change::CellStyles(old)])
}

/// `Op::ApplyCellStyle`: the cells take the style (direct formatting is
/// cleared).
pub fn apply(
    wb: &mut Workbook,
    sheet: SheetId,
    ranges: &[RangeDto],
    name: &str,
) -> Result<Vec<Change>, ApiError> {
    check_sheet(wb, sheet)?;
    let ranges: Vec<CellRange> = ranges
        .iter()
        .map(|r| to_range(sheet, r))
        .collect::<Result<_, _>>()?;
    check_format(wb, &ranges)?;
    let data =
        resolved(wb, name).ok_or_else(|| ApiError::not_found(format!("no style {name:?}")))?;
    let mut out = Vec::with_capacity(ranges.len());
    for r in &ranges {
        out.push(Change::Range(crate::undo::capture_range(
            wb, *r, false, true,
        )));
        let d = data.clone();
        map_styles(wb, *r, |table, _| table.intern(d.clone()));
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// AutoFormat
// ---------------------------------------------------------------------------

/// One AutoFormat preset: fills and font colours of the header row, the
/// body, the total row and the label column, the border style and the
/// number format of the body (`None` = General).
struct Preset {
    name: &'static str,
    header_fill: Option<u32>,
    header_color: Option<u32>,
    header_bold: bool,
    body_fill: Option<u32>,
    total_fill: Option<u32>,
    total_bold: bool,
    label_fill: Option<u32>,
    border: Option<BorderStyle>,
    number_format: Option<&'static str>,
    font: Option<&'static str>,
}

const CURRENCY: &str = "[$$-409]#,##0.00;[RED]-[$$-409]#,##0.00";

fn presets() -> Vec<Preset> {
    let plain = |name, header_fill, body_fill, total_fill, header_color, nf| Preset {
        name,
        header_fill,
        header_color,
        header_bold: true,
        body_fill,
        total_fill,
        total_bold: true,
        label_fill: None,
        border: Some(BorderStyle::Solid),
        number_format: nf,
        font: None,
    };
    vec![
        Preset {
            name: "Default",
            header_fill: None,
            header_color: None,
            header_bold: true,
            body_fill: None,
            total_fill: None,
            total_bold: true,
            label_fill: None,
            border: Some(BorderStyle::Solid),
            number_format: None,
            font: None,
        },
        plain(
            "3D",
            Some(0x8080_80ff),
            Some(0xc0c0_c0ff),
            Some(0xc0c0_c0ff),
            Some(0xffff_ffff),
            None,
        ),
        plain(
            "Black 1",
            Some(0x0000_00ff),
            None,
            Some(0x0000_00ff),
            Some(0xffff_ffff),
            None,
        ),
        plain(
            "Black 2",
            Some(0x0000_00ff),
            Some(0xc0c0_c0ff),
            Some(0xc0c0_c0ff),
            Some(0xffff_ffff),
            None,
        ),
        plain(
            "Blue",
            Some(0x0000_80ff),
            Some(0xccff_ffff),
            Some(0x0000_80ff),
            Some(0xffff_ffff),
            None,
        ),
        plain(
            "Brown",
            Some(0x8000_00ff),
            Some(0xffcc_99ff),
            Some(0x8000_00ff),
            Some(0xffff_ffff),
            None,
        ),
        plain("Currency", None, None, None, None, Some(CURRENCY)),
        plain(
            "Currency 3D",
            Some(0x8080_80ff),
            Some(0xc0c0_c0ff),
            Some(0xc0c0_c0ff),
            Some(0xffff_ffff),
            Some(CURRENCY),
        ),
        plain(
            "Currency Gray",
            Some(0x8080_80ff),
            Some(0xe6e6_e6ff),
            Some(0x8080_80ff),
            Some(0xffff_ffff),
            Some(CURRENCY),
        ),
        plain(
            "Currency Lavender",
            Some(0x6666_99ff),
            Some(0xe6e6_ffff),
            Some(0x6666_99ff),
            Some(0xffff_ffff),
            Some(CURRENCY),
        ),
        plain(
            "Currency Turquoise",
            Some(0x0080_80ff),
            Some(0xccff_ffff),
            Some(0x0080_80ff),
            Some(0xffff_ffff),
            Some(CURRENCY),
        ),
        plain(
            "Gray",
            Some(0x8080_80ff),
            Some(0xe6e6_e6ff),
            Some(0x8080_80ff),
            Some(0xffff_ffff),
            None,
        ),
        plain(
            "Green",
            Some(0x0080_00ff),
            Some(0xccff_ccff),
            Some(0x0080_00ff),
            Some(0xffff_ffff),
            None,
        ),
        plain(
            "Lavender",
            Some(0x6666_99ff),
            Some(0xe6e6_ffff),
            Some(0x6666_99ff),
            Some(0xffff_ffff),
            None,
        ),
        plain(
            "Red",
            Some(0xff00_00ff),
            Some(0xffcc_ccff),
            Some(0xff00_00ff),
            Some(0xffff_ffff),
            None,
        ),
        plain(
            "Turquoise",
            Some(0x0080_80ff),
            Some(0xccff_ffff),
            Some(0x0080_80ff),
            Some(0xffff_ffff),
            None,
        ),
        plain(
            "Yellow",
            Some(0xffcc_00ff),
            Some(0xffff_ccff),
            Some(0xffcc_00ff),
            Some(0x0000_00ff),
            None,
        ),
    ]
}

/// AOO's AutoFormat preset names, in dialog order.
pub fn preset_names() -> Vec<&'static str> {
    presets().iter().map(|p| p.name).collect()
}

/// `Op::ApplyAutoFormat`.
pub fn apply_auto_format(
    wb: &mut Workbook,
    sheet: SheetId,
    range: &RangeDto,
    preset: &str,
    options: &AutoFormatOptions,
    fx: &mut Effects,
) -> Result<Vec<Change>, ApiError> {
    check_sheet(wb, sheet)?;
    let range = to_range(sheet, range)?;
    check_format(wb, &[range])?;
    let presets = presets();
    let p = presets
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(preset))
        .ok_or_else(|| ApiError::not_found(format!("no AutoFormat preset {preset:?}")))?;
    if range.cell_count() > crate::ops::MAX_OP_CELLS {
        return Err(ApiError::invalid_argument("the range is too large"));
    }
    // AOO offers AutoFormat for at least 3 × 3 cells
    // (`sc/source/ui/view/cellsh3.cxx:849-850`).
    if range.rows() < 3 || range.cols() < 3 {
        return Err(ApiError::invalid_argument(
            "AutoFormat needs a range of at least 3 × 3 cells",
        ));
    }
    let is_default = p.name == "Default";
    let snap = crate::undo::capture_range(wb, range, false, true);
    let line = |style: BorderStyle| BorderLine {
        style,
        width_hmm: 26,
        color: 0x0000_00ff,
    };
    let rows = range.rows();
    let cols = range.cols();
    for dr in 0..rows {
        for dc in 0..cols {
            let r = range.r0 + dr;
            let c = range.c0 + dc;
            let cell = CellRange::new(sheet, r, c, r, c);
            let header = dr == 0;
            let total = rows > 1 && dr == rows - 1;
            let label = cols > 1 && dc == 0 && !header;
            let (fill, color, bold) = if is_default {
                default_preset_cell(header, total, dc == 0, dc == cols - 1)
            } else if header {
                (p.header_fill, p.header_color, p.header_bold)
            } else if total {
                (
                    p.total_fill,
                    p.header_color.filter(|_| p.total_fill.is_some()),
                    p.total_bold,
                )
            } else if label {
                (p.label_fill.or(p.body_fill), None, false)
            } else {
                (p.body_fill, None, false)
            };
            map_styles(wb, cell, |table, old| {
                let mut d = table.get(old).data.clone();
                if options.pattern {
                    d.fill = fill;
                }
                if options.font {
                    d.font.bold = bold;
                    d.font.color = color;
                    if let Some(f) = p.font {
                        d.font.family = Some(f.to_string());
                    }
                    if is_default {
                        d.font.size_pt = Some(10.0);
                    }
                }
                if options.alignment {
                    d.align.h = if header {
                        HAlign::Center
                    } else {
                        HAlign::General
                    };
                }
                if options.borders {
                    let b = p.border.map(line);
                    d.borders.top = b.clone();
                    d.borders.bottom = b.clone();
                    d.borders.left = b.clone();
                    d.borders.right = b;
                }
                if options.number_format && !header && !label {
                    d.number_format = p.number_format.map(str::to_string);
                }
                table.intern(d)
            });
        }
    }
    let mut out = vec![Change::Range(snap)];
    if options.autofit {
        // Optimal column widths and row heights for the range
        // (`ScDocFunc::AutoFormat` with `bSize`, `docfunc.cxx:3742-3758`).
        out.extend(fit_range(wb, range));
        fx.layout(sheet);
    }
    Ok(out)
}

/// AOO's built-in "Default" AutoFormat (`ScAutoFormat::ScAutoFormat`,
/// `sc/source/core/tool/autoform.cxx:939-991`): white on blue header
/// row, white on gray 70 % left column, black on gray 20 % right column
/// and bottom row, black on white body, 10 pt, thin black borders.
fn default_preset_cell(
    header: bool,
    bottom: bool,
    left: bool,
    right: bool,
) -> (Option<u32>, Option<u32>, bool) {
    const BLUE: u32 = 0x0000_80ff; // COL_BLUE
    const WHITE: u32 = 0xffff_ffff;
    const BLACK: u32 = 0x0000_00ff;
    const GRAY70: u32 = 0x4d4d_4dff;
    const GRAY20: u32 = 0xcccc_ccff;
    if header {
        (Some(BLUE), Some(WHITE), false)
    } else if left {
        (Some(GRAY70), Some(WHITE), false)
    } else if right || bottom {
        (Some(GRAY20), Some(BLACK), false)
    } else {
        (Some(WHITE), Some(BLACK), false)
    }
}

/// Sets the optimal widths of the range's columns and heights of its
/// rows; returns the undo entries.
fn fit_range(wb: &mut Workbook, range: CellRange) -> Vec<Change> {
    let Some(s) = wb.sheet(range.sheet) else {
        return Vec::new();
    };
    let out = vec![
        Change::ColDims {
            sheet: range.sheet,
            dims: s.col_dims.clone(),
        },
        Change::RowDims {
            sheet: range.sheet,
            dims: s.row_dims.clone(),
        },
    ];
    // One font context for the whole AutoFit.
    let mut m = crate::fonts::Measurer::new();
    let widths: Vec<(u32, f32)> = (range.c0..=range.c1)
        .filter_map(|c| {
            Some((
                c,
                crate::autofit::needed_width_px(&mut m, wb, s, c, range.r0, range.r1)?,
            ))
        })
        .collect();
    let mut heights: Vec<(u32, f32)> = Vec::new();
    if let Some(s) = wb.sheet_mut(range.sheet) {
        for (c, w) in widths {
            s.col_dims.set(c, c, crate::units::px_to_hmm(w), false);
        }
    }
    if let Some(s) = wb.sheet(range.sheet) {
        use crate::storage::ColumnStore;
        // Rows with content in the range (the others keep their height).
        let mut rows: Vec<u32> = Vec::new();
        for c in range.c0..=range.c1 {
            if let Some(col) = s.column(c) {
                rows.extend(col.cells.iter_range(range.r0, range.r1).map(|(r, _)| r));
            }
        }
        rows.sort_unstable();
        rows.dedup();
        for r in rows {
            let h = crate::autofit::needed_height_px(&mut m, wb, s, r)
                .unwrap_or(crate::units::DEFAULT_ROW_HEIGHT_PX);
            heights.push((r, h));
        }
    }
    if let Some(s) = wb.sheet_mut(range.sheet) {
        for (r, h) in heights {
            if !s.row_dims.is_hidden(r) {
                s.row_dims.set(r, r, crate::units::px_to_hmm(h), false);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rebase_keeps_direct_formatting() {
        let mut old = StyleData::default();
        old.font.bold = true;
        let mut cell = old.clone();
        cell.fill = Some(0xff0000ff);
        let mut new = StyleData::default();
        new.font.italic = true;
        let r = rebase(&old, &cell, &new);
        assert!(r.font.italic && !r.font.bold);
        assert_eq!(r.fill, Some(0xff0000ff));
    }

    #[test]
    fn presets_match_the_dialog() {
        assert_eq!(preset_names(), crate::tools::AUTOFORMAT_PRESETS.to_vec());
    }
}
