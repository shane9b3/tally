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
/// three decimals). Scaling commas (`#,##0,` thousands) do not change the
/// precision: AOO rounds to the format's decimal places only
/// (`nCntPost`, svl zformat.hxx:262; sc documen4.cxx:514; correctness
/// review 4, M30). Scientific formats depend on the value: see
/// [`display_decimals_for`].
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
    Some(i32::from(n.frac_digits) + 2 * i32::from(n.percent))
}

/// The decimals `value` is rounded to under "precision as shown" with
/// format `ast`: [`display_decimals_of`], and for scientific formats the
/// mantissa decimals minus the value's decimal exponent (`0.00E+00` shows
/// 0.000123456 as `1.23E-04`, i.e. 6 decimals; AOO
/// `ScDocument::RoundValueAsShown`, sc/source/core/data/documen4.cxx:
/// 519-526; correctness review 4, M29).
pub fn display_decimals_for(ast: &FormatAst, value: f64) -> Option<i32> {
    let section = ast
        .sections
        .iter()
        .find(|s| matches!(s.kind, SectionKind::Number(_)))?;
    let SectionKind::Number(n) = &section.kind else {
        return None;
    };
    if n.exponent.is_some() && n.fraction.is_none() && !section.tokens.is_empty() {
        if value == 0.0 || !value.is_finite() {
            return None;
        }
        let e = value.abs().log10().floor() as i32;
        return Some(i32::from(n.frac_digits) - e);
    }
    display_decimals_of(ast)
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
        assert_eq!(display_decimals("#,##0,"), Some(0));
        assert_eq!(display_decimals("0.00E+00"), None);
        assert_eq!(display_decimals("# ?/?"), None);
        assert_eq!(display_decimals("YYYY-MM-DD"), None);
        assert_eq!(display_decimals("@"), None);
        assert_eq!(display_decimals("[$$-409]#,##0.00"), Some(2));
        assert_eq!(display_decimals("not a ((( format"), None);
    }

    #[test]
    fn scientific_decimals_depend_on_the_value() {
        let sci = parse_format("0.00E+00").unwrap();
        assert_eq!(display_decimals_for(&sci, 0.000123456), Some(6));
        assert_eq!(display_decimals_for(&sci, -12345.0), Some(-2));
        assert_eq!(display_decimals_for(&sci, 0.0), None);
        let plain = parse_format("0.0").unwrap();
        assert_eq!(display_decimals_for(&plain, 1234.5678), Some(1));
    }
}
