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
            ("Stamp", None, "of:=NOW()"),
        ]
    );
    // The function name and the print area are not names (the print area
    // is the sheet's), and nothing is reported about them.
    assert!(
        !wb.warnings.iter().any(|w| w.contains("defined name")),
        "{:?}",
        wb.warnings
    );
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

#[test]
fn relative_names_are_offsets_from_the_cell_that_uses_them() {
    // "Above": one row up in the same column (row word 0xFFFF = -1, both
    // parts relative); "Left3": three columns to the left in the absolute
    // row 2; "Next": one row down.
    let mut g = biff::defined_name("Above", 0, 0, &[0x3A, 0, 0, 0xFF, 0xFF, 0x00, 0xC0]);
    g.extend(biff::defined_name(
        "Left3",
        0,
        2,
        &[0x3A, 1, 0, 1, 0, 0xFD, 0x40],
    ));
    g.extend(biff::defined_name(
        "Next",
        0,
        0,
        &[0x3A, 0, 0, 1, 0, 0x00, 0xC0],
    ));
    let wb = read(&book(&g, &[]));
    let got: Vec<_> = wb
        .names
        .iter()
        .map(|n| (n.name.as_str(), n.formula.as_str(), n.base_cell))
        .collect();
    assert_eq!(
        got,
        [
            // Written for a formula in DY32769 (row 32768, column 128):
            // the cell above it. Never the absolute row 65,536.
            ("Above", "of:=[$Main.DY32768]", Some((0, 32_768, 128))),
            ("Left3", "of:=[$'Other sheet'.DV$2]", Some((1, 32_768, 128))),
            // Down and right of A1: the base stays A1.
            ("Next", "of:=[$Main.A2]", None),
        ]
    );
    assert!(!wb.warnings.iter().any(|w| w.contains("defined name")));
}

#[test]
fn names_that_are_cells_or_keywords_are_left_out_with_their_users() {
    // "A1" = 6 and "Twice" = A1 * 2: written as text, Twice would double
    // the cell A1. "TRUE", an R1C1 reference and "FY2025", a cell of the
    // Excel 97-2003 sheet itself (column FY, row 2025): none of them a
    // name Excel allows.
    let mut twice = biff::ptg_name(1);
    twice.extend(biff::ptg_int(2));
    twice.push(0x05);
    let mut g = biff::defined_name("A1", 0, 0, &biff::ptg_num(6.0));
    g.extend(biff::defined_name("Twice", 0, 0, &twice));
    g.extend(biff::defined_name("TRUE", 0, 0, &biff::ptg_num(1.0)));
    g.extend(biff::defined_name("R1C1", 0, 0, &biff::ptg_num(1.0)));
    g.extend(biff::defined_name("FY2025", 0, 0, &biff::ptg_num(1.0)));
    g.extend(biff::defined_name("Kept", 0, 0, &biff::ptg_num(2.0)));
    let wb = read(&book(&g, &[]));
    assert_eq!(names(&wb), [("Kept", None, "of:=2")]);
    assert!(
        wb.warnings
            .iter()
            .any(|w| w.starts_with("5 defined name(s)")),
        "{:?}",
        wb.warnings
    );
}

#[test]
fn names_that_are_cells_only_in_the_larger_grid_are_kept() {
    // Legal names in Excel 97-2003, whose sheet ends at IV65536: a column
    // past IV, a row past 65,536. They are names of the file, and the
    // formulas that use them (other names, cells) name them.
    let wb = read(&biff::books::wide_names());
    assert_eq!(
        names(&wb),
        [
            ("TAX2024", None, "of:=0.2"),
            ("FYE2025", None, "of:=[$Main.$A$1]"),
            ("Gross", None, "of:=FYE2025*TAX2024"),
            ("A70000", None, "of:=3"),
            ("Above", None, "of:=[$Main.DY32768]"),
        ]
    );
    assert!(!wb.warnings.iter().any(|w| w.contains("defined name")));
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
    // Names, not the cells TAX2024, FYE2025 and A70000.
    assert_eq!(formula(2, 0), "of:=TAX2024+1");
    assert_eq!(formula(2, 1), "of:=FYE2025*2");
    assert_eq!(formula(2, 3), "of:=Gross");
    assert_eq!(formula(2, 4), "of:=A70000+FYE2025");
    // A reference inside the Excel 97-2003 sheet stays a reference, also
    // in its last column and next to its last row.
    assert_eq!(formula(3, 0), "of:=[.IV65535]+[.A1]");
}

