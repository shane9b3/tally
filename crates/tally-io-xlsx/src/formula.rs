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
    let dotted = protect_dotted_names(&mut src);
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
    for_each_expr_mut(&mut f.expr, |e| dotted_name(e, &dotted, origin, r));
    Ok(f)
}

/// Excel allows a dot in a defined name, and its sheet references use
/// `!`: `Main.A1` in a formula of an Excel file is a name, never a cell
/// of the sheet `Main`. The formula parser also takes a sheet and a dot
/// in Excel text (for what is typed), so such a token is replaced by a
/// stand-in identifier before parsing; [`dotted_name`] puts the name
/// back. Returns the stand-in prefix and the names, in order.
fn protect_dotted_names(src: &mut String) -> (String, Vec<String>) {
    // Only text with a dot before what could start a cell address.
    let b = src.as_bytes();
    let candidate = (1..b.len().saturating_sub(1)).any(|i| {
        b[i] == b'.'
            && (b[i + 1].is_ascii_alphabetic() || b[i + 1] == b'$')
            && (b[i - 1].is_ascii_alphanumeric()
                || matches!(b[i - 1], b'_' | b'\'')
                || b[i - 1] >= 0x80)
    });
    if !candidate {
        return (String::new(), Vec::new());
    }
    let unquoted = |t: &str, what: char| {
        let mut quoted = false;
        t.chars().any(|c| {
            quoted ^= c == '\'';
            c == what && !quoted
        })
    };
    let spans: Vec<std::ops::Range<usize>> = tally_formula::tokenize(src, Dialect::excel_a1())
        .into_iter()
        .filter(|t| {
            matches!(
                t.kind,
                tally_formula::TokenKind::CellRef | tally_formula::TokenKind::RangeRef
            )
        })
        .map(|t| t.span)
        .filter(|s| {
            src.get(s.clone())
                .is_some_and(|t| unquoted(t, '.') && !unquoted(t, '!'))
        })
        .collect();
    if spans.is_empty() {
        return (String::new(), Vec::new());
    }
    // An identifier the formula does not contain.
    let mut prefix = String::from("zz_dotted_name_");
    while src.to_ascii_lowercase().contains(&prefix) {
        prefix.push('z');
        prefix.push('_');
    }
    let mut names = Vec::new();
    let mut out = String::with_capacity(src.len());
    let mut at = 0;
    for s in spans {
        out.push_str(src.get(at..s.start).unwrap_or_default());
        out.push_str(&format!("{prefix}{}", names.len()));
        names.push(src.get(s.clone()).unwrap_or_default().to_string());
        at = s.end;
    }
    out.push_str(src.get(at..).unwrap_or_default());
    *src = out;
    (prefix, names)
}

/// Puts a name with a dot back for its stand-in
/// ([`protect_dotted_names`]). In the model's formula text the dot is the
/// sheet separator, so a name that reads as a cell of a sheet there
/// (`Main.A1`) cannot be defined (`defined_name_problem`) and cannot be
/// written: the formula gets `#NAME?` in its place, not another cell's
/// value. A dotted name that is a name there too (`Tax.Rate`) stays.
fn dotted_name(e: &mut Expr, dotted: &(String, Vec<String>), origin: CellAddr, r: &XlsxResolver) {
    let Expr::Name(tally_formula::NameRef::Unresolved(t)) = e else {
        return;
    };
    let name = t
        .strip_prefix(dotted.0.as_str())
        .filter(|_| !dotted.0.is_empty())
        .and_then(|i| i.parse::<usize>().ok())
        .and_then(|i| dotted.1.get(i));
    match name {
        Some(name) if tally_formula::defined_name_problem(name, origin, r).is_none() => {
            *e = Expr::Name(tally_formula::NameRef::Unresolved(Arc::from(name.as_str())));
        }
        Some(_) => *e = Expr::Err(tally_types::ErrorKind::Name),
        None => {}
    }
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
    let mut f = parse_excel(text, origin, &r).ok()?;
    for_each_expr_mut(&mut f.expr, |e| biff8_name(e, origin));
    Some(print_of(&f, origin, &r))
}

