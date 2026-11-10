//! Multithreaded recalculation (docs/recalc-parallel.md): every workbook is
//! recalculated single-threaded and with the parallel pre-pass forced
//! (`Threading::Parallel`), and the two must agree exactly: values bit for
//! bit, errors, array results, dynamic precedents and the recalc reports.
//!
//! `TALLY_PAR_STRESS=N` runs N times more random workbooks (race hunting).
#![cfg(feature = "parallel")]
#![allow(clippy::unwrap_used)]

use std::sync::Arc;

use tally_engine::parallel::Threading;
use tally_engine::recalc::{hard_recalc, recalc, recalc_with, RecalcBudget, RecalcReport};
use tally_engine::workbook::Rng;
use tally_engine::Workbook;
use tally_types::{CellAddr, ErrorKind, IterSettings, SheetId, Value};

const ROWS: u32 = 40;
const COLS: u32 = 8;

/// Deterministic generator for the random workbooks.
struct Gen(u64);

impl Gen {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    fn chance(&mut self, pct: u64) -> bool {
        self.below(100) < pct
    }
}

fn col_name(c: u32) -> char {
    char::from(b'A' + c as u8)
}

fn stress() -> u64 {
    std::env::var("TALLY_PAR_STRESS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1)
        .max(1)
}

/// A random workbook generator over three sheets of `ROWS` x `COLS`.
struct Builder {
    g: Gen,
    sheets: Vec<SheetId>,
    names: Vec<&'static str>,
}

impl Builder {
    fn sheet_name(&self, i: usize) -> String {
        format!("Sheet{}", i + 1)
    }

    /// A reference near `row`, mostly to earlier rows (long chains, few
    /// cycles), sometimes on another sheet.
    fn cell_ref(&mut self, sheet: usize, row: u32) -> String {
        let other = self.g.chance(15);
        let s = if other {
            self.g.below(3) as usize
        } else {
            sheet
        };
        let r = if row > 0 && self.g.chance(80) {
            self.g.below(u64::from(row)) as u32
        } else {
            self.g.below(u64::from(ROWS)) as u32
        };
        let c = self.g.below(u64::from(COLS)) as u32;
        if s == sheet {
            format!("{}{}", col_name(c), r + 1)
        } else {
            format!("${}.{}{}", self.sheet_name(s), col_name(c), r + 1)
        }
    }

    fn range(&mut self, sheet: usize, cols: u32) -> String {
        let r0 = self.g.below(u64::from(ROWS - 5)) as u32;
        let r1 = r0 + 1 + self.g.below(10) as u32;
        let r1 = r1.min(ROWS - 1);
        let c0 = self.g.below(u64::from(COLS - cols + 1)) as u32;
        let c1 = c0 + cols - 1;
        let base = format!("{}{}:{}{}", col_name(c0), r0 + 1, col_name(c1), r1 + 1);
        if self.g.chance(15) {
            let s = self.g.below(3) as usize;
            format!("${}.{}", self.sheet_name(s), base)
        } else {
            let _ = sheet;
            base
        }
    }

    fn formula(&mut self, sheet: usize, row: u32) -> String {
        let r = |b: &mut Self| b.cell_ref(sheet, row);
        match self.g.below(25) {
            // Dynamic ranges (review 7: an aggregate that stops at an
            // error constant must still see the formulas after it).
            24 => match self.g.below(3) {
                0 => format!("=SUM(INDIRECT(\"{}\"))", self.range(sheet, 1)),
                1 => format!(
                    "=SUMPRODUCT(OFFSET({};0;0;{};2))",
                    r(self),
                    2 + self.g.below(6)
                ),
                _ => format!("=COUNTIF(INDIRECT(\"{}\");\">5\")", self.range(sheet, 2)),
            },
            0 | 1 => format!("={}+{}", r(self), r(self)),
            2 => format!("=SUM({})", self.range(sheet, 2)),
            3 => format!("=IF({}>50;{};{}*2)", r(self), r(self), r(self)),
            4 => format!("=VLOOKUP({};{};2;0)", r(self), self.range(sheet, 2)),
            5 => format!("=MATCH({};{};0)", r(self), self.range(sheet, 1)),
            6 => {
                let target = r(self);
                format!("=INDIRECT(\"{}\")", target.replace('$', ""))
            }
            7 => format!(
                "=INDIRECT(\"{}\"&(1+MOD(ABS(N({}));{ROWS})))",
                col_name(self.g.below(u64::from(COLS)) as u32),
                r(self)
            ),
            8 => format!("=OFFSET({};1;1)", r(self)),
            9 => {
                let a = self.g.below(5) + 1;
                format!("=SUM($Sheet1.A{a}:$Sheet3.C{})", a + 3)
            }
            10 if self.g.chance(40) => format!("=RAND()*{}", r(self)),
            10 => format!("=RANDBETWEEN(1;10)+{}", r(self)),
            11 => format!("=COUNTIF({};\">50\")", self.range(sheet, 1)),
            12 => format!("={}/({}-{})", r(self), r(self), r(self)),
            13 => format!("=IFERROR({}*1;-1)", r(self)),
            14 => format!("=AVERAGE({})+ROW()", self.range(sheet, 3)),
            15 => format!("=CONCATENATE({};\"x\")", r(self)),
            16 => format!("=INDEX({};2;1)", self.range(sheet, 2)),
            17 => {
                let rg = self.range(sheet, 1);
                format!("=SUMPRODUCT({rg};{rg})")
            }
            18 => format!(
                "=MAX({};{})-MIN({})",
                r(self),
                r(self),
                self.range(sheet, 2)
            ),
            19 => format!("=NOW()*0+{}", r(self)),
            20 if self.g.chance(30) => {
                // MULTIPLE.OPERATIONS(formula; input cell; value)
                format!("=MULTIPLE.OPERATIONS({};{};5)", r(self), r(self))
            }
            20 => format!("={}*{}", r(self), r(self)),
            21 => {
                let n = self.names[self.g.below(self.names.len() as u64) as usize];
                format!("=SUM({n})+{}", r(self))
            }
            22 => format!("=HLOOKUP({};{};2;0)", r(self), self.range(sheet, 4)),
            _ => format!("=ROUND({}*1.5;2)", r(self)),
        }
    }

    fn input(&mut self, sheet: usize, row: u32) -> String {
        match self.g.below(100) {
            0..=34 => format!("{}", self.g.below(100)),
            35..=39 => format!("{}.{}", self.g.below(100), self.g.below(1000)),
            40..=43 => format!("t{}", self.g.below(5)),
            44..=45 => String::new(),
            // Error constants (written as values by `put`).
            46 if self.g.chance(50) => "#DIV/0!".to_string(),
            46 => "#N/A".to_string(),
            _ => self.formula(sheet, row),
        }
    }
}

/// Enters `text` at `addr`; "#DIV/0!" and "#N/A" become error *values*
/// (typed into a cell they are text, which aggregates skip).
fn put(wb: &mut Workbook, addr: CellAddr, text: &str) {
    match text {
        "#DIV/0!" => wb.set_value(addr, Value::Error(ErrorKind::Div0)),
        "#N/A" => wb.set_value(addr, Value::Error(ErrorKind::NA)),
        _ => wb.set_input(addr, text),
    }
}

fn random_workbook(seed: u64) -> Workbook {
    let mut wb = Workbook::new();
    let s1 = wb.sheets[0].id;
    let s2 = wb.add_sheet(Some("Sheet2"), None);
    let s3 = wb.add_sheet(Some("Sheet3"), None);
    let mut b = Builder {
        g: Gen(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1),
        sheets: vec![s1, s2, s3],
        names: vec!["Blk1", "Blk2"],
    };
    for (i, n) in b.names.clone().iter().enumerate() {
        let def = wb
            .parse_name_expr(&format!("$Sheet{}.$A$1:$B${}", i + 1, 5 + i * 3), None)
            .unwrap();
        wb.names.define(n, None, def);
    }
    if b.g.chance(30) {
        wb.settings.iterations = Some(IterSettings {
            steps: 1 + b.g.below(60) as u32,
            min_change: 0.001,
        });
    }
    if b.g.chance(20) {
        wb.settings.precision_as_shown = true;
        wb.settings.standard_decimals = Some(b.g.below(4) as u8);
    }
    for si in 0..3 {
        for r in 0..ROWS {
            for c in 0..COLS {
                let t = b.input(si, r);
                if !t.is_empty() {
                    put(&mut wb, CellAddr::new(b.sheets[si], r, c), &t);
                }
            }
        }
    }
    // A few array formulas.
    for _ in 0..b.g.below(4) {
        let si = b.g.below(3) as usize;
        let r = b.g.below(u64::from(ROWS - 3)) as u32;
        let c = b.g.below(u64::from(COLS - 2)) as u32;
        let origin = CellAddr::new(b.sheets[si], r, c);
        let rg = b.range(si, 2);
        let text = match b.g.below(3) {
            0 => format!("={rg}*2"),
            1 => format!("=TRANSPOSE({rg})"),
            _ => format!("=IF({rg}>10;{rg};0)"),
        };
        if let Ok(f) = wb.parse_formula(&text, origin) {
            wb.set_array_formula(origin, Arc::new(f), 2 + b.g.below(2) as u32, 2);
        }
    }
    wb.rng = Rng::new(seed);
    wb
}

/// Bit-exact value equality (numbers by their bits).
fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.to_bits() == y.to_bits(),
        (Value::Array(x), Value::Array(y)) => {
            x.rows == y.rows
                && x.cols == y.cols
                && (0..x.rows).all(|r| {
                    (0..x.cols).all(|c| match (x.get(r, c), y.get(r, c)) {
                        (Some(p), Some(q)) => same(p, q),
                        (None, None) => true,
                        _ => false,
                    })
                })
        }
        _ => a == b,
    }
}

