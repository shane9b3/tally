//! Hand-written tokenizer.
//!
//! The lexer never fails: characters it does not understand become
//! [`TokenKind::Invalid`] tokens, which the parser reports as Err:501.
//! Whitespace is kept as [`TokenKind::Whitespace`] tokens so spans cover the
//! whole input (the parser skips them). Spans are UTF-8 byte offsets into the
//! source string.

use std::ops::Range;

use crate::dialect::Dialect;
use crate::refsyntax::{
    is_ident_char, is_ident_start, is_sheet_char, scan_a1_ref, scan_bracket, scan_quoted, RawRef,
    RefShape,
};

/// The kind of a token.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum TokenKind {
    /// The leading `=`, `of:=` or `oooc:=`.
    Prefix,
    /// A run of whitespace.
    Whitespace,
    /// A number literal: `12`, `1.5`, `.5`, `1E+20`.
    Number,
    /// A string literal including its quotes; `""` escapes a quote.
    String,
    /// A bare identifier: a named range, `TRUE`/`FALSE`, or an unknown word.
    Ident,
    /// An identifier directly followed (whitespace allowed) by `(`.
    Func,
    /// A single-cell CalcA1 reference: `A1`, `$Sheet1.$A$1`, `'My Sheet'!A1`.
    CellRef,
    /// An adjacent CalcA1 range: `A1:B2`, `A:A`, `1:1`, `Sheet1.A1:Sheet2.B2`,
    /// or an ExcelA1 3D reference `Sheet1:Sheet3!A1`.
    RangeRef,
    /// An OpenFormula bracketed reference: `[.A1]`, `[$Sheet1.A1:.B2]`.
    OfRef,
    /// An error literal: `#REF!`, `#N/A`, `Err:522`, ...
    Error,
    /// `+`
    Plus,
    /// `-`
    Minus,
    /// `*`
    Star,
    /// `/`
    Slash,
    /// `^`
    Caret,
    /// `&`
    Amp,
    /// `=`
    Eq,
    /// `<>`
    Ne,
    /// `<`
    Lt,
    /// `<=`
    Le,
    /// `>`
    Gt,
    /// `>=`
    Ge,
    /// `%`
    Percent,
    /// `:` (range operator between non-adjacent operands)
    Colon,
    /// `~` (reference union)
    Tilde,
    /// `!` (reference intersection)
    Bang,
    /// `(`
    LParen,
    /// `)`
    RParen,
    /// `{`
    LBrace,
    /// `}`
    RBrace,
    /// `,`
    Comma,
    /// `;`
    Semicolon,
    /// `|` (array row separator)
    Pipe,
    /// Anything the lexer does not understand (unterminated strings, stray
    /// characters).
    Invalid,
}

impl TokenKind {
    /// Every kind, for fixture (de)serialisation.
    pub const ALL: [TokenKind; 34] = [
        TokenKind::Prefix,
        TokenKind::Whitespace,
        TokenKind::Number,
        TokenKind::String,
        TokenKind::Ident,
        TokenKind::Func,
        TokenKind::CellRef,
        TokenKind::RangeRef,
        TokenKind::OfRef,
        TokenKind::Error,
        TokenKind::Plus,
        TokenKind::Minus,
        TokenKind::Star,
        TokenKind::Slash,
        TokenKind::Caret,
        TokenKind::Amp,
        TokenKind::Eq,
        TokenKind::Ne,
        TokenKind::Lt,
        TokenKind::Le,
        TokenKind::Gt,
        TokenKind::Ge,
        TokenKind::Percent,
        TokenKind::Colon,
        TokenKind::Tilde,
        TokenKind::Bang,
        TokenKind::LParen,
        TokenKind::RParen,
        TokenKind::LBrace,
        TokenKind::RBrace,
        TokenKind::Comma,
        TokenKind::Semicolon,
        TokenKind::Pipe,
        TokenKind::Invalid,
    ];

