//! The per-recalc lookup cache behind [`tally_functions::EvalContext::lookup_index`].
//!
//! Exact-match VLOOKUP, HLOOKUP and MATCH type 0 into the same column (or
//! row) share one [`LookupIndex`] for the rest of the recalc, instead of
//! scanning the line on every call.
//!
//! # Lifetime and invalidation
//!
//! - The cache exists only while [`crate::recalc::recalc_with`] runs
//!   (`Workbook::lookup_cache` is `Some` then, `None` otherwise), so an edit,
//!   a row insertion or any other change between recalcs can never meet an
//!   old index; contexts outside a recalc (previews, goal seek, condition
//!   checks) always scan.
//! - An index is built only when every formula cell in the line is clean
//!   after the line's dirty formulas were evaluated. During a recalc a clean
//!   formula is never evaluated again, so the line's values are then final
//!   for the rest of the pass.
//! - The one exception is iterative calculation, which re-dirties the
//!   members of a cyclic component every round: the cache is cleared at the
//!   start and end of every round, and contexts iterating a component never
//!   use it.
//! - A line is indexed on its second request, so a range looked up once is
//!   scanned as before; lines with fewer than [`LookupIndex::MIN_CELLS`]
//!   used cells are never indexed. The indexes of one pass own at most
//!   [`MAX_BYTES`] (checked against the worst case before a build and
//!   against the real size after it) and cover at most [`MAX_LINES`]
//!   lines; a line that does not fit, or whose formulas are still not clean
//!   after [`MAX_ATTEMPTS`] tries, scans for the rest of the pass and is
//!   never rebuilt. Overlapping ranges (`$A$1:$A$1000`, `A:A`) are separate
//!   lines with separate indexes, within the same budget.
//!
//! The engine evaluates on one thread per document (the workbook is behind
//! `&mut`), so the cache needs no locking; the indexes are `Arc`s of
//! immutable data and are `Send + Sync`.

use std::collections::HashMap;
use std::sync::Arc;

use tally_functions::LookupIndex;
use tally_types::CellRange;

/// Most heap bytes the indexes of one recalc may own together (see
/// [`LookupIndex::heap_bytes`]). The cells' strings are shared with the
/// sheet; lower-cased key copies count. Lines past the budget scan.
pub const MAX_BYTES: usize = 256 << 20;

/// Most distinct lines the cache tracks in one recalc.
pub const MAX_LINES: usize = 65_536;

/// Build attempts for a line whose formulas are not all clean yet (a
/// cycle being followed or a deferred precedent) before it scans for
/// the rest of the pass.
pub const MAX_ATTEMPTS: u8 = 3;

#[derive(Clone, Debug)]
enum Slot {
    /// Requested before; `failed` build attempts found unclean formulas.
    Seen {
        failed: u8,
    },
    Built(Arc<LookupIndex>),
    /// Never indexed in this pass (over budget or never clean): scan.
    Skip,
}

/// Lookup indexes by line, for one recalc pass.
#[derive(Clone, Debug)]
pub struct LookupCache {
    lines: HashMap<CellRange, Slot>,
    bytes: usize,
    max_bytes: usize,
}

impl Default for LookupCache {
    fn default() -> Self {
        #[cfg(test)]
        let max_bytes = tests::MAX_BYTES_OVERRIDE.with(std::cell::Cell::get);
        #[cfg(not(test))]
        let max_bytes = MAX_BYTES;
        Self {
            lines: HashMap::new(),
            bytes: 0,
            max_bytes,
        }
    }
}

/// What [`LookupCache::request`] says to do.
pub(crate) enum Request {
    /// Use this index.
    Hit(Arc<LookupIndex>),
    /// Build an index, then report back with [`LookupCache::insert`] or
    /// [`LookupCache::not_clean`].
    Build,
    /// Scan this time.
    Scan,
}

