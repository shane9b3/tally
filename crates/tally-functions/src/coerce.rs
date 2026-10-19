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
/// - `ConvertUnambiguous` (default) and, until `tally-numfmt` lands,
///   `ConvertLocale`: [`parse_unambiguous`] or `#VALUE!` (the empty string
///   is `#VALUE!`).
pub fn text_to_number(s: &str, settings: &CalcSettings) -> Result<f64, ErrorKind> {
    match settings.text_to_number {
        TextToNumber::GenerateError => Err(ErrorKind::Value),
        TextToNumber::TreatAsZero => Ok(0.0),
        // TODO(WP4): ConvertLocale should use tally-numfmt's en-US input
        // recognizer (thousands separators, currency, US dates).
        TextToNumber::ConvertUnambiguous | TextToNumber::ConvertLocale => {
            parse_unambiguous(s, settings.date_epoch).ok_or(ErrorKind::Value)
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
    let s = s.trim_matches(|c: char| c.is_ascii_whitespace());
    if s.is_empty() {
        return None;
    }
    if let Some(n) = parse_plain_number(s) {
        return Some(n);
    }
    parse_iso_datetime(s, epoch)
}

fn parse_plain_number(s: &str) -> Option<f64> {
    let b = s.as_bytes();
    let mut i = 0;
    if matches!(b.first(), Some(b'+' | b'-')) {
        i += 1;
    }
    let int_start = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    let mut digits = i - int_start;
    if i < b.len() && b[i] == b'.' {
        i += 1;
        let frac_start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        digits += i - frac_start;
    }
    if digits == 0 {
        return None;
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        i += 1;
        if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
            i += 1;
        }
        let exp_start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == exp_start {
            return None;
        }
    }
    if i != b.len() {
        return None;
    }
    s.parse::<f64>().ok().filter(|n| n.is_finite())
}

fn digits_value(s: &str) -> Option<u32> {
    if s.is_empty() || !s.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// `hh:mm[:ss[.fff]]` as a fraction of a day; `max_hours` bounds the hour.
fn parse_iso_time(s: &str, max_hours: Option<u32>) -> Option<f64> {
    let mut parts = s.split(':');
    let h = parts.next()?;
    let m = parts.next()?;
    let sec = parts.next();
    if parts.next().is_some() || h.len() > 2 || m.len() != 2 {
        return None;
    }
    let h = digits_value(h)?;
    let m = digits_value(m)?;
    if m >= 60 || max_hours.is_some_and(|max| h > max) {
        return None;
    }
    let secs = match sec {
        None => 0.0,
        Some(sec) => {
            let (whole, frac) = match sec.split_once('.') {
                Some((w, f)) => (w, Some(f)),
                None => (sec, None),
            };
            if whole.len() != 2 {
                return None;
            }
            let w = digits_value(whole)?;
            if w >= 60 {
                return None;
            }
            let f = match frac {
                Some(f) => {
                    digits_value(f)?;
                    format!("0.{f}").parse::<f64>().ok()?
                }
                None => 0.0,
            };
            f64::from(w) + f
        }
    };
    Some((f64::from(h) * 3600.0 + f64::from(m) * 60.0 + secs) / 86_400.0)
}

fn parse_iso_datetime(s: &str, epoch: DateEpoch) -> Option<f64> {
    // Standalone time.
    if s.len() <= 12 && s.contains(':') && !s.contains('-') {
        return parse_iso_time(s, None);
    }
    let (date, time) = match s.find(['T', ' ']) {
        Some(pos) => (&s[..pos], Some(&s[pos + 1..])),
        None => (s, None),
    };
    let mut parts = date.split('-');
    let (y, m, d) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() || y.len() != 4 || m.len() != 2 || d.len() != 2 {
        return None;
    }
    let serial = date_to_serial(
        i64::from(digits_value(y)?),
        digits_value(m)?,
        digits_value(d)?,
        epoch,
    )?;
    match time {
        None => Some(serial),
        Some(t) => Some(serial + parse_iso_time(t, Some(23))?),
    }
}

// TODO(WP4): replace these date helpers with tally-numfmt's serial
// conversions once that crate lands; they exist only so ISO dates convert
// in WP3 core.

/// Days since 1970-01-01 in the proleptic Gregorian calendar
/// (Howard Hinnant's `days_from_civil`).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = i64::from(m);
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn days_in_month(y: i64, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => {
            let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
            if leap {
                29
            } else {
                28
            }
        }
    }
}

/// Serial number of a valid calendar date in `epoch`; `None` for invalid
/// dates and, in the 1900 epoch, dates before 1900-01-01.
fn date_to_serial(y: i64, m: u32, d: u32, epoch: DateEpoch) -> Option<f64> {
    if !(1..=12).contains(&m) || d == 0 || d > days_in_month(y, m) {
        return None;
    }
    let days = days_from_civil(y, m, d);
    let serial = match epoch {
        DateEpoch::D1899 => days - days_from_civil(1899, 12, 30),
        DateEpoch::D1904 => days - days_from_civil(1904, 1, 1),
        DateEpoch::D1900 => {
            if y < 1900 {
                return None;
            }
            // Excel counts a fictitious 1900-02-29 (serial 60).
            if days >= days_from_civil(1900, 3, 1) {
                days - days_from_civil(1899, 12, 30)
            } else {
                days - days_from_civil(1899, 12, 31)
            }
        }
    };
    Some(serial as f64)
}

/// Renders a number the way General format does in a text context
/// (CONCATENATE, `&`): rounded to 15 significant digits, shortest form,
/// decimal notation for `1e-4 <= |x| < 1e15`, otherwise scientific with a
/// two-digit signed exponent (`1E+20`, `1.5E-05`). `-0` renders as `0`.
///
/// TODO(WP4): switch to tally-numfmt's General formatter.
pub fn number_to_text(x: f64) -> String {
    if x.is_nan() {
        return ErrorKind::Num.display().into_owned();
    }
    if x.is_infinite() {
        return ErrorKind::Num.display().into_owned();
    }
    let r = round_to_15_digits(x);
    if r == 0.0 {
        return "0".to_owned();
    }
    let a = r.abs();
    if (1e-4..1e15).contains(&a) {
        return format!("{r}");
    }
    let s = format!("{r:e}");
    match s.split_once('e') {
        Some((mantissa, exp)) => {
            let (sign, digits) = match exp.strip_prefix('-') {
                Some(d) => ('-', d),
                None => ('+', exp),
            };
            format!("{mantissa}E{sign}{digits:0>2}")
        }
        None => s,
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_days() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
        assert_eq!(days_from_civil(1969, 12, 31), -1);
        assert_eq!(date_to_serial(1899, 12, 30, DateEpoch::D1899), Some(0.0));
        assert_eq!(date_to_serial(1900, 1, 1, DateEpoch::D1899), Some(2.0));
        assert_eq!(
            date_to_serial(2024, 1, 15, DateEpoch::D1899),
            Some(45_306.0)
        );
        assert_eq!(date_to_serial(1900, 1, 1, DateEpoch::D1900), Some(1.0));
        assert_eq!(date_to_serial(1900, 2, 28, DateEpoch::D1900), Some(59.0));
        assert_eq!(date_to_serial(1900, 3, 1, DateEpoch::D1900), Some(61.0));
        assert_eq!(date_to_serial(1899, 12, 31, DateEpoch::D1900), None);
        assert_eq!(date_to_serial(1904, 1, 1, DateEpoch::D1904), Some(0.0));
        assert_eq!(date_to_serial(2023, 2, 29, DateEpoch::D1899), None);
        assert_eq!(
            date_to_serial(2024, 2, 29, DateEpoch::D1899),
            Some(45_351.0)
        );
        assert_eq!(date_to_serial(2024, 13, 1, DateEpoch::D1899), None);
    }
}
