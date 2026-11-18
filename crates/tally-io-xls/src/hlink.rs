//! Cell hyperlinks of a BIFF8 sheet: the `HLINK` record.
//!
//! The record holds the cells the link is on and a serialized hyperlink
//! object ([MS-OSHARED] 2.3.7.1): flags, then (as the flags say) a display
//! name, a target frame, the target as a URL moniker, a file moniker or a
//! plain string, and a location inside the target. The target is put
//! together as OpenOffice 4.1 does (`XclImpHyperlink::ReadEmbeddedData`,
//! xicontent.cxx) and written the way the XLSX reader writes the model's
//! link: a URL as it is, `#Sheet.A1` for a place in the document, a file
//! as a path relative to the document or as a `file:` URL.
//!
//! Hostile input: every length is checked against the record (a string's
//! declared length is never allocated from), and a target longer than
//! [`MAX_URL_BYTES`] is refused.

use crate::prescan::{u16_at, u32_at};

/// Longest target kept (the engine's own limit for a link).
pub(crate) const MAX_URL_BYTES: usize = 8 * 1024;
/// Most `HLINK` bytes looked at (the record and its continues).
pub(crate) const MAX_RECORD_BYTES: usize = 64 * 1024;
/// Most directory levels a file link may climb (`../`).
const MAX_UP_LEVELS: u16 = 64;

/// The class id of a URL moniker.
const URL_MONIKER: [u8; 16] = [
    0xE0, 0xC9, 0xEA, 0x79, 0xF9, 0xBA, 0xCE, 0x11, 0x8C, 0x82, 0x00, 0xAA, 0x00, 0x4B, 0xA9, 0x0B,
];
/// The class id of a file moniker.
const FILE_MONIKER: [u8; 16] = [
    0x03, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46,
];

/// A hyperlink: the cells (BIFF rows and columns, inclusive) and the
/// target.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Hlink {
    pub r0: u32,
    pub r1: u32,
    pub c0: u32,
    pub c1: u32,
    pub url: String,
}

/// A checked reader over the record.
struct Cur<'a> {
    d: &'a [u8],
    p: usize,
}

impl<'a> Cur<'a> {
    fn bytes(&mut self, n: usize) -> Option<&'a [u8]> {
        let b = self.d.get(self.p..self.p.checked_add(n)?)?;
        self.p += n;
        Some(b)
    }

    fn u16(&mut self) -> Option<u16> {
        let v = u16_at(self.d, self.p)?;
        self.p += 2;
        Some(v)
    }

    fn u32(&mut self) -> Option<u32> {
        let v = u32_at(self.d, self.p)?;
        self.p += 4;
        Some(v)
    }

    /// `units` UTF-16 code units, up to the first NUL.
    fn utf16(&mut self, units: usize) -> Option<String> {
        let b = self.bytes(units.checked_mul(2)?)?;
        let units: Vec<u16> = b
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .take_while(|u| *u != 0)
            .collect();
        Some(String::from_utf16_lossy(&units))
    }

    /// A string with a 32-bit character count in front.
    fn string32(&mut self) -> Option<String> {
        let n = usize::try_from(self.u32()?).ok()?;
        self.utf16(n)
    }
}

