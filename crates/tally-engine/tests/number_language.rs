//! A language-tagged General (`[$-407]General`, from an ODS number style
//! with only `number:language`, or Format Cells ▸ Numbers ▸ Number + German)
//! is still General: typing `50%` or a date applies the recognised format
//! (review of G14, M1).

#![allow(clippy::unwrap_used)] // tests may unwrap (docs/contributing.md)

use tally_engine::Workbook;
use tally_model::{SheetData, StyleData, WorkbookData};
use tally_types::{CellAddr, Value};

#[test]
fn language_tagged_general_takes_the_recognised_format() {
    let mut data = WorkbookData::default();
    data.sheets.push(SheetData {
        name: "Sheet1".into(),
        ..SheetData::default()
    });
    let mut wb = Workbook::from_data(&data);
    let general_de = wb.styles.intern(StyleData {
        number_format: Some("[$-407]General".into()),
        ..StyleData::default()
    });
    assert!(wb.styles.get(general_de).is_general());
    let sheet = wb.sheets[0].id;
    for (row, input, value, code) in [
        (0, "50%", 0.5, "0%"),
        (1, "10/19/2026", 46314.0, "MM/DD/YYYY"),
    ] {
        let addr = CellAddr::new(sheet, row, 0);
        wb.sheets[0]
            .column_mut(0)
            .unwrap()
            .styles
            .set(row, row, general_de);
        wb.set_input(addr, input);
        assert_eq!(wb.value(addr), Value::Number(value), "{input}");
        let got = wb.styles.get(wb.style_at(addr)).format_code.clone();
        assert_eq!(&*got, code, "{input}");
    }
}
