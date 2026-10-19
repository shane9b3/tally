//! Reference syntax scanning shared by the lexer and the parser.
//!
//! The lexer uses the `scan_*` functions to find where a reference token
//! ends; the parser re-reads the token text with [`parse_a1_ref`] or
//! [`parse_of_ref`] to get its structure.

use tally_types::{letters_to_col, parse_a1, MAX_ROWS};

/// The coordinate part of a reference endpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Part {
    /// `$A$1`: 0-based row and column.
    Cell {
        row: u32,
        col: u32,
        abs_row: bool,
        abs_col: bool,
    },
    /// `$A` (only valid inside `A:B`).
    Col { col: u32, abs: bool },
    /// `$1` (only valid inside `1:2`).
    Row { row: u32, abs: bool },
}

/// A sheet qualifier as written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RawSheet {
    pub name: String,
    pub abs: bool,
    /// The sheet was written as `#REF!` (OpenFormula only).
    pub deleted: bool,
}

/// One endpoint of a reference. `part == None` means `#REF!`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RawEndpoint {
    pub sheet: Option<RawSheet>,
    pub part: Option<Part>,
}

/// A whole reference: a single endpoint or a `start:end` pair.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RawRef {
    pub start: RawEndpoint,
    pub end: Option<RawEndpoint>,
}

/// The shape of a scanned A1 reference token.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RefShape {
    Cell,
    Range,
}

/// First character of an identifier or unquoted sheet name.
pub(crate) fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || (!c.is_ascii() && c.is_alphabetic())
}

/// A character that may continue an identifier (names, function names).
pub(crate) fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '.' || (!c.is_ascii() && c.is_alphanumeric())
}

/// A character that may continue an unquoted sheet name.
pub(crate) fn is_sheet_char(c: char) -> bool {
    c != '.' && is_ident_char(c)
}

fn char_at(s: &str, i: usize) -> Option<char> {
    s.get(i..).and_then(|t| t.chars().next())
}

/// Scans a `'quoted'` name starting at `i` (which must be `'`). Returns the
/// byte index after the closing quote and the unescaped name.
pub(crate) fn scan_quoted(s: &str, i: usize) -> Option<(usize, String)> {
    let b = s.as_bytes();
    if b.get(i) != Some(&b'\'') {
        return None;
    }
    let mut j = i + 1;
    let mut name = String::new();
    loop {
        let rest = s.get(j..)?;
        let q = rest.find('\'')?;
        name.push_str(rest.get(..q)?);
        let after = j + q + 1;
        if b.get(after) == Some(&b'\'') {
            name.push('\'');
            j = after + 1;
        } else {
            return Some((after, name));
        }
    }
}

/// Scans the coordinate part at `i`. The character after it must not be an
/// identifier character, `$` or `(`.
pub(crate) fn scan_part(s: &str, i: usize) -> Option<(usize, Part)> {
    let b = s.as_bytes();
    let mut j = i;
    let abs_col = b.get(j) == Some(&b'$');
    if abs_col {
        j += 1;
    }
    let ls = j;
    while j < b.len() && b[j].is_ascii_alphabetic() {
        j += 1;
    }
    let letters = s.get(ls..j)?;
    let abs_row = b.get(j) == Some(&b'$');
    if abs_row {
        j += 1;
    }
    let ds = j;
    while j < b.len() && b[j].is_ascii_digit() {
        j += 1;
    }
    let digits = s.get(ds..j)?;
    if let Some(c) = char_at(s, j) {
        if is_ident_char(c) || c == '$' || c == '(' {
            return None;
        }
    }
    let part = match (letters.is_empty(), digits.is_empty()) {
        (false, false) => Part::Cell {
            row: parse_row(digits)?,
            col: letters_to_col(letters)?,
            abs_row,
            abs_col,
        },
        (false, true) if !abs_row => Part::Col {
            col: letters_to_col(letters)?,
            abs: abs_col,
        },
        (true, false) if !(abs_col && abs_row) => Part::Row {
            row: parse_row(digits)?,
            abs: abs_col || abs_row,
        },
        _ => return None,
    };
    Some((j, part))
}

fn parse_row(digits: &str) -> Option<u32> {
    if digits.len() > 7 {
        return None;
    }
    let n: u32 = digits.parse().ok()?;
    (1..=MAX_ROWS).contains(&n).then(|| n - 1)
}

/// Scans a CalcA1 sheet qualifier (`$Sheet1.`, `'My Sheet'!`) at `i`.
/// Returns the index after the separator.
fn scan_sheet(s: &str, i: usize) -> Option<(usize, RawSheet)> {
    let b = s.as_bytes();
    let mut j = i;
    let abs = b.get(j) == Some(&b'$');
    if abs {
        j += 1;
    }
    let (end, name, quoted) = if b.get(j) == Some(&b'\'') {
        let (e, n) = scan_quoted(s, j)?;
        (e, n, true)
    } else {
        let first = char_at(s, j)?;
        if !is_ident_start(first) {
            return None;
        }
        let mut e = j + first.len_utf8();
        while let Some(c) = char_at(s, e) {
            if !is_sheet_char(c) {
                break;
            }
            e += c.len_utf8();
        }
        (e, s.get(j..e)?.to_string(), false)
    };
    match b.get(end) {
        Some(b'.') => {}
        // `A1!B1` is an intersection, not a sheet named "A1".
        Some(b'!') if quoted || parse_a1(&name).is_none() => {}
        _ => return None,
    }
    Some((
        end + 1,
        RawSheet {
            name,
            abs,
            deleted: false,
        },
    ))
}

