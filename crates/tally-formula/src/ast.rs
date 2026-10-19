//! The formula AST, with references stored relative to the formula's cell.
//!
//! A [`RefCoord`] is either an absolute 0-based index (`abs == true`, written
//! `$A`/`$1`) or an offset from the formula's origin cell (`abs == false`).
//! The same AST therefore serves every cell of a fill-down, and copying a
//! formula is just printing it at a new origin.

use std::sync::Arc;

use tally_types::{CellAddr, CellRange, ErrorKind, SheetId, MAX_COLS, MAX_ROWS};

/// A function id assigned by the function registry (through
/// [`crate::Resolver::function`]).
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, PartialOrd, Ord)]
pub struct FuncId(pub u32);

/// A named-range id assigned by the workbook (through
/// [`crate::Resolver::name`]).
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, PartialOrd, Ord)]
pub struct NameId(pub u32);

/// One coordinate (row or column) of a reference.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub struct RefCoord {
    /// Absolute 0-based index when `abs`, otherwise an offset from the origin.
    pub v: i32,
    /// `true` for `$`-anchored coordinates.
    pub abs: bool,
}

impl RefCoord {
    /// An absolute coordinate at 0-based `index`.
    pub const fn absolute(index: i32) -> Self {
        Self {
            v: index,
            abs: true,
        }
    }

    /// A relative coordinate `offset` away from the origin.
    pub const fn relative(offset: i32) -> Self {
        Self {
            v: offset,
            abs: false,
        }
    }

    /// Encodes the absolute position `pos` relative to `origin` (or as-is
    /// when `abs`).
    pub fn encode(pos: u32, abs: bool, origin: u32) -> Self {
        let pos = i64::from(pos);
        let v = if abs { pos } else { pos - i64::from(origin) };
        Self {
            v: i32::try_from(v).unwrap_or(i32::MAX),
            abs,
        }
    }

    /// The absolute 0-based position for a formula at `origin`, or `None` if
    /// it falls outside `0..limit`.
    pub fn resolve(self, origin: u32, limit: u32) -> Option<u32> {
        let x = if self.abs {
            i64::from(self.v)
        } else {
            i64::from(origin) + i64::from(self.v)
        };
        if (0..i64::from(limit)).contains(&x) {
            u32::try_from(x).ok()
        } else {
            None
        }
    }
}

/// The sheet part of a reference.
#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub enum SheetRef {
    /// No sheet written: the formula's own sheet. As the end of an
    /// [`RefExpr::Area`] it means "the same sheet as the start".
    Current,
    /// A known sheet. `abs` records a `$Sheet` marker.
    Id {
        /// The sheet.
        id: SheetId,
        /// Written with `$`.
        abs: bool,
    },
    /// A sheet name the resolver did not know at parse time.
    Unresolved(Arc<str>),
    /// The sheet was deleted; the reference evaluates and prints as `#REF!`.
    Deleted,
}

/// One cell reference (an endpoint of an area, or a single cell).
#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub struct CellRefExpr {
    /// The sheet.
    pub sheet: SheetRef,
    /// The row.
    pub row: RefCoord,
    /// The column.
    pub col: RefCoord,
}

/// A reference.
#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub enum RefExpr {
    /// `A1`.
    Cell(CellRefExpr),
    /// `A1:B2`. The parser puts reversed endpoints in order
    /// ([`RefExpr::normalized`]).
    Area(CellRefExpr, CellRefExpr),
    /// Whole columns `A:C`.
    Cols {
        /// The sheet.
        sheet: SheetRef,
        /// First column as written.
        start: RefCoord,
        /// Last column as written.
        end: RefCoord,
    },
    /// Whole rows `1:3`.
    Rows {
        /// The sheet.
        sheet: SheetRef,
        /// First row as written.
        start: RefCoord,
        /// Last row as written.
        end: RefCoord,
    },
    /// A deleted reference, printed `#REF!`.
    Invalid,
}

