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
//! - element depth and attributes per element are capped;
//! - one event (a tag with its attributes, a text run, a comment) may span
//!   at most [`MAX_EVENT_BYTES`] of input, so one event cannot make the
//!   parser buffer more than that, however large the entry is;
//! - attribute values that parsers may keep are bounded per package: values
//!   on non-cell elements are charged to one budget ([`AttrBudget`],
//!   `Limits::max_attr_bytes`), values on cell elements longer than
//!   [`MAX_BULK_ATTR_BYTES`] are read as empty, and names and ids that are stored are capped
//!   at the source ([`capped`], [`MAX_NAME_BYTES`], [`MAX_ID_BYTES`]).

use std::cell::Cell;
use std::io::{self, BufRead, Read};

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

/// Elements that carry the per-cell bulk of worksheets and
/// `sharedStrings.xml` (by local name). Their attributes are not charged to
/// the package attribute budget (what a cell retains is bounded by
/// `max_cells` and `max_text_bytes`); instead a value longer than
/// [`MAX_BULK_ATTR_BYTES`] is read as empty.
const BULK_ELEMENTS: &[&str] = &[
    "row",
    "c",
    "v",
    "f",
    "is",
    "si",
    "t",
    "r",
    "rPr",
    "sz",
    "rFont",
    "color",
    "family",
    "scheme",
    "b",
    "i",
    "u",
    "strike",
    "vertAlign",
    "charset",
    "outline",
    "shadow",
    "condense",
    "extend",
];

/// Attributes of [`BULK_ELEMENTS`] that may be longer than
/// [`MAX_BULK_ATTR_BYTES`].
const BULK_LONG_ATTRS: &[&str] = &[];

fn element_key(name: &str) -> &str {
    local_name(name)
}

/// Longest attribute value accepted on a [`BULK_ELEMENTS`] element (style
/// names, cell references, value types; LibreOffice and Excel write a few
/// dozen bytes).
pub(crate) const MAX_BULK_ATTR_BYTES: usize = 1024;

/// Longest sheet or style name accepted (Calc allows 255 characters per
/// name; ODF style names escape spaces as `_20_`, so allow 1 KiB).
pub(crate) const MAX_NAME_BYTES: usize = 1024;

/// Longest relationship id or similar token kept (Excel writes `rId12`).
pub(crate) const MAX_ID_BYTES: usize = 256;

thread_local! {
    /// The attribute budget of the package being read: (charged, max).
    static ATTR_BUDGET: Cell<Option<(u64, u64)>> = const { Cell::new(None) };
}

/// While alive, every attribute value the [`XmlReader`]s of this thread hand
/// out on an element outside [`BULK_ELEMENTS`] is charged to one budget of
/// `max` bytes (`Limits::max_attr_bytes`), so names, ids, references and
/// other strings that parsers keep are bounded per package, not per entry.
/// Readers are synchronous, so a thread-local scope is exact.
pub(crate) struct AttrBudget {
    prev: Option<(u64, u64)>,
}

impl AttrBudget {
    pub(crate) fn install(max: u64) -> Self {
        Self {
            prev: ATTR_BUDGET.with(|b| b.replace(Some((0, max)))),
        }
    }
}

impl Drop for AttrBudget {
    fn drop(&mut self) {
        let prev = self.prev;
        ATTR_BUDGET.with(|b| b.set(prev));
    }
}

/// Applies the bulk-element cap or charges the package budget.
fn charge_attrs(name: &str, attrs: &mut [(String, String)]) -> Result<(), XlsxError> {
    if BULK_ELEMENTS.contains(&element_key(name)) {
        for (k, v) in attrs.iter_mut() {
            if v.len() > MAX_BULK_ATTR_BYTES && !BULK_LONG_ATTRS.contains(&k.as_str()) {
                // Never a real value: read it as empty (an invalid reference
                // or an unknown style), like other malformed attributes.
                *v = String::new();
            }
        }
        return Ok(());
    }
    let n: u64 = attrs.iter().map(|(_, v)| v.len() as u64).sum();
    ATTR_BUDGET.with(|b| match b.get() {
        Some((used, max)) => {
            let used = used.saturating_add(n);
            b.set(Some((used, max)));
            if used > max {
                Err(XlsxError::LimitExceeded {
                    what: "XML attribute bytes",
                    limit: max,
                })
            } else {
                Ok(())
            }
        }
        None => Ok(()),
    })
}