    /// A stable name used by the shared JSON fixtures (`"CellRef"`, ...).
    pub const fn name(self) -> &'static str {
        match self {
            TokenKind::Prefix => "Prefix",
            TokenKind::Whitespace => "Whitespace",
            TokenKind::Number => "Number",
            TokenKind::String => "String",
            TokenKind::Ident => "Ident",
            TokenKind::Func => "Func",
            TokenKind::CellRef => "CellRef",
            TokenKind::RangeRef => "RangeRef",
            TokenKind::OfRef => "OfRef",
            TokenKind::Error => "Error",
            TokenKind::Plus => "Plus",
            TokenKind::Minus => "Minus",
            TokenKind::Star => "Star",
            TokenKind::Slash => "Slash",
            TokenKind::Caret => "Caret",
            TokenKind::Amp => "Amp",
            TokenKind::Eq => "Eq",
            TokenKind::Ne => "Ne",
            TokenKind::Lt => "Lt",
            TokenKind::Le => "Le",
            TokenKind::Gt => "Gt",
            TokenKind::Ge => "Ge",
            TokenKind::Percent => "Percent",
            TokenKind::Colon => "Colon",
            TokenKind::Tilde => "Tilde",
            TokenKind::Bang => "Bang",
            TokenKind::LParen => "LParen",
            TokenKind::RParen => "RParen",
            TokenKind::LBrace => "LBrace",
            TokenKind::RBrace => "RBrace",
            TokenKind::Comma => "Comma",
            TokenKind::Semicolon => "Semicolon",
            TokenKind::Pipe => "Pipe",
            TokenKind::Invalid => "Invalid",
        }
    }

    /// Inverse of [`TokenKind::name`].
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|k| k.name() == name)
    }

    /// Returns `true` for tokens the parser ignores.
    pub const fn is_trivia(self) -> bool {
        matches!(self, TokenKind::Whitespace | TokenKind::Prefix)
    }
}

/// A token: its kind and its byte span in the source.
#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub struct Token {
    /// What the token is.
    pub kind: TokenKind,
    /// UTF-8 byte range in the source.
    pub span: Range<usize>,
}

const ERROR_NAMES: [&str; 7] = [
    "#NULL!", "#DIV/0!", "#VALUE!", "#REF!", "#NAME?", "#NUM!", "#N/A",
];

fn char_at(s: &str, i: usize) -> Option<char> {
    s.get(i..).and_then(|t| t.chars().next())
}

/// Detects a formula prefix. Returns its length and whether it switches to
/// OpenFormula.
pub(crate) fn detect_prefix(src: &str) -> Option<(usize, bool)> {
    if src.starts_with("of:=") {
        Some((4, true))
    } else if src.starts_with("oooc:=") {
        Some((6, true))
    } else if src.starts_with('=') {
        Some((1, false))
    } else {
        None
    }
}

/// Tokenizes `src`. Returns the tokens and whether OpenFormula syntax was
/// used (the dialect is OpenFormula or the source starts with `of:=` /
/// `oooc:=`).
pub(crate) fn lex(src: &str, dialect: Dialect) -> (Vec<Token>, bool) {
    let mut out = Vec::with_capacity(src.len() / 2 + 1);
    let mut of = dialect.is_open_formula();
    let mut i = 0;
    if let Some((len, is_of)) = detect_prefix(src) {
        of |= is_of;
        out.push(Token {
            kind: TokenKind::Prefix,
            span: 0..len,
        });
        i = len;
    }
    let b = src.as_bytes();
    let excel = dialect.is_excel() && !of;
    while let Some(c) = char_at(src, i) {
        let (kind, end) = if excel {
            if let Some(r) = scan_excel_3d(src, i) {
                (TokenKind::RangeRef, r.end)
            } else if let Some(q) = scan_excel_qualified(src, i) {
                (q.kind, q.end)
            } else {
                lex_one(src, b, i, c, of)
            }
        } else {
            lex_one(src, b, i, c, of)
        };
        // Defensive: always make progress.
        let end = if end <= i { i + c.len_utf8() } else { end };
        out.push(Token { kind, span: i..end });
        i = end;
    }
    (out, of)
}

