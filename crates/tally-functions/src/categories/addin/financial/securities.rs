//! Discounted and interest-at-maturity securities and treasury bills:
//! ACCRINT, ACCRINTM, DISC, INTRATE, RECEIVED, PRICEDISC, PRICEMAT,
//! YIELDDISC, YIELDMAT, TBILLEQ, TBILLPRICE, TBILLYIELD.
//!
//! Ported from Apache OpenOffice scaddins/analysis (analysishelper.cxx,
//! financial.cxx), Apache License 2.0.

use tally_types::{ErrorKind, Value};

use super::daycount::{check_freq, diff_date_360, year_diff, year_frac, IAE};
use super::{any_num, basis, finite, long, null_date};
use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const ACCRINT_PARAMS: &[Param] = &[
    Param::required("issue", ParamKind::Integer),
    Param::required("first_interest", ParamKind::Integer),
    Param::required("settlement", ParamKind::Integer),
    Param::required("rate", ParamKind::Number),
    Param::optional("par", ParamKind::Number),
    Param::required("frequency", ParamKind::Integer),
    Param::optional("basis", ParamKind::Integer),
];

/// `ACCRINT(issue; first_interest; settlement; rate; [par]; frequency; [basis])`.
/// As in AOO, `first_interest` is ignored.
pub const ACCRINT: FunctionDef =
    FunctionDef::new("ACCRINT", Category::AddIn, ACCRINT_PARAMS, accrint)
        .help("Returns the accrued interest for a security that pays periodic interest");

fn accrint(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let nd = null_date(&mut a);
        let issue = long(&mut a, 0)?;
        long(&mut a, 1)?;
        let settle = long(&mut a, 2)?;
        let rate = a.num(3)?;
        let par = any_num(&mut a, 4, 1000.0)?;
        let freq = long(&mut a, 5)?;
        if rate <= 0.0 || par <= 0.0 || check_freq(freq).is_err() || issue >= settle {
            return Err(IAE);
        }
        let b = basis(&mut a, 6)?;
        finite(par * rate * year_diff(nd, issue, settle, b)?)
    })
}

const ACCRINTM_PARAMS: &[Param] = &[
    Param::required("issue", ParamKind::Integer),
    Param::required("settlement", ParamKind::Integer),
    Param::required("rate", ParamKind::Number),
    Param::optional("par", ParamKind::Number),
    Param::optional("basis", ParamKind::Integer),
];

/// `ACCRINTM(issue; settlement; rate; [par]; [basis])`.
pub const ACCRINTM: FunctionDef =
    FunctionDef::new("ACCRINTM", Category::AddIn, ACCRINTM_PARAMS, accrintm)
        .help("Returns the accrued interest for a security that pays interest at maturity");

fn accrintm(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let nd = null_date(&mut a);
        let issue = long(&mut a, 0)?;
        let settle = long(&mut a, 1)?;
        let rate = a.num(2)?;
        let par = any_num(&mut a, 3, 1000.0)?;
        if rate <= 0.0 || par <= 0.0 || issue >= settle {
            return Err(IAE);
        }
        let b = basis(&mut a, 4)?;
        finite(par * rate * year_diff(nd, issue, settle, b)?)
    })
}

/// `settlement; maturity; x; y; [basis]` parameters (DISC, INTRATE, …).
const SM2_PARAMS: &[Param] = &[
    Param::required("settlement", ParamKind::Integer),
    Param::required("maturity", ParamKind::Integer),
    Param::required("x", ParamKind::Number),
    Param::required("y", ParamKind::Number),
    Param::optional("basis", ParamKind::Integer),
];

/// Reads `(null_date, settlement, maturity, x, y)`.
fn sm2(a: &mut Args<'_>) -> Result<(i64, i64, i64, f64, f64), ErrorKind> {
    let nd = null_date(a);
    let s = long(a, 0)?;
    let m = long(a, 1)?;
    let x = a.num(2)?;
    let y = a.num(3)?;
    Ok((nd, s, m, x, y))
}

