//! The reverse dependency index: which formulas read a changed cell
//! (plan §3.6, docs/phase-a.md "Dependency graph").
//!
//! Single-cell precedents go into a hash map from the cell to its readers;
//! area precedents stay unexpanded as `(CellRange, FormulaCellId)` per
//! sheet (a whole-column reference is one entry). Both are removed in O(1)
//! with a back-index (each formula records the list positions of its
//! entries) and `swap_remove`, so a cell read by n formulas (200k cells
//! `=A2*$B$1`) costs O(n) to build and O(1) per formula to change, not
//! O(n²) (security review 2, HIGH-3).
//!
//! 3-D precedents stay unexpanded too: one `(Span3d, FormulaCellId)` entry
//! per reference, matched by tab position (the index keeps the tab order),
//! so a span over thousands of sheets costs one entry, not one per sheet.

use std::collections::{HashMap, HashSet};

use slotmap::SecondaryMap;
use smallvec::SmallVec;
use tally_types::{CellAddr, CellRange, SheetId};

use crate::formula_cells::{FormulaCellId, Span3d};

/// Area entries per sheet beyond which a warning is due (plan: ≤20k ok).
pub const AREA_WARN_LIMIT: usize = 50_000;

/// The reverse index from cells to the formulas reading them.
pub trait DepIndex {
    /// Registers formula `id` as reading `precedents`.
    fn add(&mut self, id: FormulaCellId, precedents: &[CellRange]);
    /// Unregisters every precedent of `id`.
    fn remove(&mut self, id: FormulaCellId);
    /// Appends the formulas reading any cell of `changed` (may contain duplicates).
    fn dependents_of(&self, changed: &[CellRange], out: &mut Vec<FormulaCellId>);
    /// Marks or unmarks `id` as volatile.
    fn set_volatile(&mut self, id: FormulaCellId, volatile: bool);
    /// Every volatile formula.
    fn volatile(&self) -> Vec<FormulaCellId>;
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Loc {
    /// Position in `singles[addr]`.
    Single(CellAddr, usize),
    /// Position in `areas[sheet]`.
    Area(SheetId, usize),
    /// Position in `spans`.
    Span(usize),
}

/// A registered 3-D span with its tab positions (lowest, highest) under
/// the current tab order (`None` while an end sheet is unknown).
#[derive(Clone, Debug)]
struct SpanEntry {
    span: Span3d,
    id: FormulaCellId,
    tabs: Option<(u32, u32)>,
}

/// The Phase A [`DepIndex`].
#[derive(Clone, Debug, Default)]
pub struct HashDepIndex {
    singles: HashMap<CellAddr, SmallVec<[FormulaCellId; 2]>>,
    areas: HashMap<SheetId, Vec<(CellRange, FormulaCellId)>>,
    spans: Vec<SpanEntry>,
    /// Tab position of each sheet (for matching spans).
    tabs: HashMap<SheetId, u32>,
    back: SecondaryMap<FormulaCellId, SmallVec<[Loc; 4]>>,
    volatile: HashSet<FormulaCellId>,
    /// Name key (lowercase) → formulas using that name (directly or
    /// through other names).
    names: HashMap<String, HashSet<FormulaCellId>>,
    /// Formula → the name keys it is registered under.
    name_back: SecondaryMap<FormulaCellId, Vec<String>>,
    /// Bumped by every registration change (a formula added, removed or
    /// re-pointed): the detective's AutoRefresh redraws only when it moved
    /// (AOO `ScDocument::IsDetectiveDirty`, set in `StartListeningTo` /
    /// `EndListeningTo`).
    generation: u64,
}

impl HashDepIndex {
    /// An empty index.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of area entries on `sheet`.
    pub fn area_count(&self, sheet: SheetId) -> usize {
        self.areas.get(&sheet).map_or(0, Vec::len)
    }

    /// Number of registered 3-D spans.
    pub fn span_count(&self) -> usize {
        self.spans.len()
    }

    /// Sets the tab order spans are matched against.
    pub fn set_tab_order(&mut self, order: &[SheetId]) {
        self.tabs = order
            .iter()
            .enumerate()
            .map(|(i, s)| (*s, i as u32))
            .collect();
        for e in &mut self.spans {
            e.tabs = span_tabs(&self.tabs, &e.span);
        }
    }

