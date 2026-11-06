//! SYLK import against crafted files (Excel's and AOO's dialects) and
//! hostile input.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;

use tally_io_sylk::{read_sylk, sniff_is_sylk, Limits, SylkError};
use tally_model::{CellContentData, WorkbookData};
use tally_types::{ErrorKind, Value};

fn fixture(name: &str) -> Vec<u8> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/sylk")
        .join(name);
    std::fs::read(&p).unwrap()
}

fn at(wb: &WorkbookData, r: u32, c: u32) -> Option<CellContentData> {
    wb.sheets[0]
        .cells
        .iter()
        .find(|x| x.row == r && x.col == c)
        .map(|x| x.content.clone())
}

fn text(s: &str) -> Option<CellContentData> {
    Some(CellContentData::Value(Value::Text(s.into())))
}

fn num(n: f64) -> Option<CellContentData> {
    Some(CellContentData::Value(Value::Number(n)))
}

#[test]
fn excel_sylk() {
    let bytes = fixture("excel.slk");
    assert!(sniff_is_sylk(&bytes));
    let wb = read_sylk(&bytes, &Limits::default()).unwrap();
    assert_eq!(wb.sheets.len(), 1);
    assert_eq!(wb.sheets[0].name, "", "the file name names the sheet");
    assert_eq!(at(&wb, 0, 0), text("Item"));
    assert_eq!(at(&wb, 0, 1), text("Price"));
    assert_eq!(at(&wb, 1, 0), text("Apple; red"));
    assert_eq!(at(&wb, 1, 1), num(1.5));
    assert_eq!(
        at(&wb, 1, 2),
        Some(CellContentData::Value(Value::Bool(true)))
    );
    assert_eq!(at(&wb, 2, 2), num(45000.0));
    // R1C1 formulas become OpenFormula with the value as cached result.
    assert_eq!(
        at(&wb, 3, 1),
        Some(CellContentData::Formula {
            text: "of:=SUM([.B2:.B3])".into(),
            cached: Some(Value::Number(3.75)),
        })
    );
    assert_eq!(
        at(&wb, 3, 2),
        Some(CellContentData::Formula {
            text: "of:=1/0".into(),
            cached: Some(Value::Error(ErrorKind::Div0)),
        })
    );
    assert_eq!(at(&wb, 4, 0), text("line\ntwo"));
    // A reference off the sheet: the value alone, as AOO keeps it.
    assert_eq!(at(&wb, 4, 1), num(7.0));
    // `F;P<n>` number formats (Excel codes → model codes).
    let fmt = |r: u32, c: u32| {
        wb.sheets[0]
            .cells
            .iter()
            .find(|x| x.row == r && x.col == c)
            .and_then(|x| x.style)
            .and_then(|s| wb.styles[s as usize].number_format.clone())
    };
    assert_eq!(fmt(1, 1).as_deref(), Some("0.00"));
    assert_eq!(fmt(2, 1).as_deref(), Some("0.00"));
    assert!(fmt(2, 2).is_some_and(|f| f.contains("YYYY") || f.contains("yyyy")));
    assert_eq!(fmt(0, 0), None);
}

#[test]
fn aoo_sylk_uses_native_formulas_and_escapes() {
    let wb = read_sylk(&fixture("aoo.slk"), &Limits::default()).unwrap();
    assert_eq!(
        at(&wb, 0, 2),
        Some(CellContentData::Formula {
            text: "oooc:=SUM([.A1:.B1])".into(),
            cached: Some(Value::Number(3.0)),
        })
    );
    assert_eq!(at(&wb, 1, 0), text("a;b"));
    match at(&wb, 1, 1) {
        Some(CellContentData::Formula { text, cached }) => {
            assert_eq!(&*text, "oooc:=IF([.A1]>0;\"y;z\";\"n\")");
            assert_eq!(cached, Some(Value::Text("x".into())));
        }
        other => panic!("{other:?}"),
    }
    // `M` with `R`/`C`: an array formula block; `;I` values are ignored.
    assert_eq!(wb.sheets[0].array_formulas.len(), 1);
    let a = wb.sheets[0].array_formulas[0];
    assert_eq!((a.r0, a.c0, a.r1, a.c1), (2, 0, 2, 1));
    assert_eq!(at(&wb, 2, 1), None);
}

#[test]
fn sniffing() {
    assert!(sniff_is_sylk(b"ID;PWXL;N;E\r\n"));
    assert!(sniff_is_sylk(b"\xEF\xBB\xBFID;P\n"));
    assert!(!sniff_is_sylk(b"id;p"));
    assert!(!sniff_is_sylk(b"Name,Value\n"));
    assert!(!sniff_is_sylk(b""));
    assert!(matches!(
        read_sylk(b"a,b\n", &Limits::default()),
        Err(SylkError::NotSylk)
    ));
}

