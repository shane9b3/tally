//! 3-D references (`Sheet1.A1:Sheet3.A1`): evaluation per function family,
//! dependencies, sheet operations and undo (AOO 4.1 `svDoubleRef` with
//! `Tab1 != Tab2`).
#![allow(clippy::unwrap_used)]

mod common;

use common::{input, Fixture};
use tally_api::{DocumentService, Op};
use tally_types::{CellAddr, ErrorKind, SheetId, Value};

fn n(x: f64) -> Value {
    Value::Number(x)
}

fn e(k: ErrorKind) -> Value {
    Value::Error(k)
}

/// Sheet1, Sheet2, Sheet3, Sheet4 with A1 = 10/20/30/40, A2 = 1/2/3/4 and
/// A3 = "t" on Sheet2. Formulas go on Sheet1 columns D onwards.
struct Book {
    f: Fixture,
    s: [SheetId; 4],
}

impl Book {
    fn new() -> Self {
        Self::with(Fixture::new())
    }

    fn with(mut f: Fixture) -> Self {
        let r = f.apply(
            (2..=4)
                .map(|i| Op::AddSheet {
                    name: Some(format!("Sheet{i}")),
                    index: None,
                })
                .collect(),
        );
        let info = r.info.unwrap();
        let s = [
            info.sheets[0].id,
            info.sheets[1].id,
            info.sheets[2].id,
            info.sheets[3].id,
        ];
        assert_eq!(info.sheets[0].name, "Sheet1");
        let mut ops = Vec::new();
        for (i, id) in s.iter().enumerate() {
            ops.push(input(*id, "A1", &format!("{}", (i + 1) * 10)));
            ops.push(input(*id, "A2", &format!("{}", i + 1)));
        }
        ops.push(input(s[1], "A3", "t"));
        f.apply(ops);
        Self { f, s }
    }

    fn eval(&mut self, a1: &str, formula: &str) -> Value {
        let r = self.f.set(a1, formula);
        assert!(r.errors.is_empty(), "{formula}: {:?}", r.errors);
        self.f.value(a1)
    }

    fn set_on(&mut self, sheet: usize, a1: &str, text: &str) {
        let op = input(self.s[sheet], a1, text);
        self.f.apply(vec![op]);
    }

    fn formula(&self, a1: &str) -> String {
        let a = CellAddr::from_a1(self.f.sheet, a1).unwrap();
        let wb = &self.f.svc.document(self.f.doc).unwrap().wb;
        let (id, _) = wb.formula_at(a).unwrap();
        wb.formula_text(id).unwrap()
    }
}

fn sheet_index(b: &Book, id: SheetId) -> usize {
    b.f.svc.document(b.f.doc).unwrap().wb.tab_of(id).unwrap()
}

#[test]
fn sum_over_three_sheets() {
    let mut b = Book::new();
    assert_eq!(b.eval("D1", "=SUM(Sheet1.A1:Sheet3.A1)"), n(60.0));
    assert_eq!(b.eval("D2", "=SUM($Sheet1.A1:$Sheet3.A1)"), n(60.0));
    assert_eq!(b.eval("D3", "=SUM(Sheet1.A1:Sheet3.A2)"), n(66.0));
    // Reversed order is normalised.
    assert_eq!(b.eval("D4", "=SUM(Sheet3.A1:Sheet1.A1)"), n(60.0));
    assert_eq!(b.eval("D5", "=SUM(Sheet2.A1:Sheet4.A2)"), n(99.0));
}

/// Over `Sheet1.A1:Sheet3.A3`: 10, 1, _ | 20, 2, "t" | 30, 3, _.
const R: &str = "Sheet1.A1:Sheet3.A3";

#[test]
fn sequence_functions_iterate_every_sheet() {
    let mut b = Book::new();
    let cases: &[(&str, Value)] = &[
        ("SUM(@)", n(66.0)),
        ("SUMSQ(@)", n(1414.0)),
        ("PRODUCT(@)", n(36000.0)),
        ("COUNT(@)", n(6.0)),
        ("COUNTA(@)", n(7.0)),
        ("COUNTBLANK(@)", n(2.0)),
        ("AVERAGE(@)", n(11.0)),
        ("MIN(@)", n(1.0)),
        ("MAX(@)", n(30.0)),
        // Text counts as 0 for the *A functions.
        ("MINA(@)", n(0.0)),
        ("MAXA(@)", n(30.0)),
        ("AVERAGEA(@)*7", n(66.0)),
        ("VAR(@)", n(137.6)),
        ("VARP(@)*6", n(688.0)),
        ("STDEV(@)^2", n(137.6)),
        ("DEVSQ(@)", n(688.0)),
        ("MEDIAN(@)", n(6.5)),
        ("LARGE(@;2)", n(20.0)),
        ("SMALL(@;2)", n(2.0)),
        ("PERCENTILE(@;0.5)", n(6.5)),
        ("QUARTILE(@;2)", n(6.5)),
        ("RANK(20;@)", n(2.0)),
        ("SUBTOTAL(9;@)", n(66.0)),
        ("SUBTOTAL(2;@)", n(6.0)),
        ("ROUND(NPV(0;@);6)", n(66.0)),
        // A 3D reference inside a union is iterated too.
        ("SUM(Sheet1.A1:Sheet3.A1~Sheet4.A1)", n(100.0)),
        ("SUM(@;5)", n(71.0)),
    ];
    for (i, (f, want)) in cases.iter().enumerate() {
        let cell = format!("D{}", i + 1);
        let text = format!("={}", f.replace('@', R));
        let got = b.eval(&cell, &text);
        match (&got, want) {
            (Value::Number(x), Value::Number(y)) => {
                assert!((x - y).abs() < 1e-9, "{text}: {got:?} want {want:?}")
            }
            _ => assert_eq!(&got, want, "{text}"),
        }
    }
    // MODE over three sheets.
    b.set_on(0, "B1", "5");
    b.set_on(1, "B1", "7");
    b.set_on(2, "B1", "5");
    assert_eq!(b.eval("E1", "=MODE(Sheet1.B1:Sheet3.B1)"), n(5.0));
}

