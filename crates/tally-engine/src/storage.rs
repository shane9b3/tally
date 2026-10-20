//! Sparse, columnar cell storage (plan §3.2).
//!
//! - [`ColumnStore`] is the per-column cell map; Phase A implements it with a
//!   `BTreeMap<u32, CellContent>` ([`BTreeColumn`]). A block-run store can
//!   replace it later behind the same trait.
//! - Styles are stored separately per column as run-length ranges
//!   ([`StyleRuns`]), so styling a whole column is one entry, not 1M.
//! - Column widths and row heights are run-length [`DimRuns`] in 1/100 mm.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use tally_types::{CellRange, SheetId, Value, MAX_COLS, MAX_ROWS};

use crate::formula_cells::FormulaCellId;

/// Index into the workbook's [`crate::styles::StyleTable`]; `0` is the default style.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct StyleId(pub u32);

impl StyleId {
    /// The default style.
    pub const DEFAULT: StyleId = StyleId(0);
}

/// What a cell holds. Formula results live in the [`crate::formula_cells`] slot map.
#[derive(Clone, Debug, PartialEq)]
pub enum CellContent {
    /// A constant (never [`Value::Empty`]; empty cells are absent).
    Value(Value),
    /// A formula cell.
    Formula(FormulaCellId),
    /// A covered cell of an array formula block; the id is the anchor's.
    ArrayPart(FormulaCellId),
}

impl CellContent {
    /// The formula cell this content belongs to (itself or its array anchor).
    pub fn formula_id(&self) -> Option<FormulaCellId> {
        match self {
            CellContent::Formula(id) | CellContent::ArrayPart(id) => Some(*id),
            CellContent::Value(_) => None,
        }
    }
}

/// Direction along one column for [`ColumnStore::next_non_empty`].
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Dir {
    /// Towards row 0.
    Up,
    /// Towards the last row.
    Down,
}

/// The cells of one column.
pub trait ColumnStore {
    /// The content at `row`.
    fn get(&self, row: u32) -> Option<&CellContent>;
    /// Stores `content` at `row`, returning the previous content.
    fn set(&mut self, row: u32, content: CellContent) -> Option<CellContent>;
    /// Removes the content at `row`.
    fn remove(&mut self, row: u32) -> Option<CellContent>;
    /// Non-empty cells in `r0..=r1`, top to bottom.
    fn iter_range(&self, r0: u32, r1: u32) -> impl Iterator<Item = (u32, &CellContent)> + '_;
    /// The nearest non-empty row strictly after (`Down`) or before (`Up`) `from`.
    fn next_non_empty(&self, from: u32, dir: Dir) -> Option<u32>;
    /// The last non-empty row.
    fn last_non_empty(&self) -> Option<u32>;
    /// The first non-empty row.
    fn first_non_empty(&self) -> Option<u32>;
    /// Shifts rows `>= at` down by `count` (cells pushed past the sheet end are dropped and returned).
    fn insert_rows(&mut self, at: u32, count: u32) -> Vec<(u32, CellContent)>;
    /// Removes rows `at..at+count` (returned) and shifts the rest up.
    fn delete_rows(&mut self, at: u32, count: u32) -> Vec<(u32, CellContent)>;
    /// Number of non-empty cells.
    fn len(&self) -> usize;
    /// `true` if the column has no cells.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The Phase A [`ColumnStore`]: a `BTreeMap` keyed by row.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BTreeColumn {
    cells: BTreeMap<u32, CellContent>,
}

impl ColumnStore for BTreeColumn {
    fn get(&self, row: u32) -> Option<&CellContent> {
        self.cells.get(&row)
    }

    fn set(&mut self, row: u32, content: CellContent) -> Option<CellContent> {
        self.cells.insert(row, content)
    }

    fn remove(&mut self, row: u32) -> Option<CellContent> {
        self.cells.remove(&row)
    }

