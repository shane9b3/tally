//! Unit conversions at the API edge (96 dpi CSS pixels ↔ 1/100 mm).

/// Default column width in pixels (matches `@tally/grid`).
pub const DEFAULT_COL_WIDTH_PX: f32 = 100.0;
/// Default row height in pixels (matches `@tally/grid`).
pub const DEFAULT_ROW_HEIGHT_PX: f32 = 24.0;

/// 1/100 mm per CSS pixel at 96 dpi (25.4 mm / 96 × 100).
const HMM_PER_PX: f64 = 2540.0 / 96.0;

/// Pixels to 1/100 mm (rounded).
pub fn px_to_hmm(px: f32) -> u32 {
    let v = (f64::from(px.max(0.0)) * HMM_PER_PX).round();
    if v >= f64::from(u32::MAX) {
        u32::MAX
    } else {
        v as u32
    }
}

/// 1/100 mm to pixels.
pub fn hmm_to_px(hmm: u32) -> f32 {
    (f64::from(hmm) / HMM_PER_PX) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        assert_eq!(px_to_hmm(96.0), 2540);
        assert!((hmm_to_px(2540) - 96.0).abs() < 1e-4);
        for px in [1.0f32, 24.0, 64.0, 100.0, 333.0] {
            assert!((hmm_to_px(px_to_hmm(px)) - px).abs() < 0.05);
        }
    }
}
