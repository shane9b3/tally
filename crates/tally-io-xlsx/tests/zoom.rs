//! The per-sheet view zoom in XLSX (`sheetView/@zoomScale`): written when
//! not 100 %, read back clamped to AOO's 20–400 % (U18).

#![allow(clippy::unwrap_used)] // tests may unwrap (docs/contributing.md)

use std::io::{Cursor, Read};

use tally_io_xlsx::{read_xlsx, write_xlsx, Limits};
use tally_model::{SheetData, WorkbookData};

fn book(zooms: &[Option<u16>]) -> WorkbookData {
    WorkbookData {
        sheets: zooms
            .iter()
            .enumerate()
            .map(|(i, z)| SheetData {
                name: format!("Sheet{}", i + 1),
                zoom: *z,
                ..SheetData::default()
            })
            .collect(),
        ..WorkbookData::default()
    }
}

fn write(wb: &WorkbookData) -> Vec<u8> {
    let mut buf = Cursor::new(Vec::new());
    write_xlsx(wb, &mut buf).unwrap();
    buf.into_inner()
}

fn part(bytes: &[u8], name: &str) -> String {
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut s = String::new();
    z.by_name(name).unwrap().read_to_string(&mut s).unwrap();
    s
}

#[test]
fn zoom_scale_round_trips() {
    let bytes = write(&book(&[Some(20), None, Some(400)]));
    let s1 = part(&bytes, "xl/worksheets/sheet1.xml");
    assert!(s1.contains(r#"zoomScale="20""#), "{s1}");
    let s2 = part(&bytes, "xl/worksheets/sheet2.xml");
    assert!(!s2.contains("zoomScale"), "{s2}");
    let back = read_xlsx(Cursor::new(bytes), &Limits::default()).unwrap();
    let z: Vec<Option<u16>> = back.sheets.iter().map(|s| s.zoom).collect();
    assert_eq!(z, [Some(20), None, Some(400)]);
}

#[test]
fn excel_zoom_below_twenty_is_clamped() {
    // Excel allows 10 %; Tally (AOO MINZOOM) shows 20 %.
    let bytes = write(&book(&[Some(150)]));
    let sheet =
        part(&bytes, "xl/worksheets/sheet1.xml").replace(r#"zoomScale="150""#, r#"zoomScale="10""#);
    let mut src = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for i in 0..src.len() {
        let mut f = src.by_index(i).unwrap();
        let name = f.name().unwrap().into_owned();
        out.start_file(name.as_str(), zip::write::SimpleFileOptions::default())
            .unwrap();
        if name == "xl/worksheets/sheet1.xml" {
            std::io::Write::write_all(&mut out, sheet.as_bytes()).unwrap();
        } else {
            std::io::copy(&mut f, &mut out).unwrap();
        }
    }
    let bytes = out.finish().unwrap().into_inner();
    let back = read_xlsx(Cursor::new(bytes), &Limits::default()).unwrap();
    assert_eq!(back.sheets[0].zoom, Some(20));
}
