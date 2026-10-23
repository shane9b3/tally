//! Performance baseline for the 1.0 gate (`cargo xtask perf`).
//!
//! Builds synthetic workbooks in a temp directory (never checked in) and
//! times, in release mode: full recalc, incremental recalc, open/save of
//! ODS and XLSX files of about 10 MB and 50 MB, CSV import of 1M rows, and
//! the peak RSS of opening the 50 MB files (measured in a child process
//! under `/usr/bin/time -l`).
//!
//! ```text
//! perf [--quick] [--only NAME[,NAME]] [--dir DIR] [--runs N] [--deep]
//! perf --open-rss FILE        (internal: open FILE once and exit)
//! ```
//!
//! Scenario names: recalc, incremental, io10, io25, io50, csv, rss, deep.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::Instant;

use tally_api::{DocumentService, FileFormat};
use tally_engine::recalc::{hard_recalc, recalc};
use tally_engine::{EngineService, Workbook};
use tally_types::{CellAddr, SheetId, Value};

type Res<T> = Result<T, String>;

struct Opts {
    quick: bool,
    only: Option<Vec<String>>,
    dir: Option<PathBuf>,
    runs: usize,
    deep: bool,
    open_rss: Option<PathBuf>,
}

fn parse() -> Res<Opts> {
    let mut o = Opts {
        quick: false,
        only: None,
        dir: None,
        runs: 3,
        deep: false,
        open_rss: None,
    };
    let mut a = std::env::args().skip(1);
    while let Some(x) = a.next() {
        match x.as_str() {
            "--quick" => o.quick = true,
            "--deep" => o.deep = true,
            "--only" => {
                let v = a.next().ok_or("--only needs a value")?;
                o.only = Some(v.split(',').map(str::to_owned).collect());
            }
            "--dir" => o.dir = Some(a.next().ok_or("--dir needs a value")?.into()),
            "--runs" => {
                o.runs = a
                    .next()
                    .and_then(|v| v.parse().ok())
                    .ok_or("--runs needs a number")?
            }
            "--open-rss" => o.open_rss = Some(a.next().ok_or("--open-rss needs a file")?.into()),
            other => return Err(format!("unknown option {other}")),
        }
    }
    Ok(o)
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

fn row(name: &str, ms: f64, note: &str) {
    println!("| {name} | {ms:.1} | {note} |");
}

fn new_sheet(wb: &mut Workbook) -> SheetId {
    wb.add_sheet(Some("Sheet1"), None)
}

fn addr(sheet: SheetId, row: u32, col: u32) -> CellAddr {
    CellAddr { sheet, row, col }
}

fn col_name(c: u32) -> String {
    let mut c = c;
    let mut s = Vec::new();
    loop {
        s.push(b'A' + (c % 26) as u8);
        if c < 26 {
            break;
        }
        c = c / 26 - 1;
    }
    s.reverse();
    String::from_utf8(s).unwrap_or_default()
}

/// 100 columns x `rows` rows of chains: row 0 holds constants, each cell
/// below is `=<cell above>+1`, so every chain is `rows` deep.
fn build_chains(rows: u32, cols: u32) -> Workbook {
    let mut wb = Workbook::empty();
    let s = new_sheet(&mut wb);
    for c in 0..cols {
        wb.set_value(addr(s, 0, c), Value::Number(f64::from(c)));
        let cn = col_name(c);
        for r in 1..rows {
            wb.set_input(addr(s, r, c), &format!("={cn}{r}+1"));
        }
    }
    wb
}

/// One chain `n` deep in column A.
fn build_deep(n: u32) -> Workbook {
    build_chains_one(n)
}

fn build_chains_one(n: u32) -> Workbook {
    let mut wb = Workbook::empty();
    let s = new_sheet(&mut wb);
    wb.set_value(addr(s, 0, 0), Value::Number(1.0));
    for r in 1..n {
        wb.set_input(addr(s, r, 0), &format!("=A{r}+1"));
    }
    wb
}

/// `rows` x 10: eight numeric columns, `=SUM(A:H)` and a `VLOOKUP` into a
/// 10 000-row table on a second sheet. `rows` = 100 000 gives 1M cells.
fn build_mix(rows: u32, lookup: bool) -> Workbook {
    let mut wb = Workbook::empty();
    let s = new_sheet(&mut wb);
    let t = wb.add_sheet(Some("Lookup"), None);
    for r in 0..10_000u32 {
        wb.set_value(addr(t, r, 0), Value::Number(f64::from(r)));
        wb.set_value(addr(t, r, 1), Value::Number(f64::from(r) * 2.0));
    }
    for r in 0..rows {
        for c in 0..8u32 {
            wb.set_value(
                addr(s, r, c),
                Value::Number(f64::from((r * 7 + c * 13) % 10_000)),
            );
        }
        let n = r + 1;
        wb.set_input(addr(s, r, 8), &format!("=SUM(A{n}:H{n})"));
        if lookup {
            wb.set_input(
                addr(s, r, 9),
                &format!("=VLOOKUP(A{n};$Lookup.$A$1:$B$10000;2;0)"),
            );
        }
    }
    wb
}

/// Mixed content for the file benchmarks: numbers, text, a date-like value
/// and two formulas per row, 12 columns.
fn build_rows(rows: u32) -> Workbook {
    let mut wb = Workbook::empty();
    let s = new_sheet(&mut wb);
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    for r in 0..rows {
        for c in 0..5u32 {
            let v = (next() % 1_000_000) as f64 / 100.0;
            wb.set_value(addr(s, r, c), Value::Number(v));
        }
        for c in 5..8u32 {
            let w = format!("item-{}-{}", next() % 5000, c);
            wb.set_value(addr(s, r, c), Value::text(w));
        }
        for c in 8..10u32 {
            wb.set_value(addr(s, r, c), Value::Number((next() % 100_000) as f64));
        }
        let n = r + 1;
        wb.set_input(addr(s, r, 10), &format!("=SUM(A{n}:E{n})"));
        wb.set_input(addr(s, r, 11), &format!("=A{n}*I{n}-B{n}"));
    }
    wb
}

fn svc_with(wb: Workbook) -> (EngineService, tally_api::DocId) {
    let mut svc = EngineService::with_default_codec();
    let mut wb = wb;
    recalc(&mut wb);
    let info = svc.open_workbook(wb, "perf");
    (svc, info.doc)
}

fn fmt_ext(f: FileFormat) -> &'static str {
    match f {
        FileFormat::Xlsx => "xlsx",
        _ => "ods",
    }
}

