//! Tally-side validation of an `.xls` file before calamine sees it
//! (security review 4, C4; re-audited against calamine 0.38 on 2026-11-18,
//! docs/security.md "XLS pre-scan audit").
//!
//! calamine allocates from sizes it reads in the file (sector chains,
//! `DIMENSIONS` reservations, string capacities, the bounding box of the
//! cell coordinates) before any of Tally's budgets run, and an allocation
//! failure aborts the process (`catch_unwind` does not catch it). It also
//! indexes records without checking their length, which panics. This
//! module therefore:
//!
//! 1. Extracts the workbook stream itself from the OLE2 container with
//!    strict chain walks (visited set, every sector inside the file, no
//!    FAT sector listed twice, stream length ≤ the chain's length ≤
//!    [`MAX_STREAM_BYTES`]) and rebuilds a minimal, canonical compound
//!    file around it, so calamine's container code only ever sees a file
//!    Tally wrote.
//! 2. Walks the BIFF records the way calamine's `RecordIter` does and
//!    neutralizes what calamine would trip over where the file can still
//!    be read faithfully: `DIMENSIONS` records are hidden (calamine only
//!    uses them for a `Vec::reserve`), declared string lengths of
//!    `LABEL`/`RSTRING`/`STRING`/`FORMAT` records are clamped to the bytes
//!    actually present (the decoded text is unchanged), `PtgAttrSpace`
//!    counts in cell formulas are set to 0 (cosmetic spaces that calamine
//!    inserts one by one, quadratically), a cell formula that ends inside
//!    a token (or holds a value calamine's arithmetic overflows on) gets
//!    an empty expression, so calamine reports it as undecodable and the
//!    cell keeps its cached value, a `NAME` whose name or formula is cut
//!    short keeps its place in the name table with an empty formula, a
//!    `MERGEDCELLS` count is cut to the areas present, and optional
//!    records too short for their fixed fields (`CODEPAGE`, `DATEMODE`,
//!    `EXTERNSHEET`, `MERGEDCELLS`) are hidden. "Hidden" means the record
//!    type is overwritten with [`HIDDEN_RECORD`], which no reader knows.
//! 3. Refuses a file calamine cannot read without panicking and that
//!    cannot be repaired in place (a `BOF`, `BOUNDSHEET`, `FILEPASS`,
//!    `NAME` or `FORMULA` record shorter than its fixed part, a sheet
//!    offset past the stream, a `MULRK` that contradicts its length, a
//!    damaged shared string table).
//! 4. Refuses a file whose remaining calamine-side cost exceeds a budget:
//!    the dense area of every sheet's value and formula bounding boxes
//!    (calamine materializes `Range`s densely, all sheets at once), the
//!    number of cell records, an upper bound on the text calamine builds
//!    (shared strings, their copies per `LABELSST`, labels, decoded
//!    formulas; six bytes per character unless the code page is UTF-16),
//!    the formula decoder's quadratic work, and the `CONTINUE` records of
//!    one record (calamine keeps a slice per `CONTINUE` and takes the
//!    shared strings' ones from the front of a vector, quadratically).
//!
//! Residual risk: this models calamine [`AUDITED_CALAMINE`]'s decoder.
//! calamine is pinned exactly and the model must be re-reviewed on every
//! upgrade (docs/security.md; `tests/calamine_audit.rs` fails when the
//! locked version is not the audited one).
//!
//! The record framing here ([`record`]) is shared with the formatting
//! reader (`format.rs`), which walks the same sanitized stream.

use std::collections::HashSet;

/// Largest workbook stream accepted (256 MiB).
pub const MAX_STREAM_BYTES: usize = 256 * 1024 * 1024;
/// Most dense cells calamine may materialize, all sheets together
/// (value and formula ranges are counted separately).
pub const MAX_DENSE_CELLS: u64 = 8_000_000;
/// Upper bound on the bytes of text calamine builds.
pub const MAX_TEXT_ESTIMATE: u64 = 256 * 1024 * 1024;
/// Upper bound on calamine's formula decoder work (tokens × output length,
/// summed over formulas).
pub const MAX_FORMULA_WORK: u64 = 4_000_000_000;
/// Most `CONTINUE` records behind one record. calamine and the framing
/// here keep 16 bytes per `CONTINUE`, which may be empty (4 bytes of
/// file); a stream of the largest size holds about 32,000 full ones.
pub const MAX_CONTINUES: usize = 1 << 20;
/// Most `CONTINUE` records behind the `SST`: calamine removes each from
/// the front of a vector (about 8 n² bytes moved).
pub const MAX_SST_CONTINUES: usize = 65_536;
/// The calamine version the model in this module was checked against
/// (docs/security.md "XLS pre-scan audit"). `tests/calamine_audit.rs`
/// compares it with the lock files.
pub const AUDITED_CALAMINE: &str = "0.38.0";
/// The record type a hidden record gets: neither calamine nor the
/// formatting reader knows it.
pub const HIDDEN_RECORD: u16 = 0xFFFF;

const ENDOFCHAIN: u32 = 0xFFFF_FFFE;
const FREESECT: u32 = 0xFFFF_FFFF;
const FATSECT: u32 = 0xFFFF_FFFD;
const DIFSECT: u32 = 0xFFFF_FFFC;
const RESERVED: u32 = 0xFFFF_FFFA;

/// Why a file was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The container or record structure is malformed.
    Malformed(String),
    /// A budget was exceeded.
    Budget {
        /// What.
        what: &'static str,
        /// The limit.
        limit: u64,
    },
}

fn bad(s: impl Into<String>) -> Refusal {
    Refusal::Malformed(s.into())
}

pub(crate) fn u16_at(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes([*b.get(i)?, *b.get(i + 1)?]))
}

pub(crate) fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes([
        *b.get(i)?,
        *b.get(i + 1)?,
        *b.get(i + 2)?,
        *b.get(i + 3)?,
    ]))
}

