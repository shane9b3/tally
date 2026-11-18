//! The pre-scan against calamine 0.38 (audit of 2026-11-18,
//! docs/security.md "XLS pre-scan audit"): every place where calamine
//! indexes, subtracts or allocates from a field of the file has a hostile
//! input here. A file must either open (the damaged record neutralized,
//! with a warning where something is lost) or be refused by the pre-scan
//! itself; calamine must never be the one to panic.
#![allow(clippy::unwrap_used)]

mod biff;

use std::time::{Duration, Instant};

use tally_io_xls::{prescan, read_xls, Limits, XlsError};
use tally_model::{CellContentData, WorkbookData};
use tally_types::Value;

fn words(ws: &[u16]) -> Vec<u8> {
    ws.iter().flat_map(|w| w.to_le_bytes()).collect()
}

/// A BIFF8 workbook: `globals` after the usual ones, one sheet.
fn book(globals: &[u8], sheet: &[u8]) -> Vec<u8> {
    let mut g = biff::globals(&[], &[], &[]);
    g.extend_from_slice(globals);
    biff::file(biff::stream(&g, &[("S", sheet.to_vec())]))
}

/// A BIFF5 workbook (code page 1252), one sheet.
fn book5(globals: &[u8], sheet: &[u8]) -> Vec<u8> {
    let mut g = biff::rec(0x0042, &1252u16.to_le_bytes());
    g.extend_from_slice(globals);
    biff::file(biff::stream5(&g, &[("S", sheet.to_vec())]))
}

/// Runs calamine itself on the sanitized file, outside `read_xls`'s
/// `catch_unwind`: a panic in calamine fails the test.
fn calamine_opens(file: &[u8]) -> bool {
    let (clean, _) = prescan::sanitize(file, 1024, 10_000_000).unwrap();
    calamine::Xls::new_with_options(std::io::Cursor::new(clean), calamine::XlsOptions::default())
        .is_ok()
}

/// Reads `file`; calamine must not have panicked (the pre-scan refuses or
/// neutralizes first) and neither may the formatting pass.
fn read(file: &[u8]) -> Result<WorkbookData, XlsError> {
    if let Ok((clean, _)) = prescan::sanitize(file, 1024, 10_000_000) {
        // Not under catch_unwind: a calamine panic fails the test here.
        let _ = calamine::Xls::new_with_options(
            std::io::Cursor::new(clean),
            calamine::XlsOptions::default(),
        );
    }
    let r = read_xls(file, &Limits::default());
    if let Ok(wb) = &r {
        assert!(
            !wb.warnings.iter().any(|w| w.contains("could not be read")),
            "{:?}",
            wb.warnings
        );
    }
    r
}

fn formula_text(wb: &WorkbookData, i: usize) -> Option<String> {
    match &wb.sheets[0].cells[i].content {
        CellContentData::Formula { text, .. } => Some(text.to_string()),
        _ => None,
    }
}

// ------------------------------------------------------------ 0.38 changes

/// calamine 0.38 reads a BIFF5 `PtgStr` as `cch` + bytes (0.37: `cch`,
/// flags, bytes). A pre-scan walking the 0.37 layout loses its place and
/// misses every `PtgAttrSpace` behind the string: 2,000 tokens of 255
/// spaces each are 510 KB of text and seconds of work per 8 KB record.
#[test]
fn biff5_string_tokens_keep_the_walk_in_step() {
    let mut rgce = vec![0x17, 1, b'A'];
    for _ in 0..2000 {
        rgce.extend_from_slice(&[0x19, 0x40, 0x00, 0xFF]);
    }
    let body: Vec<u8> = (0..40)
        .flat_map(|i| biff::formula(i, 0, 0, 1.0, &rgce))
        .collect();
    let file = book5(&[], &body);
    let t = Instant::now();
    let wb = read(&file).unwrap();
    assert!(t.elapsed() < Duration::from_secs(5), "{:?}", t.elapsed());
    assert_eq!(wb.sheets[0].cells.len(), 40);
    let text = formula_text(&wb, 0).unwrap();
    assert!(text.len() < 20, "{} bytes of formula", text.len());
    assert!(text.contains("\"A\""), "{text}");
}

/// An empty `CONTINUE` inside the `SST` is an empty string for calamine.
/// A pre-scan that stops there numbers the later strings differently,
/// does not count the `LABELSST` cells that use them, and so misses their
/// bounding box: two cells far apart were 10 M dense cells per sheet.
#[test]
fn empty_continue_records_in_the_sst_are_strings() {
    let mut g = biff::sst(&["a"]);
    g.extend(biff::rec(0x003C, &[]));
    // A CONTINUE holding one more string, "b" (8-bit).
    g.extend(biff::rec(0x003C, &[1, 0, 0, b'b']));
    let mut body = biff::labelsst(0, 0, 15, 2);
    body.extend(biff::labelsst(40_000, 255, 15, 2));
    let r = read(&book(&g, &body));
    assert!(
        matches!(
            r,
            Err(XlsError::LimitExceeded {
                what: "dense sheet area (cells)",
                ..
            })
        ),
        "{:?}",
        r.map(|w| w.sheets[0].cells.len())
    );
    // Close together the same strings open, numbered as calamine does.
    let mut body = biff::labelsst(0, 0, 15, 2);
    body.extend(biff::labelsst(1, 0, 15, 1));
    let wb = read(&book(&g, &body)).unwrap();
    assert_eq!(
        wb.sheets[0].cells[0].content,
        CellContentData::Value(Value::text("b"))
    );
}

// ------------------------------------------------- the version that was audited

