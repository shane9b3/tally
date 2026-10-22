//! Document fonts: resolution by family name against injected font
//! directories (never the machine's fonts, except the conditional Arial
//! test), bold/italic faces, collections, substitutes, fallback to Inter,
//! embedding, malformed files and limits.
#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};

use hayro_syntax::Pdf;
use tally_io_pdf::{
    measure_text_with, rasterize, render_pdf, resolve_font, resolve_font_with, FontContext,
    FontSpec, HAlign, Item, PageJob, PdfOptions, PrintJob, Rect, VAlign,
};

static INTER_REGULAR: &[u8] = include_bytes!("../fonts/Inter-Regular.ttf");
static INTER_BOLD: &[u8] = include_bytes!("../fonts/Inter-Bold.ttf");

// ---------------------------------------------------------------------------
// Test fonts: Inter with its naming table replaced (and, optionally, its
// ascent changed so the metrics are recognisable).
// ---------------------------------------------------------------------------

type Tables = Vec<([u8; 4], Vec<u8>)>;

fn be16(d: &[u8], i: usize) -> usize {
    usize::from(u16::from_be_bytes([d[i], d[i + 1]]))
}

fn be32(d: &[u8], i: usize) -> usize {
    u32::from_be_bytes([d[i], d[i + 1], d[i + 2], d[i + 3]]) as usize
}

fn tables_of(font: &[u8]) -> Tables {
    let n = be16(font, 4);
    (0..n)
        .map(|k| {
            let r = 12 + 16 * k;
            let tag = [font[r], font[r + 1], font[r + 2], font[r + 3]];
            let (off, len) = (be32(font, r + 8), be32(font, r + 12));
            (tag, font[off..off + len].to_vec())
        })
        .collect()
}

fn name_table(family: &str, sub: &str) -> Vec<u8> {
    let ps: String = format!("{family}-{sub}").replace(' ', "");
    let full = format!("{family} {sub}");
    let recs: Vec<(u16, String)> = vec![
        (1, family.into()),
        (2, sub.into()),
        (4, full),
        (6, ps),
        (16, family.into()),
        (17, sub.into()),
    ];
    let mut strings: Vec<u8> = Vec::new();
    let mut out: Vec<u8> = Vec::new();
    out.extend(0u16.to_be_bytes());
    out.extend((recs.len() as u16).to_be_bytes());
    out.extend((6 + 12 * recs.len() as u16).to_be_bytes());
    for (id, s) in &recs {
        let bytes: Vec<u8> = s.encode_utf16().flat_map(u16::to_be_bytes).collect();
        for v in [
            3u16,
            1,
            0x0409,
            *id,
            bytes.len() as u16,
            strings.len() as u16,
        ] {
            out.extend(v.to_be_bytes());
        }
        strings.extend(bytes);
    }
    out.extend(strings);
    out
}

/// Inter (`src`) renamed to `family`/`sub`, with its ascent set to
/// `ascent` font units when given.
fn renamed(src: &[u8], family: &str, sub: &str, ascent: Option<i16>) -> Tables {
    let mut t = tables_of(src);
    for (tag, data) in &mut t {
        match &*tag {
            b"name" => *data = name_table(family, sub),
            b"hhea" => {
                if let Some(a) = ascent {
                    data[4..6].copy_from_slice(&a.to_be_bytes());
                }
            }
            b"OS/2" => {
                if let Some(a) = ascent {
                    data[68..70].copy_from_slice(&a.to_be_bytes());
                    data[74..76].copy_from_slice(&(a as u16).to_be_bytes());
                }
            }
            _ => {}
        }
    }
    t
}

fn checksum(d: &[u8]) -> u32 {
    d.chunks(4).fold(0u32, |acc, c| {
        let mut b = [0u8; 4];
        b[..c.len()].copy_from_slice(c);
        acc.wrapping_add(u32::from_be_bytes(b))
    })
}

