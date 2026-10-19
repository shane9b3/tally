//! BAHTTEXT: a number as Thai baht text. Follows the structure of
//! `ScInterpreter::ScBahtText` (AOO `sc/source/core/tool/interpr1.cxx`).

use tally_types::{round_to_15_digits, ErrorKind, Value};

use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry, ReturnKind};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const PARAMS: &[Param] = &[Param::required("number", ParamKind::Number)];

/// `BAHTTEXT(number)`.
pub const BAHTTEXT: FunctionDef = FunctionDef::new("BAHTTEXT", Category::Text, PARAMS, bahttext)
    .odf("COM.MICROSOFT.BAHTTEXT")
    .returns(ReturnKind::Text)
    .help("Converts a number to Thai baht text.");

const DIGITS: [&str; 10] = [
    "ศูนย์",
    "หนึ่ง",
    "สอง",
    "สาม",
    "สี่",
    "ห้า",
    "หก",
    "เจ็ด",
    "แปด",
    "เก้า",
];
/// Place names for positions 1..=5 (tens .. hundred-thousands).
const PLACES: [&str; 6] = ["", "สิบ", "ร้อย", "พัน", "หมื่น", "แสน"];
const MILLION: &str = "ล้าน";
const BAHT: &str = "บาท";
const SATANG: &str = "สตางค์";
const EXACT: &str = "ถ้วน";
const MINUS: &str = "ลบ";
const ET: &str = "เอ็ด";

/// Words for `0 < n < 1_000_000`. `has_higher` is true when a million-group
/// precedes (then a trailing 1 reads "et").
fn group(n: u64, has_higher: bool) -> String {
    let mut out = String::new();
    for pos in (0..6usize).rev() {
        let d = ((n / 10u64.pow(pos as u32)) % 10) as usize;
        if d == 0 {
            continue;
        }
        match pos {
            0 => {
                if d == 1 && (n > 1 || has_higher) {
                    out.push_str(ET);
                } else {
                    out.push_str(DIGITS[d]);
                }
            }
            1 => match d {
                1 => out.push_str(PLACES[1]),
                2 => {
                    out.push_str("ยี่");
                    out.push_str(PLACES[1]);
                }
                _ => {
                    out.push_str(DIGITS[d]);
                    out.push_str(PLACES[1]);
                }
            },
            _ => {
                out.push_str(DIGITS[d]);
                out.push_str(PLACES[pos]);
            }
        }
    }
    out
}

fn whole(n: u64) -> String {
    if n >= 1_000_000 {
        let mut s = whole(n / 1_000_000);
        s.push_str(MILLION);
        let rest = n % 1_000_000;
        if rest > 0 {
            s.push_str(&group(rest, true));
        }
        s
    } else {
        group(n, false)
    }
}

fn baht_text(x: f64) -> Result<String, ErrorKind> {
    let abs = x.abs();
    if abs >= 1e15 {
        return Err(ErrorKind::InvalidArgument);
    }
    let total = round_to_15_digits(abs * 100.0).round() as u64;
    let (baht, satang) = (total / 100, total % 100);
    if total == 0 {
        return Ok(format!("{}{BAHT}{EXACT}", DIGITS[0]));
    }
    let mut out = String::new();
    if x < 0.0 {
        out.push_str(MINUS);
    }
    if baht > 0 {
        out.push_str(&whole(baht));
        out.push_str(BAHT);
    }
    if satang == 0 {
        out.push_str(EXACT);
    } else {
        out.push_str(&group(satang, false));
        out.push_str(SATANG);
    }
    Ok(out)
}

fn bahttext(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| Ok(Value::text(baht_text(a.num(0)?)?)))
}

pub(super) fn register(r: &mut Registry) {
    r.register(BAHTTEXT);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::{call, MockEvalContext};

    fn b(x: f64) -> Value {
        call(
            &mut MockEvalContext::new(),
            "BAHTTEXT",
            vec![EvalValue::number(x)],
        )
    }

    #[test]
    fn words() {
        assert_eq!(b(0.0), Value::text("ศูนย์บาทถ้วน"));
        assert_eq!(b(1.0), Value::text("หนึ่งบาทถ้วน"));
        assert_eq!(b(11.0), Value::text("สิบเอ็ดบาทถ้วน"));
        assert_eq!(b(21.0), Value::text("ยี่สิบเอ็ดบาทถ้วน"));
        assert_eq!(b(1234.56), Value::text("หนึ่งพันสองร้อยสามสิบสี่บาทห้าสิบหกสตางค์"));
        assert_eq!(b(0.5), Value::text("ห้าสิบสตางค์"));
        assert_eq!(b(-2.0), Value::text("ลบสองบาทถ้วน"));
        assert_eq!(b(1_000_000.0), Value::text("หนึ่งล้านบาทถ้วน"));
        assert_eq!(b(1_000_001.0), Value::text("หนึ่งล้านเอ็ดบาทถ้วน"));
        assert_eq!(b(1e15), Value::Error(ErrorKind::InvalidArgument));
    }
}
