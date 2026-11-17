//! A sheet read from an Excel file has a height for every row (the file's
//! default for most). Inserting or deleting rows leaves a few rows without
//! one; saved as XLSX the sheet must still say the file's default and
//! write only the rows that differ, not 100,000 `<row>` elements with a
//! warning. Through the real operations, for `.xlsx` and `.xls` sources.
#![allow(clippy::unwrap_used, clippy::expect_used)]
#![cfg(feature = "io")]

use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};

use tally_api::{DocumentService, FileFormat, Op, Transaction};
use tally_engine::EngineService;
use tally_model::{DimRun, WorkbookData};
use tally_types::MAX_ROWS;

fn repo(p: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(p)
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tally-xlsx-sizes-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

/// A one-sheet workbook as Excel writes it: 15 pt rows, a row of 30 pt,
/// a hidden one, a column of 145 px.
fn excel_book() -> Vec<u8> {
    const NS: &str = r#"xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;
    let sheet = format!(
        concat!(
            r#"<worksheet {}><sheetFormatPr defaultRowHeight="15"/>"#,
            r#"<cols><col min="2" max="2" width="20.7109375" customWidth="1"/></cols>"#,
            r#"<sheetData>"#,
            r#"<row r="1"><c r="A1"><v>1</v></c></row>"#,
            r#"<row r="3" ht="30" customHeight="1"><c r="A3"><v>3</v></c></row>"#,
            r#"<row r="5" hidden="1"><c r="A5"><v>5</v></c></row>"#,
            r#"<row r="9"><c r="A9"><v>9</v></c></row>"#,
            r#"</sheetData></worksheet>"#
        ),
        NS
    );
    let entries = [
        (
            "xl/workbook.xml",
            format!(
                r#"<workbook {NS}><sheets><sheet name="S" sheetId="1" r:id="rId1"/></sheets></workbook>"#
            ),
        ),
        (
            "xl/_rels/workbook.xml.rels",
            concat!(
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
                r#"<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>"#,
                r#"</Relationships>"#
            )
            .to_string(),
        ),
        ("xl/worksheets/sheet1.xml", sheet),
    ];
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default();
    for (name, data) in entries {
        z.start_file(name, opts).unwrap();
        z.write_all(data.as_bytes()).unwrap();
    }
    z.finish().unwrap().into_inner()
}

/// The height and hidden flag of `row` (24 px for a row without a run).
fn height(runs: &[DimRun], row: u32) -> (u32, bool) {
    runs.iter()
        .find(|r| r.start <= row && row <= r.end)
        .map_or((tally_model::APP_DEFAULT_ROW_HEIGHT_HMM, false), |r| {
            (r.size_hmm, r.hidden)
        })
}

/// The rows looked at: the top of the sheet and its end.
fn probes() -> impl Iterator<Item = u32> {
    (0..300).chain(MAX_ROWS - 300..MAX_ROWS)
}

fn read_saved(path: &Path) -> (WorkbookData, String) {
    let bytes = std::fs::read(path).unwrap();
    let wb = tally_io_xlsx::read_xlsx(Cursor::new(bytes.clone()), &Default::default()).unwrap();
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut xml = String::new();
    z.by_name("xl/worksheets/sheet1.xml")
        .unwrap()
        .read_to_string(&mut xml)
        .unwrap();
    (wb, xml)
}

/// Opens `source`, applies the row operation, saves as XLSX and checks
/// the saved file: the heights are those of `before` shifted by the
/// operation, the default is still `default_attr`, few rows are written.
fn check(source: &Path, before: &[DimRun], tag: &str, insert: Option<(u32, u32)>, delete: u32) {
    let default_hmm = before.last().unwrap().size_hmm;
    assert_eq!(
        before.last().unwrap().end,
        MAX_ROWS - 1,
        "the source has a height for every row"
    );
    let mut svc = EngineService::with_default_codec();
    let info = svc.open(source).unwrap();
    let (doc, sheet) = (info.doc, info.sheets[0].id);
    let op = match insert {
        Some((at, count)) => Op::InsertRows { sheet, at, count },
        None => Op::DeleteRows {
            sheet,
            at: delete,
            count: 1,
        },
    };
    let r = svc
        .apply(
            doc,
            Transaction {
                ops: vec![op],
                label: None,
                selection_before: None,
                selection_after: None,
                group: None,
            },
        )
        .unwrap();
    assert!(r.errors.is_empty(), "{:?}", r.errors);

    let out = temp(&format!("{tag}.xlsx"));
    let saved = svc.save(doc, Some(&out), FileFormat::Xlsx).unwrap();
    assert!(
        !saved.warnings.iter().any(|w| w.contains("row heights")),
        "{:?}",
        saved.warnings
    );
    let (back, xml) = read_saved(&out);
    let rows = &back.sheets[0].row_heights;

    // Small: only the rows that are not the default are written.
    assert!(xml.matches("<row ").count() < 100, "{tag}: {}", xml.len());
    assert!(xml.len() < 100_000, "{tag}: {} bytes", xml.len());
    let pt = f64::from(default_hmm) * 72.0 / 2540.0;
    let pt = (pt * 4.0).round() / 4.0;
    assert!(
        xml.contains(&format!(r#"defaultRowHeight="{pt}""#)),
        "{tag}: {}",
        &xml[..xml.len().min(600)]
    );

    // The same heights as before the save (whole pixels).
    let px = |hmm: u32| (f64::from(hmm) * 96.0 / 2540.0).round() as u32;
    for row in probes() {
        let want = match insert {
            Some((at, _)) if row < at => Some(height(before, row)),
            // The inserted rows take the height of the run they are in,
            // or the application's when they are between two.
            Some((at, count)) if row < at + count => None,
            Some((_, count)) => Some(height(before, row - count)),
            None if row < delete => Some(height(before, row)),
            None if row + 1 < MAX_ROWS => Some(height(before, row + 1)),
            // The row that came in at the end has no height of its own.
            None => Some((tally_model::APP_DEFAULT_ROW_HEIGHT_HMM, false)),
        };
        let got = height(rows, row);
        if let Some(want) = want {
            assert_eq!((px(got.0), got.1), (px(want.0), want.1), "{tag}: row {row}");
        }
    }
    // The rows past the cap of 100,000 written rows in particular.
    assert_eq!(height(rows, 500_000).0, default_hmm, "{tag}");

    // Saved again from the reopened file: nothing moves.
    let again = svc.open(&out).unwrap();
    let out2 = temp(&format!("{tag}-2.xlsx"));
    svc.save(again.doc, Some(&out2), FileFormat::Xlsx).unwrap();
    let (back2, _) = read_saved(&out2);
    assert_eq!(&back2.sheets[0].row_heights, rows, "{tag}");
    assert_eq!(
        back2.sheets[0].col_widths, back.sheets[0].col_widths,
        "{tag}"
    );
}

#[test]
fn rows_deleted_or_inserted_in_an_excel_sheet_keep_the_files_default() {
    let source = temp("source.xlsx");
    std::fs::write(&source, excel_book()).unwrap();
    let before = tally_io_xlsx::read_xlsx(Cursor::new(excel_book()), &Default::default())
        .unwrap()
        .sheets
        .remove(0)
        .row_heights;
    // 15 pt everywhere but rows 3 (30 pt) and 5 (hidden).
    assert_eq!(before.len(), 5, "{before:?}");
    check(&source, &before, "xlsx-delete", None, 6);
    check(&source, &before, "xlsx-delete-first", None, 0);
    // At the start of a run (the inserted rows have no height) and inside
    // one.
    check(&source, &before, "xlsx-insert-edge", Some((2, 2)), 0);
    check(&source, &before, "xlsx-insert-top", Some((0, 3)), 0);
    check(&source, &before, "xlsx-insert-inside", Some((7, 1)), 0);
    let _ = std::fs::remove_dir_all(source.parent().unwrap());
}

#[test]
fn rows_deleted_or_inserted_in_an_xls_sheet_keep_the_files_default() {
    let source = repo("tests/fixtures/xls/formatted.xls");
    let before = tally_io_xls::read_xls(&std::fs::read(&source).unwrap(), &Default::default())
        .unwrap()
        .sheets
        .remove(0)
        .row_heights;
    check(&source, &before, "xls-delete", None, 1);
    check(&source, &before, "xls-insert", Some((0, 2)), 0);
    check(&source, &before, "xls-insert-inside", Some((40, 1)), 0);
}