/// A font file (one face) or a collection (several) from table sets.
fn build(fonts: &[Tables]) -> Vec<u8> {
    let collection = fonts.len() > 1;
    let header = if collection { 12 + 4 * fonts.len() } else { 0 };
    let dirs: Vec<usize> = fonts.iter().map(|t| 12 + 16 * t.len()).collect();
    let mut data_at = header + dirs.iter().sum::<usize>();
    let mut out: Vec<u8> = Vec::new();
    if collection {
        out.extend(b"ttcf");
        out.extend(0x0001_0000u32.to_be_bytes());
        out.extend((fonts.len() as u32).to_be_bytes());
        let mut at = header;
        for d in &dirs {
            out.extend((at as u32).to_be_bytes());
            at += d;
        }
    }
    let mut blobs: Vec<u8> = Vec::new();
    for t in fonts {
        let mut t = t.clone();
        t.sort_by_key(|(tag, _)| *tag);
        let n = t.len() as u16;
        let es = 15 - n.leading_zeros() as u16;
        let sr = (1u16 << es) * 16;
        out.extend(0x0001_0000u32.to_be_bytes());
        for v in [n, sr, es, n * 16 - sr] {
            out.extend(v.to_be_bytes());
        }
        for (tag, data) in &t {
            out.extend(tag);
            out.extend(checksum(data).to_be_bytes());
            out.extend(((data_at + blobs.len()) as u32).to_be_bytes());
            out.extend((data.len() as u32).to_be_bytes());
            blobs.extend(data);
            while !blobs.len().is_multiple_of(4) {
                blobs.push(0);
            }
        }
    }
    data_at = out.len();
    assert_eq!(data_at, header + dirs.iter().sum::<usize>());
    out.extend(blobs);
    out
}

/// A fresh directory under the target dir for one test's fonts.
fn font_dir(name: &str, files: &[(&str, Vec<u8>)]) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("fonts-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for (f, data) in files {
        std::fs::write(dir.join(f), data).unwrap();
    }
    dir
}

fn opts(dir: &Path) -> PdfOptions {
    PdfOptions {
        system_fonts: false,
        font_dirs: vec![dir.to_path_buf()],
        ..PdfOptions::default()
    }
}

fn spec(family: &str, bold: bool, italic: bool) -> FontSpec {
    FontSpec {
        family: Some(family.into()),
        bold,
        italic,
        ..FontSpec::default()
    }
}

fn job(items: Vec<Item>) -> PrintJob {
    PrintJob {
        pages: vec![PageJob {
            width_pt: 300.0,
            height_pt: 200.0,
            items,
        }],
        title: None,
    }
}

fn text(family: &str, s: &str, y: f32) -> Item {
    Item::Text {
        rect: Rect::new(10.0, y, 250.0, 20.0),
        text: s.into(),
        font: spec(family, false, false),
        h_align: HAlign::Left,
        v_align: VAlign::Middle,
        wrap: false,
        overflow: (0.0, 0.0),
        clip: true,
    }
}

fn test_sans_dir(name: &str) -> PathBuf {
    font_dir(
        name,
        &[
            (
                "TestSans-Regular.ttf",
                build(&[renamed(
                    INTER_REGULAR,
                    "Tally Test Sans",
                    "Regular",
                    Some(1500),
                )]),
            ),
            (
                "TestSans-Bold.ttf",
                build(&[renamed(INTER_BOLD, "Tally Test Sans", "Bold", Some(1500))]),
            ),
        ],
    )
}

// ---------------------------------------------------------------------------

#[test]
fn installed_family_resolves_with_its_metrics_and_bold_face() {
    let dir = test_sans_dir("resolve");
    let o = opts(&dir);
    let r = resolve_font_with(&spec("tally test sans", false, false), &o);
    assert_eq!(r.family, "Tally Test Sans");
    assert!(!r.bundled);
    assert!((r.ascent_em - 1500.0 / 2048.0).abs() < 1e-4, "{r:?}");
    assert!(
        (r.line_em() - (1500.0 + 494.0) / 2048.0).abs() < 1e-3,
        "{r:?}"
    );
    // The bold request gets the bold face: Inter Bold is wider.
    let regular = measure_text_with("Hamburgefonts", &spec("Tally Test Sans", false, false), &o);
    let bold = measure_text_with("Hamburgefonts", &spec("Tally Test Sans", true, false), &o);
    assert!(bold > regular * 1.03, "{bold} vs {regular}");
    // Italic falls back to the closest face the family has.
    let it = resolve_font_with(&spec("Tally Test Sans", false, true), &o);
    assert_eq!(it.family, "Tally Test Sans");
}

