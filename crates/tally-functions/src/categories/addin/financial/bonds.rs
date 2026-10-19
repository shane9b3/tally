//! Coupon bonds: PRICE, YIELD, DURATION_ADD, MDURATION, ODDFPRICE,
//! ODDFYIELD, ODDLPRICE, ODDLYIELD.
//!
//! Ported from Apache OpenOffice scaddins/analysis (analysishelper.cxx,
//! financial.cxx), Apache License 2.0 (`getPrice_`, `getYield_`,
//! `GetDuration`, `GetOddlprice`, `GetOddlyield`).
//!
//! AOO 4.1 does not implement ODDFPRICE / ODDFYIELD (`GetOddfprice` throws
//! a `RuntimeException`, shown as `#VALUE!`). Tally implements them from
//! the published Excel formula for odd first periods (short and long),
//! with quasi-coupon periods counted under the chosen basis.

use tally_types::{ErrorKind, Value};

use super::daycount::{check_freq, days_in_year, year_frac, Coupon, ScaDate, IAE};
use super::{basis, finite, long, null_date};
use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

/// `getPrice_`.
fn price(c: &Coupon, rate: f64, yld: f64, red: f64) -> Result<f64, ErrorKind> {
    let freq = c.freq as f64;
    let e = c.days()?;
    let dsc_e = c.daysnc()? / e;
    let n = c.num()?;
    let a = c.daybs()?;
    let mut ret = red / (1.0 + yld / freq).powf(n - 1.0 + dsc_e);
    ret -= 100.0 * rate / freq * a / e;
    let t1 = 100.0 * rate / freq;
    let t2 = 1.0 + yld / freq;
    let mut k = 0.0;
    while k < n {
        ret += t1 / t2.powf(k + dsc_e);
        k += 1.0;
    }
    Ok(ret)
}

/// `getYield_`: secant/bisection search, at most 100 steps.
fn yield_(c: &Coupon, rate: f64, pr: f64, red: f64) -> Result<f64, ErrorKind> {
    let mut price_n = 0.0;
    let mut y1 = 0.0;
    let mut y2 = 1.0;
    let mut p1 = price(c, rate, y1, red)?;
    let mut p2 = price(c, rate, y2, red)?;
    let mut yn = (y2 - y1) * 0.5;
    let mut iter = 0;
    while iter < 100 && price_n != pr {
        price_n = price(c, rate, yn, red)?;
        if pr == p1 {
            return Ok(y1);
        } else if pr == p2 {
            return Ok(y2);
        } else if pr == price_n {
            return Ok(yn);
        } else if pr < p2 {
            y2 *= 2.0;
            p2 = price(c, rate, y2, red)?;
            yn = (y2 - y1) * 0.5;
        } else {
            if pr < price_n {
                y1 = yn;
                p1 = price_n;
            } else {
                y2 = yn;
                p2 = price_n;
            }
            yn = y2 - (y2 - y1) * ((pr - p2) / (p1 - p2));
        }
        iter += 1;
    }
    if (pr - price_n).abs() > pr / 100.0 {
        return Err(IAE);
    }
    Ok(yn)
}

/// `GetDuration`: Macaulay duration.
fn duration(c: &Coupon, coup: f64, yld: f64) -> Result<f64, ErrorKind> {
    let yearfrac = year_frac(c.nd, c.settle, c.mat, c.base)?;
    let ncoups = c.num()?;
    let freq = c.freq as f64;
    let coup = coup * 100.0 / freq;
    let y = yld / freq + 1.0;
    let diff = yearfrac * freq - ncoups;
    let mut dur = 0.0;
    let mut t = 1.0;
    while t < ncoups {
        dur += (t + diff) * coup / y.powf(t + diff);
        t += 1.0;
    }
    dur += (ncoups + diff) * (coup + 100.0) / y.powf(ncoups + diff);
    let mut p = 0.0;
    let mut t = 1.0;
    while t < ncoups {
        p += coup / y.powf(t + diff);
        t += 1.0;
    }
    p += (coup + 100.0) / y.powf(ncoups + diff);
    Ok(dur / p / freq)
}

