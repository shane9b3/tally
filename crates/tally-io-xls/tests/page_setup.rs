//! The page setup of a BIFF8 sheet: `SETUP`, the margin records,
//! `HEADER`/`FOOTER`, the print options, `WSBOOL`'s fit-to-pages bit,
//! manual page breaks and the built-in names `Print_Area` and
//! `Print_Titles`. It maps onto the model as the XLSX reader maps
//! `pageSetup`, `pageMargins`, `headerFooter`, `printOptions` and the
//! `_xlnm.` names.
#![allow(clippy::unwrap_used)]

mod biff;

use biff::Setup;
use tally_io_xls::{read_xls, Limits};
use tally_io_xlsx::PageParts;
use tally_model::{
    CellRangeLocal, HeaderFooterData, MarginsData, OrientationData, PageOrderData, PageScaleData,
    PageSetupData, PaperData, WorkbookData,
};

fn read(bytes: &[u8]) -> WorkbookData {
    read_xls(bytes, &Limits::default()).unwrap()
}

/// A one-sheet workbook with `globals` after the usual ones and the page
/// records `page` after one number.
fn book(globals: &[u8], page: &[u8]) -> Vec<u8> {
    let mut g = biff::globals(&[], &[], &[]);
    g.extend_from_slice(globals);
    let mut s = biff::number(0, 0, 15, 1.0);
    s.extend_from_slice(page);
    biff::file(biff::stream(&g, &[("S", s)]))
}

fn setup_of(page: &[u8]) -> Option<PageSetupData> {
    read(&book(&[], page)).sheets[0].page_setup.clone()
}

/// Excel's default margins (0.75 in, 1 in, 0.5 in) as OpenOffice has them
/// in 1/100 mm.
const EXCEL_MARGINS: MarginsData = MarginsData {
    left: 1900,
    right: 1900,
    top: 2500,
    bottom: 2500,
    header: 1300,
    footer: 1300,
};

#[test]
fn a_sheet_without_page_records_has_no_page_setup() {
    let wb = read(&book(&[], &[]));
    assert_eq!(wb.sheets[0].page_setup, None);
    assert!(wb.sheets[0].row_breaks.is_empty() && wb.sheets[0].col_breaks.is_empty());
}

#[test]
fn every_page_record_is_read() {
    let mut page = Vec::new();
    page.extend(biff::word(0x002A, 1));
    page.extend(biff::word(0x002B, 1));
    page.extend(biff::page_breaks(true, &[20, 5, 5, 0]));
    page.extend(biff::page_breaks(false, &[3]));
    page.extend(biff::header_footer(0x0014, "&LLeft&CPage &P of &N&R&D"));
    page.extend(biff::header_footer(0x0015, ""));
    page.extend(biff::word(0x0083, 1));
    page.extend(biff::word(0x0084, 0));
    page.extend(biff::margin(0x0026, 0.166));
    page.extend(biff::margin(0x0027, 0.25));
    page.extend(biff::margin(0x0028, 1.0));
    page.extend(biff::margin(0x0029, 0.75));
    page.extend(biff::setup(&Setup {
        paper: 1,
        scale: 80,
        start: 3,
        // Landscape (no portrait bit), over then down, use the start page.
        flags: 0x0081,
        header: 0.3,
        footer: 0.4,
        ..Setup::default()
    }));
    let mut names = biff::externsheet(1);
    names.extend(biff::builtin_name(0x06, 1, &biff::ptg_area3d(0, 29, 0, 5)));
    names.extend(biff::builtin_name(
        0x07,
        1,
        &biff::ptg_union(
            &biff::ptg_area3d(0, 65_535, 0, 0),
            &biff::ptg_area3d(0, 1, 0, 255),
        ),
    ));
    let wb = read(&book(&names, &page));
    assert_eq!(wb.warnings, Vec::<String>::new());
    let sheet = &wb.sheets[0];
    let p = sheet.page_setup.as_ref().unwrap();

    // What the XLSX reader makes of the same settings.
    let header = HeaderFooterData {
        left: "Left".into(),
        center: "Page &[PAGE] of &[PAGES]".into(),
        right: "&[DATE]".into(),
    };
    let margins = MarginsData {
        left: 422,
        right: 635,
        top: 2540,
        bottom: 1905,
        header: 762,
        footer: 1016,
    };
    let mut expected = PageParts {
        any: true,
        margins: Some(margins),
        orientation: Some(OrientationData::Landscape),
        paper: Some(PaperData::Letter),
        scale: Some(80),
        fit: Some((1, 1)),
        first_page: Some(3),
        over_then_down: true,
        gridlines: true,
        headings: true,
        center_h: true,
        header: Some(header.clone()),
        footer: Some(HeaderFooterData::default()),
        ..PageParts::default()
    }
    .build()
    .unwrap();
    expected.print_range = Some(CellRangeLocal {
        r0: 0,
        c0: 0,
        r1: 29,
        c1: 5,
    });
    expected.repeat_rows = Some((0, 1));
    expected.repeat_cols = Some((0, 0));
    assert_eq!(*p, expected);

    // And in plain values: the header sits in the top margin (Excel's
    // header margin is the page margin, the area fills the rest); there
    // is no footer.
    assert_eq!(p.orientation, OrientationData::Landscape);
    assert_eq!(p.paper, PaperData::Letter);
    assert_eq!(p.scale, PageScaleData::Percent(80));
    assert_eq!(p.first_page_number, Some(3));
    assert_eq!(p.page_order, PageOrderData::LeftToRight);
    assert!(p.print_gridlines && p.print_headers && p.center_h && !p.center_v);
    assert_eq!(p.header, Some(header));
    assert_eq!(p.footer, None);
    assert!(p.header_area.on && !p.footer_area.on);
    assert_eq!((p.margins.left, p.margins.right), (422, 635));
    assert_eq!((p.margins.top, p.margins.header), (762, 2540 - 762));
    assert_eq!(p.margins.bottom, 1905);
    assert_eq!(p.excel_margins, Some(margins));
    // Breaks: sorted, once each, 0 is none.
    assert_eq!(sheet.row_breaks, vec![5, 20]);
    assert_eq!(sheet.col_breaks, vec![3]);
}

