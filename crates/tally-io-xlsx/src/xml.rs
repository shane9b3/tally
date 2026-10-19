//! A thin, hardened pull parser over `quick-xml`.
//!
//! Copied from `tally-io-ods/src/xml.rs`. TODO: share with tally-io-ods in a
//! common crate.
//!
//! Every XML stream in an XLSX package goes through [`XmlReader`], which
//! enforces §5.4 requirement 2 of the architecture plan:
//!
//! - any `<!DOCTYPE …>` is rejected with [`XlsxError::ForbiddenDoctype`]
//!   (OOXML never needs one; this blocks XXE and entity-expansion bombs);
//! - only the five predefined entities and character references are
//!   resolved (quick-xml never fetches external entities);
//! - element depth and attributes per element are capped.

use std::io::BufRead;

use quick_xml::escape::resolve_predefined_entity;
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};

use crate::XlsxError;

/// One element start tag with its (normalized, unescaped) attributes.
#[derive(Debug, Default)]
pub(crate) struct Elem {
    pub name: String,
    pub attrs: Vec<(String, String)>,
}

impl Elem {
    /// The element name without its namespace prefix (`x:c` → `c`). OOXML
    /// writers normally use the default namespace, but some prefix it.
    pub fn local(&self) -> &str {
        local_name(&self.name)
    }

    /// The value of the attribute whose local name is `key` (any prefix,
    /// or none). Namespace declarations (`xmlns:*`) never match.
    pub fn attr_local(&self, key: &str) -> Option<&str> {
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
        }
    }

    /// Number of currently open elements (after a `Start`, includes it).
    pub fn depth(&self) -> u32 {
        self.depth
    }

    /// The next event. `Eof` inside an open element is an error.
    pub fn next(&mut self) -> Result<Ev, XlsxError> {
        loop {
            self.buf.clear();
            let ev = self
                .reader
                .read_event_into(&mut self.buf)
                .map_err(xml_err)?;
            match ev {
                Event::Start(start) => {
                    if self.depth >= self.limits.max_depth {
                        return Err(XlsxError::LimitExceeded {
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
                // `expand_empty_elements` turns these into Start + End.
                Event::Empty(_) => {
                    return Err(XlsxError::Xml("unexpected empty-element event".into()));
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
                        None => Err(XlsxError::Xml(format!("undefined entity &{name};"))),
                    };
                }
                Event::DocType(_) => return Err(XlsxError::ForbiddenDoctype),
                Event::Decl(_) | Event::PI(_) | Event::Comment(_) => continue,
                Event::Eof => {
                    if self.depth > 0 {
                        return Err(XlsxError::Xml("unexpected end of document".into()));
                    }
                    return Ok(Ev::Eof);
                }
            }
        }
    }

    /// Skips the rest of the element whose `Start` was just returned.
    pub fn skip(&mut self) -> Result<(), XlsxError> {
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
    pub fn text_content(&mut self, max_len: usize) -> Result<String, XlsxError> {
        let d = self.depth;
        let mut out = String::new();
        loop {
            match self.next()? {
                Ev::End if self.depth < d => return Ok(out),
                Ev::Text(t) => {
                    if out.len() + t.len() > max_len {
                        return Err(XlsxError::LimitExceeded {
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

fn convert_start(start: &BytesStart<'_>, max_attrs: u32) -> Result<Elem, XlsxError> {
    let name = start.name().as_ref().to_owned();
    let mut attrs = Vec::new();
    for attr in start.attributes() {
        let attr = attr.map_err(|e| XlsxError::Xml(e.to_string()))?;
        if attrs.len() >= max_attrs as usize {
            return Err(XlsxError::LimitExceeded {
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

pub(crate) fn xml_err(e: quick_xml::Error) -> XlsxError {
    if let quick_xml::Error::Io(io) = &e {
        return XlsxError::Io(std::io::Error::new(io.kind(), io.to_string()));
    }
    XlsxError::Xml(e.to_string())
}