    /// Records a sheet appended at tab position `at` (no other sheet moved).
    pub fn append_tab(&mut self, sheet: SheetId, at: u32) {
        self.tabs.insert(sheet, at);
        for e in &mut self.spans {
            if e.tabs.is_none() {
                e.tabs = span_tabs(&self.tabs, &e.span);
            }
        }
    }

    /// Registers formula `id` as reading the 3-D spans `spans` (in
    /// addition to what [`DepIndex::add`] registered).
    pub fn add_spans(&mut self, id: FormulaCellId, spans: &[Span3d]) {
        if spans.is_empty() {
            return;
        }
        self.generation = self.generation.wrapping_add(1);
        let mut locs: SmallVec<[Loc; 4]> = self.back.remove(id).unwrap_or_default();
        for s in spans {
            self.spans.push(SpanEntry {
                span: *s,
                id,
                tabs: span_tabs(&self.tabs, s),
            });
            locs.push(Loc::Span(self.spans.len() - 1));
        }
        self.back.insert(id, locs);
    }

    /// The registration generation (see the field).
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Number of single-cell keys.
    pub fn single_count(&self) -> usize {
        self.singles.len()
    }

    /// Drops everything.
    pub fn clear(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.singles.clear();
        self.areas.clear();
        self.spans.clear();
        self.back.clear();
        self.volatile.clear();
        self.names.clear();
        self.name_back.clear();
    }

    /// Registers formula `id` as using the names `keys` (lowercase),
    /// replacing its previous name registrations.
    pub fn set_names(&mut self, id: FormulaCellId, keys: &[String]) {
        self.remove_names(id);
        if keys.is_empty() {
            return;
        }
        let mut uniq: Vec<String> = keys.to_vec();
        uniq.sort();
        uniq.dedup();
        for k in &uniq {
            self.names.entry(k.clone()).or_default().insert(id);
        }
        self.name_back.insert(id, uniq);
    }

    /// Unregisters the names of `id`.
    pub fn remove_names(&mut self, id: FormulaCellId) {
        if let Some(keys) = self.name_back.remove(id) {
            for k in keys {
                if let Some(v) = self.names.get_mut(&k) {
                    v.remove(&id);
                    if v.is_empty() {
                        self.names.remove(&k);
                    }
                }
            }
        }
    }

    /// The formulas using name `key` (lowercase).
    pub fn name_dependents(&self, key: &str) -> Vec<FormulaCellId> {
        let mut v: Vec<FormulaCellId> = self
            .names
            .get(key)
            .map(|s| s.iter().copied().collect())
            .unwrap_or_default();
        v.sort_unstable();
        v
    }

    /// Every formula registered under some name.
    pub fn all_name_dependents(&self) -> Vec<FormulaCellId> {
        self.name_back.keys().collect()
    }

    /// Unregisters everything about `id` (references, volatility, names):
    /// for a formula cell that goes away.
    pub fn forget(&mut self, id: FormulaCellId) {
        self.remove(id);
        self.remove_names(id);
    }

    /// The ranges `id` is registered for (static plus dynamic), for tests.
    pub fn registered_ranges(&self, id: FormulaCellId) -> Vec<CellRange> {
        let Some(locs) = self.back.get(id) else {
            return Vec::new();
        };
        locs.iter()
            .filter_map(|l| match l {
                Loc::Single(a, _) => Some(CellRange::single(*a)),
                Loc::Area(sheet, i) => self.areas.get(sheet).and_then(|v| v.get(*i)).map(|e| e.0),
                Loc::Span(i) => self.spans.get(*i).map(|e| e.span.rect),
            })
            .collect()
    }
}

impl DepIndex for HashDepIndex {
    fn add(&mut self, id: FormulaCellId, precedents: &[CellRange]) {
        self.generation = self.generation.wrapping_add(1);
        let mut locs: SmallVec<[Loc; 4]> = self.back.remove(id).unwrap_or_default();
        // Each cell is registered once per formula: deduplicate against this
        // formula's own entries, never by scanning the cell's reader list.
        // Small formulas (the common case) scan their own few entries.
        let small = precedents.len() + locs.len() <= 16;
        let mut have: HashSet<CellAddr> = if small {
            HashSet::new()
        } else {
            locs.iter()
                .filter_map(|l| match l {
                    Loc::Single(a, _) => Some(*a),
                    Loc::Area(..) | Loc::Span(_) => None,
                })
                .collect()
        };
        for r in precedents {
            if r.is_single() {
                let a = r.start();
                let fresh = if small {
                    !locs
                        .iter()
                        .any(|l| matches!(l, Loc::Single(x, _) if *x == a))
                } else {
                    have.insert(a)
                };
                if fresh {
                    let list = self.singles.entry(a).or_default();
                    list.push(id);
                    locs.push(Loc::Single(a, list.len() - 1));
                }
            } else {
                let v = self.areas.entry(r.sheet).or_default();
                v.push((*r, id));
                locs.push(Loc::Area(r.sheet, v.len() - 1));
            }
        }
        if !locs.is_empty() {
            self.back.insert(id, locs);
        }
    }