/// The pre-scan models one calamine version's decoder. An upgrade must
/// come with a new audit (docs/security.md "XLS pre-scan audit"): diff
/// calamine's `xls.rs`, `cfb.rs` and `utils.rs`, re-check every row of
/// the table there, extend this file, then change
/// `prescan::AUDITED_CALAMINE`.
#[test]
fn the_locked_calamine_is_the_audited_one() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for lock in ["Cargo.lock", "fuzz/Cargo.lock"] {
        let text = std::fs::read_to_string(root.join(lock)).unwrap();
        let versions: Vec<&str> = text
            .split("[[package]]")
            .filter(|p| p.contains("name = \"calamine\""))
            .filter_map(|p| p.split("version = \"").nth(1))
            .filter_map(|v| v.split('"').next())
            .collect();
        assert_eq!(
            versions,
            [prescan::AUDITED_CALAMINE],
            "{lock} locks a calamine the XLS pre-scan was not audited against: re-audit \
             crates/tally-io-xls/src/prescan.rs first (docs/security.md, \"XLS pre-scan audit\")"
        );
    }
}

// ------------------------------------------------------------ records cut short

/// Records as `(type, data)`.
type Records = Vec<(u16, Vec<u8>)>;

/// Splits record bytes into `(type, data)`.
fn split(bytes: &[u8]) -> Records {
    let mut out = Vec::new();
    let mut at = 0;
    while at + 4 <= bytes.len() {
        let t = u16::from_le_bytes([bytes[at], bytes[at + 1]]);
        let l = u16::from_le_bytes([bytes[at + 2], bytes[at + 3]]) as usize;
        out.push((t, bytes[at + 4..at + 4 + l].to_vec()));
        at += 4 + l;
    }
    out
}

/// A stream of global records (one of them a `BOUNDSHEET`, whose offset
/// is filled in if it has room for one) and one sheet substream.
fn assemble(globals: &[(u16, Vec<u8>)], sheet: &[(u16, Vec<u8>)]) -> Vec<u8> {
    let mut s = Vec::new();
    let mut bound = None;
    for (t, d) in globals {
        if *t == 0x0085 && d.len() >= 4 {
            bound = Some(s.len() + 4);
        }
        s.extend(biff::rec(*t, d));
    }
    if let Some(b) = bound {
        let at = s.len() as u32;
        s[b..b + 4].copy_from_slice(&at.to_le_bytes());
    }
    for (t, d) in sheet {
        s.extend(biff::rec(*t, d));
    }
    s
}

/// `NAME` (BIFF8): an 8-bit name and its formula.
fn name8(name: &str, rgce: &[u8]) -> Vec<u8> {
    let mut d = vec![0, 0, 0, name.len() as u8];
    d.extend(words(&[rgce.len() as u16, 0, 0]));
    d.extend_from_slice(&[0; 4]);
    d.push(0);
    d.extend_from_slice(name.as_bytes());
    d.extend_from_slice(rgce);
    biff::rec(0x0018, &d)
}

/// `NAME` (BIFF5): the name has no flags byte.
fn name5(name: &str, rgce: &[u8]) -> Vec<u8> {
    let mut d = vec![0, 0, 0, name.len() as u8];
    d.extend(words(&[rgce.len() as u16, 0, 0]));
    d.extend_from_slice(&[0; 4]);
    d.extend_from_slice(name.as_bytes());
    d.extend_from_slice(rgce);
    biff::rec(0x0018, &d)
}

/// `PtgArea3d` (BIFF8) with all its operands: `$A$1:$B$2`.
fn area3d() -> Vec<u8> {
    let mut d = vec![0x3b];
    d.extend(words(&[0, 0, 1, 0, 1]));
    d
}

