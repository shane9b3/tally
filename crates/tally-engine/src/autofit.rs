//! Row auto-height (`Op::AutoFitRows`) without a text shaper: text is
//! measured with an approximate advance-width table for Inter (the UI
//! font), wrapped greedily at word boundaries for wrapped cells, and laid
//! out with a 1.3 line height. Heights are capped at 409 pt like Excel.

use tally_types::{CellAddr, Value};

use crate::storage::{ColumnStore, Sheet, StyleId};
use crate::styles::DEFAULT_FONT_SIZE_PT;
use crate::units::{hmm_to_px, DEFAULT_COL_WIDTH_PX, DEFAULT_ROW_HEIGHT_PX};
use crate::workbook::Workbook;

/// Line height relative to the font size.
pub const LINE_HEIGHT: f32 = 1.3;
/// Excel's maximum row height (409 pt) in pixels.
pub const MAX_ROW_HEIGHT_PX: f32 = 409.0 * 96.0 / 72.0;
/// Horizontal padding inside a cell (both sides together), px.
const H_PADDING_PX: f32 = 8.0;

/// Vertical padding so an 11 pt line gives the default 24 px row.
fn v_padding_px() -> f32 {
    (DEFAULT_ROW_HEIGHT_PX - pt_to_px(DEFAULT_FONT_SIZE_PT) * LINE_HEIGHT).max(0.0)
}

fn pt_to_px(pt: f32) -> f32 {
    pt * 96.0 / 72.0
}

/// Approximate advance width of `c` in em for Inter Regular.
pub fn char_em(c: char) -> f32 {
    match c {
        ' ' => 0.28,
        'i' | 'l' | 'j' | '|' | '!' | '.' | ',' | ':' | ';' | '\'' | 'I' => 0.27,
        'f' | 't' | 'r' | '(' | ')' | '[' | ']' | '{' | '}' | '/' | '\\' | '"' | '-' => 0.36,
        'm' | 'w' => 0.86,
        'M' | 'W' => 0.92,
        '@' | '%' | '&' => 0.78,
        '0'..='9' => 0.62,
        'a'..='z' => 0.55,
        'A'..='Z' => 0.68,
        '\t' => 1.12,
        c if (c as u32) >= 0x2E80 => 1.0, // CJK and other wide scripts
        _ => 0.6,
    }
}

/// Width of `text` in px at `size_px` (`bold` is ~5 % wider).
pub fn text_width_px(text: &str, size_px: f32, bold: bool) -> f32 {
    let em: f32 = text.chars().map(char_em).sum();
    em * size_px * if bold { 1.05 } else { 1.0 }
}

/// Lines `text` needs at `width_px` (greedy word wrap; explicit line
/// breaks always count).
pub fn line_count(text: &str, width_px: f32, size_px: f32, bold: bool, wrap: bool) -> u32 {
    let mut lines = 0u32;
    for para in text.split('\n') {
        if !wrap {
            lines += 1;
            continue;
        }
        let mut n = 1u32;
        let mut cur = 0.0f32;
        let space = char_em(' ') * size_px;
        for word in para.split(' ') {
            let w = text_width_px(word, size_px, bold);
            if cur == 0.0 {
                // A word longer than the line breaks inside the word.
                if w > width_px && width_px > 0.0 {
                    n += (w / width_px).ceil() as u32 - 1;
                    cur = w % width_px;
                } else {
                    cur = w;
                }
            } else if cur + space + w <= width_px {
                cur += space + w;
            } else {
                n += 1;
                if w > width_px && width_px > 0.0 {
                    n += (w / width_px).ceil() as u32 - 1;
                    cur = w % width_px;
                } else {
                    cur = w;
                }
            }
        }
        lines += n;
    }
    lines.max(1)
}

fn col_width_px(sheet: &Sheet, col: u32) -> f32 {
    match sheet.col_dims.get(col) {
        Some(r) if !r.hidden => hmm_to_px(r.size_hmm),
        Some(_) => 0.0,
        None => DEFAULT_COL_WIDTH_PX,
    }
}

