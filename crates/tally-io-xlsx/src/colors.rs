//! OOXML colours: `rgb="FFRRGGBB"`, `theme="N"` (with `tint`), `indexed="N"`.
//!
//! Theme colours resolve against a fixed copy of the default Office theme
//! (2013–2022 "Office" palette) instead of `xl/theme/theme1.xml`. Model
//! colours are `0xRRGGBBAA` with alpha `ff` (OOXML alpha is ignored, as
//! Excel does for cell colours).

/// Default Office theme in SpreadsheetML theme-index order: lt1, dk1, lt2,
/// dk2, accent1..6, hlink, folHlink (indices 0/1 and 2/3 are swapped
/// relative to the theme XML, as Excel does).
pub(crate) const THEME: [u32; 12] = [
    0xFFFFFF, 0x000000, 0xE7E6E6, 0x44546A, 0x4472C4, 0xED7D31, 0xA5A5A5, 0xFFC000, 0x5B9BD5,
    0x70AD47, 0x0563C1, 0x954F72,
];

/// The legacy 64-colour indexed palette (`indexed="N"`).
pub(crate) const INDEXED: [u32; 64] = [
    0x000000, 0xFFFFFF, 0xFF0000, 0x00FF00, 0x0000FF, 0xFFFF00, 0xFF00FF, 0x00FFFF, // 0-7
    0x000000, 0xFFFFFF, 0xFF0000, 0x00FF00, 0x0000FF, 0xFFFF00, 0xFF00FF, 0x00FFFF, // 8-15
    0x800000, 0x008000, 0x000080, 0x808000, 0x800080, 0x008080, 0xC0C0C0, 0x808080, // 16-23
    0x9999FF, 0x993366, 0xFFFFCC, 0xCCFFFF, 0x660066, 0xFF8080, 0x0066CC, 0xCCCCFF, // 24-31
    0x000080, 0xFF00FF, 0xFFFF00, 0x00FFFF, 0x800080, 0x800000, 0x008080, 0x0000FF, // 32-39
    0x00CCFF, 0xCCFFFF, 0xCCFFCC, 0xFFFF99, 0x99CCFF, 0xFF99CC, 0xCC99FF, 0xFFCC99, // 40-47
    0x3366FF, 0x33CCCC, 0x99CC00, 0xFFCC00, 0xFF9900, 0xFF6600, 0x666699, 0x969696, // 48-55
    0x003366, 0x339966, 0x003300, 0x333300, 0x993300, 0x993366, 0x333399, 0x333333, // 56-63
];

/// A colour as written in an OOXML `<color>`-like element.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct XColor {
    pub rgb: Option<u32>,
    pub theme: Option<u32>,
    pub indexed: Option<u32>,
    pub tint: f64,
    pub auto: bool,
}

impl XColor {
    /// Reads the colour attributes of an element.
    pub fn from_attrs(attr: impl Fn(&str) -> Option<String>) -> Self {
        XColor {
            rgb: attr("rgb").and_then(|s| parse_argb(&s)),
            theme: attr("theme").and_then(|s| s.trim().parse().ok()),
            indexed: attr("indexed").and_then(|s| s.trim().parse().ok()),
            tint: attr("tint")
                .and_then(|s| s.trim().parse::<f64>().ok())
                .filter(|t| t.is_finite())
                .unwrap_or(0.0)
                .clamp(-1.0, 1.0),
            auto: attr("auto").is_some_and(|s| crate::units::parse_bool(&s)),
        }
    }

    /// `true` for the automatic / default text colour (`auto="1"`, theme 1
    /// without tint, system indexed colours 64/65).
    pub fn is_automatic(&self) -> bool {
        self.auto
            || (self.rgb.is_none() && self.theme == Some(1) && self.tint == 0.0)
            || (self.rgb.is_none() && self.theme.is_none() && matches!(self.indexed, Some(64 | 65)))
            || (self.rgb.is_none() && self.theme.is_none() && self.indexed.is_none())
    }