    fn remove(&mut self, id: FormulaCellId) {
        self.generation = self.generation.wrapping_add(1);
        self.volatile.remove(&id);
        let Some(mut locs) = self.back.remove(id) else {
            return;
        };
        // Areas and spans in descending index order, so an entry moved by
        // swap_remove never belongs to `id` itself.
        let rank = |l: &Loc| match l {
            Loc::Span(_) => 0,
            Loc::Area(..) => 1,
            Loc::Single(..) => 2,
        };
        locs.sort_by(|a, b| match (a, b) {
            (Loc::Area(_, i), Loc::Area(_, j)) | (Loc::Span(i), Loc::Span(j)) => j.cmp(i),
            _ => rank(a).cmp(&rank(b)),
        });
        for loc in locs {
            match loc {
                Loc::Single(a, idx) => {
                    let Some(list) = self.singles.get_mut(&a) else {
                        continue;
                    };
                    if idx >= list.len() || list[idx] != id {
                        continue;
                    }
                    let last = list.len() - 1;
                    list.swap_remove(idx);
                    if idx < last {
                        let moved = list[idx];
                        if let Some(ml) = self.back.get_mut(moved) {
                            for l in ml.iter_mut() {
                                if *l == Loc::Single(a, last) {
                                    *l = Loc::Single(a, idx);
                                    break;
                                }
                            }
                        }
                    }
                    if list.is_empty() {
                        self.singles.remove(&a);
                    }
                }
                Loc::Span(idx) => {
                    if idx >= self.spans.len() || self.spans[idx].id != id {
                        continue;
                    }
                    let last = self.spans.len() - 1;
                    self.spans.swap_remove(idx);
                    if idx < last {
                        let moved = self.spans[idx].id;
                        if let Some(ml) = self.back.get_mut(moved) {
                            for l in ml.iter_mut() {
                                if *l == Loc::Span(last) {
                                    *l = Loc::Span(idx);
                                    break;
                                }
                            }
                        }
                    }
                }
                Loc::Area(sheet, idx) => {
                    let Some(v) = self.areas.get_mut(&sheet) else {
                        continue;
                    };
                    if idx >= v.len() {
                        continue;
                    }
                    let last = v.len() - 1;
                    v.swap_remove(idx);
                    if idx < last {
                        let moved = v[idx].1;
                        if let Some(ml) = self.back.get_mut(moved) {
                            for l in ml.iter_mut() {
                                if *l == Loc::Area(sheet, last) {
                                    *l = Loc::Area(sheet, idx);
                                    break;
                                }
                            }
                        }
                    }
                    if v.is_empty() {
                        self.areas.remove(&sheet);
                    }
                }
            }
        }
    }

