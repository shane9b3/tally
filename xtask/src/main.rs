//! Repository automation, run as `cargo xtask <command>` (alias in
//! `.cargo/config.toml`).
//!
//! Commands:
//! - `gen-bindings [--check]`: regenerate the ts-rs bindings into
//!   `packages/bridge/src/generated`; with `--check`, fail if they differ
//!   from the committed files.
//! - `function-coverage [--batch N] [--check] [--json] [--doc]`: compare the
//!   function registry (listed by `tally-cli functions`) with
//!   `docs/functions/master-list.toml` and print implemented/total per
//!   category and every missing name; `--json` prints the same report as
//!   JSON, `--doc` (re)writes `docs/functions/coverage.md`, and `--check`
//!   fails when anything in scope is missing.
//!
//! - `perf [options]`: release-mode performance baseline (synthetic
//!   workbooks in a temp dir); options go to the `perf` bin
//!   (`xtask/src/bin/perf.rs`), e.g. `--quick`, `--only recalc,csv`.
//!
//! The argument parser and the master-list reader are hand-rolled on
//! purpose: xtask has no dependencies (the registry is read through the CLI).

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const USAGE: &str = "\
usage: cargo xtask <command> [options]

commands:
  gen-bindings [--check]        regenerate TS bindings (--check: fail if stale)
  function-coverage [--batch N] [--check] [--json] [--doc]
                                report implemented vs AOO function list
                                (--json: machine-readable; --doc: write
                                docs/functions/coverage.md)
  perf [--quick] [--only NAMES] [--runs N] [--deep]
                                release-mode performance baseline
                                (docs/perf-baseline.md)
  help                          show this message";

/// Options of `function-coverage`.
#[derive(Debug, PartialEq, Default, Clone, Copy)]
struct CoverageOpts {
    batch: Option<u32>,
    check: bool,
    json: bool,
    doc: bool,
}

/// A parsed command line.
#[derive(Debug, PartialEq)]
enum Task {
    GenBindings { check: bool },
    FunctionCoverage(CoverageOpts),
    Perf(Vec<String>),
    Help,
}

/// Parses arguments (without the program name).
fn parse_args<I: IntoIterator<Item = String>>(args: I) -> Result<Task, String> {
    let mut args = args.into_iter();
    let cmd = args.next().ok_or_else(|| "missing command".to_owned())?;
    match cmd.as_str() {
        "perf" => Ok(Task::Perf(args.collect())),
        "gen-bindings" => {
            let mut check = false;
            for a in args {
                match a.as_str() {
                    "--check" => check = true,
                    other => return Err(format!("gen-bindings: unknown option {other}")),
                }
            }
            Ok(Task::GenBindings { check })
        }
        "function-coverage" => {
            let mut o = CoverageOpts::default();
            while let Some(a) = args.next() {
                match a.as_str() {
                    "--batch" => {
                        let v = args.next().ok_or("--batch needs a value")?;
                        o.batch = Some(v.parse().map_err(|_| format!("--batch: bad number {v}"))?);
                    }
                    "--check" => o.check = true,
                    "--json" => o.json = true,
                    "--doc" => o.doc = true,
                    other => return Err(format!("function-coverage: unknown option {other}")),
                }
            }
            if o.doc && o.batch.is_some() {
                return Err("function-coverage: --doc covers all functions, not a batch".to_owned());
            }
            Ok(Task::FunctionCoverage(o))
        }
        "help" | "-h" | "--help" => Ok(Task::Help),
        other => Err(format!("unknown command {other}")),
    }
}

/// The workspace root (the parent of this crate's directory).
fn workspace_root() -> PathBuf {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| manifest_dir.to_path_buf())
}

/// Runs a command in the workspace root, returning whether it succeeded.
fn run(cmd: &mut Command) -> Result<bool, String> {
    cmd.current_dir(workspace_root())
        .status()
        .map(|s| s.success())
        .map_err(|e| format!("failed to run {cmd:?}: {e}"))
}

