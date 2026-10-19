//! DSTDEV, DSTDEVP, DVAR, DVARP.
//!
//! Two-pass (mean, then sum of squared deviations) for numerical stability.
//! Sample forms need at least two numbers, population forms at least one;
//! otherwise `#DIV/0!`.

use tally_types::{ErrorKind, Value};

use super::common::{numbers, select, DB_PARAMS};
use crate::registry::{Category, FunctionDef, Registry};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

/// `DSTDEV(database; field; criteria)`.
pub const DSTDEV: FunctionDef = FunctionDef::new("DSTDEV", Category::Database, DB_PARAMS, dstdev)
    .help("Standard deviation of a sample of matching records.");
/// `DSTDEVP(database; field; criteria)`.
pub const DSTDEVP: FunctionDef =
    FunctionDef::new("DSTDEVP", Category::Database, DB_PARAMS, dstdevp)
        .help("Standard deviation of the population of matching records.");
/// `DVAR(database; field; criteria)`.
pub const DVAR: FunctionDef = FunctionDef::new("DVAR", Category::Database, DB_PARAMS, dvar)
    .help("Variance of a sample of matching records.");
/// `DVARP(database; field; criteria)`.
pub const DVARP: FunctionDef = FunctionDef::new("DVARP", Category::Database, DB_PARAMS, dvarp)
    .help("Variance of the population of matching records.");

fn variance(a: &mut Args, population: bool, root: bool) -> Result<Value, ErrorKind> {
    let sel = select(a, false)?;
    let xs = numbers(&sel)?;
    let n = xs.len();
    if n < if population { 1 } else { 2 } {
        return Err(ErrorKind::Div0);
    }
    let mean = xs.iter().sum::<f64>() / n as f64;
    let ss: f64 = xs.iter().map(|x| (x - mean) * (x - mean)).sum();
    let denom = if population { n } else { n - 1 } as f64;
    let v = ss / denom;
    Ok(Value::Number(if root { v.sqrt() } else { v }))
}

fn dstdev(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| variance(&mut a, false, true))
}

fn dstdevp(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| variance(&mut a, true, true))
}

fn dvar(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| variance(&mut a, false, false))
}

fn dvarp(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| variance(&mut a, true, false))
}

pub(super) fn register(r: &mut Registry) {
    r.register(DSTDEV);
    r.register(DSTDEVP);
    r.register(DVAR);
    r.register(DVARP);
}

#[cfg(test)]
mod tests {
    use super::super::common::sample;
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{ErrorKind, Value};

    fn run(f: &str, crit: &str) -> Value {
        let mut c = MockEvalContext::new();
        sample(&mut c);
        c.set(0, "H1", "Grade").set(0, "H2", ">=4");
        c.set(0, "J1", "Name").set(0, "J2", "Gus");
        let db = c.range(0, "A1:E11");
        let cr = c.range(0, crit);
        call(&mut c, f, vec![db, EvalValue::text("Weight"), cr])
    }

    fn close(v: Value, x: f64) {
        match v {
            Value::Number(n) => assert!((n - x).abs() < 1e-9, "{n} vs {x}"),
            o => panic!("{o:?}"),
        }
    }

    #[test]
    fn dispersion() {
        // weights 100 110 120 130 140: mean 120, ss 1000
        close(run("DVAR", "H1:H2"), 250.0);
        close(run("DVARP", "H1:H2"), 200.0);
        close(run("DSTDEV", "H1:H2"), 250f64.sqrt());
        close(run("DSTDEVP", "H1:H2"), 200f64.sqrt());
    }

    #[test]
    fn single_record() {
        assert_eq!(run("DVAR", "J1:J2"), Value::Error(ErrorKind::Div0));
        assert_eq!(run("DSTDEV", "J1:J2"), Value::Error(ErrorKind::Div0));
        close(run("DVARP", "J1:J2"), 0.0);
        close(run("DSTDEVP", "J1:J2"), 0.0);
    }
}
