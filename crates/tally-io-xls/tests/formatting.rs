//! The BIFF8 formatting reader on synthetic workbooks: column widths, row
//! heights, cell formats, sheet view settings, pictures and notes.
#![allow(clippy::unwrap_used)]

mod biff;

use std::path::{Path, PathBuf};

use tally_io_xls::{read_xls, Limits};
use tally_model::{
    BorderStyle, CellContentData, CellData, CellProtectionData, DimRun, HAlign, SheetData,
    SplitData, StyleData, UnderlineStyleData, VAlign, ViewOptionsData, WorkbookData,
};
use tally_types::{Value, MAX_COLS, MAX_ROWS};

fn read(bytes: &[u8]) -> WorkbookData {
    read_xls(bytes, &Limits::default()).unwrap()
}

fn cell(s: &SheetData, r: u32, c: u32) -> &CellData {
    s.cells
        .iter()
        .find(|x| x.row == r && x.col == c)
        .unwrap_or_else(|| panic!("no cell at {r},{c}"))
}

fn style<'a>(wb: &'a WorkbookData, s: &SheetData, r: u32, c: u32) -> &'a StyleData {
    &wb.styles[cell(s, r, c).style.unwrap() as usize]
}

fn run(start: u32, end: u32, size_hmm: u32, hidden: bool) -> DimRun {
    DimRun {
        start,
        end,
        size_hmm,
        hidden,
    }
}

/// A column width of `w`/256 characters as OpenOffice converts it
/// (`XclTools::GetScColumnWidth`) for the fixtures' font 0 of 10 pt: a
/// character is 11 × 200 / 20 = 110 twips wide (the fallback of
/// `XclRoot::SetCharWidth`), the width is rounded to twips.
fn width(w: u16) -> u32 {
    let twips = (f64::from(w) / 256.0 * 110.0 + 0.5) as u32;
    tally_io_xlsx::pt_to_hmm(f64::from(twips) / 20.0)
}

/// OpenOffice's standard column width, 1285 twips (`STD_COL_WIDTH`).
const STD_COL: u32 = 2267;
/// OpenOffice's standard row height, 256 twips.
const STD_ROW: u32 = 452;

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/xls")
}

#[test]
fn column_widths_and_hidden_columns() {
    let wb = read(&biff::books::formatted());
    assert_eq!(wb.warnings, Vec::<String>::new());
    let s = &wb.sheets[0];
    // 10 characters: 1100 twips. DEFCOLWIDTH 8: 8 × 256 + 40960 / (200 −
    // 15) + 50 = 2319 (1/256 character) = 996 twips.
    assert_eq!((width(2560), width(2319)), (1940, 1757));
    assert_eq!(
        s.col_widths,
        vec![
            run(0, 0, 1940, false),
            run(1, 1, 1757, false),
            // Hidden by its flag; the width (4 characters, 440 twips) is
            // kept.
            run(2, 3, 776, true),
            // Width 0 is a hidden column of the default width.
            run(4, 4, 1757, true),
            run(5, 5, 1757, false),
            run(6, 6, 388, false),
            // `first > last` (9..8) is ignored; a range that reaches the
            // last BIFF column runs to the end of the sheet.
            run(7, 9, 1757, false),
            run(10, MAX_COLS - 1, 970, false),
        ]
    );
    // Sheet "Two" has no column records: OpenOffice's standard width.
    assert_eq!(
        wb.sheets[1].col_widths,
        vec![run(0, MAX_COLS - 1, STD_COL, false)]
    );
}

