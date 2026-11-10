//! End-to-end conformance runner for `tests/conformance/**/*.toml` (schema in
//! `docs/contributing-functions.md`). Every `[[case]]` is a sub-check: the
//! input cells and the formula are entered as typed, the workbook is
//! recalculated and the formula cell's value compared with `expect`.
//!
//! A case that calls a function not yet in the registry is SKIPPED (other
//! engineers are adding functions concurrently); the skip count is printed.
#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};

use tally_engine::eval::{array_result, eval_in_array_context};
use tally_engine::parallel::Threading;
use tally_engine::recalc::{recalc, CalcCtx};
use tally_engine::Workbook;
use tally_formula::{Expr, FuncRef};
use tally_functions::registry;
use tally_types::{
    approx_equal, CellAddr, CriteriaMode, DateEpoch, ErrorKind, TextToNumber, Value,
};
use toml::Value as Toml;

/// Cases whose vector or function is known to be wrong upstream (owned by
/// the WP3 function authors). They are reported as KNOWN, not as failures;
/// remove an entry once it passes. `(file relative to the repo, 1-based case
/// index, why)`.
const KNOWN_DEVIATIONS: &[(&str, usize, &str)] = &[];

fn repo_root() -> PathBuf {
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    here.ancestors()
        .nth(2)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| here.to_path_buf())
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            walk(&p, out);
        } else if p.extension().is_some_and(|e| e == "toml") {
            out.push(p);
        }
    }
}

enum Outcome {
    Pass,
    Skip(String),
    Fail(String),
}

fn apply_settings(wb: &mut Workbook, t: &toml::Table) -> Result<(), String> {
    for (k, v) in t {
        let s = &mut wb.settings;
        match (k.as_str(), v) {
            ("criteria_mode", Toml::String(m)) => {
                s.criteria_mode = match m.as_str() {
                    "wildcards" => CriteriaMode::Wildcards,
                    "regex" => CriteriaMode::Regex,
                    "literal" => CriteriaMode::Literal,
                    o => return Err(format!("bad criteria_mode {o}")),
                }
            }
            ("text_to_number", Toml::String(m)) => {
                s.text_to_number = match m.as_str() {
                    "generate_error" => TextToNumber::GenerateError,
                    "treat_as_zero" => TextToNumber::TreatAsZero,
                    "convert_unambiguous" => TextToNumber::ConvertUnambiguous,
                    "convert_locale" => TextToNumber::ConvertLocale,
                    o => return Err(format!("bad text_to_number {o}")),
                }
            }
            ("date_epoch", Toml::String(m)) => {
                s.date_epoch = match m.as_str() {
                    "1899" => DateEpoch::D1899,
                    "1900" => DateEpoch::D1900,
                    "1904" => DateEpoch::D1904,
                    o => return Err(format!("bad date_epoch {o}")),
                }
            }
            ("case_sensitive", Toml::Boolean(b)) => s.case_sensitive = *b,
            ("whole_cell_match", Toml::Boolean(b)) => s.whole_cell_match = *b,
            ("precision_as_shown", Toml::Boolean(b)) => s.precision_as_shown = *b,
            (o, _) => return Err(format!("unknown setting {o}")),
        }
    }
    Ok(())
}

fn unknown_function(wb: &Workbook, src: &str, origin: CellAddr) -> Option<String> {
    let f = wb.parse_formula(src, origin).ok()?;
    let mut name = None;
    f.expr.walk(|e| {
        if let Expr::Call {
            func: FuncRef::Unknown(n),
            ..
        } = e
        {
            name.get_or_insert_with(|| n.to_string());
        }
    });
    name
}

fn cell_addr(wb: &mut Workbook, key: &str) -> Result<CellAddr, String> {
    let (sheet_name, a1) = match key.rsplit_once(['.', '!']) {
        Some((s, a)) => (Some(s.trim_matches('\'')), a),
        None => (None, key),
    };
    let sheet = match sheet_name {
        None => wb.sheets[0].id,
        Some(n) => match wb.sheet_by_name_ci(n) {
            Some(id) => id,
            None => wb.add_sheet(Some(n), None),
        },
    };
    CellAddr::from_a1(sheet, a1).ok_or_else(|| format!("bad cell key {key}"))
}

