//! Canonical printer and the display-only prettifier.
//!
//! Canonical output has no whitespace, uppercase function names, shortest
//! round-trip numbers and references re-anchored to the given origin, e.g.
//! `=SUM(A1:B2,3)` or `of:=SUM([.A1:.B2];3)`. References that resolve
//! outside the grid, or to a deleted sheet, print as `#REF!`.

use std::fmt::Write as _;

use tally_types::{col_to_letters, CellAddr, MAX_COLS, MAX_ROWS};

use crate::ast::{BinOp, PostfixOp};
use crate::ast::{
    CellRefExpr, Expr, Formula, FuncRef, NameRef, RefCoord, RefExpr, SheetRef, UnOp,
    POSTFIX_PRECEDENCE, PREFIX_PRECEDENCE,
};
use crate::dialect::{Dialect, SheetSep};
use crate::lexer::{lex, TokenKind};
use crate::refsyntax::push_sheet_name;
use crate::Resolver;

/// Prints a formula with its prefix (`=` or `of:=`) for a cell at `origin`.
pub fn print(f: &Formula, origin: CellAddr, dialect: Dialect, resolver: &dyn Resolver) -> String {
    let mut p = Printer {
        origin,
        dialect,
        resolver,
        out: String::with_capacity(32),
    };
    p.out.push_str(dialect.formula_prefix());
    p.expr(&f.expr);
    p.out
}

/// Prints an expression without a prefix.
pub fn print_expr(e: &Expr, origin: CellAddr, dialect: Dialect, resolver: &dyn Resolver) -> String {
    let mut p = Printer {
        origin,
        dialect,
        resolver,
        out: String::with_capacity(32),
    };
    p.expr(e);
    p.out
}

/// Formats a number: integers without `.0`, otherwise Rust's shortest
/// round-trip representation, using `E` notation (`1E+20`) outside
/// `1e-5..1e15`.
pub(crate) fn fmt_num(x: f64) -> String {
    if !x.is_finite() {
        return "#NUM!".to_string();
    }
    let a = x.abs();
    if x == x.trunc() && a < 1e15 {
        // Exact: |x| < 2^53.
        #[allow(clippy::cast_possible_truncation)]
        let i = x as i64;
        return i.to_string();
    }
    if !(1e-5..1e15).contains(&a) {
        // OpenOffice style: `1E+20`, `1.5E-07` is written `1.5E-7`.
        let s = format!("{x:E}");
        match s.split_once('E') {
            Some((m, e)) if !e.starts_with('-') => format!("{m}E+{e}"),
            _ => s,
        }
    } else {
        format!("{x}")
    }
}

/// Schedules `e` for printing, wrapped in parentheses when `parens`.
fn push_child<'e>(stack: &mut Vec<Work<'e>>, e: &'e Expr, parens: bool) {
    if parens {
        stack.push(Work::Char(')'));
        stack.push(Work::Expr(e));
        stack.push(Work::Char('('));
    } else {
        stack.push(Work::Expr(e));
    }
}

struct Printer<'a> {
    origin: CellAddr,
    dialect: Dialect,
    resolver: &'a dyn Resolver,
    out: String,
}

