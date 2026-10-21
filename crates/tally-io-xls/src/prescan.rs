//! Tally-side validation of an `.xls` file before calamine sees it
//! (security review 4, C4).
//!
//! calamine 0.37 allocates from sizes it reads in the file (sector chains,
//! `DIMENSIONS` reservations, string capacities, the bounding box of the
//! cell coordinates) before any of Tally's budgets run, and an allocation
//! failure aborts the process (`catch_unwind` does not catch it). This
//! module therefore:
//!
//! 1. Extracts the workbook stream itself from the OLE2 container with
//!    strict chain walks (visited set, every sector inside the file, no
//!    FAT sector listed twice, stream length ≤ the chain's length ≤
//!    [`MAX_STREAM_BYTES`]) and rebuilds a minimal, canonical compound
//!    file around it, so calamine's container code only ever sees a file
//!    Tally wrote.
//! 2. Walks the BIFF records the way calamine's `RecordIter` does and
//!    neutralizes the fields calamine allocates from without a bound:
//!    `DIMENSIONS` records are zeroed (calamine only uses them for a
//!    `Vec::reserve`), declared string lengths of `LABEL`/`RSTRING`/
//!    `STRING`/`FORMAT` records are clamped to the bytes actually present
//!    (the decoded text is unchanged), and `PtgAttrSpace` counts in cell
//!    formulas are set to 0 (cosmetic spaces that calamine inserts one by
//!    one, quadratically).
//! 3. Refuses a file whose remaining calamine-side cost exceeds a budget:
//!    the dense area of every sheet's value and formula bounding boxes
//!    (calamine materializes `Range`s densely, all sheets at once), the
//!    number of cell records, an upper bound on the text calamine builds
//!    (shared strings, their copies per `LABELSST`, labels, decoded
//!    formulas) and the formula decoder's quadratic work.
//!
//! Residual risk: this models calamine 0.37's decoder; it is pinned
//! exactly (`=0.37.0`) and must be re-reviewed on upgrade.

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

fn u16_at(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes([*b.get(i)?, *b.get(i + 1)?]))
}