/// Scans one CalcA1 endpoint: an optional sheet qualifier plus a part.
pub(crate) fn scan_endpoint(s: &str, i: usize) -> Option<(usize, RawEndpoint)> {
    if let Some((j, sheet)) = scan_sheet(s, i) {
        if let Some((k, part)) = scan_part(s, j) {
            return Some((
                k,
                RawEndpoint {
                    sheet: Some(sheet),
                    part: Some(part),
                },
            ));
        }
    }
    let (k, part) = scan_part(s, i)?;
    Some((
        k,
        RawEndpoint {
            sheet: None,
            part: Some(part),
        },
    ))
}

fn compatible(a: Option<Part>, b: Option<Part>) -> bool {
    matches!(
        (a, b),
        (Some(Part::Cell { .. }), Some(Part::Cell { .. }))
            | (Some(Part::Col { .. }), Some(Part::Col { .. }))
            | (Some(Part::Row { .. }), Some(Part::Row { .. }))
    )
}

/// Scans a CalcA1 reference token at `i`: a cell (`Sheet1.$A$1`) or an
/// adjacent range (`A1:B2`, `A:A`, `1:1`, `Sheet1.A1:Sheet2.B2`).
pub(crate) fn scan_a1_ref(s: &str, i: usize) -> Option<(usize, RefShape, RawRef)> {
    let (e1, ep1) = scan_endpoint(s, i)?;
    if s.as_bytes().get(e1) == Some(&b':') {
        if let Some((e2, ep2)) = scan_endpoint(s, e1 + 1) {
            if compatible(ep1.part, ep2.part) {
                return Some((
                    e2,
                    RefShape::Range,
                    RawRef {
                        start: ep1,
                        end: Some(ep2),
                    },
                ));
            }
        }
    }
    if matches!(ep1.part, Some(Part::Cell { .. })) {
        Some((
            e1,
            RefShape::Cell,
            RawRef {
                start: ep1,
                end: None,
            },
        ))
    } else {
        None
    }
}

/// Parses the full text of a CalcA1 `CellRef` / `RangeRef` token.
pub(crate) fn parse_a1_ref(text: &str) -> Option<RawRef> {
    let (end, _, r) = scan_a1_ref(text, 0)?;
    (end == text.len()).then_some(r)
}

/// Scans an OpenFormula bracketed reference `[...]` at `i` (which must be
/// `[`), honouring quoted sheet names. Returns the index after `]`.
pub(crate) fn scan_bracket(s: &str, i: usize) -> Option<usize> {
    let b = s.as_bytes();
    if b.get(i) != Some(&b'[') {
        return None;
    }
    let mut j = i + 1;
    while j < b.len() {
        match b[j] {
            b']' => return Some(j + 1),
            b'\'' => j = scan_quoted(s, j)?.0,
            _ => j += 1,
        }
    }
    None
}

fn of_endpoint(s: &str, i: usize) -> Option<(usize, RawEndpoint)> {
    let b = s.as_bytes();
    let mut j = i;
    let abs = b.get(j) == Some(&b'$');
    if abs {
        j += 1;
    }
    let sheet = if b.get(j) == Some(&b'.') {
        if abs {
            return None;
        }
        j += 1;
        None
    } else {
        let (e, name, deleted) = if b.get(j) == Some(&b'\'') {
            let (e, n) = scan_quoted(s, j)?;
            (e, n, false)
        } else if s.get(j..)?.starts_with("#REF!") {
            (j + 5, String::new(), true)
        } else {
            let rest = s.get(j..)?;
            let n = rest.find(['.', ':', ']', '\''])?;
            if n == 0 {
                return None;
            }
            (j + n, rest.get(..n)?.to_string(), false)
        };
        if b.get(e) != Some(&b'.') {
            return None;
        }
        j = e + 1;
        Some(RawSheet { name, abs, deleted })
    };
    if s.get(j..)?.starts_with("#REF!") {
        return Some((j + 5, RawEndpoint { sheet, part: None }));
    }
    let (k, part) = scan_part(s, j)?;
    Some((
        k,
        RawEndpoint {
            sheet,
            part: Some(part),
        },
    ))
}

/// Parses the inside of an OpenFormula reference (without the brackets):
/// `.A1`, `$Sheet1.A1:.B2`, `.#REF!`, `.A:.A`.
pub(crate) fn parse_of_ref(inner: &str) -> Option<RawRef> {
    let (e1, start) = of_endpoint(inner, 0)?;
    if e1 == inner.len() {
        return Some(RawRef { start, end: None });
    }
    if inner.as_bytes().get(e1) != Some(&b':') {
        return None;
    }
    let (e2, end) = of_endpoint(inner, e1 + 1)?;
    if e2 != inner.len() {
        return None;
    }
    if start.part.is_some() && end.part.is_some() && !compatible(start.part, end.part) {
        return None;
    }
    Some(RawRef {
        start,
        end: Some(end),
    })
}

/// Whether a sheet name must be quoted when printed.
pub(crate) fn sheet_needs_quotes(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return true;
    };
    !is_ident_start(first) || !chars.all(is_sheet_char) || parse_a1(name).is_some()
}

/// Appends a sheet name, quoted and escaped when needed.
pub(crate) fn push_sheet_name(out: &mut String, name: &str) {
    if sheet_needs_quotes(name) {
        out.push('\'');
        out.push_str(&name.replace('\'', "''"));
        out.push('\'');
    } else {
        out.push_str(name);
    }
}