fn compare(seq: &Workbook, par: &Workbook, what: &str) {
    assert_eq!(seq.sheets.len(), par.sheets.len(), "{what}");
    for (sa, sb) in seq.sheets.iter().zip(&par.sheets) {
        let used = sa.used_range();
        assert_eq!(used, sb.used_range(), "{what}: used range");
        let Some(u) = used else { continue };
        for r in u.r0..=u.r1 {
            for c in u.c0..=u.c1 {
                let a = CellAddr::new(sa.id, r, c);
                let (x, y) = (seq.value(a), par.value(a));
                assert!(
                    same(&x, &y),
                    "{what}: {a:?}: sequential {x:?}, parallel {y:?}"
                );
                match (seq.formula_at(a), par.formula_at(a)) {
                    (Some((_, f)), Some((_, g))) => {
                        assert!(same(&f.cached, &g.cached), "{what}: {a:?} cached");
                        assert_eq!(f.state, g.state, "{what}: {a:?} state");
                        assert_eq!(f.dynamic_deps, g.dynamic_deps, "{what}: {a:?} dynamic");
                        assert_eq!(
                            f.dynamic_spans(),
                            g.dynamic_spans(),
                            "{what}: {a:?} dynamic spans"
                        );
                        match (&f.array_result, &g.array_result) {
                            (Some(m), Some(n)) => assert!(
                                same(&Value::Array(m.clone()), &Value::Array(n.clone())),
                                "{what}: {a:?} matrix"
                            ),
                            (None, None) => {}
                            _ => panic!("{what}: {a:?} matrix presence"),
                        }
                    }
                    (None, None) => {}
                    _ => panic!("{what}: {a:?} formula presence"),
                }
            }
        }
    }
}

