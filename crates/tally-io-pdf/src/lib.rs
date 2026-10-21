//! PDF rendering of print jobs (wave 5, W5-3).
//!
//! A [`PrintJob`] is a list of pages with resolution-independent items
//! (fills, lines, text, SVG) in PDF points with a top-left origin. The
//! engine builds it from the paginated sheet (`tally-engine/src/print.rs`,
//! `service/print.rs`); [`render_pdf`] writes it with
//! [krilla](https://github.com/LaurenzV/krilla) (fonts are subsetted by its
//! `subsetter` dependency), [`rasterize`] renders a page of a PDF to PNG
//! with hayro for previews. Export, Print and Preview all come from the
//! same job, so the three look the same on every OS.
//!
//! Fonts: the static Inter 4.1 faces (Regular, Bold, Italic, BoldItalic;
//! OFL-1.1, SHA-256 in the repository NOTICE) are embedded with
//! `include_bytes!`. Any other family is looked up in the system fonts
//! through fontdb and falls back to Inter with a warning. Characters the
//! chosen face lacks (CJK, symbols) fall back per character to a system
//! face that has them, else they are drawn as the `.notdef` box and
//! reported once. Text measurement, line breaking, wrapping, overflow into
//! free neighbouring space and clipping live here, because this crate
//! holds the fonts.
//!
//! Limits (docs/security.md): a job may hold at most [`MAX_PAGES`] pages
//! and [`MAX_ITEMS`] items, an SVG at most [`MAX_SVG_BYTES`] bytes and a
//! text item at most [`MAX_TEXT_CHARS`] characters; longer input is
//! refused or cut, never trusted.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use krilla::color::rgb;
use krilla::geom::{Path, PathBuilder, Point, Rect as KRect, Size, Transform};
use krilla::num::NormalizedF32;
use krilla::page::PageSettings;
use krilla::paint::{Fill, FillRule, LineCap, Stroke, StrokeDash};
use krilla::surface::Surface;
use krilla::text::{Font, GlyphId, KrillaGlyph};
use krilla::Document;
use krilla_svg::{SurfaceExt, SvgSettings};

/// Most pages in one job.
pub const MAX_PAGES: usize = 10_000;
/// Most items in one job.
pub const MAX_ITEMS: usize = 5_000_000;
/// Largest SVG document accepted (bytes).
pub const MAX_SVG_BYTES: usize = 4 * 1024 * 1024;
/// Longest text drawn for one item (characters); the rest is cut.
pub const MAX_TEXT_CHARS: usize = 4096;
/// Most pixels one [`rasterize`] call renders; a larger page is rendered
/// at a lower resolution (security review 4, M4: a 10 m page took 867 MB
/// at 72 dpi).
pub const MAX_RASTER_PIXELS: f32 = 40_000_000.0;
/// Largest side of a raster image embedded in a chart SVG (pixels).
pub const MAX_SVG_IMAGE_SIDE: u32 = 8192;
/// Most pixels of a raster image embedded in a chart SVG.
pub const MAX_SVG_IMAGE_PIXELS: u64 = 16_000_000;
/// Most lines one wrapped text item produces.
const MAX_LINES: usize = 1000;
/// Most system faces scanned for a missing glyph.
const MAX_FALLBACK_SCAN: usize = 500;
/// Horizontal cell padding in points.
const PAD: f32 = 1.5;

/// A rectangle in points, top-left origin.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Rect {
    /// Left.
    pub x: f32,
    /// Top.
    pub y: f32,
    /// Width.
    pub w: f32,
    /// Height.
    pub h: f32,
}

impl Rect {
    /// A rectangle.
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    fn krilla(&self) -> Option<KRect> {
        KRect::from_xywh(self.x, self.y, self.w.max(0.01), self.h.max(0.01))
    }
}

/// Horizontal alignment of a text item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HAlign {
    /// Left.
    Left,
    /// Centred.
    Center,
    /// Right.
    Right,
}

/// Vertical alignment of a text item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VAlign {
    /// Top.
    Top,
    /// Middle.
    Middle,
    /// Bottom.
    Bottom,
}

/// Line style of a [`Item::Line`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineStyle {
    /// Solid.
    Solid,
    /// Dashed.
    Dashed,
    /// Dotted.
    Dotted,
    /// Two parallel lines.
    Double,
}

/// The font of a text item.
#[derive(Clone, Debug, PartialEq)]
pub struct FontSpec {
    /// Family name (`None`: Inter).
    pub family: Option<String>,
    /// Bold.
    pub bold: bool,
    /// Italic.
    pub italic: bool,
    /// Size in points.
    pub size_pt: f32,
    /// Colour `0xRRGGBBAA`.
    pub color: u32,
    /// Underline.
    pub underline: bool,
    /// Strike-through.
    pub strike: bool,
}

impl Default for FontSpec {
    fn default() -> Self {
        Self {
            family: None,
            bold: false,
            italic: false,
            size_pt: 10.0,
            color: 0x000000ff,
            underline: false,
            strike: false,
        }
    }
}

