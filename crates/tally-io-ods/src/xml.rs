//! A thin, hardened pull parser over `quick-xml`.
//!
//! Every XML stream in an ODS package goes through [`XmlReader`], which
//! enforces §5.4 requirement 2 of the architecture plan:
//!
//! - any `<!DOCTYPE …>` is rejected with [`OdsError::ForbiddenDoctype`]
//!   (ODF never needs one; this blocks XXE and entity-expansion bombs);
//! - only the five predefined entities and character references are
//!   resolved (quick-xml never fetches external entities);
//! - element depth and attributes per element are capped.

use std::io::BufRead;

use quick_xml::escape::resolve_predefined_entity;
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};

use crate::OdsError;

/// One element start tag with its (normalized, unescaped) attributes.
#[derive(Debug, Default)]
pub(crate) struct Elem {
    pub name: String,
    pub attrs: Vec<(String, String)>,
}

impl Elem {
    /// The value of attribute `key` (qualified name such as `table:name`).
    pub fn attr(&self, key: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
}

/// A simplified, owned XML event.
#[derive(Debug)]
pub(crate) enum Ev {
    Start(Elem),
    End,
    Text(String),
    Eof,
}

/// Limits applied while parsing one XML stream.
#[derive(Copy, Clone, Debug)]
pub(crate) struct XmlLimits {
    pub max_depth: u32,
    pub max_attrs: u32,
}

/// A pull parser that yields [`Ev`]s and tracks element depth.
pub(crate) struct XmlReader<R: BufRead> {
    reader: Reader<R>,
    buf: Vec<u8>,
    depth: u32,
    limits: XmlLimits,
    /// OOo 1.x names are mapped to ODF (see `sxc`).
    legacy: bool,
}

impl<R: BufRead> XmlReader<R> {
    pub fn new(inner: R, limits: XmlLimits) -> Self {
        let mut reader = Reader::from_reader(inner);
        let config = reader.config_mut();
        config.expand_empty_elements = true;
        config.trim_text(false);
        config.check_end_names = true;
        Self {
            reader,
            buf: Vec::with_capacity(4096),
            depth: 0,
            limits,
            legacy: false,
        }
    }

    /// A reader for OOo 1.x XML: element and attribute names are mapped
    /// to their ODF spellings as they are read.
    pub fn new_legacy(inner: R, limits: XmlLimits) -> Self {
        let mut r = Self::new(inner, limits);
        r.legacy = true;
        r
    }

    /// Number of currently open elements (after a `Start`, includes it).
    pub fn depth(&self) -> u32 {
        self.depth
    }

    /// The next event. `Eof` inside an open element is an error.
    pub fn next(&mut self) -> Result<Ev, OdsError> {
        loop {
            self.buf.clear();
            let ev = self
                .reader
                .read_event_into(&mut self.buf)
                .map_err(xml_err)?;
            match ev {
                Event::Start(start) => {
                    if self.depth >= self.limits.max_depth {
                        return Err(OdsError::LimitExceeded {
                            what: "XML element depth",
                            limit: u64::from(self.limits.max_depth),
                        });
                    }
                    let mut elem = convert_start(&start, self.limits.max_attrs)?;
                    if self.legacy {
                        crate::sxc::map_elem(&mut elem);
                    }
                    self.depth += 1;
                    return Ok(Ev::Start(elem));
                }
                Event::End(_) => {
                    self.depth = self.depth.saturating_sub(1);
                    return Ok(Ev::End);
                }
                // `expand_empty_elements` turns these into Start + End.
                Event::Empty(_) => {
                    return Err(OdsError::Xml("unexpected empty-element event".into()));
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
                    if let Some(ch) = r.resolve_char_ref().map_err(xml_err)? {
                        return Ok(Ev::Text(ch.to_string()));
                    }
                    let name = r.xml10_content();
                    return match resolve_predefined_entity(&name) {
                        Some(s) => Ok(Ev::Text(s.to_string())),
                        None => Err(OdsError::Xml(format!("undefined entity &{name};"))),
                    };
                }
                Event::DocType(_) => return Err(OdsError::ForbiddenDoctype),
                Event::Decl(_) | Event::PI(_) | Event::Comment(_) => continue,
                Event::Eof => {
                    if self.depth > 0 {
                        return Err(OdsError::Xml("unexpected end of document".into()));
                    }
                    return Ok(Ev::Eof);
                }
            }
        }
    }

    /// Skips the rest of the element whose `Start` was just returned.
    pub fn skip(&mut self) -> Result<(), OdsError> {
        let d = self.depth;
        loop {
            match self.next()? {
                Ev::End if self.depth < d => return Ok(()),
                Ev::Eof => return Ok(()),
                _ => {}
            }
        }
    }

    /// Reads the concatenated text content of the element whose `Start` was
    /// just returned (including text of nested elements), up to `max_len`
    /// bytes, and consumes its end tag.
    pub fn text_content(&mut self, max_len: usize) -> Result<String, OdsError> {
        let d = self.depth;
        let mut out = String::new();
        loop {
            match self.next()? {
                Ev::End if self.depth < d => return Ok(out),
                Ev::Text(t) => {
                    if out.len() + t.len() > max_len {
                        return Err(OdsError::LimitExceeded {
                            what: "XML text length",
                            limit: max_len as u64,
                        });
                    }
                    out.push_str(&t);
                }
                Ev::Eof => return Ok(out),
                _ => {}
            }
        }
    }
}

fn convert_start(start: &BytesStart<'_>, max_attrs: u32) -> Result<Elem, OdsError> {
    let name = start.name().as_ref().to_owned();
    let mut attrs = Vec::new();
    for attr in start.attributes() {
        let attr = attr.map_err(|e| OdsError::Xml(e.to_string()))?;
        if attrs.len() >= max_attrs as usize {
            return Err(OdsError::LimitExceeded {
                what: "XML attributes per element",
                limit: u64::from(max_attrs),
            });
        }
        let key = attr.key.as_ref().to_owned();
        let value = attr
            .normalized_value(XmlVersion::Implicit1_0)
            .map_err(xml_err)?
            .into_owned();
        attrs.push((key, value));
    }
    Ok(Elem { name, attrs })
}

pub(crate) fn xml_err(e: quick_xml::Error) -> OdsError {
    if let quick_xml::Error::Io(io) = &e {
        return OdsError::Io(std::io::Error::new(io.kind(), io.to_string()));
    }
    OdsError::Xml(e.to_string())
}

/// Escapes text for element content.
pub(crate) fn escape_text(s: &str, out: &mut String) {
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c if is_xml_char(c) => out.push(c),
            _ => out.push('\u{FFFD}'),
        }
    }
}

/// Escapes text for a double-quoted attribute value.
pub(crate) fn escape_attr(s: &str, out: &mut String) {
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            '\t' => out.push_str("&#9;"),
            c if is_xml_char(c) => out.push(c),
            _ => out.push('\u{FFFD}'),
        }
    }
}

/// Characters allowed in XML 1.0 documents.
fn is_xml_char(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..)
}