fn same_report(a: &RecalcReport, b: &RecalcReport, what: &str) {
    assert_eq!(a.evaluated, b.evaluated, "{what}: evaluated");
    assert_eq!(a.circular, b.circular, "{what}: circular");
    assert_eq!(a.incomplete, b.incomplete, "{what}: incomplete");
    assert_eq!(a.skipped, b.skipped, "{what}: skipped");
    assert_eq!(
        a.iteration_evaluations, b.iteration_evaluations,
        "{what}: iteration evaluations"
    );
    assert_eq!(a.changed, b.changed, "{what}: changed");
    assert_eq!(a.precomputed, 0, "{what}: the sequential run pre-computed");
}

/// The same workbook twice: sequential and parallel.
fn pair(wb: Workbook) -> (Workbook, Workbook) {
    let mut seq = wb.clone();
    seq.threading = Threading::Sequential;
    let mut par = wb;
    par.threading = Threading::Parallel;
    (seq, par)
}

fn both(
    seq: &mut Workbook,
    par: &mut Workbook,
    f: impl Fn(&mut Workbook) -> RecalcReport,
    what: &str,
) -> RecalcReport {
    let t = std::time::Instant::now();
    let a = f(seq);
    let ts = t.elapsed();
    let b = f(par);
    if std::env::var("TALLY_PAR_VERBOSE").is_ok() {
        eprintln!(
            "  {what}: sequential {ts:?}, parallel {:?}",
            t.elapsed() - ts
        );
    }
    same_report(&a, &b, what);
    compare(seq, par, what);
    b
}

