//! Helpers shared by the descriptive-statistics families.

use tally_types::{ErrorKind, Value};

use crate::coerce;
use crate::stats_util::SparseData;
use crate::{Args, EvalValue};

/// A matrix-like argument (the shared sparse column-major reader).
pub(super) type Data = SparseData;

pub(super) use crate::stats_util::cell_number as as_number;

/// Reads argument `i` as a [`Data`] (reference, array or scalar). A scalar
/// error propagates.
pub(super) fn data(a: &mut Args, i: usize) -> Result<Data, ErrorKind> {
    match a.any(i) {
        EvalValue::Scalar(Value::Error(e)) => Err(e),
        v => Ok(SparseData::read(a.ctx(), v)),
    }
}

/// Collects every number of every argument (SUM rules, or with text in
/// references counted as 0 when `text_as_zero`).
pub(super) fn seq_numbers(a: &mut Args, text_as_zero: bool) -> Result<Vec<f64>, ErrorKind> {
    let mut out = Vec::new();
    for i in 0..a.count() {
        let v = a.any(i);
        if text_as_zero {
            coerce::number_seq_with_text_as_zero(&v, a.ctx(), |x| out.push(x))?;
        } else {
            coerce::number_seq(&v, a.ctx(), |x| out.push(x))?;
        }
    }
    Ok(out)
}

/// Arithmetic mean of a non-empty slice.
pub(super) fn mean(v: &[f64]) -> f64 {
    v.iter().sum::<f64>() / v.len() as f64
}

/// Sum of squared deviations from `m`.
pub(super) fn sum_sq_dev(v: &[f64], m: f64) -> f64 {
    v.iter().map(|x| (x - m) * (x - m)).sum()
}
