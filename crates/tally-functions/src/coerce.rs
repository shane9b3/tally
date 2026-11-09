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
//! | text | per `settings.text_to_number` ([`text_to_number_in`], document locale) | itself | `TRUE`/`FALSE` (any case), else `#VALUE!` |
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
//!
//! # Unions
//!
//! A reference union ([`EvalValue::areas`] with several areas) is a
//! sequence of its areas in formula order for the `*_seq` helpers (AOO
//! iterates `svRefList` the same way); as a scalar argument it is
//! `#VALUE!`, like any multi-cell reference that cannot be intersected.
//!
//! # 3D references
//!
//! A 3D reference ([`EvalValue::Span3d`]) is a sequence of its per-sheet
//! areas in tab order (AOO's `ScValueIterator` walks `Tab1..=Tab2`); as a
//! scalar it intersects on the formula's own sheet ([`intersect_3d`]).
//!
//! # Locale
//!
//! [`locale_of`] resolves [`EvalContext::locale_tag`] to the
//! [`tally_numfmt::Locale`] whose input recognizer and formatter AOO's
//! `pFormatter` stands for: the `ConvertLocale` rule, criteria operands,
//! VALUE, DATEVALUE, TIMEVALUE, TEXT, FIXED, DOLLAR and NUMBERVALUE
//! defaults follow the document locale, not en-US.

use std::ops::ControlFlow;

use tally_numfmt::{Locale, EN_US};
use tally_types::{
    round_to_15_digits, CalcSettings, CellAddr, CellRange, DateEpoch, ErrorKind, Matrix,
    TextToNumber, Value,
};

use crate::context::{EvalContext, EvalValue};

/// The document locale of `ctx` ([`EvalContext::locale_tag`]), en-US when
/// the tag is not a built-in locale (the engine reports that case itself).
pub fn locale_of(ctx: &dyn EvalContext) -> &'static Locale {
    tally_numfmt::locale_by_tag(ctx.locale_tag()).unwrap_or(&EN_US)
}

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

/// Implicit intersection of a 3D reference (AOO
/// `ScInterpreter::DoubleRefToPosSingleRef` with `Tab1 != Tab2`). `areas`
/// are the per-sheet areas of an [`EvalValue::Span3d`] (same rectangle).
///
/// A one-row rectangle yields the cell in the origin's column, a
/// one-column rectangle the cell in the origin's row (a single cell needs
/// either, unlike a 2D single cell); the sheet is the formula's own sheet,
/// which must be one of the spanned sheets. Anything else is `#VALUE!`
/// (`errNoValue`).
pub fn intersect_3d(areas: &[CellRange], origin: CellAddr) -> Result<CellAddr, ErrorKind> {
    let Some(r) = areas.first() else {
        return Err(ErrorKind::Value);
    };
    let (row, col) = if (r.c0..=r.c1).contains(&origin.col) {
        if r.r0 != r.r1 {
            return Err(ErrorKind::Value);
        }
        (r.r0, origin.col)
    } else if (r.r0..=r.r1).contains(&origin.row) {
        if r.c0 != r.c1 {
            return Err(ErrorKind::Value);
        }
        (origin.row, r.c0)
    } else {
        return Err(ErrorKind::Value);
    };
    if !areas.iter().any(|a| a.sheet == origin.sheet) {
        return Err(ErrorKind::Value);
    }
    Ok(CellAddr::new(origin.sheet, row, col))
}

/// The value of a 3D reference from `first` to `last` over the rectangle
/// `area` (its sheet is ignored): one area per sheet between the two in
/// tab order, either order ([`EvalValue::span_3d`]). `#REF!` if either
/// sheet is unknown to `ctx`.
pub fn span_3d_value(
    ctx: &dyn EvalContext,
    first: tally_types::SheetId,
    last: tally_types::SheetId,
    area: CellRange,
) -> EvalValue {
    let (Some(a), Some(b)) = (ctx.sheet_index(first), ctx.sheet_index(last)) else {
        return EvalValue::error(ErrorKind::Ref);
    };
    let mut areas = Vec::with_capacity((a.abs_diff(b) + 1) as usize);
    for i in a.min(b)..=a.max(b) {
        match ctx.sheet_at(i) {
            Some(sheet) => areas.push(CellRange { sheet, ..area }),
            None => return EvalValue::error(ErrorKind::Ref),
        }
    }
    EvalValue::span_3d(areas)
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
        // A 3D reference intersects on the formula's own sheet.
        #[allow(unreachable_patterns)]
        v if v.is_3d() => match intersect_3d(v.areas().unwrap_or_default(), ctx.origin()) {
            Ok(addr) => match ctx.cell_value(addr) {
                Value::Array(m) => top_left(&m).unwrap_or_else(Value::Error),
                v => v,
            },
            Err(e) => Value::Error(e),
        },
        // A union has no single cell to intersect with.
        #[allow(unreachable_patterns)]
        _ => Value::Error(ErrorKind::Value),
    }
}