fn run_seed(seed: u64) -> usize {
    if std::env::var("TALLY_PAR_VERBOSE").is_ok() {
        eprintln!("seed {seed}");
    }
    let (mut seq, mut par) = pair(random_workbook(seed));
    let mut pre = both(&mut seq, &mut par, recalc, &format!("seed {seed} first")).precomputed;
    let mut g = Gen(seed ^ 0xDEAD_BEEF_CAFE_F00D | 1);
    for round in 0..4 {
        let mut b = Builder {
            g: Gen(g.next() | 1),
            sheets: seq.sheets.iter().map(|s| s.id).collect(),
            names: vec!["Blk1", "Blk2"],
        };
        for _ in 0..1 + g.below(6) {
            let si = g.below(3) as usize;
            let r = g.below(u64::from(ROWS)) as u32;
            let c = g.below(u64::from(COLS)) as u32;
            let t = b.input(si, r);
            let a = CellAddr::new(b.sheets[si], r, c);
            if t.is_empty() {
                seq.clear_content(a);
                par.clear_content(a);
            } else {
                put(&mut seq, a, &t);
                put(&mut par, a, &t);
            }
        }
        pre += both(
            &mut seq,
            &mut par,
            recalc,
            &format!("seed {seed} edit round {round}"),
        )
        .precomputed;
    }
    pre += both(
        &mut seq,
        &mut par,
        hard_recalc,
        &format!("seed {seed} hard"),
    )
    .precomputed;
    pre
}

#[test]
fn random_workbooks_match_sequential() {
    // Unoptimized builds of deeply nested on-demand evaluation (16 levels
    // of formulas, MULTIPLE.OPERATIONS sandboxes) need more than the 2 MiB
    // of a test thread; the same workbooks run on 2 MiB in release.
    std::thread::Builder::new()
        .stack_size(16 << 20)
        .spawn(random_workbooks)
        .unwrap()
        .join()
        .unwrap();
}

fn random_workbooks() {
    let n = 60 * stress();
    let from: u64 = std::env::var("TALLY_PAR_FROM")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);
    let mut pre = 0;
    for seed in from..=n {
        pre += run_seed(seed);
    }
    assert!(pre > 0, "the pre-pass never produced a result");
}

/// Wide and deep workbooks with thousands of formulas, where many threads
/// really run at once.
#[test]
fn large_random_workbooks_match_sequential() {
    for seed in 0..(2 * stress()) {
        let mut wb = Workbook::new();
        let s = wb.sheets[0].id;
        let t = wb.add_sheet(Some("T"), None);
        let mut g = Gen(seed * 7919 + 13);
        for r in 0..2000u32 {
            wb.set_value(CellAddr::new(t, r, 0), Value::Number(f64::from(r)));
            wb.set_value(CellAddr::new(t, r, 1), Value::Number(f64::from(r) * 0.5));
        }
        for c in 0..30u32 {
            wb.set_value(CellAddr::new(s, 0, c), Value::Number(f64::from(c)));
            let cn = col_name(c % 26);
            let cn = if c >= 26 {
                format!("A{cn}")
            } else {
                cn.to_string()
            };
            for r in 1..400u32 {
                let text = match g.below(10) {
                    0..=4 => format!("={cn}{r}+1"),
                    5 => format!("=VLOOKUP(MOD({cn}{r};2000);$T.$A$1:$B$2000;2;0)"),
                    6 if c >= 4 => format!("=SUM($A{r}:$D{r})"),
                    6 => format!("=SUM($T.$A{r}:$T.$B{r})+{cn}{r}"),
                    7 => format!("=MATCH(MOD({cn}{r};2000);$T.$A$1:$A$2000;0)"),
                    8 => format!("=IF({cn}{r}>100;{cn}{r}/3;{cn}{r}*2)"),
                    _ => format!("=INDIRECT(\"{cn}\"&{r})*1.01"),
                };
                wb.set_input(CellAddr::new(s, r, c), &text);
            }
        }
        let (mut seq, mut par) = pair(wb);
        let rep = both(&mut seq, &mut par, recalc, &format!("large {seed}"));
        assert!(rep.precomputed > 5000, "pre-computed {}", rep.precomputed);
        seq.set_input(CellAddr::new(s, 0, 3), "1000");
        par.set_input(CellAddr::new(s, 0, 3), "1000");
        both(&mut seq, &mut par, recalc, &format!("large {seed} edit"));
        both(
            &mut seq,
            &mut par,
            hard_recalc,
            &format!("large {seed} hard"),
        );
    }
}

