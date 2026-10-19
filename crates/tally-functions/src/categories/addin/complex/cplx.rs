//! Complex-number core shared by the IM* functions.
//!
//! Ported from Apache OpenOffice `scaddins/source/analysis/analysishelper.cxx`
//! (class `Complex`), Apache-2.0.

use tally_types::{ErrorKind, Value};

use crate::coerce::number_to_text;
use crate::registry::{Param, ParamKind};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

/// Every failure of the add-in (bad text, domain error, mixed suffix) is
/// `Err:502`, like AOO's `IllegalArgumentException`.
pub(super) const BAD: ErrorKind = ErrorKind::InvalidArgument;

/// Parameters of a one-argument complex function.
pub(super) const UNARY_PARAMS: &[Param] = &[Param::required("complex_number", ParamKind::Text)];

/// A complex number with its (optional) imaginary-unit suffix.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(super) struct Cx {
    pub r: f64,
    pub i: f64,
    /// `Some('i')` or `Some('j')` if the text carried an imaginary part with that unit.
    pub suffix: Option<char>,
}

fn parse_double(s: &[u8], p: &mut usize) -> Option<f64> {
    let start = *p;
    let mut q = *p;
    if q < s.len() && (s[q] == b'+' || s[q] == b'-') {
        q += 1;
    }
    let mut digits = 0;
    while q < s.len() && s[q].is_ascii_digit() {
        q += 1;
        digits += 1;
    }
    if q < s.len() && s[q] == b'.' {
        q += 1;
        while q < s.len() && s[q].is_ascii_digit() {
            q += 1;
            digits += 1;
        }
    }
    if digits == 0 {
        return None;
    }
    if q < s.len() && (s[q] == b'e' || s[q] == b'E') {
        let mut e = q + 1;
        if e < s.len() && (s[e] == b'+' || s[e] == b'-') {
            e += 1;
        }
        if e < s.len() && s[e].is_ascii_digit() {
            while e < s.len() && s[e].is_ascii_digit() {
                e += 1;
            }
            q = e;
        }
    }
    let text = std::str::from_utf8(&s[start..q]).ok()?;
    let v: f64 = text.parse().ok()?;
    *p = q;
    v.is_finite().then_some(v)
}

fn unit(s: &[u8], p: usize) -> Option<char> {
    match s.get(p) {
        Some(b'i') => Some('i'),
        Some(b'j') => Some('j'),
        _ => None,
    }
}

impl Cx {
    /// A real number.
    pub fn real(r: f64) -> Self {
        Cx {
            r,
            i: 0.0,
            suffix: None,
        }
    }

    /// Parses `3+4i`, `4j`, `-i`, `1e2+3.5e-1i`, `7`; the empty text is 0.
    pub fn parse(text: &str) -> Result<Cx, ErrorKind> {
        let s = text.as_bytes();
        if s.is_empty() {
            return Ok(Cx::real(0.0));
        }
        let mk = |r, i, c| {
            Ok(Cx {
                r,
                i,
                suffix: Some(c),
            })
        };
        // "i", "j", "+i", "-i"
        if let Some(c) = unit(s, 0) {
            if s.len() == 1 {
                return mk(0.0, 1.0, c);
            }
        }
        if s.len() == 2 && (s[0] == b'+' || s[0] == b'-') {
            if let Some(c) = unit(s, 1) {
                return mk(0.0, if s[0] == b'+' { 1.0 } else { -1.0 }, c);
            }
        }
        let mut p = 0;
        let f = parse_double(s, &mut p).ok_or(BAD)?;
        if p == s.len() {
            return Ok(Cx::real(f));
        }
        if let Some(c) = unit(s, p) {
            return if p + 1 == s.len() {
                mk(0.0, f, c)
            } else {
                Err(BAD)
            };
        }
        if s[p] == b'+' || s[p] == b'-' {
            let sign = if s[p] == b'+' { 1.0 } else { -1.0 };
            let im = if let (Some(c), true) = (unit(s, p + 1), p + 2 == s.len()) {
                return mk(f, sign, c);
            } else {
                parse_double(s, &mut p).ok_or(BAD)?
            };
            if let (Some(c), true) = (unit(s, p), p + 1 == s.len()) {
                return mk(f, im, c);
            }
        }
        Err(BAD)
    }

