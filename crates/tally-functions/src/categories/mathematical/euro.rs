//! CONVERT and EUROCONVERT: conversion between the euro and the legacy
//! euro-zone currencies at their irrevocably fixed rates.
//!
//! AOO's built-in `CONVERT` (ODF `ORG.OPENOFFICE.CONVERT`, not the Excel
//! compatible Analysis add-in `CONVERT_ADD`; `ScInterpreter::ScConvert`)
//! reads its table from the configuration (`Office.Calc/UnitConversion`),
//! which ships one `EUR -> currency` entry per legacy currency. It looks up
//! `from -> to`, then the reciprocal `to -> from`, case-sensitively
//! (`ScUnitConverterData::BuildIndexString`), and pushes `#N/A` when neither
//! exists: so only conversions between `EUR` and one legacy currency work,
//! and `EUR -> EUR` or `DEM -> FRF` are `#N/A`. It never rounds.
//!
//! `EUROCONVERT` (`ScInterpreter::ScEuroConvert`, `lclConvertMoney`): codes
//! match ASCII case-insensitively and an unknown code is `Err:502`; a given
//! `triangulation_precision` below 3 is `Err:502`. Unless the source is
//! `EUR`, the value goes through the euro and that intermediate is rounded
//! to `triangulation_precision` decimals when given; the result is rounded
//! to the decimals of the target currency unless `full_precision` is true.

use tally_types::{ErrorKind, Value};

use super::rounding::round_corrected;
use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

/// (ISO code, units per euro, decimals of the currency), as AOO's
/// `lclConvertMoney` table (PTE and GRD use 2 decimals there).
const CURRENCIES: &[(&str, f64, i32)] = &[
    ("EUR", 1.0, 2),
    ("ATS", 13.7603, 2),
    ("BEF", 40.3399, 0),
    ("CYP", 0.585274, 2),
    ("DEM", 1.95583, 2),
    ("EEK", 15.6466, 2),
    ("ESP", 166.386, 0),
    ("FIM", 5.94573, 2),
    ("FRF", 6.55957, 2),
    ("GRD", 340.750, 2),
    ("IEP", 0.787564, 2),
    ("ITL", 1936.27, 0),
    ("LUF", 40.3399, 0),
    ("MTL", 0.429300, 2),
    ("NLG", 2.20371, 2),
    ("PTE", 200.482, 2),
    ("SIT", 239.640, 2),
    ("SKK", 30.1260, 2),
];

const CONVERT_PARAMS: &[Param] = &[
    Param::required("value", ParamKind::Number),
    Param::required("from", ParamKind::Text),
    Param::required("to", ParamKind::Text),
];
const EURO_PARAMS: &[Param] = &[
    Param::required("value", ParamKind::Number),
    Param::required("from_currency", ParamKind::Text),
    Param::required("to_currency", ParamKind::Text),
    Param::optional("full_precision", ParamKind::Bool),
    Param::optional("triangulation_precision", ParamKind::Integer),
];

/// `CONVERT(value; from; to)`.
pub const CONVERT: FunctionDef =
    FunctionDef::new("CONVERT", Category::Mathematical, CONVERT_PARAMS, convert)
        .odf("ORG.OPENOFFICE.CONVERT")
        .help("Converts a value according to a conversion table in the configuration.");
/// `EUROCONVERT(value; from_currency; to_currency; [full_precision]; [triangulation_precision])`.
pub const EUROCONVERT: FunctionDef = FunctionDef::new(
    "EUROCONVERT",
    Category::Mathematical,
    EURO_PARAMS,
    euroconvert,
)
.help("Converts a value from one to another Euro currency.");

/// EUROCONVERT's lookup: ASCII case-insensitive, unknown is `Err:502`.
fn lookup(code: &str) -> Result<(f64, i32), ErrorKind> {
    CURRENCIES
        .iter()
        .find(|(c, _, _)| c.eq_ignore_ascii_case(code))
        .map(|(_, rate, dec)| (*rate, *dec))
        .ok_or(ErrorKind::InvalidArgument)
}

