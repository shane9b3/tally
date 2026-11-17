//! Hostile input for the BIFF8 formatting reader: truncated records,
//! lying counts and sizes, indices without a target, limits. The pass
//! must never panic (a panic is caught and reported as "could not be
//! read", which these tests treat as a failure) and must never turn a
//! file that opens into one that does not.
#![allow(clippy::unwrap_used)]

mod biff;

use tally_io_xls::{read_xls, Limits, XlsError};
use tally_model::WorkbookData;

/// Reads `bytes`; a caught panic of the formatting pass fails the test.
fn read_with(bytes: &[u8], limits: &Limits) -> Result<WorkbookData, XlsError> {
    let r = read_xls(bytes, limits);
    if let Ok(wb) = &r {
        assert!(
            !wb.warnings.iter().any(|w| w.contains("could not be read")),
            "the formatting pass panicked: {:?}",
            wb.warnings
        );
    }
    r
}

fn read(bytes: &[u8]) -> Result<WorkbookData, XlsError> {
    read_with(bytes, &Limits::default())
}

/// A workbook with `globals` after the usual ones and `sheet` records
/// around one number cell.
fn book(globals: &[u8], sheet: &[u8]) -> Vec<u8> {
    let mut g = biff::globals(&[], &[], &[]);
    g.extend_from_slice(globals);
    let mut s = sheet.to_vec();
    s.extend(biff::number(0, 0, 15, 1.0));
    biff::file(biff::stream(&g, &[("S", s)]))
}

/// The data of a record built by a `biff` helper.
fn data(record: &[u8]) -> (u16, &[u8]) {
    (u16::from_le_bytes([record[0], record[1]]), &record[4..])
}

#[test]
fn every_record_cut_at_every_length() {
    let png = biff::bse(0xF01E, 0x6E0, &biff::fake_png(8));
    let group = biff::msodrawinggroup(&biff::dgg(&[png]), usize::MAX);
    let drawing = biff::msodrawing(&biff::dg(&[biff::shape(
        75,
        Some(1),
        [0, 0, 0, 0, 1, 0, 1, 0],
    )]));
    let note = biff::note(0, 0, 1, true, "author", "text");
    // The note helper emits OBJ, TXO, CONTINUE, CONTINUE, NOTE.
    let obj_len = 4 + usize::from(u16::from_le_bytes([note[2], note[3]]));
    let txo_len = 4 + usize::from(u16::from_le_bytes([note[obj_len + 2], note[obj_len + 3]]));
    let note_at = note.len() - {
        // NOTE: 8 bytes, the author's length, flags and 6 characters.
        4 + 8 + 2 + 1 + 12
    };
    // (in the globals?, the record, must the file still open?). Records
    // calamine reads itself (FORMAT, XF, cells with values) may fail the
    // file when cut, as they did before the formatting pass existed.
    let cases: Vec<(bool, Vec<u8>, bool)> = vec![
        (true, biff::font(&biff::Font::default()), true),
        (true, biff::format(164, "0.00"), false),
        (true, biff::xf(&biff::Xf::default()), false),
        (true, biff::palette(&[1, 2, 3]), true),
        (true, biff::window1(0), true),
        (true, group, true),
        (false, biff::colinfo(0, 3, 2560, 15, 0), true),
        (false, biff::defcolwidth(4), true),
        (false, biff::standardwidth(1024), true),
        (false, biff::defaultrowheight(0, 300), true),
        (false, biff::row(0, 400, 0x140), true),
        (false, biff::blank(1, 1, 15), true),
        (false, biff::mulblank(1, 1, &[15, 15, 15]), true),
        (false, biff::number(1, 1, 15, 2.0), false),
        (false, biff::rk(1, 1, 15, 2), false),
        (false, biff::mulrk(1, 1, &[(15, 1), (15, 2)]), false),
        (false, biff::label(1, 1, 15, "abc"), false),
        (false, biff::rstring(1, 1, 15, "abc"), false),
        (false, biff::labelsst(1, 1, 15, 0), false),
        (false, biff::boolerr(1, 1, 15, 1, false), false),
        (
            false,
            biff::formula(1, 1, 15, 1.0, &biff::ptg_int(1)),
            false,
        ),
        (false, biff::window2(0x00B6), true),
        (false, biff::pane(1, 1), true),
        (false, biff::scl(3, 2), true),
        (false, biff::sheetext(10), true),
        (false, drawing, true),
        (false, note[..obj_len].to_vec(), true),
        (false, note[obj_len..obj_len + txo_len].to_vec(), true),
        (false, note[note_at..].to_vec(), true),
    ];
    for (in_globals, record, must_open) in &cases {
        let (typ, body) = data(record);
        for cut in 0..body.len() {
            let r = biff::rec(typ, &body[..cut]);
            let bytes = if *in_globals {
                book(&r, &[])
            } else {
                book(&[], &r)
            };
            let result = read(&bytes);
            if *must_open {
                assert!(
                    result.is_ok(),
                    "record {typ:#06x} cut at {cut}: {:?}",
                    result.err()
                );
            }
        }
    }
}

