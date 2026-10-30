//! Formatting values with a parsed format code ([`format_value`]).
//!
//! Semantics follow OpenOffice's `SvNumberformat` (`zformat.cxx`):
//!
//! - **Section selection.** Without conditions, one section formats every
//!   number; with two, the first takes `>= 0` and the second `< 0`; with
//!   three, `> 0`, `< 0` and zero. The 4th section (or a trailing section
//!   containing `@`) formats text. User conditions replace the defaults:
//!   the first section's condition is tested first, then the second's, and
//!   the next section takes everything else. A number that matches no
//!   existing section is shown in General.
//! - **Minus sign.** Only the first section adds a `-` automatically, at the
//!   very start of the output (`"$"0` shows `-$5`), and not when the first
//!   section's condition is `[<0]`/`[<=0]`. Later sections never add one.
//!   A value that rounds to zero at the shown precision shows no sign
//!   (LibreOffice behaviour; Excel shows `-0`).
//! - **Rounding.** Numbers are rounded to 15 significant digits, then half
//!   away from zero at the displayed decimals (`rtl::math::round`).
//! - **Width.** With `width_chars`, General shrinks to fit (see
//!   [`general_string`]); any other number or date that is too wide shows
//!   `###`. Text never becomes `###`.

use tally_types::{round_to_15_digits, DateEpoch, ErrorKind, Value};

use crate::date::{civil_from_days, iso_week_number, unix_days_from_serial, weekday_sun0};
use crate::decimal::Decimal;
use crate::format_ast::{
    CompareOp, DateLayout, DateTimeToken, Digit, ElapsedUnit, FormatAst, GeneralKeyword,
    NumberLayout, Section, SectionKind, Token,
};
use crate::locale::Locale;

/// Horizontal alignment the grid should use when the cell has none set.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum AlignHint {
    /// Text.
    Left,
    /// Numbers, dates and booleans.
    Right,
    /// Error values.
    Center,
}

/// The result of formatting one value.
#[derive(Clone, Eq, PartialEq, Debug)]
pub struct Formatted {
    /// The display text (without fill padding).
    pub text: String,
    /// Colour from the section's `[RED]`-style modifier, as `0xRRGGBB`.
    pub color: Option<u32>,
    /// Default alignment.
    pub align: AlignHint,
    /// Fill character from a `*x` code; the grid repeats it at
    /// [`Formatted::fill_pos`] to fill the column width.
    pub fill: Option<char>,
    /// Byte offset in `text` where fill characters go (0 when no fill).
    pub fill_pos: usize,
}

impl Formatted {
    fn plain(text: impl Into<String>, align: AlignHint) -> Self {
        Formatted {
            text: text.into(),
            color: None,
            align,
            fill: None,
            fill_pos: 0,
        }
    }
}

/// Text shown when a number does not fit its column.
pub const OVERFLOW: &str = "###";

/// Formats `value` with `ast` using the default 1899-12-30 epoch.
///
/// See [`format_value_with`]; engine code should pass the document epoch.
pub fn format_value(
    value: &Value,
    ast: &FormatAst,
    locale: &Locale,
    width_chars: Option<u16>,
) -> Formatted {
    format_value_with(value, ast, locale, width_chars, DateEpoch::D1899)
}

/// Formats `value` with `ast`. `width_chars` is the column width in
/// characters (General shrinks to it; other formats show `###` past it);
/// `epoch` interprets date serials. A cell format's language tag
/// (`[$-407]`, see [`format_language`]) picks the locale.
pub fn format_value_with(
    value: &Value,
    ast: &FormatAst,
    locale: &Locale,
    width_chars: Option<u16>,
    epoch: DateEpoch,
) -> Formatted {
    let locale = format_language(ast).unwrap_or(locale);
    format_value_in_locale(value, ast, locale, width_chars, epoch)
}

/// [`format_value_with`] in `locale` whatever the code's `[$-LCID]` tag
/// says. For TEXT() and FIXED: AOO formats their code in the document
/// language, a `[$-407]` inside it only sets the NatNum language
/// (zformat.cxx), and Excel ignores it for numbers too.
pub fn format_value_in_locale(
    value: &Value,
    ast: &FormatAst,
    locale: &Locale,
    width_chars: Option<u16>,
    epoch: DateEpoch,
) -> Formatted {
    let width = width_chars.map(usize::from);
    match value {
        Value::Empty => Formatted::plain("", AlignHint::Left),
        Value::Number(x) => format_number(*x, ast, locale, width, epoch),
        Value::Text(s) => format_text(s, ast, locale),
        Value::Bool(b) => Formatted::plain(
            if *b {
                locale.true_name
            } else {
                locale.false_name
            },
            AlignHint::Right,
        ),
        // Excel centres errors; we do the same as a hint.
        Value::Error(e) => Formatted::plain(e.display().into_owned(), AlignHint::Center),
        Value::Array(m) => match m.data.first() {
            Some(v) => format_value_in_locale(v, ast, locale, width_chars, epoch),
            None => Formatted::plain("", AlignHint::Left),
        },
    }
}

