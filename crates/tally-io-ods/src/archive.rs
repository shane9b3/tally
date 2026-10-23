//! Bounded access to the entries of an ODS zip package.
//!
//! Implements §5.4 requirements 1 (zip bombs) and 4 (path traversal):
//!
//! - the entry count and the central-directory size are read from the
//!   end-of-central-directory records ([`precheck`], hand-parsed, zip64
//!   included) and capped *before* the zip crate indexes the central
//!   directory, so a flood of directory records costs nothing;
//! - every entry is read through `Read::take(limit + 1)` wrapped in a
//!   [`Guard`] that fails as soon as the per-entry or total uncompressed byte
//!   budget is exceeded (declared sizes in zip headers can lie, so they are
//!   never trusted for allocation);
//! - the compression ratio is checked continuously once the *package* has
//!   produced more than [`RATIO_FLOOR`] bytes, as total output against the
//!   total compressed bytes actually pulled for the entries read so far (a
//!   1000:1 bomb is rejected after ~10 MiB of output, not after the whole
//!   entry; many small bombs share one floor);
//! - reading an entry a second time is charged to a small re-read budget
//!   ([`MAX_REREAD_BYTES`]), so references that all point at one big part
//!   cannot multiply its cost;
//! - entries are only ever streamed into memory by exact name; names with
//!   `..` components, absolute paths, drive letters or backslashes are ignored.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom};
use std::rc::Rc;

use zip::ZipArchive;

use crate::{Limits, OdsError};

/// The compression ratio is only enforced once the package has inflated past
/// this (one allowance for the whole package, not one per entry).
pub(crate) const RATIO_FLOOR: u64 = 10 * 1024 * 1024;

/// Most bytes inflated from entries that were already read once (a picture
/// shown twice, say). Larger re-reads are refused.
pub(crate) const MAX_REREAD_BYTES: u64 = 64 * 1024 * 1024;

/// Largest central directory accepted (16 MiB: ~10,000 entries need ~1 MiB).
pub(crate) const MAX_CENTRAL_DIRECTORY_BYTES: u64 = 16 * 1024 * 1024;

const EOCD_SIG: u32 = 0x0605_4b50;
const EOCD_LEN: usize = 22;
const ZIP64_LOCATOR_SIG: u32 = 0x0706_4b50;
const ZIP64_LOCATOR_LEN: usize = 20;
const ZIP64_EOCD_SIG: u32 = 0x0606_4b50;
const ZIP64_EOCD_LEN: usize = 56;

fn le_u16(b: &[u8], i: usize) -> Option<u64> {
    let s = b.get(i..i + 2)?;
    Some(u64::from(u16::from_le_bytes([s[0], s[1]])))
}

fn le_u32(b: &[u8], i: usize) -> Option<u64> {
    let s = b.get(i..i + 4)?;
    Some(u64::from(u32::from_le_bytes([s[0], s[1], s[2], s[3]])))
}

fn le_u64(b: &[u8], i: usize) -> Option<u64> {
    let s = b.get(i..i + 8)?;
    let mut a = [0u8; 8];
    a.copy_from_slice(s);
    Some(u64::from_le_bytes(a))
}

/// Entry count and central-directory size from one end-of-central-directory
/// record (following the zip64 locator when the record says so).
fn directory_extent<R: Read + Seek>(
    reader: &mut R,
    tail: &[u8],
    tail_start: u64,
    at: usize,
) -> Result<(u64, u64), OdsError> {
    let bad = || OdsError::NotOds;
    let mut entries = le_u16(tail, at + 10).ok_or_else(bad)?;
    let mut cd_size = le_u32(tail, at + 12).ok_or_else(bad)?;
    let cd_offset = le_u32(tail, at + 16).ok_or_else(bad)?;
    if entries == 0xFFFF || cd_size == 0xFFFF_FFFF || cd_offset == 0xFFFF_FFFF {
        // zip64: the locator sits right before the classic record.
        let loc = at.checked_sub(ZIP64_LOCATOR_LEN);
        let record_at = match loc {
            Some(l) if le_u32(tail, l) == Some(u64::from(ZIP64_LOCATOR_SIG)) => le_u64(tail, l + 8),
            _ => None,
        };
        if let Some(off) = record_at {
            let mut rec = [0u8; ZIP64_EOCD_LEN];
            // The record may lie inside the tail we already have.
            let in_tail = off
                .checked_sub(tail_start)
                .and_then(|o| usize::try_from(o).ok())
                .and_then(|o| tail.get(o..o + ZIP64_EOCD_LEN));
            match in_tail {
                Some(b) => rec.copy_from_slice(b),
                None => {
                    reader.seek(SeekFrom::Start(off))?;
                    reader.read_exact(&mut rec).map_err(|_| bad())?;
                }
            }
            if le_u32(&rec, 0) != Some(u64::from(ZIP64_EOCD_SIG)) {
                return Err(OdsError::NotOds);
            }
            entries = entries.max(le_u64(&rec, 32).ok_or_else(bad)?);
            cd_size = cd_size.max(le_u64(&rec, 40).ok_or_else(bad)?);
        }
    }
    Ok((entries, cd_size))
}

