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

/// A formatted `.xls` (the synthetic fixtures built by
/// `crates/tally-io-xls/tests/biff/books.rs`) opens with its layout:
/// column widths, row heights, cell formats, view settings, pictures and
/// notes reach the document.
#[test]
fn xls_opens_with_its_formatting() {
    let mut svc = EngineService::with_default_codec();
    let info = open(&mut svc, &repo("tests/fixtures/xls/formatted.xls"));
    assert_eq!(info.sheets.len(), 2);
    let sheet = info.sheets[0].id;
    let wb = &svc.document(info.doc).unwrap().wb;
    let style = |r: u32, c: u32| wb.styles.to_dto(wb.style_at(CellAddr::new(sheet, r, c)));
    // A1: Times New Roman, bold italic red, centred, wrapped, "0.000".
    let a1 = style(0, 0);
    assert_eq!(a1.font_family.as_deref(), Some("Times New Roman"));
    assert!(a1.bold && a1.italic && a1.wrap);
    assert_eq!(a1.color, Some(0xFF00_00FF));
    assert_eq!(a1.number_format, "0.000");
    // B1: Courier New 8 pt on a red fill; A2 is a date.
    let b1 = style(0, 1);
    assert_eq!(b1.font_size_pt, 8.0);
    assert_eq!(b1.fill, Some(0xFF00_00FF));
    assert_eq!(wb.value(CellAddr::new(sheet, 1, 0)), Value::Number(45000.0));
    assert_eq!(style(1, 0).number_format, "YYYY-MM-DD");
    // A4 is an empty cell with borders; G9 has column G's default format.
    assert!(style(3, 0).borders.bottom.is_some());
    assert!(style(8, 6).borders.bottom.is_some());
    let data = wb.to_data();
    let one = &data.sheets[0];
    let col = |c: u32| {
        one.col_widths
            .iter()
            .find(|r| r.start <= c && c <= r.end)
            .map(|r| (r.size_hmm, r.hidden))
    };
    // OpenOffice's widths: 10 characters of Arial 10 are 1100 twips, the
    // sheet's default (DEFCOLWIDTH 8) 996 twips.
    assert_eq!(col(0), Some((1940, false)));
    assert_eq!(col(1), Some((1757, false)));
    assert_eq!(col(2).map(|c| c.1), Some(true));
    assert_eq!(col(200).map(|c| c.1), Some(false));
    let row = |r: u32| {
        one.row_heights
            .iter()
            .find(|x| x.start <= r && r <= x.end)
            .map(|x| (x.size_hmm, x.hidden))
    };
    // Every row has the height of the file: its own, or the sheet's
    // default of 12.75 pt.
    assert_eq!(row(0), Some((450, false)));
    assert_eq!(row(1), Some((1058, false)));
    assert_eq!(row(2), Some((529, false)));
    assert_eq!(row(3).map(|r| r.1), Some(true));
    assert_eq!(row(70_000), Some((450, false)));
    let layout = svc.sheet_layout(info.doc, sheet).unwrap();
    assert!(layout.row_runs.len() < 10, "{:?}", layout.row_runs);
    assert_eq!(layout.row_runs[0].size.round(), 17.0);
    assert_eq!(one.frozen, Some((1, 2)));
    assert_eq!(one.zoom, Some(150));
    assert_eq!(one.tab_color, Some(0xFF00_00FF));
    let view = svc.view_options(info.doc, sheet).unwrap();
    assert!(!view.show_gridlines && !view.show_headers && !view.show_zero_values);
    assert_eq!(data.settings.default_font.as_deref(), Some("Arial"));
    assert_eq!(data.settings.active_sheet, 1);

    // page.xls: the page setup, a row default format and a formatted
    // column whose cells with the default format keep the default look.
    let info = open(&mut svc, &repo("tests/fixtures/xls/page.xls"));
    let sheet = info.sheets[0].id;
    let wb = &svc.document(info.doc).unwrap().wb;
    let bordered = |r: u32, c: u32| {
        wb.styles
            .to_dto(wb.style_at(CellAddr::new(sheet, r, c)))
            .borders
            .left
            .is_some()
    };
    // Column B is bordered: B9 has it, B1 (a number) and B3 (an empty
    // cell), both with the default format, have not.
    assert!(bordered(8, 1));
    assert!(!bordered(0, 1) && !bordered(2, 1));
    // Row 2's default format: A2, B2 and D2, not the number in C2, and
    // not past the columns the sheet uses.
    assert!(bordered(1, 0) && bordered(1, 1) && bordered(1, 3));
    assert!(!bordered(1, 2) && !bordered(1, 4));
    let data = wb.to_data();
    let p = data.sheets[0].page_setup.as_ref().unwrap();
    assert_eq!(p.orientation, tally_model::OrientationData::Landscape);
    assert_eq!(p.paper, tally_model::PaperData::Letter);
    assert_eq!(p.scale, tally_model::PageScaleData::Percent(80));
    assert_eq!((p.margins.left, p.margins.right), (422, 635));
    assert_eq!(p.repeat_rows, Some((0, 1)));
    assert_eq!(data.sheets[0].row_breaks, vec![5, 20]);
    assert_eq!(
        data.sheets[1].page_setup.as_ref().map(|p| p.scale),
        Some(tally_model::PageScaleData::FitTo {
            width: 1,
            height: 0
        })
    );
    // Saved as ODS and XLSX the rows stay runs: the files are small and
    // the heights come back from the ODS.
    let dir = temp_dir("xls-save");
    for (ext, format) in [
        ("ods", tally_api::FileFormat::Ods),
        ("xlsx", tally_api::FileFormat::Xlsx),
    ] {
        let out = dir.join(format!("page.{ext}"));
        svc.save_copy(info.doc, &out, format).unwrap();
        let len = std::fs::metadata(&out).unwrap().len();
        assert!(len < 64 * 1024, "{ext}: {len} bytes");
    }
    let back = open(&mut svc, &dir.join("page.ods"));
    let rows = svc
        .document(back.doc)
        .unwrap()
        .wb
        .to_data()
        .sheets
        .swap_remove(0)
        .row_heights;
    assert_eq!(rows, data.sheets[0].row_heights);
    let _ = std::fs::remove_dir_all(&dir);

    let info = open(&mut svc, &repo("tests/fixtures/xls/pictures.xls"));
    let images = svc.images(info.doc, info.sheets[0].id).unwrap();
    assert_eq!(images.len(), 3);
    let info = open(&mut svc, &repo("tests/fixtures/xls/notes.xls"));
    let data = svc.document(info.doc).unwrap().wb.to_data();
    assert_eq!(data.sheets[0].notes.len(), 2);
    assert_eq!(data.sheets[0].notes[0].text, "first note");
}

