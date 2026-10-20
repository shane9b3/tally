//! Free-standing formulas that do not live in a cell: conditional format
//! rules, validation criteria and list sources.
//!
//! An [`FExpr`] is a parsed formula whose relative references are offsets
//! (as in every formula AST). Evaluating it "at" a cell applies those
//! offsets to that cell, which is how a rule written for the top-left cell
//! of a range applies to every cell of the range. The text forms are the UI
//! dialect (no leading `=`) and OpenFormula (`of:=…`) for the model.

use std::sync::Arc;

use tally_api::ApiError;
use tally_formula::{
    adjust_for_structure_change, parse, print, Dialect, Expr, Formula, FuncRef, RefExpr, SheetRef,
    StructureChange,
};
use tally_functions::{registry, FnFlags, FuncId};
use tally_types::{CellAddr, ErrorKind, Value};

use crate::eval::{cell_result, eval_with, EvalEnv};
use crate::recalc::CalcCtx;
use crate::workbook::Workbook;

/// A parsed free-standing formula.
#[derive(Clone, Debug, PartialEq)]
pub struct FExpr {
    /// The AST (`#NAME?` for invalid text).
    pub formula: Arc<Formula>,
    /// The original text when it did not parse (kept for saving).
    pub invalid: Option<Arc<str>>,
}

impl FExpr {
    /// A constant number.
    pub fn number(n: f64) -> Self {
        Self {
            formula: Arc::new(Formula::new(Expr::Num(n))),
            invalid: None,
        }
    }

    /// A string literal.
    pub fn text(s: &str) -> Self {
        Self {
            formula: Arc::new(Formula::new(Expr::Str(s.into()))),
            invalid: None,
        }
    }

    /// Parses UI-dialect text (with or without `=`) at `origin`.
    pub fn parse_ui(wb: &Workbook, text: &str, origin: CellAddr) -> Result<Self, ApiError> {
        let t = text.trim();
        let t = t.strip_prefix('=').unwrap_or(t);
        if t.is_empty() {
            return Err(ApiError::invalid_argument("a formula is empty"));
        }
        wb.parse_formula(&format!("={t}"), origin)
            .map(|f| Self {
                formula: Arc::new(f),
                invalid: None,
            })
            .map_err(|e| ApiError::invalid_argument(format!("bad formula {t:?}: {e}")))
    }

    /// Parses a value operand: a formula, or (when it does not parse and is
    /// not quoted) the text itself as a string, so `Paid` means `"Paid"`.
    pub fn parse_operand(wb: &Workbook, text: &str, origin: CellAddr) -> Result<Self, ApiError> {
        match Self::parse_ui(wb, text, origin) {
            Ok(f) => {
                // A bare word that parses as an unknown name is meant as text.
                if let Expr::Name(_) = &f.formula.expr {
                    if wb.names.visible(text.trim(), origin.sheet).is_none() {
                        return Ok(Self::text(text.trim()));
                    }
                }
                Ok(f)
            }
            Err(_) if !text.trim().starts_with('=') && !text.trim().is_empty() => {
                Ok(Self::text(text.trim()))
            }
            Err(e) => Err(e),
        }
    }

    /// Parses OpenFormula text (`of:=…`, `=…` or bare) at `origin`; text
    /// that does not parse is kept as [`FExpr::invalid`].
    pub fn parse_of(wb: &Workbook, text: &str, origin: CellAddr) -> Self {
        let t = text.trim();
        let full = if t.starts_with("of:") || t.starts_with("oooc:") {
            t.to_string()
        } else {
            format!("of:={}", t.strip_prefix('=').unwrap_or(t))
        };
        match parse(
            &full,
            origin,
            Dialect::open_formula(),
            &crate::workbook::OdfResolver(wb),
        ) {
            Ok(f) => Self {
                formula: Arc::new(f),
                invalid: None,
            },
            Err(_) => Self {
                formula: Arc::new(Formula::new(Expr::Err(ErrorKind::Name))),
                invalid: Some(full.into()),
            },
        }
    }

