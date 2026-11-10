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

use crate::calendar::{self, CalendarId, NumCode};
use crate::date::{civil_from_days, iso_week_number, unix_days_from_serial, weekday_sun0};
use crate::decimal::Decimal;
use crate::format_ast::{
    CompareOp, DateLayout, DateTimeToken, Digit, ElapsedUnit, FormatAst, GeneralKeyword,
    NumberLayout, Section, SectionKind, Token,
};
use crate::locale::Locale;
use crate::natnum::{self, LangId, Separators};

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
    format_in_locale(value, ast, locale, width_chars, epoch, None, true)
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
    format_in_locale(value, ast, locale, width_chars, epoch, None, false)
}

/// [`format_value_with`] for a document that limits the decimals of the
/// General format (`standard_decimals`, AOO Tools ▸ Options ▸ Calc ▸
/// Calculate ▸ "Limit decimals for general number format"; `None` =
/// unlimited). See [`general_string_limited`].
pub fn format_value_std(
    value: &Value,
    ast: &FormatAst,
    locale: &Locale,
    width_chars: Option<u16>,
    epoch: DateEpoch,
    standard_decimals: Option<u8>,
) -> Formatted {
    let locale = format_language(ast).unwrap_or(locale);
    format_in_locale(
        value,
        ast,
        locale,
        width_chars,
        epoch,
        standard_decimals,
        true,
    )
}

/// [`format_value_in_locale`] with the document's General decimals
/// (TEXT: AOO formats with the document's formatter, whose General keyword
/// follows `GetStandardPrec`).
pub fn format_value_in_locale_std(
    value: &Value,
    ast: &FormatAst,
    locale: &Locale,
    width_chars: Option<u16>,
    epoch: DateEpoch,
    standard_decimals: Option<u8>,
) -> Formatted {
    format_in_locale(
        value,
        ast,
        locale,
        width_chars,
        epoch,
        standard_decimals,
        false,
    )
}

/// Where the format's language comes from (see [`LangCtx::format_lang`]).
#[derive(Copy, Clone)]
struct LangCtx<'a> {
    ast: &'a FormatAst,
    /// The code's `[$-LCID]` tag is the format language (cell formats);
    /// `false` for TEXT / FIXED, which use the document language.
    ast_lang: bool,
}

impl LangCtx<'_> {
    /// The format's language (AOO: the format's LanguageType): the code's
    /// first language tag, also one Tally has no locale data for, else the
    /// locale's.
    fn format_lang(&self, locale: &Locale) -> LangId {
        if self.ast_lang {
            if let Some(l) = self
                .ast
                .sections
                .iter()
                .find_map(|s| section_lang(&s.tokens))
            {
                return l;
            }
        }
        locale_lang(locale)
    }

    /// Whether a tag asks for Excel's gannen (first gengou year as 元).
    fn gannen(&self) -> bool {
        self.ast
            .sections
            .iter()
            .flat_map(|s| s.tokens.iter())
            .any(|t| {
                matches!(t, Token::Currency { symbol, locale: Some(tag) }
                if symbol.is_empty() && tag.to_ascii_lowercase().ends_with("-x-gannen"))
            })
    }
}

/// The language of a locale's data.
fn locale_lang(locale: &Locale) -> LangId {
    crate::locale::lang_id(locale.lcid_hex).unwrap_or(LangId::EN_US)
}

/// A section's own language tag (`[$-411]`, also without locale data).
fn section_lang(tokens: &[Token]) -> Option<LangId> {
    tokens.iter().find_map(|t| match t {
        Token::Currency {
            symbol,
            locale: Some(tag),
        } if symbol.is_empty() => crate::locale::lang_id(tag),
        _ => None,
    })
}

/// The NatNum mode and its language for a section (AOO `SvNumberNatNum`:
/// the section's `[$-LCID]`, else the format's language), or `None`.
fn section_natnum(
    sec: &Section,
    locale: &Locale,
    ctx: LangCtx<'_>,
    date: bool,
) -> Option<(LangId, u8)> {
    let nn = sec.natnum?;
    let lang = section_lang(&sec.tokens).unwrap_or_else(|| ctx.format_lang(locale));
    Some((lang, nn.mode(lang, date)))
}

