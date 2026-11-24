//! Hostile input for the defined-name and hyperlink readers (`NAME`,
//! `EXTERNSHEET`, `SUPBOOK`, `HLINK`): truncated records, lying lengths,
//! indices without a target, nesting, limits. Like the rest of the
//! formatting pass they must never panic (a caught panic shows as the
//! "could not be read" warning, which these tests treat as a failure) and
//! never turn a file that opens into one that does not.
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

/// A two-sheet workbook with `globals` after the usual ones (and an
/// `EXTERNSHEET` for both sheets) and `sheet` records on the first sheet.
fn book(globals: &[u8], sheet: &[u8]) -> Vec<u8> {
    let mut g = biff::globals(&[], &[], &[]);
    g.extend(biff::externsheet(2));
    g.extend_from_slice(globals);
    let mut s = biff::number(0, 0, 15, 1.0);
    s.extend_from_slice(sheet);
    biff::file(biff::stream(
        &g,
        &[("Main", s), ("Other", biff::number(0, 0, 15, 2.0))],
    ))
}

fn has(wb: &WorkbookData, part: &str) -> bool {
    wb.warnings.iter().any(|w| w.contains(part))
}

#[test]
fn name_records_cut_at_every_length() {
    let mut rgce = biff::ptg_area3d_on(0, 0, 9, 0, 1);
    rgce.extend(biff::ptg_num(2.0));
    rgce.push(0x05);
    let full = biff::defined_name_data("Area", 0, 0, &rgce);
    for cut in 0..=full.len() {
        // calamine refuses a file whose NAME record is shorter than its
        // formula (a known panic it turns into an error): what matters
        // here is that the formatting pass never panics, and that a file
        // calamine reads still opens.
        let mut g = biff::rec(0x0018, &full[..cut]);
        // A later name keeps its place and its meaning.
        g.extend(biff::defined_name("After", 0, 0, &biff::ptg_num(1.0)));
        if let Ok(wb) = read(&book(&g, &[])) {
            assert!(
                wb.names.iter().any(|n| n.name == "After"),
                "cut at {cut}: {:?}",
                wb.names
            );
        }
    }
    // The tables behind 3-D references, cut anywhere.
    let ext = biff::externsheet(2);
    for cut in 0..ext.len() {
        let mut g = biff::globals(&[], &[], &[]);
        // The records' own lengths are kept consistent: cut the data of
        // the EXTERNSHEET record (the SUPBOOK record is 8 bytes).
        if cut < 12 {
            continue;
        }
        g.extend_from_slice(&ext[..8]);
        g.extend(biff::rec(0x0017, &ext[12..cut.max(12)]));
        g.extend(biff::defined_name(
            "Area",
            0,
            0,
            &biff::ptg_area3d_on(1, 0, 0, 0, 0),
        ));
        let bytes = biff::file(biff::stream(
            &g,
            &[
                ("Main", biff::number(0, 0, 15, 1.0)),
                ("Other", biff::number(0, 0, 15, 2.0)),
            ],
        ));
        let _ = read(&bytes);
    }
}

