//! R1C1 range references outside formulas (`ss:ArrayRange`,
//! `x:AutoFilter/@x:Range`, `Print_Area`, `Print_Titles`) and the
//! absolute-sheet rewrite of named-range formulas.

use tally_model::CellRangeLocal;
use tally_types::{MAX_COLS, MAX_ROWS};

/// One R1C1 reference part relative to a 0-based origin coordinate:
/// `R5` → 4, `R[-1]` → origin − 1, `R` → origin. `None` when off-sheet.
fn part(s: &str, letter: char, origin: u32, max: u32) -> Option<(Option<u32>, usize)> {
    let mut it = s.char_indices();
    match it.next() {
        Some((_, c)) if c.eq_ignore_ascii_case(&letter) => {}
        _ => return Some((None, 0)),
    }
    let rest = &s[1..];
    let (v, len): (i64, usize) = if let Some(inner) = rest.strip_prefix('[') {
        let close = inner.find(']')?;
        let n: i64 = inner[..close].trim().parse().ok()?;
        if n.abs() > i64::from(max) {
            return None;
        }
        (i64::from(origin) + n, close + 3)
    } else {
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            (i64::from(origin), 1)
        } else {
            if digits > 8 {
                return None;
            }
            let n: i64 = rest[..digits].parse().ok()?;
            (n - 1, digits + 1)
        }
    };
    (0..i64::from(max))
        .contains(&v)
        .then_some((Some(v as u32), len))
}

/// One cell, row or column reference: (row, col), each `None` when the
/// reference is a whole column / whole row.
fn one(s: &str, origin: (u32, u32)) -> Option<(Option<u32>, Option<u32>)> {
    let (r, rl) = part(s, 'R', origin.0, MAX_ROWS)?;
    let (c, cl) = part(&s[rl..], 'C', origin.1, MAX_COLS)?;
    if rl + cl != s.len() || (r.is_none() && c.is_none()) {
        return None;
    }
    Some((r, c))
}

/// An R1C1 range (`R1C1:R5C3`, `RC:R[1]C[2]`, `R2:R3`, `C1`) relative to
/// the 0-based `origin`, after an optional `Sheet!` prefix (returned).
pub(crate) fn range(s: &str, origin: (u32, u32)) -> Option<(Option<String>, CellRangeLocal)> {
    let s = s.trim().trim_start_matches('=');
    let (sheet, body) = match s.rfind('!') {
        Some(i) => {
            let name = s[..i].trim();
            let name = name
                .strip_prefix('\'')
                .and_then(|n| n.strip_suffix('\''))
                .map_or_else(|| name.to_string(), |n| n.replace("''", "'"));
            (Some(name), &s[i + 1..])
        }
        None => (None, s),
    };
    let (a, b) = match body.split_once(':') {
        Some((a, b)) => (one(a.trim(), origin)?, one(b.trim(), origin)?),
        None => {
            let a = one(body.trim(), origin)?;
            (a, a)
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
    Some((sheet, CellRangeLocal { r0, c0, r1, c1 }))
}

/// Splits a reference list at top-level commas (outside quotes).
pub(crate) fn split_list(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut quote = false;
    let mut start = 0;
    for (i, c) in s.char_indices() {
        match c {
            '\'' => quote = !quote,
            ',' if !quote => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out
}

/// Marks every sheet reference in OpenFormula text absolute
/// (`[Sheet1.$A$1]` → `[$Sheet1.$A$1]`), as the XLSX reader's
/// `excel_name_to_of` does for defined names: named ranges must keep
/// pointing at their sheet wherever they are used.
pub(crate) fn absolute_sheets(of: &str) -> String {
    let mut out = String::with_capacity(of.len() + 8);
    let mut in_ref = false;
    let mut in_str = false;
    let mut in_quote = false;
    // At the start of a reference part (after `[` or `:` inside one).
    let mut at_part = false;
    for c in of.chars() {
        if in_str {
            out.push(c);
            if c == '"' {
                in_str = false;
            }
            continue;
        }
        if in_quote {
            out.push(c);
            if c == '\'' {
                in_quote = false;
            }
            continue;
        }
        if at_part {
            at_part = false;
            if c != '.' && c != '$' && c != ']' {
                out.push('$');
            }
        }
        match c {
            '"' if !in_ref => in_str = true,
            '\'' if in_ref => in_quote = true,
            '[' => {
                in_ref = true;
                at_part = true;
            }
            ']' => in_ref = false,
            ':' if in_ref => at_part = true,
            _ => {}
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(r0: u32, c0: u32, r1: u32, c1: u32) -> CellRangeLocal {
        CellRangeLocal { r0, c0, r1, c1 }
    }

    #[test]
    fn ranges() {
        assert_eq!(range("R1C1:R5C2", (0, 0)), Some((None, r(0, 0, 4, 1))));
        assert_eq!(range("RC:R[1]C[2]", (3, 3)), Some((None, r(3, 3, 4, 5))));
        assert_eq!(
            range("='My Sheet'!R2:R3", (0, 0)),
            Some((Some("My Sheet".into()), r(1, 0, 2, MAX_COLS - 1)))
        );
        assert_eq!(range("C1", (0, 0)), Some((None, r(0, 0, MAX_ROWS - 1, 0))));
        assert_eq!(range("R[-1]C", (0, 0)), None);
        assert_eq!(range("R99999999999C1", (0, 0)), None);
        assert_eq!(range("R2000000C1", (0, 0)), None);
        assert_eq!(range("R1C1:C2", (0, 0)), None);
        for junk in [
            "",
            "R[",
            "R[-",
            "!",
            "'",
            "RC[99999999999999999999]",
            "é",
            "R1é",
        ] {
            let _ = range(junk, (5, 5));
        }
    }

    #[test]
    fn sheets_become_absolute() {
        assert_eq!(
            absolute_sheets("of:=[Sheet1.$A$1:.$B$2]"),
            "of:=[$Sheet1.$A$1:.$B$2]"
        );
        assert_eq!(
            absolute_sheets("of:=['a[b'.$A$1]&\"[x\""),
            "of:=[$'a[b'.$A$1]&\"[x\""
        );
        assert_eq!(absolute_sheets("of:=[.A1]"), "of:=[.A1]");
        assert_eq!(absolute_sheets("of:=[S1.A1:S2.B2]"), "of:=[$S1.A1:$S2.B2]");
    }
}