#[test]
fn missing_family_falls_back_to_inter_with_a_warning() {
    let dir = test_sans_dir("missing");
    let o = opts(&dir);
    let r = resolve_font_with(&spec("No Such Font 123", false, false), &o);
    assert!(r.bundled);
    assert_eq!(r.family, "Inter");
    assert!((r.line_em() - 1.21).abs() < 0.01);
    let out = render_pdf(&job(vec![text("No Such Font 123", "Hello", 10.0)]), &o).unwrap();
    assert!(
        out.warnings
            .iter()
            .any(|w| w.contains("\"No Such Font 123\" is not installed; Inter was used instead")),
        "{:?}",
        out.warnings
    );
    // Names are never paths: a path to an existing font file is not a family.
    let path = dir.join("TestSans-Regular.ttf");
    let r = resolve_font_with(&spec(path.to_str().unwrap(), false, false), &o);
    assert!(r.bundled, "{r:?}");
    let r = resolve_font_with(&spec("../TestSans-Regular.ttf", false, false), &o);
    assert!(r.bundled, "{r:?}");
}

#[test]
fn metric_compatible_substitute_is_used_for_a_missing_family() {
    let dir = font_dir(
        "substitute",
        &[(
            "LiberationSans-Regular.ttf",
            build(&[renamed(INTER_REGULAR, "Liberation Sans", "Regular", None)]),
        )],
    );
    let o = opts(&dir);
    let r = resolve_font_with(&spec("Arial", false, false), &o);
    assert_eq!(r.family, "Liberation Sans");
    assert!(!r.bundled);
    // A metric-compatible substitute is used without a warning.
    let out = render_pdf(&job(vec![text("Arial", "Hello", 10.0)]), &o).unwrap();
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
}

#[test]
fn collection_faces_resolve_by_index() {
    let dir = font_dir(
        "collection",
        &[(
            "Pair.ttc",
            build(&[
                renamed(INTER_REGULAR, "Tally Coll A", "Regular", Some(1400)),
                renamed(INTER_REGULAR, "Tally Coll B", "Regular", Some(1700)),
            ]),
        )],
    );
    let o = opts(&dir);
    let a = resolve_font_with(&spec("Tally Coll A", false, false), &o);
    let b = resolve_font_with(&spec("Tally Coll B", false, false), &o);
    assert_eq!(
        (a.family.as_str(), b.family.as_str()),
        ("Tally Coll A", "Tally Coll B")
    );
    assert!((a.ascent_em - 1400.0 / 2048.0).abs() < 1e-4, "{a:?}");
    assert!((b.ascent_em - 1700.0 / 2048.0).abs() < 1e-4, "{b:?}");
    let out = render_pdf(&job(vec![text("Tally Coll B", "Collection", 10.0)]), &o).unwrap();
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    assert!(Pdf::new(out.pdf).is_ok());
}

#[test]
fn resolved_font_is_embedded_as_a_subset() {
    let dir = test_sans_dir("embed");
    let o = opts(&dir);
    let out = render_pdf(
        &job(vec![
            text("Tally Test Sans", "Embedded 123", 10.0),
            text("Inter", "Inter text", 40.0),
        ]),
        &o,
    )
    .unwrap();
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    let pdf = String::from_utf8_lossy(&out.pdf);
    // krilla names a subset `ABCDEF+PostScriptName`.
    assert!(pdf.contains("+TallyTestSans-Regular"), "test font embedded");
    assert!(pdf.contains("+Inter"), "Inter embedded");
    // Subsetted: far smaller than the two faces.
    assert!(out.pdf.len() < INTER_REGULAR.len() / 4, "{}", out.pdf.len());
    let png = rasterize(&out.pdf, 0, 72).unwrap();
    assert!(png.starts_with(b"\x89PNG"));
}

#[test]
fn malformed_font_files_do_not_break_rendering() {
    let mut truncated = build(&[renamed(INTER_REGULAR, "Tally Broken", "Regular", None)]);
    truncated.truncate(truncated.len() / 3);
    let mut bad_metrics = renamed(INTER_REGULAR, "Tally Bad Metrics", "Regular", None);
    for (tag, data) in &mut bad_metrics {
        if tag == b"head" {
            // unitsPerEm 0.
            data[18..20].copy_from_slice(&0u16.to_be_bytes());
        }
    }
    let dir = font_dir(
        "malformed",
        &[
            (
                "garbage.ttf",
                b"\x00\x01\x00\x00garbage garbage garbage".to_vec(),
            ),
            ("empty.otf", Vec::new()),
            ("Broken.ttf", truncated),
            ("BadMetrics.ttf", build(&[bad_metrics])),
            ("junk.ttc", b"ttcf\x00\x01\x00\x00\xff\xff\xff\xff".to_vec()),
        ],
    );
    let o = opts(&dir);
    for fam in ["Tally Broken", "Tally Bad Metrics", "garbage"] {
        let r = resolve_font_with(&spec(fam, false, false), &o);
        assert!(r.line_em() > 0.5 && r.line_em() < 3.0, "{fam}: {r:?}");
    }
    let out = render_pdf(
        &job(vec![
            text("Tally Broken", "Broken", 10.0),
            text("Tally Bad Metrics", "Bad", 40.0),
        ]),
        &o,
    )
    .unwrap();
    assert!(Pdf::new(out.pdf).is_ok());
}

