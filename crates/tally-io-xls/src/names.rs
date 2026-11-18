//! Defined names of a BIFF8 workbook: `NAME` records with the
//! `EXTERNSHEET` / `SUPBOOK` tables their 3-D references go through.
//!
//! The formatting pass collects the records ([`NameRecs::name`],
//! [`NameRecs::externsheet`], [`NameRecs::supbook`]); once the sheet names
//! are known, [`NameRecs::resolve`] turns each name into a
//! [`NameData`]. A name's formula is decoded from its parsed expression
//! (`rgce`, reverse Polish) into Excel text, which the XLSX reader's
//! defined-name conversion turns into the model's OpenFormula.
//!
//! Read are the formulas made of references to this workbook's sheets
//! (`ptgRef3d`, `ptgArea3d`), numbers, text, booleans, other names and the
//! operators (union, intersection and range included). A name whose
//! formula holds anything else (a function, an array, an error, a
//! reference to another workbook or to a deleted sheet, a reference
//! without a sheet) is left out and counted. Built-in names (print ranges,
//! filter ranges), function and macro names are not names of the model
//! (the print ranges are read by the formatting pass itself).
//!
//! Hostile input: every length is checked against the record, a name
//! keeps at most [`MAX_FORMULA_TEXT`] bytes of formula text, the names
//! kept and their bytes are bounded by [`Limits`], and the text moved
//! while decoding is bounded by [`WORK_BUDGET`] for the workbook, so
//! deeply nested expressions cannot make the pass quadratic.

use tally_model::NameData;
use tally_types::{MAX_COLS, MAX_ROWS};

use crate::prescan::{u16_at, u32_at};
use crate::Limits;

/// Longest Excel text of one name's formula.
const MAX_FORMULA_TEXT: usize = 8 * 1024;
/// Most bytes of text moved while decoding the names of a workbook.
const WORK_BUDGET: u64 = 64 * 1024 * 1024;
/// Most `SUPBOOK` records looked at.
const MAX_SUPBOOKS: usize = 65_536;
/// Most `EXTERNSHEET` entries looked at (the index is 16 bits wide).
const MAX_XTIS: usize = 65_536;

/// One `NAME` record.
#[derive(Debug, Default)]
struct RawName {
    /// The name; empty for a record that is not a name of the model (a
    /// built-in, a function, a macro, a truncated record). Such a record
    /// still takes its index.
    name: String,
    /// The sheet the name is local to (1-based); 0 = the workbook.
    itab: u16,
    /// The parsed expression; `None` when the record does not hold all of
    /// it.
    rgce: Option<Vec<u8>>,
}

/// An `EXTERNSHEET` entry: the `SUPBOOK` and the sheets it stands for.
#[derive(Clone, Copy, Debug)]
struct Xti {
    book: u16,
    first: u16,
    last: u16,
}

/// The name records of a workbook.
#[derive(Debug, Default)]
pub(crate) struct NameRecs {
    names: Vec<RawName>,
    /// Bytes of names and expressions kept.
    bytes: u64,
    xtis: Vec<Xti>,
    /// Per `SUPBOOK`: it is the workbook itself.
    internal: Vec<bool>,
    /// A limit cut the list.
    pub dropped: bool,
}

impl NameRecs {
    /// Notes a `SUPBOOK` record: `ctab`, then 0x0401 for the workbook
    /// itself.
    pub fn supbook(&mut self, d: &[u8]) {
        if self.internal.len() < MAX_SUPBOOKS {
            self.internal
                .push(d.len() == 4 && u16_at(d, 2) == Some(0x0401));
        }
    }

    /// Reads an `EXTERNSHEET` record (with its continues in `d`): a count,
    /// then (SUPBOOK, first sheet, last sheet) each. A later record
    /// replaces an earlier one.
    pub fn externsheet(&mut self, d: &[u8]) {
        let n = usize::from(u16_at(d, 0).unwrap_or(0))
            .min(d.len().saturating_sub(2) / 6)
            .min(MAX_XTIS);
        self.xtis.clear();
        for k in 0..n {
            let at = 2 + 6 * k;
            if let (Some(book), Some(first), Some(last)) =
                (u16_at(d, at), u16_at(d, at + 2), u16_at(d, at + 4))
            {
                self.xtis.push(Xti { book, first, last });
            }
        }
    }

