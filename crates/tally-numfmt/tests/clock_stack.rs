//! The first read of the local clock in a process fits a small stack.
//!
//! That read finds the system time zone and parses its TZif file; later
//! reads use the cached zone. Unoptimized, the CRC-32 of that parse had a
//! 256 KiB stack frame, so the first `NOW()` (or the first recalc of any
//! document, which reads the clock once) overflowed a thread with a small
//! stack. Which thread read the clock first depended on scheduling, so
//! the overflow was intermittent in test binaries with many tests. The
//! workspace `Cargo.toml` therefore builds `jiff-core` optimized in the
//! dev profile.
//!
//! This file holds one test on purpose: it must make the first clock read
//! of its process.

use tally_numfmt::date::{now_serial, today_ymd};
use tally_types::DateEpoch;

/// Far below the 256 KiB the unoptimized frame needed, far above what the
/// optimized read needs (about 10 KiB).
const SMALL_STACK: usize = 96 * 1024;

#[test]
fn first_clock_read_fits_a_small_stack() {
    let h = std::thread::Builder::new()
        .stack_size(SMALL_STACK)
        .spawn(|| (now_serial(DateEpoch::D1899), today_ymd()))
        .expect("spawn");
    let (serial, (year, _, _)) = h.join().expect("the clock read panicked");
    // 2020-01-01 is serial 43831.
    assert!(serial > 43_831.0, "serial {serial}");
    assert!(year >= 2020, "year {year}");
}
