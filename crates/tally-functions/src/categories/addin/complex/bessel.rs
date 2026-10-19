//! BESSELI, BESSELJ, BESSELK, BESSELY.
//!
//! Ported from Apache OpenOffice `scaddins/source/analysis/bessel.cxx` and
//! `analysishelper.cxx` (Apache-2.0): the Gautschi-style recurrence for J and
//! Y0, the power series for I, and the polynomial approximations for K0/K1
//! with the upward recurrence for higher orders. Y1 is computed from a
//! Neumann series of Bessel J values (Miller's backward recurrence) for
//! `x <= 25` and the Hankel asymptotic expansion above; AOO's own Y1 loop is
//! not reproduced.

use std::f64::consts::{FRAC_1_PI, FRAC_PI_2, FRAC_PI_4, PI};

use tally_types::{ErrorKind, Value};

use super::cplx::BAD;
use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry};
use crate::{run, ArgList, Args, EvalContext};

const PARAMS: &[Param] = &[
    Param::required("x", ParamKind::Number),
    Param::required("n", ParamKind::Integer),
];

const MAX_ITERATION: f64 = 9_000_000.0;
const EULER_GAMMA: f64 = 0.577_215_664_901_532_9;

fn fak(n: i64) -> f64 {
    let mut r = 1.0;
    for k in 2..=n {
        r *= k as f64;
        if r.is_infinite() {
            break;
        }
    }
    r
}

/// Bessel function of the first kind, integer order.
pub(super) fn bessel_j(x: f64, n: i64) -> Result<f64, ErrorKind> {
    if n < 0 {
        return Err(BAD);
    }
    if x == 0.0 {
        return Ok(if n == 0 { 1.0 } else { 0.0 });
    }
    // Even order: even function; odd order: odd function.
    let sign = if n % 2 == 1 && x < 0.0 { -1.0 } else { 1.0 };
    let x = x.abs();
    let nf = n as f64;
    let estimate = x * 1.5 + nf;
    let asymptotic_possible = x.powf(0.4) > nf;
    if estimate > MAX_ITERATION {
        return if asymptotic_possible {
            Ok(sign * (2.0 / (PI * x)).sqrt() * (x - nf * FRAC_PI_2 - FRAC_PI_4).cos())
        } else {
            Err(BAD)
        };
    }
    let eps = 1.0e-15;
    let mut k: f64;
    let mut u: f64;
    let mut g: f64;
    let mut delta_u: f64 = 0.0;
    let mut f_bar: f64 = -1.0;
    if n == 0 {
        u = 1.0;
        let g_bar = -2.0 / x;
        delta_u = 0.0 / g_bar;
        u += delta_u;
        g = -1.0 / g_bar;
        f_bar *= g;
        k = 2.0;
    } else {
        u = 0.0;
        g = 0.0;
        k = 1.0;
        while k <= nf - 1.0 {
            let m_bar = 2.0 * ((k - 1.0) % 2.0) * f_bar;
            let g_bar_delta_u = -g * delta_u - m_bar * u;
            let g_bar = m_bar - 2.0 * k / x + g;
            delta_u = g_bar_delta_u / g_bar;
            u += delta_u;
            g = -1.0 / g_bar;
            f_bar *= g;
            k += 1.0;
        }
        // Step with alpha_N = 1.
        let m_bar = 2.0 * ((k - 1.0) % 2.0) * f_bar;
        let g_bar_delta_u = f_bar - g * delta_u - m_bar * u;
        let g_bar = m_bar - 2.0 * k / x + g;
        delta_u = g_bar_delta_u / g_bar;
        u += delta_u;
        g = -1.0 / g_bar;
        f_bar *= g;
        k += 1.0;
    }
    loop {
        let m_bar = 2.0 * ((k - 1.0) % 2.0) * f_bar;
        let g_bar_delta_u = -g * delta_u - m_bar * u;
        let g_bar = m_bar - 2.0 * k / x + g;
        delta_u = g_bar_delta_u / g_bar;
        u += delta_u;
        g = -1.0 / g_bar;
        f_bar *= g;
        let found = delta_u.abs() <= u.abs() * eps;
        k += 1.0;
        if found {
            return Ok(u * sign);
        }
        if k > MAX_ITERATION {
            return Err(BAD);
        }
    }
}