fn lex_one(src: &str, b: &[u8], i: usize, c: char, of: bool) -> (TokenKind, usize) {
    if c.is_whitespace() {
        let mut j = i;
        while let Some(w) = char_at(src, j) {
            if !w.is_whitespace() {
                break;
            }
            j += w.len_utf8();
        }
        return (TokenKind::Whitespace, j);
    }
    match c {
        '"' => return lex_string(src, i),
        '#' => {
            let rest = src.get(i..).unwrap_or("");
            for name in ERROR_NAMES {
                if rest.len() >= name.len()
                    && rest.is_char_boundary(name.len())
                    && rest[..name.len()].eq_ignore_ascii_case(name)
                {
                    return (TokenKind::Error, i + name.len());
                }
            }
            return (TokenKind::Invalid, i + 1);
        }
        '[' if of => {
            return match scan_bracket(src, i) {
                Some(end) => (TokenKind::OfRef, end),
                None => (TokenKind::Invalid, src.len()),
            };
        }
        _ => {}
    }
    if is_ident_start(c) {
        if let Some(end) = scan_err_literal(src, i) {
            return (TokenKind::Error, end);
        }
    }
    if !of && (c == '$' || c == '\'' || is_ident_start(c) || c.is_ascii_digit()) {
        if let Some((end, shape, _)) = scan_a1_ref(src, i) {
            let kind = match shape {
                RefShape::Cell => TokenKind::CellRef,
                RefShape::Range => TokenKind::RangeRef,
            };
            return (kind, end);
        }
    }
    if c == '\'' {
        // A quoted name that is not a sheet qualifier.
        return match crate::refsyntax::scan_quoted(src, i) {
            Some((end, _)) => (TokenKind::Invalid, end),
            None => (TokenKind::Invalid, src.len()),
        };
    }
    if c.is_ascii_digit() || (c == '.' && b.get(i + 1).is_some_and(u8::is_ascii_digit)) {
        return (TokenKind::Number, scan_number(b, i));
    }
    if is_ident_start(c) {
        let mut j = i + c.len_utf8();
        while let Some(w) = char_at(src, j) {
            if !is_ident_char(w) {
                break;
            }
            j += w.len_utf8();
        }
        let mut k = j;
        while let Some(w) = char_at(src, k) {
            if !w.is_whitespace() {
                break;
            }
            k += w.len_utf8();
        }
        let kind = if b.get(k) == Some(&b'(') {
            TokenKind::Func
        } else {
            TokenKind::Ident
        };
        return (kind, j);
    }
    let next = b.get(i + 1).copied();
    let (kind, len) = match c {
        '+' => (TokenKind::Plus, 1),
        '-' => (TokenKind::Minus, 1),
        '*' => (TokenKind::Star, 1),
        '/' => (TokenKind::Slash, 1),
        '^' => (TokenKind::Caret, 1),
        '&' => (TokenKind::Amp, 1),
        '=' => (TokenKind::Eq, 1),
        '<' => match next {
            Some(b'>') => (TokenKind::Ne, 2),
            Some(b'=') => (TokenKind::Le, 2),
            _ => (TokenKind::Lt, 1),
        },
        '>' => match next {
            Some(b'=') => (TokenKind::Ge, 2),
            _ => (TokenKind::Gt, 1),
        },
        '%' => (TokenKind::Percent, 1),
        ':' => (TokenKind::Colon, 1),
        '~' => (TokenKind::Tilde, 1),
        '!' => (TokenKind::Bang, 1),
        '(' => (TokenKind::LParen, 1),
        ')' => (TokenKind::RParen, 1),
        '{' => (TokenKind::LBrace, 1),
        '}' => (TokenKind::RBrace, 1),
        ',' => (TokenKind::Comma, 1),
        ';' => (TokenKind::Semicolon, 1),
        '|' => (TokenKind::Pipe, 1),
        other => (TokenKind::Invalid, other.len_utf8()),
    };
    (kind, i + len)
}

/// An ExcelA1 3D reference `Jan:Dec!B2`, `'Q 1:Q 4'!A1:B2`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Excel3d {
    /// Byte index after the reference.
    pub end: usize,
    /// First sheet name (unescaped).
    pub first: String,
    /// Last sheet name (unescaped).
    pub last: String,
    /// The cell or area part after the `!` (no sheet qualifiers).
    pub part: RawRef,
}

/// An ExcelA1 sheet-qualified token the CalcA1 scanner does not cover:
/// a sheet name with dots (`Sales.EU!A1`) or a deleted reference on a
/// sheet (`Sheet1!#REF!`, common in `definedName`s).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ExcelQualified {
    /// Token kind: `CellRef`, `RangeRef` or `Error` (for `#REF!`).
    pub kind: TokenKind,
    /// Byte index after the token.
    pub end: usize,
    /// The sheet name (unescaped).
    pub sheet: String,
    /// The reference part, `None` for `#REF!`.
    pub part: Option<RawRef>,
}

