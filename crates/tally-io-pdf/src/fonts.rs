//! Font resolution: the bundled Inter faces, installed system fonts looked
//! up by family name through fontdb, metric-compatible substitutes and
//! per-character fallback.
//!
//! Font names come from untrusted documents (docs/security.md): a name is
//! only ever compared with the family names of installed fonts, never used
//! as a path. A render loads at most [`MAX_SYSTEM_FACES`] system faces and
//! [`MAX_FONT_BYTES`] bytes of font files, resolves at most
//! [`MAX_FONT_REQUESTS`] distinct requests through the system fonts, and a
//! face that fails to parse (or panics while parsing) is skipped, so the
//! text falls back to Inter.

use std::collections::{HashMap, HashSet};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use krilla::text::Font;

use crate::{FontSpec, PdfError, PdfOptions};

static INTER_REGULAR: &[u8] = include_bytes!("../fonts/Inter-Regular.ttf");
static INTER_BOLD: &[u8] = include_bytes!("../fonts/Inter-Bold.ttf");
static INTER_ITALIC: &[u8] = include_bytes!("../fonts/Inter-Italic.ttf");
static INTER_BOLD_ITALIC: &[u8] = include_bytes!("../fonts/Inter-BoldItalic.ttf");

/// The bundled Inter faces: regular, bold, italic, bold italic.
pub(crate) const INTER_FACES: [&[u8]; 4] =
    [INTER_REGULAR, INTER_BOLD, INTER_ITALIC, INTER_BOLD_ITALIC];

/// Most system faces one render (or measuring context) loads.
pub const MAX_SYSTEM_FACES: usize = 64;
/// Most bytes of font files one render (or measuring context) loads, all
/// files together.
pub const MAX_FONT_BYTES: usize = 384 << 20;
/// Largest single font file loaded.
pub const MAX_FONT_FILE_BYTES: usize = 256 << 20;
/// Most distinct `(family, bold, italic)` requests one render resolves
/// through the system fonts; later ones use Inter.
pub const MAX_FONT_REQUESTS: usize = 256;
/// Longest family name looked up (longer names are cut).
const MAX_FAMILY_LEN: usize = 128;
/// Most system faces scanned for a missing glyph.
const MAX_FALLBACK_SCAN: usize = 500;
/// Most characters one render searches the system fonts for; later
/// missing characters are drawn as boxes (each search may parse hundreds
/// of faces).
const MAX_FALLBACK_SEARCHES: usize = 32;

/// Families treated as Inter without a system lookup.
const INTER_ALIASES: &[&str] = &[
    "inter",
    "inter variable",
    "sans-serif",
    "sans",
    "default",
    "",
];

