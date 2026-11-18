//! The rule for what may be a defined name: one place for the engine
//! (Define Names, names read from files) and for readers that have to
//! write a name into formula text.

use tally_types::CellAddr;

use crate::{parse, Dialect, Expr, Resolver};

/// Why a text cannot be a defined name.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum NameProblem {
    /// The name is empty.
    Empty,
    /// It does not start with a letter or `_`.
    Start,
    /// It holds something other than letters, digits, `_` and `.`.
    Chars,
    /// It is longer than 255 bytes.
    TooLong,
    /// It is `TRUE` or `FALSE`.
    Reserved,
    /// It looks like an R1C1 reference (`R1C1`, `R`, `C`, `RC`, `R2`…).
    R1C1,
    /// The formula parser does not read it back as a name: a cell
    /// reference such as `AB12`, a sheet-qualified reference…
    NotAName,
}

/// Checks a defined name: `[A-Za-z_][A-Za-z0-9_.]*` (letters and digits of
/// any script), at most 255 bytes, not `TRUE` / `FALSE`, not R1C1-like and
/// not something a formula at `origin` reads as anything but a name (a
/// cell reference of the grid, a reference into one of the `resolver`'s
/// sheets). `None` for a good name.
pub fn defined_name_problem(
    name: &str,
    origin: CellAddr,
    resolver: &dyn Resolver,
) -> Option<NameProblem> {
    let mut chars = name.chars();
    match chars.next() {
        // Letters of any script, as the formula parser reads them (AOO
        // `SC_COMPILER_C_CHAR_NAME`; correctness review 4, M18).
        Some(c) if c.is_alphabetic() || c == '_' => {}
        Some(_) => return Some(NameProblem::Start),
        None => return Some(NameProblem::Empty),
    }
    if !chars.all(|c| c.is_alphanumeric() || c == '_' || c == '.') {
        return Some(NameProblem::Chars);
    }
    if name.len() > 255 {
        return Some(NameProblem::TooLong);
    }
    if name.eq_ignore_ascii_case("TRUE") || name.eq_ignore_ascii_case("FALSE") {
        return Some(NameProblem::Reserved);
    }
    let upper = name.to_ascii_uppercase();
    let r1c1 = upper.strip_prefix('R').is_some_and(|rest| {
        let rest = rest.trim_start_matches(|c: char| c.is_ascii_digit());
        rest.is_empty()
            || rest
                .strip_prefix('C')
                .is_some_and(|x| x.chars().all(|c| c.is_ascii_digit()))
    }) || upper
        .strip_prefix('C')
        .is_some_and(|x| x.chars().all(|c| c.is_ascii_digit()));
    if r1c1 {
        return Some(NameProblem::R1C1);
    }
    match parse(
        &format!("={name}"),
        origin,
        Dialect::calc_a1_en_us(),
        resolver,
    ) {
        Ok(f) if matches!(f.expr, Expr::Name(_)) => None,
        _ => Some(NameProblem::NotAName),
    }
}

/// Columns of the Excel 97-2003 (BIFF8) sheet, the narrowest grid Tally
/// reads names from.
const NARROW_COLS: u32 = 256;
/// Rows of the Excel 97-2003 sheet.
const NARROW_ROWS: u32 = 65_536;

/// `true` if `name` is spelled like a cell that is outside the Excel
/// 97-2003 sheet (`A1`…`IV65536`) but inside Tally's (`A1`…`XFD1048576`):
/// letters and digits only, a column past `IV` or a row past 65,536, such
/// as `TAX2024` or `A70000`. Such a text is a legal defined name in a
/// program with the narrower sheet (OpenOffice 4.1's ends at `AMJ1048576`,
/// so every cell-like name it allows is of this kind too) and reads as a
/// cell reference in Tally: a file may define it, Tally keeps it under
/// another spelling.
pub fn is_narrow_grid_name(name: &str) -> bool {
    if name.len() > 255 {
        return false;
    }
    let letters = name.len()
        - name
            .trim_start_matches(|c: char| c.is_ascii_alphabetic())
            .len();
    let (letters, digits) = name.split_at(letters);
    if letters.is_empty() || digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    let Some(col) = tally_types::letters_to_col(letters) else {
        return false;
    };
    // `A0` and `A01` are not cells; a row past the sheet is a name anyway.
    let row = match digits.parse::<u32>() {
        Ok(r) if (1..=tally_types::MAX_ROWS).contains(&r) && !digits.starts_with('0') => r,
        _ => return false,
    };
    col >= NARROW_COLS || row > NARROW_ROWS
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SimpleResolver;

    #[test]
    fn names_and_what_is_not_one() {
        let r = SimpleResolver::default();
        let check = |n: &str| defined_name_problem(n, CellAddr::new(1, 0, 0), &r);
        for good in ["Rate", "_x", "Größe", "Tax.2024", "Rate_1", "XFE1"] {
            assert_eq!(check(good), None, "{good}");
        }
        assert_eq!(check(""), Some(NameProblem::Empty));
        assert_eq!(check("1st"), Some(NameProblem::Start));
        assert_eq!(check("a b"), Some(NameProblem::Chars));
        assert_eq!(check(&"a".repeat(256)), Some(NameProblem::TooLong));
        assert_eq!(check("true"), Some(NameProblem::Reserved));
        assert_eq!(check("FALSE"), Some(NameProblem::Reserved));
        for r1c1 in ["R1C1", "RC", "r2", "C", "c12"] {
            assert_eq!(check(r1c1), Some(NameProblem::R1C1), "{r1c1}");
        }
        for cell in ["A1", "N1", "ab12", "XFD1048576", "TAX2024"] {
            assert_eq!(check(cell), Some(NameProblem::NotAName), "{cell}");
        }
    }

    #[test]
    fn narrow_grid_names() {
        // Outside A1…IV65536, inside A1…XFD1048576.
        for name in [
            "TAX2024",
            "tax2024",
            "IW1",
            "A65537",
            "A70000",
            "XFD1048576",
            "AMK5",
        ] {
            assert!(is_narrow_grid_name(name), "{name}");
        }
        // Cells of the narrow sheet; not cells at all; names everywhere.
        for name in [
            "A1", "IV65536", "FY2025", "N1", "XFE1", "A1048577", "A0", "A01", "RATE2025", "Tax",
            "T_1", "2024", "", "A1.B",
        ] {
            assert!(!is_narrow_grid_name(name), "{name}");
        }
    }
}
