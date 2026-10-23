//! The ExcelA1 (OOXML) dialect: canonical identity, ExcelA1 ↔ CalcA1 ↔
//! OpenFormula round trips preserving the AST, Excel-only syntax (space
//! intersection, `(A1,B1)` union, `{1,2;3,4}` arrays, 3D references,
//! `_xlfn.` prefixes) and the OOXML function spelling via
//! `Resolver::function_name_in`.

#![allow(clippy::unwrap_used)] // Test code may unwrap.

mod common;

use std::sync::Arc;

use common::{a1, calc, sx};
use tally_formula::{
    parse, print, Dialect, Expr, FuncId, FuncRef, NameId, Resolver, SimpleResolver,
};
use tally_types::SheetId;

/// Sheets: 0 Sheet1, 1 Sheet2, 2 "My Sheet", 3 "It's", 4 Data, 5 Jan,
/// 6 Dec, 7 "Q1-2026", 8 "2020", 9 "R1C1", 10 "Sales.EU".
fn resolver() -> SimpleResolver {
    let mut r = common::resolver();
    r.add_sheet(5, "Jan");
    r.add_sheet(6, "Dec");
    r.add_sheet(7, "Q1-2026");
    r.add_sheet(8, "2020");
    r.add_sheet(9, "R1C1");
    r.add_sheet(10, "Sales.EU");
    r.add_function("IFS");
    r.add_function("XLOOKUP");
    r.add_function("SORT");
    r
}

/// A resolver that spells some functions differently in OOXML, as the
/// engine's registry does (`ooxml_name`).
struct OoxmlResolver(SimpleResolver);

impl Resolver for OoxmlResolver {
    fn sheet_by_name(&self, name: &str) -> Option<SheetId> {
        self.0.sheet_by_name(name)
    }
    fn sheet_name(&self, id: SheetId) -> Option<Arc<str>> {
        self.0.sheet_name(id)
    }
    fn name(&self, name: &str, sheet: SheetId) -> Option<NameId> {
        self.0.name(name, sheet)
    }
    fn function(&self, name: &str) -> Option<FuncId> {
        self.0.function(name)
    }
    fn name_text(&self, id: NameId) -> Option<Arc<str>> {
        self.0.name_text(id)
    }
    fn function_name(&self, id: FuncId) -> Option<Arc<str>> {
        self.0.function_name(id)
    }
    fn function_name_in(&self, id: FuncId, dialect: Dialect) -> Option<Arc<str>> {
        let n = self.0.function_name(id)?;
        if dialect.is_excel() && matches!(&*n, "IFS" | "XLOOKUP" | "IFNA") {
            return Some(Arc::from(format!("_xlfn.{n}")));
        }
        if dialect.is_excel() && &*n == "SORT" {
            return Some(Arc::from("_xlfn._xlws.SORT"));
        }
        if dialect.is_open_formula() && &*n == "IFS" {
            return Some(Arc::from("COM.MICROSOFT.IFS"));
        }
        Some(n)
    }
}

fn excel() -> Dialect {
    Dialect::excel_a1()
}