    fn iter_range(&self, r0: u32, r1: u32) -> impl Iterator<Item = (u32, &CellContent)> + '_ {
        let (lo, hi) = if r0 <= r1 { (r0, r1) } else { (1, 0) };
        let range = if lo <= hi {
            Some(self.cells.range(lo..=hi))
        } else {
            None
        };
        range.into_iter().flatten().map(|(r, c)| (*r, c))
    }

    fn next_non_empty(&self, from: u32, dir: Dir) -> Option<u32> {
        match dir {
            Dir::Down => self
                .cells
                .range(from.saturating_add(1)..)
                .next()
                .map(|(r, _)| *r)
                .filter(|r| *r > from),
            Dir::Up => self.cells.range(..from).next_back().map(|(r, _)| *r),
        }
    }

    fn last_non_empty(&self) -> Option<u32> {
        self.cells.keys().next_back().copied()
    }

    fn first_non_empty(&self) -> Option<u32> {
        self.cells.keys().next().copied()
    }

    fn insert_rows(&mut self, at: u32, count: u32) -> Vec<(u32, CellContent)> {
        let moved = self.cells.split_off(&at);
        let mut dropped = Vec::new();
        for (r, c) in moved {
            let nr = u64::from(r) + u64::from(count);
            if nr < u64::from(MAX_ROWS) {
                self.cells.insert(nr as u32, c);
            } else {
                dropped.push((r, c));
            }
        }
        dropped
    }

    fn delete_rows(&mut self, at: u32, count: u32) -> Vec<(u32, CellContent)> {
        let mut tail = self.cells.split_off(&at);
        let end = at.saturating_add(count);
        let rest = tail.split_off(&end);
        for (r, c) in rest {
            self.cells.insert(r - count, c);
        }
        tail.into_iter().collect()
    }

    fn len(&self) -> usize {
        self.cells.len()
    }
}

/// Run-length style ranges of one column: `start -> (end, style)`, sorted,
/// non-overlapping, never holding [`StyleId::DEFAULT`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StyleRuns {
    runs: BTreeMap<u32, (u32, StyleId)>,
}

impl StyleRuns {
    /// The style at `row`.
    pub fn get(&self, row: u32) -> StyleId {
        match self.runs.range(..=row).next_back() {
            Some((_, (end, id))) if *end >= row => *id,
            _ => StyleId::DEFAULT,
        }
    }

    /// `true` if every row has the default style.
    pub fn is_empty(&self) -> bool {
        self.runs.is_empty()
    }

    /// Number of runs.
    pub fn run_count(&self) -> usize {
        self.runs.len()
    }

    /// Non-default runs intersecting `r0..=r1`, clipped to it.
    pub fn runs_in(&self, r0: u32, r1: u32) -> Vec<(u32, u32, StyleId)> {
        let mut out = Vec::new();
        if r0 > r1 {
            return out;
        }
        if let Some((s, (e, id))) = self.runs.range(..r0).next_back() {
            if *e >= r0 {
                out.push((r0.max(*s), r1.min(*e), *id));
            }
        }
        for (s, (e, id)) in self.runs.range(r0..=r1) {
            out.push((*s, r1.min(*e), *id));
        }
        out
    }

    /// The last row with a non-default style.
    pub fn last_styled_row(&self) -> Option<u32> {
        self.runs.values().next_back().map(|(e, _)| *e)
    }

    /// Sets rows `r0..=r1` to `id`.
    pub fn set(&mut self, r0: u32, r1: u32, id: StyleId) {
        if r0 > r1 {
            return;
        }
        self.cut(r0, r1);
        if id != StyleId::DEFAULT {
            self.runs.insert(r0, (r1, id));
            self.merge_around(r0);
        }
    }

    /// Removes coverage of `r0..=r1`, splitting runs that straddle it.
    fn cut(&mut self, r0: u32, r1: u32) {
        // A run starting before r0 that reaches into the range.
        if let Some((&s, &(e, id))) = self.runs.range(..r0).next_back() {
            if e >= r0 {
                self.runs.insert(s, (r0 - 1, id));
                if e > r1 {
                    self.runs.insert(r1 + 1, (e, id));
                }
            }
        }
        let inside: Vec<u32> = self.runs.range(r0..=r1).map(|(s, _)| *s).collect();
        for s in inside {
            if let Some((e, id)) = self.runs.remove(&s) {
                if e > r1 {
                    self.runs.insert(r1 + 1, (e, id));
                }
            }
        }
    }