/// Reads an `HLINK` record; `None` for one that is cut, garbled, has a
/// target of a kind this reader does not know, or no target at all.
pub(crate) fn parse(d: &[u8]) -> Option<Hlink> {
    let mut c = Cur { d, p: 0 };
    let (r0, r1) = (u32::from(c.u16()?), u32::from(c.u16()?));
    // Excel ignores the high byte of the columns (OpenOffice #i80006#).
    let (c0, c1) = (u32::from(c.u16()? & 0xFF), u32::from(c.u16()? & 0xFF));
    if r0 > r1 || c0 > c1 {
        return None;
    }
    // The class id of the link object and its stream version.
    c.bytes(20)?;
    let flags = c.u32()?;
    // Display name and target frame: not kept.
    if flags & 0x10 != 0 {
        c.string32()?;
    }
    if flags & 0x80 != 0 {
        c.string32()?;
    }
    let mut target: Option<String> = None;
    if flags & 0x100 != 0 {
        // The target as a string: a UNC path.
        target = Some(file_target(&c.string32()?, 0)?);
    } else if flags & 0x01 != 0 {
        let class = c.bytes(16)?;
        if class == URL_MONIKER {
            // A byte count, then the URL (NUL-terminated; newer writers
            // add 24 bytes of version data inside the count).
            let n = usize::try_from(c.u32()?).ok()?;
            let url = c.utf16(n / 2)?;
            target = Some(if flags & 0x02 == 0 && !has_scheme(&url) {
                // A relative link.
                file_target(&url, 0)?
            } else {
                url
            });
        } else if class == FILE_MONIKER {
            // Levels up, the 8.3 path (bytes), 24 reserved bytes, then
            // optionally the full path in UTF-16.
            let up = c.u16()?;
            let n = usize::try_from(c.u32()?).ok()?;
            let short: String = c
                .bytes(n)?
                .iter()
                .take_while(|b| **b != 0)
                .map(|b| char::from(*b))
                .collect();
            c.bytes(24)?;
            let path = if c.u32()? > 0 {
                let n = usize::try_from(c.u32()?).ok()?;
                c.bytes(2)?;
                c.utf16(n / 2)?
            } else {
                short
            };
            target = Some(file_target(&path, up)?);
        } else {
            return None;
        }
    }
    let mark = if flags & 0x08 != 0 {
        Some(c.string32()?)
    } else {
        None
    };
    let url = match (target, mark) {
        (Some(t), Some(m)) if !m.is_empty() => format!("{t}#{m}"),
        (Some(t), _) => t,
        // A place in this document: `Sheet!A1` is `Sheet.A1` in the model.
        (None, Some(m)) if !m.is_empty() => format!("#{}", m.replacen('!', ".", 1)),
        (None, _) => return None,
    };
    if url.is_empty() || url.len() > MAX_URL_BYTES || url.chars().any(char::is_control) {
        return None;
    }
    Some(Hlink {
        r0,
        r1,
        c0,
        c1,
        url,
    })
}

/// Whether `url` starts with a scheme (`http:`, `mailto:`…); a drive
/// letter (`C:`) is not one.
fn has_scheme(url: &str) -> bool {
    url.split_once(':').is_some_and(|(scheme, _)| {
        scheme.len() > 1
            && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    })
}

