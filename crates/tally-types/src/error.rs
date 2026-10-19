//! OpenOffice-compatible error values (`Err:5xx` and the `#...!` family).

use std::borrow::Cow;
use std::fmt;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// A spreadsheet error value, using Apache OpenOffice's numeric codes.
///
/// The discriminant *is* the OOo code (see [`ErrorKind::code`]), which is
/// what `ERRORTYPE()` returns and what `Err:NNN` displays.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[repr(u16)]
pub enum ErrorKind {
    /// 501: invalid character in a formula.
    InvalidChar = 501,
    /// 502: invalid function argument (e.g. `SQRT(-1)` in some contexts).
    InvalidArgument = 502,
    /// 503: invalid floating point operation, displayed `#NUM!`.
    Num = 503,
    /// 504: error in the parameter list.
    ParamList = 504,
    /// 507: missing pair (e.g. unmatched bracket).
    Pair = 507,
    /// 508: missing bracket.
    Bracket = 508,
    /// 509: missing operator.
    MissingOperator = 509,
    /// 510: missing variable.
    MissingVariable = 510,
    /// 511: missing variable (parameter expected).
    MissingVariable2 = 511,
    /// 512: formula overflow (too long or nested too deeply).
    FormulaOverflow = 512,
    /// 513: string overflow.
    StringOverflow = 513,
    /// 514: internal overflow.
    InternalOverflow = 514,
    /// 516: internal syntax error (unknown variable).
    UnknownVariable = 516,
    /// 517: internal syntax error (unknown opcode).
    UnknownOpCode = 517,
    /// 518: internal syntax error (unknown stack variable).
    UnknownStackVariable = 518,
    /// 519: no result / wrong data type, displayed `#VALUE!`.
    Value = 519,
    /// 520: internal syntax error (unknown token).
    UnknownToken = 520,
    /// 521: empty intersection, displayed `#NULL!`.
    Null = 521,
    /// 522: circular reference.
    Circular = 522,
    /// 523: iteration does not converge.
    NoConvergence = 523,
    /// 524: invalid reference, displayed `#REF!`.
    Ref = 524,
    /// 525: invalid name, displayed `#NAME?`.
    Name = 525,
    /// 532: division by zero, displayed `#DIV/0!`.
    Div0 = 532,
    /// 32767 (OOo `NOTAVAILABLE`, `0x7fff`): value not available, displayed `#N/A`.
    NA = 32767,
}

impl ErrorKind {
    /// Every variant, in code order.
    pub const ALL: [ErrorKind; 24] = [
        ErrorKind::InvalidChar,
        ErrorKind::InvalidArgument,
        ErrorKind::Num,
        ErrorKind::ParamList,
        ErrorKind::Pair,
        ErrorKind::Bracket,
        ErrorKind::MissingOperator,
        ErrorKind::MissingVariable,
        ErrorKind::MissingVariable2,
        ErrorKind::FormulaOverflow,
        ErrorKind::StringOverflow,
        ErrorKind::InternalOverflow,
        ErrorKind::UnknownVariable,
        ErrorKind::UnknownOpCode,
        ErrorKind::UnknownStackVariable,
        ErrorKind::Value,
        ErrorKind::UnknownToken,
        ErrorKind::Null,
        ErrorKind::Circular,
        ErrorKind::NoConvergence,
        ErrorKind::Ref,
        ErrorKind::Name,
        ErrorKind::Div0,
        ErrorKind::NA,
    ];

    /// The OpenOffice error code (what `ERRORTYPE()` returns).
    pub const fn code(self) -> u16 {
        self as u16
    }

    /// Looks up a variant by its OpenOffice code.
    pub fn from_code(code: u16) -> Option<Self> {
        Self::ALL.iter().copied().find(|e| e.code() == code)
    }

