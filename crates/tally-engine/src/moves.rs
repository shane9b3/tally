//! Cut and paste ([`tally_api::Op::MoveRange`], plan §3.5 `move_refs`).
//!
//! The block moves with its contents, styles, merges and array blocks; the
//! destination's previous content is overwritten. References follow
//! OpenOffice's move semantics ([`tally_formula::move_refs`]): references
//! into the moved block (from any formula on any sheet, and from names)
//! follow it; references inside moved formulas that point outside keep
//! their targets; references to overwritten destination cells become
//! `#REF!`.

use std::sync::Arc;

use tally_api::{ApiError, CellPos};
use tally_formula::{move_refs, BlockMove};
use tally_model::CellRangeLocal;
use tally_types::{CellAddr, CellRange};

use crate::formula_cells::FormulaCellId;
use crate::ops::{check_array_blocks, check_merges, MAX_OP_CELLS};
use crate::undo::{
    capture_range, clear_range_content, clear_range_styles, meta_parts, names_snapshot, Change,
    Effects,
};
use crate::workbook::{SnapContent, Workbook};

fn shifted(m: &CellRangeLocal, dr: i64, dc: i64) -> CellRangeLocal {
    let s = |v: u32, d: i64| (i64::from(v) + d) as u32;
    CellRangeLocal {
        r0: s(m.r0, dr),
        c0: s(m.c0, dc),
        r1: s(m.r1, dr),
        c1: s(m.c1, dc),
    }
}

