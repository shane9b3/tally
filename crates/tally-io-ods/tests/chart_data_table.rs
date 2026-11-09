//! Charts with their own data (T18, the Chart Data Table): AOO 4.1 writes
//! them with `xlink:href="."` and series addresses into the chart's
//! `local-table` (`InternalDataProvider::convertRangeToXML`); Tally reads
//! those addresses back into literal data and writes edited own data the
//! same way, X values and bubble sizes included.

#![allow(clippy::unwrap_used)] // tests may unwrap (docs/contributing.md)

use std::fmt::Write as _;
use std::io::{Cursor, Write};

use tally_io_ods::{read_ods, write_ods, Limits};
use tally_model::{
    ChartAnchorData, ChartData, ChartKindData, ChartSeriesData, ChartValuesData, SeriesNameData,
    SheetData, WorkbookData,
};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

fn lit(v: &[Option<f64>]) -> Option<ChartValuesData> {
    Some(ChartValuesData::Literal(v.to_vec()))
}

fn text(v: &[&str]) -> Option<ChartValuesData> {
    Some(ChartValuesData::LiteralText(
        v.iter().map(|s| (*s).to_string()).collect(),
    ))
}

fn own(
    kind: ChartKindData,
    series: Vec<ChartSeriesData>,
    cats: Option<ChartValuesData>,
) -> ChartData {
    ChartData {
        name: "Object 1".into(),
        anchor: ChartAnchorData {
            row: 1,
            col: 1,
            dx_hmm: 0,
            dy_hmm: 0,
            width_hmm: 16_000,
            height_hmm: 9_000,
        },
        kind,
        literal_categories: cats,
        series,
        ..ChartData::default()
    }
}

fn named(name: &str) -> Option<SeriesNameData> {
    Some(SeriesNameData::Text(name.into()))
}

fn round_trip(c: ChartData) -> ChartData {
    let wb = WorkbookData {
        sheets: vec![SheetData {
            name: "Sheet1".into(),
            charts: vec![c],
            ..SheetData::default()
        }],
        ..WorkbookData::default()
    };
    let mut buf = Cursor::new(Vec::new());
    write_ods(&wb, &mut buf).unwrap();
    let bytes = buf.into_inner();
    // The object is written the AOO way.
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes.clone())).unwrap();
    let mut xml = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("Object 1/content.xml").unwrap(), &mut xml)
        .unwrap();
    assert!(xml.contains(r#"xlink:href=".""#), "{xml}");
    assert!(xml.contains("local-table.$"), "{xml}");
    let back = read_ods(Cursor::new(bytes), &Limits::default()).unwrap();
    assert_eq!(back.sheets[0].charts.len(), 1, "{:?}", back.warnings);
    back.sheets[0].charts[0].clone()
}

#[test]
fn edited_own_data_round_trips() {
    let c = own(
        ChartKindData::Column,
        vec![
            ChartSeriesData {
                name: named("North"),
                literal_values: lit(&[Some(1.0), None, Some(3.5), Some(-2.0)]),
                ..ChartSeriesData::default()
            },
            ChartSeriesData {
                literal_values: lit(&[Some(10.0), Some(20.0), Some(30.0), None]),
                color: Some(0x112233ff),
                ..ChartSeriesData::default()
            },
        ],
        text(&["Q1", "", "Q3 & <Q4>", "Q4"]),
    );
    let got = round_trip(c.clone());
    assert_eq!(got, c);
    assert!(got.has_own_data());
}

#[test]
fn scatter_and_bubble_keep_x_values_and_sizes() {
    let scatter = own(
        ChartKindData::Scatter,
        vec![ChartSeriesData {
            name: named("Fit"),
            literal_values: lit(&[Some(2.0), Some(4.0), Some(8.0)]),
            literal_x: lit(&[Some(1.0), Some(2.0), Some(3.0)]),
            ..ChartSeriesData::default()
        }],
        None,
    );
    let mut s = scatter.clone();
    s.markers = true;
    assert_eq!(round_trip(s.clone()), s);
    let bubble = own(
        ChartKindData::Bubble,
        vec![ChartSeriesData {
            name: named("B"),
            literal_values: lit(&[Some(5.0), Some(6.0)]),
            literal_x: lit(&[Some(1.0), Some(2.0)]),
            literal_sizes: lit(&[Some(30.0), Some(40.0)]),
            ..ChartSeriesData::default()
        }],
        None,
    );
    let got = round_trip(bubble.clone());
    assert_eq!(got.series, bubble.series);
}

const NS: &str = r#"xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:xlink="http://www.w3.org/1999/xlink" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0" xmlns:chart="urn:oasis:names:tc:opendocument:xmlns:chart:1.0""#;

/// An ODS with one chart object `object` (its `content.xml`).
fn package(object: &str) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    zip.start_file("mimetype", stored).unwrap();
    zip.write_all(tally_io_ods::MIMETYPE.as_bytes()).unwrap();
    let mut man = String::from(
        r#"<?xml version="1.0"?><manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.2"><manifest:file-entry manifest:full-path="/" manifest:media-type="application/vnd.oasis.opendocument.spreadsheet"/><manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/>"#,
    );
    let _ = write!(
        man,
        r#"<manifest:file-entry manifest:full-path="Object 1/" manifest:media-type="application/vnd.oasis.opendocument.chart"/><manifest:file-entry manifest:full-path="Object 1/content.xml" manifest:media-type="text/xml"/></manifest:manifest>"#
    );
    zip.start_file("META-INF/manifest.xml", deflated).unwrap();
    zip.write_all(man.as_bytes()).unwrap();
    zip.start_file("content.xml", deflated).unwrap();
    write!(
        zip,
        r#"<?xml version="1.0" encoding="UTF-8"?><office:document-content {NS} office:version="1.2"><office:body><office:spreadsheet><table:table table:name="Sheet1"><table:shapes><draw:frame draw:name="Object 1" svg:width="8cm" svg:height="5cm" svg:x="1cm" svg:y="1cm"><draw:object xlink:href="./Object 1"/></draw:frame></table:shapes><table:table-row><table:table-cell/></table:table-row></table:table></office:spreadsheet></office:body></office:document-content>"#
    )
    .unwrap();
    zip.start_file("Object 1/content.xml", deflated).unwrap();
    zip.write_all(object.as_bytes()).unwrap();
    zip.finish().unwrap().into_inner()
}

