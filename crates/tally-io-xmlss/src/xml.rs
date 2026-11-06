//! A thin, hardened pull parser over `quick-xml`, adapted from
//! `tally-io-xlsx/src/xml.rs` (same rules, see `docs/security.md` §2):
//!
//! - any `<!DOCTYPE …>` is rejected with [`XmlssError::ForbiddenDoctype`]
//!   (SpreadsheetML never needs one; this blocks XXE and entity bombs);
//! - only the five predefined entities and character references are
//!   resolved (quick-xml never fetches external entities);
//! - element depth and attributes per element are capped;
//! - one event (a tag with its attributes, a text run, a comment) may span
//!   at most `Limits::max_event_bytes` of input.
//!
//! Element and attribute names are matched by local name (`ss:Cell`,
//! `Cell` and `x:Cell` are the same), as Excel writes the SpreadsheetML
//! namespaces both as the default namespace and with prefixes.

use std::io::{self, BufRead, Read};

use quick_xml::escape::resolve_predefined_entity;
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};

use crate::XmlssError;

/// One element start tag with its (normalized, unescaped) attributes.
#[derive(Debug, Default)]
pub(crate) struct Elem {
    pub name: String,
    pub attrs: Vec<(String, String)>,
}

impl Elem {
    /// The element name without its namespace prefix.
    pub fn local(&self) -> &str {
        local_name(&self.name)
    }

    /// The value of the attribute whose local name is `key` (any prefix, or
    /// none). Namespace declarations never match.
    pub fn attr(&self, key: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| !k.starts_with("xmlns") && local_name(k) == key)
            .map(|(_, v)| v.as_str())
    }
}

/// `prefix:name` → `name`.
pub(crate) fn local_name(name: &str) -> &str {
    name.rsplit_once(':').map_or(name, |(_, l)| l)
}

/// A simplified, owned XML event.
#[derive(Debug)]
pub(crate) enum Ev {
    Start(Elem),
    End,
    Text(String),
    Eof,
}

/// Limits applied while parsing.
#[derive(Copy, Clone, Debug)]
pub(crate) struct XmlLimits {
    pub max_depth: u32,
    pub max_attrs: u32,
    pub max_event: u64,
}

/// A [`BufRead`] that fails once more than `max` bytes are consumed since
/// the last [`EventBudget::reset`] (called before every event).
pub(crate) struct EventBudget<R> {
    inner: R,
    used: u64,
    max: u64,
    tripped: bool,
}

impl<R: BufRead> EventBudget<R> {
    fn new(inner: R, max: u64) -> Self {
        Self {
            inner,
            used: 0,
            max,
            tripped: false,
        }
    }
}

impl<R: BufRead> Read for EventBudget<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let avail = self.fill_buf()?;
        let n = avail.len().min(buf.len());
        buf[..n].copy_from_slice(&avail[..n]);
        self.consume(n);
        Ok(n)
    }
}

impl<R: BufRead> BufRead for EventBudget<R> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        if self.used > self.max {
            self.tripped = true;
            return Err(io::Error::other("XML event too large"));
        }
        // Hand out at most one byte past the budget, so a byte slice (one
        // huge buffer) cannot overshoot it.
        let room = usize::try_from(self.max.saturating_sub(self.used).saturating_add(1))
            .unwrap_or(usize::MAX);
        let buf = self.inner.fill_buf()?;
        Ok(&buf[..buf.len().min(room)])
    }

    fn consume(&mut self, amt: usize) {
        self.used = self.used.saturating_add(amt as u64);
        self.inner.consume(amt);
    }
}

/// A pull parser that yields [`Ev`]s and tracks element depth.
pub(crate) struct XmlReader<R: BufRead> {
    reader: Reader<EventBudget<R>>,
    buf: Vec<u8>,
    depth: u32,
    limits: XmlLimits,
}

impl<R: BufRead> XmlReader<R> {
    pub fn new(inner: R, limits: XmlLimits) -> Self {
        let mut reader = Reader::from_reader(EventBudget::new(inner, limits.max_event));
        let config = reader.config_mut();
        config.expand_empty_elements = true;
        config.trim_text(false);
        config.check_end_names = true;
        Self {
            reader,
            buf: Vec::with_capacity(4096),
            depth: 0,
            limits,
        }
    }

    /// Number of currently open elements (after a `Start`, includes it).
    pub fn depth(&self) -> u32 {
        self.depth
    }

    /// The next event. `Eof` inside an open element is an error.
    pub fn next(&mut self) -> Result<Ev, XmlssError> {
        loop {
            if self.buf.capacity() > 1024 * 1024 {
                self.buf = Vec::with_capacity(4096);
            }
            self.buf.clear();
            self.reader.get_mut().used = 0;
            let ev = match self.reader.read_event_into(&mut self.buf) {
                Ok(ev) => ev,
                Err(e) => {
                    let budget = self.reader.get_ref();
                    if budget.tripped {
                        return Err(XmlssError::LimitExceeded {
                            what: "XML event bytes",
                            limit: budget.max,
                        });
                    }
                    return Err(xml_err(&e));
                }
            };
            match ev {
                Event::Start(start) => {
                    if self.depth >= self.limits.max_depth {
                        return Err(XmlssError::LimitExceeded {
                            what: "XML element depth",
                            limit: u64::from(self.limits.max_depth),
                        });
                    }
                    let elem = convert_start(&start, self.limits.max_attrs)?;
                    self.depth += 1;
                    return Ok(Ev::Start(elem));
                }
                Event::End(_) => {
                    self.depth = self.depth.saturating_sub(1);
                    return Ok(Ev::End);
                }
                Event::Empty(_) => {
                    return Err(XmlssError::Xml("unexpected empty-element event".into()));
                }
                Event::Text(t) => {
                    let s = t.xml10_content();
                    if s.is_empty() {
                        continue;
                    }
                    return Ok(Ev::Text(s.into_owned()));
                }
                Event::CData(c) => return Ok(Ev::Text(c.xml10_content().into_owned())),
                Event::GeneralRef(r) => {
                    if let Some(ch) = r.resolve_char_ref().map_err(|e| xml_err(&e))? {
                        return Ok(Ev::Text(ch.to_string()));
                    }
                    let name = r.xml10_content();
                    return match resolve_predefined_entity(&name) {
                        Some(s) => Ok(Ev::Text(s.to_string())),
                        None => Err(XmlssError::Xml(format!("undefined entity &{name};"))),
                    };
                }
                Event::DocType(_) => return Err(XmlssError::ForbiddenDoctype),
                Event::Decl(_) | Event::PI(_) | Event::Comment(_) => continue,
                Event::Eof => {
                    if self.depth > 0 {
                        return Err(XmlssError::Xml("unexpected end of document".into()));
                    }
                    return Ok(Ev::Eof);
                }
            }
        }
    }