/// Every record type calamine reads, well formed: BIFF8 or BIFF5.
fn every_record(biff5: bool) -> (Records, Records) {
    let mut g = if biff5 { biff::bof5(5) } else { biff::bof(5) };
    g.extend(biff::rec(
        0x0042,
        &(if biff5 { 1252u16 } else { 1200 }).to_le_bytes(),
    ));
    g.extend(biff::rec(0x0022, &[0, 0]));
    g.extend(biff::rec(0x002F, &[0, 0]));
    g.extend(biff::rec(0x013D, &[0, 0]));
    if biff5 {
        g.extend(biff::rec(0x041E, &{
            let mut d = words(&[164, 4]);
            d.extend_from_slice(b"0.00");
            d
        }));
        for _ in 0..17 {
            g.extend(biff::rec(0x00E0, &[0; 16]));
        }
        g.extend(biff::rec(0x0017, &[3, 2, b'S', b'1']));
        // PtgArea3d of BIFF5: 20 operand bytes.
        let mut a = vec![0x3b];
        a.extend_from_slice(&[0; 20]);
        g.extend(name5("One", &a));
        g.extend(name5("Two", &[0x1E, 1, 0]));
    } else {
        g.extend(biff::globals(&[], &[(164, "0.00")], &[]));
        g.extend(biff::rec(0x0017, &words(&[1, 0, 0, 0])));
        g.extend(name8("One", &area3d()));
        g.extend(name8("Two", &[0x1E, 1, 0]));
        // SST: "ab" (rich, with one run), then "cd" continued in a
        // CONTINUE after its first character, then an extended string.
        let mut sst = Vec::new();
        sst.extend_from_slice(&[3, 0, 0, 0, 3, 0, 0, 0]);
        sst.extend_from_slice(&[2, 0, 0x08, 1, 0, b'a', b'b', 0, 0, 0, 0]);
        sst.extend_from_slice(&[2, 0, 0, b'c']);
        g.extend(biff::rec(0x00FC, &sst));
        g.extend(biff::rec(
            0x003C,
            &[0, b'd', 1, 0, 0x04, 2, 0, 0, 0, b'e', 9, 9],
        ));
    }
    g.extend(biff::rec(0x0085, &{
        let mut d = vec![0u8; 6];
        d.push(1);
        if !biff5 {
            d.push(0);
        }
        d.push(b'S');
        d
    }));
    g.extend(biff::rec(0x000A, &[]));

    let mut s = if biff5 {
        biff::bof5(0x10)
    } else {
        biff::bof(0x10)
    };
    s.extend(biff::rec(0x0200, &[0; 14]));
    s.extend(biff::number(0, 0, 15, 1.5));
    s.extend(biff::rk(1, 0, 15, 7));
    s.extend(biff::mulrk(2, 0, &[(15, 1), (15, 2), (15, 3)]));
    s.extend(biff::boolerr(3, 0, 15, 1, false));
    // ="A"&"B" with a space, references, a function, a name: SUM(…).
    if biff5 {
        let text = |t: u16, r: u16| {
            let mut d = words(&[r, 0, 15, 2]);
            d.extend_from_slice(b"hi");
            biff::rec(t, &d)
        };
        s.extend(text(0x0204, 4));
        s.extend(text(0x00D6, 5));
        let mut rgce = vec![0x17, 1, b'A', 0x17, 1, b'B', 0x08, 0x19, 0x40, 0, 2];
        rgce.extend_from_slice(&[0x24, 1, 0, 0, 0x25, 0, 0, 1, 0, 0, 0, 0x41, 2, 0]);
        rgce.extend_from_slice(&[0x23, 2, 0, 0, 0, 0x1E, 5, 0, 0x42, 5, 4, 0]);
        s.extend(biff::formula(6, 0, 15, 1.0, &rgce));
    } else {
        s.extend(biff::label(4, 0, 15, "hi"));
        s.extend(biff::rstring(5, 0, 15, "hi"));
        s.extend(biff::labelsst(7, 0, 15, 1));
        let mut rgce = vec![
            0x17, 1, 0, b'A', 0x17, 1, 1, b'B', 0, 0x08, 0x19, 0x40, 0, 2,
        ];
        rgce.extend_from_slice(&[0x24, 1, 0, 0, 0, 0x25, 0, 0, 1, 0, 0, 0, 0, 0, 0x41, 2, 0]);
        rgce.extend_from_slice(&[0x23, 2, 0, 0, 0, 0x1E, 5, 0, 0x42, 5, 4, 0]);
        rgce.extend_from_slice(&[0x3a, 0, 0, 1, 0, 1, 0, 0x42, 2, 4, 0]);
        s.extend(biff::formula(6, 0, 15, 1.0, &rgce));
    }
    // A formula with a string result and its STRING record.
    let mut f = words(&[8, 0, 15]);
    f.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0xFF, 0xFF]);
    f.extend_from_slice(&[0; 6]);
    f.extend(words(&[3]));
    f.extend_from_slice(&[0x1E, 1, 0]);
    s.extend(biff::rec(0x0006, &f));
    s.extend(biff::rec(
        0x0207,
        if biff5 {
            &[2, 0, b'o', b'k']
        } else {
            &[2, 0, 0, b'o', b'k']
        },
    ));
    s.extend(biff::merged(&[(0, 1, 0, 1), (4, 5, 2, 3)]));
    s.extend(biff::rec(0x000A, &[]));
    (split(&g), split(&s))
}

/// The books of [`every_record`] open, with every cell.
#[test]
fn the_reference_books_open() {
    for biff5 in [false, true] {
        let (g, s) = every_record(biff5);
        let file = biff::file(assemble(&g, &s));
        assert!(calamine_opens(&file), "biff5 {biff5}");
        let wb = read(&file).unwrap();
        let cells = wb.sheets[0].cells.len();
        assert_eq!(cells, if biff5 { 10 } else { 11 }, "biff5 {biff5}");
        assert_eq!(wb.sheets[0].merges.len(), 2);
        assert!(
            !wb.warnings
                .iter()
                .any(|w| w.contains("damaged") || w.contains("formula")),
            "{:?}",
            wb.warnings
        );
    }
}

/// Every record calamine reads, cut at every length, in BIFF8 and BIFF5:
/// the file opens or is refused, and calamine never panics ([`read`] runs
/// it outside `catch_unwind`).
#[test]
fn every_record_cut_at_every_length() {
    let mut opened = 0;
    let mut refused = 0;
    for biff5 in [false, true] {
        let (g, s) = every_record(biff5);
        for part in 0..2 {
            let list = if part == 0 { &g } else { &s };
            for (k, (_, data)) in list.iter().enumerate() {
                for cut in 0..data.len() {
                    let mut l = list.clone();
                    l[k].1.truncate(cut);
                    let stream = if part == 0 {
                        assemble(&l, &s)
                    } else {
                        assemble(&g, &l)
                    };
                    match read(&biff::file(stream)) {
                        Ok(_) => opened += 1,
                        Err(_) => refused += 1,
                    }
                }
            }
        }
    }
    assert!(
        opened > 200 && refused > 100,
        "{opened} opened, {refused} refused"
    );
}

/// Every byte of the reference streams overwritten with a few values:
/// lengths, counts, flags, token types and offsets that lie.
#[test]
fn every_byte_overwritten() {
    for biff5 in [false, true] {
        let (g, s) = every_record(biff5);
        let stream = assemble(&g, &s);
        for i in 0..stream.len() {
            for v in [
                0x00u8,
                0x01,
                0x7F,
                0xFF,
                stream[i] ^ 0x5A,
                stream[i].wrapping_add(1),
            ] {
                let mut m = stream.clone();
                m[i] = v;
                let _ = read(&biff::file(m));
            }
        }
    }
}

