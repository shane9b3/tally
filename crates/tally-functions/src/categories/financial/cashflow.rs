//! Periodic cash flows: NPV, IRR, MIRR.
//!
//! Ported from Apache OpenOffice `sc/source/core/tool/interpr2.cxx`
//! (`ScNPV`, `ScIRR`, `ScMIRR`), Apache License 2.0.
//!
//! AOO accepts only cell references for the cash flows of IRR and MIRR
//! (and NPV's list); inline and computed arrays give `Err:504` there.
//! Tally also accepts arrays, as Excel and LibreOffice do.

use std::ops::ControlFlow;

use tally_types::{ErrorKind, Value};

use super::tvm::SC_EPSILON;
use crate::coerce::number_seq;
use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

/// Numbers of a cash-flow argument in AOO value-iterator order: references
/// and arrays skip text and empty cells (booleans count), errors propagate.
/// `row_major` selects `ScHorizontalValueIterator` order (NPV); otherwise
/// column-major (`ScValueIterator`, IRR/MIRR).
fn flows(a: &mut Args<'_>, i: usize, row_major: bool) -> Result<Vec<f64>, ErrorKind> {
    let v = a.any(i);
    let mut out = Vec::new();
    match &v {
        EvalValue::Range(r) if row_major => {
            let mut cells: Vec<(u32, u32, f64)> = Vec::new();
            let mut err: Option<(u32, u32, ErrorKind)> = None;
            a.ctx().for_each_in_range(r, true, &mut |addr, val| {
                match val {
                    Value::Number(n) => cells.push((addr.row, addr.col, *n)),
                    Value::Bool(b) => cells.push((addr.row, addr.col, f64::from(u8::from(*b)))),
                    Value::Error(e) => {
                        if err.is_none_or(|(r0, c0, _)| (addr.row, addr.col) < (r0, c0)) {
                            err = Some((addr.row, addr.col, *e));
                        }
                    }
                    _ => {}
                }
                ControlFlow::Continue(())
            });
            if let Some((_, _, e)) = err {
                return Err(e);
            }
            cells.sort_by_key(|&(r, c, _)| (r, c));
            out.extend(cells.into_iter().map(|(_, _, n)| n));
        }
        EvalValue::Array(m) if row_major => {
            for r in 0..m.rows {
                for c in 0..m.cols {
                    match m.get(r, c) {
                        Some(Value::Number(n)) => out.push(*n),
                        Some(Value::Bool(b)) => out.push(f64::from(u8::from(*b))),
                        Some(Value::Error(e)) => return Err(*e),
                        _ => {}
                    }
                }
            }
        }
        _ => number_seq(&v, a.ctx(), |n| out.push(n))?,
    }
    Ok(out)
}

const NPV_PARAMS: &[Param] = &[
    Param::required("rate", ParamKind::Number),
    Param::repeat("value", ParamKind::NumberSeq),
];

/// `NPV(rate; value; ...)`: at most 30 values arguments, as in AOO.
pub const NPV: FunctionDef = FunctionDef::new("NPV", Category::Financial, NPV_PARAMS, npv)
    .arity(2, 31)
    .help("Net present value. Calculates the net present value of an investment based on a series of periodic payments and a discount rate.");

fn npv(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let rate = a.num(0)?;
        let mut sum = 0.0;
        let mut count = 1.0;
        for i in 1..a.count() {
            for x in flows(&mut a, i, true)? {
                sum += x / (1.0 + rate).powf(count);
                count += 1.0;
            }
        }
        Ok(Value::Number(sum))
    })
}

/// IRR and MIRR take a reference (or, in Tally, an array); a plain scalar
/// is `Err:504` as in AOO.
fn range_flows(a: &mut Args<'_>, i: usize) -> Result<Vec<f64>, ErrorKind> {
    match a.any(i) {
        EvalValue::Range(_) | EvalValue::Array(_) | EvalValue::Scalar(Value::Array(_)) => {
            flows(a, i, false)
        }
        EvalValue::Scalar(Value::Error(e)) => Err(e),
        _ => Err(ErrorKind::ParamList),
    }
}

const IRR_PARAMS: &[Param] = &[
    Param::required("values", ParamKind::NumberSeq),
    Param::optional("guess", ParamKind::Number),
];

/// `IRR(values; [guess])`.
pub const IRR: FunctionDef = FunctionDef::new("IRR", Category::Financial, IRR_PARAMS, irr)
    .help("Returns the actuarial rate of interest of an investment excluding costs or profits.");

