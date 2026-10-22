//! Document fonts in layout math (row auto-height, optimal column width,
//! printed optimal row heights).
//!
//! A cell's font is its style's family, else the document's default font
//! ([`Workbook::default_font`]), else the application font, Inter. With the
//! `pdf` feature a named family is resolved like the PDF writer resolves
//! it (installed fonts by family name, metric-compatible substitutes) and
//! its real advance widths and ascent + descent are used; Inter and
//! families that are not installed keep the built-in Inter numbers
//! ([`INTER_LINE_EM`], [`crate::autofit::char_em`]), so layout matches what
//! is drawn and printed.

use tally_model::StyleData;

use crate::workbook::Workbook;

/// Ascent plus descent of Inter (1984 + 494 of 2048 units), in em.
pub const INTER_LINE_EM: f32 = 1.21;

/// A font request: family (`None`: the application font), bold, italic.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct FontRef<'a> {
    /// Family name.
    pub family: Option<&'a str>,
    /// Bold.
    pub bold: bool,
    /// Italic.
    pub italic: bool,
}

impl<'a> FontRef<'a> {
    /// The font a cell with style `st` is drawn with in `wb`.
    pub fn of(wb: &'a Workbook, st: &'a StyleData) -> Self {
        Self {
            family: family(wb, st),
            bold: st.font.bold,
            italic: st.font.italic,
        }
    }

    /// The application font (Inter) with this weight.
    pub fn app(bold: bool) -> Self {
        Self {
            family: None,
            bold,
            italic: false,
        }
    }

    /// `true` when this names Inter (or nothing): the built-in numbers
    /// apply without a lookup.
    #[cfg_attr(not(feature = "pdf"), allow(dead_code))]
    fn is_app_font(&self) -> bool {
        self.family.is_none_or(|f| {
            let f = f.trim();
            f.is_empty()
                || f.eq_ignore_ascii_case("inter")
                || f.eq_ignore_ascii_case("inter variable")
        })
    }
}

/// The family cells with style `st` use: the style's, else the
/// document's default font.
pub fn family<'a>(wb: &'a Workbook, st: &'a StyleData) -> Option<&'a str> {
    st.font.family.as_deref().or(wb.default_font.as_deref())
}

/// A family as stored in a style: `None` when it is the document font
/// (or empty), so the document font is never baked into styles.
pub fn own_family(family: Option<String>, default_font: Option<&str>) -> Option<String> {
    family.filter(|f| {
        let f = f.trim();
        !f.is_empty() && default_font.is_none_or(|d| !f.eq_ignore_ascii_case(d.trim()))
    })
}

/// `true` when `patch` sets the family to the document font (or to
/// nothing): the styles then keep no family of their own.
pub fn patch_picks_document_font(
    patch: &tally_api::StylePatch,
    default_font: Option<&str>,
) -> bool {
    patch
        .font_family
        .as_ref()
        .is_some_and(|f| own_family(Some(f.clone()), default_font).is_none())
}

#[cfg(feature = "pdf")]
fn spec(f: FontRef<'_>, size_pt: f32) -> tally_io_pdf::FontSpec {
    tally_io_pdf::FontSpec {
        family: f.family.map(str::to_string),
        bold: f.bold,
        italic: f.italic,
        size_pt,
        ..tally_io_pdf::FontSpec::default()
    }
}

/// A font resolved by a [`Measurer`]: Inter's built-in numbers, or an
/// installed face.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Face {
    #[cfg(feature = "pdf")]
    id: Option<tally_io_pdf::FaceId>,
    /// Ascent plus descent in em.
    pub line_em: f32,
    bold: bool,
}

impl Face {
    /// Inter (the built-in width table and [`INTER_LINE_EM`]).
    pub fn app(bold: bool) -> Self {
        Self {
            #[cfg(feature = "pdf")]
            id: None,
            line_em: INTER_LINE_EM,
            bold,
        }
    }
}

/// Resolves fonts and measures text for one operation (an AutoFit, a
/// pagination): each font is looked up once (per style), the installed
/// fonts it loads are bounded and released when it is dropped, and
/// measuring never searches other fonts per character. Documents without
/// named fonts never touch the font files.
#[derive(Default)]
pub struct Measurer {
    #[cfg(feature = "pdf")]
    ctx: Option<Option<tally_io_pdf::FontContext>>,
    by_style: std::collections::HashMap<u32, Face>,
}

