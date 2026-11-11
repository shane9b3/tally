//! `cargo xtask bump-version <x.y.z>` and `bump-version --check [x.y.z]`
//! (docs/release.md): the app version lives in several files that must
//! agree, because the updater compares it with the release's
//! `latest.json` and the release workflow compares it with the tag:
//!
//! - `Cargo.toml`: `[workspace.package] version` and the `version` of
//!   every internal crate in `[workspace.dependencies]` (a path dependency
//!   whose version requirement no longer matches would not resolve);
//! - `Cargo.lock`: every workspace member (packages without `source`);
//! - `apps/desktop/src-tauri/tauri.conf.json`: the top-level `version`
//!   (what the built app reports, and what the updater compares);
//! - every `package.json` under `apps/` and `packages/` that has one.
//!
//! The edits are textual (no TOML/JSON rewriting), so formatting and key
//! order stay as they are. xtask stays dependency-free.

use std::fs;
use std::path::{Path, PathBuf};

/// A strict `MAJOR.MINOR.PATCH` with an optional `-prerelease`
/// (`[0-9A-Za-z.-]`); no `v` prefix, no build metadata.
pub fn is_version(v: &str) -> bool {
    let (core, pre) = match v.split_once('-') {
        Some((c, p)) => (c, Some(p)),
        None => (v, None),
    };
    let parts: Vec<&str> = core.split('.').collect();
    let numeric = |p: &&str| {
        !p.is_empty()
            && p.bytes().all(|b| b.is_ascii_digit())
            && (p.len() == 1 || !p.starts_with('0'))
    };
    parts.len() == 3
        && parts.iter().all(numeric)
        && pre.is_none_or(|p| {
            !p.is_empty()
                && p.split('.').all(|id| {
                    !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                })
        })
}

/// One place a version is written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// File (relative to the root) and what in it.
    pub what: String,
    pub version: String,
}

/// The value of the first `key = "…"` line within the TOML `[section]`.
#[cfg(test)]
fn toml_section_value<'a>(text: &'a str, section: &str, key: &str) -> Option<&'a str> {
    let mut inside = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            inside = t == section;
            continue;
        }
        if inside {
            if let Some(rest) = t.strip_prefix(key) {
                let rest = rest.trim_start();
                if let Some(v) = rest.strip_prefix('=') {
                    return v.trim().strip_prefix('"')?.split('"').next();
                }
            }
        }
    }
    None
}

/// Rewrites `key = "…"` lines within `[section]` for which `pick(line)`
/// holds; returns the new text and the old values.
fn rewrite_toml(
    text: &str,
    section: &str,
    pick: impl Fn(&str) -> bool,
    new: Option<&str>,
) -> (String, Vec<(String, String)>) {
    let mut out = String::with_capacity(text.len());
    let mut found = Vec::new();
    let mut inside = false;
    for line in text.split_inclusive('\n') {
        let t = line.trim();
        if t.starts_with('[') {
            inside = t == section;
        }
        let mut replaced = None;
        if inside && !t.starts_with('#') && pick(t) {
            if let Some((name, old, start, end)) = version_field(line) {
                found.push((name, old.to_owned()));
                if let Some(v) = new {
                    replaced = Some(format!("{}{v}{}", &line[..start], &line[end..]));
                }
            }
        }
        out.push_str(replaced.as_deref().unwrap_or(line));
    }
    (out, found)
}

/// In `name = { …, version = "x" }` or `version = "x"`: the line's key,
/// the version and its byte range.
fn version_field(line: &str) -> Option<(String, &str, usize, usize)> {
    let name = line.split('=').next()?.trim().to_owned();
    let at = if name == "version" {
        line.find("version")?
    } else {
        line.find("version =").or_else(|| line.find("version="))?
    };
    let open = at + line[at..].find('"')? + 1;
    let close = open + line[open..].find('"')?;
    Some((name, &line[open..close], open, close))
}

/// `Cargo.toml`: the workspace version and the internal crates' requirements.
pub fn cargo_toml(text: &str, new: Option<&str>) -> (String, Vec<Found>) {
    let (text, ws) = rewrite_toml(
        text,
        "[workspace.package]",
        |t| t.starts_with("version"),
        new,
    );
    let (text, deps) = rewrite_toml(
        &text,
        "[workspace.dependencies]",
        |t| t.contains("path = \"crates/"),
        new,
    );
    let found = ws
        .into_iter()
        .map(|(_, v)| Found {
            what: "Cargo.toml [workspace.package] version".to_owned(),
            version: v,
        })
        .chain(deps.into_iter().map(|(name, v)| Found {
            what: format!("Cargo.toml [workspace.dependencies] {name}"),
            version: v,
        }))
        .collect();
    (text, found)
}