#[test]
fn name_lengths_and_indices_that_lie() {
    let mut g = Vec::new();
    // Tokens cut by the end of the formula. (calamine refuses a file
    // whose name formula starts with a cut reference, so the cut area
    // comes second here.)
    g.extend(biff::defined_name(
        "Cut",
        0,
        0,
        &[0x1E, 1, 0, 0x3B, 0, 0, 0],
    ));
    g.extend(biff::defined_name("CutNum", 0, 0, &[0x1F, 1, 2]));
    g.extend(biff::defined_name("CutStr", 0, 0, &[0x17, 200, 0, b'a']));
    g.extend(biff::defined_name("CutStrWide", 0, 0, &[0x17, 2, 1, b'a']));
    // Operators without operands; operands left over.
    g.extend(biff::defined_name("NoOperand", 0, 0, &[0x05]));
    g.extend(biff::defined_name("Two", 0, 0, &[0x1E, 1, 0, 0x1E, 2, 0]));
    // Names that name each other, themselves, nothing.
    g.extend(biff::defined_name("Self", 0, 0, &biff::ptg_name(7)));
    g.extend(biff::defined_name("Zero", 0, 0, &biff::ptg_name(0)));
    g.extend(biff::defined_name("Far", 0, 0, &biff::ptg_name(u32::MAX)));
    // Sheets that are not there: EXTERNSHEET entries 2…, a local sheet
    // 0xFFFF.
    for ixti in [2u16, 100, 0xFFFF] {
        g.extend(biff::defined_name(
            &format!("NoSheet_{ixti}"),
            0,
            0,
            &biff::ptg_area3d_on(ixti, 0, 0, 0, 0),
        ));
    }
    g.extend(biff::defined_name("NoTab", 0, 0xFFFF, &biff::ptg_num(1.0)));
    // Reversed rows and columns are put in order.
    g.extend(biff::defined_name(
        "Reversed",
        0,
        0,
        &biff::ptg_area3d_on(0, 9, 2, 5, 1),
    ));
    g.extend(biff::defined_name("Good", 0, 0, &biff::ptg_num(3.0)));
    let wb = read(&book(&g, &[])).unwrap();
    let got: Vec<(&str, &str)> = wb
        .names
        .iter()
        .map(|n| (n.name.as_str(), n.formula.as_str()))
        .collect();
    assert_eq!(
        got,
        [("Reversed", "of:=[$Main.$B$3:.$F$10]"), ("Good", "of:=3"),]
    );
    // A name that uses itself is left out with them.
    assert!(has(&wb, "13 defined name(s)"), "{:?}", wb.warnings);

    // EXTERNSHEET: a count far beyond its entries, entries naming
    // SUPBOOKs that are not there, no SUPBOOK at all.
    let mut ext = 0xFFFFu16.to_le_bytes().to_vec();
    for w in [0u16, 0, 0, 9, 0, 0] {
        ext.extend_from_slice(&w.to_le_bytes());
    }
    for with_supbook in [true, false] {
        let mut g = biff::globals(&[], &[], &[]);
        if with_supbook {
            g.extend(biff::rec(0x01AE, &[1, 0, 1, 4]));
        }
        g.extend(biff::rec(0x0017, &ext));
        g.extend(biff::defined_name(
            "A",
            0,
            0,
            &biff::ptg_area3d_on(0, 0, 0, 0, 0),
        ));
        g.extend(biff::defined_name(
            "B",
            0,
            0,
            &biff::ptg_area3d_on(1, 0, 0, 0, 0),
        ));
        let bytes = biff::file(biff::stream(&g, &[("S", biff::number(0, 0, 15, 1.0))]));
        let wb = read(&bytes).unwrap();
        let kept: Vec<&str> = wb.names.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(kept, if with_supbook { vec!["A"] } else { vec![] });
    }
}

#[test]
fn every_token_as_a_name_formula() {
    // Every byte as a one-token formula, as the token after an area, and
    // as a formula of twelve equal bytes: never a panic, and the file
    // opens whenever calamine reads it.
    let mut g = Vec::new();
    for b in 0..=255u8 {
        // calamine refuses a file whose name formula starts with a cut
        // 3-D reference (the decoder's own tests cover those bytes).
        if !(b >= 0x20 && matches!(b & 0x1F, 0x1A..=0x1D)) {
            g.extend(biff::defined_name(&format!("One_{b}"), 0, 0, &[b]));
        }
        let mut after = biff::ptg_area3d_on(0, 0, 1, 0, 1);
        after.push(b);
        g.extend(biff::defined_name(&format!("After_{b}"), 0, 0, &after));
        g.extend(biff::defined_name(&format!("Twelve_{b}"), 0, 0, &[b; 12]));
    }
    let wb = read(&book(&g, &[])).unwrap();
    assert!(has(&wb, "defined name(s)"));
    // Every option flag on a good name.
    let mut g = Vec::new();
    for bit in 0..16 {
        g.extend(biff::defined_name(
            &format!("Flagged_{bit}"),
            1 << bit,
            0,
            &biff::ptg_num(1.0),
        ));
    }
    read(&book(&g, &[])).unwrap();
}