fn size_mb(p: &Path) -> f64 {
    std::fs::metadata(p).map_or(0.0, |m| m.len() as f64 / 1_048_576.0)
}

/// Writes a file of about `target_mb` MB and returns its path and row count.
fn make_file(dir: &Path, fmt: FileFormat, target_mb: f64) -> Res<(PathBuf, u32)> {
    let probe = 20_000u32;
    let p = dir.join(format!("probe.{}", fmt_ext(fmt)));
    let (mut svc, doc) = svc_with(build_rows(probe));
    svc.save(doc, Some(&p), fmt).map_err(|e| e.to_string())?;
    let per_row = size_mb(&p) / f64::from(probe);
    let _ = std::fs::remove_file(&p);
    let rows = (target_mb / per_row) as u32;
    let path = dir.join(format!("gen{}.{}", target_mb as u32, fmt_ext(fmt)));
    let (mut svc, doc) = svc_with(build_rows(rows));
    svc.save(doc, Some(&path), fmt).map_err(|e| e.to_string())?;
    Ok((path, rows))
}

fn open_once(path: &Path) -> Res<f64> {
    let mut svc = EngineService::with_default_codec();
    let t = Instant::now();
    let info = svc.open(path).map_err(|e| e.to_string())?;
    let d = ms(t);
    svc.close(info.doc).map_err(|e| e.to_string())?;
    Ok(d)
}

fn peak_rss_mb(exe: &Path, file: &Path) -> Res<f64> {
    let out = Command::new("/usr/bin/time")
        .arg("-l")
        .arg(exe)
        .arg("--open-rss")
        .arg(file)
        .output()
        .map_err(|e| e.to_string())?;
    let err = String::from_utf8_lossy(&out.stderr);
    for line in err.lines() {
        if line.contains("maximum resident set size") {
            let n: f64 = line
                .split_whitespace()
                .next()
                .and_then(|v| v.parse().ok())
                .ok_or("cannot parse RSS")?;
            return Ok(n / 1_048_576.0);
        }
    }
    Err(format!("no RSS in time output: {err}"))
}