/// Many independent formulas around the cases that must stay sequential.
fn padded(wb: &mut Workbook) {
    let s = wb.sheets[0].id;
    for r in 0..3000u32 {
        wb.set_value(CellAddr::new(s, r, 20), Value::Number(f64::from(r)));
        wb.set_input(CellAddr::new(s, r, 21), &format!("=U{}*2", r + 1));
    }
}

fn a(s: &str) -> CellAddr {
    CellAddr::from_a1(1, s).unwrap()
}

#[test]
fn cycles_get_err_522_in_both_modes() {
    let mut wb = Workbook::new();
    padded(&mut wb);
    wb.set_input(a("A1"), "=B1");
    wb.set_input(a("B1"), "=A1+1");
    wb.set_input(a("C1"), "=A1*2");
    wb.set_input(a("D1"), "=D1");
    // A cycle only visible through INDIRECT at evaluation time.
    wb.set_input(a("E1"), "=INDIRECT(\"F1\")");
    wb.set_input(a("F1"), "=INDIRECT(\"E1\")+1");
    wb.set_input(a("G1"), "=F1+V1");
    let (mut seq, mut par) = pair(wb);
    let rep = both(&mut seq, &mut par, recalc, "cycles");
    assert!(rep.precomputed >= 3000);
    for c in ["A1", "B1", "C1", "D1", "E1", "F1", "G1"] {
        assert_eq!(par.value(a(c)), Value::Error(ErrorKind::Circular), "{c}");
    }
    seq.set_input(a("B1"), "5");
    par.set_input(a("B1"), "5");
    both(&mut seq, &mut par, recalc, "cycle broken");
    assert_eq!(par.value(a("C1")), Value::Number(10.0));
}

#[test]
fn iterations_match() {
    let mut wb = Workbook::new();
    padded(&mut wb);
    wb.settings.iterations = Some(IterSettings {
        steps: 100,
        min_change: 0.001,
    });
    wb.set_input(a("A1"), "=B1/2+1");
    wb.set_input(a("B1"), "=A1");
    wb.set_input(a("C1"), "=A1*3");
    wb.set_input(a("D1"), "=D1+1");
    let (mut seq, mut par) = pair(wb);
    both(&mut seq, &mut par, recalc, "iterations");
    assert_eq!(par.value(a("D1")), Value::Error(ErrorKind::NoConvergence));
    assert!(matches!(par.value(a("A1")), Value::Number(x) if (x - 2.0).abs() < 0.01));
}

#[test]
fn rand_stream_is_consumed_in_sequential_order() {
    let mut wb = Workbook::new();
    padded(&mut wb);
    for r in 1..=500u32 {
        // RAND cells at different depths of chains.
        wb.set_input(a(&format!("A{r}")), &format!("=U{r}+RAND()"));
        wb.set_input(a(&format!("B{r}")), &format!("=V{r}*RANDBETWEEN(1;1000)"));
        wb.set_input(a(&format!("C{r}")), &format!("=A{r}+B{r}"));
    }
    wb.rng = Rng::new(42);
    let (mut seq, mut par) = pair(wb);
    both(&mut seq, &mut par, recalc, "rand");
    both(&mut seq, &mut par, recalc, "rand again (volatile)");
}

#[test]
fn indirect_discovering_a_new_precedent() {
    let mut wb = Workbook::new();
    padded(&mut wb);
    // E1 reads V7 (a formula of the same recalc) only through INDIRECT.
    wb.set_input(a("D1"), "7");
    wb.set_input(a("E1"), "=INDIRECT(\"V\"&D1)+1");
    wb.set_input(a("F1"), "=E1*2");
    let (mut seq, mut par) = pair(wb);
    both(&mut seq, &mut par, recalc, "indirect first");
    assert_eq!(par.value(a("F1")), Value::Number(26.0));
    // The target moves to another formula.
    seq.set_input(a("D1"), "9");
    par.set_input(a("D1"), "9");
    both(&mut seq, &mut par, recalc, "indirect retarget");
    assert_eq!(par.value(a("F1")), Value::Number(34.0));
    seq.set_input(a("U9"), "100");
    par.set_input(a("U9"), "100");
    both(&mut seq, &mut par, recalc, "indirect precedent edit");
    assert_eq!(par.value(a("F1")), Value::Number(402.0));
}