const PRICE_PARAMS: &[Param] = &[
    Param::required("settlement", ParamKind::Integer),
    Param::required("maturity", ParamKind::Integer),
    Param::required("rate", ParamKind::Number),
    Param::required("yield", ParamKind::Number),
    Param::required("redemption", ParamKind::Number),
    Param::required("frequency", ParamKind::Integer),
    Param::optional("basis", ParamKind::Integer),
];

/// Reads `settlement; maturity; x; y; redemption; frequency; [basis]`.
fn bond_args(a: &mut Args<'_>) -> Result<(Coupon, f64, f64, f64), ErrorKind> {
    let nd = null_date(a);
    let settle = long(a, 0)?;
    let mat = long(a, 1)?;
    let x = a.num(2)?;
    let y = a.num(3)?;
    let red = a.num(4)?;
    let freq = long(a, 5)?;
    Ok((
        Coupon {
            nd,
            settle,
            mat,
            freq,
            base: 0,
        },
        x,
        y,
        red,
    ))
}

/// `PRICE(settlement; maturity; rate; yield; redemption; frequency; [basis])`.
pub const PRICE: FunctionDef = FunctionDef::new("PRICE", Category::AddIn, PRICE_PARAMS, price_fn)
    .help(
    "Returns the price per 100 currency units face value of a security that pays periodic interest",
);

fn price_fn(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let (mut c, rate, yld, red) = bond_args(&mut a)?;
        if yld < 0.0 || rate < 0.0 || red <= 0.0 || check_freq(c.freq).is_err() || c.settle >= c.mat
        {
            return Err(IAE);
        }
        c.base = basis(&mut a, 6)?;
        finite(price(&c, rate, yld, red)?)
    })
}

/// `YIELD(settlement; maturity; rate; price; redemption; frequency; [basis])`.
pub const YIELD: FunctionDef = FunctionDef::new("YIELD", Category::AddIn, PRICE_PARAMS, yield_fn)
    .help("Returns the yield on a security that pays periodic interest");

fn yield_fn(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let (mut c, coup, pr, red) = bond_args(&mut a)?;
        if coup < 0.0 || pr <= 0.0 || red <= 0.0 || check_freq(c.freq).is_err() || c.settle >= c.mat
        {
            return Err(IAE);
        }
        c.base = basis(&mut a, 6)?;
        finite(yield_(&c, coup, pr, red)?)
    })
}

const DURATION_PARAMS: &[Param] = &[
    Param::required("settlement", ParamKind::Integer),
    Param::required("maturity", ParamKind::Integer),
    Param::required("coupon", ParamKind::Number),
    Param::required("yield", ParamKind::Number),
    Param::required("frequency", ParamKind::Integer),
    Param::optional("basis", ParamKind::Integer),
];

fn duration_args(a: &mut Args<'_>) -> Result<(Coupon, f64, f64), ErrorKind> {
    let nd = null_date(a);
    let settle = long(a, 0)?;
    let mat = long(a, 1)?;
    let coup = a.num(2)?;
    let yld = a.num(3)?;
    let freq = long(a, 4)?;
    Ok((
        Coupon {
            nd,
            settle,
            mat,
            freq,
            base: 0,
        },
        coup,
        yld,
    ))
}

/// `DURATION_ADD(settlement; maturity; coupon; yield; frequency; [basis])`:
/// the Analysis add-in `DURATION` (Macaulay duration). Its ODF and OOXML
/// name is `DURATION`; the built-in `DURATION` (periods) is ODF `PDURATION`.
pub const DURATION_ADD: FunctionDef = FunctionDef::new(
    "DURATION_ADD",
    Category::AddIn,
    DURATION_PARAMS,
    duration_add,
)
.odf("DURATION")
.help("Returns the annual Macaulay duration of a security with periodic interest payments");

fn duration_add(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let (mut c, coup, yld) = duration_args(&mut a)?;
        if coup < 0.0 || yld < 0.0 || check_freq(c.freq).is_err() || c.settle >= c.mat {
            return Err(IAE);
        }
        c.base = basis(&mut a, 5)?;
        finite(duration(&c, coup, yld)?)
    })
}