fn u64_at(b: &[u8], i: usize) -> Option<u64> {
    let lo = u32_at(b, i)? as u64;
    let hi = u32_at(b, i + 4)? as u64;
    Some(lo | (hi << 32))
}

// ---------------------------------------------------------------- CFB

struct Container<'a> {
    file: &'a [u8],
    size: usize,
    header_len: usize,
    maxid: u32,
    fat: Vec<u32>,
}

impl<'a> Container<'a> {
    /// Sector `id` (calamine: data after the header; a short last sector
    /// is returned short).
    fn sector(&self, id: u32) -> Result<&'a [u8], Refusal> {
        if id > self.maxid {
            return Err(bad(format!("sector {id} beyond the end of the file")));
        }
        let start = self.header_len + id as usize * self.size;
        if start >= self.file.len() {
            return Err(bad(format!("sector {id} beyond the end of the file")));
        }
        let end = (start + self.size).min(self.file.len());
        Ok(&self.file[start..end])
    }

    /// Reads a FAT chain: acyclic, every link inside the FAT, at most
    /// `len` bytes (stops early like calamine).
    fn chain(&self, start: u32, len: usize, cap: usize) -> Result<Vec<u8>, Refusal> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        let mut id = start;
        while id != ENDOFCHAIN {
            if !seen.insert(id) {
                return Err(bad("sector chain loops"));
            }
            let s = self.sector(id)?;
            let take = s.len().min(len - out.len());
            if out.len() + take > cap {
                return Err(Refusal::Budget {
                    what: "workbook stream bytes",
                    limit: cap as u64,
                });
            }
            out.extend_from_slice(&s[..take]);
            if out.len() >= len {
                break;
            }
            id = *self
                .fat
                .get(id as usize)
                .ok_or_else(|| bad("sector chain leaves the FAT"))?;
        }
        Ok(out)
    }
}

struct DirEntry {
    name: String,
    start: u32,
    len: u64,
}

/// A parsed compound-file directory.
struct Directory<'a> {
    c: Container<'a>,
    dirs: Vec<DirEntry>,
    version: u16,
    mini_fat_start: u32,
    mini_fat_len: usize,
    size: usize,
}

/// The names of the compound file's directory entries (storages and
/// streams), read with the same strict chain walks as the workbook stream.
pub fn directory_names(file: &[u8]) -> Result<Vec<String>, Refusal> {
    Ok(read_directory(file)?
        .dirs
        .into_iter()
        .map(|d| d.name)
        .collect())
}

fn read_directory(file: &[u8]) -> Result<Directory<'_>, Refusal> {
    if file.len() < 512 || !file.starts_with(&crate::OLE2_MAGIC) {
        return Err(bad("not a compound file"));
    }
    let size = match u16_at(file, 30) {
        Some(9) => 512usize,
        Some(12) => 4096usize,
        _ => return Err(bad("invalid sector shift")),
    };
    if u16_at(file, 32) != Some(6) {
        return Err(bad("invalid mini sector shift"));
    }
    if file.len() < size {
        return Err(bad("truncated header"));
    }
    let version = u16_at(file, 26).unwrap_or(0);
    let dir_start = u32_at(file, 48).unwrap_or(ENDOFCHAIN);
    let mini_fat_start = u32_at(file, 60).unwrap_or(ENDOFCHAIN);
    let mini_fat_len = u32_at(file, 64).unwrap_or(0) as usize;
    let difat_start = u32_at(file, 68).unwrap_or(ENDOFCHAIN);
    let maxid = (file.len() / size) as u32;
    let mut c = Container {
        file,
        size,
        header_len: size,
        maxid,
        fat: Vec::new(),
    };
    // DIFAT: the header's 109 entries, then the chain whose last entry of
    // each sector is the next sector (calamine pops it).
    let mut difat: Vec<u32> = (0..109)
        .map(|i| u32_at(file, 76 + 4 * i).unwrap_or(FREESECT))
        .collect();
    let mut seen = HashSet::new();
    let mut id = difat_start;
    while id < RESERVED {
        if !seen.insert(id) {
            return Err(bad("DIFAT chain loops"));
        }
        let s = c.sector(id)?;
        difat.extend(
            s.chunks_exact(4)
                .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])),
        );
        id = difat.pop().unwrap_or(ENDOFCHAIN);
    }
    let fat_ids: Vec<u32> = difat.into_iter().filter(|id| *id < DIFSECT).collect();
    if fat_ids.len() as u64 > maxid as u64 + 1 {
        return Err(bad("more FAT sectors than the file holds"));
    }
    let mut fat_seen = HashSet::new();
    let mut fat = Vec::with_capacity(fat_ids.len() * size / 4);
    for id in fat_ids {
        if !fat_seen.insert(id) {
            return Err(bad("a FAT sector is listed twice"));
        }
        let s = c.sector(id)?;
        fat.extend(
            s.chunks_exact(4)
                .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])),
        );
    }
    c.fat = fat;
    let dir_bytes = c.chain(dir_start, usize::MAX, file.len())?;
    let dirs: Vec<DirEntry> = dir_bytes
        .chunks(128)
        .filter(|e| e.len() == 128)
        .map(|e| {
            let units: Vec<u16> = e[..64]
                .chunks_exact(2)
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .take_while(|u| *u != 0)
                .collect();
            DirEntry {
                name: String::from_utf16_lossy(&units),
                start: u32_at(e, 116).unwrap_or(ENDOFCHAIN),
                len: if size == 512 {
                    u32_at(e, 120).unwrap_or(0) as u64
                } else {
                    u64_at(e, 120).unwrap_or(0)
                },
            }
        })
        .collect();
    Ok(Directory {
        c,
        dirs,
        version,
        mini_fat_start,
        mini_fat_len,
        size,
    })
}