    /// The colour as `0xRRGGBBAA`, or `None` if it does not name one.
    pub fn resolve(&self, indexed: &[u32]) -> Option<u32> {
        let rgb = if let Some(rgb) = self.rgb {
            rgb
        } else if let Some(t) = self.theme {
            *THEME.get(t as usize)?
        } else if let Some(i) = self.indexed {
            *indexed.get(i as usize)?
        } else {
            return None;
        };
        Some((apply_tint(rgb, self.tint) << 8) | 0xff)
    }
}

/// Parses `FFRRGGBB` (ARGB) or `RRGGBB` into `0xRRGGBB`.
pub(crate) fn parse_argb(s: &str) -> Option<u32> {
    let s = s.trim();
    let hex = match s.len() {
        8 => s.get(2..)?,
        6 => s,
        _ => return None,
    };
    u32::from_str_radix(hex, 16).ok()
}

/// Applies an OOXML tint (−1..1) to `0xRRGGBB` in HLS space (ECMA-376
/// §18.8.19): negative tints darken, positive tints lighten.
pub(crate) fn apply_tint(rgb: u32, tint: f64) -> u32 {
    if tint == 0.0 {
        return rgb;
    }
    let (h, l, s) = rgb_to_hls(rgb);
    let l = if tint < 0.0 {
        l * (1.0 + tint)
    } else {
        l * (1.0 - tint) + tint
    };
    hls_to_rgb(h, l.clamp(0.0, 1.0), s)
}

fn rgb_to_hls(rgb: u32) -> (f64, f64, f64) {
    let r = f64::from((rgb >> 16) & 0xff) / 255.0;
    let g = f64::from((rgb >> 8) & 0xff) / 255.0;
    let b = f64::from(rgb & 0xff) / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    if max == min {
        return (0.0, l, 0.0);
    }
    let d = max - min;
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let h = if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    } / 6.0;
    (h, l, s)
}

fn hls_to_rgb(h: f64, l: f64, s: f64) -> u32 {
    let to_byte = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
    if s == 0.0 {
        let v = to_byte(l);
        return (v << 16) | (v << 8) | v;
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let hue = |mut t: f64| {
        t = t.rem_euclid(1.0);
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    (to_byte(hue(h + 1.0 / 3.0)) << 16) | (to_byte(hue(h)) << 8) | to_byte(hue(h - 1.0 / 3.0))
}

/// `0xRRGGBBAA` → `0xRRGGBB`.
pub(crate) fn model_to_rgb(c: u32) -> u32 {
    c >> 8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argb() {
        assert_eq!(parse_argb("FF112233"), Some(0x112233));
        assert_eq!(parse_argb("00112233"), Some(0x112233));
        assert_eq!(parse_argb("112233"), Some(0x112233));
        assert_eq!(parse_argb("xyz"), None);
    }

    #[test]
    fn theme_and_tint() {
        let c = XColor {
            theme: Some(4),
            ..XColor::default()
        };
        assert_eq!(c.resolve(&INDEXED), Some(0x4472c4ff));
        // White darkened by 15% (Excel's "White, Background 1, Darker 15%").
        assert_eq!(apply_tint(0xFFFFFF, -0.1499984740745262), 0xD9D9D9);
        // Black lightened by 50%.
        assert_eq!(apply_tint(0x000000, 0.499984740745262), 0x7F7F7F);
        assert_eq!(apply_tint(0x4472C4, 0.0), 0x4472C4);
    }

    #[test]
    fn automatic() {
        assert!(XColor {
            theme: Some(1),
            ..XColor::default()
        }
        .is_automatic());
        assert!(XColor {
            indexed: Some(64),
            ..XColor::default()
        }
        .is_automatic());
        assert!(!XColor {
            rgb: Some(0),
            ..XColor::default()
        }
        .is_automatic());
        let c = XColor {
            indexed: Some(10),
            ..XColor::default()
        };
        assert_eq!(c.resolve(&INDEXED), Some(0xff0000ff));
    }
}
