//! DIF import against a crafted file and hostile input.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;

use tally_io_dif::{read_dif, sniff_is_dif, DifError, Limits};
use tally_model::{CellContentData, WorkbookData};
use tally_types::Value;

fn fixture() -> Vec<u8> {
    std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/dif/basic.dif"))
        .unwrap()
}

fn at(wb: &WorkbookData, r: u32, c: u32) -> Option<Value> {
    wb.sheets[0]
        .cells
        .iter()
        .find(|x| x.row == r && x.col == c)
        .map(|x| match &x.content {
            CellContentData::Value(v) => v.clone(),
            other => panic!("{other:?}"),
        })
}

fn t(s: &str) -> Option<Value> {
    Some(Value::Text(s.into()))
}

#[test]
fn reads_like_aoo() {
    let bytes = fixture();
    assert!(sniff_is_dif(&bytes));
    let wb = read_dif(&bytes, &Limits::default()).unwrap();
    assert_eq!(wb.sheets[0].name, "Prices");
    assert_eq!(at(&wb, 0, 0), t("Item"));
    assert_eq!(at(&wb, 0, 1), t("Say \"hi\""));
    assert_eq!(at(&wb, 0, 2), None, "an empty string leaves the cell empty");
    assert_eq!(at(&wb, 1, 0), t("Apple"));
    assert_eq!(at(&wb, 1, 1), Some(Value::Number(1.5)));
    assert_eq!(at(&wb, 1, 2), Some(Value::Bool(true)));
    assert_eq!(at(&wb, 2, 0), t("two\nlines"));
    assert_eq!(at(&wb, 2, 1), t("NA"));
    assert_eq!(at(&wb, 2, 2), t("#IND: X?"));
    assert_eq!(at(&wb, 2, 3), t("#ERR: abc (V)"));
    assert_eq!(wb.sheets[0].cells.len(), 9);
}

#[test]
fn sniffing_and_format_errors() {
    assert!(sniff_is_dif(b"TABLE\n0,1\n\"\"\n"));
    assert!(!sniff_is_dif(b"TABLE,0,1\n"));
    assert!(!sniff_is_dif(b"ID;P\n"));
    assert!(!sniff_is_dif(b""));
    assert!(matches!(
        read_dif(b"TABLE\n0,1\n\"\"\n", &Limits::default()),
        Err(DifError::NotDif)
    ));
    assert!(matches!(
        read_dif(b"", &Limits::default()),
        Err(DifError::NotDif)
    ));
    // No BOT before the first value: row 0, AOO's base column.
    let wb = read_dif(b"DATA\n0,0\n\"\"\n0,5\nV\n-1,0\nEOD\n", &Limits::default()).unwrap();
    assert_eq!(at(&wb, 0, 0), Some(Value::Number(5.0)));
}

#[test]
fn hostile_input() {
    let lim = Limits::default();
    // Cell cap.
    let mut s = String::from("DATA\n0,0\n\"\"\n-1,0\nBOT\n");
    for i in 0..500 {
        s.push_str(&format!("0,{i}\nV\n"));
    }
    let small = Limits {
        max_cells: 100,
        ..Limits::default()
    };
    assert!(matches!(
        read_dif(s.as_bytes(), &small),
        Err(DifError::LimitExceeded { what: "cells", .. })
    ));
    // Columns past the sheet: dropped with a warning, without storing them.
    let mut wide = String::from("DATA\n0,0\n\"\"\n-1,0\nBOT\n");
    for _ in 0..16_500 {
        wide.push_str("0,1\nV\n");
    }
    let wb = read_dif(wide.as_bytes(), &lim).unwrap();
    assert_eq!(wb.sheets[0].cells.len(), 16_384);
    assert_eq!(wb.warnings.len(), 1);
    // An unterminated multi-line string is bounded.
    let open = format!("DATA\n0,0\n\"\"\n1,0\n\"{}", "line\n".repeat(1000));
    let small = Limits {
        max_string_bytes: 1000,
        ..Limits::default()
    };
    assert!(read_dif(open.as_bytes(), &small).is_err());
    let _ = read_dif(open.as_bytes(), &lim).unwrap();
    // Line and byte caps.
    let small = Limits {
        max_line_bytes: 10,
        ..Limits::default()
    };
    assert!(read_dif(b"DATA\n0,0\n\"0123456789abc\"\n", &small).is_err());
    let small = Limits {
        max_bytes: 3,
        ..Limits::default()
    };
    assert!(read_dif(b"DATA\n", &small).is_err());
    // Truncations and mutations never panic.
    let good = fixture();
    for cut in 0..good.len() {
        let _ = read_dif(&good[..cut], &lim);
    }
    let mut seed = 0x1234_5678_u32;
    for _ in 0..3000 {
        let mut m = good.clone();
        for _ in 0..4 {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let i = seed as usize % m.len();
            m[i] = b"-10,\"\nVBOTED\r\xff"[(seed >> 8) as usize % 14];
        }
        let _ = read_dif(&m, &lim);
    }
}

/// Review M1: lines are split lazily, so a file of newlines costs no
/// vector of slices; a long line deep in the file still stops the read.
#[test]
fn m1_lines_are_read_lazily() {
    let mut s = String::from("TABLE\n0,1\n\"\"\nDATA\n0,0\n\"\"\n");
    s.push_str(&"\n".repeat(8 * 1024 * 1024));
    let t = std::time::Instant::now();
    let wb = read_dif(s.as_bytes(), &Limits::default()).unwrap();
    assert!(wb.sheets[0].cells.is_empty());
    assert!(t.elapsed() < std::time::Duration::from_secs(10));
    let mut s = String::from("DATA\n0,0\n\"\"\n-1,0\nBOT\n0,1\nV\n");
    s.push_str(&"x".repeat(2000));
    s.push('\n');
    let small = Limits {
        max_line_bytes: 1000,
        ..Limits::default()
    };
    assert!(matches!(
        read_dif(s.as_bytes(), &small),
        Err(DifError::LimitExceeded {
            what: "line bytes",
            ..
        })
    ));
    // A long line in the header too.
    let s = format!("TABLE\n0,1\n\"{}\"\nDATA\n0,0\n\"\"\n", "y".repeat(2000));
    assert!(matches!(
        read_dif(s.as_bytes(), &small),
        Err(DifError::LimitExceeded {
            what: "line bytes",
            ..
        })
    ));
}