    fn merge_around(&mut self, start: u32) {
        let Some(&(end, id)) = self.runs.get(&start) else {
            return;
        };
        let (mut s, mut e) = (start, end);
        if let Some((&ps, &(pe, pid))) = self.runs.range(..start).next_back() {
            if pid == id && pe.checked_add(1) == Some(start) {
                self.runs.remove(&start);
                s = ps;
                self.runs.insert(ps, (e, id));
            }
        }
        if let Some(next) = e.checked_add(1) {
            if let Some(&(ne, nid)) = self.runs.get(&next) {
                if nid == id {
                    self.runs.remove(&next);
                    e = ne;
                    self.runs.insert(s, (e, id));
                }
            }
        }
    }

    /// Replaces the style of every row in `r0..=r1` by `f(old)`, segment by
    /// segment (gaps count as the default style).
    pub fn map(&mut self, r0: u32, r1: u32, mut f: impl FnMut(StyleId) -> StyleId) {
        if r0 > r1 {
            return;
        }
        let mut segs: Vec<(u32, u32, StyleId)> = Vec::new();
        let mut cur = r0;
        for (s, e, id) in self.runs_in(r0, r1) {
            if s > cur {
                segs.push((cur, s - 1, StyleId::DEFAULT));
            }
            segs.push((s, e, id));
            cur = e.saturating_add(1);
            if e == u32::MAX {
                break;
            }
        }
        if cur <= r1 && (segs.is_empty() || segs.last().is_some_and(|l| l.1 < r1)) {
            segs.push((cur, r1, StyleId::DEFAULT));
        }
        for (s, e, old) in segs {
            let new = f(old);
            if new != old {
                self.set(s, e, new);
            }
        }
    }

    /// Shifts runs for inserted rows. Inserted rows take the style of the
    /// row above them, as in AOO (`ScAttrArray::InsertRow`): rows inserted
    /// inside a run or directly below one extend it.
    pub fn insert_rows(&mut self, at: u32, count: u32) {
        let old = std::mem::take(&mut self.runs);
        let max = MAX_ROWS - 1;
        for (s, (e, id)) in old {
            if e < at {
                if at > 0 && e == at - 1 {
                    self.set(s, e.saturating_add(count).min(max), id);
                } else {
                    self.set(s, e, id);
                }
            } else if s >= at {
                let ns = u64::from(s) + u64::from(count);
                if ns <= u64::from(max) {
                    self.set(ns as u32, e.saturating_add(count).min(max), id);
                }
            } else {
                self.set(s, e.saturating_add(count).min(max), id);
            }
        }
    }

    /// Removes rows `at..at+count` from the runs.
    pub fn delete_rows(&mut self, at: u32, count: u32) {
        let old = std::mem::take(&mut self.runs);
        let end = at.saturating_add(count);
        for (s, (e, id)) in old {
            let mut lo: Option<u32> = None;
            let mut hi: Option<u32> = None;
            if s < at {
                lo = Some(s);
                hi = Some(e.min(at - 1));
            }
            if e >= end {
                let a = s.max(end) - count;
                lo = Some(lo.unwrap_or(a));
                hi = Some(e - count);
            }
            if let (Some(a), Some(b)) = (lo, hi) {
                self.set(a, b, id);
            }
        }
    }
}

/// Run-length sizes of rows or columns in 1/100 mm.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DimRuns {
    runs: Vec<tally_model::DimRun>,
}

impl DimRuns {
    /// Builds from runs (sorted and clipped as given).
    pub fn from_runs(runs: &[tally_model::DimRun]) -> Self {
        let mut d = DimRuns::default();
        for r in runs {
            d.set(r.start, r.end, r.size_hmm, r.hidden);
        }
        d
    }

    /// The runs, sorted and non-overlapping.
    pub fn runs(&self) -> &[tally_model::DimRun] {
        &self.runs
    }