/// One drawing primitive.
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    /// A filled rectangle.
    Fill {
        /// Area.
        rect: Rect,
        /// Colour `0xRRGGBBAA`.
        color: u32,
    },
    /// A straight line.
    Line {
        /// Start.
        from: (f32, f32),
        /// End.
        to: (f32, f32),
        /// Width in points.
        width: f32,
        /// Colour `0xRRGGBBAA`.
        color: u32,
        /// Style.
        style: LineStyle,
    },
    /// Text in a box.
    Text {
        /// The cell box.
        rect: Rect,
        /// The text (`\n` breaks lines).
        text: String,
        /// Font.
        font: FontSpec,
        /// Horizontal alignment.
        h_align: HAlign,
        /// Vertical alignment.
        v_align: VAlign,
        /// Wrap at the box width.
        wrap: bool,
        /// Free space `(left, right)` in points the text may spill into
        /// when it does not fit and does not wrap.
        overflow: (f32, f32),
        /// Clip to the box plus the free space.
        clip: bool,
    },
    /// An SVG document scaled into a box.
    Svg {
        /// Area.
        rect: Rect,
        /// The SVG text, shared by every item that draws the same chart
        /// (security review 4, M6: it was copied per page).
        svg: Arc<str>,
    },
}

/// One page.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct PageJob {
    /// Width in points.
    pub width_pt: f32,
    /// Height in points.
    pub height_pt: f32,
    /// Items in drawing order.
    pub items: Vec<Item>,
}

/// A print job.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct PrintJob {
    /// Pages.
    pub pages: Vec<PageJob>,
    /// Document title (PDF metadata).
    pub title: Option<String>,
}

/// Rendering options.
#[derive(Clone, Debug, PartialEq)]
pub struct PdfOptions {
    /// Look up families other than Inter in the system fonts (off in
    /// tests for determinism).
    pub system_fonts: bool,
    /// Embed SVG text as text (else as outlines).
    pub svg_text: bool,
}

impl Default for PdfOptions {
    fn default() -> Self {
        Self {
            system_fonts: true,
            svg_text: true,
        }
    }
}

/// A rendered PDF.
#[derive(Clone, Debug, PartialEq)]
pub struct Rendered {
    /// The PDF bytes.
    pub pdf: Vec<u8>,
    /// Font substitutions and missing glyphs, one line each.
    pub warnings: Vec<String>,
}

/// Errors of this crate.
#[derive(Debug, thiserror::Error)]
pub enum PdfError {
    /// A job limit was exceeded.
    #[error("limit exceeded: {0}")]
    Limit(String),
    /// krilla could not write the document.
    #[error("PDF writer error: {0}")]
    Write(String),
    /// The PDF could not be read back for rasterization.
    #[error("PDF could not be read: {0}")]
    Read(String),
    /// The page index is out of range.
    #[error("no page {0}")]
    NoPage(usize),
    /// A font could not be loaded.
    #[error("font error: {0}")]
    Font(String),
}

// ---------------------------------------------------------------------------
// Fonts
// ---------------------------------------------------------------------------

static INTER_REGULAR: &[u8] = include_bytes!("../fonts/Inter-Regular.ttf");
static INTER_BOLD: &[u8] = include_bytes!("../fonts/Inter-Bold.ttf");
static INTER_ITALIC: &[u8] = include_bytes!("../fonts/Inter-Italic.ttf");
static INTER_BOLD_ITALIC: &[u8] = include_bytes!("../fonts/Inter-BoldItalic.ttf");

/// Families treated as Inter without a system lookup.
const INTER_ALIASES: &[&str] = &[
    "inter",
    "inter variable",
    "sans-serif",
    "sans",
    "default",
    "",
];

/// A family name mapped to a few likely system alternatives.
fn family_candidates(family: &str) -> Vec<String> {
    let f = family.trim();
    let mut v = vec![f.to_string()];
    match f.to_ascii_lowercase().as_str() {
        "liberation sans" | "arial" | "helvetica" => {
            v.extend(["Arial", "Helvetica", "Liberation Sans"].map(String::from));
        }
        "liberation serif" | "times new roman" | "times" => {
            v.extend(["Times New Roman", "Times", "Liberation Serif"].map(String::from));
        }
        "liberation mono" | "courier new" | "courier" | "menlo" | "monospace" => {
            v.extend(["Courier New", "Menlo", "Liberation Mono"].map(String::from));
        }
        "calibri" | "carlito" => v.extend(["Carlito", "Calibri"].map(String::from)),
        "cambria" | "caladea" => v.extend(["Caladea", "Cambria"].map(String::from)),
        _ => {}
    }
    v.dedup();
    v
}

/// Preferred system faces for characters Inter lacks (CJK first).
const FALLBACK_FAMILIES: &[&str] = &[
    "Noto Sans CJK SC",
    "Noto Sans CJK JP",
    "Noto Sans CJK TC",
    "Noto Sans CJK KR",
    "PingFang SC",
    "Hiragino Sans",
    "Hiragino Kaku Gothic ProN",
    "Apple SD Gothic Neo",
    "Microsoft YaHei",
    "Yu Gothic",
    "Malgun Gothic",
    "Noto Sans",
    "Arial Unicode MS",
    "DejaVu Sans",
    "Apple Symbols",
    "Apple Color Emoji",
    "Segoe UI Symbol",
];

/// A loaded face: the bytes, the krilla font and the metrics we need.
struct Face {
    data: Arc<Vec<u8>>,
    index: u32,
    font: Font,
    units_per_em: f32,
    ascender: f32,
    descender: f32,
    /// Glyph id and advance (font units) per character, cached.
    glyphs: HashMap<char, Option<(u16, u16)>>,
}