fn f(v: f64) -> String {
    format!(
        r#"<table:table-cell office:value-type="float" office:value="{v}"><text:p>{v}</text:p></table:table-cell>"#
    )
}

fn s(t: &str) -> String {
    format!(
        r#"<table:table-cell office:value-type="string"><text:p>{t}</text:p></table:table-cell>"#
    )
}

/// The shape SchXMLExport writes for an XY chart with internal data
/// (columns: X values, Y of series 1, Y of series 2 — the series in a
/// different order than the columns).
#[test]
fn aoo_internal_data_addresses_are_read_from_the_local_table() {
    let mut rows = String::new();
    for (x, a, b) in [(1.0, 10.0, 7.0), (2.0, 20.0, 8.0), (3.0, 30.0, 9.0)] {
        let _ = write!(
            rows,
            "<table:table-row>{}{}{}</table:table-row>",
            f(x),
            f(a),
            f(b)
        );
    }
    let object = format!(
        r#"<?xml version="1.0"?><office:document-content {NS} office:version="1.2"><office:body><office:chart><chart:chart chart:class="chart:scatter" xlink:href="." xlink:type="simple"><chart:plot-area table:cell-range-address="local-table.$A$1:.$C$4" chart:data-source-has-labels="both"><chart:axis chart:dimension="x" chart:name="primary-x"/><chart:axis chart:dimension="y" chart:name="primary-y"/><chart:series chart:values-cell-range-address="local-table.$C$2:.$C$4" chart:label-cell-address="local-table.$C$1"><chart:domain table:cell-range-address="local-table.$A$2:.$A$4"/></chart:series><chart:series chart:values-cell-range-address="local-table.$B$2:.$B$4" chart:label-cell-address="local-table.$B$1"><chart:domain table:cell-range-address="local-table.$A$2:.$A$4"/></chart:series></chart:plot-area><table:table table:name="local-table"><table:table-header-columns><table:table-column/></table:table-header-columns><table:table-columns><table:table-column table:number-columns-repeated="2"/></table:table-columns><table:table-header-rows><table:table-row>{}{}{}</table:table-row></table:table-header-rows><table:table-rows>{rows}</table:table-rows></table:table></chart:chart></office:chart></office:body></office:document-content>"#,
        s(""),
        s("Up"),
        s("Down"),
    );
    let wb = read_ods(Cursor::new(package(&object)), &Limits::default()).unwrap();
    let c = &wb.sheets[0].charts[0];
    assert!(c.has_own_data(), "{c:?}");
    assert!(c.source.is_none());
    assert_eq!(c.series.len(), 2);
    assert_eq!(c.series[0].name, named("Down"));
    assert_eq!(
        c.series[0].literal_values,
        lit(&[Some(7.0), Some(8.0), Some(9.0)])
    );
    assert_eq!(
        c.series[0].literal_x,
        lit(&[Some(1.0), Some(2.0), Some(3.0)])
    );
    assert_eq!(c.series[1].name, named("Up"));
    assert_eq!(
        c.series[1].literal_values,
        lit(&[Some(10.0), Some(20.0), Some(30.0)])
    );
}

/// A category chart: the categories come from `chart:categories` in the
/// local table, empty trailing rows keep the column length.
#[test]
fn aoo_internal_categories_are_read() {
    let rows = format!(
        "<table:table-row>{}{}</table:table-row><table:table-row>{}{}</table:table-row><table:table-row>{}<table:table-cell/></table:table-row>",
        s("Jan"),
        f(4.0),
        s("Feb"),
        f(5.0),
        s("Mar"),
    );
    let object = format!(
        r#"<?xml version="1.0"?><office:document-content {NS} office:version="1.2"><office:body><office:chart><chart:chart chart:class="chart:bar" xlink:href="."><chart:plot-area table:cell-range-address="local-table.$A$1:.$B$4"><chart:axis chart:dimension="x" chart:name="primary-x"><chart:categories table:cell-range-address="local-table.$A$2:.$A$4"/></chart:axis><chart:axis chart:dimension="y" chart:name="primary-y"/><chart:series chart:values-cell-range-address="local-table.$B$2:.$B$4" chart:label-cell-address="local-table.$B$1"/></chart:plot-area><table:table table:name="local-table"><table:table-header-rows><table:table-row>{}{}</table:table-row></table:table-header-rows><table:table-rows>{rows}</table:table-rows></table:table></chart:chart></office:chart></office:body></office:document-content>"#,
        s(""),
        s("Sales"),
    );
    let wb = read_ods(Cursor::new(package(&object)), &Limits::default()).unwrap();
    let c = &wb.sheets[0].charts[0];
    assert_eq!(c.literal_categories, text(&["Jan", "Feb", "Mar"]));
    assert_eq!(
        c.series[0].literal_values,
        lit(&[Some(4.0), Some(5.0), None])
    );
    assert_eq!(c.series[0].name, named("Sales"));
}