/// Coerces an argument to a number (see the module table), with the
/// document locale for the `ConvertLocale` rule.
pub fn to_number(arg: &EvalValue, ctx: &mut dyn EvalContext) -> Result<f64, ErrorKind> {
    let v = scalar_value(arg, ctx);
    value_to_number_in(&v, ctx.settings(), locale_of(ctx))
}

/// Coerces a plain value to a number (see the module table) with the en-US
/// locale; [`value_to_number_in`] takes the document locale.
pub fn value_to_number(v: &Value, settings: &CalcSettings) -> Result<f64, ErrorKind> {
    value_to_number_in(v, settings, &EN_US)
}

/// Coerces a plain value to a number (see the module table); `locale` is
/// used by the `ConvertLocale` rule.
pub fn value_to_number_in(
    v: &Value,
    settings: &CalcSettings,
    locale: &Locale,
) -> Result<f64, ErrorKind> {
    match v {
        Value::Empty => Ok(0.0),
        Value::Number(n) => Ok(*n),
        Value::Bool(b) => Ok(bool_num(*b)),
        Value::Text(s) => text_to_number_in(s, settings, locale),
        Value::Error(e) => Err(*e),
        Value::Array(m) => value_to_number_in(&top_left(m)?, settings, locale),
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
/// - `ConvertUnambiguous` (default): [`convert_string_to_value`] (integers
///   with an optional exponent and ISO 8601 dates/times only, so `"1.5"` is
///   `#VALUE!`) or `#VALUE!`;
/// - `ConvertLocale`: tally-numfmt's input recognizer for the document
///   locale ([`tally_numfmt::recognize_input_with`]: thousands separators,
///   currency, percent, fractions, dates and times in the locale's
///   spelling; `TRUE`/`FALSE` and the localized names are 1/0), `#VALUE!`
///   if it recognises no number.
///
/// The empty string is `#VALUE!` in both converting modes.
///
/// This form uses en-US; [`text_to_number_in`] takes the document locale.
pub fn text_to_number(s: &str, settings: &CalcSettings) -> Result<f64, ErrorKind> {
    text_to_number_in(s, settings, &EN_US)
}

/// [`text_to_number`] with the document locale for the `ConvertLocale` rule
/// (under de-DE `"1,5"` is 1.5 and `"19.10.2026"` a date).
pub fn text_to_number_in(
    s: &str,
    settings: &CalcSettings,
    locale: &Locale,
) -> Result<f64, ErrorKind> {
    match settings.text_to_number {
        TextToNumber::GenerateError => Err(ErrorKind::Value),
        TextToNumber::TreatAsZero => Ok(0.0),
        TextToNumber::ConvertUnambiguous => {
            convert_string_to_value(s, settings.date_epoch).ok_or(ErrorKind::Value)
        }
        TextToNumber::ConvertLocale => {
            let ctx = tally_numfmt::RecognizeContext::for_settings(settings);
            match tally_numfmt::recognize_input_with(s, locale, &ctx).value {
                Value::Number(n) => Ok(n),
                Value::Bool(b) => Ok(bool_num(b)),
                _ => Err(ErrorKind::Value),
            }
        }
    }
}

/// The number a text names in `locale`, as AOO's `IsNumberFormat` reads
/// it: the locale's input recognizer first (`"1,5"`, `"19.10.2026"`,
/// `"1.000"` and `"50%"` under de-DE; `"$5"`, `"10/19/2026"` under
/// en-US), then [`parse_unambiguous`] (plain decimals and ISO dates in any
/// locale). `None` if neither recognises a number. Shared by the criteria
/// parser, VALUE, DATEVALUE and TIMEVALUE; `now` supplies the year of
/// year-less dates.
pub fn recognize_number(
    s: &str,
    locale: &Locale,
    now: &tally_numfmt::RecognizeContext,
) -> Option<f64> {
    match tally_numfmt::recognize_input_with(s, locale, now).value {
        Value::Number(n) => Some(n),
        _ => parse_unambiguous(s, now.epoch),
    }
}

/// Port of AOO `ScInterpreter::ConvertStringToValue` (`interpr4.cxx`), the
/// `ConvertUnambiguous` text-to-number rule (LibreOffice: "convert only
/// integer numbers including exponents, and ISO 8601 dates and times"):
///
/// - an integer with optional sign and exponent: `3`, `-2E3`, `1e+2`
///   (no decimal separator, so `1.5` and `.5` are not converted);
/// - ISO 8601 extended dates `CCYY-MM-DD`, optionally followed by `T` or
///   one blank and a time; times `hh:mm`, `hh:mm:ss`, `hh:mm:ss.s` or
///   `hh:mm:ss,s` (hours unlimited without a date, at most 23 with one;
///   minutes and seconds at most 59). A date must be a valid calendar date
///   (a day `00` reads as 1, as in AOO); no sign before a date or time.
/// - leading whitespace and trailing blanks are ignored.
///
/// Returns `None` for anything else, including the empty string (Tally,
/// like LibreOffice's default, does not treat `""` as 0). The fraction of
/// seconds is read as written (`.05` is 0.05 s; AOO 4.1 drops leading zeros
/// of the fraction, a bug).
pub fn convert_string_to_value(s: &str, epoch: DateEpoch) -> Option<f64> {
    let b = s.as_bytes();
    let len = b.len();
    let mut i = 0;
    while i < len && b[i].is_ascii_whitespace() {
        i += 1;
    }
    let start = i;
    if i < len && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
    }
    let digits = i;
    while i < len && b[i].is_ascii_digit() {
        i += 1;
    }
    if i == digits {
        return None;
    }
    let int_end = i;
    let mut end = i;
    if i < len && (b[i] == b'e' || b[i] == b'E') {
        let mut j = i + 1;
        if j < len && (b[j] == b'+' || b[j] == b'-') {
            j += 1;
        }
        let ed = j;
        while j < len && b[j].is_ascii_digit() {
            j += 1;
        }
        if j > ed {
            end = j;
        }
    }
    let value: f64 = s[start..end].parse().ok()?;
    if !value.is_finite() {
        return None;
    }
    if end == len {
        return Some(value);
    }
    match b[end] {
        b' ' => b[end..].iter().all(|&c| c == b' ').then_some(value),
        b'-' | b':' if end == int_end && start == digits => {
            iso_date_time(s, start, end, b[end] == b'-', epoch)
        }
        _ => None,
    }
}

/// The date/time part of [`convert_string_to_value`]: `s[start..end]` is
/// the leading unsigned integer (year or hour) and `s[end]` its delimiter.
fn iso_date_time(s: &str, start: usize, end: usize, date: bool, epoch: DateEpoch) -> Option<f64> {
    const YEAR: usize = 0;
    const MONTH: usize = 1;
    const DAY: usize = 2;
    const HOUR: usize = 3;
    const MINUTE: usize = 4;
    const SECOND: usize = 5;
    const FRACTION: usize = 6;
    const DONE: usize = 7;
    const BLANK: usize = 8;
    const LIMIT: [i64; DONE] = [0, 12, 31, 0, 59, 59, 0];
    let b = s.as_bytes();
    let len = b.len();
    let mut unit = [0i64; DONE];
    let mut frac_digits = "";
    let mut state = if date { MONTH } else { MINUTE };
    unit[state - 1] = s[start..end].parse().ok()?;
    let mut p = end + 1;
    let mut last = p;
    let mut store = |state: usize, text: &'_ str| -> Option<()> {
        if state == FRACTION {
            return Some(());
        }
        let v: i64 = text.parse().ok()?;
        if LIMIT[state] != 0 && LIMIT[state] < v {
            return None;
        }
        unit[state] = v;
        Some(())
    };
    while p < len && state < BLANK {
        let c = b[p];
        if c.is_ascii_digit() {
            // At most two digits per unit, except the fraction.
            if p - last >= 2 && state != FRACTION {
                return None;
            }
        } else if p > last {
            if state < DONE {
                if state == FRACTION {
                    frac_digits = &s[last..p];
                }
                store(state, &s[last..p])?;
            }
            last = p + 1;
            let at_end = p + 1 == len;
            match state {
                MONTH if c != b'-' || at_end => return None,
                DAY if (c != b'T' || at_end) && c != b' ' => return None,
                HOUR if c != b':' || at_end => return None,
                MINUTE | SECOND => {
                    let sep_ok = if state == MINUTE {
                        c == b':'
                    } else {
                        c == b',' || c == b'.'
                    };
                    if (!sep_ok || at_end) && c != b' ' {
                        return None;
                    }
                    if c == b' ' {
                        state = DONE;
                    }
                }
                FRACTION => state = DONE,
                MONTH | DAY | HOUR => {}
                _ => return None,
            }
            state += 1;
        } else {
            return None;
        }
        p += 1;
    }
    if state == BLANK {
        if b[p..].iter().any(|&c| c != b' ') {
            return None;
        }
        state = BLANK + 1;
    }
    // Month without day, or hour without minute.
    if state == MONTH
        || (state == DAY && p <= last)
        || state == HOUR
        || (state == MINUTE && p <= last)
    {
        return None;
    }
    if p > last && state < DONE {
        if state == FRACTION {
            frac_digits = &s[last..p];
        }
        store(state, &s[last..p])?;
    }
    if date && unit[HOUR] > 23 {
        return None;
    }
    if date && unit[DAY] == 0 {
        unit[DAY] = 1;
    }
    let fraction = if frac_digits.is_empty() {
        0.0
    } else {
        format!("0.{frac_digits}").parse::<f64>().ok()?
    };
    let mut v = if date {
        tally_numfmt::date::serial_from_ymd(
            epoch,
            i32::try_from(unit[YEAR]).ok()?,
            u32::try_from(unit[MONTH]).ok()?,
            u32::try_from(unit[DAY]).ok()?,
        )?
    } else {
        0.0
    };
    v += ((unit[HOUR] * 3600 + unit[MINUTE] * 60 + unit[SECOND]) as f64 + fraction) / 86400.0;
    Some(v)
}

