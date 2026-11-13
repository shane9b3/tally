//! `perf --check`: the threshold file, the comparison and the reports
//! (docs/perf-baseline.md, "CI gate").
//!
//! Pure logic only (no engine, no timing), so the unit tests run in the
//! dependency-free `xtask` bin as well as in the `perf` bin that uses it.
//!
//! Threshold file (`xtask/perf-thresholds.txt`): one workload per line,
//! whitespace-separated columns, `#` starts a comment.
//!
//! ```text
//! # workload      size   target_ms  macos  linux  windows
//! recalc_chains   full   500        200?   300?   300?
//! ```
//!
//! `size` is `full` (the size the 1.0 target is stated for) or `quick` (a
//! tenth of it). `target_ms` is the 1.0 product claim on the reference
//! machine, reported but never gated (`-` for none). Each OS column is the
//! CI limit for the best of the runs on that OS, in ms; a trailing `?`
//! marks a limit not yet calibrated on that OS's CI runner (it still
//! gates), and `-` means not gated on that OS.

use std::fmt::Write as _;

/// The OS columns of the threshold file, in order (`std::env::consts::OS`).
pub const OSES: [&str; 3] = ["macos", "linux", "windows"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Size {
    Full,
    Quick,
}

impl Size {
    pub fn name(self) -> &'static str {
        match self {
            Size::Full => "full",
            Size::Quick => "quick",
        }
    }

    /// Divisor of the workload sizes (`--quick` is 1/10).
    pub fn scale(self) -> u32 {
        match self {
            Size::Full => 1,
            Size::Quick => 10,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Limit {
    pub ms: f64,
    /// Not yet calibrated on that OS's runner (written `123?`).
    pub provisional: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Threshold {
    pub id: String,
    pub size: Size,
    pub target_ms: Option<f64>,
    /// One entry per [`OSES`] column; `None` is not gated there.
    pub limits: [Option<Limit>; 3],
}

impl Threshold {
    pub fn limit(&self, os: &str) -> Option<Limit> {
        OSES.iter()
            .position(|o| *o == os)
            .and_then(|i| self.limits[i])
    }
}

fn parse_ms(v: &str, what: &str, line: usize) -> Result<f64, String> {
    v.parse::<f64>()
        .ok()
        .filter(|x| x.is_finite() && *x > 0.0)
        .ok_or_else(|| format!("line {line}: {what} `{v}` is not a positive number of ms"))
}

/// Parses a threshold file.
pub fn parse_thresholds(text: &str) -> Result<Vec<Threshold>, String> {
    let mut out: Vec<Threshold> = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let line = i + 1;
        let body = raw.split('#').next().unwrap_or("").trim();
        if body.is_empty() {
            continue;
        }
        let f: Vec<&str> = body.split_whitespace().collect();
        if f.len() != 3 + OSES.len() {
            return Err(format!(
                "line {line}: expected 6 columns (workload size target_ms macos linux windows), got {}",
                f.len()
            ));
        }
        let id = f[0].to_owned();
        if out.iter().any(|t| t.id == id) {
            return Err(format!("line {line}: workload `{id}` listed twice"));
        }
        let size = match f[1] {
            "full" => Size::Full,
            "quick" => Size::Quick,
            other => return Err(format!("line {line}: size `{other}` is not full or quick")),
        };
        let target_ms = match f[2] {
            "-" => None,
            v => Some(parse_ms(v, "target", line)?),
        };
        let mut limits = [None; 3];
        for (k, os) in OSES.iter().enumerate() {
            let v = f[3 + k];
            limits[k] = match v {
                "-" => None,
                _ => {
                    let (num, provisional) = match v.strip_suffix('?') {
                        Some(n) => (n, true),
                        None => (v, false),
                    };
                    Some(Limit {
                        ms: parse_ms(num, &format!("{os} limit"), line)?,
                        provisional,
                    })
                }
            };
        }
        out.push(Threshold {
            id,
            size,
            target_ms,
            limits,
        });
    }
    if out.is_empty() {
        return Err("no workloads in the threshold file".into());
    }
    Ok(out)
}

/// Timings of one workload (each sample one run, ms).
#[derive(Debug, Clone, PartialEq)]
pub struct Measurement {
    pub samples: Vec<f64>,
    /// What was measured (sizes, counts), for the report.
    pub note: String,
    /// Time spent building the inputs (not gated), ms.
    pub setup_ms: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    Pass,
    Fail,
    /// No limit for this OS.
    NotGated,
    /// The workload did not run (counts as a failure).
    Error(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub threshold: Threshold,
    pub measurement: Option<Measurement>,
    pub best_ms: Option<f64>,
    pub median_ms: Option<f64>,
    pub limit: Option<Limit>,
    pub status: Status,
}

impl Outcome {
    /// Best over limit (1.0 = at the limit).
    pub fn ratio(&self) -> Option<f64> {
        Some(self.best_ms? / self.limit?.ms)
    }

    pub fn failed(&self) -> bool {
        matches!(self.status, Status::Fail | Status::Error(_))
    }
}

/// The best (smallest) sample: interference from other work on a shared
/// runner only ever adds time, so the minimum is the least noisy statistic.
pub fn best(samples: &[f64]) -> Option<f64> {
    samples
        .iter()
        .copied()
        .filter(|x| x.is_finite())
        .min_by(f64::total_cmp)
}

pub fn median(samples: &[f64]) -> Option<f64> {
    let mut v: Vec<f64> = samples.iter().copied().filter(|x| x.is_finite()).collect();
    if v.is_empty() {
        return None;
    }
    v.sort_by(f64::total_cmp);
    Some(v[v.len() / 2])
}

/// Compares one workload's result with its threshold on `os`.
pub fn evaluate(t: &Threshold, os: &str, result: Result<Measurement, String>) -> Outcome {
    let limit = t.limit(os);
    match result {
        Err(e) => Outcome {
            threshold: t.clone(),
            measurement: None,
            best_ms: None,
            median_ms: None,
            limit,
            status: Status::Error(e),
        },
        Ok(m) => {
            let best_ms = best(&m.samples);
            let median_ms = median(&m.samples);
            let status = match (best_ms, limit) {
                (None, _) => Status::Error("no valid samples".into()),
                (Some(_), None) => Status::NotGated,
                (Some(b), Some(l)) if b <= l.ms => Status::Pass,
                (Some(_), Some(_)) => Status::Fail,
            };
            Outcome {
                threshold: t.clone(),
                measurement: Some(m),
                best_ms,
                median_ms,
                limit,
                status,
            }
        }
    }
}

/// Where and how the check ran (report header and JSON).
#[derive(Debug, Clone, PartialEq)]
pub struct RunInfo {
    pub os: String,
    pub arch: String,
    pub cpus: usize,
    pub pool_threads: usize,
    pub runs: usize,
    pub thresholds_file: String,
}

fn num(v: Option<f64>) -> String {
    match v {
        Some(x) if x >= 100.0 => format!("{x:.0}"),
        Some(x) => format!("{x:.1}"),
        None => "-".into(),
    }
}

fn limit_text(l: Option<Limit>) -> String {
    match l {
        Some(l) if l.provisional => format!("{} (uncalibrated)", num(Some(l.ms))),
        Some(l) => num(Some(l.ms)),
        None => "not gated".into(),
    }
}

fn status_text(s: &Status) -> &'static str {
    match s {
        Status::Pass => "pass",
        Status::Fail => "FAIL",
        Status::NotGated => "not gated",
        Status::Error(_) => "ERROR",
    }
}

/// A Markdown report: every measured value next to its limit, then one
/// line per failure. Printed to stdout and appended to the step summary.
pub fn markdown(info: &RunInfo, outcomes: &[Outcome]) -> String {
    let failed = outcomes.iter().filter(|o| o.failed()).count();
    let mut s = String::new();
    let verdict = if failed == 0 {
        "passed".to_owned()
    } else {
        format!("FAILED ({failed} of {})", outcomes.len())
    };
    let _ = writeln!(
        s,
        "### perf check on {} ({}): {verdict}\n\n{} logical CPUs, recalc pool {} threads, `--runs {}` (each workload's run count is in the details); the best run is gated. Thresholds `{}`.\n",
        info.os, info.arch, info.cpus, info.pool_threads, info.runs, info.thresholds_file
    );
    let _ = writeln!(
        s,
        "| Workload | Size | Best ms | Median ms | {} limit ms | Best / limit | Target ms (reference machine, not gated) | Result |",
        info.os
    );
    let _ = writeln!(s, "|---|---|---|---|---|---|---|---|");
    for o in outcomes {
        let ratio = o
            .ratio()
            .map_or("-".into(), |r| format!("{:.0} %", r * 100.0));
        let _ = writeln!(
            s,
            "| `{}` | {} | {} | {} | {} | {} | {} | {} |",
            o.threshold.id,
            o.threshold.size.name(),
            num(o.best_ms),
            num(o.median_ms),
            limit_text(o.limit),
            ratio,
            num(o.threshold.target_ms),
            status_text(&o.status),
        );
    }
    let _ = writeln!(s);
    for o in outcomes {
        match &o.status {
            Status::Fail => {
                let (b, l) = (
                    o.best_ms.unwrap_or(f64::NAN),
                    o.limit.map_or(f64::NAN, |l| l.ms),
                );
                let _ = writeln!(
                    s,
                    "- **FAIL** `{}` ({} size) on {}: best {} ms is {:.0} % over the limit of {} ms ({:.2}x).",
                    o.threshold.id,
                    o.threshold.size.name(),
                    info.os,
                    num(Some(b)),
                    (b / l - 1.0) * 100.0,
                    num(Some(l)),
                    b / l
                );
            }
            Status::Error(e) => {
                let _ = writeln!(s, "- **ERROR** `{}` on {}: {e}", o.threshold.id, info.os);
            }
            _ => {}
        }
    }
    let _ = writeln!(s, "\n<details><summary>What was measured</summary>\n");
    for o in outcomes {
        if let Some(m) = &o.measurement {
            let samples: Vec<String> = m.samples.iter().map(|x| num(Some(*x))).collect();
            let _ = writeln!(
                s,
                "- `{}`: {}; {} runs: {} ms; input setup {} ms",
                o.threshold.id,
                m.note,
                samples.len(),
                samples.join(", "),
                num(Some(m.setup_ms))
            );
        }
    }
    let _ = writeln!(s, "\n</details>");
    s
}

fn json_str(v: &str) -> String {
    let mut s = String::with_capacity(v.len() + 2);
    s.push('"');
    for c in v.chars() {
        match c {
            '"' => s.push_str("\\\""),
            '\\' => s.push_str("\\\\"),
            '\n' => s.push_str("\\n"),
            '\r' => s.push_str("\\r"),
            '\t' => s.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(s, "\\u{:04x}", c as u32);
            }
            c => s.push(c),
        }
    }
    s.push('"');
    s
}

fn json_num(v: Option<f64>) -> String {
    match v {
        Some(x) if x.is_finite() => format!("{x}"),
        _ => "null".into(),
    }
}

/// The machine-readable result (uploaded as a CI artifact).
pub fn json(info: &RunInfo, outcomes: &[Outcome]) -> String {
    let mut s = String::from("{\n");
    let passed = !outcomes.iter().any(Outcome::failed);
    let _ = writeln!(s, "  \"os\": {},", json_str(&info.os));
    let _ = writeln!(s, "  \"arch\": {},", json_str(&info.arch));
    let _ = writeln!(s, "  \"cpus\": {},", info.cpus);
    let _ = writeln!(s, "  \"pool_threads\": {},", info.pool_threads);
    let _ = writeln!(s, "  \"runs\": {},", info.runs);
    let _ = writeln!(
        s,
        "  \"thresholds_file\": {},",
        json_str(&info.thresholds_file)
    );
    let _ = writeln!(s, "  \"passed\": {passed},");
    s.push_str("  \"workloads\": [");
    for (i, o) in outcomes.iter().enumerate() {
        s.push_str(if i == 0 { "\n" } else { ",\n" });
        let samples = o.measurement.as_ref().map_or(String::new(), |m| {
            m.samples
                .iter()
                .map(|x| json_num(Some(*x)))
                .collect::<Vec<_>>()
                .join(", ")
        });
        let error = match &o.status {
            Status::Error(e) => json_str(e),
            _ => "null".into(),
        };
        let _ = write!(
            s,
            "    {{\"id\": {}, \"size\": {}, \"status\": {}, \"best_ms\": {}, \"median_ms\": {}, \"limit_ms\": {}, \"limit_provisional\": {}, \"ratio\": {}, \"target_ms\": {}, \"samples_ms\": [{samples}], \"setup_ms\": {}, \"note\": {}, \"error\": {error}}}",
            json_str(&o.threshold.id),
            json_str(o.threshold.size.name()),
            json_str(match o.status {
                Status::Pass => "pass",
                Status::Fail => "fail",
                Status::NotGated => "not_gated",
                Status::Error(_) => "error",
            }),
            json_num(o.best_ms),
            json_num(o.median_ms),
            json_num(o.limit.map(|l| l.ms)),
            o.limit.is_some_and(|l| l.provisional),
            json_num(o.ratio()),
            json_num(o.threshold.target_ms),
            json_num(o.measurement.as_ref().map(|m| m.setup_ms)),
            json_str(o.measurement.as_ref().map_or("", |m| m.note.as_str())),
        );
    }
    s.push_str("\n  ]\n}\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "\
# comment line
recalc_chains  full  500  200?  300  -   # trailing comment

open_ods_50mb  quick -    9000  9000? 12000?
";

    fn m(samples: &[f64]) -> Result<Measurement, String> {
        Ok(Measurement {
            samples: samples.to_vec(),
            note: "n".into(),
            setup_ms: 1.0,
        })
    }

    fn info() -> RunInfo {
        RunInfo {
            os: "linux".into(),
            arch: "x86_64".into(),
            cpus: 4,
            pool_threads: 4,
            runs: 5,
            thresholds_file: "xtask/perf-thresholds.txt".into(),
        }
    }

    #[test]
    fn parses_the_file() {
        let t = parse_thresholds(FILE).expect("parses");
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].id, "recalc_chains");
        assert_eq!(t[0].size, Size::Full);
        assert_eq!(t[0].target_ms, Some(500.0));
        assert_eq!(
            t[0].limit("macos"),
            Some(Limit {
                ms: 200.0,
                provisional: true
            })
        );
        assert_eq!(
            t[0].limit("linux"),
            Some(Limit {
                ms: 300.0,
                provisional: false
            })
        );
        assert_eq!(t[0].limit("windows"), None);
        assert_eq!(t[0].limit("freebsd"), None);
        assert_eq!(t[1].size, Size::Quick);
        assert_eq!((Size::Full.scale(), Size::Quick.scale()), (1, 10));
        assert_eq!(t[1].target_ms, None);
        assert_eq!(t[1].limit("windows").map(|l| l.ms), Some(12000.0));
    }

    #[test]
    fn rejects_bad_files() {
        for (bad, why) in [
            ("a full 1 2 3", "6 columns"),
            ("a huge 1 2 3 4", "full or quick"),
            ("a full x 2 3 4", "target"),
            ("a full 1 0 3 4", "macos limit"),
            ("a full 1 2 -3? 4", "linux limit"),
            ("a full 1 2 3 4\na full 1 2 3 4", "twice"),
            ("# nothing\n", "no workloads"),
        ] {
            let e = parse_thresholds(bad).expect_err(bad);
            assert!(e.contains(why), "{bad}: {e}");
        }
    }

    #[test]
    fn the_checked_in_file_parses_and_gates_every_os() {
        let text = include_str!("../perf-thresholds.txt");
        let t = parse_thresholds(text).expect("xtask/perf-thresholds.txt parses");
        for th in &t {
            for os in OSES {
                assert!(th.limit(os).is_some(), "{} has no {os} limit", th.id);
            }
        }
    }

    #[test]
    fn compares_the_best_sample_with_the_limit() {
        let t = &parse_thresholds(FILE).expect("parses")[0];
        // Best of the runs gates; one slow run does not fail it.
        let o = evaluate(t, "linux", m(&[900.0, 299.0, 1500.0]));
        assert_eq!(o.status, Status::Pass);
        assert_eq!(o.best_ms, Some(299.0));
        assert_eq!(o.median_ms, Some(900.0));
        // At the limit passes, above it fails.
        assert_eq!(evaluate(t, "linux", m(&[300.0])).status, Status::Pass);
        let o = evaluate(t, "linux", m(&[301.0, 450.0]));
        assert_eq!(o.status, Status::Fail);
        assert!(o.failed());
        assert!((o.ratio().expect("ratio") - 301.0 / 300.0).abs() < 1e-12);
        // No limit for this OS: reported, never fails.
        let o = evaluate(t, "windows", m(&[1e9]));
        assert_eq!(o.status, Status::NotGated);
        assert!(!o.failed());
        // NaN samples are ignored; none left is an error.
        assert_eq!(
            evaluate(t, "linux", m(&[f64::NAN, 10.0])).best_ms,
            Some(10.0)
        );
        assert!(evaluate(t, "linux", m(&[f64::NAN])).failed());
        assert!(evaluate(t, "linux", m(&[])).failed());
        // A workload that did not run fails.
        let o = evaluate(t, "linux", Err("boom".into()));
        assert_eq!(o.status, Status::Error("boom".into()));
        assert!(o.failed());
    }

    #[test]
    fn reports_say_what_failed_and_by_how_much() {
        let t = parse_thresholds(FILE).expect("parses");
        let out = vec![
            evaluate(&t[0], "linux", m(&[450.0, 600.0])),
            evaluate(&t[1], "linux", m(&[100.0])),
        ];
        let md = markdown(&info(), &out);
        assert!(md.contains("FAILED (1 of 2)"), "{md}");
        assert!(
            md.contains("| `recalc_chains` | full | 450 | 600 | 300 | 150 % | 500 | FAIL |"),
            "{md}"
        );
        assert!(
            md.contains(
                "| `open_ods_50mb` | quick | 100 | 100 | 9000 (uncalibrated) | 1 % | - | pass |"
            ),
            "{md}"
        );
        assert!(
            md.contains("`recalc_chains` (full size) on linux: best 450 ms is 50 % over the limit of 300 ms (1.50x)"),
            "{md}"
        );
        let all_pass = vec![evaluate(&t[1], "linux", m(&[100.0]))];
        assert!(markdown(&info(), &all_pass).contains(": passed"));
    }

    #[test]
    fn json_is_well_formed_enough() {
        let t = parse_thresholds(FILE).expect("parses");
        let out = vec![
            evaluate(&t[0], "linux", m(&[450.0])),
            evaluate(&t[1], "linux", Err("bad \"quote\"\n".into())),
        ];
        let j = json(&info(), &out);
        assert!(j.contains("\"passed\": false"), "{j}");
        assert!(j.contains("\"id\": \"recalc_chains\", \"size\": \"full\", \"status\": \"fail\", \"best_ms\": 450,"), "{j}");
        assert!(
            j.contains("\"limit_ms\": 300, \"limit_provisional\": false, \"ratio\": 1.5,"),
            "{j}"
        );
        assert!(j.contains("\"error\": \"bad \\\"quote\\\"\\n\""), "{j}");
        assert!(j.contains("\"best_ms\": null"), "{j}");
        // Balanced brackets and braces outside strings.
        let (mut depth, mut in_str, mut esc) = (0i32, false, false);
        for c in j.chars() {
            match (in_str, esc, c) {
                (true, true, _) => esc = false,
                (true, false, '\\') => esc = true,
                (true, false, '"') => in_str = false,
                (false, _, '"') => in_str = true,
                (false, _, '{' | '[') => depth += 1,
                (false, _, '}' | ']') => depth -= 1,
                _ => {}
            }
            assert!(depth >= 0);
        }
        assert_eq!(depth, 0);
        assert!(!in_str);
        assert_eq!(json_str("a\u{1}"), "\"a\\u0001\"");
    }
}
