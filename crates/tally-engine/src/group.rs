//! Multi-sheet selection ("group" editing, AOO's selected sheets) and
//! Edit ▸ Fill ▸ Sheets.
//!
//! A [`Transaction`] whose `group` lists more than one sheet runs each
//! group op (see [`GROUP_OPS`]) whose `sheet` is `group[0]` (the active
//! sheet) on every sheet of the group, at the same addresses, in group
//! order. Other ops run once. Text is parsed per sheet, so unqualified
//! references point at each sheet and `Sheet.A1` references stay as typed.
//! A group transaction is all-or-nothing ([`crate::document::Document::apply`]
//! reverts every change when an op fails) and is one undo step.
//!
//! The group ops are: `SetCellInput`, `SetRangeInputs`, `SetArrayFormula`,
//! `Clear`, `DeleteContents`, `ApplyStyle`, `SetNumberFormat`,
//! `ApplyCellStyle`, `MergeCells`, `UnmergeCells`, `SetCellProtection`,
//! `AddConditionalFormat`, `AddValidation`, `InsertRows`, `DeleteRows`,
//! `InsertCols`, `DeleteCols`, `InsertCells`, `DeleteCells`, `SetColWidth`,
//! `SetRowHeight`, `SetRowsHidden`, `SetColsHidden`, `AutoFitRows`,
//! `Paste`, `PasteSpecial`, `FillRange`, `FillSeries` and `ReplaceAll`
//! (with `sheet` set).

use std::borrow::Cow;
use std::sync::Arc;

use tally_api::{
    ApiError, DeleteContentsFlags, FillSheetOptions, Op, PasteOperation, PasteOptions, RangeDto,
    Transaction,
};
use tally_formula::{adjust_3d_for_structure_change, StructureChange};
use tally_types::{CellAddr, CellRange, SheetId, MAX_COLS, MAX_ROWS};

use crate::clipboard::ClipSnapshot;
use crate::ops::{check_array_blocks, check_sheet, to_range};
use crate::protection::{check_edit, check_format};
use crate::storage::{CellContent, MetaKind};
use crate::undo::{capture_range, meta_parts, Change, Effects};
use crate::workbook::{Rng, Workbook};

/// The `type` tags (serde names) of the ops a group repeats on every
/// sheet; the TS mock mirrors this list.
pub const GROUP_OPS: &[&str] = &[
    "setCellInput",
    "setRangeInputs",
    "setArrayFormula",
    "clear",
    "deleteContents",
    "applyStyle",
    "setNumberFormat",
    "applyCellStyle",
    "mergeCells",
    "unmergeCells",
    "setCellProtection",
    "addConditionalFormat",
    "addValidation",
    "insertRows",
    "deleteRows",
    "insertCols",
    "deleteCols",
    "insertCells",
    "deleteCells",
    "setColWidth",
    "setRowHeight",
    "setRowsHidden",
    "setColsHidden",
    "autoFitRows",
    "paste",
    "pasteSpecial",
    "fillRange",
    "fillSeries",
    "replaceAll",
];

/// The sheet field of a group op (`None` for other ops, and for a
/// Replace All over every sheet).
fn group_sheet_mut(op: &mut Op) -> Option<&mut SheetId> {
    match op {
        Op::SetCellInput { sheet, .. }
        | Op::SetRangeInputs { sheet, .. }
        | Op::SetArrayFormula { sheet, .. }
        | Op::Clear { sheet, .. }
        | Op::DeleteContents { sheet, .. }
        | Op::ApplyStyle { sheet, .. }
        | Op::SetNumberFormat { sheet, .. }
        | Op::ApplyCellStyle { sheet, .. }
        | Op::MergeCells { sheet, .. }
        | Op::UnmergeCells { sheet, .. }
        | Op::SetCellProtection { sheet, .. }
        | Op::AddConditionalFormat { sheet, .. }
        | Op::AddValidation { sheet, .. }
        | Op::InsertRows { sheet, .. }
        | Op::DeleteRows { sheet, .. }
        | Op::InsertCols { sheet, .. }
        | Op::DeleteCols { sheet, .. }
        | Op::InsertCells { sheet, .. }
        | Op::DeleteCells { sheet, .. }
        | Op::SetColWidth { sheet, .. }
        | Op::SetRowHeight { sheet, .. }
        | Op::SetRowsHidden { sheet, .. }
        | Op::SetColsHidden { sheet, .. }
        | Op::AutoFitRows { sheet, .. }
        | Op::Paste { sheet, .. }
        | Op::PasteSpecial { sheet, .. }
        | Op::FillRange { sheet, .. }
        | Op::FillSeries { sheet, .. } => Some(sheet),
        // A Replace of the one current cell (the dialog's single Replace)
        // stays on the active sheet (review L2).
        Op::ReplaceAll { query, .. } if is_single_cell(query.selection.as_deref()) => None,
        Op::ReplaceAll { sheet, .. } => sheet.as_mut(),
        _ => None,
    }
}

