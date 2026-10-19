//! STDEV, STDEVA, STDEVP, STDEVPA, VAR, VARA, VARP, VARPA.

use tally_types::{ErrorKind, Value};

use super::util::{mean, seq_numbers, sum_sq_dev};
use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const SEQ: &[Param] = &[Param::repeat("number", ParamKind::NumberSeq)];
const SEQ_A: &[Param] = &[Param::repeat("value", ParamKind::ValueSeq)];

macro_rules! def {
    ($c:ident, $f:ident, $name:literal, $params:ident, $help:literal) => {
        #[doc = concat!("`", $name, "(number; ...)`.")]
        pub const $c: FunctionDef =
            FunctionDef::new($name, Category::Statistical, $params, $f).help($help);
    };
}

def!(
    STDEV,
    stdev,
    "STDEV",
    SEQ,
    "Standard deviation based on a sample."
);
def!(
    STDEVA,
    stdeva,
    "STDEVA",
    SEQ_A,
    "Standard deviation of a sample, text counted as 0."
);
def!(
    STDEVP,
    stdevp,
    "STDEVP",
    SEQ,
    "Standard deviation based on the entire population."
);
def!(
    STDEVPA,
    stdevpa,
    "STDEVPA",
    SEQ_A,
    "Standard deviation of a population, text counted as 0."
);
def!(VAR, var, "VAR", SEQ, "Variance based on a sample.");
def!(
    VARA,
    vara,
    "VARA",
    SEQ_A,
    "Variance of a sample, text counted as 0."
);
def!(
    VARP,
    varp,
    "VARP",
    SEQ,
    "Variance based on the entire population."
);
def!(
    VARPA,
    varpa,
    "VARPA",
    SEQ_A,
    "Variance of a population, text counted as 0."
);

/// Two-pass variance. Sample variants need at least 2 values and population
/// variants at least 1; otherwise `#DIV/0!` (LibreOffice; AOO's `Err:502`
/// for a single value is not reproduced).
fn dispersion(
    ctx: &mut dyn EvalContext,
    args: &mut dyn ArgList,
    text_as_zero: bool,
    population: bool,
    root: bool,
) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let v = seq_numbers(&mut a, text_as_zero)?;
        let n = v.len();
        if n < if population { 1 } else { 2 } {
            return Err(ErrorKind::Div0);
        }
        let denom = if population { n as f64 } else { n as f64 - 1.0 };
        let var = sum_sq_dev(&v, mean(&v)) / denom;
        Ok(Value::Number(if root { var.sqrt() } else { var }))
    })
}

fn stdev(c: &mut dyn EvalContext, a: &mut dyn ArgList) -> EvalValue {
    dispersion(c, a, false, false, true)
}
fn stdeva(c: &mut dyn EvalContext, a: &mut dyn ArgList) -> EvalValue {
    dispersion(c, a, true, false, true)
}
fn stdevp(c: &mut dyn EvalContext, a: &mut dyn ArgList) -> EvalValue {
    dispersion(c, a, false, true, true)
}
fn stdevpa(c: &mut dyn EvalContext, a: &mut dyn ArgList) -> EvalValue {
    dispersion(c, a, true, true, true)
}
fn var(c: &mut dyn EvalContext, a: &mut dyn ArgList) -> EvalValue {
    dispersion(c, a, false, false, false)
}
fn vara(c: &mut dyn EvalContext, a: &mut dyn ArgList) -> EvalValue {
    dispersion(c, a, true, false, false)
}
fn varp(c: &mut dyn EvalContext, a: &mut dyn ArgList) -> EvalValue {
    dispersion(c, a, false, true, false)
}
fn varpa(c: &mut dyn EvalContext, a: &mut dyn ArgList) -> EvalValue {
    dispersion(c, a, true, true, false)
}

pub(super) fn register(r: &mut Registry) {
    for d in [STDEV, STDEVA, STDEVP, STDEVPA, VAR, VARA, VARP, VARPA] {
        r.register(d);
    }
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{ErrorKind, Value};

    fn close(v: Value, want: f64) {
        match v {
            Value::Number(n) => assert!((n - want).abs() < 1e-12, "{n} vs {want}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn variants() {
        let mut c = MockEvalContext::new();
        for (i, x) in [2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0].iter().enumerate() {
            c.set(0, &format!("A{}", i + 1), *x);
        }
        let r = c.range(0, "A1:A8");
        close(call(&mut c, "STDEVP", vec![r.clone()]), 2.0);
        close(call(&mut c, "VARP", vec![r.clone()]), 4.0);
        close(call(&mut c, "VAR", vec![r.clone()]), 32.0 / 7.0);
        close(call(&mut c, "STDEV", vec![r]), (32.0f64 / 7.0).sqrt());
    }

    #[test]
    fn a_variants_and_errors() {
        let mut c = MockEvalContext::new();
        c.set(0, "A1", 2.0).set(0, "A2", "x").set(0, "A3", 4.0);
        let r = c.range(0, "A1:A3");
        close(call(&mut c, "VARA", vec![r.clone()]), 4.0);
        close(call(&mut c, "VAR", vec![r.clone()]), 2.0);
        close(call(&mut c, "VARPA", vec![r.clone()]), 8.0 / 3.0);
        close(
            call(&mut c, "STDEVPA", vec![r.clone()]),
            (8.0f64 / 3.0).sqrt(),
        );
        close(call(&mut c, "STDEVA", vec![r]), 2.0);
        assert_eq!(
            call(&mut c, "VAR", vec![EvalValue::number(1.0)]),
            Value::Error(ErrorKind::Div0)
        );
        close(call(&mut c, "VARP", vec![EvalValue::number(1.0)]), 0.0);
    }
}
