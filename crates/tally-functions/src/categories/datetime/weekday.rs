//! WEEKDAY.

use tally_numfmt::date::{iso_weekday, weekday};
use tally_types::{ErrorKind, Value};

use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry, ReturnKind};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const PARAMS: &[Param] = &[
    Param::required("number", ParamKind::Number),
    Param::optional("type", ParamKind::Integer),
];

/// `WEEKDAY(number [; type])`: type 1 (default) Sunday = 1 .. Saturday = 7,
/// type 2 Monday = 1 .. Sunday = 7, type 3 Monday = 0 .. Sunday = 6. AOO 4.1
/// supports only these three; other types are `Err:502`.
pub const WEEKDAY: FunctionDef = FunctionDef::new("WEEKDAY", Category::DateTime, PARAMS, wd)
    .returns(ReturnKind::Number)
    .help("Day of the week of a date serial.");

fn wd(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let serial = a.num(0)?;
        let ty = a.int_opt(1, 1)?;
        if !(1..=3).contains(&ty) {
            return Err(ErrorKind::InvalidArgument);
        }
        let epoch = a.ctx().settings().date_epoch;
        let r = match ty {
            1 => weekday(epoch, serial),
            2 => iso_weekday(epoch, serial),
            _ => iso_weekday(epoch, serial).map(|d| d - 1),
        };
        r.map(|d| Value::Number(f64::from(d)))
            .ok_or(ErrorKind::InvalidArgument)
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(WEEKDAY);
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{ErrorKind, Value};

    fn w(serial: f64, ty: Option<f64>) -> Value {
        let mut args = vec![EvalValue::number(serial)];
        args.extend(ty.map(EvalValue::number));
        call(&mut MockEvalContext::new(), "WEEKDAY", args)
    }

    // 46314 = 2026-10-19, a Monday; 46313 = Sunday; 46319 = Saturday.
    #[test]
    fn types() {
        assert_eq!(w(46314.0, None), Value::Number(2.0));
        assert_eq!(w(46313.0, None), Value::Number(1.0));
        assert_eq!(w(46319.0, Some(1.0)), Value::Number(7.0));
        assert_eq!(w(46314.0, Some(2.0)), Value::Number(1.0));
        assert_eq!(w(46313.0, Some(2.0)), Value::Number(7.0));
        assert_eq!(w(46314.0, Some(3.0)), Value::Number(0.0));
        assert_eq!(w(46313.0, Some(3.0)), Value::Number(6.0));
        assert_eq!(w(46314.9, None), Value::Number(2.0));
    }

    #[test]
    fn errors() {
        let bad = Value::Error(ErrorKind::InvalidArgument);
        assert_eq!(w(46314.0, Some(0.0)), bad);
        assert_eq!(w(46314.0, Some(11.0)), bad);
        assert_eq!(w(46314.0, Some(4.0)), bad);
        assert_eq!(w(1e12, None), bad);
        assert_eq!(
            call(
                &mut MockEvalContext::new(),
                "WEEKDAY",
                vec![EvalValue::text("x")]
            ),
            Value::Error(ErrorKind::Value)
        );
    }
}
