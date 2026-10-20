//! The v0.3 ops: conditional formats, validations, notes, links,
//! protection, page setup and breaks, document properties and row
//! auto-height. Each records the undo state it replaces (the parts of
//! the sheet meta it changed, the styles of a range, the row heights or
//! the document metadata).
//!
//! # How to add a meta op
//!
//! A "meta op" edits [`SheetMeta`] (the per-sheet state that is not
//! cells: conditional formats, validations, notes, links, protection,
//! page setup, breaks, charts, pivots, outline groups, scenarios, …).
//! Every collection in `SheetMeta` sits behind an `Arc` and is edited
//! copy-on-write, so undo can keep "the collection before the op" as one
//! pointer and an op that touches one collection never copies the others.
//!
//! 1. Run protection checks first (`check_sheet_action`, `check_edit`),
//!    then parse and validate the arguments, and only then mutate. The
//!    helper restores every field it snapshotted when the closure fails,
//!    but a field this transaction already recorded in an earlier op is
//!    edited in place (see [`Effects::meta_recorded`]); an op that fails
//!    after mutating such a field leaves the mutation behind, so: check,
//!    then mutate.
//! 2. Wrap the edit in [`with_meta`]:
//!    ```ignore
//!    with_meta(wb, sheet, fx, |wb, meta| {
//!        let chart = crate::charts::from_api(wb, sheet, dto)?; // parse first
//!        let id = meta.alloc_id();
//!        Arc::make_mut(&mut meta.charts).push(Arc::new(Chart { id, data: chart }));
//!        Ok(())
//!    })
//!    ```
//!    Inside the closure `meta` *is* the sheet's meta (it is taken out of
//!    the sheet for the duration, so `wb.sheet(sheet).meta` is empty
//!    there: read through `meta`). Mutate collections with
//!    `Arc::make_mut(&mut meta.field)`; replace a `Vec<Arc<T>>` element
//!    with a new `Arc` (or `Arc::make_mut` the element to edit it).
//! 3. `with_meta` compares every field before and after by pointer
//!    identity and records only the changed ones in one
//!    [`Change::Meta`]; it marks the sheet's layout as changed and
//!    invalidates the display of the whole sheet. Report anything else
//!    through `fx` (`fx.objects(sheet)`, `fx.created(..)`, …).
//! 4. For ops that touch a few cells of the notes or links map, use
//!    [`with_meta_cells`] with the touched keys: the undo entry is then
//!    the previous value of those cells, not the whole map (a note edit
//!    on a sheet with 100,000 notes records one entry). The closure must
//!    not touch other keys of that map.
//! 5. New `SheetMeta` fields: add the field to the struct and one line
//!    to the `meta_parts!` table in `storage.rs`; everything here picks
//!    it up.
//!
//! Costs and budgets: a meta op costs 1 in the transaction cell budget
//! ([`crate::ops::op_cost`]); a transaction may hold at most
//! [`MAX_META_OPS_PER_TX`] meta ops and [`MAX_NOTE_BYTES_PER_TX`] bytes
//! of note text (security review 3, CRITICAL-2).

use std::sync::Arc;

use tally_api::{ApiError, BreakAxis, DimRunPx, Op, ProtectionOptions};
use tally_model::CellProtectionData;
use tally_types::{CellRange, SheetId, MAX_COLS, MAX_ROWS};

use crate::annotations::Note;
use crate::ops::{check_sheet, map_styles, to_range};
use crate::protection::{check_edit, check_sheet_action, protected_error};
use crate::storage::{map_entries, MetaKind, MetaPart, SheetMeta};
use crate::undo::{capture_range, Change, Effects};
use crate::workbook::Workbook;

/// Longest note text.
pub const MAX_NOTE_LEN: usize = 32 * 1024;
/// Longest URL.
pub const MAX_URL_LEN: usize = 8 * 1024;
/// Most notes (or links) per sheet.
pub const MAX_NOTES: usize = 1_000_000;
/// Most meta ops (conditional formats, validations, notes, links,
/// protection, page setup, breaks, document properties) in one
/// transaction; the ones past it fail with `unsupported`.
pub const MAX_META_OPS_PER_TX: u64 = 100_000;
/// Most bytes of note text one transaction may set.
pub const MAX_NOTE_BYTES_PER_TX: u64 = 64 * 1024 * 1024;

/// Which per-cell map an op edits (per-entry undo, see
/// [`with_meta_cells`]).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CellMap {
    /// `SheetMeta::notes`.
    Notes,
    /// `SheetMeta::links`.
    Links,
}

impl CellMap {
    fn kind(self) -> MetaKind {
        match self {
            CellMap::Notes => MetaKind::Notes,
            CellMap::Links => MetaKind::Links,
        }
    }
}

fn cell_range(sheet: SheetId, r: u32, c: u32) -> Result<CellRange, ApiError> {
    if r >= MAX_ROWS || c >= MAX_COLS {
        return Err(ApiError::invalid_argument("cell outside the grid"));
    }
    Ok(CellRange::new(sheet, r, c, r, c))
}