fn wants(o: &Opts, name: &str) -> bool {
    o.only.as_ref().is_none_or(|v| v.iter().any(|x| x == name))
}

fn run(o: &Opts) -> Res<()> {
    let scale = if o.quick { 10 } else { 1 };
    let dir = o
        .dir
        .clone()
        .unwrap_or_else(|| std::env::temp_dir().join(format!("tally-perf-{}", std::process::id())));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let res = scenarios(o, &dir, scale);
    let _ = std::fs::remove_dir_all(&dir);
    res
}

fn scenarios(o: &Opts, dir: &Path, scale: u32) -> Res<()> {
    println!("| Scenario | ms (median of {}) | Notes |", o.runs);
    println!("|---|---|---|");

    if wants(o, "recalc") {
        let rows = 1000 / scale.min(10);
        let mut t = Vec::new();
        let mut hard = Vec::new();
        let mut n = 0;
        for _ in 0..o.runs {
            let mut wb = build_chains(rows, 100);
            let s = Instant::now();
            let rep = recalc(&mut wb);
            t.push(ms(s));
            n = rep.evaluated;
            let s = Instant::now();
            hard_recalc(&mut wb);
            hard.push(ms(s));
        }
        row(
            "Full recalc, chains (first)",
            median(t),
            &format!("{n} formulas, 100 chains x {rows} deep"),
        );
        row(
            "Hard recalc, chains",
            median(hard),
            "same workbook, dirty_all",
        );

        let mrows = 100_000 / scale;
        let mut t = Vec::new();
        let mut hard = Vec::new();
        let mut n = 0;
        for _ in 0..o.runs.min(2) {
            let mut wb = build_mix(mrows, true);
            let s = Instant::now();
            let rep = recalc(&mut wb);
            t.push(ms(s));
            n = rep.evaluated;
            let s = Instant::now();
            hard_recalc(&mut wb);
            hard.push(ms(s));
        }
        let cells = u64::from(mrows) * 10 + 20_000;
        row(
            "Full recalc, SUM/VLOOKUP mix (first)",
            median(t),
            &format!("{cells} cells, {n} formulas (SUM + exact VLOOKUP into 10k rows)"),
        );
        let mut wb = build_mix(mrows, false);
        let st = Instant::now();
        let rep = recalc(&mut wb);
        row(
            "Full recalc, SUM only (VLOOKUP column omitted)",
            ms(st),
            &format!("{} formulas; isolates the VLOOKUP cost", rep.evaluated),
        );
        row(
            "Hard recalc, SUM/VLOOKUP mix",
            median(hard),
            "same workbook, dirty_all",
        );
    }

    if wants(o, "incremental") {
        let deps = 10_000u32;
        let mut wb = Workbook::empty();
        let s = new_sheet(&mut wb);
        wb.set_value(addr(s, 0, 0), Value::Number(1.0));
        for r in 0..deps {
            wb.set_input(addr(s, r, 1), &format!("=$A$1*{}", r + 1));
        }
        recalc(&mut wb);
        let mut t = Vec::new();
        for i in 0..o.runs.max(5) {
            let st = Instant::now();
            wb.set_input(addr(s, 0, 0), &format!("{}", i + 2));
            let rep = recalc(&mut wb);
            t.push(ms(st));
            if rep.evaluated != deps as usize {
                return Err(format!("incremental evaluated {}", rep.evaluated));
            }
        }
        row(
            "Incremental recalc, 1 edit -> 10k dependents",
            median(t),
            "Workbook::set_input + recalc",
        );

        // Same edit through the service (what the UI does).
        let (mut svc, doc) = svc_with(wb);
        let mut t = Vec::new();
        for i in 0..o.runs.max(5) {
            let tx = tally_api::Transaction {
                ops: vec![tally_api::Op::SetCellInput {
                    sheet: s,
                    r: 0,
                    c: 0,
                    input: format!("{}", i + 100),
                }],
                label: None,
                selection_before: None,
                selection_after: None,
            };
            let st = Instant::now();
            svc.apply(doc, tx).map_err(|e| e.to_string())?;
            t.push(ms(st));
        }
        row(
            "Incremental edit via DocumentService::apply",
            median(t),
            "10k dependents, includes undo snapshot and change report",
        );
    }

    let mut big: Vec<(FileFormat, PathBuf)> = Vec::new();
    for (name, mb) in [("io10", 10.0), ("io25", 25.0), ("io50", 50.0)] {
        if !(wants(o, name) || (name == "io50" && wants(o, "rss"))) {
            continue;
        }
        let mb = mb / f64::from(scale);
        for fmt in [FileFormat::Ods, FileFormat::Xlsx] {
            let (path, rows) = make_file(dir, fmt, mb)?;
            let label = fmt_ext(fmt).to_uppercase();
            let actual = size_mb(&path);
            if wants(o, name) {
                let mut opens = Vec::new();
                for _ in 0..o.runs {
                    match open_once(&path) {
                        Ok(d) => opens.push(d),
                        Err(e) => {
                            row(
                                &format!("Open {label} ~{mb:.0} MB"),
                                f64::NAN,
                                &format!("FAILED on {actual:.1} MB file: {e}"),
                            );
                            break;
                        }
                    }
                }
                if opens.is_empty() {
                    let _ = std::fs::remove_file(&path);
                    continue;
                }
                row(
                    &format!("Open {label} ~{mb:.0} MB"),
                    median(opens),
                    &format!("{actual:.1} MB on disk, {rows} rows x 12 cols, recalc included"),
                );
                let mut svc = EngineService::with_default_codec();
                let info = svc.open(&path).map_err(|e| e.to_string())?;
                let out = dir.join(format!("save.{}", fmt_ext(fmt)));
                let mut saves = Vec::new();
                for _ in 0..o.runs {
                    let st = Instant::now();
                    svc.save(info.doc, Some(&out), fmt)
                        .map_err(|e| e.to_string())?;
                    saves.push(ms(st));
                }
                row(
                    &format!("Save {label} ~{mb:.0} MB"),
                    median(saves),
                    &format!("{:.1} MB written", size_mb(&out)),
                );
                let _ = std::fs::remove_file(&out);
            }
            if name == "io50" {
                big.push((fmt, path));
            } else {
                let _ = std::fs::remove_file(&path);
            }
        }
    }

    if wants(o, "rss") {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        for (fmt, path) in &big {
            let rss = peak_rss_mb(&exe, path)?;
            row(
                &format!("Peak RSS, open {} ~50 MB", fmt_ext(*fmt).to_uppercase()),
                rss,
                &format!("MB (value is MB, not ms); file {:.1} MB", size_mb(path)),
            );
        }
    }

    if wants(o, "csv") {
        let rows = 1_000_000 / scale;
        let p = dir.join("big.csv");
        let mut text = String::with_capacity(rows as usize * 60);
        text.push_str("id,name,amount,qty,date,note\n");
        for i in 0..rows {
            text.push_str(&format!(
                "{i},name{},{}.{:02},{},2024-{:02}-{:02},note text {}\n",
                i % 977,
                i % 10_000,
                i % 100,
                i % 53,
                i % 12 + 1,
                i % 28 + 1,
                i % 31
            ));
        }
        std::fs::write(&p, text).map_err(|e| e.to_string())?;
        let mb = size_mb(&p);
        let mut t = Vec::new();
        for _ in 0..o.runs.min(3) {
            t.push(open_once(&p)?);
        }
        row(
            "CSV import",
            median(t),
            &format!("{rows} rows x 6 cols, {mb:.1} MB, via EngineService::open"),
        );
        if wants(o, "rss") {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            row(
                "Peak RSS, open CSV",
                peak_rss_mb(&exe, &p)?,
                "MB (value is MB, not ms)",
            );
        }
    }

    if o.deep || wants_exact(o, "deep") {
        let n = 100_000 / scale;
        let mut wb = build_deep(n);
        let s = Instant::now();
        let rep = recalc(&mut wb);
        row(
            "Full recalc, one chain",
            ms(s),
            &format!("{} formulas, depth {n}", rep.evaluated),
        );
    }
    Ok(())
}

fn wants_exact(o: &Opts, name: &str) -> bool {
    o.only.as_ref().is_some_and(|v| v.iter().any(|x| x == name))
}

fn main() -> ExitCode {
    let o = match parse() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("perf: {e}");
            return ExitCode::from(2);
        }
    };
    if let Some(f) = &o.open_rss {
        return match open_once(f) {
            Ok(_) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("perf: {e}");
                ExitCode::FAILURE
            }
        };
    }
    match run(&o) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("perf: {e}");
            ExitCode::FAILURE
        }
    }
}