impl Face {
    fn load(data: Arc<Vec<u8>>, index: u32) -> Option<Self> {
        let face = ttf_parser::Face::parse(&data, index).ok()?;
        let units_per_em = f32::from(face.units_per_em().max(1));
        let ascender = f32::from(face.ascender());
        let descender = f32::from(face.descender());
        let font = Font::new(data.clone().into(), index)?;
        Some(Self {
            data,
            index,
            font,
            units_per_em,
            ascender,
            descender,
            glyphs: HashMap::new(),
        })
    }

    /// `(glyph id, advance)` of a character, `None` when the face lacks it.
    fn glyph(&mut self, ch: char) -> Option<(u16, u16)> {
        if let Some(g) = self.glyphs.get(&ch) {
            return *g;
        }
        let g = ttf_parser::Face::parse(&self.data, self.index)
            .ok()
            .and_then(|face| {
                let id = face.glyph_index(ch)?;
                let adv = face.glyph_hor_advance(id).unwrap_or(0);
                Some((id.0, adv))
            });
        self.glyphs.insert(ch, g);
        g
    }

    fn has(&mut self, ch: char) -> bool {
        self.glyph(ch).is_some()
    }
}

/// The font registry of one render.
struct Fonts {
    faces: Vec<Face>,
    /// Resolved `(family lowercase, bold, italic)` → face index.
    resolved: HashMap<(String, bool, bool), usize>,
    /// Fallback face per character (`None`: nothing has it).
    fallback: HashMap<char, Option<usize>>,
    /// System faces already loaded, by fontdb id.
    loaded_ids: HashMap<fontdb::ID, usize>,
    db: Option<Arc<fontdb::Database>>,
    warnings: Vec<String>,
    missing_reported: bool,
}

/// The system font database, loaded once per process.
fn system_db() -> Arc<fontdb::Database> {
    static DB: OnceLock<Arc<fontdb::Database>> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        Arc::new(db)
    })
    .clone()
}

/// The fontdb used for SVG text: the system fonts plus Inter.
fn svg_db(system: bool) -> Arc<fontdb::Database> {
    static WITH: OnceLock<Arc<fontdb::Database>> = OnceLock::new();
    static WITHOUT: OnceLock<Arc<fontdb::Database>> = OnceLock::new();
    let slot = if system { &WITH } else { &WITHOUT };
    slot.get_or_init(|| {
        let mut db = if system {
            (*system_db()).clone()
        } else {
            fontdb::Database::new()
        };
        for data in [INTER_REGULAR, INTER_BOLD, INTER_ITALIC, INTER_BOLD_ITALIC] {
            db.load_font_data(data.to_vec());
        }
        db.set_sans_serif_family("Inter");
        Arc::new(db)
    })
    .clone()
}

impl Fonts {
    fn new(system: bool) -> Result<Self, PdfError> {
        let mut f = Self {
            faces: Vec::new(),
            resolved: HashMap::new(),
            fallback: HashMap::new(),
            loaded_ids: HashMap::new(),
            db: system.then(system_db),
            warnings: Vec::new(),
            missing_reported: false,
        };
        // Inter faces 0–3: regular, bold, italic, bold italic.
        for (i, data) in [INTER_REGULAR, INTER_BOLD, INTER_ITALIC, INTER_BOLD_ITALIC]
            .iter()
            .enumerate()
        {
            let face = Face::load(Arc::new(data.to_vec()), 0)
                .ok_or_else(|| PdfError::Font(format!("embedded Inter face {i} is unreadable")))?;
            f.faces.push(face);
        }
        Ok(f)
    }

    fn inter(bold: bool, italic: bool) -> usize {
        match (bold, italic) {
            (false, false) => 0,
            (true, false) => 1,
            (false, true) => 2,
            (true, true) => 3,
        }
    }

    fn warn(&mut self, msg: String) {
        if !self.warnings.contains(&msg) && self.warnings.len() < 100 {
            self.warnings.push(msg);
        }
    }

    fn load_system(&mut self, id: fontdb::ID) -> Option<usize> {
        if let Some(i) = self.loaded_ids.get(&id) {
            return Some(*i);
        }
        let db = self.db.clone()?;
        let (source, index) = db.face_source(id)?;
        let data: Arc<Vec<u8>> = match source {
            fontdb::Source::Binary(b) => Arc::new(b.as_ref().as_ref().to_vec()),
            fontdb::Source::File(p) => Arc::new(std::fs::read(p).ok()?),
            fontdb::Source::SharedFile(_, b) => Arc::new(b.as_ref().as_ref().to_vec()),
        };
        let face = Face::load(data, index)?;
        self.faces.push(face);
        let i = self.faces.len() - 1;
        self.loaded_ids.insert(id, i);
        Some(i)
    }

