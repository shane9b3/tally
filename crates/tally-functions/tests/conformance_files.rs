//! Validates `tests/conformance/**/*.toml` against the schema documented in
//! `docs/contributing-functions.md`. (The runner that evaluates the cases
//! end to end lives in tally-engine.)

#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};

use tally_functions::{registry, Category};
use tally_types::{CellAddr, ErrorKind};
use toml::{Table, Value as T};

const CASE_KEYS: &[&str] = &[
    "name",
    "cells",
    "formula",
    "origin",
    "expect",
    "tol",
    "settings",
    "note",
    "aoo_expect",
    "lo_expect",
];
const SETTINGS: &[(&str, &[&str])] = &[
    ("criteria_mode", &["wildcards", "regex", "literal"]),
    (
        "text_to_number",
        &[
            "generate_error",
            "treat_as_zero",
            "convert_unambiguous",
            "convert_locale",
        ],
    ),
    ("date_epoch", &["1899", "1900", "1904"]),
    ("case_sensitive", &[]),
    ("whole_cell_match", &[]),
    ("precision_as_shown", &[]),
];
const MIN_CASES: usize = 8;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/conformance")
}

fn files() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for dir in std::fs::read_dir(root()).unwrap() {
        let dir = dir.unwrap().path();
        if !dir.is_dir() {
            continue;
        }
        for f in std::fs::read_dir(&dir).unwrap() {
            let f = f.unwrap().path();
            if f.extension().is_some_and(|e| e == "toml") {
                out.push(f);
            }
        }
    }
    out.sort();
    out
}

fn check_cell_key(key: &str, ctx: &str) {
    let a1 = match key.split_once('.') {
        Some((sheet, a1)) => {
            assert!(!sheet.is_empty(), "{ctx}: empty sheet name in {key}");
            a1
        }
        None => key,
    };
    assert!(
        CellAddr::from_a1(0, a1).is_some(),
        "{ctx}: bad cell address {key:?}"
    );
}

fn check_value(v: &T, ctx: &str, allow_array: bool) {
    match v {
        T::Integer(_) | T::Float(_) | T::Boolean(_) | T::String(_) => {}
        T::Table(t) => {
            assert!(
                t.len() == 1 && t.get("text").is_some_and(T::is_str),
                "{ctx}: only {{ text = \"...\" }} tables are allowed, got {t:?}"
            );
        }
        T::Array(rows) if allow_array => {
            for row in rows {
                let T::Array(items) = row else {
                    panic!("{ctx}: array expectations are arrays of rows");
                };
                for item in items {
                    check_value(item, ctx, false);
                }
            }
        }
        other => panic!("{ctx}: unsupported value {other:?}"),
    }
}

fn check_case(case: &Table, ctx: &str) {
    for key in case.keys() {
        assert!(
            CASE_KEYS.contains(&key.as_str()),
            "{ctx}: unknown key {key:?}"
        );
    }
    let formula = case
        .get("formula")
        .and_then(T::as_str)
        .unwrap_or_else(|| panic!("{ctx}: formula"));
    assert!(
        formula.starts_with('='),
        "{ctx}: formula must start with '='"
    );
    for key in ["expect", "aoo_expect", "lo_expect"] {
        if let Some(v) = case.get(key) {
            check_value(v, ctx, true);
            if let T::String(s) = v {
                if s.starts_with('#') || s.starts_with("Err:") {
                    assert!(ErrorKind::parse(s).is_some(), "{ctx}: unknown error {s:?}");
                }
            }
        }
    }
    assert!(case.contains_key("expect"), "{ctx}: expect");
    if let Some(tol) = case.get("tol") {
        assert!(
            tol.as_float().is_some_and(|f| f >= 0.0) || tol.as_integer().is_some_and(|i| i >= 0),
            "{ctx}: tol must be a non-negative number"
        );
    }
    if let Some(origin) = case.get("origin") {
        let o = origin
            .as_str()
            .unwrap_or_else(|| panic!("{ctx}: origin is a string"));
        check_cell_key(o, ctx);
    }
    if let Some(cells) = case.get("cells") {
        let cells = cells
            .as_table()
            .unwrap_or_else(|| panic!("{ctx}: cells is a table"));
        for (k, v) in cells {
            check_cell_key(k, ctx);
            check_value(v, ctx, false);
        }
    }
    if let Some(settings) = case.get("settings") {
        let settings = settings
            .as_table()
            .unwrap_or_else(|| panic!("{ctx}: settings is a table"));
        for (k, v) in settings {
            let (_, allowed) = SETTINGS
                .iter()
                .find(|(name, _)| name == k)
                .unwrap_or_else(|| panic!("{ctx}: unknown setting {k:?}"));
            if allowed.is_empty() {
                assert!(v.is_bool(), "{ctx}: {k} is a boolean");
            } else {
                let s = v
                    .as_str()
                    .unwrap_or_else(|| panic!("{ctx}: {k} is a string"));
                assert!(
                    allowed.contains(&s),
                    "{ctx}: {k} = {s:?} not in {allowed:?}"
                );
            }
        }
    }
}

#[test]
fn conformance_files_follow_the_schema() {
    let files = files();
    assert!(
        !files.is_empty(),
        "no conformance files under {}",
        root().display()
    );
    for path in &files {
        let name = path.file_stem().unwrap().to_str().unwrap();
        let dir = path
            .parent()
            .unwrap()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap();
        let ctx = format!("{dir}/{name}.toml");
        assert!(
            Category::ALL.iter().any(|c| c.dir_name() == dir),
            "{ctx}: unknown category directory"
        );
        if let Some(def) = registry().def_by_name(name) {
            assert_eq!(
                def.category.dir_name(),
                dir,
                "{ctx}: wrong category directory"
            );
        }
        let doc: Table = toml::from_str(&std::fs::read_to_string(path).unwrap())
            .unwrap_or_else(|e| panic!("{ctx}: {e}"));
        for key in doc.keys() {
            assert_eq!(key, "case", "{ctx}: only [[case]] tables at top level");
        }
        let cases = doc
            .get("case")
            .and_then(T::as_array)
            .unwrap_or_else(|| panic!("{ctx}: no cases"));
        assert!(
            cases.len() >= MIN_CASES,
            "{ctx}: {} cases, need {MIN_CASES}",
            cases.len()
        );
        for (i, case) in cases.iter().enumerate() {
            let case = case
                .as_table()
                .unwrap_or_else(|| panic!("{ctx}#{i}: not a table"));
            check_case(case, &format!("{ctx} case {i}"));
        }
    }
}

#[test]
fn every_registered_function_has_vectors() {
    let files = files();
    for (_, def) in registry().iter() {
        let path = root()
            .join(def.category.dir_name())
            .join(format!("{}.toml", def.name));
        assert!(
            files.contains(&path),
            "{} has no {}",
            def.name,
            path.display()
        );
    }
}
