//! Canonical printer and the display-only prettifier.
//!
//! Canonical output has no whitespace, uppercase function names, shortest
//! round-trip numbers and references re-anchored to the given origin, e.g.
//! `=SUM(A1:B2,3)` or `of:=SUM([.A1:.B2];3)`. References that resolve
//! outside the grid, or to a deleted sheet, print as `#REF!`.
//!
//! [`Dialect::ExcelA1`] output is what Excel stores in OOXML cells (without
//! the leading `=` there): `SUM(Sheet1!A1:B2,'My Sheet'!C1)`, arrays
//! `{1,2;3,4}`, intersection as a space (operands that would re-read
//! differently in brackets: `(Name) (A1)`), union as `(A1,B1)`, 3D areas as
//! `Sheet1:Sheet3!A1:B2`, no `$` before sheet names, and error literals
//! Excel does not know (`Err:502`) as `#VALUE!`.

use std::fmt::Write as _;

use tally_types::{col_to_letters, CellAddr, MAX_COLS, MAX_ROWS};

use crate::ast::{BinOp, PostfixOp};
use crate::ast::{
    CellRefExpr, Expr, Formula, FuncRef, NameRef, RefCoord, RefExpr, SheetRef, UnOp,
    POSTFIX_PRECEDENCE, PREFIX_PRECEDENCE,
};
use crate::dialect::{Dialect, SheetSep};
use crate::lexer::{lex, TokenKind};
use crate::refsyntax::{push_sheet_name, sheet_needs_quotes};
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

/// The operand printed last in `e`: follows the right operands of `:` and
/// `!` chains, which print without parentheses (conservatively: a range
/// operand may still get some).
fn last_operand(mut e: &Expr) -> &Expr {
    while let Expr::Binary(op @ (BinOp::Range | BinOp::Intersect), _, r) = e {
        if r.precedence() <= op.precedence() {
            break;
        }
        e = r;
    }
    e
}

/// The operand printed first in `e`: follows left operands of binary
/// operators and the operand of `%` (conservatively, as `last_operand`).
/// An ExcelA1 union prints in parentheses, so it is its own first operand.
fn first_operand(mut e: &Expr) -> &Expr {
    loop {
        e = match e {
            Expr::Binary(BinOp::Union, _, _) => return e,
            Expr::Binary(op, l, _) if l.precedence() >= op.precedence() => l,
            Expr::Postfix(_, x) if x.precedence() >= POSTFIX_PRECEDENCE => x,
            _ => return e,
        };
    }
}

/// A constant operand: a number, string, boolean or inline array.
fn is_constant(e: &Expr) -> bool {
    matches!(
        e,
        Expr::Num(_) | Expr::Str(_) | Expr::Bool(_) | Expr::ArrayLit(_)
    )
}

/// Schedules `e` for printing, wrapped in parentheses when `parens`.
fn push_child<'e>(stack: &mut Vec<Work<'e>>, e: &'e Expr, parens: bool) {
    if parens {
        stack.push(Work::Char(')'));
        // Already parenthesized: an ExcelA1 union needs no more.
        stack.push(Work::Bare(e));
        stack.push(Work::Char('('));
    } else {
        stack.push(Work::Expr(e));
    }
}

