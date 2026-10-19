//! The internal clipboard (plan §4.3): `copy` stores a snapshot of the
//! selection (contents with relative formulas, and styles) under a random
//! token and returns TSV and HTML renderings; pasting that token re-anchors
//! the formulas at the target (the AST is relative, so the same formula is
//! simply placed at the new address) and carries the styles. Only the last
//! snapshot is kept.

use tally_model::StyleData;
use tally_types::{CellAddr, CellRange};

use crate::storage::{ColumnStore, StyleId};
use crate::workbook::{Rng, SnapContent, Workbook};

/// A copied block.
#[derive(Clone, Debug)]
pub struct ClipSnapshot {
    /// Token embedded in the HTML (`data-tally-clip`).
    pub token: String,
    /// Rows of the block.
    pub rows: u32,
    /// Columns of the block.
    pub cols: u32,
    /// Non-empty or styled cells: `(row offset, col offset, content, style index)`.
    pub cells: Vec<(u32, u32, Option<SnapContent>, Option<usize>)>,
    /// Styles referenced by `cells`.
    pub styles: Vec<StyleData>,
}

/// Copies `range` of `wb`. Returns the snapshot and its TSV/HTML text.
pub fn copy_range(
    wb: &Workbook,
    range: CellRange,
    rng: &mut Rng,
) -> (ClipSnapshot, String, String) {
    let token = format!("{:016x}{:016x}", rng.next_u64(), rng.next_u64());
    let mut cells = Vec::new();
    let mut styles: Vec<StyleData> = Vec::new();
    let mut style_idx: Vec<(StyleId, usize)> = Vec::new();
    if let Some(sheet) = wb.sheet(range.sheet) {
        for c in range.c0..=range.c1.min(sheet.columns.len().saturating_sub(1) as u32) {
            let Some(col) = sheet.column(c) else {
                continue;
            };
            let mut rows: Vec<u32> = col
                .cells
                .iter_range(range.r0, range.r1)
                .map(|(r, _)| r)
                .collect();
            for (s, e, _) in col.styles.runs_in(range.r0, range.r1) {
                rows.extend(s..=e);
            }
            rows.sort_unstable();
            rows.dedup();
            for r in rows {
                let addr = CellAddr::new(range.sheet, r, c);
                let content = wb.snapshot_content(addr);
                let st = col.styles.get(r);
                let si = (st != StyleId::DEFAULT).then(|| {
                    if let Some((_, i)) = style_idx.iter().find(|(s, _)| *s == st) {
                        *i
                    } else {
                        styles.push(wb.styles.get(st).data.clone());
                        style_idx.push((st, styles.len() - 1));
                        styles.len() - 1
                    }
                });
                cells.push((r - range.r0, c - range.c0, content, si));
            }
        }
    }
    let tsv = render_tsv(wb, range);
    let html = render_html(wb, range, &token);
    (
        ClipSnapshot {
            token,
            rows: range.rows(),
            cols: range.cols(),
            cells,
            styles,
        },
        tsv,
        html,
    )
}

fn display(wb: &Workbook, addr: CellAddr) -> String {
    crate::service::view::display_text(wb, addr)
}

fn render_tsv(wb: &Workbook, range: CellRange) -> String {
    let mut out = String::new();
    for r in range.r0..=range.r1 {
        let mut line: Vec<String> = Vec::with_capacity(range.cols() as usize);
        for c in range.c0..=range.c1 {
            line.push(display(wb, CellAddr::new(range.sheet, r, c)));
        }
        out.push_str(line.join("\t").trim_end_matches('\t'));
        out.push('\n');
    }
    out
}

/// Escapes text for HTML.
pub fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

fn render_html(wb: &Workbook, range: CellRange, token: &str) -> String {
    let mut out = format!("<table data-tally-clip=\"{}\">", escape_html(token));
    for r in range.r0..=range.r1 {
        out.push_str("<tr>");
        for c in range.c0..=range.c1 {
            let addr = CellAddr::new(range.sheet, r, c);
            let st = wb.styles.get(wb.style_at(addr));
            let mut css = String::new();
            if st.data.font.bold {
                css.push_str("font-weight:bold;");
            }
            if st.data.font.italic {
                css.push_str("font-style:italic;");
            }
            if css.is_empty() {
                out.push_str("<td>");
            } else {
                out.push_str(&format!("<td style=\"{css}\">"));
            }
            out.push_str(&escape_html(&display(wb, addr)));
            out.push_str("</td>");
        }
        out.push_str("</tr>");
    }
    out.push_str("</table>");
    out
}

/// Splits TSV text into rows of fields (`\r\n` or `\n`; a trailing newline
/// adds no row; quoted fields may contain tabs, newlines and `""`).
pub fn parse_tsv(text: &str) -> Vec<Vec<String>> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut chars = text.chars().peekable();
    let mut at_field_start = true;
    let mut in_quotes = false;
    while let Some(ch) = chars.next() {
        if in_quotes {
            if ch == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    in_quotes = false;
                }
            } else {
                field.push(ch);
            }
            continue;
        }
        match ch {
            '"' if at_field_start => {
                in_quotes = true;
                at_field_start = false;
            }
            '\t' => {
                row.push(std::mem::take(&mut field));
                at_field_start = true;
            }
            '\r' if chars.peek() == Some(&'\n') => {}
            '\n' | '\r' => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
                at_field_start = true;
            }
            c => {
                field.push(c);
                at_field_start = false;
            }
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tsv_parsing() {
        assert_eq!(
            parse_tsv("1\t2\n3\t4\n"),
            vec![vec!["1", "2"], vec!["3", "4"]]
        );
        assert_eq!(parse_tsv("a\r\nb"), vec![vec!["a"], vec!["b"]]);
        assert_eq!(parse_tsv("\"x\ty\"\t\"q\"\"\""), vec![vec!["x\ty", "q\""]]);
        assert_eq!(parse_tsv("a\t\tb"), vec![vec!["a", "", "b"]]);
        assert!(parse_tsv("").is_empty());
    }

    #[test]
    fn html_escape() {
        assert_eq!(escape_html("<a & 'b'>"), "&lt;a &amp; &#39;b&#39;&gt;");
    }
}
