//! Performance baseline for the 1.0 gate (`cargo xtask perf`).
//!
//! Builds synthetic workbooks in a temp directory (never checked in) and
//! times, in release mode: full recalc, incremental recalc, open/save of
//! ODS and XLSX files of about 10 MB and 50 MB, CSV import of 1M rows, and
//! the peak RSS of opening the 50 MB files (measured in a child process
//! under `/usr/bin/time -l`).
//!
//! ```text
//! perf [--quick] [--only NAME[,NAME]] [--dir DIR] [--runs N] [--deep] [--keep]
//! perf --check [--thresholds FILE] [--json FILE] [--markdown FILE]
//!             [--runs N] [--only NAME[,NAME]] [--dir DIR]
//!                             (CI gate: run the workloads listed in the
//!                              threshold file, default
//!                              xtask/perf-thresholds.txt, compare the best
//!                              run with this OS's limit, exit 1 on a
//!                              failure; docs/perf-baseline.md)
//! perf --open-rss FILE        (internal: open FILE once and exit)
//! perf --dump FILE            (open FILE and print a digest of the loaded
//!                              workbook, to diff two builds)
//! perf --mem FILE             (open FILE step by step and print the RSS
//!                              and live malloc bytes after each step)
//! ```
//!
//! Scenario names: recalc, incremental, io10, io25, io50, csv, rss, deep,
//! chart, pdf, pivot, threads (chart, pdf and pivot are the 1.0 gate's `chart_data` for
//! 10k points, a 20-page PDF export and a 100k-row DataPilot refresh;
//! threads compares single- and multithreaded recalc, docs/recalc-parallel.md).

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::Instant;

#[path = "../perfcheck.rs"]
mod perfcheck;

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
    mem: Option<PathBuf>,
    keep: bool,
    dump: Option<PathBuf>,
    check: bool,
    runs_given: bool,
    thresholds: Option<PathBuf>,
    json: Option<PathBuf>,
    markdown: Option<PathBuf>,
}