/// The locale named by a language-only tag (`[$-407]`, no currency symbol)
/// in a cell's code. Tally keeps the format's language (AOO's format
/// LanguageType, ODS `number:language`) as this tag, so separators, month
/// and day names follow it. A tag with a symbol (`[$€-407]`) only
/// picks the symbol, and a language Tally has no data for is ignored.
pub fn format_language(ast: &FormatAst) -> Option<&'static Locale> {
    ast.sections
        .iter()
        .flat_map(|s| s.tokens.iter())
        .find_map(|t| match t {
            Token::Currency {
                symbol,
                locale: Some(hex),
            } if symbol.is_empty() => crate::locale::locale_by_lcid(hex),
            _ => None,
        })
}

/// Output buffer shared by the section formatters.
struct Out {
    text: String,
    fill: Option<char>,
    fill_pos: usize,
}

impl Out {
    fn new() -> Self {
        Out {
            text: String::with_capacity(24),
            fill: None,
            fill_pos: 0,
        }
    }

    fn insert_sign(&mut self) {
        self.text.insert(0, '-');
        if self.fill.is_some() {
            self.fill_pos += 1;
        }
    }
}

fn split_text_section(ast: &FormatAst) -> (&[Section], Option<&Section>) {
    let s = &ast.sections;
    let n = s.len();
    if n == 4 {
        (&s[..3], Some(&s[3]))
    } else if n >= 1 && matches!(s[n - 1].kind, SectionKind::Text) {
        (&s[..n - 1], Some(&s[n - 1]))
    } else {
        (&s[..], None)
    }
}

fn format_text(text: &str, ast: &FormatAst, locale: &Locale) -> Formatted {
    let (_, ts) = split_text_section(ast);
    let Some(sec) = ts else {
        return Formatted::plain(text, AlignHint::Left);
    };
    let mut out = Out::new();
    for t in &sec.tokens {
        match t {
            Token::At => out.text.push_str(text),
            _ => emit_literal(t, locale, &mut out),
        }
    }
    Formatted {
        text: out.text,
        color: sec.color.map(|c| c.rgb()),
        align: AlignHint::Left,
        fill: out.fill,
        fill_pos: out.fill_pos,
    }
}

/// OOo `IsFirstSubformatRealNegative`: the first section is explicitly for
/// negative numbers, so its values are shown without a sign.
fn first_real_negative(c1: Option<(CompareOp, f64)>, c2: Option<(CompareOp, f64)>) -> bool {
    let Some((op1, l1)) = c1 else { return false };
    let (op2, l2) = match c2 {
        Some((op, l)) => (Some(op), l),
        None => (None, 0.0),
    };
    if l1 != 0.0 || l2 != 0.0 {
        return false;
    }
    match op1 {
        CompareOp::Lt => matches!(
            op2,
            None | Some(CompareOp::Gt) | Some(CompareOp::Eq) | Some(CompareOp::Ge)
        ),
        CompareOp::Le => matches!(op2, None | Some(CompareOp::Gt)),
        _ => false,
    }
}

/// Picks the section for a number. Returns `(index, show_sign)`; an index
/// past the numeric sections means General.
fn select_section(numeric: &[Section], x: f64) -> (usize, bool) {
    let k = numeric.len();
    let c1 = numeric
        .first()
        .and_then(|s| s.condition)
        .map(|c| (c.op, c.value));
    let c2 = numeric
        .get(1)
        .and_then(|s| s.condition)
        .map(|c| (c.op, c.value));
    let (op1, op2) = if c1.is_none() && c2.is_none() {
        match k {
            0 | 1 => (None, None),
            2 => (Some((CompareOp::Ge, 0.0)), None),
            _ => (Some((CompareOp::Gt, 0.0)), Some((CompareOp::Lt, 0.0))),
        }
    } else {
        (c1, c2)
    };
    let pass = |c: Option<(CompareOp, f64)>| c.is_none_or(|(op, l)| op.test(x, l));
    let idx = if pass(op1) {
        0
    } else if pass(op2) {
        1
    } else {
        2
    };
    let show_sign = x < 0.0 && idx == 0 && !first_real_negative(op1, op2);
    (idx, show_sign)
}

