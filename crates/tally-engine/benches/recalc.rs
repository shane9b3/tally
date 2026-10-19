//! Recalc benchmarks with the Phase A gates (docs/phase-a.md, M-series):
//! 100k simple formulas full recalc < 500 ms; SUM(A1:A1000000) < 30 ms;
//! single edit at the head of a 10k chain < 50 ms.

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, Criterion};
use tally_engine::recalc::{hard_recalc, recalc};
use tally_engine::Workbook;
use tally_types::{CellAddr, Value};

fn sheet1(wb: &Workbook) -> u32 {
    wb.sheets[0].id
}

fn full_recalc_100k(c: &mut Criterion) {
    let mut wb = Workbook::new();
    let s = sheet1(&wb);
    for r in 0..100_000u32 {
        wb.set_value(CellAddr::new(s, r, 0), Value::Number(f64::from(r)));
        wb.set_input(CellAddr::new(s, r, 1), &format!("=A{}*2+1", r + 1));
    }
    recalc(&mut wb);
    c.bench_function("full recalc 100k simple formulas", |b| {
        b.iter(|| black_box(hard_recalc(&mut wb).evaluated))
    });
}

fn sum_1m(c: &mut Criterion) {
    let mut wb = Workbook::new();
    let s = sheet1(&wb);
    for r in 0..1_000_000u32 {
        wb.set_value(CellAddr::new(s, r, 0), Value::Number(1.0));
    }
    wb.set_input(CellAddr::new(s, 0, 1), "=SUM(A1:A1000000)");
    recalc(&mut wb);
    c.bench_function("SUM(A1:A1000000)", |b| {
        b.iter(|| {
            hard_recalc(&mut wb);
            black_box(wb.value(CellAddr::new(s, 0, 1)))
        })
    });
}

fn chain_edit_10k(c: &mut Criterion) {
    let mut wb = Workbook::new();
    let s = sheet1(&wb);
    wb.set_input(CellAddr::new(s, 0, 0), "1");
    for r in 1..10_000u32 {
        wb.set_input(CellAddr::new(s, r, 0), &format!("=A{r}+1"));
    }
    recalc(&mut wb);
    let mut n = 0u32;
    c.bench_function("10k chain single edit", |b| {
        b.iter(|| {
            n += 1;
            wb.set_input(CellAddr::new(s, 0, 0), &n.to_string());
            black_box(recalc(&mut wb).evaluated)
        })
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(10);
    targets = full_recalc_100k, sum_1m, chain_edit_10k
}
criterion_main!(benches);