/// Canonical ExcelA1 text, parsed and printed at C3.
const CANONICAL: &[&str] = &[
    // literals
    "=1",
    "=0",
    "=1.5",
    "=0.1",
    "=1E+20",
    "=1.5E-7",
    "=\"\"",
    "=\"abc\"",
    "=\"a\"\"b\"",
    "=\"héllo\"",
    "=TRUE",
    "=FALSE",
    "=#REF!",
    "=#N/A",
    "=#NAME?",
    "=#DIV/0!",
    "=#VALUE!",
    "=#NUM!",
    "=#NULL!",
    // percent
    "=10%",
    "=50%%",
    "=A1%",
    "=-50%",
    "=2^50%",
    // references
    "=A1",
    "=C3",
    "=$A$1",
    "=$A1",
    "=A$1",
    "=XFD1048576",
    "=A1:B2",
    "=$A$1:$B$2",
    "=A:A",
    "=$A:$C",
    "=1:1",
    "=$1:$3",
    "=Sheet1!A1",
    "=Sheet2!B2",
    "=Sheet2!$B$2",
    "='My Sheet'!A1",
    "='It''s'!A1",
    "='My Sheet'!$A$1:$C$3",
    "=Sheet2!A1:B2",
    "=Sheet2!A:A",
    "=Sheet2!1:2",
    "='Q1-2026'!A1",
    "='2020'!A1",
    "='R1C1'!A1",
    "='Sales.EU'!A1",
    "=Nowhere!A1",
    "='Missing Sheet'!A1",
    // 3D references
    "=SUM(Jan:Dec!B2)",
    "=SUM(Jan:Dec!B2:C3)",
    "=SUM('Sheet1:My Sheet'!A1)",
    // operators
    "=1+2",
    "=1-2",
    "=2*3",
    "=6/3",
    "=2^3",
    "=\"a\"&\"b\"",
    "=1=1",
    "=1<>2",
    "=1<2",
    "=1<=2",
    "=1>2",
    "=1>=2",
    "=A1>=B1",
    "=A1<>\"\"",
    "=1+2*3",
    "=(1+2)*3",
    "=2^3^2",
    "=-2^2",
    "=--1",
    "=-A1",
    "=1+-2*3",
    // intersection and union
    "=A1:B2 B1:C3",
    "=SUM(A1:B2 B1:C3)",
    "=SUM((A1,B1))",
    "=SUM((A1:A3,C1:C3,E1))",
    "=(A1:B2 B2:C3,D4)",
    "=SUM(Sheet2!A1:B2 Sheet2!B1:C2)",
    // functions
    "=SUM(1)",
    "=SUM()",
    "=SUM(A1:B2,3)",
    "=IF(A1,,1)",
    "=IF(,,)",
    "=NOW()",
    "=FOO(1)",
    "=ERROR.TYPE(#N/A)",
    "=LOG10(100)",
    "=IF(A1>0,\"pos\",IF(A1<0,\"neg\",\"zero\"))",
    "=VLOOKUP(A1,Sheet2!A1:B10,2,FALSE)",
    "=SUMIF(G2:G15,\"<>Paid\",F2:F15)",
    "=ROUND(A1*1.2,2)",
    "=AVERAGE(A:A)",
    "=SUM(1:1)",
    "=MAX(A1:A10)-MIN(A1:A10)",
    "=IFERROR(1/0,\"x\")",
    "=IFS(A1>1,1,TRUE,0)",
    "=XLOOKUP(A1,B:B,C:C)",
    "=_xlfn.NEWFUNC(1,2)",
    // arrays
    "={1}",
    "={1,2,3}",
    "={1;2;3}",
    "={1,2;3,4}",
    "={-1,2,3}",
    "={\"a\",TRUE;#N/A,1.5}",
    "=SUM({1,2;3,4})*2",
    "=INDEX({1,2;3,4},2,1)",
    // names
    "=MyRange",
    "=Tax",
    "=MyRange*Tax",
    "=Unknown",
    "=SUM(MyRange)",
];

#[test]
fn excel_identity() {
    assert!(CANONICAL.len() >= 80, "{}", CANONICAL.len());
    let r = resolver();
    for src in CANONICAL {
        let f = parse(src, a1("C3"), excel(), &r).unwrap_or_else(|e| panic!("{src}: {e}"));
        assert_eq!(print(&f, a1("C3"), excel(), &r), *src, "{src}");
    }
}

#[test]
fn excel_calc_open_formula_round_trip() {
    let r = resolver();
    for src in CANONICAL {
        let f = parse(src, a1("C3"), excel(), &r).unwrap();
        for d in [calc(), Dialect::open_formula()] {
            let text = print(&f, a1("C3"), d, &r);
            let g =
                parse(&text, a1("C3"), d, &r).unwrap_or_else(|e| panic!("{src} -> {text}: {e}"));
            // CalcA1 uppercases unknown function names (`_XLFN.NEWFUNC`).
            assert_eq!(
                sx(&f.expr).to_ascii_uppercase(),
                sx(&g.expr).to_ascii_uppercase(),
                "{src} -> {text}"
            );
            let back = print(&g, a1("C3"), excel(), &r);
            assert_eq!(back, *src, "{src} -> {text} -> {back}");
        }
    }
}

#[test]
fn calc_to_excel_examples() {
    let r = resolver();
    let cases = [
        ("=SUM(Sheet2.A1:B2,3)", "=SUM(Sheet2!A1:B2,3)"),
        ("=$Sheet2.$A$1", "=Sheet2!$A$1"),
        ("=$'My Sheet'.A1:B2", "='My Sheet'!A1:B2"),
        ("=Sheet1.A1:Sheet2.B2", "=Sheet1:Sheet2!A1:B2"),
        ("={1;2|3;4}", "={1,2;3,4}"),
        ("=A1:B2!B1:C3", "=A1:B2 B1:C3"),
        ("=A1~B1", "=(A1,B1)"),
        ("=SUM(A1~B1~C1)", "=SUM((A1,B1,C1))"),
        ("=SUM((A1~B1))", "=SUM((A1,B1))"),
        ("=Err:502", "=#VALUE!"),
        ("=Err:522+1", "=#VALUE!+1"),
        ("=(MyRange)!A1", "=(MyRange) A1"),
        ("=MyRange!A1", "=MyRange!A1"),
        ("=TRUE()", "=TRUE"),
        ("='R1C1'.A1", "='R1C1'!A1"),
    ];
    for (src, want) in cases {
        let f = parse(src, a1("C3"), calc(), &r).unwrap_or_else(|e| panic!("{src}: {e}"));
        assert_eq!(print(&f, a1("C3"), excel(), &r), want, "{src}");
    }
}