/// `DISC(settlement; maturity; price; redemption; [basis])`.
pub const DISC: FunctionDef = FunctionDef::new("DISC", Category::AddIn, SM2_PARAMS, disc)
    .help("Returns the discount rate for a security");

fn disc(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let (nd, s, m, price, red) = sm2(&mut a)?;
        if price <= 0.0 || red <= 0.0 || s >= m {
            return Err(IAE);
        }
        let b = basis(&mut a, 4)?;
        finite((1.0 - price / red) / year_frac(nd, s, m, b)?)
    })
}

/// `INTRATE(settlement; maturity; investment; redemption; [basis])`.
pub const INTRATE: FunctionDef = FunctionDef::new("INTRATE", Category::AddIn, SM2_PARAMS, intrate)
    .help("Returns the interest rate for a fully invested security");

fn intrate(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let (nd, s, m, invest, red) = sm2(&mut a)?;
        if invest <= 0.0 || red <= 0.0 || s >= m {
            return Err(IAE);
        }
        let b = basis(&mut a, 4)?;
        finite(((red / invest) - 1.0) / year_diff(nd, s, m, b)?)
    })
}

/// `RECEIVED(settlement; maturity; investment; discount; [basis])`.
pub const RECEIVED: FunctionDef =
    FunctionDef::new("RECEIVED", Category::AddIn, SM2_PARAMS, received)
        .help("Returns the amount paid out at maturity for a fully invested security");

fn received(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let (nd, s, m, invest, discount) = sm2(&mut a)?;
        if invest <= 0.0 || discount <= 0.0 {
            return Err(IAE);
        }
        let b = basis(&mut a, 4)?;
        finite(invest / (1.0 - (discount * year_diff(nd, s, m, b)?)))
    })
}

/// `PRICEDISC(settlement; maturity; discount; redemption; [basis])`.
pub const PRICEDISC: FunctionDef =
    FunctionDef::new("PRICEDISC", Category::AddIn, SM2_PARAMS, pricedisc)
        .help("Returns the price per 100 currency units face value of a discounted security");

fn pricedisc(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let (nd, s, m, discount, red) = sm2(&mut a)?;
        if discount <= 0.0 || red <= 0.0 || s >= m {
            return Err(IAE);
        }
        let b = basis(&mut a, 4)?;
        finite(red * (1.0 - discount * year_diff(nd, s, m, b)?))
    })
}

/// `YIELDDISC(settlement; maturity; price; redemption; [basis])`.
pub const YIELDDISC: FunctionDef =
    FunctionDef::new("YIELDDISC", Category::AddIn, SM2_PARAMS, yielddisc)
        .help("Returns the annual yield for a discounted security");

fn yielddisc(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let (nd, s, m, price, red) = sm2(&mut a)?;
        if price <= 0.0 || red <= 0.0 || s >= m {
            return Err(IAE);
        }
        let b = basis(&mut a, 4)?;
        finite(((red / price) - 1.0) / year_frac(nd, s, m, b)?)
    })
}

const MAT_PARAMS: &[Param] = &[
    Param::required("settlement", ParamKind::Integer),
    Param::required("maturity", ParamKind::Integer),
    Param::required("issue", ParamKind::Integer),
    Param::required("rate", ParamKind::Number),
    Param::required("y", ParamKind::Number),
    Param::optional("basis", ParamKind::Integer),
];

