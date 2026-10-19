//! Add-in functions: the financial family of the Analysis add-in.
//!
//! Ported from Apache OpenOffice scaddins/analysis (analysishelper.cxx,
//! financial.cxx), Apache License 2.0. Argument conversion follows
//! `ScInterpreter::ScExternal` (`sc/source/core/tool/interpr4.cxx`) and
//! `ScRangeToSequence` (`rangeseq.cxx`):
//!
//! - `long` parameters (dates, frequency) are truncated toward zero;
//!   values outside the `i32` range are `Err:502`;
//! - optional `any` parameters (basis, par, guess) take their default when
//!   omitted or given as empty text; other text must be numeric (`Err:502`);
//! - array parameters (XIRR, XNPV, FVSCHEDULE) take every cell of a range
//!   in row-major order, text and empty cells as 0; an error cell in the
//!   range is `Err:504`;
//! - an `IllegalArgumentException` (domain error) and any non-finite
//!   result are `Err:502` (`RETURN_FINITE`).

use std::ops::ControlFlow;

use tally_types::{ErrorKind, Value};

use crate::coerce::{self, parse_unambiguous};
use crate::registry::Registry;
use crate::{Args, EvalValue};

mod bonds;
mod coupon;
mod daycount;
mod misc;
mod securities;

use daycount::IAE;

/// Registers the financial functions.
pub fn register(r: &mut Registry) {
    coupon::register(r);
    securities::register(r);
    bonds::register(r);
    misc::register(r);
}

/// AOO `RETURN_FINITE`: a non-finite result is an illegal argument.
fn finite(x: f64) -> Result<Value, ErrorKind> {
    if x.is_finite() {
        Ok(Value::Number(x))
    } else {
        Err(IAE)
    }
}

/// The document's null date as an AOO day number.
fn null_date(a: &mut Args<'_>) -> i64 {
    daycount::null_date(a.ctx().settings().date_epoch)
}

/// A `long` parameter (a serial date, a frequency): truncated toward zero.
fn long(a: &mut Args<'_>, i: usize) -> Result<i64, ErrorKind> {
    a.int(i)
}

/// An optional `any` parameter read with `ScaAnyConverter::getDouble`:
/// omitted or `""` gives `default`, an empty cell 0, numeric text its
/// value, other text `Err:502`.
fn any_num(a: &mut Args<'_>, i: usize, default: f64) -> Result<f64, ErrorKind> {
    if a.is_missing(i) {
        return Ok(default);
    }
    match a.value(i) {
        Value::Empty => Ok(0.0),
        Value::Number(n) => Ok(n),
        Value::Bool(b) => Ok(f64::from(u8::from(b))),
        Value::Text(s) if s.is_empty() => Ok(default),
        Value::Text(s) => {
            let epoch = a.ctx().settings().date_epoch;
            parse_unambiguous(&s, epoch).ok_or(IAE)
        }
        Value::Error(e) => Err(e),
        Value::Array(_) => Err(IAE),
    }
}

/// An optional `any` integer (`ScaAnyConverter::getInt32`): truncated,
/// outside the `i32` range `Err:502`.
fn any_int(a: &mut Args<'_>, i: usize, default: i32) -> Result<i32, ErrorKind> {
    let x = any_num(a, i, f64::from(default))?;
    if x <= -2_147_483_649.0 || x >= 2_147_483_648.0 || x.is_nan() {
        return Err(IAE);
    }
    Ok(x.trunc() as i32)
}

/// `AnalysisAddIn::getDateMode`: the optional basis, 0..=4.
fn basis(a: &mut Args<'_>, i: usize) -> Result<i32, ErrorKind> {
    let b = any_int(a, i, 0)?;
    if (0..=4).contains(&b) {
        Ok(b)
    } else {
        Err(IAE)
    }
}

/// Upper bound on the cells of an array argument.
const MAX_ARRAY_CELLS: u64 = 1 << 22;