fn format_number(
    x: f64,
    ast: &FormatAst,
    locale: &Locale,
    width: Option<usize>,
    epoch: DateEpoch,
) -> Formatted {
    if !x.is_finite() {
        return Formatted::plain(ErrorKind::Num.display().into_owned(), AlignHint::Right);
    }
    let (numeric, _) = split_text_section(ast);
    let (idx, show_sign) = select_section(numeric, x);
    let Some(sec) = numeric.get(idx) else {
        return Formatted::plain(general_string(x, width, locale), AlignHint::Right);
    };
    let ax = x.abs();
    let mut out = Out::new();
    let mut check_width = true;
    match &sec.kind {
        SectionKind::Number(lay) => {
            let zero = number_section(ax, lay, &sec.tokens, locale, &mut out);
            if show_sign && !zero {
                out.insert_sign();
            }
        }
        SectionKind::Date(lay) => {
            let v = if idx == 0 { x } else { ax };
            if date_section(v, lay, &sec.tokens, locale, epoch, &mut out).is_err() {
                out.text.clear();
                out.text.push_str(OVERFLOW);
                out.fill = None;
            }
        }
        SectionKind::General | SectionKind::Text => {
            check_width = false;
            let v = if show_sign { x } else { ax };
            general_section(v, &sec.tokens, locale, width, &mut out);
        }
    }
    if check_width {
        if let Some(w) = width {
            if out.text.chars().count() > w {
                out.text.clear();
                out.text.push_str(OVERFLOW);
                out.fill = None;
                out.fill_pos = 0;
            }
        }
    }
    Formatted {
        text: out.text,
        color: sec.color.map(|c| c.rgb()),
        align: AlignHint::Right,
        fill: out.fill,
        fill_pos: out.fill_pos,
    }
}

/// Emits a token that is not a digit placeholder of the current region.
fn emit_literal(t: &Token, locale: &Locale, out: &mut Out) {
    match t {
        Token::Quoted(s) => out.text.push_str(s),
        Token::Escaped(c) | Token::Literal(c) => out.text.push(*c),
        Token::Skip(_) => out.text.push(' '),
        Token::Fill(c) => {
            if out.fill.is_none() {
                out.fill = Some(*c);
                out.fill_pos = out.text.len();
            }
        }
        Token::Currency { symbol, .. } => out.text.push_str(symbol),
        Token::Percent => out.text.push('%'),
        Token::Digit(d) => out.text.push(d.as_char()),
        Token::DecimalPoint => out.text.push(locale.decimal_sep),
        Token::Slash => out.text.push('/'),
        Token::Exponent { lower, plus } => {
            out.text.push(if *lower { 'e' } else { 'E' });
            out.text.push(if *plus { '+' } else { '-' });
        }
        Token::At => out.text.push('@'),
        Token::Comma
        | Token::DateTime(_)
        | Token::General(_)
        | Token::Color(_)
        | Token::Condition(_)
        | Token::NatNum(_)
        | Token::DbNum(_)
        | Token::Calendar(_) => {}
    }
}

#[inline]
fn push_digit(out: &mut Out, d: u8) {
    out.text.push(char::from(b'0' + d));
}

/// Fills an integer region right-aligned. `n` is the number of integer
/// digits and `digit(r)` the digit at position `r` from the right. With
/// `dump`, a region without placeholders still shows the digits at its end
/// (`.00` shows `12.50`).
fn emit_int_region(
    tokens: &[Token],
    n: usize,
    digit: &dyn Fn(usize) -> u8,
    grouping: bool,
    dump: bool,
    locale: &Locale,
    out: &mut Out,
) {
    let p = tokens
        .iter()
        .filter(|t| matches!(t, Token::Digit(_)))
        .count();
    let sep = |out: &mut Out, r: usize| {
        if grouping && r > 0 && r.is_multiple_of(3) {
            out.text.push(locale.group_sep);
        }
    };
    let mut k = 0;
    for t in tokens {
        match t {
            Token::Digit(kind) => {
                let r = p - 1 - k;
                if k == 0 && n > p {
                    for rr in (p..n).rev() {
                        push_digit(out, digit(rr));
                        sep(out, rr);
                    }
                }
                if r < n {
                    push_digit(out, digit(r));
                    sep(out, r);
                } else {
                    match kind {
                        Digit::Zero => {
                            out.text.push('0');
                            sep(out, r);
                        }
                        Digit::Question => out.text.push(' '),
                        Digit::Hash => {}
                    }
                }
                k += 1;
            }
            Token::Comma => {}
            _ => emit_literal(t, locale, out),
        }
    }
    if p == 0 && dump {
        for rr in (0..n).rev() {
            push_digit(out, digit(rr));
            sep(out, rr);
        }
    }
}

/// Fills a decimal fraction region left-aligned, trimming trailing zeros
/// under `#` (removed) and `?` (spaces).
fn emit_frac_region(tokens: &[Token], d: &Decimal, locale: &Locale, out: &mut Out) {
    let mut kinds = [Digit::Zero; 64];
    let mut q = 0;
    for t in tokens {
        if let Token::Digit(kind) = t {
            if q < kinds.len() {
                kinds[q] = *kind;
            }
            q += 1;
        }
    }
    let mut shown = q;
    while shown > 0 {
        let kind = kinds.get(shown - 1).copied().unwrap_or(Digit::Zero);
        if kind != Digit::Zero && d.frac_digit(shown - 1) == 0 {
            shown -= 1;
        } else {
            break;
        }
    }
    let mut k = 0;
    for t in tokens {
        match t {
            Token::Digit(kind) => {
                if k < shown {
                    push_digit(out, d.frac_digit(k));
                } else if *kind == Digit::Question {
                    out.text.push(' ');
                }
                k += 1;
            }
            Token::Comma => {}
            _ => emit_literal(t, locale, out),
        }
    }
}

