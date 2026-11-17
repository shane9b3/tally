//! The cases of the formatting reader's review: the cell budget with
//! formatted empty cells, row default formats, cells inside a formatted
//! column, the style table of a workbook with tens of thousands of cell
//! formats, fonts, built-in number formats, the picture store, tab
//! colours and formulas without a value.
#![allow(clippy::unwrap_used)]

mod biff;

use biff::{Font, Xf};
use tally_io_xls::{read_xls, Limits, XlsError};
use tally_model::{CellContentData, CellData, SheetData, StyleData, WorkbookData};
use tally_types::Value;

fn bordered() -> Xf {
    Xf {
        lines: [1, 1, 1, 1, 0],
        ..Xf::default()
    }
}

fn book(globals: &[u8], sheet: Vec<u8>) -> Vec<u8> {
    biff::file(biff::stream(globals, &[("S", sheet)]))
}

fn read(bytes: &[u8]) -> WorkbookData {
    read_xls(bytes, &Limits::default()).unwrap()
}

fn cell(s: &SheetData, r: u32, c: u32) -> Option<&CellData> {
    s.cells.iter().find(|x| x.row == r && x.col == c)
}

/// The style of the cell at (`r`, `c`); `None` for a cell without one.
fn style(wb: &WorkbookData, r: u32, c: u32) -> Option<&StyleData> {
    let cell = cell(&wb.sheets[0], r, c).unwrap_or_else(|| panic!("no cell at {r},{c}"));
    cell.style.map(|i| &wb.styles[i as usize])
}

fn has_border(st: Option<&StyleData>) -> bool {
    st.is_some_and(|s| s.borders.left.is_some())
}

// ------------------------------------------------------------ cell budget

#[test]
fn formatted_empty_cells_never_fail_a_file() {
    let g = biff::globals(&[], &[], &[bordered()]);
    let limits = Limits {
        max_cells: 10,
        ..Limits::default()
    };
    let numbers = |s: &mut Vec<u8>| {
        for c in 0..8 {
            s.extend(biff::number(1, c, 15, 1.0));
        }
    };
    // The values alone fit.
    let mut plain = Vec::new();
    numbers(&mut plain);
    let wb = read_xls(&book(&g, plain), &limits).unwrap();
    assert_eq!(wb.sheets[0].cells.len(), 8);
    assert_eq!(wb.warnings, Vec::<String>::new());
    // Five formatted empty cells before them: the values are all there,
    // the empty cells get the two places that are left.
    let mut before = biff::mulblank(0, 0, &[16; 5]);
    numbers(&mut before);
    let wb = read_xls(&book(&g, before), &limits).unwrap();
    let s = &wb.sheets[0];
    assert_eq!(s.cells.iter().filter(|c| c.row == 1).count(), 8);
    assert_eq!(s.cells.iter().filter(|c| c.row == 0).count(), 2);
    assert_eq!(wb.warnings.len(), 1, "{:?}", wb.warnings);
    assert!(wb.warnings[0].contains("formatting of some empty cells was left out"));
    // The same after the values, and with a formatted column on top.
    let mut after = biff::colinfo(9, 9, 2560, 16, 0);
    numbers(&mut after);
    after.extend(biff::mulblank(2, 0, &[16; 5]));
    let wb = read_xls(&book(&g, after), &limits).unwrap();
    let s = &wb.sheets[0];
    assert_eq!(s.cells.len(), 10);
    assert!(s.style_runs.is_empty(), "no room for the column format");
    assert_eq!(wb.warnings.len(), 1, "{:?}", wb.warnings);
    // One value too many is still an error, as before.
    let mut over = Vec::new();
    for c in 0..11 {
        over.extend(biff::number(1, c, 15, 1.0));
    }
    assert!(matches!(
        read_xls(&book(&g, over), &limits),
        Err(XlsError::LimitExceeded { what: "cells", .. })
    ));
}