/// Extracts the workbook stream: `Workbook`, then `Book`, compared without
/// case as calamine 0.38 does (the first entry of that name).
pub fn extract_workbook_stream(file: &[u8]) -> Result<Vec<u8>, Refusal> {
    let Directory {
        c,
        dirs,
        version,
        mini_fat_start,
        mini_fat_len,
        size,
    } = read_directory(file)?;
    let Some(root) = dirs.first() else {
        return Err(bad("empty root directory"));
    };
    if version != 3 && root.start == ENDOFCHAIN {
        return Err(bad("empty root directory"));
    }
    let (mini_fat, mini_stream) = if mini_fat_len > 0 {
        let ms = c.chain(
            root.start,
            root.len.min(file.len() as u64) as usize,
            file.len(),
        )?;
        let mf_len = mini_fat_len.saturating_mul(size).min(file.len());
        let mf = c.chain(mini_fat_start, mf_len, file.len())?;
        let mf: Vec<u32> = mf
            .chunks_exact(4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect();
        (mf, ms)
    } else {
        (Vec::new(), Vec::new())
    };
    let mut last_err = bad("no workbook stream");
    for name in ["Workbook", "Book"] {
        let Some(d) = dirs.iter().find(|d| d.name.eq_ignore_ascii_case(name)) else {
            continue;
        };
        if d.len > MAX_STREAM_BYTES as u64 {
            return Err(Refusal::Budget {
                what: "workbook stream bytes",
                limit: MAX_STREAM_BYTES as u64,
            });
        }
        let len = d.len as usize;
        let r = if len < 4096 {
            mini_chain(&mini_stream, &mini_fat, d.start, len)
        } else {
            c.chain(d.start, len, MAX_STREAM_BYTES)
        };
        match r {
            Ok(s) => return Ok(s),
            Err(e) => last_err = e,
        }
    }
    Err(last_err)
}

fn mini_chain(ms: &[u8], mf: &[u32], start: u32, len: usize) -> Result<Vec<u8>, Refusal> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut id = start;
    while id != ENDOFCHAIN {
        if !seen.insert(id) {
            return Err(bad("mini sector chain loops"));
        }
        let st = id as usize * 64;
        if st >= ms.len() {
            return Err(bad("mini sector beyond the mini stream"));
        }
        let s = &ms[st..(st + 64).min(ms.len())];
        let take = s.len().min(len - out.len());
        out.extend_from_slice(&s[..take]);
        if out.len() >= len {
            break;
        }
        id = *mf
            .get(id as usize)
            .ok_or_else(|| bad("mini sector chain leaves the mini FAT"))?;
    }
    Ok(out)
}

/// Builds a canonical version-3 compound file (512-byte sectors, no mini
/// stream) holding `stream` as `Workbook`. Streams shorter than 4,096
/// bytes are padded with zeros so they live in regular sectors. Returns
/// the file and the offset of the stream in it.
pub fn build_container(mut stream: Vec<u8>) -> (Vec<u8>, usize) {
    const S: usize = 512;
    if stream.len() < 4096 {
        stream.resize(4096, 0);
    }
    let len = stream.len();
    let n = len.div_ceil(S);
    // FAT sectors f, DIFAT sectors d, one directory sector.
    let mut f = (n + 1).div_ceil(128);
    let d = loop {
        let d = f.saturating_sub(109).div_ceil(127);
        if f * 128 >= n + f + d + 1 {
            break d;
        }
        f += 1;
    };
    let dir = (f + d) as u32;
    let first = dir + 1;
    let total = f + d + 1 + n;
    let mut fat = vec![FREESECT; f * 128];
    for e in fat.iter_mut().take(f) {
        *e = FATSECT;
    }
    for e in fat.iter_mut().skip(f).take(d) {
        *e = DIFSECT;
    }
    fat[dir as usize] = ENDOFCHAIN;
    for i in 0..n {
        let k = first as usize + i;
        fat[k] = if i + 1 < n {
            (k + 1) as u32
        } else {
            ENDOFCHAIN
        };
    }
    debug_assert!(total <= fat.len());
    let mut out = Vec::with_capacity(S + (f + d + 1) * S + n * S);
    out.extend_from_slice(&crate::OLE2_MAGIC);
    out.extend_from_slice(&[0u8; 16]);
    for v in [0x3Eu16, 3, 0xFFFE, 9, 6] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&[0u8; 6]);
    let difat_start = if d > 0 { f as u32 } else { ENDOFCHAIN };
    for v in [
        0u32,
        f as u32,
        dir,
        0,
        4096,
        ENDOFCHAIN,
        0,
        difat_start,
        d as u32,
    ] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    for i in 0..109 {
        let v = if i < f { i as u32 } else { FREESECT };
        out.extend_from_slice(&v.to_le_bytes());
    }
    debug_assert_eq!(out.len(), S);
    for v in &fat {
        out.extend_from_slice(&v.to_le_bytes());
    }
    // DIFAT sectors: 127 FAT sector ids each, then the next DIFAT sector.
    let mut rest = (109..f).map(|i| i as u32);
    for k in 0..d {
        for _ in 0..127 {
            out.extend_from_slice(&rest.next().unwrap_or(FREESECT).to_le_bytes());
        }
        let next = if k + 1 < d {
            (f + k + 1) as u32
        } else {
            ENDOFCHAIN
        };
        out.extend_from_slice(&next.to_le_bytes());
    }
    // Directory: Root Entry and Workbook.
    let entry = |name: &str, typ: u8, child: u32, start: u32, size: u32| {
        let mut e = vec![0u8; 128];
        let units: Vec<u16> = name.encode_utf16().collect();
        for (i, u) in units.iter().enumerate() {
            e[2 * i..2 * i + 2].copy_from_slice(&u.to_le_bytes());
        }
        e[64..66].copy_from_slice(&((units.len() as u16 + 1) * 2).to_le_bytes());
        e[66] = typ;
        e[67] = 1;
        e[68..72].copy_from_slice(&FREESECT.to_le_bytes());
        e[72..76].copy_from_slice(&FREESECT.to_le_bytes());
        e[76..80].copy_from_slice(&child.to_le_bytes());
        e[116..120].copy_from_slice(&start.to_le_bytes());
        e[120..124].copy_from_slice(&size.to_le_bytes());
        e
    };
    out.extend(entry("Root Entry", 5, 1, ENDOFCHAIN, 0));
    out.extend(entry("Workbook", 2, FREESECT, first, len as u32));
    out.extend(std::iter::repeat_n(0u8, 256));
    let off = out.len();
    out.extend_from_slice(&stream);
    out.resize(off + n * S, 0);
    (out, off)
}