#[test]
fn names_of_one_spelling_in_two_scopes_and_token_trees() {
    let wb = read(&biff::books::scoped_names());
    assert_eq!(
        names(&wb),
        [
            ("Foo", None, "of:=5"),
            ("Loc", Some(1), "of:=7"),
            ("Loc", None, "of:=100"),
            // `Bar` (the workbook's) and `Baz` are on the Loc of the
            // second sheet: read from that sheet (see below).
            ("Bar", None, "of:=Loc+1"),
            ("Baz", Some(1), "of:=Loc+1"),
            // The text has the parentheses the token tree means.
            ("Pa", None, "of:=(1+2)*3"),
            ("Pb", None, "of:=(2^3)^2"),
            ("Pc", None, "of:=1-(2-3)"),
            ("Pd", None, "of:=-(2^2)"),
            ("Pe", None, "of:=(1+2)%"),
            ("Pf", None, "of:=2^(3^2)"),
            ("Pg", None, "of:=(-2)^2"),
            // The sheet's Foo is not read (PHONETIC): it stays a name of
            // that sheet without a value, so the sheet's cells do not
            // take the workbook's Foo.
            ("Foo", Some(0), "of:=#NAME?"),
        ]
    );
    // Their base cell is on the sheet whose Loc they mean.
    let base = |name: &str| {
        let n = wb.names.iter().find(|n| n.name == name).unwrap();
        n.base_cell
    };
    assert_eq!(base("Bar"), Some((1, 0, 0)));
    assert_eq!(base("Baz"), Some((1, 0, 0)));
    assert_eq!(base("Pa"), None);
    // Foo of the sheet and Mix (on both Locs) are counted.
    assert!(
        wb.warnings
            .iter()
            .any(|w| w.starts_with("2 defined name(s)")),
        "{:?}",
        wb.warnings
    );
}

#[test]
fn by_name_calls_of_functions_excel_does_not_call_by_name() {
    // Add-in records spelled like a built-in function, like a function
    // only Tally has, or in another case; and hidden function names with
    // a prefix on a built-in function, with two prefixes, on nothing.
    let addins = [
        "SUM",
        "IF",
        "TRUE",
        "PI",
        "N",
        "AND",
        "EASTERSUNDAY",
        "MULTIPLE.OPERATIONS",
        "CONVERT_ADD",
        "ERRORTYPE",
        "quotient",
        "QUOTIENT",
    ];
    let futures = [
        "_xlfn.SUM",
        "_xlfn._xlfn.SUM",
        "_xlfn.TRUE",
        "_xlfn.EASTERSUNDAY",
        "_xlfn.NOSUCH",
        "_XLFN.IFERROR",
        "_xlfn.IFERROR",
    ];
    let mut g = Vec::new();
    for f in futures {
        g.extend(biff::future_function_name(f));
    }
    let call = |name: Vec<u8>| {
        let mut rgce = name;
        rgce.extend(biff::ptg_int(7));
        rgce.extend(biff::ptg_int(2));
        rgce.extend(biff::ptg_funcvar(3, 255));
        rgce
    };
    for i in 0..addins.len() {
        let rgce = call(biff::ptg_namex(2, i as u32 + 1));
        g.extend(biff::defined_name(&format!("AddIn_{i}"), 0, 0, &rgce));
    }
    for i in 0..futures.len() {
        let rgce = call(biff::ptg_name(i as u32 + 1));
        g.extend(biff::defined_name(&format!("Future_{i}"), 0, 0, &rgce));
    }
    let (wb, left_out) = function_book(&addins, &g);
    // Only the last of each list is a function Excel calls by name.
    assert_eq!(
        names(&wb),
        [
            ("AddIn_11", None, "of:=QUOTIENT(7;2)"),
            ("Future_6", None, "of:=COM.MICROSOFT.IFERROR(7;2)"),
        ]
    );
    assert_eq!(left_out, addins.len() + futures.len() - 2);
}

#[test]
fn renamed_and_added_table_rows() {
    // USDOLLAR is OpenOffice's (and an .xlsx's) DOLLAR; ERROR.TYPE is not
    // Tally's ERRORTYPE (other numbers), in an .xlsx neither: left out.
    let mut g = biff::defined_name(
        "Usd",
        0,
        0,
        &[biff::ptg_int(2), biff::ptg_funcvar(1, 204)].concat(),
    );
    g.extend(biff::defined_name(
        "Kind",
        0,
        0,
        &[biff::ptg_func(10), biff::ptg_func(261)].concat(),
    ));
    let (wb, left_out) = function_book(&[], &g);
    assert_eq!(names(&wb), [("Usd", None, "of:=DOLLAR(2)")]);
    assert_eq!(left_out, 1);
}