/// The defined names and the hyperlinks of an `.xls` reach the document:
/// the names are defined (with their scope), formulas that use them
/// calculate, and the links are on their cells.
#[test]
fn xls_opens_with_its_names_and_links() {
    let mut svc = EngineService::with_default_codec();
    let info = open(&mut svc, &repo("tests/fixtures/xls/names.xls"));
    let (main, other) = (info.sheets[0].id, info.sheets[1].id);
    let names = svc.names(info.doc).unwrap();
    let mut got: Vec<(&str, Option<tally_types::SheetId>, &str)> = names
        .iter()
        .map(|n| (n.name.as_str(), n.scope, n.refers_to.as_str()))
        .collect();
    got.sort();
    assert_eq!(
        got,
        [
            ("Both", None, "($Main.$A$1:$A$1~$'Other sheet'.$B$2:$B$2)",),
            ("Data", None, "$Main.$A$1:$B$1"),
            ("Double", None, "Rate*2"),
            // A whole column of the BIFF sheet is a whole column here.
            ("Local", Some(other), "$'Other sheet'.$C:$C"),
            ("Rate", None, "0.25"),
            // A name whose formula is a function.
            ("Stamp", None, "NOW()"),
        ]
    );
    // =SUM(Data) and =Double+1, calculated (not only the stored results).
    svc.recalc_all(info.doc).unwrap();
    let wb = &svc.document(info.doc).unwrap().wb;
    assert_eq!(wb.value(CellAddr::new(main, 4, 0)), Value::Number(3.0));
    assert_eq!(wb.value(CellAddr::new(main, 4, 1)), Value::Number(1.5));
    let data = wb.to_data();
    let links: Vec<(u32, u32, &str)> = data.sheets[0]
        .links
        .iter()
        .map(|l| (l.row, l.col, &*l.url))
        .collect();
    assert_eq!(
        links,
        [
            (1, 0, "https://example.org/"),
            (1, 1, "https://example.org/"),
            (2, 0, "#'Other sheet'.B2"),
            (3, 0, "../folder/book.xls"),
            (5, 0, "https://example.org/doc#part"),
        ]
    );
    // The print area next to the names is the sheet's, not a name.
    assert!(data.sheets[0]
        .page_setup
        .as_ref()
        .is_some_and(|p| p.print_range.is_some()));
}

