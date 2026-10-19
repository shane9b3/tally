//! NA.

use tally_types::ErrorKind;

use crate::registry::{Category, FunctionDef, Registry};
use crate::{ArgList, EvalContext, EvalValue};

/// `NA()`.
pub const NA: FunctionDef = FunctionDef::new("NA", Category::Information, &[], na)
    .help("Not available. Returns the error value #N/A.");

fn na(_ctx: &mut dyn EvalContext, _args: &mut dyn ArgList) -> EvalValue {
    EvalValue::error(ErrorKind::NA)
}

pub(super) fn register(r: &mut Registry) {
    r.register(NA);
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{ErrorKind, Value};

    #[test]
    fn na() {
        let mut c = MockEvalContext::new();
        assert_eq!(call(&mut c, "NA", vec![]), Value::Error(ErrorKind::NA));
        assert_eq!(
            call(&mut c, "NA", vec![EvalValue::number(1.0)]),
            Value::Error(ErrorKind::ParamList)
        );
    }
}