fn expected(v: &Toml) -> Result<Value, String> {
    Ok(match v {
        Toml::Integer(i) => Value::Number(*i as f64),
        Toml::Float(f) => Value::Number(*f),
        Toml::Boolean(b) => Value::Bool(*b),
        Toml::String(s) => match ErrorKind::parse(s) {
            Some(e) => Value::Error(e),
            None => Value::text(s.as_str()),
        },
        Toml::Table(t) => match t.get("text") {
            Some(Toml::String(s)) => Value::text(s.as_str()),
            _ => return Err("expect table needs `text`".into()),
        },
        Toml::Array(rows) => {
            let mut data = Vec::new();
            let mut cols = 0u32;
            for row in rows {
                let Toml::Array(items) = row else {
                    return Err("array expect must be rows".into());
                };
                cols = items.len() as u32;
                for it in items {
                    data.push(expected(it)?);
                }
            }
            let m = tally_types::Matrix::from_vec(rows.len() as u32, cols, data)
                .ok_or("ragged array expect")?;
            Value::Array(std::sync::Arc::new(m))
        }
        Toml::Datetime(_) => return Err("datetime expect is not supported".into()),
    })
}

fn same(actual: &Value, expect: &Value, tol: Option<f64>) -> bool {
    match (actual, expect) {
        (Value::Number(a), Value::Number(e)) => match tol {
            Some(0.0) => a.to_bits() == e.to_bits() || a == e,
            Some(t) => (a - e).abs() <= t,
            None => approx_equal(*a, *e),
        },
        (Value::Array(a), Value::Array(e)) => {
            a.rows == e.rows
                && a.cols == e.cols
                && a.data.iter().zip(&e.data).all(|(x, y)| same(x, y, tol))
        }
        (a, e) => a == e,
    }
}

fn run_case(stem: &str, case: &toml::Table, threading: Threading) -> Outcome {
    let Some(Toml::String(formula)) = case.get("formula") else {
        return Outcome::Fail("missing formula".into());
    };
    let Some(expect_raw) = case.get("expect") else {
        return Outcome::Fail("missing expect".into());
    };
    let expect = match expected(expect_raw) {
        Ok(v) => v,
        Err(e) => return Outcome::Fail(e),
    };
    let tol = case.get("tol").and_then(|t| match t {
        Toml::Float(f) => Some(*f),
        Toml::Integer(i) => Some(*i as f64),
        _ => None,
    });
    let mut wb = Workbook::new();
    wb.threading = threading;
    if let Some(Toml::Table(s)) = case.get("settings") {
        if let Err(e) = apply_settings(&mut wb, s) {
            return Outcome::Fail(e);
        }
    }
    let origin_key = match case.get("origin") {
        Some(Toml::String(s)) => s.clone(),
        _ => "Z100".to_owned(),
    };
    let origin = match cell_addr(&mut wb, &origin_key) {
        Ok(a) => a,
        Err(e) => return Outcome::Fail(e),
    };
    if registry().lookup(stem).is_none() {
        return Outcome::Skip(format!("{stem} is not registered"));
    }
    if let Some(Toml::Table(cells)) = case.get("cells") {
        for (k, v) in cells {
            let addr = match cell_addr(&mut wb, k) {
                Ok(a) => a,
                Err(e) => return Outcome::Fail(e),
            };
            match v {
                Toml::Integer(i) => wb.set_value(addr, Value::Number(*i as f64)),
                Toml::Float(f) => wb.set_value(addr, Value::Number(*f)),
                Toml::Boolean(b) => wb.set_value(addr, Value::Bool(*b)),
                Toml::String(s) if s.starts_with('=') => {
                    if let Some(n) = unknown_function(&wb, s, addr) {
                        return Outcome::Skip(format!("{n} is not registered"));
                    }
                    wb.set_input(addr, s);
                }
                Toml::String(s) => wb.set_value(addr, Value::text(s.as_str())),
                Toml::Table(t) => match t.get("text") {
                    Some(Toml::String(s)) => wb.set_value(addr, Value::text(s.as_str())),
                    _ => return Outcome::Fail(format!("cell {k}: table needs `text`")),
                },
                other => return Outcome::Fail(format!("cell {k}: unsupported value {other}")),
            }
        }
    }
    if let Some(n) = unknown_function(&wb, formula, origin) {
        return Outcome::Skip(format!("{n} is not registered"));
    }
    let actual = if matches!(expect, Value::Array(_)) {
        recalc(&mut wb);
        let f = match wb.parse_formula(formula, origin) {
            Ok(f) => f,
            Err(e) => return Outcome::Fail(format!("parse error {e}")),
        };
        let mut ctx = CalcCtx::at(&mut wb, origin);
        let v = eval_in_array_context(&f.expr, &mut ctx);
        array_result(&v, &mut ctx)
    } else {
        wb.set_input(origin, formula);
        recalc(&mut wb);
        wb.value(origin)
    };
    if same(&actual, &expect, tol) {
        Outcome::Pass
    } else {
        Outcome::Fail(format!(
            "formula {formula}: expected {expect:?}, got {actual:?}"
        ))
    }
}

