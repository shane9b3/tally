//! Numeric and data helpers shared by the statistical, distribution,
//! array and engineering families. There is exactly one implementation of
//! each; do not add private copies in a family file.
//!
//! - The standard normal density/distribution/inverse and erf/erfc, ported
//!   from Apache OpenOffice (`sc/source/core/tool/interpr3.cxx`: `phi`,
//!   `integralPhi`, `gaussinv`; `gauss` via erf, see its docs; `sal/rtl/source/math.cxx`:
//!   `rtl::math::erf`/`erfc`, Kramer & Blomquist), Apache License 2.0.
//!   AOO's Analysis add-in ERF/ERFC also call `rtl::math::erf`/`erfc`.
//! - [`SparseData`]: one argument (reference, array or scalar) read
//!   sparsely in column-major order, the order of AOO's value iterator.

// The coefficients are copied verbatim from the AOO sources (some carry more
// digits than an f64 holds) so they can be checked against the original.
#![allow(clippy::excessive_precision)]

use std::ops::ControlFlow;

use tally_types::{ErrorKind, Value};

use crate::{EvalContext, EvalValue};

// ---------------------------------------------------------------------------
// Normal distribution
// ---------------------------------------------------------------------------

/// Standard normal density (AOO `phi`).
pub(crate) fn phi(x: f64) -> f64 {
    0.398_942_280_401_432_68 * (-(x * x) / 2.0).exp()
}

/// Standard normal cumulative distribution (AOO `integralPhi`).
pub(crate) fn integral_phi(x: f64) -> f64 {
    0.5 * erfc(-x * std::f64::consts::FRAC_1_SQRT_2)
}

/// `integralPhi(x) - 0.5` (GAUSS, ZTEST).
///
/// AOO's `gauss` sums piecewise Taylor series around 0, 2 and 4 and an
/// asymptotic series beyond 5, which is only good to about 2e-11 there
/// (GAUSS(5) = 0.499999713328 instead of 0.499999713348). We compute the
/// same quantity as `erf(x / sqrt 2) / 2` with the AOO erf above, accurate
/// to ~1e-16 everywhere; AOO results differ only past the 10th digit.
pub(crate) fn gauss(x: f64) -> f64 {
    0.5 * erf(x * std::f64::consts::FRAC_1_SQRT_2)
}

/// Inverse of the standard normal distribution, Wichura's AS241 rational
/// approximation (AOO `gaussinv`, #i26836#). Requires `0 < x < 1`.
pub(crate) fn gaussinv(x: f64) -> f64 {
    let q = x - 0.5;
    if q.abs() <= 0.425 {
        let t = 0.180625 - q * q;
        return q
            * (((((((t * 2_509.080_928_730_122_672_7 + 33_430.575_583_588_128_105) * t
                + 67_265.770_927_008_700_853)
                * t
                + 45_921.953_931_549_871_457)
                * t
                + 13_731.693_765_509_461_125)
                * t
                + 1_971.590_950_306_551_442_7)
                * t
                + 133.141_667_891_784_377_45)
                * t
                + 3.387_132_872_796_366_608)
            / (((((((t * 5_226.495_278_852_854_561 + 28_729.085_735_721_942_674) * t
                + 39_307.895_800_092_710_61)
                * t
                + 21_213.794_301_586_595_867)
                * t
                + 5_394.196_021_424_751_107_7)
                * t
                + 687.187_007_492_057_908_3)
                * t
                + 42.313_330_701_600_911_252)
                * t
                + 1.0);
    }
    let mut t = if q > 0.0 { 1.0 - x } else { x };
    t = (-t.ln()).sqrt();
    let z = if t <= 5.0 {
        t -= 1.6;
        (((((((t * 7.745_450_142_783_414_076_4e-4 + 0.022_723_844_989_269_184_583_3) * t
            + 0.241_780_725_177_450_611_77)
            * t
            + 1.270_458_252_452_368_382_58)
            * t
            + 3.647_848_324_763_204_605_04)
            * t
            + 5.769_497_221_460_691_405_5)
            * t
            + 4.630_337_846_156_545_295_9)
            * t
            + 1.423_437_110_749_683_577_34)
            / (((((((t * 1.050_750_071_644_416_843_24e-9 + 5.475_938_084_995_344_946e-4) * t
                + 0.015_198_666_563_616_457_196_6)
                * t
                + 0.148_103_976_427_480_074_59)
                * t
                + 0.689_767_334_985_100_004_55)
                * t
                + 1.676_384_830_183_803_849_4)
                * t
                + 2.053_191_626_637_758_821_87)
                * t
                + 1.0)
    } else {
        t -= 5.0;
        (((((((t * 2.010_334_399_292_288_132_65e-7 + 2.711_555_568_743_487_578_15e-5) * t
            + 0.001_242_660_947_388_078_438_6)
            * t
            + 0.026_532_189_526_576_123_093)
            * t
            + 0.296_560_571_828_504_891_23)
            * t
            + 1.784_826_539_917_291_335_8)
            * t
            + 5.463_784_911_164_114_369_9)
            * t
            + 6.657_904_643_501_103_777_2)
            / (((((((t * 2.044_263_103_389_939_785_64e-15 + 1.421_511_758_316_445_888_7e-7)
                * t
                + 1.846_318_317_510_054_681_8e-5)
                * t
                + 7.868_691_311_456_132_591e-4)
                * t
                + 0.014_875_361_290_850_614_852_5)
                * t
                + 0.136_929_880_922_735_805_31)
                * t
                + 0.599_832_206_555_887_937_69)
                * t
                + 1.0)
    };
    if q < 0.0 {
        -z
    } else {
        z
    }
}

