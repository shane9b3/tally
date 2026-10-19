//! The workbook style table: deduplicated [`StyleData`] entries addressed by
//! [`StyleId`], each with its parsed number format.

use std::collections::HashMap;
use std::sync::Arc;

use tally_api::{
    BorderEdgePatch, BorderLineDto, BorderStyleDto, BordersDto, BordersPatch, DisplayDto,
    HAlign as ApiHAlign, StyleDto, StylePatch, VAlign as ApiVAlign,
};
use tally_model::{BorderLine, BorderStyle, DisplayData, HAlign, StyleData, VAlign};
use tally_numfmt::{parse_format, FormatAst, SectionKind};
use tally_types::CellRange;

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
        if code.is_empty()
            || code.eq_ignore_ascii_case("General")
            || code.chars().count() > tally_model::MAX_NUMBER_FORMAT_LEN
        {
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

    /// The style `id` with the given border edges replaced (`None` edges
    /// are left unchanged; an edge with `line: None` removes the border).
    pub fn with_borders(&mut self, id: StyleId, edges: [Option<&BorderEdgePatch>; 4]) -> StyleId {
        let mut d = self.get(id).data.clone();
        let [top, bottom, left, right] = edges;
        let set = |slot: &mut Option<BorderLine>, e: Option<&BorderEdgePatch>| {
            if let Some(e) = e {
                *slot = e.line.as_ref().map(border_from_dto);
            }
        };
        set(&mut d.borders.top, top);
        set(&mut d.borders.bottom, bottom);
        set(&mut d.borders.left, left);
        set(&mut d.borders.right, right);
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
        style_dto(id.0, &e.data, &e.format_code)
    }
}

/// The API view of a style with id `id`.
pub fn style_dto(id: u32, d: &StyleData, format_code: &str) -> StyleDto {
    StyleDto {
        id,
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
        number_format: format_code.to_string(),
        display: d.display.as_ref().map(|x| match x {
            DisplayData::Chip { bg, fg } => DisplayDto::Chip { bg: *bg, fg: *fg },
        }),
    }
}

/// Model border line → API border line.
pub fn border_dto(b: &BorderLine) -> BorderLineDto {
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

/// API border line → model border line.
pub fn border_from_dto(b: &BorderLineDto) -> BorderLine {
    BorderLine {
        style: match b.style {
            BorderStyleDto::Solid => BorderStyle::Solid,
            BorderStyleDto::Dashed => BorderStyle::Dashed,
            BorderStyleDto::Dotted => BorderStyle::Dotted,
            BorderStyleDto::Double => BorderStyle::Double,
        },
        width_hmm: crate::units::px_to_hmm(b.width_px.max(0.0)).max(1),
        color: b.color,
    }
}

/// Splits `lo..=hi` into (first line, inner lines, last line) segments as
/// `(lo, hi, is_first, is_last)`.
fn segments(lo: u32, hi: u32) -> Vec<(u32, u32, bool, bool)> {
    if lo == hi {
        return vec![(lo, lo, true, true)];
    }
    let mut v = vec![(lo, lo, true, false)];
    if hi - lo >= 2 {
        v.push((lo + 1, hi - 1, false, false));
    }
    v.push((hi, hi, false, true));
    v
}

/// Applies a border patch to `range`: outer edges on the cells along the
/// range's sides, inner lines on the cells between (each cell stores its
/// own four edges, as in AOO; neighbours outside the range are untouched).
pub fn apply_borders(wb: &mut crate::workbook::Workbook, range: CellRange, b: &BordersPatch) {
    for (r0, r1, rf, rl) in segments(range.r0, range.r1) {
        for (c0, c1, cf, cl) in segments(range.c0, range.c1) {
            let top = if rf {
                b.top.as_ref()
            } else {
                b.inner_h.as_ref()
            };
            let bottom = if rl {
                b.bottom.as_ref()
            } else {
                b.inner_h.as_ref()
            };
            let left = if cf {
                b.left.as_ref()
            } else {
                b.inner_v.as_ref()
            };
            let right = if cl {
                b.right.as_ref()
            } else {
                b.inner_v.as_ref()
            };
            if top.is_none() && bottom.is_none() && left.is_none() && right.is_none() {
                continue;
            }
            let sub = CellRange::new(range.sheet, r0, c0, r1, c1);
            crate::ops::map_styles(wb, sub, |t, old| {
                t.with_borders(old, [top, bottom, left, right])
            });
        }
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
    fn border_patches_per_edge() {
        let mut wb = crate::workbook::Workbook::new();
        let line = BorderLineDto {
            style: BorderStyleDto::Solid,
            width_px: 1.0,
            color: 0x0000_00ff,
        };
        let edge = |l: Option<BorderLineDto>| Some(BorderEdgePatch { line: l });
        let patch = BordersPatch {
            top: edge(Some(line.clone())),
            bottom: edge(Some(line.clone())),
            left: None,
            right: None,
            inner_h: edge(Some(BorderLineDto {
                style: BorderStyleDto::Dotted,
                ..line.clone()
            })),
            inner_v: None,
        };
        let range = CellRange::new(1, 0, 0, 2, 1);
        apply_borders(&mut wb, range, &patch);
        fn b(wb: &crate::workbook::Workbook, r: u32, c: u32) -> BordersDto {
            wb.styles
                .to_dto(wb.style_at(tally_types::CellAddr::new(1, r, c)))
                .borders
        }
        assert_eq!(
            b(&wb, 0, 0).top.map(|l| l.style),
            Some(BorderStyleDto::Solid)
        );
        assert_eq!(
            b(&wb, 0, 0).bottom.map(|l| l.style),
            Some(BorderStyleDto::Dotted)
        );
        assert_eq!(
            b(&wb, 1, 1).top.map(|l| l.style),
            Some(BorderStyleDto::Dotted)
        );
        assert_eq!(
            b(&wb, 2, 1).bottom.map(|l| l.style),
            Some(BorderStyleDto::Solid)
        );
        assert!(b(&wb, 1, 0).left.is_none());
        assert!(b(&wb, 3, 0).top.is_none(), "outside the range");
        // Removing the top edge only.
        let remove = BordersPatch {
            top: edge(None),
            ..BordersPatch::default()
        };
        apply_borders(&mut wb, range, &remove);
        assert!(b(&wb, 0, 0).top.is_none());
        assert!(b(&wb, 0, 0).bottom.is_some());
    }

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
