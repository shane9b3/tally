//! Replays the fuzz seeds and every checked-in fuzz finding on stable.
//!
//! A crash found by the fuzz workflow (`.github/workflows/fuzz.yml`) is
//! minimised (`cargo fuzz tmin <target> <crash>`), fixed, and its file
//! copied to `fuzz/regressions/<target>/`; from then on this test runs it
//! through the same harness as the fuzzer on every `cargo test`.

#![allow(clippy::unwrap_used)] // tests may unwrap (docs/contributing.md)

mod common;

use common::{files_in, fuzz_dir, run_caught, subdirs};

/// Every directory under `fuzz/seeds/` and `fuzz/regressions/` names a
/// target, so a misspelt directory cannot silently skip its files.
#[test]
fn every_input_directory_names_a_target() {
    for kind in ["seeds", "regressions"] {
        for name in subdirs(&fuzz_dir().join(kind)) {
            assert!(
                tally_fuzz_harness::target(&name).is_some(),
                "fuzz/{kind}/{name} is not a target in tally_fuzz_harness::TARGETS"
            );
        }
    }
}

/// Every target has seeds (the fuzz workflow starts from them).
#[test]
fn every_target_has_seeds() {
    for t in tally_fuzz_harness::TARGETS {
        assert!(
            !files_in(&fuzz_dir().join("seeds").join(t.name)).is_empty(),
            "fuzz/seeds/{} is empty",
            t.name
        );
    }
}

fn replay(kind: &str) {
    let mut failures = Vec::new();
    let mut count = 0;
    for t in tally_fuzz_harness::TARGETS {
        for path in files_in(&fuzz_dir().join(kind).join(t.name)) {
            let data = std::fs::read(&path).unwrap();
            count += 1;
            if let Err(msg) = run_caught(t.run, &data) {
                failures.push(format!("{}: {msg}", path.display()));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {count} {kind} inputs panicked:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn seeds_do_not_panic() {
    replay("seeds");
}

#[test]
fn regressions_do_not_panic() {
    replay("regressions");
}

/// The structure-aware harness round-trips a package's parts.
#[test]
fn parts_round_trip() {
    let parts = b"content.xml\0<a/>\0styles.xml\0<b/>";
    let zip = tally_fuzz_harness::build_package(parts, Some("application/x-test")).unwrap();
    let back = tally_fuzz_harness::package_to_parts(&zip).unwrap();
    assert_eq!(
        back,
        b"mimetype\0application/x-test\0content.xml\0<a/>\0styles.xml\0<b/>"
    );
}