    /// Skips the rest of the element whose `Start` was just returned.
    pub fn skip(&mut self) -> Result<(), XmlssError> {
        let d = self.depth;
        loop {
            match self.next()? {
                Ev::End if self.depth < d => return Ok(()),
                Ev::Eof => return Ok(()),
                _ => {}
            }
        }
    }

    /// The concatenated text of the element whose `Start` was just returned
    /// (including the text of nested elements, such as the `html:B` and
    /// `html:Font` runs of rich `ss:Data`), up to `max_len` bytes; consumes
    /// its end tag.
    pub fn text_content(&mut self, max_len: usize) -> Result<String, XmlssError> {
        self.text_inner(max_len, false)
    }

    /// [`XmlReader::text_content`] that cuts the text at `max_len` bytes
    /// (on a character boundary) instead of failing.
    pub fn text_truncated(&mut self, max_len: usize) -> Result<String, XmlssError> {
        self.text_inner(max_len, true)
    }

    fn text_inner(&mut self, max_len: usize, truncate: bool) -> Result<String, XmlssError> {
        let d = self.depth;
        let mut out = String::new();
        loop {
            match self.next()? {
                Ev::End if self.depth < d => return Ok(out),
                Ev::Text(t) if truncate => {
                    let room = max_len.saturating_sub(out.len());
                    let mut end = t.len().min(room);
                    while !t.is_char_boundary(end) {
                        end -= 1;
                    }
                    out.push_str(&t[..end]);
                }
                Ev::Text(t) => {
                    if out.len().saturating_add(t.len()) > max_len {
                        return Err(XmlssError::LimitExceeded {
                            what: "cell text bytes",
                            limit: max_len as u64,
                        });
                    }
                    out.push_str(&t);
                }
                // `<html:Br/>` in rich text is a line break.
                Ev::Start(e) if e.local().eq_ignore_ascii_case("br") => {
                    if out.len() < max_len {
                        out.push('\n');
                    }
                }
                Ev::Eof => return Ok(out),
                _ => {}
            }
        }
    }
}

fn convert_start(start: &BytesStart<'_>, max_attrs: u32) -> Result<Elem, XmlssError> {
    let name = start.name().as_ref().to_owned();
    let mut attrs = Vec::new();
    for attr in start.attributes() {
        let attr = attr.map_err(|e| XmlssError::Xml(e.to_string()))?;
        if attrs.len() >= max_attrs as usize {
            return Err(XmlssError::LimitExceeded {
                what: "XML attributes per element",
                limit: u64::from(max_attrs),
            });
        }
        let key = attr.key.as_ref().to_owned();
        let value = attr
            .normalized_value(XmlVersion::Implicit1_0)
            .map_err(|e| xml_err(&e))?
            .into_owned();
        attrs.push((key, value));
    }
    Ok(Elem { name, attrs })
}

fn xml_err(e: &quick_xml::Error) -> XmlssError {
    XmlssError::Xml(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMITS: XmlLimits = XmlLimits {
        max_depth: 256,
        max_attrs: 256,
        max_event: 1024 * 1024,
    };

    fn events(doc: &str) -> Result<usize, XmlssError> {
        let mut r = XmlReader::new(doc.as_bytes(), LIMITS);
        let mut n = 0;
        loop {
            match r.next()? {
                Ev::Eof => return Ok(n),
                _ => n += 1,
            }
        }
    }

    #[test]
    fn huge_attribute_and_text_hit_the_event_cap() {
        let doc = format!(r#"<a v="{}"/>"#, "x".repeat(2 * 1024 * 1024));
        assert!(matches!(
            events(&doc),
            Err(XmlssError::LimitExceeded {
                what: "XML event bytes",
                ..
            })
        ));
        let doc = format!("<a>{}</a>", "x".repeat(2 * 1024 * 1024));
        assert!(matches!(
            events(&doc),
            Err(XmlssError::LimitExceeded {
                what: "XML event bytes",
                ..
            })
        ));
        // Per event, not per stream.
        let cell = format!("<c>{}</c>", "y".repeat(64 * 1024));
        let doc = format!("<a>{}</a>", cell.repeat(64));
        assert_eq!(events(&doc).unwrap(), 2 + 64 * 3);
    }

    #[test]
    fn local_names() {
        let mut r = XmlReader::new(
            br#"<ss:Cell xmlns:ss="u" ss:Index="3" x:Other="1"/>"#.as_slice(),
            LIMITS,
        );
        let Ev::Start(e) = r.next().unwrap() else {
            panic!()
        };
        assert_eq!(e.local(), "Cell");
        assert_eq!(e.attr("Index"), Some("3"));
        assert_eq!(e.attr("ss"), None);
    }
}