/// In formula text from an Excel 97-2003 file, a cell outside that
/// format's sheet (`A1`…`IV65536`) cannot be a reference: `TAX2024` or
/// `A70000` there is a defined name (legal in that program, a cell in
/// Tally's larger grid). Such a reference, written without a sheet and
/// without `$`, becomes the name again; the engine keeps the name under
/// another spelling and resolves this one (`names::load_from_data`).
fn biff8_name(e: &mut Expr, origin: CellAddr) {
    let Expr::Ref(RefExpr::Cell(c)) = e else {
        return;
    };
    if !matches!(c.sheet, SheetRef::Current) || c.row.abs || c.col.abs {
        return;
    }
    let (Ok(row), Ok(col)) = (
        u32::try_from(i64::from(origin.row) + i64::from(c.row.v)),
        u32::try_from(i64::from(origin.col) + i64::from(c.col.v)),
    ) else {
        return;
    };
    if col < 256 && row < 65_536 {
        return;
    }
    let name = format!("{}{}", tally_types::col_to_letters(col), u64::from(row) + 1);
    *e = Expr::Name(tally_formula::NameRef::Unresolved(Arc::from(name)));
}

/// Converts the ExcelA1 text of the defined names of one workbook to
/// model OpenFormula text (one sheet table for all of them). Used by the
/// `.xls` reader (`tally-io-xls`).
pub struct ExcelNameConverter(XlsxResolver);

impl ExcelNameConverter {
    /// A converter for a workbook with `sheet_names`.
    pub fn new(sheet_names: &[String]) -> Self {
        Self(XlsxResolver::new(sheet_names))
    }

    /// The text of a defined name (without `=`) local to `sheet` (or of
    /// the workbook) → OpenFormula text with absolute sheet markers
    /// (`of:=[$Sheet1.$A$1:.$B$10]`), or `None` when it does not parse.
    pub fn to_of(&self, text: &str, sheet: Option<usize>) -> Option<String> {
        self.to_of_at(text, sheet, (0, 0))
    }

    /// [`to_of`](Self::to_of) for a name whose relative references are
    /// written for a formula in the cell `base` (row, column) of its sheet
    /// (the first sheet for a name of the workbook): the model's
    /// `NameData::base_cell`.
    ///
    /// Function names are Excel's and resolve as in an `.xlsx` (`_xlfn.`
    /// prefixes, the add-in spellings). A formula that calls a function
    /// Tally does not have is `None`: the text comes from function tokens
    /// of a binary file, and the reader leaves such a name out rather than
    /// keep a call that can only be `#NAME?`.
    pub fn to_of_at(&self, text: &str, sheet: Option<usize>, base: (u32, u32)) -> Option<String> {
        if text.len() > 64 * 1024 {
            return None;
        }
        let origin = CellAddr::new(sheet.unwrap_or(0) as SheetId, base.0, base.1);
        let mut f = parse_excel(text, origin, &self.0).ok()?;
        let mut unknown = false;
        for_each_expr_mut(&mut f.expr, |e| {
            biff8_name(e, origin);
            match e {
                Expr::Ref(re) => mark_sheets_abs(re),
                Expr::Call {
                    func: FuncRef::Unknown(_),
                    ..
                } => unknown = true,
                _ => {}
            }
        });
        if unknown {
            return None;
        }
        // The engine reads this text back when it defines the name: text
        // it would refuse (the printed form can be deeper than what was
        // parsed) is not a name.
        let text = print_of(&f, origin, &self.0);
        parse(&text, origin, Dialect::open_formula(), &self.0)
            .is_ok()
            .then_some(text)
    }