    /// The face for a font spec (Inter when the family is unknown).
    fn resolve(&mut self, spec: &FontSpec) -> usize {
        let family = spec
            .family
            .as_deref()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        let key = (family.clone(), spec.bold, spec.italic);
        if let Some(i) = self.resolved.get(&key) {
            return *i;
        }
        let inter = Self::inter(spec.bold, spec.italic);
        let mut chosen = inter;
        if !INTER_ALIASES.contains(&family.as_str()) {
            let found = self.db.clone().and_then(|db| {
                let candidates = family_candidates(spec.family.as_deref().unwrap_or(""));
                let names: Vec<fontdb::Family<'_>> =
                    candidates.iter().map(|c| fontdb::Family::Name(c)).collect();
                let q = fontdb::Query {
                    families: &names,
                    weight: if spec.bold {
                        fontdb::Weight::BOLD
                    } else {
                        fontdb::Weight::NORMAL
                    },
                    stretch: fontdb::Stretch::Normal,
                    style: if spec.italic {
                        fontdb::Style::Italic
                    } else {
                        fontdb::Style::Normal
                    },
                };
                let id = db.query(&q)?;
                // fontdb falls back to any family when nothing matches the
                // requested names: accept only a real match.
                let info = db.face(id)?;
                let matched = info
                    .families
                    .iter()
                    .any(|(n, _)| candidates.iter().any(|c| c.eq_ignore_ascii_case(n)));
                matched.then_some(id)
            });
            match found.and_then(|id| self.load_system(id)) {
                Some(i) => chosen = i,
                None => self.warn(format!(
                    "Font \"{}\" is not installed; Inter was used instead.",
                    spec.family.as_deref().unwrap_or("")
                )),
            }
        }
        self.resolved.insert(key, chosen);
        chosen
    }

    /// A face that has `ch`, searched among the preferred fallback
    /// families and then every system face (bounded), cached per character.
    fn fallback_for(&mut self, ch: char) -> Option<usize> {
        if let Some(f) = self.fallback.get(&ch) {
            return *f;
        }
        let mut found: Option<usize> = None;
        // Inter itself (another style may be the primary face).
        for i in 0..4 {
            if self.faces[i].has(ch) {
                found = Some(i);
                break;
            }
        }
        if found.is_none() {
            if let Some(db) = self.db.clone() {
                let mut ids: Vec<fontdb::ID> = Vec::new();
                for fam in FALLBACK_FAMILIES {
                    let q = fontdb::Query {
                        families: &[fontdb::Family::Name(fam)],
                        ..fontdb::Query::default()
                    };
                    if let Some(id) = db.query(&q) {
                        if db
                            .face(id)
                            .is_some_and(|f| f.families.iter().any(|(n, _)| n == fam))
                        {
                            ids.push(id);
                        }
                    }
                }
                ids.extend(db.faces().map(|f| f.id).take(MAX_FALLBACK_SCAN));
                let mut seen = std::collections::HashSet::new();
                for id in ids {
                    if !seen.insert(id) {
                        continue;
                    }
                    let has = db
                        .with_face_data(id, |data, index| {
                            ttf_parser::Face::parse(data, index)
                                .ok()
                                .is_some_and(|f| f.glyph_index(ch).is_some())
                        })
                        .unwrap_or(false);
                    if has {
                        if let Some(i) = self.load_system(id) {
                            found = Some(i);
                            break;
                        }
                    }
                }
            }
        }
        if found.is_none() && !self.missing_reported {
            self.missing_reported = true;
            self.warn(format!(
                "No installed font has the character U+{:04X}; it is drawn as a box.",
                u32::from(ch)
            ));
        }
        self.fallback.insert(ch, found);
        found
    }
}

/// One run of glyphs in one face.
struct Run {
    face: usize,
    text: String,
    glyphs: Vec<KrillaGlyph>,
    /// Width in points.
    width: f32,
}

/// A shaped line.
struct Line {
    runs: Vec<Run>,
    width: f32,
}

/// Shapes `text` (one line) at `size` with `primary` and per-character
/// fallback: glyph ids and advances from the face metrics (no kerning).
fn shape_line(fonts: &mut Fonts, primary: usize, text: &str, size: f32) -> Line {
    let mut runs: Vec<Run> = Vec::new();
    let mut width = 0.0f32;
    for ch in text.chars() {
        let ch = if ch == '\t' { ' ' } else { ch };
        if ch.is_control() {
            continue;
        }
        let face = if fonts.faces[primary].has(ch) {
            primary
        } else {
            fonts.fallback_for(ch).unwrap_or(primary)
        };
        let (gid, adv) = fonts.faces[face].glyph(ch).unwrap_or((0, {
            // `.notdef`: half an em.
            (fonts.faces[face].units_per_em / 2.0) as u16
        }));
        let upem = fonts.faces[face].units_per_em;
        let x_advance = f32::from(adv) / upem;
        width += x_advance * size;
        let need_new = runs.last().is_none_or(|r| r.face != face);
        if need_new {
            runs.push(Run {
                face,
                text: String::new(),
                glyphs: Vec::new(),
                width: 0.0,
            });
        }
        if let Some(run) = runs.last_mut() {
            let start = run.text.len();
            run.text.push(ch);
            run.glyphs.push(KrillaGlyph::new(
                GlyphId::new(u32::from(gid)),
                x_advance,
                0.0,
                0.0,
                0.0,
                start..run.text.len(),
                None,
            ));
            run.width += x_advance * size;
        }
    }
    Line { runs, width }
}

/// Width of `text` in points (single line).
fn measure(fonts: &mut Fonts, primary: usize, text: &str, size: f32) -> f32 {
    shape_line(fonts, primary, text, size).width
}

