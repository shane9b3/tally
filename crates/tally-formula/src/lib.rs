//! Formula language for Tally (WP2).
//!
//! Lexer ([`tokenize`]), parser ([`parse`]), the relative-reference AST
//! ([`ast`]), dialects ([`Dialect`]), the printer ([`print()`]), reference
//! adjustment for structural changes ([`adjust_for_structure_change`]),
//! [`explain()`] for plain-English hints and the display-only [`pretty`]
//! prettifier. See `docs/architecture-plan.md` §3.5 and §7 WP2.
//!
//! Shared lexer fixtures live in `tests/fixtures/tokens/*.json`.
//!
//! ```
//! use tally_formula::{parse, print, Dialect, SimpleResolver};
//! use tally_types::CellAddr;
//!
//! let r = SimpleResolver::with_default_functions();
//! let c1 = CellAddr::new(0, 0, 2);
//! let f = parse("=A1+$B$1", c1, Dialect::calc_a1_en_us(), &r).unwrap();
//! // Copying is printing at a new origin: C1 -> D5.
//! let d5 = CellAddr::new(0, 4, 3);
//! assert_eq!(print(&f, d5, Dialect::calc_a1_en_us(), &r), "=B5+$B$1");
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod adjust;
pub mod ast;
pub mod dialect;
pub mod explain;
pub mod lexer;
pub mod parser;
pub mod printer;
mod refsyntax;
mod resolver;

use std::sync::Arc;

use tally_types::SheetId;

pub use adjust::{
    adjust_for_structure_change, move_refs, reanchor, relocate, shifted_origin, BlockMove,
    StructureChange,
};
pub use ast::{
    ArrayLit, BinOp, CellRefExpr, Expr, Formula, FormulaFlags, FuncId, FuncRef, NameId, NameRef,
    PostfixOp, RefCoord, RefExpr, ResolvedRef, SheetRef, UnOp,
};
pub use dialect::{ArgSep, Dialect, SheetSep};
pub use explain::{explain, LabelSource, NoLabels};
pub use lexer::{Token, TokenKind};
pub use parser::{parse, ParseError, MAX_ARGS, MAX_AST_DEPTH, MAX_FORMULA_LEN, MAX_NESTING};
pub use printer::{pretty, print, print_expr};
pub use resolver::{SimpleResolver, DEFAULT_FUNCTIONS};

/// Tokenizes `src` for syntax highlighting. Never fails; unknown characters
/// become [`TokenKind::Invalid`]. A leading `=`, `of:=` or `oooc:=` becomes a
/// [`TokenKind::Prefix`] token (the latter two switch to OpenFormula syntax).
/// Concatenating the token spans reproduces `src` exactly.
pub fn tokenize(src: &str, dialect: Dialect) -> Vec<Token> {
    lexer::lex(src, dialect).0
}

/// Resolves names during parsing and printing.
///
/// The engine implements this over the workbook and the function registry,
/// so this crate does not depend on either. Lookups should be ASCII
/// case-insensitive; the parser passes function names uppercased.
pub trait Resolver {
    /// The sheet with this name, if any.
    fn sheet_by_name(&self, name: &str) -> Option<SheetId>;
    /// The current name of a sheet (`None` if it no longer exists; such
    /// references print as `#REF!`).
    fn sheet_name(&self, id: SheetId) -> Option<Arc<str>>;
    /// A named range visible from `sheet` (sheet scope first, then workbook).
    fn name(&self, name: &str, sheet: SheetId) -> Option<NameId>;
    /// The registry id of a function (name given uppercased).
    fn function(&self, name: &str) -> Option<FuncId>;
    /// The display name of a named range, for printing. Without it a
    /// [`NameRef::Id`] prints as `#NAME?`.
    fn name_text(&self, id: NameId) -> Option<Arc<str>> {
        let _ = id;
        None
    }
    /// The canonical (uppercase) name of a function, for printing. Without
    /// it a [`FuncRef::Known`] prints as `#NAME?`.
    fn function_name(&self, id: FuncId) -> Option<Arc<str>> {
        let _ = id;
        None
    }
    /// The function name to print in `dialect` (e.g. an ODF
    /// `COM.MICROSOFT.IFERROR` spelling). Defaults to [`Resolver::function_name`].
    fn function_name_in(&self, id: FuncId, dialect: Dialect) -> Option<Arc<str>> {
        let _ = dialect;
        self.function_name(id)
    }
}