    /// The UI text (without `=`) as it applies at `at`.
    pub fn print_ui(&self, wb: &Workbook, at: CellAddr) -> String {
        if let Some(t) = &self.invalid {
            let t = t.strip_prefix("of:").unwrap_or(t);
            return t.strip_prefix('=').unwrap_or(t).to_string();
        }
        let s = print(&self.formula, at, wb.ui_dialect(), wb);
        s.strip_prefix('=').map(str::to_string).unwrap_or(s)
    }

    /// The OpenFormula text (`of:=…`) as it applies at `at`.
    pub fn print_of(&self, wb: &Workbook, at: CellAddr) -> String {
        if let Some(t) = &self.invalid {
            return t.to_string();
        }
        print(&self.formula, at, Dialect::open_formula(), wb)
    }

    fn env(&self, wb: &Workbook, at: CellAddr) -> EvalEnv {
        if crate::names::has_names(&self.formula.expr) {
            EvalEnv::new(crate::names::collect_names(&wb.names, &self.formula.expr, at.sheet).0)
        } else {
            EvalEnv::default()
        }
    }

    /// Evaluates at `at` with the cached values of formula cells.
    pub fn eval(&self, wb: &Workbook, at: CellAddr) -> Value {
        self.eval_counted(wb, at).0
    }

    /// [`Self::eval`] with the work it took (element operations, as the
    /// per-formula budget counts them; at least 1).
    pub fn eval_counted(&self, wb: &Workbook, at: CellAddr) -> (Value, u64) {
        if self.invalid.is_some() {
            return (Value::Error(ErrorKind::Name), 1);
        }
        eval_formula_counted(wb, &self.formula, at)
    }

    /// The formula with every subexpression that reads only absolute
    /// references (`SUM($A:$A)`, `$B$1*2`) replaced by its value at `at`,
    /// so evaluating the result at other cells repeats only the relative
    /// part. `None` when nothing folds. A rule whose formula is entirely
    /// absolute folds to one literal (security review 3, CRITICAL-1 (4)).
    /// The caller caches the result per document version.
    pub fn fold_absolute(&self, wb: &Workbook, at: CellAddr) -> Option<Arc<Formula>> {
        if self.invalid.is_some() {
            return None;
        }
        let mut expr = self.formula.expr.clone();
        let mut work = 0u64;
        let folded = fold_expr(&mut expr, at, &mut work, &mut |f, at| {
            eval_for_fold(wb, f, at)
        });
        folded.then(|| Arc::new(Formula::new(expr)))
    }

    /// [`Self::fold_absolute`] evaluating the absolute parts with dirty
    /// precedents recalculated (as [`Self::eval_fresh`] does).
    pub fn fold_absolute_fresh(&self, wb: &mut Workbook, at: CellAddr) -> Option<Arc<Formula>> {
        if self.invalid.is_some() {
            return None;
        }
        let mut expr = self.formula.expr.clone();
        let mut work = 0u64;
        let folded = fold_expr(&mut expr, at, &mut work, &mut |f, at| {
            eval_for_fold_fresh(wb, f, at)
        });
        folded.then(|| Arc::new(Formula::new(expr)))
    }
}

/// Evaluates a formula at `at`, recalculating dirty precedents on demand,
/// returning the value and the work spent.
pub fn eval_formula_fresh_counted(
    wb: &mut Workbook,
    formula: &Formula,
    at: CellAddr,
) -> (Value, u64) {
    let env = if crate::names::has_names(&formula.expr) {
        EvalEnv::new(crate::names::collect_names(&wb.names, &formula.expr, at.sheet).0)
    } else {
        EvalEnv::default()
    };
    let mut ctx = CalcCtx::at(wb, at);
    let v = eval_with(&formula.expr, &mut ctx, false, &env);
    let out = cell_result(&v, &mut ctx);
    (out, env.work().max(1))
}

/// Evaluates a formula at `at` with the cached values of formula cells,
/// returning the value and the work spent.
pub fn eval_formula_counted(wb: &Workbook, formula: &Formula, at: CellAddr) -> (Value, u64) {
    let env = if crate::names::has_names(&formula.expr) {
        EvalEnv::new(crate::names::collect_names(&wb.names, &formula.expr, at.sheet).0)
    } else {
        EvalEnv::default()
    };
    let mut ctx = CalcCtx::shared_at(wb, at);
    let v = eval_with(&formula.expr, &mut ctx, false, &env);
    let out = cell_result(&v, &mut ctx);
    (out, env.work().max(1))
}