    /// The display string: `#NUM!`, `#VALUE!`, `#NULL!`, `#REF!`, `#NAME?`,
    /// `#DIV/0!`, `#N/A`, or `Err:NNN` for every other code.
    pub fn display(self) -> Cow<'static, str> {
        match self {
            ErrorKind::Num => Cow::Borrowed("#NUM!"),
            ErrorKind::Value => Cow::Borrowed("#VALUE!"),
            ErrorKind::Null => Cow::Borrowed("#NULL!"),
            ErrorKind::Ref => Cow::Borrowed("#REF!"),
            ErrorKind::Name => Cow::Borrowed("#NAME?"),
            ErrorKind::Div0 => Cow::Borrowed("#DIV/0!"),
            ErrorKind::NA => Cow::Borrowed("#N/A"),
            other => Cow::Owned(format!("Err:{}", other.code())),
        }
    }

    /// Parses a display string back into an error: the `#...` names
    /// (ASCII case-insensitive) or `Err:NNN` for a known code.
    pub fn parse(s: &str) -> Option<Self> {
        if let Some(code) = s.strip_prefix("Err:") {
            if code.is_empty() || !code.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            return code.parse().ok().and_then(Self::from_code);
        }
        const NAMED: [(&str, ErrorKind); 7] = [
            ("#NUM!", ErrorKind::Num),
            ("#VALUE!", ErrorKind::Value),
            ("#NULL!", ErrorKind::Null),
            ("#REF!", ErrorKind::Ref),
            ("#NAME?", ErrorKind::Name),
            ("#DIV/0!", ErrorKind::Div0),
            ("#N/A", ErrorKind::NA),
        ];
        NAMED
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(s))
            .map(|&(_, kind)| kind)
    }

    /// The Excel `ERROR.TYPE` number: 1 `#NULL!`, 2 `#DIV/0!`, 3 `#VALUE!`,
    /// 4 `#REF!`, 5 `#NAME?`, 6 `#NUM!`, 7 `#N/A`.
    ///
    /// Returns `None` for OpenOffice-only codes, which have no Excel number.
    pub const fn excel_error_type(self) -> Option<u8> {
        match self {
            ErrorKind::Null => Some(1),
            ErrorKind::Div0 => Some(2),
            ErrorKind::Value => Some(3),
            ErrorKind::Ref => Some(4),
            ErrorKind::Name => Some(5),
            ErrorKind::Num => Some(6),
            ErrorKind::NA => Some(7),
            _ => None,
        }
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.display())
    }
}

impl std::error::Error for ErrorKind {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_match_openoffice() {
        let expected: [u16; 24] = [
            501, 502, 503, 504, 507, 508, 509, 510, 511, 512, 513, 514, 516, 517, 518, 519, 520,
            521, 522, 523, 524, 525, 532, 32767,
        ];
        let actual: Vec<u16> = ErrorKind::ALL.iter().map(|e| e.code()).collect();
        assert_eq!(actual, expected);
        assert_eq!(ErrorKind::NA.code(), 0x7fff);
    }

    #[test]
    fn display_names() {
        assert_eq!(ErrorKind::Num.display(), "#NUM!");
        assert_eq!(ErrorKind::Value.display(), "#VALUE!");
        assert_eq!(ErrorKind::Null.display(), "#NULL!");
        assert_eq!(ErrorKind::Ref.display(), "#REF!");
        assert_eq!(ErrorKind::Name.display(), "#NAME?");
        assert_eq!(ErrorKind::Div0.display(), "#DIV/0!");
        assert_eq!(ErrorKind::NA.display(), "#N/A");
        assert_eq!(ErrorKind::InvalidArgument.display(), "Err:502");
        assert_eq!(ErrorKind::Circular.display(), "Err:522");
        assert_eq!(ErrorKind::Circular.to_string(), "Err:522");
    }

    #[test]
    fn display_parse_round_trip_every_variant() {
        for kind in ErrorKind::ALL {
            assert_eq!(ErrorKind::parse(&kind.display()), Some(kind), "{kind:?}");
            assert_eq!(ErrorKind::from_code(kind.code()), Some(kind));
            // `Err:NNN` is accepted for every code, including named ones.
            assert_eq!(
                ErrorKind::parse(&format!("Err:{}", kind.code())),
                Some(kind)
            );
        }
    }

    #[test]
    fn parse_rejects_unknown() {
        for s in [
            "",
            "#NUM",
            "#FOO!",
            "Err:",
            "Err:505",
            "Err:+502",
            "Err:99999999",
            "err:502",
        ] {
            assert_eq!(ErrorKind::parse(s), None, "{s:?}");
        }
        assert_eq!(ErrorKind::parse("#n/a"), Some(ErrorKind::NA));
        assert_eq!(ErrorKind::from_code(0), None);
    }

    #[test]
    fn excel_error_type_mapping() {
        assert_eq!(ErrorKind::Null.excel_error_type(), Some(1));
        assert_eq!(ErrorKind::Div0.excel_error_type(), Some(2));
        assert_eq!(ErrorKind::Value.excel_error_type(), Some(3));
        assert_eq!(ErrorKind::Ref.excel_error_type(), Some(4));
        assert_eq!(ErrorKind::Name.excel_error_type(), Some(5));
        assert_eq!(ErrorKind::Num.excel_error_type(), Some(6));
        assert_eq!(ErrorKind::NA.excel_error_type(), Some(7));
        let named = ErrorKind::ALL
            .iter()
            .filter(|e| e.excel_error_type().is_some())
            .count();
        assert_eq!(named, 7);
        assert_eq!(ErrorKind::Circular.excel_error_type(), None);
    }
}
