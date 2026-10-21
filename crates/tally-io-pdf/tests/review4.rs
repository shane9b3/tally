//! Security review 4: chart SVGs cannot read local files (H15) and
//! rasterizing has a pixel budget (M4).
#![allow(clippy::unwrap_used)]

use std::time::{Duration, Instant};

use tally_io_pdf::{rasterize, render_pdf, Item, PageJob, PdfOptions, PrintJob, Rect};

fn opts() -> PdfOptions {
    PdfOptions {
        system_fonts: false,
        ..PdfOptions::default()
    }
}

fn job_with_svg(svg: &str, w: f32, h: f32) -> PrintJob {
    PrintJob {
        pages: vec![PageJob {
            width_pt: w,
            height_pt: h,
            items: vec![Item::Svg {
                rect: Rect::new(10.0, 10.0, 100.0, 100.0),
                svg: svg.into(),
            }],
        }],
        title: None,
    }
}

/// A 1×1 PNG.
const PNG_1X1: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0xDA, 0x63, 0xFC, 0xCF, 0xC0, 0xF0,
    0x1F, 0x00, 0x05, 0x05, 0x02, 0x00, 0x5F, 0xC8, 0xF1, 0xD2, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45,
    0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
];

fn image_svg(href: &str) -> String {
    format!(
        "<svg xmlns='http://www.w3.org/2000/svg' xmlns:xlink='http://www.w3.org/1999/xlink' \
         width='100' height='100'><image x='0' y='0' width='50' height='50' \
         xlink:href='{href}'/></svg>"
    )
}

fn has_image(pdf: &[u8]) -> bool {
    pdf.windows(13)
        .any(|w| w == b"/Subtype /Ima" || w == b"/Subtype/Imag")
}

#[test]
fn local_file_images_are_not_read() {
    let dir = std::env::temp_dir().join(format!("tally-r4-pdf-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let local = dir.join("local.png");
    std::fs::write(&local, PNG_1X1).unwrap();
    let t = Instant::now();
    let abs = render_pdf(
        &job_with_svg(&image_svg(local.to_str().unwrap()), 300.0, 300.0),
        &opts(),
    )
    .unwrap();
    assert!(!has_image(&abs.pdf), "an absolute path was embedded");
    let zero = render_pdf(
        &job_with_svg(&image_svg("/dev/zero"), 300.0, 300.0),
        &opts(),
    )
    .unwrap();
    assert!(!has_image(&zero.pdf));
    #[cfg(unix)]
    {
        let fifo = dir.join("fifo.png");
        let _ = std::fs::remove_file(&fifo);
        if std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .is_ok_and(|s| s.success())
        {
            let r = render_pdf(
                &job_with_svg(&image_svg(fifo.to_str().unwrap()), 300.0, 300.0),
                &opts(),
            )
            .unwrap();
            assert!(!has_image(&r.pdf));
        }
    }
    assert!(t.elapsed() < Duration::from_secs(10), "{:?}", t.elapsed());
    // A data: image is still drawn.
    let b64 = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFBQIAX8jx0gAAAABJRU5ErkJggg==";
    let data = render_pdf(
        &job_with_svg(
            &image_svg(&format!("data:image/png;base64,{b64}")),
            300.0,
            300.0,
        ),
        &opts(),
    )
    .unwrap();
    assert!(has_image(&data.pdf), "data: images are kept");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn rasterize_has_a_pixel_budget() {
    // A 14,400 pt (5 m) page: 867 MB at 72 dpi before, > 3 GB at 150 dpi.
    for dpi in [72, 150, 600] {
        let s = tally_io_pdf::raster_scale(14_400.0, 14_400.0, dpi);
        let px = (14_400.0 * s) * (14_400.0 * s);
        assert!(px <= 40_000_100.0, "{dpi}: {px}");
    }
    // A4 at 600 dpi is under the budget and keeps its resolution.
    assert_eq!(tally_io_pdf::raster_scale(595.0, 842.0, 600), 600.0 / 72.0);
    let job = job_with_svg(
        "<svg xmlns='http://www.w3.org/2000/svg' width='1' height='1'/>",
        2000.0,
        2000.0,
    );
    let pdf = render_pdf(&job, &opts()).unwrap().pdf;
    let png = rasterize(&pdf, 0, 600).unwrap();
    let (w, h, _) = tally_io_pdf::png_pixels(&png).unwrap();
    assert!(u64::from(w) * u64::from(h) <= 40_100_000, "{w}x{h}");
    assert!(w > 5000, "{w}");
}
