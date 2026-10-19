//! All argument coercions, in one place (plan §3.7). Functions must not
//! re-implement any of these rules.
//!
//! # Scalar parameters
//!
//! | input | [`to_number`] | [`to_text`] | [`to_bool`] |
//! |---|---|---|---|
//! | missing / empty | 0 | `""` | `false` |
//! | number | itself | General format ([`number_to_text`]) | `n != 0` |
//! | bool | 1 / 0 | `TRUE` / `FALSE` | itself |
//! | text | per `settings.text_to_number` ([`text_to_number`]) | itself | `TRUE`/`FALSE` (any case), else `#VALUE!` |
//! | error | propagated | propagated | propagated |
//! | reference | implicit intersection with the origin ([`intersect`]), then the cell's value as above |||
//! | array | its top-left element (`#VALUE!` if empty) |||
//!
//! # Sequences (SUM-style)
//!
//! [`number_seq`]: direct scalar arguments are coerced like a scalar number
//! (so `SUM("3")` is 3 and `SUM("x")` is `#VALUE!` by default). Inside
//! references and arrays only numbers **and booleans** count; text and
//! empty cells are skipped; the first error (column-major order) propagates.
//!
//! **Booleans inside references count as 1/0.** AOO stores a boolean cell
//! as the number 1 or 0 with a boolean number format, so SUM, AVERAGE and
//! COUNT include them. Tally keeps a distinct [`Value::Bool`] but follows
//! that storage model here (this differs from Excel, which skips them).

use std::ops::ControlFlow;

use tally_types::{
    round_to_15_digits, CalcSettings, CellAddr, CellRange, DateEpoch, ErrorKind, Matrix,
    TextToNumber, Value,
};

use crate::context::{EvalContext, EvalValue};

/// Implicit intersection of `range` with the formula's `origin`.
///
/// A single cell is itself. A one-column range yields the cell in the
/// origin's row, a one-row range the cell in the origin's column (the sheet
/// of the origin is ignored, as in AOO). Anything else is `#VALUE!`.
pub fn intersect(range: &CellRange, origin: CellAddr) -> Result<CellAddr, ErrorKind> {
    if range.is_single() {
        return Ok(range.start());
    }
    if range.c0 == range.c1 && (range.r0..=range.r1).contains(&origin.row) {
        return Ok(CellAddr::new(range.sheet, origin.row, range.c0));
    }
    if range.r0 == range.r1 && (range.c0..=range.c1).contains(&origin.col) {
        return Ok(CellAddr::new(range.sheet, range.r0, origin.col));
    }
    Err(ErrorKind::Value)
}

/// The top-left element of a matrix (`#VALUE!` if the matrix is empty).
pub fn top_left(m: &Matrix) -> Result<Value, ErrorKind> {
    m.get(0, 0).cloned().ok_or(ErrorKind::Value)
}

/// Reduces an argument to one value without type coercion: missing →
/// `Empty`, reference → implicit intersection, array → top-left element.
/// Errors are returned as `Value::Error` (not propagated), so ISERROR-style
/// functions can inspect them.
pub fn scalar_value(arg: &EvalValue, ctx: &mut dyn EvalContext) -> Value {
    match arg {
        EvalValue::Missing => Value::Empty,
        EvalValue::Scalar(Value::Array(m)) | EvalValue::Array(m) => {
            top_left(m).unwrap_or_else(Value::Error)
        }
        EvalValue::Scalar(v) => v.clone(),
        EvalValue::Range(r) => match intersect(r, ctx.origin()) {
            Ok(addr) => match ctx.cell_value(addr) {
                Value::Array(m) => top_left(&m).unwrap_or_else(Value::Error),
                v => v,
            },
            Err(e) => Value::Error(e),
        },
    }
}

/// Coerces an argument to a number (see the module table).
pub fn to_number(arg: &EvalValue, ctx: &mut dyn EvalContext) -> Result<f64, ErrorKind> {
    let v = scalar_value(arg, ctx);
    value_to_number(&v, ctx.settings())
}

/// Coerces a plain value to a number (see the module table).
pub fn value_to_number(v: &Value, settings: &CalcSettings) -> Result<f64, ErrorKind> {
    match v {
        Value::Empty => Ok(0.0),
        Value::Number(n) => Ok(*n),
        Value::Bool(b) => Ok(bool_num(*b)),
        Value::Text(s) => text_to_number(s, settings),
        Value::Error(e) => Err(*e),
        Value::Array(m) => value_to_number(&top_left(m)?, settings),
    }
}

/// Coerces an argument to an integer: the number is snapped to 15
/// significant digits (so `2.9999999999999996` is 3) and truncated toward
/// zero. Non-finite values or values outside the `i32` range are
/// `Err:502` (AOO `GetInt32`).
pub fn to_integer(arg: &EvalValue, ctx: &mut dyn EvalContext) -> Result<i64, ErrorKind> {
    number_to_integer(to_number(arg, ctx)?)
}