#[test]
fn counts_that_lie() {
    // PALETTE announcing 65,535 colours with two present.
    let mut pal = biff::palette(&[0x11_2233, 0x44_5566]);
    pal[4..6].copy_from_slice(&0xFFFFu16.to_le_bytes());
    // FONT whose name claims 255 characters.
    let mut font = biff::font(&biff::Font::default());
    font[4 + 14] = 255;
    // FORMAT whose code claims 65,535 characters.
    let mut fmt = biff::format(164, "0.00");
    fmt[6..8].copy_from_slice(&0xFFFFu16.to_le_bytes());
    let mut g = pal;
    g.extend(font);
    g.extend(fmt);
    // MULBLANK whose last column lies, NOTE whose author claims 65,535
    // characters, TXO claiming 65,535 characters of text.
    let mut mb = biff::mulblank(1, 250, &[15; 20]);
    let n = mb.len();
    mb[n - 2..].copy_from_slice(&0xFFFFu16.to_le_bytes());
    let mut note = biff::note(2, 2, 1, false, "a", "text");
    let n = note.len();
    // NOTE data: row, col, flags, id, cch, flags byte, "a" (2 bytes).
    note[n - 5..n - 3].copy_from_slice(&0xFFFFu16.to_le_bytes());
    let obj_len = 4 + usize::from(u16::from_le_bytes([note[2], note[3]]));
    note[obj_len + 4 + 10..obj_len + 4 + 12].copy_from_slice(&0xFFFFu16.to_le_bytes());
    let mut s = mb;
    s.extend(note);
    // An OPT announcing 4,095 properties in a 6-byte body, in a drawing
    // whose container lengths are all 0xFFFFFFFF.
    let mut opt = 0x4104u16.to_le_bytes().to_vec();
    opt.extend_from_slice(&1u32.to_le_bytes());
    let mut sp = biff::art(3, 0xFFF, 0xF00B, &opt);
    let mut anchor = vec![0u8; 2];
    anchor.extend(
        [0u16, 0, 0, 0, 1, 0, 1, 0]
            .iter()
            .flat_map(|w| w.to_le_bytes()),
    );
    sp.extend(biff::art(0, 0, 0xF010, &anchor));
    let mut d = Vec::new();
    for typ in [0xF002u16, 0xF003, 0xF004] {
        d.extend_from_slice(&0x000Fu16.to_le_bytes());
        d.extend_from_slice(&typ.to_le_bytes());
        d.extend_from_slice(&u32::MAX.to_le_bytes());
    }
    d.extend(sp);
    s.extend(biff::msodrawing(&d));
    let png = biff::bse(0xF01E, 0x6E0, &biff::fake_png(8));
    g.extend(biff::msodrawinggroup(&biff::dgg(&[png]), usize::MAX));
    let wb = read(&book(&g, &s)).unwrap();
    let sheet = &wb.sheets[0];
    assert_eq!(sheet.images.len(), 1);
    assert_eq!(sheet.notes.len(), 1);
    assert_eq!(sheet.notes[0].text, "text");
    assert_eq!(wb.settings.default_font.as_deref(), Some("Arial"));
}

#[test]
fn indices_without_a_target() {
    // XFs pointing at a font and a number format that do not exist, and
    // at font 4 (which BIFF does not have); cells pointing at XFs that do
    // not exist.
    let xfs = [
        biff::Xf {
            font: 999,
            fmt: 9999,
            ..biff::Xf::default()
        },
        biff::Xf {
            font: 4,
            halign: 2,
            ..biff::Xf::default()
        },
    ];
    let mut g = biff::globals(&[], &[], &xfs);
    g.extend(biff::sst(&["s"]));
    let mut s = biff::number(0, 0, 16, 1.0);
    s.extend(biff::number(0, 1, 17, 2.0));
    s.extend(biff::number(0, 2, 60_000, 3.0));
    s.extend(biff::blank(0, 3, 0xFFFF));
    s.extend(biff::mulblank(1, 0, &[40_000, 16, 17]));
    s.extend(biff::colinfo(0, 0, 2560, 50_000, 0));
    s.extend(biff::row(1, 300, 0x1C0 | 0xFFF << 16));
    // A picture whose BLIP index is 0, one far beyond the store, and a
    // tab colour index without a colour.
    s.extend(biff::msodrawing(&biff::dg(&[
        biff::shape(75, Some(0), [0, 0, 0, 0, 1, 0, 1, 0]),
        biff::shape(75, Some(u32::MAX), [0, 0, 0, 0, 1, 0, 1, 0]),
    ])));
    s.extend(biff::sheetext(0x7E));
    let bytes = biff::file(biff::stream(&g, &[("S", s)]));
    let wb = read(&bytes).unwrap();
    let sheet = &wb.sheets[0];
    assert_eq!(sheet.cells.len(), 4, "{:?}", sheet.cells);
    assert_eq!(sheet.cells[2].style, None);
    assert!(sheet.images.is_empty());
    assert_eq!(sheet.tab_color, None);
    assert!(sheet.style_runs.is_empty());
    // WINDOW1 selecting a sheet that does not exist.
    let wb = read(&book(&biff::window1(7), &[])).unwrap();
    assert_eq!(wb.settings.active_sheet, 0);
}

