//! AutoInput (wave 5, W5-2): column completion while typing, as AOO's
//! `ScColumn::GetDataEntries` (`sc/source/core/data/column3.cxx:1571-1643`):
//! the *text* cells of the whole column (formula results never count:
//! only `CELLTYPE_STRING`/`CELLTYPE_EDIT`), searched alternately upwards
//! and downwards from the edited cell so near cells come first (AOO
//! dropped the limit to the contiguous block, `column3.cxx:1579-1582`;
//! correctness review 4, M44). The completion offered is the first match
//! in alphabetical order (case-insensitive); the same list feeds the
//! Ctrl+D selection list.
//!
//! Costs (security review 4, MEDIUM-T8): at most [`MAX_SCANNED`] text
//! cells are read, at most [`MAX_AUTOCOMPLETE_ITEMS`] items are kept (the
//! alphabetically first ones, in a bounded map), each item is cut to
//! [`MAX_ITEM_CHARS`] characters and the list to [`MAX_ITEMS_BYTES`]
//! bytes; the completion itself is the full text.

use std::collections::BTreeMap;

use tally_api::{ApiError, AutocompleteDto};
use tally_types::{SheetId, Value, MAX_COLS, MAX_ROWS};

use crate::storage::{CellContent, ColumnStore, Dir};
use crate::tools::MAX_AUTOCOMPLETE_ITEMS;
use crate::workbook::Workbook;

/// Most text cells read per request.
pub const MAX_SCANNED: u64 = 100_000;
/// Longest item returned in the list (characters).
pub const MAX_ITEM_CHARS: usize = 256;
/// Most bytes of item text per response.
pub const MAX_ITEMS_BYTES: usize = 256 * 1024;

/// The completions for `prefix` typed at `(r, c)`.
pub fn autocomplete(
    wb: &Workbook,
    sheet: SheetId,
    r: u32,
    c: u32,
    prefix: &str,
) -> Result<AutocompleteDto, ApiError> {
    let s = wb
        .sheet(sheet)
        .ok_or_else(|| ApiError::not_found(format!("no sheet {sheet}")))?;
    if r >= MAX_ROWS || c >= MAX_COLS {
        return Err(ApiError::invalid_argument("cell outside the grid"));
    }
    let Some(col) = s.column(c).map(|c| &c.cells) else {
        return Ok(AutocompleteDto {
            completion: None,
            items: Vec::new(),
        });
    };
    let needle = prefix.to_lowercase();
    // lowercase → text, the alphabetically first MAX_AUTOCOMPLETE_ITEMS.
    let mut best: BTreeMap<String, String> = BTreeMap::new();
    let mut scanned: u64 = 0;
    let mut up = col.next_non_empty(r, Dir::Up);
    let mut down = col.next_non_empty(r, Dir::Down);
    let mut take = |row: u32, best: &mut BTreeMap<String, String>| -> bool {
        let Some(CellContent::Value(Value::Text(t))) = col.get(row) else {
            return true;
        };
        scanned += 1;
        if t.is_empty() {
            return scanned < MAX_SCANNED;
        }
        let lower = t.to_lowercase();
        if (needle.is_empty() || lower.starts_with(&needle)) && lower != needle {
            let full = best.len() >= MAX_AUTOCOMPLETE_ITEMS;
            if (!full || best.last_key_value().is_some_and(|(k, _)| lower < *k))
                && !best.contains_key(&lower)
            {
                best.insert(lower, t.to_string());
                if best.len() > MAX_AUTOCOMPLETE_ITEMS {
                    best.pop_last();
                }
            }
        }
        scanned < MAX_SCANNED
    };
    while up.is_some() || down.is_some() {
        if let Some(u) = up {
            if !take(u, &mut best) {
                break;
            }
            up = col.next_non_empty(u, Dir::Up);
        }
        if let Some(d) = down {
            if !take(d, &mut best) {
                break;
            }
            down = col.next_non_empty(d, Dir::Down);
        }
    }
    let completion = if prefix.is_empty() {
        None
    } else {
        best.values().next().map(|first| {
            // The typed prefix is kept as typed; the rest comes from the match.
            let rest: String = first.chars().skip(prefix.chars().count()).collect();
            format!("{prefix}{rest}")
        })
    };
    let mut bytes = 0usize;
    let mut items = Vec::with_capacity(best.len());
    for t in best.into_values() {
        let t: String = if t.chars().count() > MAX_ITEM_CHARS {
            t.chars().take(MAX_ITEM_CHARS).collect()
        } else {
            t
        };
        bytes += t.len();
        if bytes > MAX_ITEMS_BYTES {
            break;
        }
        items.push(t);
    }
    Ok(AutocompleteDto { completion, items })
}