fn parse() -> Res<Opts> {
    let mut o = Opts {
        quick: false,
        only: None,
        dir: None,
        runs: 3,
        deep: false,
        open_rss: None,
        mem: None,
        keep: false,
        dump: None,
        check: false,
        runs_given: false,
        thresholds: None,
        json: None,
        markdown: None,
    };
    let mut a = std::env::args().skip(1);
    while let Some(x) = a.next() {
        match x.as_str() {
            "--quick" => o.quick = true,
            "--deep" => o.deep = true,
            "--keep" => o.keep = true,
            "--only" => {
                let v = a.next().ok_or("--only needs a value")?;
                o.only = Some(v.split(',').map(str::to_owned).collect());
            }
            "--dir" => o.dir = Some(a.next().ok_or("--dir needs a value")?.into()),
            "--runs" => {
                o.runs = a
                    .next()
                    .and_then(|v| v.parse().ok())
                    .filter(|n| *n > 0)
                    .ok_or("--runs needs a number > 0")?;
                o.runs_given = true;
            }
            "--check" => o.check = true,
            "--thresholds" => {
                o.thresholds = Some(a.next().ok_or("--thresholds needs a file")?.into())
            }
            "--json" => o.json = Some(a.next().ok_or("--json needs a file")?.into()),
            "--markdown" => o.markdown = Some(a.next().ok_or("--markdown needs a file")?.into()),
            "--dump" => o.dump = Some(a.next().ok_or("--dump needs a file")?.into()),
            "--mem" => o.mem = Some(a.next().ok_or("--mem needs a file")?.into()),
            "--open-rss" => o.open_rss = Some(a.next().ok_or("--open-rss needs a file")?.into()),
            other => return Err(format!("unknown option {other}")),
        }
    }
    if o.check && (o.quick || o.deep) {
        return Err("--check takes the sizes from the threshold file; drop --quick/--deep".into());
    }
    if !o.check && (o.thresholds.is_some() || o.json.is_some() || o.markdown.is_some()) {
        return Err("--thresholds, --json and --markdown need --check".into());
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
    // Files go in a fresh subdirectory that perf creates (inside `--dir`
    // or the temp dir) and deletes afterwards; a directory given with
    // `--dir` is never deleted itself.
    let base = o.dir.clone().unwrap_or_else(std::env::temp_dir);
    let dir = base.join(format!("tally-perf-{}", std::process::id()));
    if dir.exists() {
        return Err(format!("{} already exists", dir.display()));
    }
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let res = scenarios(o, &dir, scale);
    if o.keep {
        eprintln!("perf: generated files kept in {}", dir.display());
    } else {
        let _ = std::fs::remove_dir_all(&dir);
    }
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
                group: None,
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

    if wants(o, "chart") {
        bench_chart(o, scale)?;
    }
    if wants(o, "pdf") {
        bench_pdf(o)?;
    }
    if wants(o, "pivot") {
        bench_pivot(o, scale)?;
    }

    if wants(o, "threads") {
        bench_threads(o, scale)?;
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

/// `rows` x 2 constants and `rows` formulas `=A{n}*2+B{n}` per formula
/// column, `cols` columns: wide and flat, every formula independent.
fn build_flat(rows: u32, cols: u32) -> Workbook {
    let mut wb = Workbook::empty();
    let s = new_sheet(&mut wb);
    for r in 0..rows {
        let n = r + 1;
        wb.set_value(addr(s, r, 0), Value::Number(f64::from(r)));
        wb.set_value(addr(s, r, 1), Value::Number(f64::from(r % 97)));
        for c in 0..cols {
            wb.set_input(
                addr(s, r, 2 + c),
                &format!("=A{n}*{}+B{n}/{}", c + 2, c + 1),
            );
        }
    }
    wb
}

/// Single-threaded against multithreaded recalc (docs/recalc-parallel.md)
/// on the same workbooks: `Threading::Sequential` versus the default
/// (`Auto`: the app setting, the pool, and the 4,096-formula threshold).
fn bench_threads(o: &Opts, scale: u32) -> Res<()> {
    use tally_engine::parallel::{threads, Threading};

    fn time<F: Fn() -> Workbook>(
        build: &F,
        mode: Threading,
        runs: usize,
        hard: bool,
    ) -> (f64, usize, usize) {
        let mut t = Vec::new();
        let (mut n, mut pre) = (0, 0);
        for _ in 0..runs {
            let mut wb = build();
            wb.threading = mode;
            if hard {
                recalc(&mut wb);
            }
            let s = Instant::now();
            let rep = if hard {
                hard_recalc(&mut wb)
            } else {
                recalc(&mut wb)
            };
            t.push(ms(s));
            n = rep.evaluated;
            pre = rep.precomputed;
        }
        (median(t), n, pre)
    }

    let runs = o.runs.max(3);
    let pool = threads();
    let rows = 1000 / scale.min(10);
    let mrows = 100_000 / scale;
    let frows = 100_000 / scale;
    let deep = 100_000 / scale;
    type Build = Box<dyn Fn() -> Workbook>;
    let cases: Vec<(String, Build, bool)> = vec![
        (
            format!("chains 100 x {rows}, first recalc"),
            Box::new(move || build_chains(rows, 100)),
            false,
        ),
        (
            format!("chains 100 x {rows}, hard recalc"),
            Box::new(move || build_chains(rows, 100)),
            true,
        ),
        (
            "SUM/VLOOKUP mix, first recalc".to_owned(),
            Box::new(move || build_mix(mrows, true)),
            false,
        ),
        (
            "SUM/VLOOKUP mix, hard recalc".to_owned(),
            Box::new(move || build_mix(mrows, true)),
            true,
        ),
        (
            format!("flat {} independent formulas, first recalc", frows * 2),
            Box::new(move || build_flat(frows, 2)),
            false,
        ),
        (
            format!("one chain {deep} deep, first recalc"),
            Box::new(move || build_deep(deep)),
            false,
        ),
    ];
    for (name, build, hard) in &cases {
        let (seq, n, _) = time(build, Threading::Sequential, runs, *hard);
        let (par, _, pre) = time(build, Threading::Auto, runs, *hard);
        row(
            &format!("Threads: {name}, 1 thread"),
            seq,
            &format!("{n} formulas"),
        );
        row(
            &format!("Threads: {name}, {pool} threads"),
            par,
            &format!("{pre} pre-computed; speed-up x{:.2}", seq / par.max(1e-9)),
        );
    }

    // Incremental: one edit with 10k dependents (below and above the
    // threshold the pre-pass is skipped or used; this one is above).
    for mode in [Threading::Sequential, Threading::Auto] {
        let deps = 10_000u32;
        let mut wb = Workbook::empty();
        wb.threading = mode;
        let s = new_sheet(&mut wb);
        wb.set_value(addr(s, 0, 0), Value::Number(1.0));
        for r in 0..deps {
            wb.set_input(addr(s, r, 1), &format!("=$A$1*{}", r + 1));
        }
        recalc(&mut wb);
        let mut t = Vec::new();
        let mut pre = 0;
        for i in 0..runs.max(9) {
            let st = Instant::now();
            wb.set_input(addr(s, 0, 0), &format!("{}", i + 2));
            let rep = recalc(&mut wb);
            t.push(ms(st));
            pre = rep.precomputed;
        }
        // One edit with 100 dependents: always below the threshold.
        let mut small = Vec::new();
        wb.set_input(addr(s, 0, 3), "1");
        for r in 0..100u32 {
            wb.set_input(addr(s, r, 4), &format!("=$D$1+{r}"));
        }
        recalc(&mut wb);
        for i in 0..runs.max(9) {
            let st = Instant::now();
            wb.set_input(addr(s, 0, 3), &format!("{}", i + 2));
            recalc(&mut wb);
            small.push(ms(st));
        }
        let label = if mode == Threading::Sequential {
            "1 thread".to_owned()
        } else {
            format!("{pool} threads")
        };
        row(
            &format!("Threads: 1 edit -> 10k dependents, {label}"),
            median(t),
            &format!("{pre} pre-computed"),
        );
        row(
            &format!("Threads: 1 edit -> 100 dependents, {label}"),
            median(small),
            "below the threshold: never pre-computed",
        );
    }
    Ok(())
}

fn tx(ops: Vec<tally_api::Op>) -> tally_api::Transaction {
    tally_api::Transaction {
        ops,
        label: None,
        selection_before: None,
        selection_after: None,
        group: None,
    }
}

fn apply(
    svc: &mut EngineService,
    doc: tally_api::DocId,
    ops: Vec<tally_api::Op>,
) -> Res<tally_api::ApplyResult> {
    let r = svc.apply(doc, tx(ops)).map_err(|e| e.to_string())?;
    if !r.errors.is_empty() {
        return Err(format!("{:?}", r.errors));
    }
    Ok(r)
}

fn created_id(r: &tally_api::ApplyResult) -> Res<u32> {
    r.created
        .as_ref()
        .and_then(|v| v.first())
        .map(|c| c.id)
        .ok_or_else(|| "no id created".to_owned())
}

fn chart_axis() -> tally_api::ChartAxisDto {
    tally_api::ChartAxisDto {
        visible: true,
        title: None,
        min: None,
        max: None,
        major: None,
        minor_count: None,
        log: false,
        reverse: false,
        major_grid: true,
        minor_grid: false,
        number_format: None,
        axis_type: Default::default(),
        time_resolution: None,
        major_time: None,
        minor_time: None,
        crosses: None,
        label_position: Default::default(),
        mark_position: Default::default(),
    }
}

fn bench_chart(o: &Opts, scale: u32) -> Res<()> {
    let (t, note) = chart_samples(o.runs.max(5), scale)?;
    row("chart_data, 10k points", median(t), &note);
    Ok(())
}

/// `chart_data` of a line chart with 10 000 points (one series plus text
/// categories) read through the service, as the UI does on every repaint
/// of a changed chart. Returns the cache-miss timings and a note.
fn chart_samples(runs: usize, scale: u32) -> Res<(Vec<f64>, String)> {
    use tally_api::{
        ChartAnchor, ChartDto, ChartKind, ChartSeriesDto, DataLabelsDto, LegendPos, Op, SeriesName,
        Stacking,
    };
    let n = 10_000 / scale;
    let mut wb = Workbook::empty();
    let s = new_sheet(&mut wb);
    wb.set_value(addr(s, 0, 0), Value::text("Day"));
    wb.set_value(addr(s, 0, 1), Value::text("Value"));
    for r in 1..=n {
        wb.set_value(addr(s, r, 0), Value::text(format!("d{r}")));
        wb.set_value(
            addr(s, r, 1),
            Value::Number(f64::from((r * 37) % 1000) / 10.0),
        );
    }
    let (mut svc, doc) = svc_with(wb);
    let last = n + 1;
    let chart = ChartDto {
        id: 0,
        name: String::new(),
        anchor: ChartAnchor {
            r: 1,
            c: 4,
            dx_mm: 0.0,
            dy_mm: 0.0,
            width_mm: 160.0,
            height_mm: 90.0,
        },
        kind: ChartKind::Line,
        stacking: Stacking::None,
        lines: true,
        markers: false,
        smooth: false,
        hole: None,
        exploded: false,
        filled: false,
        stock: None,
        three_d: false,
        own_data: false,
        data_table: None,
        source: None,
        categories: Some(format!("$Sheet1.$A$2:$A${last}")),
        series: vec![ChartSeriesDto {
            name: Some(SeriesName::Ref {
                r#ref: "$Sheet1.$B$1".into(),
            }),
            values: format!("$Sheet1.$B$2:$B${last}"),
            x_values: None,
            sizes: None,
            kind: None,
            secondary_axis: false,
            color: None,
            marker: None,
            labels: DataLabelsDto::default(),
            trend: None,
            error_bars: None,
            mean_line: false,
        }],
        title: Some("Perf".into()),
        subtitle: None,
        legend: LegendPos::Right,
        x_axis: chart_axis(),
        y_axis: chart_axis(),
        y2_axis: None,
        background: None,
        plot_background: None,
        include_hidden: false,
    };
    let r = apply(&mut svc, doc, vec![Op::AddChart { sheet: s, chart }])?;
    let id = created_id(&r)?;
    let mut t = Vec::new();
    let mut hit = Vec::new();
    let mut points = 0;
    for i in 0..runs {
        // Edit a series cell first so the document version moves and the
        // service's chart-data cache misses (what a repaint after an edit
        // pays).
        apply(
            &mut svc,
            doc,
            vec![Op::SetCellInput {
                sheet: s,
                r: 1,
                c: 1,
                input: format!("{}", i + 1),
            }],
        )?;
        let st = Instant::now();
        let d = svc.chart_data(doc, s, id).map_err(|e| e.to_string())?;
        t.push(ms(st));
        points = d.series.iter().map(|x| x.values.len()).sum::<usize>();
        let st = Instant::now();
        svc.chart_data(doc, s, id).map_err(|e| e.to_string())?;
        hit.push(ms(st));
    }
    if points != n as usize {
        return Err(format!("chart_data returned {points} points, wanted {n}"));
    }
    let note = format!(
        "{points} points (1 series + text categories), cache miss after an edit; cached repeat {:.2} ms",
        median(hit)
    );
    Ok((t, note))
}

fn bench_pdf(o: &Opts) -> Res<()> {
    let (t, note) = pdf_samples(o.runs.max(3), 1)?;
    row("PDF export, 20 pages", median(t), &note);
    Ok(())
}

/// A 20-page PDF (2 pages with `scale` 10): `prepare_print_job`
/// (pagination + cell layout) plus `print_job_pdf` (krilla), which is what
/// File > Export as PDF does. Returns the timings and a note.
fn pdf_samples(runs: usize, scale: u32) -> Res<(Vec<f64>, String)> {
    use tally_api::{PrintOptionsDto, PrintSheets};
    let opts = PrintOptionsDto {
        sheets: PrintSheets::Active,
        selection: None,
        suppress_empty_pages: true,
        page_from: None,
        page_to: None,
        active_sheet: None,
        page_ranges: None,
    };
    // Six columns fit one A4 portrait page across; find the rows per page
    // from a probe and size the sheet for 20 pages.
    let build = |rows: u32| {
        let mut wb = Workbook::empty();
        let s = new_sheet(&mut wb);
        for r in 0..rows {
            wb.set_value(addr(s, r, 0), Value::text(format!("Item {r}")));
            wb.set_value(addr(s, r, 1), Value::text(format!("Group {}", r % 17)));
            for c in 2..5u32 {
                wb.set_value(
                    addr(s, r, c),
                    Value::Number(f64::from((r * 31 + c * 7) % 10_000) / 100.0),
                );
            }
            wb.set_input(addr(s, r, 5), &format!("=C{}*D{}", r + 1, r + 1));
        }
        svc_with(wb)
    };
    let (svc, doc) = build(500);
    let probe = svc.print_layout(doc, &opts).map_err(|e| e.to_string())?;
    let per_page = probe
        .pages
        .first()
        .map_or(50, |p| p.rows[1].saturating_sub(p.rows[0]) + 1);
    let want_pages = 20 / scale.min(20);
    let (mut svc, doc) = build(per_page * want_pages);
    let mut t = Vec::new();
    let mut pages = 0;
    let mut bytes = 0;
    for _ in 0..runs {
        let st = Instant::now();
        let job = svc
            .prepare_print_job(doc, &opts, &[])
            .map_err(|e| e.to_string())?;
        let pdf = svc.print_job_pdf(job.job).map_err(|e| e.to_string())?;
        t.push(ms(st));
        pages = job.layout.pages.len();
        bytes = pdf.len();
    }
    if pages != want_pages as usize {
        return Err(format!("PDF has {pages} pages, wanted {want_pages}"));
    }
    let note = format!(
        "{pages} pages, {} rows x 6 cols, {:.0} KB; prepare_print_job + print_job_pdf",
        per_page * want_pages,
        bytes as f64 / 1024.0
    );
    Ok((t, note))
}

fn bench_pivot(o: &Opts, scale: u32) -> Res<()> {
    let (t, note) = pivot_samples(o.runs.max(3), scale)?;
    row("DataPilot refresh, 100k rows", median(t), &note);
    Ok(())
}

/// A DataPilot over 100 000 source rows (two row fields, one column field,
/// two data fields): create, then `RefreshPivot` through the service.
/// Returns the refresh timings and a note.
fn pivot_samples(runs: usize, scale: u32) -> Res<(Vec<f64>, String)> {
    use tally_api::{
        AggFn, CellPos, DataLayout, Op, PivotDataFieldDto, PivotDto, PivotFieldDto, PivotLayout,
        PivotSort, PivotSource, PivotSubtotals, RangeDto,
    };
    let n = 100_000 / scale;
    let mut wb = Workbook::empty();
    let s = new_sheet(&mut wb);
    let t = wb.add_sheet(Some("Report"), None);
    for (c, h) in ["Region", "Product", "Quarter", "Units", "Revenue"]
        .iter()
        .enumerate()
    {
        wb.set_value(addr(s, 0, c as u32), Value::text(*h));
    }
    for r in 1..=n {
        wb.set_value(addr(s, r, 0), Value::text(format!("R{}", r % 12)));
        wb.set_value(addr(s, r, 1), Value::text(format!("P{}", (r * 7) % 50)));
        wb.set_value(addr(s, r, 2), Value::text(format!("Q{}", r % 4 + 1)));
        wb.set_value(addr(s, r, 3), Value::Number(f64::from(r % 97)));
        wb.set_value(
            addr(s, r, 4),
            Value::Number(f64::from((r * 13) % 10_000) / 10.0),
        );
    }
    let (mut svc, doc) = svc_with(wb);
    let field = |col: u32, name: &str| PivotFieldDto {
        source_col: col,
        name: name.into(),
        subtotals: PivotSubtotals::Auto,
        show_empty: false,
        sort: PivotSort::Name { ascending: true },
        hidden_items: Vec::new(),
        page_item: None,
        layout: PivotLayout::Tabular,
        empty_line_after: false,
        auto_show: None,
        group: None,
    };
    let data = |col: u32, function: AggFn| PivotDataFieldDto {
        source_col: col,
        function,
        caption: None,
        show_as: None,
    };
    let pivot = PivotDto {
        id: 0,
        name: String::new(),
        source: PivotSource::Range {
            sheet: s,
            range: RangeDto {
                r0: 0,
                c0: 0,
                r1: n,
                c1: 4,
            },
        },
        source_filter: None,
        target_sheet: t,
        target: CellPos { r: 0, c: 0 },
        output: None,
        row_fields: vec![field(0, "Region"), field(1, "Product")],
        col_fields: vec![field(2, "Quarter")],
        page_fields: Vec::new(),
        data_fields: vec![data(4, AggFn::Sum), data(3, AggFn::Count)],
        data_layout: DataLayout::Row,
        row_totals: true,
        col_totals: true,
        ignore_empty_rows: false,
        identify_categories: false,
        show_filter_button: false,
        drill_down: true,
    };
    let st = Instant::now();
    let r = apply(
        &mut svc,
        doc,
        vec![Op::CreatePivot {
            pivot,
            overwrite: false,
        }],
    )?;
    let create = ms(st);
    let id = created_id(&r)?;
    let mut times = Vec::new();
    for i in 0..runs {
        // Change a source value so the refresh has something to do.
        apply(
            &mut svc,
            doc,
            vec![Op::SetCellInput {
                sheet: s,
                r: 1,
                c: 4,
                input: format!("{}", i + 1),
            }],
        )?;
        let st = Instant::now();
        apply(
            &mut svc,
            doc,
            vec![Op::RefreshPivot {
                id,
                overwrite: false,
            }],
        )?;
        times.push(ms(st));
    }
    let note = format!(
        "{n} rows x 5 cols, 12x50 row items x 4 quarters, Sum + Count; RefreshPivot via apply (create took {create:.0} ms)"
    );
    Ok((times, note))
}

/// Feeds formatted text into a hasher.
struct HashWriter(std::collections::hash_map::DefaultHasher);

impl std::fmt::Write for HashWriter {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        use std::hash::Hasher;
        self.0.write(s.as_bytes());
        Ok(())
    }
}

/// Opens `path` through the service and prints a digest of the loaded
/// workbook (`Workbook::to_data`, cells and styles with their Debug text),
/// so the same file opened by two builds can be compared.
fn dump(path: &Path) -> Res<()> {
    use std::fmt::Write as _;
    use std::hash::Hasher;
    let mut svc = EngineService::with_default_codec();
    let info = svc.open(path).map_err(|e| e.to_string())?;
    let data = svc
        .document(info.doc)
        .map_err(|e| e.to_string())?
        .wb
        .to_data();
    let mut all = HashWriter(std::collections::hash_map::DefaultHasher::new());
    let mut cells = 0usize;
    for s in &data.sheets {
        for c in &s.cells {
            let _ = write!(all, "{c:?}");
            cells += 1;
        }
        let _ = write!(all, "{:?}{:?}", s.style_runs, s.name);
    }
    let mut styles = HashWriter(std::collections::hash_map::DefaultHasher::new());
    let _ = write!(styles, "{:?}", data.styles);
    println!(
        "{}: {cells} cells, {} styles, cells+runs digest {:016x}, styles digest {:016x}",
        path.display(),
        data.styles.len(),
        all.0.finish(),
        styles.0.finish()
    );
    Ok(())
}

/// Current RSS (MB, from `ps`), physical footprint (`footprint`, which
/// also counts compressed pages) and live malloc bytes / allocation count
/// (from the malloc zone table of `vmmap --summary`) of this process.
fn mem_now() -> String {
    let pid = std::process::id().to_string();
    let rss = Command::new("ps")
        .args(["-o", "rss=", "-p", &pid])
        .output()
        .ok()
        .and_then(|o| {
            String::from_utf8_lossy(&o.stdout)
                .trim()
                .parse::<f64>()
                .ok()
        })
        .map_or(f64::NAN, |kb| kb / 1024.0);
    let vm = Command::new("vmmap")
        .args(["--summary", &pid])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    // The zone table ends with a TOTAL row when there are several zones;
    // otherwise take the single zone row.
    let mut in_zones = false;
    let mut zone = String::new();
    for l in vm.lines() {
        if l.starts_with("MALLOC ZONE") {
            in_zones = true;
            continue;
        }
        if in_zones && !l.starts_with('=') && !l.trim().is_empty() {
            zone = l.to_owned();
            if l.starts_with("TOTAL") {
                break;
            }
        }
    }
    let fp = Command::new("footprint")
        .arg(&pid)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    let fp = fp
        .split("Footprint: ")
        .nth(1)
        .and_then(|r| r.split(" (").next())
        .unwrap_or("?")
        .to_owned();
    let f: Vec<&str> = zone.split_whitespace().collect();
    let (count, alloc) = if f.len() >= 7 {
        (f[5].to_owned(), f[6].to_owned())
    } else {
        ("?".into(), "?".into())
    };
    format!("RSS {rss:.0} MB, footprint {fp}, malloc live {alloc} in {count} blocks")
}

/// Opens `path` step by step (read, decode, build, recalc) and prints the
/// memory after each step, to see where the peak of an open comes from.
fn mem_phases(path: &Path) -> Res<()> {
    use tally_engine::FileCodec;
    println!(
        "sizes: Value {} B, CellContent {} B, FormulaCell {} B",
        std::mem::size_of::<Value>(),
        std::mem::size_of::<tally_engine::storage::CellContent>(),
        std::mem::size_of::<tally_engine::formula_cells::FormulaCell>(),
    );
    println!("start: {}", mem_now());
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    println!("file read ({:.1} MB): {}", size_mb(path), mem_now());
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let t = Instant::now();
    let decoded = tally_engine::IoCodec
        .read(&bytes, &ext)
        .map_err(|e| e.to_string())?;
    let cells: usize = decoded.data.sheets.iter().map(|s| s.cells.len()).sum();
    let cap: usize = decoded.data.sheets.iter().map(|s| s.cells.capacity()).sum();
    if let Some(c) = decoded.data.sheets.iter().find_map(|s| s.cells.first()) {
        println!(
            "model cell {} B, cells capacity {cap} ({:.0} MB)",
            std::mem::size_of_val(c),
            (cap * std::mem::size_of_val(c)) as f64 / 1_048_576.0
        );
    }
    println!("decoded ({cells} cells, {:.0} ms): {}", ms(t), mem_now());
    drop(bytes);
    println!("file bytes dropped: {}", mem_now());
    let t = Instant::now();
    let mut wb = Workbook::from_data_owned(
        decoded.data,
        decoded.recognize_text,
        decoded.evaluate_formulas,
    );
    println!(
        "workbook built, model freed while loading ({:.0} ms): {}",
        ms(t),
        mem_now()
    );

    // `PERF_MEM_STOP=build` exits here, so `/usr/bin/time -l` shows the
    // peak of reading and building alone.
    if std::env::var("PERF_MEM_STOP").is_ok_and(|v| v == "build") {
        return Ok(());
    }
    let t = Instant::now();
    let rep = recalc(&mut wb);
    println!(
        "recalc ({} formulas, {:.0} ms): {}",
        rep.evaluated,
        ms(t),
        mem_now()
    );
    drop(wb);
    println!("workbook dropped: {}", mem_now());
    Ok(())
}

/// Workloads `--check` knows (the ids in the threshold file).
const CHECK_WORKLOADS: [&str; 7] = [
    "recalc_chains",
    "recalc_mix",
    "incremental_10k",
    "open_ods_50mb",
    "chart_10k",
    "pdf_20p",
    "pivot_100k",
];

/// Runs one `--check` workload: `runs` timed samples (ms) at `size`.
fn measure(
    id: &str,
    size: perfcheck::Size,
    runs: usize,
    dir: &Path,
) -> Res<perfcheck::Measurement> {
    let scale = size.scale();
    let setup = Instant::now();
    let mut setup_ms = 0.0;
    let mut samples = Vec::new();
    let note = match id {
        // The plan's "100k-formula recalc": 100 chains x 1000 deep, first
        // recalc after building (default threading, as the app runs).
        "recalc_chains" => {
            let rows = 1000 / scale;
            let mut n = 0;
            for _ in 0..runs {
                let b = Instant::now();
                let mut wb = build_chains(rows, 100);
                setup_ms += ms(b);
                let s = Instant::now();
                n = recalc(&mut wb).evaluated;
                samples.push(ms(s));
            }
            format!("{n} formulas, 100 chains x {rows} deep, first recalc")
        }
        // 100k SUM + 100k exact VLOOKUP into 10k rows (1M cells).
        "recalc_mix" => {
            let rows = 100_000 / scale;
            let mut n = 0;
            for _ in 0..runs.min(3) {
                let b = Instant::now();
                let mut wb = build_mix(rows, true);
                setup_ms += ms(b);
                let s = Instant::now();
                n = recalc(&mut wb).evaluated;
                samples.push(ms(s));
            }
            format!(
                "{n} formulas (SUM + exact VLOOKUP into 10k rows), {} cells, first recalc",
                u64::from(rows) * 10 + 20_000
            )
        }
        "incremental_10k" => {
            let deps = 10_000 / scale;
            let mut wb = Workbook::empty();
            let s = new_sheet(&mut wb);
            wb.set_value(addr(s, 0, 0), Value::Number(1.0));
            for r in 0..deps {
                wb.set_input(addr(s, r, 1), &format!("=$A$1*{}", r + 1));
            }
            recalc(&mut wb);
            setup_ms = ms(setup);
            // Cheap (about 1 ms): more samples so scheduler noise cannot
            // hit all of them.
            for i in 0..runs * 3 {
                let st = Instant::now();
                wb.set_input(addr(s, 0, 0), &format!("{}", i + 2));
                let rep = recalc(&mut wb);
                samples.push(ms(st));
                if rep.evaluated != deps as usize {
                    return Err(format!("evaluated {}, wanted {deps}", rep.evaluated));
                }
            }
            format!("1 edit -> {deps} dependents, Workbook::set_input + recalc")
        }
        // Generated in the job (build, recalc, save), then opened through
        // `EngineService::open` (read, build, recalc), as File > Open does.
        "open_ods_50mb" => {
            let (path, rows) = make_file(dir, FileFormat::Ods, 50.0 / f64::from(scale))?;
            setup_ms = ms(setup);
            let mb = size_mb(&path);
            let res = (0..runs.min(3)).try_for_each(|_| {
                samples.push(open_once(&path)?);
                Ok::<(), String>(())
            });
            let _ = std::fs::remove_file(&path);
            res?;
            format!("{mb:.1} MB ODS, {rows} rows x 12 cols, open includes recalc")
        }
        "chart_10k" => {
            let (t, note) = chart_samples(runs * 3, scale)?;
            samples = t;
            setup_ms = ms(setup) - samples.iter().sum::<f64>();
            note
        }
        "pdf_20p" => {
            let (t, note) = pdf_samples(runs, scale)?;
            samples = t;
            setup_ms = ms(setup) - samples.iter().sum::<f64>();
            note
        }
        "pivot_100k" => {
            let (t, note) = pivot_samples(runs, scale)?;
            samples = t;
            setup_ms = ms(setup) - samples.iter().sum::<f64>();
            note
        }
        other => {
            return Err(format!(
                "unknown workload `{other}` (known: {})",
                CHECK_WORKLOADS.join(", ")
            ))
        }
    };
    Ok(perfcheck::Measurement {
        samples,
        note,
        setup_ms: setup_ms.max(0.0),
    })
}

/// `--check`: runs every workload of the threshold file (or the `--only`
/// subset), prints the report, writes `--json` / appends `--markdown`.
/// `Ok(false)` when a workload failed its limit or did not run.
fn check(o: &Opts) -> Res<bool> {
    let file = o
        .thresholds
        .clone()
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("perf-thresholds.txt"));
    let text = std::fs::read_to_string(&file)
        .map_err(|e| format!("cannot read {}: {e}", file.display()))?;
    let thresholds =
        perfcheck::parse_thresholds(&text).map_err(|e| format!("{}: {e}", file.display()))?;
    if let Some(only) = &o.only {
        if let Some(bad) = only
            .iter()
            .find(|x| !thresholds.iter().any(|t| &t.id == *x))
        {
            return Err(format!("--only: `{bad}` is not in {}", file.display()));
        }
    }
    let os = std::env::consts::OS;
    let runs = if o.runs_given { o.runs } else { 5 };
    let base = o.dir.clone().unwrap_or_else(std::env::temp_dir);
    let dir = base.join(format!("tally-perf-{}", std::process::id()));
    if dir.exists() {
        return Err(format!("{} already exists", dir.display()));
    }
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut outcomes = Vec::new();
    for t in &thresholds {
        if o.only.as_ref().is_some_and(|v| !v.contains(&t.id)) {
            continue;
        }
        eprintln!("perf check: {} ({})...", t.id, t.size.name());
        let st = Instant::now();
        let r = measure(&t.id, t.size, runs, &dir);
        let out = perfcheck::evaluate(t, os, r);
        eprintln!(
            "perf check: {} best {:.1} ms, limit {}, {:.1} s in all",
            t.id,
            out.best_ms.unwrap_or(f64::NAN),
            out.limit
                .map_or("none".into(), |l| format!("{:.0} ms", l.ms)),
            ms(st) / 1000.0
        );
        outcomes.push(out);
    }
    if o.keep {
        eprintln!("perf: generated files kept in {}", dir.display());
    } else {
        let _ = std::fs::remove_dir_all(&dir);
    }
    let shown = std::env::current_dir()
        .ok()
        .and_then(|cwd| file.strip_prefix(cwd).ok().map(Path::to_path_buf))
        .unwrap_or_else(|| file.clone());
    let info = perfcheck::RunInfo {
        os: os.to_owned(),
        arch: std::env::consts::ARCH.to_owned(),
        cpus: std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get),
        pool_threads: tally_engine::parallel::threads(),
        runs,
        thresholds_file: shown.display().to_string().replace('\\', "/"),
    };
    let md = perfcheck::markdown(&info, &outcomes);
    println!("{md}");
    if let Some(p) = &o.json {
        std::fs::write(p, perfcheck::json(&info, &outcomes))
            .map_err(|e| format!("cannot write {}: {e}", p.display()))?;
    }
    if let Some(p) = &o.markdown {
        use std::io::Write as _;
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(p)
            .and_then(|mut f| f.write_all(md.as_bytes()))
            .map_err(|e| format!("cannot write {}: {e}", p.display()))?;
    }
    Ok(!outcomes.iter().any(perfcheck::Outcome::failed))
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
    if let Some(f) = &o.dump {
        return match dump(f) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("perf: {e}");
                ExitCode::FAILURE
            }
        };
    }
    if let Some(f) = &o.mem {
        return match mem_phases(f) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("perf: {e}");
                ExitCode::FAILURE
            }
        };
    }
    if let Some(f) = &o.open_rss {
        return match open_once(f) {
            Ok(_) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("perf: {e}");
                ExitCode::FAILURE
            }
        };
    }
    if o.check {
        return match check(&o) {
            Ok(true) => ExitCode::SUCCESS,
            Ok(false) => {
                eprintln!("perf: check failed (see the table above)");
                ExitCode::FAILURE
            }
            Err(e) => {
                eprintln!("perf: {e}");
                ExitCode::from(2)
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