#[test]
fn margins_default_to_excels_and_setup_alone_is_a_page_setup() {
    let p = setup_of(&biff::setup(&Setup::default())).unwrap();
    assert_eq!(p.orientation, OrientationData::Portrait);
    assert_eq!(p.paper, PaperData::A4);
    assert_eq!(p.scale, PageScaleData::Percent(100));
    assert_eq!(p.first_page_number, None);
    assert_eq!((p.header.clone(), p.footer.clone()), (None, None));
    // No header or footer: the page margins are Excel's, as they are.
    assert_eq!(
        (
            p.margins.left,
            p.margins.right,
            p.margins.top,
            p.margins.bottom
        ),
        (1900, 1900, 2500, 2500)
    );
    // SETUP gives the header and footer margins.
    assert_eq!(
        p.excel_margins,
        Some(MarginsData {
            header: 1270,
            footer: 1270,
            ..EXCEL_MARGINS
        })
    );
    // A margin record alone is one too, the others default.
    let p = setup_of(&biff::margin(0x0029, 2.0)).unwrap();
    assert_eq!(
        p.excel_margins,
        Some(MarginsData {
            bottom: 5080,
            ..EXCEL_MARGINS
        })
    );
}

#[test]
fn fit_to_pages_needs_the_wsbool_bit() {
    let fit = Setup {
        scale: 55,
        fit_w: 1,
        fit_h: 0,
        ..Setup::default()
    };
    let mut page = biff::word(0x0081, 0x04C1);
    page.extend(biff::setup(&fit));
    assert_eq!(setup_of(&page).unwrap().scale, PageScaleData::Percent(55));
    let mut page = biff::word(0x0081, 0x05C1);
    page.extend(biff::setup(&fit));
    assert_eq!(
        setup_of(&page).unwrap().scale,
        PageScaleData::FitTo {
            width: 1,
            height: 0
        }
    );
    // The bit alone: one page.
    assert_eq!(
        setup_of(&biff::word(0x0081, 0x0100)).unwrap().scale,
        PageScaleData::FitTo {
            width: 1,
            height: 1
        }
    );
    // WSBOOL without the bit is no page setup.
    assert_eq!(setup_of(&biff::word(0x0081, 0x04C1)), None);
}