/// A deterministic generator (xorshift).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// How many random inputs the two random tests try: `default`, or
/// `TALLY_XLS_AUDIT_ITERS` (the audit ran them with 400,000).
fn iterations(default: u64) -> u64 {
    std::env::var("TALLY_XLS_AUDIT_ITERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// The reference streams with one to four random edits each (bytes and
/// 16-bit fields overwritten, bytes removed, inserted or repeated, the
/// stream cut): calamine never panics on what the pre-scan lets through.
#[test]
fn mutated_streams_never_panic_calamine() {
    let streams: Vec<Vec<u8>> = [false, true]
        .iter()
        .map(|b| {
            let (g, s) = every_record(*b);
            assemble(&g, &s)
        })
        .collect();
    let mut rng = Rng(0xD1B5_4A32_D192_ED03);
    for i in 0..iterations(3000) {
        let mut m = streams[(i % 2) as usize].clone();
        for _ in 0..1 + rng.below(4) {
            if m.is_empty() {
                break;
            }
            let at = rng.below(m.len() as u64) as usize;
            let n = (1 + rng.below(16) as usize).min(m.len() - at);
            match rng.below(7) {
                0 => m[at] = rng.next() as u8,
                1 => m[at] = [0, 1, 0x7F, 0x80, 0xFF][rng.below(5) as usize],
                2 if n >= 2 => {
                    let v = [0u16, 1, 2, 0x00FF, 0x7FFF, 0xFFFF, 0x2020][rng.below(7) as usize];
                    m[at..at + 2].copy_from_slice(&v.to_le_bytes());
                }
                3 => {
                    m.drain(at..at + n);
                }
                4 => {
                    let chunk = m[at..at + n].to_vec();
                    m.splice(at..at, chunk);
                }
                5 => {
                    let b = rng.next() as u8;
                    m.splice(at..at, std::iter::repeat_n(b, n));
                }
                _ => m.truncate(at.max(8)),
            }
        }
        let _ = read(&biff::file(m));
    }
}

// ------------------------------------------------------------------- formulas

/// Random token streams (every token type calamine knows, operands cut
/// anywhere, counts and indices at their extremes) never make calamine
/// panic, and every cell keeps its cached value.
#[test]
fn random_formulas_never_panic_calamine() {
    const PTGS: &[u8] = &[
        0x01, 0x03, 0x05, 0x08, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1C, 0x1D,
        0x1E, 0x1F, 0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x2A, 0x2B, 0x39, 0x3a, 0x3b, 0x3c, 0x3d,
        0x41, 0x42, 0x43, 0x44, 0x45, 0x59, 0x5a, 0x62, 0x7b, 0x26, 0x00,
    ];
    const ETPG: &[u8] = &[0x01, 0x02, 0x04, 0x08, 0x10, 0x20, 0x21, 0x40, 0x41, 0x99];
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for round in 0..iterations(24_000) / 200 {
        let biff5 = round % 2 == 1;
        let mut body = Vec::new();
        for row in 0..200u16 {
            let mut rgce = Vec::new();
            for _ in 0..rng.below(12) {
                let ptg = PTGS[rng.below(PTGS.len() as u64) as usize];
                rgce.push(ptg);
                if ptg == 0x19 {
                    rgce.push(ETPG[rng.below(ETPG.len() as u64) as usize]);
                }
                for _ in 0..rng.below(12) {
                    // Small values, zeros and 0xFF are the interesting ones.
                    rgce.push(match rng.below(4) {
                        0 => 0,
                        1 => 0xFF,
                        2 => rng.below(4) as u8,
                        _ => rng.next() as u8,
                    });
                }
            }
            let mut rec = biff::formula(row, 0, 15, 1.0, &rgce);
            // Sometimes the declared length lies.
            if rng.below(8) == 0 {
                let cce = rng.next() as u16;
                rec[24..26].copy_from_slice(&cce.to_le_bytes());
            }
            body.extend(rec);
        }
        let file = if biff5 {
            book5(&name5("N", &[0x1E, 1, 0]), &body)
        } else {
            book(&name8("N", &[0x1E, 1, 0]), &body)
        };
        let wb = read(&file).unwrap();
        assert_eq!(wb.sheets[0].cells.len(), 200, "round {round}");
    }
}

/// A formula that ends inside a token keeps its cell: the cached value,
/// and the usual warning. (calamine indexes the operands unchecked.)
#[test]
fn a_formula_cut_inside_a_token_keeps_its_value() {
    for rgce in [
        // PtgRef with 3 of 4 operand bytes, PtgInt with one, PtgNum.
        vec![0x44, 0, 0, 0],
        vec![0x1E, 1],
        vec![0x1F, 0, 0, 0],
        // PtgArea3d of 7 bytes, PtgFuncVar without its index.
        vec![0x3b, 0, 0, 0, 0, 0, 0],
        vec![0x1E, 1, 0, 0x42, 1],
        // PtgAttr without a type, PtgAttrChoose without its table.
        vec![0x1E, 1, 0, 0x19],
        vec![0x1E, 1, 0, 0x19, 0x04, 9, 0],
        // Row 65,536 (`row + 1` in 16 bits) and name 0 (`index - 1`).
        vec![0x44, 0xFF, 0xFF, 0, 0xC0],
        vec![0x3a, 0, 0, 0xFF, 0xFF, 0, 0],
        vec![0x23, 0, 0, 0, 0],
    ] {
        let mut body = biff::formula(0, 0, 15, 42.0, &rgce);
        body.extend(biff::formula(1, 0, 15, 7.0, &biff::ptg_int(7)));
        let file = book(&[], &body);
        let (_, scan) = prescan::sanitize(&file, 1024, 10_000_000).unwrap();
        assert_eq!(scan.damaged_formulas, 1, "{rgce:x?}");
        let wb = read(&file).unwrap();
        assert_eq!(
            wb.sheets[0].cells[0].content,
            CellContentData::Value(Value::Number(42.0)),
            "{rgce:x?}"
        );
        assert_eq!(formula_text(&wb, 1).as_deref(), Some("of:=7"));
        assert!(
            wb.warnings.iter().any(|w| w.starts_with("1 formula(s)")),
            "{:?}",
            wb.warnings
        );
    }
    // A declared length past the record (the fuzzer's xls.rs:1531).
    let mut rec = biff::formula(0, 0, 15, 42.0, &biff::ptg_int(7));
    rec[24..26].copy_from_slice(&500u16.to_le_bytes());
    let wb = read(&book(&[], &rec)).unwrap();
    assert_eq!(
        wb.sheets[0].cells[0].content,
        CellContentData::Value(Value::Number(42.0))
    );
    // No room for the length at all: refused by the pre-scan.
    let mut d = words(&[0, 0, 15]);
    d.extend_from_slice(&[0; 15]);
    let r = read(&book(&[], &biff::rec(0x0006, &d)));
    assert!(
        matches!(&r, Err(XlsError::Malformed(m)) if m.contains("FORMULA")),
        "{:?}",
        r.err()
    );
}

/// Sound formulas are untouched by the walk: strings (8 and 16 bit),
/// references, areas, functions, 3-D references and names decode as
/// before, in BIFF8 and BIFF5.
#[test]
fn sound_formulas_are_untouched() {
    use calamine::Reader;
    for biff5 in [false, true] {
        let (g, s) = every_record(biff5);
        let file = biff::file(assemble(&g, &s));
        let (clean, scan) = prescan::sanitize(&file, 1024, 10_000_000).unwrap();
        assert_eq!(scan.damaged_formulas, 0);
        assert_eq!(scan.damaged_names, 0);
        assert_eq!(scan.hidden_records, 0);
        let mut wb: calamine::Xls<_> = calamine::Xls::new(std::io::Cursor::new(clean)).unwrap();
        let f = wb.worksheet_formula("S").unwrap();
        let text = f.get_value((6, 0)).unwrap();
        assert!(text.contains("SUM(\"A\"&\"B\","), "{text}");
        assert!(text.contains("ISNA($A$1:$A$2)"), "{text}");
        assert!(text.contains(",Two,5"), "{text}");
        assert_eq!(wb.defined_names().len(), 2);
    }
}

// ---------------------------------------------------------------------- names

/// A `NAME` whose formula ends inside its token (a 7-byte `PtgArea3d`)
/// made calamine panic and the file unreadable. Now the name keeps its
/// place in the table with an empty formula (formulas refer to names by
/// index), the file opens and a warning says a name was left out.
#[test]
fn a_name_with_a_truncated_formula_is_neutralized() {
    let mut g = name8("Broken", &area3d()[..7]);
    g.extend(name8("Good", &area3d()));
    // =Good (the second name) and =Broken.
    let mut body = biff::formula(0, 0, 15, 1.0, &[0x23, 2, 0, 0, 0]);
    body.extend(biff::formula(1, 0, 15, 2.0, &[0x23, 1, 0, 0, 0]));
    let file = book(&g, &body);
    let (clean, scan) = prescan::sanitize(&file, 1024, 10_000_000).unwrap();
    assert_eq!(scan.damaged_names, 1);
    {
        use calamine::Reader;
        let mut wb: calamine::Xls<_> = calamine::Xls::new(std::io::Cursor::new(clean)).unwrap();
        let names = wb.defined_names().to_vec();
        assert_eq!(names.len(), 2);
        assert_eq!(names[0], ("Broken".to_string(), "empty rgce".to_string()));
        assert_eq!(names[1].0, "Good");
        assert!(names[1].1.ends_with("$A$1:$B$2"), "{}", names[1].1);
        let f = wb.worksheet_formula("S").unwrap();
        assert_eq!(f.get_value((0, 0)).unwrap(), "Good");
        assert_eq!(f.get_value((1, 0)).unwrap(), "Broken");
    }
    let wb = read(&file).unwrap();
    assert_eq!(wb.sheets[0].cells.len(), 2);
    assert!(
        wb.warnings
            .iter()
            .any(|w| w == "1 defined name(s) in this file are damaged and were left out."),
        "{:?}",
        wb.warnings
    );

    // The other ways a NAME lies: a formula longer than the record, a
    // name longer than the record (cut to what is there), BIFF5 operands.
    let mut long = name8("Long", &[0x1E, 1, 0]);
    long[8..10].copy_from_slice(&9000u16.to_le_bytes());
    let mut wide = name8("Wide", &[]);
    wide[7] = 200;
    for g in [long, wide] {
        let file = book(&g, &biff::number(0, 0, 15, 1.0));
        let (_, scan) = prescan::sanitize(&file, 1024, 10_000_000).unwrap();
        assert_eq!(scan.damaged_names, 1);
        assert_eq!(read(&file).unwrap().sheets[0].cells.len(), 1);
    }
    let file = book5(
        &name5("Five", &[0x3b, 0, 0, 0, 0, 0, 0]),
        &biff::number(0, 0, 15, 1.0),
    );
    let (_, scan) = prescan::sanitize(&file, 1024, 10_000_000).unwrap();
    assert_eq!(scan.damaged_names, 1);
    assert!(read(&file).is_ok());

    // A sound name is not touched.
    let file = book(&name8("Good", &area3d()), &biff::number(0, 0, 15, 1.0));
    let (_, scan) = prescan::sanitize(&file, 1024, 10_000_000).unwrap();
    assert_eq!(scan.damaged_names, 0);
    assert!(read(&file).unwrap().warnings.is_empty());

    // Too short for its fixed part: nothing to keep the index with.
    let file = book(&biff::rec(0x0018, &[0; 14]), &biff::number(0, 0, 15, 1.0));
    let r = read(&file);
    assert!(
        matches!(&r, Err(XlsError::Malformed(m)) if m.contains("NAME")),
        "{:?}",
        r.err()
    );
}

// --------------------------------------------------------------- other records

/// Records calamine reads without a length check, each too short: the
/// optional ones are skipped with a warning, the others refuse the file
/// with the pre-scan's message (not calamine's panic).
#[test]
fn short_records_are_hidden_or_refused() {
    let cell = biff::number(0, 0, 15, 1.0);
    // CODEPAGE, DATEMODE, EXTERNSHEET (globals), MERGEDCELLS (sheet).
    for (g, s) in [
        (biff::rec(0x0042, &[0xB0]), Vec::new()),
        (biff::rec(0x0022, &[1]), Vec::new()),
        (biff::rec(0x0017, &[1]), Vec::new()),
        (Vec::new(), biff::rec(0x00E5, &[1])),
    ] {
        let mut sheet = s.clone();
        sheet.extend(&cell);
        let wb = read(&book(&g, &sheet)).unwrap();
        assert_eq!(wb.sheets[0].cells.len(), 1);
        assert_eq!(
            wb.warnings,
            ["1 damaged record(s) in this file were skipped."],
            "{g:x?} {s:x?}"
        );
    }
    // FILEPASS, BOF, BOUNDSHEET (globals).
    for (g, what) in [
        (biff::rec(0x002F, &[0]), "FILEPASS"),
        (biff::rec(0x0809, &[0]), "BOF"),
        (biff::rec(0x0085, &[0; 5]), "BOUNDSHEET"),
    ] {
        let r = read(&book(&g, &cell));
        assert!(
            matches!(&r, Err(XlsError::Malformed(m)) if m.contains(what)),
            "{what}: {:?}",
            r.err()
        );
    }
    // A sheet that starts past the stream (the fuzzer's xls.rs:633).
    let mut stream = biff::stream(&biff::globals(&[], &[], &[]), &[("S", cell.clone())]);
    let at = stream
        .windows(4)
        .position(|w| w == [0x85, 0, 10, 0])
        .unwrap();
    stream[at + 4..at + 8].copy_from_slice(&0x00FF_FFFFu32.to_le_bytes());
    let r = read(&biff::file(stream));
    assert!(
        matches!(&r, Err(XlsError::Malformed(m)) if m.contains("sheet starts")),
        "{:?}",
        r.err()
    );
}

/// `MULRK` whose last column contradicts its length (calamine subtracts
/// in 16 bits), `MERGEDCELLS` that count more areas than they hold, and
/// `DIMENSIONS` of any length.
#[test]
fn counts_and_columns_that_lie() {
    let mut m = biff::mulrk(0, 5, &[(15, 1), (15, 2)]);
    let n = m.len();
    m[n - 2..].copy_from_slice(&2u16.to_le_bytes());
    let r = read(&book(&[], &m));
    assert!(
        matches!(&r, Err(XlsError::Malformed(m)) if m.contains("MULRK")),
        "{:?}",
        r.err()
    );
    // First column 65,530 with ten values: the last column wraps.
    let mut m = biff::mulrk(0, 0, &[(15, 1); 10]);
    m[6..8].copy_from_slice(&65_530u16.to_le_bytes());
    let n = m.len();
    m[n - 2..].copy_from_slice(&3u16.to_le_bytes());
    assert!(read(&book(&[], &m)).is_err());

    let mut merged = biff::merged(&[(0, 1, 0, 1), (4, 5, 2, 3)]);
    merged[4..6].copy_from_slice(&9000u16.to_le_bytes());
    merged.extend(biff::number(0, 0, 15, 1.0));
    let wb = read(&book(&[], &merged)).unwrap();
    assert_eq!(wb.sheets[0].merges.len(), 2);
    assert!(wb.warnings.is_empty());

    // DIMENSIONS: a length calamine refuses, and rows that go backwards.
    for d in [vec![0u8; 3], vec![0xFF; 14], {
        let mut d = Vec::new();
        d.extend_from_slice(&9u32.to_le_bytes());
        d.extend_from_slice(&2u32.to_le_bytes());
        d.extend_from_slice(&[9, 0, 2, 0, 0, 0]);
        d
    }] {
        let mut s = biff::rec(0x0200, &d);
        s.extend(biff::number(0, 0, 15, 1.0));
        assert_eq!(read(&book(&[], &s)).unwrap().sheets[0].cells.len(), 1);
    }
}

/// calamine counts the `[` of a number format in a `u8`.
#[test]
fn a_format_of_brackets_opens() {
    for wide in [false, true] {
        let mut d = words(&[164, 300]);
        d.push(u8::from(wide));
        let one: &[u8] = if wide { b"[\0" } else { b"[" };
        d.extend(one.repeat(300));
        let wb = read(&book(&biff::rec(0x041E, &d), &biff::number(0, 0, 15, 1.0))).unwrap();
        assert_eq!(wb.sheets[0].cells.len(), 1);
    }
}

// ------------------------------------------------------------- shared strings

/// A shared string table calamine cannot read to its end (it panics on
/// some of these) is refused by the pre-scan.
#[test]
fn a_damaged_shared_string_table_is_refused() {
    let head = [1u8, 0, 0, 0, 1, 0, 0, 0];
    let with = |tail: &[u8], cont: Option<&[u8]>| {
        let mut d = head.to_vec();
        d.extend_from_slice(tail);
        let mut g = biff::rec(0x00FC, &d);
        if let Some(c) = cont {
            g.extend(biff::rec(0x003C, c));
        }
        book(&g, &biff::labelsst(0, 0, 15, 0))
    };
    for file in [
        // Rich text and extended flags without their counts.
        with(&[1, 0, 0x08], None),
        with(&[1, 0, 0x08, 1], None),
        with(&[1, 0, 0x04, 1, 0], None),
        // A string that goes on in an empty CONTINUE, or nowhere.
        with(&[4, 0, 0, b'a'], Some(&[])),
        with(&[4, 0, 0, b'a'], None),
        // Two bytes of a string header.
        with(&[4, 0], None),
        // Formatting runs and extended data that are not there.
        with(&[1, 0, 0x08, 9, 0, b'a'], None),
        with(&[1, 0, 0x04, 0xFF, 0xFF, 0xFF, 0xFF, b'a'], None),
        // A header too short for its counts.
        book(&biff::rec(0x00FC, &[0; 7]), &biff::labelsst(0, 0, 15, 0)),
    ] {
        let r = read(&file);
        assert!(
            matches!(&r, Err(XlsError::Malformed(m)) if m.contains("shared string")),
            "{:?}",
            r.map(|w| w.sheets[0].cells.len())
        );
    }
    // A string continued properly still reads.
    let wb = read(&with(&[4, 0, 0, b'a'], Some(&[0, b'b', b'c', b'd']))).unwrap();
    assert_eq!(
        wb.sheets[0].cells[0].content,
        CellContentData::Value(Value::text("abcd"))
    );
}

/// calamine keeps a slice per `CONTINUE` record of every record (16 bytes
/// for 4 bytes of file) and removes the shared strings' ones from the
/// front of a vector, one by one.
#[test]
fn continue_floods_are_refused() {
    // 70,000 empty CONTINUE records behind the SST: about 40 GB of moves.
    let mut g = biff::sst(&["a"]);
    for _ in 0..70_000 {
        g.extend_from_slice(&[0x3C, 0, 0, 0]);
    }
    let file = book(&g, &biff::labelsst(0, 0, 15, 0));
    let t = Instant::now();
    let r = read_xls(&file, &Limits::default());
    assert!(
        matches!(
            r,
            Err(XlsError::LimitExceeded {
                what: "CONTINUE records of the shared strings",
                ..
            })
        ),
        "{:?}",
        r.err()
    );
    assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());

    // More than a million behind any other record.
    let mut g = biff::rec(0x00EB, &[0; 8]);
    g.resize(g.len() + 4 * (prescan::MAX_CONTINUES + 1), 0);
    for c in g[12..].chunks_exact_mut(4) {
        c[0] = 0x3C;
    }
    let file = book(&g, &biff::number(0, 0, 15, 1.0));
    let t = Instant::now();
    let r = read_xls(&file, &Limits::default());
    assert!(
        matches!(
            r,
            Err(XlsError::LimitExceeded {
                what: "CONTINUE records of one record",
                ..
            })
        ),
        "{:?}",
        r.err()
    );
    assert!(t.elapsed() < Duration::from_secs(5), "{:?}", t.elapsed());
}

/// Under a code page other than UTF-16 calamine decodes the bytes of a
/// string one by one: up to six bytes of text per 16-bit character. The
/// text bound covers it.
#[test]
fn the_text_bound_holds_under_any_code_page() {
    use calamine::Reader;
    for codepage in [1200u16, 1252, 932, 65001, 1201] {
        // 0x80 0x80 per character: "€€" in windows-1252.
        let unit = [0x80u8, 0x80];
        let mut sst = vec![1u8, 0, 0, 0, 1, 0, 0, 0];
        sst.extend(words(&[2000]));
        sst.push(1);
        for _ in 0..2000 {
            sst.extend_from_slice(&unit);
        }
        let mut g = biff::rec(0x0042, &codepage.to_le_bytes());
        g.extend(biff::rec(0x00FC, &sst));
        let mut body = Vec::new();
        for r in 0..50u16 {
            body.extend(biff::labelsst(r, 0, 15, 0));
            let mut d = words(&[r, 1, 15, 2000]);
            d.push(1);
            for _ in 0..2000 {
                d.extend_from_slice(&unit);
            }
            body.extend(biff::rec(0x0204, &d));
            // ="…" with a 16-bit string of 100 characters.
            let mut rgce = vec![0x17, 100, 1];
            for _ in 0..100 {
                rgce.extend_from_slice(&unit);
            }
            body.extend(biff::formula(r, 2, 15, 1.0, &rgce));
        }
        let file = book(&g, &body);
        let (clean, scan) = prescan::sanitize(&file, 1024, 10_000_000).unwrap();
        let mut wb: calamine::Xls<_> = calamine::Xls::new(std::io::Cursor::new(clean)).unwrap();
        let sheet = wb.sheet_names()[0].clone();
        let mut built = 0u64;
        for (_, _, v) in wb.worksheet_range(&sheet).unwrap().used_cells() {
            if let calamine::Data::String(s) = v {
                built += s.len() as u64;
            }
        }
        for (_, _, f) in wb.worksheet_formula(&sheet).unwrap().used_cells() {
            built += f.len() as u64;
        }
        assert!(built > 100_000, "code page {codepage}: {built}");
        assert!(
            scan.text >= built,
            "code page {codepage}: bound {} < {built} bytes built",
            scan.text
        );
    }
}

// ------------------------------------------------------------------ container

/// calamine 0.38 finds the workbook stream whatever the case of its name;
/// so does the pre-scan (which hands calamine a `Workbook` stream).
#[test]
fn the_workbook_stream_name_ignores_case() {
    let good = book(&[], &biff::number(0, 0, 15, 1.0));
    for name in ["workbook", "WORKBOOK", "WorkBook"] {
        let mut file = good.clone();
        let old: Vec<u8> = "Workbook"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        let new: Vec<u8> = name.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let at = file.windows(old.len()).position(|w| w == old).unwrap();
        file[at..at + new.len()].copy_from_slice(&new);
        assert_eq!(read(&file).unwrap().sheets[0].cells.len(), 1, "{name}");
    }
}

// ------------------------------------------------------------------ fuzz inputs

fn fuzz_dir(kind: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fuzz")
        .join(kind)
        .join("xls")
}

/// The hostile inputs of this audit, as regression files for the fuzz
/// harness (`fuzz/regressions/xls/`, replayed by `tally-fuzz-harness`).
fn regressions() -> Vec<(&'static str, Vec<u8>)> {
    let cell = biff::number(0, 0, 15, 1.0);
    let mut out = Vec::new();
    // BIFF5 PtgStr followed by PtgAttrSpace tokens (few, to stay small).
    let mut rgce = vec![0x17, 1, b'A'];
    for _ in 0..40 {
        rgce.extend_from_slice(&[0x19, 0x40, 0x00, 0xFF]);
    }
    out.push((
        "prescan-biff5-ptgstr-attrspace",
        book5(&[], &biff::formula(0, 0, 0, 1.0, &rgce)),
    ));
    let mut g = biff::sst(&["a"]);
    g.extend(biff::rec(0x003C, &[]));
    g.extend(biff::rec(0x003C, &[1, 0, 0, b'b']));
    let mut body = biff::labelsst(0, 0, 15, 2);
    body.extend(biff::labelsst(40_000, 255, 15, 2));
    out.push(("prescan-sst-empty-continue", book(&g, &body)));
    out.push((
        "prescan-sst-string-ends-in-empty-continue",
        book(
            &{
                let mut g = biff::rec(0x00FC, &[1, 0, 0, 0, 1, 0, 0, 0, 4, 0, 0, b'a']);
                g.extend(biff::rec(0x003C, &[]));
                g
            },
            &cell,
        ),
    ));
    let mut g = name8("Broken", &area3d()[..7]);
    g.extend(name8("Good", &area3d()));
    out.push((
        "prescan-name-truncated-formula",
        book(&g, &biff::formula(0, 0, 15, 1.0, &[0x23, 2, 0, 0, 0])),
    ));
    out.push((
        "prescan-name-short-record",
        book(&biff::rec(0x0018, &[0; 14]), &cell),
    ));
    let mut body = Vec::new();
    for (row, rgce) in [
        vec![0x44, 0, 0, 0],
        vec![0x3b, 0, 0, 0, 0, 0, 0],
        vec![0x1E, 1, 0, 0x19],
        vec![0x44, 0xFF, 0xFF, 0, 0xC0],
        vec![0x23, 0, 0, 0, 0],
    ]
    .iter()
    .enumerate()
    {
        body.extend(biff::formula(row as u16, 0, 15, 42.0, rgce));
    }
    let mut lying = biff::formula(9, 0, 15, 42.0, &biff::ptg_int(7));
    lying[24..26].copy_from_slice(&500u16.to_le_bytes());
    body.extend(lying);
    out.push(("prescan-formulas-cut-inside-a-token", book(&[], &body)));
    let mut d = words(&[0, 0, 15]);
    d.extend_from_slice(&[0; 15]);
    out.push((
        "prescan-formula-short-record",
        book(&[], &biff::rec(0x0006, &d)),
    ));
    let mut m = biff::mulrk(0, 0, &[(15, 1); 10]);
    m[6..8].copy_from_slice(&65_530u16.to_le_bytes());
    let n = m.len();
    m[n - 2..].copy_from_slice(&3u16.to_le_bytes());
    out.push(("prescan-mulrk-columns-wrap", book(&[], &m)));
    let mut merged = biff::merged(&[(0, 1, 0, 1)]);
    merged[4..6].copy_from_slice(&9000u16.to_le_bytes());
    merged.extend(biff::rec(0x00E5, &[1]));
    merged.extend(biff::rec(0x0200, &[0xFF; 14]));
    merged.extend(&cell);
    out.push(("prescan-sheet-records-that-lie", book(&[], &merged)));
    let mut g = biff::rec(0x0042, &[0xB0]);
    g.extend(biff::rec(0x0022, &[1]));
    g.extend(biff::rec(0x0017, &[1]));
    let mut f = words(&[164, 300]);
    f.push(0);
    f.extend_from_slice(&[b'['; 300]);
    g.extend(biff::rec(0x041E, &f));
    out.push(("prescan-short-global-records", book(&g, &cell)));
    out.push((
        "prescan-short-boundsheet",
        book(&biff::rec(0x0085, &[0; 5]), &cell),
    ));
    out
}

/// Writes [`regressions`] to `fuzz/regressions/xls/`:
/// `cargo test -p tally-io-xls --test calamine_audit -- --ignored`.
#[test]
#[ignore = "writes fuzz/regressions/xls"]
fn write_regressions() {
    for (name, bytes) in regressions() {
        std::fs::write(fuzz_dir("regressions").join(name), bytes).unwrap();
    }
}

/// The checked-in files are the generated ones, and every regression and
/// seed of the `xls` fuzz target passes the strict check of this file:
/// calamine does not panic on what the pre-scan lets through.
#[test]
fn fuzz_inputs_never_panic_calamine() {
    for (name, bytes) in regressions() {
        let on_disk = std::fs::read(fuzz_dir("regressions").join(name)).unwrap_or_default();
        assert!(
            on_disk == bytes,
            "fuzz/regressions/xls/{name} is stale: run the ignored `write_regressions` test"
        );
    }
    let mut n = 0;
    for kind in ["regressions", "seeds"] {
        for entry in std::fs::read_dir(fuzz_dir(kind)).unwrap() {
            let path = entry.unwrap().path();
            if path.is_file() {
                let _ = read(&std::fs::read(&path).unwrap());
                n += 1;
            }
        }
    }
    assert!(n >= 12, "{n} files");
}