fn is_single_cell(sel: Option<&[RangeDto]>) -> bool {
    matches!(sel, Some([r]) if r.r0 == r.r1 && r.c0 == r.c1)
}

/// The sheet of a group op.
pub fn group_sheet(op: &Op) -> Option<SheetId> {
    group_sheet_mut(&mut op.clone()).copied()
}

/// `op` on sheet `to`, if it is a group op.
pub fn retarget(op: &Op, to: SheetId) -> Option<Op> {
    let mut op = op.clone();
    *group_sheet_mut(&mut op)? = to;
    if let Op::ReplaceAll { query, .. } = &mut op {
        if query.sheet.is_some() {
            query.sheet = Some(to);
        }
    }
    Some(op)
}

/// The sheets a transaction's group expands to: `None` when it has no
/// group of two or more sheets. Unknown sheets are refused; duplicates
/// and hidden sheets other than the first are dropped.
pub fn group_sheets(wb: &Workbook, tx: &Transaction) -> Result<Option<Vec<SheetId>>, ApiError> {
    let Some(group) = tx.group.as_ref().filter(|g| g.len() > 1) else {
        return Ok(None);
    };
    let mut out: Vec<SheetId> = Vec::with_capacity(group.len());
    for (i, &s) in group.iter().enumerate() {
        let sheet = wb
            .sheet(s)
            .ok_or_else(|| ApiError::not_found(format!("no sheet {s}")))?;
        if out.contains(&s) || (i > 0 && sheet.hidden) {
            continue;
        }
        out.push(s);
    }
    Ok(Some(out))
}

/// One op of an expanded transaction.
pub struct Step<'a> {
    /// Index of the op in `Transaction::ops`.
    pub index: u32,
    /// The op (the original, or a copy for another group sheet).
    pub op: Cow<'a, Op>,
    /// The last copy of a group op: the group's sheets.
    pub ends_group: Option<&'a [SheetId]>,
}

/// The ops of `tx` with their original index, each group op on
/// `group[0]` repeated on every group sheet (in group order). Lazy: each
/// copy is made when the caller reaches it (review M2).
pub fn expand<'a>(
    tx: &'a Transaction,
    group: Option<&'a [SheetId]>,
) -> impl Iterator<Item = Step<'a>> + 'a {
    tx.ops.iter().enumerate().flat_map(move |(i, op)| {
        let index = i as u32;
        let sheets: &'a [SheetId] = match group {
            Some(g) if g.len() > 1 && group_sheet(op) == Some(g[0]) => g,
            _ => &[],
        };
        let n = sheets.len().max(1);
        (0..n).map(move |k| Step {
            index,
            op: if k == 0 {
                Cow::Borrowed(op)
            } else {
                // `retarget` is `Some` for every group op.
                retarget(op, sheets[k]).map_or(Cow::Borrowed(op), Cow::Owned)
            },
            ends_group: (sheets.len() > 1 && k + 1 == n).then_some(sheets),
        })
    })
}