/// Modified Bessel function of the first kind (power series).
pub(super) fn bessel_i(x: f64, n: i64) -> Result<f64, ErrorKind> {
    if n < 0 {
        return Err(BAD);
    }
    const MAX_ITER: i64 = 5000;
    let half = x / 2.0;
    let mut term = half.powi(n.min(i64::from(i32::MAX)) as i32) / fak(n);
    let mut result = term;
    let mut k = 0i64;
    while term.abs() > 1.0e-15 * result.abs() && term != 0.0 {
        k += 1;
        if k > MAX_ITER {
            return Err(BAD);
        }
        term = term * half / k as f64 * half / (k + n) as f64;
        result += term;
    }
    Ok(result)
}

fn bessk0(x: f64) -> Result<f64, ErrorKind> {
    if x <= 2.0 {
        let x2 = x * 0.5;
        let y = x2 * x2;
        Ok(-x2.ln() * bessel_i(x, 0)?
            + (-0.577_215_66
                + y * (0.422_784_20
                    + y * (0.230_697_56
                        + y * (0.348_859_0e-1
                            + y * (0.262_698e-2 + y * (0.107_50e-3 + y * 0.74e-5)))))))
    } else {
        let y = 2.0 / x;
        Ok((-x).exp() / x.sqrt()
            * (1.253_314_14
                + y * (-0.783_235_8e-1
                    + y * (0.218_956_8e-1
                        + y * (-0.106_244_6e-1
                            + y * (0.587_872e-2 + y * (-0.251_540e-2 + y * 0.532_08e-3)))))))
    }
}

fn bessk1(x: f64) -> Result<f64, ErrorKind> {
    if x <= 2.0 {
        let x2 = x * 0.5;
        let y = x2 * x2;
        Ok(x2.ln() * bessel_i(x, 1)?
            + (1.0
                + y * (0.154_431_44
                    + y * (-0.672_785_79
                        + y * (-0.181_568_97
                            + y * (-0.191_940_2e-1 + y * (-0.110_404e-2 + y * (-0.4686e-4)))))))
                / x)
    } else {
        let y = 2.0 / x;
        Ok((-x).exp() / x.sqrt()
            * (1.253_314_14
                + y * (0.234_986_19
                    + y * (-0.365_562_0e-1
                        + y * (0.150_426_8e-1
                            + y * (-0.780_353e-2 + y * (0.325_614e-2 + y * (-0.682_45e-3))))))))
    }
}

/// Modified Bessel function of the second kind.
pub(super) fn bessel_k(x: f64, n: i64) -> Result<f64, ErrorKind> {
    if n < 0 || x <= 0.0 {
        return Err(BAD);
    }
    match n {
        0 => bessk0(x),
        1 => bessk1(x),
        _ => {
            let tox = 2.0 / x;
            let mut bkm = bessk0(x)?;
            let mut bk = bessk1(x)?;
            for j in 1..n {
                let bkp = bkm + j as f64 * tox * bk;
                bkm = bk;
                bk = bkp;
            }
            Ok(bk)
        }
    }
}

fn bessely0(x: f64) -> Result<f64, ErrorKind> {
    if x <= 0.0 {
        return Err(BAD);
    }
    if x > 5e6 {
        return Ok((1.0 / PI / x).sqrt() * (x.sin() - x.cos()));
    }
    let eps = 1.0e-15;
    let mut u = (x / 2.0).ln() + EULER_GAMMA;
    let mut g_bar = -2.0 / x;
    let mut delta_u = 0.0 / g_bar;
    let mut g = -1.0 / g_bar;
    let mut f_bar = -g;
    let mut sign_alpha = 1.0;
    let mut k = 2.0;
    loop {
        let km1mod2 = (k - 1.0) % 2.0;
        let m_bar = 2.0 * km1mod2 * f_bar;
        let alpha = if km1mod2 == 0.0 {
            0.0
        } else {
            let a = sign_alpha * (4.0 / k);
            sign_alpha = -sign_alpha;
            a
        };
        let g_bar_delta_u = f_bar * alpha - g * delta_u - m_bar * u;
        g_bar = m_bar - 2.0 * k / x + g;
        delta_u = g_bar_delta_u / g_bar;
        u += delta_u;
        g = -1.0 / g_bar;
        f_bar *= g;
        let found = delta_u.abs() <= u.abs() * eps;
        k += 1.0;
        if found {
            return Ok(u * 2.0 * FRAC_1_PI);
        }
        if k >= MAX_ITERATION {
            return Err(BAD);
        }
    }
}

