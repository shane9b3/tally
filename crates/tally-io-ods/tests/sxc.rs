//! OpenOffice.org 1.x (`.sxc`) import through the ODS reader.
#![allow(clippy::unwrap_used)]

use std::path::Path;

use tally_io_ods::{read_ods, read_sxc, sniff_is_ods, sniff_is_sxc, Limits};
use tally_model::CellContentData;
use tally_types::Value;

#[test]
fn sxc_fixture_maps_onto_ods() {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/sxc/ooo1.sxc");
    let bytes = std::fs::read(&p).unwrap();
    assert!(sniff_is_sxc(&bytes));
    assert!(!sniff_is_ods(&bytes));
    assert!(read_ods(std::io::Cursor::new(&bytes), &Limits::default()).is_err());
    let wb = read_sxc(std::io::Cursor::new(&bytes), &Limits::default()).unwrap();
    assert_eq!(wb.sheets.len(), 2);
    let s = &wb.sheets[0];
    let get = |r: u32, c: u32| {
        s.cells
            .iter()
            .find(|x| x.row == r && x.col == c)
            .map(|x| x.content.clone())
    };
    assert_eq!(get(0, 0), Some(CellContentData::Value(Value::Number(1.0))));
    match get(1, 0) {
        Some(CellContentData::Formula { text, cached }) => {
            assert_eq!(&*text, "oooc:=SUM([.A1:.B1])");
            assert_eq!(cached, Some(Value::Number(3.0)));
        }
        other => panic!("{other:?}"),
    }
    match get(1, 2) {
        Some(CellContentData::Formula { text, .. }) => {
            assert_eq!(&*text, "oooc:=[Sheet2.A1]&\"!\"");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(get(2, 1), Some(CellContentData::Value(Value::Bool(true))));
    let bold = s
        .cells
        .iter()
        .find(|x| x.row == 0 && x.col == 2)
        .unwrap()
        .style
        .unwrap();
    assert!(wb.styles[bold as usize].font.bold);
    assert!(wb.styles[bold as usize].font.underline);
    assert_eq!(s.col_widths[0].size_hmm, 3500);
    assert_eq!(s.row_heights[0].size_hmm, 600);
    assert_eq!(wb.names.len(), 1);
    assert_eq!(wb.settings.locale.as_deref(), Some("de-DE"));
}
