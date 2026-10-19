//! Operator-precedence parser.
//!
//! This is a Pratt parser driven by explicit operand and operator stacks
//! instead of Rust recursion, so hostile input (`((((...` ten thousand deep)
//! can never overflow the stack. Nesting is still limited to
//! [`MAX_NESTING`] parentheses/calls and the resulting tree to
//! [`MAX_AST_DEPTH`] levels, both reported as Err:512.
//!
//! Precedence, loosest to tightest: comparisons, `&`, `+ -`, `* /`, `^`
//! (left-associative), prefix `- +`, postfix `%`, `~`, `!`, `:`.

use std::fmt;
use std::ops::Range;
use std::sync::Arc;

use tally_types::{CellAddr, ErrorKind};

use crate::ast::{
    ArrayLit, BinOp, CellRefExpr, Expr, Formula, FuncRef, NameRef, PostfixOp, RefCoord, RefExpr,
    SheetRef, UnOp, POSTFIX_PRECEDENCE, PREFIX_PRECEDENCE,
};
use crate::dialect::Dialect;
use crate::lexer::{lex, TokenKind};
use crate::refsyntax::{parse_a1_ref, parse_of_ref, Part, RawEndpoint, RawRef, RawSheet};
use crate::Resolver;

/// Maximum formula length in characters (longer gives Err:512).
pub const MAX_FORMULA_LEN: usize = 8192;
/// Maximum nesting of parentheses and function calls (deeper gives Err:512).
pub const MAX_NESTING: usize = 256;
/// Maximum depth of the resulting tree (deeper gives Err:512). Long operator
/// chains such as `1+1+...+1` count one level per operator.
pub const MAX_AST_DEPTH: u32 = 1024;
/// Maximum number of arguments to one function (more gives Err:504).
pub const MAX_ARGS: usize = 255;

/// A parse failure: the OpenOffice error code and the offending byte span.
#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub struct ParseError {
    /// Err:501 invalid character, 504 parameter list, 508 bracket, 509
    /// missing operator, 510 missing variable, 512 formula overflow, 503
    /// numeric literal overflow.
    pub kind: ErrorKind,
    /// UTF-8 byte span in the source.
    pub span: Range<usize>,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} at {}..{}",
            self.kind.display(),
            self.span.start,
            self.span.end
        )
    }
}

impl std::error::Error for ParseError {}

/// Parses a formula. `src` may start with `=`, `of:=` or `oooc:=` (the latter
/// two switch to OpenFormula syntax); without a prefix it is parsed as a bare
/// expression. Relative references are encoded against `origin`.
pub fn parse(
    src: &str,
    origin: CellAddr,
    dialect: Dialect,
    resolver: &dyn Resolver,
) -> Result<Formula, ParseError> {
    if src.len() > MAX_FORMULA_LEN && src.chars().count() > MAX_FORMULA_LEN {
        return Err(ParseError {
            kind: ErrorKind::FormulaOverflow,
            span: 0..src.len(),
        });
    }
    let (tokens, _of) = lex(src, dialect);
    let toks: Vec<(TokenKind, Range<usize>)> = tokens
        .into_iter()
        .filter(|t| !t.kind.is_trivia())
        .map(|t| (t.kind, t.span))
        .collect();
    let mut p = Parser {
        src,
        toks,
        pos: 0,
        origin,
        resolver,
        operands: Vec::new(),
        frames: Vec::new(),
        nesting: 0,
    };
    let expr = p.run()?;
    Ok(Formula::new(expr))
}

struct Operand {
    expr: Expr,
    depth: u32,
}

enum Pending {
    Bin(BinOp),
    Prefix(UnOp),
}

enum Frame {
    Op {
        op: Pending,
        span: Range<usize>,
    },
    Paren {
        span: Range<usize>,
    },
    Call {
        func: FuncRef,
        args: Vec<Expr>,
        depth: u32,
        span: Range<usize>,
        /// Uppercased name, used to fold `TRUE()`/`FALSE()`.
        name: Arc<str>,
    },
}

struct Parser<'a> {
    src: &'a str,
    toks: Vec<(TokenKind, Range<usize>)>,
    pos: usize,
    origin: CellAddr,
    resolver: &'a dyn Resolver,
    operands: Vec<Operand>,
    frames: Vec<Frame>,
    nesting: usize,
}