/// ExcelA1: schedules an operand of a union printed inside its own
/// parentheses. A left-nested union continues the same list (`(A1,B1,C1)`);
/// anything binding looser than union (and a right-nested union) gets
/// parentheses.
fn push_union_child<'e>(stack: &mut Vec<Work<'e>>, x: &'e Expr, right: bool) {
    let p = BinOp::Union.precedence();
    if !right && matches!(x, Expr::Binary(BinOp::Union, _, _)) {
        stack.push(Work::Bare(x));
    } else {
        let wrap = if right {
            x.precedence() <= p
        } else {
            x.precedence() < p
        };
        push_child(stack, x, wrap);
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
    /// ExcelA1: an expression whose union (if it is one) needs no extra
    /// parentheses (it is directly inside `(...)` or another union).
    Bare(&'e Expr),
    Char(char),
    Str(&'static str),
}

/// ExcelA1: does an unquoted sheet name read as an R1C1 reference
/// (`R1C1`, `R2`, `C3`, `RC`)? Excel quotes those.
fn looks_like_r1c1(name: &str) -> bool {
    let u = name.to_ascii_uppercase();
    let b = u.as_bytes();
    let mut i = 0;
    let digits = |i: &mut usize| {
        while *i < b.len() && b[*i].is_ascii_digit() {
            *i += 1;
        }
    };
    let mut any = false;
    if b.get(i) == Some(&b'R') {
        i += 1;
        digits(&mut i);
        any = true;
    }
    if b.get(i) == Some(&b'C') {
        i += 1;
        digits(&mut i);
        any = true;
    }
    any && i == b.len()
}

/// Excel's spelling of an error literal (`#VALUE!` for OOo-only codes).
fn excel_error(k: tally_types::ErrorKind) -> std::borrow::Cow<'static, str> {
    let d = k.display();
    if d.starts_with('#') {
        d
    } else {
        std::borrow::Cow::Borrowed("#VALUE!")
    }
}

impl Printer<'_> {
    /// Prints `root` with an explicit work stack (no recursion), so trees of
    /// any depth are safe on small stacks.
    fn expr(&mut self, root: &Expr) {
        let excel = self.dialect.is_excel();
        let mut stack: Vec<Work<'_>> = vec![Work::Expr(root)];
        while let Some(w) = stack.pop() {
            let (e, bare) = match w {
                Work::Char(c) => {
                    self.out.push(c);
                    continue;
                }
                Work::Str(s) => {
                    self.out.push_str(s);
                    continue;
                }
                Work::Expr(e) => (e, false),
                Work::Bare(e) => (e, true),
            };
            // ExcelA1 unions only parse inside parentheses: `SUM((A1,B1))`.
            if excel && !bare && matches!(e, Expr::Binary(BinOp::Union, _, _)) {
                stack.push(Work::Char(')'));
                stack.push(Work::Bare(e));
                stack.push(Work::Char('('));
                continue;
            }
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
                Expr::Err(k) if excel => self.out.push_str(&excel_error(*k)),
                Expr::Err(k) => self.out.push_str(&k.display()),
                Expr::ArrayLit(a) => {
                    self.out.push('{');
                    stack.push(Work::Char('}'));
                    let cols = a.cols.max(1) as usize;
                    let (col_sep, row_sep) = if excel { (',', ';') } else { (';', '|') };
                    for (i, item) in a.items.iter().enumerate().rev() {
                        match item {
                            // Plain constants inside arrays in every dialect.
                            Expr::Bool(true) => stack.push(Work::Str("TRUE")),
                            Expr::Bool(false) => stack.push(Work::Str("FALSE")),
                            other => stack.push(Work::Expr(other)),
                        }
                        if i > 0 {
                            stack.push(Work::Char(if i % cols == 0 { row_sep } else { col_sep }));
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
                        FuncRef::Unknown(n) if excel => push_excel_unknown(&mut self.out, n),
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
                Expr::Binary(BinOp::Union, l, r) if excel => {
                    // Inside the union's own parentheses: `(A1,B1,C1)`.
                    push_union_child(&mut stack, r, true);
                    stack.push(Work::Char(','));
                    push_union_child(&mut stack, l, false);
                }
                Expr::Binary(op, l, r) => {
                    let p = op.precedence();
                    // `Name!A1` (also `x!Name!A1`, `A:A!1`) would re-lex
                    // with a sheet qualifier.
                    let guard_bang = !excel && *op == BinOp::Intersect && self.ends_in_word(l);
                    // `g:W.rq`, `a:a`, `1:1` and `x:A1` then `:B2` would
                    // re-lex as one reference token (a column range on sheet
                    // `W`, whole columns, whole rows, the area `A1:B2`); the
                    // right operand then gets parentheses. (A bare `A1:B2`
                    // is already an area.)
                    let guard_range = *op == BinOp::Range && self.guard_range(l, r);
                    // ExcelA1 intersection is a space: see `excel_isect_parens`.
                    let (wrap_l, wrap_r) = if excel && *op == BinOp::Intersect {
                        self.excel_isect_parens(l, r)
                    } else {
                        (l.precedence() < p, r.precedence() <= p)
                    };
                    push_child(&mut stack, r, wrap_r || guard_range);
                    stack.push(Work::Str(if excel && *op == BinOp::Intersect {
                        " "
                    } else {
                        op.symbol()
                    }));
                    push_child(&mut stack, l, wrap_l || guard_bang);
                }
                Expr::Postfix(PostfixOp::Percent, x) => {
                    stack.push(Work::Char('%'));
                    push_child(&mut stack, x, x.precedence() < POSTFIX_PRECEDENCE);
                }
                Expr::Paren(x) => {
                    stack.push(Work::Char(')'));
                    stack.push(if excel { Work::Bare(x) } else { Work::Expr(x) });
                    stack.push(Work::Char('('));
                }
                Expr::Missing => {}
            }
        }
    }

    /// Whether the right operand of `l:r` needs parentheses because it would
    /// merge with the left one into one reference token (`a:a`, `g:W.rq`).
    /// (A bare `A1:B2` is already an area.)
    fn guard_range(&self, l: &Expr, r: &Expr) -> bool {
        !matches!(
            (l, r),
            (Expr::Ref(RefExpr::Cell(_)), Expr::Ref(RefExpr::Cell(_)))
        ) && self.range_merges(last_operand(l), r)
    }

    /// ExcelA1: which operands of the intersection `l r` (written with a
    /// space) need parentheses, as `(left, right)`.
    ///
    /// Besides precedence: a constant operand (`1`, `"x"`, `TRUE`, `{1}`)
    /// is bracketed, since Excel intersects references only and a space
    /// before a number or string does not read as the operator. A left
    /// operand would also be bracketed if it plus ` (` lexed as a function
    /// call; the ExcelA1 lexer, like Excel, reads `Name (A1)` as an
    /// intersection, so this is only a guard that the real lexer decides.
    fn excel_isect_parens(&self, l: &Expr, r: &Expr) -> (bool, bool) {
        let p = BinOp::Intersect.precedence();
        let first = first_operand(r);
        let wrap_r = r.precedence() <= p || is_constant(first);
        let r_paren = wrap_r || matches!(first, Expr::Paren(_) | Expr::Binary(BinOp::Union, _, _));
        let last = self.excel_last_operand(l);
        let wrap_l =
            l.precedence() < p || is_constant(last) || (r_paren && self.lexes_as_call(last));
        (wrap_l, wrap_r)
    }

    /// ExcelA1: the operand printed last in `e` without parentheses after
    /// it, or the expression whose printed form ends in `)`.
    fn excel_last_operand<'e>(&self, mut e: &'e Expr) -> &'e Expr {
        loop {
            let next = match e {
                Expr::Binary(BinOp::Range, l, r)
                    if r.precedence() > BinOp::Range.precedence() && !self.guard_range(l, r) =>
                {
                    r
                }
                Expr::Binary(BinOp::Intersect, l, r) if !self.excel_isect_parens(l, r).1 => r,
                _ => return e,
            };
            e = next;
        }
    }

    /// Whether the leaf `e` followed by ` (` lexes as a function call in
    /// the target dialect.
    fn lexes_as_call(&self, e: &Expr) -> bool {
        let Some(t) = self.leaf_text(e) else {
            return false;
        };
        lex(&format!("{t} ("), self.dialect)
            .0
            .iter()
            .any(|t| t.kind == TokenKind::Func)
    }

    /// The text of a leaf operand as printed, or `None` for anything that
    /// prints with brackets or parentheses around it.
    fn leaf_text(&self, e: &Expr) -> Option<String> {
        match e {
            Expr::Name(NameRef::Unresolved(t)) => Some(t.to_string()),
            Expr::Name(NameRef::Id(id)) => self.resolver.name_text(*id).map(|t| t.to_string()),
            Expr::Num(x) => Some(fmt_num(*x)),
            Expr::Bool(b) => Some((if *b { "TRUE" } else { "FALSE" }).to_string()),
            Expr::Ref(r) => self.a1_ref(&r.clone().normalized(self.origin)),
            _ => None,
        }
    }

    /// Whether `left:right`, printed without parentheses, would re-lex in
    /// the target dialect with one token spanning the `:` (`a:a`, `g:W.rq`,
    /// `A1:B2`). Bracketed OpenFormula references never merge that way.
    fn range_merges(&self, left: &Expr, right: &Expr) -> bool {
        if self.dialect.is_open_formula() {
            return false;
        }
        let (Some(lt), Some(rt)) = (self.leaf_text(left), self.leaf_text(right)) else {
            return false;
        };
        let text = format!("{lt}:{rt}");
        let at = lt.len();
        lex(&text, self.dialect)
            .0
            .iter()
            .any(|t| t.span.start < at && t.span.end > at)
    }

    /// Whether the printed `e` ends in a word (a name, `TRUE`, the column
    /// letters of `A:B`) that a following `!` would turn into a sheet name.
    fn ends_in_word(&self, e: &Expr) -> bool {
        match last_operand(e) {
            Expr::Name(_) | Expr::Bool(_) => true,
            Expr::Ref(RefExpr::Cols { .. }) => !self.dialect.is_open_formula(),
            _ => false,
        }
    }

    fn reference(&mut self, r: &RefExpr) {
        let of = self.dialect.is_open_formula();
        // Reversed ranges print in order (AOO), in every dialect.
        let r = &r.clone().normalized(self.origin);
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
        if self.dialect.is_excel() {
            // Excel has no `$Sheet` marker.
            push_excel_sheet_name(&mut out, &name);
            return Some(out);
        }
        if abs {
            out.push('$');
        }
        push_sheet_name(&mut out, &name);
        Some(out)
    }

    /// The bare (unquoted) name of a sheet, or `None` for a deleted or
    /// unknown sheet (and for [`SheetRef::Current`]).
    fn sheet_raw_name(&self, s: &SheetRef) -> Option<std::sync::Arc<str>> {
        match s {
            SheetRef::Id { id, .. } => self.resolver.sheet_name(*id),
            SheetRef::Unresolved(n) => Some(n.clone()),
            SheetRef::Current | SheetRef::Deleted => None,
        }
    }

    fn sheet_sep(&self) -> char {
        match self.dialect {
            Dialect::CalcA1 {
                sheet_sep: SheetSep::Bang,
                ..
            }
            | Dialect::ExcelA1 => '!',
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
            RefExpr::Area(a, b) if self.dialect.is_excel() => {
                self.excel_area(a, b, &mut out)?;
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

    /// ExcelA1 area: `Sheet1!A1:B2`, or a 3D `Sheet1:Sheet3!A1:B2` when the
    /// endpoints name different sheets.
    fn excel_area(&self, a: &CellRefExpr, b: &CellRefExpr, out: &mut String) -> Option<()> {
        let same_sheet = match (&a.sheet, &b.sheet) {
            (_, SheetRef::Current) => true,
            (SheetRef::Deleted, _) | (_, SheetRef::Deleted) => return None,
            (x, y) => match (self.sheet_raw_name(x), self.sheet_raw_name(y)) {
                (Some(n1), Some(n2)) => n1.eq_ignore_ascii_case(&n2),
                (None, None) => true,
                _ => false,
            },
        };
        if same_sheet {
            self.a1_sheet(&a.sheet, out)?;
            self.coords(a, out)?;
            out.push(':');
            return self.coords(b, out);
        }
        // `A1:Sheet2!B2`: the start is on the formula's own sheet, which
        // Excel has to name (`Sheet1:Sheet2!A1:B2`).
        let first = match &a.sheet {
            SheetRef::Current => self.resolver.sheet_name(self.origin.sheet)?,
            s => self.sheet_raw_name(s)?,
        };
        let last = self.sheet_raw_name(&b.sheet)?;
        let quote = excel_needs_quotes(&first) || excel_needs_quotes(&last);
        if quote {
            out.push('\'');
            out.push_str(&first.replace('\'', "''"));
            out.push(':');
            out.push_str(&last.replace('\'', "''"));
            out.push('\'');
        } else {
            out.push_str(&first);
            out.push(':');
            out.push_str(&last);
        }
        out.push('!');
        self.coords(a, out)?;
        if a.row != b.row || a.col != b.col {
            out.push(':');
            self.coords(b, out)?;
        }
        Some(())
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
            // AOO 4.1 has no whole-column/row references (`[.A:.A]`), so
            // they are written as the full area with absolute rows (or
            // columns), which the OpenFormula parser reads back as whole
            // columns (rows).
            RefExpr::Cols { sheet, start, end } => {
                self.of_sheet(sheet, &mut out)?;
                self.col(*start, &mut out)?;
                out.push_str("$1:.");
                self.col(*end, &mut out)?;
                let _ = write!(out, "${MAX_ROWS}");
            }
            RefExpr::Rows { sheet, start, end } => {
                self.of_sheet(sheet, &mut out)?;
                out.push_str("$A");
                self.row(*start, &mut out)?;
                out.push_str(":.$");
                out.push_str(&col_to_letters(MAX_COLS - 1));
                self.row(*end, &mut out)?;
            }
            RefExpr::Invalid => return None,
        }
        out.push(']');
        Some(out)
    }
}

/// An unknown function in ExcelA1: `_xlfn.` / `_xlws.` prefixes are
/// written in lowercase, as Excel stores them.
fn push_excel_unknown(out: &mut String, name: &str) {
    let mut rest = name;
    loop {
        let has = |p: &str| {
            rest.len() > p.len()
                && rest.is_char_boundary(p.len())
                && rest[..p.len()].eq_ignore_ascii_case(p)
        };
        if has("_xlfn.") {
            out.push_str("_xlfn.");
        } else if has("_xlws.") {
            out.push_str("_xlws.");
        } else {
            break;
        }
        rest = &rest[6..];
    }
    out.push_str(rest);
}

/// Whether Excel needs a sheet name quoted.
fn excel_needs_quotes(name: &str) -> bool {
    sheet_needs_quotes(name) || looks_like_r1c1(name)
}

/// Appends a sheet name the way Excel writes it (quoted when needed).
fn push_excel_sheet_name(out: &mut String, name: &str) {
    if excel_needs_quotes(name) {
        out.push('\'');
        out.push_str(&name.replace('\'', "''"));
        out.push('\'');
    } else {
        out.push_str(name);
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