#[test]
fn three_d_and_arrays_and_table_ops() {
    let mut wb = Workbook::new();
    padded(&mut wb);
    let s2 = wb.add_sheet(Some("Sheet2"), None);
    let s3 = wb.add_sheet(Some("Sheet3"), None);
    for (i, s) in [s2, s3].iter().enumerate() {
        for r in 0..10u32 {
            wb.set_input(
                CellAddr::new(*s, r, 0),
                &format!("=$Sheet1.U{}*{}", r + 1, i + 2),
            );
        }
    }
    wb.set_input(
        a("A1"),
        "=SUM($Sheet1.V1:$Sheet3.V10)+SUM($Sheet2.A1:$Sheet3.A10)",
    );
    let f = wb.parse_formula("=V1:V4*2", a("B1")).unwrap();
    wb.set_array_formula(a("B1"), Arc::new(f), 4, 1);
    wb.set_input(a("C1"), "=SUM(B1:B4)");
    wb.set_input(a("D1"), "=E1*2");
    wb.set_input(a("E1"), "3");
    wb.set_input(a("F1"), "=MULTIPLE.OPERATIONS(D1;E1;10)");
    wb.set_input(a("G1"), "=F1+C1");
    let (mut seq, mut par) = pair(wb);
    both(&mut seq, &mut par, recalc, "3d arrays table ops");
    assert_eq!(par.value(a("F1")), Value::Number(20.0));
    assert_eq!(
        par.value(a("C1")),
        Value::Number(2.0 * (0.0 + 2.0 + 4.0 + 6.0))
    );
    seq.set_input(a("U2"), "50");
    par.set_input(a("U2"), "50");
    both(&mut seq, &mut par, recalc, "3d arrays edit");
}

#[test]
fn budget_and_deadline() {
    let mut wb = Workbook::new();
    padded(&mut wb);
    let (mut seq, mut par) = pair(wb);
    // A formula budget smaller than the dirty set: sequential in both.
    let b = RecalcBudget::within(100, None);
    let rep = both(&mut seq, &mut par, |w| recalc_with(w, &b), "max cells");
    assert!(rep.incomplete);
    assert_eq!(rep.precomputed, 0);
    // A deadline already passed: nothing is evaluated in either mode.
    let (mut seq, mut par) = pair({
        let mut wb = Workbook::new();
        padded(&mut wb);
        wb
    });
    let b = RecalcBudget {
        max_cells_evaluated: u64::MAX,
        deadline: Some(std::time::Instant::now()),
    };
    let rep = both(&mut seq, &mut par, |w| recalc_with(w, &b), "deadline");
    assert!(rep.incomplete);
}

#[test]
fn auto_mode_uses_the_pool_above_the_threshold() {
    let mut wb = Workbook::new();
    let s = wb.sheets[0].id;
    for r in 0..10_000u32 {
        wb.set_input(CellAddr::new(s, r, 0), &format!("={}*2", r));
    }
    let mut auto = wb.clone();
    auto.threading = Threading::Auto;
    let mut seq = wb;
    seq.threading = Threading::Sequential;
    let a = recalc(&mut seq);
    let b = recalc(&mut auto);
    same_report(&a, &b, "auto");
    compare(&seq, &auto, "auto");
    if tally_engine::parallel::threads() > 1
        && tally_engine::parallel::enabled()
        && std::env::var("TALLY_CALC_THREADING").is_err()
    {
        // An automatic recalc goes single-threaded while another test's
        // pre-pass holds the pool: try until it is free.
        let mut pre = b.precomputed;
        for _ in 0..200 {
            if pre == 10_000 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
            let a = hard_recalc(&mut seq);
            let b = hard_recalc(&mut auto);
            same_report(&a, &b, "auto again");
            pre = b.precomputed;
        }
        assert_eq!(pre, 10_000);
    }
    // A small recalc stays sequential.
    auto.set_input(CellAddr::new(s, 0, 0), "=5");
    let small = recalc(&mut auto);
    if std::env::var("TALLY_CALC_THREADING").is_err() {
        assert_eq!(small.precomputed, 0);
    }
}