#[test]
fn errors_inside_propagate_in_tab_order() {
    let mut b = Book::new();
    b.set_on(2, "A5", "=1/0");
    b.set_on(1, "A5", "=NA()");
    // Sheet2 comes before Sheet3, so #N/A is met first.
    assert_eq!(b.eval("D1", "=SUM(Sheet1.A1:Sheet3.A5)"), e(ErrorKind::NA));
    assert_eq!(b.eval("D2", "=COUNT(Sheet1.A1:Sheet3.A5)"), n(6.0));
    assert_eq!(b.eval("D3", "=COUNTA(Sheet1.A1:Sheet3.A5)"), n(9.0));
    assert_eq!(
        b.eval("D4", "=SUM(Sheet3.A1:Sheet3.A5)"),
        e(ErrorKind::Div0)
    );
}

#[test]
fn hidden_sheets_and_rows() {
    let mut b = Book::new();
    let s2 = b.s[1];
    b.f.apply(vec![Op::SetSheetHidden {
        sheet: s2,
        hidden: true,
    }]);
    assert_eq!(b.eval("D1", "=SUM(Sheet1.A1:Sheet3.A1)"), n(60.0));
    b.f.apply(vec![Op::SetRowsHidden {
        sheet: s2,
        start: 0,
        end: 0,
        hidden: true,
    }]);
    // SUBTOTAL 109 skips the hidden row on each sheet.
    assert_eq!(b.eval("D2", "=SUBTOTAL(109;Sheet1.A1:Sheet3.A2)"), n(46.0));
    assert_eq!(b.eval("D3", "=SUBTOTAL(9;Sheet1.A1:Sheet3.A2)"), n(66.0));
}

#[test]
fn single_area_functions_reject_3d_refs() {
    let mut b = Book::new();
    let p = ErrorKind::ParamList; // Err:504, errIllegalParameter
    let a = ErrorKind::InvalidArgument; // Err:502, errIllegalArgument
    let cases: &[(&str, ErrorKind)] = &[
        ("=COUNTIF(Sheet1.A1:Sheet3.A3;\">5\")", p),
        ("=SUMIF(Sheet1.A1:Sheet3.A3;\">5\")", p),
        ("=SUMIF(A1:A3;\">5\";Sheet1.A1:Sheet3.A3)", p),
        ("=AVERAGEIF(Sheet1.A1:Sheet3.A3;\">5\")", p),
        ("=COUNTIFS(Sheet1.A1:Sheet3.A3;\">5\")", a),
        ("=SUMIFS(Sheet1.A1:Sheet3.A3;A1:A3;\">5\")", a),
        ("=AVERAGEIFS(A1:A3;Sheet1.A1:Sheet3.A3;\">5\")", a),
        ("=VLOOKUP(10;Sheet1.A1:Sheet3.A3;1;0)", p),
        ("=HLOOKUP(10;Sheet1.A1:Sheet3.A3;1;0)", p),
        ("=MATCH(10;Sheet1.A1:Sheet3.A3;0)", p),
        ("=LOOKUP(10;Sheet1.A1:Sheet3.A3)", p),
        ("=INDEX(Sheet1.A1:Sheet3.A3;1;1)", a),
        ("=SUM(OFFSET(Sheet1.A1:Sheet3.A3;0;0))", a),
        ("=SUMPRODUCT(Sheet1.A1:Sheet3.A3)", p),
        ("=SUMPRODUCT(Sheet1.A1:Sheet3.A1*2)", p),
        ("=MMULT(Sheet1.A1:Sheet3.A1;1)", p),
        ("=CORREL(Sheet1.A1:Sheet3.A1;A1:A3)", p),
        ("=TTEST(Sheet1.A1:Sheet3.A2;A1:A3;2;1)", p),
        ("=PROB(Sheet1.A1:Sheet3.A1;A1:A3;1)", p),
        ("=SUMX2MY2(Sheet1.A1:Sheet3.A1;A1:A3)", p),
        ("=DSUM(Sheet1.A1:Sheet3.A3;1;A1:A2)", p),
    ];
    for (i, (f, want)) in cases.iter().enumerate() {
        let cell = format!("D{}", i + 1);
        assert_eq!(b.eval(&cell, f), e(*want), "{f}");
    }
}

