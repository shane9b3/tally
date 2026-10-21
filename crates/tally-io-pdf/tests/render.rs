//! PDF rendering tests: page geometry, font subsetting, SVG embedding,
//! glyph fallback, rasterization and the time budget.
#![allow(clippy::unwrap_used)]

use std::time::Instant;

use hayro_syntax::Pdf;
use tally_io_pdf::{
    rasterize, render_pdf, FontSpec, HAlign, Item, LineStyle, PageJob, PdfOptions, PrintJob, Rect,
    VAlign,
};

const A4: (f32, f32) = (595.28, 841.89);

fn text(rect: Rect, s: &str) -> Item {
    Item::Text {
        rect,
        text: s.into(),
        font: FontSpec::default(),
        h_align: HAlign::Left,
        v_align: VAlign::Middle,
        wrap: false,
        overflow: (0.0, 0.0),
        clip: true,
    }
}

fn page(items: Vec<Item>) -> PageJob {
    PageJob {
        width_pt: A4.0,
        height_pt: A4.1,
        items,
    }
}

fn opts() -> PdfOptions {
    PdfOptions {
        system_fonts: false,
        svg_text: true,
    }
}

#[test]
fn page_count_and_mediabox() {
    let job = PrintJob {
        pages: vec![
            page(vec![text(Rect::new(50.0, 50.0, 200.0, 14.0), "Page one")]),
            page(vec![]),
            PageJob {
                width_pt: 841.89,
                height_pt: 595.28,
                items: vec![],
            },
        ],
        title: Some("Test".into()),
    };
    let r = render_pdf(&job, &opts()).unwrap();
    assert!(r.pdf.starts_with(b"%PDF-"));
    let pdf = Pdf::new(r.pdf.clone()).unwrap();
    let pages = pdf.pages();
    assert_eq!(pages.len(), 3);
    let mb = pages[0].media_box();
    assert!((mb.width() - 595.28).abs() < 0.05, "{mb:?}");
    assert!((mb.height() - 841.89).abs() < 0.05, "{mb:?}");
    assert!((pages[2].media_box().width() - 841.89).abs() < 0.05);
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
}

#[test]
fn fonts_subset_embedded() {
    let mut items = Vec::new();
    for (i, s) in ["Regular text", "Bold text", "Italic text"]
        .iter()
        .enumerate()
    {
        let mut font = FontSpec {
            size_pt: 12.0,
            ..FontSpec::default()
        };
        font.bold = i == 1;
        font.italic = i == 2;
        items.push(Item::Text {
            rect: Rect::new(40.0, 40.0 + 20.0 * i as f32, 300.0, 16.0),
            text: (*s).into(),
            font,
            h_align: HAlign::Left,
            v_align: VAlign::Middle,
            wrap: false,
            overflow: (0.0, 0.0),
            clip: true,
        });
    }
    let r = render_pdf(
        &PrintJob {
            pages: vec![page(items)],
            title: None,
        },
        &opts(),
    )
    .unwrap();
    let bytes = r.pdf;
    let s = String::from_utf8_lossy(&bytes);
    assert!(s.contains("/FontFile2"), "TrueType font programs embedded");
    assert!(s.contains("Inter"), "font name kept");
    // Three faces of 410–425 KB each; the subsets are a small fraction.
    assert!(bytes.len() < 120_000, "subsetted: {} bytes", bytes.len());
    let pdf = Pdf::new(bytes).unwrap();
    assert_eq!(pdf.pages().len(), 1);
}

#[test]
fn svg_chart_embedded_and_rasterized() {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100" viewBox="0 0 200 100">
        <rect x="0" y="0" width="200" height="100" fill="#ffffff"/>
        <rect x="20" y="10" width="60" height="80" fill="#ff0000"/>
        <path d="M100 90 L180 10" stroke="#0000ff" stroke-width="4"/>
        <text x="100" y="50" font-size="12">Series 1</text></svg>"##;
    let base = PrintJob {
        pages: vec![page(vec![text(
            Rect::new(50.0, 20.0, 200.0, 14.0),
            "Chart",
        )])],
        title: None,
    };
    let mut with = base.clone();
    with.pages[0].items.push(Item::Svg {
        rect: Rect::new(100.0, 100.0, 200.0, 100.0),
        svg: svg.into(),
    });
    let a = render_pdf(&base, &opts()).unwrap();
    let b = render_pdf(&with, &opts()).unwrap();
    assert!(b.pdf.len() > a.pdf.len());
    assert!(b.warnings.is_empty(), "{:?}", b.warnings);
    // The red bar of the chart lands at (120..180, 110..190) pt.
    let png = rasterize(&b.pdf, 0, 72).unwrap();
    let pix = hayro::vello_cpu::Pixmap::from_png(std::io::Cursor::new(&png)).unwrap();
    assert_eq!((pix.width(), pix.height()), (595, 841));
    let p = pix.sample(150, 150);
    assert!(p.r > 200 && p.g < 60 && p.b < 60, "red bar: {p:?}");
    let bg = pix.sample(400, 600);
    assert!(bg.r > 240 && bg.g > 240 && bg.b > 240, "white page: {bg:?}");
    assert!(rasterize(&b.pdf, 5, 72).is_err());
    // A broken SVG is reported, not fatal.
    let mut bad = base.clone();
    bad.pages[0].items.push(Item::Svg {
        rect: Rect::new(0.0, 0.0, 10.0, 10.0),
        svg: "<svg".into(),
    });
    let r = render_pdf(&bad, &opts()).unwrap();
    assert!(r.warnings.iter().any(|w| w.contains("chart image")));
}

