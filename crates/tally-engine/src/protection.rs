//! Sheet, cell and document-structure protection.
//!
//! Passwords are stored as hashes in the formats the files use (ODF:
//! base64 SHA-256/SHA-1 of the UTF-8 password; OOXML: the legacy 16-bit
//! hash, or `algorithmName` + salt + spin count). This is compatibility,
//! not security: the hashes are weak and the file is not encrypted.
//! Unknown algorithms cannot be verified and accept any password.

use tally_api::{ApiError, Direction, ProtectionInfo, ProtectionOptions, RangeDto};
use tally_model::{hash_algorithms, CellProtectionData, SheetProtectionData};
use tally_types::{CellRange, SheetId, MAX_COLS, MAX_ROWS};

use crate::storage::StyleId;
use crate::workbook::Workbook;

/// Error code of edits refused by protection.
pub const PROTECTED: &str = "protected";

/// Most hash rounds honoured for an OOXML `spinCount` (Excel's default;
/// a file asking for more is clamped, so one unlock attempt stays
/// around 30 ms; security review 3, MEDIUM-5).
pub const MAX_SPIN_COUNT: u32 = 100_000;
/// Longest salt or hash value (base64) kept from a file; longer ones are
/// dropped at load (the sheet stays protected, any password unlocks it,
/// as for an unknown algorithm).
pub const MAX_HASH_LEN: usize = 1024;

/// The error for an edit of a protected cell.
pub fn protected_error() -> ApiError {
    ApiError::new(
        PROTECTED,
        "The cell or chart you're trying to change is protected. Unprotect the sheet first.",
    )
}

fn structure_error() -> ApiError {
    ApiError::new(
        PROTECTED,
        "The workbook structure is protected; sheets cannot be added, deleted, moved or renamed.",
    )
}

// ---------------------------------------------------------------------------
// Hashes
// ---------------------------------------------------------------------------

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with padding.
pub fn base64(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            B64[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            B64[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// Decodes base64 (whitespace ignored); `None` if malformed.
pub fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut acc = 0u32;
    let mut bits = 0;
    for ch in s.bytes() {
        if ch.is_ascii_whitespace() || ch == b'=' {
            continue;
        }
        let v = B64.iter().position(|&c| c == ch)? as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

/// SHA-1.
pub fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [
        0x6745_2301,
        0xEFCD_AB89,
        0x98BA_DCFE,
        0x1032_5476,
        0xC3D2_E1F0,
    ];
    for block in md_pad(data, 64, false).chunks(64) {
        let mut w = [0u32; 80];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                block[4 * i],
                block[4 * i + 1],
                block[4 * i + 2],
                block[4 * i + 3],
            ]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A82_7999),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6),
            };
            let t = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = t;
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e]) {
            *x = x.wrapping_add(y);
        }
    }
    let mut out = [0u8; 20];
    for (i, x) in h.iter().enumerate() {
        out[4 * i..4 * i + 4].copy_from_slice(&x.to_be_bytes());
    }
    out
}

/// Merkle–Damgård padding with a 64-bit (or 128-bit for `wide`) length.
fn md_pad(data: &[u8], block: usize, wide: bool) -> Vec<u8> {
    let len_bytes = if wide { 16 } else { 8 };
    let mut m = data.to_vec();
    m.push(0x80);
    while !(m.len() + len_bytes).is_multiple_of(block) {
        m.push(0);
    }
    let bits = (data.len() as u128) * 8;
    if wide {
        m.extend_from_slice(&bits.to_be_bytes());
    } else {
        m.extend_from_slice(&(bits as u64).to_be_bytes());
    }
    m
}

const K256: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// SHA-256.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    for block in md_pad(data, 64, false).chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                block[4 * i],
                block[4 * i + 1],
                block[4 * i + 2],
                block[4 * i + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let mut v = h;
        for i in 0..64 {
            let [a, b, c, d, e, f, g, hh] = v;
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K256[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            v = [t1.wrapping_add(t2), a, b, c, d.wrapping_add(t1), e, f, g];
        }
        for (x, y) in h.iter_mut().zip(v) {
            *x = x.wrapping_add(y);
        }
    }
    let mut out = [0u8; 32];
    for (i, x) in h.iter().enumerate() {
        out[4 * i..4 * i + 4].copy_from_slice(&x.to_be_bytes());
    }
    out
}