#[test]
fn reference_functions() {
    let mut b = Book::new();
    assert_eq!(b.eval("D1", "=ROWS(Sheet1.A1:Sheet3.A3)"), n(9.0));
    assert_eq!(b.eval("D2", "=COLUMNS(Sheet1.A1:Sheet3.B3)"), n(6.0));
    assert_eq!(b.eval("D3", "=AREAS(Sheet1.A1:Sheet3.A3)"), n(1.0));
    assert_eq!(b.eval("D4", "=SHEETS(Sheet1.A1:Sheet3.A3)"), n(3.0));
    assert_eq!(b.eval("D5", "=SHEET(Sheet3.A1:Sheet2.A1)"), n(2.0));
    assert_eq!(b.eval("D6", "=ROW(Sheet1.B2:Sheet3.C4)"), n(2.0));
    assert_eq!(b.eval("D7", "=COLUMN(Sheet1.B2:Sheet3.C4)"), n(2.0));
    assert_eq!(
        b.eval("D8", "=ISREF(Sheet1.A1:Sheet3.A3)"),
        Value::Bool(true)
    );
    assert_eq!(
        b.eval("E1", "=SUM(INDIRECT(\"Sheet1.A1:Sheet3.A2\"))"),
        n(66.0)
    );
    assert_eq!(
        b.eval("E2", "=SUM(INDIRECT(\"$Sheet2.A1:$Sheet4.A1\"))"),
        n(90.0)
    );
    // INDIRECT's dynamic dependencies cover every sheet.
    b.set_on(2, "A1", "31");
    assert_eq!(b.f.value("E1"), n(67.0));
    // Intersection with a 2D range picks the sheets both cover.
    assert_eq!(b.eval("D9", "=SUM(Sheet1.A1:Sheet3.A3!A1:A1)"), n(10.0));
    assert_eq!(
        b.eval("D10", "=SUM(Sheet1.A1:Sheet3.A3!Sheet1.A1:Sheet2.A2)"),
        n(33.0)
    );
    assert_eq!(
        b.eval("D11", "=SUM(Sheet1.A1:Sheet3.A1!Sheet1.B1:Sheet3.B1)"),
        e(ErrorKind::Null)
    );
}