/// A pending piece of output for the iterative printer.
enum Work<'e> {
    Expr(&'e Expr),
    Char(char),
    Str(&'static str),
}

impl Printer<'_> {
    /// Prints `root` with an explicit work stack (no recursion), so trees of
    /// any depth are safe on small stacks.
    fn expr(&mut self, root: &Expr) {
        let mut stack: Vec<Work<'_>> = vec![Work::Expr(root)];
        while let Some(w) = stack.pop() {
            let e = match w {
                Work::Char(c) => {
                    self.out.push(c);
                    continue;
                }
                Work::Str(s) => {
                    self.out.push_str(s);
                    continue;
                }
                Work::Expr(e) => e,
            };
            match e {
                Expr::Num(x) => {
                    let s = fmt_num(*x);
                    self.out.push_str(&s);
                }
                Expr::Str(s) => {
                    self.out.push('"');
                    self.out.push_str(&s.replace('"', "\"\""));
                    self.out.push('"');
                }
                Expr::Bool(b) => {
                    self.out.push_str(if *b { "TRUE" } else { "FALSE" });
                    if self.dialect.is_open_formula() {
                        self.out.push_str("()");
                    }
                }
                Expr::Err(k) => self.out.push_str(&k.display()),
                Expr::ArrayLit(a) => {
                    self.out.push('{');
                    stack.push(Work::Char('}'));
                    let cols = a.cols.max(1) as usize;
                    for (i, item) in a.items.iter().enumerate().rev() {
                        match item {
                            // Plain constants inside arrays in every dialect.
                            Expr::Bool(true) => stack.push(Work::Str("TRUE")),
                            Expr::Bool(false) => stack.push(Work::Str("FALSE")),
                            other => stack.push(Work::Expr(other)),
                        }
                        if i > 0 {
                            stack.push(Work::Char(if i % cols == 0 { '|' } else { ';' }));
                        }
                    }
                }
                Expr::Ref(r) => self.reference(r),
                Expr::Name(n) => match n {
                    NameRef::Id(id) => match self.resolver.name_text(*id) {
                        Some(t) => self.out.push_str(&t),
                        None => self.out.push_str("#NAME?"),
                    },
                    NameRef::Unresolved(t) => self.out.push_str(t),
                },
                Expr::Call { func, args } => {
                    match func {
                        FuncRef::Known(id) => {
                            match self.resolver.function_name_in(*id, self.dialect) {
                                Some(n) => self.out.push_str(&n),
                                None => self.out.push_str("#NAME?"),
                            }
                        }
                        FuncRef::Unknown(n) => self.out.push_str(n),
                    }
                    self.out.push('(');
                    stack.push(Work::Char(')'));
                    let sep = self.dialect.arg_sep_char();
                    for (i, a) in args.iter().enumerate().rev() {
                        stack.push(Work::Expr(a));
                        if i > 0 {
                            stack.push(Work::Char(sep));
                        }
                    }
                }
                Expr::Unary(op, x) => {
                    self.out.push(match op {
                        UnOp::Neg => '-',
                        UnOp::Plus => '+',
                    });
                    push_child(&mut stack, x, x.precedence() < PREFIX_PRECEDENCE);
                }
                Expr::Binary(op, l, r) => {
                    let p = op.precedence();
                    // `Name!A1` would re-lex as a sheet qualifier.
                    let guard_bang =
                        *op == BinOp::Intersect && matches!(**l, Expr::Name(_) | Expr::Bool(_));
                    push_child(&mut stack, r, r.precedence() <= p);
                    stack.push(Work::Str(op.symbol()));
                    push_child(&mut stack, l, l.precedence() < p || guard_bang);
                }
                Expr::Postfix(PostfixOp::Percent, x) => {
                    stack.push(Work::Char('%'));
                    push_child(&mut stack, x, x.precedence() < POSTFIX_PRECEDENCE);
                }
                Expr::Paren(x) => push_child(&mut stack, x, true),
                Expr::Missing => {}
            }
        }
    }

    fn reference(&mut self, r: &RefExpr) {
        let of = self.dialect.is_open_formula();
        let text = if of { self.of_ref(r) } else { self.a1_ref(r) };
        match text {
            Some(t) => self.out.push_str(&t),
            None if of => self.out.push_str("[.#REF!]"),
            None => self.out.push_str("#REF!"),
        }
    }

    /// The sheet qualifier text, or `None` for a deleted/unknown sheet.
    /// Returns `Some("")` for [`SheetRef::Current`].
    fn sheet_text(&self, s: &SheetRef) -> Option<String> {
        let (name, abs) = match s {
            SheetRef::Current => return Some(String::new()),
            SheetRef::Id { id, abs } => (self.resolver.sheet_name(*id)?, *abs),
            SheetRef::Unresolved(n) => (n.clone(), false),
            SheetRef::Deleted => return None,
        };
        let mut out = String::new();
        if abs {
            out.push('$');
        }
        push_sheet_name(&mut out, &name);
        Some(out)
    }

    fn sheet_sep(&self) -> char {
        match self.dialect {
            Dialect::CalcA1 {
                sheet_sep: SheetSep::Bang,
                ..
            } => '!',
            _ => '.',
        }
    }

    fn coords(&self, c: &CellRefExpr, out: &mut String) -> Option<()> {
        let row = c.row.resolve(self.origin.row, MAX_ROWS)?;
        let col = c.col.resolve(self.origin.col, MAX_COLS)?;
        if c.col.abs {
            out.push('$');
        }
        out.push_str(&col_to_letters(col));
        if c.row.abs {
            out.push('$');
        }
        let _ = write!(out, "{}", u64::from(row) + 1);
        Some(())
    }

    fn col(&self, c: RefCoord, out: &mut String) -> Option<()> {
        let col = c.resolve(self.origin.col, MAX_COLS)?;
        if c.abs {
            out.push('$');
        }
        out.push_str(&col_to_letters(col));
        Some(())
    }

    fn row(&self, r: RefCoord, out: &mut String) -> Option<()> {
        let row = r.resolve(self.origin.row, MAX_ROWS)?;
        if r.abs {
            out.push('$');
        }
        let _ = write!(out, "{}", u64::from(row) + 1);
        Some(())
    }

    fn a1_sheet(&self, s: &SheetRef, out: &mut String) -> Option<()> {
        let t = self.sheet_text(s)?;
        if !t.is_empty() {
            out.push_str(&t);
            out.push(self.sheet_sep());
        }
        Some(())
    }

    fn a1_ref(&self, r: &RefExpr) -> Option<String> {
        let mut out = String::new();
        match r {
            RefExpr::Cell(c) => {
                self.a1_sheet(&c.sheet, &mut out)?;
                self.coords(c, &mut out)?;
            }
            RefExpr::Area(a, b) => {
                self.a1_sheet(&a.sheet, &mut out)?;
                self.coords(a, &mut out)?;
                out.push(':');
                self.a1_sheet(&b.sheet, &mut out)?;
                self.coords(b, &mut out)?;
            }
            RefExpr::Cols { sheet, start, end } => {
                self.a1_sheet(sheet, &mut out)?;
                self.col(*start, &mut out)?;
                out.push(':');
                self.col(*end, &mut out)?;
            }
            RefExpr::Rows { sheet, start, end } => {
                self.a1_sheet(sheet, &mut out)?;
                self.row(*start, &mut out)?;
                out.push(':');
                self.row(*end, &mut out)?;
            }
            RefExpr::Invalid => return None,
        }
        Some(out)
    }

    fn of_sheet(&self, s: &SheetRef, out: &mut String) -> Option<()> {
        out.push_str(&self.sheet_text(s)?);
        out.push('.');
        Some(())
    }

    fn of_ref(&self, r: &RefExpr) -> Option<String> {
        let mut out = String::from("[");
        match r {
            RefExpr::Cell(c) => {
                self.of_sheet(&c.sheet, &mut out)?;
                self.coords(c, &mut out)?;
            }
            RefExpr::Area(a, b) => {
                self.of_sheet(&a.sheet, &mut out)?;
                self.coords(a, &mut out)?;
                out.push(':');
                self.of_sheet(&b.sheet, &mut out)?;
                self.coords(b, &mut out)?;
            }
            RefExpr::Cols { sheet, start, end } => {
                self.of_sheet(sheet, &mut out)?;
                self.col(*start, &mut out)?;
                out.push_str(":.");
                self.col(*end, &mut out)?;
            }
            RefExpr::Rows { sheet, start, end } => {
                self.of_sheet(sheet, &mut out)?;
                self.row(*start, &mut out)?;
                out.push_str(":.");
                self.row(*end, &mut out)?;
            }
            RefExpr::Invalid => return None,
        }
        out.push(']');
        Some(out)
    }
}