/// A family name followed by its metric-compatible substitutes, then
/// near look-alikes (a missing Arial prints with Liberation Sans or Arimo,
/// which have Arial's advance widths, before Helvetica).
pub fn family_candidates(family: &str) -> Vec<String> {
    let f: String = family.trim().chars().take(MAX_FAMILY_LEN).collect();
    let mut v = vec![f.clone()];
    let subs: &[&str] = match f.to_ascii_lowercase().as_str() {
        "arial" | "liberation sans" | "arimo" | "helvetica" | "helvetica neue" => {
            &["Arial", "Liberation Sans", "Arimo", "Helvetica"]
        }
        "times new roman" | "liberation serif" | "tinos" | "times" => {
            &["Times New Roman", "Liberation Serif", "Tinos", "Times"]
        }
        "courier new" | "liberation mono" | "cousine" | "courier" => {
            &["Courier New", "Liberation Mono", "Cousine", "Courier"]
        }
        "calibri" | "carlito" => &["Calibri", "Carlito"],
        "cambria" | "caladea" => &["Cambria", "Caladea"],
        "menlo" | "monospace" => &["Menlo", "Courier New", "Liberation Mono"],
        _ => &[],
    };
    for s in subs {
        if !v.iter().any(|x| x.eq_ignore_ascii_case(s)) {
            v.push((*s).to_string());
        }
    }
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
pub(crate) struct Face {
    data: Arc<Vec<u8>>,
    index: u32,
    pub font: Font,
    /// The family name the face reports.
    pub family: String,
    pub units_per_em: f32,
    pub ascender: f32,
    pub descender: f32,
    /// Glyph id and advance (font units) per character, cached.
    glyphs: HashMap<char, Option<(u16, u16)>>,
}

impl Face {
    /// Parses a face; `None` when it is unreadable. Parsing a malformed
    /// file must not take the render down, so a panic counts as unreadable.
    fn load(data: Arc<Vec<u8>>, index: u32, family: &str) -> Option<Self> {
        catch_unwind(AssertUnwindSafe(|| {
            let face = ttf_parser::Face::parse(&data, index).ok()?;
            let units_per_em = f32::from(face.units_per_em().max(16));
            let ascender = f32::from(face.ascender());
            let descender = f32::from(face.descender());
            // A face whose metrics are nonsense would lay text out badly.
            if !(0.0..=4.0 * units_per_em).contains(&ascender)
                || !(-4.0 * units_per_em..=0.0).contains(&descender)
            {
                return None;
            }
            let font = Font::new(data.clone().into(), index)?;
            Some(Self {
                data: data.clone(),
                index,
                font,
                family: family.to_string(),
                units_per_em,
                ascender,
                descender,
                glyphs: HashMap::new(),
            })
        }))
        .ok()
        .flatten()
    }

    /// `(glyph id, advance)` of a character, `None` when the face lacks it.
    pub fn glyph(&mut self, ch: char) -> Option<(u16, u16)> {
        if let Some(g) = self.glyphs.get(&ch) {
            return *g;
        }
        let (data, index) = (&self.data, self.index);
        let g = catch_unwind(AssertUnwindSafe(|| {
            ttf_parser::Face::parse(data, index).ok().and_then(|face| {
                let id = face.glyph_index(ch)?;
                let adv = face.glyph_hor_advance(id).unwrap_or(0);
                Some((id.0, adv))
            })
        }))
        .ok()
        .flatten();
        self.glyphs.insert(ch, g);
        g
    }

    pub fn has(&mut self, ch: char) -> bool {
        self.glyph(ch).is_some()
    }
}

/// What a font request resolves to (see [`resolve_font`]).
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedFont {
    /// The family of the face used (`Inter` for the bundled face).
    pub family: String,
    /// The bundled Inter face is used (the request named the app font, or
    /// no installed family matched).
    pub bundled: bool,
    /// Ascent above the baseline, in em of the font size.
    pub ascent_em: f32,
    /// Descent below the baseline (positive), in em of the font size.
    pub descent_em: f32,
}

impl ResolvedFont {
    /// The height of one line of text (ascent plus descent), in em.
    pub fn line_em(&self) -> f32 {
        self.ascent_em + self.descent_em
    }
}

/// The font registry of one render (or one measuring context).
pub(crate) struct Fonts {
    pub faces: Vec<Face>,
    /// Resolved `(family lowercase, bold, italic)` → face index.
    resolved: HashMap<(String, bool, bool), usize>,
    /// Fallback face per character (`None`: nothing has it).
    fallback: HashMap<char, Option<usize>>,
    /// System faces already loaded, by fontdb id.
    loaded_ids: HashMap<fontdb::ID, usize>,
    /// Font files already read (a collection's faces share one).
    files: HashMap<PathBuf, Arc<Vec<u8>>>,
    loaded_bytes: usize,
    lookups: usize,
    db: Option<Arc<fontdb::Database>>,
    pub warnings: Vec<String>,
    missing_reported: bool,
    limit_reported: bool,
    /// System-font searches for missing characters so far.
    fallback_searches: usize,
    /// 128-character blocks a search found nothing for (not searched again).
    missing_blocks: HashSet<u32>,
}

/// Font directories looked at besides fontdb's defaults, per OS (only
/// those fontdb did not already load are added).
fn extra_font_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from);
    let mut v: Vec<PathBuf> = Vec::new();
    if cfg!(target_os = "macos") {
        v.extend(
            [
                "/System/Library/Fonts",
                "/System/Library/Fonts/Supplemental",
                "/Library/Fonts",
            ]
            .map(PathBuf::from),
        );
        if let Some(h) = &home {
            v.push(h.join("Library/Fonts"));
        }
    } else if cfg!(windows) {
        if let Some(w) = std::env::var_os("WINDIR").or_else(|| std::env::var_os("SystemRoot")) {
            v.push(PathBuf::from(w).join("Fonts"));
        }
        if let Some(l) = std::env::var_os("LOCALAPPDATA") {
            v.push(
                PathBuf::from(l)
                    .join("Microsoft")
                    .join("Windows")
                    .join("Fonts"),
            );
        }
    } else {
        v.extend(["/usr/share/fonts", "/usr/local/share/fonts"].map(PathBuf::from));
        if let Some(h) = &home {
            v.push(h.join(".local/share/fonts"));
            v.push(h.join(".fonts"));
        }
    }
    v
}