/// Hankel asymptotic expansion of `Y_nu(x)` for large `x`.
fn hankel_y(nu: f64, x: f64) -> f64 {
    let mu = 4.0 * nu * nu;
    let (mut p, mut q) = (1.0, 0.0);
    let mut a = 1.0;
    let mut last = f64::INFINITY;
    for k in 1..60 {
        let kf = f64::from(k);
        a *= (mu - (2.0 * kf - 1.0).powi(2)) / (kf * 8.0 * x);
        if a.abs() > last || a.abs() < 1e-18 {
            break;
        }
        last = a.abs();
        match k % 4 {
            1 => q += a,
            2 => p -= a,
            3 => q -= a,
            _ => p += a,
        }
    }
    let chi = x - (nu / 2.0 + 0.25) * PI;
    (2.0 / (PI * x)).sqrt() * (p * chi.sin() + q * chi.cos())
}

fn bessely1(x: f64) -> Result<f64, ErrorKind> {
    if x <= 0.0 {
        return Err(BAD);
    }
    if x > 25.0 {
        return Ok(hankel_y(1.0, x));
    }
    // Miller backward recurrence for J_0..J_m, normalised by J0 + 2 sum J_2k = 1.
    let m = (x as usize + 40) / 2 * 2 + 20;
    let mut j = vec![0.0f64; m + 2];
    j[m] = 1e-30;
    for k in (1..m).rev() {
        j[k - 1] = 2.0 * k as f64 / x * j[k] - j[k + 1];
        if j[k - 1].abs() > 1e250 {
            for v in &mut j[k - 1..=m] {
                *v *= 1e-250;
            }
        }
    }
    let norm = j[0] + 2.0 * j.iter().skip(2).step_by(2).take(m / 2).sum::<f64>();
    let jj = |k: usize| j[k] / norm;
    let mut s = jj(1);
    let mut sgn = -1.0;
    let mut mm = 1;
    while 2 * mm + 1 < m {
        s += sgn * (1.0 / mm as f64 + 1.0 / (mm + 1) as f64) * jj(2 * mm + 1);
        sgn = -sgn;
        mm += 1;
    }
    Ok(2.0 * FRAC_1_PI * (-jj(0) / x + ((x / 2.0).ln() + EULER_GAMMA) * jj(1) - s))
}

/// Bessel function of the second kind, integer order.
pub(super) fn bessel_y(x: f64, n: i64) -> Result<f64, ErrorKind> {
    if n < 0 || x <= 0.0 {
        return Err(BAD);
    }
    match n {
        0 => bessely0(x),
        1 => bessely1(x),
        _ => {
            let tox = 2.0 / x;
            let mut bym = bessely0(x)?;
            let mut by = bessely1(x)?;
            for j in 1..n {
                let byp = j as f64 * tox * by - bym;
                bym = by;
                by = byp;
            }
            Ok(by)
        }
    }
}

macro_rules! bessel_fn {
    ($konst:ident, $name:literal, $help:literal, $f:path) => {
        pub const $konst: FunctionDef = FunctionDef::new(
            $name,
            Category::AddIn,
            PARAMS,
            |ctx: &mut dyn EvalContext, args: &mut dyn ArgList| {
                let mut a = Args::new(ctx, args);
                run(|| {
                    let x = a.num(0)?;
                    let n = a.int(1)?;
                    Ok(Value::Number($f(x, n)?))
                })
            },
        )
        .help($help);
    };
}

bessel_fn!(
    BESSELI,
    "BESSELI",
    "Returns the modified Bessel function In(x)",
    bessel_i
);
bessel_fn!(
    BESSELJ,
    "BESSELJ",
    "Returns the Bessel function Jn(x)",
    bessel_j
);
bessel_fn!(
    BESSELK,
    "BESSELK",
    "Returns the modified Bessel function Kn(x)",
    bessel_k
);
bessel_fn!(
    BESSELY,
    "BESSELY",
    "Returns the Bessel function Yn(x)",
    bessel_y
);