/// Formats a number section; returns whether the displayed value is zero
/// (no sign is added then). A section without any digit placeholder, such
/// as `"zero"` or an empty section, shows only its literals.
fn number_section(
    x: f64,
    lay: &NumberLayout,
    tokens: &[Token],
    locale: &Locale,
    out: &mut Out,
) -> bool {
    if !tokens.iter().any(|t| matches!(t, Token::Digit(_))) {
        for t in tokens {
            emit_literal(t, locale, out);
        }
        return true;
    }
    if lay.fraction.is_some() {
        return fraction_section(x, lay, tokens, locale, out);
    }
    let mut d = Decimal::from_f64(x);
    d.shift(2 * i32::from(lay.percent) - 3 * i32::from(lay.scale_commas));
    let frac = i32::from(lay.frac_digits);
    let mut exp_val = 0i32;
    if lay.exponent.is_some() {
        let k = i32::from(lay.int_digits);
        let has_opt = tokens[..lay.int_end]
            .iter()
            .any(|t| matches!(t, Token::Digit(Digit::Hash) | Token::Digit(Digit::Question)));
        let eng = has_opt && k > 1;
        let calc = |p: i32| {
            if eng {
                (p - 1).div_euclid(k) * k
            } else {
                p - k
            }
        };
        if !d.is_zero() {
            let p = d.point;
            let mut e = calc(p);
            d.round_sig((p - e + frac).max(1) as usize);
            if d.point != p {
                e = calc(d.point);
            }
            d.shift(-e);
            exp_val = e;
        }
    } else {
        d.round_frac(frac);
    }
    let zero = d.is_zero();
    let n = d.int_len();
    emit_int_region(
        &tokens[..lay.int_end],
        n,
        &|r| d.int_digit_from_right(r),
        lay.grouping,
        true,
        locale,
        out,
    );
    if let Some(di) = lay.decimal_index {
        out.text.push(locale.decimal_sep);
        emit_frac_region(&tokens[di + 1..lay.frac_end], &d, locale, out);
    }
    if let Some(e) = lay.exponent {
        if let Token::Exponent { lower, .. } = tokens[e.index] {
            out.text.push(if lower { 'e' } else { 'E' });
        }
        if exp_val < 0 {
            out.text.push('-');
        } else if e.plus {
            out.text.push('+');
        }
        let s = exp_val.unsigned_abs().to_string();
        for _ in s.len()..usize::from(e.digits) {
            out.text.push('0');
        }
        out.text.push_str(&s);
        let mut i = e.index + 1;
        while i < tokens.len() && matches!(tokens[i], Token::Digit(_)) {
            i += 1;
        }
        for t in &tokens[i..] {
            emit_literal(t, locale, out);
        }
    }
    zero
}

/// Best rational approximation `p/q` of `x >= 0` with `q <= max_den`
/// (continued fractions with semiconvergents).
pub(crate) fn best_rational(x: f64, max_den: u64) -> (u64, u64) {
    if x.is_nan() || x <= 0.0 {
        return (0, 1);
    }
    let max_den = max_den.max(1);
    if x >= 1e15 {
        return (x.round() as u64, 1);
    }
    let (mut h0, mut k0, mut h1, mut k1) = (0u64, 1u64, 1u64, 0u64);
    let mut v = x;
    for _ in 0..64 {
        let af = v.floor();
        if af >= 1e15 {
            break;
        }
        let a = af as u64;
        let h2 = a.checked_mul(h1).and_then(|t| t.checked_add(h0));
        let k2 = a.checked_mul(k1).and_then(|t| t.checked_add(k0));
        let (Some(h2), Some(k2)) = (h2, k2) else {
            break;
        };
        if k2 > max_den {
            if k1 == 0 {
                break;
            }
            let t = (max_den - k0) / k1;
            let hs = h0.saturating_add(t.saturating_mul(h1));
            let ks = k0.saturating_add(t.saturating_mul(k1));
            let err_s = (x - hs as f64 / ks as f64).abs();
            let err_c = (x - h1 as f64 / k1 as f64).abs();
            return if ks > 0 && err_s < err_c {
                (hs, ks)
            } else {
                (h1, k1)
            };
        }
        h0 = h1;
        k0 = k1;
        h1 = h2;
        k1 = k2;
        let f = v - af;
        if f < 1e-12 || (h1 as f64 / k1 as f64 - x).abs() <= 1e-15 * x.max(1.0) {
            break;
        }
        v = 1.0 / f;
    }
    if k1 == 0 {
        return (x.round() as u64, 1);
    }
    (h1, k1)
}