/// Loads `dir` into `db` unless a face from it is already there.
fn ensure_dir(db: &mut fontdb::Database, dir: &Path) {
    if !dir.is_dir() {
        return;
    }
    let present = db.faces().any(|f| match &f.source {
        fontdb::Source::File(p) | fontdb::Source::SharedFile(p, _) => p.starts_with(dir),
        fontdb::Source::Binary(_) => false,
    });
    if !present {
        db.load_fonts_dir(dir);
    }
}

/// The system font database, loaded once per process. A panic while
/// scanning (a malformed font file) leaves it empty: text prints in Inter.
fn system_db() -> Arc<fontdb::Database> {
    static DB: OnceLock<Arc<fontdb::Database>> = OnceLock::new();
    DB.get_or_init(|| {
        let db = catch_unwind(|| {
            let mut db = fontdb::Database::new();
            db.load_system_fonts();
            for dir in extra_font_dirs() {
                ensure_dir(&mut db, &dir);
            }
            db
        })
        .unwrap_or_default();
        Arc::new(db)
    })
    .clone()
}

/// The database for a set of options: the system fonts (when enabled)
/// plus `font_dirs`, cached per set.
pub(crate) fn database(opts: &PdfOptions) -> Option<Arc<fontdb::Database>> {
    if opts.font_dirs.is_empty() {
        return opts.system_fonts.then(system_db);
    }
    type Key = (bool, Vec<PathBuf>);
    static DBS: OnceLock<Mutex<HashMap<Key, Arc<fontdb::Database>>>> = OnceLock::new();
    let key = (opts.system_fonts, opts.font_dirs.clone());
    let m = DBS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut g = match m.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    if let Some(db) = g.get(&key) {
        return Some(db.clone());
    }
    let db = catch_unwind(|| {
        let mut db = if opts.system_fonts {
            (*system_db()).clone()
        } else {
            fontdb::Database::new()
        };
        for d in &opts.font_dirs {
            db.load_fonts_dir(d);
        }
        db
    })
    .unwrap_or_default();
    let db = Arc::new(db);
    if g.len() >= 8 {
        g.clear();
    }
    g.insert(key, db.clone());
    Some(db)
}

/// The fontdb used for SVG text: the system fonts plus Inter.
pub(crate) fn svg_db(system: bool) -> Arc<fontdb::Database> {
    static WITH: OnceLock<Arc<fontdb::Database>> = OnceLock::new();
    static WITHOUT: OnceLock<Arc<fontdb::Database>> = OnceLock::new();
    let slot = if system { &WITH } else { &WITHOUT };
    slot.get_or_init(|| {
        let mut db = if system {
            (*system_db()).clone()
        } else {
            fontdb::Database::new()
        };
        for data in INTER_FACES {
            db.load_font_data(data.to_vec());
        }
        db.set_sans_serif_family("Inter");
        Arc::new(db)
    })
    .clone()
}

/// The installed family whose name equals `name` ignoring ASCII case.
fn installed_family(db: &fontdb::Database, name: &str) -> Option<String> {
    db.faces()
        .flat_map(|f| f.families.iter())
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(n, _)| n.clone())
}

