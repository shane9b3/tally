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

/// The owner's report: a two-line centred header over a 10 mm top margin
/// was drawn from the page edge, clipped and over the first rows. Now the
/// header lines sit inside the page below the top margin, the body starts
/// after the header area and its spacing, and nothing is drawn between.
#[test]
fn two_line_header_is_inside_the_page_and_above_the_body() {
    let mut f = Fixture::new();
    let s = f.sheet;
    f.set("A1", "Unit base price $16,465.00+HST");
    f.set("A2", "Finish basement");
    let mut setup: PageSetupDto = f.svc.page_setup(f.doc, s).unwrap();
    setup.margins.top_mm = 10.0;
    setup.margins.header_mm = 7.5;
    setup.header = Some(HeaderFooter {
        left: String::new(),
        center: "The Redwood - 10/22/2026\nBlock 90 - North Side".into(),
        right: String::new(),
    });
    setup.footer = None;
    let area = setup.header_area.unwrap();
    assert!(area.on && area.autofit);
    assert_eq!(area.spacing_mm, 2.5);
    f.apply(vec![Op::SetPageSetup { sheet: s, setup }]);
    let layout = f.svc.print_layout(f.doc, &opts()).unwrap();
    let page = &layout.pages[0];
    // 10 mm + 2 lines of 4.94 mm + 2.5 mm spacing.
    assert!((page.origin_mm[1] - 22.38).abs() < 0.01, "{page:?}");
    let job = f.svc.prepare_print_job(f.doc, &opts(), &[]).unwrap();
    let pdf = f.svc.print_job_pdf(job.job).unwrap();
    assert!(pdf.starts_with(b"%PDF-"));
    // 72 dpi: one pixel per point.
    let png =
        tally_engine::protection::base64_decode(&f.svc.print_job_page_png(job.job, 0, 72).unwrap())
            .unwrap();
    let (w, _h, px) = tally_io_pdf::png_pixels(&png).unwrap();
    let w = w as usize;
    let dark_rows: Vec<usize> = px
        .chunks(4 * w)
        .enumerate()
        .filter(|(_, row)| row.chunks(4).any(|p| p[0] < 128))
        .map(|(y, _)| y)
        .collect();
    let pt = |mm: f32| mm * 72.0 / 25.4;
    let header_top = pt(10.0);
    let header_bottom = pt(10.0 + 2.0 * 4.94);
    let body_top = pt(22.38);
    let first = *dark_rows.first().unwrap() as f32;
    assert!(first >= header_top, "ink above the header area at {first}");
    // Both header lines are drawn (ink in each line box)…
    let ink = |a: f32, b: f32| {
        dark_rows
            .iter()
            .any(|y| (*y as f32) >= a && (*y as f32) < b)
    };
    assert!(ink(header_top, header_top + pt(4.94)), "line 1");
    assert!(ink(header_top + pt(4.94), header_bottom), "line 2");
    // …and nothing between the header and the body (the spacing).
    assert!(!ink(header_bottom + 0.5, body_top - 0.5), "{dark_rows:?}");
    // The first row is printed below.
    assert!(ink(body_top, body_top + pt(4.52)), "row 1");
}

#[test]
fn header_on_off_keeps_the_text() {
    let mut f = Fixture::new();
    let s = f.sheet;
    f.set("A1", "x");
    let mut setup: PageSetupDto = f.svc.page_setup(f.doc, s).unwrap();
    let mut area = setup.header_area.unwrap();
    area.on = false;
    setup.header_area = Some(area);
    f.apply(vec![Op::SetPageSetup { sheet: s, setup }]);
    let back = f.svc.page_setup(f.doc, s).unwrap();
    assert!(!back.header_area.unwrap().on);
    assert_eq!(back.header.as_ref().unwrap().center, "&[SHEET]");
    let layout = f.svc.print_layout(f.doc, &opts()).unwrap();
    assert!(layout.pages[0].header.is_none());
    assert_eq!(layout.pages[0].origin_mm[1], 20.0);
    // Switched on again: the text is back; "on" without a text gives an
    // empty header.
    let mut setup = back;
    let mut area = setup.header_area.unwrap();
    area.on = true;
    setup.header_area = Some(area);
    setup.footer = None;
    setup.footer_area = Some(area);
    f.apply(vec![Op::SetPageSetup { sheet: s, setup }]);
    let back = f.svc.page_setup(f.doc, s).unwrap();
    assert_eq!(back.header.unwrap().center, "&[SHEET]");
    assert_eq!(back.footer.unwrap(), HeaderFooter::default());
    // Older clients without the areas: on when a text is set.
    let mut setup = f.svc.page_setup(f.doc, s).unwrap();
    setup.header_area = None;
    setup.footer_area = None;
    setup.footer = None;
    f.apply(vec![Op::SetPageSetup { sheet: s, setup }]);
    let back = f.svc.page_setup(f.doc, s).unwrap();
    assert!(back.header_area.unwrap().on);
    assert!(!back.footer_area.unwrap().on);
}