const K512: [u64; 80] = [
    0x428a2f98d728ae22,
    0x7137449123ef65cd,
    0xb5c0fbcfec4d3b2f,
    0xe9b5dba58189dbbc,
    0x3956c25bf348b538,
    0x59f111f1b605d019,
    0x923f82a4af194f9b,
    0xab1c5ed5da6d8118,
    0xd807aa98a3030242,
    0x12835b0145706fbe,
    0x243185be4ee4b28c,
    0x550c7dc3d5ffb4e2,
    0x72be5d74f27b896f,
    0x80deb1fe3b1696b1,
    0x9bdc06a725c71235,
    0xc19bf174cf692694,
    0xe49b69c19ef14ad2,
    0xefbe4786384f25e3,
    0x0fc19dc68b8cd5b5,
    0x240ca1cc77ac9c65,
    0x2de92c6f592b0275,
    0x4a7484aa6ea6e483,
    0x5cb0a9dcbd41fbd4,
    0x76f988da831153b5,
    0x983e5152ee66dfab,
    0xa831c66d2db43210,
    0xb00327c898fb213f,
    0xbf597fc7beef0ee4,
    0xc6e00bf33da88fc2,
    0xd5a79147930aa725,
    0x06ca6351e003826f,
    0x142929670a0e6e70,
    0x27b70a8546d22ffc,
    0x2e1b21385c26c926,
    0x4d2c6dfc5ac42aed,
    0x53380d139d95b3df,
    0x650a73548baf63de,
    0x766a0abb3c77b2a8,
    0x81c2c92e47edaee6,
    0x92722c851482353b,
    0xa2bfe8a14cf10364,
    0xa81a664bbc423001,
    0xc24b8b70d0f89791,
    0xc76c51a30654be30,
    0xd192e819d6ef5218,
    0xd69906245565a910,
    0xf40e35855771202a,
    0x106aa07032bbd1b8,
    0x19a4c116b8d2d0c8,
    0x1e376c085141ab53,
    0x2748774cdf8eeb99,
    0x34b0bcb5e19b48a8,
    0x391c0cb3c5c95a63,
    0x4ed8aa4ae3418acb,
    0x5b9cca4f7763e373,
    0x682e6ff3d6b2b8a3,
    0x748f82ee5defb2fc,
    0x78a5636f43172f60,
    0x84c87814a1f0ab72,
    0x8cc702081a6439ec,
    0x90befffa23631e28,
    0xa4506cebde82bde9,
    0xbef9a3f7b2c67915,
    0xc67178f2e372532b,
    0xca273eceea26619c,
    0xd186b8c721c0c207,
    0xeada7dd6cde0eb1e,
    0xf57d4f7fee6ed178,
    0x06f067aa72176fba,
    0x0a637dc5a2c898a6,
    0x113f9804bef90dae,
    0x1b710b35131c471b,
    0x28db77f523047d84,
    0x32caab7b40c72493,
    0x3c9ebe0a15c9bebc,
    0x431d67c49c100d4c,
    0x4cc5d4becb3e42b6,
    0x597f299cfc657e2a,
    0x5fcb6fab3ad6faec,
    0x6c44198c4a475817,
];

