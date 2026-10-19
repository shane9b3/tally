//! Bounded access to the entries of an ODS zip package.
//!
//! Implements §5.4 requirements 1 (zip bombs) and 4 (path traversal):
//!
//! - the entry count is capped before anything is inflated;
//! - every entry is read through `Read::take(limit + 1)` wrapped in a
//!   [`Guard`] that fails as soon as the per-entry or total uncompressed byte
//!   budget is exceeded (declared sizes in zip headers can lie, so they are
//!   never trusted for allocation);
//! - the compression ratio is checked continuously once an entry has
//!   produced more than [`RATIO_FLOOR`] bytes, against the compressed bytes
//!   actually pulled from the archive so far (a 1000:1 bomb is rejected after
//!   ~10 MiB of output, not after the whole entry);
//! - entries are only ever streamed into memory by exact name; names with
//!   `..` components, absolute paths, drive letters or backslashes are ignored.

use std::cell::Cell;
use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom};
use std::rc::Rc;

use zip::ZipArchive;

use crate::{Limits, OdsError};

/// The compression ratio is only enforced once an entry has inflated past this.
pub(crate) const RATIO_FLOOR: u64 = 10 * 1024 * 1024;

/// Wraps the archive reader and counts the bytes read from it.
struct CountingReader<R> {
    inner: R,
    count: Rc<Cell<u64>>,
}

impl<R: Read> Read for CountingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.count.set(self.count.get().saturating_add(n as u64));
        Ok(n)
    }
}

impl<R: Seek> Seek for CountingReader<R> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.inner.seek(pos)
    }
}

/// Which limit an entry read tripped.
type Trip = Rc<Cell<Option<(&'static str, u64)>>>;

/// Enforces the byte and ratio budgets on an inflating entry stream.
pub(crate) struct Guard<R> {
    inner: R,
    out: u64,
    max_entry: u64,
    total: Rc<Cell<u64>>,
    max_total: u64,
    consumed: Rc<Cell<u64>>,
    consumed_start: u64,
    declared_compressed: u64,
    max_ratio: u64,
    trip: Trip,
}

impl<R: Read> Guard<R> {
    fn fail(&self, what: &'static str, limit: u64) -> io::Error {
        self.trip.set(Some((what, limit)));
        io::Error::other(format!("{what} limit {limit} exceeded"))
    }
}

impl<R: Read> Read for Guard<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if let Some((what, limit)) = self.trip.get() {
            return Err(self.fail(what, limit));
        }
        let n = self.inner.read(buf)?;
        self.out = self.out.saturating_add(n as u64);
        self.total.set(self.total.get().saturating_add(n as u64));
        if self.out > self.max_entry {
            return Err(self.fail("zip entry uncompressed bytes", self.max_entry));
        }
        if self.total.get() > self.max_total {
            return Err(self.fail("total uncompressed bytes", self.max_total));
        }
        if self.out > RATIO_FLOOR {
            let pulled = self
                .consumed
                .get()
                .saturating_sub(self.consumed_start)
                .max(1);
            let compressed = pulled.min(self.declared_compressed.max(1));
            if self.out > compressed.saturating_mul(self.max_ratio) {
                return Err(self.fail("compression ratio", self.max_ratio));
            }
        }
        Ok(n)
    }
}

/// Returns `true` for entry names we refuse to look at.
pub(crate) fn is_unsafe_name(name: &str) -> bool {
    name.is_empty()
        || name.starts_with('/')
        || name.contains('\\')
        || name.contains('\0')
        || name.split('/').any(|c| c == ".." || c.contains(':'))
}

/// An opened ODS package.
pub(crate) struct Package<R: Read + Seek> {
    archive: ZipArchive<CountingReader<R>>,
    consumed: Rc<Cell<u64>>,
    total: Rc<Cell<u64>>,
    names: HashMap<String, usize>,
    limits: Limits,
}

impl<R: Read + Seek> Package<R> {
    /// Opens the zip and indexes safe entry names. A non-zip is `NotOds`.
    pub fn open(reader: R, limits: &Limits) -> Result<Self, OdsError> {
        let consumed = Rc::new(Cell::new(0));
        let counting = CountingReader {
            inner: reader,
            count: Rc::clone(&consumed),
        };
        let archive = ZipArchive::new(counting).map_err(|e| match e {
            zip::result::ZipError::Io(io) => OdsError::Io(io),
            _ => OdsError::NotOds,
        })?;
        if archive.len() > limits.max_entries {
            return Err(OdsError::LimitExceeded {
                what: "zip entries",
                limit: limits.max_entries as u64,
            });
        }
        let mut names = HashMap::new();
        for i in 0..archive.len() {
            let Some(Ok(name)) = archive.name_for_index(i) else {
                continue;
            };
            if is_unsafe_name(&name) {
                continue;
            }
            names.entry(name.into_owned()).or_insert(i);
        }
        Ok(Self {
            archive,
            consumed,
            total: Rc::new(Cell::new(0)),
            names,
            limits: limits.clone(),
        })
    }