/// The names of an `.xls` whose formula calls functions (built-in ones by
/// their index, `SUM` as an attribute, `IF` and `CHOOSE` with their jumps,
/// an add-in function and a newer function by name, an array constant)
/// are defined, and the cells that use them calculate; the stored results
/// in the file are all 0.
#[test]
fn xls_names_with_functions_calculate() {
    let mut svc = EngineService::with_default_codec();
    let info = open(&mut svc, &repo("tests/fixtures/xls/funcnames.xls"));
    let (main, other) = (info.sheets[0].id, info.sheets[1].id);
    let names = svc.names(info.doc).unwrap();
    let mut listed: Vec<(&str, Option<tally_types::SheetId>)> =
        names.iter().map(|n| (n.name.as_str(), n.scope)).collect();
    listed.sort_unstable();
    assert_eq!(
        listed,
        [
            ("Choice", None),
            ("Data", None),
            ("Half", None),
            ("Plus", None),
            ("Rounded", None),
            ("Safe", None),
            ("Size", None),
            ("Sparse", None),
            ("Table", None),
            ("Top", Some(other)),
            ("Total", None),
        ]
    );
    // The two names with a function Tally does not read are reported.
    assert!(
        info.warnings
            .iter()
            .any(|w| w.starts_with("2 defined name(s)")),
        "{:?}",
        info.warnings
    );
    svc.recalc_all(info.doc).unwrap();
    let wb = &svc.document(info.doc).unwrap().wb;
    let number = |sheet, r: u32, c: u32, want: f64| match wb.value(CellAddr::new(sheet, r, c)) {
        Value::Number(n) => assert!((n - want).abs() < 1e-9, "({r}, {c}): {n} != {want}"),
        other => panic!("({r}, {c}): {other:?}"),
    };
    // Row 3: Total = SUM(A1:B1), Plus = SUM(Data, 10), Size = IF(Total >
    // 2, "big", "small"), Rounded = ABS(ROUND(-2.567, 1)), Sparse =
    // IF(A1 > 5, , 7).
    number(main, 2, 0, 3.0);
    number(main, 2, 1, 13.0);
    assert_eq!(wb.value(CellAddr::new(main, 2, 2)), Value::text("big"));
    number(main, 2, 3, 2.6);
    number(main, 2, 4, 7.0);
    // Row 4: Safe = IFERROR(1 / 0, -1), Half = QUOTIENT(7, 2),
    // SUM(Table) over {1,2;3,4}, Choice = CHOOSE(2, 10, 20).
    number(main, 3, 0, -1.0);
    number(main, 3, 1, 3.0);
    number(main, 3, 2, 10.0);
    number(main, 3, 3, 20.0);
    // The second sheet's own name: Top = MAX(C1:C3).
    number(other, 2, 0, 9.0);
}