/// The truncation rule of [`to_integer`] for an already-coerced number.
pub fn number_to_integer(x: f64) -> Result<i64, ErrorKind> {
    let t = round_to_15_digits(x).trunc();
    if !t.is_finite() || t.abs() > f64::from(i32::MAX) {
        return Err(ErrorKind::InvalidArgument);
    }
    Ok(t as i64)
}

/// AOO `rtl::math::approxFloor`: floor after snapping to 15 significant digits.
pub fn approx_floor(x: f64) -> f64 {
    round_to_15_digits(x).floor()
}

/// Coerces an argument to text (see the module table).
pub fn to_text(
    arg: &EvalValue,
    ctx: &mut dyn EvalContext,
) -> Result<std::sync::Arc<str>, ErrorKind> {
    let v = scalar_value(arg, ctx);
    value_to_text(&v)
}

/// Coerces a plain value to text (see the module table).
pub fn value_to_text(v: &Value) -> Result<std::sync::Arc<str>, ErrorKind> {
    match v {
        Value::Empty => Ok("".into()),
        Value::Number(n) => Ok(number_to_text(*n).into()),
        Value::Bool(b) => Ok(if *b { "TRUE" } else { "FALSE" }.into()),
        Value::Text(s) => Ok(s.clone()),
        Value::Error(e) => Err(*e),
        Value::Array(m) => value_to_text(&top_left(m)?),
    }
}

/// Coerces an argument to a boolean (see the module table).
pub fn to_bool(arg: &EvalValue, ctx: &mut dyn EvalContext) -> Result<bool, ErrorKind> {
    let v = scalar_value(arg, ctx);
    value_to_bool(&v)
}

/// Coerces a plain value to a boolean (see the module table).
pub fn value_to_bool(v: &Value) -> Result<bool, ErrorKind> {
    match v {
        Value::Empty => Ok(false),
        Value::Number(n) => Ok(*n != 0.0),
        Value::Bool(b) => Ok(*b),
        Value::Text(s) => {
            if s.eq_ignore_ascii_case("TRUE") {
                Ok(true)
            } else if s.eq_ignore_ascii_case("FALSE") {
                Ok(false)
            } else {
                Err(ErrorKind::Value)
            }
        }
        Value::Error(e) => Err(*e),
        Value::Array(m) => value_to_bool(&top_left(m)?),
    }
}

const fn bool_num(b: bool) -> f64 {
    if b {
        1.0
    } else {
        0.0
    }
}

/// Converts text used where a number is expected, per `settings.text_to_number`:
///
/// - `GenerateError`: always `#VALUE!`;
/// - `TreatAsZero`: always 0;
/// - `ConvertUnambiguous` (default): [`parse_unambiguous`] or `#VALUE!`;
/// - `ConvertLocale`: tally-numfmt's en-US input recognizer
///   ([`tally_numfmt::recognize_input_with`]: thousands separators,
///   currency, percent, fractions, US dates and times; `TRUE`/`FALSE` are
///   1/0), `#VALUE!` if it recognises no number.
///
/// The empty string is `#VALUE!` in both converting modes.
pub fn text_to_number(s: &str, settings: &CalcSettings) -> Result<f64, ErrorKind> {
    match settings.text_to_number {
        TextToNumber::GenerateError => Err(ErrorKind::Value),
        TextToNumber::TreatAsZero => Ok(0.0),
        TextToNumber::ConvertUnambiguous => {
            parse_unambiguous(s, settings.date_epoch).ok_or(ErrorKind::Value)
        }
        TextToNumber::ConvertLocale => {
            let ctx = tally_numfmt::RecognizeContext::now(settings.date_epoch);
            match tally_numfmt::recognize_input_with(s, &tally_numfmt::EN_US, &ctx).value {
                Value::Number(n) => Ok(n),
                Value::Bool(b) => Ok(bool_num(b)),
                _ => Err(ErrorKind::Value),
            }
        }
    }
}

/// Parses text that is unambiguously a number, ignoring surrounding ASCII
/// whitespace:
///
/// - plain decimals with optional sign, fraction and exponent: `12`,
///   `-1.5`, `.5`, `1e3`, `2.5E-2` (no thousands separators, no `inf`/`nan`);
/// - ISO 8601 extended dates `YYYY-MM-DD`, date-times
///   `YYYY-MM-DDThh:mm[:ss[.fff]]` (`T` or a space) and times
///   `hh:mm[:ss[.fff]]`, converted to serials in `epoch`.
///
/// Returns `None` for anything else.
pub fn parse_unambiguous(s: &str, epoch: DateEpoch) -> Option<f64> {
    tally_numfmt::parse_number_unambiguous_with(s, epoch)
}

