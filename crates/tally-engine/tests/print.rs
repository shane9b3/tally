//! Printing through the service (wave 5, W5-3): layout, breaks, jobs,
//! PDF bytes, PNG previews and a golden raster of the invoices template.
#![allow(clippy::unwrap_used)]
#![cfg(feature = "io")]

mod common;

use std::path::PathBuf;

use common::Fixture;
use tally_api::{
    ChartSvgDto, DocumentService, HeaderFooter, NoteDto, NotesPrint, Op, PageOrder, PageScale,
    PageSetupDto, PrintOptionsDto, PrintSheets, RangeDto,
};

fn opts() -> PrintOptionsDto {
    PrintOptionsDto {
        sheets: PrintSheets::Active,
        selection: None,
        suppress_empty_pages: true,
        page_from: None,
        page_to: None,
    }
}

fn golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/print/invoices_page1.png")
}

#[test]
fn invoices_template_prints_and_matches_golden() {
    let mut svc = tally_engine::EngineService::with_default_codec();
    let info = svc.new_workbook(Some("invoices")).unwrap();
    let layout = svc.print_layout(info.doc, &opts()).unwrap();
    assert!(!layout.pages.is_empty());
    assert_eq!(layout.pages[0].number, 1);
    assert_eq!(layout.pages[0].paper_mm, [210.0, 297.0]);
    let breaks = svc.page_breaks(info.doc, info.sheets[0].id).unwrap();
    assert!(breaks.area.is_some());
    let job = svc.prepare_print_job(info.doc, &opts(), &[]).unwrap();
    assert_eq!(job.layout.pages.len(), layout.pages.len());
    let pdf = svc.print_job_pdf(job.job).unwrap();
    assert!(pdf.starts_with(b"%PDF-"));
    assert!(pdf.len() > 2_000);
    let png_b64 = svc.print_job_page_png(job.job, 0, 36).unwrap();
    assert!(png_b64.starts_with("iVBOR"), "base64 PNG");
    let png = tally_engine::protection::base64_decode(&png_b64).unwrap();
    let (w, h, pixels) = tally_io_pdf::png_pixels(&png).unwrap();
    assert_eq!((w, h), (297, 420), "A4 at 36 dpi");
    // Something is drawn (not a blank page).
    let dark = pixels.chunks(4).filter(|p| p[0] < 128).count();
    assert!(dark > 200, "{dark} dark pixels");
    // Golden comparison with a tolerance; the golden is written on first
    // use so a deliberate rendering change is updated by deleting it.
    let path = golden_path();
    if let Ok(gold) = std::fs::read(&path) {
        let (gw, gh, gp) = tally_io_pdf::png_pixels(&gold).unwrap();
        assert_eq!((gw, gh), (w, h));
        let diff: u64 = gp
            .iter()
            .zip(&pixels)
            .map(|(a, b)| u64::from(a.abs_diff(*b)))
            .sum();
        let mean = diff as f64 / gp.len() as f64;
        assert!(
            mean < 6.0,
            "mean channel difference {mean:.2} against {}",
            path.display()
        );
    } else {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &png).unwrap();
    }
    // Unknown jobs and pages are errors.
    assert!(svc.print_job_pdf(999).is_err());
    assert!(svc.print_job_page_png(job.job, 99, 72).is_err());
}

#[test]
fn print_options_sheets_breaks_notes_and_charts() {
    let mut f = Fixture::new();
    let s = f.sheet;
    for r in 1..=120 {
        f.set(&format!("A{r}"), &r.to_string());
        f.set(&format!("B{r}"), &format!("=A{r}*2"));
    }
    // Page setup: landscape-ish fit to one page wide, header with placeholders.
    let mut setup: PageSetupDto = f.svc.page_setup(f.doc, s).unwrap();
    setup.scale = PageScale::FitTo {
        width: 1,
        height: 0,
    };
    setup.header = Some(HeaderFooter {
        left: "&[FILE]".into(),
        center: "&[SHEET]".into(),
        right: "Page &[PAGE] of &[PAGES]".into(),
    });
    setup.row_breaks = vec![20];
    setup.print_notes = Some(NotesPrint::AtEnd);
    setup.print_gridlines = true;
    setup.print_headers = true;
    setup.page_order = PageOrder::LeftToRight;
    f.apply(vec![Op::SetPageSetup { sheet: s, setup }]);
    f.apply(vec![Op::SetNote {
        sheet: s,
        r: 0,
        c: 0,
        note: NoteDto {
            r: 0,
            c: 0,
            text: "first row".into(),
            author: None,
            created: None,
            shown: false,
        },
    }]);
    let breaks = f.svc.page_breaks(f.doc, s).unwrap();
    assert!(
        breaks.row_breaks.iter().any(|b| b.at == 20 && b.manual),
        "{breaks:?}"
    );
    let layout = f.svc.print_layout(f.doc, &opts()).unwrap();
    assert!(layout.pages.len() >= 2);
    assert_eq!(layout.pages[0].rows, [0, 19]);
    let h = layout.pages[1].header.clone().unwrap();
    assert_eq!(h[1], "Sheet1");
    assert_eq!(h[2], format!("Page 2 of {}", layout.pages.len()));
    // Page range.
    let mut o = opts();
    o.page_from = Some(2);
    o.page_to = Some(2);
    let one = f.svc.print_layout(f.doc, &o).unwrap();
    assert_eq!(one.pages.len(), 1);
    assert_eq!(one.pages[0].number, 2);
    // A job with notes at the end gets an extra page in the PDF, charts
    // without SVG are reported.
    let job = f
        .svc
        .prepare_print_job(
            f.doc,
            &opts(),
            &[ChartSvgDto {
                sheet: s,
                id: 12345,
                svg: "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"10\" height=\"10\"/>"
                    .into(),
            }],
        )
        .unwrap();
    let pdf = f.svc.print_job_pdf(job.job).unwrap();
    assert!(pdf.starts_with(b"%PDF-"));
    // Selection on the active sheet.
    let mut o = opts();
    o.selection = Some(RangeDto {
        r0: 0,
        c0: 0,
        r1: 9,
        c1: 1,
    });
    let sel = f.svc.print_layout(f.doc, &o).unwrap();
    assert_eq!(sel.pages.len(), 1);
    assert_eq!(sel.pages[0].rows, [0, 9]);
    // All sheets.
    let mut o = opts();
    o.sheets = PrintSheets::All;
    assert!(f.svc.print_layout(f.doc, &o).unwrap().pages.len() >= layout.pages.len());
    o.sheets = PrintSheets::Some { ids: vec![4242] };
    assert!(f.svc.print_layout(f.doc, &o).is_err());
}