fn gen_bindings(check: bool) -> Result<(), String> {
    let root = workspace_root();
    let out = root.join("packages/bridge/src/generated");
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let ok = run(Command::new(cargo)
        .args(["test", "-p", "tally-api", "--quiet"])
        .env("TS_RS_EXPORT_DIR", &out))?;
    if !ok {
        return Err("cargo test -p tally-api failed".to_owned());
    }
    if check {
        let fresh = run(Command::new("git").args([
            "diff",
            "--exit-code",
            "--",
            "packages/bridge/src/generated",
        ]))?;
        let untracked = Command::new("git")
            .args([
                "ls-files",
                "--others",
                "--exclude-standard",
                "--",
                "packages/bridge/src/generated",
            ])
            .current_dir(&root)
            .output()
            .map_err(|e| format!("git ls-files failed: {e}"))?;
        if !fresh || !untracked.stdout.is_empty() {
            return Err(
                "TS bindings are stale: run `cargo xtask gen-bindings` and commit".to_owned(),
            );
        }
        println!("bindings are up to date");
    } else {
        println!("bindings written to {}", out.display());
    }
    Ok(())
}

/// One `[[function]]` entry of the master list.
#[derive(Debug, Clone, PartialEq)]
struct MasterEntry {
    name: String,
    category: String,
    batch: Option<u32>,
    /// `source = "extension"`: not in the AOO 4.1 list (IFERROR, IFNA).
    tally_only: bool,
    /// `extension = true` (also NUMBERVALUE, listed but unimplemented by AOO).
    extension: bool,
}

/// Reads the `[[function]]` tables of `docs/functions/master-list.toml`
/// (flat `key = value` lines; `name`, `category`, `batch`, `source` and
/// `extension` are used).
fn parse_master_list(text: &str) -> Vec<MasterEntry> {
    let mut out: Vec<MasterEntry> = Vec::new();
    let mut cur: Option<MasterEntry> = None;
    for line in text.lines() {
        let line = line.trim();
        if line == "[[function]]" {
            if let Some(e) = cur.take() {
                out.push(e);
            }
            cur = Some(MasterEntry {
                name: String::new(),
                category: String::new(),
                batch: None,
                tally_only: false,
                extension: false,
            });
            continue;
        }
        let (Some(e), Some((k, v))) = (cur.as_mut(), line.split_once('=')) else {
            continue;
        };
        let v = v.trim();
        let unquoted = v.trim_matches('"').to_owned();
        match k.trim() {
            "name" => e.name = unquoted,
            "category" => e.category = unquoted,
            "batch" => e.batch = v.parse().ok(),
            "source" => e.tally_only = unquoted == "extension",
            "extension" => e.extension = v == "true",
            _ => {}
        }
    }
    if let Some(e) = cur {
        out.push(e);
    }
    out.retain(|e| !e.name.is_empty());
    out
}

/// Registered function names from `tally-cli functions` output (`NAME\tCATEGORY\tSIGNATURE`).
fn parse_registry_listing(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| l.split('\t').next())
        .map(|n| n.trim().to_ascii_uppercase())
        .filter(|n| !n.is_empty())
        .collect()
}

/// Coverage of one category.
#[derive(Debug, Clone, PartialEq)]
struct CategoryCoverage {
    category: String,
    implemented: usize,
    total: usize,
    /// Of `total`, functions that are not in AOO 4.1's list (IFERROR, IFNA).
    tally_only: usize,
    /// Every master-list name of the category, with whether it is registered.
    functions: Vec<(String, bool)>,
}

impl CategoryCoverage {
    fn missing(&self) -> Vec<&str> {
        self.functions
            .iter()
            .filter(|f| !f.1)
            .map(|f| f.0.as_str())
            .collect()
    }
}

/// The whole report.
#[derive(Debug, Clone, PartialEq)]
struct Report {
    scope: String,
    categories: Vec<CategoryCoverage>,
    /// Registered names that are not in the master list.
    unknown: Vec<String>,
}

impl Report {
    fn implemented(&self) -> usize {
        self.categories.iter().map(|c| c.implemented).sum()
    }
    fn total(&self) -> usize {
        self.categories.iter().map(|c| c.total).sum()
    }
    fn tally_only(&self) -> usize {
        self.categories.iter().map(|c| c.tally_only).sum()
    }
    fn missing(&self) -> Vec<&str> {
        self.categories.iter().flat_map(|c| c.missing()).collect()
    }
}