/// A fixed-height header with more lines than fit draws only the lines
/// that fit, never over the body.
#[test]
fn fixed_height_header_leaves_off_the_lines_that_do_not_fit() {
    let mut f = Fixture::new();
    let s = f.sheet;
    f.set("A1", "Unit base price");
    let mut setup: PageSetupDto = f.svc.page_setup(f.doc, s).unwrap();
    setup.margins.top_mm = 10.0;
    setup.margins.header_mm = 7.5;
    setup.header = Some(HeaderFooter {
        left: String::new(),
        center: "one\ntwo\nthree".into(),
        right: String::new(),
    });
    setup.footer = None;
    let mut area = setup.header_area.unwrap();
    area.autofit = false;
    setup.header_area = Some(area);
    f.apply(vec![Op::SetPageSetup { sheet: s, setup }]);
    let job = f.svc.prepare_print_job(f.doc, &opts(), &[]).unwrap();
    assert!((job.layout.pages[0].origin_mm[1] - 17.5).abs() < 0.01);
    let png =
        tally_engine::protection::base64_decode(&f.svc.print_job_page_png(job.job, 0, 72).unwrap())
            .unwrap();
    let (w, _h, px) = tally_io_pdf::png_pixels(&png).unwrap();
    let dark_rows: Vec<f32> = px
        .chunks(4 * w as usize)
        .enumerate()
        .filter(|(_, row)| row.chunks(4).any(|p| p[0] < 128))
        .map(|(y, _)| y as f32)
        .collect();
    let pt = |mm: f32| mm * 72.0 / 25.4;
    let ink = |a: f32, b: f32| dark_rows.iter().any(|y| *y >= a && *y < b);
    assert!(ink(pt(10.0), pt(10.0 + 4.94)), "line 1");
    // Lines 2 and 3 would run from 14.94 mm into the body at 17.5 mm.
    assert!(!ink(pt(10.0 + 4.94) + 0.5, pt(17.5) - 0.5), "{dark_rows:?}");
}