/// Review 7 (correctness finding 2): an aggregate stops reading its range
/// at the first error, but the sequential pass evaluates every dirty
/// formula of a dynamic range before reading it (and finds the reader
/// itself in it). The pre-pass must not pre-compute such a reader.
#[test]
fn a_range_read_that_stops_early_still_sees_later_formulas() {
    let cases: [&[(&str, &str)]; 5] = [
        // The range includes the reader: Err:522 in both modes.
        &[("B1", "#DIV/0!"), ("F1", "=SUM(OFFSET(B1;0;0;1;6))")],
        // A formula in the range reads the reader: a cycle.
        &[
            ("B1", "#DIV/0!"),
            ("F1", "=SUM(INDIRECT(\"B1:B10\"))"),
            ("B5", "=F1+1"),
        ],
        // A later formula in the range: evaluated inside the reader.
        &[
            ("B1", "#DIV/0!"),
            ("F1", "=SUM(INDIRECT(\"B1:B10\"))"),
            ("B5", "=U1+1"),
        ],
        &[
            ("B1", "#DIV/0!"),
            ("F1", "=SUMPRODUCT(INDIRECT(\"B1:B10\"))"),
            ("B5", "=F1+1"),
        ],
        &[
            ("B1", "#N/A"),
            (
                "F1",
                "=SUMIF(INDIRECT(\"B1:B10\");\">0\")+NPV(0.1;INDIRECT(\"B1:B10\"))",
            ),
            ("B7", "=F1*2"),
        ],
    ];
    for (i, cells) in cases.iter().enumerate() {
        let mut wb = Workbook::new();
        padded(&mut wb);
        for (c, t) in *cells {
            put(&mut wb, a(c), t);
        }
        let (mut seq, mut par) = pair(wb);
        let rep = both(&mut seq, &mut par, recalc, &format!("case {i}"));
        assert!(rep.precomputed >= 3000, "case {i}");
        // The next recalc knows the dynamic precedents.
        seq.set_input(a("U1"), "5");
        par.set_input(a("U1"), "5");
        both(&mut seq, &mut par, recalc, &format!("case {i} edit"));
        both(&mut seq, &mut par, hard_recalc, &format!("case {i} hard"));
    }
}

/// A lookup line holding a formula ordered after the reader (found only
/// through INDIRECT): the reader goes to the sequential pass.
#[test]
fn a_lookup_line_with_a_later_formula() {
    let mut wb = Workbook::new();
    padded(&mut wb);
    for r in 1..=200u32 {
        wb.set_input(a(&format!("C{r}")), &r.to_string());
        wb.set_input(a(&format!("D{r}")), &format!("=C{r}*3"));
    }
    for r in 1..=50u32 {
        wb.set_input(
            a(&format!("E{r}")),
            &format!("=VLOOKUP({r};INDIRECT(\"C1:D200\");2;0)+MATCH({r};INDIRECT(\"D1:D200\");0)"),
        );
    }
    wb.set_input(a("D100"), "=E1+1");
    let (mut seq, mut par) = pair(wb);
    both(&mut seq, &mut par, recalc, "lookup line");
    both(&mut seq, &mut par, hard_recalc, "lookup line hard");
}

/// Security review 7, H2: the pre-pass keeps every dirty precedent edge;
/// past a cap (about 12 bytes per edge) the recalc runs single-threaded.
/// 2,100 formulas each summing 2,100 formulas are 4.4 M edges.
#[test]
fn too_many_precedent_edges_fall_back_to_sequential() {
    let n = 2100u32;
    let mut wb = Workbook::new();
    for r in 1..=n {
        wb.set_input(a(&format!("A{r}")), &format!("={r}"));
        wb.set_input(a(&format!("B{r}")), &format!("=SUM($A$1:$A${n})"));
    }
    let (mut seq, mut par) = pair(wb);
    let rep = both(&mut seq, &mut par, recalc, "edges");
    assert_eq!(rep.precomputed, 0);
    let total = f64::from(n) * f64::from(n + 1) / 2.0;
    assert_eq!(par.value(a("B7")), Value::Number(total));
    // Below the cap the pre-pass runs.
    let mut wb = Workbook::new();
    for r in 1..=n {
        wb.set_input(a(&format!("A{r}")), &format!("={r}"));
        wb.set_input(a(&format!("B{r}")), "=SUM($A$1:$A$100)");
    }
    let (mut seq, mut par) = pair(wb);
    let rep = both(&mut seq, &mut par, recalc, "edges below the cap");
    assert_eq!(rep.precomputed, 2 * n as usize);
}