fn num_digits(mut v: u64) -> usize {
    let mut n = 0;
    while v > 0 {
        v /= 10;
        n += 1;
    }
    n
}

fn digit_of(v: u64, r: usize) -> u8 {
    let mut v = v;
    for _ in 0..r {
        v /= 10;
    }
    (v % 10) as u8
}

fn fraction_section(
    x: f64,
    lay: &NumberLayout,
    tokens: &[Token],
    locale: &Locale,
    out: &mut Out,
) -> bool {
    let Some(f) = lay.fraction else {
        return true;
    };
    let mut x = round_to_15_digits(x);
    for _ in 0..lay.percent {
        x *= 100.0;
    }
    for _ in 0..lay.scale_commas {
        x /= 1000.0;
    }
    let x = round_to_15_digits(x);
    let mixed = f.int_range.is_some();
    let (mut ip, frac) = if mixed {
        let ip = x.trunc();
        (ip, x - ip)
    } else {
        (0.0, x)
    };
    let (mut num, den) = match f.fixed_den {
        Some(d) => {
            let d = u64::from(d.max(1));
            ((frac * d as f64).round() as u64, d)
        }
        None => {
            let digits = u32::from(f.den_digits.min(7));
            best_rational(frac, 10u64.pow(digits) - 1)
        }
    };
    if mixed && num >= den {
        ip += (num / den) as f64;
        num %= den;
    }
    let ipu = if ip < 1e18 { ip as u64 } else { 0 };
    let zero_frac = mixed && num == 0;
    let zero = zero_frac && ipu == 0;
    let (num_start, num_end) = f.num_range;
    let (den_start, den_end) = f.den_range;
    let mut i = 0;
    while i < tokens.len() {
        if let Some((a, b)) = f.int_range {
            if i == a {
                let n = if zero_frac && ipu == 0 {
                    1
                } else {
                    num_digits(ipu)
                };
                emit_int_region(
                    &tokens[a..b],
                    n,
                    &|r| digit_of(ipu, r),
                    lay.grouping,
                    true,
                    locale,
                    out,
                );
                i = b;
                continue;
            }
        }
        if i == num_start {
            if zero_frac {
                for _ in num_start..num_end {
                    out.text.push(' ');
                }
            } else {
                let n = num_digits(num).max(1);
                emit_int_region(
                    &tokens[num_start..num_end],
                    n,
                    &|r| digit_of(num, r),
                    false,
                    true,
                    locale,
                    out,
                );
            }
            i = num_end;
            continue;
        }
        if i == f.slash {
            out.text.push(if zero_frac { ' ' } else { '/' });
            i += 1;
            continue;
        }
        if i == den_start {
            if zero_frac {
                for _ in den_start..den_end {
                    out.text.push(' ');
                }
            } else if f.fixed_den.is_some() {
                for t in &tokens[den_start..den_end] {
                    emit_literal(t, locale, out);
                }
            } else {
                let s = den.to_string();
                out.text.push_str(&s);
                let mut k = s.len();
                for t in &tokens[den_start..den_end] {
                    if let Token::Digit(kind) = t {
                        if k > 0 {
                            k -= 1;
                        } else {
                            match kind {
                                Digit::Question => out.text.push(' '),
                                Digit::Zero => out.text.push('0'),
                                Digit::Hash => {}
                            }
                        }
                    }
                }
            }
            i = den_end.max(den_start + 1);
            continue;
        }
        emit_literal(&tokens[i], locale, out);
        i += 1;
    }
    zero
}

fn general_section(x: f64, tokens: &[Token], locale: &Locale, width: Option<usize>, out: &mut Out) {
    let render = |out: &mut Out, w: Option<usize>| -> usize {
        let mut glen = 0;
        let mut any = false;
        for t in tokens {
            match t {
                Token::General(GeneralKeyword::Boolean) => {
                    // AOO's BOOLEAN format: TRUE for any non-zero number.
                    let g = if x != 0.0 {
                        locale.true_name
                    } else {
                        locale.false_name
                    };
                    glen += g.chars().count();
                    out.text.push_str(g);
                    any = true;
                }
                Token::General(_) | Token::At => {
                    let g = general_string(x, w, locale);
                    glen += g.chars().count();
                    out.text.push_str(&g);
                    any = true;
                }
                _ => emit_literal(t, locale, out),
            }
        }
        if !any {
            let g = general_string(x, w, locale);
            glen += g.chars().count();
            out.text.push_str(&g);
        }
        glen
    };
    let glen = render(out, None);
    if let Some(w) = width {
        let total = out.text.chars().count();
        if total > w {
            let lit = total - glen;
            out.text.clear();
            out.fill = None;
            out.fill_pos = 0;
            if lit >= w {
                out.text.push_str(OVERFLOW);
            } else {
                render(out, Some(w - lit));
            }
        }
    }
}

