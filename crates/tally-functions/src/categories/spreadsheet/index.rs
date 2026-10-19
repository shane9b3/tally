//! INDEX (AOO `ScInterpreter::ScIndex`).

use std::sync::Arc;

use tally_types::{CellRange, ErrorKind, Matrix, Value};

use crate::registry::{Category, FnFlags, FunctionDef, Param, ParamKind, Registry, ReturnKind};
use crate::{run_ref, ArgList, Args, EvalContext, EvalValue};

const PARAMS: &[Param] = &[
    Param::required("reference", ParamKind::Reference),
    Param::optional("row", ParamKind::Integer),
    Param::optional("column", ParamKind::Integer),
    Param::optional("range", ParamKind::Integer),
];

/// `INDEX(reference; [row]; [column]; [range])`.
pub const INDEX: FunctionDef = FunctionDef::new("INDEX", Category::Spreadsheet, PARAMS, index)
    .returns(ReturnKind::Any)
    .flags(FnFlags::RETURNS_REF)
    .help("Returns a reference to a cell from a defined range.");

/// A reference argument yields a reference (so `SUM(INDEX(A1:C3;0;2))`
/// sums a column); an array yields a value or sub-array. Row or column 0
/// selects the whole column or row. If only a row is given for a
/// single-row range it is the column (`INDEX(A1:C1;2)` is B1). Out of range
/// is `#REF!`, negative `#VALUE!`. Only one area is supported, so `range`
/// must be 1 if given.
fn index(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run_ref(|| {
        let first = a.any(0);
        let (rows, cols) = match &first {
            EvalValue::Range(r) => (r.rows(), r.cols()),
            EvalValue::Array(m) | EvalValue::Scalar(Value::Array(m)) => (m.rows, m.cols),
            EvalValue::Scalar(Value::Error(e)) => return Err(*e),
            EvalValue::Scalar(_) => (1, 1),
            EvalValue::Missing => return Err(ErrorKind::Value),
        };
        let mut row = a.int_opt(1, 0)?;
        let col_missing = a.is_missing(2);
        let mut col = a.int_opt(2, 0)?;
        let area = a.int_opt(3, 1)?;
        if row < 0 || col < 0 || area < 0 {
            return Err(ErrorKind::Value);
        }
        if area != 1 {
            return Err(ErrorKind::Ref);
        }
        if col_missing && rows == 1 && cols > 1 {
            col = row;
            row = 0;
        }
        if row > i64::from(rows) || col > i64::from(cols) {
            return Err(ErrorKind::Ref);
        }
        let (rlo, rhi) = if row == 0 {
            (0, rows - 1)
        } else {
            (row as u32 - 1, row as u32 - 1)
        };
        let (clo, chi) = if col == 0 {
            (0, cols - 1)
        } else {
            (col as u32 - 1, col as u32 - 1)
        };
        match first {
            EvalValue::Range(r) => Ok(EvalValue::Range(CellRange::new(
                r.sheet,
                r.r0 + rlo,
                r.c0 + clo,
                r.r0 + rhi,
                r.c0 + chi,
            ))),
            EvalValue::Array(m) | EvalValue::Scalar(Value::Array(m)) => {
                let mut out = Matrix::new_filled(rhi - rlo + 1, chi - clo + 1, Value::Empty);
                for r in rlo..=rhi {
                    for c in clo..=chi {
                        let v = m.get(r, c).cloned().unwrap_or(Value::Empty);
                        out.set(r - rlo, c - clo, v);
                    }
                }
                if out.rows * out.cols == 1 {
                    Ok(EvalValue::from_value(
                        out.data.into_iter().next().unwrap_or_default(),
                    ))
                } else {
                    Ok(EvalValue::Array(Arc::new(out)))
                }
            }
            other => Ok(other),
        }
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(INDEX);
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, call_raw, MockArgs, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{CellRange, ErrorKind, Value};

    fn n(x: f64) -> EvalValue {
        EvalValue::number(x)
    }

    fn sheet() -> MockEvalContext {
        let mut c = MockEvalContext::new();
        let mut k = 1.0;
        for r in 1..=3 {
            for col in ['A', 'B', 'C'] {
                c.set(0, &format!("{col}{r}"), k);
                k += 1.0;
            }
        }
        c
    }

    #[test]
    fn picks_cell() {
        let mut c = sheet();
        let r = c.range(0, "A1:C3");
        assert_eq!(
            call(&mut c, "INDEX", vec![r.clone(), n(2.0), n(3.0)]),
            Value::Number(6.0)
        );
        assert_eq!(
            call(&mut c, "INDEX", vec![r.clone(), n(4.0), n(1.0)]),
            Value::Error(ErrorKind::Ref)
        );
        assert_eq!(
            call(&mut c, "INDEX", vec![r, n(-1.0), n(1.0)]),
            Value::Error(ErrorKind::Value)
        );
    }

    #[test]
    fn returns_reference() {
        let mut c = sheet();
        let r = c.range(0, "A1:C3");
        let mut args = MockArgs::new(vec![r.clone(), n(0.0), n(2.0)]);
        let raw = call_raw(&mut c, "INDEX", &mut args);
        assert_eq!(
            raw,
            EvalValue::Range(CellRange::from_a1(0, "B1:B3").expect("range"))
        );
        let mut args = MockArgs::new(vec![r.clone(), n(3.0), EvalValue::Missing]);
        let raw = call_raw(&mut c, "INDEX", &mut args);
        assert_eq!(
            raw,
            EvalValue::Range(CellRange::from_a1(0, "A3:C3").expect("range"))
        );
        let mut args = MockArgs::new(vec![r, n(1.0), n(1.0), n(2.0)]);
        assert_eq!(
            call_raw(&mut c, "INDEX", &mut args),
            EvalValue::error(ErrorKind::Ref)
        );
    }

    #[test]
    fn single_row_takes_column() {
        let mut c = sheet();
        let r = c.range(0, "A1:C1");
        assert_eq!(call(&mut c, "INDEX", vec![r, n(2.0)]), Value::Number(2.0));
    }
}