#[test]
fn nested_name_formulas_are_bounded() {
    // 300 names of 1 wrapped in 8,000 pairs of parentheses each: every
    // one is past the text limit after about 4,000 pairs, having moved
    // about 16 MB of text; the workbook's budget (64 MiB) ends the
    // decoding of the rest at their first operator. Plain names after
    // them are still read.
    let mut deep = biff::ptg_int(1);
    deep.extend(std::iter::repeat_n(0x15, 8_000));
    let mut g = Vec::new();
    for i in 0..300 {
        g.extend(biff::defined_name(&format!("Deep_{i}"), 0, 0, &deep));
    }
    // Within the text limit but after the budget: left out too.
    let mut shallow = biff::ptg_int(1);
    shallow.extend(std::iter::repeat_n(0x15, 10));
    g.extend(biff::defined_name("Shallow", 0, 0, &shallow));
    g.extend(biff::defined_name("Plain", 0, 0, &biff::ptg_num(7.0)));
    let started = std::time::Instant::now();
    let wb = read(&book(&g, &[])).unwrap();
    let names: Vec<&str> = wb.names.iter().map(|n| n.name.as_str()).collect();
    assert_eq!(names, ["Plain"]);
    assert!(has(&wb, "301 defined name(s)"), "{:?}", wb.warnings);
    // Generous: the decoding moves at most 64 MiB of text.
    assert!(started.elapsed().as_secs() < 60);
}

#[test]
fn name_limits_degrade_with_a_warning() {
    let mut g = Vec::new();
    for i in 0..10u16 {
        g.extend(biff::defined_name(
            &format!("Item_{i}"),
            0,
            0,
            &biff::ptg_num(f64::from(i)),
        ));
    }
    // A name on a later name (index 10) that the limit cuts.
    let bytes = book(&g, &[]);
    let limits = Limits {
        max_names: 4,
        ..Limits::default()
    };
    let wb = read_with(&bytes, &limits).unwrap();
    assert_eq!(wb.names.len(), 4);
    assert!(has(&wb, "Only the first 4 defined names"));
    // The byte budget: each name keeps 6 bytes of name and 9 of formula.
    let limits = Limits {
        max_name_bytes: 35,
        ..Limits::default()
    };
    let wb = read_with(&bytes, &limits).unwrap();
    assert_eq!(wb.names.len(), 2);
    assert!(has(&wb, "defined names"));
    // The defaults read all of them without a warning.
    let wb = read(&bytes).unwrap();
    assert_eq!(wb.names.len(), 10);
    assert_eq!(wb.warnings, Vec::<String>::new());
}

/// [`book`] with the add-in functions `functions` behind the `EXTERNSHEET`
/// entry 2.
fn function_book(functions: &[&str], globals: &[u8]) -> Vec<u8> {
    let mut g = biff::globals(&[], &[], &[]);
    g.extend(biff::externsheet_with_addin(2, functions));
    g.extend_from_slice(globals);
    biff::file(biff::stream(
        &g,
        &[
            ("Main", biff::number(0, 0, 15, 1.0)),
            ("Other", biff::number(0, 0, 15, 2.0)),
        ],
    ))
}

/// The names of `wb`, and that a plain name after the hostile ones is
/// among them.
fn names_with_the_last(wb: &WorkbookData) -> Vec<&str> {
    let names: Vec<&str> = wb.names.iter().map(|n| n.name.as_str()).collect();
    assert_eq!(names.last(), Some(&"Plain"), "{names:?}");
    names
}

