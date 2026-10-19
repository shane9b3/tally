//! Data region, header and column-type detection.

use tally_types::{col_to_letters, CellAddr, CellRange, Value};

use crate::SheetView;

/// Maximum number of cells read while locating a region.
pub const MAX_SCAN_CELLS: u64 = 200_000;

/// Distinct values tracked for a category column.
pub const MAX_CATEGORIES: usize = 12;

/// Detected column type.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ColumnType {
    /// Numbers.
    Number,
    /// Numbers that look like date serials (header says date/when/day).
    Date,
    /// Free text.
    Text,
    /// Low-cardinality text (see [`Column::distinct`]).
    Category,
    /// Booleans.
    Bool,
    /// No body values.
    Empty,
}

/// One analysed column.
#[derive(Clone, Debug)]
pub struct Column {
    /// Absolute 0-based column index.
    pub col: u32,
    /// Header text, when the region has a header row.
    pub header: Option<String>,
    /// Detected type.
    pub ty: ColumnType,
    /// Distinct values in first-seen order (Category columns only).
    pub distinct: Vec<String>,
    /// Number of non-blank body cells.
    pub filled: usize,
}

/// A detected data region.
#[derive(Clone, Debug)]
pub struct Region {
    /// The whole block including the header row.
    pub range: CellRange,
    /// Whether the first row is a header.
    pub has_header: bool,
    /// Columns left to right.
    pub columns: Vec<Column>,
}

impl Region {
    /// First body row (0-based).
    pub fn body_first_row(&self) -> u32 {
        self.range.r0 + u32::from(self.has_header)
    }

    /// Plain A1 range of a column's body, e.g. `G2:G15`.
    pub fn body_a1(&self, idx: usize) -> Option<String> {
        let c = self.columns.get(idx)?;
        let l = col_to_letters(c.col);
        Some(format!(
            "{l}{}:{l}{}",
            u64::from(self.body_first_row()) + 1,
            u64::from(self.range.r1) + 1
        ))
    }

    /// Display label of a column: its header or "column F".
    pub fn label(&self, idx: usize) -> String {
        match self.columns.get(idx) {
            Some(Column {
                header: Some(h), ..
            }) => h.clone(),
            Some(c) => format!("column {}", col_to_letters(c.col)),
            None => String::new(),
        }
    }
}

fn is_blank(v: &Value) -> bool {
    match v {
        Value::Empty => true,
        Value::Text(s) => s.trim().is_empty(),
        _ => false,
    }
}

struct Scan<'a> {
    view: &'a dyn SheetView,
    reads: u64,
}

impl Scan<'_> {
    fn filled(&mut self, row: u32, col: u32) -> bool {
        self.reads += 1;
        !is_blank(&self.view.value(row, col))
    }

    fn row_has(&mut self, row: u32, c0: u32, c1: u32) -> bool {
        (c0..=c1).any(|c| self.filled(row, c))
    }

    fn col_has(&mut self, col: u32, r0: u32, r1: u32) -> bool {
        (r0..=r1).any(|r| self.filled(r, col))
    }

    /// Grows `b` (like Excel's current region) while a bordering row or
    /// column, diagonals included, holds data. Never leaves `bounds`.
    fn expand(&mut self, mut b: CellRange, bounds: CellRange) -> CellRange {
        loop {
            let mut changed = false;
            let c_lo = b.c0.saturating_sub(1).max(bounds.c0);
            let c_hi = b.c1.saturating_add(1).min(bounds.c1);
            while b.r0 > bounds.r0
                && self.reads <= MAX_SCAN_CELLS
                && self.row_has(b.r0 - 1, c_lo, c_hi)
            {
                b.r0 -= 1;
                changed = true;
            }
            while b.r1 < bounds.r1
                && self.reads <= MAX_SCAN_CELLS
                && self.row_has(b.r1 + 1, c_lo, c_hi)
            {
                b.r1 += 1;
                changed = true;
            }
            let r_lo = b.r0.saturating_sub(1).max(bounds.r0);
            let r_hi = b.r1.saturating_add(1).min(bounds.r1);
            if b.c0 > bounds.c0 && self.col_has(b.c0 - 1, r_lo, r_hi) {
                b.c0 -= 1;
                changed = true;
            }
            if b.c1 < bounds.c1 && self.col_has(b.c1 + 1, r_lo, r_hi) {
                b.c1 += 1;
                changed = true;
            }
            if !changed || self.reads > MAX_SCAN_CELLS {
                return b;
            }
        }
    }

    /// Shrinks `b` to the bounding box of its filled cells.
    fn trim(&mut self, mut b: CellRange) -> Option<CellRange> {
        while b.r0 <= b.r1 && !self.row_has(b.r0, b.c0, b.c1) {
            b.r0 += 1;
        }
        if b.r0 > b.r1 {
            return None;
        }
        while !self.row_has(b.r1, b.c0, b.c1) {
            b.r1 -= 1;
        }
        while !self.col_has(b.c0, b.r0, b.r1) {
            b.c0 += 1;
        }
        while !self.col_has(b.c1, b.r0, b.r1) {
            b.c1 -= 1;
        }
        Some(b)
    }
}

