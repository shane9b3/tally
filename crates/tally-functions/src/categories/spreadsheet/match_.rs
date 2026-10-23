//! MATCH (AOO `ScInterpreter::ScMatch`).

use tally_types::{ErrorKind, Value};

use super::lookup::{make_key, Mode, Table};
use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry, ReturnKind};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const PARAMS: &[Param] = &[
    Param::required("search_criterion", ParamKind::Any),
    Param::required("lookup_array", ParamKind::Reference),
    Param::optional("type", ParamKind::Number),
];

/// `MATCH(search_criterion; lookup_array; [type])`.
pub const MATCH: FunctionDef = FunctionDef::new("MATCH", Category::Spreadsheet, PARAMS, match_fn)
    .returns(ReturnKind::Number)
    .help("Defines a position in an array after comparing values.");

/// Type 1 (default): largest value `<=` the key in ascending data; 0:
/// exact (wildcards per settings); -1: smallest value `>=` in descending
/// data. Only one-dimensional ranges and arrays are searched (Err:504
/// otherwise, AOO's `PushIllegalParameter`); not found is `#N/A`.
fn match_fn(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let key = make_key(a.value(0))?;
        let table = Table::from_eval(a.any(1))?;
        let kind = a.num_opt(2, 1.0)?;
        if table.rows() != 1 && table.cols() != 1 {
            return Err(ErrorKind::ParamList);
        }
        let mode = if kind > 0.0 {
            Mode::Ascending
        } else if kind < 0.0 {
            Mode::Descending
        } else {
            Mode::Exact
        };
        let line = table.search_line(a.ctx(), table.cols() == 1, 0);
        let settings = a.ctx().settings().clone();
        let pos = line.search(&key, mode, &settings).ok_or(ErrorKind::NA)?;
        Ok(Value::Number(f64::from(pos) + 1.0))
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(MATCH);
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{ErrorKind, Value};

    fn n(x: f64) -> EvalValue {
        EvalValue::number(x)
    }

    fn sheet() -> MockEvalContext {
        let mut c = MockEvalContext::new();
        for (i, v) in [1.0, 3.0, 5.0, 7.0].iter().enumerate() {
            c.set(0, &format!("A{}", i + 1), *v);
        }
        for (i, v) in ["apple", "Banana", "cherry"].iter().enumerate() {
            c.set(0, &format!("C{}", i + 1), *v);
        }
        for (i, v) in [9.0, 6.0, 2.0].iter().enumerate() {
            c.set(0, &format!("{}5", (b'E' + i as u8) as char), *v);
        }
        c
    }

    #[test]
    fn match_types() {
        let mut c = sheet();
        let r = c.range(0, "A1:A4");
        let m = |c: &mut MockEvalContext, k: f64, t: Option<f64>| {
            let mut v = vec![n(k), r.clone()];
            v.extend(t.map(n));
            call(c, "MATCH", v)
        };
        assert_eq!(m(&mut c, 5.0, None), Value::Number(3.0));
        assert_eq!(m(&mut c, 6.0, None), Value::Number(3.0));
        assert_eq!(m(&mut c, 0.0, None), Value::Error(ErrorKind::NA));
        assert_eq!(m(&mut c, 6.0, Some(0.0)), Value::Error(ErrorKind::NA));
        assert_eq!(m(&mut c, 7.0, Some(0.0)), Value::Number(4.0));
    }

    #[test]
    fn match_descending_and_text() {
        let mut c = sheet();
        let r = c.range(0, "E5:G5");
        assert_eq!(
            call(&mut c, "MATCH", vec![n(5.0), r, n(-1.0)]),
            Value::Number(2.0)
        );
        let r = c.range(0, "C1:C3");
        assert_eq!(
            call(
                &mut c,
                "MATCH",
                vec![EvalValue::text("BANANA"), r.clone(), n(0.0)]
            ),
            Value::Number(2.0)
        );
        assert_eq!(
            call(
                &mut c,
                "MATCH",
                vec![EvalValue::text("ch*"), r.clone(), n(0.0)]
            ),
            Value::Number(3.0)
        );
        assert_eq!(
            call(&mut c, "MATCH", vec![n(1.0), r, n(0.0)]),
            Value::Error(ErrorKind::NA)
        );
    }

    #[test]
    fn match_shape_errors() {
        let mut c = sheet();
        let r = c.range(0, "A1:C3");
        assert_eq!(
            call(&mut c, "MATCH", vec![n(1.0), r, n(0.0)]),
            Value::Error(ErrorKind::ParamList)
        );
        let r = c.range(0, "A1:A4");
        assert_eq!(
            call(
                &mut c,
                "MATCH",
                vec![EvalValue::error(ErrorKind::Num), r, n(0.0)]
            ),
            Value::Error(ErrorKind::Num)
        );
    }
}