/// A resolved reference: what the cells actually are for a given origin.
#[derive(Clone, Copy, Eq, PartialEq, Hash, Debug)]
pub enum ResolvedRef {
    /// A rectangle on one sheet (cells, areas, whole columns or rows).
    Range(CellRange),
    /// A 3D area `Sheet1.A1:Sheet3.B2`. `area.sheet == first`; the sheets in
    /// between depend on tab order, which only the engine knows.
    Span3d {
        /// Sheet of the start endpoint.
        first: SheetId,
        /// Sheet of the end endpoint.
        last: SheetId,
        /// The rectangle (on `first`).
        area: CellRange,
    },
    /// Out of the grid, deleted, or on an unknown sheet.
    Invalid,
}

fn resolve_sheet(s: &SheetRef, current: SheetId) -> Option<SheetId> {
    match s {
        SheetRef::Current => Some(current),
        SheetRef::Id { id, .. } => Some(*id),
        SheetRef::Unresolved(_) | SheetRef::Deleted => None,
    }
}

impl CellRefExpr {
    /// The absolute cell for a formula at `origin`, if valid. `default_sheet`
    /// is used for [`SheetRef::Current`].
    pub fn resolve_with(&self, origin: CellAddr, default_sheet: SheetId) -> Option<CellAddr> {
        let sheet = resolve_sheet(&self.sheet, default_sheet)?;
        let row = self.row.resolve(origin.row, MAX_ROWS)?;
        let col = self.col.resolve(origin.col, MAX_COLS)?;
        Some(CellAddr::new(sheet, row, col))
    }

    /// The absolute cell for a formula at `origin`, if valid.
    pub fn resolve(&self, origin: CellAddr) -> Option<CellAddr> {
        self.resolve_with(origin, origin.sheet)
    }
}

/// `(a, b)` in ascending order of their positions for `origin` (when both
/// resolve); each coordinate keeps its own `$`.
fn ordered(a: RefCoord, b: RefCoord, origin: u32, limit: u32) -> (RefCoord, RefCoord) {
    match (a.resolve(origin, limit), b.resolve(origin, limit)) {
        (Some(x), Some(y)) if x > y => (b, a),
        _ => (a, b),
    }
}

impl RefExpr {
    /// The reference with reversed endpoints put in order for a formula at
    /// `origin`, rows and columns independently (`B2:A1` → `A1:B2`,
    /// `$B2:A$1` → `A$1:$B2`), as AOO normalises them.
    pub fn normalized(self, origin: CellAddr) -> RefExpr {
        match self {
            RefExpr::Area(a, b) => {
                let (r0, r1) = ordered(a.row, b.row, origin.row, MAX_ROWS);
                let (c0, c1) = ordered(a.col, b.col, origin.col, MAX_COLS);
                RefExpr::Area(
                    CellRefExpr {
                        sheet: a.sheet,
                        row: r0,
                        col: c0,
                    },
                    CellRefExpr {
                        sheet: b.sheet,
                        row: r1,
                        col: c1,
                    },
                )
            }
            RefExpr::Cols { sheet, start, end } => {
                let (start, end) = ordered(start, end, origin.col, MAX_COLS);
                RefExpr::Cols { sheet, start, end }
            }
            RefExpr::Rows { sheet, start, end } => {
                let (start, end) = ordered(start, end, origin.row, MAX_ROWS);
                RefExpr::Rows { sheet, start, end }
            }
            other => other,
        }
    }

    /// Resolves the reference for a formula at `origin`.
    pub fn resolve(&self, origin: CellAddr) -> ResolvedRef {
        self.try_resolve(origin).unwrap_or(ResolvedRef::Invalid)
    }