impl Fonts {
    pub fn new(opts: &PdfOptions) -> Result<Self, PdfError> {
        let mut f = Self {
            faces: Vec::new(),
            resolved: HashMap::new(),
            fallback: HashMap::new(),
            loaded_ids: HashMap::new(),
            files: HashMap::new(),
            loaded_bytes: 0,
            lookups: 0,
            db: database(opts),
            warnings: Vec::new(),
            missing_reported: false,
            limit_reported: false,
            fallback_searches: 0,
            missing_blocks: HashSet::new(),
        };
        // Inter faces 0–3: regular, bold, italic, bold italic.
        for (i, data) in INTER_FACES.iter().enumerate() {
            let face = Face::load(Arc::new(data.to_vec()), 0, "Inter")
                .ok_or_else(|| PdfError::Font(format!("embedded Inter face {i} is unreadable")))?;
            f.faces.push(face);
        }
        Ok(f)
    }

    pub fn inter(bold: bool, italic: bool) -> usize {
        match (bold, italic) {
            (false, false) => 0,
            (true, false) => 1,
            (false, true) => 2,
            (true, true) => 3,
        }
    }

    pub fn warn(&mut self, msg: String) {
        if !self.warnings.contains(&msg) && self.warnings.len() < 100 {
            self.warnings.push(msg);
        }
    }

    fn limit_reached(&mut self) {
        if !self.limit_reported {
            self.limit_reported = true;
            self.warn(
                "The document asks for more fonts than Tally loads for one print job; \
                 the rest was printed with Inter."
                    .into(),
            );
        }
    }

    /// The bytes of a font file, read once and counted against the
    /// limits.
    fn file_bytes(&mut self, path: &Path) -> Option<Arc<Vec<u8>>> {
        if let Some(d) = self.files.get(path) {
            return Some(d.clone());
        }
        let len = usize::try_from(std::fs::metadata(path).ok()?.len()).ok()?;
        if len > MAX_FONT_FILE_BYTES || self.loaded_bytes.saturating_add(len) > MAX_FONT_BYTES {
            self.limit_reached();
            return None;
        }
        let data = Arc::new(std::fs::read(path).ok()?);
        self.loaded_bytes += data.len();
        self.files.insert(path.to_path_buf(), data.clone());
        Some(data)
    }

    fn load_system(&mut self, id: fontdb::ID) -> Option<usize> {
        if let Some(i) = self.loaded_ids.get(&id) {
            return Some(*i);
        }
        if self.faces.len() >= 4 + MAX_SYSTEM_FACES {
            self.limit_reached();
            return None;
        }
        let db = self.db.clone()?;
        let info = db.face(id)?;
        let family = info
            .families
            .first()
            .map(|(n, _)| n.clone())
            .unwrap_or_default();
        let (source, index) = (info.source.clone(), info.index);
        let data: Arc<Vec<u8>> = match source {
            fontdb::Source::File(p) | fontdb::Source::SharedFile(p, _) => self.file_bytes(&p)?,
            fontdb::Source::Binary(b) => {
                let bytes = b.as_ref().as_ref();
                if self.loaded_bytes.saturating_add(bytes.len()) > MAX_FONT_BYTES {
                    self.limit_reached();
                    return None;
                }
                self.loaded_bytes += bytes.len();
                Arc::new(bytes.to_vec())
            }
        };
        let face = Face::load(data, index, &family)?;
        self.faces.push(face);
        let i = self.faces.len() - 1;
        self.loaded_ids.insert(id, i);
        Some(i)
    }

    /// The installed face for `family` (or a substitute), bold/italic as
    /// close as the family has.
    fn find_system(&mut self, family: &str, bold: bool, italic: bool) -> Option<usize> {
        let db = self.db.clone()?;
        let names: Vec<String> = family_candidates(family)
            .iter()
            .filter_map(|c| installed_family(&db, c))
            .collect();
        let families: Vec<fontdb::Family<'_>> =
            names.iter().map(|c| fontdb::Family::Name(c)).collect();
        if families.is_empty() {
            return None;
        }
        let q = fontdb::Query {
            families: &families,
            weight: if bold {
                fontdb::Weight::BOLD
            } else {
                fontdb::Weight::NORMAL
            },
            stretch: fontdb::Stretch::Normal,
            style: if italic {
                fontdb::Style::Italic
            } else {
                fontdb::Style::Normal
            },
        };
        let id = db.query(&q)?;
        self.load_system(id)
    }

