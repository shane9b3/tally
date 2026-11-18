//! Short date formats of a `.xls`: the built-in format 14 and a custom
//! `FORMAT` record `m/d/yyyy` (what Excel writes for a US short date).
//!
//! OpenOffice 4.1 reads both as `M/D/YYYY` on an English (US) system
//! (`spBuiltInFormats_ENGLISH_US`, xlstyle.cxx) and then converts the code
//! from English (US) to the *system* locale
//! (`XclImpNumFmtBuffer::CreateScFormats` → `PutandConvertEntry`), which
//! replaces the date separator `/` by the system locale's: under English
//! (Canada), whose separator is `-`, a cell that shows `3/15/2023` here
//! shows `3-15-2023` there.
//! Tally has no system locale: a document without a locale of its own is
//! English (US), so the code stays `M/D/YYYY` (docs/parity-deviations.md
//! X26).
#![allow(clippy::unwrap_used)]

mod biff;

use tally_io_xls::{read_xls, Limits};
use tally_model::CellContentData;
use tally_types::Value;

#[test]
fn us_short_dates_keep_their_slashes() {
    let custom = biff::Xf {
        fmt: 164,
        ..biff::Xf::default()
    };
    let builtin = biff::Xf {
        fmt: 14,
        ..biff::Xf::default()
    };
    let with_time = biff::Xf {
        fmt: 22,
        ..biff::Xf::default()
    };
    let g = biff::globals(&[], &[(164, "m/d/yyyy")], &[custom, builtin, with_time]);
    // 2023-03-15 in Excel's 1900 date system.
    let mut s = biff::number(0, 0, 16, 45_000.0);
    s.extend(biff::number(0, 1, 17, 45_000.0));
    s.extend(biff::number(0, 2, 18, 45_000.5));
    let wb = read_xls(
        &biff::file(biff::stream(&g, &[("S", s)])),
        &Limits::default(),
    )
    .unwrap();
    assert_eq!(wb.warnings, Vec::<String>::new());
    let codes: Vec<Option<&str>> = wb.sheets[0]
        .cells
        .iter()
        .map(|c| {
            wb.styles[c.style.unwrap() as usize]
                .number_format
                .as_deref()
        })
        .collect();
    assert_eq!(
        codes,
        [Some("M/D/YYYY"), Some("M/D/YYYY"), Some("M/D/YYYY H:MM")]
    );
    // Dates, not plain numbers: the same serial in Tally's date system.
    assert_eq!(
        wb.sheets[0].cells[0].content,
        CellContentData::Value(Value::Number(45_000.0))
    );
}