/// `MDURATION(settlement; maturity; coupon; yield; frequency; [basis])`.
pub const MDURATION: FunctionDef =
    FunctionDef::new("MDURATION", Category::AddIn, DURATION_PARAMS, mduration)
        .help("Returns the Macaulay modified duration for a security with an assumed par value of 100 currency units");

fn mduration(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let (mut c, coup, yld) = duration_args(&mut a)?;
        if coup < 0.0 || yld < 0.0 || check_freq(c.freq).is_err() {
            return Err(IAE);
        }
        c.base = basis(&mut a, 5)?;
        let d = duration(&c, coup, yld)?;
        finite(d / (1.0 + yld / c.freq as f64))
    })
}

const ODDL_PARAMS: &[Param] = &[
    Param::required("settlement", ParamKind::Integer),
    Param::required("maturity", ParamKind::Integer),
    Param::required("last_interest", ParamKind::Integer),
    Param::required("rate", ParamKind::Number),
    Param::required("y", ParamKind::Number),
    Param::required("redemption", ParamKind::Number),
    Param::required("frequency", ParamKind::Integer),
    Param::optional("basis", ParamKind::Integer),
];

/// ODDLPRICE (`price_mode`, `y` is the yield) and ODDLYIELD (`y` is the
/// price): `GetOddlprice` / `GetOddlyield`.
fn oddl(a: &mut Args<'_>, price_mode: bool) -> Result<Value, ErrorKind> {
    let nd = null_date(a);
    let settle = long(a, 0)?;
    let mat = long(a, 1)?;
    let last = long(a, 2)?;
    let rate = a.num(3)?;
    let y = a.num(4)?;
    let red = a.num(5)?;
    let freq = long(a, 6)?;
    let bad_y = if price_mode { y < 0.0 } else { y <= 0.0 };
    if rate < 0.0 || bad_y || check_freq(freq).is_err() || mat <= settle || settle <= last {
        return Err(IAE);
    }
    let b = basis(a, 7)?;
    let f = freq as f64;
    let dci = year_frac(nd, last, mat, b)? * f;
    let dsci = year_frac(nd, settle, mat, b)? * f;
    let ai = year_frac(nd, last, settle, b)? * f;
    if price_mode {
        let mut p = red + dci * 100.0 * rate / f;
        p /= dsci * y / f + 1.0;
        p -= ai * 100.0 * rate / f;
        finite(p)
    } else {
        let mut r = red + dci * 100.0 * rate / f;
        r /= y + ai * 100.0 * rate / f;
        r -= 1.0;
        r *= f / dsci;
        finite(r)
    }
}

/// `ODDLPRICE(settlement; maturity; last_interest; rate; yield; redemption; frequency; [basis])`.
pub const ODDLPRICE: FunctionDef =
    FunctionDef::new("ODDLPRICE", Category::AddIn, ODDL_PARAMS, oddlprice)
        .help("Returns the price per $100 face value of a security with an odd last period");

fn oddlprice(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| oddl(&mut a, true))
}

/// `ODDLYIELD(settlement; maturity; last_interest; rate; price; redemption; frequency; [basis])`.
pub const ODDLYIELD: FunctionDef =
    FunctionDef::new("ODDLYIELD", Category::AddIn, ODDL_PARAMS, oddlyield)
        .help("Returns the yield of a security with an odd last period");

fn oddlyield(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| oddl(&mut a, false))
}

/// An odd-first-period security (serials and terms).
struct OddFirst {
    nd: i64,
    settle: i64,
    mat: i64,
    issue: i64,
    first: i64,
    rate: f64,
    red: f64,
    freq: i64,
    base: i32,
}

/// Upper bound on quasi-coupon periods in the odd first period.
const MAX_QUASI: i64 = 10_000;

impl OddFirst {
    /// Days between two serials under the basis (`ScaDate::getDiff`).
    fn days(&self, d1: i64, d2: i64) -> Result<f64, ErrorKind> {
        let a = ScaDate::new(self.nd, d1, self.base)?;
        let b = ScaDate::new(self.nd, d2, self.base)?;
        Ok(ScaDate::diff(&a, &b)? as f64)
    }

