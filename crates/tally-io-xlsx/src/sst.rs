//! `xl/sharedStrings.xml` and rich/inline text (`<si>`, `<is>`).

use std::io::BufRead;
use std::sync::Arc;

use crate::xml::{Ev, XmlReader};
use crate::{Limits, XlsxError};

/// Longest single string we accept (Excel's own limit is 32,767 chars).
pub(crate) const MAX_CELL_TEXT: usize = 16 * 1024 * 1024;

/// Reads the text of the string item whose start tag (`<si>` / `<is>`) was
/// just returned: plain `<t>` or the concatenated `<r><t>` runs. Phonetic
/// runs (`<rPh>`) are skipped. Consumes the end tag.
pub(crate) fn read_string_item<R: BufRead>(
    xr: &mut XmlReader<R>,
    max_len: usize,
) -> Result<String, XlsxError> {
    let base = xr.depth();
    let mut out = String::new();
    // Depth of the `<t>` we are inside, if any.
    let mut in_t: Option<u32> = None;
    loop {
        match xr.next()? {
            Ev::Start(e) => match e.local() {
                "t" => in_t = Some(xr.depth()),
                "r" => {}
                // Phonetic guides, run properties, phonetic properties.
                _ => xr.skip()?,
            },
            Ev::Text(t) => {
                if in_t.is_some() {
                    if out.len() + t.len() > max_len {
                        return Err(XlsxError::LimitExceeded {
                            what: "cell text bytes",
                            limit: max_len as u64,
                        });
                    }
                    out.push_str(&t);
                }
            }
            Ev::End => {
                if xr.depth() < base {
                    return Ok(out);
                }
                if in_t.is_some_and(|d| xr.depth() < d) {
                    in_t = None;
                }
            }
            Ev::Eof => return Ok(out),
        }
    }
}

/// Parses the shared string table.
pub(crate) fn parse_sst<R: BufRead>(
    xr: &mut XmlReader<R>,
    limits: &Limits,
) -> Result<Vec<Arc<str>>, XlsxError> {
    let mut out: Vec<Arc<str>> = Vec::new();
    let mut bytes: u64 = 0;
    loop {
        match xr.next()? {
            Ev::Start(e) => match e.local() {
                "sst" => {}
                "si" => {
                    if out.len() as u64 >= limits.max_cells {
                        return Err(XlsxError::LimitExceeded {
                            what: "shared strings",
                            limit: limits.max_cells,
                        });
                    }
                    let s = read_string_item(xr, MAX_CELL_TEXT)?;
                    bytes = bytes.saturating_add(s.len() as u64);
                    if bytes > limits.max_text_bytes {
                        return Err(XlsxError::LimitExceeded {
                            what: "materialized cell text bytes",
                            limit: limits.max_text_bytes,
                        });
                    }
                    out.push(Arc::from(s));
                }
                _ => xr.skip()?,
            },
            Ev::Eof => return Ok(out),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xml::XmlLimits;

    fn sst(xml: &str) -> Vec<Arc<str>> {
        let mut xr = XmlReader::new(
            xml.as_bytes(),
            XmlLimits {
                max_depth: 64,
                max_attrs: 64,
            },
        );
        parse_sst(&mut xr, &Limits::default()).unwrap_or_default()
    }

    #[test]
    fn plain_rich_and_phonetic() {
        let v = sst(concat!(
            r#"<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="4" uniqueCount="4">"#,
            r#"<si><t>plain</t></si>"#,
            r#"<si><r><rPr><b/></rPr><t>bo</t></r><r><t xml:space="preserve"> ld </t></r></si>"#,
            r#"<si><t>漢字</t><rPh sb="0" eb="2"><t>かんじ</t></rPh><phoneticPr fontId="1"/></si>"#,
            r#"<si><t/></si><si><t>a&amp;b&#10;c</t></si>"#,
            "</sst>"
        ));
        let v: Vec<&str> = v.iter().map(|s| &**s).collect();
        assert_eq!(v, ["plain", "bo ld ", "漢字", "", "a&b\nc"]);
    }
}
