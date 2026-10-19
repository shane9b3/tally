//! A small decimal digit buffer used for display rounding.
//!
//! Numbers are first rounded to 15 significant digits (exactly what
//! `tally_types::round_to_15_digits` does) and then rounded half away from
//! zero on the decimal digits. This reproduces OOo's corrected
//! `rtl::math::round`: `2.675` shows as `2.68` and `0.125` as `0.13`, even
//! though their binary values sit just below or exactly on the tie.

use std::fmt::Write;

/// Maximum significant digits kept.
const MAX_DIGITS: usize = 15;

/// A non-negative decimal `0.d1 d2 … dn × 10^point`, i.e. the first `point`
/// digits are the integer part. Digits are values 0..=9 with no leading or
/// trailing zeros; zero has `len == 0`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Decimal {
    digits: [u8; MAX_DIGITS + 1],
    len: usize,
    /// Position of the decimal point counted from the first digit.
    pub point: i32,
}

/// A fixed-capacity stack buffer for `write!`.
struct StackBuf {
    buf: [u8; 48],
    len: usize,
}

impl Write for StackBuf {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        let bytes = s.as_bytes();
        let end = self.len + bytes.len();
        if end > self.buf.len() {
            return Err(std::fmt::Error);
        }
        self.buf[self.len..end].copy_from_slice(bytes);
        self.len = end;
        Ok(())
    }
}

impl Decimal {
    /// Zero.
    pub const ZERO: Decimal = Decimal {
        digits: [0; MAX_DIGITS + 1],
        len: 0,
        point: 0,
    };

    /// The magnitude of `x` rounded to 15 significant digits. Non-finite
    /// input yields zero (callers handle NaN/infinity first).
    pub fn from_f64(x: f64) -> Decimal {
        let x = x.abs();
        if x == 0.0 || !x.is_finite() {
            return Self::ZERO;
        }
        let mut sb = StackBuf {
            buf: [0; 48],
            len: 0,
        };
        if write!(sb, "{x:.14e}").is_err() {
            return Self::ZERO;
        }
        let s = &sb.buf[..sb.len];
        let mut d = Self::ZERO;
        let mut i = 0;
        let mut exp: i32 = 0;
        while i < s.len() {
            let c = s[i];
            if c.is_ascii_digit() {
                if d.len < MAX_DIGITS {
                    d.digits[d.len] = c - b'0';
                    d.len += 1;
                }
            } else if c == b'e' {
                let rest = &s[i + 1..];
                let (neg, rest) = match rest.first() {
                    Some(b'-') => (true, &rest[1..]),
                    _ => (false, rest),
                };
                for &e in rest {
                    exp = exp * 10 + i32::from(e - b'0');
                }
                if neg {
                    exp = -exp;
                }
                break;
            }
            i += 1;
        }
        d.point = exp + 1;
        d.trim();
        d
    }

    /// Removes trailing zero digits (and normalises zero).
    fn trim(&mut self) {
        while self.len > 0 && self.digits[self.len - 1] == 0 {
            self.len -= 1;
        }
        if self.len == 0 {
            self.point = 0;
        }
    }

    /// Whether the value is zero.
    pub fn is_zero(&self) -> bool {
        self.len == 0
    }

    /// Number of significant digits.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Digit at index `i` counted from the first significant digit (0 for
    /// positions past either end).
    pub fn digit(&self, i: i32) -> u8 {
        if i < 0 || i as usize >= self.len {
            0
        } else {
            self.digits[i as usize]
        }
    }

    /// Number of integer digits (0 when the value is below 1).
    pub fn int_len(&self) -> usize {
        if self.len == 0 || self.point <= 0 {
            0
        } else {
            self.point as usize
        }
    }

    /// Integer digit at position `r` counted from the right (units = 0).
    pub fn int_digit_from_right(&self, r: usize) -> u8 {
        let n = self.int_len();
        if r >= n {
            0
        } else {
            self.digit((n - 1 - r) as i32)
        }
    }

    /// Fraction digit `k` (0 = tenths).
    pub fn frac_digit(&self, k: usize) -> u8 {
        self.digit(self.point + k as i32)
    }

    /// Number of fraction digits that are significant.
    pub fn frac_len(&self) -> usize {
        let l = self.len as i32 - self.point;
        if l > 0 {
            l as usize
        } else {
            0
        }
    }

    /// Keeps the first `keep` digits, rounding half away from zero.
    fn round_keep(&mut self, keep: i32) {
        if keep >= self.len as i32 {
            return;
        }
        if keep < 0 {
            *self = Self::ZERO;
            return;
        }
        let keep = keep as usize;
        let up = self.digits[keep] >= 5;
        self.len = keep;
        if up {
            let mut i = keep;
            loop {
                if i == 0 {
                    // Carry out of the leading digit: 99.6 -> 100.
                    self.digits[0] = 1;
                    self.len = 1;
                    self.point += 1;
                    break;
                }
                i -= 1;
                if self.digits[i] == 9 {
                    self.digits[i] = 0;
                } else {
                    self.digits[i] += 1;
                    break;
                }
            }
        }
        self.trim();
    }

    /// Rounds to `f` fraction digits (negative `f` rounds to tens etc.).
    pub fn round_frac(&mut self, f: i32) {
        if self.len == 0 {
            return;
        }
        self.round_keep(self.point.saturating_add(f));
    }

    /// Rounds to `s` significant digits (`s >= 1`).
    pub fn round_sig(&mut self, s: usize) {
        if self.len == 0 {
            return;
        }
        self.round_keep(s.max(1) as i32);
    }

    /// Shifts the decimal point: multiplies by `10^k`.
    pub fn shift(&mut self, k: i32) {
        if self.len > 0 {
            self.point = self.point.saturating_add(k);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(d: &Decimal) -> String {
        let mut out = String::new();
        for i in 0..d.len {
            out.push((b'0' + d.digits[i]) as char);
        }
        format!("{out}@{}", d.point)
    }

    #[test]
    fn parse() {
        assert_eq!(s(&Decimal::from_f64(123.45)), "12345@3");
        assert_eq!(s(&Decimal::from_f64(0.00123)), "123@-2");
        assert_eq!(s(&Decimal::from_f64(0.1 + 0.2)), "3@0");
        assert_eq!(s(&Decimal::from_f64(1e300)), "1@301");
    }

    #[test]
    fn rounding() {
        let mut d = Decimal::from_f64(2.675);
        d.round_frac(2);
        assert_eq!(s(&d), "268@1");
        let mut d = Decimal::from_f64(99.96);
        d.round_frac(1);
        assert_eq!(s(&d), "1@3");
        let mut d = Decimal::from_f64(0.4);
        d.round_frac(0);
        assert!(d.is_zero());
        let mut d = Decimal::from_f64(0.5);
        d.round_frac(0);
        assert_eq!(s(&d), "1@1");
        let mut d = Decimal::from_f64(0.004);
        d.round_frac(2);
        assert!(d.is_zero());
    }
}