fn not_found(what: &str, id: u32) -> ApiError {
    ApiError::not_found(format!("no {what} {id}"))
}

/// Runs `f` on the sheet's meta and records the fields it changed for
/// undo (see the module documentation).
pub fn with_meta(
    wb: &mut Workbook,
    sheet: SheetId,
    fx: &mut Effects,
    f: impl FnOnce(&Workbook, &mut SheetMeta) -> Result<(), ApiError>,
) -> Result<Vec<Change>, ApiError> {
    with_meta_cells(wb, sheet, fx, None, f)
}

/// [`with_meta`] for an op that edits only `cells` of one per-cell map
/// (`cells = Some((map, keys))`): that map is recorded per entry instead
/// of as a whole, and `f` must not touch other keys of it.
pub fn with_meta_cells(
    wb: &mut Workbook,
    sheet: SheetId,
    fx: &mut Effects,
    cells: Option<(CellMap, &[(u32, u32)])>,
    f: impl FnOnce(&Workbook, &mut SheetMeta) -> Result<(), ApiError>,
) -> Result<Vec<Change>, ApiError> {
    check_sheet(wb, sheet)?;
    let no_sheet = || ApiError::not_found("no sheet");
    let s = wb.sheet(sheet).ok_or_else(no_sheet)?;
    let hinted = cells.map(|(m, _)| m.kind());
    // Snapshots (reference counts only) of the fields this transaction
    // does not hold yet; the hinted map is recorded per entry instead.
    let before: Vec<MetaPart> = MetaKind::ALL
        .iter()
        .copied()
        .filter(|k| Some(*k) != hinted && !fx.meta_recorded(sheet, *k))
        .map(|k| s.meta.part(k))
        .collect();
    let entries: Option<MetaPart> = cells.map(|(m, keys)| match m {
        CellMap::Notes => MetaPart::NoteEntries(map_entries(&s.meta.notes, keys)),
        CellMap::Links => MetaPart::LinkEntries(map_entries(&s.meta.links, keys)),
    });
    let mut meta = std::mem::take(&mut wb.sheet_mut(sheet).ok_or_else(no_sheet)?.meta);
    let result = f(wb, &mut meta);
    let s = wb.sheet_mut(sheet).ok_or_else(no_sheet)?;
    s.meta = meta;
    if let Err(e) = result {
        for p in before {
            p.swap_into(&mut s.meta);
        }
        if let Some(p) = entries {
            p.swap_into(&mut s.meta);
        }
        return Err(e);
    }
    let mut parts: Vec<MetaPart> = before
        .into_iter()
        .filter(|p| !p.unchanged_in(&s.meta))
        .collect();
    for p in &parts {
        if let Some(k) = p.kind() {
            fx.record_meta(sheet, k);
        }
    }
    parts.extend(entries);
    wb.pending
        .display_only
        .push(CellRange::new(sheet, 0, 0, MAX_ROWS - 1, MAX_COLS - 1));
    fx.layout(sheet);
    Ok(vec![Change::Meta { sheet, parts }])
}

