//! BIFF8 fixture import and a malicious corpus (mutations of the fixture).
#![allow(clippy::unwrap_used)]

use std::path::Path;

use tally_io_xls::{read_xls, sniff_is_xls, Limits};
use tally_model::CellContentData;
use tally_types::{ErrorKind, Value};

fn fixture(name: &str) -> Vec<u8> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/xls")
        .join(name);
    std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

#[test]
fn biff8_fixture_values_match() {
    let bytes = fixture("basic.xls");
    assert!(sniff_is_xls(&bytes));
    let wb = read_xls(&bytes, &Limits::default()).unwrap();
    assert_eq!(wb.sheets.len(), 2);
    assert_eq!(wb.sheets[0].name, "Sheet1");
    assert_eq!(wb.sheets[1].name, "Data");
    let s = &wb.sheets[0];
    let get = |r: u32, c: u32| {
        s.cells
            .iter()
            .find(|x| x.row == r && x.col == c)
            .map(|x| x.content.clone())
    };
    assert_eq!(get(0, 0), Some(CellContentData::Value(Value::Number(1.0))));
    assert_eq!(get(1, 0), Some(CellContentData::Value(Value::Number(2.0))));
    assert_eq!(
        get(0, 1),
        Some(CellContentData::Value(Value::text("hello")))
    );
    assert_eq!(
        get(1, 1),
        Some(CellContentData::Value(Value::text("Zürich")))
    );
    assert_eq!(get(2, 0), Some(CellContentData::Value(Value::Bool(true))));
    assert_eq!(
        get(2, 1),
        Some(CellContentData::Value(Value::Error(ErrorKind::Div0)))
    );
    match get(3, 0) {
        Some(CellContentData::Formula { text, cached }) => {
            assert_eq!(&*text, "of:=[.A1]+[.A2]");
            assert_eq!(cached, Some(Value::Number(3.0)));
        }
        other => panic!("{other:?}"),
    }
    match get(3, 1) {
        Some(CellContentData::Formula { text, cached }) => {
            assert_eq!(&*text, "of:=SUM([.A1:.A2])");
            assert_eq!(cached, Some(Value::Number(3.0)));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(s.merges.len(), 1);
    assert_eq!((s.merges[0].r0, s.merges[0].c0, s.merges[0].c1), (0, 1, 2));
    assert_eq!(
        wb.sheets[1].cells[0].content,
        CellContentData::Value(Value::Number(42.0))
    );
}

#[test]
fn malicious_corpus_is_rejected_without_panic() {
    let bytes = fixture("basic.xls");
    // Truncations at every sector boundary and a few odd offsets.
    for cut in [
        8usize,
        100,
        511,
        512,
        600,
        1024,
        1500,
        2000,
        bytes.len() - 1,
    ] {
        let _ = read_xls(&bytes[..cut.min(bytes.len())], &Limits::default());
    }
    // Bit flips through the whole file (header, FAT, directory, records).
    for i in (0..bytes.len()).step_by(3) {
        let mut m = bytes.clone();
        m[i] ^= 0x5A;
        let _ = read_xls(&m, &Limits::default());
    }
    // Lying sizes: huge directory stream size, huge record length.
    let mut lying = bytes.clone();
    lying[0x200 + 0x80 + 0x78..0x200 + 0x80 + 0x7C].copy_from_slice(&u32::MAX.to_le_bytes());
    let _ = read_xls(&lying, &Limits::default());
    // Limits: one cell allowed.
    let tiny = Limits {
        max_cells: 1,
        ..Limits::default()
    };
    assert!(read_xls(&bytes, &tiny).is_err());
    let one_sheet = Limits {
        max_sheets: 1,
        ..Limits::default()
    };
    assert!(read_xls(&bytes, &one_sheet).is_err());
}