/// Excel's margins of an opened XLSX survive a page setup edit that leaves
/// the margins alone (and its undo) and are saved back unchanged.
#[test]
fn xlsx_margins_survive_an_unrelated_page_setup_edit() {
    use tally_model::{HeaderFooterData, MarginsData, PageSetupData, SheetData, WorkbookData};
    let raw = MarginsData {
        top: 1905,
        bottom: 1905,
        left: 1778,
        right: 1778,
        header: 762,
        footer: 762,
    };
    let mut p = PageSetupData {
        header: Some(HeaderFooterData {
            center: "one\ntwo".into(),
            ..HeaderFooterData::default()
        }),
        footer: None,
        ..PageSetupData::default()
    };
    p.margins = raw;
    let wb = WorkbookData {
        sheets: vec![SheetData {
            name: "S".into(),
            page_setup: Some(p),
            ..SheetData::default()
        }],
        ..WorkbookData::default()
    };
    let dir = std::env::temp_dir().join(format!("tally-hf-xlsx-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("in.xlsx");
    tally_io_xlsx::write_xlsx(&wb, std::fs::File::create(&src).unwrap()).unwrap();
    // What Excel's import makes of the written file.
    let first =
        tally_io_xlsx::read_xlsx(std::fs::File::open(&src).unwrap(), &Default::default()).unwrap();
    let want = first.sheets[0].page_setup.clone().unwrap().excel_margins;
    assert!(want.is_some());
    let mut svc = tally_engine::EngineService::with_default_codec();
    let info = svc.open(&src).unwrap();
    let mut f = Fixture {
        svc,
        doc: info.doc,
        sheet: info.sheets[0].id,
    };
    let s = f.sheet;
    let mut setup = f.svc.page_setup(f.doc, s).unwrap();
    setup.orientation = tally_api::Orientation::Landscape;
    f.apply(vec![Op::SetPageSetup { sheet: s, setup }]);
    f.svc.undo(f.doc).unwrap();
    f.svc.redo(f.doc).unwrap();
    let out = dir.join("out.xlsx");
    f.svc
        .save_copy(f.doc, &out, tally_api::FileFormat::Xlsx)
        .unwrap();
    let back =
        tally_io_xlsx::read_xlsx(std::fs::File::open(&out).unwrap(), &Default::default()).unwrap();
    let bp = back.sheets[0].page_setup.clone().unwrap();
    assert_eq!(bp.excel_margins, want);
    assert_eq!(
        bp.margins,
        first.sheets[0].page_setup.as_ref().unwrap().margins
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Prints "Unit base price …" at 13 pt in A1 (the owner's North Side file:
/// row style `ro1`, 0.557 cm with `use-optimal-row-height="true"`) and
/// returns `(body top, ink top, ink bottom)` in pixels at 144 dpi, with
/// `px` as row 1's set height (`None`: an optimal row, as ODS import
/// leaves it) and with or without the owner's two-line header.
fn row1_ink(px: Option<f32>, header: bool) -> (f32, f32, f32) {
    let mut f = Fixture::new();
    let s = f.sheet;
    f.set("A1", "Unit base price $16,655.00+HST");
    f.apply(vec![Op::ApplyStyle {
        sheet: s,
        ranges: vec![common::range("A1")],
        patch: tally_api::StylePatch {
            font_size_pt: Some(13.0),
            ..Default::default()
        },
    }]);
    if let Some(px) = px {
        f.apply(vec![Op::SetRowHeight {
            sheet: s,
            start: 0,
            end: 0,
            px,
        }]);
    }
    let mut setup: PageSetupDto = f.svc.page_setup(f.doc, s).unwrap();
    setup.margins.top_mm = 10.0;
    setup.margins.header_mm = 7.5;
    setup.header = header.then(|| HeaderFooter {
        left: String::new(),
        center: "The Pine - 10/22/2026\nBlock 90 - North Side".into(),
        right: String::new(),
    });
    setup.header_area = setup.header_area.map(|mut a| {
        a.on = header;
        a
    });
    setup.footer = None;
    f.apply(vec![Op::SetPageSetup { sheet: s, setup }]);
    let layout = f.svc.print_layout(f.doc, &opts()).unwrap();
    let dpi = 144u32;
    let px_per_mm = dpi as f32 / 25.4;
    let body_top = layout.pages[0].origin_mm[1] * px_per_mm;
    let job = f.svc.prepare_print_job(f.doc, &opts(), &[]).unwrap();
    let png = tally_engine::protection::base64_decode(
        &f.svc.print_job_page_png(job.job, 0, dpi).unwrap(),
    )
    .unwrap();
    let (w, _h, px) = tally_io_pdf::png_pixels(&png).unwrap();
    let w = w as usize;
    // Ink of the body only (the header is above it).
    let dark: Vec<f32> = px
        .chunks(4 * w)
        .enumerate()
        .filter(|(y, row)| *y as f32 >= body_top && row.chunks(4).any(|p| p[0] < 160))
        .map(|(y, _)| y as f32)
        .collect();
    assert!(!dark.is_empty(), "row 1 printed");
    (body_top, dark[0], *dark.last().unwrap())
}

/// The owner's report: the top of row 1's 13 pt text was cut off in
/// print. An optimal row prints at the height its text needs with Tally's
/// fonts (AOO recomputes optimal rows with its own), the text sits a
/// descent above the cell bottom without extra leading, and so it prints
/// whole, with or without a header, as it does in a tall row. In AOO's
/// own 0.557 cm (a set height) it fits too.
#[test]
fn large_text_in_row_one_is_not_clipped() {
    // The ink of the same text in a row far taller than it needs.
    let (top, a, b) = row1_ink(Some(80.0), false);
    let full = b - a;
    assert!(a > top && full > 20.0, "{top} {a} {b}");
    let row_px = |hmm: f32| hmm / 100.0 * 144.0 / 25.4;
    // 13 pt × 1.21 + 0.41 mm.
    let optimal = row_px(596.0);
    for header in [false, true] {
        for (px, row) in [
            (None, optimal),
            // 0.557 cm in screen pixels (96 dpi).
            (Some(557.0 / 2540.0 * 96.0), row_px(557.0)),
        ] {
            let (top, a, b) = row1_ink(px, header);
            // The whole glyph height is printed (not clipped at the top)…
            assert!(
                (b - a - full).abs() <= 1.0,
                "{px:?} {header}: {a}..{b} vs {full}"
            );
            // …inside the row.
            assert!(
                a > top && b < top + row,
                "{px:?} {header}: {top} {a} {b} {row}"
            );
        }
    }
    // A row lower than the text (0.35 cm, 15 px set by hand): like AOO the
    // text keeps its baseline a descent above the bottom and only its top
    // is cut, at the row top.
    let (top, a, b) = row1_ink(Some(15.0), false);
    let row = row_px(15.0 * 2540.0 / 96.0);
    assert!(a >= top && a <= top + 1.0, "{top} {a}");
    assert!(b < top + row && top + row - b < 4.5, "{top} {b} {row}");
}

/// Printing computes optimal heights without touching the document: the
/// row keeps no set height (it is still written as an optimal row) and
/// the document stays clean.
#[test]
fn optimal_print_heights_leave_the_document_alone() {
    let mut f = Fixture::new();
    let s = f.sheet;
    f.set("A1", "Big");
    f.apply(vec![Op::ApplyStyle {
        sheet: s,
        ranges: vec![common::range("A1")],
        patch: tally_api::StylePatch {
            font_size_pt: Some(20.0),
            ..Default::default()
        },
    }]);
    let before = f.data();
    let job = f.svc.prepare_print_job(f.doc, &opts(), &[]).unwrap();
    let _ = f.svc.print_job_pdf(job.job).unwrap();
    let after = f.data();
    assert!(after.sheets[0].row_heights.is_empty());
    assert_eq!(before.sheets[0].row_heights, after.sheets[0].row_heights);
}

/// An ODS sheet whose column default styles carry a bordered, filled
/// table and whose rows stop at the content (AOO formats only the cells
/// the rows contain): it opens unformatted below the table and prints on
/// one page, where it used to ask for more than 10,000.
#[test]
fn column_default_styles_print_on_one_page() {
    use std::io::Write;
    let content = r##"<?xml version="1.0" encoding="UTF-8"?><office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:version="1.2"><office:automatic-styles><style:style style:name="box" style:family="table-cell"><style:table-cell-properties fo:background-color="#eeeeee" fo:border="0.002cm solid #808080"/></style:style></office:automatic-styles><office:body><office:spreadsheet><table:table table:name="Grand">
<table:table-column/><table:table-column table:number-columns-repeated="10" table:default-cell-style-name="box"/>
<table:table-row><table:table-cell office:value-type="string"><text:p>Foyer</text:p></table:table-cell><table:table-cell office:value-type="float" office:value="60"/><table:table-cell table:number-columns-repeated="9"/></table:table-row>
<table:table-row table:number-rows-repeated="20"><table:table-cell office:value-type="string"><text:p>Hall</text:p></table:table-cell><table:table-cell table:number-columns-repeated="10"/></table:table-row>
</table:table></office:spreadsheet></office:body></office:document-content>"##;
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let stored =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip.start_file("mimetype", stored).unwrap();
    zip.write_all(b"application/vnd.oasis.opendocument.spreadsheet")
        .unwrap();
    zip.start_file("content.xml", stored).unwrap();
    zip.write_all(content.as_bytes()).unwrap();
    let bytes = zip.finish().unwrap().into_inner();
    let dir = std::env::temp_dir().join(format!("tally-print-coldefault-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("grand.ods");
    std::fs::write(&path, bytes).unwrap();

    let mut svc = tally_engine::EngineService::with_default_codec();
    let info = svc.open(&path).unwrap();
    let sheet = info.sheets[0].id;
    let breaks = svc.page_breaks(info.doc, sheet).unwrap();
    let area = breaks.area.unwrap();
    assert_eq!((area.r1, area.c1), (20, 10), "{area:?}");
    let layout = svc.print_layout(info.doc, &opts()).unwrap();
    assert_eq!(layout.pages.len(), 1);
    let job = svc.prepare_print_job(info.doc, &opts(), &[]).unwrap();
    assert_eq!(job.layout.pages.len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}