fn push_fixed(s: &mut String, neg: bool, d: &Decimal, locale: &Locale) {
    if neg {
        s.push('-');
    }
    let n = d.int_len();
    if n == 0 {
        s.push('0');
    } else {
        for r in (0..n).rev() {
            s.push(char::from(b'0' + d.int_digit_from_right(r)));
        }
    }
    let f = d.frac_len();
    if f > 0 {
        s.push(locale.decimal_sep);
        for k in 0..f {
            s.push(char::from(b'0' + d.frac_digit(k)));
        }
    }
}

fn push_sci(s: &mut String, neg: bool, d: &Decimal, locale: &Locale) {
    if neg {
        s.push('-');
    }
    s.push(char::from(b'0' + d.digit(0)));
    if d.len() > 1 {
        s.push(locale.decimal_sep);
        for i in 1..d.len() {
            s.push(char::from(b'0' + d.digit(i as i32)));
        }
    }
    let e = d.point - 1;
    s.push('E');
    s.push(if e < 0 { '-' } else { '+' });
    let a = e.unsigned_abs();
    if a < 10 {
        s.push('0');
    }
    s.push_str(&a.to_string());
}

/// The General (Standard) format.
///
/// Without a width: at most 15 significant digits, no trailing zeros;
/// scientific (`1.5E+15`, `1E-05`, exponent at least two digits) when the
/// rounded magnitude is `>= 1E15` or `< 1E-4`, fixed otherwise.
///
/// With `width` (characters): if the full text is too wide, a fixed number
/// whose integer part (with sign) fits is rounded to as many decimals as
/// fit (`1/3` at width 5 is `0.333`). Otherwise, or if that rounding gives
/// zero, scientific notation with as many mantissa digits as fit is used
/// (`123456789012` at width 6 is `1E+11`). If even `dE+XX` does not fit,
/// a fixed form that rounded to zero shows as `0` (`1/3` at width 1);
/// otherwise the result is `###`.
pub fn general_string(x: f64, width: Option<usize>, locale: &Locale) -> String {
    let mut s = String::with_capacity(20);
    if !x.is_finite() {
        s.push_str(&ErrorKind::Num.display());
        return s;
    }
    let d = Decimal::from_f64(x);
    if d.is_zero() {
        s.push('0');
        return s;
    }
    let neg = x < 0.0;
    let sci = d.point > 15 || d.point < -3;
    if sci {
        push_sci(&mut s, neg, &d, locale);
    } else {
        push_fixed(&mut s, neg, &d, locale);
    }
    let Some(w) = width else { return s };
    if s.chars().count() <= w {
        return s;
    }
    // A fixed form that rounded to zero is kept as a last resort.
    let mut zero_fallback = false;
    if !sci {
        let int_chars = usize::from(neg) + d.int_len().max(1);
        if int_chars <= w {
            let dec = if w >= int_chars + 2 {
                (w - int_chars - 1) as i32
            } else {
                0
            };
            let mut r = d;
            r.round_frac(dec);
            if r.is_zero() {
                zero_fallback = true;
            } else {
                s.clear();
                push_fixed(&mut s, neg, &r, locale);
                if s.chars().count() <= w {
                    return s;
                }
            }
        }
    }
    for sig in (1..=d.len().max(1)).rev() {
        let mut r = d;
        r.round_sig(sig);
        s.clear();
        push_sci(&mut s, neg, &r, locale);
        if s.chars().count() <= w {
            return s;
        }
    }
    s.clear();
    s.push_str(if zero_fallback { "0" } else { OVERFLOW });
    s
}

fn push_padded(out: &mut Out, v: u64, width: usize) {
    let s = v.to_string();
    for _ in s.len()..width {
        out.text.push('0');
    }
    out.text.push_str(&s);
}