    /// Notes a `NAME` record: flags, shortcut key, the name's length, the
    /// expression's length, a reserved word, the sheet, four lengths of
    /// texts Excel no longer writes, the name, the expression.
    pub fn name(&mut self, d: &[u8], limits: &Limits) {
        if self.names.len() >= limits.max_names {
            self.dropped = true;
            return;
        }
        let raw = raw_name(d).unwrap_or_default();
        let bytes = (raw.name.len() + raw.rgce.as_ref().map_or(0, Vec::len)) as u64;
        if self.bytes.saturating_add(bytes) > limits.max_name_bytes {
            // The record keeps its index (other names may refer to it).
            self.dropped = true;
            self.names.push(RawName::default());
            return;
        }
        self.bytes += bytes;
        self.names.push(raw);
    }

    /// The names of the model, in file order, and how many were left out
    /// because their formula is not one this reader decodes. `sheets` are
    /// the workbook's sheet names in `BOUNDSHEET` order.
    pub fn resolve(&self, sheets: &[String]) -> (Vec<NameData>, usize) {
        let mut out = Vec::new();
        let mut skipped = 0usize;
        let mut work = WORK_BUDGET;
        let converter = tally_io_xlsx::ExcelNameConverter::new(sheets);
        for raw in &self.names {
            if raw.name.is_empty() {
                continue;
            }
            let sheet = match raw.itab {
                0 => None,
                i if usize::from(i) <= sheets.len() => Some(u32::from(i) - 1),
                // Local to a sheet that is not there.
                _ => {
                    skipped += 1;
                    continue;
                }
            };
            let formula = raw
                .rgce
                .as_deref()
                .and_then(|rgce| self.excel_text(rgce, sheets, &mut work))
                .and_then(|text| converter.to_of(&text, sheet.map(|s| s as usize)));
            match formula {
                Some(formula) => out.push(NameData {
                    name: raw.name.clone(),
                    sheet,
                    formula,
                    base_cell: None,
                    usage: Default::default(),
                }),
                None => skipped += 1,
            }
        }
        (out, skipped)
    }

    /// The sheet of an `EXTERNSHEET` index, quoted for Excel text: one
    /// sheet of this workbook.
    fn sheet_ref(&self, ixti: u16, sheets: &[String]) -> Option<String> {
        let x = self.xtis.get(usize::from(ixti))?;
        if self.internal.get(usize::from(x.book)) != Some(&true) || x.first != x.last {
            return None;
        }
        let name = sheets.get(usize::from(x.first))?;
        // A control character cannot be written in a formula.
        if name.is_empty() || name.chars().any(char::is_control) {
            return None;
        }
        Some(format!("'{}'!", name.replace('\'', "''")))
    }