/// Reads the end-of-central-directory record(s) and rejects the archive if
/// it declares more than `limits.max_entries` entries or a central directory
/// larger than [`MAX_CENTRAL_DIRECTORY_BYTES`], before anything is indexed.
///
/// Every EOCD signature in the last 64 KiB + 22 bytes is checked (a comment
/// may hide extra ones and parsers differ in which they pick), so the zip
/// crate cannot be steered to a bigger directory than the one checked here.
pub(crate) fn precheck<R: Read + Seek>(reader: &mut R, limits: &Limits) -> Result<(), OdsError> {
    let len = reader.seek(SeekFrom::End(0))?;
    let tail_len = len.min(u64::from(u16::MAX) + EOCD_LEN as u64);
    let tail_start = len - tail_len;
    reader.seek(SeekFrom::Start(tail_start))?;
    let mut tail = vec![0u8; tail_len as usize];
    reader.read_exact(&mut tail)?;
    let mut found = false;
    let sig = EOCD_SIG.to_le_bytes();
    let last = tail.len().saturating_sub(EOCD_LEN);
    for at in (0..=last).rev() {
        if tail.get(at..at + 4) != Some(sig.as_slice()) {
            continue;
        }
        let (entries, cd_size) = match directory_extent(reader, &tail, tail_start, at) {
            Ok(x) => x,
            // A stray signature in a comment: only a real record counts.
            Err(OdsError::NotOds) => continue,
            Err(e) => return Err(e),
        };
        found = true;
        if entries > limits.max_entries as u64 {
            return Err(OdsError::LimitExceeded {
                what: "zip entries",
                limit: limits.max_entries as u64,
            });
        }
        if cd_size > MAX_CENTRAL_DIRECTORY_BYTES {
            return Err(OdsError::LimitExceeded {
                what: "zip central directory bytes",
                limit: MAX_CENTRAL_DIRECTORY_BYTES,
            });
        }
    }
    reader.seek(SeekFrom::Start(0))?;
    if found {
        Ok(())
    } else {
        Err(OdsError::NotOds)
    }
}

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
    /// Compressed bytes credited to the package so far (all entries).
    credited: Rc<Cell<u64>>,
    /// This entry's share of `credited`.
    credited_self: u64,
    max_ratio: u64,
    /// Package bytes inflated by re-reads, when this entry was read before.
    reread: Option<Rc<Cell<u64>>>,
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
        if let Some(re) = &self.reread {
            re.set(re.get().saturating_add(n as u64));
            if re.get() > MAX_REREAD_BYTES {
                return Err(self.fail("re-read zip entry bytes", MAX_REREAD_BYTES));
            }
        }
        // Credit what this entry really pulled from the archive (never more
        // than its declared compressed size: the zip reader may read ahead).
        let pulled = self.consumed.get().saturating_sub(self.consumed_start);
        let mine = pulled.min(self.declared_compressed);
        let package_in = self
            .credited
            .get()
            .saturating_sub(self.credited_self)
            .saturating_add(mine);
        self.credited.set(package_in);
        self.credited_self = mine;
        let package_out = self.total.get();
        if package_out > RATIO_FLOOR
            && package_out > package_in.max(1).saturating_mul(self.max_ratio)
        {
            return Err(self.fail("compression ratio", self.max_ratio));
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
    credited: Rc<Cell<u64>>,
    reread: Rc<Cell<u64>>,
    read_before: HashSet<usize>,
    names: HashMap<String, usize>,
    limits: Limits,
}

impl<R: Read + Seek> Package<R> {
    /// Opens the zip and indexes safe entry names. A non-zip is `NotOds`.
    pub fn open(mut reader: R, limits: &Limits) -> Result<Self, OdsError> {
        precheck(&mut reader, limits)?;
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
            credited: Rc::new(Cell::new(0)),
            reread: Rc::new(Cell::new(0)),
            read_before: HashSet::new(),
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
            credited: Rc::clone(&self.credited),
            credited_self: 0,
            max_ratio: u64::from(self.limits.max_ratio),
            reread: (!self.read_before.insert(index)).then(|| Rc::clone(&self.reread)),
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
                credited: Rc::new(Cell::new(0)),
                credited_self: 0,
                max_ratio: ratio,
                reread: None,
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

    /// An entry read many times (say, every sheet pointing at one part)
    /// is charged to the re-read budget from the second read on.
    #[test]
    fn rereads_are_budgeted() {
        use std::io::Write;
        let mut z = zip::ZipWriter::new(io::Cursor::new(Vec::new()));
        let stored = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        z.start_file("big.xml", stored).unwrap();
        z.write_all(&vec![b'x'; 1024 * 1024]).unwrap();
        let bytes = z.finish().unwrap().into_inner();
        let mut pkg = Package::open(io::Cursor::new(bytes), &Limits::default()).unwrap();
        let read = |pkg: &mut Package<_>| {
            pkg.with_entry("big.xml", 2 * 1024 * 1024, |r| {
                io::copy(r, &mut io::sink())?;
                Ok(())
            })
        };
        let budget = (MAX_REREAD_BYTES / (1024 * 1024)) as usize;
        for _ in 0..=budget {
            read(&mut pkg).unwrap();
        }
        match read(&mut pkg) {
            Err(OdsError::LimitExceeded { what, .. }) => {
                assert_eq!(what, "re-read zip entry bytes")
            }
            other => panic!("{other:?}"),
        }
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