fn u32_at(b: &[u8], i: usize) -> Option<u32> {
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

/// Extracts the workbook stream (`Workbook`, `Book`, `WORKBOOK`, `BOOK`,
/// in calamine's order).
pub fn extract_workbook_stream(file: &[u8]) -> Result<Vec<u8>, Refusal> {
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
    for name in ["Workbook", "Book", "WORKBOOK", "BOOK"] {
        let Some(d) = dirs.iter().find(|d| d.name == name) else {
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
struct Rec {
    typ: u16,
    /// Offset of the record data in the stream.
    data: usize,
    len: usize,
    /// (offset, len) of each following CONTINUE record's data.
    cont: Vec<(usize, usize)>,
    /// Offset after the record and its continues.
    next: usize,
}

/// Frames the record at `pos`; `None` at the end, `Err` on a short stream
/// (calamine fails the workbook then).
fn record(s: &[u8], pos: usize) -> Option<Result<Rec, ()>> {
    let rest = s.len().saturating_sub(pos);
    if rest == 0 {
        return None;
    }
    if rest < 4 {
        return Some(Err(()));
    }
    let typ = u16_at(s, pos).unwrap_or(0);
    let len = u16_at(s, pos + 2).unwrap_or(0) as usize;
    if rest < len + 4 {
        return Some(Err(()));
    }
    let data = pos + 4;
    let mut next = data + len;
    let mut cont = Vec::new();
    if s.len() - next > 4 && u16_at(s, next) == Some(0x003C) {
        while s.len() - next > 4 && u16_at(s, next) == Some(0x003C) {
            let l = u16_at(s, next + 2).unwrap_or(0) as usize;
            if s.len() - next < l + 4 {
                return Some(Err(()));
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

/// Bytes of text calamine builds for the shared strings, and each
/// string's decoded-length bound (for `LABELSST` copies).
fn scan_sst(s: &[u8], r: &Rec) -> (u64, Vec<u32>) {
    let mut out = Vec::new();
    let mut total = 0u64;
    if r.len < 8 {
        return (0, out);
    }
    let mut c = Cur {
        s,
        at: r.data + 8,
        end: r.data + r.len,
        cont: r.cont.iter(),
    };
    loop {
        if c.left() == 0 && !c.next_cont() {
            break;
        }
        // read_rich_extended_string
        if c.left() < 3 {
            break;
        }
        let cch = u16::from_le_bytes([c.byte(0).unwrap_or(0), c.byte(1).unwrap_or(0)]) as usize;
        let flags = c.byte(2).unwrap_or(0);
        c.at += 3;
        let mut hb = flags & 1 != 0;
        let mut c_run = 0usize;
        let mut ext = 0usize;
        if flags & 0x8 != 0 {
            let (Some(a), Some(b)) = (c.byte(0), c.byte(1)) else {
                break;
            };
            c_run = u16::from_le_bytes([a, b]) as usize;
            c.at += 2;
        }
        if flags & 0x4 != 0 {
            let v = (0..4).map(|i| c.byte(i)).collect::<Option<Vec<u8>>>();
            let Some(v) = v else { break };
            ext = i32::from_le_bytes([v[0], v[1], v[2], v[3]]) as usize;
            c.at += 4;
        }
        let mut est = 0u64;
        let mut left = cch;
        let mut ok = true;
        while left > 0 {
            let per = if hb { 2 } else { 1 };
            let n = (c.left() / per).min(left);
            est += n as u64 * if hb { 3 } else { 2 };
            c.at += n * per;
            left -= n;
            if left > 0 {
                if c.next_cont() && c.left() > 0 {
                    hb = c.byte(0).unwrap_or(0) & 1 != 0;
                    c.at += 1;
                } else {
                    ok = false;
                    break;
                }
            }
        }
        // The capacity is cch bytes whatever was decoded.
        total += 24 + est.max(cch as u64);
        if !ok {
            break;
        }
        out.push(est.min(u32::MAX as u64) as u32);
        if !c.skip(c_run.saturating_mul(4)) || !c.skip(ext) {
            break;
        }
    }
    (total, out)
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
}

struct Budgets {
    max_sheets: usize,
    max_cells: u64,
}

fn over(what: &'static str, limit: u64) -> Refusal {
    Refusal::Budget { what, limit }
}

/// Upper bound of the text calamine's formula decoder emits for one
/// formula, and its token count. Patches `PtgAttrSpace` counts to 0.
fn scan_formula(rgce: &mut [u8], pre_biff8: bool, name_max: u64, sheet_max: u64) -> (u64, u64) {
    let mut i = 0usize;
    let mut out = 0u64;
    let mut tokens = 0u64;
    let n = rgce.len();
    while i < n {
        let ptg = rgce[i];
        i += 1;
        tokens += 1;
        let (emit, size): (u64, usize) = match ptg {
            0x3a | 0x5a | 0x7a | 0x3c | 0x5c | 0x7c => (sheet_max + 20, 6),
            0x3b | 0x5b | 0x7b | 0x3d | 0x5d | 0x7d => (sheet_max + 30, 10),
            0x01 => (0, 4),
            0x03..=0x11 => (2, 0),
            0x12..=0x15 => (2, 0),
            0x16 => (0, 0),
            0x17 => {
                let Some(&cch) = rgce.get(i) else { break };
                let hb = rgce.get(i + 1).is_some_and(|f| f & 1 != 0);
                let bytes = if hb { 2 * cch as usize } else { cch as usize };
                (2 + 3 * cch as u64, 2 + bytes)
            }
            0x18 => (0, 5),
            0x19 => {
                let Some(&etpg) = rgce.get(i) else { break };
                match etpg {
                    0x01 | 0x02 | 0x08 | 0x20 | 0x21 => (0, 3),
                    0x04 => {
                        let k = u16_at(rgce, i + 1).unwrap_or(0) as usize + 1;
                        (0, 3 + 2 * k)
                    }
                    0x10 => (5, 3),
                    0x40 | 0x41 => {
                        if i + 2 < n {
                            rgce[i + 2] = 0;
                        }
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
            0x21 | 0x41 | 0x61 => (40, 2),
            0x22 | 0x42 | 0x62 => (40 + 256, 3),
            0x23 | 0x43 | 0x63 => (name_max.max(5), 4),
            0x24 | 0x44 | 0x64 => (14, if pre_biff8 { 3 } else { 4 }),
            0x25 | 0x45 | 0x65 => (26, if pre_biff8 { 6 } else { 8 }),
            0x2A | 0x4A | 0x6A => (5, if pre_biff8 { 3 } else { 4 }),
            0x2B | 0x4B | 0x6B => (5, if pre_biff8 { 6 } else { 8 }),
            0x39 | 0x59 => (10, 6),
            // calamine fails the formula: a short error text.
            _ => {
                out += 200;
                break;
            }
        };
        out += emit;
        i += size;
    }
    (out, tokens)
}

/// Scans (and patches, see the module docs) the workbook stream.
fn scan_stream(s: &mut [u8], b: &Budgets) -> Result<Scan, Refusal> {
    let mut scan = Scan::default();
    let mut sheet_pos: Vec<usize> = Vec::new();
    let mut sst: Vec<u32> = Vec::new();
    let mut pre_biff8 = false;
    let mut name_max = 0u64;
    let mut sheet_max = 0u64;
    let mut pos = 0usize;
    // Globals.
    while let Some(r) = record(s, pos) {
        let Ok(r) = r else { break };
        pos = r.next;
        let d = r.data;
        match r.typ {
            0x0809 => {
                // calamine's parse_bof.
                let v = u16_at(s, d).unwrap_or(0);
                let dt = if r.len >= 4 {
                    u16_at(s, d + 2).unwrap_or(0)
                } else {
                    0
                };
                pre_biff8 = match v {
                    0x0200 | 0x0002 | 0x0007 | 0x0300 | 0x0400 | 0x0500 => true,
                    0 => dt == 0x1000,
                    _ => false,
                };
            }
            0x0085 => {
                scan.sheets += 1;
                if scan.sheets > b.max_sheets {
                    return Err(over("sheets", b.max_sheets as u64));
                }
                sheet_pos.push(u32_at(s, d).unwrap_or(u32::MAX) as usize);
                let cch = s.get(d + 6).copied().unwrap_or(0) as u64;
                sheet_max = sheet_max.max(cch * 3);
                scan.text += 24 + cch * 3;
            }
            0x0018 => {
                let cch = s.get(d + 3).copied().unwrap_or(0) as u64;
                name_max = name_max.max(cch * 3);
                scan.text += 48 + cch * 3 + r.len as u64 * 40;
            }
            0x041E => {
                clamp_string(s, d + 2, r.len.saturating_sub(2), pre_biff8);
            }
            0x00FC => {
                let (t, v) = scan_sst(s, &r);
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
            // calamine panics on the slice: the workbook is refused.
            break;
        }
        let mut vals = BBox::default();
        let mut fmls = BBox::default();
        let mut fmla_pos = (0u32, 0u32);
        let mut seen_bof = false;
        let mut depth = 0usize;
        let mut pos = start;
        while let Some(r) = record(s, pos) {
            let Ok(r) = r else { break };
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
                0x0200 => {
                    // Only used for a reserve: make it empty.
                    for x in &mut s[d..d + len] {
                        *x = 0;
                    }
                }
                0x0203 | 0x0205 | 0x027E if len >= 4 => {
                    let (r, c) = rc(s);
                    cell(&mut scan, &mut vals, r, c)?;
                }
                0x0204 | 0x00D6 if len >= 6 => {
                    let (r, c) = rc(s);
                    cell(&mut scan, &mut vals, r, c)?;
                    let n = clamp_string(s, d + 6, len - 6, pre_biff8);
                    scan.text += 24 + 3 * n;
                }
                0x0207 => {
                    cell(&mut scan, &mut vals, fmla_pos.0, fmla_pos.1)?;
                    let n = clamp_string(s, d, len, pre_biff8);
                    scan.text += 24 + 3 * n;
                }
                0x00FD if len >= 10 => {
                    let (r, c) = rc(s);
                    let i = u32_at(s, d + 6).unwrap_or(u32::MAX) as usize;
                    if let Some(&t) = sst.get(i) {
                        cell(&mut scan, &mut vals, r, c)?;
                        scan.text += t as u64;
                    }
                }
                0x00BD if len >= 6 => {
                    let (r, c0) = rc(s);
                    let k = (len - 6) / 6;
                    for j in 0..k as u32 {
                        cell(&mut scan, &mut vals, r, c0 + j)?;
                    }
                }
                0x0006 if len >= 20 => {
                    let (r, c) = rc(s);
                    fmla_pos = (r, c);
                    cell(&mut scan, &mut vals, r, c)?;
                    fmls.add(r, c);
                    let cce = u16_at(s, d + 20).unwrap_or(0) as usize;
                    let lo = (d + 22).min(d + len);
                    let hi = (d + 22 + cce).min(d + len);
                    let (emit, tokens) =
                        scan_formula(&mut s[lo..hi], pre_biff8, name_max, sheet_max);
                    let emit = emit + 64;
                    scan.text += 24 + emit;
                    scan.formula_work = scan.formula_work.saturating_add(tokens.max(1) * emit);
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
    let stream = extract_workbook_stream(file)?;
    let (mut out, off) = build_container(stream);
    let scan = scan_stream(
        &mut out[off..],
        &Budgets {
            max_sheets,
            max_cells,
        },
    )?;
    Ok((out, scan))
}