#[test]
fn the_text_of_the_names_is_bounded() {
    // 4,100 names that each spell a 255-letter name 30 times: 31 MB of
    // formula text from a file of 765 KB. The names and formulas kept are
    // bounded like the records (16 MiB); the rest is left out with a
    // warning.
    let long = "L".repeat(255);
    let mut g = biff::defined_name(&long, 0, 0, &biff::ptg_int(1));
    let mut rgce = Vec::new();
    for _ in 0..30 {
        rgce.extend(biff::ptg_name(1));
    }
    rgce.extend(biff::ptg_funcvar(30, 336));
    for i in 0..4_100 {
        g.extend(biff::defined_name(&format!("Item_{i}"), 0, 0, &rgce));
    }
    let wb = read(&book(&g, &[]));
    let total: usize = wb
        .names
        .iter()
        .map(|n| n.name.len() + n.formula.len())
        .sum();
    assert!(total <= 16 * 1024 * 1024, "{total}");
    assert!(wb.names.len() > 2_000, "{}", wb.names.len());
    assert!(
        wb.warnings
            .iter()
            .any(|w| w.contains("more than 16 MiB of text")),
        "{:?}",
        wb.warnings
    );
}

/// The workbook with `globals` (after an `EXTERNSHEET` whose entry 2 is
/// the add-in functions `functions`), and how many names its warning says
/// were left out.
fn function_book(functions: &[&str], globals: &[u8]) -> (WorkbookData, usize) {
    let mut g = biff::globals(&[], &[], &[]);
    g.extend(biff::externsheet_with_addin(2, functions));
    g.extend_from_slice(globals);
    let wb = read(&biff::file(biff::stream(
        &g,
        &[
            ("Main", biff::number(0, 0, 15, 1.0)),
            ("Other sheet", biff::number(0, 0, 15, 2.0)),
        ],
    )));
    let left_out = wb
        .warnings
        .iter()
        .find(|w| w.contains("defined name(s) with a formula"))
        .map_or(0, |w| w.split(' ').next().unwrap().parse().unwrap());
    (wb, left_out)
}

#[test]
fn names_with_functions_are_read() {
    let wb = read(&biff::books::function_names());
    assert_eq!(
        names(&wb),
        [
            ("Data", None, "of:=[$Main.$A$1:.$B$1]"),
            // SUM with one argument (an attribute) and with two.
            ("Total", None, "of:=SUM([$Main.$A$1:.$B$1])"),
            ("Plus", None, "of:=SUM(Data;10)"),
            // IF with its jumps; fixed argument counts, nested.
            ("Size", None, "of:=IF(Total>2;\"big\";\"small\")"),
            ("Rounded", None, "of:=ABS(ROUND(-2.567;1))"),
            // An argument that is left out.
            ("Sparse", None, "of:=IF([$Main.$A$1]>5;;7)"),
            // A name of the second sheet.
            ("Top", Some(1), "of:=MAX([$'Other sheet'.$C$1:.$C$3])"),
            // A function newer than the format and an add-in function,
            // both called by name.
            ("Safe", None, "of:=COM.MICROSOFT.IFERROR(1/0;-1)"),
            ("Half", None, "of:=QUOTIENT(7;2)"),
            ("Table", None, "of:={1;2|3;4}"),
            ("Choice", None, "of:=CHOOSE(2;10;20)"),
        ]
    );
    // PHONETIC (not a function of Tally) and the macro-sheet function
    // are counted; the hidden function name is not a name.
    let left_out: Vec<&String> = wb
        .warnings
        .iter()
        .filter(|w| w.contains("defined name"))
        .collect();
    assert_eq!(left_out.len(), 1, "{:?}", wb.warnings);
    assert!(left_out[0].starts_with("2 defined name(s)"));
    // The cells name them.
    let formula = |sheet: usize, r: u32, c: u32| {
        let cell = wb.sheets[sheet]
            .cells
            .iter()
            .find(|x| (x.row, x.col) == (r, c))
            .unwrap();
        match &cell.content {
            CellContentData::Formula { text, .. } => text.to_string(),
            other => panic!("{other:?}"),
        }
    };
    assert_eq!(formula(0, 2, 0), "of:=Total");
    assert_eq!(formula(0, 3, 2), "of:=SUM(Table)");
    assert_eq!(formula(1, 2, 0), "of:=Top");
}