    /// Size of index `i`, if it differs from the default.
    pub fn get(&self, i: u32) -> Option<&tally_model::DimRun> {
        let pos = self.runs.partition_point(|r| r.end < i);
        self.runs.get(pos).filter(|r| r.start <= i)
    }

    /// Sets `start..=end` to `size_hmm`.
    pub fn set(&mut self, start: u32, end: u32, size_hmm: u32, hidden: bool) {
        if start > end {
            return;
        }
        let mut out: Vec<tally_model::DimRun> = Vec::with_capacity(self.runs.len() + 2);
        for r in self.runs.drain(..) {
            if r.end < start || r.start > end {
                out.push(r);
                continue;
            }
            if r.start < start {
                out.push(tally_model::DimRun {
                    start: r.start,
                    end: start - 1,
                    ..r.clone()
                });
            }
            if r.end > end {
                out.push(tally_model::DimRun {
                    start: end + 1,
                    end: r.end,
                    ..r.clone()
                });
            }
        }
        out.push(tally_model::DimRun {
            start,
            end,
            size_hmm,
            hidden,
        });
        out.sort_by_key(|r| r.start);
        // Merge equal neighbours.
        let mut merged: Vec<tally_model::DimRun> = Vec::with_capacity(out.len());
        for r in out {
            if let Some(last) = merged.last_mut() {
                if last.end.checked_add(1) == Some(r.start)
                    && last.size_hmm == r.size_hmm
                    && last.hidden == r.hidden
                {
                    last.end = r.end;
                    continue;
                }
            }
            merged.push(r);
        }
        self.runs = merged;
    }
}

impl DimRuns {
    /// Hides or shows `start..=end`, keeping each index's size
    /// (`default_hmm` for indices without a run). Shown runs of the
    /// default size are dropped.
    pub fn set_hidden(&mut self, start: u32, end: u32, hidden: bool, default_hmm: u32) {
        if start > end {
            return;
        }
        let mut segs: Vec<(u32, u32, u32)> = Vec::new();
        let mut cur = start;
        let mut i = self.runs.partition_point(|r| r.end < start);
        while cur <= end {
            match self.runs.get(i) {
                Some(r) if r.start <= cur => {
                    let e = r.end.min(end);
                    segs.push((cur, e, r.size_hmm));
                    i += 1;
                    if e == u32::MAX {
                        break;
                    }
                    cur = e + 1;
                }
                Some(r) if r.start <= end => {
                    segs.push((cur, r.start - 1, default_hmm));
                    cur = r.start;
                }
                _ => {
                    segs.push((cur, end, default_hmm));
                    break;
                }
            }
        }
        for (a, b, size) in segs {
            self.set(a, b, size, hidden);
        }
        if !hidden {
            self.runs.retain(|r| r.hidden || r.size_hmm != default_hmm);
        }
    }

    /// `true` if index `i` is hidden.
    pub fn is_hidden(&self, i: u32) -> bool {
        self.get(i).is_some_and(|r| r.hidden)
    }

    /// Shifts runs for `count` indices inserted before `at` (`limit` is the
    /// axis length); inserted indices inside a run take its size.
    pub fn insert(&mut self, at: u32, count: u32, limit: u32) {
        let max = limit - 1;
        let old = std::mem::take(&mut self.runs);
        for r in old {
            if r.end < at {
                self.set(r.start, r.end, r.size_hmm, r.hidden);
            } else if r.start >= at {
                let s = u64::from(r.start) + u64::from(count);
                if s <= u64::from(max) {
                    self.set(
                        s as u32,
                        r.end.saturating_add(count).min(max),
                        r.size_hmm,
                        r.hidden,
                    );
                }
            } else {
                self.set(
                    r.start,
                    r.end.saturating_add(count).min(max),
                    r.size_hmm,
                    r.hidden,
                );
            }
        }
    }