#[test]
fn function_tokens_that_lie() {
    let cat = |parts: &[&[u8]]| parts.concat();
    let one = biff::ptg_int(1);
    let mut g = Vec::new();
    // Function tokens cut by the end of the formula (after an operand:
    // calamine only looks at the first token of a name).
    for cut in [
        &[0x41u8][..],
        &[0x41, 24],
        &[0x42],
        &[0x42, 1],
        &[0x42, 1, 4],
        &[0x19],
        &[0x19, 0x10],
        &[0x19, 0x10, 0],
        &[0x19, 0x04, 1],
        &[0x19, 0x04, 1, 0, 0],
        &[0x39, 2, 0, 1],
        &[0x60, 0, 0],
    ] {
        let name: String = cut.iter().map(|b| format!("{b:02X}")).collect();
        g.extend(biff::defined_name(
            &format!("Cut_{name}"),
            0,
            0,
            &cat(&[&one, cut]),
        ));
    }
    // More arguments than operands: a fixed count, the largest count, the
    // count of a call by name, "sum" of nothing.
    g.extend(biff::defined_name("Few1", 0, 0, &biff::ptg_func(27)));
    g.extend(biff::defined_name(
        "Few2",
        0,
        0,
        &cat(&[&one, &biff::ptg_funcvar(0x7F, 4)]),
    ));
    g.extend(biff::defined_name(
        "Few3",
        0,
        0,
        &cat(&[&biff::ptg_namex(2, 1), &biff::ptg_funcvar(0x7F, 255)]),
    ));
    g.extend(biff::defined_name("Few4", 0, 0, &biff::ptg_attr(0x10, 0)));
    // A call by name without a name, with a name that is nowhere, with
    // no operand at all; a function's name that is never called.
    g.extend(biff::defined_name(
        "NoName1",
        0,
        0,
        &cat(&[&one, &biff::ptg_funcvar(1, 255)]),
    ));
    g.extend(biff::defined_name(
        "NoName2",
        0,
        0,
        &cat(&[
            &biff::ptg_namex(2, u32::MAX),
            &one,
            &biff::ptg_funcvar(2, 255),
        ]),
    ));
    g.extend(biff::defined_name(
        "NoName3",
        0,
        0,
        &cat(&[
            &biff::ptg_namex(0xFFFF, 1),
            &one,
            &biff::ptg_funcvar(2, 255),
        ]),
    ));
    g.extend(biff::defined_name(
        "NoName4",
        0,
        0,
        &biff::ptg_funcvar(0, 255),
    ));
    g.extend(biff::defined_name("NoCall", 0, 0, &biff::ptg_namex(2, 1)));
    // A jump table of 65,536 words in a formula of five bytes; an
    // attribute of every type.
    g.extend(biff::defined_name(
        "Jumps",
        0,
        0,
        &cat(&[&one, &biff::ptg_attr(0x04, 0xFFFF)]),
    ));
    for kind in 0..=255u8 {
        g.extend(biff::defined_name(
            &format!("Attr_{kind}"),
            0,
            0,
            &cat(&[&one, &biff::ptg_attr(kind, 0xFFFF)]),
        ));
    }
    // Every function index of the table and around it, with one operand
    // and with four.
    for iftab in (0..600u16).chain([0x7FFF, 0x8000, 0x8004, 0xFFFF]) {
        g.extend(biff::defined_name(
            &format!("Fn_{iftab}"),
            0,
            0,
            &cat(&[&one, &biff::ptg_func(iftab)]),
        ));
        g.extend(biff::defined_name(
            &format!("FnVar_{iftab}"),
            0,
            0,
            &cat(&[&one, &one, &one, &one, &biff::ptg_funcvar(4, iftab)]),
        ));
    }
    // An array constant without its values, with values that are cut, and
    // with 256 × 65,536 elements announced.
    let array = [0x60u8, 0, 0, 0, 0, 0, 0, 0];
    let values = biff::array_values(&[&[1.0, 2.0], &[3.0, 4.0]]);
    for cut in 0..values.len() {
        g.extend(biff::defined_name_with_arrays(
            &format!("Array_{cut}"),
            0,
            &array,
            &values[..cut],
        ));
    }
    g.extend(biff::defined_name_with_arrays(
        "Huge",
        0,
        &array,
        &[0xFF, 0xFF, 0xFF, 0x01, 0, 0, 0, 0, 0, 0, 0, 0],
    ));
    g.extend(biff::defined_name("Plain", 0, 0, &biff::ptg_num(7.0)));
    let wb = read(&function_book(&["GCD"], &g)).unwrap();
    let names = names_with_the_last(&wb);
    // Nothing of the hostile part is a name, except the calls that are
    // good ones: an attribute that changes nothing, a function with one
    // or four arguments.
    for name in &names {
        assert!(
            ["Attr_", "Fn_", "FnVar_", "Plain"]
                .iter()
                .any(|p| name.starts_with(p)),
            "{names:?}"
        );
    }
    assert!(has(&wb, "defined name(s)"));
    // Every formula the reader kept is one the engine's parser reads.
    assert!(wb.names.iter().all(|n| n.formula.starts_with("of:=")));
}

