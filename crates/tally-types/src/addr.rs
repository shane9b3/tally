//! Cell addressing: sheet ids, cell addresses, inclusive ranges and A1 helpers.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Stable sheet identifier.
///
/// Ids are never reused and are independent of tab order, so formulas that
/// reference a sheet by id survive renames and reordering.
pub type SheetId = u32;

/// Number of rows in a sheet (1,048,576, matching XLSX and modern LibreOffice).
pub const MAX_ROWS: u32 = 1_048_576;

/// Number of columns in a sheet (16,384, i.e. `A`..=`XFD`).
pub const MAX_COLS: u32 = 16_384;

/// A single cell: 0-based `row` and `col` on `sheet`.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct CellAddr {
    /// Sheet the cell lives on.
    pub sheet: SheetId,
    /// 0-based row index.
    pub row: u32,
    /// 0-based column index.
    pub col: u32,
}

impl CellAddr {
    /// Creates an address.
    pub const fn new(sheet: SheetId, row: u32, col: u32) -> Self {
        Self { sheet, row, col }
    }

    /// Returns `true` if the address lies inside the grid limits.
    pub const fn is_valid(&self) -> bool {
        self.row < MAX_ROWS && self.col < MAX_COLS
    }

    /// Formats the address as relative A1 notation without the sheet, e.g. `B3`.
    pub fn to_a1(&self) -> String {
        format!("{}{}", col_to_letters(self.col), u64::from(self.row) + 1)
    }

    /// Parses A1 notation (`B3`, `$B$3`, `b3`) into an address on `sheet`.
    ///
    /// Absolute markers are accepted and ignored.
    pub fn from_a1(sheet: SheetId, s: &str) -> Option<Self> {
        parse_a1(s).map(|(row, col, _, _)| Self { sheet, row, col })
    }
}

/// An inclusive rectangular range `r0..=r1` × `c0..=c1` on one sheet.
///
/// Constructors normalise the corners so that `r0 <= r1` and `c0 <= c1`.
/// Code that builds the struct literally must uphold that invariant.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct CellRange {
    /// Sheet the range lives on.
    pub sheet: SheetId,
    /// First row (inclusive).
    pub r0: u32,
    /// First column (inclusive).
    pub c0: u32,
    /// Last row (inclusive).
    pub r1: u32,
    /// Last column (inclusive).
    pub c1: u32,
}

impl CellRange {
    /// Creates a range from two corners, normalising their order.
    pub fn new(sheet: SheetId, r0: u32, c0: u32, r1: u32, c1: u32) -> Self {
        Self {
            sheet,
            r0: r0.min(r1),
            c0: c0.min(c1),
            r1: r0.max(r1),
            c1: c0.max(c1),
        }
    }

    /// A range covering exactly one cell.
    pub const fn single(addr: CellAddr) -> Self {
        Self {
            sheet: addr.sheet,
            r0: addr.row,
            c0: addr.col,
            r1: addr.row,
            c1: addr.col,
        }
    }

    /// A range spanning two cells on the same sheet (the sheet of `a` is used).
    pub fn from_corners(a: CellAddr, b: CellAddr) -> Self {
        Self::new(a.sheet, a.row, a.col, b.row, b.col)
    }

    /// Top-left cell.
    pub const fn start(&self) -> CellAddr {
        CellAddr::new(self.sheet, self.r0, self.c0)
    }

    /// Bottom-right cell.
    pub const fn end(&self) -> CellAddr {
        CellAddr::new(self.sheet, self.r1, self.c1)
    }

    /// Number of rows.
    pub const fn rows(&self) -> u32 {
        self.r1 - self.r0 + 1
    }

    /// Number of columns.
    pub const fn cols(&self) -> u32 {
        self.c1 - self.c0 + 1
    }

    /// Number of cells (as `u64`; a whole sheet does not fit in `u32`).
    pub const fn cell_count(&self) -> u64 {
        self.rows() as u64 * self.cols() as u64
    }

    /// Returns `true` if the range is a single cell.
    pub const fn is_single(&self) -> bool {
        self.r0 == self.r1 && self.c0 == self.c1
    }

