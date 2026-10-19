//! CONVERT (Analysis add-in; registered as `CONVERT_ADD`): measurement unit converter.
//!
//! Unit classes, names, metric prefixes (`y`..`Y`), binary prefixes
//! (`ki`..`Yi`) and the `^2`/`^3` area/volume forms follow Apache OpenOffice
//! `scaddins/source/analysis/analysishelper.cxx` (`ConvertDataList`,
//! Apache-2.0). Factors are the exact definitions of each unit rather than
//! AOO's 17-digit constants.

#![allow(clippy::inconsistent_digit_grouping)]

use tally_types::{ErrorKind, Value};

use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const PARAMS: &[Param] = &[
    Param::required("number", ParamKind::Number),
    Param::required("from_unit", ParamKind::Text),
    Param::required("to_unit", ParamKind::Text),
];

/// `CONVERT(number; from_unit; to_unit)`; the plain name belongs to the built-in CONVERT.
pub const CONVERT_ADD: FunctionDef =
    FunctionDef::new("CONVERT_ADD", Category::AddIn, PARAMS, convert)
        .odf("CONVERT")
        .help("Converts a number between measurement systems.");

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum Class {
    Mass,
    Length,
    Time,
    Pressure,
    Force,
    Energy,
    Power,
    Magnetism,
    Temperature,
    Volume,
    Area,
    Speed,
    Information,
}

#[derive(Debug)]
struct Unit {
    name: &'static str,
    /// Value of one unit in the base unit of its class.
    factor: f64,
    class: Class,
    /// Accepts metric prefixes.
    prefix: bool,
    /// Length unit that also exists as `name2`/`name3`/`name^2`/`name^3`.
    dim: bool,
}

const fn u(name: &'static str, factor: f64, class: Class, prefix: bool) -> Unit {
    Unit {
        name,
        factor,
        class,
        prefix,
        dim: false,
    }
}
const fn len(name: &'static str, factor: f64, prefix: bool) -> Unit {
    Unit {
        name,
        factor,
        class: Class::Length,
        prefix,
        dim: true,
    }
}

const INCH: f64 = 0.0254;
const LB: f64 = 453.592_37;
const GAL_US: f64 = 3.785_411_784e-3;

