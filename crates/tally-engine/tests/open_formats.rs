//! Opening every format AOO 4.1 opens through the default codec: the
//! macro-enabled and template OOXML files, flat ODS, Excel 2003 XML, SYLK
//! and DIF, sniffed by content before extension, with the one-shot macro
//! notice (`WorkbookInfo::has_macros`).
#![allow(clippy::unwrap_used, clippy::expect_used)]
#![cfg(feature = "io")]

use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};

use tally_api::DocumentService;
use tally_engine::service::codec::{detect, Kind};
use tally_engine::EngineService;
use tally_model::{CellContentData, CellData, SheetData, WorkbookData};
use tally_types::{CellAddr, Value};

fn repo(p: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(p)
}

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("tally-open-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn book() -> WorkbookData {
    WorkbookData {
        sheets: vec![SheetData {
            name: "Data".into(),
            cells: vec![
                CellData {
                    row: 0,
                    col: 0,
                    content: CellContentData::Value(Value::Number(2.0)),
                    style: None,
                },
                CellData {
                    row: 0,
                    col: 1,
                    content: CellContentData::Formula {
                        text: "of:=[.A1]*21".into(),
                        cached: None,
                    },
                    style: None,
                },
            ],
            ..SheetData::default()
        }],
        ..WorkbookData::default()
    }
}

/// Rewrites a zip package: `edit` changes an entry's bytes (or drops it
/// with `None`), and `extra` entries are appended.
fn repack(
    pkg: &[u8],
    edit: impl Fn(&str, Vec<u8>) -> Option<Vec<u8>>,
    extra: &[(&str, &[u8])],
) -> Vec<u8> {
    let mut src = zip::ZipArchive::new(Cursor::new(pkg)).unwrap();
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default();
    for i in 0..src.len() {
        let mut f = src.by_index(i).unwrap();
        let name = f.name().unwrap().to_string();
        let mut bytes = Vec::new();
        f.read_to_end(&mut bytes).unwrap();
        if let Some(b) = edit(&name, bytes) {
            out.start_file(name, opts).unwrap();
            out.write_all(&b).unwrap();
        }
    }
    for (name, bytes) in extra {
        out.start_file(*name, opts).unwrap();
        out.write_all(bytes).unwrap();
    }
    out.finish().unwrap().into_inner()
}

fn xlsx_bytes() -> Vec<u8> {
    let mut cur = Cursor::new(Vec::new());
    tally_io_xlsx::write_xlsx(&book(), &mut cur).unwrap();
    cur.into_inner()
}

/// An Excel package whose workbook part has `content_type` (the
/// macro-enabled or template main part), with a VBA project if `vba`.
fn ooxml_variant(content_type: &str, vba: bool) -> Vec<u8> {
    const MAIN: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml";
    let ct = content_type.to_string();
    let extra: Vec<(&str, &[u8])> = if vba {
        vec![(
            "xl/vbaProject.bin",
            b"\xD0\xCF\x11\xE0 not really a VBA project",
        )]
    } else {
        Vec::new()
    };
    repack(
        &xlsx_bytes(),
        move |name, bytes| {
            if name == "xl/_rels/workbook.xml.rels" && vba {
                let s = String::from_utf8(bytes).unwrap().replace(
                    "</Relationships>",
                    "<Relationship Id=\"rIdVba\" Type=\"http://schemas.microsoft.com/office/2006/relationships/vbaProject\" Target=\"vbaProject.bin\"/></Relationships>",
                );
                return Some(s.into_bytes());
            }
            if name != "[Content_Types].xml" {
                return Some(bytes);
            }
            let s = String::from_utf8(bytes).unwrap();
            assert!(s.contains(MAIN));
            let mut s = s.replace(MAIN, &ct);
            if vba {
                s = s.replace(
                    "<Default ",
                    "<Default Extension=\"bin\" ContentType=\"application/vnd.ms-office.vbaProject\"/><Default ",
                );
            }
            Some(s.into_bytes())
        },
        &extra,
    )
}

