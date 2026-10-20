//! MULTIPLE.OPERATIONS (AOO `ScTableOp`, Data ▸ Multiple Operations).
//!
//! The function validates its arguments and delegates the what-if
//! evaluation to [`EvalContext::table_op`]: the engine re-evaluates the
//! formula cell with the input cells temporarily replaced (a generalized
//! goal-seek sandbox). The default implementation returns `None`, which is
//! reported as `Err:504`, the same error AOO gives for an unusable
//! argument list.

use tally_types::{CellAddr, ErrorKind, Value};

use crate::coerce::scalar_value;
use crate::registry::{Category, FnFlags, FunctionDef, Param, ParamKind, Registry, ReturnKind};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const PARAMS: &[Param] = &[
    Param::required("formula", ParamKind::Lazy),
    Param::required("column_cell", ParamKind::Lazy),
    Param::required("column_replacement", ParamKind::Lazy),
    Param::optional("row_cell", ParamKind::Lazy),
    Param::optional("row_replacement", ParamKind::Lazy),
];

/// `MULTIPLE.OPERATIONS(formula; column_cell; column_replacement
/// [; row_cell; row_replacement])`.
pub const MULTIPLE_OPERATIONS: FunctionDef = FunctionDef::new(
    "MULTIPLE.OPERATIONS",
    Category::Spreadsheet,
    PARAMS,
    multiple_operations,
)
.returns(ReturnKind::Any)
.flags(FnFlags::LAZY_ARGS)
.help("What-if table: the formula cell recalculated with input cells replaced.");

/// AOO's `ScTableOp` (`sc/source/core/tool/interpr4.cxx`) accepts exactly
/// 3 or 5 arguments (one or two input cells); 4 is `Err:504`. `formula`
/// and the input cells must be single-cell references (`PopSingleRef`:
/// anything else is `Err:504`). A replacement is the value of a
/// single-cell reference in AOO; a constant or an expression is accepted
/// here as well, since the dialog writes references and a literal cannot
/// mean anything else. Errors in the arguments propagate. The hook
/// returning `None` (no sandbox, or the formula cell is not a formula) is
/// `Err:504`.
fn multiple_operations(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let n = a.count();
        if n != 3 && n != 5 {
            return Err(ErrorKind::ParamList);
        }
        let formula = single_ref(&mut a, 0)?;
        let mut subs: Vec<(CellAddr, Value)> = Vec::with_capacity(2);
        for i in (1..n).step_by(2) {
            let cell = single_ref(&mut a, i)?;
            let value = replacement(&mut a, i + 1)?;
            subs.push((cell, value));
        }
        match a.ctx().table_op(formula, &subs) {
            Some(Value::Error(e)) => Err(e),
            Some(v) => Ok(v),
            None => Err(ErrorKind::ParamList),
        }
    })
}

/// Argument `i` as a single-cell reference (`Err:504` otherwise).
fn single_ref(a: &mut Args<'_>, i: usize) -> Result<CellAddr, ErrorKind> {
    match a.any(i) {
        EvalValue::Range(r) if r.is_single() => Ok(r.start()),
        EvalValue::Scalar(Value::Error(e)) => Err(e),
        _ => Err(ErrorKind::ParamList),
    }
}

/// Argument `i` as a replacement value: a single cell's value, or a scalar.
fn replacement(a: &mut Args<'_>, i: usize) -> Result<Value, ErrorKind> {
    match a.any(i) {
        EvalValue::Range(r) if r.is_single() => Ok(a.ctx().cell_value(r.start())),
        EvalValue::Range(_) => Err(ErrorKind::ParamList),
        EvalValue::Scalar(Value::Error(e)) => Err(e),
        other => Ok(scalar_value(&other, a.ctx())),
    }
}

