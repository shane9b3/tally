//! Formula translation between the ExcelA1 text stored in XLSX and the
//! OpenFormula text stored in the model.
//!
//! Functions resolve through the `tally-functions` registry, so the
//! OpenFormula side gets ODF spellings (`COM.MICROSOFT.IFS`) and the Excel
//! side OOXML spellings (`_xlfn.IFS`). Unknown functions pass through
//! unchanged (an `_xlfn.` prefix on an unknown function is kept, so it
//! round-trips back to Excel); an unknown ODF `COM.MICROSOFT.X` is written
//! to Excel as `_xlfn.X`.

use std::sync::Arc;

use tally_formula::{
    parse, print, Dialect, Expr, Formula, FuncId, FuncRef, NameId, ParseError, RefExpr, Resolver,
    SheetRef,
};
use tally_types::{CellAddr, SheetId};

/// A resolver over the workbook's sheet names and the function registry.
/// Defined names are never resolved (they print as written).
pub(crate) struct XlsxResolver {
    /// Names used to *look up* sheets (as written in formulas).
    lookup: Vec<Arc<str>>,
    /// Names printed for each sheet id (the writer's sanitized Excel names).
    printed: Vec<Arc<str>>,
}

impl XlsxResolver {
    pub fn new(names: &[String]) -> Self {
        let v: Vec<Arc<str>> = names.iter().map(|n| Arc::from(n.as_str())).collect();
        Self {
            lookup: v.clone(),
            printed: v,
        }
    }

    /// Looks sheets up by `lookup` names but prints `printed` names.
    pub fn renaming(lookup: &[String], printed: &[String]) -> Self {
        Self {
            lookup: lookup.iter().map(|n| Arc::from(n.as_str())).collect(),
            printed: printed.iter().map(|n| Arc::from(n.as_str())).collect(),
        }
    }
}

impl Resolver for XlsxResolver {
    fn sheet_by_name(&self, name: &str) -> Option<SheetId> {
        self.lookup
            .iter()
            .position(|n| n.eq_ignore_ascii_case(name))
            .and_then(|i| SheetId::try_from(i).ok())
    }

    fn sheet_name(&self, id: SheetId) -> Option<Arc<str>> {
        self.printed.get(id as usize).cloned()
    }

    fn name(&self, _name: &str, _sheet: SheetId) -> Option<NameId> {
        None
    }

    fn function(&self, name: &str) -> Option<FuncId> {
        tally_functions::registry()
            .lookup_any(name)
            .map(|id| FuncId(id.0))
    }

    fn function_name(&self, id: FuncId) -> Option<Arc<str>> {
        tally_functions::registry()
            .get(tally_functions::FuncId(id.0))
            .map(|d| Arc::from(d.name))
    }

    fn function_name_in(&self, id: FuncId, dialect: Dialect) -> Option<Arc<str>> {
        let d = tally_functions::registry().get(tally_functions::FuncId(id.0))?;
        Some(Arc::from(match dialect {
            Dialect::OpenFormula => d.odf_name,
            Dialect::ExcelA1 => d.ooxml_name.unwrap_or(d.name),
            Dialect::CalcA1 { .. } => d.name,
        }))
    }
}

/// Visits every expression node (iteratively; trees can be deep).
fn for_each_expr_mut(root: &mut Expr, mut f: impl FnMut(&mut Expr)) {
    let mut stack: Vec<&mut Expr> = vec![root];
    while let Some(e) = stack.pop() {
        f(e);
        match e {
            Expr::ArrayLit(a) => stack.extend(a.items.iter_mut()),
            Expr::Call { args, .. } => stack.extend(args.iter_mut()),
            Expr::Unary(_, x) | Expr::Postfix(_, x) | Expr::Paren(x) => stack.push(x),
            Expr::Binary(_, l, r) => {
                stack.push(l);
                stack.push(r);
            }
            _ => {}
        }
    }
}

/// Parses ExcelA1 cell formula text (as stored in `<f>`, without `=`).
pub(crate) fn parse_excel(
    text: &str,
    origin: CellAddr,
    r: &XlsxResolver,
) -> Result<Formula, ParseError> {
    let mut src = String::with_capacity(text.len() + 1);
    src.push('=');
    src.push_str(text);
    parse(&src, origin, Dialect::excel_a1(), r)
}

/// Prints a parsed formula as model OpenFormula text (`of:=...`).
pub(crate) fn print_of(f: &Formula, origin: CellAddr, r: &XlsxResolver) -> String {
    print(f, origin, Dialect::open_formula(), r)
}

/// ExcelA1 cell formula text → OpenFormula text. On a parse failure the
/// original text is kept behind `of:=` (the engine then reports Err:5xx).
pub(crate) fn excel_to_of(text: &str, origin: CellAddr, r: &XlsxResolver) -> (String, bool) {
    match parse_excel(text, origin, r) {
        Ok(f) => (print_of(&f, origin, r), true),
        Err(_) => (format!("of:={text}"), false),
    }
}

/// ExcelA1 defined-name text → OpenFormula text with absolute sheet
/// markers (`of:=[$Sheet1.$A$1:.$B$10]`), or `None` if it does not parse.
pub(crate) fn excel_name_to_of(text: &str, origin: CellAddr, r: &XlsxResolver) -> Option<String> {
    let mut f = parse_excel(text, origin, r).ok()?;
    for_each_expr_mut(&mut f.expr, |e| {
        if let Expr::Ref(re) = e {
            mark_sheets_abs(re);
        }
    });
    Some(print_of(&f, origin, r))
}