#[test]
fn externname_records_cut_and_garbled() {
    // EXTERNNAME cut at every length, with every flag, with a name
    // length that lies, with names that are not functions; one after
    // another in the add-in SUPBOOK, in a SUPBOOK that is not one, and
    // with no SUPBOOK at all.
    let full = biff::externname_data("GCD");
    let mut records = Vec::new();
    for cut in 0..=full.len() {
        records.extend(biff::rec(0x0023, &full[..cut]));
    }
    for bit in 0..16 {
        let mut d = full.clone();
        d[..2].copy_from_slice(&(1u16 << bit).to_le_bytes());
        records.extend(biff::rec(0x0023, &d));
    }
    for len in [0u8, 4, 200, 255] {
        for wide in [0u8, 1] {
            let mut d = full.clone();
            d[6] = len;
            d[7] = wide;
            records.extend(biff::rec(0x0023, &d));
        }
    }
    for name in ["", "A1(", "1+1", "SUM(1)+", "a b", "\"", "'", "É"] {
        records.extend(biff::rec(0x0023, &biff::externname_data(name)));
    }
    let count = 4 + 5 + 16 + 8 + 8 + 1;
    for supbook in [Some(0x3A01u16), Some(0x0401), Some(0x1234), None] {
        let mut g = biff::globals(&[], &[], &[]);
        g.extend(biff::rec(0x01AE, &[1, 0, 1, 4]));
        if let Some(kind) = supbook {
            let mut d = 1u16.to_le_bytes().to_vec();
            d.extend_from_slice(&kind.to_le_bytes());
            g.extend(biff::rec(0x01AE, &d));
        }
        g.extend_from_slice(&records);
        let mut ext = 2u16.to_le_bytes().to_vec();
        for w in [0u16, 0, 0, 1, 0xFFFE, 0xFFFE] {
            ext.extend_from_slice(&w.to_le_bytes());
        }
        g.extend(biff::rec(0x0017, &ext));
        // A call of every record by name.
        for i in 0..=count + 2 {
            let mut rgce = biff::ptg_namex(1, i);
            rgce.extend(biff::ptg_int(4));
            rgce.extend(biff::ptg_int(6));
            rgce.extend(biff::ptg_funcvar(3, 255));
            g.extend(biff::defined_name(&format!("Call_{i}"), 0, 0, &rgce));
        }
        g.extend(biff::defined_name("Plain", 0, 0, &biff::ptg_num(7.0)));
        let bytes = biff::file(biff::stream(&g, &[("S", biff::number(0, 0, 15, 1.0))]));
        let wb = read(&bytes).unwrap();
        let names = names_with_the_last(&wb);
        // Only records that name the function whole are functions: every
        // call that is read is GCD(4;6).
        for n in &wb.names {
            assert!(
                n.name == "Plain" || n.formula == "of:=GCD(4;6)",
                "{supbook:?}: {n:?}"
            );
        }
        if supbook != Some(0x3A01) {
            assert_eq!(names, ["Plain"], "{supbook:?}");
        } else {
            assert!(names.len() > 1);
        }
    }
}

#[test]
fn nested_calls_are_bounded() {
    // 20 names of 1 inside 20,000 × ABS(…) each (past the text limit
    // after about 1,600), 20 of 20,000 left-out arguments for one SUM,
    // and one of 1,500 nested calls that fits the text limit: the
    // decoding is a loop, the conversion has its own depth limit.
    let nested = |depth: usize| {
        let mut rgce = biff::ptg_int(1);
        for _ in 0..depth {
            rgce.extend(biff::ptg_func(24));
        }
        rgce
    };
    let mut wide = biff::ptg_int(1);
    wide.extend(std::iter::repeat_n(0x16, 20_000));
    wide.extend(biff::ptg_funcvar(30, 4));
    let mut g = Vec::new();
    for i in 0..20 {
        g.extend(biff::defined_name(
            &format!("Deep_{i}"),
            0,
            0,
            &nested(20_000),
        ));
        g.extend(biff::defined_name(&format!("Wide_{i}"), 0, 0, &wide));
    }
    g.extend(biff::defined_name("Fits", 0, 0, &nested(1_500)));
    g.extend(biff::defined_name("Plain", 0, 0, &biff::ptg_num(7.0)));
    let started = std::time::Instant::now();
    let wb = read(&function_book(&[], &g)).unwrap();
    let names = names_with_the_last(&wb);
    assert!(
        names.iter().all(|n| ["Fits", "Plain"].contains(n)),
        "{names:?}"
    );
    assert!(has(&wb, "defined name(s)"), "{:?}", wb.warnings);
    assert!(started.elapsed().as_secs() < 60);
}