// ---------------------------------------------------------------- BIFF

/// One record as calamine's `RecordIter` frames it.
pub(crate) struct Rec {
    pub(crate) typ: u16,
    /// Offset of the record data in the stream.
    pub(crate) data: usize,
    pub(crate) len: usize,
    /// (offset, len) of each following CONTINUE record's data.
    pub(crate) cont: Vec<(usize, usize)>,
    /// Offset after the record and its continues.
    pub(crate) next: usize,
}

/// Why a record cannot be framed.
enum FrameError {
    /// The stream ends inside the record (calamine fails the workbook).
    Short,
    /// More than [`MAX_CONTINUES`] `CONTINUE` records follow.
    Continues,
}

/// Frames the record at `pos`; `None` at the end, `Err` on a short stream
/// (calamine fails the workbook then) or past [`MAX_CONTINUES`] (the
/// pre-scan refuses such a file, so no reader behind it gets there).
pub(crate) fn record(s: &[u8], pos: usize) -> Option<Result<Rec, ()>> {
    frame(s, pos).map(|r| r.map_err(|_| ()))
}

fn frame(s: &[u8], pos: usize) -> Option<Result<Rec, FrameError>> {
    let rest = s.len().saturating_sub(pos);
    if rest == 0 {
        return None;
    }
    if rest < 4 {
        return Some(Err(FrameError::Short));
    }
    let typ = u16_at(s, pos).unwrap_or(0);
    let len = u16_at(s, pos + 2).unwrap_or(0) as usize;
    if rest < len + 4 {
        return Some(Err(FrameError::Short));
    }
    let data = pos + 4;
    let mut next = data + len;
    let mut cont = Vec::new();
    if s.len() - next > 4 && u16_at(s, next) == Some(0x003C) {
        while s.len() - next > 4 && u16_at(s, next) == Some(0x003C) {
            let l = u16_at(s, next + 2).unwrap_or(0) as usize;
            if s.len() - next < l + 4 {
                return Some(Err(FrameError::Short));
            }
            if cont.len() == MAX_CONTINUES {
                return Some(Err(FrameError::Continues));
            }
            cont.push((next + 4, l));
            next += 4 + l;
        }
    }
    Some(Ok(Rec {
        typ,
        data,
        len,
        cont,
        next,
    }))
}

/// A cursor over a record's data and its continues (calamine's
/// `Record::continue_record`).
struct Cur<'a> {
    s: &'a [u8],
    at: usize,
    end: usize,
    cont: std::slice::Iter<'a, (usize, usize)>,
}

impl Cur<'_> {
    fn left(&self) -> usize {
        self.end - self.at
    }
    fn next_cont(&mut self) -> bool {
        match self.cont.next() {
            Some(&(o, l)) => {
                self.at = o;
                self.end = o + l;
                true
            }
            None => false,
        }
    }
    fn byte(&self, i: usize) -> Option<u8> {
        (self.at + i < self.end).then(|| self.s[self.at + i])
    }
    fn skip(&mut self, mut n: usize) -> bool {
        while n > 0 {
            if self.left() == 0 && !self.next_cont() {
                return false;
            }
            let l = n.min(self.left());
            self.at += l;
            n -= l;
        }
        true
    }
}

/// An upper bound on the UTF-8 bytes calamine's string decoder produces.
///
/// With the UTF-16 code page (1200, the default and what every BIFF8 file
/// declares) an 8-bit character is one unit below U+0100 (2 bytes) and a
/// 16-bit one at most 3 bytes. With any other `CODEPAGE` calamine hands
/// the bytes of the string (8-bit characters widened with a zero byte
/// each) to that code page's decoder, which yields at most 3 bytes per
/// input byte: 6 per character. The strings of a stream older than BIFF8
/// have no high-byte flag and are decoded as they are: 3 per byte.
#[derive(Clone, Copy)]
struct Enc {
    utf16: bool,
}

impl Enc {
    /// Per character of a string whose high-byte flag is `hb`.
    fn per(self, hb: bool) -> u64 {
        match (self.utf16, hb) {
            (true, false) => 2,
            (true, true) => 3,
            (false, _) => 6,
        }
    }
    /// Per character of a string of a BIFF8 stream (whatever its flag) or
    /// of an older one.
    fn max(self, pre_biff8: bool) -> u64 {
        if self.utf16 || pre_biff8 {
            3
        } else {
            6
        }
    }
}

