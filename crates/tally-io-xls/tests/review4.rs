//! Security review 4 (C4): hostile compound files and BIFF streams that
//! made calamine 0.37 allocate without bound are refused by the pre-scan
//! before calamine runs. Correctness review 4 (H10): time serials.
//!
//! The generator mirrors the reviewer's `sec4/imp/gen/gen_xls.py`.
#![allow(clippy::unwrap_used)]

use std::time::{Duration, Instant};

use tally_io_xls::{prescan, read_xls, Limits, XlsError};
use tally_model::CellContentData;
use tally_types::Value;

const ENDC: u32 = 0xFFFF_FFFE;
const FREE: u32 = 0xFFFF_FFFF;
const FATS: u32 = 0xFFFF_FFFD;

fn rec(t: u16, d: &[u8]) -> Vec<u8> {
    let mut v = t.to_le_bytes().to_vec();
    v.extend_from_slice(&(d.len() as u16).to_le_bytes());
    v.extend_from_slice(d);
    v
}

fn bof(kind: u16) -> Vec<u8> {
    let mut d = Vec::new();
    for x in [0x0600u16, kind, 0x0DBB, 0x07CC] {
        d.extend_from_slice(&x.to_le_bytes());
    }
    d.extend_from_slice(&0u32.to_le_bytes());
    d.extend_from_slice(&6u32.to_le_bytes());
    rec(0x0809, &d)
}

fn number(r: u16, c: u16, v: f64, xf: u16) -> Vec<u8> {
    let mut d = Vec::new();
    for x in [r, c, xf] {
        d.extend_from_slice(&x.to_le_bytes());
    }
    d.extend_from_slice(&v.to_le_bytes());
    rec(0x0203, &d)
}

fn dims(rf: u32, rl: u32, cf: u16, cl: u16) -> Vec<u8> {
    let mut d = Vec::new();
    d.extend_from_slice(&rf.to_le_bytes());
    d.extend_from_slice(&rl.to_le_bytes());
    for x in [cf, cl, 0] {
        d.extend_from_slice(&x.to_le_bytes());
    }
    rec(0x0200, &d)
}

/// A minimal workbook: 16 XFs (XF 15 general, XF 16 a time format) and one
/// BOUNDSHEET per body.
fn workbook(sheets: &[Vec<u8>], extra_globals: &[u8]) -> Vec<u8> {
    let mut g = bof(5);
    g.extend(rec(0x0042, &0x04B0u16.to_le_bytes()));
    // FORMAT 164 = "h:mm".
    let mut f = 164u16.to_le_bytes().to_vec();
    f.extend_from_slice(&4u16.to_le_bytes());
    f.push(0);
    f.extend_from_slice(b"h:mm");
    g.extend(rec(0x041E, &f));
    for i in 0..17u16 {
        let ifmt = if i == 16 { 164u16 } else { 0 };
        let mut d = Vec::new();
        d.extend_from_slice(&0u16.to_le_bytes());
        d.extend_from_slice(&ifmt.to_le_bytes());
        d.extend_from_slice(&(if i < 15 { 0xFFF5u16 } else { 0 }).to_le_bytes());
        d.extend_from_slice(&[0x20, 0, 0, 0]);
        d.extend_from_slice(&0u32.to_le_bytes());
        d.extend_from_slice(&0x20C0u16.to_le_bytes());
        g.extend(rec(0x00E0, &d));
    }
    g.extend_from_slice(extra_globals);
    let mut pos = Vec::new();
    for i in 0..sheets.len() {
        pos.push(g.len() + 4);
        let name = format!("S{i}");
        let mut d = vec![0u8; 6];
        d.push(name.len() as u8);
        d.push(1);
        for u in name.encode_utf16() {
            d.extend_from_slice(&u.to_le_bytes());
        }
        g.extend(rec(0x0085, &d));
    }
    g.extend(rec(0x000A, &[]));
    for (p, body) in pos.iter().zip(sheets) {
        let at = g.len() as u32;
        g[*p..*p + 4].copy_from_slice(&at.to_le_bytes());
        g.extend(bof(0x10));
        g.extend_from_slice(body);
        g.extend(rec(0x000A, &[]));
    }
    g
}