// ---------------------------------------------------------------------------
// erf / erfc (Kramer & Blomquist rational approximations, AOO rtl::math)
// ---------------------------------------------------------------------------

fn erf_0065(x: f64) -> f64 {
    const PN: [f64; 5] = [
        std::f64::consts::FRAC_2_SQRT_PI, // AOO: 1.12837916709551256 (same f64)
        1.358_948_876_272_779_16e-1,
        4.032_594_885_317_952_74e-2,
        1.203_393_808_630_794_57e-3,
        6.492_545_564_819_043_54e-5,
    ];
    const QN: [f64; 5] = [
        1.0,
        4.537_670_417_800_025_45e-1,
        8.699_362_226_153_858_90e-2,
        8.497_173_711_686_933_57e-3,
        3.649_152_806_293_510_82e-4,
    ];
    let (mut p, mut q, mut pow) = (0.0, 0.0, 1.0);
    for i in 0..5 {
        p += PN[i] * pow;
        q += QN[i] * pow;
        pow *= x * x;
    }
    x * p / q
}

fn erfc_0600(x: f64) -> f64 {
    const PN22: [f64; 6] = [
        9.999_999_920_497_990_98e-1,
        1.331_541_639_367_653_07,
        8.781_158_041_558_817_82e-1,
        3.318_995_595_782_132_15e-1,
        7.141_938_325_067_760_67e-2,
        7.069_408_437_632_531_31e-3,
    ];
    const QN22: [f64; 7] = [
        1.0,
        2.459_920_701_442_455_33,
        2.653_839_728_697_757_52,
        1.618_766_555_438_713_76,
        5.946_513_112_864_815_02e-1,
        1.265_794_130_301_779_40e-1,
        1.253_049_365_494_133_93e-2,
    ];
    const PN60: [f64; 6] = [
        9.999_211_400_097_144_09e-1,
        1.623_565_844_893_666_47,
        1.267_399_014_558_732_22,
        5.815_285_741_777_411_35e-1,
        1.572_896_207_428_387_02e-1,
        2.257_169_829_192_175_55e-2,
    ];
    const QN60: [f64; 7] = [
        1.0,
        2.751_438_706_763_762_08,
        3.373_673_346_572_845_35,
        2.385_741_947_853_443_89,
        1.050_740_046_148_272_06,
        2.787_884_392_736_289_83e-1,
        4.000_729_645_268_613_62e-2,
    ];
    let (pn, qn) = if x < 2.2 {
        (&PN22, &QN22)
    } else {
        (&PN60, &QN60)
    };
    let (mut p, mut q, mut pow) = (0.0, 0.0, 1.0);
    for i in 0..6 {
        p += pn[i] * pow;
        q += qn[i] * pow;
        pow *= x;
    }
    q += qn[6] * pow;
    (-x * x).exp() * p / q
}