#[test]
fn default_column_widths_fill_the_columns_without_a_record() {
    let book = |fonts: &[biff::Font], extra: Vec<u8>| {
        let mut sheet = biff::colinfo(1, 1, 2560, 15, 0);
        sheet.extend(extra);
        sheet.extend(biff::number(0, 0, 15, 1.0));
        read(&biff::file(biff::stream(
            &biff::globals(fonts, &[], &[]),
            &[("S", sheet)],
        )))
    };
    let widths = |wb: &WorkbookData, default: u32, b: u32| {
        assert_eq!(
            wb.sheets[0].col_widths,
            vec![
                run(0, 0, default, false),
                run(1, 1, b, false),
                run(2, MAX_COLS - 1, default, false),
            ]
        );
    };
    // No default width record: OpenOffice's standard width.
    widths(&book(&[], Vec::new()), STD_COL, 1940);
    // DEFCOLWIDTH counts characters and gets Excel's extra room for the
    // font: 8 × 256 + 40960 / max(200 − 15, 60) + 50 = 2319.4 → 2319.
    widths(&book(&[], biff::defcolwidth(8)), 1757, 1940);
    // 4 × 256 + 271.4 = 1295 → 1295 / 256 × 110 + 0.5 = 556 twips.
    assert_eq!(width(1295), tally_io_xlsx::pt_to_hmm(27.8));
    widths(&book(&[], biff::defcolwidth(4)), 981, 1940);
    // STANDARDWIDTH is in 1/256 character, has no extra room and wins
    // over DEFCOLWIDTH, whatever their order.
    widths(&book(&[], biff::standardwidth(1024)), 776, 1940);
    let mut both = biff::standardwidth(1024);
    both.extend(biff::defcolwidth(8));
    widths(&book(&[], both), 776, 1940);
    // A font 0 of 20 pt: a character is 11 × 400 / 20 = 220 twips, and
    // DEFCOLWIDTH 8 is 2048 + 40960 / 385 + 50 = 2204 → 1894 twips.
    let big = [biff::Font {
        twips: 400,
        ..biff::Font::default()
    }];
    widths(
        &book(&big, biff::defcolwidth(8)),
        tally_io_xlsx::pt_to_hmm(1894.0 / 20.0),
        tally_io_xlsx::pt_to_hmm(2200.0 / 20.0),
    );
    // A font below 3.75 pt: the correction's divisor stops at 60.
    let tiny = [biff::Font {
        twips: 40,
        ..biff::Font::default()
    }];
    // 2048 + 40960 / 60 + 50 = 2780 (1/256 of 22 twips) = 239 twips.
    widths(
        &book(&tiny, biff::defcolwidth(8)),
        tally_io_xlsx::pt_to_hmm(239.0 / 20.0),
        tally_io_xlsx::pt_to_hmm(220.0 / 20.0),
    );
}

#[test]
fn every_row_has_the_height_of_the_file() {
    let wb = read(&biff::books::formatted());
    let s = &wb.sheets[0];
    assert_eq!(
        s.row_heights,
        vec![
            // DEFAULTROWHEIGHT: 255 twips = 12.75 pt.
            run(0, 0, 450, false),
            // 600 twips = 30 pt, set by hand ("unsynced").
            run(1, 1, tally_io_xlsx::pt_to_hmm(30.0), false),
            // Row 2 has an automatic height of 15 pt: shown at it.
            run(2, 2, tally_io_xlsx::pt_to_hmm(15.0), false),
            // Row 3 is hidden.
            run(3, 3, 450, true),
            run(4, 4, 450, false),
            run(5, 5, tally_io_xlsx::pt_to_hmm(20.0), false),
            run(6, MAX_ROWS - 1, 450, false),
        ]
    );
    // Sheet "Two" has no row records at all: OpenOffice's standard
    // height, in one run.
    assert_eq!(
        wb.sheets[1].row_heights,
        vec![run(0, MAX_ROWS - 1, STD_ROW, false)]
    );
}

/// A sheet with the given row records around one number.
fn rows_book(records: Vec<u8>) -> WorkbookData {
    let mut sheet = records;
    sheet.extend(biff::number(0, 0, 15, 1.0));
    read(&biff::file(biff::stream(
        &biff::globals(&[], &[], &[]),
        &[("S", sheet)],
    )))
}