/// `Cargo.lock`: the workspace members (`[[package]]` without `source`).
pub fn cargo_lock(text: &str, new: Option<&str>) -> (String, Vec<Found>) {
    let mut out = String::with_capacity(text.len());
    let mut found = Vec::new();
    for (i, block) in text.split("\n[[package]]\n").enumerate() {
        if i > 0 {
            out.push_str("\n[[package]]\n");
        }
        let is_member = i > 0 && !block.lines().any(|l| l.starts_with("source = "));
        if !is_member {
            out.push_str(block);
            continue;
        }
        let name = block
            .lines()
            .find_map(|l| l.strip_prefix("name = \""))
            .and_then(|r| r.strip_suffix('"'))
            .unwrap_or("?")
            .to_owned();
        let mut done = false;
        for line in block.split_inclusive('\n') {
            match line.strip_prefix("version = \"") {
                Some(rest) if !done => {
                    done = true;
                    let old = rest.trim_end().trim_end_matches('"');
                    found.push(Found {
                        what: format!("Cargo.lock {name}"),
                        version: old.to_owned(),
                    });
                    match new {
                        Some(v) => {
                            let nl = if line.ends_with('\n') { "\n" } else { "" };
                            out.push_str(&format!("version = \"{v}\"{nl}"));
                        }
                        None => out.push_str(line),
                    }
                }
                _ => out.push_str(line),
            }
        }
    }
    (out, found)
}

/// In `"key": "value",`: the value and its byte range.
fn json_string_after_colon(line: &str) -> Option<(&str, usize, usize)> {
    let colon = line.find(':')?;
    let open = colon + line[colon..].find('"')? + 1;
    let close = open + line[open..].find('"')?;
    Some((&line[open..close], open, close))
}

/// A JSON file's top-level `"version": "…"` (the first one at two-space
/// indentation, as the repo's files are formatted).
pub fn json_version(text: &str, what: &str, new: Option<&str>) -> (String, Vec<Found>) {
    let mut out = String::with_capacity(text.len());
    let mut found = Vec::new();
    for line in text.split_inclusive('\n') {
        if found.is_empty() && line.starts_with("  \"version\":") {
            if let Some((old, start, end)) = json_string_after_colon(line) {
                found.push(Found {
                    what: format!("{what} version"),
                    version: old.to_owned(),
                });
                if let Some(v) = new {
                    out.push_str(&format!("{}{v}{}", &line[..start], &line[end..]));
                    continue;
                }
            }
        }
        out.push_str(line);
    }
    (out, found)
}