/// Bytes of text calamine builds for the shared strings, and each
/// string's decoded-length bound (for `LABELSST` copies), exactly as
/// calamine's `parse_sst` numbers them. Where calamine would fail or
/// panic (a string cut short, an empty `CONTINUE` inside a string) the
/// file is refused.
fn scan_sst(s: &[u8], r: &Rec, enc: Enc) -> Result<(u64, Vec<u32>), Refusal> {
    let damaged = || bad("the shared string table is damaged");
    let mut out = Vec::new();
    let mut total = 0u64;
    if r.len < 8 {
        return Err(damaged());
    }
    let mut c = Cur {
        s,
        at: r.data + 8,
        end: r.data + r.len,
        cont: r.cont.iter(),
    };
    loop {
        if total > MAX_TEXT_ESTIMATE {
            return Err(over("text bytes", MAX_TEXT_ESTIMATE));
        }
        if c.left() == 0 {
            if !c.next_cont() {
                break;
            }
            if c.left() == 0 {
                // An empty CONTINUE between two strings is an empty
                // string of its own for calamine.
                total += 24;
                out.push(0);
                continue;
            }
        }
        // read_rich_extended_string
        if c.left() < 3 {
            return Err(damaged());
        }
        let cch = u16::from_le_bytes([c.byte(0).unwrap_or(0), c.byte(1).unwrap_or(0)]) as usize;
        let flags = c.byte(2).unwrap_or(0);
        c.at += 3;
        let mut hb = flags & 1 != 0;
        let mut c_run = 0usize;
        let mut ext = 0usize;
        if flags & 0x8 != 0 {
            let (Some(a), Some(b)) = (c.byte(0), c.byte(1)) else {
                return Err(damaged());
            };
            c_run = u16::from_le_bytes([a, b]) as usize;
            c.at += 2;
        }
        if flags & 0x4 != 0 {
            let v = (0..4).map(|i| c.byte(i)).collect::<Option<Vec<u8>>>();
            let Some(v) = v else {
                return Err(damaged());
            };
            ext = i32::from_le_bytes([v[0], v[1], v[2], v[3]]) as usize;
            c.at += 4;
        }
        let mut est = 0u64;
        let mut left = cch;
        while left > 0 {
            let per = if hb { 2 } else { 1 };
            let n = (c.left() / per).min(left);
            est += n as u64 * enc.per(hb);
            c.at += n * per;
            left -= n;
            if left > 0 {
                if c.next_cont() && c.left() > 0 {
                    hb = c.byte(0).unwrap_or(0) & 1 != 0;
                    c.at += 1;
                } else {
                    return Err(damaged());
                }
            }
        }
        // The capacity is cch bytes whatever was decoded.
        total += 24 + est.max(cch as u64);
        out.push(est.min(u32::MAX as u64) as u32);
        if !c.skip(c_run.saturating_mul(4)) || !c.skip(ext) {
            return Err(damaged());
        }
    }
    Ok((total, out))
}

#[derive(Default)]
struct BBox {
    any: bool,
    r0: u32,
    r1: u32,
    c0: u32,
    c1: u32,
}

impl BBox {
    fn add(&mut self, r: u32, c: u32) {
        if !self.any {
            *self = BBox {
                any: true,
                r0: r,
                r1: r,
                c0: c,
                c1: c,
            };
        } else {
            self.r0 = self.r0.min(r);
            self.r1 = self.r1.max(r);
            self.c0 = self.c0.min(c);
            self.c1 = self.c1.max(c);
        }
    }
    fn area(&self) -> u64 {
        if !self.any {
            return 0;
        }
        (self.r1 - self.r0 + 1) as u64 * (self.c1 - self.c0 + 1) as u64
    }
}

/// The counters of a scan.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Scan {
    /// Sheets (BOUNDSHEET records).
    pub sheets: usize,
    /// Cell records.
    pub cells: u64,
    /// Dense cells of all value and formula ranges.
    pub dense: u64,
    /// Text bytes bound.
    pub text: u64,
    /// Formula decoder work bound.
    pub formula_work: u64,
    /// `NAME` records whose name or formula was cut short: they keep
    /// their place in the name table, with an empty formula.
    pub damaged_names: u64,
    /// Cell formulas calamine cannot decode without panicking (cut inside
    /// a token, or a value its arithmetic overflows on): their expression
    /// was emptied, the cells keep their cached values.
    pub damaged_formulas: u64,
    /// Optional records too short for their fixed fields, hidden.
    pub hidden_records: u64,
}

impl Scan {
    /// What the pre-scan left out of the file, for the user. (Formulas
    /// are reported by the reader, which counts every formula that is
    /// imported as its value.)
    pub fn warnings(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.damaged_names > 0 {
            out.push(format!(
                "{} defined name(s) in this file are damaged and were left out.",
                self.damaged_names
            ));
        }
        if self.hidden_records > 0 {
            out.push(format!(
                "{} damaged record(s) in this file were skipped.",
                self.hidden_records
            ));
        }
        out
    }
}

struct Budgets {
    max_sheets: usize,
    max_cells: u64,
}

fn over(what: &'static str, limit: u64) -> Refusal {
    Refusal::Budget { what, limit }
}

/// What the scan of one cell formula found.
struct Formula {
    /// Upper bound of the text calamine's decoder emits.
    emit: u64,
    /// Tokens.
    tokens: u64,
    /// `false` if calamine would index past the end of the expression or
    /// overflow (a panic, at least in a build with overflow checks).
    sound: bool,
}