/// The row/column change of a whole-row/column insert or delete (or a
/// cell shift across the whole sheet, which is one).
fn line_change(op: &Op) -> Option<StructureChange> {
    use tally_api::ShiftDir;
    Some(match *op {
        Op::InsertRows { sheet, at, count } => StructureChange::InsertRows { sheet, at, count },
        Op::DeleteRows { sheet, at, count } => StructureChange::DeleteRows { sheet, at, count },
        Op::InsertCols { sheet, at, count } => StructureChange::InsertCols { sheet, at, count },
        Op::DeleteCols { sheet, at, count } => StructureChange::DeleteCols { sheet, at, count },
        Op::InsertCells {
            sheet,
            range: r,
            shift,
        }
        | Op::DeleteCells {
            sheet,
            range: r,
            shift,
        } => {
            let insert = matches!(op, Op::InsertCells { .. });
            match shift {
                ShiftDir::Down | ShiftDir::Up if r.c0 == 0 && r.c1 == MAX_COLS - 1 => {
                    let (at, count) = (r.r0, r.r1.checked_sub(r.r0)? + 1);
                    if insert {
                        StructureChange::InsertRows { sheet, at, count }
                    } else {
                        StructureChange::DeleteRows { sheet, at, count }
                    }
                }
                ShiftDir::Right | ShiftDir::Left if r.r0 == 0 && r.r1 == MAX_ROWS - 1 => {
                    let (at, count) = (r.c0, r.c1.checked_sub(r.c0)? + 1);
                    if insert {
                        StructureChange::InsertCols { sheet, at, count }
                    } else {
                        StructureChange::DeleteCols { sheet, at, count }
                    }
                }
                _ => return None,
            }
        }
        _ => return None,
    })
}

/// After a row/column insert or delete ran on every sheet of a group:
/// moves, once, the references over several sheets that all lie in the
/// group (AOO `ScRefUpdate::Update` needs the changed sheets to contain
/// the reference's sheets; one sheet's change leaves them alone). Cell
/// formulas and names; returns the undo changes.
pub fn structure_3d(wb: &mut Workbook, op: &Op, sheets: &[SheetId]) -> Vec<Change> {
    let Some(change) = line_change(op) else {
        return Vec::new();
    };
    let order: Vec<SheetId> = wb.sheets.iter().map(|s| s.id).collect();
    let covers = |a: SheetId, b: SheetId| -> bool {
        let (Some(i), Some(j)) = (
            order.iter().position(|s| *s == a),
            order.iter().position(|s| *s == b),
        ) else {
            return false;
        };
        order[i.min(j)..=i.max(j)]
            .iter()
            .all(|s| sheets.contains(s))
    };
    let mut rewritten = Vec::new();
    for (_, fc) in &wb.formulas {
        if fc.invalid_text.is_some() {
            continue;
        }
        let mut f = (*fc.formula).clone();
        if adjust_3d_for_structure_change(&mut f, fc.addr, &change, &covers) {
            rewritten.push((fc.addr, Arc::new(f)));
        }
    }
    let mut out = Vec::new();
    let name_hits: Vec<bool> = wb
        .names
        .defs()
        .map(|d| {
            let mut f = (*d.formula).clone();
            adjust_3d_for_structure_change(&mut f, d.base, &change, &covers)
        })
        .collect();
    if name_hits.iter().any(|h| *h) {
        out.push(crate::undo::names_snapshot(wb));
        for d in wb.names.defs_mut() {
            let mut f = (*d.formula).clone();
            if adjust_3d_for_structure_change(&mut f, d.base, &change, &covers) {
                d.formula = Arc::new(f);
            }
        }
        wb.refresh_all_name_dependents();
    }
    if !rewritten.is_empty() {
        let old: Vec<_> = rewritten
            .into_iter()
            .map(|(addr, f)| (addr, wb.replace_formula_ast(addr, f)))
            .collect();
        out.push(Change::Cells(old));
    }
    out
}

/// The source ranges `Op::FillSheets` works on: without formats, each
/// range is cut to the box of the cells and notes of the source and the
/// targets (so whole columns are fine, as in AOO); with formats, the
/// ranges as given.
fn fill_areas(
    wb: &Workbook,
    sheet: SheetId,
    ranges: &[CellRange],
    targets: &[SheetId],
    formats: bool,
) -> Vec<CellRange> {
    if formats {
        return ranges.to_vec();
    }
    let mut boxes: Vec<CellRange> = Vec::new();
    for s in std::iter::once(sheet).chain(targets.iter().copied()) {
        let Some(sh) = wb.sheet(s) else { continue };
        if let Some(u) = sh.used_range() {
            boxes.push(u);
        }
        for ((r, c), _) in sh.meta.notes.iter() {
            boxes.push(CellRange::new(s, *r, *c, *r, *c));
        }
    }
    let Some(b) = boxes.into_iter().reduce(|a, b| {
        CellRange::new(
            sheet,
            a.r0.min(b.r0),
            a.c0.min(b.c0),
            a.r1.max(b.r1),
            a.c1.max(b.c1),
        )
    }) else {
        return Vec::new();
    };
    let b = CellRange::new(sheet, b.r0, b.c0, b.r1, b.c1);
    ranges.iter().filter_map(|r| r.intersect(&b)).collect()
}