/// The height row `row` needs, or `None` when it has no wrapped or
/// larger-than-default text (it then keeps its height).
pub fn needed_height_px(wb: &Workbook, sheet: &Sheet, row: u32) -> Option<f32> {
    let mut best: Option<f32> = None;
    for (ci, col) in sheet.columns.iter().enumerate() {
        let c = ci as u32;
        let style_id = col.styles.get(row);
        let content = col.cells.get(row);
        if content.is_none() {
            continue;
        }
        if sheet.is_merge_covered(row, c) || sheet.merge_at(row, c).is_some() {
            continue;
        }
        let st = wb.styles.get(style_id);
        let size_pt = st.data.font.size_pt.unwrap_or(DEFAULT_FONT_SIZE_PT);
        let wrap = st.data.align.wrap;
        let text = cell_text(wb, CellAddr::new(sheet.id, row, c), style_id);
        if text.is_empty() {
            continue;
        }
        let has_break = text.contains('\n');
        if !wrap && !has_break && size_pt <= DEFAULT_FONT_SIZE_PT {
            continue;
        }
        let size_px = pt_to_px(size_pt);
        let width = (col_width_px(sheet, c) - H_PADDING_PX).max(1.0);
        let lines = line_count(&text, width, size_px, st.data.font.bold, wrap);
        let h = (lines as f32 * size_px * LINE_HEIGHT + v_padding_px()).min(MAX_ROW_HEIGHT_PX);
        best = Some(best.map_or(h, |b: f32| b.max(h)));
    }
    best.map(|h| h.max(DEFAULT_ROW_HEIGHT_PX).round())
}

/// The optimal width of column `col` over rows `r0..=r1` (the widest
/// unwrapped text plus padding, as AOO's `GetOptimalColWidth`), or `None`
/// when no cell there has text.
pub fn needed_width_px(wb: &Workbook, sheet: &Sheet, col: u32, r0: u32, r1: u32) -> Option<f32> {
    let column = sheet.column(col)?;
    let mut best: Option<f32> = None;
    for (row, _) in column.cells.iter_range(r0, r1) {
        if sheet.is_merge_covered(row, col) || sheet.merge_at(row, col).is_some() {
            continue;
        }
        let style_id = column.styles.get(row);
        let st = wb.styles.get(style_id);
        let text = cell_text(wb, CellAddr::new(sheet.id, row, col), style_id);
        if text.is_empty() {
            continue;
        }
        let size_px = pt_to_px(st.data.font.size_pt.unwrap_or(DEFAULT_FONT_SIZE_PT));
        let w = text
            .split('\n')
            .map(|l| text_width_px(l, size_px, st.data.font.bold))
            .fold(0.0f32, f32::max)
            + H_PADDING_PX;
        best = Some(best.map_or(w, |b: f32| b.max(w)));
    }
    best.map(|w| w.ceil().min(4000.0))
}

/// The displayed text of a cell (empty for an empty cell).
pub(crate) fn cell_text(wb: &Workbook, addr: CellAddr, style: StyleId) -> String {
    let v = wb.value(addr);
    match v {
        Value::Empty => String::new(),
        v => crate::service::view::format_with(wb, &v, wb.styles.get(style)).text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrapping_counts_lines() {
        let px = pt_to_px(11.0);
        assert_eq!(line_count("hello", 100.0, px, false, true), 1);
        let long = "the quick brown fox jumps over the lazy dog again and again";
        let n = line_count(long, 92.0, px, false, true);
        assert!((4..=7).contains(&n), "{n}");
        assert_eq!(line_count(long, 92.0, px, false, false), 1);
        assert_eq!(line_count("a\nb\nc", 92.0, px, false, false), 3);
        assert!(text_width_px("WWW", px, true) > text_width_px("iii", px, false));
        // The default line is the default row height.
        let h = px * LINE_HEIGHT + v_padding_px();
        assert!((h - DEFAULT_ROW_HEIGHT_PX).abs() < 0.01);
    }
}
