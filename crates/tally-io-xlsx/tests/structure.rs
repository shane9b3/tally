//! Structural checks of writer output by an independent parser: Python's
//! `zipfile` + `xml.etree` (skipped when `python3` is not installed), plus
//! the same package-consistency checks in Rust.

#![allow(clippy::unwrap_used)] // tests may unwrap (docs/contributing.md)

mod common;

use std::io::Read;
use std::process::Command;
use std::sync::Arc;

use common::to_bytes;
use tally_model::{
    CellContentData, CellData, CellRangeLocal, NameData, SheetData, StyleData, WorkbookData,
};
use tally_types::{DateEpoch, Value};

fn sample(epoch: DateEpoch) -> WorkbookData {
    let cell = |row, col, content, style| CellData {
        row,
        col,
        content,
        style,
    };
    let mut wb = WorkbookData {
        sheets: vec![
            SheetData {
                name: "Data".into(),
                cells: vec![
                    cell(0, 0, CellContentData::Value(Value::Number(1.0)), None),
                    cell(0, 1, CellContentData::Value(Value::Number(2.0)), None),
                    cell(
                        0,
                        2,
                        CellContentData::Value(Value::text("a<b>&\"c\"")),
                        None,
                    ),
                    cell(
                        1,
                        0,
                        CellContentData::Formula {
                            text: Arc::from("of:=SUM([.A1:.B1])"),
                            cached: Some(Value::Number(3.0)),
                        },
                        None,
                    ),
                    cell(
                        1,
                        1,
                        CellContentData::Formula {
                            text: Arc::from("of:=['My Sheet'.A1]*2"),
                            cached: Some(Value::Number(8.0)),
                        },
                        None,
                    ),
                    cell(
                        1,
                        2,
                        CellContentData::Formula {
                            text: Arc::from("of:=IFNA([.A1];{1;2|3;4})"),
                            cached: Some(Value::Number(1.0)),
                        },
                        None,
                    ),
                    cell(
                        2,
                        0,
                        CellContentData::Value(Value::Number(46314.0)),
                        Some(0),
                    ),
                ],
                merges: vec![CellRangeLocal {
                    r0: 4,
                    c0: 0,
                    r1: 4,
                    c1: 2,
                }],
                frozen: Some((1, 0)),
                ..SheetData::default()
            },
            SheetData {
                name: "My Sheet".into(),
                cells: vec![cell(0, 0, CellContentData::Value(Value::Number(4.0)), None)],
                hidden: true,
                ..SheetData::default()
            },
        ],
        styles: vec![StyleData {
            number_format: Some("YYYY-MM-DD".into()),
            ..StyleData::default()
        }],
        names: vec![NameData {
            base_cell: None,
            name: "Total".into(),
            sheet: None,
            formula: "of:=[$Data.$A$1:.$B$1]".into(),
        }],
        ..WorkbookData::default()
    };
    wb.settings.calc_settings.date_epoch = epoch;
    wb
}

const CHECK: &str = r#"
import sys, zipfile, xml.etree.ElementTree as ET
M = "{http://schemas.openxmlformats.org/spreadsheetml/2006/main}"
z = zipfile.ZipFile(sys.argv[1])
assert z.testzip() is None
names = set(z.namelist())
for p in names:
    if p.endswith(".xml") or p.endswith(".rels"):
        ET.fromstring(z.read(p))
ct = ET.fromstring(z.read("[Content_Types].xml"))
for o in ct:
    if o.tag.endswith("Override"):
        assert o.get("PartName").lstrip("/") in names, o.get("PartName")
wb = ET.fromstring(z.read("xl/workbook.xml"))
sheets = [(s.get("name"), s.get("state")) for s in wb.iter(M + "sheet")]
assert sheets == [("Data", None), ("My Sheet", "hidden")], sheets
pr = wb.find(M + "workbookPr")
print("date1904=%s" % (pr.get("date1904") if pr is not None else None))
dn = {d.get("name"): d.text for d in wb.iter(M + "definedName")}
assert dn.get("Total") == "Data!$A$1:$B$1", dn
s1 = ET.fromstring(z.read("xl/worksheets/sheet1.xml"))
f = {c.get("r"): c.find(M + "f").text for c in s1.iter(M + "c") if c.find(M + "f") is not None}
assert f["A2"] == "SUM(A1:B1)", f
assert f["B2"] == "'My Sheet'!A1*2", f
assert f["C2"] == "_xlfn.IFNA(A1,{1,2;3,4})", f
v = {c.get("r"): c.find(M + "v").text for c in s1.iter(M + "c") if c.find(M + "v") is not None}
print("A3=%s" % v["A3"])
assert [m.get("ref") for m in s1.iter(M + "mergeCell")] == ["A5:C5"]
pane = s1.find(M + "sheetViews/" + M + "sheetView/" + M + "pane")
assert pane.get("state") == "frozen" and pane.get("ySplit") == "1"
styles = ET.fromstring(z.read("xl/styles.xml"))
codes = [n.get("formatCode") for n in styles.iter(M + "numFmt")]
assert "yyyy-mm-dd" in codes, codes
# The shared-strings table holds the escaped text exactly once.
sst = ET.fromstring(z.read("xl/sharedStrings.xml"))
texts = ["".join(t.itertext()) for t in sst.iter(M + "si")]
assert 'a<b>&"c"' in texts, texts
print("OK")
"#;

fn python() -> Option<&'static str> {
    ["python3", "python"].into_iter().find(|p| {
        Command::new(p)
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
    })
}

#[test]
fn python_parses_writer_output() {
    let Some(py) = python() else {
        eprintln!("python3 not found; skipping the independent structural check");
        return;
    };
    for (epoch, flag, serial) in [
        (DateEpoch::D1899, "None", "46314"),
        (DateEpoch::D1904, "1", "46314"),
    ] {
        let bytes = to_bytes(&sample(epoch));
        let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
        let path = dir.join(format!("structure-{epoch:?}.xlsx"));
        std::fs::write(&path, &bytes).unwrap();
        let out = Command::new(py)
            .arg("-I")
            .arg("-c")
            .arg(CHECK)
            .arg(&path)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{epoch:?}\n{stdout}\n{stderr}");
        assert!(stdout.contains("OK"), "{stdout}");
        assert!(
            stdout.contains(&format!("date1904={flag}")),
            "{epoch:?}: {stdout}"
        );
        assert!(
            stdout.contains(&format!("A3={serial}")),
            "{epoch:?}: {stdout}"
        );
    }
}

#[test]
fn package_is_consistent() {
    let bytes = to_bytes(&sample(DateEpoch::D1899));
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let names: Vec<String> = z.file_names().map(|n| n.unwrap().into_owned()).collect();
    for required in [
        "[Content_Types].xml",
        "_rels/.rels",
        "xl/workbook.xml",
        "xl/_rels/workbook.xml.rels",
        "xl/styles.xml",
        "xl/worksheets/sheet1.xml",
        "xl/worksheets/sheet2.xml",
        "xl/sharedStrings.xml",
    ] {
        assert!(
            names.iter().any(|n| n == required),
            "{required} in {names:?}"
        );
    }
    let mut ct = String::new();
    z.by_name("[Content_Types].xml")
        .unwrap()
        .read_to_string(&mut ct)
        .unwrap();
    assert!(ct.contains("spreadsheetml.sheet.main+xml"));
    // No absolute or parent paths in our own output.
    assert!(names
        .iter()
        .all(|n| !n.starts_with('/') && !n.contains("..")));
}