    /// The key of the spelling under which the engine defines a name
    /// spelled `name` in a file (`tally_formula::imported_name_spelling`:
    /// the name, or for a name that is a cell only in Tally's larger grid
    /// the name with `_` appended), folded as the engine looks names up;
    /// `None` for a spelling that is not a name. Two names of one scope
    /// with the same key are one name to the engine.
    pub fn name_key(&self, name: &str) -> Option<String> {
        let origin = CellAddr::new(0, 0, 0);
        let spelling = tally_formula::imported_name_spelling(name, |s| {
            tally_formula::defined_name_problem(s, origin, &self.0).is_none()
        })?;
        Some(tally_formula::name_key(&spelling))
    }

    /// The key of `name` as a file spells it, folded as the engine looks
    /// names up: the key a name defined under another spelling still
    /// answers to (its alias).
    pub fn spelling_key(&self, name: &str) -> String {
        tally_formula::name_key(name)
    }

    /// Whether `name` is a function an `.xlsx` formula spells with the
    /// `_xlfn.` prefix, exactly as given (`_xlfn.DAYS`; not
    /// `_xlfn.SUM`, which only resolves because the prefix is ignored on
    /// a name without one, and not `IFERROR`, which has none).
    pub fn is_prefixed_function(&self, name: &str) -> bool {
        let reg = tally_functions::registry();
        name.starts_with("_xlfn.")
            && reg
                .lookup_ooxml(name)
                .and_then(|id| reg.get(id))
                .and_then(|d| d.ooxml_name)
                .is_some_and(|o| o.eq_ignore_ascii_case(name))
    }

    /// Whether `name` can stand in the formula text given to
    /// [`to_of`](Self::to_of) as a defined name. Either it is a name by
    /// the engine's rule (`tally_formula::defined_name_problem`: not a
    /// cell or R1C1 reference of the grid, not `TRUE` / `FALSE`…) and
    /// reads back as one from Excel formula text; or it is spelled like a
    /// cell outside the Excel 97-2003 sheet
    /// (`tally_formula::is_narrow_grid_name`: `TAX2024`), which that
    /// program allows as a name and which the conversion reads as one.
    pub fn is_name(&self, name: &str) -> bool {
        let origin = CellAddr::new(0, 0, 0);
        tally_formula::is_narrow_grid_name(name)
            || tally_formula::defined_name_problem(name, origin, &self.0).is_none()
                && parse_excel(name, origin, &self.0).is_ok_and(|f| matches!(f.expr, Expr::Name(_)))
    }
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
        // A name with a dot is a name in Excel text; where the model's
        // text would read it as a cell of a sheet, it is `#NAME?`.
        for (x, want) in [
            ("Data.A1+0", "of:=#NAME?+0"),
            ("data.IW1", "of:=#NAME?"),
            ("SUM(Sheet1.B2,1)", "of:=SUM(#NAME?;1)"),
            ("Tax.Rate*2", "of:=Tax.Rate*2"),
            ("Nosheet.A1+0", "of:=#NAME?+0"),
            ("$Data.$A$1*Tax.Rate+0.5", "of:=#NAME?*Tax.Rate+0.5"),
            ("Data.A1:B2", "of:=#NAME?"),
            ("'Data'.A1", "of:=#NAME?"),
            ("zz_dotted_name_0+Data.A1", "of:=zz_dotted_name_0+#NAME?"),
            (
                "_xlfn.STDEV.S(A1:A2)",
                // A function with dots in its name is not a name.
                "of:=_xlfn.STDEV.S([.A1:.A2])",
            ),
            ("Data!A1+0", "of:=[Data.A1]+0"),
        ] {
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
            // A 3D range starting on the formula's own sheet (Sheet1).
            ("of:=SUM([.A1:$Data.B2])", "SUM(Sheet1:Data!A1:B2)"),
            (
                "of:=SUM([$Sheet1.A1:$'My Sheet'.A1])",
                "SUM('Sheet1:My Sheet'!A1)",
            ),
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