#[test]
fn every_file_that_opens_without_formats_opens_with_them() {
    // Formatted empty cells, a row format and a column format around
    // values, with every cell limit from "too small" to "roomy": the
    // result is an error only where the values alone are one.
    let g = biff::globals(&[], &[], &[bordered()]);
    let values = |s: &mut Vec<u8>| {
        for c in 0..6 {
            s.extend(biff::number(2, c, 15, 1.0));
        }
    };
    let mut bare = Vec::new();
    values(&mut bare);
    let mut formatted = biff::colinfo(1, 2, 2560, 16, 0);
    formatted.extend(biff::row(4, 300, 0x0180 | (16 << 16)));
    formatted.extend(biff::mulblank(0, 0, &[16; 12]));
    values(&mut formatted);
    formatted.extend(biff::mulblank(3, 0, &[16; 12]));
    let (bare, formatted) = (book(&g, bare), book(&g, formatted));
    for max_cells in 0..40 {
        let limits = Limits {
            max_cells,
            ..Limits::default()
        };
        let plain = read_xls(&bare, &limits);
        let full = read_xls(&formatted, &limits);
        assert_eq!(plain.is_ok(), full.is_ok(), "max_cells {max_cells}");
        if let Ok(wb) = full {
            let s = &wb.sheets[0];
            assert!(
                (s.cells.len() + s.style_runs.len()) as u64 <= max_cells,
                "max_cells {max_cells}"
            );
            assert_eq!(s.cells.iter().filter(|c| c.row == 2).count(), 6);
        }
    }
}

// ------------------------------------------------------- row default formats

#[test]
fn a_row_default_format_formats_the_empty_cells_of_the_row() {
    let g = biff::globals(&[], &[], &[bordered()]);
    // Row 2: fGhostDirty with XF 16 (bordered); its own cell B3 has the
    // default format. The sheet uses columns A–D.
    let mut s = biff::row(2, 300, 0x0180 | (16 << 16));
    s.extend(biff::number(0, 0, 15, 1.0));
    s.extend(biff::number(0, 3, 15, 1.0));
    s.extend(biff::number(2, 1, 15, 2.0));
    let wb = read(&book(&g, s));
    let sheet = &wb.sheets[0];
    assert_eq!(wb.warnings, Vec::<String>::new());
    let row2: Vec<u32> = sheet
        .cells
        .iter()
        .filter(|c| c.row == 2)
        .map(|c| c.col)
        .collect();
    assert_eq!(row2, vec![0, 1, 2, 3]);
    for c in [0, 2, 3] {
        assert!(has_border(style(&wb, 2, c)), "column {c}");
        assert_eq!(
            cell(sheet, 2, c).unwrap().content,
            CellContentData::Value(Value::Empty)
        );
    }
    // The cell with a record keeps its own (default) format.
    assert!(!has_border(style(&wb, 2, 1)));
    // Without the flag the XF bits mean nothing.
    let mut s = biff::row(2, 300, 0x0100 | (16 << 16));
    s.extend(biff::number(0, 3, 15, 1.0));
    let wb = read(&book(&g, s));
    assert_eq!(wb.sheets[0].cells.len(), 1);
}

#[test]
fn a_row_default_format_wins_over_a_column_format() {
    let g = biff::globals(&[], &[], &[bordered()]);
    // Column B is bordered; row 1 has the plain default format, row 2 a
    // bordered one, row 3 a format index without a target.
    let mut s = biff::colinfo(1, 1, 2560, 16, 0);
    s.extend(biff::row(1, 300, 0x0180 | (15 << 16)));
    s.extend(biff::row(2, 300, 0x0180 | (16 << 16)));
    s.extend(biff::row(3, 300, 0x0180 | (900 << 16)));
    s.extend(biff::number(0, 0, 15, 1.0));
    let wb = read(&book(&g, s));
    let sheet = &wb.sheets[0];
    assert_eq!(sheet.style_runs.len(), 1);
    // Row 1: only B2 needs a cell (the default look, not the column's).
    let row1: Vec<u32> = sheet
        .cells
        .iter()
        .filter(|c| c.row == 1)
        .map(|c| c.col)
        .collect();
    assert_eq!(row1, vec![1]);
    assert_eq!(style(&wb, 1, 1), Some(&StyleData::default()));
    assert_eq!(style(&wb, 3, 1), Some(&StyleData::default()));
    // Row 2: A3 and B3 bordered.
    assert!(has_border(style(&wb, 2, 0)));
    assert!(has_border(style(&wb, 2, 1)));
}