/// The targets of `Op::FillSheets`: existing, visible, not the source,
/// each once.
fn fill_targets_of(wb: &Workbook, sheet: SheetId, to: &[SheetId]) -> Vec<SheetId> {
    let mut out: Vec<SheetId> = Vec::with_capacity(to.len());
    for &t in to {
        if t != sheet && !out.contains(&t) && wb.sheet(t).is_some_and(|s| !s.hidden) {
            out.push(t);
        }
    }
    out
}

/// The cells `Op::FillSheets` writes (for the transaction cell budget):
/// the areas it works on, times the targets (review L4).
pub fn fill_sheets_cost(wb: &Workbook, sheet: SheetId, ranges: &[RangeDto], to: &[SheetId]) -> u64 {
    let targets = fill_targets_of(wb, sheet, to);
    let Ok(src) = ranges
        .iter()
        .map(|r| to_range(sheet, r))
        .collect::<Result<Vec<_>, _>>()
    else {
        return 1;
    };
    // Formats count one per column per range (styles are column runs),
    // as `ApplyStyle` does.
    let area: u64 = fill_areas(wb, sheet, &src, &targets, false)
        .iter()
        .map(CellRange::cell_count)
        .sum::<u64>()
        + src.iter().map(|r| u64::from(r.cols())).sum::<u64>();
    area.saturating_mul(targets.len() as u64)
}

/// Whether a source cell's content is of a kind `o` copies (AOO
/// `IDF_STRING`, `IDF_VALUE`, `IDF_DATETIME`, `IDF_FORMULA`).
fn kind_selected(
    wb: &Workbook,
    sheet: SheetId,
    row: u32,
    col: u32,
    c: &CellContent,
    o: &FillSheetOptions,
) -> bool {
    use tally_types::Value;
    match c {
        CellContent::Formula(_) | CellContent::ArrayPart(_) => o.formulas,
        CellContent::Value(Value::Text(_)) => o.text,
        CellContent::Value(Value::Number(_)) => {
            let date = wb
                .sheet(sheet)
                .is_some_and(|s| wb.styles.get(s.style(row, col)).format.is_date());
            if date {
                o.date_time
            } else {
                o.numbers
            }
        }
        CellContent::Value(Value::Bool(_)) => o.numbers,
        CellContent::Value(_) => o.text || o.numbers,
    }
}

