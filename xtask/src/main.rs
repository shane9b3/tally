//! Repository automation, run as `cargo xtask <command>` (alias in
//! `.cargo/config.toml`).
//!
//! Commands:
//! - `gen-bindings [--check]`: regenerate the ts-rs bindings into
//!   `packages/bridge/src/generated`; with `--check`, fail if they differ
//!   from the committed files.
//! - `function-coverage [--batch N] [--check]`: compare the function registry
//!   (listed by `tally-cli functions`) with `docs/functions/master-list.toml`
//!   and print implemented/missing per category; `--check` fails when
//!   anything in scope is missing.
//!
//! The argument parser and the master-list reader are hand-rolled on
//! purpose: xtask has no dependencies (the registry is read through the CLI).

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const USAGE: &str = "\
usage: cargo xtask <command> [options]

commands:
  gen-bindings [--check]        regenerate TS bindings (--check: fail if stale)
  function-coverage [--batch N] [--check]
                                report implemented vs AOO function list
  help                          show this message";

/// A parsed command line.
#[derive(Debug, PartialEq)]
enum Task {
    GenBindings { check: bool },
    FunctionCoverage { batch: Option<u32>, check: bool },
    Help,
}

/// Parses arguments (without the program name).
fn parse_args<I: IntoIterator<Item = String>>(args: I) -> Result<Task, String> {
    let mut args = args.into_iter();
    let cmd = args.next().ok_or_else(|| "missing command".to_owned())?;
    match cmd.as_str() {
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
            let mut batch = None;
            let mut check = false;
            while let Some(a) = args.next() {
                match a.as_str() {
                    "--batch" => {
                        let v = args.next().ok_or("--batch needs a value")?;
                        batch = Some(v.parse().map_err(|_| format!("--batch: bad number {v}"))?);
                    }
                    "--check" => check = true,
                    other => return Err(format!("function-coverage: unknown option {other}")),
                }
            }
            Ok(Task::FunctionCoverage { batch, check })
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
}

/// Reads the `[[function]]` tables of `docs/functions/master-list.toml`
/// (flat `key = value` lines; only `name`, `category` and `batch` are used).
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

/// Coverage report: per category `(implemented, total, missing names)`.
fn coverage(
    master: &[MasterEntry],
    registered: &[String],
    batch: Option<u32>,
) -> Vec<(String, usize, usize, Vec<String>)> {
    let mut cats: Vec<(String, usize, usize, Vec<String>)> = Vec::new();
    for e in master
        .iter()
        .filter(|e| batch.is_none() || e.batch == batch)
    {
        let idx = match cats.iter().position(|c| c.0 == e.category) {
            Some(i) => i,
            None => {
                cats.push((e.category.clone(), 0, 0, Vec::new()));
                cats.len() - 1
            }
        };
        let c = &mut cats[idx];
        c.2 += 1;
        if registered.contains(&e.name.to_ascii_uppercase()) {
            c.1 += 1;
        } else {
            c.3.push(e.name.clone());
        }
    }
    cats.sort_by(|a, b| a.0.cmp(&b.0));
    cats
}

fn function_coverage(batch: Option<u32>, check: bool) -> Result<(), String> {
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
    let scope = batch.map_or_else(|| "all functions".to_owned(), |b| format!("batch {b}"));
    let report = coverage(&master, &registered, batch);
    let (mut done, mut total) = (0usize, 0usize);
    println!("function coverage ({scope}):");
    for (cat, d, t, missing) in &report {
        done += d;
        total += t;
        println!("  {cat:<14} {d:>3}/{t:<3}");
        if !missing.is_empty() && (batch.is_some() || missing.len() <= 12) {
            println!("      missing: {}", missing.join(", "));
        }
    }
    if let Some(b) = batch {
        let mut names: Vec<&MasterEntry> = master.iter().filter(|e| e.batch == Some(b)).collect();
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
    let pct = if total == 0 {
        100.0
    } else {
        done as f64 * 100.0 / total as f64
    };
    println!("total: {done}/{total} ({pct:.1}%)");
    let unknown: Vec<&String> = registered
        .iter()
        .filter(|n| !master.iter().any(|e| e.name.eq_ignore_ascii_case(n)))
        .collect();
    if !unknown.is_empty() {
        println!(
            "registered but not in the master list: {}",
            unknown
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    if check && done < total {
        return Err(format!("{} function(s) missing in {scope}", total - done));
    }
    Ok(())
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
        Task::FunctionCoverage { batch, check } => function_coverage(batch, check),
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
            Ok(Task::FunctionCoverage {
                batch: None,
                check: false
            })
        );
        assert_eq!(
            parse(&["function-coverage", "--batch", "0", "--check"]),
            Ok(Task::FunctionCoverage {
                batch: Some(0),
                check: true
            })
        );
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
"#;
        let m = parse_master_list(text);
        assert_eq!(m.len(), 3);
        assert_eq!(m[0].batch, Some(0));
        assert_eq!(m[2].batch, None);
        let reg = parse_registry_listing("SUM\tMathematical\tSUM(number, ...)\n");
        assert_eq!(reg, vec!["SUM".to_owned()]);
        let all = coverage(&m, &reg, None);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].0, "Database");
        assert_eq!((all[1].1, all[1].2), (1, 2));
        assert_eq!(all[1].3, vec!["ABS".to_owned()]);
        let b0 = coverage(&m, &reg, Some(0));
        assert_eq!(b0.len(), 1);
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
