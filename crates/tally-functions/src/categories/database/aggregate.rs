//! DSUM, DAVERAGE, DCOUNT, DCOUNTA, DMAX, DMIN, DPRODUCT.

use tally_types::{ErrorKind, Value};

use super::common::{numbers, select, DB_PARAMS};
use crate::numeric::ApproxSum;
use crate::registry::{Category, FunctionDef, Registry};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

/// `DSUM(database; field; criteria)`.
pub const DSUM: FunctionDef = FunctionDef::new("DSUM", Category::Database, DB_PARAMS, dsum)
    .help("Totals the contents of all records in a database that match the search criteria.");
/// `DAVERAGE(database; field; criteria)`.
pub const DAVERAGE: FunctionDef =
    FunctionDef::new("DAVERAGE", Category::Database, DB_PARAMS, daverage)
        .help("Returns the average of the values of matching records.");
/// `DCOUNT(database; field; criteria)`.
pub const DCOUNT: FunctionDef = FunctionDef::new("DCOUNT", Category::Database, DB_PARAMS, dcount)
    .help("Counts the numeric cells of matching records.");
/// `DCOUNTA(database; field; criteria)`.
pub const DCOUNTA: FunctionDef =
    FunctionDef::new("DCOUNTA", Category::Database, DB_PARAMS, dcounta)
        .help("Counts the non-empty cells of matching records.");
/// `DMAX(database; field; criteria)`.
pub const DMAX: FunctionDef = FunctionDef::new("DMAX", Category::Database, DB_PARAMS, dmax)
    .help("Returns the maximum value of matching records.");
/// `DMIN(database; field; criteria)`.
pub const DMIN: FunctionDef = FunctionDef::new("DMIN", Category::Database, DB_PARAMS, dmin)
    .help("Returns the minimum value of matching records.");
/// `DPRODUCT(database; field; criteria)`.
pub const DPRODUCT: FunctionDef =
    FunctionDef::new("DPRODUCT", Category::Database, DB_PARAMS, dproduct)
        .help("Multiplies the values of matching records.");

fn dsum(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let sel = select(&mut a, false)?;
        let mut acc = ApproxSum::new();
        for x in numbers(&sel)? {
            acc.add(x);
        }
        Ok(Value::Number(acc.total()))
    })
}

fn daverage(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let sel = select(&mut a, false)?;
        let mut acc = ApproxSum::new();
        for x in numbers(&sel)? {
            acc.add(x);
        }
        if acc.count() == 0 {
            return Err(ErrorKind::Div0);
        }
        Ok(Value::Number(acc.total() / acc.count() as f64))
    })
}

fn dcount(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let sel = select(&mut a, true)?;
        if sel.values.is_empty() {
            return Ok(Value::Number(sel.matched as f64));
        }
        let n = sel
            .values
            .iter()
            .filter(|v| matches!(v, Value::Number(_) | Value::Bool(_)))
            .count();
        Ok(Value::Number(n as f64))
    })
}

fn dcounta(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let sel = select(&mut a, true)?;
        if sel.values.is_empty() {
            return Ok(Value::Number(sel.matched as f64));
        }
        let n = sel.values.iter().filter(|v| !v.is_empty()).count();
        Ok(Value::Number(n as f64))
    })
}

fn extreme(a: &mut Args, max: bool) -> Result<Value, ErrorKind> {
    let sel = select(a, false)?;
    let nums = numbers(&sel)?;
    let r = nums
        .into_iter()
        .reduce(|x, y| if max == (y > x) { y } else { x })
        .unwrap_or(0.0);
    Ok(Value::Number(r))
}

fn dmax(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| extreme(&mut a, true))
}

fn dmin(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| extreme(&mut a, false))
}

fn dproduct(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let sel = select(&mut a, false)?;
        let nums = numbers(&sel)?;
        if nums.is_empty() {
            return Ok(Value::Number(0.0));
        }
        Ok(Value::Number(nums.iter().product()))
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(DSUM);
    r.register(DAVERAGE);
    r.register(DCOUNT);
    r.register(DCOUNTA);
    r.register(DMAX);
    r.register(DMIN);
    r.register(DPRODUCT);
}