#[derive(Default)]
struct Ole {
    dir_len: Option<u32>,
    fat_patch: Vec<(usize, u32)>,
    dir_chain_loop: bool,
    difat_loop: bool,
}

fn ole(data: &[u8], o: &Ole) -> Vec<u8> {
    let sec = 512usize;
    let mut data = data.to_vec();
    if data.len() < 4096 {
        data.resize(4096, 0);
    }
    let n = data.len().div_ceil(sec);
    data.resize(n * sec, 0);
    assert!(n + 2 <= 128);
    let mut fat = vec![FATS, ENDC];
    for i in 0..n {
        fat.push(if i + 1 < n { (2 + i + 1) as u32 } else { ENDC });
    }
    fat.resize(128, FREE);
    if o.dir_chain_loop {
        fat[1] = 1;
    }
    for &(k, v) in &o.fat_patch {
        fat[k] = v;
    }
    let de = |name: &str, typ: u8, child: u32, start: u32, size: u32| {
        let mut e = vec![0u8; 128];
        let u: Vec<u16> = name.encode_utf16().collect();
        for (i, x) in u.iter().enumerate() {
            e[2 * i..2 * i + 2].copy_from_slice(&x.to_le_bytes());
        }
        e[64..66].copy_from_slice(&((u.len() as u16 + 1) * 2).to_le_bytes());
        e[66] = typ;
        e[67] = 1;
        e[68..72].copy_from_slice(&FREE.to_le_bytes());
        e[72..76].copy_from_slice(&FREE.to_le_bytes());
        e[76..80].copy_from_slice(&child.to_le_bytes());
        e[116..120].copy_from_slice(&start.to_le_bytes());
        e[120..124].copy_from_slice(&size.to_le_bytes());
        e
    };
    let mut d = de("Root Entry", 5, 1, ENDC, 0);
    d.extend(de(
        "Workbook",
        2,
        FREE,
        2,
        o.dir_len.unwrap_or(data.len() as u32),
    ));
    d.resize(sec, 0);
    let mut hdr = vec![0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
    hdr.extend_from_slice(&[0u8; 16]);
    for x in [0x3Eu16, 3, 0xFFFE, 9, 6] {
        hdr.extend_from_slice(&x.to_le_bytes());
    }
    hdr.extend_from_slice(&[0u8; 6]);
    for x in [0u32, 1, 1, 0, 4096, ENDC, 0] {
        hdr.extend_from_slice(&x.to_le_bytes());
    }
    let mut extra = Vec::new();
    if o.difat_loop {
        let k = (2 + n) as u32;
        hdr.extend_from_slice(&k.to_le_bytes());
        hdr.extend_from_slice(&1u32.to_le_bytes());
        for i in 0..128 {
            extra.extend_from_slice(&(if i == 127 { k } else { FREE }).to_le_bytes());
        }
        fat[k as usize] = 0xFFFF_FFFC;
    } else {
        hdr.extend_from_slice(&ENDC.to_le_bytes());
        hdr.extend_from_slice(&0u32.to_le_bytes());
    }
    hdr.extend_from_slice(&0u32.to_le_bytes());
    for _ in 0..108 {
        hdr.extend_from_slice(&FREE.to_le_bytes());
    }
    let mut out = hdr;
    for x in &fat {
        out.extend_from_slice(&x.to_le_bytes());
    }
    out.extend(d);
    out.extend(data);
    out.extend(extra);
    out
}

fn timed(bytes: &[u8]) -> (Result<tally_model::WorkbookData, XlsError>, Duration) {
    let t = Instant::now();
    let r = read_xls(bytes, &Limits::default());
    (r, t.elapsed())
}

#[test]
fn well_formed_generated_workbook_still_reads() {
    let b = ole(
        &workbook(
            &[[number(0, 0, 1.0, 15), number(1, 1, 2.0, 15)].concat()],
            &[],
        ),
        &Ole::default(),
    );
    let wb = read_xls(&b, &Limits::default()).unwrap();
    assert_eq!(wb.sheets[0].cells.len(), 2);
}

#[test]
fn hostile_dimensions_are_neutralized() {
    // calamine reserved rows × cols cells from DIMENSIONS (wrapping when
    // first > last): abort / > 1 GB before. The record is zeroed now.
    for d in [
        dims(0, 0xFFFF_FFFF, 0, 0xFFFF),
        dims(10, 5, 0, 0xFFFF),
        dims(0, 1_000_000, 0, 256),
    ] {
        let b = ole(
            &workbook(&[[d, number(0, 0, 1.0, 15)].concat()], &[]),
            &Ole::default(),
        );
        let (r, t) = timed(&b);
        let wb = r.unwrap();
        assert_eq!(wb.sheets[0].cells.len(), 1);
        assert!(t < Duration::from_secs(2), "{t:?}");
    }
}

#[test]
fn far_apart_cells_hit_the_dense_area_budget() {
    // (0,0) and (65535,255): 16.7M dense cells per sheet (1–1.8 GB before);
    // 8 sheets > 3 GB; (65535,65535): 4G cells (abort).
    let corner = |c: u16| [number(0, 0, 1.0, 15), number(65535, c, 1.0, 15)].concat();
    for sheets in [
        vec![corner(255)],
        vec![corner(16383)],
        vec![corner(65535)],
        vec![corner(255); 8],
    ] {
        let b = ole(&workbook(&sheets, &[]), &Ole::default());
        let (r, t) = timed(&b);
        assert!(
            matches!(r, Err(XlsError::LimitExceeded { what, .. }) if what.contains("dense")),
            "{r:?}"
        );
        assert!(t < Duration::from_secs(1), "{t:?}");
    }
    // A sheet that fits the budget is read.
    let ok = [number(0, 0, 1.0, 15), number(9999, 255, 1.0, 15)].concat();
    let b = ole(&workbook(&[ok], &[]), &Ole::default());
    assert_eq!(
        read_xls(&b, &Limits::default()).unwrap().sheets[0]
            .cells
            .len(),
        2
    );
}

#[test]
fn malformed_chains_are_refused() {
    let wb = workbook(&[number(0, 0, 1.0, 15)], &[]);
    let cases = [
        Ole {
            dir_chain_loop: true,
            ..Ole::default()
        },
        Ole {
            difat_loop: true,
            ..Ole::default()
        },
        // The stream claims 256 MiB but its chain cycles.
        Ole {
            dir_len: Some(256 * 1024 * 1024),
            fat_patch: vec![(2 + 7, 2)],
            ..Ole::default()
        },
        Ole {
            dir_len: Some(0xFFFF_FFF0),
            fat_patch: vec![(2 + 7, 2)],
            ..Ole::default()
        },
    ];
    for o in cases {
        let b = ole(&wb, &o);
        let (r, t) = timed(&b);
        assert!(r.is_err(), "{:?}", r.map(|w| w.sheets.len()));
        assert!(t < Duration::from_secs(1), "{t:?}");
    }
    // A length claim past the chain's end reads what the chain holds
    // (calamine stops at ENDOFCHAIN): no 256 MiB buffer.
    let b = ole(
        &wb,
        &Ole {
            dir_len: Some(256 * 1024 * 1024),
            ..Ole::default()
        },
    );
    let (r, t) = timed(&b);
    assert!(t < Duration::from_secs(1), "{t:?}");
    if let Ok(w) = r {
        assert_eq!(w.sheets[0].cells.len(), 1);
    }
}

#[test]
fn string_capacity_and_labelsst_copies_are_bounded() {
    // LABEL records declaring 65,535 characters with 1 byte present:
    // capacity amplification ×4,000 before; clamped now.
    let wb = workbook(
        &[{
            let mut v = Vec::new();
            for i in 0..2000u16 {
                v.extend(rec(0x0204, &{
                    let mut d = Vec::new();
                    for x in [i, 0, 15, 0xFFFF] {
                        d.extend_from_slice(&x.to_le_bytes());
                    }
                    d.push(0);
                    d.push(b'x');
                    d
                }));
            }
            v
        }],
        &[],
    );
    let (file, _) = prescan::build_container(wb);
    let (clean, scan) = prescan::sanitize(&file, 1024, 10_000_000).unwrap();
    assert_eq!(scan.cells, 2000);
    assert!(scan.text < 1_000_000, "{}", scan.text);
    let w = read_xls(&clean, &Limits::default()).unwrap();
    assert_eq!(w.sheets[0].cells.len(), 2000);
    assert_eq!(
        w.sheets[0].cells[0].content,
        CellContentData::Value(Value::text("x"))
    );

    // One 8,000-character shared string copied by 40,000 LABELSST cells:
    // 960 MB of copies before; refused by the text estimate now.
    let mut sst = Vec::new();
    sst.extend_from_slice(&1u32.to_le_bytes());
    sst.extend_from_slice(&1u32.to_le_bytes());
    sst.extend_from_slice(&8000u16.to_le_bytes());
    sst.push(0);
    sst.extend(std::iter::repeat_n(0x41u8, 8000));
    let glob = rec(0x00FC, &sst);
    let mut body = Vec::new();
    for i in 0..40_000u32 {
        let mut d = Vec::new();
        for x in [(i % 60_000) as u16, (i / 60_000) as u16, 15] {
            d.extend_from_slice(&x.to_le_bytes());
        }
        d.extend_from_slice(&0u32.to_le_bytes());
        body.extend(rec(0x00FD, &d));
    }
    let (file, _) = prescan::build_container(workbook(&[body], &glob));
    let t = Instant::now();
    let r = read_xls(&file, &Limits::default());
    assert!(
        matches!(
            r,
            Err(XlsError::LimitExceeded {
                what: "text bytes",
                ..
            })
        ),
        "{:?}",
        r.map(|w| w.sheets[0].cells.len())
    );
    assert!(t.elapsed() < Duration::from_secs(1));
}

#[test]
fn attr_space_tokens_are_neutralized() {
    // A formula of 2,000 PtgAttrSpace tokens of 255 spaces each: calamine
    // inserted 510,000 spaces one by one at the front (quadratic).
    let mut rgce = vec![0x1E, 1, 0]; // PtgInt 1
    for _ in 0..2000 {
        rgce.extend_from_slice(&[0x19, 0x40, 0x00, 0xFF]);
    }
    let mut f = Vec::new();
    for x in [0u16, 0, 15] {
        f.extend_from_slice(&x.to_le_bytes());
    }
    f.extend_from_slice(&1.0f64.to_le_bytes());
    f.extend_from_slice(&[0u8; 6]);
    f.extend_from_slice(&(rgce.len() as u16).to_le_bytes());
    f.extend_from_slice(&rgce);
    let body: Vec<u8> = (0..50).flat_map(|_| rec(0x0006, &f)).collect();
    let (file, _) = prescan::build_container(workbook(&[body], &[]));
    let t = Instant::now();
    let w = read_xls(&file, &Limits::default()).unwrap();
    assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
    match &w.sheets[0].cells[0].content {
        CellContentData::Formula { text, .. } => assert_eq!(&**text, "of:=1"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn canonical_container_round_trips_large_streams() {
    // > 109 FAT sectors needs DIFAT sectors.
    let mut body = Vec::new();
    for i in 0..400_000u32 {
        body.extend(number(
            (i % 60_000) as u16,
            (i / 60_000) as u16,
            i as f64,
            15,
        ));
    }
    let wb = workbook(&[body], &[]);
    let len = wb.len();
    let (file, off) = prescan::build_container(wb.clone());
    assert!(len > 109 * 128 * 512);
    assert_eq!(&file[off..off + len], &wb[..]);
    let back = prescan::extract_workbook_stream(&file).unwrap();
    assert_eq!(&back[..len], &wb[..]);
    let w = read_xls(&file, &Limits::default()).unwrap();
    assert_eq!(w.sheets[0].cells.len(), 400_000);
}

#[test]
fn time_serials_below_one_day_are_not_shifted() {
    // Correctness review 4, H10: 0.75 under "h:mm" stays 0.75 (18:00), a
    // date ≥ 1 still loses the 1900 leap-year bug.
    let body = [
        number(0, 0, 0.75, 16),
        number(1, 0, 45000.5, 16),
        number(2, 0, 30.25, 16),
    ]
    .concat();
    let b = ole(&workbook(&[body], &[]), &Ole::default());
    let w = read_xls(&b, &Limits::default()).unwrap();
    let v = |r: u32| match &w.sheets[0]
        .cells
        .iter()
        .find(|c| c.row == r)
        .unwrap()
        .content
    {
        CellContentData::Value(Value::Number(n)) => *n,
        o => panic!("{o:?}"),
    };
    assert_eq!(v(0), 0.75);
    assert_eq!(v(1), 45000.5);
    assert_eq!(v(2), 31.25);
}
