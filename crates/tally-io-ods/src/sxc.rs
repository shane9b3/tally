//! StarOffice 6 / OpenOffice.org 1.x spreadsheets (`.sxc`, wave 5, W5-3).
//!
//! The OOo 1.x XML format is the direct ancestor of ODF: the same package
//! layout (`mimetype`, `META-INF/manifest.xml`, `content.xml`,
//! `styles.xml`, `settings.xml`, `meta.xml`), the same element names under
//! the same prefixes (`office:`, `table:`, `text:`, `style:`, `fo:`,
//! `number:`), with these differences that the reader maps on the fly
//! ([`map_elem`], applied by the XML reader in legacy mode):
//!
//! - cell values are `table:value-type`, `table:value`, `table:date-value`
//!   … instead of `office:*`;
//! - `table:formula` has no `of:`/`oooc:` prefix; the syntax is the native
//!   Calc one (`=SUM(A1:B2)`, `=Sheet2.A1`), rewritten to OpenFormula
//!   references (`oooc:=SUM([.A1:.B2])`);
//! - every style property sits in one `style:properties` element (the
//!   styles parser accepts it in addition to the split ODF elements), with
//!   `style:text-underline="single"` and `style:text-crossout` spellings;
//! - `office:body` holds the tables directly (no `office:spreadsheet`),
//!   so it is mapped to `office:spreadsheet`;
//! - the mimetype is `application/vnd.sun.xml.calc` (templates
//!   `…calc.template`).
//!
//! Everything else (repeated rows/columns, covered cells, data styles with
//! `style:map`, annotations, named ranges, print ranges) goes through the
//! ODS reader unchanged.

use crate::xml::Elem;

/// The OOo 1.x spreadsheet media type.
pub const MIMETYPE: &str = "application/vnd.sun.xml.calc";
/// The OOo 1.x spreadsheet template media type.
pub const MIMETYPE_TEMPLATE: &str = "application/vnd.sun.xml.calc.template";

/// Renames OOo 1.x elements and attributes to their ODF spellings.
pub(crate) fn map_elem(e: &mut Elem) {
    match e.name.as_str() {
        "office:body" => e.name = "office:spreadsheet".to_string(),
        "table:table-cell" | "table:covered-table-cell" => {
            for (k, v) in e.attrs.iter_mut() {
                match k.as_str() {
                    "table:value-type" => *k = "office:value-type".to_string(),
                    "table:value" => *k = "office:value".to_string(),
                    "table:date-value" => *k = "office:date-value".to_string(),
                    "table:time-value" => *k = "office:time-value".to_string(),
                    "table:boolean-value" => *k = "office:boolean-value".to_string(),
                    "table:string-value" => *k = "office:string-value".to_string(),
                    "table:currency" => *k = "office:currency".to_string(),
                    "table:formula" => *v = map_formula(v),
                    _ => {}
                }
            }
        }
        "style:properties" => {
            for (k, v) in e.attrs.iter_mut() {
                match k.as_str() {
                    "style:text-underline" => {
                        *k = "style:text-underline-style".to_string();
                        if v == "single" || v == "double" || v == "bold" {
                            *v = "solid".to_string();
                        }
                    }
                    "style:text-crossout" => {
                        *k = "style:text-line-through-style".to_string();
                        if v == "single" || v == "double" || v == "bold" {
                            *v = "solid".to_string();
                        }
                    }
                    _ => {}
                }
            }
        }
        "table:named-expression" | "table:named-range" => {
            for (k, v) in e.attrs.iter_mut() {
                if k == "table:expression" {
                    *v = map_formula(v);
                }
            }
        }
        _ => {}
    }
}

/// An OOo 1.x formula as OpenFormula text: prefixed `oooc:=`, with
/// native references (`A1`, `$B$2:C3`, `Sheet2.A1`, `'My Sheet'.A1`)
/// bracketed (`[.A1]`, `[.$B$2:.C3]`, `[Sheet2.A1]`) when the text holds
/// none yet.
pub(crate) fn map_formula(text: &str) -> String {
    let t = text.trim();
    if t.starts_with("of:") || t.starts_with("oooc:") {
        return t.to_string();
    }
    let body = t.strip_prefix('=').unwrap_or(t);
    if body.contains('[') {
        return format!("oooc:={body}");
    }
    format!("oooc:={}", bracket_references(body))
}

fn is_ref_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '$' || c == '.' || c == '_'
}