#[test]
fn row_heights_follow_openoffice() {
    // DEFAULTROWHEIGHT 20 pt; row 1: 30 pt, automatic; row 2: 30 pt, set
    // by hand; row 4: the default height (bit 0x8000; its own number is
    // ignored); row 6: height 0 = hidden; row 8: hidden by its flag.
    let mut s = biff::defaultrowheight(1, 400);
    s.extend(biff::row(1, 600, 0x0100));
    s.extend(biff::row(2, 600, 0x0140));
    s.extend(biff::row(4, 0x8000 | 900, 0x0100));
    s.extend(biff::row(6, 0, 0x0100));
    s.extend(biff::row(8, 300, 0x0120));
    let wb = rows_book(s);
    let (d, h30) = (
        tally_io_xlsx::pt_to_hmm(20.0),
        tally_io_xlsx::pt_to_hmm(30.0),
    );
    assert_eq!((d, h30), (706, 1058));
    assert_eq!(
        wb.sheets[0].row_heights,
        vec![
            run(0, 0, d, false),
            run(1, 2, h30, false),
            run(3, 5, d, false),
            run(6, 6, d, true),
            run(7, 7, d, false),
            run(8, 8, tally_io_xlsx::pt_to_hmm(15.0), true),
            run(9, MAX_ROWS - 1, d, false),
        ]
    );
    assert_eq!(wb.warnings, Vec::<String>::new());
}

#[test]
fn rows_hidden_by_default_stay_runs() {
    // DEFAULTROWHEIGHT bit 1: the rows without a record are hidden. Row 2
    // has a height of its own; row 3 takes the default, hidden with it.
    for default in [biff::defaultrowheight(2, 300), biff::defaultrowheight(0, 0)] {
        let zero = default[6..8] == [0, 0];
        let mut s = default;
        s.extend(biff::row(2, 500, 0x0140));
        s.extend(biff::row(3, 0x8000 | 255, 0x0100));
        let wb = rows_book(s);
        // A default height of 0 is OpenOffice's standard height, hidden.
        let d = if zero {
            STD_ROW
        } else {
            tally_io_xlsx::pt_to_hmm(15.0)
        };
        assert_eq!(
            wb.sheets[0].row_heights,
            vec![
                run(0, 1, d, true),
                run(2, 2, tally_io_xlsx::pt_to_hmm(25.0), false),
                run(3, MAX_ROWS - 1, d, true),
            ]
        );
    }
}

#[test]
fn sixty_thousand_rows_are_runs() {
    // Every row has a record: equal neighbours are one run, and the rows
    // after the last record are one more.
    let mut s = biff::defaultrowheight(0, 255);
    for r in 0..60_000u32 {
        s.extend(biff::row(r as u16, 300 + (r / 20_000) as u16, 0x0100));
    }
    let wb = rows_book(s);
    assert_eq!(
        wb.sheets[0]
            .row_heights
            .iter()
            .map(|r| (r.start, r.end))
            .collect::<Vec<_>>(),
        vec![
            (0, 19_999),
            (20_000, 39_999),
            (40_000, 59_999),
            (60_000, MAX_ROWS - 1)
        ]
    );
}

#[test]
fn fonts_follow_the_xf_and_skip_index_4() {
    let wb = read(&biff::books::formatted());
    let s = &wb.sheets[0];
    assert_eq!(wb.settings.default_font.as_deref(), Some("Arial"));
    // XF 16 → font 5 = the fifth FONT record.
    let st = style(&wb, s, 0, 0);
    assert_eq!(st.font.family.as_deref(), Some("Times New Roman"));
    assert_eq!(st.font.size_pt, None, "11 pt is the size cells default to");
    assert!(st.font.bold && st.font.italic && !st.font.underline && !st.font.strike);
    assert_eq!(st.font.color, Some(0xFF00_00FF));
    // XF 17 → font 6; its colour is palette entry 8, replaced by PALETTE.
    let st = style(&wb, s, 0, 1);
    assert_eq!(st.font.family.as_deref(), Some("Courier New"));
    assert_eq!(st.font.size_pt, Some(8.0));
    assert!(st.font.underline && st.font.strike && !st.font.bold);
    assert_eq!(st.font.underline_style, UnderlineStyleData::Double);
    assert_eq!(st.font.color, Some(0x1122_33FF));
    // The default font: no family (the document default), its own size,
    // automatic colour.
    let st = style(&wb, s, 1, 2);
    assert_eq!(st.font.family, None);
    assert_eq!(st.font.size_pt, Some(10.0));
    assert_eq!(st.font.color, None);
}