    /// Price for yield `y` (Excel's odd-first-coupon formula).
    fn price(&self, y: f64) -> Result<f64, ErrorKind> {
        let f = self.freq as f64;
        let step = 12 / self.freq;
        let first = ScaDate::new(self.nd, self.first, self.base)?;
        let quasi = |k: i64| -> Result<i64, ErrorKind> {
            let mut d = first;
            d.add_months(-k * step)?;
            Ok(d.date(self.nd))
        };
        // Quasi-coupon dates q_0 <= issue < q_1 < ... < q_nc = first coupon.
        let mut nc = 1;
        while quasi(nc)? > self.issue {
            nc += 1;
            if nc > MAX_QUASI {
                return Err(IAE);
            }
        }
        let q: Vec<i64> = (0..=nc).rev().map(quasi).collect::<Result<_, _>>()?;
        let period_len = |i: usize| -> Result<f64, ErrorKind> {
            if self.base == 1 {
                self.days(q[i - 1], q[i])
            } else {
                Ok(days_in_year(0, 0, self.base)? as f64 / f)
            }
        };
        let mut dc_sum = 0.0;
        let mut a_sum = 0.0;
        // Settlement lies in quasi-coupon period j: q_{j-1} <= settle < q_j.
        let j = (1..=nc as usize)
            .find(|&i| self.settle < q[i])
            .unwrap_or(nc as usize);
        for i in 1..=nc as usize {
            let start = q[i - 1].max(self.issue);
            let nl = period_len(i)?;
            dc_sum += self.days(start, q[i])? / nl;
            if self.settle > start {
                a_sum += self.days(start, self.settle.min(q[i]))? / nl;
            }
        }
        let e = period_len(j)?;
        let dsc_e = self.days(self.settle, q[j])? / e;
        let nq = (nc as usize - j) as f64;
        let n = Coupon {
            nd: self.nd,
            settle: self.first,
            mat: self.mat,
            freq: self.freq,
            base: self.base,
        }
        .num()?;
        let c = 100.0 * self.rate / f;
        let v = 1.0 + y / f;
        let mut p = self.red / v.powf(nq + n + dsc_e);
        p += c * dc_sum / v.powf(nq + dsc_e);
        let mut k = 1.0;
        while k <= n {
            p += c / v.powf(nq + k + dsc_e);
            k += 1.0;
        }
        Ok(p - c * a_sum)
    }

    /// Yield for price `pr`: bisection on the (decreasing) price curve.
    fn yield_for(&self, pr: f64) -> Result<f64, ErrorKind> {
        let f = self.freq as f64;
        let mut lo = 0.0;
        let mut hi = 1.0;
        let mut guard = 0;
        while self.price(hi)? > pr {
            hi *= 2.0;
            guard += 1;
            if guard > 60 {
                return Err(IAE);
            }
        }
        guard = 0;
        while self.price(lo)? < pr {
            lo = (lo - f) / 2.0;
            guard += 1;
            if guard > 60 {
                return Err(IAE);
            }
        }
        for _ in 0..200 {
            let mid = 0.5 * (lo + hi);
            if mid == lo || mid == hi {
                break;
            }
            if self.price(mid)? > pr {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        Ok(0.5 * (lo + hi))
    }
}

const ODDF_PARAMS: &[Param] = &[
    Param::required("settlement", ParamKind::Integer),
    Param::required("maturity", ParamKind::Integer),
    Param::required("issue", ParamKind::Integer),
    Param::required("first_coupon", ParamKind::Integer),
    Param::required("rate", ParamKind::Number),
    Param::required("y", ParamKind::Number),
    Param::required("redemption", ParamKind::Number),
    Param::required("frequency", ParamKind::Integer),
    Param::optional("basis", ParamKind::Integer),
];

fn oddf(a: &mut Args<'_>, price_mode: bool) -> Result<Value, ErrorKind> {
    let nd = null_date(a);
    let settle = long(a, 0)?;
    let mat = long(a, 1)?;
    let issue = long(a, 2)?;
    let first = long(a, 3)?;
    let rate = a.num(4)?;
    let y = a.num(5)?;
    let red = a.num(6)?;
    let freq = long(a, 7)?;
    let bad_y = if price_mode { y < 0.0 } else { y <= 0.0 };
    if rate < 0.0
        || bad_y
        || check_freq(freq).is_err()
        || mat <= first
        || first <= settle
        || settle <= issue
    {
        return Err(IAE);
    }
    let base = basis(a, 8)?;
    let s = OddFirst {
        nd,
        settle,
        mat,
        issue,
        first,
        rate,
        red,
        freq,
        base,
    };
    if price_mode {
        finite(s.price(y)?)
    } else {
        finite(s.yield_for(y)?)
    }
}

/// `ODDFPRICE(settlement; maturity; issue; first_coupon; rate; yield; redemption; frequency; [basis])`.
pub const ODDFPRICE: FunctionDef =
    FunctionDef::new("ODDFPRICE", Category::AddIn, ODDF_PARAMS, oddfprice)
        .help("Returns the price per $100 face value of a security with an odd first period");

fn oddfprice(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| oddf(&mut a, true))
}

/// `ODDFYIELD(settlement; maturity; issue; first_coupon; rate; price; redemption; frequency; [basis])`.
pub const ODDFYIELD: FunctionDef =
    FunctionDef::new("ODDFYIELD", Category::AddIn, ODDF_PARAMS, oddfyield)
        .help("Returns the yield of a security with an odd first period");

fn oddfyield(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| oddf(&mut a, false))
}

