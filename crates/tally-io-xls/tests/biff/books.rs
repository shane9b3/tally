//! The synthetic workbooks of the formatting tests. They are also written
//! to `tests/fixtures/xls/` (the fuzz seeds are generated from that
//! directory); `tests/formatting.rs` checks that the committed files are
//! what this code builds.

use super::*;

/// Fonts, number formats, a custom palette, cell formats of every kind,
/// column and row sizes, view settings; two sheets.
pub fn formatted() -> Vec<u8> {
    let fonts = [
        Font::default(),
        Font::default(),
        Font::default(),
        Font::default(),
        // Font 5 (BIFF skips index 4).
        Font {
            name: "Times New Roman",
            twips: 220,
            bold: true,
            italic: true,
            color: 10,
            ..Font::default()
        },
        // Font 6.
        Font {
            name: "Courier New",
            twips: 160,
            strike: true,
            underline: 2,
            color: 8,
            ..Font::default()
        },
    ];
    let formats = [(164, "0.000"), (165, "yyyy-mm-dd"), (166, "h:mm")];
    let centered = Xf {
        font: 5,
        fmt: 164,
        halign: 2,
        valign: 1,
        wrap: true,
        ..Xf::default()
    };
    let xfs = [
        // 16
        centered.clone(),
        // 17: a date on a solid red fill, unlocked.
        Xf {
            font: 6,
            fmt: 165,
            pattern: 1,
            fore: 10,
            locked: false,
            ..Xf::default()
        },
        // 18: a time, right-aligned, indented, rotated 45° up.
        Xf {
            fmt: 166,
            halign: 3,
            indent: 2,
            rotation: 45,
            ..Xf::default()
        },
        // 19: borders (left thin blue, right double, top dashed, bottom
        // thick in palette colour 9, a hair diagonal going down).
        Xf {
            lines: [1, 6, 3, 5, 7],
            line_colors: [12, 0x40, 0x40, 9, 0x40],
            diag_down: true,
            ..Xf::default()
        },
        // 20: a 50 % pattern of palette colours 8 and 9.
        Xf {
            pattern: 2,
            fore: 8,
            back: 9,
            ..Xf::default()
        },
        // 21: the same as 16.
        centered,
        // 22: a built-in format, shrink to fit, hidden formula, rotated
        // 45° down.
        Xf {
            fmt: 4,
            shrink: true,
            hidden: true,
            rotation: 135,
            ..Xf::default()
        },
        // 23: nothing but the default font, like XF 15.
        Xf::default(),
        // 24: stacked text, justified, top.
        Xf {
            rotation: 255,
            halign: 5,
            valign: 0,
            ..Xf::default()
        },
    ];
    let mut g = window1(1);
    g.extend(globals(&fonts, &formats, &xfs));
    g.extend(palette(&[0x11_2233, 0x44_5566]));
    g.extend(sst(&["shared"]));

    let mut one = defcolwidth(8);
    one.extend(colinfo(0, 0, 2560, 15, 0));
    one.extend(colinfo(2, 3, 1024, 15, 1));
    one.extend(colinfo(4, 4, 0, 15, 0));
    one.extend(colinfo(6, 6, 512, 19, 0));
    one.extend(colinfo(9, 8, 4096, 15, 0));
    one.extend(colinfo(10, 256, 1280, 15, 0));
    one.extend(defaultrowheight(0, 255));
    one.extend(row(1, 600, 0x140));
    one.extend(row(2, 300, 0x100));
    one.extend(row(3, 255, 0x120));
    one.extend(row(5, 400, 0x1C0 | 19 << 16));
    one.extend(labelsst(0, 0, 16, 0));
    one.extend(label(0, 1, 17, "x"));
    one.extend(number(1, 0, 17, 45000.0));
    one.extend(number(1, 1, 17, 30.0));
    one.extend(number(1, 2, 18, 0.5));
    one.extend(rk(2, 0, 19, 7));
    one.extend(mulrk(2, 1, &[(20, 1), (21, 2)]));
    one.extend(blank(3, 0, 19));
    one.extend(mulblank(3, 1, &[20, 23, 15]));
    let mut rgce = ptg_ref(2, 0);
    rgce.extend(ptg_int(2));
    rgce.push(0x05);
    one.extend(formula(4, 0, 22, 14.0, &rgce));
    one.extend(boolerr(4, 1, 16, 1, false));
    one.extend(rstring(4, 2, 18, "r"));
    one.extend(number(4, 3, 24, 1.0));
    // Formulas shown, no grid, no headers, no zero values, frozen.
    one.extend(window2(0x0109));
    one.extend(pane(2, 1));
    one.extend(scl(3, 2));
    one.extend(sheetext(10));

    // Grid, headers and zeros shown; a split window.
    let mut two = number(0, 0, 15, 1.0);
    two.extend(window2(0x00B6));
    two.extend(pane(1440, 720));

    file(stream(&g, &[("One", one), ("Two", two)]))
}

/// A picture store split over `CONTINUE` records (PNG, JPEG, EMF and a
/// PNG stored in a JPEG record) and a sheet whose drawing, split over two
/// `MSODRAWING` records, places them next to a text box.
pub fn pictures() -> Vec<u8> {
    let mut jpeg = b"\xff\xd8\xff\xe0".to_vec();
    jpeg.extend_from_slice(&[7u8; 20]);
    let store = dgg(&[
        bse(0xF01E, 0x6E0, &fake_png(40)),
        bse(0xF01D, 0x46A, &jpeg),
        bse(0xF01A, 0x3D4, &[1u8; 30]),
        bse(0xF01D, 0x46A, &fake_png(12)),
    ]);
    let mut g = globals(&[], &[], &[]);
    g.extend(msodrawinggroup(&store, 60));

    let first = shape(75, Some(1), [1, 512, 2, 128, 3, 0, 4, 0]);
    let rest = [
        shape(75, Some(2), [0, 0, 0, 0, 1, 0, 1, 0]),
        shape(75, Some(3), [0, 0, 0, 0, 1, 0, 1, 0]),
        shape(202, None, [0, 0, 0, 0, 1, 0, 1, 0]),
        shape(75, Some(4), [5, 0, 5, 0, 5, 1023, 5, 255]),
        shape(75, Some(9), [0, 0, 0, 0, 1, 0, 1, 0]),
    ];
    let mut shapes = vec![first.clone()];
    shapes.extend(rest);
    let drawing = dg(&shapes);
    // As Excel writes it: one MSODRAWING per shape, an OBJ after each.
    let cut = drawing.len() - shapes[1..].concat().len();
    let mut sheet = colinfo(1, 1, 2560, 15, 0);
    sheet.extend(row(2, 400, 0x140));
    sheet.extend(number(0, 0, 15, 1.0));
    sheet.extend(msodrawing(&drawing[..cut]));
    sheet.extend(obj());
    sheet.extend(msodrawing(&drawing[cut..]));
    sheet.extend(obj());
    file(stream(&g, &[("Pictures", sheet)]))
}

/// Two cell notes (one always shown, one with two lines) on a sheet that
/// starts at B2.
pub fn notes() -> Vec<u8> {
    let mut sheet = number(1, 1, 15, 1.0);
    sheet.extend(note(1, 1, 1, true, "An Author", "first note"));
    sheet.extend(note(3, 2, 2, false, "", "two\nlines"));
    file(stream(&globals(&[], &[], &[]), &[("Notes", sheet)]))
}

/// The fixtures by file name.
pub fn all() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("formatted.xls", formatted()),
        ("pictures.xls", pictures()),
        ("notes.xls", notes()),
    ]
}