#[test]
fn number_formats_and_dates() {
    let wb = read(&biff::books::formatted());
    let s = &wb.sheets[0];
    assert_eq!(style(&wb, s, 0, 0).number_format.as_deref(), Some("0.000"));
    assert_eq!(
        style(&wb, s, 1, 0).number_format.as_deref(),
        Some("YYYY-MM-DD")
    );
    assert_eq!(style(&wb, s, 1, 2).number_format.as_deref(), Some("H:MM"));
    // Built-in format 4.
    assert_eq!(
        style(&wb, s, 4, 0).number_format.as_deref(),
        Some("#,##0.00")
    );
    let num = |r, c| match &cell(s, r, c).content {
        CellContentData::Value(Value::Number(n)) => *n,
        other => panic!("{other:?}"),
    };
    assert_eq!(num(1, 0), 45000.0);
    // A date before 1900-03-01: Excel's serial 30 is Tally's 31.
    assert_eq!(num(1, 1), 31.0);
    // A time keeps its serial.
    assert_eq!(num(1, 2), 0.5);
}

#[test]
fn date_guess_is_the_fallback_without_a_format() {
    // No XF is read: calamine still knows the cells are dates and times.
    let limits = Limits {
        max_xfs: 0,
        ..Limits::default()
    };
    let wb = read_xls(&biff::books::formatted(), &limits).unwrap();
    assert!(wb.warnings.iter().any(|w| w.contains("cell formats")));
    let s = &wb.sheets[0];
    let st = |r, c| {
        wb.styles[cell(s, r, c).style.unwrap() as usize]
            .number_format
            .clone()
    };
    assert_eq!(st(1, 0).as_deref(), Some("YYYY-MM-DD"));
    assert_eq!(st(1, 2).as_deref(), Some("HH:MM:SS"));
    assert_eq!(
        cell(s, 1, 1).content,
        CellContentData::Value(Value::Number(31.0))
    );
    assert_eq!(
        cell(s, 1, 2).content,
        CellContentData::Value(Value::Number(0.5))
    );
    assert_eq!(cell(s, 0, 0).style, None);
}

#[test]
fn alignment_protection_fill_and_borders() {
    let wb = read(&biff::books::formatted());
    let s = &wb.sheets[0];
    let st = style(&wb, s, 0, 0);
    assert_eq!((st.align.h, st.align.v), (HAlign::Center, VAlign::Middle));
    assert!(st.align.wrap);
    assert_eq!(st.protection, None);

    let st = style(&wb, s, 1, 0);
    assert_eq!(st.fill, Some(0xFF00_00FF));
    assert_eq!(
        st.protection,
        Some(CellProtectionData {
            locked: false,
            ..CellProtectionData::default()
        })
    );

    let st = style(&wb, s, 1, 2);
    assert_eq!(st.align.h, HAlign::Right);
    assert_eq!(st.align.indent, 2);
    assert_eq!(st.align.rotation_deg, 45);

    let st = style(&wb, s, 2, 0);
    let b = &st.borders;
    let line =
        |l: &Option<tally_model::BorderLine>| l.as_ref().map(|l| (l.style, l.width_hmm, l.color));
    assert_eq!(line(&b.left), Some((BorderStyle::Solid, 26, 0x0000_FFFF)));
    assert_eq!(line(&b.right), Some((BorderStyle::Double, 79, 0x0000_00FF)));
    assert_eq!(line(&b.top), Some((BorderStyle::Dashed, 26, 0x0000_00FF)));
    assert_eq!(line(&b.bottom), Some((BorderStyle::Solid, 79, 0x4455_66FF)));
    assert_eq!(
        line(&b.diagonal_down),
        Some((BorderStyle::Dotted, 9, 0x0000_00FF))
    );
    assert_eq!(b.diagonal_up, None);

    // A 50 % pattern is shown as the mix of its two colours.
    assert_eq!(style(&wb, s, 2, 1).fill, Some(0x2A3B_4CFF));

    let st = style(&wb, s, 4, 0);
    assert!(st.align.shrink_to_fit);
    assert_eq!(st.align.rotation_deg, -45);
    assert_eq!(
        st.protection,
        Some(CellProtectionData {
            hidden_formula: true,
            ..CellProtectionData::default()
        })
    );
    let st = style(&wb, s, 4, 3);
    assert!(st.align.stacked);
    assert_eq!(st.align.rotation_deg, 0);
    assert_eq!((st.align.h, st.align.v), (HAlign::Justify, VAlign::Top));
}

