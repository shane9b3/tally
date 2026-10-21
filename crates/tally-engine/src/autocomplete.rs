//! AutoInput (wave 5, W5-2): column completion while typing. AOO's
//! `ScColumn::GetDataEntries` collects the text cells of the column
//! inside the data block around the edited cell; the completion offered
//! is the first match in alphabetical order (case-insensitive), and the
//! same list feeds the Ctrl+D selection list.

use tally_api::{ApiError, AutocompleteDto};
use tally_types::{CellAddr, SheetId, Value, MAX_COLS, MAX_ROWS};

use crate::storage::ColumnStore;
use crate::tools::{data_block_rows, MAX_AUTOCOMPLETE_ITEMS};
use crate::workbook::Workbook;

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
    let Some(col) = s.column(c) else {
        return Ok(AutocompleteDto {
            completion: None,
            items: Vec::new(),
        });
    };
    // The block: the run around the cell, extended over the cell itself
    // when it is the empty cell being typed into.
    let (top, bottom) = data_block_rows(wb, sheet, r, c);
    let (top, bottom) = if s.is_non_empty(r, c) {
        (top, bottom)
    } else {
        let above = if r > 0 && s.is_non_empty(r - 1, c) {
            data_block_rows(wb, sheet, r - 1, c).0
        } else {
            top
        };
        let below = if r + 1 < MAX_ROWS && s.is_non_empty(r + 1, c) {
            data_block_rows(wb, sheet, r + 1, c).1
        } else {
            bottom
        };
        (above, below)
    };
    let needle = prefix.to_lowercase();
    let mut items: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut scanned: u64 = 0;
    for (row, _) in col.cells.iter_range(top, bottom) {
        if row == r {
            continue;
        }
        scanned += 1;
        if scanned > 1_000_000 {
            break;
        }
        let v = wb.value(CellAddr::new(sheet, row, c));
        let Value::Text(t) = v else {
            continue;
        };
        if t.is_empty() {
            continue;
        }
        let lower = t.to_lowercase();
        if (needle.is_empty() || lower.starts_with(&needle))
            && lower != needle
            && seen.insert(lower)
        {
            items.push(t.to_string());
        }
    }
    items.sort_by_key(|a| a.to_lowercase());
    items.truncate(MAX_AUTOCOMPLETE_ITEMS);
    let completion = if prefix.is_empty() {
        None
    } else {
        items.first().map(|first| {
            // The typed prefix is kept as typed; the rest comes from the match.
            let rest: String = first.chars().skip(prefix.chars().count()).collect();
            format!("{prefix}{rest}")
        })
    };
    Ok(AutocompleteDto { completion, items })
}
