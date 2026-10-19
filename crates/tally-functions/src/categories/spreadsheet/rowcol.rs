//! ROW and COLUMN.

use tally_types::{ErrorKind, Value};

use crate::registry::{Category, FnFlags, FunctionDef, Param, ParamKind, Registry};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const PARAMS: &[Param] = &[Param::optional("reference", ParamKind::Reference)];

/// `ROW([reference])`.
pub const ROW: FunctionDef = FunctionDef::new("ROW", Category::Spreadsheet, PARAMS, row)
    .flags(FnFlags::NEEDS_ORIGIN.union(FnFlags::REF_ADDRESS_ONLY))
    .help("Returns the row number of a reference.");

/// `COLUMN([reference])`.
pub const COLUMN: FunctionDef = FunctionDef::new("COLUMN", Category::Spreadsheet, PARAMS, column)
    .flags(FnFlags::NEEDS_ORIGIN.union(FnFlags::REF_ADDRESS_ONLY))
    .help("Returns the column number of a reference.");

// TODO: in array context a multi-row/column reference should yield an array
// of row/column numbers (`{ROW(A1:A3)}` = {1;2;3}); only the scalar case
// (first row/column) is implemented.
fn position(ctx: &mut dyn EvalContext, args: &mut dyn ArgList, want_row: bool) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let n = if a.is_missing(0) {
            let o = a.ctx().origin();
            if want_row {
                o.row
            } else {
                o.col
            }
        } else {
            match a.any(0) {
                EvalValue::Range(r) => {
                    if want_row {
                        r.r0
                    } else {
                        r.c0
                    }
                }
                EvalValue::Scalar(Value::Error(e)) => return Err(e),
                _ => return Err(ErrorKind::Value),
            }
        };
        Ok(Value::Number(f64::from(n) + 1.0))
    })
}

fn row(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    position(ctx, args, true)
}

fn column(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    position(ctx, args, false)
}

pub(super) fn register(r: &mut Registry) {
    r.register(ROW);
    r.register(COLUMN);
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{ErrorKind, Value};

    #[test]
    fn origin_and_reference() {
        let mut c = MockEvalContext::new();
        c.set_origin(0, "C7");
        assert_eq!(call(&mut c, "ROW", vec![]), Value::Number(7.0));
        assert_eq!(call(&mut c, "COLUMN", vec![]), Value::Number(3.0));
        let r = c.range(0, "E2:G9");
        assert_eq!(call(&mut c, "ROW", vec![r.clone()]), Value::Number(2.0));
        assert_eq!(call(&mut c, "COLUMN", vec![r]), Value::Number(5.0));
    }

    #[test]
    fn non_references() {
        let mut c = MockEvalContext::new();
        assert_eq!(
            call(&mut c, "ROW", vec![EvalValue::number(1.0)]),
            Value::Error(ErrorKind::Value)
        );
        assert_eq!(
            call(&mut c, "COLUMN", vec![EvalValue::error(ErrorKind::NA)]),
            Value::Error(ErrorKind::NA)
        );
    }
}