/// Renders a number the way General format does in a text context
/// (CONCATENATE, `&`): tally-numfmt's [`tally_numfmt::general_string`]
/// without a width, en-US: rounded to 15 significant digits, shortest
/// form, decimal notation for `1e-4 <= |x| < 1e15`, otherwise scientific
/// with a two-digit signed exponent (`1E+20`, `1.5E-05`). `-0` renders as
/// `0`; NaN and infinities as `#NUM!`.
pub fn number_to_text(x: f64) -> String {
    tally_numfmt::general_string(x, None, &tally_numfmt::EN_US)
}

/// Where a value visited by [`values_seq`] came from.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum SeqSource {
    /// A direct argument (`COUNTA(1; "x")`), including missing ones (as `Empty`).
    Direct,
    /// A cell of a reference argument (empty cells are not visited).
    Reference,
    /// An element of an array argument (empty elements are not visited).
    Array,
}

/// Visits every value of one argument. Shared by the `*_seq` helpers.
fn visit(
    arg: &EvalValue,
    ctx: &mut dyn EvalContext,
    f: &mut dyn FnMut(&Value, SeqSource) -> ControlFlow<ErrorKind>,
) -> Result<(), ErrorKind> {
    let flow = match arg {
        EvalValue::Missing => f(&Value::Empty, SeqSource::Direct),
        EvalValue::Scalar(Value::Array(m)) | EvalValue::Array(m) => visit_matrix(m, f),
        EvalValue::Scalar(v) => f(v, SeqSource::Direct),
        EvalValue::Range(r) => {
            let mut out = ControlFlow::Continue(());
            ctx.for_each_in_range(r, true, &mut |_, v| match f(v, SeqSource::Reference) {
                ControlFlow::Continue(()) => ControlFlow::Continue(()),
                ControlFlow::Break(e) => {
                    out = ControlFlow::Break(e);
                    ControlFlow::Break(())
                }
            });
            out
        }
    };
    match flow {
        ControlFlow::Continue(()) => Ok(()),
        ControlFlow::Break(e) => Err(e),
    }
}

/// Column-major, like AOO's matrix iteration.
fn visit_matrix(
    m: &Matrix,
    f: &mut dyn FnMut(&Value, SeqSource) -> ControlFlow<ErrorKind>,
) -> ControlFlow<ErrorKind> {
    for c in 0..m.cols {
        for r in 0..m.rows {
            match m.get(r, c) {
                None | Some(Value::Empty) => {}
                Some(v) => f(v, SeqSource::Array)?,
            }
        }
    }
    ControlFlow::Continue(())
}

/// SUM semantics: calls `f` for each number of `arg` (see the module docs).
/// Missing direct arguments count as 0 (`SUM(1;)`, like Excel).
pub fn number_seq(
    arg: &EvalValue,
    ctx: &mut dyn EvalContext,
    mut f: impl FnMut(f64),
) -> Result<(), ErrorKind> {
    let settings = ctx.settings().clone();
    visit(arg, ctx, &mut |v, src| {
        match (v, src) {
            (Value::Number(n), _) => f(*n),
            (Value::Bool(b), _) => f(bool_num(*b)),
            (Value::Error(e), _) => return ControlFlow::Break(*e),
            (Value::Empty, SeqSource::Direct) => f(0.0),
            (Value::Text(s), SeqSource::Direct) => match text_to_number(s, &settings) {
                Ok(n) => f(n),
                Err(e) => return ControlFlow::Break(e),
            },
            _ => {}
        }
        ControlFlow::Continue(())
    })
}

/// AVERAGEA semantics: like [`number_seq`], but text inside references and
/// arrays counts as 0 instead of being skipped.
pub fn number_seq_with_text_as_zero(
    arg: &EvalValue,
    ctx: &mut dyn EvalContext,
    mut f: impl FnMut(f64),
) -> Result<(), ErrorKind> {
    let settings = ctx.settings().clone();
    visit(arg, ctx, &mut |v, src| {
        match (v, src) {
            (Value::Number(n), _) => f(*n),
            (Value::Bool(b), _) => f(bool_num(*b)),
            (Value::Error(e), _) => return ControlFlow::Break(*e),
            (Value::Empty, SeqSource::Direct) => f(0.0),
            (Value::Text(s), SeqSource::Direct) => match text_to_number(s, &settings) {
                Ok(n) => f(n),
                Err(e) => return ControlFlow::Break(e),
            },
            (Value::Text(_), _) => f(0.0),
            _ => {}
        }
        ControlFlow::Continue(())
    })
}

/// COUNTA-style: calls `f` for every value of `arg` without coercion or
/// error propagation. Empty cells/elements inside references and arrays are
/// skipped; direct arguments are always visited (a missing one as `Empty`).
pub fn values_seq(
    arg: &EvalValue,
    ctx: &mut dyn EvalContext,
    mut f: impl FnMut(&Value, SeqSource),
) {
    // `visit` only fails when the callback breaks, which this one never does.
    let _ = visit(arg, ctx, &mut |v, src| {
        f(v, src);
        ControlFlow::Continue(())
    });
}
