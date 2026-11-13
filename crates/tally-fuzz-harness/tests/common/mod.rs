//! Paths shared by the harness tests.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

/// The repository root.
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// `fuzz/` (the cargo-fuzz crate, its seeds, dictionaries and regressions).
pub fn fuzz_dir() -> PathBuf {
    repo_root().join("fuzz")
}

/// The regular files directly in `dir`, sorted; empty if `dir` is missing.
/// Dot files (`.gitkeep`) are skipped.
pub fn files_in(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut v: Vec<PathBuf> = rd
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .filter(|p| {
            !p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with('.'))
        })
        .collect();
    v.sort();
    v
}

/// The subdirectories of `dir` by name, sorted; empty if `dir` is missing.
pub fn subdirs(dir: &Path) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut v: Vec<String> = rd
        .filter_map(Result::ok)
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().to_str().map(str::to_owned))
        .collect();
    v.sort();
    v
}

/// Runs `run(data)`; `Err` with the panic message if it panicked.
pub fn run_caught(run: fn(&[u8]), data: &[u8]) -> Result<(), String> {
    std::panic::catch_unwind(|| run(data)).map_err(|e| {
        e.downcast_ref::<&str>()
            .map(|s| (*s).to_owned())
            .or_else(|| e.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "non-string panic".to_owned())
    })
}