/// Moves `from` so that its top-left lands at `to` (same sheet).
pub fn move_range(
    wb: &mut Workbook,
    from: CellRange,
    to: CellPos,
    fx: &mut Effects,
) -> Result<Vec<Change>, ApiError> {
    let sheet = from.sheet;
    let mv = BlockMove {
        from,
        dr: i64::from(to.r) - i64::from(from.r0),
        dc: i64::from(to.c) - i64::from(from.c0),
    };
    let dest = mv
        .to()
        .ok_or_else(|| ApiError::invalid_argument("the destination is outside the grid"))?;
    if mv.dr == 0 && mv.dc == 0 {
        return Ok(Vec::new());
    }
    if from.cell_count() > MAX_OP_CELLS {
        return Err(ApiError::invalid_argument("the range is too large to move"));
    }
    check_array_blocks(wb, &[from])?;
    check_merges(wb, &from)?;
    // Blocks and merges that move with the source may overlap the
    // destination; others must lie inside it or outside it.
    let within = |b: &CellRange| from.contains_range(b);
    for b in wb.array_blocks_in(&dest) {
        if !within(&b) && !dest.contains_range(&b) {
            return Err(crate::ops::array_block_error());
        }
    }
    if let Some(sh) = wb.sheet(sheet) {
        for m in sh.meta.merges.iter() {
            let mr = CellRange::new(sheet, m.r0, m.c0, m.r1, m.c1);
            if !within(&mr) && mr.intersects(&dest) && !dest.contains_range(&mr) {
                return Err(ApiError::new(
                    crate::ops::MERGED,
                    "You cannot change only part of a merged range.",
                ));
            }
        }
    }

    let names = names_snapshot(wb);
    let meta = meta_parts(
        wb,
        sheet,
        &[
            crate::storage::MetaKind::Merges,
            crate::storage::MetaKind::Notes,
            crate::storage::MetaKind::Links,
        ],
    )
    .ok_or_else(|| ApiError::not_found("no sheet"))?;
    let src = capture_range(wb, from, true, true);
    let dst = capture_range(wb, dest, true, true);

    // Formulas outside the block (and not overwritten) that reference it.
    let inside = |a: CellAddr| a.sheet == sheet && (from.contains(a) || dest.contains(a));
    let touched: Vec<(FormulaCellId, CellAddr)> = wb
        .formulas
        .iter()
        .filter(|(_, f)| {
            !inside(f.addr)
                && f.invalid_text.is_none()
                && f.precedents
                    .iter()
                    .any(|p| p.intersects(&from) || p.intersects(&dest))
        })
        .map(|(id, f)| (id, f.addr))
        .collect();
    let mut outside = Vec::new();
    for (id, addr) in touched {
        let Some(f) = wb.formulas.get(id) else {
            continue;
        };
        let mut nf = (*f.formula).clone();
        if move_refs(&mut nf, addr, addr, &mv) {
            let old = wb.replace_formula_ast(addr, Arc::new(nf));
            outside.push((addr, old));
        }
    }

    clear_range_content(wb, from);
    clear_range_styles(wb, from);
    clear_range_content(wb, dest);
    clear_range_styles(wb, dest);

    // Contents at the destination, references rewritten.
    let (dr, dc) = (mv.dr, mv.dc);
    let at = |r: u32, c: u32| {
        CellAddr::new(
            sheet,
            (i64::from(r) + dr) as u32,
            (i64::from(c) + dc) as u32,
        )
    };
    for (r, c, content) in src.contents.iter().flatten() {
        let old = CellAddr::new(sheet, *r, *c);
        let new = at(*r, *c);
        let content = match content {
            SnapContent::Formula {
                formula,
                invalid_text,
                array,
            } => {
                let mut nf = (**formula).clone();
                if invalid_text.is_none() {
                    move_refs(&mut nf, old, new, &mv);
                }
                SnapContent::Formula {
                    formula: Arc::new(nf),
                    invalid_text: invalid_text.clone(),
                    array: *array,
                }
            }
            v @ SnapContent::Value(_) => v.clone(),
        };
        wb.restore_content(new, Some(content));
    }
    if let Some(sh) = wb.sheet_mut(sheet) {
        for (c, runs) in src.styles.iter().flatten() {
            let nc = (i64::from(*c) + dc) as u32;
            if let Some(col) = sh.column_mut(nc) {
                for (s, e, id) in runs {
                    col.styles.set(
                        (i64::from(*s) + dr) as u32,
                        (i64::from(*e) + dr) as u32,
                        *id,
                    );
                }
            }
        }
    }
    wb.pending.display_only.push(dest);

    // Names follow the block too.
    let mut renamed = false;
    for def in wb.names.defs_mut() {
        let mut f = (*def.formula).clone();
        if move_refs(&mut f, def.base, def.base, &mv) {
            def.formula = Arc::new(f);
            renamed = true;
        }
    }
    if renamed {
        wb.refresh_all_name_dependents();
    }

    // Merges inside the block move; merges in the destination are replaced.
    if let Some(sh) = wb.sheet_mut(sheet) {
        let within = |m: &CellRangeLocal, r: &CellRange| {
            r.r0 <= m.r0 && m.r1 <= r.r1 && r.c0 <= m.c0 && m.c1 <= r.c1
        };
        let mut moved = Vec::new();
        let touches_merge = sh
            .meta
            .merges
            .iter()
            .any(|m| within(m, &from) || within(m, &dest));
        if touches_merge {
            let merges = Arc::make_mut(&mut sh.meta.merges);
            merges.retain(|m| {
                if within(m, &from) {
                    moved.push(shifted(m, dr, dc));
                    false
                } else {
                    !within(m, &dest)
                }
            });
            // (Merges that stay are disjoint from the destination, see above.)
            merges.extend(moved);
        }
        // Notes and hyperlinks move with their cells.
        fn inside_any<T>(
            map: &std::collections::BTreeMap<(u32, u32), T>,
            a: &CellRange,
            b: &CellRange,
        ) -> bool {
            crate::annotations::entries_in(map, a).next().is_some()
                || crate::annotations::entries_in(map, b).next().is_some()
        }
        if inside_any(&sh.meta.notes, &from, &dest) {
            crate::annotations::move_block(Arc::make_mut(&mut sh.meta.notes), &from, dr, dc);
        }
        if inside_any(&sh.meta.links, &from, &dest) {
            crate::annotations::move_block(Arc::make_mut(&mut sh.meta.links), &from, dr, dc);
        }
    }
    fx.layout(sheet);
    Ok(vec![
        names,
        meta,
        Change::Cells(outside),
        Change::Range(src),
        Change::Range(dst),
    ])
}