fn mark_sheets_abs(r: &mut RefExpr) {
    let fix = |s: &mut SheetRef| {
        if let SheetRef::Id { abs, .. } = s {
            *abs = true;
        }
    };
    match r {
        RefExpr::Cell(c) => fix(&mut c.sheet),
        RefExpr::Area(a, b) => {
            fix(&mut a.sheet);
            fix(&mut b.sheet);
        }
        RefExpr::Cols { sheet, .. } | RefExpr::Rows { sheet, .. } => fix(sheet),
        RefExpr::Invalid => {}
    }
}

/// Model formula text (OpenFormula `of:=`, legacy `oooc:=`, or a bare `=`
/// CalcA1 formula) → Excel `<f>` text without the leading `=`.
pub(crate) fn of_to_excel(text: &str, origin: CellAddr, r: &XlsxResolver) -> Option<String> {
    let dialect = if text.starts_with('=') {
        Dialect::calc_a1_en_us()
    } else {
        Dialect::open_formula()
    };
    let mut f = parse(text, origin, dialect, r).ok()?;
    for_each_expr_mut(&mut f.expr, |e| {
        if let Expr::Call {
            func: FuncRef::Unknown(n),
            ..
        } = e
        {
            let prefixed = n
                .get(..14)
                .is_some_and(|p| p.eq_ignore_ascii_case("COM.MICROSOFT."));
            if prefixed && n.len() > 14 {
                *n = Arc::from(format!("_xlfn.{}", &n[14..]));
            }
        }
    });
    let s = print(&f, origin, Dialect::excel_a1(), r);
    Some(s.strip_prefix('=').unwrap_or(&s).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r() -> XlsxResolver {
        XlsxResolver::new(&["Sheet1".into(), "My Sheet".into(), "Data".into()])
    }

    fn at(s: &str) -> CellAddr {
        CellAddr::from_a1(0, s).unwrap_or(CellAddr::new(0, 0, 0))
    }

    #[test]
    fn excel_to_open_formula() {
        let r = r();
        let cases = [
            ("SUM(A1:B2,3)", "of:=SUM([.A1:.B2];3)"),
            ("'My Sheet'!A1*2", "of:=['My Sheet'.A1]*2"),
            ("_xlfn.IFNA(A1,0)", "of:=COM.MICROSOFT.IFNA([.A1];0)"),
            ("IFERROR(1/0,\"x\")", "of:=COM.MICROSOFT.IFERROR(1/0;\"x\")"),
            ("_xlfn.NOSUCH(1)", "of:=_xlfn.NOSUCH(1)"),
            ("{1,2;3,4}", "of:={1;2|3;4}"),
            ("A1:B2 B2:C3", "of:=[.A1:.B2]![.B2:.C3]"),
            ("SUM(Sheet1:Data!A1)", "of:=SUM([Sheet1.A1:Data.A1])"),
        ];
        for (x, want) in cases {
            let (got, ok) = excel_to_of(x, at("C3"), &r);
            assert!(ok, "{x}");
            assert_eq!(got, want, "{x}");
        }
        let (bad, ok) = excel_to_of("SUM(Table1[Col])", at("A1"), &r);
        assert!(!ok);
        assert_eq!(bad, "of:=SUM(Table1[Col])");
    }

    #[test]
    fn names() {
        let r = r();
        assert_eq!(
            excel_name_to_of("Sheet1!$A$1:$B$10", at("A1"), &r).as_deref(),
            Some("of:=[$Sheet1.$A$1:.$B$10]")
        );
        assert_eq!(
            excel_name_to_of("'My Sheet'!$C$3", at("A1"), &r).as_deref(),
            Some("of:=[$'My Sheet'.$C$3]")
        );
        assert_eq!(
            excel_name_to_of("Sheet1!#REF!", at("A1"), &r).as_deref(),
            Some("of:=[.#REF!]")
        );
    }

    #[test]
    fn open_formula_to_excel() {
        let r = r();
        let cases = [
            ("of:=SUM([.A1:.B2];3)", "SUM(A1:B2,3)"),
            ("of:=[$'My Sheet'.A1]*2", "'My Sheet'!A1*2"),
            ("of:=COM.MICROSOFT.XYZ(1)", "_xlfn.XYZ(1)"),
            ("of:={1;2|3;4}", "{1,2;3,4}"),
            ("of:=TRUE()", "TRUE"),
            ("=SUM(A1,B1)", "SUM(A1,B1)"),
        ];
        for (of, want) in cases {
            assert_eq!(of_to_excel(of, at("C3"), &r).as_deref(), Some(want), "{of}");
        }
        assert_eq!(of_to_excel("of:=SUM(", at("A1"), &r), None);
    }

    #[test]
    fn registry_spellings() {
        let r = r();
        // The Excel spelling is the registry's `ooxml_name` (e.g. `_xlfn.X`
        // for post-2007 functions), else the canonical name; rust_xlsxwriter
        // adds `_xlfn.` to known future functions that lack it.
        let reg = tally_functions::registry();
        let def = reg
            .def_by_name("IFNA")
            .map(|d| d.ooxml_name.unwrap_or(d.name));
        assert_eq!(
            of_to_excel("of:=COM.MICROSOFT.IFNA([.A1];0)", at("A2"), &r),
            def.map(|n| format!("{n}(A1,0)"))
        );
        assert_eq!(
            of_to_excel("of:=SUM([.A1])", at("A2"), &r).as_deref(),
            Some("SUM(A1)")
        );
    }

    #[test]
    fn renaming_resolver_prints_new_names() {
        let r = XlsxResolver::renaming(&["A/B".into()], &["A_B".into()]);
        assert_eq!(
            of_to_excel("of:=['A/B'.A1]", at("A1"), &r).as_deref(),
            Some("A_B!A1")
        );
    }
}