#[test]
fn column_and_row_records_out_of_range() {
    let mut s = Vec::new();
    // first > last, a first column beyond BIFF8, the whole u16 range.
    s.extend(biff::colinfo(5, 2, 2560, 15, 0));
    s.extend(biff::colinfo(256, 300, 2560, 15, 0));
    s.extend(biff::colinfo(0xFFFF, 0xFFFF, 2560, 15, 0));
    s.extend(biff::colinfo(0, 0xFFFF, 0xFFFF, 15, 0xFFFF));
    // The same row three times (the last one counts), the last row, a
    // height with the "default" bit, a height of 0x7FFF twips.
    s.extend(biff::row(3, 300, 0x140));
    s.extend(biff::row(3, 500, 0x140));
    s.extend(biff::row(3, 400, 0x140));
    s.extend(biff::row(0xFFFF, 400, 0x140));
    s.extend(biff::row(1, 0x8000 | 300, 0x140));
    s.extend(biff::row(0, 0x7FFF, 0x140));
    let wb = read(&book(&[], &s)).unwrap();
    let sheet = &wb.sheets[0];
    assert_eq!(sheet.col_widths.len(), 1);
    let c = &sheet.col_widths[0];
    assert_eq!(
        (c.start, c.end, c.hidden),
        (0, tally_types::MAX_COLS - 1, true)
    );
    let rows: Vec<(u32, u32)> = sheet.row_heights.iter().map(|r| (r.start, r.end)).collect();
    assert_eq!(rows, vec![(0, 0), (3, 3), (65_535, 65_535)]);
    assert_eq!(
        sheet.row_heights[1].size_hmm,
        tally_io_xlsx::pt_to_hmm(20.0)
    );
}

#[test]
fn limits_degrade_with_a_warning() {
    let mut g = Vec::new();
    for _ in 0..10 {
        g.extend(biff::font(&biff::Font::default()));
        g.extend(biff::xf(&biff::Xf::default()));
    }
    for id in 164..174 {
        g.extend(biff::format(id, "0.0"));
    }
    let mut s = Vec::new();
    for r in 0..50u16 {
        s.extend(biff::row(r, 400, 0x140));
        s.extend(biff::colinfo(r, r, 2560, 15, 0));
    }
    let bytes = book(&g, &s);
    let limits = Limits {
        max_fonts: 2,
        max_xfs: 3,
        max_number_formats: 4,
        max_col_infos: 5,
        max_row_records: 6,
        ..Limits::default()
    };
    let wb = read_with(&bytes, &limits).unwrap();
    assert_eq!(wb.sheets[0].col_widths.len(), 1);
    assert_eq!(wb.sheets[0].col_widths[0].end, 4);
    assert_eq!(wb.sheets[0].row_heights.len(), 1);
    assert_eq!(wb.sheets[0].row_heights[0].end, 5);
    for part in ["first 2 fonts", "column definitions", "row definitions"] {
        assert!(
            wb.warnings.iter().any(|w| w.contains(part)),
            "{part}: {:?}",
            wb.warnings
        );
    }
    // The defaults read all of it without a warning.
    let wb = read(&bytes).unwrap();
    assert_eq!(wb.warnings, Vec::<String>::new());
    assert_eq!(wb.sheets[0].row_heights[0].end, 49);
}