/// Greedy word wrap of one paragraph into lines no wider than `max_w`
/// (a word longer than the width is broken between characters).
fn wrap_paragraph(
    fonts: &mut Fonts,
    primary: usize,
    para: &str,
    size: f32,
    max_w: f32,
) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut cur_w = 0.0f32;
    let space_w = measure(fonts, primary, " ", size);
    for word in para.split(' ') {
        let w = measure(fonts, primary, word, size);
        if cur.is_empty() {
            if w <= max_w || word.chars().count() <= 1 {
                cur.push_str(word);
                cur_w = w;
            } else {
                // Break the word by characters.
                for ch in word.chars() {
                    let cw = measure(fonts, primary, &ch.to_string(), size);
                    if !cur.is_empty() && cur_w + cw > max_w {
                        lines.push(std::mem::take(&mut cur));
                        cur_w = 0.0;
                        if lines.len() >= MAX_LINES {
                            return lines;
                        }
                    }
                    cur.push(ch);
                    cur_w += cw;
                }
            }
            continue;
        }
        if cur_w + space_w + w <= max_w {
            cur.push(' ');
            cur.push_str(word);
            cur_w += space_w + w;
        } else {
            lines.push(std::mem::take(&mut cur));
            if lines.len() >= MAX_LINES {
                return lines;
            }
            cur.push_str(word);
            cur_w = w;
        }
    }
    lines.push(cur);
    lines
}

fn color(c: u32) -> (rgb::Color, NormalizedF32) {
    let r = (c >> 24) as u8;
    let g = (c >> 16) as u8;
    let b = (c >> 8) as u8;
    let a = (c & 0xff) as u8;
    (
        rgb::Color::new(r, g, b),
        NormalizedF32::new(f32::from(a) / 255.0).unwrap_or(NormalizedF32::ONE),
    )
}

fn fill_of(c: u32) -> Fill {
    let (col, a) = color(c);
    Fill {
        paint: col.into(),
        opacity: a,
        rule: FillRule::NonZero,
    }
}

fn rect_path(r: &Rect) -> Option<Path> {
    let mut pb = PathBuilder::new();
    pb.push_rect(r.krilla()?);
    pb.finish()
}

fn line_path(from: (f32, f32), to: (f32, f32)) -> Option<Path> {
    let mut pb = PathBuilder::new();
    pb.move_to(from.0, from.1);
    pb.line_to(to.0, to.1);
    pb.finish()
}

fn draw_line(
    surface: &mut Surface<'_>,
    from: (f32, f32),
    to: (f32, f32),
    width: f32,
    c: u32,
    style: LineStyle,
) {
    let (col, a) = color(c);
    let w = width.max(0.2);
    let dash = match style {
        LineStyle::Solid | LineStyle::Double => None,
        LineStyle::Dashed => Some(StrokeDash {
            array: vec![3.0 * w.max(1.0), 2.0 * w.max(1.0)],
            offset: 0.0,
        }),
        LineStyle::Dotted => Some(StrokeDash {
            array: vec![w.max(1.0), w.max(1.0)],
            offset: 0.0,
        }),
    };
    surface.set_fill(None);
    let stroke = |width: f32| Stroke {
        paint: col.into(),
        width,
        opacity: a,
        dash: dash.clone(),
        line_cap: LineCap::Butt,
        ..Stroke::default()
    };
    match style {
        LineStyle::Double => {
            // Two thin lines either side of the nominal one.
            let thin = (w / 3.0).max(0.2);
            let (dx, dy) = if (from.1 - to.1).abs() < f32::EPSILON {
                (0.0, w / 1.5)
            } else {
                (w / 1.5, 0.0)
            };
            surface.set_stroke(Some(stroke(thin)));
            for sign in [-1.0f32, 1.0] {
                let f = (from.0 + sign * dx, from.1 + sign * dy);
                let t = (to.0 + sign * dx, to.1 + sign * dy);
                if let Some(p) = line_path(f, t) {
                    surface.draw_path(&p);
                }
            }
        }
        _ => {
            surface.set_stroke(Some(stroke(w)));
            if let Some(p) = line_path(from, to) {
                surface.draw_path(&p);
            }
        }
    }
    surface.set_stroke(None);
}