#[test]
fn every_cell_record_carries_its_xf_and_equal_xfs_share_a_style() {
    let wb = read(&biff::books::formatted());
    let s = &wb.sheets[0];
    // LABELSST (XF 16), MULRK's second cell (XF 21, equal to 16) and
    // BOOLERR (XF 16) share one style.
    let shared = cell(s, 0, 0).style;
    assert!(shared.is_some());
    assert_eq!(cell(s, 2, 2).style, shared);
    assert_eq!(cell(s, 4, 1).style, shared);
    // LABEL and NUMBER (XF 17).
    assert_eq!(cell(s, 0, 1).style, cell(s, 1, 0).style);
    // RK and BLANK (XF 19), MULRK's first cell and MULBLANK's (XF 20).
    assert_eq!(cell(s, 2, 0).style, cell(s, 3, 0).style);
    assert_eq!(cell(s, 2, 1).style, cell(s, 3, 1).style);
    // FORMULA (XF 22) and RSTRING (XF 18).
    assert!(matches!(
        cell(s, 4, 0).content,
        CellContentData::Formula { .. }
    ));
    assert!(style(&wb, s, 4, 0).align.shrink_to_fit);
    assert_eq!(cell(s, 4, 2).style, cell(s, 1, 2).style);
    // Values survive next to their formats.
    assert_eq!(
        cell(s, 0, 0).content,
        CellContentData::Value(Value::text("shared"))
    );
    assert_eq!(
        cell(s, 2, 2).content,
        CellContentData::Value(Value::Number(2.0))
    );
    // No two styles are equal.
    for (i, a) in wb.styles.iter().enumerate() {
        assert!(!wb.styles[..i].contains(a), "style {i} is a duplicate");
    }
}

#[test]
fn blank_cells_are_styled_empty_cells() {
    let wb = read(&biff::books::formatted());
    let s = &wb.sheets[0];
    // BLANK with borders, MULBLANK's first cell with a fill.
    for c in [0, 1] {
        assert_eq!(cell(s, 3, c).content, CellContentData::Value(Value::Empty));
        assert!(cell(s, 3, c).style.is_some());
    }
    // The MULBLANK cells with nothing but the default font are no cells.
    assert!(!s.cells.iter().any(|x| x.row == 3 && x.col >= 2));
    // Cells are row-major.
    let at: Vec<(u32, u32)> = s.cells.iter().map(|c| (c.row, c.col)).collect();
    let mut sorted = at.clone();
    sorted.sort_unstable();
    assert_eq!(at, sorted);
}

#[test]
fn blank_cells_respect_the_cell_budget() {
    // One value and 200 formatted blanks, with room for 10 cells.
    let xfs = [biff::Xf {
        pattern: 1,
        fore: 10,
        ..biff::Xf::default()
    }];
    let mut sheet = biff::number(0, 0, 15, 1.0);
    sheet.extend(biff::mulblank(1, 0, &[16; 200]));
    let bytes = biff::file(biff::stream(
        &biff::globals(&[], &[], &xfs),
        &[("S", sheet)],
    ));
    let limits = Limits {
        max_cells: 10,
        ..Limits::default()
    };
    let wb = read_xls(&bytes, &limits).unwrap();
    assert_eq!(wb.sheets[0].cells.len(), 10);
    assert!(wb.warnings.iter().any(|w| w.contains("empty cells")));
    // The formats kept are capped too.
    let limits = Limits {
        max_styled_cells: 5,
        ..Limits::default()
    };
    let wb = read_xls(&bytes, &limits).unwrap();
    assert_eq!(wb.sheets[0].cells.len(), 6);
    assert!(wb.warnings.iter().any(|w| w.contains("first 5 cells")));
}

#[test]
fn column_default_format_becomes_a_style_run() {
    let wb = read(&biff::books::formatted());
    let s = &wb.sheets[0];
    // COLINFO 6..6 carries XF 19 (borders); the others XF 15.
    assert_eq!(s.style_runs.len(), 1);
    let r = s.style_runs[0];
    assert_eq!((r.col, r.start_row, r.end_row), (6, 0, MAX_ROWS - 1));
    assert_eq!(Some(r.style), cell(s, 2, 0).style);
}

