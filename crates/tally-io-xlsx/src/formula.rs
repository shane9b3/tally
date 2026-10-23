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

/// Marks a sheet placeholder written by [`protect_colon_sheets`]
/// (a private-use character, never in a real formula).
const SHEET_PLACEHOLDER: char = '\u{E000}';

impl XlsxResolver {
    fn index_of(&self, name: &str) -> Option<usize> {
        if let Some(i) = name.strip_prefix(SHEET_PLACEHOLDER) {
            return i.parse::<usize>().ok().filter(|i| *i < self.lookup.len());
        }
        self.lookup
            .iter()
            .position(|n| n.eq_ignore_ascii_case(name))
    }
}

/// Quoted sheet names that contain `:` and name an existing sheet
/// (`'a:b'!A1`) are replaced by a placeholder before the ExcelA1 parser
/// sees them, which would read them as a 3-D span `a` to `b` (review 2,
/// LOW-2). Quote-aware: string literals and `''` escapes are honoured; a
/// quoted span that is not a sheet name (`'Jan:Mar'!A1` over sheets Jan
/// and Mar) is left alone.
fn protect_colon_sheets<'t>(text: &'t str, r: &XlsxResolver) -> std::borrow::Cow<'t, str> {
    if !text.contains('\'') || !r.lookup.iter().any(|n| n.contains(':')) {
        return std::borrow::Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut chars = text.char_indices().peekable();
    let mut in_string = false;
    while let Some((_, c)) = chars.next() {
        if in_string {
            out.push(c);
            if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => {
                in_string = true;
                out.push(c);
            }
            '\'' => {
                let mut name = String::new();
                let mut closed = false;
                while let Some((_, d)) = chars.next() {
                    if d == '\'' {
                        if chars.peek().is_some_and(|(_, n)| *n == '\'') {
                            chars.next();
                            name.push('\'');
                            continue;
                        }
                        closed = true;
                        break;
                    }
                    name.push(d);
                }
                let bang = chars.peek().is_some_and(|(_, n)| *n == '!');
                let idx = (closed && bang && name.contains(':'))
                    .then(|| r.lookup.iter().position(|n| n.eq_ignore_ascii_case(&name)))
                    .flatten();
                match idx {
                    Some(i) => {
                        out.push('\'');
                        out.push(SHEET_PLACEHOLDER);
                        out.push_str(&i.to_string());
                        out.push('\'');
                    }
                    None => {
                        out.push('\'');
                        out.push_str(&name.replace('\'', "''"));
                        if closed {
                            out.push('\'');
                        }
                    }
                }
            }
            _ => out.push(c),
        }
    }
    std::borrow::Cow::Owned(out)
}

impl Resolver for XlsxResolver {
    fn sheet_by_name(&self, name: &str) -> Option<SheetId> {
        self.index_of(name).and_then(|i| SheetId::try_from(i).ok())
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

/// Which spelling wins when a function name is looked up while parsing.
#[derive(Copy, Clone, PartialEq, Eq)]
enum Lookup {
    /// OOXML stored names first (`WEEKNUM` = WEEKNUM_ADD, `CONVERT` =
    /// CONVERT_ADD, `DURATION` = DURATION_ADD), then canonical.
    Ooxml,
    /// ODF stored names first (`ISOWEEKNUM` = the built-in WEEKNUM).
    Odf,
    /// Canonical first (CalcA1 text).
    Canonical,
}

/// [`XlsxResolver`] with a function lookup order for one parse.
struct Parsing<'a> {
    r: &'a XlsxResolver,
    lookup: Lookup,
}

impl Resolver for Parsing<'_> {
    fn sheet_by_name(&self, name: &str) -> Option<SheetId> {
        self.r.sheet_by_name(name)
    }

    fn sheet_name(&self, id: SheetId) -> Option<Arc<str>> {
        self.r.sheet_name(id)
    }

    fn name(&self, name: &str, sheet: SheetId) -> Option<NameId> {
        self.r.name(name, sheet)
    }