pub(super) fn register(r: &mut Registry) {
    r.register(BESSELI);
    r.register(BESSELJ);
    r.register(BESSELK);
    r.register(BESSELY);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;

    fn close(got: Result<f64, ErrorKind>, want: f64, rel: f64) {
        let g = got.expect("ok");
        assert!(
            (g - want).abs() <= rel * want.abs().max(1e-3),
            "got {g}, want {want}"
        );
    }

    // Reference values from scipy.special.
    #[test]
    fn j() {
        close(bessel_j(1.0, 0), 0.765197686557967, 1e-13);
        close(bessel_j(1.0, 1), 0.440050585744934, 1e-13);
        close(bessel_j(2.5, 3), 0.216600391039114, 1e-13);
        close(bessel_j(10.0, 2), 0.254630313685121, 1e-13);
        close(bessel_j(30.0, 5), -0.143240295512077, 1e-12);
        close(bessel_j(100.0, 10), -0.054732176935472, 1e-12);
        close(bessel_j(0.1, 7), 1.54961486762023e-13, 1e-12);
        close(bessel_j(1000.0, 3), -0.00482742082520395, 1e-11);
        close(bessel_j(-2.5, 3), -0.216600391039114, 1e-13);
        assert_eq!(bessel_j(0.0, 0), Ok(1.0));
        assert_eq!(bessel_j(0.0, 2), Ok(0.0));
        assert_eq!(bessel_j(1.0, -1), Err(BAD));
    }

    #[test]
    fn i_k() {
        close(bessel_i(1.0, 0), 1.26606587775201, 1e-13);
        close(bessel_i(1.0, 1), 0.565159103992485, 1e-13);
        close(bessel_i(10.0, 2), 2281.518967726, 1e-12);
        close(bessel_i(30.0, 5), 512151465476.935, 1e-12);
        close(bessel_i(-2.5, 3), -0.474370408778036, 1e-12);
        // K uses AOO's ~1e-7 polynomial approximations.
        close(bessel_k(1.0, 0), 0.421024438240708, 2e-7);
        close(bessel_k(1.0, 1), 0.601907230197235, 2e-7);
        close(bessel_k(2.5, 3), 0.268227146393449, 5e-6);
        close(bessel_k(0.5, 0), 0.924419071227666, 2e-7);
        close(bessel_k(7.0, 0), 0.000424795741869232, 1e-5);
        assert_eq!(bessel_k(0.0, 1), Err(BAD));
        assert_eq!(bessel_k(1.0, -1), Err(BAD));
    }

    #[test]
    fn y() {
        close(bessel_y(1.0, 0), 0.088256964215677, 1e-12);
        close(bessel_y(1.0, 1), -0.781212821300289, 1e-12);
        close(bessel_y(2.5, 3), -0.756055496753671, 1e-12);
        close(bessel_y(10.0, 2), -0.00586808244220859, 1e-11);
        close(bessel_y(30.0, 5), 0.0316273592892644, 1e-11);
        close(bessel_y(100.0, 10), 0.0583315742364149, 1e-11);
        close(bessel_y(0.5, 0), -0.444518733506707, 1e-12);
        close(bessel_y(7.0, 0), -0.0259497439672093, 1e-11);
        close(bessel_y(0.1, 7), -293476652667.392, 1e-10);
        close(bessel_y(1000.0, 3), 0.024765269345791, 1e-10);
        assert_eq!(bessel_y(0.0, 0), Err(BAD));
        assert_eq!(bessel_y(-1.0, 1), Err(BAD));
    }

    #[test]
    fn through_registry() {
        let mut c = MockEvalContext::new();
        let v = call(
            &mut c,
            "BESSELJ",
            vec![EvalValue::number(1.0), EvalValue::number(0.0)],
        );
        match v {
            Value::Number(x) => assert!((x - 0.7651976866).abs() < 1e-9),
            o => panic!("{o:?}"),
        }
        let v = call(
            &mut c,
            "BESSELJ",
            vec![EvalValue::number(1.0), EvalValue::number(-1.0)],
        );
        assert_eq!(v, Value::Error(BAD));
    }
}