    /// Decodes the reverse Polish expression `rgce` into Excel formula
    /// text; `None` for an expression with a token this reader does not
    /// decode, a malformed one, or one past the text or work limits.
    fn excel_text(&self, rgce: &[u8], sheets: &[String], work: &mut u64) -> Option<String> {
        let mut stack: Vec<String> = Vec::new();
        // Charges `n` bytes of moved text to the workbook's budget.
        // Once it is spent it stays spent.
        let mut charge = |n: usize| -> Option<()> {
            let left = work.checked_sub(n as u64);
            *work = left.unwrap_or(0);
            left.map(|_| ())
        };
        let mut p = 0usize;
        while let Some(&ptg) = rgce.get(p) {
            // Operand tokens come in three classes (reference, value,
            // array) that only differ in bits 5 and 6.
            let id = if ptg >= 0x20 {
                (ptg & 0x1F) | 0x20
            } else {
                ptg
            };
            let binary = |op: &str, stack: &mut Vec<String>| -> Option<usize> {
                let b = stack.pop()?;
                let a = stack.last_mut()?;
                a.push_str(op);
                a.push_str(&b);
                Some(b.len() + op.len())
            };
            let step = match id {
                0x03..=0x0F | 0x11 => {
                    let op = match id {
                        0x03 => "+",
                        0x04 => "-",
                        0x05 => "*",
                        0x06 => "/",
                        0x07 => "^",
                        0x08 => "&",
                        0x09 => "<",
                        0x0A => "<=",
                        0x0B => "=",
                        0x0C => ">=",
                        0x0D => ">",
                        0x0E => "<>",
                        0x0F => " ",
                        _ => ":",
                    };
                    charge(binary(op, &mut stack)?)?;
                    1
                }
                // Union: Excel text wants it in parentheses.
                0x10 => {
                    charge(binary(",", &mut stack)?)?;
                    let a = stack.last_mut()?;
                    charge(a.len())?;
                    a.insert(0, '(');
                    a.push(')');
                    1
                }
                // Unary plus and minus, percent, parentheses.
                0x12..=0x15 => {
                    let a = stack.last_mut()?;
                    charge(a.len())?;
                    match id {
                        0x12 => a.insert(0, '+'),
                        0x13 => a.insert(0, '-'),
                        0x14 => a.push('%'),
                        _ => {
                            a.insert(0, '(');
                            a.push(')');
                        }
                    }
                    1
                }
                // Text: a length, option flags, the characters.
                0x17 => {
                    let cch = usize::from(*rgce.get(p + 1)?);
                    let wide = rgce.get(p + 2)? & 1 != 0;
                    let len = if wide { 2 * cch } else { cch };
                    let chars = rgce.get(p + 3..(p + 3).checked_add(len)?)?;
                    let text: String = if wide {
                        let units: Vec<u16> = chars
                            .chunks_exact(2)
                            .map(|b| u16::from_le_bytes([b[0], b[1]]))
                            .collect();
                        String::from_utf16(&units).ok()?
                    } else {
                        chars.iter().map(|b| char::from(*b)).collect()
                    };
                    if text.chars().any(char::is_control) {
                        return None;
                    }
                    stack.push(format!("\"{}\"", text.replace('"', "\"\"")));
                    3 + len
                }
                // Attributes: spaces and "volatile" change nothing; the
                // others belong to functions.
                0x19 => match *rgce.get(p + 1)? {
                    0x01 | 0x40 | 0x41 => 4,
                    _ => return None,
                },
                0x1D => {
                    stack.push(
                        if *rgce.get(p + 1)? != 0 {
                            "TRUE"
                        } else {
                            "FALSE"
                        }
                        .to_string(),
                    );
                    2
                }
                0x1E => {
                    stack.push(u16_at(rgce, p + 1)?.to_string());
                    3
                }
                0x1F => {
                    let b = rgce.get(p + 1..p + 9)?;
                    let v = f64::from_le_bytes(b.try_into().ok()?);
                    if !v.is_finite() {
                        return None;
                    }
                    // Exponent form keeps a huge or tiny number short.
                    stack.push(if v != 0.0 && !(1e-5..1e15).contains(&v.abs()) {
                        format!("{v:E}")
                    } else {
                        v.to_string()
                    });
                    9
                }
                // Another name, by its 1-based index.
                0x23 => {
                    let i = usize::try_from(u32_at(rgce, p + 1)?).ok()?;
                    let name = &self.names.get(i.checked_sub(1)?)?.name;
                    if !is_plain_name(name) {
                        return None;
                    }
                    stack.push(name.clone());
                    5
                }
                // ptgMemArea, ptgMemErr, ptgMemNoMem and ptgMemFunc only
                // announce the reference expression that follows.
                0x26..=0x28 => 7,
                0x29 => 3,
                // ptgRef3d: sheet, row, column.
                0x3A => {
                    let sheet = self.sheet_ref(u16_at(rgce, p + 1)?, sheets)?;
                    let (row, col) = (u16_at(rgce, p + 3)?, u16_at(rgce, p + 5)?);
                    let mut t = sheet;
                    push_cell(&mut t, u32::from(row), col);
                    stack.push(t);
                    7
                }
                // ptgArea3d: sheet, rows, columns.
                0x3B => {
                    let sheet = self.sheet_ref(u16_at(rgce, p + 1)?, sheets)?;
                    let (r0, r1) = (u16_at(rgce, p + 3)?, u16_at(rgce, p + 5)?);
                    let (c0, c1) = (u16_at(rgce, p + 7)?, u16_at(rgce, p + 9)?);
                    let mut t = sheet;
                    push_area(&mut t, (r0, r1), (c0, c1));
                    stack.push(t);
                    11
                }
                _ => return None,
            };
            if stack.last().is_some_and(|t| t.len() > MAX_FORMULA_TEXT) {
                return None;
            }
            p += step;
        }
        // Exactly the declared bytes, and one result.
        let text = stack.pop()?;
        (p == rgce.len() && stack.is_empty()).then_some(text)
    }
}