/// Security review 7, M1: a pre-pass evaluation stops at its first bail
/// instead of running on (here into the expensive branch) with Err:522 in
/// place of the value it could not read.
#[test]
fn a_bailed_evaluation_stops_at_once() {
    let n = 300u32;
    let mut wb = Workbook::new();
    padded(&mut wb);
    for r in 1..=n {
        // Z<r> is ordered after C<r> and found only through INDIRECT.
        wb.set_input(
            a(&format!("C{r}")),
            "=IF(ISNUMBER(INDIRECT(\"Z\"&ROW()));0;SUMPRODUCT(LEN($A$1:$A$1000000)))",
        );
    }
    for r in 1..=n {
        wb.set_input(a(&format!("Z{r}")), "=1");
    }
    let (mut seq, mut par) = pair(wb);
    let rep = both(&mut seq, &mut par, hard_recalc, "branch steer");
    assert_eq!(par.value(a("C1")), Value::Number(0.0));
    // Before the fix each bailed attempt evaluated the 1M-cell branch.
    assert!(
        rep.prepass_wasted_work < 1000 * u64::from(n),
        "wasted {}",
        rep.prepass_wasted_work
    );
}

/// Security review 7, M1: retries of formulas that wait for a precedent
/// found through INDIRECT are bounded by the number of dirty formulas in
/// all. Here 2,000 formulas each read ten points along a 3,000-long chain
/// (computed one cell after another), so each could wait and retry up to
/// 8 times (16,000 retries) without the pre-pass budget (5,000).
#[test]
fn prepass_retries_are_bounded() {
    let mut wb = Workbook::new();
    wb.set_input(a("V1"), "=1");
    for r in 2..=3000u32 {
        wb.set_input(a(&format!("V{r}")), &format!("=V{}+1", r - 1));
    }
    let reads: Vec<String> = (1..=10u32)
        .map(|k| format!("INDIRECT(\"V{}\")", k * 300))
        .collect();
    let text = format!("={}", reads.join("+"));
    for r in 1..=2000u32 {
        wb.set_input(a(&format!("X{r}")), &text);
    }
    let (mut seq, mut par) = pair(wb);
    let rep = both(&mut seq, &mut par, recalc, "retries");
    assert_eq!(rep.evaluated, 5000);
    assert!(
        rep.prepass_retries <= rep.evaluated,
        "{} retries",
        rep.prepass_retries
    );
    assert_eq!(par.value(a("X1")), Value::Number(16_500.0));
}

// The M2 case (an evaluation that already bailed must not count a failed
// index build) is covered by the unit test
// `recalc::tests::prepass_lookup_failures_do_not_skip_the_line`, which
// observes the lookup cache directly; a differential test only sees values,
// which the index never changes.

/// `SEED=n cargo test --test recalc_parallel debug_seed -- --ignored
/// --nocapture`: every cell of a random workbook in both modes, `!` where
/// they differ.
#[test]
#[ignore]
fn debug_seed() {
    let seed: u64 = std::env::var("SEED").unwrap().parse().unwrap();
    let (mut seq, mut par) = pair(random_workbook(seed));
    recalc(&mut seq);
    recalc(&mut par);
    for s in &seq.sheets {
        let Some(u) = s.used_range() else { continue };
        for r in u.r0..=u.r1 {
            for c in u.c0..=u.c1 {
                let a = CellAddr::new(s.id, r, c);
                let t = seq
                    .formula_at(a)
                    .and_then(|(id, _)| seq.formula_text(id))
                    .unwrap_or_default();
                let (x, y) = (seq.value(a), par.value(a));
                let mark = if same(&x, &y) { ' ' } else { '!' };
                println!(
                    "{mark} s{} {}{} {t} | {x:?} | {y:?}",
                    s.id,
                    col_name(c),
                    r + 1
                );
            }
        }
    }
}