#[test]
fn row_default_formats_stay_within_the_styled_cell_budget() {
    let g = biff::globals(&[], &[], &[bordered()]);
    let mut s = Vec::new();
    for r in 0..100u16 {
        s.extend(biff::row(r, 300, 0x0180 | (16 << 16)));
    }
    // The sheet uses 256 columns: 25,600 cells to format.
    s.extend(biff::number(0, 255, 15, 1.0));
    let bytes = book(&g, s);
    let wb = read(&bytes);
    assert_eq!(wb.sheets[0].cells.len(), 25_600);
    assert_eq!(wb.warnings, Vec::<String>::new());
    let limits = Limits {
        max_styled_cells: 1_000,
        ..Limits::default()
    };
    let wb = read_xls(&bytes, &limits).unwrap();
    // The value and the first 999 empty cells (the value's record came
    // first and has no format worth a place).
    assert_eq!(wb.sheets[0].cells.len(), 1_000);
    assert!(
        wb.warnings
            .iter()
            .any(|w| w.contains("formatting of the first 1000 cells")),
        "{:?}",
        wb.warnings
    );
}

// ------------------------------------------------ cells in a formatted column

#[test]
fn cells_in_a_formatted_column_keep_their_own_format() {
    let g = biff::globals(&[], &[], &[bordered()]);
    let mut s = biff::colinfo(1, 1, 2560, 16, 0);
    // In the bordered column B: an empty cell and a number with the
    // default format, and an empty cell with the column's.
    s.extend(biff::blank(3, 1, 15));
    s.extend(biff::number(4, 1, 15, 1.0));
    s.extend(biff::blank(5, 1, 16));
    s.extend(biff::mulblank(6, 0, &[15, 15, 15]));
    // Outside it nothing changes: a default empty cell is no cell.
    s.extend(biff::blank(3, 0, 15));
    s.extend(biff::number(4, 0, 15, 1.0));
    let wb = read(&book(&g, s));
    let sheet = &wb.sheets[0];
    assert_eq!(sheet.style_runs.len(), 1);
    assert!(has_border(Some(
        &wb.styles[sheet.style_runs[0].style as usize]
    )));
    assert_eq!(style(&wb, 3, 1), Some(&StyleData::default()));
    assert_eq!(style(&wb, 4, 1), Some(&StyleData::default()));
    assert!(has_border(style(&wb, 5, 1)));
    assert_eq!(style(&wb, 6, 1), Some(&StyleData::default()));
    assert!(cell(sheet, 3, 0).is_none());
    assert!(cell(sheet, 6, 0).is_none() && cell(sheet, 6, 2).is_none());
    assert_eq!(style(&wb, 4, 0), None);
    // One "default look" style serves them all.
    let plain = wb
        .styles
        .iter()
        .filter(|s| **s == StyleData::default())
        .count();
    assert_eq!(plain, 1);
}

// ------------------------------------------------------------ style table

#[test]
fn equal_cell_formats_share_a_style() {
    // 3,000 XFs of 30 distinct looks.
    let xfs: Vec<Xf> = (0..3_000u32)
        .map(|i| Xf {
            indent: (i % 15) as u8,
            wrap: i % 2 == 0,
            ..Xf::default()
        })
        .collect();
    let g = biff::globals(&[], &[], &xfs);
    let mut s = Vec::new();
    for i in 0..3_000u16 {
        s.extend(biff::number(i, 0, 16 + i, 1.0));
    }
    let wb = read(&book(&g, s));
    assert_eq!(wb.styles.len(), 29, "the 30 looks but the default one");
    let a = style(&wb, 7, 0).unwrap();
    assert_eq!((a.align.indent, a.align.wrap), (7, false));
    assert_eq!(
        cell(&wb.sheets[0], 7, 0).unwrap().style,
        cell(&wb.sheets[0], 37, 0).unwrap().style
    );
}

/// `n` number formats of about `len` characters each (ids 200…), and one
/// XF per format.
fn long_formats(n: u16, len: usize) -> Vec<u8> {
    let mut g = Vec::new();
    for i in 0..n {
        let code = format!("0.00\"{}\"", format!("{i:05}").repeat(len / 5));
        g.extend(biff::format(200 + i, &code));
    }
    let xfs: Vec<Xf> = (0..n)
        .map(|i| Xf {
            fmt: 200 + i,
            ..Xf::default()
        })
        .collect();
    let mut all = biff::globals(&[], &[], &xfs);
    all.extend(g);
    all
}