impl LookupCache {
    /// Looks `line` (with `cells` used cells) up, recording the first
    /// request. A build is asked for only when the worst case fits; a line
    /// that cannot fit is skipped for the rest of the pass.
    pub(crate) fn request(&mut self, line: &CellRange, cells: usize) -> Request {
        match self.lines.get(line) {
            Some(Slot::Built(ix)) => Request::Hit(ix.clone()),
            Some(Slot::Skip) => Request::Scan,
            Some(Slot::Seen { .. }) => {
                let worst = LookupIndex::max_bytes_without_keys(cells);
                if self.bytes.saturating_add(worst) > self.max_bytes {
                    self.lines.insert(*line, Slot::Skip);
                    Request::Scan
                } else {
                    Request::Build
                }
            }
            None => {
                if self.lines.len() < MAX_LINES {
                    self.lines.insert(*line, Slot::Seen { failed: 0 });
                }
                Request::Scan
            }
        }
    }

    /// A build attempt found formulas that are not clean yet: retry on a
    /// later request, at most [`MAX_ATTEMPTS`] times in all.
    pub(crate) fn not_clean(&mut self, line: &CellRange) {
        if let Some(slot) = self.lines.get_mut(line) {
            *slot = match slot {
                Slot::Seen { failed } if *failed + 1 < MAX_ATTEMPTS => Slot::Seen {
                    failed: *failed + 1,
                },
                _ => Slot::Skip,
            };
        }
    }