#[test]
fn sheet_view_settings() {
    let wb = read(&biff::books::formatted());
    let one = &wb.sheets[0];
    assert_eq!(
        one.view_options,
        Some(ViewOptionsData {
            show_formulas: true,
            value_highlighting: false,
            show_zero_values: false,
            show_gridlines: false,
            show_headers: false,
        })
    );
    assert_eq!(one.frozen, Some((1, 2)));
    assert_eq!(one.split, None);
    assert_eq!(one.zoom, Some(150));
    assert_eq!(one.tab_color, Some(0xFF00_00FF));
    let two = &wb.sheets[1];
    assert_eq!(two.view_options, None);
    assert_eq!(two.frozen, None);
    assert_eq!(
        two.split,
        Some(SplitData {
            x_hmm: 2540,
            y_hmm: 1270
        })
    );
    assert_eq!((two.zoom, two.tab_color), (None, None));
    // WINDOW1 selects the second tab.
    assert_eq!(wb.settings.active_sheet, 1);
}

#[test]
fn pictures_are_anchored_and_other_objects_are_reported() {
    let wb = read(&biff::books::pictures());
    let s = &wb.sheets[0];
    let kinds: Vec<(&str, usize)> = s
        .images
        .iter()
        .map(|i| (i.mime.as_str(), i.bytes.len()))
        .collect();
    // PNG, JPEG and a PNG stored in a JPEG record (recognized by its
    // signature).
    assert_eq!(
        kinds,
        vec![("image/png", 48), ("image/jpeg", 24), ("image/png", 20)]
    );
    assert_eq!(&s.images[0].bytes[..8], b"\x89PNG\r\n\x1a\n");
    // Column 1 is 1940 wide, row 2 is 20 pt high; the first picture starts
    // in the middle of both and ends at the start of column 3 and row 4,
    // across a column and a row of the sizes the sheet shows them at.
    let a = &s.images[0].anchor;
    let h = tally_io_xlsx::pt_to_hmm(20.0);
    assert_eq!((a.row, a.col), (2, 1));
    assert_eq!((a.dx_hmm, a.dy_hmm), (970, h / 2));
    assert_eq!(a.width_hmm, 970 + STD_COL);
    assert_eq!(a.height_hmm, h / 2 + STD_ROW);
    assert_eq!(s.col_widths[2], run(2, MAX_COLS - 1, STD_COL, false));
    assert_eq!(s.row_heights[2], run(3, MAX_ROWS - 1, STD_ROW, false));
    // The third one covers nearly all of F6.
    let a = &s.images[2].anchor;
    assert_eq!((a.row, a.col, a.dx_hmm, a.dy_hmm), (5, 5, 0, 0));
    assert_eq!(a.width_hmm, STD_COL * 1023 / 1024);
    assert_eq!(a.height_hmm, STD_ROW * 255 / 256);
    // The EMF is one warning, the text box and the picture whose BLIP is
    // missing another.
    assert_eq!(wb.warnings.len(), 2, "{:?}", wb.warnings);
    assert!(wb.warnings.iter().any(|w| w.starts_with("1 picture(s)")));
    assert!(wb
        .warnings
        .iter()
        .any(|w| w.starts_with("2 drawing object(s)")));
}

#[test]
fn picture_budgets() {
    let bytes = biff::books::pictures();
    let read = |limits: Limits| read_xls(&bytes, &limits).unwrap();
    let wb = read(Limits {
        max_images: 1,
        ..Limits::default()
    });
    assert_eq!(wb.sheets[0].images.len(), 1);
    assert!(wb.warnings.iter().any(|w| w.contains("first 1 pictures")));
    // The 48-byte PNG is over the size of one picture; the others fit.
    let wb = read(Limits {
        max_image_bytes: 30,
        ..Limits::default()
    });
    assert_eq!(wb.sheets[0].images.len(), 2);
    // 48 + 24 bytes fit in 80, the third picture does not.
    let wb = read(Limits {
        max_total_image_bytes: 80,
        ..Limits::default()
    });
    assert_eq!(wb.sheets[0].images.len(), 2);
    // No drawing data at all: no pictures, the cells are still there.
    let wb = read(Limits {
        max_drawing_bytes: 0,
        max_total_image_bytes: 0,
        ..Limits::default()
    });
    assert!(wb.sheets[0].images.is_empty());
    assert_eq!(wb.sheets[0].cells.len(), 1);
}

