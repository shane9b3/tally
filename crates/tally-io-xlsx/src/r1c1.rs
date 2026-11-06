//! Excel R1C1 formula text → Excel A1 text, for the readers of formats that
//! store formulas in R1C1 notation (Excel 2003 XML `ss:Formula`, SYLK `E`
//! fields). The tally-formula parser has no R1C1 mode, so references are
//! rewritten textually relative to the formula cell and the result goes
//! through the ExcelA1 parser ([`crate::excel_formula_to_of`]).
//!
//! Recognised: `R1C1` (absolute), `R[-1]C[2]` (relative), `RC` (this
//! cell), `R2` / `R[1]` (whole rows), `C3` / `C[-1]` (whole columns), any
//! of them after a sheet prefix (`Sheet2!R1C1`, `'My Sheet'!RC`) and in
//! ranges (`R1C1:R2C2`, `R1:R3`, `C[-1]:C`). String literals and quoted
//! sheet names are copied unchanged. A token followed by `(` or by a name
//! character is a function or a name (`ROUND(`, `RATE`), not a reference.

use tally_types::{col_to_letters, MAX_COLS, MAX_ROWS};

/// Longest formula text converted (longer text is refused).
const MAX_LEN: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    /// No number: the formula cell's row or column.
    Current,
    Abs(u32),
    Rel(i64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Token {
    row: Option<Part>,
    col: Option<Part>,
    /// Byte length in the source.
    len: usize,
}

fn is_name_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'.' || c == b'\\' || c >= 0x80
}

/// Parses `R…` or `C…` (one part) at `s`; returns the part and its length.
fn part(s: &[u8], letter: u8) -> Option<(Part, usize)> {
    if s.first().map(u8::to_ascii_uppercase) != Some(letter) {
        return None;
    }
    let rest = &s[1..];
    if rest.first() == Some(&b'[') {
        let mut i = 1;
        let neg = rest.get(i) == Some(&b'-');
        if neg || rest.get(i) == Some(&b'+') {
            i += 1;
        }
        let start = i;
        while rest.get(i).is_some_and(u8::is_ascii_digit) && i - start < 8 {
            i += 1;
        }
        if i == start || rest.get(i) != Some(&b']') {
            return None;
        }
        let n: i64 = std::str::from_utf8(&rest[start..i]).ok()?.parse().ok()?;
        return Some((Part::Rel(if neg { -n } else { n }), i + 2));
    }
    let mut i = 0;
    while rest.get(i).is_some_and(u8::is_ascii_digit) && i < 8 {
        i += 1;
    }
    if i == 0 {
        return Some((Part::Current, 1));
    }
    let n: u32 = std::str::from_utf8(&rest[..i]).ok()?.parse().ok()?;
    Some((Part::Abs(n), i + 1))
}

/// An R1C1 reference token at `s` (the caller checked the character before).
fn token(s: &[u8]) -> Option<Token> {
    let (row, rl) = match part(s, b'R') {
        Some((p, l)) => (Some(p), l),
        None => (None, 0),
    };
    let (col, cl) = match part(&s[rl..], b'C') {
        Some((p, l)) => (Some(p), l),
        None => (None, 0),
    };
    if row.is_none() && col.is_none() {
        return None;
    }
    let len = rl + cl;
    // `ROUND(`, `RATE`, `CR`, `R1C1X`: a name or a function, not a reference.
    match s.get(len) {
        Some(&c) if is_name_char(c) || c == b'(' => None,
        _ => Some(Token { row, col, len }),
    }
}

/// 1-based absolute index of `p` relative to `base` (0-based), within `max`.
fn resolve(p: Part, base: u32, max: u32) -> Option<(u32, bool)> {
    let (v, abs) = match p {
        Part::Current => (i64::from(base) + 1, false),
        Part::Abs(n) => (i64::from(n), true),
        Part::Rel(d) => (i64::from(base) + 1 + d, false),
    };
    (1..=i64::from(max)).contains(&v).then_some((v as u32, abs))
}

fn dollar(abs: bool) -> &'static str {
    if abs {
        "$"
    } else {
        ""
    }
}

fn row_text(p: Part, row: u32) -> Option<String> {
    let (r, abs) = resolve(p, row, MAX_ROWS)?;
    Some(format!("{}{r}", dollar(abs)))
}

fn col_text(p: Part, col: u32) -> Option<String> {
    let (c, abs) = resolve(p, col, MAX_COLS)?;
    Some(format!("{}{}", dollar(abs), col_to_letters(c - 1)))
}

