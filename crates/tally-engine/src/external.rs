//! External links and DDE are disabled by policy (docs/security.md; wave 5,
//! W5-3): a `DDE()` formula gives `#N/A`, a reference into another file
//! (`'file:///…'#$Sheet.A1`, Excel's `[1]Sheet!A1`) gives `#REF!`, a sheet
//! linked to another file (`table:table-source`) is imported as values, and
//! a linked (not embedded) picture is dropped. Each case is counted in
//! [`Workbook::external_links`] and shown once as a warning.
//!
//! Readers report what they disabled as a tagged entry in
//! `WorkbookData::warnings` (`tally:external-link:<kind>[:<detail>]`), so
//! the model needs no new type; [`collect_import_warnings`] turns the tags
//! into [`ExternalLinkWarning`]s and readable messages when the workbook
//! is built. Formulas are classified by their text when they are loaded.

use tally_api::{ExternalLinkKind, ExternalLinkWarning};
use tally_types::ErrorKind;

use crate::workbook::Workbook;

/// Prefix of a reader's tagged warning.
pub const WARNING_PREFIX: &str = "tally:external-link:";

/// Longest detail kept (a file name or DDE topic).
const MAX_DETAIL: usize = 200;

fn kind_tag(kind: ExternalLinkKind) -> &'static str {
    match kind {
        ExternalLinkKind::Dde => "dde",
        ExternalLinkKind::ExternalReference => "externalReference",
        ExternalLinkKind::LinkedSheet => "linkedSheet",
        ExternalLinkKind::LinkedImage => "linkedImage",
    }
}

fn kind_of_tag(tag: &str) -> Option<ExternalLinkKind> {
    Some(match tag {
        "dde" => ExternalLinkKind::Dde,
        "externalReference" => ExternalLinkKind::ExternalReference,
        "linkedSheet" => ExternalLinkKind::LinkedSheet,
        "linkedImage" => ExternalLinkKind::LinkedImage,
        _ => return None,
    })
}

/// A reader's tagged warning for one disabled link.
pub fn tagged_warning(kind: ExternalLinkKind, detail: Option<&str>) -> String {
    match detail {
        Some(d) => format!("{WARNING_PREFIX}{}:{}", kind_tag(kind), clip(d)),
        None => format!("{WARNING_PREFIX}{}", kind_tag(kind)),
    }
}

fn clip(s: &str) -> String {
    let s = s.trim();
    if s.chars().count() <= MAX_DETAIL {
        return s.to_string();
    }
    let mut out: String = s.chars().take(MAX_DETAIL).collect();
    out.push('…');
    out
}

/// The error value a disabled formula shows.
pub fn policy_error(kind: ExternalLinkKind) -> ErrorKind {
    match kind {
        ExternalLinkKind::Dde => ErrorKind::NA,
        _ => ErrorKind::Ref,
    }
}

/// Classifies a stored formula (OpenFormula `of:=…`, `oooc:=…` or Excel
/// text) as an external link, with the first target file or DDE topic.
pub fn classify_formula(text: &str) -> Option<(ExternalLinkKind, Option<String>)> {
    let t = text.trim();
    let body = t
        .strip_prefix("of:")
        .or_else(|| t.strip_prefix("oooc:"))
        .unwrap_or(t);
    // DDE("server";"topic";"item")
    if let Some(topic) = dde_topic(body) {
        return Some((ExternalLinkKind::Dde, topic));
    }
    // ODF: ['file:///a.ods'#$Sheet1.A1], ['http://…'#$S.A1]; also the
    // UI spelling 'file:///a.ods'#$Sheet1.A1 without brackets.
    if let Some(target) = odf_external_target(body) {
        return Some((ExternalLinkKind::ExternalReference, Some(target)));
    }
    // Excel: [1]Sheet1!A1, '[Book.xlsx]Sheet 1'!A1, [Book.xlsx]Sheet1!A1
    if let Some(target) = excel_external_target(body) {
        return Some((ExternalLinkKind::ExternalReference, Some(target)));
    }
    None
}