/// Coverage report, per category in master-list (AOO wizard) order.
fn coverage(master: &[MasterEntry], registered: &[String], batch: Option<u32>) -> Report {
    let mut cats: Vec<CategoryCoverage> = Vec::new();
    for e in master
        .iter()
        .filter(|e| batch.is_none() || e.batch == batch)
    {
        let idx = match cats.iter().position(|c| c.category == e.category) {
            Some(i) => i,
            None => {
                cats.push(CategoryCoverage {
                    category: e.category.clone(),
                    implemented: 0,
                    total: 0,
                    tally_only: 0,
                    functions: Vec::new(),
                });
                cats.len() - 1
            }
        };
        let c = &mut cats[idx];
        c.total += 1;
        c.tally_only += usize::from(e.tally_only);
        let done = registered.contains(&e.name.to_ascii_uppercase());
        c.implemented += usize::from(done);
        c.functions.push((e.name.clone(), done));
    }
    let unknown = registered
        .iter()
        .filter(|n| !master.iter().any(|e| e.name.eq_ignore_ascii_case(n)))
        .cloned()
        .collect();
    Report {
        scope: batch.map_or_else(|| "all functions".to_owned(), |b| format!("batch {b}")),
        categories: cats,
        unknown,
    }
}

fn pct(done: usize, total: usize) -> f64 {
    if total == 0 {
        100.0
    } else {
        done as f64 * 100.0 / total as f64
    }
}

/// The human-readable report.
fn render_text(r: &Report) -> String {
    let mut out = format!("function coverage ({}):\n", r.scope);
    for c in &r.categories {
        out.push_str(&format!(
            "  {:<14} {:>3}/{:<3}\n",
            c.category, c.implemented, c.total
        ));
    }
    let (done, total) = (r.implemented(), r.total());
    out.push_str(&format!(
        "total: {done}/{total} ({:.1}%; {} AOO 4.1 + {} extensions)\n",
        pct(done, total),
        total - r.tally_only(),
        r.tally_only()
    ));
    let missing = r.missing();
    if missing.is_empty() {
        out.push_str("missing: none\n");
    } else {
        out.push_str(&format!("missing ({}):\n", missing.len()));
        for c in &r.categories {
            let m = c.missing();
            if !m.is_empty() {
                out.push_str(&format!("  {}: {}\n", c.category, m.join(", ")));
            }
        }
    }
    if !r.unknown.is_empty() {
        out.push_str(&format!(
            "registered but not in the master list: {}\n",
            r.unknown.join(", ")
        ));
    }
    out
}

fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn json_list<'a>(items: impl IntoIterator<Item = &'a str>) -> String {
    let v: Vec<String> = items.into_iter().map(json_str).collect();
    format!("[{}]", v.join(", "))
}

/// The report as JSON (hand-rolled: xtask has no dependencies).
fn render_json(r: &Report) -> String {
    let cats: Vec<String> = r
        .categories
        .iter()
        .map(|c| {
            format!(
                "    {{\"category\": {}, \"implemented\": {}, \"total\": {}, \"extensions\": {}, \"missing\": {}}}",
                json_str(&c.category),
                c.implemented,
                c.total,
                c.tally_only,
                json_list(c.missing())
            )
        })
        .collect();
    format!(
        "{{\n  \"scope\": {},\n  \"implemented\": {},\n  \"total\": {},\n  \"aoo_total\": {},\n  \"extensions\": {},\n  \"missing\": {},\n  \"unknown\": {},\n  \"categories\": [\n{}\n  ]\n}}\n",
        json_str(&r.scope),
        r.implemented(),
        r.total(),
        r.total() - r.tally_only(),
        r.tally_only(),
        json_list(r.missing()),
        json_list(r.unknown.iter().map(String::as_str)),
        cats.join(",\n")
    )
}