/// PRICEMAT (`price_mode`, `y` is the yield) and YIELDMAT (`y` is the price).
fn mat_common(a: &mut Args<'_>, price_mode: bool) -> Result<Value, ErrorKind> {
    let nd = null_date(a);
    let s = long(a, 0)?;
    let m = long(a, 1)?;
    let issue = long(a, 2)?;
    let rate = a.num(3)?;
    let y = a.num(4)?;
    if price_mode {
        if rate < 0.0 || y < 0.0 || s >= m {
            return Err(IAE);
        }
    } else if rate <= 0.0 || s >= m {
        return Err(IAE);
    }
    let b = basis(a, 5)?;
    let iss_mat = year_frac(nd, issue, m, b)?;
    let iss_set = year_frac(nd, issue, s, b)?;
    let set_mat = year_frac(nd, s, m, b)?;
    if price_mode {
        // GetPricemat
        let mut p = 1.0 + iss_mat * rate;
        p /= 1.0 + set_mat * y;
        p -= iss_set * rate;
        finite(p * 100.0)
    } else {
        // GetYieldmat: y is the price.
        let mut r = 1.0 + iss_mat * rate;
        r /= y / 100.0 + iss_set * rate;
        r -= 1.0;
        finite(r / set_mat)
    }
}

/// `PRICEMAT(settlement; maturity; issue; rate; yield; [basis])`.
pub const PRICEMAT: FunctionDef = FunctionDef::new("PRICEMAT", Category::AddIn, MAT_PARAMS, pricemat)
    .help("Returns the price per 100 currency units face value of a security that pays interest at maturity");

fn pricemat(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| mat_common(&mut a, true))
}

/// `YIELDMAT(settlement; maturity; issue; rate; price; [basis])`.
pub const YIELDMAT: FunctionDef =
    FunctionDef::new("YIELDMAT", Category::AddIn, MAT_PARAMS, yieldmat)
        .help("Returns the annual yield of a security that pays interest at maturity");

fn yieldmat(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| mat_common(&mut a, false))
}

const TBILL_PARAMS: &[Param] = &[
    Param::required("settlement", ParamKind::Integer),
    Param::required("maturity", ParamKind::Integer),
    Param::required("x", ParamKind::Number),
];

fn tbill_args(a: &mut Args<'_>) -> Result<(i64, i64, i64, f64), ErrorKind> {
    let nd = null_date(a);
    let s = long(a, 0)?;
    let m = long(a, 1)?;
    let x = a.num(2)?;
    Ok((nd, s, m, x))
}

/// `TBILLEQ(settlement; maturity; discount)`.
pub const TBILLEQ: FunctionDef =
    FunctionDef::new("TBILLEQ", Category::AddIn, TBILL_PARAMS, tbilleq)
        .help("Returns the bond-equivalent yield for a treasury bill");

fn tbilleq(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let (nd, s, m, discount) = tbill_args(&mut a)?;
        let m = m + 1;
        let diff = diff_date_360(nd, s, m, true)?;
        if discount <= 0.0 || s >= m || diff > 360 {
            return Err(IAE);
        }
        finite((365.0 * discount) / (360.0 - (discount * diff as f64)))
    })
}

/// `TBILLPRICE(settlement; maturity; discount)`.
pub const TBILLPRICE: FunctionDef =
    FunctionDef::new("TBILLPRICE", Category::AddIn, TBILL_PARAMS, tbillprice)
        .help("Returns the price of 100 currency units face value for a treasury bill");

fn tbillprice(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let (nd, s, m, discount) = tbill_args(&mut a)?;
        if discount <= 0.0 || s > m {
            return Err(IAE);
        }
        let frac = year_frac(nd, s, m + 1, 0)?;
        if frac.fract() == 0.0 {
            return Err(IAE);
        }
        finite(100.0 * (1.0 - discount * frac))
    })
}

/// `TBILLYIELD(settlement; maturity; price)`.
pub const TBILLYIELD: FunctionDef =
    FunctionDef::new("TBILLYIELD", Category::AddIn, TBILL_PARAMS, tbillyield)
        .help("Returns the yield for a treasury bill");