/// Finds the data block around the selection or active cell. When the active
/// cell is not on or next to data, the largest block of the used range wins.
pub fn detect_range(
    view: &dyn SheetView,
    active: CellAddr,
    selection: Option<CellRange>,
) -> Option<CellRange> {
    let used = view.used()?;
    let sheet = view.sheet();
    let mut scan = Scan { view, reads: 0 };
    let grown = CellRange::new(
        sheet,
        used.r0.saturating_sub(1),
        used.c0.saturating_sub(1),
        used.r1.saturating_add(1),
        used.c1.saturating_add(1),
    );
    let seed = selection.unwrap_or_else(|| CellRange::single(active));
    let clipped =
        (seed.r0 <= grown.r1 && seed.r1 >= grown.r0 && seed.c0 <= grown.c1 && seed.c1 >= grown.c0)
            .then(|| {
                CellRange::new(
                    sheet,
                    seed.r0.max(grown.r0),
                    seed.c0.max(grown.c0),
                    seed.r1.min(grown.r1),
                    seed.c1.min(grown.c1),
                )
            });
    if let Some(start) = clipped {
        let expanded = scan.expand(start, grown);
        if let Some(r) = scan.trim(expanded) {
            return Some(r);
        }
    }
    largest_region(&mut scan, used)
}

fn largest_region(scan: &mut Scan<'_>, used: CellRange) -> Option<CellRange> {
    let mut found: Vec<CellRange> = Vec::new();
    let mut best: Option<(u64, CellRange)> = None;
    let mut r = used.r0;
    'rows: while r <= used.r1 {
        let mut c = used.c0;
        while c <= used.c1 {
            if scan.reads > MAX_SCAN_CELLS {
                break 'rows;
            }
            if let Some(f) = found
                .iter()
                .find(|f| r >= f.r0 && r <= f.r1 && c >= f.c0 && c <= f.c1)
            {
                c = f.c1 + 1;
                continue;
            }
            if scan.filled(r, c) {
                let b = scan.expand(CellRange::new(used.sheet, r, c, r, c), used);
                found.push(b);
                let area = u64::from(b.rows()) * u64::from(b.cols());
                if best.as_ref().is_none_or(|(a, _)| area > *a) {
                    best = Some((area, b));
                }
            }
            c += 1;
        }
        r += 1;
    }
    best.map(|(_, b)| b)
}

