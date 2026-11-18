//! Defined names (`NAME`) and cell hyperlinks (`HLINK`) of a BIFF8
//! workbook, on synthetic fixtures.
#![allow(clippy::unwrap_used)]

mod biff;

use tally_io_xls::{read_xls, Limits};
use tally_model::{CellContentData, WorkbookData};

fn read(bytes: &[u8]) -> WorkbookData {
    let wb = read_xls(bytes, &Limits::default()).unwrap();
    assert!(
        !wb.warnings.iter().any(|w| w.contains("could not be read")),
        "{:?}",
        wb.warnings
    );
    wb
}

/// (name, sheet, formula) of every name.
fn names(wb: &WorkbookData) -> Vec<(&str, Option<u32>, &str)> {
    wb.names
        .iter()
        .map(|n| (n.name.as_str(), n.sheet, n.formula.as_str()))
        .collect()
}

/// (row, col, target) of every link of a sheet.
fn links(wb: &WorkbookData, sheet: usize) -> Vec<(u32, u32, &str)> {
    wb.sheets[sheet]
        .links
        .iter()
        .map(|l| (l.row, l.col, &*l.url))
        .collect()
}

/// A workbook of two sheets with `globals` after the usual ones and an
/// `EXTERNSHEET` for both sheets, and `sheet` records on the first.
fn book(globals: &[u8], sheet: &[u8]) -> Vec<u8> {
    let mut g = biff::globals(&[], &[], &[]);
    g.extend(biff::externsheet(2));
    g.extend_from_slice(globals);
    let mut s = biff::number(0, 0, 15, 1.0);
    s.extend_from_slice(sheet);
    biff::file(biff::stream(
        &g,
        &[("Main", s), ("Other sheet", biff::number(0, 0, 15, 2.0))],
    ))
}

#[test]
fn defined_names_are_read() {
    let wb = read(&biff::books::names());
    assert_eq!(
        names(&wb),
        [
            ("Data", None, "of:=[$Main.$A$1:.$B$1]"),
            ("Rate", None, "of:=0.25"),
            ("Local", Some(1), "of:=[$'Other sheet'.$C$1:.$C$1048576]"),
            ("Double", None, "of:=Rate*2"),
            (
                "Both",
                None,
                "of:=([$Main.$A$1:.$A$1]~[$'Other sheet'.$B$2:.$B$2])"
            ),
        ]
    );
    // The name with a function is reported; the function name and the
    // print area are not names (the print area is the sheet's).
    let left_out: Vec<&String> = wb
        .warnings
        .iter()
        .filter(|w| w.contains("defined name"))
        .collect();
    assert_eq!(left_out.len(), 1, "{:?}", wb.warnings);
    assert!(left_out[0].starts_with("1 defined name(s)"));
    let page = wb.sheets[0].page_setup.as_ref().unwrap();
    assert_eq!(
        page.print_range.map(|r| (r.r0, r.c0, r.r1, r.c1)),
        Some((0, 0, 9, 3))
    );
    // Formulas that use the names keep them.
    let formula = |r: u32, c: u32| {
        let cell = wb.sheets[0]
            .cells
            .iter()
            .find(|x| (x.row, x.col) == (r, c))
            .unwrap();
        match &cell.content {
            CellContentData::Formula { text, .. } => text.to_string(),
            other => panic!("{other:?}"),
        }
    };
    assert_eq!(formula(4, 0), "of:=SUM(Data)");
    assert_eq!(formula(4, 1), "of:=Double+1");
}

#[test]
fn hyperlinks_are_read() {
    let wb = read(&biff::books::names());
    assert_eq!(
        links(&wb, 0),
        [
            (1, 0, "https://example.org/"),
            (1, 1, "https://example.org/"),
            (2, 0, "#'Other sheet'.B2"),
            (3, 0, "../folder/book.xls"),
            (5, 0, "https://example.org/doc#part"),
        ]
    );
    assert!(links(&wb, 1).is_empty());
    // A link over a range shares one target.
    let l = &wb.sheets[0].links;
    assert!(std::sync::Arc::ptr_eq(&l[0].url, &l[1].url));
    // The linked cells keep their text; a link on an empty cell is a link
    // without a cell.
    assert!(wb.sheets[0].cells.iter().all(|c| (c.row, c.col) != (5, 0)));
}