/// Scans an [`ExcelQualified`] token at `i`.
pub(crate) fn scan_excel_qualified(src: &str, i: usize) -> Option<ExcelQualified> {
    let b = src.as_bytes();
    let (after, sheet, dotted) = if b.get(i) == Some(&b'\'') {
        let (end, name) = scan_quoted(src, i)?;
        (end, name, false)
    } else {
        let end = scan_excel_sheet_name(src, i)?;
        let name = src.get(i..end)?;
        (end, name.to_string(), name.contains('.'))
    };
    if b.get(after) != Some(&b'!') {
        return None;
    }
    let start = after + 1;
    let rest = src.get(start..)?;
    if rest.len() >= 5 && rest.is_char_boundary(5) && rest[..5].eq_ignore_ascii_case("#REF!") {
        return Some(ExcelQualified {
            kind: TokenKind::Error,
            end: start + 5,
            sheet,
            part: None,
        });
    }
    if !dotted {
        return None;
    }
    let (end, shape, part) = scan_a1_ref(src, start)?;
    if part.start.sheet.is_some() || part.end.as_ref().is_some_and(|e| e.sheet.is_some()) {
        return None;
    }
    let kind = match shape {
        RefShape::Cell => TokenKind::CellRef,
        RefShape::Range => TokenKind::RangeRef,
    };
    Some(ExcelQualified {
        kind,
        end,
        sheet,
        part: Some(part),
    })
}

/// Scans an unquoted Excel sheet name at `i` (letters, digits, `_`, `.`
/// and non-ASCII alphanumerics, not starting with a digit).
fn scan_excel_sheet_name(src: &str, i: usize) -> Option<usize> {
    let first = char_at(src, i)?;
    if !is_ident_start(first) {
        return None;
    }
    let mut j = i + first.len_utf8();
    while let Some(c) = char_at(src, j) {
        if !(is_sheet_char(c) || c == '.') {
            break;
        }
        j += c.len_utf8();
    }
    Some(j)
}

/// Scans an ExcelA1 3D reference at `i`: `First:Last!A1`, `First:Last!A1:B2`
/// or the quoted form `'First sheet:Last sheet'!A1` (Excel sheet names can
/// never contain `:`). Unquoted names that look like cell references are not
/// sheet names (`A1:B2!C3` is not 3D).
pub(crate) fn scan_excel_3d(src: &str, i: usize) -> Option<Excel3d> {
    let b = src.as_bytes();
    let (after_names, first, last) = if b.get(i) == Some(&b'\'') {
        let (end, name) = scan_quoted(src, i)?;
        let (f, l) = name.split_once(':')?;
        if f.is_empty() || l.is_empty() || l.contains(':') {
            return None;
        }
        (end, f.to_string(), l.to_string())
    } else {
        let e1 = scan_excel_sheet_name(src, i)?;
        if b.get(e1) != Some(&b':') {
            return None;
        }
        let e2 = scan_excel_sheet_name(src, e1 + 1)?;
        let f = src.get(i..e1)?;
        let l = src.get(e1 + 1..e2)?;
        if tally_types::parse_a1(f).is_some() || tally_types::parse_a1(l).is_some() {
            return None;
        }
        (e2, f.to_string(), l.to_string())
    };
    if b.get(after_names) != Some(&b'!') {
        return None;
    }
    let start = after_names + 1;
    let (end, _, part) = scan_a1_ref(src, start)?;
    if part.start.sheet.is_some() || part.end.as_ref().is_some_and(|e| e.sheet.is_some()) {
        return None;
    }
    Some(Excel3d {
        end,
        first,
        last,
        part,
    })
}

fn lex_string(src: &str, i: usize) -> (TokenKind, usize) {
    let b = src.as_bytes();
    let mut j = i + 1;
    loop {
        let Some(q) = src.get(j..).and_then(|r| r.find('"')) else {
            return (TokenKind::Invalid, src.len());
        };
        let after = j + q + 1;
        if b.get(after) == Some(&b'"') {
            j = after + 1;
        } else {
            return (TokenKind::String, after);
        }
    }
}

/// `Err:NNN` (prefix matched ASCII case-insensitively).
fn scan_err_literal(src: &str, i: usize) -> Option<usize> {
    let rest = src.get(i..)?;
    if rest.len() < 5 || !rest.is_char_boundary(4) || !rest[..4].eq_ignore_ascii_case("err:") {
        return None;
    }
    let digits = rest[4..].bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    let end = i + 4 + digits;
    match char_at(src, end) {
        Some(c) if is_ident_char(c) => None,
        _ => Some(end),
    }
}

fn scan_number(b: &[u8], i: usize) -> usize {
    let mut j = i;
    while j < b.len() && b[j].is_ascii_digit() {
        j += 1;
    }
    if b.get(j) == Some(&b'.') {
        j += 1;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
    }
    if matches!(b.get(j), Some(b'e' | b'E')) {
        let mut k = j + 1;
        if matches!(b.get(k), Some(b'+' | b'-')) {
            k += 1;
        }
        if b.get(k).is_some_and(u8::is_ascii_digit) {
            while k < b.len() && b[k].is_ascii_digit() {
                k += 1;
            }
            j = k;
        }
    }
    j
}