/// A `double[][]` parameter flattened row by row (see the module docs).
fn double_array(a: &mut Args<'_>, i: usize) -> Result<Vec<f64>, ErrorKind> {
    match a.any(i) {
        EvalValue::Range(r) if !r.is_single() => {
            if r.cell_count() > MAX_ARRAY_CELLS {
                return Err(ErrorKind::ParamList);
            }
            let cols = r.cols() as usize;
            let mut out = vec![0.0; r.cell_count() as usize];
            let mut bad = false;
            let (r0, c0) = (r.r0, r.c0);
            a.ctx().for_each_in_range(&r, true, &mut |addr, v| {
                let k = (addr.row - r0) as usize * cols + (addr.col - c0) as usize;
                match v {
                    Value::Number(n) => out[k] = *n,
                    Value::Bool(b) => out[k] = f64::from(u8::from(*b)),
                    Value::Error(_) => {
                        bad = true;
                        return ControlFlow::Break(());
                    }
                    _ => {}
                }
                ControlFlow::Continue(())
            });
            if bad {
                return Err(ErrorKind::ParamList);
            }
            Ok(out)
        }
        EvalValue::Array(m) | EvalValue::Scalar(Value::Array(m)) => {
            let mut out = Vec::with_capacity(m.data.len());
            for r in 0..m.rows {
                for c in 0..m.cols {
                    match m.get(r, c) {
                        Some(Value::Number(n)) => out.push(*n),
                        Some(Value::Bool(b)) => out.push(f64::from(u8::from(*b))),
                        Some(Value::Error(_)) => return Err(ErrorKind::ParamList),
                        _ => out.push(0.0),
                    }
                }
            }
            Ok(out)
        }
        v => Ok(vec![coerce::to_number(&v, a.ctx())?]),
    }
}

/// A `long[][]` parameter: [`double_array`] with each value truncated
/// toward zero (out-of-range values become 0, as in AOO `lcl_DoubleToLong`).
fn long_array(a: &mut Args<'_>, i: usize) -> Result<Vec<f64>, ErrorKind> {
    let single = !matches!(a.any(i), EvalValue::Range(r) if !r.is_single())
        && !matches!(
            a.any(i),
            EvalValue::Array(_) | EvalValue::Scalar(Value::Array(_))
        );
    let mut v = double_array(a, i)?;
    for x in &mut v {
        let t = coerce::approx_floor(x.abs()).copysign(*x);
        *x = if t.abs() <= f64::from(i32::MAX) {
            t
        } else if single {
            return Err(IAE);
        } else {
            0.0
        };
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use crate::registry;

    /// Name clashes with built-ins (docs/phase-a.md): plain CUMIPMT,
    /// CUMPRINC, EFFECT and NOMINAL mean the built-in in every table;
    /// plain DURATION means the add-in in ODF and OOXML.
    #[test]
    fn add_names_resolve_like_aoo() {
        let r = registry();
        let name = |id: Option<crate::FuncId>| id.and_then(|i| r.get(i)).map(|d| d.name);
        for (plain, builtin) in [
            ("CUMIPMT", "CUMIPMT"),
            ("CUMPRINC", "CUMPRINC"),
            ("EFFECT", "EFFECTIVE"),
            ("NOMINAL", "NOMINAL"),
        ] {
            assert_eq!(name(r.lookup_odf(plain)), Some(builtin), "{plain}");
            assert_eq!(name(r.lookup_ooxml(plain)), Some(builtin), "{plain}");
        }
        assert_eq!(name(r.lookup_odf("DURATION")), Some("DURATION_ADD"));
        assert_eq!(name(r.lookup_ooxml("DURATION")), Some("DURATION_ADD"));
        assert_eq!(name(r.lookup_odf("PDURATION")), Some("DURATION"));
        assert_eq!(name(r.lookup_ooxml("_xlfn.PDURATION")), Some("DURATION"));
        assert_eq!(name(r.lookup_ooxml("_xlfn.RRI")), Some("ZGZ"));
        assert_eq!(
            name(r.lookup_odf("com.sun.star.sheet.addin.Analysis.getCumipmt")),
            Some("CUMIPMT_ADD")
        );
        assert_eq!(name(r.lookup("EFFECT_ADD")), Some("EFFECT_ADD"));
    }
}
