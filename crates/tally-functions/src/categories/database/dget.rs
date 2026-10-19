//! DGET.

use tally_types::{ErrorKind, Value};

use super::common::{select, DB_PARAMS};
use crate::registry::{Category, FunctionDef, Registry};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

/// `DGET(database; field; criteria)`.
pub const DGET: FunctionDef = FunctionDef::new("DGET", Category::Database, DB_PARAMS, dget)
    .help("Returns the content of the cell of the one record that matches the criteria.");

/// No matching record is `#VALUE!`; more than one is `Err:502` (AOO
/// `ScInterpreter::ScDBGet`). A blank result cell yields 0.
fn dget(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let sel = select(&mut a, false)?;
        match sel.matched {
            0 => Err(ErrorKind::Value),
            1 => match sel.values.into_iter().next() {
                Some(Value::Empty) | None => Ok(Value::Number(0.0)),
                Some(Value::Error(e)) => Err(e),
                Some(v) => Ok(v),
            },
            _ => Err(ErrorKind::InvalidArgument),
        }
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(DGET);
}

#[cfg(test)]
mod tests {
    use super::super::common::sample;
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{ErrorKind, Value};

    fn get(field: EvalValue, crit: &str) -> Value {
        let mut c = MockEvalContext::new();
        sample(&mut c);
        c.set(0, "H1", "Name").set(0, "H2", "Carl");
        c.set(0, "J1", "Age").set(0, "J2", 14.0);
        c.set(0, "L1", "Name").set(0, "L2", "Zzz");
        let db = c.range(0, "A1:E11");
        let cr = c.range(0, crit);
        call(&mut c, "DGET", vec![db, field, cr])
    }

    #[test]
    fn dget_cases() {
        assert_eq!(get(EvalValue::text("Weight"), "H1:H2"), Value::Number(80.0));
        assert_eq!(get(EvalValue::number(1.0), "H1:H2"), Value::text("Carl"));
        assert_eq!(
            get(EvalValue::text("Weight"), "J1:J2"),
            Value::Error(ErrorKind::InvalidArgument)
        );
        assert_eq!(
            get(EvalValue::text("Weight"), "L1:L2"),
            Value::Error(ErrorKind::Value)
        );
        assert_eq!(
            get(EvalValue::number(0.0), "H1:H2"),
            Value::Error(ErrorKind::InvalidArgument)
        );
    }
}