fn irr(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let guess = if a.count() == 2 { a.num(1)? } else { 0.1 };
        let values = range_flows(&mut a, 0)?;
        let mut x = if guess == -1.0 { 0.1 } else { guess };
        let mut eps = 1.0;
        let mut iter = 0;
        // Newton: x(i+1) = x(i) - f(x(i)) / f'(x(i)), at most 20 steps.
        while eps > SC_EPSILON && iter < 20 {
            let mut num = 0.0;
            let mut den = 0.0;
            for (k, &v) in values.iter().enumerate() {
                let k = k as f64;
                num += v / (1.0 + x).powf(k);
                den += -k * v / (1.0 + x).powf(k + 1.0);
            }
            let xn = x - num / den;
            iter += 1;
            eps = (xn - x).abs();
            x = xn;
        }
        if guess == 0.0 && x.abs() < SC_EPSILON {
            x = 0.0;
        }
        if eps < SC_EPSILON {
            Ok(Value::Number(x))
        } else {
            Err(ErrorKind::NoConvergence)
        }
    })
}

const MIRR_PARAMS: &[Param] = &[
    Param::required("values", ParamKind::NumberSeq),
    Param::required("investment", ParamKind::Number),
    Param::required("reinvest_rate", ParamKind::Number),
];

/// `MIRR(values; investment; reinvest_rate)`.
pub const MIRR: FunctionDef = FunctionDef::new("MIRR", Category::Financial, MIRR_PARAMS, mirr)
    .help("Returns the modified internal rate of return for a series of investments.");

fn mirr(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let r_invest = a.num(1)? + 1.0;
        let r_reinvest = a.num(2)? + 1.0;
        let values = range_flows(&mut a, 0)?;
        let (mut npv_re, mut pow_re) = (0.0, 1.0);
        let (mut npv_in, mut pow_in) = (0.0, 1.0);
        for &v in &values {
            if v > 0.0 {
                npv_re += v * pow_re;
            } else if v < 0.0 {
                npv_in += v * pow_in;
            }
            pow_re /= r_reinvest;
            pow_in /= r_invest;
        }
        let n = values.len() as f64;
        let mut res = -npv_re / npv_in;
        res *= r_reinvest.powf(n - 1.0);
        res = res.powf(1.0 / (n - 1.0));
        Ok(Value::Number(res - 1.0))
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(NPV);
    r.register(IRR);
    r.register(MIRR);
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{ErrorKind, Matrix, Value};

    fn row(xs: &[f64]) -> EvalValue {
        let mut m = Matrix::new_filled(1, xs.len() as u32, Value::Empty);
        for (i, &x) in xs.iter().enumerate() {
            m.set(0, i as u32, Value::Number(x));
        }
        EvalValue::Array(Arc::new(m))
    }

    fn num(v: Value) -> f64 {
        match v {
            Value::Number(n) => n,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn npv_irr_mirr_arrays() {
        let mut c = MockEvalContext::new();
        let v = num(call(
            &mut c,
            "NPV",
            vec![
                EvalValue::number(0.1),
                row(&[-10000.0, 3000.0, 4200.0, 6800.0]),
            ],
        ));
        assert!((v - 1188.443412).abs() < 1e-5, "{v}");
        let v = num(call(
            &mut c,
            "IRR",
            vec![row(&[
                -70000.0, 12000.0, 15000.0, 18000.0, 21000.0, 26000.0,
            ])],
        ));
        assert!((v - 0.0866309480365).abs() < 1e-10, "{v}");
        let v = num(call(
            &mut c,
            "MIRR",
            vec![
                row(&[-120000.0, 39000.0, 30000.0, 21000.0, 37000.0, 46000.0]),
                EvalValue::number(0.1),
                EvalValue::number(0.12),
            ],
        ));
        assert!((v - 0.126094130).abs() < 1e-8, "{v}");
    }

    #[test]
    fn ranges_skip_text_and_scalars_rejected() {
        let mut c = MockEvalContext::new();
        c.set(0, "A1", -100.0).set(0, "A2", "x").set(0, "A3", 110.0);
        let r = c.range(0, "A1:A3");
        let v = num(call(&mut c, "IRR", vec![r.clone()]));
        assert!((v - 0.1).abs() < 1e-9, "{v}");
        assert_eq!(
            call(&mut c, "IRR", vec![EvalValue::number(1.0)]),
            Value::Error(ErrorKind::ParamList)
        );
        c.set(0, "B1", 1.0).set(0, "C1", 2.0).set(0, "B2", 3.0);
        let r = c.range(0, "B1:C2");
        // Row-major: 1, 2, 3.
        let v = num(call(&mut c, "NPV", vec![EvalValue::number(1.0), r]));
        assert!((v - (0.5 + 0.5 + 3.0 / 8.0)).abs() < 1e-12, "{v}");
    }
}