#[cfg(test)]
mod tests {
    use super::super::common::sample;
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{ErrorKind, Value};

    fn setup() -> MockEvalContext {
        let mut c = MockEvalContext::new();
        sample(&mut c);
        // Age >= 17 AND Grade < 5
        c.set(0, "H1", "Age").set(0, "H2", ">=17");
        c.set(0, "I1", "Grade").set(0, "I2", "<5");
        // Name starts with "F" OR Name = Ivy
        c.set(0, "K1", "Name")
            .set(0, "K2", "F*")
            .set(0, "K3", "Ivy");
        c
    }

    fn d(c: &mut MockEvalContext, f: &str, field: EvalValue, crit: &str) -> Value {
        let db = c.range(0, "A1:E11");
        let cr = c.range(0, crit);
        call(c, f, vec![db, field, cr])
    }

    fn n(x: f64) -> Value {
        Value::Number(x)
    }

    #[test]
    fn sum_average_extremes() {
        let mut c = setup();
        let w = || EvalValue::text("weight");
        assert_eq!(d(&mut c, "DSUM", w(), "H1:I2"), n(210.0));
        assert_eq!(d(&mut c, "DAVERAGE", w(), "H1:I2"), n(105.0));
        assert_eq!(d(&mut c, "DMAX", w(), "H1:I2"), n(110.0));
        assert_eq!(d(&mut c, "DMIN", w(), "H1:I2"), n(100.0));
        assert_eq!(
            d(&mut c, "DPRODUCT", EvalValue::number(2.0), "H1:I2"),
            n(4.0 * 4.0)
        );
    }

    #[test]
    fn or_rows_and_wildcards() {
        let mut c = setup();
        assert_eq!(
            d(&mut c, "DSUM", EvalValue::number(4.0), "K1:K3"),
            n(4.0 + 10.0 + 14.0)
        );
        assert_eq!(d(&mut c, "DCOUNT", EvalValue::number(0.0), "K1:K3"), n(3.0));
    }

    #[test]
    fn counts() {
        let mut c = setup();
        c.set(0, "A12", "Zed").set(0, "B12", "n/a");
        let db = c.range(0, "A1:E12");
        let cr = c.range(0, "H1:H2");
        let r = call(
            &mut c,
            "DCOUNT",
            vec![db.clone(), EvalValue::text("Grade"), cr.clone()],
        );
        assert_eq!(r, n(5.0));
        let r = call(
            &mut c,
            "DCOUNTA",
            vec![db.clone(), EvalValue::text("Grade"), cr.clone()],
        );
        assert_eq!(r, n(5.0));
        let all = c.range(0, "M1:M1");
        let r = call(&mut c, "DCOUNTA", vec![db, EvalValue::text("Grade"), all]);
        assert_eq!(r, n(11.0));
    }

    #[test]
    fn errors() {
        let mut c = setup();
        assert_eq!(
            d(&mut c, "DSUM", EvalValue::text("nope"), "H1:H2"),
            Value::Error(ErrorKind::InvalidArgument)
        );
        assert_eq!(
            d(&mut c, "DSUM", EvalValue::number(9.0), "H1:H2"),
            Value::Error(ErrorKind::InvalidArgument)
        );
        assert_eq!(
            d(&mut c, "DSUM", EvalValue::number(0.0), "H1:H2"),
            Value::Error(ErrorKind::InvalidArgument)
        );
        assert_eq!(
            d(&mut c, "DAVERAGE", EvalValue::number(2.0), "K1:K1"),
            n(3.5)
        );
        c.set(0, "N1", "Age").set(0, "N2", ">100");
        assert_eq!(
            d(&mut c, "DAVERAGE", EvalValue::number(2.0), "N1:N2"),
            Value::Error(ErrorKind::Div0)
        );
        assert_eq!(d(&mut c, "DMAX", EvalValue::number(2.0), "N1:N2"), n(0.0));
    }
}