#[rustfmt::skip]
static UNITS: &[Unit] = &[
    // Mass (base: gram)
    u("g", 1.0, Class::Mass, true),
    u("sg", 14_593.902_937_206_4, Class::Mass, false),
    u("lbm", LB, Class::Mass, false),
    u("u", 1.660_538_86e-24, Class::Mass, true),
    u("ozm", LB / 16.0, Class::Mass, false),
    u("stone", LB * 14.0, Class::Mass, false),
    u("ton", LB * 2000.0, Class::Mass, false),
    u("grain", LB / 7000.0, Class::Mass, false),
    u("pweight", LB / 291.666_666_666_666_7, Class::Mass, false),
    u("hweight", LB * 112.0, Class::Mass, false),
    u("shweight", LB * 100.0, Class::Mass, false),
    u("cwt", LB * 100.0, Class::Mass, false),
    u("uk_cwt", LB * 112.0, Class::Mass, false),
    u("lcwt", LB * 112.0, Class::Mass, false),
    u("brton", LB * 2240.0, Class::Mass, false),
    u("uk_ton", LB * 2240.0, Class::Mass, false),
    u("LTON", LB * 2240.0, Class::Mass, false),
    // Length (base: metre)
    len("m", 1.0, true),
    len("mi", 1609.344, false),
    len("Nmi", 1852.0, false),
    len("in", INCH, false),
    len("ft", INCH * 12.0, false),
    len("yd", INCH * 36.0, false),
    len("ang", 1e-10, true),
    len("Pica", INCH / 72.0, false),
    len("picapt", INCH / 72.0, false),
    len("pica", INCH / 6.0, false),
    len("ly", 9.460_730_472_580_8e15, true),
    u("ell", 1.143, Class::Length, false),
    u("parsec", 3.085_677_581_491_367e16, Class::Length, true),
    u("pc", 3.085_677_581_491_367e16, Class::Length, true),
    u("lightyear", 9.460_730_472_580_8e15, Class::Length, true),
    u("survey_mi", 1609.347_218_694_437_4, Class::Length, false),
    // Time (base: second)
    u("yr", 31_557_600.0, Class::Time, false),
    u("day", 86_400.0, Class::Time, false),
    u("d", 86_400.0, Class::Time, false),
    u("hr", 3600.0, Class::Time, false),
    u("mn", 60.0, Class::Time, false),
    u("min", 60.0, Class::Time, false),
    u("sec", 1.0, Class::Time, true),
    u("s", 1.0, Class::Time, true),
    // Pressure (base: pascal)
    u("Pa", 1.0, Class::Pressure, true),
    u("p", 1.0, Class::Pressure, true),
    u("atm", 101_325.0, Class::Pressure, true),
    u("at", 101_325.0, Class::Pressure, true),
    u("mmHg", 101_325.0 / 760.0, Class::Pressure, true),
    u("Torr", 101_325.0 / 760.0, Class::Pressure, true),
    u("psi", 6_894.757_293_168_361, Class::Pressure, false),
    // Force (base: newton)
    u("N", 1.0, Class::Force, true),
    u("dyn", 1e-5, Class::Force, true),
    u("dy", 1e-5, Class::Force, true),
    u("lbf", 4.448_221_615_260_5, Class::Force, false),
    u("pond", 9.806_65e-3, Class::Force, true),
    // Energy (base: joule)
    u("J", 1.0, Class::Energy, true),
    u("e", 1e-7, Class::Energy, true),
    u("c", 4.184, Class::Energy, true),
    u("cal", 4.1868, Class::Energy, true),
    u("eV", 1.602_176_46e-19, Class::Energy, true),
    u("ev", 1.602_176_46e-19, Class::Energy, true),
    u("HPh", 2_684_519.537_696_172, Class::Energy, false),
    u("hh", 2_684_519.537_696_172, Class::Energy, false),
    u("Wh", 3600.0, Class::Energy, true),
    u("wh", 3600.0, Class::Energy, true),
    u("flb", 1.355_817_948_331_400_4, Class::Energy, false),
    u("BTU", 1055.055_852_62, Class::Energy, false),
    u("btu", 1055.055_852_62, Class::Energy, false),
    // Power (base: watt)
    u("W", 1.0, Class::Power, true),
    u("w", 1.0, Class::Power, true),
    u("HP", 745.699_871_582_270_2, Class::Power, false),
    u("h", 745.699_871_582_270_2, Class::Power, false),
    u("PS", 735.498_75, Class::Power, false),
    // Magnetism (base: tesla)
    u("T", 1.0, Class::Magnetism, true),
    u("ga", 1e-4, Class::Magnetism, true),
    // Temperature (base: kelvin; converted by formula)
    u("C", 1.0, Class::Temperature, false),
    u("cel", 1.0, Class::Temperature, false),
    u("F", 1.0, Class::Temperature, false),
    u("fah", 1.0, Class::Temperature, false),
    u("K", 1.0, Class::Temperature, true),
    u("kel", 1.0, Class::Temperature, true),
    u("Reau", 1.0, Class::Temperature, false),
    u("Rank", 1.0, Class::Temperature, false),
    // Volume (base: cubic metre)
    u("tsp", GAL_US / 768.0, Class::Volume, false),
    u("tspm", 5e-6, Class::Volume, false),
    u("tbs", GAL_US / 256.0, Class::Volume, false),
    u("oz", GAL_US / 128.0, Class::Volume, false),
    u("cup", GAL_US / 16.0, Class::Volume, false),
    u("pt", GAL_US / 8.0, Class::Volume, false),
    u("us_pt", GAL_US / 8.0, Class::Volume, false),
    u("uk_pt", 4.546_09e-3 / 8.0, Class::Volume, false),
    u("qt", GAL_US / 4.0, Class::Volume, false),
    u("uk_qt", 4.546_09e-3 / 4.0, Class::Volume, false),
    u("gal", GAL_US, Class::Volume, false),
    u("uk_gal", 4.546_09e-3, Class::Volume, false),
    u("l", 1e-3, Class::Volume, true),
    u("L", 1e-3, Class::Volume, true),
    u("lt", 1e-3, Class::Volume, true),
    u("barrel", 0.158_987_294_928, Class::Volume, false),
    u("bushel", 0.035_239_070_166_88, Class::Volume, false),
    u("regton", 2.831_684_659_2, Class::Volume, false),
    u("GRT", 2.831_684_659_2, Class::Volume, false),
    u("MTON", 1.132_673_863_68, Class::Volume, false),
    // Area (base: square metre)
    u("uk_acre", 4046.856_422_4, Class::Area, false),
    u("us_acre", 4046.872_609_874_251_4, Class::Area, false),
    u("ar", 100.0, Class::Area, true),
    u("Morgen", 2500.0, Class::Area, false),
    u("ha", 10_000.0, Class::Area, true),
    // Speed (base: metre per second)
    u("m/s", 1.0, Class::Speed, true),
    u("m/sec", 1.0, Class::Speed, true),
    u("m/h", 1.0 / 3600.0, Class::Speed, true),
    u("m/hr", 1.0 / 3600.0, Class::Speed, true),
    u("mph", 0.447_04, Class::Speed, false),
    u("kn", 1852.0 / 3600.0, Class::Speed, false),
    u("admkn", 1853.184 / 3600.0, Class::Speed, false),
    // Information (base: bit)
    u("bit", 1.0, Class::Information, true),
    u("byte", 8.0, Class::Information, true),
];

