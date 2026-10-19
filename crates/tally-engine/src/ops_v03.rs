//! The v0.3 ops: conditional formats, validations, notes, links,
//! protection, page setup and breaks, document properties and row
//! auto-height. Each records the undo state it replaces (the sheet meta,
//! the styles of a range, the row heights or the document metadata).

use std::sync::Arc;

use tally_api::{ApiError, BreakAxis, DimRunPx, Op, ProtectionOptions};
use tally_model::CellProtectionData;
use tally_types::{CellRange, SheetId, MAX_COLS, MAX_ROWS};

use crate::annotations::Note;
use crate::ops::{check_sheet, map_styles, to_range};
use crate::protection::{check_edit, check_sheet_action, protected_error};
use crate::undo::{capture_range, sheet_meta, Change, Effects};
use crate::workbook::Workbook;

/// Longest note text.
pub const MAX_NOTE_LEN: usize = 32 * 1024;
/// Longest URL.
pub const MAX_URL_LEN: usize = 8 * 1024;
/// Most notes (or links) per sheet.
pub const MAX_NOTES: usize = 1_000_000;

fn cell_range(sheet: SheetId, r: u32, c: u32) -> Result<CellRange, ApiError> {
    if r >= MAX_ROWS || c >= MAX_COLS {
        return Err(ApiError::invalid_argument("cell outside the grid"));
    }
    Ok(CellRange::new(sheet, r, c, r, c))
}

fn not_found(what: &str, id: u32) -> ApiError {
    ApiError::not_found(format!("no {what} {id}"))
}

/// Records the sheet meta, then runs `f` on it.
fn with_meta(
    wb: &mut Workbook,
    sheet: SheetId,
    fx: &mut Effects,
    f: impl FnOnce(&Workbook, &mut crate::storage::SheetMeta) -> Result<(), ApiError>,
) -> Result<Vec<Change>, ApiError> {
    check_sheet(wb, sheet)?;
    let undo = sheet_meta(wb, sheet).ok_or_else(|| ApiError::not_found("no sheet"))?;
    let mut meta = wb.sheet(sheet).map(|s| s.meta.clone()).unwrap_or_default();
    f(wb, &mut meta)?;
    if let Some(s) = wb.sheet_mut(sheet) {
        s.meta = meta;
    }
    wb.pending
        .display_only
        .push(CellRange::new(sheet, 0, 0, MAX_ROWS - 1, MAX_COLS - 1));
    fx.layout(sheet);
    Ok(vec![undo])
}

/// The current local time as ISO 8601 (seconds).
pub fn now_iso() -> String {
    use tally_numfmt::date::{hms_from_serial, now_serial, ymd_from_serial};
    let epoch = tally_types::DateEpoch::D1899;
    let s = now_serial(epoch);
    let (y, m, d) = ymd_from_serial(epoch, s).unwrap_or((1970, 1, 1));
    let (hh, mm, ss, _) = hms_from_serial(s);
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}")
}

fn check_url(url: &str) -> Result<(), ApiError> {
    let u = url.trim();
    if u.is_empty() || u.len() > MAX_URL_LEN {
        return Err(ApiError::invalid_argument("bad URL"));
    }
    if u.chars().any(char::is_control) {
        return Err(ApiError::invalid_argument(
            "the URL contains control characters",
        ));
    }
    let lower = u.to_ascii_lowercase();
    if lower.starts_with("javascript:")
        || lower.starts_with("data:")
        || lower.starts_with("vbscript:")
    {
        return Err(ApiError::invalid_argument(
            "this kind of link is not allowed",
        ));
    }
    Ok(())
}

