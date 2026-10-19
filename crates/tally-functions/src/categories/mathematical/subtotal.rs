//! SUBTOTAL.
//!
//! Function codes 1 to 11 are AVERAGE, COUNT, COUNTA, MAX, MIN, PRODUCT,
//! STDEV, STDEVP, SUM, VAR, VARP; 101 to 111 are the same functions that
//! additionally ignore manually hidden rows.
//!
//! As in AOO (`ScInterpreter::ScSubTotal`) cells whose formula itself
//! contains a `SUBTOTAL(` call are skipped, so subtotals do not double count.
//! Detection goes through `EvalContext::cell_format_info(..).formula`.
//!
//! Codes 101 to 111 skip rows for which [`EvalContext::hidden_row`] is true
//! (hidden or filtered); the default implementation hides nothing, so they
//! behave like 1 to 11 until the engine reports row visibility. Codes 1 to
//! 11 include hidden rows. (AOO also drops filtered rows for 1 to 11; that
//! needs a separate filtered-row query and is not modelled yet.)

use tally_types::{CellRange, ErrorKind, Value};

use crate::numeric::ApproxSum;
use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const PARAMS: &[Param] = &[
    Param::required("function", ParamKind::Integer),
    Param::required("range", ParamKind::Reference),
];

/// `SUBTOTAL(function; range)`.
pub const SUBTOTAL: FunctionDef =
    FunctionDef::new("SUBTOTAL", Category::Mathematical, PARAMS, subtotal)
        .help("Calculates subtotals in a spreadsheet.");

/// Everything one SUBTOTAL needs from the range.
#[derive(Default)]
struct Collected {
    numbers: Vec<f64>,
    non_empty: u64,
    error: Option<ErrorKind>,
}

impl Collected {
    fn add(&mut self, v: &Value) {
        match v {
            Value::Empty => return,
            Value::Number(x) => self.numbers.push(*x),
            Value::Bool(b) => self.numbers.push(if *b { 1.0 } else { 0.0 }),
            Value::Error(e) => {
                if self.error.is_none() {
                    self.error = Some(*e);
                }
            }
            _ => {}
        }
        self.non_empty += 1;
    }
}

fn is_subtotal_formula(text: &str) -> bool {
    text.to_ascii_uppercase().contains("SUBTOTAL(")
}

fn collect(ctx: &mut dyn EvalContext, range: &CellRange, skip_hidden: bool) -> Collected {
    let mut cells = Vec::new();
    ctx.for_each_in_range(range, true, &mut |addr, v| {
        cells.push((addr, v.clone()));
        std::ops::ControlFlow::Continue(())
    });
    let mut out = Collected::default();
    for (addr, v) in cells {
        if skip_hidden && ctx.hidden_row(addr.sheet, addr.row) {
            continue;
        }
        let info = ctx.cell_format_info(addr);
        if info.formula.as_deref().is_some_and(is_subtotal_formula) {
            continue;
        }
        out.add(&v);
    }
    out
}

fn variance(xs: &[f64], sample: bool) -> Result<f64, ErrorKind> {
    let n = xs.len();
    if n == 0 || (sample && n < 2) {
        return Err(ErrorKind::Div0);
    }
    let mut s = ApproxSum::new();
    xs.iter().for_each(|x| s.add(*x));
    let mean = s.total() / n as f64;
    let ss: f64 = xs.iter().map(|x| (x - mean) * (x - mean)).sum();
    Ok(ss / if sample { (n - 1) as f64 } else { n as f64 })
}

