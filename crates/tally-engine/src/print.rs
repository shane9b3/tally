//! Pagination and print jobs (wave 5, W5-3). All sizes are in 1/100 mm,
//! converted to points (1 mm = 72/25.4 pt) when building the job.
//!
//! TODO(W5-3):
//! - area: the print range, else the used area plus visible formatting
//!   and chart bounds;
//! - body: paper minus margins minus the header/footer blocks;
//! - scale: `Percent`; `FitTo{w,h}` searches 100 % → 10 % for the largest
//!   integer percent with pages across ≤ w and down ≤ h (0 = unlimited);
//!   `FitPages{n}` the largest percent with total pages ≤ n;
//! - breaks: a page holds at least one row/column, hidden rows/columns
//!   are skipped, manual breaks force a page, repeat rows/columns take
//!   space on every page after the first that naturally holds them;
//! - page order top-to-bottom (default) or left-to-right; empty pages
//!   suppressed; centring; `&[PAGE]`-style placeholders;
//! - `PrintJob` items for `tally-io-pdf`, an LRU of 4 jobs, PNG preview
//!   through hayro, `page_breaks` for Page Break Preview, and
//!   `document_statistics.pages`.

/// Points per millimetre.
pub const PT_PER_MM: f32 = 72.0 / 25.4;

/// Most jobs kept per process.
pub const JOB_LRU: usize = 4;

/// Converts 1/100 mm to points.
pub fn hmm_to_pt(hmm: u32) -> f32 {
    hmm as f32 / 100.0 * PT_PER_MM
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a4_width_in_points() {
        assert!((hmm_to_pt(21_000) - 595.28).abs() < 0.01);
    }
}