/// Applies a v0.3 op; `None` for the other ops.
pub fn apply(
    wb: &mut Workbook,
    op: &Op,
    fx: &mut Effects,
) -> Option<Result<Vec<Change>, ApiError>> {
    Some(match op {
        Op::AddConditionalFormat {
            sheet,
            ranges,
            rules,
        } => (|| {
            check_sheet_action(wb, *sheet, |_| false)?;
            let sheet = *sheet;
            with_meta(wb, sheet, fx, |wb, meta| {
                if meta.cond_formats.len() >= crate::condfmt::MAX_CF_PER_SHEET {
                    return Err(ApiError::invalid_argument("too many conditional formats"));
                }
                let id = meta.alloc_id();
                let cf = crate::condfmt::from_api(wb, sheet, id, ranges, rules)?;
                meta.cond_formats.push(cf);
                Ok(())
            })
        })(),
        Op::UpdateConditionalFormat {
            sheet,
            id,
            ranges,
            rules,
        } => (|| {
            check_sheet_action(wb, *sheet, |_| false)?;
            let (sheet, id) = (*sheet, *id);
            with_meta(wb, sheet, fx, |wb, meta| {
                let i = meta
                    .cond_formats
                    .iter()
                    .position(|c| c.id == id)
                    .ok_or_else(|| not_found("conditional format", id))?;
                meta.cond_formats[i] = crate::condfmt::from_api(wb, sheet, id, ranges, rules)?;
                Ok(())
            })
        })(),
        Op::DeleteConditionalFormat { sheet, id } => (|| {
            check_sheet_action(wb, *sheet, |_| false)?;
            let id = *id;
            with_meta(wb, *sheet, fx, |_, meta| {
                let i = meta
                    .cond_formats
                    .iter()
                    .position(|c| c.id == id)
                    .ok_or_else(|| not_found("conditional format", id))?;
                meta.cond_formats.remove(i);
                Ok(())
            })
        })(),
        Op::AddValidation { sheet, validation } => (|| {
            check_sheet_action(wb, *sheet, |_| false)?;
            let sheet = *sheet;
            with_meta(wb, sheet, fx, |wb, meta| {
                if meta.validations.len() >= crate::validation::MAX_VALIDATIONS {
                    return Err(ApiError::invalid_argument("too many validations"));
                }
                let id = meta.alloc_id();
                let v = crate::validation::from_api(wb, sheet, id, validation)?;
                // A cell has one validation: the new one replaces the
                // others where they overlap (as Calc's Validity dialog).
                for old in &mut meta.validations {
                    old.ranges = subtract_all(&old.ranges, &v.ranges);
                }
                meta.validations.retain(|o| !o.ranges.is_empty());
                meta.validations.push(v);
                Ok(())
            })
        })(),
        Op::UpdateValidation {
            sheet,
            id,
            validation,
        } => (|| {
            check_sheet_action(wb, *sheet, |_| false)?;
            let (sheet, id) = (*sheet, *id);
            with_meta(wb, sheet, fx, |wb, meta| {
                let i = meta
                    .validations
                    .iter()
                    .position(|v| v.id == id)
                    .ok_or_else(|| not_found("validation", id))?;
                let v = crate::validation::from_api(wb, sheet, id, validation)?;
                for (j, old) in meta.validations.iter_mut().enumerate() {
                    if j != i {
                        old.ranges = subtract_all(&old.ranges, &v.ranges);
                    }
                }
                meta.validations[i] = v;
                meta.validations.retain(|o| !o.ranges.is_empty());
                Ok(())
            })
        })(),
        Op::DeleteValidation { sheet, id } => (|| {
            check_sheet_action(wb, *sheet, |_| false)?;
            let id = *id;
            with_meta(wb, *sheet, fx, |_, meta| {
                let i = meta
                    .validations
                    .iter()
                    .position(|v| v.id == id)
                    .ok_or_else(|| not_found("validation", id))?;
                meta.validations.remove(i);
                Ok(())
            })
        })(),
        Op::SetNote { sheet, r, c, note } => (|| {
            check_sheet(wb, *sheet)?;
            let cell = cell_range(*sheet, *r, *c)?;
            check_edit(wb, &[cell])?;
            if note.text.len() > MAX_NOTE_LEN {
                return Err(ApiError::invalid_argument("the note is too long"));
            }
            let (r, c) = (*r, *c);
            with_meta(wb, *sheet, fx, |_, meta| {
                if !meta.notes.contains_key(&(r, c)) && meta.notes.len() >= MAX_NOTES {
                    return Err(ApiError::invalid_argument("too many notes"));
                }
                let created = note
                    .created
                    .clone()
                    .or_else(|| {
                        meta.notes
                            .get(&(r, c))
                            .and_then(|n| n.created.as_ref().map(|s| s.to_string()))
                    })
                    .unwrap_or_else(now_iso);
                meta.notes.insert(
                    (r, c),
                    Note {
                        text: note.text.as_str().into(),
                        author: note
                            .author
                            .as_deref()
                            .filter(|a| !a.is_empty())
                            .map(Into::into),
                        created: Some(created.into()),
                        shown: note.shown,
                    },
                );
                Ok(())
            })
        })(),
        Op::DeleteNote { sheet, r, c } => (|| {
            check_sheet(wb, *sheet)?;
            let cell = cell_range(*sheet, *r, *c)?;
            check_edit(wb, &[cell])?;
            let (r, c) = (*r, *c);
            with_meta(wb, *sheet, fx, |_, meta| {
                meta.notes
                    .remove(&(r, c))
                    .map(|_| ())
                    .ok_or_else(|| ApiError::not_found("the cell has no note"))
            })
        })(),
        Op::SetLink { sheet, r, c, url } => (|| {
            check_sheet(wb, *sheet)?;
            let cell = cell_range(*sheet, *r, *c)?;
            check_edit(wb, &[cell])?;
            check_url(url)?;
            let (r, c) = (*r, *c);
            with_meta(wb, *sheet, fx, |_, meta| {
                if !meta.links.contains_key(&(r, c)) && meta.links.len() >= MAX_NOTES {
                    return Err(ApiError::invalid_argument("too many links"));
                }
                meta.links.insert((r, c), Arc::from(url.trim()));
                Ok(())
            })
        })(),
        Op::ClearLink { sheet, ranges } => (|| {
            check_sheet(wb, *sheet)?;
            let ranges: Vec<CellRange> = ranges
                .iter()
                .map(|r| to_range(*sheet, r))
                .collect::<Result<_, _>>()?;
            check_edit(wb, &ranges)?;
            with_meta(wb, *sheet, fx, |_, meta| {
                for r in &ranges {
                    crate::annotations::clear_in(&mut meta.links, r);
                }
                Ok(())
            })
        })(),
        Op::ProtectSheet {
            sheet,
            password,
            options,
        } => (|| {
            check_sheet(wb, *sheet)?;
            if wb
                .sheet(*sheet)
                .is_some_and(|s| s.meta.protection.is_some())
            {
                return Err(ApiError::invalid_argument("the sheet is already protected"));
            }
            let opts = options.clone().unwrap_or_else(ProtectionOptions::default);
            let p = crate::protection::new_protection(password.as_deref(), &opts);
            with_meta(wb, *sheet, fx, |_, meta| {
                meta.protection = Some(p);
                Ok(())
            })
        })(),
        Op::UnprotectSheet { sheet, password } => (|| {
            check_sheet(wb, *sheet)?;
            let Some(p) = wb.sheet(*sheet).and_then(|s| s.meta.protection.clone()) else {
                return Err(ApiError::invalid_argument("the sheet is not protected"));
            };
            if !crate::protection::verify(&p, password.as_deref()) {
                return Err(ApiError::new(
                    crate::protection::PROTECTED,
                    "Incorrect password.",
                ));
            }
            with_meta(wb, *sheet, fx, |_, meta| {
                meta.protection = None;
                Ok(())
            })
        })(),
        Op::SetCellProtection {
            sheet,
            ranges,
            locked,
            hidden_formula,
        } => (|| {
            check_sheet(wb, *sheet)?;
            if wb
                .sheet(*sheet)
                .is_some_and(|s| s.meta.protection.is_some())
            {
                return Err(protected_error());
            }
            let ranges: Vec<CellRange> = ranges
                .iter()
                .map(|r| to_range(*sheet, r))
                .collect::<Result<_, _>>()?;
            let mut inv = Vec::with_capacity(ranges.len());
            for range in ranges {
                inv.push(Change::Range(capture_range(wb, range, false, true)));
                map_styles(wb, range, |t, old| {
                    let mut d = t.get(old).data.clone();
                    let mut p = d.protection.unwrap_or_default();
                    if let Some(l) = locked {
                        p.locked = *l;
                    }
                    if let Some(h) = hidden_formula {
                        p.hidden_formula = *h;
                    }
                    d.protection = (p != CellProtectionData::default()).then_some(p);
                    t.intern(d)
                });
            }
            Ok(inv)
        })(),
        Op::ProtectDocument { password } => (|| {
            if wb.structure_protection.is_some() {
                return Err(ApiError::invalid_argument(
                    "the document structure is already protected",
                ));
            }
            let old = wb.structure_protection.clone();
            wb.structure_protection = Some(crate::protection::new_protection(
                password.as_deref(),
                &ProtectionOptions::default(),
            ));
            fx.sheets_changed = true;
            Ok(vec![Change::DocProtection(old)])
        })(),
        Op::UnprotectDocument { password } => (|| {
            let Some(p) = wb.structure_protection.clone() else {
                return Err(ApiError::invalid_argument(
                    "the document structure is not protected",
                ));
            };
            if !crate::protection::verify(&p, password.as_deref()) {
                return Err(ApiError::new(
                    crate::protection::PROTECTED,
                    "Incorrect password.",
                ));
            }
            wb.structure_protection = None;
            fx.sheets_changed = true;
            Ok(vec![Change::DocProtection(Some(p))])
        })(),
        Op::SetPageSetup { sheet, setup } => (|| {
            let p = crate::pagesetup::from_api(setup)?;
            with_meta(wb, *sheet, fx, |_, meta| {
                meta.page_setup = p;
                Ok(())
            })
        })(),
        Op::SetPageBreak {
            sheet,
            axis,
            at,
            present,
        } => (|| {
            let limit = match axis {
                BreakAxis::Row => MAX_ROWS,
                BreakAxis::Col => MAX_COLS,
            };
            if *at == 0 || *at >= limit {
                return Err(ApiError::invalid_argument("bad page break position"));
            }
            let (axis, at, present) = (*axis, *at, *present);
            with_meta(wb, *sheet, fx, |_, meta| {
                let set = match axis {
                    BreakAxis::Row => &mut meta.row_breaks,
                    BreakAxis::Col => &mut meta.col_breaks,
                };
                if present {
                    if set.len() >= 10_000 && !set.contains(&at) {
                        return Err(ApiError::invalid_argument("too many page breaks"));
                    }
                    set.insert(at);
                } else {
                    set.remove(&at);
                }
                Ok(())
            })
        })(),
        Op::RefreshFilter { sheet } => (|| {
            check_sheet(wb, *sheet)?;
            let old = wb
                .sheet(*sheet)
                .map(|s| s.filtered_rows.clone())
                .unwrap_or_default();
            wb.reapply_filter(*sheet);
            fx.layout(*sheet);
            Ok(vec![Change::FilteredRows {
                sheet: *sheet,
                rows: old,
            }])
        })(),
        Op::SetDocumentProperties { props } => (|| {
            let old = wb.meta.clone();
            crate::pagesetup::apply_props(&mut wb.meta, props)?;
            Ok(vec![Change::DocMeta(Box::new(old))])
        })(),
        Op::AutoFitRows { sheet, start, end } => (|| {
            check_sheet(wb, *sheet)?;
            if start > end || *end >= MAX_ROWS {
                return Err(ApiError::invalid_argument("bad row range"));
            }
            check_sheet_action(wb, *sheet, |p| p.format_cells)?;
            let s = wb
                .sheet(*sheet)
                .ok_or_else(|| ApiError::not_found("no sheet"))?;
            let old = s.row_dims.clone();
            // Only rows with content can need a different height.
            let mut rows: Vec<u32> = Vec::new();
            for col in &s.columns {
                use crate::storage::ColumnStore;
                rows.extend(col.cells.iter_range(*start, *end).map(|(r, _)| r));
                if rows.len() > 1_000_000 {
                    break;
                }
            }
            rows.sort_unstable();
            rows.dedup();
            let mut heights: Vec<(u32, f32)> = Vec::new();
            for r in rows {
                if s.row_dims.is_hidden(r) {
                    continue;
                }
                if let Some(h) = crate::autofit::needed_height_px(wb, s, r) {
                    heights.push((r, h));
                }
            }
            let mut fitted: Vec<DimRunPx> = Vec::new();
            if let Some(s) = wb.sheet_mut(*sheet) {
                for &(r, h) in &heights {
                    s.row_dims.set(r, r, crate::units::px_to_hmm(h), false);
                    match fitted.last_mut() {
                        Some(l) if l.end + 1 == r && l.size == h => l.end = r,
                        _ => fitted.push(DimRunPx {
                            start: r,
                            end: r,
                            size: h,
                            hidden: false,
                            filtered: None,
                        }),
                    }
                }
            }
            fx.fitted_rows.get_or_insert_with(Vec::new).extend(fitted);
            fx.layout(*sheet);
            Ok(vec![Change::RowDims {
                sheet: *sheet,
                dims: old,
            }])
        })(),
        _ => return None,
    })
}