#[test]
fn open_formula_to_excel_examples() {
    let r = resolver();
    let cases = [
        ("of:=SUM([.A1:.B2];3)", "=SUM(A1:B2,3)"),
        ("of:=[$Sheet2.$A$1:.$B$10]", "=Sheet2!$A$1:$B$10"),
        ("of:=[.A$1:.A$1048576]", "=A:A"),
        ("of:={1;2|3;4}", "={1,2;3,4}"),
        ("of:=TRUE()", "=TRUE"),
        ("of:=[.#REF!]", "=#REF!"),
        ("of:=[.A1:.B2]![.B1:.C3]", "=A1:B2 B1:C3"),
    ];
    for (src, want) in cases {
        let f = parse(src, a1("C3"), Dialect::open_formula(), &r)
            .unwrap_or_else(|e| panic!("{src}: {e}"));
        assert_eq!(print(&f, a1("C3"), excel(), &r), want, "{src}");
    }
}

#[test]
fn excel_leniency_and_normalisation() {
    let r = resolver();
    let cases = [
        ("= 1 + 2 ", "=1+2"),
        ("=SUM( A1 , B1 )", "=SUM(A1,B1)"),
        ("=sum(a1:b2)", "=SUM(A1:B2)"),
        ("=sheet2!a1", "=Sheet2!A1"),
        ("=true", "=TRUE"),
        ("=TRUE()", "=TRUE"),
        ("=#n/a", "=#N/A"),
        ("1+2", "=1+2"),
        ("=A1  B1", "=A1 B1"),
        // (CalcA1 needs `(MyRange)!A1:B2` here, so this is not in CANONICAL.)
        ("=MyRange A1:B2", "=MyRange A1:B2"),
        ("=SUM(A1:B2  B1:C3 )", "=SUM(A1:B2 B1:C3)"),
        ("=(A1:B2) (B1:C3)", "=(A1:B2) (B1:C3)"),
        ("={1, 2; 3, 4}", "={1,2;3,4}"),
        ("={-1,+2,--3}", "={-1,2,3}"),
        ("=A1 :B2", "=A1:B2"),
        // As in Excel, a space before `(` is intersection, not a call.
        ("=SUM (1)", "=SUM (1)"),
        ("=MyRange (A1:B2)", "=MyRange (A1:B2)"),
        ("=_xlfn.IFS(A1>1,1,TRUE,0)", "=IFS(A1>1,1,TRUE,0)"),
        ("=_XLFN.IFS(1,2)", "=IFS(1,2)"),
        ("=_xlfn._xlws.SORT(A1:A3)", "=SORT(A1:A3)"),
        ("=_xlfn.newfunc(1)", "=_xlfn.NEWFUNC(1)"),
        ("=_xlfn._xlws.FILTERX(A1:A3)", "=_xlfn._xlws.FILTERX(A1:A3)"),
        ("=jan:dec!b2", "=Jan:Dec!B2"),
        ("=Jan:Dec!B2:B2", "=Jan:Dec!B2"),
        ("='Jan:Dec'!B2", "=Jan:Dec!B2"),
        ("=Sheet2!A1:Sheet2!B2", "=Sheet2!A1:B2"),
        ("=Sales.EU!A1", "='Sales.EU'!A1"),
        ("=SUM(Sales.EU!A1:B2)", "=SUM('Sales.EU'!A1:B2)"),
        ("=Sheet1!#REF!", "=#REF!"),
        ("='My Sheet'!#REF!+1", "=#REF!+1"),
        // Calc-style input is still accepted.
        ("=Sheet2.A1", "=Sheet2!A1"),
        ("=SUM(A1;B1)", "=SUM(A1,B1)"),
    ];
    for (src, want) in cases {
        let f = parse(src, a1("C3"), excel(), &r).unwrap_or_else(|e| panic!("{src}: {e}"));
        assert_eq!(print(&f, a1("C3"), excel(), &r), want, "{src}");
    }
}

