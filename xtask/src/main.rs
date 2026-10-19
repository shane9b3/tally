//! Repository automation, run as `cargo xtask <command>` (alias in
//! `.cargo/config.toml`).
//!
//! Commands:
//! - `gen-bindings [--check]`: regenerate the ts-rs bindings into
//!   `packages/bridge/src/generated`; with `--check`, fail if they differ
//!   from the committed files.
//! - `function-coverage [--batch N]`: compare the function registry with
//!   `docs/functions/master-list.toml` (stub until WP3).
//!
//! The argument parser is hand-rolled on purpose: xtask has no dependencies.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const USAGE: &str = "\
usage: cargo xtask <command> [options]

commands:
  gen-bindings [--check]        regenerate TS bindings (--check: fail if stale)
  function-coverage [--batch N] report implemented vs AOO function list
  help                          show this message";

/// A parsed command line.
#[derive(Debug, PartialEq)]
enum Task {
    GenBindings { check: bool },
    FunctionCoverage { batch: Option<u32> },
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
            while let Some(a) = args.next() {
                match a.as_str() {
                    "--batch" => {
                        let v = args.next().ok_or("--batch needs a value")?;
                        batch = Some(v.parse().map_err(|_| format!("--batch: bad number {v}"))?);
                    }
                    other => return Err(format!("function-coverage: unknown option {other}")),
                }
            }
            Ok(Task::FunctionCoverage { batch })
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
        Task::FunctionCoverage { batch } => {
            // TODO(WP3): compare tally_functions::registry() with
            // docs/functions/master-list.toml and print per-category coverage.
            let scope = batch.map_or_else(|| "all".to_owned(), |b| format!("batch {b}"));
            println!("function-coverage ({scope}): not implemented yet (WP3)");
            Ok(())
        }
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
            Ok(Task::FunctionCoverage { batch: None })
        );
        assert_eq!(
            parse(&["function-coverage", "--batch", "0"]),
            Ok(Task::FunctionCoverage { batch: Some(0) })
        );
        assert_eq!(parse(&["--help"]), Ok(Task::Help));
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
