//! Review 5 regressions for charts with their own data read from a small
//! XLSX (HIGH-1, HIGH-2): one chart part with 32,000 literal category
//! slots and 255 series without data, shown by several drawing frames.
//! `charts()` must stay small (no data table in the list), the table
//! query must refuse the padded table, and an XLSX save must stay linear
//! with its literal XML capped.

#![allow(clippy::unwrap_used)]
#![cfg(feature = "io")]

use std::io::{Cursor, Write};

use tally_api::{DocumentService, FileFormat};
use tally_engine::EngineService;

const NS: &str = r#"xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;
const RELS_NS: &str = r#"xmlns="http://schemas.openxmlformats.org/package/2006/relationships""#;
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const XDR: &str = r#"xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;

fn rels(items: &[(&str, &str, &str)]) -> String {
    let mut s = format!(r#"<?xml version="1.0"?><Relationships {RELS_NS}>"#);
    for (id, t, target) in items {
        s.push_str(&format!(
            r#"<Relationship Id="{id}" Type="{R}/{t}" Target="{target}"/>"#
        ));
    }
    s.push_str("</Relationships>");
    s
}

/// The review's probe package, scaled to `anchors` frames.
fn package(anchors: usize) -> Vec<u8> {
    let pts: String = (0..8000)
        .map(|k| format!(r#"<c:pt idx="{}"><c:v>c</c:v></c:pt>"#, k * 4))
        .collect();
    let mut sers = format!(
        r#"<c:ser><c:idx val="0"/><c:order val="0"/><c:cat><c:strLit><c:ptCount val="32000"/>{pts}</c:strLit></c:cat><c:val><c:numLit/></c:val></c:ser>"#
    );
    for k in 1..255 {
        sers.push_str(&format!(
            r#"<c:ser><c:idx val="{k}"/><c:order val="{k}"/><c:val><c:numLit/></c:val></c:ser>"#
        ));
    }
    let cx = format!(
        r#"<?xml version="1.0"?><c:chartSpace {XDR}><c:chart><c:plotArea><c:barChart><c:barDir val="col"/>{sers}</c:barChart></c:plotArea></c:chart></c:chartSpace>"#
    );
    let frames: String = (0..anchors)
        .map(|i| {
            let r = i * 6;
            format!(
                r#"<xdr:twoCellAnchor><xdr:from><xdr:col>0</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>{r}</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from><xdr:to><xdr:col>3</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>{}</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:to><xdr:graphicFrame><xdr:nvGraphicFramePr><xdr:cNvPr id="{}" name="Chart {i}"/><xdr:cNvGraphicFramePr/></xdr:nvGraphicFramePr><xdr:xfrm/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart r:id="rIdC"/></a:graphicData></a:graphic></xdr:graphicFrame><xdr:clientData/></xdr:twoCellAnchor>"#,
                r + 5,
                i + 2
            )
        })
        .collect();
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default();
    let entries: Vec<(&str, String)> = vec![
        (
            "xl/workbook.xml",
            format!(
                r#"<workbook {NS}><sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets></workbook>"#
            ),
        ),
        (
            "xl/_rels/workbook.xml.rels",
            rels(&[("rId1", "worksheet", "worksheets/sheet1.xml")]),
        ),
        (
            "xl/worksheets/sheet1.xml",
            format!(r#"<worksheet {NS}><sheetData/><drawing r:id="rIdD"/></worksheet>"#),
        ),
        (
            "xl/worksheets/_rels/sheet1.xml.rels",
            rels(&[("rIdD", "drawing", "../drawings/drawing1.xml")]),
        ),
        (
            "xl/drawings/drawing1.xml",
            format!(r#"<?xml version="1.0"?><xdr:wsDr {XDR}>{frames}</xdr:wsDr>"#),
        ),
        (
            "xl/drawings/_rels/drawing1.xml.rels",
            rels(&[("rIdC", "chart", "../charts/chart1.xml")]),
        ),
        ("xl/charts/chart1.xml", cx),
    ];
    for (name, data) in entries {
        z.start_file(name, opts).unwrap();
        z.write_all(data.as_bytes()).unwrap();
    }
    z.finish().unwrap().into_inner()
}

#[test]
fn a_small_file_with_huge_padded_tables_stays_small() {
    let dir = std::env::temp_dir().join(format!("tally-review5-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("own.xlsx");
    std::fs::write(&path, package(3)).unwrap();
    let mut svc = EngineService::with_default_codec();
    let info = svc.open(&path).unwrap();
    let sheet = info.sheets[0].id;

    // HIGH-1: the list carries the flag only.
    let charts = svc.charts(info.doc, sheet).unwrap();
    assert_eq!(charts.len(), 3);
    assert!(charts.iter().all(|c| c.own_data && c.data_table.is_none()));
    let json = serde_json::to_string(&charts).unwrap();
    assert!(json.len() < 1024 * 1024, "{} bytes", json.len());
    // 32,000 rows × 256 columns is over the cap: refused, not built.
    let err = svc
        .chart_data_table(info.doc, sheet, charts[0].id)
        .unwrap_err();
    assert_eq!(err.code, "invalid_argument");

    // HIGH-2: the XLSX save is linear and its literal XML capped.
    let t = std::time::Instant::now();
    let out = dir.join("out.xlsx");
    svc.save_copy(info.doc, &out, FileFormat::Xlsx).unwrap();
    assert!(t.elapsed().as_secs() < 60, "{:?}", t.elapsed());
    let len = std::fs::metadata(&out).unwrap().len();
    assert!(len < 40 * 1024 * 1024, "{len} bytes");
    let _ = std::fs::remove_dir_all(&dir);
}