#[test]
fn cjk_fallback_or_warning() {
    let job = PrintJob {
        pages: vec![page(vec![text(
            Rect::new(50.0, 50.0, 200.0, 14.0),
            "漢字 テスト",
        )])],
        title: None,
    };
    // Without system fonts Inter has no CJK glyphs: a warning, no failure.
    let r = render_pdf(&job, &opts()).unwrap();
    assert!(
        r.warnings.iter().any(|w| w.contains("U+")),
        "missing glyphs reported: {:?}",
        r.warnings
    );
    // With system fonts either a fallback face is found or it is reported.
    let r = render_pdf(&job, &PdfOptions::default()).unwrap();
    assert!(r.pdf.len() > 1000);
    let s = String::from_utf8_lossy(&r.pdf);
    assert!(s.contains("/FontFile2") || s.contains("/FontFile3"));
}

#[test]
fn unknown_family_falls_back_with_warning() {
    let job = PrintJob {
        pages: vec![page(vec![Item::Text {
            rect: Rect::new(50.0, 50.0, 200.0, 14.0),
            text: "x".into(),
            font: FontSpec {
                family: Some("No Such Font 123".into()),
                ..FontSpec::default()
            },
            h_align: HAlign::Left,
            v_align: VAlign::Top,
            wrap: false,
            overflow: (0.0, 0.0),
            clip: false,
        }])],
        title: None,
    };
    let r = render_pdf(&job, &PdfOptions::default()).unwrap();
    assert!(r.warnings.iter().any(|w| w.contains("No Such Font 123")));
}

#[test]
fn lines_fills_wrap_and_overflow_render() {
    let long = "This is a long text that overflows into the free space on the right of the cell";
    let items = vec![
        Item::Fill {
            rect: Rect::new(40.0, 40.0, 100.0, 20.0),
            color: 0x3366ccff,
        },
        Item::Line {
            from: (40.0, 60.0),
            to: (140.0, 60.0),
            width: 1.0,
            color: 0x000000ff,
            style: LineStyle::Dashed,
        },
        Item::Line {
            from: (40.0, 70.0),
            to: (140.0, 70.0),
            width: 2.0,
            color: 0x000000ff,
            style: LineStyle::Double,
        },
        Item::Line {
            from: (40.0, 80.0),
            to: (140.0, 80.0),
            width: 1.0,
            color: 0x000000ff,
            style: LineStyle::Dotted,
        },
        Item::Text {
            rect: Rect::new(40.0, 100.0, 100.0, 60.0),
            text: long.into(),
            font: FontSpec::default(),
            h_align: HAlign::Left,
            v_align: VAlign::Top,
            wrap: true,
            overflow: (0.0, 0.0),
            clip: true,
        },
        Item::Text {
            rect: Rect::new(40.0, 200.0, 60.0, 14.0),
            text: long.into(),
            font: FontSpec {
                underline: true,
                strike: true,
                ..FontSpec::default()
            },
            h_align: HAlign::Left,
            v_align: VAlign::Bottom,
            wrap: false,
            overflow: (0.0, 300.0),
            clip: true,
        },
    ];
    let r = render_pdf(
        &PrintJob {
            pages: vec![page(items)],
            title: None,
        },
        &opts(),
    )
    .unwrap();
    let png = rasterize(&r.pdf, 0, 72).unwrap();
    let pix = hayro::vello_cpu::Pixmap::from_png(std::io::Cursor::new(&png)).unwrap();
    let p = pix.sample(90, 50);
    assert!(p.b > 150 && p.r < 100, "blue fill: {p:?}");
    // Overflowing text paints beyond the 60 pt cell (dark pixels at x ≈ 200).
    let dark = (150..300).any(|x| (200..214).any(|y| pix.sample(x, y).r < 120));
    assert!(dark, "overflowed text visible");
}

#[test]
fn twenty_page_budget() {
    let mut pages = Vec::new();
    for p in 0..20 {
        let mut items = Vec::new();
        for r in 0..50 {
            for c in 0..8 {
                let rect = Rect::new(40.0 + 65.0 * c as f32, 40.0 + 14.0 * r as f32, 65.0, 14.0);
                items.push(Item::Text {
                    rect,
                    text: format!("{}", (p * 400 + r * 8 + c) as f64 * 1.25),
                    font: FontSpec {
                        bold: c == 0,
                        ..FontSpec::default()
                    },
                    h_align: HAlign::Right,
                    v_align: VAlign::Middle,
                    wrap: false,
                    overflow: (0.0, 0.0),
                    clip: true,
                });
            }
            items.push(Item::Line {
                from: (40.0, 54.0 + 14.0 * r as f32),
                to: (560.0, 54.0 + 14.0 * r as f32),
                width: 0.5,
                color: 0x999999ff,
                style: LineStyle::Solid,
            });
        }
        pages.push(page(items));
    }
    let job = PrintJob { pages, title: None };
    let t = Instant::now();
    let r = render_pdf(&job, &opts()).unwrap();
    let elapsed = t.elapsed();
    assert_eq!(Pdf::new(r.pdf.clone()).unwrap().pages().len(), 20);
    // The 1.0 s gate is for release builds (benches); debug builds are
    // allowed 15× that.
    let budget = if cfg!(debug_assertions) { 15.0 } else { 1.0 };
    assert!(
        elapsed.as_secs_f64() < budget,
        "20-page PDF took {elapsed:?} (budget {budget} s)"
    );
    assert!(r.pdf.len() < 2 * 1024 * 1024, "{} bytes", r.pdf.len());
}