#[test]
fn name_scopes_and_references() {
    // A relative reference (stored as the cell it means from A1), a name
    // on the second sheet's cells, the same name local to each sheet.
    let mut g = biff::defined_name("Rel", 0, 0, &[0x3A, 0, 0, 2, 0, 1, 0xC0]);
    g.extend(biff::defined_name(
        "Same",
        0,
        1,
        &biff::ptg_area3d_on(0, 0, 1, 0, 0),
    ));
    g.extend(biff::defined_name(
        "Same",
        0,
        2,
        &biff::ptg_area3d_on(1, 0, 1, 0, 0),
    ));
    // Whole rows 3–4.
    g.extend(biff::defined_name(
        "Rows",
        0,
        0,
        &biff::ptg_area3d_on(0, 2, 3, 0, 255),
    ));
    // A hidden name is a name (as in OpenOffice).
    g.extend(biff::defined_name("Hidden", 0x0001, 0, &biff::ptg_num(1.5)));
    let wb = read(&book(&g, &[]));
    assert_eq!(
        names(&wb),
        [
            ("Rel", None, "of:=[$Main.B3]"),
            ("Same", Some(0), "of:=[$Main.$A$1:.$A$2]"),
            ("Same", Some(1), "of:=[$'Other sheet'.$A$1:.$A$2]"),
            ("Rows", None, "of:=[$Main.$A$3:.$XFD$4]"),
            ("Hidden", None, "of:=1.5"),
        ]
    );
    assert!(!wb.warnings.iter().any(|w| w.contains("defined name")));
}

#[test]
fn names_that_are_left_out_are_counted() {
    // A reference to another workbook's sheet (no such EXTERNSHEET entry),
    // a name local to a sheet that is not there, an error value. A name
    // without a formula is not counted: the pre-scan empties a damaged
    // name's formula and reports it itself.
    let mut g = biff::defined_name("Far", 0, 0, &biff::ptg_area3d_on(7, 0, 0, 0, 0));
    g.extend(biff::defined_name("Nowhere", 0, 9, &biff::ptg_num(1.0)));
    g.extend(biff::defined_name("Broken", 0, 0, &[0x1C, 0x17]));
    g.extend(biff::defined_name("Empty", 0, 0, &[]));
    g.extend(biff::defined_name("Kept", 0, 0, &biff::ptg_num(2.0)));
    let wb = read(&book(&g, &[]));
    assert_eq!(names(&wb), [("Kept", None, "of:=2")]);
    assert!(wb
        .warnings
        .iter()
        .any(|w| w.starts_with("3 defined name(s)")));
}

#[test]
fn link_kinds() {
    let mut s = biff::hlink_url((2, 2, 0, 0), "mailto:someone@example.org", None);
    // A relative URL moniker (no "absolute" flag) is a path.
    let mut rel = biff::hlink_head((3, 3, 0, 0), 0x01);
    rel.extend_from_slice(&biff::URL_MONIKER);
    let units: Vec<u8> = "docs\\a.html\0"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    rel.extend_from_slice(&(units.len() as u32).to_le_bytes());
    rel.extend(units);
    s.extend(biff::rec(0x01B8, &rel));
    // A file with a drive letter, a defined name as the place.
    s.extend(biff::hlink_file((4, 4, 0, 0), 0, "C:\\dir\\book.xls"));
    s.extend(biff::hlink_place((5, 5, 0, 0), "Data"));
    // The first record of a cell wins; the high byte of a column is
    // ignored.
    s.extend(biff::hlink_url(
        (2, 2, 0, 0),
        "https://example.org/later",
        None,
    ));
    s.extend(biff::hlink_url(
        (6, 6, 0x0102, 0x0102),
        "https://example.org/col",
        None,
    ));
    let wb = read(&book(&[], &s));
    assert_eq!(
        links(&wb, 0),
        [
            (2, 0, "mailto:someone@example.org"),
            (3, 0, "docs/a.html"),
            (4, 0, "file:///C:/dir/book.xls"),
            (5, 0, "#Data"),
            (6, 2, "https://example.org/col"),
        ]
    );
}

#[test]
fn a_link_split_over_continue_records() {
    let d = biff::hlink_url_data((1, 1, 1, 1), "https://example.org/continued", None);
    let (a, b) = d.split_at(40);
    let mut s = biff::rec(0x01B8, a);
    s.extend(biff::rec(0x003C, b));
    let wb = read(&book(&[], &s));
    assert_eq!(links(&wb, 0), [(1, 1, "https://example.org/continued")]);
}
