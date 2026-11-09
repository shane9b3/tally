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

fn write_report(sheets: Vec<SheetData>) -> (Vec<u8>, Vec<String>) {
    let wb = WorkbookData {
        sheets,
        ..WorkbookData::default()
    };
    let mut buf = Cursor::new(Vec::new());
    let report = write_xlsx(&wb, &mut buf).unwrap();
    (buf.into_inner(), report.warnings)
}

/// Review 5 (LOW-2): characters XML 1.0 cannot hold are written as
/// `_xHHHH_` (a literal `_xHHHH_` gets `_x005F_`), and read back.
#[test]
fn control_characters_in_literals_are_escaped() {
    let cats = vec![
        "a\u{1}b".to_string(),
        "_x0041_".to_string(),
        "t\tab & <x>".into(),
    ];
    let c = own(
        ChartKindData::Column,
        vec![ChartSeriesData {
            literal_values: lit(&[Some(1.0), Some(2.0), Some(3.0)]),
            ..ChartSeriesData::default()
        }],
        Some(ChartValuesData::LiteralText(cats.clone())),
    );
    let bytes = write(c);
    let xml = chart_xml(&bytes);
    assert!(!xml.contains('\u{1}'));
    assert!(xml.contains("<c:v>a_x0001_b</c:v>"), "{xml}");
    assert!(xml.contains("<c:v>_x005F_x0041_</c:v>"), "{xml}");
    assert!(xml.contains("<c:v>t\tab &amp; &lt;x&gt;</c:v>"), "{xml}");
    let got = read_back(bytes);
    assert_eq!(
        got.literal_categories,
        Some(ChartValuesData::LiteralText(cats))
    );
}

/// Review 5 (L4): values are always `c:numLit` (`CT_NumDataSource`), even
/// when the file held them as text; text that is no number is left out.
#[test]
fn text_values_are_written_as_numbers() {
    let c = own(
        ChartKindData::Column,
        vec![ChartSeriesData {
            literal_values: Some(ChartValuesData::LiteralText(vec![
                "1.5".into(),
                "x".into(),
                " 3 ".into(),
            ])),
            ..ChartSeriesData::default()
        }],
        None,
    );
    let bytes = write(c);
    let xml = chart_xml(&bytes);
    let val = &xml[xml.find("<c:val>").unwrap()..xml.find("</c:val>").unwrap()];
    assert!(
        val.contains("<c:numLit>") && !val.contains("strLit"),
        "{val}"
    );
    assert!(val.contains(r#"<c:ptCount val="3"/>"#), "{val}");
    let got = read_back(bytes);
    assert_eq!(
        got.series[0].literal_values,
        lit(&[Some(1.5), None, Some(3.0)])
    );
}

/// Review 5 (HIGH-2): many series naming large literal categories save in
/// linear time; the category list is rendered once per chart, the save's
/// literal XML is capped, and the cut is reported.
#[test]
fn large_literal_categories_save_within_the_budget() {
    let cats: Vec<String> = (0..32_000)
        .map(|i| format!("category label {i:06}"))
        .collect();
    let chart = |n: u32| {
        let mut c = own(
            ChartKindData::Column,
            (0..100)
                .map(|_| ChartSeriesData {
                    literal_values: Some(ChartValuesData::Literal(Vec::new())),
                    ..ChartSeriesData::default()
                })
                .collect(),
            Some(ChartValuesData::LiteralText(cats.clone())),
        );
        c.name = format!("Chart {n}");
        c.anchor.row = n * 20;
        c
    };
    let t = std::time::Instant::now();
    let (bytes, warnings) = write_report(vec![SheetData {
        name: "Sheet1".into(),
        charts: vec![chart(1), chart(2), chart(3)],
        ..SheetData::default()
    }]);
    assert!(t.elapsed().as_secs() < 60, "{:?}", t.elapsed());
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("own data was not written")),
        "{warnings:?}"
    );
    // Every part together holds at most the budget of literal XML.
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut total = 0usize;
    let mut cat_lists = 0usize;
    let names: Vec<String> = zip.file_names().map(|n| n.unwrap().to_string()).collect();
    for name in names {
        if name.starts_with("xl/charts/chart") {
            let mut xml = String::new();
            zip.by_name(&name)
                .unwrap()
                .read_to_string(&mut xml)
                .unwrap();
            assert!(!xml.contains("_TallyLit["), "every placeholder is replaced");
            total += xml.len();
            cat_lists += xml.matches("<c:cat><c:strLit>").count();
        }
    }
    assert!(total < 34 * 1024 * 1024, "{total}");
    assert!(cat_lists > 0 && cat_lists < 300, "{cat_lists}");
}