    pub fn has(&self, name: &str) -> bool {
        self.names.contains_key(name)
    }

    /// Streams entry `name` (if present) through `f`, bounded by `max_bytes`.
    pub fn with_entry<T>(
        &mut self,
        name: &str,
        max_bytes: u64,
        f: impl FnOnce(&mut dyn BufRead) -> Result<T, OdsError>,
    ) -> Result<Option<T>, OdsError> {
        let Some(&index) = self.names.get(name) else {
            return Ok(None);
        };
        let consumed_start = self.consumed.get();
        let file = self.archive.by_index(index).map_err(|e| match e {
            zip::result::ZipError::UnsupportedArchive(msg) => OdsError::Unsupported(msg.into()),
            other => OdsError::Zip(other),
        })?;
        if file.encrypted() {
            return Err(OdsError::Unsupported("encrypted zip entries".into()));
        }
        let declared_compressed = file.compressed_size();
        let trip: Trip = Rc::new(Cell::new(None));
        let guard = Guard {
            inner: file.take(max_bytes.saturating_add(1)),
            out: 0,
            max_entry: max_bytes,
            total: Rc::clone(&self.total),
            max_total: self.limits.max_total_bytes,
            consumed: Rc::clone(&self.consumed),
            consumed_start,
            declared_compressed,
            max_ratio: u64::from(self.limits.max_ratio),
            trip: Rc::clone(&trip),
        };
        let mut reader = BufReader::with_capacity(64 * 1024, guard);
        let result = f(&mut reader);
        if let Some((what, limit)) = trip.get() {
            return Err(OdsError::LimitExceeded { what, limit });
        }
        result.map(Some)
    }

    /// Reads a small entry fully (bounded) into memory.
    pub fn read_small(&mut self, name: &str, max_bytes: u64) -> Result<Option<Vec<u8>>, OdsError> {
        self.with_entry(name, max_bytes, |r| {
            let mut v = Vec::new();
            r.read_to_end(&mut v)?;
            Ok(v)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A reader that claims nothing but yields `n` bytes of zeros.
    struct Endless(u64);
    impl Read for Endless {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let n = (buf.len() as u64).min(self.0) as usize;
            buf[..n].fill(0);
            self.0 -= n as u64;
            Ok(n)
        }
    }

    fn guard(
        inner: Endless,
        max: u64,
        consumed: u64,
        ratio: u64,
    ) -> (Guard<io::Take<Endless>>, Trip) {
        let trip: Trip = Rc::new(Cell::new(None));
        (
            Guard {
                inner: inner.take(max + 1),
                out: 0,
                max_entry: max,
                total: Rc::new(Cell::new(0)),
                max_total: u64::MAX,
                consumed: Rc::new(Cell::new(consumed)),
                consumed_start: 0,
                declared_compressed: 10,
                max_ratio: ratio,
                trip: Rc::clone(&trip),
            },
            trip,
        )
    }

    #[test]
    fn take_bounds_a_lying_entry() {
        // The "header" says 10 compressed bytes; the stream yields 1 GiB.
        let (mut g, trip) = guard(Endless(1 << 30), 1 << 20, u64::MAX, u64::MAX);
        let mut sink = Vec::new();
        assert!(g.read_to_end(&mut sink).is_err());
        assert!(sink.len() as u64 <= (1 << 20) + 64 * 1024);
        assert_eq!(
            trip.get().map(|t| t.0),
            Some("zip entry uncompressed bytes")
        );
    }

    #[test]
    fn ratio_trips_after_floor() {
        let (mut g, trip) = guard(Endless(1 << 30), 1 << 31, 1000, 200);
        let mut sink = Vec::new();
        assert!(g.read_to_end(&mut sink).is_err());
        assert!((sink.len() as u64) < RATIO_FLOOR + 1024 * 1024);
        assert_eq!(trip.get().map(|t| t.0), Some("compression ratio"));
    }

    #[test]
    fn unsafe_names() {
        for n in [
            "../content.xml",
            "/etc/passwd",
            "a\\b.xml",
            "x/../../y",
            "C:/x",
            "",
        ] {
            assert!(is_unsafe_name(n), "{n}");
        }
        for n in ["content.xml", "META-INF/manifest.xml", "Pictures/a..b.png"] {
            assert!(!is_unsafe_name(n), "{n}");
        }
    }
}