fn separators(locale: &Locale) -> Separators {
    Separators {
        decimal: locale.decimal_sep,
        group: locale.group_sep,
    }
}

/// `ImpTransliterate` of a whole output text, keeping the fill position.
fn transliterate_out(out: &mut Out, lang: LangId, mode: u8, locale: &Locale) {
    let sep = separators(locale);
    if out.fill.is_some() {
        let (a, b) = out.text.split_at(out.fill_pos);
        let a = natnum::transliterate(a, lang, mode, sep).into_owned();
        let b = natnum::transliterate(b, lang, mode, sep).into_owned();
        out.fill_pos = a.len();
        out.text = a + &b;
    } else if let std::borrow::Cow::Owned(t) = natnum::transliterate(&out.text, lang, mode, sep) {
        out.text = t;
    }
}

/// `ImpTransliterate` of the text written since byte `start`, keeping the
/// fill position.
fn transliterate_tail(out: &mut Out, start: usize, nat: Option<(LangId, u8)>, locale: &Locale) {
    let Some((lang, mode)) = nat else {
        return;
    };
    let sep = separators(locale);
    if out.fill.is_some() && out.fill_pos > start {
        let tail = out.text.split_off(start);
        let (a, b) = tail.split_at(out.fill_pos - start);
        out.text
            .push_str(&natnum::transliterate(a, lang, mode, sep));
        out.fill_pos = out.text.len();
        out.text
            .push_str(&natnum::transliterate(b, lang, mode, sep));
    } else if let std::borrow::Cow::Owned(t) =
        natnum::transliterate(&out.text[start..], lang, mode, sep)
    {
        out.text.truncate(start);
        out.text.push_str(&t);
    }
}