    /// The face for a font spec (Inter when the family is unknown).
    pub fn resolve(&mut self, spec: &FontSpec) -> usize {
        let requested: String = spec
            .family
            .as_deref()
            .unwrap_or("")
            .trim()
            .chars()
            .take(MAX_FAMILY_LEN)
            .collect();
        let family = requested.to_ascii_lowercase();
        let key = (family.clone(), spec.bold, spec.italic);
        if let Some(i) = self.resolved.get(&key) {
            return *i;
        }
        let inter = Self::inter(spec.bold, spec.italic);
        let mut chosen = inter;
        let is_inter =
            INTER_ALIASES.contains(&family.as_str()) || requested.chars().any(char::is_control);
        if !is_inter {
            if self.lookups >= MAX_FONT_REQUESTS {
                self.limit_reached();
            } else {
                self.lookups += 1;
                match self.find_system(&requested, spec.bold, spec.italic) {
                    // A metric-compatible substitute is used silently.
                    Some(i) => chosen = i,
                    None => self.warn(format!(
                        "Font \"{requested}\" is not installed; Inter was used instead."
                    )),
                }
            }
        }
        self.resolved.insert(key, chosen);
        chosen
    }

    /// Width in points of one line of `text` in face `face` at `size`,
    /// without searching other fonts for missing characters (layout
    /// only needs widths): Inter's advance when Inter has the character,
    /// else an estimate (1 em for wide scripts, 0.6 em otherwise).
    pub fn measure_only(&mut self, face: usize, text: &str, size: f32) -> f32 {
        let face = face.min(self.faces.len().saturating_sub(1));
        let inter = 0;
        let mut em = 0.0f32;
        for ch in text.chars() {
            let ch = if ch == '\t' { ' ' } else { ch };
            if ch.is_control() {
                continue;
            }
            let f = &mut self.faces[face];
            if let Some((_, adv)) = f.glyph(ch) {
                em += f32::from(adv) / f.units_per_em;
            } else if let Some((_, adv)) = self.faces[inter].glyph(ch) {
                em += f32::from(adv) / self.faces[inter].units_per_em;
            } else {
                em += if u32::from(ch) >= 0x2E80 { 1.0 } else { 0.6 };
            }
        }
        em * size
    }

    /// What `spec` resolves to.
    pub fn resolved_font(&mut self, spec: &FontSpec) -> ResolvedFont {
        let i = self.resolve(spec);
        let f = &self.faces[i];
        ResolvedFont {
            family: f.family.clone(),
            bundled: i < 4,
            ascent_em: f.ascender / f.units_per_em,
            descent_em: -f.descender / f.units_per_em,
        }
    }

