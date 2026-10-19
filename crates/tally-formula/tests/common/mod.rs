#![allow(dead_code)]

pub mod cases;

use tally_formula::{parse, print, Dialect, Formula, SimpleResolver};
use tally_types::CellAddr;

/// Sheets: 0 Sheet1, 1 Sheet2, 2 "My Sheet", 3 "It's", 4 Data.
/// Names: MyRange (workbook), Tax (sheet 0 scope).
pub fn resolver() -> SimpleResolver {
    let mut r = SimpleResolver::with_default_functions();
    r.add_sheet(1, "Sheet2");
    r.add_sheet(2, "My Sheet");
    r.add_sheet(3, "It's");
    r.add_sheet(4, "Data");
    r.add_name("MyRange", None);
    r.add_name("Tax", Some(0));
    r
}

pub fn a1(s: &str) -> CellAddr {
    CellAddr::from_a1(0, s).unwrap()
}

pub fn calc() -> Dialect {
    Dialect::calc_a1_en_us()
}

pub fn p(src: &str) -> Formula {
    parse(src, a1("A1"), calc(), &resolver()).unwrap_or_else(|e| panic!("{src}: {e}"))
}

pub fn p_at(src: &str, at: &str) -> Formula {
    parse(src, a1(at), calc(), &resolver()).unwrap_or_else(|e| panic!("{src}: {e}"))
}

pub fn pr(f: &Formula, at: &str) -> String {
    print(f, a1(at), calc(), &resolver())
}

pub fn pr_of(f: &Formula, at: &str) -> String {
    print(f, a1(at), Dialect::open_formula(), &resolver())
}

pub fn code(src: &str) -> u16 {
    match parse(src, a1("A1"), calc(), &resolver()) {
        Ok(f) => panic!("{src} parsed: {f:?}"),
        Err(e) => e.kind.code(),
    }
}

use tally_formula::{
    BinOp, CellRefExpr, Expr, FuncRef, NameRef, RefCoord, RefExpr, SheetRef, UnOp,
};

fn coord(axis: char, c: RefCoord) -> String {
    if c.abs {
        format!("{axis}{}", i64::from(c.v) + 1)
    } else {
        format!("{axis}[{}]", c.v)
    }
}

fn sheet(s: &SheetRef) -> String {
    match s {
        SheetRef::Current => String::new(),
        SheetRef::Id { id, abs } => format!("{}#{id}!", if *abs { "$" } else { "" }),
        SheetRef::Unresolved(n) => format!("?{n}!"),
        SheetRef::Deleted => "#DELETED!".to_string(),
    }
}

fn cell(c: &CellRefExpr) -> String {
    format!(
        "{}{}{}",
        sheet(&c.sheet),
        coord('R', c.row),
        coord('C', c.col)
    )
}

/// Compact S-expression dump of an AST. Relative coordinates are shown
/// R1C1-style: `R[-1]C[0]` relative, `R1C1` absolute (1-based).
pub fn sx(e: &Expr) -> String {
    match e {
        Expr::Num(n) => format!("{n:?}"),
        Expr::Str(s) => format!("{:?}", &**s),
        Expr::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
        Expr::Err(k) => k.display().into_owned(),
        Expr::Missing => "_".to_string(),
        Expr::ArrayLit(a) => {
            let items: Vec<String> = a.items.iter().map(sx).collect();
            format!("{{{}x{} {}}}", a.rows, a.cols, items.join(" "))
        }
        Expr::Ref(r) => match r {
            RefExpr::Cell(c) => format!("(cell {})", cell(c)),
            RefExpr::Area(a, b) => format!("(area {} {})", cell(a), cell(b)),
            RefExpr::Cols {
                sheet: s,
                start,
                end,
            } => {
                format!(
                    "(cols {}{} {})",
                    sheet(s),
                    coord('C', *start),
                    coord('C', *end)
                )
            }
            RefExpr::Rows {
                sheet: s,
                start,
                end,
            } => {
                format!(
                    "(rows {}{} {})",
                    sheet(s),
                    coord('R', *start),
                    coord('R', *end)
                )
            }
            RefExpr::Invalid => "(ref #REF!)".to_string(),
        },
        Expr::Name(NameRef::Id(id)) => format!("(name #{})", id.0),
        Expr::Name(NameRef::Unresolved(n)) => format!("(name? {n})"),
        Expr::Call { func, args } => {
            let f = match func {
                FuncRef::Known(id) => format!("#{}", id.0),
                FuncRef::Unknown(n) => format!("{n}?"),
            };
            let mut s = format!("(call {f}");
            for a in args {
                s.push(' ');
                s.push_str(&sx(a));
            }
            s.push(')');
            s
        }
        Expr::Unary(op, x) => format!(
            "({} {})",
            match op {
                UnOp::Neg => "neg",
                UnOp::Plus => "pos",
            },
            sx(x)
        ),
        Expr::Binary(op, l, r) => {
            let name = match op {
                BinOp::Range => "range",
                BinOp::Intersect => "isect",
                BinOp::Union => "union",
                other => other.symbol(),
            };
            format!("({name} {} {})", sx(l), sx(r))
        }
        Expr::Postfix(_, x) => format!("(% {})", sx(x)),
        Expr::Paren(x) => format!("(paren {})", sx(x)),
    }
}

/// A copy of `e` without `Paren` nodes (test helper; recursion is fine for
/// parser-bounded trees).
pub fn strip_parens(e: &Expr) -> Expr {
    match e {
        Expr::Paren(x) => strip_parens(x),
        Expr::Unary(op, x) => Expr::Unary(*op, Box::new(strip_parens(x))),
        Expr::Postfix(op, x) => Expr::Postfix(*op, Box::new(strip_parens(x))),
        Expr::Binary(op, l, r) => {
            Expr::Binary(*op, Box::new(strip_parens(l)), Box::new(strip_parens(r)))
        }
        Expr::Call { func, args } => Expr::Call {
            func: func.clone(),
            args: args.iter().map(strip_parens).collect(),
        },
        other => other.clone(),
    }
}
