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
    adjust_for_structure_change, parse, print, Dialect, Expr, Formula, StructureChange,
};
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
        if self.invalid.is_some() {
            return Value::Error(ErrorKind::Name);
        }
        let env = self.env(wb, at);
        let mut ctx = CalcCtx::shared_at(wb, at);
        let v = eval_with(&self.formula.expr, &mut ctx, false, &env);
        cell_result(&v, &mut ctx)
    }

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
}
