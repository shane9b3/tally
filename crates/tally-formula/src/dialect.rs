//! Formula dialects: which separators and reference syntax a formula uses.
//!
//! Parsing is lenient: in [`Dialect::CalcA1`] both `,` and `;` are accepted as
//! argument separators and both `.` and `!` as sheet separators, whatever the
//! dialect's own choice. Printing always uses the dialect's chosen
//! separators exactly.

/// The argument separator a CalcA1 dialect prints.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum ArgSep {
    /// `,` (Tally's en-US default).
    Comma,
    /// `;` (OpenOffice's default).
    Semicolon,
}

impl ArgSep {
    /// The separator character.
    pub const fn as_char(self) -> char {
        match self {
            ArgSep::Comma => ',',
            ArgSep::Semicolon => ';',
        }
    }
}

/// The sheet separator a CalcA1 dialect prints.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum SheetSep {
    /// `Sheet1.A1` (OpenOffice style).
    Dot,
    /// `Sheet1!A1` (Excel style).
    Bang,
}

impl SheetSep {
    /// The separator character.
    pub const fn as_char(self) -> char {
        match self {
            SheetSep::Dot => '.',
            SheetSep::Bang => '!',
        }
    }
}

/// A formula dialect.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum Dialect {
    /// The UI dialect: `Sheet1.A1`, `$Sheet1.$A$1`, `'My Sheet'.A1`, whole
    /// columns `A:A`, inline arrays `{1;2|3;4}`.
    CalcA1 {
        /// Argument separator used when printing.
        arg_sep: ArgSep,
        /// Sheet separator used when printing.
        sheet_sep: SheetSep,
    },
    /// ODF OpenFormula as stored in `.ods` files: `of:=SUM([.A1:.B2];3)`.
    /// Parsing also accepts the legacy OOo 2.x `oooc:=` prefix.
    OpenFormula,
}

impl Dialect {
    /// CalcA1 with `,` arguments and `.` sheet separators (Tally's default).
    pub const fn calc_a1_en_us() -> Self {
        Dialect::CalcA1 {
            arg_sep: ArgSep::Comma,
            sheet_sep: SheetSep::Dot,
        }
    }

    /// CalcA1 with the given separators.
    pub const fn calc_a1(arg_sep: ArgSep, sheet_sep: SheetSep) -> Self {
        Dialect::CalcA1 { arg_sep, sheet_sep }
    }

    /// The OpenFormula (ODF storage) dialect.
    pub const fn open_formula() -> Self {
        Dialect::OpenFormula
    }

    /// Returns `true` for [`Dialect::OpenFormula`].
    pub const fn is_open_formula(self) -> bool {
        matches!(self, Dialect::OpenFormula)
    }

    /// The argument separator this dialect prints.
    pub const fn arg_sep_char(self) -> char {
        match self {
            Dialect::CalcA1 { arg_sep, .. } => arg_sep.as_char(),
            Dialect::OpenFormula => ';',
        }
    }

    /// The prefix printed before the expression: `=` or `of:=`.
    pub const fn formula_prefix(self) -> &'static str {
        match self {
            Dialect::CalcA1 { .. } => "=",
            Dialect::OpenFormula => "of:=",
        }
    }
}

impl Default for Dialect {
    fn default() -> Self {
        Dialect::calc_a1_en_us()
    }
}