fn dde_topic(body: &str) -> Option<Option<String>> {
    let upper = body.to_ascii_uppercase();
    let mut from = 0;
    while let Some(i) = upper[from..].find("DDE(") {
        let at = from + i;
        let before = upper[..at].chars().next_back();
        if before.is_none_or(|c| !(c.is_alphanumeric() || c == '_' || c == '.')) {
            let args = &body[at + 4..];
            let mut quoted = args.split('"').skip(1).step_by(2);
            let server = quoted.next();
            let topic = quoted.next();
            let detail = match (server, topic) {
                (Some(s), Some(t)) => Some(format!("{s}|{t}")),
                (Some(s), None) => Some(s.to_string()),
                _ => None,
            };
            return Some(detail);
        }
        from = at + 4;
    }
    None
}

fn odf_external_target(body: &str) -> Option<String> {
    let mut rest = body;
    while let Some(q) = rest.find('\'') {
        let after = &rest[q + 1..];
        let end = find_closing_quote(after)?;
        let inner = &after[..end];
        let tail = &after[end + 1..];
        if tail.starts_with("#$") || tail.starts_with('#') {
            let lower = inner.to_ascii_lowercase();
            if lower.starts_with("file:")
                || lower.starts_with("http:")
                || lower.starts_with("https:")
                || lower.starts_with("ftp:")
                || lower.starts_with("smb:")
                || lower.starts_with('/')
                || lower.starts_with("\\\\")
                || lower.contains(".ods")
                || lower.contains(".xls")
                || lower.contains(".sxc")
            {
                return Some(inner.replace("''", "'"));
            }
        }
        rest = tail;
    }
    None
}

/// Index of the quote closing a `'…'` name (`''` is an escaped quote).
fn find_closing_quote(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\'' {
            if bytes.get(i + 1) == Some(&b'\'') {
                i += 2;
                continue;
            }
            return Some(i);
        }
        i += 1;
    }
    None
}

fn excel_external_target(body: &str) -> Option<String> {
    let bytes = body.as_bytes();
    let mut i = 0;
    let mut in_string = false;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'"' {
            in_string = !in_string;
            i += 1;
            continue;
        }
        if in_string {
            i += 1;
            continue;
        }
        if b == b'[' {
            // `[.A1]` and `[Sheet.A1]` are ODF references, not links.
            let close = body[i + 1..].find(']')?;
            let inner = &body[i + 1..i + 1 + close];
            let after = &body[i + 1 + close + 1..];
            let is_index = !inner.is_empty() && inner.bytes().all(|c| c.is_ascii_digit());
            let is_file = inner.to_ascii_lowercase().ends_with(".xlsx")
                || inner.to_ascii_lowercase().ends_with(".xls")
                || inner.to_ascii_lowercase().ends_with(".xlsm")
                || inner.to_ascii_lowercase().ends_with(".ods");
            let followed_by_sheet = after
                .chars()
                .next()
                .is_some_and(|c| c.is_alphanumeric() || c == '\'' || c == '_');
            if (is_index || is_file) && followed_by_sheet && after.contains('!') {
                return Some(inner.to_string());
            }
            i += close + 2;
            continue;
        }
        i += 1;
    }
    None
}

/// Counts one disabled link.
pub fn record(wb: &mut Workbook, kind: ExternalLinkKind, detail: Option<String>) {
    if let Some(w) = wb.external_links.iter_mut().find(|w| w.kind == kind) {
        w.count = w.count.saturating_add(1);
        if w.detail.is_none() {
            w.detail = detail.map(|d| clip(&d));
        }
        return;
    }
    wb.external_links.push(ExternalLinkWarning {
        kind,
        count: 1,
        detail: detail.map(|d| clip(&d)),
    });
}