    fn function(&self, name: &str) -> Option<FuncId> {
        let reg = tally_functions::registry();
        match self.lookup {
            Lookup::Ooxml => reg.lookup_ooxml_first(name),
            Lookup::Odf => reg.lookup_odf_first(name),
            Lookup::Canonical => reg.lookup_any(name),
        }
        .map(|id| FuncId(id.0))
    }

    fn function_name(&self, id: FuncId) -> Option<Arc<str>> {
        self.r.function_name(id)
    }

    fn function_name_in(&self, id: FuncId, dialect: Dialect) -> Option<Arc<str>> {
        self.r.function_name_in(id, dialect)
    }
}

/// The Excel name under which AOO's built-in WEEKNUM (Sunday or Monday
/// start, week 1 = first week with ≥ 4 days) is exported when it has no
/// exact Excel equivalent: Excel shows `#NAME?`, Tally (and LibreOffice,
/// which uses the same `ORG.OPENOFFICE.` convention) reads it back.
const AOO_WEEKNUM_XLSX: &str = "_xlfn.ORG.OPENOFFICE.WEEKNUM";

fn builtin_weeknum() -> Option<tally_functions::FuncId> {
    tally_functions::registry().lookup("WEEKNUM")
}

/// Export rewrite of the built-in WEEKNUM. With a literal mode other than
/// 1 (Monday start, ≥ 4 days) it is exactly ISO 8601 and becomes Excel's
/// one-argument `ISOWEEKNUM`; otherwise it keeps AOO's semantics under
/// [`AOO_WEEKNUM_XLSX`].
fn export_weeknum(e: &mut Expr) {
    let Some(wk) = builtin_weeknum() else {
        return;
    };
    let Expr::Call { func, args } = e else {
        return;
    };
    if *func != FuncRef::Known(FuncId(wk.0)) {
        return;
    }
    let mode = args.get(1).and_then(|m| match m {
        Expr::Num(n) => Some(*n),
        _ => None,
    });
    match (mode, tally_functions::registry().lookup("ISOWEEKNUM")) {
        (Some(m), Some(iso)) if m.trunc() != 1.0 && !args.is_empty() => {
            *func = FuncRef::Known(FuncId(iso.0));
            args.truncate(1);
        }
        _ => *func = FuncRef::Unknown(Arc::from(AOO_WEEKNUM_XLSX)),
    }
}

/// The Excel name of Calc's euro `CONVERT` (ODF `ORG.OPENOFFICE.CONVERT`),
/// which has no Excel equivalent: written as an unknown function so it
/// does not reload as Excel's unit `CONVERT` (= Calc `CONVERT_ADD`)
/// (correctness review 3, M10).
const AOO_CONVERT_XLSX: &str = "_xlfn.ORG.OPENOFFICE.CONVERT";

fn calc_convert() -> Option<tally_functions::FuncId> {
    tally_functions::registry().lookup("CONVERT")
}

/// Export rewrite of Calc's euro `CONVERT` to [`AOO_CONVERT_XLSX`].
fn export_convert(e: &mut Expr) {
    let Some(cv) = calc_convert() else {
        return;
    };
    if let Expr::Call { func, .. } = e {
        if *func == FuncRef::Known(FuncId(cv.0)) {
            *func = FuncRef::Unknown(Arc::from(AOO_CONVERT_XLSX));
        }
    }
}

/// Import rewrite: [`AOO_CONVERT_XLSX`] back to Calc's euro `CONVERT`.
fn import_convert(e: &mut Expr) {
    if let Expr::Call {
        func: func @ FuncRef::Unknown(_),
        ..
    } = e
    {
        let FuncRef::Unknown(n) = &*func else {
            return;
        };
        let bare = n.strip_prefix("_xlfn.").unwrap_or(n);
        if bare.eq_ignore_ascii_case("ORG.OPENOFFICE.CONVERT") {
            if let Some(cv) = calc_convert() {
                *func = FuncRef::Known(FuncId(cv.0));
            }
        }
    }
}