    /// AOO `Complex::GetString`; the unit defaults to `i`.
    pub fn format(&self) -> Result<String, ErrorKind> {
        if !self.r.is_finite() || !self.i.is_finite() {
            return Err(ErrorKind::Num);
        }
        let has_imag = self.i != 0.0;
        let has_real = !has_imag || self.r != 0.0;
        let mut out = String::new();
        if has_real {
            out.push_str(&number_to_text(self.r));
        }
        if has_imag {
            if self.i == 1.0 {
                if has_real {
                    out.push('+');
                }
            } else if self.i == -1.0 {
                out.push('-');
            } else {
                if has_real && self.i > 0.0 {
                    out.push('+');
                }
                out.push_str(&number_to_text(self.i));
            }
            out.push(self.suffix.unwrap_or('i'));
        }
        Ok(out)
    }

    /// The combined suffix of two operands; two different units are an error.
    pub fn merge_suffix(a: Option<char>, b: Option<char>) -> Result<Option<char>, ErrorKind> {
        match (a, b) {
            (Some(x), Some(y)) if x != y => Err(BAD),
            (Some(x), _) | (_, Some(x)) => Ok(Some(x)),
            _ => Ok(None),
        }
    }

    fn with(&self, r: f64, i: f64) -> Cx {
        Cx {
            r,
            i,
            suffix: self.suffix,
        }
    }

    /// Modulus.
    pub fn abs(&self) -> f64 {
        (self.r * self.r + self.i * self.i).sqrt()
    }

    /// Argument (angle); undefined for 0.
    pub fn arg(&self) -> Result<f64, ErrorKind> {
        if self.r == 0.0 && self.i == 0.0 {
            return Err(BAD);
        }
        let phi = (self.r / self.abs()).acos();
        Ok(if self.i < 0.0 { -phi } else { phi })
    }

    pub fn conj(&self) -> Result<Cx, ErrorKind> {
        Ok(self.with(self.r, -self.i))
    }

    pub fn add(&self, o: &Cx) -> Result<Cx, ErrorKind> {
        Ok(Cx {
            r: self.r + o.r,
            i: self.i + o.i,
            suffix: Cx::merge_suffix(self.suffix, o.suffix)?,
        })
    }

    pub fn sub(&self, o: &Cx) -> Result<Cx, ErrorKind> {
        Ok(Cx {
            r: self.r - o.r,
            i: self.i - o.i,
            suffix: Cx::merge_suffix(self.suffix, o.suffix)?,
        })
    }

    pub fn mul(&self, o: &Cx) -> Result<Cx, ErrorKind> {
        Ok(Cx {
            r: self.r * o.r - self.i * o.i,
            i: self.r * o.i + self.i * o.r,
            suffix: Cx::merge_suffix(self.suffix, o.suffix)?,
        })
    }

    pub fn div(&self, o: &Cx) -> Result<Cx, ErrorKind> {
        let suffix = Cx::merge_suffix(self.suffix, o.suffix)?;
        if o.r == 0.0 && o.i == 0.0 {
            return Err(BAD);
        }
        let den = o.r * o.r + o.i * o.i;
        Ok(Cx {
            r: (self.r * o.r + self.i * o.i) / den,
            i: (self.i * o.r - self.r * o.i) / den,
            suffix,
        })
    }

    /// Reciprocal (`1 / self`) keeping this number's suffix.
    pub fn recip(&self) -> Result<Cx, ErrorKind> {
        let one = Cx::real(1.0);
        let mut q = one.div(self)?;
        q.suffix = self.suffix;
        Ok(q)
    }
}

/// Evaluates a one-argument complex function: parses argument 0, applies
/// `op`, formats the result as text.
pub(super) fn unary(
    ctx: &mut dyn EvalContext,
    args: &mut dyn ArgList,
    op: fn(&Cx) -> Result<Cx, ErrorKind>,
) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let z = Cx::parse(&a.text(0)?)?;
        Ok(Value::text(op(&z)?.format()?))
    })
}

/// Parses argument `i` as a complex number.
pub(super) fn arg(a: &mut Args<'_>, i: usize) -> Result<Cx, ErrorKind> {
    Cx::parse(&a.text(i)?)
}

/// Test helper: text of a complex-valued call.
#[cfg(test)]
pub(super) fn call_text(name: &str, args: Vec<EvalValue>) -> Value {
    let mut c = crate::mock::MockEvalContext::new();
    crate::mock::call(&mut c, name, args)
}
