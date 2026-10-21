//! Correctness review 4: Visual FoxPro headers (M32), FoxPro `Y`/`I`
//! fields (M33), language driver 0 → IBM 850 (M34).
#![allow(clippy::unwrap_used)]

use tally_io_dbf::{read_dbf, sniff_is_dbf, DbfOptions, Limits};
use tally_model::CellContentData;
use tally_types::Value;

/// A table with the given fields `(name, type, len, dec)` and one record.
fn table(version: u8, driver: u8, fields: &[(&str, u8, u8, u8)], record: &[u8]) -> Vec<u8> {
    let vfp = (0x30..=0x32).contains(&version);
    let header_len = 32 + 32 * fields.len() + 1 + if vfp { 263 } else { 0 };
    let record_len = 1 + fields.iter().map(|f| f.2 as usize).sum::<usize>();
    let mut b = vec![version, 126, 10, 21];
    b.extend_from_slice(&1u32.to_le_bytes());
    b.extend_from_slice(&(header_len as u16).to_le_bytes());
    b.extend_from_slice(&(record_len as u16).to_le_bytes());
    b.extend_from_slice(&[0u8; 17]);
    b.push(driver);
    b.extend_from_slice(&[0u8; 2]);
    for (name, kind, len, dec) in fields {
        let mut d = [0u8; 32];
        d[..name.len()].copy_from_slice(name.as_bytes());
        d[11] = *kind;
        d[16] = *len;
        d[17] = *dec;
        b.extend_from_slice(&d);
    }
    b.push(0x0D);
    if vfp {
        b.extend(std::iter::repeat_n(0u8, 263));
    }
    assert_eq!(b.len(), header_len);
    b.push(b' ');
    assert_eq!(record.len() + 1, record_len);
    b.extend_from_slice(record);
    b.push(0x1A);
    b
}

fn cell(wb: &tally_model::WorkbookData, r: u32, c: u32) -> Value {
    wb.sheets[0]
        .cells
        .iter()
        .find(|x| x.row == r && x.col == c)
        .map(|x| match &x.content {
            CellContentData::Value(v) => v.clone(),
            _ => Value::Empty,
        })
        .unwrap_or(Value::Empty)
}

#[test]
fn visual_foxpro_tables_with_currency_integer_double_and_datetime() {
    let mut rec = Vec::new();
    rec.extend_from_slice(&123_456i64.to_le_bytes()); // Y 12.3456
    rec.extend_from_slice(&42i32.to_le_bytes()); // I
    rec.extend_from_slice(&2.5f64.to_le_bytes()); // B (double)
                                                  // T: 2026-10-21 12:00 = JDN 2461335.
    rec.extend_from_slice(&2_461_335i32.to_le_bytes());
    rec.extend_from_slice(&43_200_000i32.to_le_bytes());
    rec.extend_from_slice(b"abc ");
    rec.push(0); // _NullFlags
    let fields = [
        ("PRICE", b'Y', 8, 4),
        ("QTY", b'I', 4, 0),
        ("RATE", b'B', 8, 0),
        ("STAMP", b'T', 8, 0),
        ("CODE", b'C', 4, 0),
        ("_NullFlags", b'0', 1, 0),
    ];
    for version in [0x30u8, 0x31] {
        let b = table(version, 0x03, &fields, &rec);
        assert!(sniff_is_dbf(&b), "{version:#x}");
        let wb = read_dbf(&b, &DbfOptions::default(), &Limits::default()).unwrap();
        assert_eq!(cell(&wb, 1, 0), Value::Number(12.3456));
        assert_eq!(cell(&wb, 1, 1), Value::Number(42.0));
        assert_eq!(cell(&wb, 1, 2), Value::Number(2.5));
        // 2026-10-21 is D1899 serial 46316.
        assert_eq!(cell(&wb, 1, 3), Value::Number(46316.5));
        assert_eq!(cell(&wb, 1, 4), Value::text("abc"));
        assert_eq!(cell(&wb, 0, 4), Value::text("CODE,C,4"));
        // The system field is not shown.
        assert_eq!(cell(&wb, 0, 5), Value::Empty);
    }
}

#[test]
fn language_driver_zero_is_ibm850() {
    let b = table(0x03, 0x00, &[("N", b'C', 2, 0)], &[0x81, 0x84]);
    let wb = read_dbf(&b, &DbfOptions::default(), &Limits::default()).unwrap();
    assert_eq!(cell(&wb, 1, 0), Value::text("üä"));
    // 0x03 stays windows-1252.
    let b = table(0x03, 0x03, &[("N", b'C', 2, 0)], &[0xFC, 0xE4]);
    let wb = read_dbf(&b, &DbfOptions::default(), &Limits::default()).unwrap();
    assert_eq!(cell(&wb, 1, 0), Value::text("üä"));
}