fn tbillyield(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let (nd, s, m, price) = tbill_args(&mut a)?;
        let diff = diff_date_360(nd, s, m, true)? + 1;
        if price <= 0.0 || s >= m || diff > 360 {
            return Err(IAE);
        }
        finite((100.0 / price - 1.0) / diff as f64 * 360.0)
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(ACCRINT);
    r.register(ACCRINTM);
    r.register(DISC);
    r.register(INTRATE);
    r.register(RECEIVED);
    r.register(PRICEDISC);
    r.register(PRICEMAT);
    r.register(YIELDDISC);
    r.register(YIELDMAT);
    r.register(TBILLEQ);
    r.register(TBILLPRICE);
    r.register(TBILLYIELD);
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{ErrorKind, Value};

    fn num(c: &mut MockEvalContext, f: &str, xs: &[f64]) -> f64 {
        match call(c, f, xs.iter().map(|&x| EvalValue::number(x)).collect()) {
            Value::Number(v) => v,
            other => panic!("{f}: {other:?}"),
        }
    }

    fn close(a: f64, b: f64, tol: f64) {
        assert!((a - b).abs() <= tol, "{a} vs {b}");
    }

    // Serials (1899-12-30 epoch).
    const D2008_02_15: f64 = 39493.0;
    const D2008_05_15: f64 = 39583.0;
    const D2008_03_31: f64 = 39538.0;
    const D2008_06_01: f64 = 39600.0;

    #[test]
    fn discounted() {
        let mut c = MockEvalContext::new();
        let (s, m) = (D2008_02_15, D2008_05_15);
        close(
            num(&mut c, "INTRATE", &[s, m, 1e6, 1014420.0, 2.0]),
            0.05768,
            1e-12,
        );
        close(
            num(&mut c, "RECEIVED", &[s, m, 1e6, 0.0575, 2.0]),
            1014584.6544,
            1e-4,
        );
        close(
            num(&mut c, "DISC", &[39107.0, 39248.0, 97.975, 100.0, 1.0]),
            0.0524202,
            1e-7,
        );
        close(
            num(&mut c, "YIELDDISC", &[39494.0, 39508.0, 99.795, 100.0, 2.0]),
            0.052823,
            1e-6,
        );
        close(
            num(&mut c, "PRICEDISC", &[39494.0, 39508.0, 0.0525, 100.0, 2.0]),
            99.79583,
            1e-5,
        );
        assert_eq!(
            call(
                &mut c,
                "DISC",
                [m, s, 97.0, 100.0].map(EvalValue::number).to_vec()
            ),
            Value::Error(ErrorKind::InvalidArgument)
        );
    }

    #[test]
    fn accrued_and_maturity() {
        let mut c = MockEvalContext::new();
        close(
            num(
                &mut c,
                "ACCRINT",
                &[39508.0, 39691.0, 39569.0, 0.1, 1000.0, 2.0, 0.0],
            ),
            16.6666666667,
            1e-9,
        );
        close(
            num(&mut c, "ACCRINTM", &[39539.0, 39614.0, 0.1, 1000.0, 3.0]),
            20.5479452,
            1e-6,
        );
        close(
            num(
                &mut c,
                "PRICEMAT",
                &[39493.0, 39551.0, 39397.0, 0.061, 0.061, 0.0],
            ),
            99.98449888,
            1e-6,
        );
        close(
            num(
                &mut c,
                "YIELDMAT",
                &[39522.0, 39755.0, 39394.0, 0.0625, 100.0123, 0.0],
            ),
            0.060954,
            1e-6,
        );
    }

    #[test]
    fn tbills() {
        let mut c = MockEvalContext::new();
        let (s, m) = (D2008_03_31, D2008_06_01);
        close(num(&mut c, "TBILLEQ", &[s, m, 0.0914]), 0.0941514, 1e-7);
        close(num(&mut c, "TBILLPRICE", &[s, m, 0.09]), 98.45, 1e-9);
        close(num(&mut c, "TBILLYIELD", &[s, m, 98.45]), 0.0914170, 1e-7);
        assert_eq!(
            call(
                &mut c,
                "TBILLYIELD",
                [s, s + 400.0, 98.0].map(EvalValue::number).to_vec()
            ),
            Value::Error(ErrorKind::InvalidArgument)
        );
    }
}