#[test]
fn conformance_vectors() {
    run_all(Threading::Sequential);
}

/// The same vectors with the parallel recalc pre-pass forced
/// (docs/recalc-parallel.md): every formula of every case goes through the
/// multithreaded path, which must give the same results.
#[test]
fn conformance_vectors_parallel() {
    run_all(Threading::Parallel);
}

fn run_all(threading: Threading) {
    let dir = repo_root().join("tests/conformance");
    let mut files = Vec::new();
    walk(&dir, &mut files);
    assert!(!files.is_empty(), "no vectors under {}", dir.display());
    let (mut pass, mut skip, mut known) = (0usize, 0usize, 0usize);
    let mut failures: Vec<String> = Vec::new();
    let mut skipped_funcs: Vec<String> = Vec::new();
    for file in &files {
        let rel = file
            .strip_prefix(repo_root())
            .unwrap_or(file)
            .display()
            .to_string();
        let text = match std::fs::read_to_string(file) {
            Ok(t) => t,
            Err(e) => {
                failures.push(format!("{rel}: {e}"));
                continue;
            }
        };
        let table: toml::Table = match text.parse() {
            Ok(t) => t,
            Err(e) => {
                failures.push(format!("{rel}: TOML error {e}"));
                continue;
            }
        };
        let stem = file
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let Some(Toml::Array(cases)) = table.get("case") else {
            failures.push(format!("{rel}: no [[case]]"));
            continue;
        };
        for (i, case) in cases.iter().enumerate() {
            let Toml::Table(case) = case else {
                failures.push(format!("{rel} case {}: not a table", i + 1));
                continue;
            };
            let name = case
                .get("name")
                .and_then(|n| n.as_str())
                .map(|n| format!(" ({n})"))
                .unwrap_or_default();
            match run_case(&stem, case, threading) {
                Outcome::Pass => pass += 1,
                Outcome::Skip(why) => {
                    skip += 1;
                    if !skipped_funcs.contains(&why) {
                        skipped_funcs.push(why);
                    }
                }
                Outcome::Fail(msg) => {
                    let rel_unix = rel.replace('\\', "/");
                    if let Some((_, _, why)) = KNOWN_DEVIATIONS
                        .iter()
                        .find(|(f, n, _)| *f == rel_unix && *n == i + 1)
                    {
                        known += 1;
                        println!("KNOWN {rel} case {}{name}: {msg} [{why}]", i + 1);
                    } else {
                        failures.push(format!("{rel} case {}{name}: {msg}", i + 1));
                    }
                }
            }
        }
    }
    println!(
        "conformance ({threading:?}): {} files, {pass} passed, {skip} SKIPPED, {known} KNOWN, {} failed",
        files.len(),
        failures.len()
    );
    if !skipped_funcs.is_empty() {
        println!("skipped because: {}", skipped_funcs.join("; "));
    }
    for f in &failures {
        println!("FAIL {f}");
    }
    assert!(
        failures.is_empty(),
        "{} conformance case(s) failed",
        failures.len()
    );
}