/// Import rewrite: [`AOO_WEEKNUM_XLSX`] back to the built-in WEEKNUM.
fn import_weeknum(e: &mut Expr) {
    if let Expr::Call {
        func: func @ FuncRef::Unknown(_),
        ..
    } = e
    {
        let FuncRef::Unknown(n) = &*func else {
            return;
        };
        let bare = n.strip_prefix("_xlfn.").unwrap_or(n);
        if bare.eq_ignore_ascii_case("ORG.OPENOFFICE.WEEKNUM") {
            if let Some(wk) = builtin_weeknum() {
                *func = FuncRef::Known(FuncId(wk.0));
            }
        }
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
    let text = protect_colon_sheets(text, r);
    let mut src = String::with_capacity(text.len() + 1);
    src.push('=');
    src.push_str(&text);
    let mut f = parse(
        &src,
        origin,
        Dialect::excel_a1(),
        &Parsing {
            r,
            lookup: Lookup::Ooxml,
        },
    )?;
    for_each_expr_mut(&mut f.expr, import_weeknum);
    for_each_expr_mut(&mut f.expr, import_convert);
    Ok(f)
}

/// ExcelA1 formula text (as a BIFF/OOXML file stores it, without `=`)
/// written at `(sheet index, row, col)` of a workbook with `sheet_names`
/// → model OpenFormula text (`of:=…`), or `None` when it does not parse.
/// Used by the `.xls` reader (`tally-io-xls`).
pub fn excel_formula_to_of(
    text: &str,
    sheet_names: &[String],
    sheet: usize,
    row: u32,
    col: u32,
) -> Option<String> {
    if text.len() > 64 * 1024 {
        return None;
    }
    let r = XlsxResolver::new(sheet_names);
    let origin = CellAddr::new(sheet as SheetId, row, col);
    let f = parse_excel(text, origin, &r).ok()?;
    Some(print_of(&f, origin, &r))
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

/// The export rewrites: the built-in WEEKNUM ([`export_weeknum`]) and
/// unknown `COM.MICROSOFT.X` functions as `_xlfn.X`.
fn export_rewrites(f: &mut Formula) {
    for_each_expr_mut(&mut f.expr, export_weeknum);
    for_each_expr_mut(&mut f.expr, export_convert);
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
}

/// Model formula text (OpenFormula `of:=`, legacy `oooc:=`, or a bare `=`
/// CalcA1 formula) → Excel `<f>` text without the leading `=`.
pub(crate) fn of_to_excel(text: &str, origin: CellAddr, r: &XlsxResolver) -> Option<String> {
    let (dialect, lookup) = if text.starts_with('=') {
        (Dialect::calc_a1_en_us(), Lookup::Canonical)
    } else {
        (Dialect::open_formula(), Lookup::Odf)
    };
    let mut f = parse(text, origin, dialect, &Parsing { r, lookup }).ok()?;
    export_rewrites(&mut f);
    let s = print(&f, origin, Dialect::excel_a1(), r);
    Some(s.strip_prefix('=').unwrap_or(&s).to_string())
}

/// Like [`of_to_excel`], for a formula whose relative references were
/// written for `parsed_at` but that Excel reads relative to `printed_at`
/// (conditional formats and validations are relative to the top-left cell
/// of their range): the same offsets, printed at the new origin.
pub(crate) fn of_to_excel_at(
    text: &str,
    parsed_at: CellAddr,
    printed_at: CellAddr,
    r: &XlsxResolver,
) -> Option<String> {
    let (dialect, lookup) = if text.starts_with('=') {
        (Dialect::calc_a1_en_us(), Lookup::Canonical)
    } else {
        (Dialect::open_formula(), Lookup::Odf)
    };
    let mut f = parse(text, parsed_at, dialect, &Parsing { r, lookup }).ok()?;
    export_rewrites(&mut f);
    let s = print(&f, printed_at, Dialect::excel_a1(), r);
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

    /// Excel reads a space before `(` after a name as intersection, never
    /// as a call: `MyRange (A1:B2)` imports as an intersection, and import
    /// then export is a fixed point. (The model text brackets the name,
    /// `(MyRange)!`, so the first export does too.)
    #[test]
    fn name_space_paren_is_intersection() {
        let r = r();
        for (x, of, out) in [
            (
                "MyRange (A1:B2)",
                "of:=(MyRange)!([.A1:.B2])",
                "(MyRange) (A1:B2)",
            ),
            ("SUM(A1)", "of:=SUM([.A1])", "SUM(A1)"),
            ("SUM (A1)", "of:=(SUM)!([.A1])", "(SUM) (A1)"),
            (
                "SUM(MyRange (A1:B2))",
                "of:=SUM((MyRange)!([.A1:.B2]))",
                "SUM((MyRange) (A1:B2))",
            ),
        ] {
            let (got, ok) = excel_to_of(x, at("C3"), &r);
            assert!(ok, "{x}");
            assert_eq!(got, of, "{x}");
            let back = of_to_excel(&got, at("C3"), &r);
            assert_eq!(back.as_deref(), Some(out), "{x}");
            let (again, ok) = excel_to_of(out, at("C3"), &r);
            assert!(ok, "{out}");
            assert_eq!(again, got, "{x}");
            assert_eq!(
                of_to_excel(&again, at("C3"), &r).as_deref(),
                Some(out),
                "{x}"
            );
        }
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
    fn same_name_functions_resolve_by_file_format() {
        let r = r();
        // Excel's CONVERT / DURATION / WEEKNUM are the add-ins.
        let cases = [
            ("CONVERT(1,\"m\",\"ft\")", "of:=CONVERT([.A1]"),
            ("DURATION(1,2,3,4,1)", "of:=DURATION("),
            ("WEEKNUM(A1,2)", "of:=WEEKNUM([.A1];2)"),
        ];
        let reg = tally_functions::registry();
        for (x, _) in cases {
            let f = parse_excel(x, at("B1"), &r).unwrap();
            let Expr::Call {
                func: FuncRef::Known(id),
                ..
            } = &f.expr
            else {
                panic!("{x}");
            };
            let name = reg.get(tally_functions::FuncId(id.0)).unwrap().name;
            assert!(name.ends_with("_ADD"), "{x} resolved to {name}");
        }
        // Round trips: Excel → model → Excel.
        for x in [
            "CONVERT(1,\"m\",\"ft\")",
            "DURATION(A1,A2,0.1,0.1,2)",
            "WEEKNUM(A1,2)",
            "_xlfn.PDURATION(0.1,100,200)",
        ] {
            let (of, ok) = excel_to_of(x, at("B1"), &r);
            assert!(ok, "{x}");
            assert_eq!(of_to_excel(&of, at("B1"), &r).as_deref(), Some(x), "{of}");
        }
        // The model's ODF names: WEEKNUM = the add-in, ISOWEEKNUM = the
        // built-in WEEKNUM (mode 2 is ISO 8601 → Excel ISOWEEKNUM).
        assert_eq!(
            of_to_excel("of:=WEEKNUM([.A1];1)", at("B1"), &r).as_deref(),
            Some("WEEKNUM(A1,1)")
        );
        assert_eq!(
            of_to_excel("of:=ISOWEEKNUM([.A1];2)", at("B1"), &r).as_deref(),
            Some("_xlfn.ISOWEEKNUM(A1)")
        );
        let sunday = of_to_excel("of:=ISOWEEKNUM([.A1];1)", at("B1"), &r).unwrap();
        assert_eq!(sunday, "_xlfn.ORG.OPENOFFICE.WEEKNUM(A1,1)");
        // ... and that reads back as the built-in WEEKNUM.
        let (back, ok) = excel_to_of(&sunday, at("B1"), &r);
        assert!(ok);
        assert_eq!(back, "of:=ISOWEEKNUM([.A1];1)");
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