    fn dependents_of(&self, changed: &[CellRange], out: &mut Vec<FormulaCellId>) {
        for r in changed {
            if r.is_single() {
                if let Some(list) = self.singles.get(&r.start()) {
                    out.extend(list.iter().copied());
                }
            } else if r.cell_count() <= self.singles.len() as u64 {
                for a in r.cells() {
                    if let Some(list) = self.singles.get(&a) {
                        out.extend(list.iter().copied());
                    }
                }
            } else {
                for (a, list) in &self.singles {
                    if r.contains(*a) {
                        out.extend(list.iter().copied());
                    }
                }
            }
            if let Some(v) = self.areas.get(&r.sheet) {
                for (area, id) in v {
                    if area.intersects(r) {
                        out.push(*id);
                    }
                }
            }
        }
        if self.spans.is_empty() {
            return;
        }
        // Each span is reported once however many changed ranges it covers
        // (thousands of edited sheets under one span must not produce
        // sheets × spans entries).
        let changed: Vec<(u32, &CellRange)> = changed
            .iter()
            .filter_map(|r| self.tabs.get(&r.sheet).map(|t| (*t, r)))
            .collect();
        for e in &self.spans {
            let Some((lo, hi)) = e.tabs else {
                continue;
            };
            if changed
                .iter()
                .any(|(t, r)| lo <= *t && *t <= hi && e.span.rect_hits(r))
            {
                out.push(e.id);
            }
        }
    }

    fn set_volatile(&mut self, id: FormulaCellId, volatile: bool) {
        if volatile {
            self.volatile.insert(id);
        } else {
            self.volatile.remove(&id);
        }
    }