    /// Stores a built index if it fits the byte budget (its key copies can
    /// exceed the estimate); otherwise the line is skipped from now on.
    pub(crate) fn insert(&mut self, line: CellRange, ix: Arc<LookupIndex>) {
        #[cfg(test)]
        tests::BUILDS.with(|b| b.set(b.get() + 1));
        let n = ix.heap_bytes();
        if self.bytes.saturating_add(n) <= self.max_bytes {
            self.bytes += n;
            self.lines.insert(line, Slot::Built(ix));
        } else {
            self.lines.insert(line, Slot::Skip);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use tally_types::{CellAddr, ErrorKind, IterSettings, Value};

    use crate::recalc::recalc;
    use crate::Workbook;

    thread_local! {
        pub(super) static BUILDS: Cell<u32> = const { Cell::new(0) };
        pub(super) static MAX_BYTES_OVERRIDE: Cell<usize> = const { Cell::new(super::MAX_BYTES) };
    }

    fn builds() -> u32 {
        BUILDS.with(Cell::get)
    }

    fn a(s: &str) -> CellAddr {
        CellAddr::from_a1(1, s).unwrap_or_else(|| panic!("bad cell {s}"))
    }

    /// A1:A200 = 1..200, B1:B200 = "v1".."v200".
    fn table() -> Workbook {
        let mut wb = Workbook::new();
        for r in 1..=200 {
            wb.set_input(a(&format!("A{r}")), &r.to_string());
            wb.set_input(a(&format!("B{r}")), &format!("v{r}"));
        }
        wb
    }

    #[test]
    fn one_index_per_line_shared_by_all_lookups() {
        let mut wb = table();
        for r in 1..=50 {
            wb.set_input(
                a(&format!("D{r}")),
                &format!("=VLOOKUP({};$A$1:$B$200;2;0)", r * 3),
            );
            wb.set_input(
                a(&format!("E{r}")),
                &format!("=MATCH({};$A$1:$A$200;0)", r * 3),
            );
            // A line under the size threshold is never indexed.
            wb.set_input(a(&format!("F{r}")), &format!("=MATCH({r};$A$1:$A$63;0)"));
        }
        let before = builds();
        recalc(&mut wb);
        // A1:A200 is the search line of both functions: one index.
        assert_eq!(builds() - before, 1);
        assert!(wb.lookup_cache.is_none());
        for r in 1..=50u32 {
            let k = r * 3;
            let want = if k <= 200 {
                Value::text(format!("v{k}"))
            } else {
                Value::Error(ErrorKind::NA)
            };
            assert_eq!(wb.value(a(&format!("D{r}"))), want);
        }
        assert_eq!(wb.value(a("E7")), Value::Number(21.0));
        assert_eq!(wb.value(a("F50")), Value::Number(50.0));
        // An edit and an incremental recalc build a fresh index.
        wb.set_input(a("A1"), "21");
        recalc(&mut wb);
        assert_eq!(wb.value(a("E7")), Value::Number(1.0));
        assert_eq!(wb.value(a("D7")), Value::text("v1"));
    }

    #[test]
    fn lines_of_dirty_formulas_are_evaluated_first() {
        let mut wb = table();
        // The looked-up column is made of formulas over H.
        for r in 1..=200 {
            wb.set_input(a(&format!("H{r}")), &r.to_string());
            wb.set_input(a(&format!("A{r}")), &format!("=H{r}*2"));
        }
        for r in 1..=20 {
            wb.set_input(a(&format!("E{r}")), &format!("=MATCH({};A1:A200;0)", r * 4));
        }
        recalc(&mut wb);
        assert_eq!(wb.value(a("E1")), Value::Number(2.0));
        wb.set_input(a("H1"), "2");
        recalc(&mut wb);
        assert_eq!(wb.value(a("E1")), Value::Number(1.0));
        assert_eq!(wb.value(a("E2")), Value::Number(4.0));
    }

    #[test]
    fn iterated_cycles_inside_the_line() {
        let mut wb = table();
        wb.settings.iterations = Some(IterSettings::default());
        wb.set_input(a("A1"), "=C1+1");
        wb.set_input(a("C1"), "=MIN(A1;5)+MATCH(A1;A1:A200;0)-1");
        for r in 1..=5 {
            wb.set_input(a(&format!("E{r}")), "=MATCH(6;A1:A200;0)");
        }
        recalc(&mut wb);
        assert_eq!(wb.value(a("A1")), Value::Number(6.0));
        assert_eq!(wb.value(a("E5")), Value::Number(1.0));
    }

    #[test]
    fn unclean_lines_back_off_then_skip() {
        let mut c = super::LookupCache::default();
        let line = tally_types::CellRange::new(1, 0, 0, 999, 0);
        assert!(matches!(c.request(&line, 1000), super::Request::Scan));
        for _ in 0..super::MAX_ATTEMPTS {
            assert!(matches!(c.request(&line, 1000), super::Request::Build));
            c.not_clean(&line);
        }
        for _ in 0..10 {
            assert!(matches!(c.request(&line, 1000), super::Request::Scan));
        }
    }

    /// Runs `f` with the byte budget set to `max`.
    fn with_budget<T>(max: usize, f: impl FnOnce() -> T) -> T {
        MAX_BYTES_OVERRIDE.with(|m| m.set(max));
        let out = f();
        MAX_BYTES_OVERRIDE.with(|m| m.set(super::MAX_BYTES));
        out
    }

    /// 200 long upper-case keys (their lower-cased copies push the real
    /// size past the worst-case estimate) and 100 lookups into them.
    fn long_keys() -> Workbook {
        let mut wb = Workbook::new();
        for r in 1..=200 {
            wb.set_input(a(&format!("A{r}")), &format!("{}{r}", "X".repeat(400)));
        }
        for r in 1..=100 {
            wb.set_input(
                a(&format!("E{r}")),
                &format!("=MATCH(\"{}{r}\";A1:A200;0)", "x".repeat(400)),
            );
        }
        wb
    }

    #[test]
    fn a_line_over_budget_is_built_at_most_once() {
        let mut wb = long_keys();
        let estimate = tally_functions::LookupIndex::max_bytes_without_keys(200);
        let before = builds();
        with_budget(estimate + 1000, || recalc(&mut wb));
        assert_eq!(builds() - before, 1);
        assert_eq!(wb.value(a("E77")), Value::Number(77.0));

        // A budget below the worst case never builds.
        wb.dirty_all();
        let before = builds();
        with_budget(estimate - 1, || recalc(&mut wb));
        assert_eq!(builds() - before, 0);
        assert_eq!(wb.value(a("E100")), Value::Number(100.0));
    }
}