/// `docs/functions/coverage.md`.
fn render_doc(r: &Report, master: &[MasterEntry]) -> String {
    let ext: Vec<&str> = master
        .iter()
        .filter(|e| e.extension)
        .map(|e| e.name.as_str())
        .collect();
    let (done, total) = (r.implemented(), r.total());
    let mut out = String::from(
        "<!-- GENERATED by `cargo xtask function-coverage --doc` from\n     docs/functions/master-list.toml and the registry. Do not edit by hand. -->\n\n# Function coverage\n\n",
    );
    out.push_str(&format!(
        "{done} of {total} functions implemented ({:.1}%): the {} functions of Apache OpenOffice 4.1 \
         (docs/calc-functions.json) plus {} extensions that AOO lacks. Functions marked \
         `extension = true` in the master list: {}.\n\n",
        pct(done, total),
        total - r.tally_only(),
        r.tally_only(),
        ext.join(", ")
    ));
    out.push_str("| Category | Implemented | Total | Missing |\n|---|---:|---:|---|\n");
    for c in &r.categories {
        let m = c.missing();
        out.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            c.category,
            c.implemented,
            c.total,
            if m.is_empty() {
                "-".to_owned()
            } else {
                m.join(", ")
            }
        ));
    }
    out.push_str(&format!("| **Total** | **{done}** | **{total}** | |\n"));
    for c in &r.categories {
        out.push_str(&format!(
            "\n## {} ({}/{})\n\n",
            c.category, c.implemented, c.total
        ));
        let names: Vec<String> = c
            .functions
            .iter()
            .map(|(n, ok)| {
                let mark = if ext.contains(&n.as_str()) { "\\*" } else { "" };
                if *ok {
                    format!("{n}{mark}")
                } else {
                    format!("~~{n}~~{mark}")
                }
            })
            .collect();
        out.push_str(&names.join(", "));
        out.push('\n');
    }
    out.push_str(
        "\n\\* extension (not implemented by AOO 4.1). Struck-through names are missing.\n",
    );
    out
}