    /// Returns `true` if `addr` is inside the range (same sheet).
    pub const fn contains(&self, addr: CellAddr) -> bool {
        addr.sheet == self.sheet
            && addr.row >= self.r0
            && addr.row <= self.r1
            && addr.col >= self.c0
            && addr.col <= self.c1
    }

    /// Returns `true` if `other` lies entirely inside `self`.
    pub const fn contains_range(&self, other: &CellRange) -> bool {
        other.sheet == self.sheet
            && other.r0 >= self.r0
            && other.r1 <= self.r1
            && other.c0 >= self.c0
            && other.c1 <= self.c1
    }

    /// Returns `true` if the two ranges share at least one cell.
    pub const fn intersects(&self, other: &CellRange) -> bool {
        self.sheet == other.sheet
            && self.r0 <= other.r1
            && other.r0 <= self.r1
            && self.c0 <= other.c1
            && other.c0 <= self.c1
    }

    /// The overlapping part of two ranges, if any.
    pub fn intersect(&self, other: &CellRange) -> Option<CellRange> {
        if !self.intersects(other) {
            return None;
        }
        Some(CellRange {
            sheet: self.sheet,
            r0: self.r0.max(other.r0),
            c0: self.c0.max(other.c0),
            r1: self.r1.min(other.r1),
            c1: self.c1.min(other.c1),
        })
    }

    /// Iterates every cell in row-major order.
    ///
    /// This materialises addresses one by one; never use it to expand large
    /// ranges in the engine's hot paths.
    pub fn cells(&self) -> CellRangeIter {
        CellRangeIter {
            range: *self,
            next: Some((self.r0, self.c0)),
        }
    }

    /// Formats as A1 notation without the sheet: `A1` for a single cell,
    /// otherwise `A1:C5`.
    pub fn to_a1(&self) -> String {
        if self.is_single() {
            self.start().to_a1()
        } else {
            format!("{}:{}", self.start().to_a1(), self.end().to_a1())
        }
    }

    /// Parses `A1` or `A1:C5` (absolute markers allowed and ignored) into a
    /// range on `sheet`. Corners are normalised.
    pub fn from_a1(sheet: SheetId, s: &str) -> Option<Self> {
        match s.split_once(':') {
            None => CellAddr::from_a1(sheet, s).map(Self::single),
            Some((a, b)) => {
                let a = CellAddr::from_a1(sheet, a)?;
                let b = CellAddr::from_a1(sheet, b)?;
                Some(Self::from_corners(a, b))
            }
        }
    }
}

impl From<CellAddr> for CellRange {
    fn from(addr: CellAddr) -> Self {
        Self::single(addr)
    }
}

/// Row-major iterator over the cells of a [`CellRange`].
#[derive(Clone, Debug)]
pub struct CellRangeIter {
    range: CellRange,
    next: Option<(u32, u32)>,
}

impl Iterator for CellRangeIter {
    type Item = CellAddr;

    fn next(&mut self) -> Option<CellAddr> {
        let (row, col) = self.next?;
        self.next = if col < self.range.c1 {
            Some((row, col + 1))
        } else if row < self.range.r1 {
            Some((row + 1, self.range.c0))
        } else {
            None
        };
        Some(CellAddr::new(self.range.sheet, row, col))
    }
}

/// Converts a 0-based column index to letters: `0 → "A"`, `25 → "Z"`,
/// `26 → "AA"`, `16383 → "XFD"`.
///
/// Works for any `u32`, including indices beyond [`MAX_COLS`].
pub fn col_to_letters(col: u32) -> String {
    // Bijective base-26. u32::MAX needs 7 letters.
    let mut buf = [0u8; 8];
    let mut i = buf.len();
    let mut n = u64::from(col) + 1;
    while n > 0 {
        let rem = ((n - 1) % 26) as u8;
        i -= 1;
        buf[i] = b'A' + rem;
        n = (n - 1) / 26;
    }
    buf[i..].iter().map(|&b| char::from(b)).collect()
}

/// Converts column letters (case-insensitive) to a 0-based index:
/// `"A" → 0`, `"xfd" → 16383`.
///
/// Returns `None` for an empty string, non-ASCII-letter input, or a column
/// at or beyond [`MAX_COLS`].
pub fn letters_to_col(s: &str) -> Option<u32> {
    if s.is_empty() || s.len() > 3 {
        return None;
    }
    let mut n: u32 = 0;
    for b in s.bytes() {
        if !b.is_ascii_alphabetic() {
            return None;
        }
        n = n * 26 + u32::from(b.to_ascii_uppercase() - b'A') + 1;
    }
    let col = n - 1;
    (col < MAX_COLS).then_some(col)
}