/// A file path as a link target: `\` becomes `/`; a UNC path and a path
/// with a drive letter become `file:` URLs; a relative path stays
/// relative to the document, `up` directories above it.
fn file_target(path: &str, up: u16) -> Option<String> {
    if up > MAX_UP_LEVELS || path.is_empty() {
        return None;
    }
    let p = path.replace('\\', "/");
    let b = p.as_bytes();
    Some(if p.starts_with("//") {
        format!("file:{p}")
    } else if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        format!("file:///{p}")
    } else if p.starts_with('/') {
        format!("file://{p}")
    } else {
        let mut out = "../".repeat(usize::from(up));
        out.push_str(&p);
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s32(s: &str) -> Vec<u8> {
        let units: Vec<u16> = s.encode_utf16().chain(std::iter::once(0)).collect();
        let mut d = (units.len() as u32).to_le_bytes().to_vec();
        d.extend(units.iter().flat_map(|u| u.to_le_bytes()));
        d
    }

    fn head(flags: u32) -> Vec<u8> {
        let mut d = Vec::new();
        for w in [1u16, 2, 3, 4] {
            d.extend_from_slice(&w.to_le_bytes());
        }
        d.extend_from_slice(&[0xD0; 16]);
        d.extend_from_slice(&2u32.to_le_bytes());
        d.extend_from_slice(&flags.to_le_bytes());
        d
    }

    fn url_link(url: &str, mark: Option<&str>) -> Vec<u8> {
        let mut d = head(0x03 | if mark.is_some() { 0x08 } else { 0 });
        d.extend_from_slice(&URL_MONIKER);
        let units: Vec<u16> = url.encode_utf16().chain(std::iter::once(0)).collect();
        d.extend_from_slice(&(2 * units.len() as u32).to_le_bytes());
        d.extend(units.iter().flat_map(|u| u.to_le_bytes()));
        if let Some(m) = mark {
            d.extend(s32(m));
        }
        d
    }

    fn file_link(up: u16, short: &str, long: Option<&str>) -> Vec<u8> {
        let mut d = head(0x01);
        d.extend_from_slice(&FILE_MONIKER);
        d.extend_from_slice(&up.to_le_bytes());
        d.extend_from_slice(&(short.len() as u32 + 1).to_le_bytes());
        d.extend_from_slice(short.as_bytes());
        d.push(0);
        d.extend_from_slice(&[0; 24]);
        match long {
            Some(l) => {
                let bytes: Vec<u8> = l.encode_utf16().flat_map(u16::to_le_bytes).collect();
                d.extend_from_slice(&(bytes.len() as u32 + 6).to_le_bytes());
                d.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
                d.extend_from_slice(&[3, 0]);
                d.extend(bytes);
            }
            None => d.extend_from_slice(&0u32.to_le_bytes()),
        }
        d
    }

    fn url_of(d: &[u8]) -> Option<String> {
        parse(d).map(|h| h.url)
    }

    #[test]
    fn targets() {
        let h = parse(&url_link("https://example.org/a?b=1", None)).unwrap();
        assert_eq!((h.r0, h.r1, h.c0, h.c1), (1, 2, 3, 4));
        assert_eq!(h.url, "https://example.org/a?b=1");
        assert_eq!(
            url_of(&url_link("https://example.org/", Some("part"))).as_deref(),
            Some("https://example.org/#part")
        );
        assert_eq!(
            url_of(&url_link("mailto:someone@example.org", None)).as_deref(),
            Some("mailto:someone@example.org")
        );
        // A place in the document, with a display name in front.
        let mut d = head(0x18);
        d.extend(s32("shown text"));
        d.extend(s32("'My Sheet'!B7"));
        assert_eq!(url_of(&d).as_deref(), Some("#'My Sheet'.B7"));
        // Files: relative (two levels up), with a drive, 8.3 only.
        assert_eq!(
            url_of(&file_link(2, "DATA~1.XLS", Some("sub\\data file.xls"))).as_deref(),
            Some("../../sub/data file.xls")
        );
        assert_eq!(
            url_of(&file_link(0, "C:\\D\\B.XLS", Some("C:\\dir\\book.xls"))).as_deref(),
            Some("file:///C:/dir/book.xls")
        );
        assert_eq!(
            url_of(&file_link(0, "BOOK.XLS", None)).as_deref(),
            Some("BOOK.XLS")
        );
        // A UNC path as a string, with a location.
        let mut d = head(0x0100 | 0x01 | 0x08);
        d.extend(s32("\\\\server\\share\\book.xls"));
        d.extend(s32("Sheet1!A1"));
        assert_eq!(
            url_of(&d).as_deref(),
            Some("file://server/share/book.xls#Sheet1!A1")
        );
    }

    #[test]
    fn refused() {
        // No target; an unknown moniker; reversed cells; too many levels
        // up; a control character; a target over the limit.
        assert_eq!(parse(&head(0)), None);
        let mut unknown = head(0x01);
        unknown.extend_from_slice(&[7; 16]);
        unknown.extend(s32("x"));
        assert_eq!(parse(&unknown), None);
        let mut reversed = url_link("https://example.org/", None);
        reversed[0] = 9;
        assert_eq!(parse(&reversed), None);
        assert_eq!(parse(&file_link(65, "A.XLS", None)), None);
        assert_eq!(parse(&url_link("https://example.org/\u{1}", None)), None);
        let long = format!("https://example.org/{}", "a".repeat(MAX_URL_BYTES));
        assert_eq!(parse(&url_link(&long, None)), None);
        // A string that claims more characters than the record holds.
        let mut lying = head(0x08);
        lying.extend_from_slice(&u32::MAX.to_le_bytes());
        lying.extend_from_slice(&[b'a', 0]);
        assert_eq!(parse(&lying), None);
    }

    #[test]
    fn every_cut_and_every_flag() {
        let mut both = head(0x18 | 0x80);
        both.extend(s32("name"));
        both.extend(s32("frame"));
        both.extend(s32("Sheet1!A1"));
        let samples = [
            url_link("https://example.org/", Some("m")),
            file_link(1, "A.XLS", Some("a long name.xls")),
            both,
        ];
        for full in &samples {
            assert!(parse(full).is_some());
            for cut in 0..full.len() {
                let _ = parse(&full[..cut]);
            }
            // Every flag combination over the same bytes.
            for flags in 0..0x400u32 {
                let mut m = full.clone();
                m[28..32].copy_from_slice(&flags.to_le_bytes());
                let _ = parse(&m);
            }
            for at in 0..full.len() {
                for b in [0u8, 1, 0x7F, 0x80, 0xFF] {
                    let mut m = full.clone();
                    m[at] = b;
                    let _ = parse(&m);
                }
            }
        }
    }
}
