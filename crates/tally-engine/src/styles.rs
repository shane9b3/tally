//! The workbook style table: deduplicated [`StyleData`] entries addressed by
//! [`StyleId`], each with its parsed number format.

use std::collections::HashMap;
use std::sync::Arc;

use tally_api::{
    BorderLineDto, BorderStyleDto, BordersDto, DisplayDto, HAlign as ApiHAlign, StyleDto,
    StylePatch, VAlign as ApiVAlign,
};
use tally_model::{BorderLine, BorderStyle, DisplayData, HAlign, StyleData, VAlign};
use tally_numfmt::{parse_format, FormatAst, SectionKind};

use crate::storage::StyleId;

/// Default font size in points.
pub const DEFAULT_FONT_SIZE_PT: f32 = 11.0;

/// One style.
#[derive(Clone, Debug)]
pub struct StyleEntry {
    /// The style.
    pub data: StyleData,
    /// Number format code (`General` when unset).
    pub format_code: Arc<str>,
    /// Parsed number format.
    pub format: Arc<FormatAst>,
}

impl StyleEntry {
    /// `true` if the number format is General.
    pub fn is_general(&self) -> bool {
        self.format.is_general()
    }

    /// `true` if the number format is the text format `@`.
    pub fn is_text_format(&self) -> bool {
        self.format.sections.len() == 1 && matches!(self.format.sections[0].kind, SectionKind::Text)
    }
}

/// Deduplicating style table; id 0 is the default style.
#[derive(Clone, Debug)]
pub struct StyleTable {
    entries: Vec<StyleEntry>,
    default: StyleEntry,
    index: HashMap<String, StyleId>,
    formats: HashMap<String, Arc<FormatAst>>,
}

impl Default for StyleTable {
    fn default() -> Self {
        Self::new()
    }
}

fn key(data: &StyleData) -> String {
    format!("{data:?}")
}

fn general() -> Arc<FormatAst> {
    match parse_format("General") {
        Ok(ast) => Arc::new(ast),
        // parse_format("General") cannot fail; keep a fallback anyway.
        Err(_) => Arc::new(FormatAst {
            sections: Vec::new(),
        }),
    }
}

/// Normalises a style so equal-looking styles dedupe (`General` → `None`).
fn normalize(mut data: StyleData) -> StyleData {
    if let Some(code) = &data.number_format {
        if code.is_empty() || code.eq_ignore_ascii_case("General") {
            data.number_format = None;
        }
    }
    data
}

impl StyleTable {
    /// A table holding only the default style.
    pub fn new() -> Self {
        let d = StyleData::default();
        let g = general();
        let default = StyleEntry {
            data: d.clone(),
            format_code: "General".into(),
            format: g.clone(),
        };
        let mut t = Self {
            entries: vec![default.clone()],
            default,
            index: HashMap::new(),
            formats: HashMap::new(),
        };
        t.index.insert(key(&d), StyleId(0));
        t.formats.insert("General".into(), g);
        t
    }

    /// Number of styles.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Always `false`: the default style always exists.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The style `id` (the default style for unknown ids).
    pub fn get(&self, id: StyleId) -> &StyleEntry {
        self.entries.get(id.0 as usize).unwrap_or(&self.default)
    }

    /// `true` if `id` exists.
    pub fn contains(&self, id: u32) -> bool {
        (id as usize) < self.entries.len()
    }

    /// The parsed format for `code` (cached; invalid codes fall back to General).
    pub fn format(&mut self, code: &str) -> Arc<FormatAst> {
        if let Some(f) = self.formats.get(code) {
            return f.clone();
        }
        let f = parse_format(code)
            .map(Arc::new)
            .unwrap_or_else(|_| general());
        self.formats.insert(code.to_owned(), f.clone());
        f
    }

    /// Interns a style.
    pub fn intern(&mut self, data: StyleData) -> StyleId {
        let data = normalize(data);
        let k = key(&data);
        if let Some(id) = self.index.get(&k) {
            return *id;
        }
        let code: Arc<str> = data.number_format.as_deref().unwrap_or("General").into();
        let format = self.format(&code);
        let id = StyleId(self.entries.len() as u32);
        self.entries.push(StyleEntry {
            data,
            format_code: code,
            format,
        });
        self.index.insert(k, id);
        id
    }

    /// The style `id` with `patch` applied.
    pub fn patched(&mut self, id: StyleId, patch: &StylePatch) -> StyleId {
        let mut d = self.get(id).data.clone();
        apply_patch(&mut d, patch);
        self.intern(d)
    }

