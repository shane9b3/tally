//! A cheap stable-toolchain stand-in for the fuzzer: every seed in
//! `fuzz/seeds/<target>/` plus deterministic random mutations of them
//! (bit flips, byte and integer overwrites, deletions, duplications,
//! truncation, dictionary tokens from `fuzz/dict/`) through each harness.
//! It catches shallow crashes before the nightly fuzz workflow does; it is
//! no substitute for coverage-guided fuzzing.
//!
//! `TALLY_FUZZ_SMOKE_ITERS` sets the mutations per target (default 150),
//! `TALLY_FUZZ_SMOKE_SEED` the PRNG seed. A panicking input is written to
//! `$TMPDIR/tally-fuzz-smoke/<target>-<n>` and named in the failure.

#![allow(clippy::unwrap_used)] // tests may unwrap (docs/contributing.md)

mod common;

use std::path::PathBuf;

use common::{files_in, fuzz_dir, run_caught};

/// xorshift64*: deterministic, no dependency.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
}

/// The dictionary the fuzz workflow uses for `target` (libFuzzer format:
/// one `"token"` per line, `\\`, `\"` and `\xNN` escapes).
fn dictionary(target: &str) -> Vec<Vec<u8>> {
    let name = match target {
        "ods" | "ods_parts" | "fods" | "open" => "ods.dict",
        "xlsx" | "xlsx_parts" => "xlsx.dict",
        "xmlss" => "xmlss.dict",
        "formula" => "formula.dict",
        "numfmt" => "numfmt.dict",
        _ => return Vec::new(),
    };
    let Ok(text) = std::fs::read_to_string(fuzz_dir().join("dict").join(name)) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|l| l.trim().strip_prefix('"')?.strip_suffix('"'))
        .map(unescape)
        .collect()
}

fn unescape(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 1 < b.len() {
            if b[i + 1] == b'x' && i + 3 < b.len() {
                if let Ok(v) = u8::from_str_radix(&s[i + 2..i + 4], 16) {
                    out.push(v);
                    i += 4;
                    continue;
                }
            }
            out.push(b[i + 1]);
            i += 2;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    out
}

const INTERESTING_BYTES: &[u8] = &[0, 1, 0x7f, 0x80, 0xff, b'<', b'>', b'"', b'0', b'9', b'-'];
const INTERESTING_U32: &[u32] = &[0, 1, 0x7fff_ffff, 0x8000_0000, 0xffff_ffff, 0xffff, 1 << 20];

fn mutate(rng: &mut Rng, data: &mut Vec<u8>, dict: &[Vec<u8>]) {
    for _ in 0..1 + rng.below(4) {
        let len = data.len();
        match rng.below(8) {
            0 if len > 0 => {
                let i = rng.below(len);
                data[i] ^= 1 << rng.below(8);
            }
            1 if len > 0 => {
                let i = rng.below(len);
                data[i] = rng.next() as u8;
            }
            2 if len > 0 => {
                let i = rng.below(len);
                data[i] = INTERESTING_BYTES[rng.below(INTERESTING_BYTES.len())];
            }
            3 if len > 0 => {
                let at = rng.below(len);
                let n = 1 + rng.below(16.min(len - at));
                data.drain(at..at + n);
            }
            4 if len > 0 => {
                let at = rng.below(len);
                let n = 1 + rng.below(64.min(len - at));
                let chunk = data[at..at + n].to_vec();
                let to = rng.below(len + 1);
                data.splice(to..to, chunk);
            }
            5 if !dict.is_empty() => {
                let tok = &dict[rng.below(dict.len())];
                let to = rng.below(len + 1);
                data.splice(to..to, tok.iter().copied());
            }
            6 if len > 1 => data.truncate(rng.below(len)),
            7 if len >= 4 => {
                let at = rng.below(len - 3);
                let v = INTERESTING_U32[rng.below(INTERESTING_U32.len())].to_le_bytes();
                data[at..at + 4].copy_from_slice(&v);
            }
            _ => data.push(rng.next() as u8),
        }
    }
}

fn smoke(name: &str) {
    let target = tally_fuzz_harness::target(name).unwrap();
    let seeds: Vec<Vec<u8>> = files_in(&fuzz_dir().join("seeds").join(name))
        .iter()
        .map(|p| std::fs::read(p).unwrap())
        .collect();
    assert!(!seeds.is_empty(), "no seeds for {name}");
    let dict = dictionary(name);
    let iters: usize = std::env::var("TALLY_FUZZ_SMOKE_ITERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(150);
    let seed: u64 = std::env::var("TALLY_FUZZ_SMOKE_SEED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0x5eed);
    let mut rng = Rng(name
        .bytes()
        .fold(seed | 1, |h, b| h.rotate_left(5) ^ u64::from(b))
        .max(1));
    let mut failures = Vec::new();
    let out_dir = std::env::temp_dir().join("tally-fuzz-smoke");
    let mut record = |data: &[u8], msg: String, n: usize| {
        let _ = std::fs::create_dir_all(&out_dir);
        let path: PathBuf = out_dir.join(format!("{name}-{n}"));
        let _ = std::fs::write(&path, data);
        failures.push(format!("{}: {msg}", path.display()));
    };
    for (i, s) in seeds.iter().enumerate() {
        if let Err(msg) = run_caught(target.run, s) {
            record(s, msg, i);
        }
    }
    for i in 0..iters {
        let mut data = seeds[rng.below(seeds.len())].clone();
        mutate(&mut rng, &mut data, &dict);
        if let Err(msg) = run_caught(target.run, &data) {
            record(&data, msg, seeds.len() + i);
        }
    }
    assert!(
        failures.is_empty(),
        "{name}: {} panics (reproducers written):\n{}",
        failures.len(),
        failures.join("\n")
    );
}

macro_rules! smoke_tests {
    ($($t:ident),* $(,)?) => {
        $(
            #[test]
            fn $t() {
                smoke(stringify!($t));
            }
        )*

        /// The list above covers every target.
        #[test]
        fn smoke_covers_every_target() {
            let names = [$(stringify!($t)),*];
            for t in tally_fuzz_harness::TARGETS {
                assert!(names.contains(&t.name), "no smoke test for {}", t.name);
            }
        }
    };
}

smoke_tests!(
    ods, ods_parts, xlsx, xlsx_parts, xls, fods, xmlss, csv, html, dbf, dif, sylk, formula, numfmt,
    open,
);