    fn try_resolve(&self, origin: CellAddr) -> Option<ResolvedRef> {
        match self {
            RefExpr::Cell(c) => Some(ResolvedRef::Range(CellRange::single(c.resolve(origin)?))),
            RefExpr::Area(a, b) => {
                let a = a.resolve(origin)?;
                let b = b.resolve_with(origin, a.sheet)?;
                let area = CellRange::new(a.sheet, a.row, a.col, b.row, b.col);
                if a.sheet == b.sheet {
                    Some(ResolvedRef::Range(area))
                } else {
                    Some(ResolvedRef::Span3d {
                        first: a.sheet,
                        last: b.sheet,
                        area,
                    })
                }
            }
            RefExpr::Cols { sheet, start, end } => {
                let s = resolve_sheet(sheet, origin.sheet)?;
                let c0 = start.resolve(origin.col, MAX_COLS)?;
                let c1 = end.resolve(origin.col, MAX_COLS)?;
                Some(ResolvedRef::Range(CellRange::new(
                    s,
                    0,
                    c0,
                    MAX_ROWS - 1,
                    c1,
                )))
            }
            RefExpr::Rows { sheet, start, end } => {
                let s = resolve_sheet(sheet, origin.sheet)?;
                let r0 = start.resolve(origin.row, MAX_ROWS)?;
                let r1 = end.resolve(origin.row, MAX_ROWS)?;
                Some(ResolvedRef::Range(CellRange::new(
                    s,
                    r0,
                    0,
                    r1,
                    MAX_COLS - 1,
                )))
            }
            RefExpr::Invalid => None,
        }
    }
}

/// A function reference in a call.
#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub enum FuncRef {
    /// Resolved by the registry.
    Known(FuncId),
    /// Not known to the registry (stored uppercased); evaluates to `#NAME?`.
    Unknown(Arc<str>),
}

/// A named-range reference.
#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub enum NameRef {
    /// Resolved by the workbook.
    Id(NameId),
    /// Not defined at parse time (as written); evaluates to `#NAME?`.
    Unresolved(Arc<str>),
}

/// Prefix operators.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum UnOp {
    /// `-x`
    Neg,
    /// `+x`
    Plus,
}

/// Binary operators.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum BinOp {
    /// `+`
    Add,
    /// `-`
    Sub,
    /// `*`
    Mul,
    /// `/`
    Div,
    /// `^` (left-associative)
    Pow,
    /// `&`
    Concat,
    /// `=`
    Eq,
    /// `<>`
    Ne,
    /// `<`
    Lt,
    /// `<=`
    Le,
    /// `>`
    Gt,
    /// `>=`
    Ge,
    /// `:` between operands that are not two plain cell references.
    Range,
    /// `!` reference intersection.
    Intersect,
    /// `~` reference union.
    Union,
}

impl BinOp {
    /// Binding power; higher binds tighter.
    ///
    /// Comparisons 1, `&` 2, `+ -` 3, `* /` 4, `^` 5, (prefix 6, `%` 7),
    /// `~` 8, `!` 9, `:` 10.
    pub const fn precedence(self) -> u8 {
        match self {
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => 1,
            BinOp::Concat => 2,
            BinOp::Add | BinOp::Sub => 3,
            BinOp::Mul | BinOp::Div => 4,
            BinOp::Pow => 5,
            BinOp::Union => 8,
            BinOp::Intersect => 9,
            BinOp::Range => 10,
        }
    }

    /// The operator as written.
    pub const fn symbol(self) -> &'static str {
        match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Pow => "^",
            BinOp::Concat => "&",
            BinOp::Eq => "=",
            BinOp::Ne => "<>",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
            BinOp::Range => ":",
            BinOp::Intersect => "!",
            BinOp::Union => "~",
        }
    }
}

/// Binding power of prefix `-`/`+` (tighter than `^`: `-2^2` is 4).
pub const PREFIX_PRECEDENCE: u8 = 6;
/// Binding power of postfix `%`.
pub const POSTFIX_PRECEDENCE: u8 = 7;

/// Postfix operators.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum PostfixOp {
    /// `x%` (divide by 100).
    Percent,
}

/// An inline array `{1;2|3;4}` of literal expressions, row-major.
#[derive(Clone, PartialEq, Debug)]
pub struct ArrayLit {
    /// Row count.
    pub rows: u32,
    /// Column count.
    pub cols: u32,
    /// `rows * cols` literals (numbers, strings, booleans, errors).
    pub items: Vec<Expr>,
}

