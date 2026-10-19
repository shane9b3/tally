//! COUPDAYBS, COUPDAYS, COUPDAYSNC, COUPNCD, COUPNUM, COUPPCD.
//!
//! Ported from Apache OpenOffice scaddins/analysis (analysishelper.cxx,
//! financial.cxx), Apache License 2.0.

use tally_types::ErrorKind;

use super::daycount::Coupon;
use super::{basis, finite, long, null_date};
use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry, ReturnKind};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const COUP_PARAMS: &[Param] = &[
    Param::required("settlement", ParamKind::Integer),
    Param::required("maturity", ParamKind::Integer),
    Param::required("frequency", ParamKind::Integer),
    Param::optional("basis", ParamKind::Integer),
];

fn coupon_args(a: &mut Args<'_>) -> Result<Coupon, ErrorKind> {
    let nd = null_date(a);
    let settle = long(a, 0)?;
    let mat = long(a, 1)?;
    let freq = long(a, 2)?;
    let base = basis(a, 3)?;
    Ok(Coupon {
        nd,
        settle,
        mat,
        freq,
        base,
    })
}

fn eval_coupon(
    ctx: &mut dyn EvalContext,
    args: &mut dyn ArgList,
    f: fn(&Coupon) -> Result<f64, ErrorKind>,
) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| finite(f(&coupon_args(&mut a)?)?))
}

fn coupdaybs(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    eval_coupon(ctx, args, Coupon::daybs)
}
fn coupdays(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    eval_coupon(ctx, args, Coupon::days)
}
fn coupdaysnc(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    eval_coupon(ctx, args, Coupon::daysnc)
}
fn coupncd(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    eval_coupon(ctx, args, Coupon::ncd)
}
fn coupnum(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    eval_coupon(ctx, args, Coupon::num)
}
fn couppcd(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    eval_coupon(ctx, args, Coupon::pcd)
}

/// `COUPDAYBS(settlement; maturity; frequency; [basis])`.
pub const COUPDAYBS: FunctionDef =
    FunctionDef::new("COUPDAYBS", Category::AddIn, COUP_PARAMS, coupdaybs).help(
        "Returns the number of days from the beginning of the coupon period to the settlement date",
    );

/// `COUPDAYS(settlement; maturity; frequency; [basis])`.
pub const COUPDAYS: FunctionDef =
    FunctionDef::new("COUPDAYS", Category::AddIn, COUP_PARAMS, coupdays)
        .help("Returns the number of days in the coupon period containing the settlement date");

/// `COUPDAYSNC(settlement; maturity; frequency; [basis])`.
pub const COUPDAYSNC: FunctionDef =
    FunctionDef::new("COUPDAYSNC", Category::AddIn, COUP_PARAMS, coupdaysnc)
        .help("Returns the number of days from the settlement date to the next coupon date");

/// `COUPNCD(settlement; maturity; frequency; [basis])`: a date serial.
pub const COUPNCD: FunctionDef = FunctionDef::new("COUPNCD", Category::AddIn, COUP_PARAMS, coupncd)
    .help("Returns the first coupon date after the settlement date");

/// `COUPNUM(settlement; maturity; frequency; [basis])`.
pub const COUPNUM: FunctionDef = FunctionDef::new("COUPNUM", Category::AddIn, COUP_PARAMS, coupnum)
    .help("Returns the number of coupons payable between the settlement and maturity dates");

/// `COUPPCD(settlement; maturity; frequency; [basis])`: a date serial.
pub const COUPPCD: FunctionDef = FunctionDef::new("COUPPCD", Category::AddIn, COUP_PARAMS, couppcd)
    .help("Returns the last coupon date preceding the settlement date");

pub(super) fn register(r: &mut Registry) {
    r.register(COUPDAYBS);
    r.register(COUPDAYS);
    r.register(COUPDAYSNC);
    r.register(COUPNCD.returns(ReturnKind::Number));
    r.register(COUPNUM);
    r.register(COUPPCD);
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{ErrorKind, Value};

    fn args(xs: &[f64]) -> Vec<EvalValue> {
        xs.iter().map(|&x| EvalValue::number(x)).collect()
    }

    #[test]
    fn coupon_dates() {
        let mut c = MockEvalContext::new();
        // 2007-01-25, 2008-11-15
        let base = [39107.0, 39767.0, 2.0, 1.0];
        assert_eq!(call(&mut c, "COUPNUM", args(&base)), Value::Number(4.0));
        assert_eq!(call(&mut c, "COUPDAYS", args(&base)), Value::Number(181.0));
        assert_eq!(call(&mut c, "COUPDAYBS", args(&base)), Value::Number(71.0));
        assert_eq!(
            call(&mut c, "COUPDAYSNC", args(&base)),
            Value::Number(110.0)
        );
        assert_eq!(call(&mut c, "COUPNCD", args(&base)), Value::Number(39217.0));
        assert_eq!(call(&mut c, "COUPPCD", args(&base)), Value::Number(39036.0));
        assert_eq!(
            call(&mut c, "COUPDAYS", args(&[39107.0, 39767.0, 2.0, 0.0])),
            Value::Number(180.0)
        );
        assert_eq!(
            call(&mut c, "COUPNUM", args(&[39767.0, 39107.0, 2.0])),
            Value::Error(ErrorKind::InvalidArgument)
        );
        assert_eq!(
            call(&mut c, "COUPNUM", args(&[39107.0, 39767.0, 3.0])),
            Value::Error(ErrorKind::InvalidArgument)
        );
        assert_eq!(
            call(&mut c, "COUPNUM", args(&[39107.0, 39767.0, 2.0, 5.0])),
            Value::Error(ErrorKind::InvalidArgument)
        );
    }
}
