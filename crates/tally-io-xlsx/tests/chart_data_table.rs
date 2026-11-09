//! Charts with their own data (T18, the Chart Data Table) in XLSX: the
//! literal lists are written as `c:numLit` / `c:strLit` (Excel's static
//! chart data; rust_xlsxwriter writes only references, so a patch pass
//! replaces placeholder references) and read back as literal data.

#![allow(clippy::unwrap_used)] // tests may unwrap (docs/contributing.md)

use std::io::{Cursor, Read};

use tally_io_xlsx::{read_xlsx, write_xlsx, Limits};
use tally_model::{
    ChartAnchorData, ChartData, ChartKindData, ChartSeriesData, ChartValuesData, SeriesNameData,
    SheetData, WorkbookData,
};

fn lit(v: &[Option<f64>]) -> Option<ChartValuesData> {
    Some(ChartValuesData::Literal(v.to_vec()))
}

fn own(
    kind: ChartKindData,
    series: Vec<ChartSeriesData>,
    cats: Option<ChartValuesData>,
) -> ChartData {
    ChartData {
        name: "Chart 1".into(),
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

fn write(c: ChartData) -> Vec<u8> {
    let wb = WorkbookData {
        sheets: vec![SheetData {
            name: "Sheet1".into(),
            charts: vec![c],
            ..SheetData::default()
        }],
        ..WorkbookData::default()
    };
    let mut buf = Cursor::new(Vec::new());
    write_xlsx(&wb, &mut buf).unwrap();
    buf.into_inner()
}

fn chart_xml(bytes: &[u8]) -> String {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes.to_vec())).unwrap();
    let mut xml = String::new();
    zip.by_name("xl/charts/chart1.xml")
        .unwrap()
        .read_to_string(&mut xml)
        .unwrap();
    xml
}

fn read_back(bytes: Vec<u8>) -> ChartData {
    let wb = read_xlsx(Cursor::new(bytes), &Limits::default()).unwrap();
    assert_eq!(wb.sheets[0].charts.len(), 1, "{:?}", wb.warnings);
    wb.sheets[0].charts[0].clone()
}

#[test]
fn edited_own_data_is_written_as_literals() {
    let c = own(
        ChartKindData::Column,
        vec![
            ChartSeriesData {
                name: Some(SeriesNameData::Text("North".into())),
                literal_values: lit(&[Some(1.0), None, Some(3.5)]),
                ..ChartSeriesData::default()
            },
            ChartSeriesData {
                name: Some(SeriesNameData::Text("South".into())),
                literal_values: lit(&[Some(10.0), Some(20.0), Some(-30.25)]),
                ..ChartSeriesData::default()
            },
        ],
        Some(ChartValuesData::LiteralText(vec![
            "Q1".into(),
            "Q2 & <more>".into(),
            "Q3".into(),
        ])),
    );
    let bytes = write(c.clone());
    let xml = chart_xml(&bytes);
    assert!(xml.contains("<c:numLit>"), "{xml}");
    assert!(xml.contains("<c:strLit>"), "{xml}");
    assert!(!xml.contains("_TallyLit"), "{xml}");
    let got = read_back(bytes);
    assert!(got.has_own_data());
    assert_eq!(got.literal_categories, c.literal_categories);
    assert_eq!(got.series.len(), 2);
    for (g, w) in got.series.iter().zip(&c.series) {
        assert_eq!(g.name, w.name);
        assert_eq!(g.literal_values, w.literal_values);
    }
}

#[test]
fn scatter_own_data_keeps_x_values() {
    let c = own(
        ChartKindData::Scatter,
        vec![ChartSeriesData {
            name: Some(SeriesNameData::Text("Fit".into())),
            literal_values: lit(&[Some(2.0), Some(4.0), Some(8.0)]),
            literal_x: lit(&[Some(1.0), Some(2.0), Some(3.0)]),
            ..ChartSeriesData::default()
        }],
        None,
    );
    let got = read_back(write(c.clone()));
    assert_eq!(got.series[0].literal_values, c.series[0].literal_values);
    assert_eq!(got.series[0].literal_x, c.series[0].literal_x);
}