/// Evaluates a subexpression for folding: in array context, so a matrix
/// result stays a matrix (and is then not folded) instead of being
/// reduced to the value at `at`.
fn eval_for_fold(wb: &Workbook, formula: &Formula, at: CellAddr) -> (Value, u64) {
    let env = if crate::names::has_names(&formula.expr) {
        EvalEnv::new(crate::names::collect_names(&wb.names, &formula.expr, at.sheet).0)
    } else {
        EvalEnv::default()
    };
    let mut ctx = CalcCtx::shared_at(wb, at);
    let v = eval_with(&formula.expr, &mut ctx, true, &env);
    let out = crate::eval::array_result(&v, &mut ctx);
    (out, env.work().max(1))
}

fn eval_for_fold_fresh(wb: &mut Workbook, formula: &Formula, at: CellAddr) -> (Value, u64) {
    let env = if crate::names::has_names(&formula.expr) {
        EvalEnv::new(crate::names::collect_names(&wb.names, &formula.expr, at.sheet).0)
    } else {
        EvalEnv::default()
    };
    let mut ctx = CalcCtx::at(wb, at);
    let v = eval_with(&formula.expr, &mut ctx, true, &env);
    let out = crate::eval::array_result(&v, &mut ctx);
    (out, env.work().max(1))
}

/// Largest folding work per formula (element operations); past it the
/// rest of the formula stays as written.
const MAX_FOLD_WORK: u64 = crate::eval::MAX_FORMULA_WORK;

/// Whether `e` has the same value at every cell (it reads only absolute
/// references, no name, no volatile, origin-dependent, array-forcing or
/// reference-returning function), and whether it reads any reference.
///
/// A multi-cell absolute range is position-independent only as a direct
/// argument of a function (`SUM($A:$A)`): anywhere else scalar
/// evaluation intersects it with the current row/column (`range_ok`).
fn absolute(e: &Expr, range_ok: bool) -> (bool, bool) {
    match e {
        Expr::Num(_) | Expr::Str(_) | Expr::Bool(_) | Expr::Err(_) | Expr::Missing => (true, false),
        Expr::ArrayLit(a) => {
            let mut has = false;
            for x in &a.items {
                let (abs, h) = absolute(x, false);
                if !abs {
                    return (false, true);
                }
                has |= h;
            }
            (true, has)
        }
        Expr::Ref(r) => (ref_absolute(r, range_ok), true),
        Expr::Name(_) => (false, true),
        Expr::Call { func, args } => {
            let ok = match func {
                FuncRef::Known(id) => registry().get(FuncId(id.0)).is_some_and(|d| {
                    !d.flags.contains(FnFlags::VOLATILE)
                        && !d.flags.contains(FnFlags::NEEDS_ORIGIN)
                        && !d.flags.contains(FnFlags::FORCE_ARRAY)
                        && !d.flags.contains(FnFlags::RETURNS_REF)
                }),
                FuncRef::Unknown(_) => false,
            };
            let mut has = false;
            for x in args {
                let (abs, h) = absolute(x, true);
                if !abs {
                    return (false, true);
                }
                has |= h;
            }
            (ok, has)
        }
        Expr::Unary(_, x) | Expr::Postfix(_, x) | Expr::Paren(x) => absolute(x, false),
        Expr::Binary(_, l, r) => {
            let (a, ha) = absolute(l, false);
            let (b, hb) = absolute(r, false);
            (a && b, ha || hb)
        }
    }
}

fn ref_absolute(r: &RefExpr, range_ok: bool) -> bool {
    let sheet_ok = |s: &SheetRef| !matches!(s, SheetRef::Unresolved(_));
    match r {
        RefExpr::Cell(c) => sheet_ok(&c.sheet) && c.row.abs && c.col.abs,
        RefExpr::Area(a, b) => {
            range_ok
                && sheet_ok(&a.sheet)
                && sheet_ok(&b.sheet)
                && a.row.abs
                && a.col.abs
                && b.row.abs
                && b.col.abs
        }
        RefExpr::Cols { sheet, start, end } | RefExpr::Rows { sheet, start, end } => {
            range_ok && sheet_ok(sheet) && start.abs && end.abs
        }
        RefExpr::Invalid => true,
    }
}