const METRIC: &[(&str, f64)] = &[
    ("Y", 1e24),
    ("Z", 1e21),
    ("E", 1e18),
    ("P", 1e15),
    ("T", 1e12),
    ("G", 1e9),
    ("M", 1e6),
    ("k", 1e3),
    ("h", 1e2),
    ("da", 1e1),
    ("e", 1e1),
    ("d", 1e-1),
    ("c", 1e-2),
    ("m", 1e-3),
    ("u", 1e-6),
    ("n", 1e-9),
    ("p", 1e-12),
    ("f", 1e-15),
    ("a", 1e-18),
    ("z", 1e-21),
    ("y", 1e-24),
];

const BINARY: &[(&str, u32)] = &[
    ("ki", 10),
    ("Mi", 20),
    ("Gi", 30),
    ("Ti", 40),
    ("Pi", 50),
    ("Ei", 60),
    ("Zi", 70),
    ("Yi", 80),
];

#[derive(Copy, Clone, Debug)]
struct Resolved {
    class: Class,
    factor: f64,
    /// Index into UNITS (for temperature kinds and the length dimension).
    unit: &'static Unit,
    /// Power applied to a length unit for `name2`/`name3`/`name^n` forms.
    power: i32,
}

fn exact(name: &str) -> Option<&'static Unit> {
    UNITS.iter().find(|x| x.name == name)
}

/// A unit with an optional metric/binary prefix, without dimension suffixes.
fn plain(name: &str) -> Option<(&'static Unit, f64)> {
    if let Some(unit) = exact(name) {
        return Some((unit, 1.0));
    }
    for (p, m) in BINARY {
        if let Some(rest) = name.strip_prefix(p) {
            if let Some(unit) = exact(rest) {
                if unit.class == Class::Information {
                    return Some((unit, 2f64.powi(*m as i32)));
                }
            }
        }
    }
    for (p, m) in METRIC {
        if let Some(rest) = name.strip_prefix(p) {
            if let Some(unit) = exact(rest) {
                if unit.prefix {
                    return Some((unit, *m));
                }
            }
        }
    }
    None
}

fn resolve(name: &str) -> Option<Resolved> {
    if let Some((unit, pre)) = plain(name) {
        return Some(Resolved {
            class: unit.class,
            factor: unit.factor * pre,
            unit,
            power: 1,
        });
    }
    // Area and volume forms of length units: m2, km^3, ...
    let (stem, power) = if let Some(s) = name.strip_suffix("^2") {
        (s, 2)
    } else if let Some(s) = name.strip_suffix("^3") {
        (s, 3)
    } else if let Some(s) = name.strip_suffix('2') {
        (s, 2)
    } else if let Some(s) = name.strip_suffix('3') {
        (s, 3)
    } else {
        return None;
    };
    let (unit, pre) = plain(stem)?;
    if !unit.dim {
        return None;
    }
    Some(Resolved {
        class: if power == 2 {
            Class::Area
        } else {
            Class::Volume
        },
        factor: (unit.factor * pre).powi(power),
        unit,
        power,
    })
}

/// Kelvin from a value in temperature unit `name` (already scaled by any prefix).
fn to_kelvin(x: f64, unit: &str) -> f64 {
    match unit {
        "C" | "cel" => x + 273.15,
        "F" | "fah" => (x - 32.0) * 5.0 / 9.0 + 273.15,
        "Reau" => x * 1.25 + 273.15,
        "Rank" => x * 5.0 / 9.0,
        _ => x,
    }
}