#[test]
fn scalar_context_intersects_on_the_formula_sheet() {
    let mut b = Book::new();
    // Row 1 of Sheet1 (inside Sheet1..Sheet3): the column's cell on Sheet1.
    assert_eq!(b.eval("D1", "=Sheet1.A1:Sheet3.A1"), n(10.0));
    assert_eq!(b.eval("D2", "=Sheet1.A1:Sheet3.A3+1"), n(2.0));
    assert_eq!(b.eval("D3", "=ABS(Sheet1.A1:Sheet3.A3)"), n(0.0));
    // Not in the row or column of a single cell: #VALUE!.
    assert_eq!(b.eval("D4", "=Sheet1.A1:Sheet3.A1+1"), e(ErrorKind::Value));
    // The formula's sheet is outside the span: #VALUE!.
    assert_eq!(b.eval("E1", "=Sheet2.A1:Sheet3.A1+0"), e(ErrorKind::Value));
    // A two-dimensional rectangle cannot be intersected.
    assert_eq!(b.eval("E2", "=Sheet1.A1:Sheet3.B3*1"), e(ErrorKind::Value));
    // Array context: no matrix from a 3D reference (Err:504).
    let s = b.f.sheet;
    let r = b.f.apply(vec![Op::SetArrayFormula {
        sheet: s,
        range: common::range("F1:F3"),
        input: "=Sheet1.A1:Sheet3.A3*1".into(),
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(b.f.value("F1"), e(ErrorKind::ParamList));
    // SUM keeps accepting it in an array formula (a reference parameter).
    b.f.apply(vec![Op::SetArrayFormula {
        sheet: s,
        range: common::range("G1"),
        input: "=SUM(Sheet1.A1:Sheet3.A3)".into(),
    }]);
    assert_eq!(b.f.value("G1"), n(66.0));
}

#[test]
fn edits_inside_the_span_recalculate() {
    let mut b = Book::new();
    assert_eq!(b.eval("D1", "=SUM(Sheet1.A1:Sheet3.A1)"), n(60.0));
    b.eval("D2", "=D1*2");
    b.set_on(1, "A1", "25");
    assert_eq!(b.f.value("D1"), n(65.0));
    assert_eq!(b.f.value("D2"), n(130.0));
    // A cell that was empty, on the middle sheet.
    b.set_on(1, "A2", "=5");
    assert_eq!(b.eval("D3", "=SUM(Sheet1.A2:Sheet3.A2)"), n(9.0));
    b.set_on(1, "A2", "7");
    assert_eq!(b.f.value("D3"), n(11.0));
    // Outside the span: no change.
    b.set_on(3, "A1", "1000");
    assert_eq!(b.f.value("D1"), n(65.0));
    // Undo the edit on Sheet2.
    b.f.svc.undo(b.f.doc).unwrap();
    b.f.svc.undo(b.f.doc).unwrap();
    assert_eq!(b.f.value("D3"), n(9.0));
}

// ----- sheet operations (formulas on Sheet1, span Sheet2..Sheet4 = 90) -----

fn span_book() -> Book {
    let mut b = Book::new();
    assert_eq!(b.eval("D1", "=SUM(Sheet2.A1:Sheet4.A1)"), n(90.0));
    assert_eq!(b.eval("D2", "=SUM($Sheet2.A1:$Sheet4.A1)"), n(90.0));
    b
}

#[test]
fn inserting_a_sheet_inside_the_span_extends_it() {
    let mut b = span_book();
    let r = b.f.apply(vec![Op::AddSheet {
        name: Some("New".into()),
        index: Some(2),
    }]);
    let new = r.info.unwrap().sheets[2].id;
    b.f.apply(vec![input(new, "A1", "5")]);
    assert_eq!(b.f.value("D1"), n(95.0));
    assert_eq!(b.f.value("D2"), n(95.0));
    b.f.apply(vec![input(new, "A1", "6")]);
    assert_eq!(b.f.value("D1"), n(96.0));
    // At the position of the last sheet (before it): inside.
    let r = b.f.apply(vec![Op::AddSheet {
        name: Some("Before4".into()),
        index: Some(4),
    }]);
    let before4 = r.info.unwrap().sheets[4].id;
    b.f.apply(vec![input(before4, "A1", "100")]);
    assert_eq!(b.f.value("D1"), n(196.0));
    // Before the first or after the last: outside.
    let r = b.f.apply(vec![
        Op::AddSheet {
            name: Some("First".into()),
            index: Some(0),
        },
        Op::AddSheet {
            name: Some("Last".into()),
            index: None,
        },
    ]);
    let info = r.info.unwrap();
    let (first, last) = (info.sheets[0].id, info.sheets.last().unwrap().id);
    b.f.apply(vec![input(first, "A1", "1000"), input(last, "A1", "1000")]);
    assert_eq!(b.f.value("D1"), n(196.0));
    assert_eq!(b.formula("D1"), "=SUM(Sheet2.A1:Sheet4.A1)");
}

#[test]
fn copying_a_sheet_into_the_span_extends_it() {
    let mut b = span_book();
    let s2 = b.s[1];
    let r = b.f.apply(vec![Op::CopySheet {
        sheet: s2,
        name: None,
        index: None,
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(b.f.value("D1"), n(110.0));
}

#[test]
fn deleting_sheets_at_the_ends_and_in_the_middle() {
    let mut b = span_book();
    let [_, s2, s3, s4] = b.s;
    // Middle: the span loses it.
    b.f.apply(vec![Op::DeleteSheet { sheet: s3 }]);
    assert_eq!(b.f.value("D1"), n(60.0));
    assert_eq!(b.formula("D1"), "=SUM(Sheet2.A1:Sheet4.A1)");
    b.f.svc.undo(b.f.doc).unwrap();
    assert_eq!(b.f.value("D1"), n(90.0));
    // First end: the span starts on the next sheet (AOO UpdateDeleteTab).
    b.f.apply(vec![Op::DeleteSheet { sheet: s2 }]);
    assert_eq!(b.f.value("D1"), n(70.0));
    assert_eq!(b.formula("D1"), "=SUM(Sheet3.A1:Sheet4.A1)");
    assert_eq!(b.formula("D2"), "=SUM($Sheet3.A1:$Sheet4.A1)");
    b.f.svc.undo(b.f.doc).unwrap();
    assert_eq!(b.f.value("D1"), n(90.0));
    assert_eq!(b.formula("D1"), "=SUM(Sheet2.A1:Sheet4.A1)");
    assert_eq!(b.formula("D2"), "=SUM($Sheet2.A1:$Sheet4.A1)");
    // The recalc graph is back: Sheet3 is inside again.
    b.set_on(2, "A1", "31");
    assert_eq!(b.f.value("D1"), n(91.0));
    // Last end: the span ends on the previous sheet.
    b.f.apply(vec![Op::DeleteSheet { sheet: s4 }]);
    assert_eq!(b.f.value("D1"), n(51.0));
    assert_eq!(b.formula("D1"), "=SUM(Sheet2.A1:Sheet3.A1)");
    // Down to one sheet, then none.
    b.f.apply(vec![Op::DeleteSheet { sheet: s2 }]);
    assert_eq!(b.f.value("D1"), n(31.0));
    b.f.apply(vec![Op::DeleteSheet { sheet: s3 }]);
    assert_eq!(b.f.value("D1"), e(ErrorKind::Ref));
    for _ in 0..3 {
        b.f.svc.undo(b.f.doc).unwrap();
    }
    assert_eq!(b.f.value("D1"), n(91.0));
    assert_eq!(b.formula("D1"), "=SUM(Sheet2.A1:Sheet4.A1)");
}

#[test]
fn moving_sheets_changes_the_span() {
    let mut b = span_book();
    let [_, s2, s3, s4] = b.s;
    // A middle sheet moved out (after the last end).
    b.f.apply(vec![Op::MoveSheet { sheet: s3, to: 3 }]);
    assert_eq!(sheet_index(&b, s3), 3);
    assert_eq!(b.f.value("D1"), n(60.0));
    b.set_on(2, "A1", "1000");
    assert_eq!(b.f.value("D1"), n(60.0), "Sheet3 is outside now");
    b.f.svc.undo(b.f.doc).unwrap();
    b.f.svc.undo(b.f.doc).unwrap();
    assert_eq!(b.f.value("D1"), n(90.0));
    // An end moved before the other end: the span is normalised.
    b.f.apply(vec![Op::MoveSheet { sheet: s4, to: 1 }]);
    // Order: Sheet1, Sheet4, Sheet2, Sheet3 -> Sheet4..Sheet2.
    assert_eq!(b.f.value("D1"), n(60.0));
    // An outside sheet moved in.
    b.f.apply(vec![Op::MoveSheet {
        sheet: b.f.sheet,
        to: 1,
    }]);
    // Order: Sheet4, Sheet1, Sheet2, Sheet3 -> Sheet4..Sheet2 includes Sheet1.
    assert_eq!(sheet_index(&b, s2), 2);
    assert_eq!(b.f.value("D1"), n(70.0));
    b.f.svc.undo(b.f.doc).unwrap();
    b.f.svc.undo(b.f.doc).unwrap();
    assert_eq!(b.f.value("D1"), n(90.0));
}

#[test]
fn renaming_and_hiding_keep_the_span() {
    let mut b = span_book();
    let s3 = b.s[2];
    let s4 = b.s[3];
    b.f.apply(vec![Op::RenameSheet {
        sheet: s4,
        name: "Last One".into(),
    }]);
    assert_eq!(b.formula("D1"), "=SUM(Sheet2.A1:'Last One'.A1)");
    assert_eq!(b.f.value("D1"), n(90.0));
    b.f.apply(vec![Op::SetSheetHidden {
        sheet: s3,
        hidden: true,
    }]);
    assert_eq!(b.f.value("D1"), n(90.0));
    b.set_on(2, "A1", "40");
    assert_eq!(b.f.value("D1"), n(100.0), "hidden sheets stay in the span");
}

#[test]
fn names_with_3d_refs() {
    let mut b = Book::new();
    let r = b.f.apply(vec![Op::DefineName {
        name: "Tot".into(),
        scope: None,
        refers_to: "$Sheet2.$A$1:$Sheet4.$A$1".into(),
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(b.eval("D1", "=SUM(Tot)"), n(90.0));
    b.set_on(2, "A1", "35");
    assert_eq!(b.f.value("D1"), n(95.0));
    b.f.apply(vec![Op::DeleteSheet { sheet: b.s[1] }]);
    assert_eq!(b.f.value("D1"), n(75.0));
}

#[test]
fn fifty_sheets_of_ten_thousand_cells() {
    let mut f = Fixture::new();
    let r = f.apply(
        (2..=50)
            .map(|i| Op::AddSheet {
                name: Some(format!("Sheet{i}")),
                index: None,
            })
            .collect(),
    );
    let sheets: Vec<SheetId> = r.info.unwrap().sheets.iter().map(|s| s.id).collect();
    {
        let wb = &mut f.svc.document_mut(f.doc).unwrap().wb;
        for s in &sheets {
            for row in 0..10_000u32 {
                wb.set_value(CellAddr::new(*s, row, 0), Value::Number(1.0));
            }
        }
    }
    let t = std::time::Instant::now();
    f.set("C1", "=SUM(Sheet1.A1:Sheet50.A10000)");
    assert_eq!(f.value("C1"), n(500_000.0));
    let first = t.elapsed();
    let t = std::time::Instant::now();
    f.apply(vec![input(sheets[25], "A5", "2")]);
    assert_eq!(f.value("C1"), n(500_001.0));
    let again = t.elapsed();
    assert!(
        first.as_secs_f64() < 5.0 && again.as_secs_f64() < 5.0,
        "{first:?} / {again:?}"
    );
}

#[cfg(feature = "io")]
mod files {
    use super::*;
    use tally_api::FileFormat;
    use tally_engine::EngineService;

    fn round_trip(fmt: FileFormat, ext: &str) {
        let mut svc = EngineService::with_default_codec();
        let info = svc.new_workbook(None).unwrap();
        let mut b = Book::with(Fixture {
            svc,
            doc: info.doc,
            sheet: info.sheets[0].id,
        });
        b.eval("D1", "=SUM(Sheet1.A1:Sheet3.A1)");
        b.eval("D2", "=SUM($Sheet2.A1:$Sheet4.A2)");
        b.eval("D3", "=COUNTIF(Sheet1.A1:Sheet3.A1;\">5\")");
        // The start on the formula's own sheet (Excel must name it).
        b.eval("D4", "=SUM(A1:Sheet3.A1)");
        // The start on the formula's own sheet (Excel must name it).
        b.eval("D4", "=SUM(A1:Sheet3.A1)");
        let dir = std::env::temp_dir().join(format!("tally-3d-{ext}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("three_d.{ext}"));
        b.f.svc.save(b.f.doc, Some(&path), fmt).unwrap();
        if ext == "ods" {
            let bytes = std::fs::read(&path).unwrap();
            let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
            let mut xml = String::new();
            std::io::Read::read_to_string(&mut zip.by_name("content.xml").unwrap(), &mut xml)
                .unwrap();
            let at = xml.find("of:=SUM(").unwrap();
            assert!(
                xml.contains("of:=SUM([Sheet1.A1:Sheet3.A1])")
                    && xml.contains("of:=SUM([$Sheet2.A1:$Sheet4.A2])"),
                "ODS keeps the 3D reference: {}",
                &xml[at..(at + 200).min(xml.len())]
            );
        }
        let mut svc = EngineService::with_default_codec();
        let info = svc.open(&path).unwrap();
        let s1 = info.sheets[0].id;
        let wb = &mut svc.document_mut(info.doc).unwrap().wb;
        tally_engine::recalc::recalc(wb);
        let v = |a1: &str| wb.value(CellAddr::from_a1(s1, a1).unwrap());
        assert_eq!(v("D1"), n(60.0), "{ext}");
        assert_eq!(v("D2"), n(99.0), "{ext}");
        assert_eq!(v("D3"), e(ErrorKind::ParamList), "{ext}");
        assert_eq!(v("D4"), n(60.0), "{ext}");
        assert_eq!(v("D4"), n(60.0), "{ext}");
        let (id, _) = wb.formula_at(CellAddr::from_a1(s1, "D1").unwrap()).unwrap();
        assert_eq!(wb.formula_text(id).unwrap(), "=SUM(Sheet1.A1:Sheet3.A1)");
        let (id, _) = wb.formula_at(CellAddr::from_a1(s1, "D2").unwrap()).unwrap();
        // XLSX has no absolute sheet marker.
        let want = if ext == "ods" {
            "=SUM($Sheet2.A1:$Sheet4.A2)"
        } else {
            "=SUM(Sheet2.A1:Sheet4.A2)"
        };
        assert_eq!(wb.formula_text(id).unwrap(), want);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ods_round_trip() {
        round_trip(FileFormat::Ods, "ods");
    }

    #[test]
    fn xlsx_round_trip() {
        round_trip(FileFormat::Xlsx, "xlsx");
    }
}

/// A span over every sheet on every sheet (security review of the 3D
/// work: 10,000 such formulas used to store 10⁸ expanded precedents,
/// 8.5 GB). Spans stay one dependency entry each, so memory scales with
/// formulas, not formulas × sheets.
#[test]
fn spans_over_many_sheets_stay_unexpanded() {
    use tally_engine::recalc::recalc;
    use tally_engine::Workbook;
    const N: usize = 1_000;
    let mut wb = Workbook::new();
    let mut ids = vec![wb.sheets[0].id];
    for i in 2..=N {
        ids.push(wb.add_sheet(Some(&format!("Sheet{i}")), None));
    }
    let text = format!("=SUM(Sheet1.A1:Sheet{N}.A1)");
    let t = std::time::Instant::now();
    for s in &ids {
        wb.set_input(CellAddr::new(*s, 0, 0), "1");
        wb.set_input(CellAddr::new(*s, 0, 1), &text);
    }
    recalc(&mut wb);
    assert_eq!(wb.value(CellAddr::new(ids[0], 0, 1)), n(N as f64));
    assert_eq!(wb.deps.span_count(), N, "one entry per formula");
    let (_, fc) = wb.formula_at(CellAddr::new(ids[7], 0, 1)).unwrap();
    assert!(fc.precedents.is_empty() && fc.spans().len() == 1);
    // A sheet inserted in the middle joins every span.
    let mid = wb.add_sheet(Some("Mid"), Some(N / 2));
    wb.refresh_all_precedents();
    wb.set_input(CellAddr::new(mid, 0, 0), "5");
    recalc(&mut wb);
    assert_eq!(wb.value(CellAddr::new(ids[0], 0, 1)), n(N as f64 + 5.0));
    // An edit on one sheet recalculates the N formulas once each.
    wb.set_input(CellAddr::new(ids[3], 0, 0), "2");
    recalc(&mut wb);
    assert_eq!(wb.value(CellAddr::new(ids[9], 0, 1)), n(N as f64 + 6.0));
    // Debug build; release takes a fraction of this.
    assert!(t.elapsed().as_secs_f64() < 60.0, "{:?}", t.elapsed());
}

#[test]
fn reordering_sheets_inside_a_span_recalculates_order_dependent_formulas() {
    let mut b = Book::new();
    // NPV discounts in tab order: 10, 20, 30, 40 at 100 %.
    assert_eq!(b.eval("D1", "=NPV(1;Sheet1.A1:Sheet4.A1)"), n(16.25));
    // Sheet3 moved between Sheet1 and Sheet2: the same sheets, now
    // 10, 30, 20, 40.
    b.f.apply(vec![Op::MoveSheet {
        sheet: b.s[2],
        to: 1,
    }]);
    assert_eq!(b.f.value("D1"), n(17.5));
}

#[test]
fn deleting_the_sheet_of_a_2d_range_is_ref_error() {
    let mut b = Book::new();
    assert_eq!(b.eval("D1", "=SUM(Sheet3.A1:A2)"), n(33.0));
    assert_eq!(b.eval("D2", "=SUM($Sheet3.A1:$Sheet3.A2)"), n(33.0));
    assert_eq!(b.eval("D3", "=SUM(Sheet3.A1:A2;Sheet2.A1)"), n(53.0));
    b.f.apply(vec![Op::DefineName {
        name: "OnThree".into(),
        scope: None,
        refers_to: "$Sheet3.$A$1:$A$2".into(),
    }]);
    assert_eq!(b.eval("D4", "=SUM(OnThree)"), n(33.0));
    b.f.apply(vec![Op::DeleteSheet { sheet: b.s[2] }]);
    for a1 in ["D1", "D2", "D3", "D4"] {
        assert_eq!(b.f.value(a1), e(ErrorKind::Ref), "{a1}");
    }
    assert_eq!(b.formula("D1"), "=SUM(#REF!)");
    b.f.svc.undo(b.f.doc).unwrap();
    assert_eq!(b.f.value("D1"), n(33.0));
    assert_eq!(b.f.value("D4"), n(33.0));
    assert_eq!(b.formula("D1"), "=SUM(Sheet3.A1:A2)");
}

#[test]
fn unions_keep_3d_entries_and_the_range_operator_spans_sheets() {
    let mut b = Book::new();
    assert_eq!(b.eval("D1", "=AREAS((Sheet1.A1:Sheet3.A1~A2))"), n(2.0));
    assert_eq!(
        b.eval("D2", "=COUNTIF((Sheet1.A1:Sheet3.A1~A2);\">0\")"),
        e(ErrorKind::ParamList)
    );
    assert_eq!(
        b.eval("D3", "=INDEX((Sheet1.A1:Sheet3.A1~Sheet4.A1);1;1;2)"),
        n(40.0)
    );
    assert_eq!(
        b.eval("D4", "=INDEX((Sheet1.A1:Sheet3.A1~Sheet4.A1);1;1;1)"),
        e(ErrorKind::InvalidArgument)
    );
    assert_eq!(
        b.eval("D5", "=SUM((Sheet1.A1:Sheet3.A1~Sheet4.A1))"),
        n(100.0)
    );
    assert_eq!(b.eval("D6", "=AREAS((A1~A2~Sheet2.A1:Sheet3.B2))"), n(3.0));
    // `:` between references on different sheets builds a 3D span.
    assert_eq!(
        b.eval("D7", "=SUM(Sheet1.A1:Sheet3.A1:Sheet4.A1)"),
        n(100.0)
    );
    assert_eq!(b.eval("D8", "=SUM(Sheet2.A1:Sheet3.A1:Sheet3.A2)"), n(55.0));
    // A bare `A2` is on the formula's sheet (Sheet1), which widens the span.
    assert_eq!(b.eval("D10", "=SUM(Sheet2.A1:Sheet3.A1:A2)"), n(66.0));
    // CELL, FORMULA and ISFORMULA intersect on the formula's sheet.
    b.set_on(0, "A4", "=1+1");
    assert_eq!(
        b.eval("E4", "=ISFORMULA(Sheet1.A4:Sheet3.A4)"),
        Value::Bool(true)
    );
    assert_eq!(
        b.eval("F4", "=FORMULA(Sheet1.A4:Sheet3.A4)"),
        Value::text("=1+1")
    );
    assert_eq!(
        b.eval("G4", "=CELL(\"address\";Sheet1.A4:Sheet3.A4)"),
        Value::text("$A$4")
    );
    assert_eq!(
        b.eval("D9", "=ISFORMULA(Sheet1.A4:Sheet3.A4)"),
        e(ErrorKind::Value)
    );
}

#[test]
fn sheet_and_sheets_follow_sheet_operations() {
    let mut b = Book::new();
    assert_eq!(b.eval("D1", "=SHEET(Sheet3.A1)"), n(3.0));
    assert_eq!(b.eval("D2", "=SHEETS(Sheet2.A1:Sheet4.A1)"), n(3.0));
    assert_eq!(b.eval("D3", "=SHEET(\"Sheet4\")"), n(4.0));
    b.f.apply(vec![Op::MoveSheet {
        sheet: b.s[2],
        to: 1,
    }]);
    assert_eq!(b.f.value("D1"), n(2.0));
    assert_eq!(b.f.value("D2"), n(2.0), "Sheet2..Sheet4 lost Sheet3");
    b.f.apply(vec![Op::AddSheet {
        name: None,
        index: Some(0),
    }]);
    assert_eq!(b.f.value("D1"), n(3.0));
    assert_eq!(b.f.value("D3"), n(5.0));
    b.f.apply(vec![Op::RenameSheet {
        sheet: b.s[3],
        name: "Four".into(),
    }]);
    assert_eq!(b.f.value("D3"), e(ErrorKind::NA));
}

/// A 3D range built by `:` at run time (through a name) follows sheets
/// moved or inserted into it (the span is a dynamic dependency).
#[test]
fn range_operator_spans_follow_sheet_moves_and_inserts() {
    let mut b = Book::new();
    b.f.apply(vec![Op::DefineName {
        name: "qq".into(),
        scope: None,
        refers_to: "$Sheet1.$A$1".into(),
    }]);
    assert_eq!(b.eval("C3", "=SUM(qq:$Sheet3.A1)"), n(60.0));
    b.f.apply(vec![Op::MoveSheet {
        sheet: b.s[3],
        to: 1,
    }]);
    assert_eq!(b.f.value("C3"), n(100.0));
    let r = b.f.apply(vec![Op::AddSheet {
        name: Some("X".into()),
        index: Some(1),
    }]);
    let x = r.info.unwrap().sheets[1].id;
    b.f.apply(vec![input(x, "A1", "100")]);
    assert_eq!(b.f.value("C3"), n(200.0));
    b.f.apply(vec![input(x, "A1", "7")]);
    assert_eq!(b.f.value("C3"), n(107.0));
    // INDIRECT's 3D reference is one dynamic span too.
    assert_eq!(
        b.eval("C4", "=SUM(INDIRECT(\"Sheet1.A1:Sheet2.A1\"))"),
        n(10.0 + 7.0 + 40.0 + 20.0)
    );
    let wb = &b.f.svc.document(b.f.doc).unwrap().wb;
    let (_, fc) = wb
        .formula_at(CellAddr::from_a1(b.f.sheet, "C4").unwrap())
        .unwrap();
    assert_eq!(fc.dynamic_spans().len(), 1);
    assert!(fc.dynamic_deps.is_empty());
}

// ----- references to sheets that appear later (L4) -----

#[test]
fn copy_sheet_and_rename_resolve_references_typed_before_the_sheet() {
    let mut b = Book::new();
    assert_eq!(b.eval("D1", "=SUM($S6.A1:$Sheet2.A1)"), e(ErrorKind::Ref));
    assert_eq!(b.eval("D2", "=S6.A1"), e(ErrorKind::Ref));
    // CopySheet of Sheet4 (A1 = 40) named "S6", after Sheet4.
    let r = b.f.apply(vec![Op::CopySheet {
        sheet: b.s[3],
        name: Some("S6".into()),
        index: None,
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(b.f.value("D2"), n(40.0));
    // S6 (tab 4) .. Sheet2 (tab 1): Sheet2, Sheet3, Sheet4, S6.
    assert_eq!(b.f.value("D1"), n(20.0 + 30.0 + 40.0 + 40.0));
    b.f.svc.undo(b.f.doc).unwrap();
    assert_eq!(b.f.value("D2"), e(ErrorKind::Ref));
    // Renaming a sheet to that name resolves them too.
    b.f.apply(vec![Op::RenameSheet {
        sheet: b.s[2],
        name: "S6".into(),
    }]);
    assert_eq!(b.f.value("D2"), n(30.0));
    b.f.svc.undo(b.f.doc).unwrap();
    assert_eq!(b.f.value("D2"), e(ErrorKind::Ref));
}

#[test]
fn undoing_a_sheet_deletion_resolves_references_typed_meanwhile() {
    let mut b = Book::new();
    b.f.apply(vec![Op::DeleteSheet { sheet: b.s[3] }]);
    assert_eq!(b.eval("D1", "=Sheet4.A1+1"), e(ErrorKind::Ref));
    b.f.svc.undo(b.f.doc).unwrap(); // the formula
    b.eval("D1", "=Sheet4.A1+1");
    // Undo the formula, then the deletion: no stale state either way.
    b.f.svc.undo(b.f.doc).unwrap();
    b.f.svc.undo(b.f.doc).unwrap();
    assert_eq!(b.f.value_on(b.s[3], "A1"), n(40.0));
    // A formula typed while the sheet is gone, then the deletion undone.
    b.f.svc.redo(b.f.doc).unwrap();
    let fixture_sheet = b.f.sheet;
    let op = input(fixture_sheet, "E1", "=Sheet4.A1*2");
    let r = b.f.svc.apply(b.f.doc, common::tx(vec![op])).unwrap();
    assert!(r.errors.is_empty());
    assert_eq!(b.f.value("E1"), e(ErrorKind::Ref));
    // Undo E1, undo the deletion, redo E1: the text resolves on entry.
    b.f.svc.undo(b.f.doc).unwrap();
    b.f.svc.undo(b.f.doc).unwrap();
    assert_eq!(b.f.value_on(b.s[3], "A1"), n(40.0));
}

#[test]
fn names_resolve_when_their_sheet_appears() {
    let mut b = Book::new();
    let r = b.f.apply(vec![Op::DefineName {
        name: "qqa".into(),
        scope: None,
        refers_to: "$'S1_2'.$A$1:$B$3".into(),
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(b.eval("D1", "=SUM(qqa)"), e(ErrorKind::Ref));
    let r = b.f.apply(vec![Op::AddSheet {
        name: Some("S1_2".into()),
        index: None,
    }]);
    let new = r.info.unwrap().sheets.last().unwrap().id;
    assert_eq!(b.f.value("D1"), n(0.0));
    b.f.apply(vec![input(new, "B2", "7")]);
    assert_eq!(b.f.value("D1"), n(7.0));
    // Undo the edit and the new sheet: the name is unresolved again.
    b.f.svc.undo(b.f.doc).unwrap();
    b.f.svc.undo(b.f.doc).unwrap();
    assert_eq!(b.f.value("D1"), e(ErrorKind::Ref));
}

#[test]
fn pasting_a_formula_whose_sheet_was_deleted_is_ref_error() {
    use tally_api::PastePayload;
    let mut b = Book::new();
    b.set_on(2, "D8", "5");
    b.set_on(2, "C2", "=SUM($'Sheet3'.D8:$'Sheet3'.D8)");
    let clip = b.f.svc.copy(b.f.doc, &common::sel(b.s[2], "C2")).unwrap();
    b.f.apply(vec![Op::DeleteSheet { sheet: b.s[2] }]);
    b.f.apply(vec![Op::Paste {
        sheet: b.f.sheet,
        target: common::range("H1"),
        payload: PastePayload::Internal { token: clip.token },
    }]);
    assert_eq!(b.formula("H1"), "=SUM(#REF!)");
    assert_eq!(b.f.value("H1"), e(ErrorKind::Ref));
}

/// Cycles through INDIRECT, OFFSET and VLOOKUP give the same result
/// whether built edit by edit or loaded at once (as AOO, which detects
/// them while interpreting).
#[test]
fn dynamic_cycles_agree_between_incremental_and_reload() {
    use tally_engine::recalc::recalc;
    use tally_engine::Workbook;
    let cases: &[&[(&str, &str)]] = &[
        &[("A1", "=INDIRECT(\"B1\")+1"), ("B1", "=A1+1")],
        &[("B1", "=A1+1"), ("A1", "=INDIRECT(\"B1\")+1")],
        &[("A1", "=OFFSET(A1;0;1)+1"), ("B1", "=A1+1")],
        &[("A1", "=VLOOKUP(1;A1:B2;2;0)"), ("A2", "1")],
        &[("C1", "=INDIRECT(\"C1\")")],
        &[
            ("A1", "=INDIRECT(\"B1\")"),
            ("B1", "=A1"),
            ("B1", "5"),
            ("C1", "=A1+1"),
        ],
        &[
            ("D1", "=C1"),
            ("C1", "=A1+1"),
            ("A1", "=INDIRECT(\"B1\")"),
            ("B1", "=C1"),
            ("B1", "2"),
        ],
        &[("B2", "=A1"), ("A2", "1"), ("A1", "=VLOOKUP(1;A2:B3;2;0)")],
        &[
            ("A1", "=SUM(OFFSET(B1;0;0;2;1))"),
            ("B2", "=A1"),
            ("B1", "3"),
        ],
        &[
            ("A1", "=IF(B1>0;INDIRECT(\"C1\");0)"),
            ("C1", "=A1"),
            ("B1", "1"),
            ("B1", "0"),
        ],
    ];
    for cells in cases {
        let mut f = Fixture::new();
        for (a, t) in *cells {
            f.set(a, t);
        }
        let mut wb = Workbook::from_data(&f.data());
        recalc(&mut wb);
        let s = wb.sheets[0].id;
        for (a, _) in *cells {
            let reload = wb.value(CellAddr::from_a1(s, a).unwrap());
            assert_eq!(f.value(a), reload, "{a} in {cells:?}");
        }
    }
}