/// Replaces the maximal absolute subtrees of `e` that read references by
/// literals (evaluated with `eval`); `true` if anything changed.
fn fold_expr(
    e: &mut Expr,
    at: CellAddr,
    work: &mut u64,
    eval: &mut dyn FnMut(&Formula, CellAddr) -> (Value, u64),
) -> bool {
    if *work > MAX_FOLD_WORK {
        return false;
    }
    let (abs, has_ref) = absolute(e, false);
    if abs {
        // Bare references stay references (a function may consume them
        // as a range); only operations over absolute parts fold.
        if !has_ref || matches!(e, Expr::Ref(_)) {
            return false;
        }
        let f = Formula::new(e.clone());
        let (v, w) = eval(&f, at);
        *work += w;
        let lit = match v {
            Value::Number(n) => Expr::Num(n),
            Value::Text(t) => Expr::Str(t),
            Value::Bool(b) => Expr::Bool(b),
            Value::Error(ErrorKind::Ref) => Expr::Ref(RefExpr::Invalid),
            Value::Error(k) => Expr::Err(k),
            // No literal form: keep the subtree.
            Value::Empty | Value::Array(_) => return false,
        };
        *e = lit;
        return true;
    }
    match e {
        Expr::Call { args, .. } => {
            let mut any = false;
            for x in args.iter_mut() {
                any |= fold_expr(x, at, work, eval);
            }
            any
        }
        Expr::Unary(_, x) | Expr::Postfix(_, x) | Expr::Paren(x) => fold_expr(x, at, work, eval),
        Expr::Binary(_, l, r) => {
            let a = fold_expr(l, at, work, eval);
            let b = fold_expr(r, at, work, eval);
            a || b
        }
        _ => false,
    }
}

/// `true` if `f` has the same value at every cell: it reads only absolute
/// references (a bare range counts, for list sources that are consumed
/// whole).
pub fn is_absolute(f: &FExpr) -> bool {
    f.invalid.is_none() && absolute(&f.formula.expr, true).0
}

impl FExpr {
    /// Evaluates at `at`, recalculating dirty precedents on demand.
    pub fn eval_fresh(&self, wb: &mut Workbook, at: CellAddr) -> Value {
        if self.invalid.is_some() {
            return Value::Error(ErrorKind::Name);
        }
        let env = self.env(wb, at);
        let mut ctx = CalcCtx::at(wb, at);
        let v = eval_with(&self.formula.expr, &mut ctx, false, &env);
        cell_result(&v, &mut ctx)
    }

    /// The constant number this formula is, if it is one.
    pub fn as_number(&self) -> Option<f64> {
        let mut e = &self.formula.expr;
        while let Expr::Paren(x) = e {
            e = x;
        }
        match e {
            Expr::Num(n) => Some(*n),
            Expr::Unary(tally_formula::UnOp::Neg, x) => match &**x {
                Expr::Num(n) => Some(-n),
                _ => None,
            },
            _ => None,
        }
    }

    /// Rewrites references for a row/column change; `origin` is the
    /// position the offsets are relative to, before the change.
    pub fn adjust(&mut self, origin: CellAddr, change: &StructureChange) {
        if self.invalid.is_some() {
            return;
        }
        let mut f = (*self.formula).clone();
        if adjust_for_structure_change(&mut f, origin, change) {
            self.formula = Arc::new(f);
        }
    }
}

/// `true` for a value a condition formula treats as true.
pub fn truthy(v: &Value) -> bool {
    match v {
        Value::Number(n) => *n != 0.0,
        Value::Bool(b) => *b,
        Value::Array(m) => m.get(0, 0).is_some_and(truthy),
        _ => false,
    }
}