    fn volatile(&self) -> Vec<FormulaCellId> {
        self.volatile.iter().copied().collect()
    }
}

fn span_tabs(tabs: &HashMap<SheetId, u32>, s: &Span3d) -> Option<(u32, u32)> {
    let (a, b) = (*tabs.get(&s.first)?, *tabs.get(&s.last)?);
    Some((a.min(b), a.max(b)))
}

/// Coalesces cells and ranges into fewer rectangles: cells of one column
/// merge into vertical runs, then runs with equal rows in adjacent columns
/// merge. Multi-cell input ranges are kept as they are.
pub fn coalesce(cells: &[CellAddr], ranges: &[CellRange]) -> Vec<CellRange> {
    let mut sorted: Vec<CellAddr> = cells.to_vec();
    sorted.sort_by_key(|a| (a.sheet, a.col, a.row));
    sorted.dedup();
    let mut runs: Vec<CellRange> = Vec::new();
    for a in sorted {
        if let Some(last) = runs.last_mut() {
            if last.sheet == a.sheet && last.c0 == a.col && last.r1.checked_add(1) == Some(a.row) {
                last.r1 = a.row;
                continue;
            }
        }
        runs.push(CellRange::single(a));
    }
    runs.sort_by_key(|r| (r.sheet, r.r0, r.r1, r.c0));
    let mut out: Vec<CellRange> = Vec::with_capacity(runs.len() + ranges.len());
    for r in runs {
        if let Some(last) = out.last_mut() {
            if last.sheet == r.sheet
                && last.r0 == r.r0
                && last.r1 == r.r1
                && last.c1.checked_add(1) == Some(r.c0)
            {
                last.c1 = r.c1;
                continue;
            }
        }
        out.push(r);
    }
    for r in ranges {
        if !out.iter().any(|o| o.contains_range(r)) {
            out.push(*r);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use slotmap::SlotMap;

    fn ids(n: usize) -> Vec<FormulaCellId> {
        let mut sm: SlotMap<FormulaCellId, ()> = SlotMap::with_key();
        (0..n).map(|_| sm.insert(())).collect()
    }

    fn r(a1: &str) -> CellRange {
        CellRange::from_a1(0, a1).unwrap_or_else(|| panic!("{a1}"))
    }

    #[test]
    fn singles_and_areas() {
        let id = ids(3);
        let mut d = HashDepIndex::new();
        d.add(id[0], &[r("A1")]);
        d.add(id[1], &[r("A1:A10"), r("B1")]);
        d.add(
            id[2],
            &[CellRange::new(0, 0, 2, tally_types::MAX_ROWS - 1, 2)],
        );
        let mut out = Vec::new();
        d.dependents_of(&[r("A1")], &mut out);
        out.sort();
        assert_eq!(out, {
            let mut v = vec![id[0], id[1]];
            v.sort();
            v
        });
        out.clear();
        d.dependents_of(&[r("C999999")], &mut out);
        assert_eq!(out, vec![id[2]]);
        out.clear();
        d.dependents_of(&[r("B1:B5")], &mut out);
        assert_eq!(out, vec![id[1]]);
    }

    #[test]
    fn remove_with_swap_keeps_back_index() {
        let id = ids(4);
        let mut d = HashDepIndex::new();
        d.add(id[0], &[r("A1:A2"), r("C1:C2")]);
        d.add(id[1], &[r("B1:B2")]);
        d.add(id[2], &[r("D1:D2")]);
        d.add(id[3], &[r("A1:B2"), r("E1:E2")]);
        assert_eq!(d.area_count(0), 6);
        d.remove(id[0]);
        assert_eq!(d.area_count(0), 4);
        d.remove(id[3]);
        assert_eq!(d.area_count(0), 2);
        let mut out = Vec::new();
        d.dependents_of(&[r("A1:E2")], &mut out);
        out.sort();
        let mut expect = vec![id[1], id[2]];
        expect.sort();
        assert_eq!(out, expect);
        d.remove(id[1]);
        d.remove(id[2]);
        assert_eq!(d.area_count(0), 0);
        assert_eq!(d.single_count(), 0);
    }

    #[test]
    fn many_readers_of_one_cell_stay_linear() {
        // HIGH-3: 200k formulas reading one cell; removing them one by one
        // in any order keeps the index consistent and is O(1) each.
        let n = 200_000;
        let id = ids(n);
        let mut d = HashDepIndex::new();
        let t = std::time::Instant::now();
        for (i, x) in id.iter().enumerate() {
            d.add(*x, &[r("B1"), r(&format!("A{}", i + 1)), r("B1")]);
        }
        let mut out = Vec::new();
        d.dependents_of(&[r("B1")], &mut out);
        assert_eq!(out.len(), n, "registered once per formula");
        for x in id.iter().step_by(2) {
            d.remove(*x);
        }
        out.clear();
        d.dependents_of(&[r("B1")], &mut out);
        assert_eq!(out.len(), n / 2);
        out.sort();
        let mut odd: Vec<FormulaCellId> = id.iter().skip(1).step_by(2).copied().collect();
        odd.sort();
        assert_eq!(out, odd);
        for x in id.iter().skip(1).step_by(2) {
            d.remove(*x);
        }
        assert_eq!(d.single_count(), 0);
        let took = t.elapsed();
        assert!(took < std::time::Duration::from_secs(5), "{took:?}");
    }

    #[test]
    fn names_map() {
        let id = ids(2);
        let mut d = HashDepIndex::new();
        d.set_names(id[0], &["total".into(), "rate".into()]);
        d.set_names(id[1], &["total".into()]);
        let mut t = d.name_dependents("total");
        t.sort();
        let mut e = vec![id[0], id[1]];
        e.sort();
        assert_eq!(t, e);
        d.add(id[0], &[r("A1")]);
        d.remove(id[0]);
        assert_eq!(d.name_dependents("rate"), vec![id[0]], "remove keeps names");
        d.forget(id[0]);
        assert!(d.name_dependents("rate").is_empty());
        assert_eq!(d.name_dependents("total"), vec![id[1]]);
        d.set_names(id[1], &[]);
        assert!(d.name_dependents("total").is_empty());
    }

    #[test]
    fn dynamic_ranges_are_indexed_like_static_ones() {
        let id = ids(1);
        let mut d = HashDepIndex::new();
        // Static A1, dynamic B5:B9 (as re-registered after an evaluation).
        d.add(id[0], &[r("A1"), r("B5:B9")]);
        let mut out = Vec::new();
        d.dependents_of(&[r("B7")], &mut out);
        assert_eq!(out, vec![id[0]]);
        assert_eq!(d.registered_ranges(id[0]).len(), 2);
        // Re-collected: now only C1.
        d.remove(id[0]);
        d.add(id[0], &[r("A1"), r("C1")]);
        out.clear();
        d.dependents_of(&[r("B7")], &mut out);
        assert!(out.is_empty());
        d.dependents_of(&[r("C1")], &mut out);
        assert_eq!(out, vec![id[0]]);
    }

    #[test]
    fn coalesce_runs() {
        let cells: Vec<CellAddr> = ["A1", "A2", "A3", "B1", "B2", "B3", "D7"]
            .iter()
            .filter_map(|s| CellAddr::from_a1(0, s))
            .collect();
        let out = coalesce(&cells, &[]);
        assert_eq!(out, vec![r("A1:B3"), r("D7")]);
    }
}