fn erfc_2654(x: f64) -> f64 {
    const PN: [f64; 5] = [
        5.641_895_835_477_560_78e-1,
        8.802_537_461_055_257_75,
        3.846_831_037_161_173_20e1,
        4.772_099_658_744_363_77e1,
        8.080_407_290_523_016_77,
    ];
    const QN: [f64; 5] = [
        1.0,
        1.610_209_142_058_690_03e1,
        7.548_435_056_659_547_43e1,
        1.121_238_708_010_260_15e2,
        3.739_975_701_450_408_50e1,
    ];
    let (mut p, mut q, mut pow) = (0.0, 0.0, 1.0);
    for i in 0..5 {
        p += PN[i] * pow;
        q += QN[i] * pow;
        pow /= x * x;
    }
    (-x * x).exp() * p / (x * q)
}

/// The error function (AOO `rtl::math::erf`).
pub(crate) fn erf(x: f64) -> f64 {
    if x == 0.0 {
        return 0.0;
    }
    let a = x.abs();
    let v = if a < 1.0e-10 {
        a * std::f64::consts::FRAC_2_SQRT_PI
    } else if a < 0.65 {
        erf_0065(a)
    } else {
        1.0 - erfc(a)
    };
    if x < 0.0 {
        -v
    } else {
        v
    }
}

/// The complementary error function (AOO `rtl::math::erfc`).
pub(crate) fn erfc(x: f64) -> f64 {
    if x == 0.0 {
        return 1.0;
    }
    let a = x.abs();
    let v = if a >= 0.65 {
        if a < 6.0 {
            erfc_0600(a)
        } else {
            erfc_2654(a)
        }
    } else {
        1.0 - erf(a)
    };
    if x < 0.0 {
        2.0 - v
    } else {
        v
    }
}

// ---------------------------------------------------------------------------
// Sparse column-major argument reader
// ---------------------------------------------------------------------------

/// One argument as a matrix: its shape and its non-empty cells in
/// column-major order (`index = col * rows + row`, ascending).
#[derive(Clone, Debug, Default)]
pub(crate) struct SparseData {
    /// Rows of the reference or array (1 for a scalar).
    pub rows: usize,
    /// Columns of the reference or array (1 for a scalar).
    pub cols: usize,
    /// `(column-major index, value)` of every non-empty cell, ascending.
    pub items: Vec<(usize, Value)>,
}

/// The numeric value of a stored cell (numbers and booleans), if any.
pub(crate) fn cell_number(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => Some(*n),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        _ => None,
    }
}

impl SparseData {
    /// Reads `arg` without interpreting it: references are visited sparsely
    /// through [`EvalContext::for_each_in_range`], arrays element by
    /// element, and a scalar (an error included) is a 1×1 matrix. A missing
    /// argument is an empty 1×1 matrix. A reference union is one column of
    /// its areas' cells in order (AOO's number-sequence iteration of
    /// `svRefList`), so paired functions see a shape mismatch. Empty cells
    /// are never stored.
    pub(crate) fn read(ctx: &mut dyn EvalContext, arg: EvalValue) -> Self {
        if let Some(areas) = arg.areas().filter(|a| a.len() != 1) {
            let mut items = Vec::new();
            let mut base = 0usize;
            for r in areas {
                let rows = r.rows() as usize;
                let mut part = Vec::new();
                ctx.for_each_in_range(r, true, &mut |addr, v| {
                    if !matches!(v, Value::Empty) {
                        let idx = (addr.col - r.c0) as usize * rows + (addr.row - r.r0) as usize;
                        part.push((base + idx, v.clone()));
                    }
                    ControlFlow::Continue(())
                });
                part.sort_by_key(|x| x.0);
                items.extend(part);
                base += r.cell_count() as usize;
            }
            return Self {
                rows: base.max(1),
                cols: 1,
                items,
            };
        }
        match arg {
            EvalValue::Missing => Self::scalar(Value::Empty),
            EvalValue::Scalar(Value::Array(m)) | EvalValue::Array(m) => {
                let (rows, cols) = (m.rows as usize, m.cols as usize);
                let mut items = Vec::new();
                for c in 0..m.cols {
                    for r in 0..m.rows {
                        match m.get(r, c) {
                            None | Some(Value::Empty) => {}
                            Some(v) => items.push((c as usize * rows + r as usize, v.clone())),
                        }
                    }
                }
                Self { rows, cols, items }
            }
            EvalValue::Scalar(v) => Self::scalar(v),
            EvalValue::Range(r) => {
                let rows = r.rows() as usize;
                let cols = r.cols() as usize;
                let mut items = Vec::new();
                ctx.for_each_in_range(&r, true, &mut |addr, v| {
                    if !matches!(v, Value::Empty) {
                        let idx = (addr.col - r.c0) as usize * rows + (addr.row - r.r0) as usize;
                        items.push((idx, v.clone()));
                    }
                    ControlFlow::Continue(())
                });
                items.sort_by_key(|x| x.0);
                Self { rows, cols, items }
            }
            #[allow(unreachable_patterns)]
            _ => Self::scalar(Value::Error(ErrorKind::Value)),
        }
    }

