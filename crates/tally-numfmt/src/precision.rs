//! "Precision as shown" (Tools ▸ Options ▸ Calculate): the number of decimals
//! a format code displays, so the engine can round values to it before they
//! are used in formulas (AOO `ScDocOptions::IsCalcAsShown`, applied through
//! `SvNumberFormatter::GetFormatDecimals`).

use crate::format_ast::{FormatAst, SectionKind};
use crate::parse_format::parse_format;

/// The decimals a format code shows for a positive number, or `None` when
/// the value is not rounded: `General` (shown with full precision), text,
/// date/time, scientific and fraction formats. Percent formats add two
/// decimals (`0.0%` shows 0.1234 as `12.3%`, which is the value rounded to
/// three decimals); scaling commas (`#,##0,` thousands) subtract three per
/// comma, so the result can be negative (`#,##0,` rounds to thousands).
pub fn display_decimals(code: &str) -> Option<i32> {
    let ast = parse_format(code).ok()?;
    display_decimals_of(&ast)
}

/// [`display_decimals`] over an already parsed format.
pub fn display_decimals_of(ast: &FormatAst) -> Option<i32> {
    // The first number section decides (AOO uses the positive section).
    let section = ast
        .sections
        .iter()
        .find(|s| matches!(s.kind, SectionKind::Number(_)))?;
    let SectionKind::Number(n) = &section.kind else {
        return None;
    };
    if n.exponent.is_some() || n.fraction.is_some() {
        return None;
    }
    if section.tokens.is_empty() {
        return None;
    }
    Some(i32::from(n.frac_digits) + 2 * i32::from(n.percent) - 3 * i32::from(n.scale_commas))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimals_of_common_codes() {
        assert_eq!(display_decimals("General"), None);
        assert_eq!(display_decimals("0"), Some(0));
        assert_eq!(display_decimals("0.00"), Some(2));
        assert_eq!(display_decimals("#,##0.000"), Some(3));
        assert_eq!(display_decimals("0.00;[RED]-0.0000"), Some(2));
        assert_eq!(display_decimals("0%"), Some(2));
        assert_eq!(display_decimals("0.0%"), Some(3));
        assert_eq!(display_decimals("#,##0,"), Some(-3));
        assert_eq!(display_decimals("0.00E+00"), None);
        assert_eq!(display_decimals("# ?/?"), None);
        assert_eq!(display_decimals("YYYY-MM-DD"), None);
        assert_eq!(display_decimals("@"), None);
        assert_eq!(display_decimals("[$$-409]#,##0.00"), Some(2));
        assert_eq!(display_decimals("not a ((( format"), None);
    }
}
