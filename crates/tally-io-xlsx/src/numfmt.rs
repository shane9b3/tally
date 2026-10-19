//! Number formats: the built-in `numFmtId` table and the Excel ↔ OOo code
//! translation, on top of `tally_numfmt::excel`.
//!
//! Extensions over `tally_numfmt::excel::{from_excel_code, to_excel_code}`
//! (candidates to move into tally-numfmt):
//!
//! - Excel locale-only tags `[$-409]`, `[$-x-sysdate]`, `[$-F800]` (system
//!   long date) and `[$-F400]` (system time) are mapped to OOo codes or
//!   dropped before translation;
//! - codes the OOo grammar rejects fall back to `None` (General) instead of
//!   failing the import.

use tally_numfmt::{excel, parse_format, SectionKind};

/// The Excel code of a built-in `numFmtId` (en-US), as Excel 365 shows it.
/// Ids 27–36 and 50–58 are East-Asian locale dates; they map to an ISO date.
pub fn builtin_format_code(id: u32) -> Option<&'static str> {
    Some(match id {
        0 => "General",
        1 => "0",
        2 => "0.00",
        3 => "#,##0",
        4 => "#,##0.00",
        5 => "\"$\"#,##0_);\\(\"$\"#,##0\\)",
        6 => "\"$\"#,##0_);[Red]\\(\"$\"#,##0\\)",
        7 => "\"$\"#,##0.00_);\\(\"$\"#,##0.00\\)",
        8 => "\"$\"#,##0.00_);[Red]\\(\"$\"#,##0.00\\)",
        9 => "0%",
        10 => "0.00%",
        11 => "0.00E+00",
        12 => "# ?/?",
        13 => "# ??/??",
        14 => "m/d/yyyy",
        15 => "d-mmm-yy",
        16 => "d-mmm",
        17 => "mmm-yy",
        18 => "h:mm AM/PM",
        19 => "h:mm:ss AM/PM",
        20 => "h:mm",
        21 => "h:mm:ss",
        22 => "m/d/yyyy h:mm",
        27..=36 | 50..=58 => "yyyy-mm-dd",
        37 => "#,##0_);\\(#,##0\\)",
        38 => "#,##0_);[Red]\\(#,##0\\)",
        39 => "#,##0.00_);\\(#,##0.00\\)",
        40 => "#,##0.00_);[Red]\\(#,##0.00\\)",
        41 => "_(* #,##0_);_(* \\(#,##0\\);_(* \"-\"_);_(@_)",
        42 => "_(\"$\"* #,##0_);_(\"$\"* \\(#,##0\\);_(\"$\"* \"-\"_);_(@_)",
        43 => "_(* #,##0.00_);_(* \\(#,##0.00\\);_(* \"-\"??_);_(@_)",
        44 => "_(\"$\"* #,##0.00_);_(\"$\"* \\(#,##0.00\\);_(\"$\"* \"-\"??_);_(@_)",
        45 => "mm:ss",
        46 => "[h]:mm:ss",
        47 => "mm:ss.0",
        48 => "##0.0E+0",
        49 => "@",
        _ => return None,
    })
}

/// Rewrites Excel-only bracket tags the OOo grammar does not know.
fn preprocess_excel(code: &str) -> String {
    let lower = code.to_ascii_lowercase();
    if lower.starts_with("[$-f800]") {
        return "dddd, mmmm d, yyyy".into();
    }
    if lower.starts_with("[$-f400]") {
        return "h:mm:ss AM/PM".into();
    }
    let mut out = String::with_capacity(code.len());
    let mut rest = code;
    let mut in_quote = false;
    while let Some(c) = rest.chars().next() {
        if c == '"' {
            in_quote = !in_quote;
        }
        if !in_quote && c == '\\' {
            // Copy the escape and the escaped character verbatim.
            let mut it = rest.char_indices();
            it.next();
            let end = it.next().map_or(rest.len(), |(i, ch)| i + ch.len_utf8());
            out.push_str(&rest[..end]);
            rest = &rest[end..];
            continue;
        }
        if !in_quote && rest.starts_with("[$-") {
            // `[$-409]`, `[$-x-sysdate]`: a locale without a currency symbol.
            if let Some(close) = rest.find(']') {
                rest = &rest[close + 1..];
                continue;
            }
        }
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    out
}

/// Excel format code → OOo code, or `None` for General / untranslatable.
pub(crate) fn excel_to_ooo(code: &str) -> Option<String> {
    let code = code.trim();
    if code.is_empty() || code.eq_ignore_ascii_case("general") {
        return None;
    }
    let pre = preprocess_excel(code);
    let ooo = excel::from_excel_code(&pre)
        .or_else(|_| excel::from_excel_code(code))
        .ok()?;
    if ooo.eq_ignore_ascii_case("general") || ooo.is_empty() {
        None
    } else {
        Some(ooo)
    }
}

/// OOo code → Excel code (`None` if it has no translation).
pub(crate) fn ooo_to_excel(code: &str) -> Option<String> {
    excel::to_excel_code(code).ok()
}

/// Whether an OOo format code displays a calendar date (day, month, year,
/// weekday…), which makes the cell value a date serial subject to the
/// 1900 leap-year mapping. Pure time and elapsed-time formats are not.
pub fn is_date_format(ooo_code: &str) -> bool {
    match parse_format(ooo_code) {
        Ok(ast) => ast
            .sections
            .first()
            .is_some_and(|s| matches!(s.kind, SectionKind::Date(d) if d.has_date)),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_translate() {
        for id in (0..=22).chain(37..=49) {
            let code = builtin_format_code(id).unwrap_or_else(|| panic!("{id}"));
            if id == 0 {
                assert_eq!(excel_to_ooo(code), None);
                continue;
            }
            assert!(excel_to_ooo(code).is_some(), "{id}: {code}");
        }
        assert_eq!(builtin_format_code(23), None);
        assert_eq!(builtin_format_code(164), None);
    }

    #[test]
    fn custom_codes() {
        assert_eq!(excel_to_ooo("yyyy-mm-dd").as_deref(), Some("YYYY-MM-DD"));
        assert_eq!(
            excel_to_ooo("[$-409]mmm d, yyyy").as_deref(),
            Some("MMM D, YYYY")
        );
        assert_eq!(excel_to_ooo("[h]:mm:ss").as_deref(), Some("[H]:MM:SS"));
        assert_eq!(excel_to_ooo("General"), None);
        assert_eq!(excel_to_ooo("0.00%").as_deref(), Some("0.00%"));
        assert!(excel_to_ooo("[$-F800]").is_some());
        assert!(excel_to_ooo("\\[$-409\\]0").is_some());
    }

    #[test]
    fn date_detection() {
        assert!(is_date_format("YYYY-MM-DD"));
        assert!(is_date_format("M/D/YYYY H:MM"));
        assert!(is_date_format("MMM-YY"));
        assert!(!is_date_format("H:MM:SS"));
        assert!(!is_date_format("[H]:MM:SS"));
        assert!(!is_date_format("0.00"));
        assert!(!is_date_format("General"));
        assert!(!is_date_format("@"));
    }

    #[test]
    fn back_to_excel() {
        assert_eq!(ooo_to_excel("YYYY-MM-DD").as_deref(), Some("yyyy-mm-dd"));
        assert_eq!(ooo_to_excel("#,##0.00").as_deref(), Some("#,##0.00"));
    }
}