    /// The style `id` with number format `code`.
    pub fn with_format(&mut self, id: StyleId, code: &str) -> StyleId {
        let mut d = self.get(id).data.clone();
        d.number_format = Some(code.to_owned());
        self.intern(d)
    }

    /// The API view of style `id`.
    pub fn to_dto(&self, id: StyleId) -> StyleDto {
        let e = self.get(id);
        let d = &e.data;
        StyleDto {
            id: id.0,
            bold: d.font.bold,
            italic: d.font.italic,
            underline: d.font.underline,
            strike: d.font.strike,
            font_family: d.font.family.clone(),
            font_size_pt: d.font.size_pt.unwrap_or(DEFAULT_FONT_SIZE_PT),
            color: d.font.color,
            fill: d.fill,
            h_align: match d.align.h {
                HAlign::General => ApiHAlign::General,
                HAlign::Left => ApiHAlign::Left,
                HAlign::Center => ApiHAlign::Center,
                HAlign::Right => ApiHAlign::Right,
                HAlign::Justify => ApiHAlign::Justify,
            },
            v_align: match d.align.v {
                VAlign::Bottom => ApiVAlign::Bottom,
                VAlign::Middle => ApiVAlign::Middle,
                VAlign::Top => ApiVAlign::Top,
            },
            wrap: d.align.wrap,
            indent: d.align.indent,
            borders: BordersDto {
                top: d.borders.top.as_ref().map(border_dto),
                bottom: d.borders.bottom.as_ref().map(border_dto),
                left: d.borders.left.as_ref().map(border_dto),
                right: d.borders.right.as_ref().map(border_dto),
            },
            number_format: e.format_code.to_string(),
            display: d.display.as_ref().map(|x| match x {
                DisplayData::Chip { bg, fg } => DisplayDto::Chip { bg: *bg, fg: *fg },
            }),
        }
    }
}

fn border_dto(b: &BorderLine) -> BorderLineDto {
    BorderLineDto {
        style: match b.style {
            BorderStyle::Solid => BorderStyleDto::Solid,
            BorderStyle::Dashed => BorderStyleDto::Dashed,
            BorderStyle::Dotted => BorderStyleDto::Dotted,
            BorderStyle::Double => BorderStyleDto::Double,
        },
        width_px: crate::units::hmm_to_px(b.width_hmm),
        color: b.color,
    }
}

/// Applies the set fields of `p` to `d`.
pub fn apply_patch(d: &mut StyleData, p: &StylePatch) {
    if let Some(v) = p.bold {
        d.font.bold = v;
    }
    if let Some(v) = p.italic {
        d.font.italic = v;
    }
    if let Some(v) = p.underline {
        d.font.underline = v;
    }
    if let Some(v) = p.strike {
        d.font.strike = v;
    }
    if let Some(v) = &p.font_family {
        d.font.family = Some(v.clone());
    }
    if let Some(v) = p.font_size_pt {
        d.font.size_pt = Some(v);
    }
    if let Some(v) = p.color {
        d.font.color = Some(v);
    }
    if let Some(v) = p.fill {
        d.fill = Some(v);
    }
    if let Some(v) = p.h_align {
        d.align.h = match v {
            ApiHAlign::General => HAlign::General,
            ApiHAlign::Left => HAlign::Left,
            ApiHAlign::Center => HAlign::Center,
            ApiHAlign::Right => HAlign::Right,
            ApiHAlign::Justify => HAlign::Justify,
        };
    }
    if let Some(v) = p.v_align {
        d.align.v = match v {
            ApiVAlign::Bottom => VAlign::Bottom,
            ApiVAlign::Middle => VAlign::Middle,
            ApiVAlign::Top => VAlign::Top,
        };
    }
    if let Some(v) = p.wrap {
        d.align.wrap = v;
    }
    if let Some(v) = &p.number_format {
        d.number_format = Some(v.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedupe_and_patch() {
        let mut t = StyleTable::new();
        assert_eq!(t.len(), 1);
        let bold = StylePatch {
            bold: Some(true),
            ..StylePatch::default()
        };
        let a = t.patched(StyleId(0), &bold);
        let b = t.patched(StyleId(0), &bold);
        assert_eq!(a, b);
        assert_ne!(a, StyleId(0));
        let unbold = StylePatch {
            bold: Some(false),
            ..StylePatch::default()
        };
        assert_eq!(t.patched(a, &unbold), StyleId(0));
        let g = t.with_format(StyleId(0), "General");
        assert_eq!(g, StyleId(0), "General normalises to the default");
        let pct = t.with_format(StyleId(0), "0%");
        assert_eq!(t.to_dto(pct).number_format, "0%");
        assert!(t.get(pct).format.sections.len() == 1);
        assert!(t.to_dto(a).bold);
    }
}