#[test]
fn setup_flags_say_what_is_valid() {
    let landscape_letter = |flags: u16| {
        setup_of(&biff::setup(&Setup {
            paper: 1,
            scale: 150,
            start: 9,
            flags,
            ..Setup::default()
        }))
        .unwrap()
    };
    let p = landscape_letter(0x0000);
    assert_eq!(
        (p.paper, p.orientation, p.scale),
        (
            PaperData::Letter,
            OrientationData::Landscape,
            PageScaleData::Percent(150)
        )
    );
    // The start page needs its flag.
    assert_eq!(p.first_page_number, None);
    assert_eq!(landscape_letter(0x0080).first_page_number, Some(9));
    // Bit 2: paper, scale and orientation were never set.
    let p = landscape_letter(0x0004);
    assert_eq!(
        (p.paper, p.orientation, p.scale),
        (
            PaperData::A4,
            OrientationData::Portrait,
            PageScaleData::Percent(100)
        )
    );
    // Bit 6: the orientation was not.
    let p = landscape_letter(0x0040);
    assert_eq!(
        (p.paper, p.orientation),
        (PaperData::Letter, OrientationData::Portrait)
    );
    // Black and white, draft and copies have no place in the model and
    // change nothing.
    assert_eq!(landscape_letter(0x0018), landscape_letter(0x0000));
}

#[test]
fn paper_sizes_follow_openoffices_table() {
    let paper = |code: u16| {
        setup_of(&biff::setup(&Setup {
            paper: code,
            ..Setup::default()
        }))
        .unwrap()
        .paper
    };
    let custom = |w: u32, h: u32| PaperData::Custom {
        width_hmm: w,
        height_hmm: h,
    };
    assert_eq!(paper(1), PaperData::Letter);
    assert_eq!(paper(5), PaperData::Legal);
    assert_eq!(paper(8), PaperData::A3);
    assert_eq!(paper(9), PaperData::A4);
    assert_eq!(paper(11), PaperData::A5);
    assert_eq!(paper(34), PaperData::B5);
    // B5 (JIS) is not ISO B5; B4 (JIS); Statement (5.5 × 8.5 in);
    // Ledger is Tabloid on its side.
    assert_eq!(paper(13), custom(18_200, 25_700));
    assert_eq!(paper(12), custom(25_700, 36_400));
    assert_eq!(paper(6), custom(13_970, 21_590));
    assert_eq!(paper(4), PaperData::Tabloid);
    // Rotated papers are their upright size.
    assert_eq!(paper(77), PaperData::A4);
    assert_eq!(paper(75), PaperData::Letter);
    // Undefined codes are the default paper.
    for code in [0, 48, 71, 91, 255, 0xFFFF] {
        assert_eq!(paper(code), PaperData::A4, "code {code}");
    }
}

#[test]
fn absurd_values_are_clamped_or_dropped() {
    let margins = |v: f64| {
        let mut page = biff::margin(0x0026, v);
        page.extend(biff::margin(0x0028, v));
        page.extend(biff::setup(&Setup {
            header: v,
            footer: v,
            ..Setup::default()
        }));
        setup_of(&page).unwrap().excel_margins.unwrap()
    };
    // Not a length: the default stays.
    for v in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(margins(v), EXCEL_MARGINS, "{v}");
    }
    // Negative is 0; huge is 50 in.
    let m = margins(-3.0);
    assert_eq!((m.left, m.top, m.header, m.footer), (0, 0, 0, 0));
    let m = margins(1e300);
    assert_eq!((m.left, m.top, m.header), (127_000, 127_000, 127_000));
    let m = margins(f64::MIN_POSITIVE);
    assert_eq!((m.left, m.top), (0, 0));
    // A scale outside 10–400 % is clamped; 0 is "none" (100 %).
    let scale = |s: u16| {
        setup_of(&biff::setup(&Setup {
            scale: s,
            ..Setup::default()
        }))
        .unwrap()
        .scale
    };
    assert_eq!(scale(0), PageScaleData::Percent(100));
    assert_eq!(scale(1), PageScaleData::Percent(10));
    assert_eq!(scale(0xFFFF), PageScaleData::Percent(400));
    // A start page that is not a positive 16-bit number is none.
    for start in [0u16, 0x8000, 0xFFFF] {
        let p = setup_of(&biff::setup(&Setup {
            start,
            flags: 0x0082,
            ..Setup::default()
        }))
        .unwrap();
        assert_eq!(p.first_page_number, None, "{start}");
    }
}