    /// Removes indices `at..at+count` and shifts the rest down.
    pub fn delete(&mut self, at: u32, count: u32) {
        let end = at.saturating_add(count);
        let old = std::mem::take(&mut self.runs);
        for r in old {
            let mut lo: Option<u32> = None;
            let mut hi: Option<u32> = None;
            if r.start < at {
                lo = Some(r.start);
                hi = Some(r.end.min(at - 1));
            }
            if r.end >= end {
                lo = Some(lo.unwrap_or(r.start.max(end) - count));
                hi = Some(r.end - count);
            }
            if let (Some(a), Some(b)) = (lo, hi) {
                self.set(a, b, r.size_hmm, r.hidden);
            }
        }
    }
}

/// Sorted, non-overlapping inclusive spans of rows (filter-hidden rows).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Spans {
    spans: Vec<(u32, u32)>,
}

impl Spans {
    /// From a sorted list of row numbers.
    pub fn from_sorted_rows(rows: impl IntoIterator<Item = u32>) -> Self {
        let mut spans: Vec<(u32, u32)> = Vec::new();
        for r in rows {
            match spans.last_mut() {
                Some(last) if last.1.checked_add(1) == Some(r) => last.1 = r,
                Some(last) if last.1 >= r => {}
                _ => spans.push((r, r)),
            }
        }
        Self { spans }
    }

    /// From spans in any order (merged).
    pub fn from_spans(mut v: Vec<(u32, u32)>) -> Self {
        v.retain(|(a, b)| a <= b);
        v.sort_unstable();
        let mut spans: Vec<(u32, u32)> = Vec::with_capacity(v.len());
        for (a, b) in v {
            match spans.last_mut() {
                Some(last) if last.1.saturating_add(1) >= a => last.1 = last.1.max(b),
                _ => spans.push((a, b)),
            }
        }
        Self { spans }
    }

    /// The spans.
    pub fn spans(&self) -> &[(u32, u32)] {
        &self.spans
    }

    /// `true` if `row` is in a span.
    pub fn contains(&self, row: u32) -> bool {
        let i = self.spans.partition_point(|s| s.1 < row);
        self.spans.get(i).is_some_and(|s| s.0 <= row)
    }

    /// `true` if there are no spans.
    pub fn is_empty(&self) -> bool {
        self.spans.is_empty()
    }
}

/// An AutoFilter column criterion (engine form of `tally_api::ColumnFilter`).
#[derive(Clone, Debug, PartialEq)]
pub struct ColumnCriteria {
    /// Formatted texts to keep (case-insensitive), `""` for empty cells.
    pub values: Option<Vec<String>>,
    /// A condition.
    pub condition: Option<(tally_model::FilterOpData, String)>,
}

/// A sheet's AutoFilter.
#[derive(Clone, Debug, PartialEq)]
pub struct AutoFilter {
    /// The area including its header row.
    pub range: tally_model::CellRangeLocal,
    /// Per absolute column.
    pub columns: BTreeMap<u32, ColumnCriteria>,
}