/// An expression.
#[derive(Clone, PartialEq, Debug)]
pub enum Expr {
    /// A number literal.
    Num(f64),
    /// A string literal (unescaped).
    Str(Arc<str>),
    /// `TRUE` / `FALSE`.
    Bool(bool),
    /// An error literal other than `#REF!` (which parses as
    /// [`RefExpr::Invalid`]).
    Err(ErrorKind),
    /// An inline array.
    ArrayLit(ArrayLit),
    /// A reference.
    Ref(RefExpr),
    /// A named range.
    Name(NameRef),
    /// A function call; `args` may contain [`Expr::Missing`].
    Call {
        /// The function.
        func: FuncRef,
        /// Arguments in order.
        args: Vec<Expr>,
    },
    /// A prefix operator.
    Unary(UnOp, Box<Expr>),
    /// A binary operator.
    Binary(BinOp, Box<Expr>, Box<Expr>),
    /// A postfix operator.
    Postfix(PostfixOp, Box<Expr>),
    /// Explicit parentheses (kept so printing round-trips).
    Paren(Box<Expr>),
    /// An omitted argument: `IF(A1;;1)`.
    Missing,
}

impl Expr {
    /// Binding power of this node when printed (atoms bind tightest).
    pub fn precedence(&self) -> u8 {
        match self {
            Expr::Binary(op, _, _) => op.precedence(),
            Expr::Unary(..) => PREFIX_PRECEDENCE,
            Expr::Postfix(..) => POSTFIX_PRECEDENCE,
            _ => u8::MAX,
        }
    }

    /// Visits every expression node in pre-order, left to right. Iterative,
    /// so arbitrarily deep trees are safe.
    pub fn walk<F: FnMut(&Expr)>(&self, mut f: F) {
        let mut stack: Vec<&Expr> = vec![self];
        while let Some(e) = stack.pop() {
            f(e);
            match e {
                Expr::ArrayLit(a) => stack.extend(a.items.iter().rev()),
                Expr::Call { args, .. } => stack.extend(args.iter().rev()),
                Expr::Unary(_, x) | Expr::Postfix(_, x) | Expr::Paren(x) => stack.push(x),
                Expr::Binary(_, l, r) => {
                    stack.push(r);
                    stack.push(l);
                }
                _ => {}
            }
        }
    }

    /// Visits every reference, left to right.
    pub fn walk_refs<F: FnMut(&RefExpr)>(&self, mut f: F) {
        self.walk(|e| {
            if let Expr::Ref(r) = e {
                f(r);
            }
        });
    }

    /// Visits every reference mutably, left to right. Iterative.
    pub fn walk_refs_mut<F: FnMut(&mut RefExpr)>(&mut self, mut f: F) {
        let mut stack: Vec<&mut Expr> = vec![self];
        while let Some(e) = stack.pop() {
            match e {
                Expr::Ref(r) => f(r),
                Expr::ArrayLit(a) => stack.extend(a.items.iter_mut().rev()),
                Expr::Call { args, .. } => stack.extend(args.iter_mut().rev()),
                Expr::Unary(_, x) | Expr::Postfix(_, x) | Expr::Paren(x) => stack.push(x),
                Expr::Binary(_, l, r) => {
                    stack.push(r);
                    stack.push(l);
                }
                _ => {}
            }
        }
    }

    /// The statically known references of this expression, resolved for a
    /// formula at `origin`. Whole columns/rows are clamped to the grid;
    /// anything out of range or on a deleted/unknown sheet is
    /// [`ResolvedRef::Invalid`].
    pub fn static_precedents(&self, origin: CellAddr) -> Vec<ResolvedRef> {
        let mut out = Vec::new();
        self.walk_refs(|r| out.push(r.resolve(origin)));
        out
    }
}

/// Flags the engine attaches to a compiled formula.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Default)]
pub struct FormulaFlags {
    /// Contains a volatile function (`NOW`, `RAND`, ...). The parser leaves
    /// this `false`; the engine sets it from the function registry.
    pub volatile: bool,
}

/// A parsed formula.
#[derive(Clone, PartialEq, Debug)]
pub struct Formula {
    /// The expression (without the leading `=`).
    pub expr: Expr,
    /// Engine flags.
    pub flags: FormulaFlags,
}

impl Formula {
    /// Wraps an expression with default flags.
    pub fn new(expr: Expr) -> Self {
        Self {
            expr,
            flags: FormulaFlags::default(),
        }
    }

    /// See [`Expr::static_precedents`].
    pub fn static_precedents(&self, origin: CellAddr) -> Vec<ResolvedRef> {
        self.expr.static_precedents(origin)
    }
}