/// The `package.json` files under `apps/*` and `packages/*` (sorted).
fn package_jsons(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for dir in ["apps", "packages"] {
        let Ok(entries) = fs::read_dir(root.join(dir)) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path().join("package.json");
            if p.is_file() {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// Every file the version lives in, with its rewrite.
type Rewrite = fn(&str, &str, Option<&str>) -> (String, Vec<Found>);

fn files(root: &Path) -> Vec<(PathBuf, String, Rewrite)> {
    let mut v: Vec<(PathBuf, String, Rewrite)> = vec![
        (root.join("Cargo.toml"), "Cargo.toml".into(), |t, _, n| {
            cargo_toml(t, n)
        }),
        (root.join("Cargo.lock"), "Cargo.lock".into(), |t, _, n| {
            cargo_lock(t, n)
        }),
        (
            root.join("apps/desktop/src-tauri/tauri.conf.json"),
            "apps/desktop/src-tauri/tauri.conf.json".into(),
            json_version,
        ),
    ];
    for p in package_jsons(root) {
        let rel = p
            .strip_prefix(root)
            .unwrap_or(&p)
            .to_string_lossy()
            .replace('\\', "/");
        v.push((p, rel, json_version));
    }
    v
}

/// The versions found everywhere (or an error naming a file without one).
pub fn collect(root: &Path) -> Result<Vec<Found>, String> {
    let mut all = Vec::new();
    for (path, rel, rewrite) in files(root) {
        let text = fs::read_to_string(&path).map_err(|e| format!("reading {rel}: {e}"))?;
        let (_, found) = rewrite(&text, &rel, None);
        if found.is_empty() && !rel.ends_with("package.json") {
            return Err(format!("{rel}: no version found"));
        }
        all.extend(found);
    }
    Ok(all)
}

/// `--check`: every version agrees (and equals `expected` when given).
/// Returns the version.
pub fn check(found: &[Found], expected: Option<&str>) -> Result<String, String> {
    let first = found
        .first()
        .map(|f| f.version.clone())
        .ok_or("no versions found")?;
    let want = expected.map_or(first, str::to_owned);
    let wrong: Vec<String> = found
        .iter()
        .filter(|f| f.version != want)
        .map(|f| format!("  {}: {} (expected {want})", f.what, f.version))
        .collect();
    if wrong.is_empty() {
        Ok(want)
    } else {
        Err(format!("versions disagree:\n{}", wrong.join("\n")))
    }
}

/// `bump-version [--check] [version]`.
pub fn run(root: &Path, version: Option<&str>, check_only: bool) -> Result<(), String> {
    if let Some(v) = version {
        if !is_version(v) {
            return Err(format!("not a version (x.y.z): {v}"));
        }
    }
    if check_only {
        let v = check(&collect(root)?, version)?;
        println!("all versions are {v}");
        return Ok(());
    }
    let v = version.ok_or("bump-version needs a version (x.y.z)")?;
    // Read and rewrite everything first; write only when every file had one.
    let mut writes = Vec::new();
    for (path, rel, rewrite) in files(root) {
        let text = fs::read_to_string(&path).map_err(|e| format!("reading {rel}: {e}"))?;
        let (next, found) = rewrite(&text, &rel, Some(v));
        if found.is_empty() {
            if rel.ends_with("package.json") {
                continue;
            }
            return Err(format!("{rel}: no version found"));
        }
        if next != text {
            writes.push((path, rel, next));
        }
    }
    for (path, rel, text) in &writes {
        fs::write(path, text).map_err(|e| format!("writing {rel}: {e}"))?;
        println!("updated {rel}");
    }
    check(&collect(root)?, Some(v))?;
    println!("all versions are {v}; add a CHANGELOG.md section, commit, then tag v{v}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOML: &str = r#"[workspace]
members = ["crates/a"]

[workspace.package]
version = "0.1.0"
edition = "2021"

[workspace.dependencies]
# Internal crates
tally-types = { path = "crates/tally-types", version = "0.1.0" }
tally-engine = { path = "crates/tally-engine", version = "0.1.0" }
tauri = "=2.12.4"
serde = { version = "=1.0.229", features = ["derive"] }
"#;

    const LOCK: &str = r#"version = 4

[[package]]
name = "serde"
version = "1.0.229"
source = "registry+https://github.com/rust-lang/crates.io-index"

[[package]]
name = "tally-types"
version = "0.1.0"
dependencies = [
 "serde",
]

[[package]]
name = "xtask"
version = "0.1.0"
"#;

    #[test]
    fn versions_are_strict_semver() {
        for ok in ["0.1.0", "1.20.3", "2.0.0-rc.1", "1.0.0-beta-2"] {
            assert!(is_version(ok), "{ok}");
        }
        for bad in [
            "",
            "v0.1.0",
            "0.1",
            "0.1.0.0",
            "01.0.0",
            "0.1.x",
            "0.1.0-",
            "0.1.0+b",
            "0.1.0-a..b",
            " 0.1.0",
        ] {
            assert!(!is_version(bad), "{bad}");
        }
    }

    #[test]
    fn cargo_toml_bumps_the_workspace_and_internal_crates_only() {
        let (next, found) = cargo_toml(TOML, Some("0.2.0"));
        assert_eq!(found.len(), 3);
        assert!(found.iter().all(|f| f.version == "0.1.0"));
        assert!(next.contains("[workspace.package]\nversion = \"0.2.0\""));
        assert!(
            next.contains("tally-types = { path = \"crates/tally-types\", version = \"0.2.0\" }")
        );
        assert!(
            next.contains("tally-engine = { path = \"crates/tally-engine\", version = \"0.2.0\" }")
        );
        // External pins and the rest untouched.
        assert!(next.contains("tauri = \"=2.12.4\""));
        assert!(next.contains("serde = { version = \"=1.0.229\""));
        assert_eq!(next.lines().count(), TOML.lines().count());
        let (_, again) = cargo_toml(&next, None);
        assert!(again.iter().all(|f| f.version == "0.2.0"));
        assert_eq!(
            toml_section_value(&next, "[workspace.package]", "version"),
            Some("0.2.0")
        );
    }

    #[test]
    fn cargo_lock_bumps_workspace_members_only() {
        let (next, found) = cargo_lock(LOCK, Some("0.2.0"));
        assert_eq!(
            found.iter().map(|f| f.what.as_str()).collect::<Vec<_>>(),
            ["Cargo.lock tally-types", "Cargo.lock xtask"]
        );
        assert!(next.contains("name = \"tally-types\"\nversion = \"0.2.0\""));
        assert!(next.contains("name = \"xtask\"\nversion = \"0.2.0\""));
        assert!(next.contains("name = \"serde\"\nversion = \"1.0.229\""));
        // The lock file's own format version is not a package.
        assert!(next.starts_with("version = 4\n"));
        assert_eq!(next.len(), LOCK.len());
    }

    #[test]
    fn json_bumps_the_top_level_version_only() {
        let pkg = "{\n  \"name\": \"x\",\n  \"version\": \"0.1.0\",\n  \"dependencies\": {\n    \"version\": \"9\"\n  }\n}\n";
        let (next, found) = json_version(pkg, "p/package.json", Some("1.2.3"));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].version, "0.1.0");
        assert!(next.contains("  \"version\": \"1.2.3\",\n"));
        assert!(next.contains("    \"version\": \"9\"\n"));
        let (same, none) = json_version(
            "{\n  \"name\": \"root\"\n}\n",
            "package.json",
            Some("1.0.0"),
        );
        assert!(none.is_empty());
        assert_eq!(same, "{\n  \"name\": \"root\"\n}\n");
    }

    #[test]
    fn check_reports_every_disagreement() {
        let f = |what: &str, v: &str| Found {
            what: what.to_owned(),
            version: v.to_owned(),
        };
        let same = [f("a", "0.1.0"), f("b", "0.1.0")];
        assert_eq!(check(&same, None), Ok("0.1.0".to_owned()));
        assert_eq!(check(&same, Some("0.1.0")), Ok("0.1.0".to_owned()));
        let err = check(&same, Some("0.2.0")).unwrap_err();
        assert!(err.contains("a: 0.1.0 (expected 0.2.0)") && err.contains("b: 0.1.0"));
        let mixed = [f("a", "0.1.0"), f("b", "0.1.1")];
        let err = check(&mixed, None).unwrap_err();
        assert!(err.contains("b: 0.1.1 (expected 0.1.0)") && !err.contains("a:"));
        assert!(check(&[], None).is_err());
    }

    #[test]
    fn bump_and_check_a_whole_tree() {
        let root = std::env::temp_dir().join(format!("tally-xtask-bump-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let w = |rel: &str, text: &str| {
            let p = root.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, text).unwrap();
        };
        w("Cargo.toml", TOML);
        w("Cargo.lock", LOCK);
        w(
            "apps/desktop/src-tauri/tauri.conf.json",
            "{\n  \"productName\": \"Tally\",\n  \"version\": \"0.1.0\",\n  \"plugins\": {}\n}\n",
        );
        w(
            "apps/desktop/package.json",
            "{\n  \"name\": \"d\",\n  \"version\": \"0.1.0\"\n}\n",
        );
        w(
            "packages/app/package.json",
            "{\n  \"name\": \"a\",\n  \"version\": \"0.1.0\"\n}\n",
        );
        w("packages/nover/package.json", "{\n  \"name\": \"n\"\n}\n");
        assert!(run(&root, None, true).is_ok());
        assert!(run(&root, Some("0.2.0"), true).is_err());
        assert!(run(&root, Some("v0.2.0"), false).is_err());
        assert!(run(&root, None, false).is_err());
        run(&root, Some("0.2.0"), false).unwrap();
        assert!(run(&root, Some("0.2.0"), true).is_ok());
        let found = collect(&root).unwrap();
        // 3 in Cargo.toml, 2 in Cargo.lock, tauri.conf.json, 2 package.json.
        assert_eq!(found.len(), 8);
        // A file left behind is caught.
        w(
            "packages/app/package.json",
            "{\n  \"name\": \"a\",\n  \"version\": \"0.1.9\"\n}\n",
        );
        let err = run(&root, None, true).unwrap_err();
        assert!(err.contains("packages/app/package.json version: 0.1.9"));
        let _ = fs::remove_dir_all(&root);
    }
}