pub(super) fn register(r: &mut Registry) {
    r.register(MULTIPLE_OPERATIONS);
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{CellAddr, ErrorKind, Value};

    fn n(x: f64) -> EvalValue {
        EvalValue::number(x)
    }

    /// `A1 = B1 * C1` evaluated by hand: the hook multiplies the
    /// substituted (or stored) values of B1 and C1.
    fn table_ctx() -> MockEvalContext {
        let mut c = MockEvalContext::new();
        c.set(0, "A1", 6.0).set(0, "B1", 2.0).set(0, "C1", 3.0);
        c.set(0, "B2", 5.0).set(0, "C2", 7.0);
        let b1 = CellAddr::new(0, 0, 1);
        let c1 = CellAddr::new(0, 0, 2);
        c.set_table_op(move |formula, subs| {
            if formula != CellAddr::new(0, 0, 0) {
                return None;
            }
            let pick = |cell: CellAddr, stored: f64| {
                subs.iter()
                    .find(|(c, _)| *c == cell)
                    .map(|(_, v)| match v {
                        Value::Number(x) => *x,
                        _ => f64::NAN,
                    })
                    .unwrap_or(stored)
            };
            Some(Value::Number(pick(b1, 2.0) * pick(c1, 3.0)))
        });
        c
    }

    #[test]
    fn one_and_two_variables() {
        let mut c = table_ctx();
        let a1 = c.range(0, "A1");
        let b1 = c.range(0, "B1");
        let c1 = c.range(0, "C1");
        let b2 = c.range(0, "B2");
        let c2 = c.range(0, "C2");
        // Replacement from a cell (as the dialog writes it).
        assert_eq!(
            call(
                &mut c,
                "MULTIPLE.OPERATIONS",
                vec![a1.clone(), b1.clone(), b2]
            ),
            Value::Number(15.0)
        );
        // Replacement as a constant.
        assert_eq!(
            call(
                &mut c,
                "MULTIPLE.OPERATIONS",
                vec![a1.clone(), b1.clone(), n(10.0)]
            ),
            Value::Number(30.0)
        );
        // Two variables.
        assert_eq!(
            call(
                &mut c,
                "MULTIPLE.OPERATIONS",
                vec![a1.clone(), b1.clone(), n(4.0), c1.clone(), c2]
            ),
            Value::Number(28.0)
        );
        assert_eq!(
            c.table_op_calls.last().cloned(),
            Some((
                CellAddr::new(0, 0, 0),
                vec![
                    (CellAddr::new(0, 0, 1), Value::Number(4.0)),
                    (CellAddr::new(0, 0, 2), Value::Number(7.0))
                ]
            ))
        );
        // Not a formula the hook knows: Err:504.
        let d9 = c.range(0, "D9");
        assert_eq!(
            call(&mut c, "MULTIPLE.OPERATIONS", vec![d9, b1, n(1.0)]),
            Value::Error(ErrorKind::ParamList)
        );
        let _ = a1;
        let _ = c1;
    }

    #[test]
    fn argument_errors() {
        let mut c = table_ctx();
        let a1 = c.range(0, "A1");
        let b1 = c.range(0, "B1");
        let c1 = c.range(0, "C1");
        let a1a2 = c.range(0, "A1:A2");
        let b1b2 = c.range(0, "B1:B2");
        let bad = Value::Error(ErrorKind::ParamList);
        // Four arguments, multi-cell references, constants where a cell is needed.
        assert_eq!(
            call(
                &mut c,
                "MULTIPLE.OPERATIONS",
                vec![a1.clone(), b1.clone(), n(1.0), c1]
            ),
            bad
        );
        assert_eq!(
            call(
                &mut c,
                "MULTIPLE.OPERATIONS",
                vec![a1a2, b1.clone(), n(1.0)]
            ),
            bad
        );
        assert_eq!(
            call(&mut c, "MULTIPLE.OPERATIONS", vec![n(1.0), n(2.0), n(3.0)]),
            bad
        );
        assert_eq!(
            call(
                &mut c,
                "MULTIPLE.OPERATIONS",
                vec![a1.clone(), b1.clone(), b1b2]
            ),
            bad
        );
        // Errors propagate.
        assert_eq!(
            call(
                &mut c,
                "MULTIPLE.OPERATIONS",
                vec![a1.clone(), b1.clone(), EvalValue::error(ErrorKind::Div0)]
            ),
            Value::Error(ErrorKind::Div0)
        );
        // Arity.
        assert_eq!(
            call(&mut c, "MULTIPLE.OPERATIONS", vec![a1.clone(), b1.clone()]),
            Value::Error(ErrorKind::MissingVariable2)
        );
        assert_eq!(
            call(
                &mut c,
                "MULTIPLE.OPERATIONS",
                vec![a1, b1, n(1.0), n(1.0), n(1.0), n(1.0)]
            ),
            bad
        );
    }

    #[test]
    fn no_hook_is_err504() {
        let mut c = MockEvalContext::new();
        c.set(0, "B1", 1.0);
        let a1 = c.range(0, "A1");
        let b1 = c.range(0, "B1");
        assert_eq!(
            call(&mut c, "MULTIPLE.OPERATIONS", vec![a1, b1, n(5.0)]),
            Value::Error(ErrorKind::ParamList)
        );
    }
}
