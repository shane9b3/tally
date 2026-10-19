//! Plain-English hints for the formula bar: `=D6*E6` → "Quantity × Price".

use std::sync::Arc;

use tally_types::{col_to_letters, CellAddr, CellRange, MAX_COLS, MAX_ROWS};

use crate::ast::{BinOp, Expr, Formula, FuncId, FuncRef, RefExpr, ResolvedRef, UnOp};
use crate::printer::fmt_num;

/// Supplies human labels for cells and ranges (typically the header found
/// above or to the left of the referenced cells).
pub trait LabelSource {
    /// A label for one cell, e.g. "Quantity" for `D6`.
    fn label_for(&self, addr: CellAddr) -> Option<String>;
    /// A label for a range, e.g. "Total" for `F2:F15`.
    fn range_label(&self, r: &CellRange) -> Option<String>;
    /// The name of a registry function, so calls to known functions can be
    /// explained. Without it only [`FuncRef::Unknown`] names are recognised.
    fn function_name(&self, id: FuncId) -> Option<Arc<str>> {
        let _ = id;
        None
    }
}

/// A [`LabelSource`] that knows no labels (hints fall back to `A1` text).
#[derive(Clone, Copy, Debug, Default)]
pub struct NoLabels;

impl LabelSource for NoLabels {
    fn label_for(&self, _addr: CellAddr) -> Option<String> {
        None
    }
    fn range_label(&self, _r: &CellRange) -> Option<String> {
        None
    }
}

const MAX_EXPLAIN_DEPTH: u32 = 32;

/// Explains a formula at `origin` in plain English, or `None` if it is not a
/// simple shape: arithmetic over cells and numbers (`A1 × B1`), or
/// SUM/AVERAGE/COUNT/MIN/MAX of one range, or SUMIF.
pub fn explain(f: &Formula, origin: CellAddr, labels: &dyn LabelSource) -> Option<String> {
    let x = Explainer { origin, labels };
    match strip(&f.expr) {
        e @ Expr::Call { .. } => x.call(e),
        e @ Expr::Binary(op, _, _) if is_arith(*op) => x.term(e, 0),
        _ => None,
    }
}

fn strip(mut e: &Expr) -> &Expr {
    while let Expr::Paren(x) = e {
        e = x;
    }
    e
}

fn is_arith(op: BinOp) -> bool {
    matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div)
}

struct Explainer<'a> {
    origin: CellAddr,
    labels: &'a dyn LabelSource,
}

impl Explainer<'_> {
    fn term(&self, e: &Expr, depth: u32) -> Option<String> {
        if depth > MAX_EXPLAIN_DEPTH {
            return None;
        }
        match e {
            Expr::Num(n) => Some(fmt_num(*n)),
            Expr::Ref(RefExpr::Cell(c)) => {
                let addr = c.resolve(self.origin)?;
                Some(self.labels.label_for(addr).unwrap_or_else(|| addr.to_a1()))
            }
            Expr::Paren(x) => Some(format!("({})", self.term(x, depth + 1)?)),
            Expr::Unary(UnOp::Neg, x) => Some(format!("−{}", self.term(x, depth + 1)?)),
            Expr::Binary(op, l, r) if is_arith(*op) => {
                let sym = match op {
                    BinOp::Add => "+",
                    BinOp::Sub => "−",
                    BinOp::Mul => "×",
                    _ => "÷",
                };
                Some(format!(
                    "{} {sym} {}",
                    self.term(l, depth + 1)?,
                    self.term(r, depth + 1)?
                ))
            }
            Expr::Call { .. } => self.call(e),
            _ => None,
        }
    }

    fn range_text(&self, e: &Expr) -> Option<String> {
        let Expr::Ref(r) = strip(e) else {
            return None;
        };
        let ResolvedRef::Range(range) = r.resolve(self.origin) else {
            return None;
        };
        if let Some(l) = self.labels.range_label(&range) {
            return Some(l);
        }
        if range.is_single() {
            if let Some(l) = self.labels.label_for(range.start()) {
                return Some(l);
            }
        }
        Some(if range.r0 == 0 && range.r1 == MAX_ROWS - 1 {
            format!("{}:{}", col_to_letters(range.c0), col_to_letters(range.c1))
        } else if range.c0 == 0 && range.c1 == MAX_COLS - 1 {
            format!("{}:{}", range.r0 + 1, range.r1 + 1)
        } else {
            range.to_a1()
        })
    }

    fn call(&self, e: &Expr) -> Option<String> {
        let Expr::Call { func, args } = e else {
            return None;
        };
        let name: Arc<str> = match func {
            FuncRef::Known(id) => self.labels.function_name(*id)?,
            FuncRef::Unknown(n) => n.clone(),
        };
        match (&*name, args.as_slice()) {
            ("SUM", [a]) => Some(format!("Sum of {}", self.range_text(a)?)),
            ("AVERAGE", [a]) => Some(format!("Average of {}", self.range_text(a)?)),
            ("COUNT", [a]) | ("COUNTA", [a]) => Some(format!("Count of {}", self.range_text(a)?)),
            ("MIN", [a]) => Some(format!("Smallest {}", self.range_text(a)?)),
            ("MAX", [a]) => Some(format!("Largest {}", self.range_text(a)?)),
            ("SUMIF", [range, crit]) => {
                let r = self.range_text(range)?;
                Some(format!("Sum of {r} where {r} {}", criterion(crit)?))
            }
            ("SUMIF", [range, crit, sum]) => Some(format!(
                "Sum of {} where {} {}",
                self.range_text(sum)?,
                self.range_text(range)?,
                criterion(crit)?
            )),
            _ => None,
        }
    }
}

/// "is Paid", "is not Paid", "is greater than 5", ...
fn criterion(e: &Expr) -> Option<String> {
    let text = match strip(e) {
        Expr::Str(s) => s.to_string(),
        Expr::Num(n) => fmt_num(*n),
        _ => return None,
    };
    let (phrase, rest) = if let Some(r) = text.strip_prefix("<>") {
        ("is not", r)
    } else if let Some(r) = text.strip_prefix(">=") {
        ("is at least", r)
    } else if let Some(r) = text.strip_prefix("<=") {
        ("is at most", r)
    } else if let Some(r) = text.strip_prefix('>') {
        ("is greater than", r)
    } else if let Some(r) = text.strip_prefix('<') {
        ("is less than", r)
    } else if let Some(r) = text.strip_prefix('=') {
        ("is", r)
    } else {
        ("is", text.as_str())
    };
    Some(format!("{phrase} {rest}"))
}