/// An `.xls` formula names a name by its record, Tally's by its spelling
/// (a sheet's own name first, then the workbook's). Where the two would
/// differ the result is `#NAME?`, never another name's value: a sheet's
/// name that is not read does not become the workbook's namesake, a
/// workbook name on a sheet's name means that sheet's wherever it is used,
/// and a name on both namesakes is left out. And a formula whose
/// tokens carry no parentheses calculates as the token tree, not as its
/// text without them.
#[test]
fn xls_names_bind_to_their_record_and_keep_their_token_tree() {
    let mut svc = EngineService::with_default_codec();
    let info = open(&mut svc, &repo("tests/fixtures/xls/scopednames.xls"));
    let (main, other) = (info.sheets[0].id, info.sheets[1].id);
    let names = svc.names(info.doc).unwrap();
    let mut listed: Vec<(&str, Option<tally_types::SheetId>)> =
        names.iter().map(|n| (n.name.as_str(), n.scope)).collect();
    listed.sort_unstable();
    assert_eq!(
        listed,
        [
            ("Bar", None),
            ("Baz", Some(other)),
            ("Foo", None),
            // The sheet's Foo, kept without a value.
            ("Foo", Some(main)),
            ("Loc", None),
            ("Loc", Some(other)),
            ("Pa", None),
            ("Pb", None),
            ("Pc", None),
            ("Pd", None),
            ("Pe", None),
            ("Pf", None),
            ("Pg", None),
        ]
    );
    svc.recalc_all(info.doc).unwrap();
    let wb = &svc.document(info.doc).unwrap().wb;
    let value = |sheet, r: u32, c: u32| wb.value(CellAddr::new(sheet, r, c));
    let number = |sheet, r: u32, c: u32, want: f64| match value(sheet, r, c) {
        Value::Number(n) => assert!((n - want).abs() < 1e-9, "({r}, {c}): {n} != {want}"),
        other => panic!("({r}, {c}): {other:?}"),
    };
    let no_name = Value::Error(tally_types::ErrorKind::Name);
    // Main!B1 = the sheet's Foo (not read): not the workbook's 5.
    assert_eq!(value(main, 0, 1), no_name);
    // Main!C1 = Bar = the Loc of the other sheet + 1: not 101 with the
    // workbook's Loc.
    number(main, 0, 2, 8.0);
    // Main!E1 = Mix = that Loc + the workbook's Loc: left out.
    assert_eq!(value(main, 0, 4), no_name);
    // Main!D1 = the workbook's Loc.
    number(main, 0, 3, 100.0);
    // Other!B1 = Baz = that sheet's Loc + 1, C1 = that Loc, D1 = the
    // workbook's Foo (this sheet has none).
    number(other, 0, 1, 8.0);
    number(other, 0, 2, 7.0);
    number(other, 0, 3, 5.0);
    // Main!B2…H2: (1 + 2) * 3, (2 ^ 3) ^ 2, 1 - (2 - 3), -(2 ^ 2),
    // (1 + 2) %, 2 ^ (3 ^ 2), (-2) ^ 2.
    for (col, want) in [9.0, 64.0, 2.0, -4.0, 0.03, 512.0, 4.0]
        .into_iter()
        .enumerate()
    {
        number(main, 1, col as u32 + 1, want);
    }
}

/// Names that Excel 97-2003 allows and that are cells in Tally's larger
/// grid (`TAX2024`, `FYE2025`, `A70000`) are kept: the engine lists them
/// with a trailing `_` and resolves the file's spelling, so the formulas
/// and names that use them calculate (they must not read the cells of
/// those addresses). And a name with a relative reference gives each
/// cell that uses it its own result.
#[test]
fn xls_names_that_are_cells_in_the_larger_grid_and_relative_names() {
    let mut svc = EngineService::with_default_codec();
    let info = open(&mut svc, &repo("tests/fixtures/xls/widenames.xls"));
    let main = info.sheets[0].id;
    let names = svc.names(info.doc).unwrap();
    let mut listed: Vec<&str> = names.iter().map(|n| n.name.as_str()).collect();
    listed.sort_unstable();
    assert_eq!(
        listed,
        ["A70000_", "Above", "FYE2025_", "Gross", "TAX2024_"]
    );
    assert!(
        info.warnings
            .iter()
            .any(|w| w.contains("TAX2024 → TAX2024_") && w.contains("FYE2025 → FYE2025_")),
        "{:?}",
        info.warnings
    );
    // The stored results in the file are all 0: these are calculated.
    svc.recalc_all(info.doc).unwrap();
    let wb = &svc.document(info.doc).unwrap().wb;
    let value = |r: u32, c: u32| wb.value(CellAddr::new(main, r, c));
    let close = |v: Value, want: f64| match v {
        Value::Number(n) => assert!((n - want).abs() < 1e-9, "{n} != {want}"),
        other => panic!("{other:?}"),
    };
    // A3 = TAX2024 + 1, B3 = FYE2025 * 2, D3 = Gross (= FYE2025 *
    // TAX2024), E3 = A70000 + FYE2025.
    close(value(2, 0), 1.2);
    close(value(2, 1), 10.0);
    close(value(2, 3), 1.0);
    close(value(2, 4), 8.0);
    // "Above" from B2 is B1, from C3 it is C2.
    close(value(1, 1), 10.0);
    close(value(2, 2), 20.0);
    // A4 = IV65535 + A1: a reference at the edge of the old sheet.
    close(value(3, 0), 5.0);
}