fn from_kelvin(k: f64, unit: &str) -> f64 {
    match unit {
        "C" | "cel" => k - 273.15,
        "F" | "fah" => (k - 273.15) * 9.0 / 5.0 + 32.0,
        "Reau" => (k - 273.15) * 0.8,
        "Rank" => k * 9.0 / 5.0,
        _ => k,
    }
}

/// Converts `x` between two unit names; `None` for unknown or incompatible units.
fn convert_units(x: f64, from: &str, to: &str) -> Option<f64> {
    let f = resolve(from)?;
    let t = resolve(to)?;
    if f.class != t.class {
        return None;
    }
    if f.class == Class::Temperature {
        // Only K takes a prefix; the factor is then the prefix itself.
        let k = to_kelvin(x * f.factor, f.unit.name);
        return Some(from_kelvin(k, t.unit.name) / t.factor);
    }
    let _ = (f.power, t.power);
    Some(x * f.factor / t.factor)
}

fn convert(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let x = a.num(0)?;
        let from = a.text(1)?;
        let to = a.text(2)?;
        convert_units(x, &from, &to)
            .map(Value::Number)
            .ok_or(ErrorKind::InvalidArgument)
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(CONVERT_ADD);
}

#[cfg(test)]
mod tests {
    use super::convert_units;
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{ErrorKind, Value};

    fn cv(x: f64, a: &str, b: &str) -> Option<f64> {
        convert_units(x, a, b)
    }
    fn close(got: Option<f64>, want: f64) {
        let g = got.unwrap_or(f64::NAN);
        assert!(((g - want) / want).abs() < 1e-12, "{g} vs {want}");
    }

    #[test]
    fn length_mass_time() {
        close(cv(1.0, "mi", "km"), 1.609344);
        close(cv(1.0, "ft", "in"), 12.0);
        close(cv(1.0, "lbm", "g"), 453.59237);
        close(cv(1.0, "kg", "lbm"), 2.2046226218487757);
        close(cv(1.0, "hr", "sec"), 3600.0);
        close(cv(1.0, "day", "mn"), 1440.0);
        close(cv(1.0, "ang", "nm"), 0.1);
    }

    #[test]
    fn temperature() {
        close(cv(100.0, "C", "F"), 212.0);
        close(cv(32.0, "F", "C").map(|v| v + 1.0), 1.0);
        close(cv(0.0, "C", "K"), 273.15);
        close(cv(491.67, "Rank", "K"), 273.15);
        close(cv(80.0, "Reau", "C"), 100.0);
        close(cv(1.0, "kK", "K"), 1000.0);
    }

    #[test]
    fn dimensions_and_prefixes() {
        close(cv(1.0, "m2", "cm2"), 10_000.0);
        close(cv(1.0, "m^2", "ft^2"), 10.763910416709722);
        close(cv(1.0, "l", "m3"), 1e-3);
        close(cv(1.0, "m3", "l"), 1000.0);
        close(cv(1.0, "gal", "l"), 3.785411784);
        close(cv(1.0, "ha", "m2"), 10_000.0);
        close(cv(1.0, "km/h", "m/s"), 1.0 / 3.6);
        close(cv(1.0, "mph", "km/h"), 1.609344);
        close(cv(1.0, "kbyte", "bit"), 8000.0);
        close(cv(1.0, "kibyte", "bit"), 8192.0);
        close(cv(1.0, "Mibit", "kibit"), 1024.0);
        close(cv(1.0, "J", "cal"), 1.0 / 4.1868);
        close(cv(1.0, "HP", "W"), 745.6998715822702);
        close(cv(1.0, "atm", "Pa"), 101325.0);
        close(cv(1.0, "min", "sec"), 60.0);
    }

    #[test]
    fn invalid() {
        assert_eq!(cv(1.0, "m", "kg"), None);
        assert_eq!(cv(1.0, "xx", "m"), None);
        assert_eq!(cv(1.0, "kft", "m"), None);
        assert_eq!(cv(1.0, "mi2", "m3"), None);
        assert_eq!(cv(1.0, "kibyte", "kg"), None);
        let mut c = MockEvalContext::new();
        let r = call(
            &mut c,
            "CONVERT_ADD",
            vec![
                EvalValue::number(1.0),
                EvalValue::text("m"),
                EvalValue::text("s"),
            ],
        );
        assert_eq!(r, Value::Error(ErrorKind::InvalidArgument));
    }
}