/// Sheet state that is not cells: merges, frozen panes, the AutoFilter,
/// conditional formats, validations, notes, links, protection and page
/// setup. Undo snapshots it whole ([`crate::undo::Change::SheetMeta`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SheetMeta {
    /// Merged areas (non-overlapping, each larger than one cell).
    pub merges: Vec<tally_model::CellRangeLocal>,
    /// Frozen `(rows, cols)`; `None` when not frozen.
    pub frozen: Option<(u32, u32)>,
    /// The AutoFilter.
    pub auto_filter: Option<AutoFilter>,
    /// Conditional formats in priority order.
    pub cond_formats: Vec<crate::condfmt::CondFormat>,
    /// Data validations.
    pub validations: Vec<crate::validation::Validation>,
    /// Cell notes by `(row, col)`.
    pub notes: BTreeMap<(u32, u32), crate::annotations::Note>,
    /// Cell hyperlinks by `(row, col)`.
    pub links: BTreeMap<(u32, u32), Arc<str>>,
    /// Sheet protection (`None`: not protected).
    pub protection: Option<tally_model::SheetProtectionData>,
    /// Page setup.
    pub page_setup: tally_model::PageSetupData,
    /// Manual page breaks before these rows.
    pub row_breaks: BTreeSet<u32>,
    /// Manual page breaks before these columns.
    pub col_breaks: BTreeSet<u32>,
    /// Next id for conditional formats, validations, charts, pictures,
    /// pivots and scenarios.
    pub next_id: u32,
    /// Charts anchored on the sheet (v0.4, W5-1).
    pub charts: Vec<crate::charts::Chart>,
    /// Pictures anchored on the sheet (v0.4, W5-1).
    pub images: Vec<crate::charts::Image>,
    /// DataPilot tables whose output is on the sheet (v0.5, W5-2).
    pub pivots: Vec<crate::pivot::PivotTable>,
    /// Row outline groups (v0.5, W5-2).
    pub row_groups: Vec<tally_model::OutlineGroupData>,
    /// Column outline groups (v0.5, W5-2).
    pub col_groups: Vec<tally_model::OutlineGroupData>,
    /// Scenarios of this base sheet (v0.5, W5-2).
    pub scenarios: Vec<crate::scenarios::Scenario>,
    /// Named database ranges on the sheet (v0.5, W5-2).
    pub database_ranges: Vec<tally_model::DatabaseRangeData>,
    /// The standard/advanced filter of the AutoFilter area (v0.5, W5-2).
    pub standard_filter: Option<tally_model::StandardFilterData>,
    /// The Data ▸ Subtotals definition (v0.5, W5-2).
    pub subtotals: Option<tally_model::SubtotalsData>,
    /// View toggles (`None`: defaults) (v0.5).
    pub view_options: Option<tally_model::ViewOptionsData>,
    /// Window ▸ Split position (v0.5).
    pub split: Option<tally_model::SplitData>,
}

impl SheetMeta {
    /// A fresh id for a conditional format, validation, chart, picture,
    /// pivot or scenario (one counter per sheet).
    pub fn alloc_id(&mut self) -> u32 {
        self.next_id = self.next_id.max(1);
        let id = self.next_id;
        self.next_id += 1;
        id
    }
}

/// Shifts a set of row numbers for inserted rows (values pushed past `limit` are dropped).
pub fn shift_set_insert(set: &mut BTreeSet<u32>, at: u32, count: u32, limit: u32) {
    let moved = set.split_off(&at);
    for r in moved {
        let n = u64::from(r) + u64::from(count);
        if n < u64::from(limit) {
            set.insert(n as u32);
        }
    }
}

/// Removes `at..at+count` from a set of row numbers and shifts the rest down.
pub fn shift_set_delete(set: &mut BTreeSet<u32>, at: u32, count: u32) {
    let mut tail = set.split_off(&at);
    let rest = tail.split_off(&at.saturating_add(count));
    for r in rest {
        set.insert(r - count);
    }
}

/// One column: cells, the rows holding formulas, and style runs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Column {
    /// Cell contents.
    pub cells: BTreeColumn,
    /// Rows whose content is a formula (for dependency ordering).
    pub formula_rows: BTreeSet<u32>,
    /// Style runs.
    pub styles: StyleRuns,
}

/// A sheet.
#[derive(Clone, Debug, PartialEq)]
pub struct Sheet {
    /// Stable id.
    pub id: SheetId,
    /// Tab name.
    pub name: Arc<str>,
    /// Columns, grown lazily.
    pub columns: Vec<Column>,
    /// Column widths.
    pub col_dims: DimRuns,
    /// Row heights.
    pub row_dims: DimRuns,
    /// Hidden sheet.
    pub hidden: bool,
    /// Tab colour `0xRRGGBBAA`.
    pub tab_color: Option<u32>,
    /// Merges, frozen panes and the AutoFilter.
    pub meta: SheetMeta,
    /// Rows hidden by the AutoFilter (derived from `meta.auto_filter` and
    /// the cell values; recomputed by `Workbook::reapply_filter`).
    pub filtered_rows: Spans,
}

impl Sheet {
    /// An empty sheet.
    pub fn new(id: SheetId, name: &str) -> Self {
        Self {
            id,
            name: name.into(),
            columns: Vec::new(),
            col_dims: DimRuns::default(),
            row_dims: DimRuns::default(),
            hidden: false,
            tab_color: None,
            meta: SheetMeta::default(),
            filtered_rows: Spans::default(),
        }
    }