/// Converts R1C1 references in `text` (a formula without or with its
/// leading `=`, which is kept) to A1 for a formula in 0-based
/// (`row`, `col`). `None` when a reference falls off the sheet or the text
/// is too long.
pub fn r1c1_to_a1(text: &str, row: u32, col: u32) -> Option<String> {
    if text.len() > MAX_LEN {
        return None;
    }
    let s = text.as_bytes();
    let mut out = String::with_capacity(text.len() + 8);
    let mut i = 0;
    let mut copied = 0;
    while i < s.len() {
        let c = s[i];
        if c == b'"' || c == b'\'' {
            // A string literal or a quoted sheet name: copy to its end
            // (doubled quotes are escapes).
            i += 1;
            while i < s.len() {
                if s[i] == c {
                    if s.get(i + 1) == Some(&c) {
                        i += 2;
                        continue;
                    }
                    break;
                }
                i += 1;
            }
            i += 1;
            continue;
        }
        let prev_ok = i == 0 || !(is_name_char(s[i - 1]) || s[i - 1] == b'$');
        let Some(t) = (prev_ok && matches!(c.to_ascii_uppercase(), b'R' | b'C'))
            .then(|| token(&s[i..]))
            .flatten()
        else {
            i += 1;
            continue;
        };
        out.push_str(&text[copied..i]);
        let mut end = i + t.len;
        // A second token of a range (`R1:R3`, `C1:C2`, `R1C1:R2C2`).
        let second = (s.get(end) == Some(&b':'))
            .then(|| token(&s[end + 1..]))
            .flatten()
            .filter(|u| u.row.is_some() == t.row.is_some() && u.col.is_some() == t.col.is_some());
        let one = |t: &Token| -> Option<String> {
            Some(match (t.row, t.col) {
                (Some(r), Some(c)) => format!("{}{}", col_text(c, col)?, row_text(r, row)?),
                (Some(r), None) => row_text(r, row)?,
                (None, Some(c)) => col_text(c, col)?,
                (None, None) => return None,
            })
        };
        let a = one(&t)?;
        match second {
            Some(u) => {
                out.push_str(&a);
                out.push(':');
                out.push_str(&one(&u)?);
                end += 1 + u.len;
            }
            // A lone whole row or column is a range of one (`3:3`).
            None if t.row.is_none() || t.col.is_none() => {
                out.push_str(&a);
                out.push(':');
                out.push_str(&a);
            }
            None => out.push_str(&a),
        }
        i = end;
        copied = end;
    }
    out.push_str(&text[copied..]);
    Some(out)
}

/// R1C1 formula text (with or without `=`) at 0-based (`sheet`, `row`,
/// `col`) of a workbook with `sheet_names` → model OpenFormula text
/// (`of:=…`), or `None` when it does not convert or parse.
pub fn r1c1_formula_to_of(
    text: &str,
    sheet_names: &[String],
    sheet: usize,
    row: u32,
    col: u32,
) -> Option<String> {
    let a1 = r1c1_to_a1(text, row, col)?;
    let body = a1.strip_prefix('=').unwrap_or(&a1);
    crate::excel_formula_to_of(body, sheet_names, sheet, row, col)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conv(t: &str, r: u32, c: u32) -> String {
        r1c1_to_a1(t, r, c).unwrap_or_else(|| "<none>".into())
    }

    #[test]
    fn cells_rows_columns_and_ranges() {
        // Formula in C5 (row 4, col 2).
        assert_eq!(conv("=R1C1", 4, 2), "=$A$1");
        assert_eq!(conv("=RC", 4, 2), "=C5");
        assert_eq!(conv("=R[-1]C+RC[-2]", 4, 2), "=C4+A5");
        assert_eq!(conv("=SUM(R[-4]C:R[-1]C)", 4, 2), "=SUM(C1:C4)");
        assert_eq!(conv("=SUM(R2)", 4, 2), "=SUM($2:$2)");
        assert_eq!(conv("=SUM(C[1]:C3)", 4, 2), "=SUM(D:$C)");
        assert_eq!(conv("=SUM(R1:R[1])", 4, 2), "=SUM($1:6)");
        assert_eq!(conv("=Sheet2!R1C[1]", 4, 2), "=Sheet2!D$1");
        assert_eq!(conv("='R1C1'!RC", 0, 0), "='R1C1'!A1");
        assert_eq!(conv("=r2c2", 0, 0), "=$B$2");
    }

    #[test]
    fn names_functions_and_strings_are_kept() {
        assert_eq!(conv("=ROUND(RC[1],2)", 0, 0), "=ROUND(B1,2)");
        assert_eq!(conv("=RATE+CR", 0, 0), "=RATE+CR");
        assert_eq!(conv("=\"R1C1\"&RC", 0, 0), "=\"R1C1\"&A1");
        assert_eq!(conv("=COUNT(C:C)", 0, 0), "=COUNT(A:A)");
        assert_eq!(conv("=MyName_RC", 0, 0), "=MyName_RC");
    }

    #[test]
    fn off_sheet_and_hostile_input() {
        assert!(r1c1_to_a1("=R[-1]C", 0, 0).is_none());
        assert!(r1c1_to_a1("=RC[-1]", 0, 0).is_none());
        // More than 8 digits: not a reference, copied as written.
        assert_eq!(conv("=R9999999999C1", 0, 0), "=R9999999999C1");
        assert!(r1c1_to_a1("=R2000000C1", 0, 0).is_none());
        assert!(r1c1_to_a1(&"R".repeat(MAX_LEN + 1), 0, 0).is_none());
        // Unterminated quotes and brackets never panic.
        for t in [
            "=\"abc",
            "='abc",
            "=R[",
            "=R[-",
            "=C[99999999999]",
            "=R[1",
            "é=RCé",
            "=R",
        ] {
            let _ = r1c1_to_a1(t, 3, 3);
        }
    }

    #[test]
    fn to_open_formula() {
        let names = vec!["Sheet1".to_string(), "Data".to_string()];
        assert_eq!(
            r1c1_formula_to_of("=SUM(R[-2]C:R[-1]C)", &names, 0, 2, 0).as_deref(),
            Some("of:=SUM([.A1:.A2])")
        );
        assert_eq!(
            r1c1_formula_to_of("=Data!R1C1*2", &names, 0, 0, 0).as_deref(),
            Some("of:=[Data.$A$1]*2")
        );
    }
}