#[test]
fn pictures_with_lying_sizes() {
    let png = biff::fake_png(8);
    let good = biff::bse(0xF01E, 0x6E0, &png);
    // The BLIP record claims more bytes than its store entry holds.
    let mut long_blip = good.clone();
    long_blip[8 + 36 + 4..8 + 36 + 8].copy_from_slice(&0x00FF_FFFFu32.to_le_bytes());
    // The store entry claims more bytes than the group holds.
    let mut long_entry = good.clone();
    long_entry[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
    // The name length points past the entry.
    let mut long_name = good.clone();
    long_name[8 + 33] = 255;
    // A PNG record without a PNG in it.
    let not_png = biff::bse(0xF01E, 0x6E0, b"not a picture");
    let sheet_for = |n: u32| {
        let shapes: Vec<Vec<u8>> = (1..=n)
            .map(|i| biff::shape(75, Some(i), [0, 0, 0, 0, 1, 0, 1, 0]))
            .collect();
        biff::msodrawing(&biff::dg(&shapes))
    };
    for (store, pictures) in [
        (vec![long_blip, good.clone()], 1),
        (vec![long_name, good.clone()], 1),
        (vec![not_png, good.clone()], 1),
        // Nothing after the lying entry can be trusted.
        (vec![good.clone(), long_entry, good.clone()], 1),
    ] {
        let g = biff::msodrawinggroup(&biff::dgg(&store), 100);
        let wb = read(&book(&g, &sheet_for(3))).unwrap();
        assert_eq!(wb.sheets[0].images.len(), pictures, "{:?}", wb.warnings);
        assert_eq!(&*wb.sheets[0].images[0].bytes, &png[..]);
    }
    // The group cut at every length.
    let full = biff::dgg(&[good]);
    for cut in 0..full.len() {
        let g = biff::msodrawinggroup(&full[..cut], 50);
        let wb = read(&book(&g, &sheet_for(1))).unwrap();
        assert!(wb.sheets[0].images.is_empty());
    }
    // The sheet's drawing cut at every length.
    let g = biff::msodrawinggroup(&full, 50);
    let drawing = biff::dg(&[biff::shape(75, Some(1), [0, 0, 0, 0, 1, 0, 1, 0])]);
    for cut in 0..drawing.len() {
        let _ = read(&book(&g, &biff::msodrawing(&drawing[..cut]))).unwrap();
    }
    // Anchors at the corners of the u16 range.
    let wild = biff::msodrawing(&biff::dg(&[
        biff::shape(75, Some(1), [0xFFFF; 8]),
        biff::shape(75, Some(1), [0xFFFF, 0, 0xFFFF, 0, 0, 0, 0, 0]),
    ]));
    let wb = read(&book(&g, &wild)).unwrap();
    assert_eq!(wb.sheets[0].images.len(), 2);
    for i in &wb.sheets[0].images {
        assert!(i.anchor.width_hmm >= 100 && i.anchor.height_hmm >= 100);
        assert!(i.anchor.col < tally_types::MAX_COLS && i.anchor.row < tally_types::MAX_ROWS);
    }
}

#[test]
fn mutated_fixtures_never_panic() {
    for (name, bytes) in biff::books::all() {
        assert!(read(&bytes).is_ok(), "{name}");
        // The workbook stream starts after the header, FAT and directory
        // of the canonical container (3 × 512 bytes).
        for i in 1536..bytes.len() {
            if bytes[i] == 0 && i % 7 != 0 {
                continue;
            }
            for flip in [0xFFu8, 0x80, 0x01] {
                let mut m = bytes.clone();
                m[i] ^= flip;
                let _ = read(&m);
            }
        }
        for cut in (1536..bytes.len()).step_by(13) {
            let _ = read(&bytes[..cut]);
        }
    }
}

#[test]
fn garbage_records_never_panic() {
    // Every record type the pass reads, with pseudo-random data of every
    // short length and a few long ones.
    let types: [u16; 28] = [
        0x0031, 0x041E, 0x00E0, 0x0092, 0x003D, 0x00EB, 0x007D, 0x0055, 0x0099, 0x0225, 0x0208,
        0x0201, 0x00BE, 0x0203, 0x027E, 0x00BD, 0x0204, 0x00D6, 0x00FD, 0x0205, 0x023E, 0x0041,
        0x00A0, 0x0862, 0x00EC, 0x005D, 0x01B6, 0x001C,
    ];
    let mut seed = 0x2545_F491u32;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed
    };
    for round in 0..40 {
        let mut g = Vec::new();
        let mut s = Vec::new();
        for (k, typ) in types.iter().enumerate() {
            let len = match round {
                0..=31 => round,
                _ => 32 + (next() % 400) as usize,
            };
            let body: Vec<u8> = (0..len)
                .map(|_| if round % 2 == 0 { 0xFF } else { next() as u8 })
                .collect();
            let r = biff::rec(*typ, &body);
            if k < 6 {
                g.extend(r);
            } else {
                s.extend(r);
            }
        }
        // With and without the regular globals around them.
        let _ = read(&book(&g, &s));
        let _ = read(&biff::file(biff::stream(&g, &[("S", s)])));
    }
}
