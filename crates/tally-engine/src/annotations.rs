//! Per-cell annotations kept in [`crate::storage::SheetMeta`]: notes
//! (comments) and hyperlinks, and how they follow row/column changes,
//! moves and sorts.

use std::collections::BTreeMap;
use std::sync::Arc;

use tally_api::NoteDto;
use tally_types::CellRange;

use crate::structure::{point_after, Shape};

/// A cell note.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    /// Text.
    pub text: Arc<str>,
    /// Author.
    pub author: Option<Arc<str>>,
    /// Creation time (ISO 8601).
    pub created: Option<Arc<str>>,
    /// Always shown.
    pub shown: bool,
}

impl Note {
    /// The API view at `(r, c)`.
    pub fn to_dto(&self, r: u32, c: u32) -> NoteDto {
        NoteDto {
            r,
            c,
            text: self.text.to_string(),
            author: self.author.as_ref().map(|s| s.to_string()),
            created: self.created.as_ref().map(|s| s.to_string()),
            shown: self.shown,
        }
    }

    /// From the model.
    pub fn from_model(n: &tally_model::NoteData) -> Self {
        Self {
            text: n.text.as_str().into(),
            author: n.author.as_deref().map(Into::into),
            created: n.created.as_deref().map(Into::into),
            shown: n.shown,
        }
    }

    /// To the model.
    pub fn to_model(&self, row: u32, col: u32) -> tally_model::NoteData {
        tally_model::NoteData {
            row,
            col,
            text: self.text.to_string(),
            author: self.author.as_ref().map(|s| s.to_string()),
            created: self.created.as_ref().map(|s| s.to_string()),
            shown: self.shown,
        }
    }
}

/// Moves the entries of a `(row, col)` map for a row/column change
/// (entries in deleted rows/columns, or pushed off the grid, are dropped).
pub fn shift_for_structure<T>(map: &mut BTreeMap<(u32, u32), T>, sh: &Shape) {
    if map.is_empty() {
        return;
    }
    let old = std::mem::take(map);
    for ((r, c), v) in old {
        let n = if sh.rows {
            point_after(sh, r).map(|r| (r, c))
        } else {
            point_after(sh, c).map(|c| (r, c))
        };
        if let Some(k) = n {
            map.insert(k, v);
        }
    }
}

/// Moves the entries inside `from` by `(dr, dc)`; entries at the
/// destination are replaced.
pub fn move_block<T>(map: &mut BTreeMap<(u32, u32), T>, from: &CellRange, dr: i64, dc: i64) {
    if map.is_empty() {
        return;
    }
    let inside = |r: u32, c: u32, a: &CellRange| a.r0 <= r && r <= a.r1 && a.c0 <= c && c <= a.c1;
    let mut moved = Vec::new();
    let keys: Vec<(u32, u32)> = map
        .range((from.r0, 0)..=(from.r1, u32::MAX))
        .filter(|((r, c), _)| inside(*r, *c, from))
        .map(|(k, _)| *k)
        .collect();
    for k in keys {
        if let Some(v) = map.remove(&k) {
            moved.push((
                (i64::from(k.0) + dr) as u32,
                (i64::from(k.1) + dc) as u32,
                v,
            ));
        }
    }
    let dest = CellRange::new(
        from.sheet,
        (i64::from(from.r0) + dr) as u32,
        (i64::from(from.c0) + dc) as u32,
        (i64::from(from.r1) + dr) as u32,
        (i64::from(from.c1) + dc) as u32,
    );
    map.retain(|(r, c), _| !inside(*r, *c, &dest));
    for (r, c, v) in moved {
        map.insert((r, c), v);
    }
}

/// Reorders the rows `r0..r0+perm.len()` between columns `c0..=c1`: the
/// entry of source row `r0 + perm[i]` moves to row `r0 + i`.
pub fn permute_rows<T>(map: &mut BTreeMap<(u32, u32), T>, r0: u32, c0: u32, c1: u32, perm: &[u32]) {
    if map.is_empty() || perm.is_empty() {
        return;
    }
    let r1 = r0 + perm.len() as u32 - 1;
    let mut dest_of = vec![0u32; perm.len()];
    for (dst, &src) in perm.iter().enumerate() {
        dest_of[src as usize] = r0 + dst as u32;
    }
    let keys: Vec<(u32, u32)> = map
        .range((r0, 0)..=(r1, u32::MAX))
        .filter(|((_, c), _)| c0 <= *c && *c <= c1)
        .map(|(k, _)| *k)
        .collect();
    let mut moved = Vec::with_capacity(keys.len());
    for k in keys {
        if let Some(v) = map.remove(&k) {
            moved.push((dest_of[(k.0 - r0) as usize], k.1, v));
        }
    }
    for (r, c, v) in moved {
        map.insert((r, c), v);
    }
}

/// Removes the entries inside `range`; `true` if any.
pub fn clear_in<T>(map: &mut BTreeMap<(u32, u32), T>, range: &CellRange) -> bool {
    let before = map.len();
    map.retain(|(r, c), _| !(range.r0 <= *r && *r <= range.r1 && range.c0 <= *c && *c <= range.c1));
    map.len() != before
}

/// The entries inside `range`, row-major.
pub fn entries_in<'a, T>(
    map: &'a BTreeMap<(u32, u32), T>,
    range: &'a CellRange,
) -> impl Iterator<Item = ((u32, u32), &'a T)> + 'a {
    map.range((range.r0, 0)..=(range.r1, u32::MAX))
        .filter(move |((_, c), _)| range.c0 <= *c && *c <= range.c1)
        .map(|(k, v)| (*k, v))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shifts_moves_and_permutes() {
        let mut m: BTreeMap<(u32, u32), &str> = BTreeMap::new();
        m.insert((1, 1), "a");
        m.insert((5, 2), "b");
        let ins = Shape {
            sheet: 1,
            rows: true,
            at: 2,
            count: 3,
            insert: true,
        };
        shift_for_structure(&mut m, &ins);
        assert_eq!(m.get(&(8, 2)), Some(&"b"));
        assert_eq!(m.get(&(1, 1)), Some(&"a"));
        let del = Shape {
            sheet: 1,
            rows: false,
            at: 2,
            count: 1,
            insert: false,
        };
        shift_for_structure(&mut m, &del);
        assert_eq!(m.len(), 1, "column 2 deleted");
        move_block(&mut m, &CellRange::new(1, 0, 0, 2, 2), 10, 1);
        assert_eq!(m.get(&(11, 2)), Some(&"a"));
        let mut p: BTreeMap<(u32, u32), u32> = BTreeMap::new();
        p.insert((10, 0), 0);
        p.insert((11, 0), 1);
        p.insert((12, 5), 2);
        permute_rows(&mut p, 10, 0, 3, &[1, 0, 2]);
        assert_eq!(p.get(&(10, 0)), Some(&1));
        assert_eq!(p.get(&(11, 0)), Some(&0));
        assert_eq!(p.get(&(12, 5)), Some(&2), "outside the columns");
        assert!(clear_in(&mut p, &CellRange::new(1, 0, 0, 20, 0)));
        assert_eq!(p.len(), 1);
    }
}