#[allow(clippy::too_many_arguments)]
fn format_in_locale(
    value: &Value,
    ast: &FormatAst,
    locale: &Locale,
    width_chars: Option<u16>,
    epoch: DateEpoch,
    std_dec: Option<u8>,
    ast_lang: bool,
) -> Formatted {
    let width = width_chars.map(usize::from);
    let ctx = LangCtx { ast, ast_lang };
    match value {
        Value::Empty => Formatted::plain("", AlignHint::Left),
        Value::Number(x) => format_number(*x, ast, locale, width, epoch, std_dec, ctx),
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
            Some(v) => format_in_locale(v, ast, locale, width_chars, epoch, std_dec, ast_lang),
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

#[allow(clippy::too_many_arguments)]
fn format_number(
    x: f64,
    ast: &FormatAst,
    locale: &Locale,
    width: Option<usize>,
    epoch: DateEpoch,
    std_dec: Option<u8>,
    ctx: LangCtx<'_>,
) -> Formatted {
    if !x.is_finite() {
        return Formatted::plain(ErrorKind::Num.display().into_owned(), AlignHint::Right);
    }
    let (numeric, _) = split_text_section(ast);
    let (idx, show_sign) = select_section(numeric, x);
    // ImpGetOutputStdToPrecision transliterates General text with the
    // first section's NatNum, whichever section shows it; the E notation of
    // |x| > 1e15 (ImpGetOutputStandard) is not transliterated there.
    let nat0 = ast
        .sections
        .first()
        .and_then(|s0| section_natnum(s0, locale, ctx, false));
    let Some(sec) = numeric.get(idx) else {
        let text = general_string_limited(x, width, locale, std_dec);
        let text = general_natnum(text, nat0, locale);
        return Formatted::plain(text, AlignHint::Right);
    };
    let ax = x.abs();
    let mut out = Out::new();
    let mut check_width = true;
    match &sec.kind {
        SectionKind::Number(lay) => {
            let nat = if sec.natnum.is_some()
                && lay.exponent.is_none()
                && sec.tokens.iter().any(|t| matches!(t, Token::Digit(_)))
            {
                section_natnum(sec, locale, ctx, false)
            } else {
                None
            };
            // Fractions transliterate only the integer, numerator and
            // denominator (zformat.cxx NUMBERFORMAT_FRACTION:
            // ImpTransliterate / ImpIntToString); the sign stays ASCII.
            let frac_nat = nat.filter(|_| lay.fraction.is_some());
            let zero = number_section(ax, lay, &sec.tokens, locale, frac_nat, &mut out);
            if show_sign && !zero {
                out.insert_sign();
            }
            // ImpGetNumberOutput transliterates the whole text (literals
            // too); scientific and literal-only sections are left alone.
            if lay.fraction.is_none() {
                if let Some((lang, mode)) = nat {
                    transliterate_out(&mut out, lang, mode, locale);
                }
            }
        }
        SectionKind::Date(lay) => {
            let v = if idx == 0 { x } else { ax };
            if date_section(v, lay, sec, locale, epoch, &mut out, ctx).is_err() {
                out.text.clear();
                out.text.push_str(OVERFLOW);
                out.fill = None;
            }
        }
        SectionKind::General | SectionKind::Text => {
            check_width = false;
            let v = if show_sign { x } else { ax };
            let general = matches!(sec.kind, SectionKind::General);
            let gen_nat = nat0.filter(|_| general);
            general_section(v, &sec.tokens, locale, width, std_dec, gen_nat, &mut out);
            // ImpGetNumberOutput then transliterates the whole section text
            // with the section's own NatNum.
            if general {
                if let Some((lang, mode)) = section_natnum(sec, locale, ctx, false) {
                    transliterate_out(&mut out, lang, mode, locale);
                }
            }
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
    frac_nat: Option<(LangId, u8)>,
    out: &mut Out,
) -> bool {
    if !tokens.iter().any(|t| matches!(t, Token::Digit(_))) {
        for t in tokens {
            emit_literal(t, locale, out);
        }
        return true;
    }
    if lay.fraction.is_some() {
        return fraction_section(x, lay, tokens, locale, frac_nat, out);
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
    nat: Option<(LangId, u8)>,
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
                let start = out.text.len();
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
                transliterate_tail(out, start, nat, locale);
                i = b;
                continue;
            }
        }
        if i == num_start {
            let start = out.text.len();
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
            transliterate_tail(out, start, nat, locale);
            i = num_end;
            continue;
        }
        if i == f.slash {
            out.text.push(if zero_frac { ' ' } else { '/' });
            i += 1;
            continue;
        }
        if i == den_start {
            let start = out.text.len();
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
            transliterate_tail(out, start, nat, locale);
            i = den_end.max(den_start + 1);
            continue;
        }
        emit_literal(&tokens[i], locale, out);
        i += 1;
    }
    zero
}

/// General text transliterated with the first section's NatNum, unless it
/// is in E notation (`ImpGetOutputStandard`).
fn general_natnum(text: String, nat0: Option<(LangId, u8)>, locale: &Locale) -> String {
    match nat0 {
        Some((lang, mode)) if !text.contains('E') => {
            match natnum::transliterate(&text, lang, mode, separators(locale)) {
                std::borrow::Cow::Borrowed(_) => text,
                std::borrow::Cow::Owned(t) => t,
            }
        }
        _ => text,
    }
}

fn general_section(
    x: f64,
    tokens: &[Token],
    locale: &Locale,
    width: Option<usize>,
    std_dec: Option<u8>,
    gen_nat: Option<(LangId, u8)>,
    out: &mut Out,
) {
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
                    let g = general_natnum(
                        general_string_limited(x, w, locale, std_dec),
                        gen_nat,
                        locale,
                    );
                    glen += g.chars().count();
                    out.text.push_str(&g);
                    any = true;
                }
                _ => emit_literal(t, locale, out),
            }
        }
        if !any {
            let g = general_natnum(
                general_string_limited(x, w, locale, std_dec),
                gen_nat,
                locale,
            );
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

/// The General format with at most `decimals` decimals (AOO
/// `SvNumberformat::ImpGetOutputStandard` with a limited standard
/// precision, zformat.cxx): numbers up to 1E15 are rounded to `decimals`
/// places without trailing zeros (`π` → `3.14` with 2, `0.00001` → `0`,
/// no `-0`); larger ones are scientific with `min(decimals, 14)` mantissa
/// decimals. A result wider than `width` shrinks as [`general_string`]
/// does. `None` is [`general_string`] itself.
pub fn general_string_limited(
    x: f64,
    width: Option<usize>,
    locale: &Locale,
    decimals: Option<u8>,
) -> String {
    let Some(n) = decimals else {
        return general_string(x, width, locale);
    };
    if !x.is_finite() {
        return general_string(x, width, locale);
    }
    let neg = x < 0.0;
    let mut s = String::with_capacity(24);
    let shown: f64;
    if x.abs() > 1e15 {
        let k = usize::from(n.min(14));
        let e = format!("{:.*e}", k, x.abs());
        let (mant, exp) = e.split_once('e').unwrap_or((e.as_str(), "0"));
        let exp: i32 = exp.parse().unwrap_or(0);
        if neg {
            s.push('-');
        }
        s.push_str(&mant.replace('.', &locale.decimal_sep.to_string()));
        s.push('E');
        s.push(if exp < 0 { '-' } else { '+' });
        let a = exp.unsigned_abs();
        if a < 10 {
            s.push('0');
        }
        s.push_str(&a.to_string());
        shown = x;
    } else {
        let mut d = Decimal::from_f64(x);
        d.round_frac(i32::from(n));
        if d.is_zero() {
            s.push('0');
            shown = 0.0;
        } else {
            push_fixed(&mut s, neg, &d, locale);
            shown = s.replace(locale.decimal_sep, ".").parse().unwrap_or(x);
        }
    }
    match width {
        Some(w) if s.chars().count() > w => general_string(shown, Some(w), locale),
        _ => s,
    }
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
    sec: &Section,
    locale: &Locale,
    epoch: DateEpoch,
    out: &mut Out,
    ctx: LangCtx<'_>,
) -> Result<(), ()> {
    let tokens = &sec.tokens[..];
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
    if lay.calendar || sec.natnum.is_some() {
        let p = DateParts {
            secs,
            frac,
            fd,
            hour,
            minute,
            second,
            y,
            m,
            d,
            wd,
            unix_days,
        };
        date_section_cal(&p, lay, sec, locale, out, ctx);
        if neg && (secs > 0 || frac > 0) {
            out.insert_sign();
        }
        return Ok(());
    }
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
            // Era codes set `DateLayout::calendar` (date_section_cal).
            DateTimeToken::Era(_) | DateTimeToken::EraYear(_) | DateTimeToken::EraAndYear(_) => {}
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

/// The fields of a date/time value, computed once in [`date_section`].
struct DateParts {
    secs: u64,
    frac: u64,
    fd: u32,
    hour: u64,
    minute: u64,
    second: u64,
    y: i64,
    m: u32,
    d: u32,
    wd: u32,
    unix_days: i64,
}

/// `ImpIntToString`: `v` zero-padded to `min` digits, transliterated with
/// the section's NatNum.
fn push_int_natnum(out: &mut Out, v: u64, min: usize, nat: Option<(LangId, u8)>, sep: Separators) {
    let s = v.to_string();
    let mut t = String::with_capacity(min.max(s.len()));
    for _ in s.len()..min {
        t.push('0');
    }
    t.push_str(&s);
    match nat {
        Some((lang, mode)) => out
            .text
            .push_str(&natnum::transliterate(&t, lang, mode, sep)),
        None => out.text.push_str(&t),
    }
}

/// `(abbreviated, full)` name `i` of a calendar table, or of the locale's.
fn name_at(
    table: Option<&'static [(&'static str, &'static str)]>,
    i: usize,
    short: &[&'static str],
    long: &[&'static str],
) -> (&'static str, &'static str) {
    match table {
        Some(t) => t.get(i).copied().unwrap_or(("", "")),
        None => (
            short.get(i).copied().unwrap_or(""),
            long.get(i).copied().unwrap_or(""),
        ),
    }
}

/// A date/time section with era codes, `[~calendar]` or a NatNum
/// (`SvNumberformat::ImpGetDateOutput` / `ImpGetDateTimeOutput` /
/// `ImpGetTimeOutput`): calendar fields come from
/// `Calendar_*::getDisplayString` with the section's NatNum, time fields
/// from `ImpIntToString`.
fn date_section_cal(
    p: &DateParts,
    lay: &DateLayout,
    sec: &Section,
    locale: &Locale,
    out: &mut Out,
    ctx: LangCtx<'_>,
) {
    let sep = separators(locale);
    let cal_lang = ctx.format_lang(locale);
    let gannen = ctx.gannen();
    let nat = section_natnum(sec, locale, ctx, lay.has_date);
    let mode = nat.map_or(0, |(_, m)| m);
    let int_nat = nat.filter(|&(l, m)| natnum::is_valid_natnum(l, m));
    let tokens = &sec.tokens;
    // ImpIsOtherCalendar: E, EE, R, RR, AAA, AAAA switch to the language's
    // other calendar unless a [~calendar] comes first.
    let implicit = tokens
        .iter()
        .find_map(|t| match t {
            Token::Calendar(_) => Some(false),
            Token::DateTime(
                DateTimeToken::EraYear(_)
                | DateTimeToken::EraAndYear(_)
                | DateTimeToken::AaaDayName(_),
            ) => Some(true),
            _ => None,
        })
        .unwrap_or(false);
    // The fields of each calendar, computed once (a code can switch
    // calendars with hundreds of `[~x]` tags).
    let mut cache: [Option<calendar::Fields>; 7] = [None; 7];
    let mut at = |cal: CalendarId| {
        *cache[cal as usize]
            .get_or_insert_with(|| calendar::fields(cal, p.y, p.m, p.d, p.unix_days, p.wd))
    };
    let greg = at(CalendarId::Gregorian);
    let mut cur = CalendarId::Gregorian;
    let mut other = false;
    if implicit && lay.has_date {
        if let Some(c) = calendar::other_calendar(cal_lang) {
            cur = c;
            other = true;
        }
    }
    let mut f = if cur == CalendarId::Gregorian {
        greg
    } else {
        at(cur)
    };
    // ImpFallBackToGregorianCalendar (a gengou date before Meiji).
    if calendar::falls_back(cur, &f) {
        cur = CalendarId::Gregorian;
        other = false;
        f = greg;
    }
    // The locale's Gregorian names unless the calendar language differs
    // and has its own (zh, ko, th, ar, he).
    let own_locale = locale_lang(locale).primary() == cal_lang.primary();
    let frac_chars: Vec<char> = if p.fd > 0 {
        let s = p.frac.to_string();
        let mut t = String::with_capacity(p.fd as usize);
        for _ in s.len()..p.fd as usize {
            t.push('0');
        }
        t.push_str(&s);
        match int_nat {
            Some((lang, m)) => natnum::transliterate(&t, lang, m, sep).chars().collect(),
            None => t.chars().collect(),
        }
    } else {
        Vec::new()
    };
    let mut frac_k = 0usize;
    let mut in_frac = false;
    let mut last_cal: Option<&str> = None;
    for t in tokens {
        if in_frac {
            if let Token::Digit(Digit::Zero) = t {
                if let Some(c) = frac_chars.get(frac_k) {
                    out.text.push(*c);
                }
                frac_k += 1;
                continue;
            }
            in_frac = false;
        }
        let dt = match t {
            Token::DateTime(dt) => dt,
            Token::DecimalPoint => {
                out.text.push(locale.decimal_sep);
                in_frac = true;
                frac_k = 0;
                continue;
            }
            Token::Calendar(name) => {
                // Repeating the tag last applied changes nothing.
                if last_cal == Some(name.as_str()) {
                    continue;
                }
                last_cal = Some(name.as_str());
                if let Some(c) = CalendarId::from_name(name) {
                    if calendar::has_calendar(cal_lang, c) {
                        cur = c;
                        f = at(c);
                        if calendar::falls_back(cur, &f) {
                            cur = CalendarId::Gregorian;
                            f = greg;
                        }
                    }
                }
                continue;
            }
            _ => {
                emit_literal(t, locale, out);
                continue;
            }
        };
        let num = |out: &mut Out, cal: CalendarId, fl: &calendar::Fields, code: NumCode| {
            out.text.push_str(&calendar::display_number(
                cal, cal_lang, fl, code, mode, gannen, sep,
            ));
        };
        let day_name = |cal: CalendarId, wd: u32| {
            name_at(
                calendar::day_names(cal, cal_lang),
                wd as usize,
                &locale.short_day_names,
                &locale.day_names,
            )
        };
        let era = |cal: CalendarId, fl: &calendar::Fields| -> (&'static str, &'static str) {
            let table = calendar::era_names(cal, cal_lang)
                .filter(|_| !(cal == CalendarId::Gregorian && own_locale));
            match table {
                Some(t) => t.get(fl.era).copied().unwrap_or(("", "")),
                None if fl.era >= 1 => (locale.era_abbrev.1, locale.era_names.1),
                None => (locale.era_abbrev.0, locale.era_names.0),
            }
        };
        match dt {
            DateTimeToken::Day(1) => num(out, cur, &f, NumCode::ShortDay),
            DateTimeToken::Day(_) => num(out, cur, &f, NumCode::LongDay),
            DateTimeToken::DayName(n) => {
                let cal = if other { CalendarId::Gregorian } else { cur };
                let (a, l) = day_name(cal, p.wd);
                out.text.push_str(if *n >= 4 { l } else { a });
            }
            DateTimeToken::AooDayName(n) => {
                let (a, l) = day_name(cur, p.wd);
                if *n == 2 {
                    out.text.push_str(a);
                } else {
                    out.text.push_str(l);
                    if *n >= 4 {
                        out.text.push_str(locale.long_date_day_of_week_sep);
                    }
                }
            }
            DateTimeToken::AaaDayName(n) => {
                let (a, l) = day_name(cur, p.wd);
                out.text.push_str(if *n >= 4 { l } else { a });
            }
            DateTimeToken::Month(1) => num(out, cur, &f, NumCode::ShortMonth),
            DateTimeToken::Month(2) => num(out, cur, &f, NumCode::LongMonth),
            DateTimeToken::Month(n) => {
                let (a, l) = name_at(
                    calendar::month_names(cur, cal_lang),
                    (f.month as usize).saturating_sub(1),
                    &locale.short_month_names,
                    &locale.month_names,
                );
                match n {
                    3 => out.text.push_str(a),
                    4 => out.text.push_str(l),
                    _ => {
                        if let Some(c) = l.chars().next() {
                            out.text.push(c);
                        }
                    }
                }
            }
            DateTimeToken::Year(n) => {
                let cal = if other { CalendarId::Gregorian } else { cur };
                let code = if *n <= 2 {
                    NumCode::ShortYear
                } else {
                    NumCode::LongYear
                };
                if cal == CalendarId::Gregorian {
                    // Tally's Gregorian years (as the plain date path).
                    let text = if *n <= 2 {
                        format!("{:02}", p.y.rem_euclid(100))
                    } else {
                        format!("{:04}", p.y.max(0))
                    };
                    out.text.push_str(&calendar::calendar_natnum(
                        text, cal, cal_lang, code, p.y, mode, sep,
                    ));
                } else {
                    num(out, cal, &f, code);
                }
            }
            DateTimeToken::EraYear(1) => num(out, cur, &f, NumCode::ShortYear),
            DateTimeToken::EraYear(_) | DateTimeToken::EraAndYear(1) => {
                num(out, cur, &f, NumCode::LongYear)
            }
            DateTimeToken::EraAndYear(_) => {
                let (a, l) = era(cur, &f);
                if cur == CalendarId::Buddhist && f.era == 0 {
                    num(out, cur, &f, NumCode::ShortYear);
                    out.text.push_str(a);
                } else {
                    out.text.push_str(l);
                    num(out, cur, &f, NumCode::LongYear);
                }
            }
            DateTimeToken::Era(n) => {
                let (a, l) = era(cur, &f);
                match n {
                    // ImpAppendEraG: the gengou era letter.
                    1 if cur == CalendarId::Gengou => {
                        out.text.push(['?', 'M', 'T', 'S', 'H', 'R'][f.era.min(5)]);
                    }
                    1 | 2 => out.text.push_str(a),
                    _ => out.text.push_str(l),
                }
            }
            DateTimeToken::Quarter(n) => {
                let q = ((f.month.saturating_sub(1)) / 3).min(3) as usize;
                let text = if *n >= 2 {
                    locale.quarter_names[q]
                } else {
                    locale.quarter_abbrev[q]
                };
                out.text.push_str(&calendar::calendar_natnum(
                    text.to_string(),
                    cur,
                    cal_lang,
                    NumCode::Quarter,
                    0,
                    mode,
                    sep,
                ));
            }
            DateTimeToken::Week(_) => push_int_natnum(
                out,
                u64::from(iso_week_number(p.unix_days)),
                0,
                int_nat,
                sep,
            ),
            DateTimeToken::Hour(n) => {
                let h = if lay.twelve_hour {
                    match p.hour % 12 {
                        0 => 12,
                        h => h,
                    }
                } else {
                    p.hour
                };
                push_int_natnum(out, h, usize::from(*n), int_nat, sep);
            }
            DateTimeToken::Minute(n) => {
                push_int_natnum(out, p.minute, usize::from(*n), int_nat, sep)
            }
            DateTimeToken::Second(n) => {
                push_int_natnum(out, p.second, usize::from(*n), int_nat, sep)
            }
            DateTimeToken::AmPm(_) => {
                // Calendar_hanja::getDisplayName: the ja locale's AM / PM.
                let (am, pm) = if cur == CalendarId::Hanja {
                    ("午前", "午後")
                } else {
                    locale.am_pm
                };
                out.text.push_str(if p.hour < 12 { am } else { pm });
            }
            DateTimeToken::AP(s) => {
                let lower = s.starts_with('a');
                out.text.push(match (p.hour < 12, lower) {
                    (true, false) => 'A',
                    (true, true) => 'a',
                    (false, false) => 'P',
                    (false, true) => 'p',
                });
            }
            DateTimeToken::Elapsed(unit, n) => {
                let v = match unit {
                    ElapsedUnit::Hours => p.secs / 3600,
                    ElapsedUnit::Minutes => p.secs / 60,
                    ElapsedUnit::Seconds => p.secs,
                };
                push_int_natnum(out, v, usize::from(*n), int_nat, sep);
            }
        }
    }
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
    fn general_limited_decimals() {
        // AOO "Limit decimals for general number format" (ImpGetOutputStandard).
        let l = Locale::en_us();
        let g = |x: f64, n: u8| general_string_limited(x, None, &l, Some(n));
        assert_eq!(g(std::f64::consts::PI, 2), "3.14");
        assert_eq!(g(std::f64::consts::PI, 0), "3");
        assert_eq!(g(1.5, 2), "1.5");
        assert_eq!(g(2.0, 2), "2");
        assert_eq!(g(-0.004, 2), "0");
        assert_eq!(g(-2.346, 2), "-2.35");
        assert_eq!(g(0.00001, 2), "0");
        assert_eq!(g(0.00001, 5), "0.00001");
        assert_eq!(g(1234567.891, 1), "1234567.9");
        assert_eq!(g(1e16, 2), "1.00E+16");
        assert_eq!(g(-2.5e20, 0), "-2E+20");
        assert_eq!(
            general_string_limited(1.0 / 3.0, None, &l, None),
            "0.333333333333333"
        );
        // A narrow column still shrinks the rounded number.
        assert_eq!(
            general_string_limited(1234.5678, Some(6), &l, Some(3)),
            "1234.6"
        );
        // The document decimal separator.
        let de = crate::locale_by_tag("de-DE").unwrap();
        assert_eq!(general_string_limited(2.346, None, de, Some(2)), "2,35");
        // Through the formatter: General and a General with literals.
        let ast = crate::parse_format("General").unwrap();
        let v = Value::Number(std::f64::consts::PI);
        assert_eq!(
            format_value_std(&v, &ast, &l, None, DateEpoch::D1899, Some(2)).text,
            "3.14"
        );
        assert_eq!(
            format_value_with(&v, &ast, &l, None, DateEpoch::D1899).text,
            "3.14159265358979"
        );
        let ast = crate::parse_format("General\" m\"").unwrap();
        assert_eq!(
            format_value_std(&v, &ast, &l, None, DateEpoch::D1899, Some(1)).text,
            "3.1 m"
        );
        // Fixed-decimal formats are not affected.
        let ast = crate::parse_format("0.000").unwrap();
        assert_eq!(
            format_value_std(&v, &ast, &l, None, DateEpoch::D1899, Some(1)).text,
            "3.142"
        );
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