/// `Op::FillSheets` (AOO `ScDocument::FillTab`): copies `ranges` of
/// `sheet` to the same addresses on each sheet of `to`. Without Skip
/// empty cells or an operation, the selected kinds of content (and the
/// notes and formats, when selected) are first removed from the target;
/// then the selected source cells are written (formulas keep their
/// relative references, so unqualified ones point at the target sheet),
/// combined with the target's numbers by `operation`. Formats carry the
/// cell attributes, merges, conditional formats and validations as Paste
/// Special ▸ Formats does.
pub fn fill_sheets(
    wb: &mut Workbook,
    sheet: SheetId,
    ranges: &[RangeDto],
    to: &[SheetId],
    o: &FillSheetOptions,
    fx: &mut Effects,
) -> Result<Vec<Change>, ApiError> {
    check_sheet(wb, sheet)?;
    let src: Vec<_> = ranges
        .iter()
        .map(|r| to_range(sheet, r))
        .collect::<Result<_, _>>()?;
    for &t in to {
        check_sheet(wb, t)?;
    }
    let targets = fill_targets_of(wb, sheet, to);
    let src = fill_areas(wb, sheet, &src, &targets, o.formats);
    let any_content = o.text || o.numbers || o.date_time || o.formulas;
    // Refuse up front: nothing changes when one target is protected or an
    // array would be split (AOO STR_PROTECTIONERR).
    for &t in &targets {
        let tr: Vec<_> = src
            .iter()
            .map(|r| CellRange::new(t, r.r0, r.c0, r.r1, r.c1))
            .collect();
        if any_content || o.notes {
            check_edit(wb, &tr)?;
        }
        if any_content {
            check_array_blocks(wb, &tr)?;
        }
        if o.formats {
            check_format(wb, &tr)?;
        }
    }
    // Snapshots of the sources, taken before any target changes.
    let mut rng = Rng::new(0x5EED);
    let clips: Vec<ClipSnapshot> = src
        .iter()
        .map(|r| crate::clipboard::copy_range(wb, *r, &mut rng).0)
        .collect();
    // The kind of each source cell (judged on the source sheet).
    let selected: Vec<Vec<bool>> = src
        .iter()
        .zip(&clips)
        .map(|(r, clip)| {
            clip.cells
                .iter()
                .map(|cell| {
                    let (row, col) = (r.r0 + cell.dr, r.c0 + cell.dc);
                    wb.sheet(sheet)
                        .and_then(|s| s.get(row, col))
                        .is_some_and(|c| kind_selected(wb, sheet, row, col, c, o))
                })
                .collect()
        })
        .collect();
    let replace = !o.skip_empty && o.operation == PasteOperation::None;
    let mut changes = Vec::new();
    let r = fill_targets(
        wb,
        &targets,
        &src,
        &clips,
        &selected,
        o,
        replace,
        &mut changes,
        fx,
    );
    match r {
        Ok(()) => Ok(changes),
        Err(e) => {
            // A step failed half-way (e.g. a merge across the target's
            // edge): nothing of the op stands.
            crate::ops::revert(wb, changes, fx);
            Err(e)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn fill_targets(
    wb: &mut Workbook,
    targets: &[SheetId],
    src: &[CellRange],
    clips: &[ClipSnapshot],
    selected: &[Vec<bool>],
    o: &FillSheetOptions,
    replace: bool,
    changes: &mut Vec<Change>,
    fx: &mut Effects,
) -> Result<(), ApiError> {
    let any_content = o.text || o.numbers || o.date_time || o.formulas;
    for &t in targets {
        for ((r, clip), sel) in src.iter().zip(clips).zip(selected) {
            let tr = CellRange::new(t, r.r0, r.c0, r.r1, r.c1);
            let dto = crate::document::range_dto(&tr);
            if replace && (any_content || o.notes) {
                let what = DeleteContentsFlags {
                    text: o.text,
                    numbers: o.numbers,
                    dates: o.date_time,
                    formulas: o.formulas,
                    notes: o.notes,
                    formats: false,
                    objects: false,
                };
                changes.extend(crate::tools::delete_contents(wb, t, &[dto], &what, fx)?);
            }
            if any_content {
                changes.push(Change::Range(capture_range(wb, tr, true, false)));
                for (cell, &on) in clip.cells.iter().zip(sel) {
                    if !on {
                        continue;
                    }
                    let addr = CellAddr::new(t, tr.r0 + cell.dr, tr.c0 + cell.dc);
                    if o.operation != PasteOperation::None {
                        crate::clipboard::paste_arithmetic(wb, addr, cell, o.operation);
                    } else if let Some(content) = &cell.content {
                        // Arrays cut by the range were copied as values.
                        wb.restore_content(addr, Some(content.clone()));
                    }
                }
                wb.pending.ranges.push(tr);
            }
            if o.notes && !clip.notes.is_empty() {
                changes.extend(meta_parts(wb, t, &[MetaKind::Notes]));
                if let Some(s) = wb.sheet_mut(t) {
                    let map = Arc::make_mut(&mut s.meta.notes);
                    for (dr, dc, note) in &clip.notes {
                        map.insert((tr.r0 + dr, tr.c0 + dc), note.clone());
                    }
                }
                fx.layout(t);
            }
            if o.formats {
                let opts = PasteOptions {
                    formats_only: true,
                    skip_empty: o.skip_empty,
                    ..PasteOptions::default()
                };
                changes.extend(crate::clipboard::paste_internal(
                    wb, t, tr, clip, &opts, fx,
                )?);
            }
        }
    }
    Ok(())
}
