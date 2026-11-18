//! The synthetic workbooks of the formatting and page setup tests. They are also written
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

/// Page setup of every kind on two sheets (the first with a header, page
/// breaks, a print area and print titles, the second fitted to one page
/// wide), a row with a default format and a formatted column with cells
/// of the default format in it.
pub fn page() -> Vec<u8> {
    let bordered = Xf {
        lines: [1, 1, 1, 1, 0],
        ..Xf::default()
    };
    let mut g = globals(&[], &[], &[bordered]);
    g.extend(externsheet(2));
    g.extend(builtin_name(0x06, 1, &ptg_area3d(0, 29, 0, 5)));
    g.extend(builtin_name(
        0x07,
        1,
        &ptg_union(&ptg_area3d(0, 65_535, 0, 0), &ptg_area3d(0, 1, 0, 255)),
    ));

    let mut one = word(0x002A, 1);
    one.extend(word(0x002B, 1));
    one.extend(defaultrowheight(0, 255));
    one.extend(word(0x0081, 0x04C1));
    one.extend(page_breaks(true, &[20, 5]));
    one.extend(page_breaks(false, &[3]));
    one.extend(header_footer(0x0014, "&LLeft&CPage &P of &N&R&D"));
    one.extend(header_footer(0x0015, ""));
    one.extend(word(0x0083, 1));
    one.extend(word(0x0084, 0));
    one.extend(margin(0x0026, 0.166));
    one.extend(margin(0x0027, 0.25));
    one.extend(margin(0x0028, 1.0));
    one.extend(margin(0x0029, 0.75));
    one.extend(setup(&Setup {
        paper: 1,
        scale: 80,
        start: 3,
        flags: 0x0081,
        header: 0.3,
        footer: 0.4,
        ..Setup::default()
    }));
    one.extend(defcolwidth(8));
    // Column B is bordered; row 2 has the bordered format as its default,
    // row 4 is hidden by a height of 0.
    one.extend(colinfo(1, 1, 2560, 16, 0));
    one.extend(row(0, 255, 0x0100));
    one.extend(row(1, 300, 0x0180 | 16 << 16));
    one.extend(row(3, 0, 0x0100));
    one.extend(number(0, 0, 15, 1.0));
    one.extend(number(0, 1, 15, 2.0));
    one.extend(number(0, 3, 15, 3.0));
    one.extend(number(1, 2, 15, 4.0));
    one.extend(blank(2, 1, 15));
    one.extend(window2(0x00B6));

    let mut two = word(0x0081, 0x05C1);
    two.extend(header_footer(0x0014, ""));
    two.extend(header_footer(0x0015, "&C&A"));
    two.extend(setup(&Setup {
        fit_w: 1,
        fit_h: 0,
        ..Setup::default()
    }));
    two.extend(number(0, 0, 15, 1.0));
    two.extend(window2(0x00B6));

    file(stream(&g, &[("Page", one), ("Fit", two)]))
}

/// Defined names (of the workbook and of a sheet, a constant, a name
/// built on another name, a union, a formula that is not read, a function
/// name, a print area among them), a formula that uses a name, and
/// hyperlinks (a URL over two cells, a URL with a location, a place in
/// the workbook, a file).
pub fn names() -> Vec<u8> {
    let mut g = globals(&[], &[], &[]);
    g.extend(externsheet(2));
    // 1: Data = Main!$A$1:$B$1.
    g.extend(defined_name("Data", 0, 0, &ptg_area3d_on(0, 0, 0, 0, 1)));
    // 2: Rate = 0.25.
    g.extend(defined_name("Rate", 0, 0, &ptg_num(0.25)));
    // 3: the print area of the first sheet.
    g.extend(builtin_name(0x06, 1, &ptg_area3d(0, 9, 0, 3)));
    // 4: Local = 'Other sheet'!$C:$C, local to the second sheet.
    g.extend(defined_name(
        "Local",
        0,
        2,
        &ptg_area3d_on(1, 0, 65_535, 2, 2),
    ));
    // 5: Double = Rate * 2.
    let mut double = ptg_name(2);
    double.extend(ptg_int(2));
    double.push(0x05);
    g.extend(defined_name("Double", 0, 0, &double));
    // 6: Both = two areas, a union.
    g.extend(defined_name(
        "Both",
        0,
        0,
        &ptg_union(&ptg_area3d_on(0, 0, 0, 0, 0), &ptg_area3d_on(1, 1, 1, 1, 1)),
    ));
    // 7: a formula with a function (NOW()): not read.
    g.extend(defined_name("Stamp", 0, 0, &[0x21, 74, 0]));
    // 8: a function name: not a name of the model.
    g.extend(defined_name("Macro", 0x000E, 0, &[0x1E, 1, 0]));

    let mut one = number(0, 0, 15, 1.0);
    one.extend(number(0, 1, 15, 2.0));
    one.extend(label(1, 0, 15, "site"));
    one.extend(label(2, 0, 15, "place"));
    one.extend(label(3, 0, 15, "file"));
    // A5: =SUM(Data), B5: =Double+1.
    let mut sum = vec![0x43, 1, 0, 0, 0];
    sum.extend_from_slice(&[0x42, 1, 4, 0]);
    one.extend(formula(4, 0, 15, 3.0, &sum));
    let mut plus = vec![0x43, 5, 0, 0, 0];
    plus.extend(ptg_int(1));
    plus.push(0x03);
    one.extend(formula(4, 1, 15, 1.5, &plus));
    one.extend(window2(0x00B6));
    one.extend(hlink_url((1, 1, 0, 1), "https://example.org/", None));
    one.extend(hlink_url(
        (5, 5, 0, 0),
        "https://example.org/doc",
        Some("part"),
    ));
    one.extend(hlink_place((2, 2, 0, 0), "'Other sheet'!B2"));
    one.extend(hlink_file((3, 3, 0, 0), 1, "folder\\book.xls"));

    let mut two = number(0, 2, 15, 5.0);
    two.extend(window2(0x00B6));

    file(stream(&g, &[("Main", one), ("Other sheet", two)]))
}