pub(super) fn register(r: &mut Registry) {
    r.register(PRICE);
    r.register(YIELD);
    r.register(DURATION_ADD);
    r.register(MDURATION);
    r.register(ODDFPRICE);
    r.register(ODDFYIELD);
    r.register(ODDLPRICE);
    r.register(ODDLYIELD);
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

    #[test]
    fn price_yield() {
        let mut c = MockEvalContext::new();
        // 2008-02-15, 2017-11-15
        close(
            num(
                &mut c,
                "PRICE",
                &[39493.0, 43054.0, 0.0575, 0.065, 100.0, 2.0, 0.0],
            ),
            94.6343616213,
            1e-8,
        );
        // 2008-02-15, 2016-11-15
        close(
            num(
                &mut c,
                "YIELD",
                &[39493.0, 42689.0, 0.0575, 95.04287, 100.0, 2.0, 0.0],
            ),
            0.065,
            1e-6,
        );
        assert_eq!(
            call(
                &mut c,
                "PRICE",
                [39493.0, 43054.0, 0.0575, 0.065, 100.0, 3.0]
                    .map(EvalValue::number)
                    .to_vec()
            ),
            Value::Error(ErrorKind::InvalidArgument)
        );
    }

    #[test]
    fn durations() {
        let mut c = MockEvalContext::new();
        // 2008-01-01, 2016-01-01
        let d = [39448.0, 42370.0, 0.08, 0.09, 2.0, 1.0];
        close(num(&mut c, "DURATION_ADD", &d), 5.993775, 1e-6);
        close(num(&mut c, "MDURATION", &d), 5.73567, 1e-5);
    }

    #[test]
    fn odd_periods() {
        let mut c = MockEvalContext::new();
        // 2008-11-11, 2021-03-01, 2008-10-15, 2009-03-01
        let f = [
            39763.0, 44256.0, 39736.0, 39873.0, 0.0785, 0.0625, 100.0, 2.0, 1.0,
        ];
        close(num(&mut c, "ODDFPRICE", &f), 113.597717, 1e-5);
        let mut g = f;
        g[5] = 113.597717474079;
        close(num(&mut c, "ODDFYIELD", &g), 0.0625, 1e-9);
        // 2008-02-07, 2008-06-15, 2007-10-15
        let l = [39485.0, 39614.0, 39370.0, 0.0375, 0.0405, 100.0, 2.0, 0.0];
        close(num(&mut c, "ODDLPRICE", &l), 99.8782860147, 1e-8);
        // 2008-04-20, 2008-06-15, 2007-12-24
        let m = [39558.0, 39614.0, 39440.0, 0.0375, 99.875, 100.0, 2.0, 0.0];
        close(num(&mut c, "ODDLYIELD", &m), 0.0451922, 1e-6);
    }
}
