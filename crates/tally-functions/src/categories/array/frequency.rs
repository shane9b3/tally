//! FREQUENCY.
//!
//! After AOO `interpr5.cxx` `ScFrequency` (Apache-2.0): numbers of both
//! arguments are collected (text and empties skipped), the bins are sorted
//! for counting, and the result is a column of `bins + 1` counts written back
//! in the caller's original bin order (`aBinIndexOrder`:
//! `PutDouble(nCount, aBinIndexOrder[j])`). The last slot is the overflow.

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
        let mut bins: Vec<(f64, usize)> = Vec::new();
        a.for_each_number(1, |x| bins.push((x, bins.len())))?;
        // Stable sort: duplicate bins keep their order, so the first of
        // equal bins collects the values.
        bins.sort_by(|p, q| p.0.total_cmp(&q.0));
        let mut counts = vec![0.0; bins.len() + 1];
        for x in data {
            // First sorted bin with x <= bin; the last slot takes the overflow.
            let i = bins.partition_point(|b| b.0 < x);
            counts[i] += 1.0;
        }
        let n = bins.len();
        let mut out = Dense::zeros(n + 1, 1);
        for (j, &(_, orig)) in bins.iter().enumerate() {
            out.set(orig, 0, counts[j]);
        }
        out.set(n, 0, counts[n]);
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

    #[test]
    fn unsorted_bins_keep_caller_order() {
        // AOO interpr5.cxx ScFrequency: counts go back to aBinIndexOrder[j].
        let mut c = MockEvalContext::new();
        for (i, v) in [1.0, 2.0, 2.0, 3.0, 5.0].iter().enumerate() {
            c.set(0, &format!("A{}", i + 1), *v);
        }
        c.set(0, "B1", 4.0).set(0, "B2", 2.0);
        let (d, b) = (c.range(0, "A1:A5"), c.range(0, "B1:B2"));
        let r = call(&mut c, "FREQUENCY", vec![d, b]);
        assert_eq!(col(&r), vec![1.0, 3.0, 1.0]);
    }
}