/// The user-facing message of one warning.
pub fn message(w: &ExternalLinkWarning) -> String {
    let n = w.count;
    let plural = |one: &str, many: &str| {
        if n == 1 {
            one.to_string()
        } else {
            many.to_string()
        }
    };
    let detail = w
        .detail
        .as_deref()
        .map(|d| format!(" (first: {d})"))
        .unwrap_or_default();
    match w.kind {
        ExternalLinkKind::Dde => format!(
            "{n} DDE {} disabled by policy; the cells show #N/A{detail}.",
            plural("link was", "links were")
        ),
        ExternalLinkKind::ExternalReference => format!(
            "{n} {} to other files {} disabled by policy; the cells show #REF!{detail}.",
            plural("reference", "references"),
            plural("was", "were")
        ),
        ExternalLinkKind::LinkedSheet => format!(
            "{n} linked {} imported as values; the link to the source file is disabled by policy{detail}.",
            plural("sheet was", "sheets were")
        ),
        ExternalLinkKind::LinkedImage => format!(
            "{n} linked {} not loaded; links to external pictures are disabled by policy{detail}.",
            plural("picture was", "pictures were")
        ),
    }
}

/// Moves the readers' tagged warnings from `import_warnings` into
/// `external_links`, then appends one readable message per kind.
pub fn collect_import_warnings(wb: &mut Workbook) {
    let mut keep = Vec::with_capacity(wb.import_warnings.len());
    for w in std::mem::take(&mut wb.import_warnings) {
        match w.strip_prefix(WARNING_PREFIX) {
            Some(rest) => {
                let (tag, detail) = match rest.split_once(':') {
                    Some((t, d)) => (t, Some(d.to_string())),
                    None => (rest, None),
                };
                match kind_of_tag(tag) {
                    Some(kind) => record(wb, kind, detail),
                    None => keep.push(w),
                }
            }
            None => keep.push(w),
        }
    }
    for w in &wb.external_links {
        keep.push(message(w));
    }
    wb.import_warnings = keep;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_external_formulas() {
        let ext = classify_formula;
        assert_eq!(
            ext("of:=['file:///home/x/a.ods'#$Sheet1.A1]"),
            Some((
                ExternalLinkKind::ExternalReference,
                Some("file:///home/x/a.ods".into())
            ))
        );
        assert_eq!(
            ext("of:=[.A1]+['http://example.com/b.ods'#$S.B2]"),
            Some((
                ExternalLinkKind::ExternalReference,
                Some("http://example.com/b.ods".into())
            ))
        );
        assert_eq!(
            ext("=[1]Sheet1!A1*2"),
            Some((ExternalLinkKind::ExternalReference, Some("1".into())))
        );
        assert_eq!(
            ext("='[Book.xlsx]Sheet 1'!A1"),
            Some((
                ExternalLinkKind::ExternalReference,
                Some("Book.xlsx".into())
            ))
        );
        assert_eq!(
            ext("of:=DDE(\"soffice\";\"c:\\\\x.ods\";\"Sheet1.A1\")"),
            Some((ExternalLinkKind::Dde, Some("soffice|c:\\\\x.ods".into())))
        );
        assert_eq!(ext("of:=[.A1]+[Sheet2.B1]"), None);
        assert_eq!(ext("of:=SUM([.A1:.B2])"), None);
        assert_eq!(ext("=\"[1]Sheet1!A1\""), None, "inside a string");
        assert_eq!(ext("=ADDE(1)"), None);
        assert_eq!(ext("='It''s'.A1"), None);
    }

    #[test]
    fn collects_tagged_warnings() {
        let mut wb = Workbook::new();
        wb.import_warnings = vec![
            "plain".into(),
            tagged_warning(ExternalLinkKind::LinkedSheet, Some("file:///a.ods")),
            tagged_warning(ExternalLinkKind::LinkedSheet, None),
            tagged_warning(ExternalLinkKind::LinkedImage, None),
        ];
        collect_import_warnings(&mut wb);
        assert_eq!(wb.external_links.len(), 2);
        assert_eq!(wb.external_links[0].count, 2);
        assert_eq!(
            wb.external_links[0].detail.as_deref(),
            Some("file:///a.ods")
        );
        assert_eq!(wb.import_warnings.len(), 3);
        assert!(wb.import_warnings[1].contains("2 linked sheets"));
    }
}