#[test]
fn function_name_limits_degrade_with_a_warning() {
    // The EXTERNNAME records count against the limit of names: past it
    // the calls of the later ones are left out, and the file opens.
    let functions: Vec<String> = (0..10).map(|_| "GCD".to_string()).collect();
    let functions: Vec<&str> = functions.iter().map(String::as_str).collect();
    let mut g = Vec::new();
    for i in 1..=10u32 {
        let mut rgce = biff::ptg_namex(2, i);
        rgce.extend(biff::ptg_int(4));
        rgce.extend(biff::ptg_int(6));
        rgce.extend(biff::ptg_funcvar(3, 255));
        g.extend(biff::defined_name(&format!("Call_{i}"), 0, 0, &rgce));
    }
    let bytes = function_book(&functions, &g);
    let wb = read(&bytes).unwrap();
    assert_eq!(wb.names.len(), 10);
    assert_eq!(wb.warnings, Vec::<String>::new());
    let limits = Limits {
        max_names: 12,
        ..Limits::default()
    };
    let wb = read_with(&bytes, &limits).unwrap();
    assert_eq!(wb.names.len(), 10);
    let limits = Limits {
        max_names: 4,
        ..Limits::default()
    };
    let wb = read_with(&bytes, &limits).unwrap();
    // Four function names and four names are kept: Call_1 … Call_4.
    let names: Vec<&str> = wb.names.iter().map(|n| n.name.as_str()).collect();
    assert_eq!(names, ["Call_1", "Call_2", "Call_3", "Call_4"]);
    assert!(has(&wb, "Only the first 4 defined names"));
}

#[test]
fn hlink_records_cut_at_every_length() {
    let samples = [
        biff::hlink_url_data((1, 1, 0, 0), "https://example.org/a", Some("mark")),
        biff::hlink_file_data((1, 1, 0, 0), 2, "dir\\file.xls"),
    ];
    for full in &samples {
        for cut in 0..full.len() {
            let mut s = biff::rec(0x01B8, &full[..cut]);
            s.extend(biff::hlink_url(
                (9, 9, 0, 0),
                "https://example.org/after",
                None,
            ));
            let wb = read(&book(&[], &s)).unwrap();
            // The cut record is left out and counted; the next one is
            // read.
            let l = &wb.sheets[0].links;
            assert_eq!(l.len(), 1, "cut at {cut}");
            assert_eq!(&*l[0].url, "https://example.org/after");
            assert!(has(&wb, "1 hyperlink(s)"), "cut at {cut}");
        }
    }
}