/// Builds a [`Region`] over `range`: header detection and column typing.
pub fn analyze(view: &dyn SheetView, range: CellRange) -> Region {
    let has_header = detect_header(view, range);
    let body_r0 = range.r0 + u32::from(has_header);
    let mut columns = Vec::with_capacity(range.cols() as usize);
    for col in range.c0..=range.c1 {
        let header = has_header.then(|| match view.value(range.r0, col) {
            Value::Text(s) => s.trim().to_string(),
            _ => String::new(),
        });
        let hl = header.as_deref().map(str::to_lowercase).unwrap_or_default();
        let datey = ["date", "when", "day"].iter().any(|k| hl.contains(k));
        let (mut nums, mut texts, mut bools, mut others) = (0usize, 0usize, 0usize, 0usize);
        let mut all_date_range = true;
        let mut distinct: Vec<(String, usize)> = Vec::new();
        let mut too_many = false;
        for row in body_r0..=range.r1 {
            match view.value(row, col) {
                v if is_blank(&v) => {}
                Value::Number(n) => {
                    nums += 1;
                    if !(1.0..=80_000.0).contains(&n) {
                        all_date_range = false;
                    }
                }
                Value::Bool(_) => bools += 1,
                Value::Text(s) => {
                    texts += 1;
                    if !too_many {
                        let key = s.trim().to_lowercase();
                        if let Some(d) = distinct.iter_mut().find(|(k, _)| k.to_lowercase() == key)
                        {
                            d.1 += 1;
                        } else if distinct.len() >= MAX_CATEGORIES {
                            too_many = true;
                        } else {
                            distinct.push((s.trim().to_string(), 1));
                        }
                    }
                }
                _ => others += 1,
            }
        }
        let filled = nums + texts + bools + others;
        let mut ty = if filled == 0 {
            ColumnType::Empty
        } else if nums == filled {
            if datey && all_date_range {
                ColumnType::Date
            } else {
                ColumnType::Number
            }
        } else if bools == filled {
            ColumnType::Bool
        } else {
            ColumnType::Text
        };
        let mut values = Vec::new();
        if ty == ColumnType::Text && texts == filled && !too_many && !distinct.is_empty() {
            #[allow(clippy::cast_precision_loss)]
            let avg = filled as f64 / distinct.len() as f64;
            if avg >= 2.0 {
                ty = ColumnType::Category;
                values = distinct.into_iter().map(|(v, _)| v).collect();
            }
        }
        columns.push(Column {
            col,
            header,
            ty,
            distinct: values,
            filled,
        });
    }
    Region {
        range,
        has_header,
        columns,
    }
}

