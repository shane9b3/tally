//! Length conversions between OOXML units and the model's 1/100 mm.
//!
//! Screen pixels are at 96 dpi (1 px = 2540/96 hmm). Column widths are
//! Excel "character" widths as stored in `<col width>`, which already
//! include the 5 px cell padding, with Calibri 11's maximum digit width of
//! 7 px (ECMA-376 §18.3.1.13): `px = trunc((256·width + trunc(128/7)) / 256
//! · 7)`. The often-quoted `width·7 + 5` applies to the *displayed*
//! character count (8.43 for a default column, stored as 9.140625).

/// Maximum digit width of the default font (Calibri 11) in pixels.
pub(crate) const MDW: f64 = 7.0;

/// `<col width>` → pixels.
pub(crate) fn col_width_to_px(width: f64) -> f64 {
    if !width.is_finite() || width <= 0.0 {
        return 0.0;
    }
    ((256.0 * width + (128.0 / MDW).trunc()) / 256.0 * MDW).trunc()
}

/// Pixels → 1/100 mm.
pub(crate) fn px_to_hmm(px: f64) -> u32 {
    clamp_u32(px * 2540.0 / 96.0)
}

/// 1/100 mm → pixels (rounded).
pub(crate) fn hmm_to_px(hmm: u32) -> u32 {
    clamp_u32(f64::from(hmm) * 96.0 / 2540.0)
}

/// Points → 1/100 mm.
pub(crate) fn pt_to_hmm(pt: f64) -> u32 {
    clamp_u32(pt * 2540.0 / 72.0)
}

fn clamp_u32(v: f64) -> u32 {
    let r = v.round();
    if r.is_finite() && r > 0.0 {
        if r >= f64::from(u32::MAX) {
            u32::MAX
        } else {
            r as u32
        }
    } else {
        0
    }
}

/// Parses an OOXML boolean attribute (`1`, `true`, `on`).
pub(crate) fn parse_bool(s: &str) -> bool {
    matches!(s.trim(), "1" | "true" | "True" | "TRUE" | "on")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widths() {
        // Excel's default column: 8.43 characters, stored 9.140625, 64 px.
        assert_eq!(col_width_to_px(9.140625), 64.0);
        assert_eq!(px_to_hmm(64.0), 1693);
        assert_eq!(hmm_to_px(1693), 64);
        // 20 characters displayed (width 20.7109375) = 145 px.
        assert_eq!(col_width_to_px(20.7109375), 145.0);
        assert_eq!(col_width_to_px(-1.0), 0.0);
        assert_eq!(col_width_to_px(f64::NAN), 0.0);
    }

    #[test]
    fn heights() {
        assert_eq!(pt_to_hmm(15.0), 529);
        assert_eq!(pt_to_hmm(72.0), 2540);
        assert_eq!(hmm_to_px(529), 20);
    }

    #[test]
    fn bools() {
        assert!(parse_bool("1") && parse_bool("true") && !parse_bool("0"));
        assert!(!parse_bool("false") && !parse_bool(""));
    }
}