/// Walks a cell formula token by token exactly as calamine 0.38's
/// `parse_formula` does and patches `PtgAttrSpace` counts to 0. Where
/// calamine returns an error the walk stops (calamine reads no further);
/// where it would panic the formula is reported as not sound.
fn scan_formula(
    rgce: &mut [u8],
    pre_biff8: bool,
    enc: Enc,
    name_max: u64,
    sheet_max: u64,
) -> Formula {
    let n = rgce.len();
    let mut f = Formula {
        emit: 0,
        tokens: 0,
        sound: true,
    };
    let mut i = 0usize;
    while i < n {
        let ptg = rgce[i];
        i += 1;
        f.tokens += 1;
        let left = n - i;
        // (text emitted, operand bytes calamine indexes without a check)
        let (emit, size): (u64, usize) = match ptg {
            0x3a | 0x5a | 0x7a => {
                // calamine prints `row + 1` computed in 16 bits.
                if u16_at(rgce, i + 2) == Some(0xFFFF) {
                    f.sound = false;
                }
                (sheet_max + 20, 6)
            }
            0x3c | 0x5c | 0x7c => (sheet_max + 20, 6),
            0x3b | 0x5b | 0x7b | 0x3d | 0x5d | 0x7d => (sheet_max + 30, 10),
            0x01 => {
                if left < 4 {
                    break;
                }
                (0, 4)
            }
            0x03..=0x15 => (2, 0),
            // A missing argument emits nothing itself, but a function
            // call adds a comma per argument.
            0x16 => (1, 0),
            0x17 => {
                // BIFF8: cch, flags, characters. Before BIFF8 (calamine
                // 0.38): cch, bytes. A short string is an error.
                let Some(&cch) = rgce.get(i) else { break };
                let cch = cch as usize;
                let (per, size) = if pre_biff8 {
                    (enc.max(true), 1 + cch)
                } else {
                    let Some(&flags) = rgce.get(i + 1) else { break };
                    let hb = flags & 1 != 0;
                    (enc.per(hb), 2 + if hb { 2 * cch } else { cch })
                };
                if left < size {
                    break;
                }
                (2 + cch as u64 * per, size)
            }
            0x18 => (0, 5),
            0x19 => {
                let Some(&etpg) = rgce.get(i) else {
                    f.sound = false;
                    break;
                };
                match etpg {
                    0x01 | 0x02 | 0x08 | 0x20 | 0x21 => (0, 3),
                    0x04 => {
                        let Some(k) = u16_at(rgce, i + 1) else {
                            f.sound = false;
                            break;
                        };
                        (0, 3 + 2 * (k as usize + 1))
                    }
                    0x10 => (5, 3),
                    0x40 | 0x41 => {
                        if left < 3 {
                            f.sound = false;
                            break;
                        }
                        rgce[i + 2] = 0;
                        (0, 3)
                    }
                    _ => break,
                }
            }
            0x1C => (14, 1),
            0x1D => (5, 1),
            0x1E => (5, 2),
            0x1F => (340, 8),
            0x20 | 0x40 | 0x60 => (10, 7),
            0x21 | 0x41 | 0x61 => match u16_at(rgce, i) {
                // Not in calamine's function table: an error.
                Some(iftab) if iftab >= 485 => break,
                _ => (40, 2),
            },
            0x22 | 0x42 | 0x62 => (40 + 256, 3),
            0x23 | 0x43 | 0x63 => {
                // calamine computes `index - 1` unsigned.
                if u32_at(rgce, i) == Some(0) {
                    f.sound = false;
                }
                (name_max.max(5), 4)
            }
            0x24 | 0x44 | 0x64 => {
                // BIFF8: `row + 1` computed in 16 bits.
                if !pre_biff8 && u16_at(rgce, i) == Some(0xFFFF) {
                    f.sound = false;
                }
                (14, if pre_biff8 { 3 } else { 4 })
            }
            0x25 | 0x45 | 0x65 => (26, if pre_biff8 { 6 } else { 8 }),
            0x2A | 0x4A | 0x6A => (5, if pre_biff8 { 3 } else { 4 }),
            0x2B | 0x4B | 0x6B => (5, if pre_biff8 { 6 } else { 8 }),
            0x39 | 0x59 => (10, 6),
            // calamine fails the formula.
            _ => break,
        };
        if left < size {
            f.sound = false;
            break;
        }
        f.emit += emit;
        i += size;
    }
    f
}

/// Whether a `BOF` with version `v` and substream type `dt` opens a
/// BIFF2–BIFF5 stream (calamine's `parse_bof`).
pub(crate) fn bof_is_pre_biff8(v: u16, dt: u16) -> bool {
    match v {
        0x0200 | 0x0002 | 0x0007 | 0x0300 | 0x0400 | 0x0500 => true,
        0 => dt == 0x1000,
        _ => false,
    }
}

/// Overwrites the type of the record whose data starts at `data`.
fn hide(s: &mut [u8], data: usize) {
    s[data - 4..data - 2].copy_from_slice(&HIDDEN_RECORD.to_le_bytes());
}

/// Checks a `NAME` record (data at `d`, `len` bytes) against what
/// calamine's `parse_lbl` and `parse_defined_names` index. A name or a
/// formula that is cut short is repaired in place: the name is cut to the
/// characters present and the formula length set to 0, so the record
/// keeps its index in the name table (formulas refer to names by index)
/// and no reader takes a formula from it. Returns the name's length in
/// characters and whether the record was repaired.
fn scan_name(s: &mut [u8], d: usize, len: usize, pre_biff8: bool) -> Result<(u64, bool), Refusal> {
    // The fixed part, and in BIFF8 the flags byte of the name.
    if len < if pre_biff8 { 14 } else { 15 } {
        return Err(bad("a NAME record is too short"));
    }
    let mut repaired = false;
    let mut cch = s[d + 3] as usize;
    if !pre_biff8 {
        let per = if s[d + 14] & 1 != 0 { 2 } else { 1 };
        let fit = (len - 15) / per;
        if cch > fit {
            cch = fit;
            s[d + 3] = fit as u8;
            repaired = true;
        }
    }
    // calamine takes the last `cce` bytes of the record as the formula
    // and reads the operands of a leading 3-D reference unchecked.
    let cce = u16_at(s, d + 4).unwrap_or(0) as usize;
    let fits = cce <= len && {
        let rgce = &s[d + len - cce..d + len];
        let need = match rgce.first() {
            None => 0,
            Some(0x3a | 0x5a | 0x7a) => {
                if pre_biff8 {
                    18
                } else {
                    7
                }
            }
            Some(0x3b | 0x5b | 0x7b) => {
                if pre_biff8 {
                    21
                } else {
                    11
                }
            }
            Some(0x3c | 0x5c | 0x7c | 0x3d | 0x5d | 0x7d) => {
                if pre_biff8 {
                    13
                } else {
                    3
                }
            }
            Some(_) => 1,
        };
        rgce.len() >= need
    };
    if !fits || repaired {
        s[d + 4..d + 6].copy_from_slice(&[0, 0]);
        repaired = true;
    }
    Ok((cch as u64, repaired))
}

/// calamine's date-format detection counts the `[` of a `FORMAT` code in
/// a `u8`. Cuts the string (declared length at `at`, `len` bytes of
/// record data from there) before the 256th `[`; no format code of Excel
/// has that many.
fn clamp_brackets(s: &mut [u8], at: usize, len: usize, pre_biff8: bool, enc: Enc) {
    let head = if pre_biff8 { 2 } else { 3 };
    if len < head {
        return;
    }
    let cch = u16_at(s, at).unwrap_or(0) as usize;
    let hb = !pre_biff8 && s[at + 2] & 1 != 0;
    let per = if hb { 2 } else { 1 };
    let chars = cch.min((len - head) / per);
    let mut open = 0usize;
    for k in 0..chars {
        let o = at + head + k * per;
        open += if hb && enc.utf16 {
            usize::from(s[o] == 0x5B && s[o + 1] == 0)
        } else if hb {
            // Another code page decodes the two bytes one by one.
            usize::from(s[o] == 0x5B) + usize::from(s[o + 1] == 0x5B)
        } else {
            usize::from(s[o] == 0x5B)
        };
        if open > 255 {
            s[at..at + 2].copy_from_slice(&(k as u16).to_le_bytes());
            return;
        }
    }
}