#[test]
fn header_and_footer_codes() {
    let hf = |text: &str| {
        let mut page = biff::header_footer(0x0014, text);
        page.extend(biff::header_footer(0x0015, text));
        let p = setup_of(&page).unwrap();
        assert_eq!(p.header, p.footer);
        p.header
    };
    assert_eq!(
        hf("&L&A&C&\"Arial,Bold\"&12&F&R&T &Z&F && &P/&N"),
        Some(HeaderFooterData {
            left: "&[SHEET]".into(),
            center: "&[FILE]".into(),
            right: "&[TIME] &[PATH] & &[PAGE]/&[PAGES]".into(),
        })
    );
    // Text without a section code is the centre.
    assert_eq!(
        hf("plain"),
        Some(HeaderFooterData {
            center: "plain".into(),
            ..HeaderFooterData::default()
        })
    );
    // Nothing printable is no header.
    assert_eq!(hf("&L&C&R"), None);
    assert_eq!(hf(""), None);
}

#[test]
fn print_names_need_plain_areas_of_their_sheet() {
    let area = |names: Vec<u8>| {
        let mut g = biff::externsheet(1);
        g.extend(names);
        let wb = read(&book(&g, &[]));
        wb.sheets[0].page_setup.clone()
    };
    let range = |p: Option<PageSetupData>| p.and_then(|p| p.print_range);
    let a = biff::ptg_area3d(1, 4, 2, 3);
    let b = biff::ptg_area3d(8, 9, 0, 0);
    // One area; without other page records the rest is the default page.
    let p = area(biff::builtin_name(0x06, 1, &a)).unwrap();
    let expected = CellRangeLocal {
        r0: 1,
        c0: 2,
        r1: 4,
        c1: 3,
    };
    assert_eq!(p.print_range, Some(expected));
    assert_eq!(
        PageSetupData {
            print_range: None,
            ..p
        },
        PageSetupData::default()
    );
    // One cell.
    assert_eq!(
        range(area(biff::builtin_name(0x06, 1, &biff::ptg_ref3d(7, 2)))),
        Some(CellRangeLocal {
            r0: 7,
            c0: 2,
            r1: 7,
            c1: 2
        })
    );
    // The column's "relative" bits are not part of the column.
    let mut rel = biff::ptg_area3d(1, 4, 2, 3);
    rel[8] |= 0xC0;
    rel[10] |= 0xC0;
    assert_eq!(
        range(area(biff::builtin_name(0x06, 1, &rel))),
        Some(expected)
    );
    // Several areas are no single print range; of two names the last
    // counts.
    assert_eq!(
        range(area(biff::builtin_name(0x06, 1, &biff::ptg_union(&a, &b)))),
        None
    );
    let mut two = biff::builtin_name(0x06, 1, &b);
    two.extend(biff::builtin_name(0x06, 1, &a));
    assert_eq!(range(area(two)), Some(expected));
    // Not for a sheet (global), a sheet that is not there, another
    // built-in name, an empty formula, a function call, an inverted area.
    // (A formula cut inside a token is refused by calamine, with or
    // without this reader; `format.rs` tests it on the record alone.)
    let mut call = a.clone();
    call.extend_from_slice(&[0x42, 1, 4, 0]);
    let inverted = biff::ptg_area3d(4, 1, 2, 3);
    for name in [
        biff::builtin_name(0x06, 0, &a),
        biff::builtin_name(0x06, 2, &a),
        biff::builtin_name(0x06, 0xFFFF, &a),
        biff::builtin_name(0x0D, 1, &a),
        biff::builtin_name(0x06, 1, &[]),
        biff::builtin_name(0x06, 1, &call),
        biff::builtin_name(0x06, 1, &inverted),
    ] {
        assert_eq!(area(name), None);
    }
    // Titles: rows over all 256 columns, columns over all rows; an area
    // that is neither names nothing.
    let titles = |rgce: &[u8]| {
        area(biff::builtin_name(0x07, 1, rgce)).map(|p| (p.repeat_rows, p.repeat_cols))
    };
    assert_eq!(
        titles(&biff::ptg_area3d(2, 3, 0, 255)),
        Some((Some((2, 3)), None))
    );
    assert_eq!(
        titles(&biff::ptg_area3d(0, 65_535, 1, 2)),
        Some((None, Some((1, 2))))
    );
    assert_eq!(titles(&a), None);
}