/// A number from a value (booleans count; text and errors do not).
pub fn number_of(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => Some(*n),
        Value::Bool(b) => Some(f64::from(u8::from(*b))),
        Value::Array(m) => m.get(0, 0).and_then(number_of),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_offsets_apply_per_cell() {
        let mut wb = Workbook::new();
        let s = wb.sheets[0].id;
        wb.set_input(CellAddr::new(s, 0, 1), "10");
        wb.set_input(CellAddr::new(s, 1, 1), "20");
        // Written for A1: "B1*2".
        let f = FExpr::parse_ui(&wb, "B1*2", CellAddr::new(s, 0, 0)).unwrap();
        assert_eq!(f.eval(&wb, CellAddr::new(s, 0, 0)), Value::Number(20.0));
        assert_eq!(f.eval(&wb, CellAddr::new(s, 1, 0)), Value::Number(40.0));
        assert_eq!(f.print_ui(&wb, CellAddr::new(s, 1, 0)), "B2*2");
        assert_eq!(f.print_of(&wb, CellAddr::new(s, 0, 0)), "of:=[.B1]*2");
        let back = FExpr::parse_of(&wb, "of:=[.B1]*2", CellAddr::new(s, 0, 0));
        assert_eq!(back.formula, f.formula);
        let bad = FExpr::parse_of(&wb, "of:=SUM(", CellAddr::new(s, 0, 0));
        assert!(bad.invalid.is_some());
        assert_eq!(bad.print_of(&wb, CellAddr::new(s, 0, 0)), "of:=SUM(");
        let op = FExpr::parse_operand(&wb, "Paid", CellAddr::new(s, 0, 0)).unwrap();
        assert_eq!(op.eval(&wb, CellAddr::new(s, 0, 0)), Value::text("Paid"));
        assert_eq!(
            FExpr::parse_operand(&wb, "-3", CellAddr::new(s, 0, 0))
                .unwrap()
                .as_number(),
            Some(-3.0)
        );
    }

    #[test]
    fn absolute_subexpressions_fold_to_literals() {
        let mut wb = Workbook::new();
        let s = wb.sheets[0].id;
        for r in 0..10 {
            wb.set_input(CellAddr::new(s, r, 1), &format!("{r}"));
        }
        let at = CellAddr::new(s, 0, 0);
        let f = FExpr::parse_ui(&wb, "SUM($B:$B)>A1", at).unwrap();
        let folded = f.fold_absolute(&wb, at).expect("the SUM folds");
        assert_eq!(
            print(&folded, at, wb.ui_dialect(), &wb),
            "=45>A1",
            "the absolute SUM became a literal"
        );
        let whole = FExpr::parse_ui(&wb, "SUM($B$1:$B$3)*2", at).unwrap();
        let folded = whole.fold_absolute(&wb, at).unwrap();
        assert_eq!(print(&folded, at, wb.ui_dialect(), &wb), "=6");
        // Relative, volatile, origin-dependent and name-bearing parts stay.
        for text in [
            "A1+1",
            "RAND()",
            "ROW()+1",
            "SUM(B:B)",
            "SUM($B:$B)*RAND()*ROW()",
        ] {
            let f = FExpr::parse_ui(&wb, text, at).unwrap();
            let folded = f.fold_absolute(&wb, at);
            if text.starts_with("SUM($B:$B)") {
                // Only the absolute factor folds; RAND() and ROW() stay.
                let p = print(&folded.unwrap(), at, wb.ui_dialect(), &wb);
                assert_eq!(p, "=45*RAND()*ROW()");
            } else {
                assert!(folded.is_none(), "{text} must not fold");
            }
        }
        // Bare references stay (a function may take them as a range);
        // operations over absolute parts inside a call fold.
        let mixed = FExpr::parse_ui(&wb, "IF(A1>$B$2;$B$3;ROW())", at).unwrap();
        assert!(mixed.fold_absolute(&wb, at).is_none());
        let mixed = FExpr::parse_ui(&wb, "IF(A1>SUM($B$1:$B$3)*2,$B$3,ROW())", at).unwrap();
        let folded = mixed.fold_absolute(&wb, at).unwrap();
        assert_eq!(
            print(&folded, at, wb.ui_dialect(), &wb),
            "=IF(A1>6,$B$3,ROW())"
        );
        // A multi-cell range outside a function is intersected per cell:
        // never folded. Array-forcing functions are left alone too.
        for text in [
            "$B$1:$B$3*2>A1",
            "SUMPRODUCT($B$1:$B$3*2)>A1",
            "INDEX($B$1:$B$3,1)>A1",
        ] {
            let f = FExpr::parse_ui(&wb, text, at).unwrap();
            assert!(f.fold_absolute(&wb, at).is_none(), "{text} must not fold");
        }
    }
}