#[test]
fn hlink_lengths_that_lie() {
    let mut s = Vec::new();
    // String lengths of 4 billion characters: display name, location.
    for flags in [0x10u32, 0x08, 0x80, 0x100] {
        let mut d = biff::hlink_head((1, 1, 0, 0), flags);
        d.extend_from_slice(&u32::MAX.to_le_bytes());
        d.extend_from_slice(&[b'a', 0, 0, 0]);
        s.extend(biff::rec(0x01B8, &d));
    }
    // A URL moniker and a file moniker with lengths beyond the record.
    let mut url = biff::hlink_head((1, 1, 0, 0), 0x03);
    url.extend_from_slice(&biff::URL_MONIKER);
    url.extend_from_slice(&0x7FFF_FFFEu32.to_le_bytes());
    url.extend_from_slice(&[b'h', 0, 0, 0]);
    s.extend(biff::rec(0x01B8, &url));
    let mut file = biff::hlink_file_data((1, 1, 0, 0), 0, "a.xls");
    let at = 8 + 24 + 16 + 2;
    file[at..at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    s.extend(biff::rec(0x01B8, &file));
    // 65,535 directories up; an unknown moniker; no target at all.
    s.extend(biff::hlink_file((1, 1, 0, 0), u16::MAX, "a.xls"));
    let mut unknown = biff::hlink_head((1, 1, 0, 0), 0x01);
    unknown.extend_from_slice(&[0x11; 40]);
    s.extend(biff::rec(0x01B8, &unknown));
    s.extend(biff::rec(0x01B8, &biff::hlink_head((1, 1, 0, 0), 0)));
    // Reversed cells; an empty record.
    s.extend(biff::hlink_url((5, 2, 0, 0), "https://example.org/", None));
    s.extend(biff::hlink_url((2, 2, 7, 3), "https://example.org/", None));
    s.extend(biff::rec(0x01B8, &[]));
    // A target with a control character, and one over 8 KiB (in a record
    // continued over CONTINUE records).
    s.extend(biff::hlink_url(
        (1, 1, 0, 0),
        "https://example.org/\n",
        None,
    ));
    let long = biff::hlink_url_data(
        (1, 1, 0, 0),
        &format!("https://example.org/{}", "a".repeat(9_000)),
        None,
    );
    let mut chunks = long.chunks(8_000);
    s.extend(biff::rec(0x01B8, chunks.next().unwrap()));
    for c in chunks {
        s.extend(biff::rec(0x003C, c));
    }
    // A record continued past the bytes looked at (64 KiB).
    s.extend(biff::rec(
        0x01B8,
        &biff::hlink_url_data((1, 1, 0, 0), "https://example.org/", None),
    ));
    for _ in 0..10 {
        s.extend(biff::rec(0x003C, &[0u8; 8_000]));
    }
    let wb = read(&book(&[], &s)).unwrap();
    assert!(wb.sheets[0].links.is_empty(), "{:?}", wb.sheets[0].links);
    assert!(has(&wb, "15 hyperlink(s)"), "{:?}", wb.warnings);
}

#[test]
fn link_limits_degrade_with_a_warning() {
    // A link over 10,001 cells is left out; one over 10,000 is read.
    let mut s = biff::hlink_url((0, 10_000, 0, 0), "https://example.org/big", None);
    s.extend(biff::hlink_url(
        (0, 9_999, 1, 1),
        "https://example.org/ok",
        None,
    ));
    let bytes = book(&[], &s);
    let wb = read(&bytes).unwrap();
    assert_eq!(wb.sheets[0].links.len(), 10_000);
    assert!(has(&wb, "1 hyperlink(s)"));
    // The cells with a link are capped for the whole workbook.
    let limits = Limits {
        max_links: 25,
        ..Limits::default()
    };
    let wb = read_with(&bytes, &limits).unwrap();
    assert_eq!(wb.sheets[0].links.len(), 25);
    assert!(has(&wb, "Only the first 25 cells with a hyperlink"));
    // So are the bytes of the targets.
    let mut s = Vec::new();
    for r in 0..10u16 {
        s.extend(biff::hlink_url(
            (r, r, 0, 0),
            "https://example.org/0123456789",
            None,
        ));
    }
    let bytes = book(&[], &s);
    let limits = Limits {
        max_link_text_bytes: 100,
        ..Limits::default()
    };
    let wb = read_with(&bytes, &limits).unwrap();
    assert_eq!(wb.sheets[0].links.len(), 3);
    assert!(has(&wb, "cells with a hyperlink"));
    // Links count as cells: they get the room the value cells leave and
    // never fail a file.
    let limits = Limits {
        max_cells: 6,
        ..Limits::default()
    };
    let wb = read_with(&bytes, &limits).unwrap();
    assert_eq!(wb.sheets[0].links.len(), 4);
    assert!(has(&wb, "some hyperlinks were left out"));
    let wb = read(&bytes).unwrap();
    assert_eq!(wb.sheets[0].links.len(), 10);
    assert_eq!(wb.warnings, Vec::<String>::new());
}

#[test]
fn many_links_on_the_same_cells_are_one_each() {
    // 5,000 records on the same two cells: two links, the first target.
    let mut s = Vec::new();
    for i in 0..5_000u32 {
        s.extend(biff::hlink_url(
            (3, 3, 0, 1),
            &format!("https://example.org/{i}"),
            None,
        ));
    }
    let wb = read(&book(&[], &s)).unwrap();
    let l = &wb.sheets[0].links;
    assert_eq!(l.len(), 2);
    assert_eq!(&*l[0].url, "https://example.org/0");
    assert_eq!(&*l[1].url, "https://example.org/0");
}