#[test]
fn function_tokens_in_every_class_and_with_spaces() {
    let cat = |parts: &[&[u8]]| parts.concat();
    let mut g = Vec::new();
    // The reference, value and array classes of both function tokens.
    for (i, base) in [0x20u8, 0x40, 0x60].into_iter().enumerate() {
        g.extend(biff::defined_name(
            &format!("Fixed{i}"),
            0,
            0,
            &cat(&[&biff::ptg_num(1.5), &[0x13], &[base | 0x01, 24, 0]]),
        ));
        g.extend(biff::defined_name(
            &format!("Counted{i}"),
            0,
            0,
            &cat(&[
                &biff::ptg_int(1),
                &biff::ptg_int(2),
                &[base | 0x02, 2, 6, 0],
            ]),
        ));
    }
    // Volatile and space attributes around a call; no arguments at all.
    g.extend(biff::defined_name(
        "Spaced",
        0,
        0,
        &cat(&[
            &biff::ptg_attr(0x01, 0),
            &biff::ptg_attr(0x40, 0x0100),
            &biff::ptg_func(74),
            &biff::ptg_attr(0x41, 0x0100),
            &biff::ptg_func(25),
        ]),
    ));
    g.extend(biff::defined_name("None", 0, 0, &biff::ptg_funcvar(0, 4)));
    // A function over a union, and a percentage.
    g.extend(biff::defined_name(
        "Mixed",
        0,
        0,
        &cat(&[
            &biff::ptg_union(
                &biff::ptg_area3d_on(0, 0, 0, 0, 0),
                &biff::ptg_area3d_on(1, 1, 1, 1, 1),
            ),
            &biff::ptg_attr(0x10, 0),
            &biff::ptg_int(50),
            &[0x14, 0x05],
        ]),
    ));
    let (wb, left_out) = function_book(&[], &g);
    assert_eq!(
        names(&wb),
        [
            ("Fixed0", None, "of:=ABS(-1.5)"),
            ("Counted0", None, "of:=MIN(1;2)"),
            ("Fixed1", None, "of:=ABS(-1.5)"),
            ("Counted1", None, "of:=MIN(1;2)"),
            ("Fixed2", None, "of:=ABS(-1.5)"),
            ("Counted2", None, "of:=MIN(1;2)"),
            ("Spaced", None, "of:=INT(NOW())"),
            ("None", None, "of:=SUM()"),
            (
                "Mixed",
                None,
                "of:=SUM(([$Main.$A$1:.$A$1]~[$'Other sheet'.$B$2:.$B$2]))*50%"
            ),
        ]
    );
    assert_eq!(left_out, 0);
}

#[test]
fn functions_that_are_not_read_leave_their_names_out() {
    let cat = |parts: &[&[u8]]| parts.concat();
    let one = biff::ptg_int(1);
    let mut g = Vec::new();
    // 1–3: an index past the table, a macro-sheet function, a macro
    // command.
    g.extend(biff::defined_name(
        "Past",
        0,
        0,
        &cat(&[&one, &biff::ptg_func(500)]),
    ));
    g.extend(biff::defined_name("Halt", 0, 0, &biff::ptg_func(54)));
    g.extend(biff::defined_name(
        "Command",
        0,
        0,
        &cat(&[&one, &biff::ptg_funcvar(1, 0x8004)]),
    ));
    // 4, 5: worksheet functions Tally does not have, by index and by
    // name.
    g.extend(biff::defined_name(
        "Phonetic",
        0,
        0,
        &cat(&[&one, &biff::ptg_func(360)]),
    ));
    g.extend(biff::defined_name(
        "AddIn",
        0,
        0,
        &cat(&[&biff::ptg_namex(2, 2), &one, &biff::ptg_funcvar(2, 255)]),
    ));
    // 6: a macro function of the workbook (not a name, not counted),
    // and 7: a call of it.
    g.extend(biff::defined_name("MyMacro", 0x000E, 0, &[0x1C, 0x1D]));
    g.extend(biff::defined_name(
        "Calls",
        0,
        0,
        &cat(&[&biff::ptg_name(6), &one, &biff::ptg_funcvar(2, 255)]),
    ));
    // 8: a name on a name that is left out; 9: a count the function does
    // not take; 10: a call by name without a name; 11: more arguments
    // than there are operands.
    g.extend(biff::defined_name(
        "OnPhonetic",
        0,
        0,
        &cat(&[&biff::ptg_name(4), &[0x12]]),
    ));
    g.extend(biff::defined_name(
        "TwoForAbs",
        0,
        0,
        &cat(&[&one, &one, &biff::ptg_funcvar(2, 24)]),
    ));
    g.extend(biff::defined_name(
        "NoName",
        0,
        0,
        &cat(&[&one, &biff::ptg_funcvar(1, 255)]),
    ));
    g.extend(biff::defined_name(
        "TooFew",
        0,
        0,
        &cat(&[&one, &biff::ptg_funcvar(9, 4)]),
    ));
    // An add-in function Tally has, next to them, is read.
    g.extend(biff::defined_name(
        "Kept",
        0,
        0,
        &cat(&[
            &biff::ptg_namex(2, 1),
            &one,
            &one,
            &biff::ptg_funcvar(3, 255),
        ]),
    ));
    let (wb, left_out) = function_book(&["GCD", "NOSUCHFUNCTION"], &g);
    assert_eq!(names(&wb), [("Kept", None, "of:=GCD(1;1)")]);
    assert_eq!(left_out, 10, "{:?}", wb.warnings);
}