    /// The merge whose area contains (`row`, `col`).
    pub fn merge_at(&self, row: u32, col: u32) -> Option<tally_model::CellRangeLocal> {
        self.meta
            .merges
            .iter()
            .find(|m| m.r0 <= row && row <= m.r1 && m.c0 <= col && col <= m.c1)
            .copied()
    }

    /// `true` if (`row`, `col`) is a covered (non-anchor) cell of a merge.
    pub fn is_merge_covered(&self, row: u32, col: u32) -> bool {
        self.merge_at(row, col)
            .is_some_and(|m| (m.r0, m.c0) != (row, col))
    }

    /// `true` if `row` is hidden by the user or by the AutoFilter.
    pub fn row_hidden(&self, row: u32) -> bool {
        self.row_dims.is_hidden(row) || self.filtered_rows.contains(row)
    }

    /// The column, if it exists.
    pub fn column(&self, col: u32) -> Option<&Column> {
        self.columns.get(col as usize)
    }

    /// The column, created on demand (`None` past the grid).
    pub fn column_mut(&mut self, col: u32) -> Option<&mut Column> {
        if col >= MAX_COLS {
            return None;
        }
        let i = col as usize;
        if self.columns.len() <= i {
            self.columns.resize_with(i + 1, Column::default);
        }
        self.columns.get_mut(i)
    }

    /// The content of a cell.
    pub fn get(&self, row: u32, col: u32) -> Option<&CellContent> {
        self.column(col).and_then(|c| c.cells.get(row))
    }

    /// Stores content, returning the previous content.
    pub fn set(&mut self, row: u32, col: u32, content: CellContent) -> Option<CellContent> {
        if row >= MAX_ROWS {
            return None;
        }
        let column = self.column_mut(col)?;
        if content.formula_id().is_some() {
            column.formula_rows.insert(row);
        } else {
            column.formula_rows.remove(&row);
        }
        column.cells.set(row, content)
    }

    /// Removes content, returning it.
    pub fn remove(&mut self, row: u32, col: u32) -> Option<CellContent> {
        let column = self.columns.get_mut(col as usize)?;
        column.formula_rows.remove(&row);
        column.cells.remove(row)
    }

    /// The style of a cell.
    pub fn style(&self, row: u32, col: u32) -> StyleId {
        self.column(col)
            .map_or(StyleId::DEFAULT, |c| c.styles.get(row))
    }

    /// Bounding box of non-empty cells.
    pub fn used_range(&self) -> Option<CellRange> {
        let mut bounds: Option<(u32, u32, u32, u32)> = None;
        for (ci, col) in self.columns.iter().enumerate() {
            let (Some(first), Some(last)) =
                (col.cells.first_non_empty(), col.cells.last_non_empty())
            else {
                continue;
            };
            let c = ci as u32;
            bounds = Some(match bounds {
                None => (first, c, last, c),
                Some((r0, c0, r1, c1)) => (r0.min(first), c0.min(c), r1.max(last), c1.max(c)),
            });
        }
        bounds.map(|(r0, c0, r1, c1)| CellRange::new(self.id, r0, c0, r1, c1))
    }

    /// Number of non-empty cells.
    pub fn cell_count(&self) -> usize {
        self.columns.iter().map(|c| c.cells.len()).sum()
    }