#[allow(clippy::too_many_arguments)]
fn draw_text(
    surface: &mut Surface<'_>,
    fonts: &mut Fonts,
    rect: &Rect,
    text: &str,
    font: &FontSpec,
    h_align: HAlign,
    v_align: VAlign,
    wrap: bool,
    overflow: (f32, f32),
    clip: bool,
) {
    let size = font.size_pt.clamp(1.0, 400.0);
    let primary = fonts.resolve(font);
    let text: String = text.chars().take(MAX_TEXT_CHARS).collect();
    let inner_w = (rect.w - 2.0 * PAD).max(0.0);
    let mut lines: Vec<String> = Vec::new();
    for para in text.split('\n') {
        if wrap && inner_w > 0.0 {
            lines.extend(wrap_paragraph(fonts, primary, para, size, inner_w));
        } else {
            lines.push(para.to_string());
        }
        if lines.len() >= MAX_LINES {
            lines.truncate(MAX_LINES);
            break;
        }
    }
    let shaped: Vec<Line> = lines
        .iter()
        .map(|l| shape_line(fonts, primary, l, size))
        .collect();
    let widest = shaped.iter().map(|l| l.width).fold(0.0, f32::max);
    let (ascender, descender, upem) = {
        let f = &fonts.faces[primary];
        (f.ascender, f.descender, f.units_per_em)
    };
    let ascent = ascender / upem * size;
    let descent = -descender / upem * size;
    let line_h = (ascent + descent).max(size) * 1.15;
    let block_h = line_h * shaped.len() as f32;
    // Horizontal box the text may occupy: the cell plus free space when it
    // does not fit (and does not wrap).
    let (mut left, mut right) = (rect.x + PAD, rect.x + rect.w - PAD);
    if widest > inner_w && !wrap {
        match h_align {
            HAlign::Left => right += overflow.1,
            HAlign::Right => left -= overflow.0,
            HAlign::Center => {
                let extra = ((widest - inner_w) / 2.0).max(0.0);
                left -= extra.min(overflow.0);
                right += extra.min(overflow.1);
            }
        }
    }
    let top = match v_align {
        VAlign::Top => rect.y + PAD * 0.5,
        VAlign::Middle => rect.y + (rect.h - block_h) / 2.0,
        VAlign::Bottom => rect.y + rect.h - block_h - PAD * 0.5,
    };
    let clip_rect = Rect::new(
        left - PAD,
        rect.y,
        (right - left + 2.0 * PAD).max(0.0),
        rect.h,
    );
    let need_clip = clip && (widest > right - left || block_h > rect.h);
    if need_clip {
        if let Some(p) = rect_path(&clip_rect) {
            surface.push_clip_path(&p, &FillRule::NonZero);
        }
    }
    surface.set_stroke(None);
    surface.set_fill(Some(fill_of(font.color)));
    for (i, line) in shaped.iter().enumerate() {
        let x0 = match h_align {
            HAlign::Left => left,
            HAlign::Right => right - line.width,
            HAlign::Center => (left + right - line.width) / 2.0,
        };
        let baseline = top + line_h * i as f32 + ascent + (line_h - ascent - descent) / 2.0;
        let mut x = x0;
        for run in &line.runs {
            let f = fonts.faces[run.face].font.clone();
            surface.draw_glyphs(
                Point::from_xy(x, baseline),
                &run.glyphs,
                f,
                &run.text,
                size,
                false,
            );
            x += run.width;
        }
        if font.underline || font.strike {
            let (col, a) = color(font.color);
            let w = (size / 14.0).max(0.4);
            surface.set_stroke(Some(Stroke {
                paint: col.into(),
                width: w,
                opacity: a,
                ..Stroke::default()
            }));
            let mut ys = Vec::new();
            if font.underline {
                ys.push(baseline + descent * 0.6);
            }
            if font.strike {
                ys.push(baseline - ascent * 0.35);
            }
            for y in ys {
                if let Some(p) = line_path((x0, y), (x0 + line.width, y)) {
                    surface.draw_path(&p);
                }
            }
            surface.set_stroke(None);
            surface.set_fill(Some(fill_of(font.color)));
        }
    }
    if need_clip {
        surface.pop();
    }
}

/// usvg's `data:` resolver, refusing SVGZ and raster images over
/// [`MAX_SVG_IMAGE_SIDE`] / [`MAX_SVG_IMAGE_PIXELS`] (or of unknown size).
fn data_image_resolver() -> usvg::ImageHrefDataResolverFn<'static> {
    let default = usvg::ImageHrefResolver::default_data_resolver();
    Box::new(move |mime: &str, data: Arc<Vec<u8>>, o: &usvg::Options| {
        if data.starts_with(&[0x1f, 0x8b]) {
            return None;
        }
        let kind = default(mime, data, o)?;
        let raster = match &kind {
            usvg::ImageKind::JPEG(d)
            | usvg::ImageKind::PNG(d)
            | usvg::ImageKind::GIF(d)
            | usvg::ImageKind::WEBP(d) => Some(d.clone()),
            _ => None,
        };
        if let Some(d) = raster {
            let (w, h) = image_size(&d)?;
            if w == 0
                || h == 0
                || w > MAX_SVG_IMAGE_SIDE
                || h > MAX_SVG_IMAGE_SIDE
                || u64::from(w) * u64::from(h) > MAX_SVG_IMAGE_PIXELS
            {
                return None;
            }
        }
        Some(kind)
    })
}

fn be16(d: &[u8], i: usize) -> Option<u32> {
    Some(u32::from(u16::from_be_bytes([*d.get(i)?, *d.get(i + 1)?])))
}

fn le16(d: &[u8], i: usize) -> Option<u32> {
    Some(u32::from(u16::from_le_bytes([*d.get(i)?, *d.get(i + 1)?])))
}

fn le24(d: &[u8], i: usize) -> Option<u32> {
    Some(u32::from(*d.get(i)?) | u32::from(*d.get(i + 1)?) << 8 | u32::from(*d.get(i + 2)?) << 16)
}

