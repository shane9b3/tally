//! Linked sheets (`table:table-source`) are imported as values and tagged
//! for the engine's external-link policy (wave 5, W5-3).
#![allow(clippy::unwrap_used)]

use std::io::Cursor;

use tally_io_ods::{read_ods, write_ods, Limits};
use tally_model::{CellContentData, CellData, SheetData, WorkbookData};
use tally_types::Value;

#[test]
fn linked_sheet_is_tagged_and_values_kept() {
    let mut wb = WorkbookData::default();
    wb.sheets.push(SheetData {
        name: "Linked".into(),
        cells: vec![CellData {
            row: 0,
            col: 0,
            content: CellContentData::Value(Value::Number(7.0)),
            style: None,
        }],
        ..SheetData::default()
    });
    let mut buf = Cursor::new(Vec::new());
    write_ods(&wb, &mut buf).unwrap();
    let mut z = zip::ZipArchive::new(Cursor::new(buf.into_inner())).unwrap();
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for i in 0..z.len() {
        let mut f = z.by_index(i).unwrap();
        let name = f.name().unwrap().to_string();
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut f, &mut bytes).unwrap();
        if name == "content.xml" {
            let xml = String::from_utf8(bytes).unwrap();
            let at = xml.find("<table:table-column").unwrap();
            let injected = format!(
                "{}<table:table-source xlink:href=\"file:///tmp/source.ods\" table:table-name=\"Sheet1\" table:mode=\"copy-all\"/>{}",
                &xml[..at],
                &xml[at..]
            );
            bytes = injected.into_bytes();
        }
        let opts = zip::write::SimpleFileOptions::default();
        out.start_file(name.as_str(), opts).unwrap();
        std::io::Write::write_all(&mut out, &bytes).unwrap();
    }
    let bytes = out.finish().unwrap().into_inner();
    let back = read_ods(Cursor::new(bytes), &Limits::default()).unwrap();
    assert_eq!(
        back.sheets[0].cells[0].content,
        CellContentData::Value(Value::Number(7.0))
    );
    assert!(
        back.warnings
            .iter()
            .any(|w| w == "tally:external-link:linkedSheet:file:///tmp/source.ods"),
        "{:?}",
        back.warnings
    );
}