/// Whether `name` can be written into a formula as it is: letters,
/// digits, `_`, `.`, `\` and `?`, not starting with a digit.
fn is_plain_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with(|c: char| c.is_ascii_digit())
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | '\\' | '?'))
}

/// The fields of a `NAME` record; `None` when it is too short for them.
fn raw_name(d: &[u8]) -> Option<RawName> {
    let flags = u16_at(d, 0)?;
    let cch = usize::from(*d.get(3)?);
    let cce = usize::from(u16_at(d, 4)?);
    let itab = u16_at(d, 8)?;
    // Function, Visual Basic, macro, built-in and "big" names.
    if flags & (0x0002 | 0x0004 | 0x0008 | 0x0020 | 0x1000) != 0 || cch == 0 {
        return None;
    }
    let wide = d.get(14)? & 1 != 0;
    let len = if wide { 2 * cch } else { cch };
    let chars = d.get(15..15 + len)?;
    let name: String = if wide {
        let units: Vec<u16> = chars
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        chars.iter().map(|b| char::from(*b)).collect()
    };
    if name.chars().any(char::is_control) {
        return None;
    }
    let at = 15 + len;
    let rgce = (cce > 0)
        .then(|| d.get(at..at + cce))
        .flatten()
        .map(<[u8]>::to_vec);
    Some(RawName { name, itab, rgce })
}

/// Appends a column's letters.
fn push_col(t: &mut String, col: u32) {
    let mut letters = [0u8; 8];
    let mut n = 0;
    let mut c = col + 1;
    while c > 0 && n < letters.len() {
        letters[n] = b'A' + ((c - 1) % 26) as u8;
        c = (c - 1) / 26;
        n += 1;
    }
    t.extend(letters[..n].iter().rev().map(|b| char::from(*b)));
}

/// Appends a cell. The column word's bits 14 and 15 say "relative"
/// (column, row); a name's relative references are stored as the cell
/// they mean for a formula in A1, which is the model's base cell.
fn push_cell(t: &mut String, row: u32, col: u16) {
    if col & 0x4000 == 0 {
        t.push('$');
    }
    push_col(t, u32::from(col & 0xFF));
    if col & 0x8000 == 0 {
        t.push('$');
    }
    t.push_str(&(row + 1).to_string());
}