#[test]
fn hostile_input() {
    let lim = Limits::default();
    // Malformed records are refused as AOO does.
    assert!(matches!(
        read_sylk(b"ID;P\nCX1\n", &lim),
        Err(SylkError::Malformed(2))
    ));
    // Coordinates off the sheet or absurd: dropped with a warning.
    let wb = read_sylk(
        b"ID;P\nC;X99999999999999999999;Y1;K1\nC;X0;Y0;K2\nC;X-5;Y3;K3\nC;X16385;Y1;K4\nC;X1;Y1048577;K5\nE\n",
        &lim,
    )
    .unwrap();
    assert!(wb.sheets[0].cells.is_empty());
    assert_eq!(wb.warnings.len(), 1);
    // Cell cap.
    let mut many = String::from("ID;P\n");
    for i in 1..=200 {
        many.push_str(&format!("C;X1;Y{i};K{i}\n"));
    }
    let small = Limits {
        max_cells: 100,
        ..Limits::default()
    };
    assert!(matches!(
        read_sylk(many.as_bytes(), &small),
        Err(SylkError::LimitExceeded { what: "cells", .. })
    ));
    // Format table cap, line cap, byte cap.
    let fmts = format!("ID;P\n{}", "P;P0.00\n".repeat(20));
    let small = Limits {
        max_formats: 10,
        ..Limits::default()
    };
    assert!(read_sylk(fmts.as_bytes(), &small).is_err());
    let long = format!("ID;P\nC;X1;Y1;K\"{}\"\n", "a".repeat(2000));
    let small = Limits {
        max_line_bytes: 1000,
        ..Limits::default()
    };
    assert!(read_sylk(long.as_bytes(), &small).is_err());
    let small = Limits {
        max_bytes: 4,
        ..Limits::default()
    };
    assert!(read_sylk(b"ID;PWXL\n", &small).is_err());
    // F;P with a huge or negative index, unterminated strings and formulas.
    let odd = b"ID;P\nP;P0\nF;P999999999999;X1;Y1\nF;P-1\nC;X1;Y1;K\"abc\nC;X2;Y1;K1;E\"SUM(\nC;X3;Y1;K;E\nC;Y2;X1;K\"\"\nC;\nF;\n";
    let wb = read_sylk(odd, &lim).unwrap();
    assert_eq!(at(&wb, 0, 0), text("abc"));
    // Invalid UTF-8 is read as Windows-1252.
    let wb = read_sylk(b"ID;P\nC;X1;Y1;K\"caf\xe9\"\n", &lim).unwrap();
    assert_eq!(at(&wb, 0, 0), text("café"));
    // Mutations of the fixtures never panic.
    for name in ["excel.slk", "aoo.slk"] {
        let good = fixture(name);
        let mut seed = 0x9e37_79b9_u32;
        for _ in 0..2000 {
            let mut m = good.clone();
            for _ in 0..4 {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                let i = seed as usize % m.len();
                m[i] = b";\"CXYKEMRIPF0123456789-[]\x1b:"[(seed >> 8) as usize % 27];
            }
            let _ = read_sylk(&m, &lim);
        }
    }
}

/// Review M2: over-long `P;P` codes are refused before conversion, and
/// many distinct formats dedupe in constant time each.
#[test]
fn m2_long_and_many_formats_are_cheap() {
    let mut s = String::from("ID;PWXL\n");
    for i in 0..50 {
        s.push_str(&format!("P;P{}{}\n", "0".repeat(1000 * 1000), i));
    }
    s.push_str("C;X1;Y1;K1\nF;P3;X1;Y1\nE\n");
    let t = std::time::Instant::now();
    let wb = read_sylk(s.as_bytes(), &Limits::default()).unwrap();
    assert!(
        t.elapsed() < std::time::Duration::from_secs(10),
        "{:?}",
        t.elapsed()
    );
    assert!(wb.styles.is_empty());
    assert_eq!(wb.warnings.len(), 1);
    assert_eq!(wb.sheets[0].cells[0].style, None);
    let mut s = String::from("ID;PWXL\n");
    for i in 0..10_000 {
        s.push_str(&format!("P;P0.{}\n", "0".repeat(i % 40 + 1)));
    }
    s.push_str("C;X1;Y1;K1\nF;P9999;X1;Y1\nE\n");
    let t = std::time::Instant::now();
    let wb = read_sylk(s.as_bytes(), &Limits::default()).unwrap();
    assert!(
        t.elapsed() < std::time::Duration::from_secs(5),
        "{:?}",
        t.elapsed()
    );
    assert_eq!(wb.styles.len(), 40);
}