/// Display-only prettifier for the formula bar: adds spaces around binary
/// operators, shows `*` as `×` and `<>` as `≠`. Never use the result for
/// editing or storage. `=D6*E6` becomes `=D6 × E6`.
pub fn pretty(src: &str) -> String {
    let (tokens, _) = lex(src, Dialect::calc_a1_en_us());
    let mut out = String::with_capacity(src.len() + 8);
    let mut prev: Option<TokenKind> = None;
    for t in &tokens {
        let text = src.get(t.span.clone()).unwrap_or("");
        let kind = t.kind;
        if kind == TokenKind::Whitespace {
            continue;
        }
        let operand_expected = match prev {
            None => true,
            Some(p) => !matches!(
                p,
                TokenKind::Number
                    | TokenKind::String
                    | TokenKind::Ident
                    | TokenKind::CellRef
                    | TokenKind::RangeRef
                    | TokenKind::OfRef
                    | TokenKind::Error
                    | TokenKind::RParen
                    | TokenKind::RBrace
                    | TokenKind::Percent
            ),
        };
        let spaced = match kind {
            TokenKind::Plus | TokenKind::Minus => !operand_expected,
            TokenKind::Star
            | TokenKind::Slash
            | TokenKind::Caret
            | TokenKind::Amp
            | TokenKind::Eq
            | TokenKind::Ne
            | TokenKind::Lt
            | TokenKind::Le
            | TokenKind::Gt
            | TokenKind::Ge => !operand_expected,
            _ => false,
        };
        let shown = match kind {
            TokenKind::Star => "×",
            TokenKind::Ne => "≠",
            _ => text,
        };
        if spaced {
            out.push(' ');
            out.push_str(shown);
            out.push(' ');
        } else {
            out.push_str(shown);
        }
        prev = Some(kind);
    }
    out
}