    /// `true` if the cell has content.
    pub fn is_non_empty(&self, row: u32, col: u32) -> bool {
        self.get(row, col).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn btree_column_million_rows() {
        let mut c = BTreeColumn::default();
        for r in 0..1_000_000u32 {
            c.set(r, CellContent::Value(Value::Number(f64::from(r))));
        }
        assert_eq!(c.len(), 1_000_000);
        assert_eq!(
            c.get(999_999),
            Some(&CellContent::Value(Value::Number(999_999.0)))
        );
        let sum: f64 = c
            .iter_range(0, MAX_ROWS - 1)
            .map(|(_, v)| match v {
                CellContent::Value(Value::Number(n)) => *n,
                _ => 0.0,
            })
            .sum();
        assert_eq!(sum, 499_999_500_000.0);
        assert_eq!(c.iter_range(10, 19).count(), 10);
        assert_eq!(c.last_non_empty(), Some(999_999));
        assert_eq!(c.next_non_empty(999_999, Dir::Down), None);
        assert_eq!(c.next_non_empty(5, Dir::Up), Some(4));
        assert_eq!(c.remove(3), Some(CellContent::Value(Value::Number(3.0))));
        assert_eq!(c.next_non_empty(2, Dir::Down), Some(4));
        assert_eq!(c.next_non_empty(0, Dir::Up), None);
    }

    #[test]
    fn column_insert_delete_rows() {
        let mut c = BTreeColumn::default();
        for r in [0u32, 5, 10] {
            c.set(r, CellContent::Value(Value::Number(f64::from(r))));
        }
        c.insert_rows(5, 2);
        assert!(c.get(5).is_none());
        assert!(c.get(7).is_some());
        assert!(c.get(12).is_some());
        let removed = c.delete_rows(6, 3);
        assert_eq!(removed.len(), 1);
        assert!(c.get(9).is_some(), "row 12 moved up by 3");
        assert_eq!(c.len(), 2);
    }

    #[test]
    fn style_runs_set_split_merge() {
        let mut s = StyleRuns::default();
        s.set(0, MAX_ROWS - 1, StyleId(1));
        assert_eq!(s.run_count(), 1);
        assert_eq!(s.get(500_000), StyleId(1));
        s.set(10, 19, StyleId(2));
        assert_eq!(s.run_count(), 3);
        assert_eq!(s.get(9), StyleId(1));
        assert_eq!(s.get(10), StyleId(2));
        assert_eq!(s.get(20), StyleId(1));
        s.set(10, 19, StyleId(1));
        assert_eq!(s.run_count(), 1, "merged back");
        s.set(0, MAX_ROWS - 1, StyleId::DEFAULT);
        assert!(s.is_empty());
        s.set(5, 5, StyleId(3));
        s.set(6, 6, StyleId(3));
        assert_eq!(s.run_count(), 1);
        assert_eq!(s.runs_in(0, 100), vec![(5, 6, StyleId(3))]);
    }

    #[test]
    fn style_runs_map_covers_gaps() {
        let mut s = StyleRuns::default();
        s.set(3, 4, StyleId(2));
        s.map(0, 9, |old| StyleId(old.0 + 10));
        assert_eq!(s.get(0), StyleId(10));
        assert_eq!(s.get(3), StyleId(12));
        assert_eq!(s.get(9), StyleId(10));
        assert_eq!(s.get(10), StyleId::DEFAULT);
        assert_eq!(s.run_count(), 3);
    }

    #[test]
    fn style_runs_whole_column_is_one_entry() {
        let mut s = StyleRuns::default();
        s.map(0, MAX_ROWS - 1, |_| StyleId(7));
        assert_eq!(s.run_count(), 1);
    }

    #[test]
    fn dim_runs() {
        let mut d = DimRuns::default();
        d.set(0, 9, 100, false);
        d.set(3, 4, 200, false);
        assert_eq!(d.runs().len(), 3);
        assert_eq!(d.get(3).map(|r| r.size_hmm), Some(200));
        assert_eq!(d.get(9).map(|r| r.size_hmm), Some(100));
        assert!(d.get(10).is_none());
        d.set(3, 4, 100, false);
        assert_eq!(d.runs().len(), 1);
    }

    #[test]
    fn sheet_used_range() {
        let mut s = Sheet::new(0, "Sheet1");
        assert_eq!(s.used_range(), None);
        s.set(4, 2, CellContent::Value(Value::Number(1.0)));
        s.set(1, 5, CellContent::Value(Value::Number(1.0)));
        assert_eq!(s.used_range(), Some(CellRange::new(0, 1, 2, 4, 5)));
        assert_eq!(s.cell_count(), 2);
    }
}
