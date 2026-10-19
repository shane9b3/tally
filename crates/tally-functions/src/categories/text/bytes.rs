//! LEFTB, RIGHTB, MIDB and LENB.
//!
//! Byte counts follow the double-byte model: East Asian wide and
//! full-width characters count as two bytes, everything else as one
//! (LibreOffice behaviour on a DBCS system; AOO 4.1 on a single-byte
//! locale makes these identical to LEFT/RIGHT/MID/LEN). A selection that
//! cuts a double-byte character in half yields a space for that half.

use tally_types::{ErrorKind, Value};

use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry, ReturnKind};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const LR_PARAMS: &[Param] = &[
    Param::required("text", ParamKind::Text),
    Param::optional("number", ParamKind::Integer),
];
const MID_PARAMS: &[Param] = &[
    Param::required("text", ParamKind::Text),
    Param::required("start", ParamKind::Integer),
    Param::required("number", ParamKind::Integer),
];
const LEN_PARAMS: &[Param] = &[Param::required("text", ParamKind::Text)];

/// `LEFTB(text [; number])`.
pub const LEFTB: FunctionDef = FunctionDef::new("LEFTB", Category::Text, LR_PARAMS, leftb)
    .returns(ReturnKind::Text)
    .help("Leading bytes of a text.");
/// `RIGHTB(text [; number])`.
pub const RIGHTB: FunctionDef = FunctionDef::new("RIGHTB", Category::Text, LR_PARAMS, rightb)
    .returns(ReturnKind::Text)
    .help("Trailing bytes of a text.");
/// `MIDB(text; start; number)`.
pub const MIDB: FunctionDef = FunctionDef::new("MIDB", Category::Text, MID_PARAMS, midb)
    .returns(ReturnKind::Text)
    .help("Bytes from the middle of a text.");
/// `LENB(text)`.
pub const LENB: FunctionDef = FunctionDef::new("LENB", Category::Text, LEN_PARAMS, lenb)
    .returns(ReturnKind::Number)
    .help("Length of a text in bytes.");

/// `true` for East Asian Wide / Fullwidth characters (two bytes).
fn is_double(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x115F
        | 0x2E80..=0x303E
        | 0x3041..=0x33FF
        | 0x3400..=0x4DBF
        | 0x4E00..=0xA4CF
        | 0xA960..=0xA97F
        | 0xAC00..=0xD7A3
        | 0xF900..=0xFAFF
        | 0xFE30..=0xFE6F
        | 0xFF01..=0xFF60
        | 0xFFE0..=0xFFE6
        | 0x1F300..=0x1F64F
        | 0x1F900..=0x1F9FF
        | 0x20000..=0x3FFFD)
}

#[derive(Copy, Clone)]
enum Slot {
    Single(char),
    First(char),
    Second,
}

fn slots(s: &str) -> Vec<Slot> {
    let mut v = Vec::with_capacity(s.len());
    for c in s.chars() {
        if is_double(c) {
            v.push(Slot::First(c));
            v.push(Slot::Second);
        } else {
            v.push(Slot::Single(c));
        }
    }
    v
}

fn collect(slots: &[Slot], from: usize, to: usize) -> String {
    let to = to.min(slots.len());
    let mut out = String::new();
    for (i, slot) in slots.iter().enumerate().take(to).skip(from) {
        match *slot {
            Slot::Single(c) => out.push(c),
            Slot::First(c) => {
                if i + 1 < to {
                    out.push(c);
                } else {
                    out.push(' ');
                }
            }
            Slot::Second => {
                if i == from {
                    out.push(' ');
                }
            }
        }
    }
    out
}

fn count(n: i64) -> Result<usize, ErrorKind> {
    usize::try_from(n).map_err(|_| ErrorKind::InvalidArgument)
}

fn leftb(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let s = a.text(0)?;
        let n = count(a.int_opt(1, 1)?)?;
        Ok(Value::text(collect(&slots(&s), 0, n)))
    })
}

fn rightb(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let s = a.text(0)?;
        let n = count(a.int_opt(1, 1)?)?;
        let sl = slots(&s);
        Ok(Value::text(collect(
            &sl,
            sl.len().saturating_sub(n),
            sl.len(),
        )))
    })
}

fn midb(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let s = a.text(0)?;
        let start = a.int(1)?;
        let n = count(a.int(2)?)?;
        if start < 1 {
            return Err(ErrorKind::InvalidArgument);
        }
        let from = count(start - 1)?;
        Ok(Value::text(collect(
            &slots(&s),
            from,
            from.saturating_add(n),
        )))
    })
}

fn lenb(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let s = a.text(0)?;
        Ok(Value::Number(
            s.chars()
                .map(|c| if is_double(c) { 2.0 } else { 1.0 })
                .sum(),
        ))
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(LEFTB);
    r.register(RIGHTB);
    r.register(MIDB);
    r.register(LENB);
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{ErrorKind, Value};

    fn t(name: &str, a: Vec<EvalValue>) -> Value {
        call(&mut MockEvalContext::new(), name, a)
    }
    fn s(x: &str) -> EvalValue {
        EvalValue::text(x)
    }
    fn n(x: f64) -> EvalValue {
        EvalValue::number(x)
    }

    #[test]
    fn ascii_behaves_like_char_functions() {
        assert_eq!(t("LENB", vec![s("hello")]), Value::Number(5.0));
        assert_eq!(t("LEFTB", vec![s("hello"), n(2.0)]), Value::text("he"));
        assert_eq!(t("RIGHTB", vec![s("hello")]), Value::text("o"));
        assert_eq!(
            t("MIDB", vec![s("hello"), n(2.0), n(3.0)]),
            Value::text("ell")
        );
    }

    #[test]
    fn double_byte() {
        assert_eq!(t("LENB", vec![s("日本a")]), Value::Number(5.0));
        assert_eq!(t("LEFTB", vec![s("日本a"), n(4.0)]), Value::text("日本"));
        assert_eq!(t("LEFTB", vec![s("日本a"), n(3.0)]), Value::text("日 "));
        assert_eq!(t("RIGHTB", vec![s("日本a"), n(2.0)]), Value::text(" a"));
        assert_eq!(
            t("MIDB", vec![s("日本a"), n(3.0), n(2.0)]),
            Value::text("本")
        );
        assert_eq!(
            t("MIDB", vec![s("日本a"), n(2.0), n(2.0)]),
            Value::text("  ")
        );
        assert_eq!(
            t("MIDB", vec![s("abc"), n(0.0), n(1.0)]),
            Value::Error(ErrorKind::InvalidArgument)
        );
    }
}