impl std::fmt::Debug for Measurer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Measurer")
            .field("styles", &self.by_style.len())
            .finish()
    }
}

impl Measurer {
    /// A measurer with nothing loaded yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// The face of `f`.
    pub fn face(&mut self, f: FontRef<'_>) -> Face {
        #[cfg(feature = "pdf")]
        if !f.is_app_font() {
            let ctx = self.ctx.get_or_insert_with(|| {
                tally_io_pdf::FontContext::new(&tally_io_pdf::PdfOptions::default()).ok()
            });
            if let Some(ctx) = ctx.as_mut() {
                let (id, r) = ctx.face(&spec(f, 10.0));
                let em = r.line_em();
                if !r.bundled && em.is_finite() && em > 0.5 {
                    return Face {
                        id: Some(id),
                        line_em: em.min(3.0),
                        bold: f.bold,
                    };
                }
            }
        }
        Face::app(f.bold)
    }

    /// The face cells with style `id` (data `st`) use in `wb`, cached per
    /// style for the measurer's lifetime.
    pub fn style_face(&mut self, wb: &Workbook, id: u32, st: &StyleData) -> Face {
        if let Some(f) = self.by_style.get(&id) {
            return *f;
        }
        let f = self.face(FontRef::of(wb, st));
        if self.by_style.len() < 100_000 {
            self.by_style.insert(id, f);
        }
        f
    }

    /// Width of one line of `text` in px at `size_px`.
    pub fn width_px(&mut self, face: Face, text: &str, size_px: f32) -> f32 {
        #[cfg(feature = "pdf")]
        if let (Some(id), Some(Some(ctx))) = (face.id, self.ctx.as_mut()) {
            let pt = size_px * 72.0 / 96.0;
            let w = ctx.width(id, text, pt) * 96.0 / 72.0;
            if w.is_finite() {
                return w;
            }
        }
        crate::autofit::text_width_px(text, size_px, face.bold)
    }
}

/// Height of one line of text (ascent plus descent) in em of the font
/// size: the resolved face's, else Inter's. One-off convenience; loops
/// use a [`Measurer`].
pub fn line_em(f: FontRef<'_>) -> f32 {
    Measurer::new().face(f).line_em
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_font_keeps_inter_numbers() {
        assert_eq!(line_em(FontRef::default()), INTER_LINE_EM);
        assert_eq!(line_em(FontRef::app(true)), INTER_LINE_EM);
        let inter = FontRef {
            family: Some("Inter"),
            ..FontRef::default()
        };
        assert_eq!(line_em(inter), INTER_LINE_EM);
        let mut m = Measurer::new();
        let face = m.face(inter);
        assert_eq!(face, Face::app(false));
        assert_eq!(
            m.width_px(face, "abc", 13.0),
            crate::autofit::text_width_px("abc", 13.0, false)
        );
    }

    #[test]
    fn style_family_overrides_the_document_default() {
        let mut wb = Workbook::new();
        let mut st = StyleData::default();
        assert_eq!(family(&wb, &st), None);
        wb.default_font = Some("Arial".into());
        assert_eq!(family(&wb, &st), Some("Arial"));
        st.font.family = Some("Courier New".into());
        assert_eq!(FontRef::of(&wb, &st).family, Some("Courier New"));
    }

    #[test]
    fn missing_family_keeps_inter_numbers() {
        let f = FontRef {
            family: Some("No Such Font Tally 42"),
            ..FontRef::default()
        };
        assert_eq!(line_em(f), INTER_LINE_EM);
        assert_eq!(Measurer::new().face(f), Face::app(false));
    }

    /// Arial's own metrics where it is installed (macOS, Windows).
    #[cfg(feature = "pdf")]
    #[test]
    fn installed_arial_is_lower_and_narrower_than_inter() {
        let arial = FontRef {
            family: Some("Arial"),
            ..FontRef::default()
        };
        if tally_io_pdf::resolve_font(&spec(arial, 10.0), true).family != "Arial" {
            return;
        }
        assert!((line_em(arial) - 2288.0 / 2048.0).abs() < 0.01);
        let mut m = Measurer::new();
        let face = m.face(arial);
        let w = m.width_px(face, "1,234,567.89", 13.0);
        assert!(w < crate::autofit::text_width_px("1,234,567.89", 13.0, false));
    }
}