#[test]
fn excel_specific_asts() {
    let r = resolver();
    let ast = |src: &str| sx(&parse(src, a1("A1"), excel(), &r).unwrap().expr);
    assert_eq!(
        ast("=A1:B2 B1:C3"),
        "(isect (area R[0]C[0] R[1]C[1]) (area R[0]C[1] R[2]C[2]))"
    );
    assert_eq!(
        ast("=(A1,B1)"),
        "(paren (union (cell R[0]C[0]) (cell R[0]C[1])))"
    );
    assert_eq!(ast("={1,2;3,4}"), "{2x2 1.0 2.0 3.0 4.0}");
    assert_eq!(ast("={1;2}"), "{2x1 1.0 2.0}");
    assert_eq!(ast("=Jan:Dec!B2"), "(area #5!R[1]C[1] #6!R[1]C[1])");
    // Without whitespace, `A1 B1` adjacency is an error as in CalcA1.
    assert!(parse("=1 2", a1("A1"), excel(), &r).is_err());
    assert!(parse("=\"a\" \"b\"", a1("A1"), excel(), &r).is_err());
    // Union commas only apply directly inside parentheses.
    assert_eq!(
        ast("=SUM(A1,B1)"),
        "(call #0 (cell R[0]C[0]) (cell R[0]C[1]))"
    );
    // Structured references are not supported: they fail to parse (the
    // XLSX reader keeps such formulas as text).
    assert!(parse("=SUM(Table1[Col])", a1("A1"), excel(), &r).is_err());
    // CalcA1 keeps rejecting space intersections.
    assert!(parse("=A1:B2 B1:C3", a1("A1"), calc(), &r).is_err());
}

#[test]
fn future_function_prefixes() {
    let r = resolver();
    // Known: the prefix is stripped and the function resolved.
    let f = parse("=_xlfn.XLOOKUP(1,A:A,B:B)", a1("A1"), excel(), &r).unwrap();
    match &f.expr {
        Expr::Call {
            func: FuncRef::Known(id),
            ..
        } => assert_eq!(r.function_name(*id).as_deref(), Some("XLOOKUP")),
        other => panic!("{other:?}"),
    }
    // Unknown: kept (lowercase prefix) so it prints back unchanged.
    let f = parse("=_XLFN.Something(1)", a1("A1"), excel(), &r).unwrap();
    match &f.expr {
        Expr::Call {
            func: FuncRef::Unknown(n),
            ..
        } => assert_eq!(&**n, "_xlfn.SOMETHING"),
        other => panic!("{other:?}"),
    }
    // Prefixes are only special in ExcelA1.
    let f = parse("=_xlfn.IFS(1,2)", a1("A1"), calc(), &r).unwrap();
    assert!(matches!(
        &f.expr,
        Expr::Call {
            func: FuncRef::Unknown(_),
            ..
        }
    ));
    // `_xlfn.TRUE()` still folds to the constant.
    let f = parse("=_xlfn.TRUE()", a1("A1"), excel(), &r).unwrap();
    assert_eq!(f.expr, Expr::Bool(true));
}

#[test]
fn ooxml_spelling_comes_from_the_resolver() {
    let r = OoxmlResolver(resolver());
    let cases = [
        ("=IFS(A1>1,1,TRUE,0)", "=_xlfn.IFS(A1>1,1,TRUE,0)"),
        ("=XLOOKUP(1,A:A,B:B)", "=_xlfn.XLOOKUP(1,A:A,B:B)"),
        ("=SORT(A1:A3)", "=_xlfn._xlws.SORT(A1:A3)"),
        ("=SUM(1)", "=SUM(1)"),
        ("=IFNA(1/0,2)", "=_xlfn.IFNA(1/0,2)"),
    ];
    for (src, want) in cases {
        let f = parse(src, a1("A1"), calc(), &r).unwrap();
        let x = print(&f, a1("A1"), excel(), &r);
        assert_eq!(x, want, "{src}");
        // ... and parses back to the same function.
        let g = parse(&x, a1("A1"), excel(), &r).unwrap();
        assert_eq!(f, g, "{x}");
    }
    let f = parse("=_xlfn.IFS(1,2)", a1("A1"), excel(), &r).unwrap();
    assert_eq!(
        print(&f, a1("A1"), Dialect::open_formula(), &r),
        "of:=COM.MICROSOFT.IFS(1;2)"
    );
}

#[test]
fn relocation_in_excel() {
    // Shared formulas in XLSX are re-anchored by printing at another origin.
    let r = resolver();
    let f = parse("=A1*$B$1+Sheet2!C1", a1("D1"), excel(), &r).unwrap();
    assert_eq!(print(&f, a1("D5"), excel(), &r), "=A5*$B$1+Sheet2!C5");
    assert_eq!(print(&f, a1("F1"), excel(), &r), "=C1*$B$1+Sheet2!E1");
    assert_eq!(
        print(&f, a1("D5"), Dialect::open_formula(), &r),
        "of:=[.A5]*[.$B$1]+[Sheet2.C5]"
    );
}

#[test]
fn deleted_sheets_print_ref_in_excel() {
    let mut r = resolver();
    let f = parse("=Sheet2!A1+SUM(Jan:Dec!A1)", a1("A1"), excel(), &r).unwrap();
    r.remove_sheet(1);
    r.remove_sheet(6);
    assert_eq!(print(&f, a1("A1"), excel(), &r), "=#REF!+SUM(#REF!)");
}