/// Names Excel 97-2003 allows because its sheet ends at IV65536 and that
/// are cells in a larger grid (`TAX2024`, `FYE2025`, `A70000`), used by a
/// name and by cell formulas; and a relative name ("the cell above") used
/// from two cells.
pub fn wide_names() -> Vec<u8> {
    let mut g = globals(&[], &[], &[]);
    g.extend(externsheet(1));
    // 1: TAX2024 = 0.2; 2: FYE2025 = Main!$A$1; 3: Gross = FYE2025 *
    // TAX2024; 4: A70000 = 3; 5: Above = one row up, same column.
    g.extend(defined_name("TAX2024", 0, 0, &ptg_num(0.2)));
    g.extend(defined_name("FYE2025", 0, 0, &[0x3A, 0, 0, 0, 0, 0, 0]));
    let mut gross = ptg_name(2);
    gross.extend(ptg_name(1));
    gross.push(0x05);
    g.extend(defined_name("Gross", 0, 0, &gross));
    g.extend(defined_name("A70000", 0, 0, &ptg_int(3)));
    g.extend(defined_name(
        "Above",
        0,
        0,
        &[0x3A, 0, 0, 0xFF, 0xFF, 0x00, 0xC0],
    ));

    // A name in a cell formula: PtgName (value class) with its index.
    let name = |i: u8| vec![0x43, i, 0, 0, 0];
    let mut s = number(0, 0, 15, 5.0);
    s.extend(number(0, 1, 15, 10.0));
    // B2 = Above (B1).
    s.extend(formula(1, 1, 15, 0.0, &name(5)));
    s.extend(number(1, 2, 15, 20.0));
    // A3 = TAX2024 + 1.
    let mut f = name(1);
    f.extend(ptg_int(1));
    f.push(0x03);
    s.extend(formula(2, 0, 15, 0.0, &f));
    // B3 = FYE2025 * 2.
    let mut f = name(2);
    f.extend(ptg_int(2));
    f.push(0x05);
    s.extend(formula(2, 1, 15, 0.0, &f));
    // C3 = Above (C2).
    s.extend(formula(2, 2, 15, 0.0, &name(5)));
    // D3 = Gross.
    s.extend(formula(2, 3, 15, 0.0, &name(3)));
    // E3 = A70000 + FYE2025.
    let mut f = name(4);
    f.extend(name(2));
    f.push(0x03);
    s.extend(formula(2, 4, 15, 0.0, &f));
    // A4 = IV65535 + A1: a cell at the edge of the sheet is a cell.
    let mut f = vec![0x44, 0xFE, 0xFF, 0xFF, 0xC0];
    f.extend_from_slice(&[0x44, 0, 0, 0, 0xC0]);
    f.push(0x03);
    s.extend(formula(3, 0, 15, 0.0, &f));
    s.extend(window2(0x00B6));

    file(stream(&g, &[("Main", s)]))
}

/// The fixtures by file name.
pub fn all() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("widenames.xls", wide_names()),
        ("names.xls", names()),
        ("formatted.xls", formatted()),
        ("pictures.xls", pictures()),
        ("notes.xls", notes()),
        ("page.xls", page()),
    ]
}