/// Wraps reference tokens outside string literals in brackets.
fn bracket_references(body: &str) -> String {
    let chars: Vec<char> = body.chars().collect();
    let mut out = String::with_capacity(body.len() + 8);
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '"' {
            // String literal: copy verbatim (`""` is an escaped quote).
            let start = i;
            i += 1;
            while i < chars.len() {
                if chars[i] == '"' {
                    if chars.get(i + 1) == Some(&'"') {
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                i += 1;
            }
            out.extend(&chars[start..i]);
            continue;
        }
        if c == '\'' {
            // Quoted sheet name: 'My Sheet'.A1 → [My Sheet.A1] is not
            // valid OpenFormula, so keep the quotes: ['My Sheet'.A1].
            let start = i;
            i += 1;
            while i < chars.len() && chars[i] != '\'' {
                i += 1;
            }
            i += 1;
            while i < chars.len() && is_ref_char(chars[i]) {
                i += 1;
            }
            let token: String = chars[start..i.min(chars.len())].iter().collect();
            if token.contains(".") && is_cell_or_range(token.rsplit('.').next().unwrap_or("")) {
                let mut full = token.clone();
                if i < chars.len() && chars[i] == ':' {
                    let mut j = i + 1;
                    while j < chars.len() && is_ref_char(chars[j]) {
                        j += 1;
                    }
                    let second: String = chars[i + 1..j].iter().collect();
                    if is_cell_or_range(&second) {
                        full.push(':');
                        if !second.contains('.') {
                            full.push('.');
                        }
                        full.push_str(&second);
                        i = j;
                    }
                }
                out.push('[');
                out.push_str(&full);
                out.push(']');
            } else {
                out.push_str(&token);
            }
            continue;
        }
        if is_ref_char(c) && !c.is_ascii_digit() || c == '$' {
            let start = i;
            while i < chars.len() && is_ref_char(chars[i]) {
                i += 1;
            }
            let token: String = chars[start..i].iter().collect();
            let next = chars.get(i).copied();
            if next == Some('(') || token.chars().all(|c| c.is_ascii_digit() || c == '.') {
                out.push_str(&token);
                continue;
            }
            // Optional range end.
            let mut end_token: Option<String> = None;
            if next == Some(':') {
                let mut j = i + 1;
                while j < chars.len() && is_ref_char(chars[j]) {
                    j += 1;
                }
                let second: String = chars[i + 1..j].iter().collect();
                if is_cell_or_range(&second) && is_cell_or_range(&token) {
                    end_token = Some(second);
                    i = j;
                }
            }
            if is_cell_or_range(&token) {
                out.push('[');
                if !token.contains('.') {
                    out.push('.');
                }
                out.push_str(&token);
                if let Some(e) = end_token {
                    out.push(':');
                    if !e.contains('.') {
                        out.push('.');
                    }
                    out.push_str(&e);
                }
                out.push(']');
            } else {
                out.push_str(&token);
                if let Some(e) = end_token {
                    out.push(':');
                    out.push_str(&e);
                }
            }
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

/// `A1`, `$B$2`, `Sheet1.A1`, `$Sheet1.$A$1` (the part after the last
/// `.` must be a cell address).
fn is_cell_or_range(token: &str) -> bool {
    let cell = token.rsplit('.').next().unwrap_or(token);
    let cell = cell.strip_prefix('$').unwrap_or(cell);
    let letters: String = cell
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect();
    let rest = &cell[letters.len()..];
    let rest = rest.strip_prefix('$').unwrap_or(rest);
    !letters.is_empty()
        && letters.len() <= 3
        && !rest.is_empty()
        && rest.chars().all(|c| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formulas_are_bracketed() {
        assert_eq!(map_formula("=SUM(A1:A2)"), "oooc:=SUM([.A1:.A2])");
        assert_eq!(map_formula("=A1+$B$2*2"), "oooc:=[.A1]+[.$B$2]*2");
        assert_eq!(map_formula("=Sheet2.A1"), "oooc:=[Sheet2.A1]");
        assert_eq!(map_formula("='My Sheet'.A1+1"), "oooc:=['My Sheet'.A1]+1");
        assert_eq!(map_formula("=LOG10(100)"), "oooc:=LOG10(100)");
        assert_eq!(map_formula("=\"A1\"&A1"), "oooc:=\"A1\"&[.A1]");
        assert_eq!(map_formula("=SUM([.A1:.A2])"), "oooc:=SUM([.A1:.A2])");
        assert_eq!(map_formula("of:=[.A1]"), "of:=[.A1]");
        assert_eq!(map_formula("=1.5+2"), "oooc:=1.5+2");
    }

    #[test]
    fn cell_attributes_are_mapped() {
        let mut e = Elem {
            name: "table:table-cell".into(),
            attrs: vec![
                ("table:value-type".into(), "float".into()),
                ("table:value".into(), "3".into()),
                ("table:formula".into(), "=A1+A2".into()),
            ],
        };
        map_elem(&mut e);
        assert_eq!(e.attr("office:value-type"), Some("float"));
        assert_eq!(e.attr("office:value"), Some("3"));
        assert_eq!(e.attr("table:formula"), Some("oooc:=[.A1]+[.A2]"));
        let mut b = Elem {
            name: "office:body".into(),
            attrs: vec![],
        };
        map_elem(&mut b);
        assert_eq!(b.name, "office:spreadsheet");
    }
}