#[test]
fn the_text_of_the_styles_is_bounded() {
    let g = long_formats(200, 1_000);
    let mut s = Vec::new();
    for i in 0..200u16 {
        s.extend(biff::number(i, 0, 16 + i, 1.5));
    }
    let bytes = book(&g, s);
    let text = |wb: &WorkbookData| -> usize {
        wb.styles
            .iter()
            .map(|s| s.number_format.as_ref().map_or(0, String::len))
            .sum()
    };
    // Roomy: every format is there.
    let wb = read(&bytes);
    assert_eq!(wb.warnings, Vec::<String>::new());
    assert_eq!(wb.styles.len(), 200);
    assert!(text(&wb) > 200 * 1_000);
    // 64 KiB: the formats that fit are kept, the others are read without
    // one, with a warning; every value is still there.
    let limits = Limits {
        max_style_text_bytes: 64 * 1024,
        ..Limits::default()
    };
    let wb = read_xls(&bytes, &limits).unwrap();
    assert!(text(&wb) <= 64 * 1024);
    assert!(text(&wb) > 32 * 1024);
    assert_eq!(wb.sheets[0].cells.len(), 200);
    assert!(
        wb.warnings
            .iter()
            .any(|w| w.contains("number formats and font names")),
        "{:?}",
        wb.warnings
    );
    assert!(style(&wb, 0, 0).unwrap().number_format.is_some());
    assert!(style(&wb, 199, 0).is_none_or(|s| s.number_format.is_none()));
}

// ------------------------------------------------------------ fonts, formats

#[test]
fn font_4_is_the_bold_default_font_and_semibold_is_bold() {
    let fonts = [
        Font::default(),
        Font::default(),
        Font::default(),
        Font::default(),
        // Fonts 5, 6, 7: weights 500 (medium), 550 (semibold), 700.
        Font {
            weight: Some(500),
            italic: true,
            ..Font::default()
        },
        Font {
            weight: Some(550),
            ..Font::default()
        },
        Font {
            weight: Some(700),
            ..Font::default()
        },
    ];
    let xfs: Vec<Xf> = [4, 5, 6, 7]
        .iter()
        .map(|f| Xf {
            font: *f,
            ..Xf::default()
        })
        .collect();
    let g = biff::globals(&fonts, &[], &xfs);
    let mut s = Vec::new();
    for i in 0..4u16 {
        s.extend(biff::number(0, i, 16 + i, 1.0));
    }
    let wb = read(&book(&g, s));
    let bold: Vec<bool> = (0..4)
        .map(|c| style(&wb, 0, c).is_some_and(|s| s.font.bold))
        .collect();
    assert_eq!(bold, vec![true, false, true, true]);
    // Font 4 is font 0 otherwise.
    let f4 = &style(&wb, 0, 0).unwrap().font;
    assert_eq!((f4.family.as_deref(), f4.italic), (None, false));
}

#[test]
fn built_in_number_formats_follow_openoffice_past_the_ecma_table() {
    // (format id, the id it stands for).
    let pairs = [
        (32u16, 21u16),
        (35, 21),
        (60, 2),
        (62, 4),
        (67, 9),
        (81, 47),
    ];
    let xfs: Vec<Xf> = pairs
        .iter()
        .flat_map(|(a, b)| [*a, *b])
        .map(|fmt| Xf {
            fmt,
            ..Xf::default()
        })
        .collect();
    let g = biff::globals(&[], &[], &xfs);
    let mut s = Vec::new();
    for i in 0..xfs.len() as u16 {
        s.extend(biff::number(0, i, 16 + i, 0.25));
    }
    let wb = read(&book(&g, s));
    for (k, (a, b)) in pairs.iter().enumerate() {
        let left = style(&wb, 0, 2 * k as u32).unwrap();
        let right = style(&wb, 0, 2 * k as u32 + 1).unwrap();
        assert!(left.number_format.is_some(), "format {a}");
        assert_eq!(left.number_format, right.number_format, "{a} is {b}");
    }
    // 14 and 37–40 are the en-US forms of both tables.
    assert_eq!(
        tally_io_xlsx::builtin_format_code(37),
        Some("#,##0_);\\(#,##0\\)")
    );
}

// ------------------------------------------------------------ pictures, tabs

