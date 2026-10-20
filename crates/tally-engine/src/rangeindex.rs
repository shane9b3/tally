//! A lookup index over rectangles: which of many ranges contain a cell,
//! in O(log n + k) for the common layouts (security review 3,
//! CRITICAL-1 / HIGH-4). Conditional formats and validations keep one
//! per sheet and document version.
//!
//! Identical rectangles are stored once with every id that uses them, so
//! a thousand formats on the same range are one entry. Entries are
//! sorted by first row with a running maximum of the last row, which
//! lets a query stop as soon as no earlier entry can reach its row.

use tally_model::CellRangeLocal;

/// Rectangles with ids.
#[derive(Clone, Debug, Default)]
pub struct RangeIndex<T> {
    /// Distinct rectangles, sorted by `(r0, c0, r1, c1)`.
    rects: Vec<CellRangeLocal>,
    /// The ids of each rectangle (in insertion order).
    ids: Vec<Vec<T>>,
    /// Rectangle positions sorted by `r0`.
    by_r0: Vec<u32>,
    /// `max(r1)` over `by_r0[..=i]`.
    prefix_max_r1: Vec<u32>,
}

impl<T: Copy + Ord> RangeIndex<T> {
    /// Builds the index from `(rectangle, id)` pairs.
    pub fn build(items: impl IntoIterator<Item = (CellRangeLocal, T)>) -> Self {
        let mut pairs: Vec<(CellRangeLocal, T)> = items.into_iter().collect();
        pairs.sort_by_key(|(r, _)| (r.r0, r.c0, r.r1, r.c1));
        let mut rects: Vec<CellRangeLocal> = Vec::new();
        let mut ids: Vec<Vec<T>> = Vec::new();
        for (r, id) in pairs {
            match rects.last() {
                Some(last) if *last == r => {
                    if let Some(v) = ids.last_mut() {
                        if v.last() != Some(&id) {
                            v.push(id);
                        }
                    }
                }
                _ => {
                    rects.push(r);
                    ids.push(vec![id]);
                }
            }
        }
        let mut by_r0: Vec<u32> = (0..rects.len() as u32).collect();
        by_r0.sort_by_key(|&i| rects[i as usize].r0);
        let mut prefix_max_r1 = Vec::with_capacity(by_r0.len());
        let mut m = 0u32;
        for &i in &by_r0 {
            m = m.max(rects[i as usize].r1);
            prefix_max_r1.push(m);
        }
        Self {
            rects,
            ids,
            by_r0,
            prefix_max_r1,
        }
    }

    /// `true` if the index holds nothing.
    pub fn is_empty(&self) -> bool {
        self.rects.is_empty()
    }

    /// The distinct rectangles.
    pub fn rects(&self) -> &[CellRangeLocal] {
        &self.rects
    }

    /// Appends the ids of every rectangle containing `(row, col)` to
    /// `out` (unsorted; an id appears once per rectangle holding it).
    pub fn covering(&self, row: u32, col: u32, out: &mut Vec<T>) {
        let hi = self
            .by_r0
            .partition_point(|&i| self.rects[i as usize].r0 <= row);
        for k in (0..hi).rev() {
            if self.prefix_max_r1[k] < row {
                break;
            }
            let i = self.by_r0[k] as usize;
            let r = &self.rects[i];
            if r.r1 >= row && r.c0 <= col && col <= r.c1 {
                out.extend_from_slice(&self.ids[i]);
            }
        }
    }

    /// The ids covering `(row, col)`, sorted and deduplicated.
    pub fn covering_sorted(&self, row: u32, col: u32, out: &mut Vec<T>) {
        out.clear();
        self.covering(row, col, out);
        if out.len() > 1 {
            out.sort_unstable();
            out.dedup();
        }
    }

    /// The distinct rectangles intersecting `range` (clipped to it).
    pub fn intersections(&self, range: &CellRangeLocal) -> Vec<CellRangeLocal> {
        let hi = self
            .by_r0
            .partition_point(|&i| self.rects[i as usize].r0 <= range.r1);
        let mut out = Vec::new();
        for k in (0..hi).rev() {
            if self.prefix_max_r1[k] < range.r0 {
                break;
            }
            let r = &self.rects[self.by_r0[k] as usize];
            if r.r1 >= range.r0 && r.c0 <= range.c1 && range.c0 <= r.c1 {
                out.push(CellRangeLocal {
                    r0: r.r0.max(range.r0),
                    c0: r.c0.max(range.c0),
                    r1: r.r1.min(range.r1),
                    c1: r.c1.min(range.c1),
                });
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(r0: u32, c0: u32, r1: u32, c1: u32) -> CellRangeLocal {
        CellRangeLocal { r0, c0, r1, c1 }
    }

    #[test]
    fn finds_covering_rectangles_and_dedupes_identical_ones() {
        // 1,000 formats on one rectangle, 1,000 disjoint blocks, one tall
        // column range.
        let mut items: Vec<(CellRangeLocal, u32)> =
            (0..1000).map(|i| (r(0, 0, 999, 19), i)).collect();
        items.extend((0..1000u32).map(|j| (r(j * 1000, 0, j * 1000 + 999, 19), 1000 + j)));
        items.push((r(0, 5, 1_000_000, 5), 5000));
        let idx = RangeIndex::build(items);
        assert_eq!(
            idx.rects().len(),
            1001,
            "identical rectangles are stored once"
        );
        let mut out = Vec::new();
        idx.covering_sorted(500, 3, &mut out);
        assert_eq!(out.len(), 1001, "{out:?}");
        assert_eq!(out[0], 0);
        assert_eq!(out[1000], 1000);
        idx.covering_sorted(500, 5, &mut out);
        assert_eq!(out.len(), 1002);
        assert_eq!(out[1001], 5000);
        idx.covering_sorted(123_456, 0, &mut out);
        assert_eq!(out, vec![1123]);
        idx.covering_sorted(123_456, 25, &mut out);
        assert!(out.is_empty());
        let hits = idx.intersections(&r(999_000, 0, 999_999, 19));
        assert_eq!(hits.len(), 2, "the block and the column: {hits:?}");
        assert!(RangeIndex::<u32>::build(Vec::new()).is_empty());
    }
}