    /// A 1×1 matrix holding `v` (nothing if `v` is empty).
    pub(crate) fn scalar(v: Value) -> Self {
        Self {
            rows: 1,
            cols: 1,
            items: if matches!(v, Value::Empty) {
                Vec::new()
            } else {
                vec![(0, v)]
            },
        }
    }

    /// Numbers and booleans in column-major order; text is skipped, the
    /// first error (in that order) propagates.
    pub(crate) fn numbers(&self) -> Result<Vec<f64>, ErrorKind> {
        let mut out = Vec::with_capacity(self.items.len());
        for (_, v) in &self.items {
            if let Value::Error(e) = v {
                return Err(*e);
            }
            if let Some(n) = cell_number(v) {
                out.push(n);
            }
        }
        Ok(out)
    }

    /// All `rows * cols` cells as numbers, or `#VALUE!` if any is empty or
    /// not a number.
    pub(crate) fn dense_numbers(&self) -> Result<Vec<f64>, ErrorKind> {
        if self.items.len() != self.rows * self.cols {
            return Err(ErrorKind::Value);
        }
        self.items
            .iter()
            .map(|(_, v)| cell_number(v).ok_or(ErrorKind::Value))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::MockEvalContext;

    fn close(a: f64, b: f64, rel: f64) -> bool {
        (a - b).abs() <= rel * b.abs().max(1e-300)
    }

    #[test]
    fn normal_and_erf() {
        assert!(close(integral_phi(1.96), 0.975_002_104_851_779_5, 1e-15));
        assert!(close(integral_phi(1.0), 0.841_344_746_068_542_9, 1e-15));
        assert!(close(gaussinv(0.975), 1.959_963_984_540_054, 1e-15));
        assert!(close(gaussinv(1e-10), -6.361_340_902_404_056, 1e-14));
        assert!(close(erf(0.5), 0.520_499_877_813_046_5, 1e-15));
        assert!(close(erf(-0.5), -0.520_499_877_813_046_5, 1e-15));
        assert!(close(erfc(3.0), 2.209_049_699_858_544e-5, 1e-14));
        assert!(close(erfc(-1.0), 1.842_700_792_949_715, 1e-15));
        assert!(close(gauss(1.0) + 0.5, 0.841_344_746_068_542_9, 1e-13));
        assert!(close(phi(0.0), 0.398_942_280_401_432_7, 1e-15));
    }

    #[test]
    fn sparse_reader_is_column_major() {
        let mut c = MockEvalContext::new();
        c.set(0, "A1", 1.0).set(0, "B1", 2.0).set(0, "A2", "x");
        c.set(0, "B2", Value::Error(ErrorKind::Div0));
        let r = c.range(0, "A1:B2");
        let d = SparseData::read(&mut c, r);
        assert_eq!((d.rows, d.cols), (2, 2));
        let idx: Vec<usize> = d.items.iter().map(|x| x.0).collect();
        assert_eq!(idx, vec![0, 1, 2, 3]);
        assert_eq!(d.numbers(), Err(ErrorKind::Div0));
        let m = SparseData::read(&mut c, EvalValue::Missing);
        assert!(m.items.is_empty());
        assert_eq!(
            SparseData::scalar(Value::Number(3.0)).dense_numbers(),
            Ok(vec![3.0])
        );
    }
}