#[test]
fn page_setup_is_per_sheet() {
    let mut g = biff::globals(&[], &[], &[]);
    g.extend(biff::externsheet(2));
    g.extend(biff::builtin_name(0x06, 2, &biff::ptg_area3d(0, 9, 0, 1)));
    let mut one = biff::number(0, 0, 15, 1.0);
    one.extend(biff::setup(&Setup {
        flags: 0,
        ..Setup::default()
    }));
    one.extend(biff::page_breaks(true, &[4]));
    let two = biff::number(0, 0, 15, 1.0);
    let wb = read(&biff::file(biff::stream(&g, &[("One", one), ("Two", two)])));
    let p = wb.sheets[0].page_setup.as_ref().unwrap();
    assert_eq!(p.orientation, OrientationData::Landscape);
    assert_eq!(p.print_range, None);
    assert_eq!(wb.sheets[0].row_breaks, vec![4]);
    let p = wb.sheets[1].page_setup.as_ref().unwrap();
    assert_eq!(p.orientation, OrientationData::Portrait);
    assert_eq!(
        p.print_range,
        Some(CellRangeLocal {
            r0: 0,
            c0: 0,
            r1: 9,
            c1: 1
        })
    );
    assert!(wb.sheets[1].row_breaks.is_empty());
}

#[test]
fn page_breaks_are_bounded() {
    let rows: Vec<u16> = (1..=1_000).collect();
    let mut page = biff::page_breaks(true, &rows);
    // Columns past the 256th are no column of the sheet.
    page.extend(biff::page_breaks(false, &[1, 255, 256, 4_000]));
    let bytes = book(&[], &page);
    let wb = read(&bytes);
    assert_eq!(wb.sheets[0].row_breaks.len(), 1_000);
    assert_eq!(wb.sheets[0].col_breaks, vec![1, 255]);
    assert_eq!(wb.warnings, Vec::<String>::new());
    let limits = Limits {
        max_page_breaks: 10,
        ..Limits::default()
    };
    let wb = read_xls(&bytes, &limits).unwrap();
    assert_eq!(wb.sheets[0].row_breaks, (1..=10).collect::<Vec<u32>>());
    assert!(
        wb.warnings.iter().any(|w| w.contains("manual page breaks")),
        "{:?}",
        wb.warnings
    );
    // A later record replaces an earlier one.
    let mut page = biff::page_breaks(true, &[7, 8]);
    page.extend(biff::page_breaks(true, &[2]));
    assert_eq!(read(&book(&[], &page)).sheets[0].row_breaks, vec![2]);
}

#[test]
fn the_page_fixture() {
    let wb = read(&biff::books::page());
    assert_eq!(wb.warnings, Vec::<String>::new());
    let one = &wb.sheets[0];
    let p = one.page_setup.as_ref().unwrap();
    assert_eq!(p.orientation, OrientationData::Landscape);
    assert_eq!(p.scale, PageScaleData::Percent(80));
    assert_eq!(p.repeat_rows, Some((0, 1)));
    assert_eq!(p.repeat_cols, Some((0, 0)));
    assert!(p.print_range.is_some() && p.header.is_some() && p.footer.is_none());
    assert_eq!(
        (one.row_breaks.clone(), one.col_breaks.clone()),
        (vec![5, 20], vec![3])
    );
    // Row 2 (default format): A2, B2 and D2 are formatted empty cells
    // around the number in C2; B3 keeps the default look in the bordered
    // column B; row 4 is hidden.
    let at: Vec<(u32, u32)> = one.cells.iter().map(|c| (c.row, c.col)).collect();
    assert_eq!(
        at,
        vec![
            (0, 0),
            (0, 1),
            (0, 3),
            (1, 0),
            (1, 1),
            (1, 2),
            (1, 3),
            (2, 1)
        ]
    );
    assert!(one
        .row_heights
        .iter()
        .any(|r| (r.start, r.end, r.hidden) == (3, 3, true)));
    let two = wb.sheets[1].page_setup.as_ref().unwrap();
    assert_eq!(
        two.scale,
        PageScaleData::FitTo {
            width: 1,
            height: 0
        }
    );
    assert_eq!(
        two.footer.as_ref().map(|f| f.center.as_str()),
        Some("&[SHEET]")
    );
    assert_eq!(two.header, None);
}