fn err<T>(kind: ErrorKind, span: Range<usize>) -> Result<T, ParseError> {
    Err(ParseError { kind, span })
}

fn bin_op(kind: TokenKind) -> Option<BinOp> {
    Some(match kind {
        TokenKind::Plus => BinOp::Add,
        TokenKind::Minus => BinOp::Sub,
        TokenKind::Star => BinOp::Mul,
        TokenKind::Slash => BinOp::Div,
        TokenKind::Caret => BinOp::Pow,
        TokenKind::Amp => BinOp::Concat,
        TokenKind::Eq => BinOp::Eq,
        TokenKind::Ne => BinOp::Ne,
        TokenKind::Lt => BinOp::Lt,
        TokenKind::Le => BinOp::Le,
        TokenKind::Gt => BinOp::Gt,
        TokenKind::Ge => BinOp::Ge,
        TokenKind::Colon => BinOp::Range,
        TokenKind::Bang => BinOp::Intersect,
        TokenKind::Tilde => BinOp::Union,
        _ => return None,
    })
}

impl Parser<'_> {
    fn eof_span(&self) -> Range<usize> {
        self.src.len()..self.src.len()
    }

    fn text(&self, span: &Range<usize>) -> &str {
        self.src.get(span.clone()).unwrap_or("")
    }

    fn run(&mut self) -> Result<Expr, ParseError> {
        let mut expect_operand = true;
        // True right after `NAME(` or an argument separator inside a call.
        let mut at_arg_start = false;
        while self.pos < self.toks.len() {
            let (kind, span) = self.toks[self.pos].clone();
            self.pos += 1;
            if kind == TokenKind::Invalid {
                return err(ErrorKind::InvalidChar, span);
            }
            if expect_operand {
                match kind {
                    TokenKind::Minus | TokenKind::Plus => {
                        let op = if kind == TokenKind::Minus {
                            UnOp::Neg
                        } else {
                            UnOp::Plus
                        };
                        self.frames.push(Frame::Op {
                            op: Pending::Prefix(op),
                            span,
                        });
                        at_arg_start = false;
                    }
                    TokenKind::LParen => {
                        self.push_nesting(&span)?;
                        self.frames.push(Frame::Paren { span });
                        at_arg_start = false;
                    }
                    TokenKind::Func => {
                        self.open_call(span)?;
                        at_arg_start = true;
                    }
                    TokenKind::LBrace => {
                        let arr = self.array_literal(span)?;
                        self.operands.push(Operand {
                            expr: Expr::ArrayLit(arr),
                            depth: 2,
                        });
                        expect_operand = false;
                        at_arg_start = false;
                    }
                    TokenKind::RParen => {
                        let in_call = matches!(self.frames.last(), Some(Frame::Call { .. }));
                        if !(in_call && at_arg_start) {
                            return err(ErrorKind::MissingVariable, span);
                        }
                        if let Some(Frame::Call { args, .. }) = self.frames.last_mut() {
                            // `F()` has no arguments; `F(1,)` ends in Missing.
                            if !args.is_empty() {
                                args.push(Expr::Missing);
                            }
                        }
                        self.close_call(span)?;
                        expect_operand = false;
                        at_arg_start = false;
                    }
                    TokenKind::Comma | TokenKind::Semicolon => {
                        let in_call = matches!(self.frames.last(), Some(Frame::Call { .. }));
                        if !(in_call && at_arg_start) {
                            return err(ErrorKind::MissingVariable, span);
                        }
                        if let Some(Frame::Call { args, .. }) = self.frames.last_mut() {
                            args.push(Expr::Missing);
                            if args.len() > MAX_ARGS {
                                return err(ErrorKind::ParamList, span);
                            }
                        }
                    }
                    _ => {
                        let expr = self.atom(kind, &span)?;
                        self.operands.push(Operand { expr, depth: 1 });
                        expect_operand = false;
                        at_arg_start = false;
                    }
                }
            } else if let Some(op) = bin_op(kind) {
                self.reduce_while(|p| p >= op.precedence())?;
                self.frames.push(Frame::Op {
                    op: Pending::Bin(op),
                    span,
                });
                expect_operand = true;
            } else {
                match kind {
                    TokenKind::Percent => {
                        self.reduce_while(|p| p > POSTFIX_PRECEDENCE)?;
                        let Some(top) = self.operands.pop() else {
                            return err(ErrorKind::MissingVariable, span);
                        };
                        let expr = Expr::Postfix(PostfixOp::Percent, Box::new(top.expr));
                        self.push_operand(expr, top.depth + 1, &span)?;
                    }
                    TokenKind::RParen => {
                        self.reduce_while(|_| true)?;
                        match self.frames.last() {
                            Some(Frame::Paren { .. }) => {
                                self.frames.pop();
                                self.nesting = self.nesting.saturating_sub(1);
                                let Some(top) = self.operands.pop() else {
                                    return err(ErrorKind::MissingVariable, span);
                                };
                                let expr = Expr::Paren(Box::new(top.expr));
                                self.push_operand(expr, top.depth + 1, &span)?;
                            }
                            Some(Frame::Call { .. }) => {
                                self.push_arg(&span)?;
                                self.close_call(span)?;
                            }
                            _ => return err(ErrorKind::Bracket, span),
                        }
                    }
                    TokenKind::Comma | TokenKind::Semicolon => {
                        self.reduce_while(|_| true)?;
                        if !matches!(self.frames.last(), Some(Frame::Call { .. })) {
                            return err(ErrorKind::ParamList, span);
                        }
                        self.push_arg(&span)?;
                        expect_operand = true;
                        at_arg_start = true;
                    }
                    TokenKind::RBrace | TokenKind::Pipe => {
                        return err(ErrorKind::ParamList, span);
                    }
                    _ => return err(ErrorKind::MissingOperator, span),
                }
            }
        }
        // End of input.
        if expect_operand {
            if let Some(f) = self
                .frames
                .iter()
                .rev()
                .find(|f| !matches!(f, Frame::Op { .. }))
            {
                let span = match f {
                    Frame::Paren { span } | Frame::Call { span, .. } | Frame::Op { span, .. } => {
                        span.clone()
                    }
                };
                return err(ErrorKind::Bracket, span);
            }
            return err(ErrorKind::MissingVariable, self.eof_span());
        }
        self.reduce_while(|_| true)?;
        if let Some(f) = self.frames.last() {
            let span = match f {
                Frame::Paren { span } | Frame::Call { span, .. } | Frame::Op { span, .. } => {
                    span.clone()
                }
            };
            return err(ErrorKind::Bracket, span);
        }
        match (self.operands.pop(), self.operands.is_empty()) {
            (Some(o), true) => Ok(o.expr),
            _ => err(ErrorKind::MissingOperator, self.eof_span()),
        }
    }

    fn push_nesting(&mut self, span: &Range<usize>) -> Result<(), ParseError> {
        self.nesting += 1;
        if self.nesting > MAX_NESTING {
            return err(ErrorKind::FormulaOverflow, span.clone());
        }
        Ok(())
    }

    fn push_operand(
        &mut self,
        expr: Expr,
        depth: u32,
        span: &Range<usize>,
    ) -> Result<(), ParseError> {
        if depth > MAX_AST_DEPTH {
            return err(ErrorKind::FormulaOverflow, span.clone());
        }
        self.operands.push(Operand { expr, depth });
        Ok(())
    }

    fn open_call(&mut self, span: Range<usize>) -> Result<(), ParseError> {
        // The lexer guarantees `(` follows (after optional whitespace).
        match self.toks.get(self.pos) {
            Some((TokenKind::LParen, _)) => self.pos += 1,
            _ => return err(ErrorKind::Bracket, span),
        }
        self.push_nesting(&span)?;
        let name: Arc<str> = Arc::from(self.text(&span).to_ascii_uppercase());
        let func = match self.resolver.function(&name) {
            Some(id) => FuncRef::Known(id),
            None => FuncRef::Unknown(name.clone()),
        };
        self.frames.push(Frame::Call {
            func,
            args: Vec::new(),
            depth: 1,
            span,
            name,
        });
        Ok(())
    }

    fn push_arg(&mut self, span: &Range<usize>) -> Result<(), ParseError> {
        let Some(top) = self.operands.pop() else {
            return err(ErrorKind::MissingVariable, span.clone());
        };
        if let Some(Frame::Call { args, depth, .. }) = self.frames.last_mut() {
            args.push(top.expr);
            *depth = (*depth).max(top.depth);
            if args.len() > MAX_ARGS {
                return err(ErrorKind::ParamList, span.clone());
            }
        }
        Ok(())
    }

    fn close_call(&mut self, span: Range<usize>) -> Result<(), ParseError> {
        let Some(Frame::Call {
            func,
            args,
            depth,
            name,
            ..
        }) = self.frames.pop()
        else {
            return err(ErrorKind::Bracket, span);
        };
        self.nesting = self.nesting.saturating_sub(1);
        // `TRUE()`/`FALSE()` are the boolean constants (OpenFormula writes
        // them this way); fold them so every dialect round-trips.
        let expr = if args.is_empty() && (&*name == "TRUE" || &*name == "FALSE") {
            Expr::Bool(&*name == "TRUE")
        } else {
            Expr::Call { func, args }
        };
        self.push_operand(expr, depth + 1, &span)
    }

    /// Reduces pending operators on top of the stack (up to the nearest
    /// paren/call frame) while `pred(precedence)` holds.
    fn reduce_while(&mut self, pred: impl Fn(u8) -> bool) -> Result<(), ParseError> {
        loop {
            let prec = match self.frames.last() {
                Some(Frame::Op {
                    op: Pending::Bin(op),
                    ..
                }) => op.precedence(),
                Some(Frame::Op {
                    op: Pending::Prefix(_),
                    ..
                }) => PREFIX_PRECEDENCE,
                _ => return Ok(()),
            };
            if !pred(prec) {
                return Ok(());
            }
            let Some(Frame::Op { op, span }) = self.frames.pop() else {
                return Ok(());
            };
            match op {
                Pending::Prefix(u) => {
                    let Some(x) = self.operands.pop() else {
                        return err(ErrorKind::MissingVariable, span);
                    };
                    self.push_operand(Expr::Unary(u, Box::new(x.expr)), x.depth + 1, &span)?;
                }
                Pending::Bin(b) => {
                    let (Some(r), Some(l)) = (self.operands.pop(), self.operands.pop()) else {
                        return err(ErrorKind::MissingVariable, span);
                    };
                    let depth = l.depth.max(r.depth) + 1;
                    let expr = match (b, l.expr, r.expr) {
                        // `Sheet1.A1 : B2` written with spaces is still an area.
                        (
                            BinOp::Range,
                            Expr::Ref(RefExpr::Cell(a)),
                            Expr::Ref(RefExpr::Cell(c)),
                        ) => Expr::Ref(RefExpr::Area(a, c)),
                        (b, le, re) => Expr::Binary(b, Box::new(le), Box::new(re)),
                    };
                    self.push_operand(expr, depth, &span)?;
                }
            }
        }
    }

    fn atom(&self, kind: TokenKind, span: &Range<usize>) -> Result<Expr, ParseError> {
        let text = self.text(span);
        match kind {
            TokenKind::Number => self.number(text, span),
            TokenKind::String => Ok(Expr::Str(unescape_string(text).into())),
            TokenKind::Error => error_literal(text, span),
            TokenKind::Ident => Ok(self.ident(text)),
            TokenKind::CellRef | TokenKind::RangeRef => {
                let Some(raw) = parse_a1_ref(text) else {
                    return err(ErrorKind::InvalidChar, span.clone());
                };
                self.build_ref(raw, span).map(Expr::Ref)
            }
            TokenKind::OfRef => {
                let inner = text
                    .strip_prefix('[')
                    .and_then(|t| t.strip_suffix(']'))
                    .unwrap_or("");
                let Some(raw) = parse_of_ref(inner) else {
                    return err(ErrorKind::InvalidChar, span.clone());
                };
                self.build_ref(raw, span).map(Expr::Ref)
            }
            TokenKind::RBrace | TokenKind::Pipe => err(ErrorKind::ParamList, span.clone()),
            _ => err(ErrorKind::MissingVariable, span.clone()),
        }
    }

    fn number(&self, text: &str, span: &Range<usize>) -> Result<Expr, ParseError> {
        match text.parse::<f64>() {
            Ok(v) if v.is_finite() => Ok(Expr::Num(v)),
            Ok(_) => err(ErrorKind::Num, span.clone()),
            Err(_) => err(ErrorKind::InvalidChar, span.clone()),
        }
    }

    fn ident(&self, text: &str) -> Expr {
        if text.eq_ignore_ascii_case("TRUE") {
            return Expr::Bool(true);
        }
        if text.eq_ignore_ascii_case("FALSE") {
            return Expr::Bool(false);
        }
        match self.resolver.name(text, self.origin.sheet) {
            Some(id) => Expr::Name(NameRef::Id(id)),
            None => Expr::Name(NameRef::Unresolved(text.into())),
        }
    }

    fn sheet(&self, raw: &Option<RawSheet>) -> SheetRef {
        match raw {
            None => SheetRef::Current,
            Some(s) if s.deleted => SheetRef::Deleted,
            Some(s) => match self.resolver.sheet_by_name(&s.name) {
                Some(id) => SheetRef::Id { id, abs: s.abs },
                None => SheetRef::Unresolved(s.name.as_str().into()),
            },
        }
    }

    fn cell(&self, ep: &RawEndpoint, row: u32, col: u32, ar: bool, ac: bool) -> CellRefExpr {
        CellRefExpr {
            sheet: self.sheet(&ep.sheet),
            row: RefCoord::encode(row, ar, self.origin.row),
            col: RefCoord::encode(col, ac, self.origin.col),
        }
    }

    fn build_ref(&self, raw: RawRef, span: &Range<usize>) -> Result<RefExpr, ParseError> {
        let start_part = raw.start.part;
        let end_part = raw.end.as_ref().map(|e| e.part);
        let Some(sp) = start_part else {
            return Ok(RefExpr::Invalid);
        };
        if let Some(None) = end_part {
            return Ok(RefExpr::Invalid);
        }
        let end = raw.end.as_ref().and_then(|e| e.part.map(|p| (e, p)));
        match (sp, end) {
            (
                Part::Cell {
                    row,
                    col,
                    abs_row,
                    abs_col,
                },
                None,
            ) => Ok(RefExpr::Cell(
                self.cell(&raw.start, row, col, abs_row, abs_col),
            )),
            (
                Part::Cell {
                    row,
                    col,
                    abs_row,
                    abs_col,
                },
                Some((
                    ep,
                    Part::Cell {
                        row: r2,
                        col: c2,
                        abs_row: ar2,
                        abs_col: ac2,
                    },
                )),
            ) => Ok(RefExpr::Area(
                self.cell(&raw.start, row, col, abs_row, abs_col),
                self.cell(ep, r2, c2, ar2, ac2),
            )),
            (Part::Col { col, abs }, Some((ep, Part::Col { col: c2, abs: a2 }))) => {
                self.check_same_sheet(&raw.start, ep, span)?;
                Ok(RefExpr::Cols {
                    sheet: self.sheet(&raw.start.sheet),
                    start: RefCoord::encode(col, abs, self.origin.col),
                    end: RefCoord::encode(c2, a2, self.origin.col),
                })
            }
            (Part::Row { row, abs }, Some((ep, Part::Row { row: r2, abs: a2 }))) => {
                self.check_same_sheet(&raw.start, ep, span)?;
                Ok(RefExpr::Rows {
                    sheet: self.sheet(&raw.start.sheet),
                    start: RefCoord::encode(row, abs, self.origin.row),
                    end: RefCoord::encode(r2, a2, self.origin.row),
                })
            }
            _ => err(ErrorKind::InvalidChar, span.clone()),
        }
    }

    /// Whole-column/row ranges live on one sheet: an explicit second sheet
    /// must name the same sheet as the first.
    fn check_same_sheet(
        &self,
        a: &RawEndpoint,
        b: &RawEndpoint,
        span: &Range<usize>,
    ) -> Result<(), ParseError> {
        match (&a.sheet, &b.sheet) {
            (_, None) => Ok(()),
            (Some(x), Some(y)) if x.name.eq_ignore_ascii_case(&y.name) => Ok(()),
            _ => err(ErrorKind::InvalidChar, span.clone()),
        }
    }

    /// Parses `{1;2|3;4}` after the `{` (iteratively; arrays cannot nest).
    fn array_literal(&mut self, open: Range<usize>) -> Result<ArrayLit, ParseError> {
        let mut items = Vec::new();
        let mut cols: Option<u32> = None;
        let mut row_len: u32 = 0;
        let mut rows: u32 = 0;
        loop {
            // Element: optional signs then a literal.
            let mut negate = false;
            let mut signed = false;
            let elem = loop {
                let Some((kind, span)) = self.toks.get(self.pos).cloned() else {
                    return err(ErrorKind::Bracket, open);
                };
                self.pos += 1;
                match kind {
                    TokenKind::Minus => {
                        negate = !negate;
                        signed = true;
                    }
                    TokenKind::Plus => signed = true,
                    TokenKind::Number => {
                        let Expr::Num(v) = self.number(self.text(&span), &span)? else {
                            return err(ErrorKind::ParamList, span);
                        };
                        break Expr::Num(if negate { -v } else { v });
                    }
                    _ if signed => return err(ErrorKind::ParamList, span),
                    TokenKind::String => break Expr::Str(unescape_string(self.text(&span)).into()),
                    TokenKind::Error => break error_literal(self.text(&span), &span)?,
                    TokenKind::Ident if self.text(&span).eq_ignore_ascii_case("TRUE") => {
                        break Expr::Bool(true)
                    }
                    TokenKind::Ident if self.text(&span).eq_ignore_ascii_case("FALSE") => {
                        break Expr::Bool(false)
                    }
                    TokenKind::Func => {
                        let t = self.text(&span).to_ascii_uppercase();
                        let closed = matches!(
                            (self.toks.get(self.pos), self.toks.get(self.pos + 1)),
                            (Some((TokenKind::LParen, _)), Some((TokenKind::RParen, _)))
                        );
                        if closed && (t == "TRUE" || t == "FALSE") {
                            self.pos += 2;
                            break Expr::Bool(t == "TRUE");
                        }
                        return err(ErrorKind::ParamList, span);
                    }
                    TokenKind::Semicolon | TokenKind::Pipe | TokenKind::RBrace => {
                        return err(ErrorKind::MissingVariable, span)
                    }
                    TokenKind::Invalid => return err(ErrorKind::InvalidChar, span),
                    _ => return err(ErrorKind::ParamList, span),
                }
            };
            items.push(elem);
            row_len += 1;
            let Some((kind, span)) = self.toks.get(self.pos).cloned() else {
                return err(ErrorKind::Bracket, open);
            };
            self.pos += 1;
            match kind {
                TokenKind::Semicolon => {}
                TokenKind::Pipe | TokenKind::RBrace => {
                    match cols {
                        None => cols = Some(row_len),
                        Some(c) if c == row_len => {}
                        Some(_) => return err(ErrorKind::ParamList, span),
                    }
                    rows += 1;
                    row_len = 0;
                    if kind == TokenKind::RBrace {
                        return Ok(ArrayLit {
                            rows,
                            cols: cols.unwrap_or(0),
                            items,
                        });
                    }
                }
                TokenKind::Invalid => return err(ErrorKind::InvalidChar, span),
                _ => return err(ErrorKind::ParamList, span),
            }
        }
    }
}

fn error_literal(text: &str, span: &Range<usize>) -> Result<Expr, ParseError> {
    let normalized;
    let t = if text.len() > 4 && text.is_char_boundary(4) && text[..4].eq_ignore_ascii_case("err:")
    {
        normalized = format!("Err:{}", &text[4..]);
        normalized.as_str()
    } else {
        text
    };
    match ErrorKind::parse(t) {
        Some(ErrorKind::Ref) => Ok(Expr::Ref(RefExpr::Invalid)),
        Some(k) => Ok(Expr::Err(k)),
        None => err(ErrorKind::InvalidChar, span.clone()),
    }
}

/// Strips the quotes from a string token and collapses `""` to `"`.
pub(crate) fn unescape_string(text: &str) -> String {
    let inner = text
        .strip_prefix('"')
        .and_then(|t| t.strip_suffix('"'))
        .unwrap_or(text);
    inner.replace("\"\"", "\"")
}