fn function_coverage(o: CoverageOpts) -> Result<(), String> {
    let root = workspace_root();
    let master_path = root.join("docs/functions/master-list.toml");
    let text = std::fs::read_to_string(&master_path)
        .map_err(|e| format!("cannot read {}: {e}", master_path.display()))?;
    let master = parse_master_list(&text);
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let out = Command::new(cargo)
        .args(["run", "-q", "-p", "tally-cli", "--", "functions"])
        .current_dir(&root)
        .output()
        .map_err(|e| format!("failed to run tally-cli: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "tally-cli functions failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let registered = parse_registry_listing(&String::from_utf8_lossy(&out.stdout));
    let report = coverage(&master, &registered, o.batch);
    if o.json {
        print!("{}", render_json(&report));
    } else {
        print!("{}", render_text(&report));
        if let Some(b) = o.batch {
            let mut names: Vec<&MasterEntry> =
                master.iter().filter(|e| e.batch == Some(b)).collect();
            names.sort_by(|a, b| a.name.cmp(&b.name));
            let list: Vec<String> = names
                .iter()
                .map(|e| {
                    let mark = if registered.contains(&e.name.to_ascii_uppercase()) {
                        "+"
                    } else {
                        "-"
                    };
                    format!("{mark}{}", e.name)
                })
                .collect();
            println!("  batch {b} ({}): {}", names.len(), list.join(" "));
        }
    }
    if o.doc {
        let path = root.join("docs/functions/coverage.md");
        std::fs::write(&path, render_doc(&report, &master))
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        if !o.json {
            println!("wrote {}", path.display());
        }
    }
    let missing = report.total() - report.implemented();
    if o.check && missing > 0 {
        return Err(format!("{missing} function(s) missing in {}", report.scope));
    }
    Ok(())
}

/// Runs the `perf` bin in release mode (it needs the engine crates, which the
/// other commands avoid, so it sits behind the `perf` feature).
fn perf(args: &[String]) -> Result<(), String> {
    let mut cmd = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    cmd.current_dir(workspace_root())
        .args(["run", "--release", "--quiet", "-p", "xtask"])
        .args(["--features", "perf", "--bin", "perf", "--"])
        .args(args);
    if run(&mut cmd)? {
        Ok(())
    } else {
        Err("perf failed".into())
    }
}

fn main() -> ExitCode {
    let task = match parse_args(std::env::args().skip(1)) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("xtask: {e}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let result = match task {
        Task::Help => {
            println!("{USAGE}");
            Ok(())
        }
        Task::GenBindings { check } => gen_bindings(check),
        Task::FunctionCoverage(o) => function_coverage(o),
        Task::Perf(args) => perf(&args),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("xtask: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Task, String> {
        parse_args(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn parses_commands() {
        assert_eq!(
            parse(&["gen-bindings"]),
            Ok(Task::GenBindings { check: false })
        );
        assert_eq!(
            parse(&["gen-bindings", "--check"]),
            Ok(Task::GenBindings { check: true })
        );
        assert_eq!(
            parse(&["function-coverage"]),
            Ok(Task::FunctionCoverage(CoverageOpts::default()))
        );
        assert_eq!(
            parse(&["function-coverage", "--batch", "0", "--check"]),
            Ok(Task::FunctionCoverage(CoverageOpts {
                batch: Some(0),
                check: true,
                ..CoverageOpts::default()
            }))
        );
        assert_eq!(
            parse(&["function-coverage", "--json", "--doc"]),
            Ok(Task::FunctionCoverage(CoverageOpts {
                json: true,
                doc: true,
                ..CoverageOpts::default()
            }))
        );
        assert!(parse(&["function-coverage", "--doc", "--batch", "0"]).is_err());
        assert_eq!(parse(&["--help"]), Ok(Task::Help));
    }

    #[test]
    fn master_list_and_coverage() {
        let text = r#"
# comment
[[function]]
name = "SUM"
category = "Mathematical"
batch = 0

[[function]]
name = "ABS"
category = "Mathematical"
batch = 0

[[function]]
name = "DSUM"
category = "Database"
source = "builtin"
extension = false

[[function]]
name = "IFNA"
category = "Logical"
source = "extension"
extension = true
"#;
        let m = parse_master_list(text);
        assert_eq!(m.len(), 4);
        assert_eq!(m[0].batch, Some(0));
        assert_eq!(m[2].batch, None);
        assert!(m[3].tally_only && m[3].extension && !m[0].extension);
        let reg = parse_registry_listing("SUM\tMathematical\tSUM(number, ...)\nZZZ\tText\tZZZ()\n");
        assert_eq!(reg, vec!["SUM".to_owned(), "ZZZ".to_owned()]);
        let all = coverage(&m, &reg, None);
        assert_eq!(all.categories.len(), 3);
        assert_eq!(all.categories[0].category, "Mathematical");
        let math = &all.categories[0];
        assert_eq!((math.implemented, math.total), (1, 2));
        assert_eq!(math.missing(), vec!["ABS"]);
        assert_eq!(all.missing(), vec!["ABS", "DSUM", "IFNA"]);
        assert_eq!(
            (all.implemented(), all.total(), all.tally_only()),
            (1, 4, 1)
        );
        assert_eq!(all.unknown, vec!["ZZZ".to_owned()]);
        let b0 = coverage(&m, &reg, Some(0));
        assert_eq!(b0.categories.len(), 1);
        let text = render_text(&all);
        assert!(text.contains("total: 1/4 (25.0%; 3 AOO 4.1 + 1 extensions)"));
        assert!(text.contains("  Database: DSUM"));
        let json = render_json(&all);
        assert!(json.contains("\"missing\": [\"ABS\", \"DSUM\", \"IFNA\"]"));
        assert!(json.contains("\"unknown\": [\"ZZZ\"]"));
        let doc = render_doc(&all, &m);
        assert!(doc.contains("| Mathematical | 1 | 2 | ABS |"));
        assert!(doc.contains("~~IFNA~~\\*"));
        assert_eq!(json_str("a\"b\\"), "\"a\\\"b\\\\\"");
    }

    #[test]
    fn rejects_bad_input() {
        assert!(parse(&[]).is_err());
        assert!(parse(&["frobnicate"]).is_err());
        assert!(parse(&["gen-bindings", "--nope"]).is_err());
        assert!(parse(&["function-coverage", "--batch"]).is_err());
        assert!(parse(&["function-coverage", "--batch", "x"]).is_err());
    }
}