/// CONVERT's configuration table: the factor of the exact pair
/// `EUR -> code` (case-sensitive).
fn unit_factor(from: &str, to: &str) -> Option<f64> {
    if from != "EUR" || to == "EUR" {
        return None;
    }
    CURRENCIES
        .iter()
        .find(|(c, _, _)| *c == to)
        .map(|(_, rate, _)| *rate)
}

fn convert(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let v = a.num(0)?;
        let from = a.text(1)?;
        let to = a.text(2)?;
        if let Some(f) = unit_factor(&from, &to) {
            Ok(Value::Number(v * f))
        } else if let Some(f) = unit_factor(&to, &from) {
            Ok(Value::Number(v / f))
        } else {
            Err(ErrorKind::NA)
        }
    })
}

fn euroconvert(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let v = a.num(0)?;
        let from_code = a.text(1)?;
        let to_code = a.text(2)?;
        let full = a.bool_opt(3, false)?;
        let tri = if a.is_missing(4) {
            None
        } else {
            let p = a.num(4)?.floor();
            if p < 3.0 {
                return Err(ErrorKind::InvalidArgument);
            }
            Some(p.min(f64::from(i32::MAX)) as i32)
        };
        let (from, _) = lookup(&from_code)?;
        let (to, to_dec) = lookup(&to_code)?;
        if from_code.eq_ignore_ascii_case(&to_code) {
            return Ok(Value::Number(v));
        }
        let r = if from_code.eq_ignore_ascii_case("EUR") {
            v * to
        } else {
            let mut eur = v / from;
            if let Some(p) = tri {
                eur = round_corrected(eur, p);
            }
            eur * to
        };
        Ok(Value::Number(if full {
            r
        } else {
            round_corrected(r, to_dec)
        }))
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(CONVERT);
    r.register(EUROCONVERT);
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{ErrorKind, Value};

    fn t(s: &str) -> EvalValue {
        EvalValue::text(s)
    }
    fn n(x: f64) -> EvalValue {
        EvalValue::number(x)
    }

    #[test]
    fn convert_fixed_rates() {
        let mut c = MockEvalContext::new();
        let v = call(&mut c, "CONVERT", vec![n(1.0), t("EUR"), t("DEM")]);
        assert!(matches!(v, Value::Number(x) if (x - 1.95583).abs() < 1e-12));
        let v = call(&mut c, "CONVERT", vec![n(1.95583), t("DEM"), t("EUR")]);
        assert!(matches!(v, Value::Number(x) if (x - 1.0).abs() < 1e-12));
        for (from, to) in [
            ("eur", "DEM"),
            ("DEM", "FRF"),
            ("EUR", "EUR"),
            ("EUR", "XXX"),
        ] {
            assert_eq!(
                call(&mut c, "CONVERT", vec![n(1.0), t(from), t(to)]),
                Value::Error(ErrorKind::NA),
                "{from} -> {to}"
            );
        }
    }

    #[test]
    fn euroconvert_rounding() {
        let mut c = MockEvalContext::new();
        assert_eq!(
            call(&mut c, "EUROCONVERT", vec![n(100.0), t("DEM"), t("EUR")]),
            Value::Number(51.13)
        );
        assert_eq!(
            call(&mut c, "EUROCONVERT", vec![n(1.0), t("EUR"), t("ITL")]),
            Value::Number(1936.0)
        );
        let v = call(
            &mut c,
            "EUROCONVERT",
            vec![n(1.0), t("EUR"), t("ITL"), EvalValue::bool(true)],
        );
        assert!(matches!(v, Value::Number(x) if (x - 1936.27).abs() < 1e-9));
        // Triangulation DEM -> FRF through the euro.
        let v = call(
            &mut c,
            "EUROCONVERT",
            vec![n(100.0), t("DEM"), t("FRF"), EvalValue::bool(false), n(3.0)],
        );
        assert_eq!(v, Value::Number(335.38));
        assert_eq!(
            call(&mut c, "EUROCONVERT", vec![n(100.0), t("dem"), t("eur")]),
            Value::Number(51.13)
        );
        assert_eq!(
            call(
                &mut c,
                "EUROCONVERT",
                vec![n(1.0), t("DEM"), t("FRF"), EvalValue::bool(false), n(2.0)]
            ),
            Value::Error(ErrorKind::InvalidArgument)
        );
    }
}