fn open(svc: &mut EngineService, path: &Path) -> tally_api::WorkbookInfo {
    svc.open(path)
        .unwrap_or_else(|e| panic!("{}: {}", path.display(), e.message))
}

fn value(svc: &EngineService, info: &tally_api::WorkbookInfo, r: u32, c: u32) -> Value {
    let s = info.sheets[0].id;
    svc.document(info.doc)
        .unwrap()
        .wb
        .value(CellAddr::new(s, r, c))
}

#[test]
fn xlsm_opens_with_the_macro_notice_once() {
    let dir = temp_dir("xlsm");
    let p = dir.join("Book.xlsm");
    std::fs::write(
        &p,
        ooxml_variant("application/vnd.ms-excel.sheet.macroEnabled.main+xml", true),
    )
    .unwrap();
    let mut svc = EngineService::with_default_codec();
    let info = open(&mut svc, &p);
    assert!(info.has_macros);
    assert_eq!(info.sheets[0].name, "Data");
    assert_eq!(value(&svc, &info, 0, 1), Value::Number(42.0));
    // Delivered once, like the warnings.
    assert!(!svc.info(info.doc).unwrap().has_macros);
    // The same package without its VBA project has no macros.
    let q = dir.join("NoVba.xlsm");
    std::fs::write(
        &q,
        ooxml_variant(
            "application/vnd.ms-excel.sheet.macroEnabled.main+xml",
            false,
        ),
    )
    .unwrap();
    assert!(!open(&mut svc, &q).has_macros);
    // A stray `vbaProject.bin` that no relationship names is not a project.
    let stray = repack(
        &xlsx_bytes(),
        |_, b| Some(b),
        &[("xl/vbaProject.bin", b"x")],
    );
    assert!(!tally_io_xlsx::has_macros(&stray));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn templates_open() {
    let dir = temp_dir("xltx");
    let mut svc = EngineService::with_default_codec();
    for (name, ct, vba) in [
        (
            "T.xltx",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.template.main+xml",
            false,
        ),
        (
            "T.xltm",
            "application/vnd.ms-excel.template.macroEnabled.main+xml",
            true,
        ),
    ] {
        let p = dir.join(name);
        let bytes = ooxml_variant(ct, vba);
        assert_eq!(detect(&bytes, "xltx"), Some(Kind::Xlsx));
        std::fs::write(&p, bytes).unwrap();
        let info = open(&mut svc, &p);
        assert_eq!(info.has_macros, vba, "{name}");
        assert_eq!(value(&svc, &info, 0, 1), Value::Number(42.0), "{name}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ods_with_basic_has_macros() {
    let mut cur = Cursor::new(Vec::new());
    tally_io_ods::write_ods(&book(), &mut cur).unwrap();
    let plain = cur.into_inner();
    assert!(!tally_io_ods::has_macros(&plain));
    let index_only = repack(&plain, |_, b| Some(b), &[("Basic/script-lc.xml", b"<x/>")]);
    assert!(!tally_io_ods::has_macros(&index_only));
    let with_module = repack(
        &plain,
        |_, b| Some(b),
        &[("Basic/Standard/Module1.xml", b"<script:module/>")],
    );
    assert!(tally_io_ods::has_macros(&with_module));
    let dir = temp_dir("odsbasic");
    let p = dir.join("m.ods");
    std::fs::write(&p, with_module).unwrap();
    let mut svc = EngineService::with_default_codec();
    assert!(open(&mut svc, &p).has_macros);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn text_formats_open_as_their_own() {
    let mut svc = EngineService::with_default_codec();
    // Flat ODS, with an embedded Basic module.
    let info = open(&mut svc, &repo("tests/fixtures/fods/basic.fods"));
    assert!(info.has_macros);
    assert_eq!(info.sheets.len(), 2);
    assert_eq!(value(&svc, &info, 1, 2), Value::Number(3.0));
    // SYLK (Excel's): the sheet takes the file name; R1C1 formulas compute.
    let info = open(&mut svc, &repo("tests/fixtures/sylk/excel.slk"));
    assert!(!info.has_macros);
    assert_eq!(info.sheets[0].name, "excel");
    assert_eq!(value(&svc, &info, 3, 1), Value::Number(3.75));
    let info = open(&mut svc, &repo("tests/fixtures/sylk/aoo.slk"));
    assert_eq!(value(&svc, &info, 0, 2), Value::Number(3.0));
    // Excel 2003 XML.
    let info = open(&mut svc, &repo("tests/fixtures/xmlss/basic.xml"));
    assert!(!info.has_macros);
    assert!(!info.sheets.is_empty());
    // DIF: the TABLE topic names the sheet.
    let info = open(&mut svc, &repo("tests/fixtures/dif/basic.dif"));
    assert_eq!(info.sheets[0].name, "Prices");
    assert_eq!(value(&svc, &info, 1, 1), Value::Number(1.5));
}

#[test]
fn content_wins_over_the_extension() {
    let xlsx = xlsx_bytes();
    assert_eq!(detect(&xlsx, "xls"), Some(Kind::Xlsx));
    assert_eq!(detect(&xlsx, "ods"), Some(Kind::Xlsx));
    let fods = std::fs::read(repo("tests/fixtures/fods/basic.fods")).unwrap();
    assert_eq!(detect(&fods, "xml"), Some(Kind::Fods));
    assert_eq!(detect(&fods, "ods"), Some(Kind::Fods));
    let slk = std::fs::read(repo("tests/fixtures/sylk/excel.slk")).unwrap();
    assert_eq!(detect(&slk, ""), Some(Kind::Sylk));
    assert_eq!(detect(&slk, "xls"), Some(Kind::Sylk));
    let xml = std::fs::read(repo("tests/fixtures/xmlss/basic.xml")).unwrap();
    assert_eq!(detect(&xml, "xml"), Some(Kind::Xmlss));
    assert_eq!(detect(&xml, ""), Some(Kind::Xmlss));
    let dif = std::fs::read(repo("tests/fixtures/dif/basic.dif")).unwrap();
    assert_eq!(detect(&dif, "dif"), Some(Kind::Dif));
    // A semicolon CSV that starts like SYLK stays CSV.
    let csv = b"ID;Price;Qty\r\n1;2;3\r\n";
    assert_eq!(detect(csv, "csv"), Some(Kind::Csv));
    assert_eq!(detect(csv, "txt"), Some(Kind::Csv));
    assert_eq!(detect(csv, "slk"), Some(Kind::Sylk));
    // Review L1: without an extension, `ID;` alone is not SYLK.
    assert_eq!(detect(csv, ""), Some(Kind::Csv));
    assert_eq!(detect(b"ID;PWXL\nC;X1;Y1;K1\nE\n", ""), Some(Kind::Sylk));
    // Unknown extensions with unknown content are refused.
    assert_eq!(detect(b"hello", "png"), None);
    let dir = temp_dir("csvid");
    let p = dir.join("ids.csv");
    std::fs::write(&p, csv).unwrap();
    let mut svc = EngineService::with_default_codec();
    let info = open(&mut svc, &p);
    assert_eq!(value(&svc, &info, 0, 0), Value::text("ID"));
    // A renamed ODS opens by its content.
    let mut cur = Cursor::new(Vec::new());
    tally_io_ods::write_ods(&book(), &mut cur).unwrap();
    let q = dir.join("renamed.xls");
    std::fs::write(&q, cur.into_inner()).unwrap();
    let info = open(&mut svc, &q);
    assert_eq!(value(&svc, &info, 0, 1), Value::Number(42.0));
    let _ = std::fs::remove_dir_all(&dir);
}
