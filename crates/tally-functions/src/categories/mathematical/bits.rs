//! BITAND, BITOR, BITXOR, BITLSHIFT, BITRSHIFT.
//!
//! Operands must be non-negative and below 2^48 (AOO Analysis add-in
//! `AnalysisAddIn::getBitand` & co.), otherwise `Err:502`. Non-integers are
//! truncated (the parameters are declared `integer`). A shift count whose
//! magnitude exceeds 53, or a left shift whose result reaches 2^48, is
//! `Err:502`; a negative shift reverses the direction.

use tally_types::{ErrorKind, Value};

use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const LIMIT: f64 = 281_474_976_710_656.0; // 2^48

const PAIR: &[Param] = &[
    Param::required("number1", ParamKind::Integer),
    Param::required("number2", ParamKind::Integer),
];
const SHIFT: &[Param] = &[
    Param::required("number", ParamKind::Integer),
    Param::required("shift", ParamKind::Integer),
];

/// `BITAND(number1; number2)`.
pub const BITAND: FunctionDef = FunctionDef::new("BITAND", Category::Mathematical, PAIR, bitand)
    .help("Returns the bitwise AND of two integers.");
/// `BITOR(number1; number2)`.
pub const BITOR: FunctionDef = FunctionDef::new("BITOR", Category::Mathematical, PAIR, bitor)
    .help("Returns the bitwise OR of two integers.");
/// `BITXOR(number1; number2)`.
pub const BITXOR: FunctionDef = FunctionDef::new("BITXOR", Category::Mathematical, PAIR, bitxor)
    .help("Returns the bitwise exclusive OR of two integers.");
/// `BITLSHIFT(number; shift)`.
pub const BITLSHIFT: FunctionDef =
    FunctionDef::new("BITLSHIFT", Category::Mathematical, SHIFT, bitlshift)
        .help("Shifts a number left by the given number of bits.");
/// `BITRSHIFT(number; shift)`.
pub const BITRSHIFT: FunctionDef =
    FunctionDef::new("BITRSHIFT", Category::Mathematical, SHIFT, bitrshift)
        .help("Shifts a number right by the given number of bits.");

/// Reads argument `i` as an integer in `0 ..< 2^48`.
fn operand(a: &mut Args, i: usize) -> Result<u64, ErrorKind> {
    let x = a.num(i)?;
    if !(0.0..LIMIT).contains(&x) {
        return Err(ErrorKind::InvalidArgument);
    }
    Ok(x.trunc() as u64)
}

fn binary(
    ctx: &mut dyn EvalContext,
    args: &mut dyn ArgList,
    op: impl FnOnce(u64, u64) -> u64,
) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let x = operand(&mut a, 0)?;
        let y = operand(&mut a, 1)?;
        Ok(Value::Number(op(x, y) as f64))
    })
}

fn bitand(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    binary(ctx, args, |x, y| x & y)
}

fn bitor(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    binary(ctx, args, |x, y| x | y)
}

fn bitxor(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    binary(ctx, args, |x, y| x ^ y)
}

/// Shifts `n` left by `shift` bits (right when negative), `Err:502` out of range.
fn shifted(n: u64, shift: i64) -> Result<Value, ErrorKind> {
    if shift.abs() > 53 {
        return Err(ErrorKind::InvalidArgument);
    }
    let r = if shift >= 0 {
        n << shift
    } else {
        n >> shift.unsigned_abs()
    };
    if r as f64 >= LIMIT {
        return Err(ErrorKind::InvalidArgument);
    }
    Ok(Value::Number(r as f64))
}

fn bitlshift(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let n = operand(&mut a, 0)?;
        let s = a.int(1)?;
        shifted(n, s)
    })
}

fn bitrshift(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let n = operand(&mut a, 0)?;
        let s = a.int(1)?;
        shifted(n, -s)
    })
}

pub(super) fn register(r: &mut Registry) {
    for d in [BITAND, BITOR, BITXOR, BITLSHIFT, BITRSHIFT] {
        r.register(d);
    }
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{ErrorKind, Value};

    fn f(name: &str, args: &[f64]) -> Value {
        call(
            &mut MockEvalContext::new(),
            name,
            args.iter().map(|x| EvalValue::number(*x)).collect(),
        )
    }
    fn num(x: f64) -> Value {
        Value::Number(x)
    }
    fn e502() -> Value {
        Value::Error(ErrorKind::InvalidArgument)
    }

    #[test]
    fn logic_ops() {
        assert_eq!(f("BITAND", &[12.0, 10.0]), num(8.0));
        assert_eq!(f("BITOR", &[12.0, 10.0]), num(14.0));
        assert_eq!(f("BITXOR", &[12.0, 10.0]), num(6.0));
        assert_eq!(f("BITAND", &[-1.0, 1.0]), e502());
        assert_eq!(f("BITOR", &[281_474_976_710_656.0, 1.0]), e502());
        assert_eq!(
            f("BITOR", &[281_474_976_710_655.0, 0.0]),
            num(281_474_976_710_655.0)
        );
        assert_eq!(f("BITAND", &[7.9, 3.0]), num(3.0));
    }

    #[test]
    fn shifts() {
        assert_eq!(f("BITLSHIFT", &[1.0, 4.0]), num(16.0));
        assert_eq!(f("BITRSHIFT", &[16.0, 4.0]), num(1.0));
        assert_eq!(f("BITLSHIFT", &[16.0, -4.0]), num(1.0));
        assert_eq!(f("BITRSHIFT", &[1.0, -4.0]), num(16.0));
        assert_eq!(f("BITLSHIFT", &[1.0, 48.0]), e502());
        assert_eq!(f("BITLSHIFT", &[1.0, 47.0]), num(140_737_488_355_328.0));
        assert_eq!(f("BITRSHIFT", &[1.0, 54.0]), e502());
        assert_eq!(f("BITRSHIFT", &[1.0, 1.0]), num(0.0));
    }
}