/// Scans (and patches, see the module docs) the workbook stream.
fn scan_stream(s: &mut [u8], b: &Budgets) -> Result<Scan, Refusal> {
    let mut scan = Scan::default();
    let mut sheet_pos: Vec<usize> = Vec::new();
    let mut sst: Vec<u32> = Vec::new();
    let mut pre_biff8 = false;
    let mut enc = Enc { utf16: true };
    let mut name_max = 0u64;
    let mut sheet_max = 0u64;
    let mut pos = 0usize;
    let too_many = || over("CONTINUE records of one record", MAX_CONTINUES as u64);
    // Globals.
    while let Some(r) = frame(s, pos) {
        let r = match r {
            Ok(r) => r,
            Err(FrameError::Continues) => return Err(too_many()),
            // calamine fails the workbook.
            Err(FrameError::Short) => break,
        };
        pos = r.next;
        let d = r.data;
        match r.typ {
            0x0809 => {
                // calamine's parse_bof.
                if r.len < 2 {
                    return Err(bad("a BOF record is too short"));
                }
                let v = u16_at(s, d).unwrap_or(0);
                let dt = if r.len >= 4 {
                    u16_at(s, d + 2).unwrap_or(0)
                } else {
                    0
                };
                pre_biff8 = bof_is_pre_biff8(v, dt);
            }
            // calamine reads these three unchecked.
            0x002F if r.len < 2 => return Err(bad("a FILEPASS record is too short")),
            0x0042 => {
                if r.len < 2 {
                    hide(s, d);
                    scan.hidden_records += 1;
                } else {
                    // 1200 is UTF-16LE; an unknown code page fails the
                    // workbook in calamine.
                    enc.utf16 = u16_at(s, d) == Some(1200);
                }
            }
            0x0022 if r.len < 2 => {
                hide(s, d);
                scan.hidden_records += 1;
            }
            // EXTERNSHEET: the BIFF8 one starts with a count.
            0x0017 if !pre_biff8 && r.len < 2 => {
                hide(s, d);
                scan.hidden_records += 1;
            }
            0x0085 => {
                // lbPlyPos, hsState and dt are read unchecked; a sheet
                // cannot be left out.
                if r.len < 6 {
                    return Err(bad("a BOUNDSHEET record is too short"));
                }
                scan.sheets += 1;
                if scan.sheets > b.max_sheets {
                    return Err(over("sheets", b.max_sheets as u64));
                }
                sheet_pos.push(u32_at(s, d).unwrap_or(u32::MAX) as usize);
                let cch = if r.len > 6 { s[d + 6] as u64 } else { 0 };
                sheet_max = sheet_max.max(cch * enc.max(pre_biff8));
                scan.text += 24 + cch * enc.max(pre_biff8);
            }
            0x0018 => {
                let (cch, repaired) = scan_name(s, d, r.len, pre_biff8)?;
                scan.damaged_names += u64::from(repaired);
                name_max = name_max.max(cch * enc.max(pre_biff8));
                scan.text += 48 + cch * enc.max(pre_biff8) + r.len as u64 * 40;
            }
            0x041E if r.len >= 2 => {
                clamp_string(s, d + 2, r.len - 2, pre_biff8);
                clamp_brackets(s, d + 2, r.len - 2, pre_biff8, enc);
            }
            0x00FC => {
                if r.cont.len() > MAX_SST_CONTINUES {
                    return Err(over(
                        "CONTINUE records of the shared strings",
                        MAX_SST_CONTINUES as u64,
                    ));
                }
                let (t, v) = scan_sst(s, &r, enc)?;
                scan.text += t;
                sst = v;
            }
            0x000A => break,
            _ => {}
        }
        if scan.text > MAX_TEXT_ESTIMATE {
            return Err(over("text bytes", MAX_TEXT_ESTIMATE));
        }
    }
    // Worksheets, walked from each BOUNDSHEET offset like calamine.
    let walk_budget = 2 * s.len() as u64 + (1 << 20);
    let mut walked = 0u64;
    for &start in &sheet_pos {
        if start > s.len() {
            // calamine slices the stream there.
            return Err(bad("a sheet starts past the end of the workbook stream"));
        }
        let mut vals = BBox::default();
        let mut fmls = BBox::default();
        let mut fmla_pos = (0u32, 0u32);
        let mut seen_bof = false;
        let mut depth = 0usize;
        let mut pos = start;
        while let Some(r) = frame(s, pos) {
            let r = match r {
                Ok(r) => r,
                Err(FrameError::Continues) => return Err(too_many()),
                Err(FrameError::Short) => break,
            };
            walked += (r.next - pos) as u64;
            if walked > walk_budget {
                return Err(over("worksheet stream bytes walked", walk_budget));
            }
            pos = r.next;
            if depth > 0 {
                match r.typ {
                    0x0809 => depth += 1,
                    0x000A => depth -= 1,
                    _ => {}
                }
                continue;
            }
            let d = r.data;
            let len = r.len;
            let rc = |s: &[u8]| {
                (
                    u16_at(s, d).unwrap_or(0) as u32,
                    u16_at(s, d + 2).unwrap_or(0) as u32,
                )
            };
            let cell = |scan: &mut Scan, bb: &mut BBox, r: u32, c: u32| -> Result<(), Refusal> {
                scan.cells += 1;
                if scan.cells > b.max_cells {
                    return Err(over("cells", b.max_cells));
                }
                bb.add(r, c);
                Ok(())
            };
            match r.typ {
                0x0809 => {
                    if seen_bof {
                        depth += 1;
                    } else {
                        seen_bof = true;
                    }
                }
                // Only used for a reserve, computed with unchecked
                // subtractions, and a length calamine does not expect
                // fails the workbook: calamine never sees it.
                0x0200 => hide(s, d),
                0x0203 | 0x0205 | 0x027E if len >= 4 => {
                    let (r, c) = rc(s);
                    cell(&mut scan, &mut vals, r, c)?;
                }
                0x0204 | 0x00D6 if len >= 6 => {
                    let (r, c) = rc(s);
                    cell(&mut scan, &mut vals, r, c)?;
                    let n = clamp_string(s, d + 6, len - 6, pre_biff8);
                    scan.text += 24 + n * enc.max(pre_biff8);
                }
                0x0207 => {
                    cell(&mut scan, &mut vals, fmla_pos.0, fmla_pos.1)?;
                    let n = clamp_string(s, d, len, pre_biff8);
                    scan.text += 24 + n * enc.max(pre_biff8);
                }
                0x00FD if len >= 10 => {
                    // Counted whether or not the string exists: the
                    // bounding box must never be smaller than calamine's.
                    let (r, c) = rc(s);
                    cell(&mut scan, &mut vals, r, c)?;
                    let i = u32_at(s, d + 6).unwrap_or(u32::MAX) as usize;
                    scan.text += sst.get(i).map_or(0, |t| u64::from(*t));
                }
                0x00BD if len >= 6 => {
                    // calamine computes `last - first + 1` in 16 bits and
                    // fails the workbook unless it matches the length.
                    let (r, c0) = rc(s);
                    let last = u16_at(s, d + len - 2).unwrap_or(0) as u32;
                    if last < c0 || 6 + 6 * (last - c0 + 1) as usize != len {
                        return Err(bad("a MULRK record contradicts its length"));
                    }
                    for c in c0..=last {
                        cell(&mut scan, &mut vals, r, c)?;
                    }
                }
                0x00E5 => {
                    // The count is trusted over the bytes present.
                    if len < 2 {
                        hide(s, d);
                        scan.hidden_records += 1;
                    } else {
                        let fit = ((len - 2) / 8) as u16;
                        if u16_at(s, d).unwrap_or(0) > fit {
                            s[d..d + 2].copy_from_slice(&fit.to_le_bytes());
                        }
                    }
                }
                0x0006 if len >= 20 => {
                    if len < 22 {
                        // No room for the expression's length, which
                        // calamine reads unchecked.
                        return Err(bad("a FORMULA record is too short"));
                    }
                    let (r, c) = rc(s);
                    fmla_pos = (r, c);
                    cell(&mut scan, &mut vals, r, c)?;
                    fmls.add(r, c);
                    let cce = u16_at(s, d + 20).unwrap_or(0) as usize;
                    let f = if cce > len - 22 {
                        None
                    } else {
                        let f = scan_formula(
                            &mut s[d + 22..d + 22 + cce],
                            pre_biff8,
                            enc,
                            name_max,
                            sheet_max,
                        );
                        f.sound.then_some(f)
                    };
                    // The formula text, or calamine's error text.
                    let (emit, tokens) = match f {
                        Some(f) => (f.emit + 160, f.tokens.max(1)),
                        None => {
                            // An empty expression is an error for
                            // calamine, not a panic.
                            s[d + 20..d + 22].copy_from_slice(&[0, 0]);
                            scan.damaged_formulas += 1;
                            (160, 1)
                        }
                    };
                    scan.text += 24 + emit;
                    scan.formula_work = scan.formula_work.saturating_add(tokens * emit);
                    if scan.formula_work > MAX_FORMULA_WORK {
                        return Err(over("formula decoding work", MAX_FORMULA_WORK));
                    }
                }
                0x000A => break,
                _ => {}
            }
            if scan.text > MAX_TEXT_ESTIMATE {
                return Err(over("text bytes", MAX_TEXT_ESTIMATE));
            }
        }
        scan.dense += vals.area() + fmls.area();
        if scan.dense > MAX_DENSE_CELLS {
            return Err(over("dense sheet area (cells)", MAX_DENSE_CELLS));
        }
    }
    Ok(scan)
}