/// `(width, height)` of a PNG, GIF, JPEG or WebP from its header.
fn image_size(d: &[u8]) -> Option<(u32, u32)> {
    if d.starts_with(b"\x89PNG\r\n\x1a\n") {
        let w = u32::from_be_bytes(d.get(16..20)?.try_into().ok()?);
        let h = u32::from_be_bytes(d.get(20..24)?.try_into().ok()?);
        return Some((w, h));
    }
    if d.starts_with(b"GIF8") {
        return Some((le16(d, 6)?, le16(d, 8)?));
    }
    if d.starts_with(&[0xFF, 0xD8]) {
        // Walk the JPEG markers to the first start-of-frame.
        let mut i = 2;
        while i + 4 <= d.len() {
            if d[i] != 0xFF {
                return None;
            }
            let m = d[i + 1];
            if m == 0xFF {
                i += 1;
                continue;
            }
            if matches!(m, 0xD8 | 0x01 | 0xD0..=0xD7) {
                i += 2;
                continue;
            }
            let len = be16(d, i + 2)? as usize;
            if matches!(m, 0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF) {
                return Some((be16(d, i + 7)?, be16(d, i + 5)?));
            }
            if len < 2 {
                return None;
            }
            i += 2 + len;
        }
        return None;
    }
    if d.len() >= 30 && &d[0..4] == b"RIFF" && &d[8..12] == b"WEBP" {
        return match &d[12..16] {
            b"VP8 " => Some((le16(d, 26)? & 0x3FFF, le16(d, 28)? & 0x3FFF)),
            b"VP8L" => {
                let b = u32::from_le_bytes(d.get(21..25)?.try_into().ok()?);
                Some(((b & 0x3FFF) + 1, ((b >> 14) & 0x3FFF) + 1))
            }
            b"VP8X" => Some((le24(d, 24)? + 1, le24(d, 27)? + 1)),
            _ => None,
        };
    }
    None
}

fn draw_svg(surface: &mut Surface<'_>, rect: &Rect, svg: &str, opts: &PdfOptions) -> Option<()> {
    if svg.len() > MAX_SVG_BYTES {
        return None;
    }
    // Only `data:` images, of bounded dimensions (security review 4, H15:
    // usvg's default resolver read any local path, `/dev/zero` and FIFOs
    // included). No resources directory, no SVGZ (unbounded inflate).
    let options = usvg::Options {
        fontdb: svg_db(opts.system_fonts),
        font_family: "Inter".into(),
        resources_dir: None,
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: data_image_resolver(),
            resolve_string: Box::new(|_, _| None),
        },
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_str(svg, &options).ok()?;
    let size = Size::from_wh(rect.w.max(0.1), rect.h.max(0.1))?;
    surface.push_transform(&Transform::from_translate(rect.x, rect.y));
    let r = surface.draw_svg(
        &tree,
        size,
        SvgSettings {
            embed_text: opts.svg_text,
            ..SvgSettings::default()
        },
    );
    surface.pop();
    r
}

/// Renders a job to PDF.
pub fn render_pdf(job: &PrintJob, opts: &PdfOptions) -> Result<Rendered, PdfError> {
    if job.pages.len() > MAX_PAGES {
        return Err(PdfError::Limit(format!("more than {MAX_PAGES} pages")));
    }
    let items: usize = job.pages.iter().map(|p| p.items.len()).sum();
    if items > MAX_ITEMS {
        return Err(PdfError::Limit(format!("more than {MAX_ITEMS} items")));
    }
    let mut fonts = Fonts::new(opts.system_fonts)?;
    let mut document = Document::new();
    let mut metadata = krilla::metadata::Metadata::new().creator("Tally".into());
    if let Some(t) = &job.title {
        metadata = metadata.title(t.clone());
    }
    document.set_metadata(metadata);
    let mut svg_failures = 0usize;
    for page in &job.pages {
        let w = page.width_pt.clamp(10.0, 14_400.0);
        let h = page.height_pt.clamp(10.0, 14_400.0);
        let settings =
            PageSettings::from_wh(w, h).ok_or_else(|| PdfError::Write("bad page size".into()))?;
        let mut p = document.start_page_with(settings);
        let mut surface = p.surface();
        for item in &page.items {
            match item {
                Item::Fill { rect, color: c } => {
                    if let Some(path) = rect_path(rect) {
                        surface.set_stroke(None);
                        surface.set_fill(Some(fill_of(*c)));
                        surface.draw_path(&path);
                    }
                }
                Item::Line {
                    from,
                    to,
                    width,
                    color: c,
                    style,
                } => draw_line(&mut surface, *from, *to, *width, *c, *style),
                Item::Text {
                    rect,
                    text,
                    font,
                    h_align,
                    v_align,
                    wrap,
                    overflow,
                    clip,
                } => {
                    if text.is_empty() {
                        continue;
                    }
                    draw_text(
                        &mut surface,
                        &mut fonts,
                        rect,
                        text,
                        font,
                        *h_align,
                        *v_align,
                        *wrap,
                        *overflow,
                        *clip,
                    );
                }
                Item::Svg { rect, svg } => {
                    if draw_svg(&mut surface, rect, svg, opts).is_none() {
                        svg_failures += 1;
                    }
                }
            }
        }
        surface.finish();
        p.finish();
    }
    let pdf = document
        .finish()
        .map_err(|e| PdfError::Write(format!("{e:?}")))?;
    let mut warnings = fonts.warnings;
    if svg_failures > 0 {
        warnings.push(format!(
            "{svg_failures} chart image(s) could not be rendered and were left blank."
        ));
    }
    Ok(Rendered { pdf, warnings })
}