/// `ranges` minus every rectangle of `cut`.
pub fn subtract_all(
    ranges: &[tally_model::CellRangeLocal],
    cut: &[tally_model::CellRangeLocal],
) -> Vec<tally_model::CellRangeLocal> {
    let mut cur: Vec<tally_model::CellRangeLocal> = ranges.to_vec();
    for c in cut {
        let mut next = Vec::with_capacity(cur.len());
        for r in cur {
            next.extend(subtract(&r, c));
        }
        cur = next;
    }
    cur
}

/// `a` minus `b` as up to four rectangles.
pub fn subtract(
    a: &tally_model::CellRangeLocal,
    b: &tally_model::CellRangeLocal,
) -> Vec<tally_model::CellRangeLocal> {
    use tally_model::CellRangeLocal as R;
    if b.r0 > a.r1 || b.r1 < a.r0 || b.c0 > a.c1 || b.c1 < a.c0 {
        return vec![*a];
    }
    let mut out = Vec::new();
    if b.r0 > a.r0 {
        out.push(R { r1: b.r0 - 1, ..*a });
    }
    if b.r1 < a.r1 {
        out.push(R { r0: b.r1 + 1, ..*a });
    }
    let (r0, r1) = (a.r0.max(b.r0), a.r1.min(b.r1));
    if b.c0 > a.c0 {
        out.push(R {
            r0,
            r1,
            c0: a.c0,
            c1: b.c0 - 1,
        });
    }
    if b.c1 < a.c1 {
        out.push(R {
            r0,
            r1,
            c0: b.c1 + 1,
            c1: a.c1,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tally_model::CellRangeLocal as R;

    #[test]
    fn rectangle_subtraction() {
        let a = R {
            r0: 0,
            c0: 0,
            r1: 9,
            c1: 9,
        };
        let b = R {
            r0: 3,
            c0: 3,
            r1: 5,
            c1: 5,
        };
        let parts = subtract(&a, &b);
        let cells: u32 = parts
            .iter()
            .map(|r| (r.r1 - r.r0 + 1) * (r.c1 - r.c0 + 1))
            .sum();
        assert_eq!(cells, 100 - 9);
        assert_eq!(subtract(&b, &a), vec![]);
        assert!(check_url("javascript:alert(1)").is_err());
        assert!(check_url("https://example.com").is_ok());
        assert_eq!(now_iso().len(), 19);
    }
}