/// Clamps an `XLUnicodeString`'s declared length at `at` (record data of
/// `len` bytes) to the bytes present; returns the clamped length.
fn clamp_string(s: &mut [u8], at: usize, len: usize, pre_biff8: bool) -> u64 {
    let expected = if pre_biff8 { 2 } else { 3 };
    if len < expected {
        return 0;
    }
    let Some(cch) = u16_at(s, at) else { return 0 };
    let avail = (len - expected).min(u16::MAX as usize) as u16;
    let n = cch.min(avail);
    s[at..at + 2].copy_from_slice(&n.to_le_bytes());
    n as u64
}

/// Validates `file`, then returns a canonical compound file for calamine
/// (see the module docs) and the scan counters.
pub fn sanitize(
    file: &[u8],
    max_sheets: usize,
    max_cells: u64,
) -> Result<(Vec<u8>, Scan), Refusal> {
    let (out, _, scan) = sanitize_with_offset(file, max_sheets, max_cells)?;
    Ok((out, scan))
}

/// [`sanitize`], also returning the offset of the (patched) workbook
/// stream in the canonical file; the stream runs to the end of the file
/// (zero padded). The formatting reader walks it.
pub(crate) fn sanitize_with_offset(
    file: &[u8],
    max_sheets: usize,
    max_cells: u64,
) -> Result<(Vec<u8>, usize, Scan), Refusal> {
    let stream = extract_workbook_stream(file)?;
    // What calamine reads back: the stream, padded to 4,096 bytes (not
    // the zero padding of the last sector behind it).
    let len = stream.len().max(4096);
    let (mut out, off) = build_container(stream);
    let scan = scan_stream(
        &mut out[off..off + len],
        &Budgets {
            max_sheets,
            max_cells,
        },
    )?;
    Ok((out, off, scan))
}