/// The first row is a header if all its non-empty cells are text and the row
/// below has a non-empty, non-text cell.
pub fn detect_header(view: &dyn SheetView, range: CellRange) -> bool {
    if range.rows() < 2 {
        return false;
    }
    let mut any = false;
    for c in range.c0..=range.c1 {
        match view.value(range.r0, c) {
            v if is_blank(&v) => {}
            Value::Text(_) => any = true,
            _ => return false,
        }
    }
    if !any {
        return false;
    }
    (range.c0..=range.c1).any(|c| {
        let v = view.value(range.r0 + 1, c);
        !is_blank(&v) && !matches!(v, Value::Text(_))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct Grid(HashMap<(u32, u32), Value>);
    impl SheetView for Grid {
        fn value(&self, row: u32, col: u32) -> Value {
            self.0.get(&(row, col)).cloned().unwrap_or_default()
        }
        fn used(&self) -> Option<CellRange> {
            let r1 = self.0.keys().map(|k| k.0).max()?;
            let c1 = self.0.keys().map(|k| k.1).max()?;
            Some(CellRange::new(0, 0, 0, r1, c1))
        }
        fn sheet(&self) -> u32 {
            0
        }
    }

    fn grid(rows: &[(u32, u32, &[Value])]) -> Grid {
        let mut m = HashMap::new();
        for (r0, c0, vals) in rows {
            for (i, v) in vals.iter().enumerate() {
                m.insert((*r0, c0 + i as u32), v.clone());
            }
        }
        Grid(m)
    }

    fn t(s: &str) -> Value {
        Value::text(s)
    }
    fn n(x: f64) -> Value {
        Value::Number(x)
    }

    #[test]
    fn header_and_types() {
        let g = grid(&[
            (
                0,
                0,
                &[t("Date"), t("Name"), t("Amount"), t("Status"), t("Flag")],
            ),
            (
                1,
                0,
                &[n(46000.0), t("a1"), n(10.0), t("Paid"), Value::Bool(true)],
            ),
            (
                2,
                0,
                &[n(46001.0), t("a2"), n(20.0), t("Due"), Value::Bool(false)],
            ),
            (
                3,
                0,
                &[n(46002.0), t("a3"), n(30.0), t("Paid"), Value::Bool(true)],
            ),
            (
                4,
                0,
                &[n(46003.0), t("a4"), n(5.0), t("Due"), Value::Bool(true)],
            ),
        ]);
        let range = detect_range(&g, CellAddr::new(0, 2, 2), None).unwrap();
        assert_eq!(range, CellRange::new(0, 0, 0, 4, 4));
        let reg = analyze(&g, range);
        assert!(reg.has_header);
        let tys: Vec<_> = reg.columns.iter().map(|c| c.ty).collect();
        assert_eq!(
            tys,
            [
                ColumnType::Date,
                ColumnType::Text,
                ColumnType::Number,
                ColumnType::Category,
                ColumnType::Bool
            ]
        );
        assert_eq!(reg.columns[3].distinct, ["Paid", "Due"]);
        assert_eq!(reg.body_a1(2).as_deref(), Some("C2:C5"));
    }

    #[test]
    fn no_header_numeric_sheet() {
        let g = grid(&[
            (0, 0, &[n(1.0), n(2.0)]),
            (1, 0, &[n(3.0), n(4.0)]),
            (2, 0, &[n(5.0), n(6.0)]),
        ]);
        let range = detect_range(&g, CellAddr::new(0, 0, 0), None).unwrap();
        let reg = analyze(&g, range);
        assert!(!reg.has_header);
        assert_eq!(reg.body_a1(1).as_deref(), Some("B1:B3"));
        assert_eq!(reg.label(1), "column B");
    }

    #[test]
    fn blank_column_splits_regions() {
        let g = grid(&[
            (0, 0, &[t("A"), t("B")]),
            (1, 0, &[n(1.0), n(2.0)]),
            (0, 3, &[t("X"), t("Y"), t("Z")]),
            (1, 3, &[n(1.0), n(2.0), n(3.0)]),
            (2, 3, &[n(1.0), n(2.0), n(3.0)]),
        ]);
        let left = detect_range(&g, CellAddr::new(0, 1, 0), None).unwrap();
        assert_eq!(left, CellRange::new(0, 0, 0, 1, 1));
        let right = detect_range(&g, CellAddr::new(0, 1, 4), None).unwrap();
        assert_eq!(right, CellRange::new(0, 0, 3, 2, 5));
    }

    #[test]
    fn active_outside_data_uses_largest() {
        let g = grid(&[
            (0, 0, &[t("A"), t("B")]),
            (1, 0, &[n(1.0), n(2.0)]),
            (0, 5, &[t("X"), t("Y"), t("Z")]),
            (1, 5, &[n(1.0), n(2.0), n(3.0)]),
            (2, 5, &[n(1.0), n(2.0), n(3.0)]),
            (3, 5, &[n(1.0), n(2.0), n(3.0)]),
        ]);
        let r = detect_range(&g, CellAddr::new(0, 30, 30), None).unwrap();
        assert_eq!(r, CellRange::new(0, 0, 5, 3, 7));
    }

    #[test]
    fn active_adjacent_below_data_joins_it() {
        let g = grid(&[(0, 0, &[t("A"), t("B")]), (1, 0, &[n(1.0), n(2.0)])]);
        let r = detect_range(&g, CellAddr::new(0, 2, 0), None).unwrap();
        assert_eq!(r, CellRange::new(0, 0, 0, 1, 1));
    }

    #[test]
    fn selection_expands_to_region() {
        let g = grid(&[
            (0, 0, &[t("A"), t("B")]),
            (1, 0, &[n(1.0), n(2.0)]),
            (2, 0, &[n(1.0), n(2.0)]),
        ]);
        let sel = CellRange::new(0, 1, 1, 2, 1);
        let r = detect_range(&g, CellAddr::new(0, 1, 1), Some(sel)).unwrap();
        assert_eq!(r, CellRange::new(0, 0, 0, 2, 1));
    }

    #[test]
    fn all_text_is_not_a_header_and_empty_sheet_is_none() {
        let g = grid(&[(0, 0, &[t("x"), t("y")]), (1, 0, &[t("p"), t("q")])]);
        assert!(!detect_header(&g, CellRange::new(0, 0, 0, 1, 1)));
        let e = Grid(HashMap::new());
        assert!(detect_range(&e, CellAddr::new(0, 0, 0), None).is_none());
    }

    #[test]
    fn high_cardinality_text_is_not_category() {
        let mut rows: Vec<(u32, u32, Vec<Value>)> = vec![(0, 0, vec![t("Name"), t("N")])];
        for i in 0..20u32 {
            rows.push((i + 1, 0, vec![t(&format!("person{i}")), n(f64::from(i))]));
        }
        let refs: Vec<(u32, u32, &[Value])> = rows
            .iter()
            .map(|(a, b, v)| (*a, *b, v.as_slice()))
            .collect();
        let g = grid(&refs);
        let reg = analyze(&g, CellRange::new(0, 0, 0, 20, 1));
        assert_eq!(reg.columns[0].ty, ColumnType::Text);
        assert!(reg.columns[0].distinct.is_empty());
    }
}