fn subtotal(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let code = a.int(0)?;
        let (code, skip_hidden) = match code {
            1..=11 => (code, false),
            101..=111 => (code - 100, true),
            _ => return Err(ErrorKind::InvalidArgument),
        };
        let c = match a.any(1) {
            EvalValue::Range(r) => collect(a.ctx(), &r, skip_hidden),
            EvalValue::Array(m) => {
                let mut c = Collected::default();
                m.data.iter().for_each(|v| c.add(v));
                c
            }
            EvalValue::Scalar(Value::Error(e)) => return Err(e),
            _ => return Err(ErrorKind::Value),
        };
        // COUNT and COUNTA never propagate errors.
        match code {
            2 => return Ok(Value::Number(c.numbers.len() as f64)),
            3 => return Ok(Value::Number(c.non_empty as f64)),
            _ => {}
        }
        if let Some(e) = c.error {
            return Err(e);
        }
        let xs = &c.numbers;
        let r = match code {
            1 => {
                if xs.is_empty() {
                    return Err(ErrorKind::Div0);
                }
                let mut s = ApproxSum::new();
                xs.iter().for_each(|x| s.add(*x));
                s.total() / xs.len() as f64
            }
            4 => xs
                .iter()
                .copied()
                .fold(None, |m: Option<f64>, x| Some(m.map_or(x, |m| m.max(x))))
                .unwrap_or(0.0),
            5 => xs
                .iter()
                .copied()
                .fold(None, |m: Option<f64>, x| Some(m.map_or(x, |m| m.min(x))))
                .unwrap_or(0.0),
            6 => {
                if xs.is_empty() {
                    0.0
                } else {
                    xs.iter().product()
                }
            }
            7 => variance(xs, true)?.sqrt(),
            8 => variance(xs, false)?.sqrt(),
            9 => {
                let mut s = ApproxSum::new();
                xs.iter().for_each(|x| s.add(*x));
                s.total()
            }
            10 => variance(xs, true)?,
            _ => variance(xs, false)?,
        };
        Ok(Value::Number(r))
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(SUBTOTAL);
}

#[cfg(test)]
mod tests {
    use crate::context::CellFormatInfo;
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{ErrorKind, Value};

    /// A1:A6 = 1, "x", TRUE, 4, (empty), A6 = a nested SUBTOTAL(9; A1:A4) = 6.
    fn grid() -> MockEvalContext {
        let mut c = MockEvalContext::new();
        c.set(0, "A1", 1.0).set(0, "A2", "x").set(0, "A3", true);
        c.set(0, "A4", 4.0).set(0, "A6", 6.0);
        let info = CellFormatInfo {
            has_formula: true,
            formula: Some("=SUBTOTAL(9;A1:A4)".into()),
            ..CellFormatInfo::default()
        };
        c.set_format(0, "A6", info);
        c
    }

    fn sub(c: &mut MockEvalContext, code: f64) -> Value {
        let r = c.range(0, "A1:A6");
        call(c, "SUBTOTAL", vec![EvalValue::number(code), r])
    }

    #[test]
    fn functions_skip_nested_subtotals() {
        let mut c = grid();
        assert_eq!(sub(&mut c, 9.0), Value::Number(6.0));
        assert_eq!(sub(&mut c, 109.0), Value::Number(6.0));
        assert_eq!(sub(&mut c, 1.0), Value::Number(2.0));
        assert_eq!(sub(&mut c, 2.0), Value::Number(3.0));
        assert_eq!(sub(&mut c, 3.0), Value::Number(4.0));
        assert_eq!(sub(&mut c, 4.0), Value::Number(4.0));
        assert_eq!(sub(&mut c, 5.0), Value::Number(1.0));
        assert_eq!(sub(&mut c, 6.0), Value::Number(4.0));
        assert_eq!(sub(&mut c, 10.0), Value::Number(3.0));
        assert_eq!(sub(&mut c, 11.0), Value::Number(2.0));
    }

    #[test]
    fn codes_101_to_111_skip_hidden_rows() {
        let mut c = grid();
        c.hidden_rows.insert((0, 3)); // A4 = 4
        assert_eq!(sub(&mut c, 9.0), Value::Number(6.0));
        assert_eq!(sub(&mut c, 109.0), Value::Number(2.0));
        assert_eq!(sub(&mut c, 102.0), Value::Number(2.0));
        assert_eq!(sub(&mut c, 103.0), Value::Number(3.0));
        assert_eq!(sub(&mut c, 104.0), Value::Number(1.0));
        c.hidden_rows.insert((1, 0)); // another sheet: no effect
        assert_eq!(sub(&mut c, 101.0), Value::Number(1.0));
    }

    #[test]
    fn errors() {
        let mut c = grid();
        assert_eq!(sub(&mut c, 12.0), Value::Error(ErrorKind::InvalidArgument));
        assert_eq!(sub(&mut c, 0.0), Value::Error(ErrorKind::InvalidArgument));
        c.set(0, "A5", Value::Error(ErrorKind::Div0));
        assert_eq!(sub(&mut c, 9.0), Value::Error(ErrorKind::Div0));
        assert_eq!(sub(&mut c, 2.0), Value::Number(3.0));
        assert_eq!(sub(&mut c, 3.0), Value::Number(5.0));
        let r = EvalValue::number(1.0);
        assert_eq!(
            call(&mut c, "SUBTOTAL", vec![EvalValue::number(9.0), r]),
            Value::Error(ErrorKind::Value)
        );
    }
}