/// Appends an area. An area over all 65,536 rows or all 256 columns of
/// the BIFF8 sheet is a whole column or row of the model's larger sheet,
/// as in OpenOffice (`ExcelToSc8`: `SetComplCol`, `SetComplRow`).
fn push_area(t: &mut String, (r0, r1): (u16, u16), (c0, c1): (u16, u16)) {
    let (mut row0, mut row1) = (u32::from(r0), u32::from(r1));
    let (mut col0, mut col1) = (u32::from(c0 & 0xFF), u32::from(c1 & 0xFF));
    if row0 == 0 && row1 == 0xFFFF {
        row1 = MAX_ROWS - 1;
    } else if col0 == 0 && col1 == 0xFF {
        col1 = MAX_COLS - 1;
    }
    if row0 > row1 {
        std::mem::swap(&mut row0, &mut row1);
    }
    if col0 > col1 {
        std::mem::swap(&mut col0, &mut col1);
    }
    let cell = |t: &mut String, row: u32, col: u32, flags: u16| {
        if flags & 0x4000 == 0 {
            t.push('$');
        }
        push_col(t, col);
        if flags & 0x8000 == 0 {
            t.push('$');
        }
        t.push_str(&(row + 1).to_string());
    };
    cell(t, row0, col0, c0);
    t.push(':');
    cell(t, row1, col1, c1);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recs() -> NameRecs {
        let mut n = NameRecs::default();
        n.supbook(&[2, 0, 1, 4]);
        // Entry 0: sheet 0; entry 1: sheet 1; entry 2: sheets 0–1; entry
        // 3: another workbook; entry 4: a deleted sheet.
        let mut d = vec![5, 0];
        for (book, first, last) in [
            (0u16, 0u16, 0u16),
            (0, 1, 1),
            (0, 0, 1),
            (1, 0, 0),
            (0, 0xFFFF, 0xFFFF),
        ] {
            for w in [book, first, last] {
                d.extend_from_slice(&w.to_le_bytes());
            }
        }
        n.externsheet(&d);
        n
    }

    fn sheets() -> Vec<String> {
        vec!["One".to_string(), "It's two".to_string()]
    }

    fn text(n: &NameRecs, rgce: &[u8]) -> Option<String> {
        n.excel_text(rgce, &sheets(), &mut WORK_BUDGET.clone())
    }

    fn area(ixti: u16, r0: u16, r1: u16, c0: u16, c1: u16) -> Vec<u8> {
        let mut d = vec![0x3B];
        for w in [ixti, r0, r1, c0, c1] {
            d.extend_from_slice(&w.to_le_bytes());
        }
        d
    }

    #[test]
    fn references_numbers_text_and_operators() {
        let n = recs();
        assert_eq!(
            text(&n, &area(0, 0, 9, 0, 1)).as_deref(),
            Some("'One'!$A$1:$B$10")
        );
        // Relative column and row, a quoted sheet name.
        assert_eq!(
            text(&n, &[0x3A, 1, 0, 4, 0, 2, 0xC0]).as_deref(),
            Some("'It''s two'!C5")
        );
        // Whole columns and whole rows.
        assert_eq!(
            text(&n, &area(0, 0, 0xFFFF, 1, 2)).as_deref(),
            Some("'One'!$B$1:$C$1048576")
        );
        assert_eq!(
            text(&n, &area(0, 2, 3, 0, 255)).as_deref(),
            Some("'One'!$A$3:$XFD$4")
        );
        // 2 * 0.5 + "a""b" in parentheses, negated: RPN.
        let mut rgce = vec![0x1E, 2, 0, 0x1F];
        rgce.extend_from_slice(&0.5f64.to_le_bytes());
        rgce.extend_from_slice(&[0x05, 0x17, 3, 0, b'a', b'"', b'b', 0x08, 0x15, 0x13]);
        assert_eq!(text(&n, &rgce).as_deref(), Some("-(2*0.5&\"a\"\"b\")"));
        // A union under ptgMemFunc.
        let (a, b) = (area(0, 0, 0, 0, 0), area(1, 1, 1, 1, 1));
        let mut rgce = vec![0x29, (a.len() + b.len() + 1) as u8, 0];
        rgce.extend_from_slice(&a);
        rgce.extend_from_slice(&b);
        rgce.push(0x10);
        assert_eq!(
            text(&n, &rgce).as_deref(),
            Some("('One'!$A$1:$A$1,'It''s two'!$B$2:$B$2)")
        );
        assert_eq!(text(&n, &[0x1D, 1]).as_deref(), Some("TRUE"));
        let mut big = vec![0x1F];
        big.extend_from_slice(&1e300f64.to_le_bytes());
        assert_eq!(text(&n, &big).as_deref(), Some("1E300"));
    }

    #[test]
    fn what_is_not_decoded_is_none() {
        let n = recs();
        // A 3-D span, another workbook, a deleted sheet, an entry that is
        // not there.
        for ixti in [2u16, 3, 4, 5, 0xFFFF] {
            assert_eq!(text(&n, &area(ixti, 0, 0, 0, 0)), None, "ixti {ixti}");
        }
        // A function, an error, a 2-D reference, an array, a missing
        // argument, `IF`'s attribute.
        for rgce in [
            &[0x21u8, 10, 0][..],
            &[0x1C, 7],
            &[0x24, 0, 0, 0, 0],
            &[0x20, 0, 0, 0, 0, 0, 0, 0],
            &[0x16],
            &[0x19, 0x02, 0, 0],
        ] {
            assert_eq!(text(&n, rgce), None, "{rgce:?}");
        }
        // Operators without operands, two results, nothing.
        assert_eq!(text(&n, &[0x03]), None);
        assert_eq!(text(&n, &[0x1E, 1, 0, 0x03]), None);
        assert_eq!(text(&n, &[0x1E, 1, 0, 0x1E, 2, 0]), None);
        assert_eq!(text(&n, &[]), None);
        // NaN, a control character in text, a name index without a name.
        let mut nan = vec![0x1F];
        nan.extend_from_slice(&f64::NAN.to_le_bytes());
        assert_eq!(text(&n, &nan), None);
        assert_eq!(text(&n, &[0x17, 1, 0, 7]), None);
        assert_eq!(text(&n, &[0x23, 0, 0, 0, 0]), None);
        assert_eq!(text(&n, &[0x23, 9, 0, 0, 0]), None);
    }

    #[test]
    fn every_cut_and_every_byte() {
        let n = recs();
        let mut full = area(0, 0, 9, 0, 1);
        full.extend_from_slice(&[0x17, 2, 1, b'a', 0, b'b', 0, 0x08, 0x1E, 1, 0, 0x03, 0x15]);
        assert!(text(&n, &full).is_some());
        for cut in 0..full.len() {
            let _ = text(&n, &full[..cut]);
        }
        for b in 0..=255u8 {
            let _ = text(&n, &[b]);
            let _ = text(&n, &[b, b, b, b, b, b, b, b, b, b, b, b]);
            let mut after = area(0, 0, 0, 0, 0);
            after.push(b);
            let _ = text(&n, &after);
            for at in 0..full.len() {
                let mut m = full.clone();
                m[at] = b;
                let _ = text(&n, &m);
            }
        }
    }

    #[test]
    fn nesting_is_bounded_by_the_text_and_work_limits() {
        let n = recs();
        // 1, then 20,000 × "(…)": the text limit ends it.
        let mut deep = vec![0x1E, 1, 0];
        deep.extend(std::iter::repeat_n(0x15, 20_000));
        assert_eq!(text(&n, &deep), None);
        // Within the text limit, the work is charged: a small budget ends
        // the decoding, and what was charged never exceeds the budget.
        let mut parens = vec![0x1E, 1, 0];
        parens.extend(std::iter::repeat_n(0x15, 1_000));
        assert!(text(&n, &parens).is_some());
        let mut work = 10_000u64;
        assert_eq!(n.excel_text(&parens, &sheets(), &mut work), None);
        assert!(work <= 10_000);
    }

    #[test]
    fn name_records_cut_anywhere() {
        // "Rate" (8-bit) = 5, workbook scope.
        let mut d = vec![0, 0, 0, 4, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        d.extend_from_slice(b"Rate");
        d.extend_from_slice(&[0x1E, 5, 0]);
        let full = raw_name(&d).unwrap();
        assert_eq!((full.name.as_str(), full.itab), ("Rate", 0));
        assert_eq!(full.rgce.as_deref(), Some(&[0x1E, 5, 0][..]));
        for cut in 0..d.len() {
            if let Some(r) = raw_name(&d[..cut]) {
                // The name is whole; the expression is whole or absent.
                assert_eq!(r.name, "Rate");
                assert_eq!(r.rgce, None);
            }
        }
        // A formula length beyond the record.
        let mut long = d.clone();
        long[4] = 200;
        assert_eq!(raw_name(&long).unwrap().rgce, None);
        // Built-in, function, macro and Visual Basic names; no name.
        for flags in [0x20u8, 0x02, 0x04, 0x08] {
            let mut m = d.clone();
            m[0] = flags;
            assert!(raw_name(&m).is_none());
        }
        let mut empty = d.clone();
        empty[3] = 0;
        assert!(raw_name(&empty).is_none());
    }

    #[test]
    fn plain_names() {
        assert!(is_plain_name("Rate_1"));
        assert!(is_plain_name("Größe"));
        assert!(!is_plain_name("1st"));
        assert!(!is_plain_name("a b"));
        assert!(!is_plain_name("a+b"));
        assert!(!is_plain_name(""));
    }
}
