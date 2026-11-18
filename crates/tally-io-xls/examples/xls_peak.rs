//! Measures what the worst `.xls` inside the default `Limits` costs
//! (docs/security.md "XLS pre-scan audit"). Synthetic files only.
//!
//! ```text
//! cargo build --release -p tally-io-xls --example xls_peak
//! xls_peak gen mixed /tmp/mixed.xls
//! /usr/bin/time -l xls_peak read /tmp/mixed.xls     # maximum resident set size
//! ```
//!
//! Scenarios: `cells` (10 M `RK` cells over an 8 M-cell area), `text`
//! (the text bound used up by `LABEL`s of three-byte characters), `mixed`
//! (the text bound, then cells to the end of the stream), `continues`
//! (the most `CONTINUE` records the shared strings may have), `small`
//! (one cell: what the copies of the file alone cost). Every file is
//! padded to the largest size accepted, 256 MiB.
#![allow(clippy::unwrap_used, clippy::print_stdout)]

use std::time::Instant;

use tally_io_xls::{prescan, read_xls, Limits};

/// The largest stream whose compound file (the stream plus about 2 MiB of
/// allocation table) still fits `Limits::max_bytes`, 256 MiB.
const STREAM_BYTES: usize = prescan::MAX_STREAM_BYTES - (5 << 19);

fn rec(out: &mut Vec<u8>, t: u16, d: &[u8]) {
    out.extend_from_slice(&t.to_le_bytes());
    out.extend_from_slice(&(d.len() as u16).to_le_bytes());
    out.extend_from_slice(d);
}

fn bof(out: &mut Vec<u8>, kind: u16) {
    let mut d = Vec::new();
    for x in [0x0600u16, kind, 0x0DBB, 0x07CC] {
        d.extend_from_slice(&x.to_le_bytes());
    }
    d.extend_from_slice(&[0, 0, 0, 0, 6, 0, 0, 0]);
    rec(out, 0x0809, &d);
}

fn rk(out: &mut Vec<u8>, r: u32, c: u32) {
    let mut d = Vec::new();
    for x in [r as u16, c as u16, 0] {
        d.extend_from_slice(&x.to_le_bytes());
    }
    d.extend_from_slice(&((7i32 << 2) | 2).to_le_bytes());
    rec(out, 0x027E, &d);
}

/// `n` RK cells over rows × 256 columns (an area of at most `area`
/// cells), the cells past the area on top of the first ones.
fn cells(out: &mut Vec<u8>, n: u32, area: u32, row0: u32) {
    for i in 0..n {
        let k = i % area;
        rk(out, row0 + k / 256, k % 256);
    }
}

/// `LABEL`s of 4,000 three-byte characters until `budget` bytes of the
/// text bound (24 + 3 per character) are used; returns the next row.
fn labels(out: &mut Vec<u8>, budget: u64) -> u32 {
    let per = 24 + 3 * 4000u64;
    let n = (budget / per) as u32;
    let mut d = Vec::new();
    for r in 0..n {
        d.clear();
        for x in [(r / 256) as u16, (r % 256) as u16, 0, 4000] {
            d.extend_from_slice(&x.to_le_bytes());
        }
        d.push(1);
        for _ in 0..4000 {
            d.extend_from_slice(&0x4E2Du16.to_le_bytes());
        }
        rec(out, 0x0204, &d);
    }
    n / 256 + 1
}

fn generate(scenario: &str) -> Vec<u8> {
    let mut s = Vec::new();
    bof(&mut s, 5);
    rec(&mut s, 0x0042, &1200u16.to_le_bytes());
    if scenario == "continues" {
        // One string, then the most CONTINUE records allowed, one string
        // each.
        rec(&mut s, 0x00FC, &[1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, b'a']);
        for _ in 0..prescan::MAX_SST_CONTINUES {
            rec(&mut s, 0x003C, &[1, 0, 0, b'b']);
        }
    }
    let bound = s.len() + 4;
    rec(&mut s, 0x0085, &[0, 0, 0, 0, 0, 0, 1, 0, b'S']);
    rec(&mut s, 0x000A, &[]);
    let at = s.len() as u32;
    s[bound..bound + 4].copy_from_slice(&at.to_le_bytes());
    bof(&mut s, 0x10);
    let room = |s: &Vec<u8>| ((STREAM_BYTES - s.len() - 64) / 14) as u32;
    match scenario {
        "cells" => cells(&mut s, 10_000_000, 7_999_744, 0),
        "text" => {
            labels(&mut s, prescan::MAX_TEXT_ESTIMATE - 4096);
        }
        "mixed" => {
            let row0 = labels(&mut s, prescan::MAX_TEXT_ESTIMATE - 4096);
            let n = room(&s).min(9_900_000);
            // The labels' area and the cells' area share the 8 M budget.
            cells(&mut s, n, 7_000_000, row0);
        }
        _ => rk(&mut s, 0, 0),
    }
    rec(&mut s, 0x000A, &[]);
    // Padding nobody walks, to the largest stream.
    let junk = [0u8; 8000];
    while s.len() + 8004 <= STREAM_BYTES {
        rec(&mut s, 0x00FE, &junk);
    }
    prescan::build_container(s).0
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("gen") => {
            std::fs::write(&args[3], generate(&args[2])).unwrap();
        }
        Some("read") => {
            let bytes = std::fs::read(&args[2]).unwrap();
            let t = Instant::now();
            match read_xls(&bytes, &Limits::default()) {
                Ok(wb) => {
                    let cells: usize = wb.sheets.iter().map(|s| s.cells.len()).sum();
                    println!(
                        "{} MiB file: {} cells in {:.2} s",
                        bytes.len() >> 20,
                        cells,
                        t.elapsed().as_secs_f64()
                    );
                }
                Err(e) => println!("refused in {:.2} s: {e}", t.elapsed().as_secs_f64()),
            }
        }
        // The pre-scan alone, and the pre-scan plus calamine's decoder
        // (without Tally's cells): where the peak of `read` comes from.
        Some(stage @ ("scan" | "calamine")) => {
            let bytes = std::fs::read(&args[2]).unwrap();
            let (clean, scan) = prescan::sanitize(&bytes, 1024, 10_000_000).unwrap();
            println!("{scan:?}");
            if stage == "calamine" {
                let wb = calamine::Xls::new_with_options(
                    std::io::Cursor::new(clean),
                    Default::default(),
                );
                println!("calamine: {}", if wb.is_ok() { "read" } else { "failed" });
            }
        }
        _ => println!("usage: xls_peak gen <scenario> <file> | xls_peak read|scan|calamine <file>"),
    }
}
