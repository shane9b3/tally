//! R1C1 reference text for `INDIRECT(text; 0)` (and `ADDRESS`-style
//! round trips): `R1C1`, `R[2]C[-1]`, `RC`, `R3` (a whole row), `C2` (a
//! whole column), areas `R1C1:R2C3`, optionally sheet-qualified
//! (`Sheet2!R1C1`, `'My Sheet'.R1C1`). Relative parts are relative to the
//! formula cell.

use tally_types::{CellAddr, CellRange, SheetId, MAX_COLS, MAX_ROWS};

use crate::workbook::Workbook;

/// One axis: `None` when the letter is absent.
fn axis(s: &str, letter: char, origin: u32, limit: u32) -> Option<(Option<u32>, &str)> {
    let mut chars = s.char_indices();
    match chars.next() {
        Some((_, c)) if c.eq_ignore_ascii_case(&letter) => {}
        _ => return Some((None, s)),
    }
    let rest = &s[1..];
    if let Some(inner) = rest.strip_prefix('[') {
        let end = inner.find(']')?;
        let off: i64 = inner[..end].trim().parse().ok()?;
        let v = i64::from(origin) + off;
        if !(0..i64::from(limit)).contains(&v) {
            return None;
        }
        return Some((Some(v as u32), &inner[end + 1..]));
    }
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return Some((Some(origin), rest));
    }
    let n: u64 = rest[..digits].parse().ok()?;
    if n == 0 || n > u64::from(limit) {
        return None;
    }
    Some((Some(n as u32 - 1), &rest[digits..]))
}

/// `(row, col)` of one R1C1 endpoint; `None` parts are "whole".
fn endpoint(s: &str, origin: CellAddr) -> Option<(Option<u32>, Option<u32>)> {
    let (row, rest) = axis(s, 'R', origin.row, MAX_ROWS)?;
    let (col, rest) = axis(rest, 'C', origin.col, MAX_COLS)?;
    if !rest.is_empty() || (row.is_none() && col.is_none()) {
        return None;
    }
    Some((row, col))
}

fn split_sheet(text: &str) -> (Option<String>, &str) {
    if let Some(rest) = text.strip_prefix('\'') {
        let mut name = String::new();
        let mut it = rest.char_indices().peekable();
        while let Some((i, c)) = it.next() {
            if c == '\'' {
                if it.peek().map(|(_, c)| *c) == Some('\'') {
                    it.next();
                    name.push('\'');
                    continue;
                }
                let after = &rest[i + 1..];
                let r = after
                    .strip_prefix('!')
                    .or_else(|| after.strip_prefix('.'))
                    .unwrap_or(after);
                return (Some(name), r);
            }
            name.push(c);
        }
        return (None, text);
    }
    match text.rfind(['!', '.']) {
        Some(i) => (
            Some(text[..i].trim_start_matches('$').to_string()),
            &text[i + 1..],
        ),
        None => (None, text),
    }
}

/// Parses R1C1 reference text at `origin`.
pub fn parse(wb: &Workbook, text: &str, origin: CellAddr) -> Option<CellRange> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    let (sheet_name, body) = split_sheet(t);
    let sheet: SheetId = match sheet_name {
        Some(n) => wb.sheet_by_name_ci(&n)?,
        None => origin.sheet,
    };
    let (a, b) = match body.split_once(':') {
        Some((a, b)) => (endpoint(a, origin)?, endpoint(b, origin)?),
        None => {
            let e = endpoint(body, origin)?;
            (e, e)
        }
    };
    let (r0, r1) = match (a.0, b.0) {
        (Some(x), Some(y)) => (x.min(y), x.max(y)),
        (None, None) => (0, MAX_ROWS - 1),
        _ => return None,
    };
    let (c0, c1) = match (a.1, b.1) {
        (Some(x), Some(y)) => (x.min(y), x.max(y)),
        (None, None) => (0, MAX_COLS - 1),
        _ => return None,
    };
    Some(CellRange::new(sheet, r0, c0, r1, c1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forms() {
        let mut wb = Workbook::new();
        let s = wb.sheets[0].id;
        let s2 = wb.add_sheet(Some("My Data"), None);
        let at = CellAddr::new(s, 4, 3); // D5
        let p = |t: &str| parse(&wb, t, at);
        assert_eq!(p("R1C1"), Some(CellRange::new(s, 0, 0, 0, 0)));
        assert_eq!(p("r2c3"), Some(CellRange::new(s, 1, 2, 1, 2)));
        assert_eq!(p("R[2]C[-1]"), Some(CellRange::new(s, 6, 2, 6, 2)));
        assert_eq!(p("RC"), Some(CellRange::new(s, 4, 3, 4, 3)));
        assert_eq!(p("R1C1:R2C3"), Some(CellRange::new(s, 0, 0, 1, 2)));
        assert_eq!(p("R3"), Some(CellRange::new(s, 2, 0, 2, MAX_COLS - 1)));
        assert_eq!(p("C2:C3"), Some(CellRange::new(s, 0, 1, MAX_ROWS - 1, 2)));
        assert_eq!(p("'My Data'!R1C2"), Some(CellRange::new(s2, 0, 1, 0, 1)));
        assert_eq!(p("Sheet1.R1C1"), Some(CellRange::new(s, 0, 0, 0, 0)));
        assert_eq!(p("R[-9]C1"), None, "above row 1");
        assert_eq!(p("R0C1"), None);
        assert_eq!(p("A1"), None);
        assert_eq!(p("Nope!R1C1"), None);
    }
}
