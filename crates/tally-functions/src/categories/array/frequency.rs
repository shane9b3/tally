//! FREQUENCY.
//!
//! After AOO `interpr3.cxx` `ScFrequency` (Apache-2.0): numbers of both
//! arguments are collected (text and empties skipped), the bins are sorted,
//! and the result is a column of `bins + 1` counts.

use super::mat::Dense;
use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry, ReturnKind};
use crate::{run_ref, ArgList, Args, EvalContext, EvalValue};

const PARAMS: &[Param] = &[
    Param::required("data", ParamKind::Array),
    Param::required("classes", ParamKind::Array),
];

/// `FREQUENCY(data; classes)`.
pub const FREQUENCY: FunctionDef =
    FunctionDef::new("FREQUENCY", Category::Array, PARAMS, frequency)
        .returns(ReturnKind::Array)
        .help("Categorizes values into intervals and counts the values in each interval.");

fn frequency(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run_ref(|| {
        let mut data = Vec::new();
        a.for_each_number(0, |x| data.push(x))?;
        let mut bins = Vec::new();
        a.for_each_number(1, |x| bins.push(x))?;
        bins.sort_by(f64::total_cmp);
        let mut out = Dense::zeros(bins.len() + 1, 1);
        for x in data {
            // First bin with x <= bin; the last slot takes the overflow.
            let i = bins.partition_point(|b| *b < x);
            out.set(i, 0, out.at(i, 0) + 1.0);
        }
        Ok(out.into_eval())
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(FREQUENCY);
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use tally_types::Value;

    fn col(v: &Value) -> Vec<f64> {
        match v {
            Value::Array(m) => m.data.iter().filter_map(Value::as_number).collect(),
            _ => vec![],
        }
    }

    #[test]
    fn counts() {
        let mut c = MockEvalContext::new();
        for i in 1..=5 {
            c.set(0, &format!("A{i}"), f64::from(i));
        }
        c.set(0, "B1", 4.0).set(0, "B2", 2.0).set(0, "A6", "x");
        let (d, b) = (c.range(0, "A1:A6"), c.range(0, "B1:B2"));
        let r = call(&mut c, "FREQUENCY", vec![d.clone(), b]);
        assert_eq!(col(&r), vec![2.0, 2.0, 1.0]);
        let dup = c.range(0, "B1:B1");
        let r = call(&mut c, "FREQUENCY", vec![d, dup]);
        assert_eq!(col(&r), vec![4.0, 1.0]);
    }
}