/// Formats a date/time section. `Err` means the date is out of range.
fn date_section(
    x: f64,
    lay: &DateLayout,
    tokens: &[Token],
    locale: &Locale,
    epoch: DateEpoch,
    out: &mut Out,
) -> Result<(), ()> {
    let mut v = x;
    let mut neg = false;
    if v < 0.0 && !lay.has_date {
        neg = true;
        v = -v;
    }
    let fd = u32::from(lay.frac_sec_digits.min(6));
    let scale = 10f64.powi(fd as i32);
    let ticks_per_day = 86_400.0 * scale;
    let mut days = v.floor();
    let mut ticks = ((v - days) * ticks_per_day).round();
    if ticks >= ticks_per_day {
        days += 1.0;
        ticks -= ticks_per_day;
    }
    let total_ticks = (v * ticks_per_day).round();
    if !total_ticks.is_finite() || total_ticks.abs() > 9.0e15 {
        return Err(());
    }
    let scale_u = scale as u64;
    let (secs, frac) = if lay.elapsed.is_some() {
        let t = total_ticks as u64;
        (t / scale_u, t % scale_u)
    } else {
        let t = ticks as u64;
        (t / scale_u, t % scale_u)
    };
    let hour = (secs / 3600) % 24;
    let minute = (secs / 60) % 60;
    let second = secs % 60;
    let (y, m, d, wd) = if lay.has_date {
        let ud = unix_days_from_serial(epoch, days).ok_or(())?;
        let (y, m, d) = civil_from_days(ud);
        (y, m, d, weekday_sun0(ud))
    } else {
        (1899, 12, 30, 6)
    };
    let unix_days = if lay.has_date {
        unix_days_from_serial(epoch, days).unwrap_or(0)
    } else {
        0
    };
    let mut frac_k = 0usize;
    let frac_str = if fd > 0 {
        let s = frac.to_string();
        let mut p = String::with_capacity(fd as usize);
        for _ in s.len()..fd as usize {
            p.push('0');
        }
        p.push_str(&s);
        p
    } else {
        String::new()
    };
    let mut in_frac = false;
    for t in tokens {
        if in_frac {
            if let Token::Digit(Digit::Zero) = t {
                if let Some(c) = frac_str.as_bytes().get(frac_k) {
                    out.text.push(char::from(*c));
                }
                frac_k += 1;
                continue;
            }
            in_frac = false;
        }
        let Token::DateTime(dt) = t else {
            if matches!(t, Token::DecimalPoint) {
                out.text.push(locale.decimal_sep);
                in_frac = true;
                frac_k = 0;
                continue;
            }
            emit_literal(t, locale, out);
            continue;
        };
        match dt {
            DateTimeToken::Day(n) => push_padded(out, u64::from(d), usize::from(*n)),
            DateTimeToken::DayName(n) | DateTimeToken::AaaDayName(n) => {
                let names = if *n >= 4 {
                    &locale.day_names
                } else {
                    &locale.short_day_names
                };
                out.text.push_str(names[wd as usize]);
            }
            DateTimeToken::AooDayName(n) => {
                if *n == 2 {
                    out.text.push_str(locale.short_day_names[wd as usize]);
                } else {
                    out.text.push_str(locale.day_names[wd as usize]);
                    if *n >= 4 {
                        out.text.push_str(locale.long_date_day_of_week_sep);
                    }
                }
            }
            DateTimeToken::Month(n) => {
                let mi = (m - 1) as usize;
                match n {
                    1 | 2 => push_padded(out, u64::from(m), usize::from(*n)),
                    3 => out.text.push_str(locale.short_month_names[mi]),
                    4 => out.text.push_str(locale.month_names[mi]),
                    _ => {
                        if let Some(c) = locale.month_names[mi].chars().next() {
                            out.text.push(c);
                        }
                    }
                }
            }
            DateTimeToken::Minute(n) => push_padded(out, minute, usize::from(*n)),
            DateTimeToken::Year(n) => {
                if *n <= 2 {
                    push_padded(out, y.rem_euclid(100) as u64, 2);
                } else {
                    push_padded(out, y.max(0) as u64, 4);
                }
            }
            DateTimeToken::Hour(n) => {
                let h = if lay.twelve_hour {
                    match hour % 12 {
                        0 => 12,
                        h => h,
                    }
                } else {
                    hour
                };
                push_padded(out, h, usize::from(*n));
            }
            DateTimeToken::Second(n) => push_padded(out, second, usize::from(*n)),
            DateTimeToken::AmPm(_) => out.text.push_str(if hour < 12 {
                locale.am_pm.0
            } else {
                locale.am_pm.1
            }),
            DateTimeToken::AP(s) => {
                let lower = s.starts_with('a');
                let c = match (hour < 12, lower) {
                    (true, false) => 'A',
                    (true, true) => 'a',
                    (false, false) => 'P',
                    (false, true) => 'p',
                };
                out.text.push(c);
            }
            DateTimeToken::Quarter(n) => {
                let q = ((m - 1) / 3) as usize;
                out.text.push_str(if *n >= 2 {
                    locale.quarter_names[q]
                } else {
                    locale.quarter_abbrev[q]
                });
            }
            DateTimeToken::Week(_) => {
                out.text.push_str(&iso_week_number(unix_days).to_string());
            }
            DateTimeToken::Era(n) => {
                let (abbr, long) = if y > 0 {
                    (locale.era_abbrev.1, locale.era_names.1)
                } else {
                    (locale.era_abbrev.0, locale.era_names.0)
                };
                match n {
                    1 => {
                        if let Some(c) = abbr.chars().next() {
                            out.text.push(c);
                        }
                    }
                    2 => out.text.push_str(abbr),
                    _ => out.text.push_str(long),
                }
            }
            DateTimeToken::EraYear(n) => push_padded(out, y.max(0) as u64, usize::from(*n)),
            DateTimeToken::Elapsed(unit, n) => {
                let v = match unit {
                    ElapsedUnit::Hours => secs / 3600,
                    ElapsedUnit::Minutes => secs / 60,
                    ElapsedUnit::Seconds => secs,
                };
                push_padded(out, v, usize::from(*n));
            }
        }
    }
    if neg && (secs > 0 || frac > 0) {
        out.insert_sign();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_language_tag() {
        let f = |code: &str, x: f64| {
            let ast = crate::parse_format(code).unwrap();
            format_value(&Value::Number(x), &ast, &crate::EN_US, None).text
        };
        // `[$-407]`: German separators and month names in an en-US document.
        assert_eq!(f("[$-407]#,##0.00", 1234.5), "1.234,50");
        assert_eq!(f("[$-407]DD. MMMM YYYY", 46000.0), "09. Dezember 2025");
        // A currency tag only picks the symbol (AOO, Excel).
        assert_eq!(f("#,##0.00 [$€-407]", 1234.5), "1,234.50 €");
        // Unknown languages fall back to the document locale.
        assert_eq!(f("[$-416]#,##0.00", 1234.5), "1,234.50");
        let ast = crate::parse_format("[$-40C]0.00").unwrap();
        assert_eq!(format_language(&ast).map(|l| l.tag), Some("fr-FR"));
        // TEXT / FIXED keep the document locale.
        let ast = crate::parse_format("[$-407]#,##0.00").unwrap();
        let t = format_value_in_locale(
            &Value::Number(1234.5),
            &ast,
            &crate::EN_US,
            None,
            DateEpoch::D1899,
        );
        assert_eq!(t.text, "1,234.50");
    }

    #[test]
    fn general_with_language_and_boolean() {
        let p = |c: &str| crate::parse_format(c).unwrap();
        // A language-tagged General is still General (input recognition,
        // column fit, the format bar).
        assert!(p("General").is_general());
        assert!(p("[$-407]General").is_general());
        assert!(p("[$-409]Standard").is_general());
        assert!(!p("[$€-407]General").is_general());
        assert!(!p("General;-General").is_general());
        assert!(!p("BOOLEAN").is_general());
        assert!(crate::is_general_code("[$-407]General"));
        assert!(crate::is_general_code(" general "));
        assert!(!crate::is_general_code("[$€-407]General"));
        assert!(!crate::is_general_code("[$-407]0.00"));
        // AOO's BOOLEAN: TRUE / FALSE for numbers (was "BOOL1899AN").
        let f = |c: &str, v: Value, l: &Locale| format_value(&v, &p(c), l, None).text;
        assert_eq!(f("BOOLEAN", Value::Number(1.0), &crate::EN_US), "TRUE");
        assert_eq!(f("BOOLEAN", Value::Number(-2.5), &crate::EN_US), "TRUE");
        assert_eq!(f("BOOLEAN", Value::Number(0.0), &crate::EN_US), "FALSE");
        assert_eq!(f("BOOLEAN", Value::Bool(false), &crate::EN_US), "FALSE");
        assert_eq!(
            f("[$-407]BOOLEAN", Value::Number(1.0), &crate::EN_US),
            "WAHR"
        );
        assert_eq!(f("BOOLEAN", Value::Number(1.0), &crate::DE_DE), "WAHR");
        assert_eq!(crate::print_format(&p("[RED]BOOLEAN")), "[RED]BOOLEAN");
    }

    #[test]
    fn rational() {
        assert_eq!(best_rational(0.5, 9), (1, 2));
        assert_eq!(best_rational(0.3, 9), (2, 7));
        assert_eq!(best_rational(0.333, 9), (1, 3));
        assert_eq!(best_rational(std::f64::consts::PI - 3.0, 99), (14, 99));
        assert_eq!(best_rational(std::f64::consts::PI - 3.0, 9), (1, 7));
        assert_eq!(best_rational(std::f64::consts::PI - 3.0, 999), (16, 113));
        assert_eq!(best_rational(0.0, 9), (0, 1));
        assert_eq!(best_rational(1.5, 9), (3, 2));
    }

    #[test]
    fn general_widths() {
        let l = Locale::en_us();
        assert_eq!(general_string(1.0 / 3.0, Some(5), &l), "0.333");
        assert_eq!(general_string(123_456_789_012.0, Some(6), &l), "1E+11");
        assert_eq!(general_string(123_456_789_012.0, Some(4), &l), "###");
        assert_eq!(general_string(1e15, None, &l), "1E+15");
        assert_eq!(general_string(0.0001, None, &l), "0.0001");
        assert_eq!(general_string(0.00001, None, &l), "1E-05");
    }
}