/// Parses text that is unambiguously a number in any locale, ignoring
/// surrounding ASCII whitespace (used where AOO consults the number
/// formatter, e.g. criteria operands and COUNT's direct text arguments;
/// arithmetic conversion uses the stricter [`convert_string_to_value`]):
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
        // A range, or each area of a union in turn.
        other => {
            let mut out = ControlFlow::Continue(());
            for r in other.areas().unwrap_or_default() {
                ctx.for_each_in_range(r, true, &mut |_, v| match f(v, SeqSource::Reference) {
                    ControlFlow::Continue(()) => ControlFlow::Continue(()),
                    ControlFlow::Break(e) => {
                        out = ControlFlow::Break(e);
                        ControlFlow::Break(())
                    }
                });
                if out.is_break() {
                    break;
                }
            }
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
    let locale = locale_of(ctx);
    visit(arg, ctx, &mut |v, src| {
        match (v, src) {
            (Value::Number(n), _) => f(*n),
            (Value::Bool(b), _) => f(bool_num(*b)),
            (Value::Error(e), _) => return ControlFlow::Break(*e),
            (Value::Empty, SeqSource::Direct) => f(0.0),
            (Value::Text(s), SeqSource::Direct) => match text_to_number_in(s, &settings, locale) {
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
    let locale = locale_of(ctx);
    visit(arg, ctx, &mut |v, src| {
        match (v, src) {
            (Value::Number(n), _) => f(*n),
            (Value::Bool(b), _) => f(bool_num(*b)),
            (Value::Error(e), _) => return ControlFlow::Break(*e),
            (Value::Empty, SeqSource::Direct) => f(0.0),
            (Value::Text(s), SeqSource::Direct) => match text_to_number_in(s, &settings, locale) {
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