/// Parses an A1 cell reference without a sheet name.
///
/// Returns `(row, col, abs_row, abs_col)` with 0-based `row` and `col`.
/// Accepts lowercase letters and `$` markers before the column and/or the
/// row (`A1`, `$A1`, `A$1`, `$A$1`). Returns `None` for anything outside the
/// grid (`XFE1`, `A0`, `A1048577`) or malformed input (whitespace, missing
/// parts, trailing characters).
pub fn parse_a1(s: &str) -> Option<(u32, u32, bool, bool)> {
    let bytes = s.as_bytes();
    let mut i = 0;

    let abs_col = bytes.first() == Some(&b'$');
    if abs_col {
        i += 1;
    }
    let col_start = i;
    while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
        i += 1;
    }
    let col = letters_to_col(s.get(col_start..i)?)?;

    let abs_row = bytes.get(i) == Some(&b'$');
    if abs_row {
        i += 1;
    }
    let digits = s.get(i..)?;
    if digits.is_empty() || digits.len() > 7 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let row_1based: u32 = digits.parse().ok()?;
    if row_1based == 0 || row_1based > MAX_ROWS {
        return None;
    }
    Some((row_1based - 1, col, abs_row, abs_col))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn col_letters_known_values() {
        assert_eq!(col_to_letters(0), "A");
        assert_eq!(col_to_letters(25), "Z");
        assert_eq!(col_to_letters(26), "AA");
        assert_eq!(col_to_letters(51), "AZ");
        assert_eq!(col_to_letters(52), "BA");
        assert_eq!(col_to_letters(701), "ZZ");
        assert_eq!(col_to_letters(702), "AAA");
        assert_eq!(col_to_letters(MAX_COLS - 1), "XFD");
        assert_eq!(col_to_letters(u32::MAX), "MWLQKWV");
    }

    #[test]
    fn col_letters_round_trip_whole_grid() {
        for col in 0..MAX_COLS {
            let letters = col_to_letters(col);
            assert_eq!(letters_to_col(&letters), Some(col), "{letters}");
            assert_eq!(letters_to_col(&letters.to_lowercase()), Some(col));
        }
    }

    #[test]
    fn letters_to_col_rejects_bad_input() {
        assert_eq!(letters_to_col(""), None);
        assert_eq!(letters_to_col("XFE"), None);
        assert_eq!(letters_to_col("ZZZZ"), None);
        assert_eq!(letters_to_col("A1"), None);
        assert_eq!(letters_to_col("$A"), None);
        assert_eq!(letters_to_col("Ä"), None);
    }

    #[test]
    fn parse_a1_valid() {
        assert_eq!(parse_a1("A1"), Some((0, 0, false, false)));
        assert_eq!(parse_a1("a1"), Some((0, 0, false, false)));
        assert_eq!(parse_a1("$A$1"), Some((0, 0, true, true)));
        assert_eq!(parse_a1("$A1"), Some((0, 0, false, true)));
        assert_eq!(parse_a1("A$1"), Some((0, 0, true, false)));
        assert_eq!(parse_a1("AA10"), Some((9, 26, false, false)));
        assert_eq!(
            parse_a1("XFD1048576"),
            Some((MAX_ROWS - 1, MAX_COLS - 1, false, false))
        );
    }

    #[test]
    fn parse_a1_invalid() {
        for s in [
            "",
            "A",
            "1",
            "$",
            "$$A1",
            "A$$1",
            "XFE1",
            "A0",
            "A1048577",
            "A99999999999",
            " A1",
            "A1 ",
            "A-1",
            "A1B",
            "1A",
            "A+1",
            "$1",
        ] {
            assert_eq!(parse_a1(s), None, "{s:?}");
        }
    }

    #[test]
    fn addr_a1_round_trip() {
        let a = CellAddr::new(3, 9, 26);
        assert!(a.is_valid());
        assert_eq!(a.to_a1(), "AA10");
        assert_eq!(CellAddr::from_a1(3, "AA10"), Some(a));
        assert_eq!(CellAddr::from_a1(3, "$AA$10"), Some(a));
        assert!(!CellAddr::new(0, MAX_ROWS, 0).is_valid());
    }

    #[test]
    fn range_new_normalises_and_measures() {
        let r = CellRange::new(1, 5, 3, 2, 1);
        assert_eq!((r.r0, r.c0, r.r1, r.c1), (2, 1, 5, 3));
        assert_eq!(r.rows(), 4);
        assert_eq!(r.cols(), 3);
        assert_eq!(r.cell_count(), 12);
        assert!(!r.is_single());
        assert_eq!(r.start(), CellAddr::new(1, 2, 1));
        assert_eq!(r.end(), CellAddr::new(1, 5, 3));
        let whole = CellRange::new(0, 0, 0, MAX_ROWS - 1, MAX_COLS - 1);
        assert_eq!(whole.cell_count(), 17_179_869_184);
    }

    #[test]
    fn range_contains() {
        let r = CellRange::from_a1(0, "B2:C3").unwrap();
        assert!(r.contains(CellAddr::new(0, 1, 1)));
        assert!(r.contains(CellAddr::new(0, 2, 2)));
        assert!(!r.contains(CellAddr::new(0, 0, 1)));
        assert!(!r.contains(CellAddr::new(0, 3, 2)));
        assert!(!r.contains(CellAddr::new(1, 1, 1)), "other sheet");
        assert!(r.contains_range(&CellRange::from_a1(0, "C3").unwrap()));
        assert!(!r.contains_range(&CellRange::from_a1(0, "C3:D3").unwrap()));
    }

    #[test]
    fn range_intersections() {
        let a = CellRange::from_a1(0, "A1:C3").unwrap();
        let b = CellRange::from_a1(0, "B2:D4").unwrap();
        let c = CellRange::from_a1(0, "D1:E2").unwrap();
        assert!(a.intersects(&b));
        assert_eq!(a.intersect(&b), CellRange::from_a1(0, "B2:C3"));
        assert!(!a.intersects(&c));
        assert_eq!(a.intersect(&c), None);
        let other_sheet = CellRange { sheet: 1, ..a };
        assert!(!a.intersects(&other_sheet));
        // Touching edges share cells.
        let edge = CellRange::from_a1(0, "C3:C9").unwrap();
        assert_eq!(a.intersect(&edge), CellRange::from_a1(0, "C3"));
    }

    #[test]
    fn range_cells_row_major() {
        let r = CellRange::from_a1(2, "B2:C3").unwrap();
        let cells: Vec<String> = r.cells().map(|a| a.to_a1()).collect();
        assert_eq!(cells, ["B2", "C2", "B3", "C3"]);
        assert!(r.cells().all(|a| a.sheet == 2));
        assert_eq!(CellRange::from_a1(0, "A1").unwrap().cells().count(), 1);
        // Last row/column of the grid must not overflow.
        let corner = CellRange::new(0, MAX_ROWS - 1, MAX_COLS - 2, MAX_ROWS - 1, MAX_COLS - 1);
        assert_eq!(corner.cells().count(), 2);
    }

    #[test]
    fn range_a1_round_trip() {
        let r = CellRange::from_a1(0, "C5:A1").unwrap();
        assert_eq!(r.to_a1(), "A1:C5");
        assert_eq!(CellRange::from_a1(0, "$A$1:$C$5"), Some(r));
        assert_eq!(CellRange::from_a1(0, "B7").unwrap().to_a1(), "B7");
        assert_eq!(CellRange::from(CellAddr::new(0, 6, 1)).to_a1(), "B7");
        assert_eq!(CellRange::from_a1(0, "A1:"), None);
        assert_eq!(CellRange::from_a1(0, "A1:B2:C3"), None);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_round_trip() {
        let r = CellRange::new(4, 1, 2, 3, 4);
        let json = serde_json::to_string(&r).unwrap();
        assert_eq!(json, r#"{"sheet":4,"r0":1,"c0":2,"r1":3,"c1":4}"#);
        assert_eq!(serde_json::from_str::<CellRange>(&json).unwrap(), r);
    }
}