#[test]
fn distinct_font_requests_are_bounded() {
    let dir = test_sans_dir("limits");
    let o = opts(&dir);
    let items: Vec<Item> = (0..tally_io_pdf::MAX_FONT_REQUESTS + 40)
        .map(|i| text(&format!("Missing Family {i}"), "x", 10.0))
        .collect();
    let out = render_pdf(&job(items), &o).unwrap();
    assert!(out.warnings.len() <= 100, "{}", out.warnings.len());
}

/// Review: limits are sticky within a context (no rebuild-and-reload
/// loop), and measuring never searches other fonts per character.
#[test]
fn context_limits_are_sticky_and_measuring_is_cheap() {
    let n = tally_io_pdf::MAX_SYSTEM_FACES + 6;
    let files: Vec<(String, Vec<u8>)> = (0..n)
        .map(|i| {
            (
                format!("F{i}.ttf"),
                build(&[renamed(
                    INTER_REGULAR,
                    &format!("Tally Many {i}"),
                    "Regular",
                    Some(1500),
                )]),
            )
        })
        .collect();
    let refs: Vec<(&str, Vec<u8>)> = files.iter().map(|(n, d)| (n.as_str(), d.clone())).collect();
    let dir = font_dir("many", &refs);
    let mut ctx = FontContext::new(&opts(&dir)).unwrap();
    let mut installed = 0;
    let mut faces = Vec::new();
    for i in 0..n {
        let (f, r) = ctx.face(&spec(&format!("Tally Many {i}"), false, false));
        installed += usize::from(!r.bundled);
        faces.push(f);
    }
    assert_eq!(installed, tally_io_pdf::MAX_SYSTEM_FACES);
    // Past the limit: Inter, and it stays so (no reload).
    assert!(
        ctx.face(&spec(&format!("Tally Many {}", n - 1), false, false))
            .1
            .bundled
    );
    assert!(!ctx.face(&spec("Tally Many 0", false, false)).1.bundled);
    let t = std::time::Instant::now();
    for k in 0..20_000 {
        let f = faces[k % faces.len()];
        // Characters no face has: estimated, never searched for.
        let w = ctx.width(f, "\u{E000}\u{E100}\u{F0000}abc", 10.0);
        assert!(w > 0.0);
    }
    assert!(t.elapsed().as_secs() < 10, "{:?}", t.elapsed());
    assert!(ctx.warnings().len() <= 100);
    // Rendering many distinct missing characters stays bounded too.
    let many: String = (0..2000u32)
        .filter_map(|i| char::from_u32(0x1_0000 + i * 128))
        .collect();
    let t = std::time::Instant::now();
    let out = render_pdf(&job(vec![text("Tally Many 0", &many, 10.0)]), &opts(&dir)).unwrap();
    assert!(Pdf::new(out.pdf).is_ok());
    assert!(t.elapsed().as_secs() < 30, "{:?}", t.elapsed());
}

/// On a machine with Arial installed (macOS, Windows), Arial resolves to
/// itself with its real metrics (ascent 1854, descent 434 of 2048).
#[test]
fn installed_arial_uses_its_own_metrics_when_present() {
    let r = resolve_font(&spec("Arial", false, false), true);
    if r.family != "Arial" {
        eprintln!("Arial is not installed here ({r:?}); skipped");
        return;
    }
    assert!(!r.bundled);
    assert!((r.line_em() - 2288.0 / 2048.0).abs() < 0.01, "{r:?}");
    let arial = tally_io_pdf::measure_text("1,234,567.89", &spec("Arial", false, false), true);
    let inter = tally_io_pdf::measure_text("1,234,567.89", &FontSpec::default(), true);
    assert!(
        arial < inter,
        "Arial digits are narrower: {arial} vs {inter}"
    );
}