/// SHA-512.
pub fn sha512(data: &[u8]) -> [u8; 64] {
    let mut h: [u64; 8] = [
        0x6a09e667f3bcc908,
        0xbb67ae8584caa73b,
        0x3c6ef372fe94f82b,
        0xa54ff53a5f1d36f1,
        0x510e527fade682d1,
        0x9b05688c2b3e6c1f,
        0x1f83d9abfb41bd6b,
        0x5be0cd19137e2179,
    ];
    for block in md_pad(data, 128, true).chunks(128) {
        let mut w = [0u64; 80];
        for i in 0..16 {
            let mut b = [0u8; 8];
            b.copy_from_slice(&block[8 * i..8 * i + 8]);
            w[i] = u64::from_be_bytes(b);
        }
        for i in 16..80 {
            let s0 = w[i - 15].rotate_right(1) ^ w[i - 15].rotate_right(8) ^ (w[i - 15] >> 7);
            let s1 = w[i - 2].rotate_right(19) ^ w[i - 2].rotate_right(61) ^ (w[i - 2] >> 6);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let mut v = h;
        for i in 0..80 {
            let [a, b, c, d, e, f, g, hh] = v;
            let s1 = e.rotate_right(14) ^ e.rotate_right(18) ^ e.rotate_right(41);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K512[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(28) ^ a.rotate_right(34) ^ a.rotate_right(39);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            v = [t1.wrapping_add(t2), a, b, c, d.wrapping_add(t1), e, f, g];
        }
        for (x, y) in h.iter_mut().zip(v) {
            *x = x.wrapping_add(y);
        }
    }
    let mut out = [0u8; 64];
    for (i, x) in h.iter().enumerate() {
        out[8 * i..8 * i + 8].copy_from_slice(&x.to_be_bytes());
    }
    out
}

/// Excel's legacy 16-bit password hash (OOXML `password`, 4 hex digits).
pub fn xlsx_legacy_hash(password: &str) -> String {
    let bytes: Vec<u8> = password
        .chars()
        .map(|c| {
            let u = c as u32;
            if u < 0x100 {
                u as u8
            } else {
                b'?'
            }
        })
        .take(15)
        .collect();
    let mut hash: u16 = 0;
    for &b in bytes.iter().rev() {
        hash = ((hash >> 14) & 1) | ((hash << 1) & 0x7fff);
        hash ^= u16::from(b);
    }
    hash = ((hash >> 14) & 1) | ((hash << 1) & 0x7fff);
    hash ^= bytes.len() as u16;
    hash ^= 0xCE4B;
    format!("{hash:04X}")
}

/// OOXML agile hash: `H(salt + UTF-16LE password)`, then `spin` times
/// `H(previous + iteration as u32 LE)`. Only SHA-512, SHA-256 and SHA-1.
fn ooxml_hash(algorithm: &str, salt: &[u8], password: &str, spin: u32) -> Option<Vec<u8>> {
    let h = |d: &[u8]| -> Option<Vec<u8>> {
        Some(match algorithm.to_ascii_uppercase().as_str() {
            "SHA-512" | "SHA512" => sha512(d).to_vec(),
            "SHA-256" | "SHA256" => sha256(d).to_vec(),
            "SHA-1" | "SHA1" => sha1(d).to_vec(),
            _ => return None,
        })
    };
    let mut buf = salt.to_vec();
    for u in password.encode_utf16() {
        buf.extend_from_slice(&u.to_le_bytes());
    }
    let mut cur = h(&buf)?;
    for i in 0..spin.min(MAX_SPIN_COUNT) {
        let mut b = cur;
        b.extend_from_slice(&i.to_le_bytes());
        cur = h(&b)?;
    }
    Some(cur)
}

/// SHA-1 of the UTF-16LE password, as AOO's `SvPasswordHelper::GetHashPassword`
/// (`GetHashPasswordLittleEndian`) computes ODS protection keys.
pub fn sha1_utf16le(password: &str) -> [u8; 20] {
    let mut buf = Vec::with_capacity(password.len() * 2);
    for u in password.encode_utf16() {
        buf.extend_from_slice(&u.to_le_bytes());
    }
    sha1(&buf)
}

/// A protection record for `password`: the ODF 1.0/1.1 SHA-1 key (of the
/// UTF-16LE password) that Apache OpenOffice 4.1 and LibreOffice both
/// verify, plus Excel's legacy hash for XLSX (review 3, M8). SHA-256 keys
/// written by LibreOffice are still read and verified.
pub fn new_protection(password: Option<&str>, opts: &ProtectionOptions) -> SheetProtectionData {
    let (hash, alg) = match password.filter(|p| !p.is_empty()) {
        Some(p) => (
            Some(base64(&sha1_utf16le(p))),
            Some(hash_algorithms::SHA1.to_string()),
        ),
        None => (None, None),
    };
    let mut d = options_to_model(opts);
    d.password_hash = hash;
    d.algorithm = alg;
    d.xlsx_legacy_hash = password.filter(|p| !p.is_empty()).map(xlsx_legacy_hash);
    d
}

/// `true` if `password` matches the protection's hash (no hash: any
/// password; an algorithm we cannot compute: any password).
pub fn verify(p: &SheetProtectionData, password: Option<&str>) -> bool {
    let pw = password.unwrap_or("");
    let Some(hash) = p.password_hash.as_deref().filter(|h| !h.is_empty()) else {
        // Only the legacy hash survived (e.g. a sheet from an old file).
        return p
            .xlsx_legacy_hash
            .as_deref()
            .is_none_or(|h| xlsx_legacy_hash(pw).eq_ignore_ascii_case(h));
    };
    let alg = p.algorithm.as_deref().unwrap_or(hash_algorithms::SHA1);
    match alg {
        hash_algorithms::SHA256 | "http://www.w3.org/2000/09/xmldsig#sha256" => {
            base64(&sha256(pw.as_bytes())) == hash.trim()
        }
        hash_algorithms::SHA1 | "http://www.w3.org/2000/09/xmldsig#sha1" => {
            // AOO/LibreOffice hash the UTF-16LE password; older writers
            // (and Tally before v0.6) the UTF-8 bytes.
            let h = hash.trim();
            base64(&sha1_utf16le(pw)) == h || base64(&sha1(pw.as_bytes())) == h
        }
        hash_algorithms::XLSX_LEGACY => xlsx_legacy_hash(pw).eq_ignore_ascii_case(hash.trim()),
        other => {
            let salt = p
                .salt
                .as_deref()
                .and_then(base64_decode)
                .unwrap_or_default();
            match ooxml_hash(other, &salt, pw, p.spin_count.unwrap_or(0)) {
                Some(h) => base64(&h) == hash.trim(),
                None => true,
            }
        }
    }
}

/// API options → model flags (no password).
pub fn options_to_model(o: &ProtectionOptions) -> SheetProtectionData {
    SheetProtectionData {
        select_locked: o.select_locked,
        select_unlocked: o.select_unlocked,
        insert_rows: o.insert_rows,
        insert_cols: o.insert_cols,
        delete_rows: o.delete_rows,
        delete_cols: o.delete_cols,
        format_cells: o.format_cells,
        sort: o.sort,
        auto_filter: o.auto_filter,
        ..SheetProtectionData::default()
    }
}

/// Model flags → API options.
pub fn options_to_api(p: &SheetProtectionData) -> ProtectionOptions {
    ProtectionOptions {
        select_locked: p.select_locked,
        select_unlocked: p.select_unlocked,
        insert_rows: p.insert_rows,
        insert_cols: p.insert_cols,
        delete_rows: p.delete_rows,
        delete_cols: p.delete_cols,
        format_cells: p.format_cells,
        sort: p.sort,
        auto_filter: p.auto_filter,
    }
}

/// The API state.
pub fn info(wb: &Workbook, sheet: SheetId) -> Option<ProtectionInfo> {
    let s = wb.sheet(sheet)?;
    let p = s.meta.protection();
    let has_pw = |p: &SheetProtectionData| {
        p.password_hash.as_deref().is_some_and(|h| !h.is_empty()) || p.xlsx_legacy_hash.is_some()
    };
    Some(ProtectionInfo {
        sheet_protected: p.is_some(),
        sheet_has_password: p.is_some_and(has_pw),
        options: p.map_or_else(ProtectionOptions::default, options_to_api),
        structure_protected: wb.structure_protection.is_some(),
        structure_has_password: wb.structure_protection.as_ref().is_some_and(has_pw),
    })
}

/// The unlocked cells of a sheet as `[r0, c0, r1, c1]` per-column row runs,
/// from the style runs (cells outside any run have the default style).
fn unlocked_runs(wb: &Workbook, s: &crate::storage::Sheet) -> Vec<[i64; 4]> {
    let mut out: Vec<[i64; 4]> = Vec::new();
    for c in 0..u32::try_from(s.columns.len()).unwrap_or(u32::MAX) {
        let Some(col) = s.column(c) else { continue };
        for (a, b, id) in col.styles.runs_in(0, MAX_ROWS - 1) {
            if cell_protection(wb, id).is_protected() {
                continue;
            }
            let (a, b, c) = (i64::from(a), i64::from(b), i64::from(c));
            match out.last_mut() {
                Some(l) if l[1] == c && l[3] == c && l[2] + 1 == a => l[2] = b,
                _ => out.push([a, c, b, c]),
            }
        }
    }
    out
}

/// The first position strictly after (`r`, `c`) in row-major order inside
/// one of `rects` (`[r0, c0, r1, c1]`).
fn first_after(rects: &[[i64; 4]], r: i64, c: i64) -> Option<(i64, i64)> {
    let mut best: Option<(i64, i64)> = None;
    for &[a0, b0, a1, b1] in rects {
        let cand = if r < a0 {
            Some((a0, b0))
        } else if r <= a1 {
            if c < b1 {
                Some((r, (c + 1).max(b0)))
            } else if r < a1 {
                Some((r + 1, b0))
            } else {
                None
            }
        } else {
            None
        };
        if let Some(x) = cand {
            if best.is_none_or(|b| x < b) {
                best = Some(x);
            }
        }
    }
    best
}

/// Tab / Enter on a protected sheet (AOO `ScTable::ValidNextPos`): the next
/// unlocked cell from `from` in reading order, skipping hidden rows and
/// columns and the covered cells of merges, wrapping. Tab (`Right` / `Left`)
/// walks along rows, Enter (`Down` / `Up`) down columns. `within` limits the
/// walk to a selection. `None`: the sheet is not protected or has no such cell.
pub fn next_unlocked(
    wb: &Workbook,
    sheet: SheetId,
    from: (u32, u32),
    dir: Direction,
    within: Option<&RangeDto>,
) -> Option<(u32, u32)> {
    let s = wb.sheet(sheet)?;
    s.meta.protection()?;
    let by_col = matches!(dir, Direction::Up | Direction::Down);
    let fwd = matches!(dir, Direction::Right | Direction::Down);
    let sg: i64 = if fwd { 1 } else { -1 };
    let clip = within.map_or(
        [0, 0, i64::from(MAX_ROWS - 1), i64::from(MAX_COLS - 1)],
        |w| {
            [
                i64::from(w.r0),
                i64::from(w.c0),
                i64::from(w.r1),
                i64::from(w.c1),
            ]
        },
    );
    // Transformed space: (major, minor) = row-major position, negated going back.
    let mut rects: Vec<[i64; 4]> = Vec::new();
    let runs = if cell_protection(wb, StyleId::DEFAULT).is_protected() {
        unlocked_runs(wb, s)
    } else {
        vec![[0, 0, i64::from(MAX_ROWS - 1), i64::from(MAX_COLS - 1)]]
    };
    for [r0, c0, r1, c1] in runs {
        let (r0, c0, r1, c1) = (
            r0.max(clip[0]),
            c0.max(clip[1]),
            r1.min(clip[2]),
            c1.min(clip[3]),
        );
        if r0 > r1 || c0 > c1 {
            continue;
        }
        let (a0, a1, b0, b1) = if by_col {
            (c0, c1, r0, r1)
        } else {
            (r0, r1, c0, c1)
        };
        rects.push(if fwd {
            [a0, b0, a1, b1]
        } else {
            [-a1, -b1, -a0, -b0]
        });
    }
    if rects.is_empty() {
        return None;
    }
    let (fr, fc) = (i64::from(from.0), i64::from(from.1));
    let (mut major, mut minor) = if by_col {
        (fc * sg, fr * sg)
    } else {
        (fr * sg, fc * sg)
    };
    let mut wrapped = false;
    for _ in 0..4_000_000 {
        let Some((a, b)) = first_after(&rects, major, minor) else {
            if wrapped {
                return None;
            }
            wrapped = true;
            major = -4_000_000;
            minor = 0;
            continue;
        };
        let (a, b) = (a * sg, b * sg);
        let (r, c) = if by_col { (b, a) } else { (a, b) };
        let (ru, cu) = (u32::try_from(r).ok()?, u32::try_from(c).ok()?);
        let major_hidden = if by_col {
            s.col_hidden(cu)
        } else {
            s.row_hidden(ru)
        };
        if major_hidden {
            // The whole row (column) is out: continue after its last cell.
            major = a * sg;
            minor = 4_000_000;
            continue;
        }
        if s.col_hidden(cu) || s.row_hidden(ru) || s.is_merge_covered(ru, cu) {
            major = a * sg;
            minor = b * sg;
            continue;
        }
        return Some((ru, cu));
    }
    None
}

// ---------------------------------------------------------------------------
// Checks
// ---------------------------------------------------------------------------

/// The protection attributes of a style.
pub fn cell_protection(wb: &Workbook, id: StyleId) -> CellProtectionData {
    wb.styles.get(id).data.protection.unwrap_or_default()
}

/// The sheet's protection, if protected.
pub fn sheet_protection(wb: &Workbook, sheet: SheetId) -> Option<&SheetProtectionData> {
    wb.sheet(sheet).and_then(|s| s.meta.protection())
}

/// The protection attributes of a cell that are in effect: those of its
/// style while the sheet is protected, `None` on an unprotected sheet
/// ("Hide all" and "Hide formula" only act on a protected sheet, AOO
/// `ScDocument::FillInfo` and `ScTabViewShell::UpdateInputHandler`).
pub fn effective_cell_protection(
    wb: &Workbook,
    sheet: SheetId,
    r: u32,
    c: u32,
) -> Option<CellProtectionData> {
    let s = wb.sheet(sheet)?;
    s.meta.protection()?;
    Some(cell_protection(wb, s.style(r, c)))
}

/// `true` when the cell's content is hidden: "Hide all" on a protected
/// sheet.
pub fn cell_hidden(wb: &Workbook, sheet: SheetId, r: u32, c: u32) -> bool {
    effective_cell_protection(wb, sheet, r, c).is_some_and(|p| p.hidden)
}

/// `true` if every cell of `range` is unlocked.
pub fn all_unlocked(wb: &Workbook, range: &CellRange) -> bool {
    let Some(s) = wb.sheet(range.sheet) else {
        return true;
    };
    let unlocked = |id: StyleId| !cell_protection(wb, id).is_protected();
    // Gaps between style runs have the default (locked) style.
    for c in range.c0..=range.c1 {
        let Some(col) = s.column(c) else {
            return unlocked(StyleId::DEFAULT);
        };
        let mut cur = range.r0;
        for (a, b, id) in col.styles.runs_in(range.r0, range.r1) {
            if a > cur && !unlocked(StyleId::DEFAULT) {
                return false;
            }
            if !unlocked(id) {
                return false;
            }
            cur = b.saturating_add(1);
        }
        if cur <= range.r1 && !unlocked(StyleId::DEFAULT) {
            return false;
        }
        if c == u32::MAX {
            break;
        }
    }
    true
}

/// Refuses content edits of locked cells on a protected sheet.
pub fn check_edit(wb: &Workbook, ranges: &[CellRange]) -> Result<(), ApiError> {
    for r in ranges {
        if sheet_protection(wb, r.sheet).is_some() && !all_unlocked(wb, r) {
            return Err(protected_error());
        }
    }
    Ok(())
}

/// Refuses formatting on a protected sheet unless allowed or unlocked.
pub fn check_format(wb: &Workbook, ranges: &[CellRange]) -> Result<(), ApiError> {
    for r in ranges {
        if let Some(p) = sheet_protection(wb, r.sheet) {
            if !p.format_cells && !all_unlocked(wb, r) {
                return Err(protected_error());
            }
        }
    }
    Ok(())
}

/// Refuses an action on a protected sheet unless `allowed(protection)`.
pub fn check_sheet_action(
    wb: &Workbook,
    sheet: SheetId,
    allowed: impl Fn(&SheetProtectionData) -> bool,
) -> Result<(), ApiError> {
    match sheet_protection(wb, sheet) {
        Some(p) if !allowed(p) => Err(protected_error()),
        _ => Ok(()),
    }
}

/// Refuses sheet add/delete/rename/move while the structure is protected.
pub fn check_structure(wb: &Workbook) -> Result<(), ApiError> {
    if wb.structure_protection.is_some() {
        Err(structure_error())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    #[test]
    fn digests() {
        assert_eq!(
            hex(&sha1(b"abc")),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        assert_eq!(
            hex(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            hex(&sha256(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hex(&sha512(b"abc")),
            "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a\
             2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"
        );
        let long = vec![b'a'; 1000];
        assert_eq!(
            hex(&sha256(&long)),
            "41edece42d63e8d9bf515a9ba6932e1c20cbc9f5a5d134645adb5db1b9737ea3"
        );
    }

    #[test]
    fn base64_round_trip() {
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"M"), "TQ==");
        assert_eq!(base64_decode("TWE=").unwrap(), b"Ma");
        assert!(base64_decode("!!").is_none());
    }

    #[test]
    fn legacy_hash_and_verify() {
        // Known value: Excel's hash of "password" is 83AF.
        assert_eq!(xlsx_legacy_hash("password"), "83AF");
        let p = new_protection(Some("secret"), &ProtectionOptions::default());
        assert!(verify(&p, Some("secret")));
        assert!(!verify(&p, Some("Secret")));
        assert!(!verify(&p, None));
        let legacy = SheetProtectionData {
            password_hash: Some("83AF".into()),
            algorithm: Some(hash_algorithms::XLSX_LEGACY.into()),
            ..SheetProtectionData::default()
        };
        assert!(verify(&legacy, Some("password")));
        assert!(!verify(&legacy, Some("nope")));
        let open = new_protection(None, &ProtectionOptions::default());
        assert!(verify(&open, None));
        // OOXML agile hash: salt + UTF-16LE, spun.
        let salt = b"0123456789abcdef";
        let h = ooxml_hash("SHA-512", salt, "pw", 3).unwrap();
        let agile = SheetProtectionData {
            password_hash: Some(base64(&h)),
            algorithm: Some("SHA-512".into()),
            salt: Some(base64(salt)),
            spin_count: Some(3),
            ..SheetProtectionData::default()
        };
        assert!(verify(&agile, Some("pw")));
        assert!(!verify(&agile, Some("px")));
    }
}