#[test]
fn cell_notes() {
    let wb = read(&biff::books::notes());
    assert_eq!(wb.warnings, Vec::<String>::new());
    let s = &wb.sheets[0];
    assert_eq!(s.notes.len(), 2);
    let n = &s.notes[0];
    assert_eq!((n.row, n.col, n.shown), (1, 1, true));
    assert_eq!(n.text, "first note");
    assert_eq!(n.author.as_deref(), Some("An Author"));
    let n = &s.notes[1];
    assert_eq!((n.row, n.col, n.shown), (3, 2, false));
    assert_eq!(n.text, "two\nlines");
    assert_eq!(n.author, None);
    let wb = read_xls(
        &biff::books::notes(),
        &Limits {
            max_notes: 1,
            ..Limits::default()
        },
    )
    .unwrap();
    assert_eq!(wb.sheets[0].notes.len(), 1);
    assert!(wb.warnings.iter().any(|w| w.contains("cell notes")));
}

#[test]
fn biff5_opens_with_values_only_and_a_warning() {
    let mut sheet = biff::colinfo(0, 0, 2560, 15, 0);
    sheet.extend(biff::row(0, 600, 0x140));
    sheet.extend(biff::number(0, 0, 0, 7.0));
    let bytes = biff::file(biff::stream5(&[], &[("Old", sheet)]));
    let wb = read(&bytes);
    assert_eq!(wb.sheets[0].name, "Old");
    assert_eq!(
        wb.sheets[0].cells[0].content,
        CellContentData::Value(Value::Number(7.0))
    );
    assert!(wb.sheets[0].col_widths.is_empty());
    assert!(wb.sheets[0].row_heights.is_empty());
    assert!(wb.styles.is_empty());
    assert_eq!(wb.warnings.len(), 1);
    assert!(wb.warnings[0].contains("Excel 5.0/95"));
}

#[test]
fn rich_text_in_shared_strings_is_flattened() {
    // An SST string with two formatting runs: the cell gets its text and
    // the font of its XF.
    let text = "rich";
    let mut d = Vec::new();
    d.extend_from_slice(&1u32.to_le_bytes());
    d.extend_from_slice(&1u32.to_le_bytes());
    d.extend_from_slice(&(text.len() as u16).to_le_bytes());
    d.push(0x08);
    d.extend_from_slice(&2u16.to_le_bytes());
    d.extend_from_slice(text.as_bytes());
    d.extend_from_slice(&[0, 0, 5, 0, 2, 0, 6, 0]);
    let xfs = [biff::Xf {
        font: 5,
        ..biff::Xf::default()
    }];
    let fonts = [
        biff::Font::default(),
        biff::Font::default(),
        biff::Font::default(),
        biff::Font::default(),
        biff::Font {
            bold: true,
            ..biff::Font::default()
        },
    ];
    let mut g = biff::globals(&fonts, &[], &xfs);
    g.extend(biff::rec(0x00FC, &d));
    let bytes = biff::file(biff::stream(&g, &[("S", biff::labelsst(0, 0, 16, 0))]));
    let wb = read(&bytes);
    let s = &wb.sheets[0];
    assert_eq!(
        cell(s, 0, 0).content,
        CellContentData::Value(Value::text("rich"))
    );
    assert!(style(&wb, s, 0, 0).font.bold);
}

#[test]
fn committed_fixtures_are_what_the_code_builds() {
    for (name, bytes) in biff::books::all() {
        let on_disk = std::fs::read(fixture_dir().join(name)).unwrap();
        assert!(
            on_disk == bytes,
            "{name} is stale: run `cargo test -p tally-io-xls --test formatting -- --ignored`"
        );
    }
}

/// Writes the synthetic fixtures (the fuzz seeds are generated from them:
/// `cargo test -p tally-fuzz-harness --test generate -- --ignored`).
#[test]
#[ignore = "writes tests/fixtures/xls"]
fn write_fixtures() {
    for (name, bytes) in biff::books::all() {
        std::fs::write(fixture_dir().join(name), bytes).unwrap();
    }
}
