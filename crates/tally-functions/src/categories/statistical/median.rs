//! MEDIAN.

use tally_types::{ErrorKind, Value};

use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const PARAMS: &[Param] = &[Param::repeat("number", ParamKind::NumberSeq)];

/// `MEDIAN(number; ...)`.
pub const MEDIAN: FunctionDef = FunctionDef::new("MEDIAN", Category::Statistical, PARAMS, median)
    .help("Returns the median of a given sample.");

/// Sorts the numbers (SUM collection rules); an even count averages the
/// two middle values. No numbers is `#VALUE!` (AOO `ScMedian` pushes
/// NoValue, which LibreOffice also reports as `#VALUE!`).
fn median(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let mut v: Vec<f64> = Vec::new();
        a.for_each_number_all(|x| v.push(x))?;
        if v.is_empty() {
            return Err(ErrorKind::Value);
        }
        v.sort_by(f64::total_cmp);
        let n = v.len();
        let mid = n / 2;
        let m = if n % 2 == 1 {
            v[mid]
        } else {
            (v[mid - 1] + v[mid]) / 2.0
        };
        Ok(Value::Number(m))
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(MEDIAN);
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{ErrorKind, Value};

    #[test]
    fn median_cases() {
        let mut c = MockEvalContext::new();
        c.set(0, "A1", 5.0)
            .set(0, "A2", "x")
            .set(0, "A3", 1.0)
            .set(0, "A4", 3.0)
            .set(0, "B1", Value::Error(ErrorKind::Div0));
        let r = c.range(0, "A1:A4");
        assert_eq!(call(&mut c, "MEDIAN", vec![r]), Value::Number(3.0));
        let r = c.range(0, "A1:A3");
        assert_eq!(call(&mut c, "MEDIAN", vec![r]), Value::Number(3.0));
        let r = c.range(0, "A2");
        assert_eq!(
            call(&mut c, "MEDIAN", vec![r]),
            Value::Error(ErrorKind::Value)
        );
        let r = c.range(0, "B1");
        assert_eq!(
            call(&mut c, "MEDIAN", vec![r]),
            Value::Error(ErrorKind::Div0)
        );
        let args = vec![
            EvalValue::number(4.0),
            EvalValue::number(1.0),
            EvalValue::number(2.0),
            EvalValue::number(9.0),
        ];
        assert_eq!(call(&mut c, "MEDIAN", args), Value::Number(3.0));
    }
}
