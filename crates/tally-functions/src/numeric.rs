//! Numeric helpers with AOO semantics for function authors.

use tally_types::approx_equal;

/// AOO `rtl::math::approxAdd`: `a + b`, but exactly 0 when the operands
/// cancel within [`approx_equal`] (so `0.1 + 0.2 - 0.3` sums to 0).
///
/// Ported from Apache OpenOffice `sal/inc/rtl/math.hxx` (Apache-2.0).
pub fn approx_add(a: f64, b: f64) -> f64 {
    if ((a < 0.0 && b > 0.0) || (b < 0.0 && a > 0.0)) && approx_equal(a, -b) {
        return 0.0;
    }
    a + b
}

/// AOO's SUM accumulator (`ScInterpreter::IterateParameters`, ifSUM): the
/// first non-zero value is held apart and added back with [`approx_add`]
/// at the end, which cancels the classic `0.1 + 0.2 - 0.3` residue.
#[derive(Copy, Clone, Debug, Default)]
pub struct ApproxSum {
    first: Option<f64>,
    rest: f64,
    count: u64,
}

impl ApproxSum {
    /// An empty sum.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds one value.
    pub fn add(&mut self, x: f64) {
        self.count += 1;
        if self.first.is_none() && x != 0.0 {
            self.first = Some(x);
        } else {
            self.rest += x;
        }
    }

    /// Number of values added (including zeros).
    pub fn count(&self) -> u64 {
        self.count
    }

    /// The total.
    pub fn total(&self) -> f64 {
        approx_add(self.rest, self.first.unwrap_or(0.0))
    }
}
