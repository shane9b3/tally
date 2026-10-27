//! AutoFill direction and ordinals (T16) and formulas typed with a
//! leading `+` / `-` (P18), checked against AOO 4.1 (`ScTable::FillAuto`,
//! `lcl_DecompValueString`, `ScViewFunc::EnterData`).
#![allow(clippy::unwrap_used)]

mod common;

use common::{range, Fixture};
use tally_api::{CsvColumnType, CsvImportOptionsDto, DocumentService, FillMode, Op, PastePayload};
use tally_types::Value;

fn n(x: f64) -> Value {
    Value::Number(x)
}

fn t(s: &str) -> Value {
    Value::text(s)
}

fn fill(f: &mut Fixture, source: &str, target: &str, mode: FillMode) {
    let s = f.sheet;
    let r = f.apply(vec![Op::FillRange {
        sheet: s,
        source: range(source),
        target: range(target),
        mode,
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
}

fn input_of(f: &Fixture, a1: &str) -> String {
    let a = tally_types::CellAddr::from_a1(f.sheet, a1).unwrap();
    f.svc
        .cell_edit_info(f.doc, f.sheet, a.row, a.col)
        .unwrap()
        .input
}

fn is_formula(f: &Fixture, a1: &str) -> bool {
    let a = tally_types::CellAddr::from_a1(f.sheet, a1).unwrap();
    f.svc.document(f.doc).unwrap().wb.formula_at(a).is_some()
}

// ----- T16: AutoFill ----------------------------------------------------------------

#[test]
fn single_number_counts_down_when_filled_up_or_left() {
    let mut f = Fixture::new();
    f.set("A5", "5");
    fill(&mut f, "A5", "A2:A5", FillMode::Auto);
    assert_eq!(
        [f.value("A4"), f.value("A3"), f.value("A2")],
        [n(4.0), n(3.0), n(2.0)]
    );
    f.set("E1", "5");
    fill(&mut f, "E1", "B1:E1", FillMode::Auto);
    assert_eq!(
        [f.value("D1"), f.value("C1"), f.value("B1")],
        [n(4.0), n(3.0), n(2.0)]
    );
    // Down and right still count up.
    f.set("G1", "5");
    fill(&mut f, "G1", "G1:G3", FillMode::Auto);
    assert_eq!(f.value("G3"), n(7.0));
    f.set("H5", "5");
    fill(&mut f, "H5", "H5:J5", FillMode::Auto);
    assert_eq!(f.value("J5"), n(7.0));
    // Copy mode (AOO's Ctrl-drag) copies in every direction.
    f.set("K5", "5");
    fill(&mut f, "K5", "K3:K5", FillMode::Copy);
    assert_eq!(f.value("K3"), n(5.0));
}

#[test]
fn single_date_time_and_numbered_text_count_down_up() {
    let mut f = Fixture::new();
    f.set("A5", "03/10/2024");
    fill(&mut f, "A5", "A3:A5", FillMode::Auto);
    assert_eq!(f.text("A4"), "03/09/2024");
    assert_eq!(f.text("A3"), "03/08/2024");
    // A time steps by one day (the time of day is kept), like AOO.
    f.set("B5", "10:30");
    fill(&mut f, "B5", "B4:B5", FillMode::Auto);
    let (Value::Number(b4), Value::Number(b5)) = (f.value("B4"), f.value("B5")) else {
        panic!("numbers expected");
    };
    assert!((b5 - b4 - 1.0).abs() < 1e-9);
    f.set("C5", "Item 5");
    fill(&mut f, "C5", "C3:C5", FillMode::Auto);
    assert_eq!([f.value("C4"), f.value("C3")], [t("Item 4"), t("Item 3")]);
    f.set("D5", "Wed");
    fill(&mut f, "D5", "D4:D5", FillMode::Auto);
    assert_eq!(f.value("D4"), t("Tue"));
    f.set("G1", "Item 1");
    fill(&mut f, "G1", "E1:G1", FillMode::Auto);
    assert_eq!([f.value("F1"), f.value("E1")], [t("Item 0"), t("Item -1")]);
}

#[test]
fn ordinals_fill_with_english_suffixes() {
    let mut f = Fixture::new();
    f.set("A1", "1st");
    fill(&mut f, "A1", "A1:A112", FillMode::Auto);
    for (cell, want) in [
        ("A2", "2nd"),
        ("A3", "3rd"),
        ("A4", "4th"),
        ("A11", "11th"),
        ("A12", "12th"),
        ("A13", "13th"),
        ("A21", "21st"),
        ("A22", "22nd"),
        ("A23", "23rd"),
        ("A101", "101st"),
        ("A111", "111th"),
        ("A112", "112th"),
    ] {
        assert_eq!(f.value(cell), t(want), "{cell}");
    }
    // Two ordinals form a linear series.
    f.set("B1", "1st");
    f.set("B2", "3rd");
    fill(&mut f, "B1:B2", "B1:B4", FillMode::Auto);
    assert_eq!([f.value("B3"), f.value("B4")], [t("5th"), t("7th")]);
    // Up counts down.
    f.set("C5", "3rd");
    fill(&mut f, "C5", "C3:C5", FillMode::Auto);
    assert_eq!([f.value("C4"), f.value("C3")], [t("2nd"), t("1st")]);
    // A suffix that is not the number's ordinal is kept (AOO).
    f.set("D1", "1th");
    fill(&mut f, "D1", "D1:D2", FillMode::Auto);
    assert_eq!(f.value("D2"), t("2th"));
}

#[test]
fn numbered_text_follows_aoo_decomposition() {
    let mut f = Fixture::new();
    // A leading number followed by a space wins.
    f.set("A1", "1 to 5");
    fill(&mut f, "A1", "A1:A2", FillMode::Auto);
    assert_eq!(f.value("A2"), t("2 to 5"));
    // No zero padding without a leading zero: "Item 10" up gives "Item 9".
    f.set("B5", "Item 10");
    fill(&mut f, "B5", "B4:B5", FillMode::Auto);
    assert_eq!(f.value("B4"), t("Item 9"));
    // A non-linear pattern repeats, each repetition +1 (AOO FILL_SIMPLE).
    f.set("C1", "Item 1");
    f.set("C2", "x");
    f.set("C3", "7");
    fill(&mut f, "C1:C3", "C1:C6", FillMode::Auto);
    assert_eq!(
        [f.value("C4"), f.value("C5"), f.value("C6")],
        [t("Item 2"), t("x"), n(8.0)]
    );
    // ... and −1 when filled up.
    f.set("D5", "Item 1");
    f.set("D6", "x");
    fill(&mut f, "D5:D6", "D3:D6", FillMode::Auto);
    assert_eq!([f.value("D4"), f.value("D3")], [t("x"), t("Item 0")]);
}

// ----- P18: leading + / - -------------------------------------------------------------

#[test]
fn leading_sign_starts_a_formula() {
    let mut f = Fixture::new();
    f.set("A1", "2");
    f.set("A2", "3");
    f.set("A3", "4");
    f.set("B1", "10");
    f.set("B2", "20");
    for (cell, typed, stored, value) in [
        ("C1", "+B1+B2", "=+B1+B2", 30.0),
        ("C2", "+A1", "=+A1", 2.0),
        ("C3", "-A1*2", "=-A1*2", -4.0),
        ("C4", "+SUM(A1:A3)", "=+SUM(A1:A3)", 9.0),
        ("C5", "-B1", "=-B1", -10.0),
        ("C6", "+TRUE()", "=+TRUE", 1.0),
    ] {
        f.set(cell, typed);
        assert!(is_formula(&f, cell), "{typed}");
        assert_eq!(input_of(&f, cell), stored, "{typed}");
        let v = f.value(cell);
        let num = match v {
            Value::Number(x) => x,
            Value::Bool(b) => f64::from(u8::from(b)),
            other => panic!("{typed}: {other:?}"),
        };
        assert_eq!(num, value, "{typed}");
    }
}

#[test]
fn signed_numbers_and_text_stay_values() {
    let mut f = Fixture::new();
    for (typed, want) in [
        ("-5", n(-5.0)),
        ("+5", n(5.0)),
        ("+1e3", n(1000.0)),
        ("-2020", n(-2020.0)),
        ("-", t("-")),
        ("+", t("+")),
        ("+-", t("+-")),
        ("--5", t("--5")),
        ("+-5", t("+-5")),
        // Unknown names and functions: AOO's extended error detection
        // keeps the text (not a #NAME? formula).
        ("-abc", t("-abc")),
        ("+foo(1)", t("+foo(1)")),
        ("-Total", t("-Total")),
        // Does not compile: text.
        ("+1)", t("+1)")),
        ("+B1+", t("+B1+")),
        ("'+B1", t("+B1")),
    ] {
        f.set("A1", typed);
        assert!(!is_formula(&f, "A1"), "{typed} became a formula");
        assert_eq!(f.value("A1"), want, "{typed}");
    }
    f.set("A1", "-5%");
    assert!(!is_formula(&f, "A1"));
    assert_eq!(f.value("A1"), n(-0.05));
}

#[test]
fn sign_then_blank_is_a_number() {
    // AOO's number scanner skips blanks after a sign: "- 5" is −5, a
    // number, not a formula.
    let mut f = Fixture::new();
    f.set("A1", "- 5");
    assert!(!is_formula(&f, "A1"));
    assert_eq!(f.value("A1"), n(-5.0));
}

#[test]
fn defined_names_count_as_known() {
    let mut f = Fixture::new();
    f.set("A1", "7");
    let r = f.apply(vec![Op::DefineName {
        name: "Rate".into(),
        scope: None,
        refers_to: "$Sheet1.$A$1".into(),
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    f.set("B1", "-Rate");
    assert!(is_formula(&f, "B1"));
    assert_eq!(f.value("B1"), n(-7.0));
}

#[test]
fn text_formatted_cell_keeps_signed_input() {
    let mut f = Fixture::new();
    f.set("B1", "1");
    let s = f.sheet;
    let r = f.apply(vec![Op::SetNumberFormat {
        sheet: s,
        ranges: vec![range("A1")],
        code: "@".into(),
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    f.set("A1", "+B1+B1");
    assert!(!is_formula(&f, "A1"));
    assert_eq!(f.value("A1"), t("+B1+B1"));
}

#[test]
fn paste_and_text_to_columns_do_not_make_formulas() {
    let mut f = Fixture::new();
    f.set("B1", "1");
    let s = f.sheet;
    let r = f.apply(vec![Op::Paste {
        sheet: s,
        target: range("C1"),
        payload: PastePayload::Tsv {
            text: "+B1+B1\t-abc\t-5".into(),
        },
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(f.value("C1"), t("+B1+B1"));
    assert_eq!(f.value("D1"), t("-abc"));
    assert_eq!(f.value("E1"), n(-5.0));
    // Fill Selection (Alt+Enter) writes a block: no conversion (AOO
    // EnterBlock uses SetString for more than one cell).
    let r = f.apply(vec![Op::SetRangeInputs {
        sheet: s,
        r: 4,
        c: 0,
        rows: vec![vec!["+B1".into(), "+B1".into()]],
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(f.value("A5"), t("+B1"));
    // Text to columns.
    f.set("A10", "x;+B1+B1;-5");
    let r = f.apply(vec![Op::TextToColumns {
        sheet: s,
        range: range("A10"),
        options: CsvImportOptionsDto {
            delimiters: ";".into(),
            column_types: vec![CsvColumnType::Standard; 3],
            ..CsvImportOptionsDto::default()
        },
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(f.value("B10"), t("+B1+B1"));
    assert_eq!(f.value("C10"), n(-5.0));
}

#[cfg(feature = "io")]
#[test]
fn csv_import_does_not_make_formulas() {
    let dir = std::env::temp_dir().join(format!("tally-signed-csv-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("signed.csv");
    std::fs::write(&p, "1,+A1+A1,-abc,-5\n").unwrap();
    let mut svc = tally_engine::EngineService::with_default_codec();
    let info = svc.open(&p).unwrap();
    let s = info.sheets[0].id;
    let wb = &svc.document(info.doc).unwrap().wb;
    assert_eq!(wb.value(tally_types::CellAddr::new(s, 0, 1)), t("+A1+A1"));
    assert_eq!(wb.value(tally_types::CellAddr::new(s, 0, 2)), t("-abc"));
    assert_eq!(wb.value(tally_types::CellAddr::new(s, 0, 3)), n(-5.0));
    std::fs::remove_dir_all(&dir).ok();
}

// ----- review follow-ups ----------------------------------------------------------------

#[test]
fn autofill_uses_the_options_sort_lists() {
    let mut f = Fixture::new();
    let r = f.apply(vec![Op::SetCustomSortLists {
        lists: vec![vec![
            "North".into(),
            "East".into(),
            "South".into(),
            "West".into(),
        ]],
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    f.set("A1", "South");
    fill(&mut f, "A1", "A1:A4", FillMode::Auto);
    assert_eq!(
        [f.value("A2"), f.value("A3"), f.value("A4")],
        [t("West"), t("North"), t("East")]
    );
    f.set("B5", "North");
    fill(&mut f, "B5", "B4:B5", FillMode::Auto);
    assert_eq!(f.value("B4"), t("West"));
    // "May" is found in the abbreviated month list first (AOO ScUserList order).
    f.set("C1", "May");
    fill(&mut f, "C1", "C1:C2", FillMode::Auto);
    assert_eq!(f.value("C2"), t("Jun"));
}

#[test]
fn copy_mode_copies_and_reanchors_formulas() {
    // ⌘D / ⌘R send mode `copy` (AOO FillSimple).
    let mut f = Fixture::new();
    f.set("A1", "5");
    f.set("B1", "=A1*2");
    f.set("C1", "Item 1");
    fill(&mut f, "A1:C1", "A1:C3", FillMode::Copy);
    assert_eq!([f.value("A2"), f.value("A3")], [n(5.0), n(5.0)]);
    assert_eq!(input_of(&f, "B3"), "=A3*2");
    assert_eq!(f.value("B3"), n(10.0));
    assert_eq!(f.value("C3"), t("Item 1"));
}

#[test]
fn literal_block_input_never_makes_signed_formulas() {
    // Hyperlink display text and Merge Cells content go through a 1×1
    // SetRangeInputs (AOO does not route them through EnterData).
    let mut f = Fixture::new();
    let s = f.sheet;
    for (i, text) in ["+1-555-1234", "-1-2", "-abc"].iter().enumerate() {
        let r = f.apply(vec![Op::SetRangeInputs {
            sheet: s,
            r: i as u32,
            c: 0,
            rows: vec![vec![(*text).into()]],
        }]);
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        let a1 = format!("A{}", i + 1);
        assert!(!is_formula(&f, &a1), "{text}");
        assert_eq!(f.value(&a1), t(text));
    }
}

#[test]
fn signed_date_is_a_formula_like_aoo() {
    // AOO's number scanner rejects a signed date (zforfind.cxx: "signed
    // date/datetime"), so EnterData compiles it: 12/31/2024 is a division.
    let mut f = Fixture::new();
    f.set("A1", "+12/31/2024");
    assert!(is_formula(&f, "A1"));
    assert_eq!(input_of(&f, "A1"), "=+12/31/2024");
    let Value::Number(x) = f.value("A1") else {
        panic!("number expected")
    };
    assert!((x - 12.0 / 31.0 / 2024.0).abs() < 1e-12);
    // A signed time is a number.
    f.set("A2", "-12:30");
    assert!(!is_formula(&f, "A2"));
    assert!(matches!(f.value("A2"), Value::Number(v) if v < 0.0));
}
