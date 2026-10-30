//! Reading committed fixtures that mimic LibreOffice / AOO output shapes.
//! Regenerate with `python3 -I tests/fixtures/ods/gen_fixtures.py`.

#![allow(clippy::unwrap_used)] // tests may unwrap (docs/contributing.md)

mod common;

use std::fs::File;
use std::io::{BufReader, Cursor};

use common::{cell, content, fixture, read_fixture, style};
use tally_io_ods::{read_ods, sniff_is_ods, write_ods, Limits};
use tally_model::*;
use tally_types::{CriteriaMode, DateEpoch, ErrorKind, IterSettings, Value};

fn open(rel: &str) -> WorkbookData {
    let f = BufReader::new(File::open(fixture(rel)).unwrap());
    read_ods(f, &Limits::default()).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn val(x: Value) -> CellContentData {
    CellContentData::Value(x)
}

fn formula(text: &str, cached: Value) -> CellContentData {
    CellContentData::Formula {
        text: text.into(),
        cached: Some(cached),
    }
}

fn number_format(wb: &WorkbookData, s: usize, r: u32, c: u32) -> Option<&str> {
    style(wb, s, r, c).and_then(|st| st.number_format.as_deref())
}

#[test]
fn libreoffice_shapes() {
    let wb = open("ods/libreoffice-style.ods");
    assert_eq!(
        wb.sheets
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>(),
        ["Orders", "Summary & Notes"]
    );
    assert_eq!(wb.settings.active_sheet, 1);
    let calc = &wb.settings.calc_settings;
    assert_eq!(calc.criteria_mode, CriteriaMode::Wildcards);
    assert!(!calc.case_sensitive);
    assert!(calc.whole_cell_match);
    assert_eq!(calc.date_epoch, DateEpoch::D1899);
    assert!(wb
        .meta
        .generator
        .as_deref()
        .unwrap()
        .starts_with("LibreOffice/7.6"));
    assert_eq!(
        wb.meta.created.as_deref(),
        Some("2026-10-18T10:00:00.123456789")
    );
    assert_eq!(wb.meta.modified.as_deref(), Some("2026-10-19T08:30:00.5"));

    let s = &wb.sheets[0];
    assert_eq!(s.cells.len(), 39, "{:#?}", s.cells);
    assert!(s
        .cells
        .windows(2)
        .all(|w| (w[0].row, w[0].col) < (w[1].row, w[1].col)));
    assert_eq!(
        s.col_widths,
        [DimRun {
            start: 0,
            end: 0,
            size_hmm: 4500,
            hidden: false
        }]
    );
    assert_eq!(
        s.row_heights,
        [DimRun {
            start: 7,
            end: 7,
            size_hmm: 1000,
            hidden: false
        }]
    );
    assert_eq!(
        s.merges,
        [CellRangeLocal {
            r0: 7,
            c0: 0,
            r1: 7,
            c1: 1
        }]
    );
    assert_eq!(s.frozen, Some((1, 0)));

    // Header row: bold, deduplicated into one style.
    for c in 0..5 {
        assert!(style(&wb, 0, 0, c).unwrap().font.bold);
        assert_eq!(
            cell(&wb, 0, 0, c).unwrap().style,
            cell(&wb, 0, 0, 0).unwrap().style
        );
    }
    assert_eq!(content(&wb, 0, 0, 1), &val(Value::text("Amount")));

    // Values.
    assert_eq!(content(&wb, 0, 1, 0), &val(Value::text("Widget blue")));
    assert_eq!(content(&wb, 0, 1, 1), &val(Value::Number(1234.5)));
    assert_eq!(content(&wb, 0, 1, 2), &val(Value::Number(46_314.0)));
    assert_eq!(content(&wb, 0, 1, 3), &val(Value::Bool(true)));
    assert_eq!(content(&wb, 0, 2, 0), &val(Value::text("Gadget   two")));
    assert_eq!(content(&wb, 0, 2, 1), &val(Value::Number(-20.0)));
    assert_eq!(content(&wb, 0, 2, 2), &val(Value::Number(46_314.75)));
    assert_eq!(content(&wb, 0, 2, 3), &val(Value::Bool(false)));

    // Column D's default cell style (bold) applies to unstyled values.
    assert!(style(&wb, 0, 1, 3).unwrap().font.bold);
    assert!(style(&wb, 0, 3, 3).unwrap().font.bold);
    assert!(style(&wb, 0, 0, 5).is_none());

    // Formulas: text kept verbatim with namespace prefix; cached values typed.
    assert_eq!(
        content(&wb, 0, 3, 0),
        &formula("of:=CONCATENATE(\"Tot\";\"al\")", Value::text("Total"))
    );
    assert_eq!(
        content(&wb, 0, 3, 1),
        &formula("of:=SUM([.B2:.B3])", Value::Number(1214.5))
    );
    assert_eq!(
        content(&wb, 0, 3, 2),
        &formula("of:=1/0", Value::Error(ErrorKind::Div0))
    );
    assert_eq!(
        content(&wb, 0, 3, 3),
        &formula("of:=[.D2]<>[.D3]", Value::Bool(true))
    );
    assert_eq!(
        content(&wb, 0, 3, 4),
        &formula("of:=[.B3]/[.B2]", Value::Number(-0.0162008910490077))
    );
    assert_eq!(
        content(&wb, 0, 8, 2),
        &formula("oooc:=[.A5]*2", Value::Number(14.0))
    );

    // Repeated non-empty rows are materialized (rows 5-7, cols A-B).
    for r in 4..7 {
        for c in 0..2 {
            assert_eq!(content(&wb, 0, r, c), &val(Value::Number(7.0)));
        }
    }

    // Rich text.
    assert_eq!(content(&wb, 0, 7, 0), &val(Value::text("Merged")));
    assert!(cell(&wb, 0, 7, 1).is_none(), "covered cell is empty");
    assert_eq!(content(&wb, 0, 7, 2), &val(Value::text("line1\nline2")));
    assert_eq!(content(&wb, 0, 7, 3), &val(Value::text("a\tb\nc link")));
    assert_eq!(
        content(&wb, 0, 7, 4),
        &val(Value::text("with note")),
        "annotation skipped"
    );
    assert_eq!(
        content(&wb, 0, 8, 3),
        &val(Value::text("<tag> & \"q\" \u{263A}"))
    );

    // Styled empty cell is kept.
    assert_eq!(content(&wb, 0, 8, 4), &val(Value::Empty));
    assert!(style(&wb, 0, 8, 4).unwrap().font.bold);

    // Rich cell style.
    let st = style(&wb, 0, 7, 0).unwrap();
    assert!(st.font.italic && st.font.underline && !st.font.bold);
    assert_eq!(st.font.color, Some(0xc921_1eff));
    assert_eq!(st.font.size_pt, Some(14.0));
    assert_eq!(st.font.family.as_deref(), Some("DejaVu Sans"));
    assert_eq!(st.fill, Some(0xffff_00ff));
    assert_eq!(st.align.h, HAlign::Center);
    assert_eq!(st.align.v, VAlign::Middle);
    assert!(st.align.wrap);

    // Inheritance from a common style in styles.xml.
    let st = style(&wb, 0, 8, 1).unwrap();
    assert!(st.font.bold && st.font.italic);
    assert_eq!(st.font.color, Some(0x0000_00ff));

    // "value-type" alignment source means General.
    assert!(style(&wb, 0, 9, 5).is_none());

    // Number formats. The currency symbol's number:language / number:country
    // become the `[$$-409]` tag, as AOO's import gives.
    assert_eq!(
        number_format(&wb, 0, 1, 1),
        Some("[$$-409]#,##0.00;[RED]-[$$-409]#,##0.00")
    );
    assert_eq!(number_format(&wb, 0, 1, 2), Some("MM/DD/YYYY"));
    assert_eq!(number_format(&wb, 0, 3, 4), Some("0.00%"));
    assert_eq!(number_format(&wb, 0, 8, 0), Some("HH:MM AM/PM"));
    assert_eq!(number_format(&wb, 0, 9, 0), Some("DD MMM YYYY"));
    assert_eq!(number_format(&wb, 0, 9, 1), Some("0.00E+00"));
    assert_eq!(number_format(&wb, 0, 9, 2), Some("# ?/?"));
    assert_eq!(number_format(&wb, 0, 9, 3), Some("@"));
    assert_eq!(number_format(&wb, 0, 9, 4), Some("\"Qty \"0"));
    assert!(
        !style(&wb, 0, 9, 3).unwrap().font.bold,
        "explicit style beats column default"
    );
    let CellContentData::Value(Value::Number(t)) = content(&wb, 0, 8, 0) else {
        panic!()
    };
    assert!((t - 14.5 / 24.0).abs() < 1e-12);
    assert_eq!(content(&wb, 0, 9, 0), &val(Value::Number(36_585.0)));

    let s2 = &wb.sheets[1];
    assert_eq!(s2.tab_color, Some(0x3366_ccff));
    assert_eq!(s2.cells.len(), 1);
    assert_eq!(
        content(&wb, 1, 0, 0),
        &formula("of:=SUM([$Orders.B2:.B3])", Value::Number(1214.5))
    );
}

#[test]
fn aoo_shapes() {
    let wb = open("ods/aoo-style.ods");
    assert_eq!(wb.sheets.len(), 2);
    assert_eq!(wb.settings.active_sheet, 0);
    assert_eq!(wb.meta.title.as_deref(), Some("AOO sample"));
    let calc = &wb.settings.calc_settings;
    assert_eq!(calc.criteria_mode, CriteriaMode::Regex);
    assert!(!calc.case_sensitive && !calc.whole_cell_match);
    assert_eq!(
        calc.iterations,
        Some(IterSettings {
            steps: 50,
            min_change: 0.0001
        })
    );
    let s = &wb.sheets[0];
    assert_eq!(s.frozen, Some((1, 1)));
    assert_eq!(
        s.col_widths,
        [DimRun {
            start: 0,
            end: 1023,
            size_hmm: 2267,
            hidden: false
        }]
    );
    assert_eq!(content(&wb, 0, 1, 0), &val(Value::Number(1_234_567.0)));
    assert_eq!(number_format(&wb, 0, 1, 0), Some("#,##0"));
    assert_eq!(content(&wb, 0, 1, 1), &val(Value::Number(36_525.0)));
    assert_eq!(number_format(&wb, 0, 1, 1), Some("YYYY-MM-DD"));
    assert_eq!(
        content(&wb, 0, 1, 2),
        &val(Value::Number(86_399.0 / 86_400.0))
    );
    assert_eq!(number_format(&wb, 0, 1, 2), Some("HH:MM:SS"));
    assert_eq!(
        content(&wb, 0, 2, 0),
        &formula("of:=SUM([.A2];1)", Value::Number(1_234_568.0))
    );
    assert_eq!(
        content(&wb, 0, 2, 1),
        &formula("of:=1/0", Value::Error(ErrorKind::Div0))
    );
    assert_eq!(
        content(&wb, 0, 2, 2),
        &formula("of:=NA()", Value::Error(ErrorKind::NA))
    );
    assert_eq!(
        content(&wb, 0, 2, 3),
        &formula("of:=0.25", Value::Number(0.25))
    );
    assert_eq!(number_format(&wb, 0, 2, 3), Some("0%"));
    assert!(style(&wb, 0, 0, 0).unwrap().font.bold);
    assert!(wb.sheets[1].hidden);
    assert_eq!(content(&wb, 1, 0, 0), &val(Value::text("secret")));
}

#[test]
fn legacy_and_minimal() {
    let wb = open("ods/ooo2-legacy.ods");
    assert_eq!(wb.settings.calc_settings.date_epoch, DateEpoch::D1904);
    // ODF attribute defaults when calculation-settings is present but bare.
    assert_eq!(wb.settings.calc_settings.criteria_mode, CriteriaMode::Regex);
    assert!(wb.settings.calc_settings.case_sensitive);
    assert_eq!(
        content(&wb, 0, 0, 2),
        &formula("oooc:=SUM([.A1:.B1])", Value::Number(5.0))
    );
    assert_eq!(
        content(&wb, 0, 1, 0),
        &val(Value::Number(1.0)),
        "1904 epoch serial"
    );

    let wb = open("ods/minimal-no-manifest.ods");
    assert_eq!(wb.sheets[0].name, "Only");
    assert_eq!(content(&wb, 0, 0, 0), &val(Value::text("plain text")));
    assert_eq!(content(&wb, 0, 0, 1), &val(Value::Number(1e300)));
    assert_eq!(wb.settings.calc_settings, Default::default());
}

#[test]
fn sniffing() {
    for f in [
        "ods/libreoffice-style.ods",
        "ods/aoo-style.ods",
        "ods/minimal-no-manifest.ods",
    ] {
        assert!(sniff_is_ods(&read_fixture(f)), "{f}");
    }
    assert!(!sniff_is_ods(&read_fixture("malicious/not-a-zip.ods")));
    assert!(!sniff_is_ods(&read_fixture("malicious/text-document.odt")));
    assert!(!sniff_is_ods(b"PK\x03\x04garbage"));
    assert!(!sniff_is_ods(b""));
}

/// Fixtures survive read → write → read with every supported feature intact.
#[test]
fn fixtures_round_trip_through_our_writer() {
    for f in [
        "ods/libreoffice-style.ods",
        "ods/aoo-style.ods",
        "ods/ooo2-legacy.ods",
        "ods/minimal-no-manifest.ods",
    ] {
        let mut wb = open(f);
        let mut buf = Cursor::new(Vec::new());
        write_ods(&wb, &mut buf).unwrap();
        let back = read_ods(Cursor::new(buf.into_inner()), &Limits::default()).unwrap();
        // The generator is rewritten.
        wb.meta.generator.clone_from(&back.meta.generator);
        assert_eq!(common::normalize(&back), common::normalize(&wb), "{f}");
    }
}