/// `true` for an op [`MAX_META_OPS_PER_TX`] counts, with the note bytes
/// it sets (for [`MAX_NOTE_BYTES_PER_TX`]).
pub fn meta_op_cost(op: &Op) -> Option<u64> {
    match op {
        Op::SetNote { note, .. } => Some(note.text.len() as u64),
        Op::AddConditionalFormat { .. }
        | Op::UpdateConditionalFormat { .. }
        | Op::DeleteConditionalFormat { .. }
        | Op::AddValidation { .. }
        | Op::UpdateValidation { .. }
        | Op::DeleteValidation { .. }
        | Op::DeleteNote { .. }
        | Op::SetLink { .. }
        | Op::ClearLink { .. }
        | Op::ProtectSheet { .. }
        | Op::UnprotectSheet { .. }
        | Op::SetCellProtection { .. }
        | Op::ProtectDocument { .. }
        | Op::UnprotectDocument { .. }
        | Op::SetPageSetup { .. }
        | Op::SetPageBreak { .. }
        | Op::SetDocumentProperties { .. } => Some(0),
        _ => None,
    }
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

/// Refuses empty, over-long, control-character and active-content URLs
/// (`javascript:`, `data:`, `vbscript:`). Applied to `SetLink` and to
/// links read from files (security review 3, LOW-10).
pub fn check_url(url: &str) -> Result<(), ApiError> {
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

/// `true` if any range of `a` intersects any range of `b`.
fn ranges_intersect(a: &[tally_model::CellRangeLocal], b: &[tally_model::CellRangeLocal]) -> bool {
    a.iter().any(|x| {
        b.iter()
            .any(|y| x.r0 <= y.r1 && y.r0 <= x.r1 && x.c0 <= y.c1 && y.c0 <= x.c1)
    })
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
                let id = meta.next_id.max(1);
                let cf = crate::condfmt::from_api(wb, sheet, id, ranges, rules)?;
                meta.alloc_id();
                Arc::make_mut(&mut meta.cond_formats).push(Arc::new(cf));
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
                let cf = crate::condfmt::from_api(wb, sheet, id, ranges, rules)?;
                Arc::make_mut(&mut meta.cond_formats)[i] = Arc::new(cf);
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
                Arc::make_mut(&mut meta.cond_formats).remove(i);
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
                let id = meta.next_id.max(1);
                let v = crate::validation::from_api(wb, sheet, id, validation)?;
                meta.alloc_id();
                // A cell has one validation: the new one replaces the
                // others where they overlap (as Calc's Validity dialog).
                let list = Arc::make_mut(&mut meta.validations);
                for old in list.iter_mut() {
                    if ranges_intersect(&old.ranges, &v.ranges) {
                        Arc::make_mut(old).ranges = subtract_all(&old.ranges, &v.ranges);
                    }
                }
                list.retain(|o| !o.ranges.is_empty());
                list.push(Arc::new(v));
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
                let list = Arc::make_mut(&mut meta.validations);
                for (j, old) in list.iter_mut().enumerate() {
                    if j != i && ranges_intersect(&old.ranges, &v.ranges) {
                        Arc::make_mut(old).ranges = subtract_all(&old.ranges, &v.ranges);
                    }
                }
                list[i] = Arc::new(v);
                list.retain(|o| !o.ranges.is_empty());
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
                Arc::make_mut(&mut meta.validations).remove(i);
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
            let keys = [(r, c)];
            with_meta_cells(wb, *sheet, fx, Some((CellMap::Notes, &keys)), |_, meta| {
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
                Arc::make_mut(&mut meta.notes).insert(
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
            let keys = [(r, c)];
            with_meta_cells(wb, *sheet, fx, Some((CellMap::Notes, &keys)), |_, meta| {
                if !meta.notes.contains_key(&(r, c)) {
                    return Err(ApiError::not_found("the cell has no note"));
                }
                Arc::make_mut(&mut meta.notes).remove(&(r, c));
                Ok(())
            })
        })(),
        Op::SetLink { sheet, r, c, url } => (|| {
            check_sheet(wb, *sheet)?;
            let cell = cell_range(*sheet, *r, *c)?;
            check_edit(wb, &[cell])?;
            check_url(url)?;
            let (r, c) = (*r, *c);
            let keys = [(r, c)];
            with_meta_cells(wb, *sheet, fx, Some((CellMap::Links, &keys)), |_, meta| {
                if !meta.links.contains_key(&(r, c)) && meta.links.len() >= MAX_NOTES {
                    return Err(ApiError::invalid_argument("too many links"));
                }
                Arc::make_mut(&mut meta.links).insert((r, c), Arc::from(url.trim()));
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
            let keys: Vec<(u32, u32)> = wb
                .sheet(*sheet)
                .map(|s| {
                    ranges
                        .iter()
                        .flat_map(|r| crate::annotations::entries_in(&s.meta.links, r))
                        .map(|(k, _)| k)
                        .collect()
                })
                .unwrap_or_default();
            with_meta_cells(wb, *sheet, fx, Some((CellMap::Links, &keys)), |_, meta| {
                if !keys.is_empty() {
                    let map = Arc::make_mut(&mut meta.links);
                    for k in &keys {
                        map.remove(k);
                    }
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
            let out = with_meta(wb, *sheet, fx, |_, meta| {
                meta.protection = Some(Arc::new(p));
                Ok(())
            })?;
            // Protection is not undoable (LOW-9): the stacks are cleared.
            fx.clear_undo = true;
            Ok(out)
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
            let out = with_meta(wb, *sheet, fx, |_, meta| {
                meta.protection = None;
                Ok(())
            })?;
            fx.clear_undo = true;
            Ok(out)
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
            fx.clear_undo = true;
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
            fx.clear_undo = true;
            Ok(vec![Change::DocProtection(Some(p))])
        })(),
        Op::SetPageSetup { sheet, setup } => (|| {
            check_sheet(wb, *sheet)?;
            // Page setup is formatting: allowed on a protected sheet only
            // when the protection permits formatting (LOW-9).
            check_sheet_action(wb, *sheet, |p| p.format_cells)?;
            let p = crate::pagesetup::from_api(setup)?;
            with_meta(wb, *sheet, fx, |_, meta| {
                meta.page_setup = Arc::new(p);
                Ok(())
            })
        })(),
        Op::SetPageBreak {
            sheet,
            axis,
            at,
            present,
        } => (|| {
            check_sheet(wb, *sheet)?;
            check_sheet_action(wb, *sheet, |p| p.format_cells)?;
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
                    if set.contains(&at) {
                        return Ok(());
                    }
                    if set.len() >= 10_000 {
                        return Err(ApiError::invalid_argument("too many page breaks"));
                    }
                    Arc::make_mut(set).insert(at);
                } else if set.contains(&at) {
                    Arc::make_mut(set).remove(&at);
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