/// Width of `text` in points when drawn with `font` (single line, Inter
/// metrics or the resolved system face), for callers that lay out text.
pub fn measure_text(text: &str, font: &FontSpec, system_fonts: bool) -> f32 {
    static CACHE: OnceLock<Mutex<Option<Fonts>>> = OnceLock::new();
    let m = CACHE.get_or_init(|| Mutex::new(None));
    let mut guard = match m.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    if guard.is_none() {
        *guard = Fonts::new(system_fonts).ok();
    }
    let Some(fonts) = guard.as_mut() else {
        return text.chars().count() as f32 * font.size_pt * 0.55;
    };
    let primary = fonts.resolve(font);
    measure(fonts, primary, text, font.size_pt.clamp(1.0, 400.0))
}

/// The pixels-per-point scale [`rasterize`] uses for a `w × h` pt page at
/// `dpi` (18–600): lowered so the image has at most [`MAX_RASTER_PIXELS`]
/// pixels.
pub fn raster_scale(w: f32, h: f32, dpi: u32) -> f32 {
    let mut scale = (dpi.clamp(18, 600) as f32) / 72.0;
    let px = w.max(1.0) * h.max(1.0) * scale * scale;
    if px > MAX_RASTER_PIXELS {
        scale *= (MAX_RASTER_PIXELS / px).sqrt();
    }
    scale
}

/// Rasterizes page `page` (0-based) of a PDF to PNG at `dpi`; pages over
/// [`MAX_RASTER_PIXELS`] are rendered at a lower resolution.
pub fn rasterize(pdf: &[u8], page: usize, dpi: u32) -> Result<Vec<u8>, PdfError> {
    use hayro::hayro_interpret::InterpreterSettings;
    use hayro::hayro_syntax::Pdf;
    use hayro::{RenderCache, RenderSettings};
    let doc = Pdf::new(pdf.to_vec()).map_err(|e| PdfError::Read(format!("{e:?}")))?;
    let pages = doc.pages();
    let p = pages.get(page).ok_or(PdfError::NoPage(page))?;
    let (w, h) = p.render_dimensions();
    let scale = raster_scale(w, h, dpi);
    let settings = RenderSettings {
        x_scale: scale,
        y_scale: scale,
        width: None,
        height: None,
        bg_color: hayro::vello_cpu::color::palette::css::WHITE,
    };
    let pixmap = hayro::render(
        p,
        &RenderCache::new(),
        &InterpreterSettings::default(),
        &settings,
    );
    pixmap
        .into_png()
        .map_err(|e| PdfError::Write(format!("PNG encoding failed: {e}")))
}

/// Decodes a PNG (as [`rasterize`] produces) into `(width, height, RGBA8)`
/// for pixel comparisons in tests.
pub fn png_pixels(png: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let pix = hayro::vello_cpu::Pixmap::from_png(std::io::Cursor::new(png)).ok()?;
    let (w, h) = (u32::from(pix.width()), u32::from(pix.height()));
    Some((w, h, pix.data_as_u8_slice().to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_and_measures() {
        let mut fonts = Fonts::new(false).unwrap();
        let w = measure(&mut fonts, 0, "Hello", 10.0);
        assert!(w > 15.0 && w < 40.0, "{w}");
        let lines = wrap_paragraph(&mut fonts, 0, "one two three four five six", 10.0, 60.0);
        assert!(lines.len() >= 2, "{lines:?}");
        assert!(lines
            .iter()
            .all(|l| measure(&mut fonts, 0, l, 10.0) <= 60.5));
        let broken = wrap_paragraph(&mut fonts, 0, "abcdefghijklmnopqrstuvwxyz", 10.0, 30.0);
        assert!(broken.len() >= 2);
        assert!(fonts.faces[1].has('A'));
    }

    #[test]
    fn svg_images_are_data_only_and_bounded() {
        // Security review 4, H15.
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend_from_slice(&100_000u32.to_be_bytes());
        png.extend_from_slice(&100_000u32.to_be_bytes());
        png.extend_from_slice(&[8, 6, 0, 0, 0]);
        assert_eq!(image_size(&png), Some((100_000, 100_000)));
        let opts = usvg::Options::default();
        let r = data_image_resolver();
        assert!(r("image/png", Arc::new(png.clone()), &opts).is_none());
        png[16..20].copy_from_slice(&16u32.to_be_bytes());
        png[20..24].copy_from_slice(&16u32.to_be_bytes());
        assert!(r("image/png", Arc::new(png), &opts).is_some());
        assert_eq!(image_size(b"GIF89a\x10\x00\x20\x00"), Some((16, 32)));
        assert!(r("image/svg+xml", Arc::new(vec![0x1f, 0x8b, 8, 0]), &opts).is_none());
        assert!(r("image/png", Arc::new(b"junk".to_vec()), &opts).is_none());
    }

    #[test]
    fn colours() {
        let (c, a) = color(0xff8000ff);
        assert_eq!(c, rgb::Color::new(255, 128, 0));
        assert_eq!(a, NormalizedF32::ONE);
        let (_, a) = color(0x00000000);
        assert_eq!(a.get(), 0.0);
    }
}
