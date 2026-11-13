//! Window ▸ Split in XLSX: a `pane` that is not frozen, positions in
//! 1/20 pt (ECMA-376 §18.3.1.66), written into `sheetView` and read back
//! as `SheetData::split` (1/100 mm).

#![allow(clippy::unwrap_used)] // tests may unwrap (docs/contributing.md)

use std::io::{Cursor, Read};

use tally_io_xlsx::{read_xlsx, write_xlsx, Limits};
use tally_model::{SheetData, SplitData, WorkbookData};

fn book(sheets: Vec<SheetData>) -> WorkbookData {
    WorkbookData {
        sheets,
        ..WorkbookData::default()
    }
}

fn sheet(name: &str, split: Option<SplitData>, frozen: Option<(u32, u32)>) -> SheetData {
    SheetData {
        name: name.into(),
        split,
        frozen,
        ..SheetData::default()
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

fn replace_part(bytes: Vec<u8>, name: &str, xml: &str) -> Vec<u8> {
    let mut src = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for i in 0..src.len() {
        let mut f = src.by_index(i).unwrap();
        let n = f.name().unwrap().into_owned();
        out.start_file(n.as_str(), zip::write::SimpleFileOptions::default())
            .unwrap();
        if n == name {
            std::io::Write::write_all(&mut out, xml.as_bytes()).unwrap();
        } else {
            std::io::copy(&mut f, &mut out).unwrap();
        }
    }
    out.finish().unwrap().into_inner()
}

#[test]
fn split_round_trips() {
    let both = SplitData {
        x_hmm: 5080, // 2 in = 2880 twips
        y_hmm: 2540,
    };
    let x_only = SplitData {
        x_hmm: 1234,
        y_hmm: 0,
    };
    let y_only = SplitData {
        x_hmm: 0,
        y_hmm: 777,
    };
    let wb = book(vec![
        sheet("Both", Some(both), None),
        sheet("X", Some(x_only), None),
        sheet("Y", Some(y_only), None),
        sheet("None", None, None),
    ]);
    let bytes = write(&wb);
    let s1 = part(&bytes, "xl/worksheets/sheet1.xml");
    assert!(
        s1.contains(
            r#"<pane xSplit="2880" ySplit="1440" activePane="bottomRight" state="split"/>"#
        ),
        "{s1}"
    );
    // `pane` comes first in `sheetView`, which is closed properly.
    let sv = s1.find("<sheetView ").unwrap();
    let pane = s1.find("<pane").unwrap();
    let end = s1.find("</sheetView>").unwrap();
    assert!(sv < pane && pane < end, "{s1}");
    assert!(!s1[sv..pane].contains("/>"), "{s1}");
    let s2 = part(&bytes, "xl/worksheets/sheet2.xml");
    assert!(s2.contains(r#"activePane="topRight""#), "{s2}");
    assert!(!s2.contains("ySplit"), "{s2}");
    let s3 = part(&bytes, "xl/worksheets/sheet3.xml");
    assert!(s3.contains(r#"activePane="bottomLeft""#), "{s3}");
    assert!(!s3.contains("xSplit"), "{s3}");
    assert!(!part(&bytes, "xl/worksheets/sheet4.xml").contains("<pane"));

    let back = read_xlsx(Cursor::new(bytes), &Limits::default()).unwrap();
    let got: Vec<Option<SplitData>> = back.sheets.iter().map(|s| s.split).collect();
    assert_eq!(got, [Some(both), Some(x_only), Some(y_only), None]);
    assert!(back.sheets.iter().all(|s| s.frozen.is_none()));
}

#[test]
fn every_position_round_trips_exactly() {
    // One decimal of twips is enough to get the same 1/100 mm back.
    let wb = book(
        (1..=40u32)
            .map(|i| {
                let sp = SplitData {
                    x_hmm: i * 97 + 3,
                    y_hmm: i * 131,
                };
                sheet(&format!("S{i}"), Some(sp), None)
            })
            .collect(),
    );
    let back = read_xlsx(Cursor::new(write(&wb)), &Limits::default()).unwrap();
    for (a, b) in wb.sheets.iter().zip(&back.sheets) {
        assert_eq!(a.split, b.split, "{}", a.name);
    }
}

#[test]
fn frozen_pane_wins_over_split() {
    let sp = SplitData {
        x_hmm: 3000,
        y_hmm: 3000,
    };
    let bytes = write(&book(vec![sheet("F", Some(sp), Some((2, 1)))]));
    let s1 = part(&bytes, "xl/worksheets/sheet1.xml");
    assert_eq!(s1.matches("<pane").count(), 1, "{s1}");
    assert!(s1.contains(r#"state="frozen""#), "{s1}");
    let back = read_xlsx(Cursor::new(bytes), &Limits::default()).unwrap();
    assert_eq!(back.sheets[0].frozen, Some((2, 1)));
    assert_eq!(back.sheets[0].split, None);
}

/// The written sheet XML of an empty sheet with `pane` as the body of its
/// (self-closed) `sheetView`.
fn with_pane(pane: &str) -> Vec<u8> {
    let bytes = write(&book(vec![sheet("A", None, None)]));
    let xml = part(&bytes, "xl/worksheets/sheet1.xml");
    let at = xml.find("<sheetView ").unwrap();
    let gt = at + xml[at..].find('>').unwrap();
    assert!(xml[..gt].ends_with('/'), "{xml}");
    let patched = format!("{}>{pane}</sheetView>{}", &xml[..gt - 1], &xml[gt + 1..]);
    replace_part(bytes, "xl/worksheets/sheet1.xml", &patched)
}

#[test]
fn excel_split_without_state_is_read() {
    // `state` defaults to "split"; Excel writes fractional twips and a
    // `topLeftCell`.
    let bytes =
        with_pane(r#"<pane xSplit="2880.5" ySplit="-5" topLeftCell="D1" activePane="topRight"/>"#);
    let back = read_xlsx(Cursor::new(bytes), &Limits::default()).unwrap();
    assert_eq!(
        back.sheets[0].split,
        Some(SplitData {
            x_hmm: 5081,
            y_hmm: 0
        })
    );
    assert_eq!(back.sheets[0].frozen, None);
}

#[test]
fn huge_split_is_capped() {
    let bytes = with_pane(r#"<pane ySplit="1e300"/>"#);
    let back = read_xlsx(Cursor::new(bytes), &Limits::default()).unwrap();
    let sp = back.sheets[0].split.unwrap();
    assert_eq!(sp.x_hmm, 0);
    assert_eq!(sp.y_hmm, 26_458_333);
}