#[test]
fn a_blip_directly_in_the_store_takes_an_index() {
    let mut jpeg = b"\xff\xd8\xff\xe0".to_vec();
    jpeg.extend_from_slice(&[7u8; 20]);
    let store = biff::dgg(&[
        biff::bse(0xF01E, 0x6E0, &biff::fake_png(10)),
        biff::bare_blip(0xF01D, 0x46A, &jpeg),
        biff::bse(0xF01E, 0x6E0, &biff::fake_png(30)),
    ]);
    let mut g = biff::globals(&[], &[], &[]);
    g.extend(biff::msodrawinggroup(&store, usize::MAX));
    let shapes = [
        biff::shape(75, Some(3), [0, 0, 0, 0, 1, 0, 1, 0]),
        biff::shape(75, Some(2), [0, 0, 0, 0, 1, 0, 1, 0]),
    ];
    let mut s = biff::msodrawing(&biff::dg(&shapes));
    s.extend(biff::number(0, 0, 15, 1.0));
    let wb = read(&book(&g, s));
    let kinds: Vec<(&str, usize)> = wb.sheets[0]
        .images
        .iter()
        .map(|i| (i.mime.as_str(), i.bytes.len()))
        .collect();
    assert_eq!(kinds, vec![("image/png", 38), ("image/jpeg", 24)]);
    assert_eq!(wb.warnings, Vec::<String>::new());
}

#[test]
fn pictures_in_a_group_are_reported() {
    let store = biff::dgg(&[biff::bse(0xF01E, 0x6E0, &biff::fake_png(10))]);
    let mut g = biff::globals(&[], &[], &[]);
    g.extend(biff::msodrawinggroup(&store, usize::MAX));
    let shapes = [
        biff::group(
            [0, 0, 0, 0, 4, 0, 4, 0],
            &[biff::child_picture(1), biff::child_picture(1)],
        ),
        biff::shape(75, Some(1), [0, 0, 0, 0, 1, 0, 1, 0]),
    ];
    let mut s = biff::msodrawing(&biff::dg(&shapes));
    s.extend(biff::number(0, 0, 15, 1.0));
    let wb = read(&book(&g, s));
    // The picture on the sheet is there; the group and its two pictures
    // are counted as left out.
    assert_eq!(wb.sheets[0].images.len(), 1);
    assert_eq!(wb.warnings.len(), 1, "{:?}", wb.warnings);
    assert!(wb.warnings[0].starts_with("3 drawing object(s)"));
}

#[test]
fn the_tab_colour_is_a_palette_colour() {
    let tab = |icv: u32| {
        let mut s = biff::sheetext(icv);
        s.extend(biff::number(0, 0, 15, 1.0));
        read(&book(&biff::globals(&[], &[], &[]), s)).sheets[0].tab_color
    };
    assert_eq!(tab(10), Some(0xFF00_00FF));
    assert_eq!(tab(8), Some(0x0000_00FF));
    assert_eq!(tab(63), Some(0x3333_33FF));
    // The fixed colours 0–7, the system colours and "none" are no tab
    // colour.
    for icv in [0, 2, 7, 0x40, 0x41, 0x4D, 0x7F] {
        assert_eq!(tab(icv), None, "index {icv:#x}");
    }
}

// ------------------------------------------------------ formulas without value

#[test]
fn a_formula_without_a_value_is_kept() {
    let g = biff::globals(&[], &[], &[bordered()]);
    let mut s = biff::number(1, 1, 15, 1.0);
    // A string result whose STRING record is missing, outside the area of
    // the values, and an "empty string" result inside it.
    let mut f = biff::formula(5, 3, 16, 0.0, &biff::ptg_int(7));
    f[4 + 6..4 + 14].copy_from_slice(&[0, 0, 0, 0, 0, 0, 0xFF, 0xFF]);
    s.extend(f);
    let mut f = biff::formula(0, 0, 15, 0.0, &biff::ptg_int(8));
    f[4 + 6..4 + 14].copy_from_slice(&[3, 0, 0, 0, 0, 0, 0xFF, 0xFF]);
    s.extend(f);
    s.extend(biff::number(6, 4, 15, 2.0));
    let wb = read(&book(&g, s));
    let sheet = &wb.sheets[0];
    let at: Vec<(u32, u32)> = sheet.cells.iter().map(|c| (c.row, c.col)).collect();
    assert_eq!(at, vec![(0, 0), (1, 1), (5, 3), (6, 4)]);
    // The first has no cached value, the second an empty text.
    for (r, c, text, value) in [
        (5, 3, "of:=7", None),
        (0, 0, "of:=8", Some(Value::text(""))),
    ] {
        match &cell(sheet, r, c).unwrap().content {
            CellContentData::Formula { text: t, cached } => {
                assert_eq!(&**t, text);
                assert_eq!(*cached, value);
            }
            other => panic!("({r}, {c}): {other:?}"),
        }
    }
    assert!(has_border(style(&wb, 5, 3)));
    assert_eq!(wb.warnings, Vec::<String>::new());
}
