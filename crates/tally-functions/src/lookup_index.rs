//! [`LookupIndex`]: a prebuilt index of one lookup line (a column or a
//! row of a range) for exact-match VLOOKUP, HLOOKUP and MATCH type 0.
//!
//! The engine builds one per line on demand during a recalc (see
//! [`crate::EvalContext::lookup_index`]) and shares it between every
//! lookup into that line; the functions decide per call whether the
//! index can answer (see `categories/spreadsheet/lookup.rs`) and fall back
//! to a scan of [`LookupIndex::items`] otherwise.
//!
//! The index answers exactly what the linear scan answers:
//!
//! - numbers (and booleans, as 1/0) are found by [`approx_equal`] among
//!   the numeric cells, the first (lowest offset) match winning;
//! - text is found by equality after lower-casing (unless the index was
//!   built case-sensitive), the first match winning;
//! - empty cells, errors and nested arrays are in `items` but never match.

use std::borrow::Cow;
use std::collections::HashMap;
use std::mem::size_of;
use std::sync::Arc;

use tally_types::{approx_equal, Value};

/// A lookup line: its non-empty cells plus hash and sorted indexes.
#[derive(Debug)]
pub struct LookupIndex {
    /// The non-empty cells of the line as `(offset, value)`, in order.
    items: Vec<(u32, Value)>,
    /// Distinct numeric values (booleans as 1/0) sorted ascending, each
    /// with the lowest offset holding exactly that value. No NaN.
    nums: Vec<(f64, u32)>,
    /// Normalised text (lower-cased unless `case_sensitive`) to the lowest
    /// offset holding it. A key that is already normalised shares the
    /// cell's string; only lower-cased copies are new allocations.
    texts: HashMap<Arc<str>, u32>,
    /// Bytes of the lower-cased key copies.
    key_bytes: usize,
    case_sensitive: bool,
}

/// Bytes one hash entry costs besides its key text (the `Arc` pointer and
/// offset, the control byte, the `Arc` counters of a copied key).
const TEXT_ENTRY_BYTES: usize = size_of::<(Arc<str>, u32)>() + 1 + 16;

/// Wider than the relative tolerance of [`approx_equal`] (2^-48 ≈ 3.6e-15),
/// so the candidate window always contains every approximate match.
const WINDOW: f64 = 1e-14;

impl LookupIndex {
    /// Lines with fewer cells than this are not worth indexing.
    pub const MIN_CELLS: u64 = 64;

    /// An upper bound of the bytes an index of `cells` cells takes, not
    /// counting the lower-cased copies of text keys (see
    /// [`LookupIndex::heap_bytes`]): what a cache checks before building.
    pub const fn max_bytes_without_keys(cells: usize) -> usize {
        cells.saturating_mul(
            size_of::<(u32, Value)>() + size_of::<(f64, u32)>() + 2 * TEXT_ENTRY_BYTES,
        )
    }

    /// Builds the index from a line's non-empty cells, `(offset, value)`.
    pub fn build(mut items: Vec<(u32, Value)>, case_sensitive: bool) -> Self {
        items.shrink_to_fit();
        let mut nums: Vec<(f64, u32)> = Vec::new();
        let mut texts: HashMap<Arc<str>, u32> = HashMap::new();
        let mut key_bytes = 0usize;
        for (off, v) in &items {
            match v {
                Value::Number(n) if !n.is_nan() => nums.push((*n, *off)),
                Value::Bool(b) => nums.push((if *b { 1.0 } else { 0.0 }, *off)),
                Value::Text(s) => match normalise(s, case_sensitive) {
                    Cow::Borrowed(_) => {
                        texts
                            .entry(s.clone())
                            .and_modify(|o| *o = (*o).min(*off))
                            .or_insert(*off);
                    }
                    Cow::Owned(k) => {
                        if let Some(o) = texts.get_mut(k.as_str()) {
                            *o = (*o).min(*off);
                        } else {
                            key_bytes += k.len();
                            texts.insert(Arc::from(k), *off);
                        }
                    }
                },
                _ => {}
            }
        }
        // No NaN, so `partial_cmp` is total; -0.0 and 0.0 compare equal and
        // merge. Exact duplicates keep the lowest offset.
        nums.sort_by(|a, b| {
            a.0.partial_cmp(&b.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.1.cmp(&b.1))
        });
        nums.dedup_by(|later, kept| later.0 == kept.0);
        nums.shrink_to_fit();
        texts.shrink_to_fit();
        Self {
            items,
            nums,
            texts,
            key_bytes,
            case_sensitive,
        }
    }

    /// The line's non-empty cells, `(offset, value)` in order.
    pub fn items(&self) -> &[(u32, Value)] {
        &self.items
    }

    /// Whether text was indexed case-sensitively.
    pub fn case_sensitive(&self) -> bool {
        self.case_sensitive
    }

    /// Approximate heap bytes owned by the index (the cells' own strings
    /// are shared with the sheet and not counted), for memory caps.
    pub fn heap_bytes(&self) -> usize {
        self.items.capacity() * size_of::<(u32, Value)>()
            + self.nums.capacity() * size_of::<(f64, u32)>()
            + self.texts.capacity() * TEXT_ENTRY_BYTES
            + self.key_bytes
    }

    /// The first offset whose number or boolean is [`approx_equal`] to `k`.
    pub fn find_number(&self, k: f64) -> Option<u32> {
        if k.is_nan() {
            return None;
        }
        let (lo, hi) = if k.is_finite() {
            let d = k.abs() * WINDOW;
            (k - d, k + d)
        } else {
            (k, k)
        };
        let start = self.nums.partition_point(|e| e.0 < lo);
        self.nums[start..]
            .iter()
            .take_while(|e| e.0 <= hi)
            .filter(|e| approx_equal(e.0, k))
            .map(|e| e.1)
            .min()
    }

    /// The first offset whose text equals `s` (case-insensitively unless
    /// the index is case-sensitive).
    pub fn find_text(&self, s: &str) -> Option<u32> {
        self.texts
            .get(normalise(s, self.case_sensitive).as_ref())
            .copied()
    }
}

/// The hash key of `s`: itself when case-sensitive or already lower-case.
fn normalise(s: &str, case_sensitive: bool) -> Cow<'_, str> {
    if case_sensitive {
        return Cow::Borrowed(s);
    }
    let lower = s.to_lowercase();
    if lower == s {
        Cow::Borrowed(s)
    } else {
        Cow::Owned(lower)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_and_text() {
        let items = vec![
            (0, Value::text("B")),
            (1, Value::Number(-0.0)),
            (2, Value::Number(3.0)),
            (3, Value::Bool(true)),
            (4, Value::Number(1.0)),
            (5, Value::Number(0.0)),
            (6, Value::text("b")),
            (7, Value::Number(0.1 + 0.2)),
            (8, Value::Number(f64::NAN)),
            (9, Value::Error(tally_types::ErrorKind::NA)),
        ];
        let ix = LookupIndex::build(items.clone(), false);
        assert_eq!(ix.find_number(0.0), Some(1));
        assert_eq!(ix.find_number(1.0), Some(3));
        assert_eq!(ix.find_number(0.3), Some(7));
        assert_eq!(ix.find_number(2.0), None);
        assert_eq!(ix.find_number(f64::NAN), None);
        assert_eq!(ix.find_text("b"), Some(0));
        assert_eq!(ix.find_text("c"), None);
        let cs = LookupIndex::build(items, true);
        assert_eq!(cs.find_text("b"), Some(6));
        assert_eq!(cs.find_text("B"), Some(0));
    }
}