/// `value` if it is at most `max` bytes, else a limit error for `what`.
pub(crate) fn capped<'a>(
    value: Option<&'a str>,
    max: usize,
    what: &'static str,
) -> Result<Option<&'a str>, XlsxError> {
    match value {
        Some(v) if v.len() > max => Err(XlsxError::LimitExceeded {
            what,
            limit: max as u64,
        }),
        v => Ok(v),
    }
}

/// Most input bytes one XML event may span. quick-xml buffers a whole event
/// (a start tag with all its attributes, or a text run) before returning it,
/// so this bounds the parser's memory independently of the entry size. Cell
/// text is capped at 16 MiB and formulas at 8,192 characters, so real files
/// stay far below it.
pub(crate) const MAX_EVENT_BYTES: u64 = 64 * 1024 * 1024;

/// A [`BufRead`] that fails once more than `max` bytes are consumed since
/// the last [`EventBudget::reset`] (called before every event), having read
/// at most `max + 1` of them.
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

    fn reset(&mut self) {
        self.used = 0;
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
        // Hand out at most one byte past the budget, so an inner reader with
        // a huge buffer (a byte slice) cannot overshoot it.
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
        Self::with_event_cap(inner, limits, MAX_EVENT_BYTES)
    }

    /// [`XmlReader::new`] with a custom per-event byte cap (tests).
    pub fn with_event_cap(inner: R, limits: XmlLimits, max_event: u64) -> Self {
        let mut reader = Reader::from_reader(EventBudget::new(inner, max_event));
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
            // Do not keep a huge buffer around after one large event.
            if self.buf.capacity() > 1024 * 1024 {
                self.buf = Vec::with_capacity(4096);
            }
            self.buf.clear();
            self.reader.get_mut().reset();
            let ev = match self.reader.read_event_into(&mut self.buf) {
                Ok(ev) => ev,
                Err(e) => {
                    let budget = self.reader.get_ref();
                    if budget.tripped {
                        return Err(XlsxError::LimitExceeded {
                            what: "XML event bytes",
                            limit: budget.max,
                        });
                    }
                    return Err(xml_err(e));
                }
            };
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
    charge_attrs(&name, &mut attrs)?;
    Ok(Elem { name, attrs })
}

pub(crate) fn xml_err(e: quick_xml::Error) -> XlsxError {
    if let quick_xml::Error::Io(io) = &e {
        return XlsxError::Io(std::io::Error::new(io.kind(), io.to_string()));
    }
    XlsxError::Xml(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMITS: XmlLimits = XmlLimits {
        max_depth: 256,
        max_attrs: 256,
    };

    fn events(doc: &str, cap: u64) -> Result<Vec<Ev>, XlsxError> {
        let mut r = XmlReader::with_event_cap(doc.as_bytes(), LIMITS, cap);
        let mut out = Vec::new();
        loop {
            match r.next()? {
                Ev::Eof => return Ok(out),
                ev => out.push(ev),
            }
        }
    }

    #[test]
    fn one_huge_attribute_hits_the_event_cap() {
        let doc = format!(r#"<a v="{}"/>"#, "x".repeat(2 * 1024 * 1024));
        match events(&doc, 1024 * 1024) {
            Err(XlsxError::LimitExceeded { what, limit }) => {
                assert_eq!(what, "XML event bytes");
                assert_eq!(limit, 1024 * 1024);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn one_huge_text_run_hits_the_event_cap() {
        let doc = format!("<a>{}</a>", "x".repeat(2 * 1024 * 1024));
        assert!(matches!(
            events(&doc, 1024 * 1024),
            Err(XlsxError::LimitExceeded {
                what: "XML event bytes",
                ..
            })
        ));
    }

    #[test]
    fn the_cap_is_per_event_not_per_stream() {
        // 64 events of 64 KiB each: 4 MiB in total, far over a 1 MiB cap.
        let cell = format!("<c>{}</c>", "y".repeat(64 * 1024));
        let doc = format!("<a>{}</a>", cell.repeat(64));
        let evs = events(&doc, 1024 * 1024).unwrap();
        assert_eq!(evs.len(), 2 + 64 * 3);
        // The default cap is far above any legitimate event.
        assert_eq!(MAX_EVENT_BYTES, 64 * 1024 * 1024);
    }
}