    /// A face that has `ch`, searched among the preferred fallback
    /// families and then every system face (bounded), cached per character.
    pub fn fallback_for(&mut self, ch: char) -> Option<usize> {
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
        let block = u32::from(ch) >> 7;
        let searchable = found.is_none()
            && self.fallback_searches < MAX_FALLBACK_SEARCHES
            && !self.missing_blocks.contains(&block);
        if searchable {
            self.fallback_searches += 1;
            if let Some(db) = self.db.clone() {
                let mut ids: Vec<fontdb::ID> = Vec::new();
                for fam in FALLBACK_FAMILIES {
                    let q = fontdb::Query {
                        families: &[fontdb::Family::Name(fam)],
                        ..fontdb::Query::default()
                    };
                    if let Some(id) = db.query(&q) {
                        ids.push(id);
                    }
                }
                ids.extend(db.faces().map(|f| f.id).take(MAX_FALLBACK_SCAN));
                let mut seen = HashSet::new();
                for id in ids {
                    if !seen.insert(id) {
                        continue;
                    }
                    let has = catch_unwind(AssertUnwindSafe(|| {
                        db.with_face_data(id, |data, index| {
                            ttf_parser::Face::parse(data, index)
                                .ok()
                                .is_some_and(|f| f.glyph_index(ch).is_some())
                        })
                        .unwrap_or(false)
                    }))
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
        if searchable && found.is_none() {
            self.missing_blocks.insert(block);
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

/// Runs `f` with the shared measuring registry for `opts` (one per option
/// set, kept for the process; at most a few sets).
pub(crate) fn with_measuring<T>(opts: &PdfOptions, f: impl FnOnce(&mut Fonts) -> T) -> Option<T> {
    type Key = (bool, Vec<PathBuf>);
    static CACHE: OnceLock<Mutex<HashMap<Key, Fonts>>> = OnceLock::new();
    let m = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = match m.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    // Limits are sticky: a context that reached one keeps using Inter for
    // new requests (rebuilding it would re-read the fonts again and again).
    // Layout and printing use their own short-lived [`FontContext`]s.
    let key = (opts.system_fonts, opts.font_dirs.clone());
    if !guard.contains_key(&key) {
        if guard.len() >= 4 {
            guard.clear();
        }
        guard.insert(key.clone(), Fonts::new(opts).ok()?);
    }
    let fonts = guard.get_mut(&key)?;
    catch_unwind(AssertUnwindSafe(|| f(fonts))).ok()
}

/// What `font` resolves to with the system fonts (`system_fonts`) or only
/// Inter: the family used and its ascent and descent. Font names are only
/// matched against installed family names.
pub fn resolve_font(font: &FontSpec, system_fonts: bool) -> ResolvedFont {
    resolve_font_with(
        font,
        &PdfOptions {
            system_fonts,
            ..PdfOptions::default()
        },
    )
}

/// [`resolve_font`] with explicit options (extra font directories).
pub fn resolve_font_with(font: &FontSpec, opts: &PdfOptions) -> ResolvedFont {
    with_measuring(opts, |f| f.resolved_font(font)).unwrap_or_else(|| ResolvedFont {
        family: "Inter".into(),
        bundled: true,
        ascent_em: 1984.0 / 2048.0,
        descent_em: 494.0 / 2048.0,
    })
}

/// A face resolved in a [`FontContext`].
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct FaceId(usize);

/// A font registry for one operation (a print job, an AutoFit, a
/// pagination): resolve each font once, then measure with the face.
/// Limits ([`MAX_SYSTEM_FACES`], [`MAX_FONT_BYTES`], [`MAX_FONT_REQUESTS`])
/// hold for the context's lifetime, and its font bytes are released with
/// it.
pub struct FontContext {
    fonts: Fonts,
}

impl FontContext {
    /// A context for `opts` (system fonts and/or extra directories).
    pub fn new(opts: &PdfOptions) -> Result<Self, PdfError> {
        Ok(Self {
            fonts: Fonts::new(opts)?,
        })
    }

    /// Resolves `spec` (Inter when the family is unknown or something
    /// fails) and describes the face used.
    pub fn face(&mut self, spec: &FontSpec) -> (FaceId, ResolvedFont) {
        let fonts = &mut self.fonts;
        let r = catch_unwind(AssertUnwindSafe(|| {
            let i = fonts.resolve(spec);
            (i, fonts.resolved_font(spec))
        }));
        match r {
            Ok((i, info)) => (FaceId(i), info),
            Err(_) => {
                let i = Fonts::inter(spec.bold, spec.italic);
                let f = &self.fonts.faces[i];
                let info = ResolvedFont {
                    family: "Inter".into(),
                    bundled: true,
                    ascent_em: f.ascender / f.units_per_em,
                    descent_em: -f.descender / f.units_per_em,
                };
                (FaceId(i), info)
            }
        }
    }

    /// Width in points of one line of `text` in `face` at `size_pt`
    /// (no per-character fallback search).
    pub fn width(&mut self, face: FaceId, text: &str, size_pt: f32) -> f32 {
        let size = size_pt.clamp(1.0, 400.0);
        let fonts = &mut self.fonts;
        catch_unwind(AssertUnwindSafe(|| fonts.measure_only(face.0, text, size)))
            .unwrap_or_else(|_| text.chars().count() as f32 * size * 0.55)
    }

    /// Font substitutions so far, one line each.
    pub fn warnings(&self) -> &[String] {
        &self.fonts.warnings
    }
}
