//! `xl/styles.xml` → model styles.
//!
//! `cellXfs` are used for cells (each cell's `s` indexes it) and `dxfs`
//! (differential formats) for conditional formatting; the named cell
//! styles (`cellStyleXfs`) and table styles are ignored. The `applyX` flags of a cell
//! xf are not consulted: a cell xf's ids are the effective formatting, which
//! is what Excel displays (the flags only say whether the xf overrides its
//! parent cell style).

use std::collections::HashMap;
use std::io::BufRead;

use tally_model::{
    AlignData, BorderLine, BorderStyle, BordersData, CellProtectionData, CfStyleData, FontData,
    HAlign, StyleData, VAlign,
};

use crate::colors::{XColor, INDEXED};
use crate::numfmt::{builtin_format_code, excel_to_ooo, is_date_format};
use crate::units::parse_bool;
use crate::xml::{Elem, Ev, XmlReader};
use crate::XlsxError;

/// Most entries accepted in any one styles list (Excel itself allows 64,000
/// cell formats).
const MAX_STYLE_ENTRIES: usize = 1_000_000;

#[derive(Clone, Debug, Default)]
struct RawFont {
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
    size: Option<f32>,
    name: Option<String>,
    color: Option<XColor>,
}

#[derive(Clone, Debug, Default)]
struct RawXf {
    num_fmt: u32,
    font: usize,
    fill: usize,
    border: usize,
    align: AlignData,
    protection: Option<CellProtectionData>,
}

/// A differential format as read.
#[derive(Clone, Debug, Default)]
struct RawDxf {
    font: Option<RawFont>,
    fill: Option<XColor>,
    border: Option<RawBorders>,
    num_fmt: Option<String>,
}

/// The cell formats of a workbook, mapped to the model style table.
#[derive(Clone, Debug, Default)]
pub(crate) struct StyleTable {
    /// Deduplicated model styles.
    pub styles: Vec<StyleData>,
    /// `cellXfs` index → model style (`None` for the default look).
    pub xf_style: Vec<Option<u32>>,
    /// `cellXfs` index → the number format shows a calendar date.
    pub xf_date: Vec<bool>,
    /// `dxfs` (conditional formatting styles).
    pub dxfs: Vec<CfStyleData>,
}

impl StyleTable {
    pub fn style_of(&self, xf: u32) -> Option<u32> {
        self.xf_style.get(xf as usize).copied().flatten()
    }

    pub fn is_date(&self, xf: u32) -> bool {
        self.xf_date.get(xf as usize).copied().unwrap_or(false)
    }
}

#[derive(Default)]
struct Raw {
    num_fmts: HashMap<u32, String>,
    fonts: Vec<RawFont>,
    fills: Vec<Option<XColor>>,
    borders: Vec<RawBorders>,
    xfs: Vec<RawXf>,
    dxfs: Vec<RawDxf>,
    indexed: Option<Vec<u32>>,
}

fn too_many() -> XlsxError {
    XlsxError::LimitExceeded {
        what: "style entries",
        limit: MAX_STYLE_ENTRIES as u64,
    }
}

fn color_of(e: &Elem) -> XColor {
    XColor::from_attrs(|k| e.attr_local(k).map(str::to_string))
}

/// Calls `f` for every child element start of the element whose start was
/// just returned; `f` must consume the child (skip it or read it).
fn children<R: BufRead>(
    xr: &mut XmlReader<R>,
    mut f: impl FnMut(&mut XmlReader<R>, Elem) -> Result<(), XlsxError>,
) -> Result<(), XlsxError> {
    let base = xr.depth();
    loop {
        match xr.next()? {
            Ev::Start(e) => f(xr, e)?,
            Ev::End if xr.depth() < base => return Ok(()),
            Ev::Eof => return Ok(()),
            _ => {}
        }
    }
}

fn val_bool(e: &Elem) -> bool {
    e.attr_local("val").is_none_or(parse_bool)
}

fn parse_font<R: BufRead>(xr: &mut XmlReader<R>) -> Result<RawFont, XlsxError> {
    let mut f = RawFont::default();
    children(xr, |xr, e| {
        match e.local() {
            "b" => f.bold = val_bool(&e),
            "i" => f.italic = val_bool(&e),
            "strike" => f.strike = val_bool(&e),
            "u" => f.underline = e.attr_local("val") != Some("none"),
            "sz" => {
                f.size = e
                    .attr_local("val")
                    .and_then(|v| v.trim().parse::<f32>().ok())
                    .filter(|s| s.is_finite() && *s > 0.0 && *s < 4096.0);
            }
            "name" | "rFont" => f.name = e.attr_local("val").map(str::to_string),
            "color" => f.color = Some(color_of(&e)),
            _ => {}
        }
        xr.skip()
    })?;
    Ok(f)
}

fn parse_fill<R: BufRead>(xr: &mut XmlReader<R>) -> Result<Option<XColor>, XlsxError> {
    let mut fill = None;
    children(xr, |xr, e| {
        if e.local() == "patternFill" {
            let pattern = e.attr_local("patternType").unwrap_or("none").to_string();
            let mut fg = None;
            let mut bg = None;
            children(xr, |xr, c| {
                match c.local() {
                    "fgColor" => fg = Some(color_of(&c)),
                    "bgColor" => bg = Some(color_of(&c)),
                    _ => {}
                }
                xr.skip()
            })?;
            fill = match pattern.as_str() {
                "none" => None,
                // Solid uses the foreground colour; other patterns are
                // approximated by it (or the background if absent).
                _ => fg.or(bg),
            };
            Ok(())
        } else {
            xr.skip()
        }
    })?;
    Ok(fill)
}

type RawBorders = [Option<(String, Option<XColor>)>; 4];

fn parse_border<R: BufRead>(xr: &mut XmlReader<R>) -> Result<RawBorders, XlsxError> {
    let mut b: RawBorders = Default::default();
    children(xr, |xr, e| {
        // top, bottom, left, right (start/end are the bidi spellings).
        let slot = match e.local() {
            "top" => Some(0),
            "bottom" => Some(1),
            "left" | "start" => Some(2),
            "right" | "end" => Some(3),
            _ => None,
        };
        let Some(slot) = slot else {
            return xr.skip();
        };
        let style = e.attr_local("style").unwrap_or("none").to_string();
        let mut color = None;
        children(xr, |xr, c| {
            if c.local() == "color" {
                color = Some(color_of(&c));
            }
            xr.skip()
        })?;
        if style != "none" {
            b[slot] = Some((style, color));
        }
        Ok(())
    })?;
    Ok(b)
}

fn parse_alignment(e: &Elem) -> AlignData {
    let h = match e.attr_local("horizontal").unwrap_or("general") {
        "left" => HAlign::Left,
        "center" | "centerContinuous" => HAlign::Center,
        "right" => HAlign::Right,
        "justify" | "distributed" => HAlign::Justify,
        // `fill` repeats the content; closest is left.
        "fill" => HAlign::Left,
        _ => HAlign::General,
    };
    let v = match e.attr_local("vertical").unwrap_or("bottom") {
        "top" => VAlign::Top,
        "center" | "justify" | "distributed" => VAlign::Middle,
        _ => VAlign::Bottom,
    };
    AlignData {
        h,
        v,
        wrap: e.attr_local("wrapText").is_some_and(parse_bool),
        indent: e
            .attr_local("indent")
            .and_then(|v| v.trim().parse::<u32>().ok())
            .map_or(0, |i| u8::try_from(i.min(250)).unwrap_or(250)),
    }
}

fn parse_xf<R: BufRead>(xr: &mut XmlReader<R>, e: &Elem) -> Result<RawXf, XlsxError> {
    let id = |k: &str| {
        e.attr_local(k)
            .and_then(|v| v.trim().parse::<u32>().ok())
            .unwrap_or(0)
    };
    let mut xf = RawXf {
        num_fmt: id("numFmtId"),
        font: id("fontId") as usize,
        fill: id("fillId") as usize,
        border: id("borderId") as usize,
        align: AlignData::default(),
        protection: None,
    };
    children(xr, |xr, c| {
        if c.local() == "alignment" {
            xf.align = parse_alignment(&c);
        }
        if c.local() == "protection" {
            let p = CellProtectionData {
                locked: c.attr_local("locked").is_none_or(parse_bool),
                hidden_formula: c.attr_local("hidden").is_some_and(parse_bool),
                ..CellProtectionData::default()
            };
            xf.protection = (p != CellProtectionData::default()).then_some(p);
        }
        xr.skip()
    })?;
    Ok(xf)
}

fn push_checked<T>(v: &mut Vec<T>, x: T) -> Result<(), XlsxError> {
    if v.len() >= MAX_STYLE_ENTRIES {
        return Err(too_many());
    }
    v.push(x);
    Ok(())
}

fn parse_raw<R: BufRead>(xr: &mut XmlReader<R>) -> Result<Raw, XlsxError> {
    let mut raw = Raw::default();
    loop {
        match xr.next()? {
            Ev::Start(e) => match e.local() {
                "styleSheet" => {}
                "numFmts" => children(xr, |xr, c| {
                    if c.local() == "numFmt" {
                        if let (Some(id), Some(code)) = (
                            c.attr_local("numFmtId").and_then(|v| v.trim().parse().ok()),
                            c.attr_local("formatCode"),
                        ) {
                            if raw.num_fmts.len() >= MAX_STYLE_ENTRIES {
                                return Err(too_many());
                            }
                            raw.num_fmts.insert(id, code.to_string());
                        }
                    }
                    xr.skip()
                })?,
                "fonts" => children(xr, |xr, c| {
                    if c.local() == "font" {
                        let f = parse_font(xr)?;
                        push_checked(&mut raw.fonts, f)
                    } else {
                        xr.skip()
                    }
                })?,
                "fills" => children(xr, |xr, c| {
                    if c.local() == "fill" {
                        let f = parse_fill(xr)?;
                        push_checked(&mut raw.fills, f)
                    } else {
                        xr.skip()
                    }
                })?,
                "borders" => children(xr, |xr, c| {
                    if c.local() == "border" {
                        let b = parse_border(xr)?;
                        push_checked(&mut raw.borders, b)
                    } else {
                        xr.skip()
                    }
                })?,
                "cellXfs" => children(xr, |xr, c| {
                    if c.local() == "xf" {
                        let xf = parse_xf(xr, &c)?;
                        push_checked(&mut raw.xfs, xf)
                    } else {
                        xr.skip()
                    }
                })?,
                "dxfs" => children(xr, |xr, c| {
                    if c.local() == "dxf" {
                        let d = parse_dxf(xr)?;
                        push_checked(&mut raw.dxfs, d)
                    } else {
                        xr.skip()
                    }
                })?,
                "colors" => children(xr, |xr, c| {
                    if c.local() == "indexedColors" {
                        let mut pal = Vec::new();
                        children(xr, |xr, rgb| {
                            if rgb.local() == "rgbColor" && pal.len() < 64 {
                                pal.push(
                                    rgb.attr_local("rgb")
                                        .and_then(crate::colors::parse_argb)
                                        .unwrap_or(0),
                                );
                            }
                            xr.skip()
                        })?;
                        raw.indexed = Some(pal);
                        Ok(())
                    } else {
                        xr.skip()
                    }
                })?,
                _ => xr.skip()?,
            },
            Ev::Eof => return Ok(raw),
            _ => {}
        }
    }
}

fn parse_dxf<R: BufRead>(xr: &mut XmlReader<R>) -> Result<RawDxf, XlsxError> {
    let mut d = RawDxf::default();
    children(xr, |xr, e| match e.local() {
        "font" => {
            d.font = Some(parse_font(xr)?);
            Ok(())
        }
        "fill" => {
            // Differential solid fills use the background colour.
            children(xr, |xr, p| {
                if p.local() == "patternFill" {
                    let mut fg = None;
                    let mut bg = None;
                    children(xr, |xr, c| {
                        match c.local() {
                            "fgColor" => fg = Some(color_of(&c)),
                            "bgColor" => bg = Some(color_of(&c)),
                            _ => {}
                        }
                        xr.skip()
                    })?;
                    d.fill = bg.or(fg);
                    Ok(())
                } else {
                    xr.skip()
                }
            })
        }
        "border" => {
            d.border = Some(parse_border(xr)?);
            Ok(())
        }
        "numFmt" => {
            d.num_fmt = e.attr_local("formatCode").map(str::to_string);
            xr.skip()
        }
        _ => xr.skip(),
    })?;
    Ok(d)
}

/// Excel border style → (model style, width in 1/100 mm).
fn border_line(style: &str) -> (BorderStyle, u32) {
    match style {
        "thin" => (BorderStyle::Solid, 26),
        "medium" => (BorderStyle::Solid, 53),
        "thick" => (BorderStyle::Solid, 79),
        "hair" => (BorderStyle::Dotted, 9),
        "dotted" => (BorderStyle::Dotted, 26),
        "double" => (BorderStyle::Double, 79),
        "dashed" | "dashDot" | "dashDotDot" => (BorderStyle::Dashed, 26),
        "mediumDashed" | "mediumDashDot" | "mediumDashDotDot" | "slantDashDot" => {
            (BorderStyle::Dashed, 53)
        }
        _ => (BorderStyle::Solid, 26),
    }
}

/// Builds the model style table from `styles.xml` (or the defaults when the
/// part is missing).
pub(crate) fn parse_styles<R: BufRead>(xr: &mut XmlReader<R>) -> Result<StyleTable, XlsxError> {
    let raw = parse_raw(xr)?;
    Ok(build(&raw))
}

fn build(raw: &Raw) -> StyleTable {
    let indexed: Vec<u32> = match &raw.indexed {
        Some(p) => {
            let mut v = INDEXED.to_vec();
            for (i, c) in p.iter().enumerate() {
                if let Some(slot) = v.get_mut(i) {
                    *slot = *c;
                }
            }
            v
        }
        None => INDEXED.to_vec(),
    };
    let default_font = raw.fonts.first().cloned().unwrap_or_default();
    let mut table = StyleTable::default();
    let mut dedup: HashMap<String, u32> = HashMap::new();
    let mut fmt_cache: HashMap<u32, Option<String>> = HashMap::new();
    for xf in &raw.xfs {
        let rf = raw.fonts.get(xf.font).cloned().unwrap_or_default();
        let font = FontData {
            bold: rf.bold,
            italic: rf.italic,
            underline: rf.underline,
            strike: rf.strike,
            family: rf
                .name
                .clone()
                .filter(|n| Some(n) != default_font.name.as_ref()),
            size_pt: rf.size.filter(|s| Some(*s) != default_font.size),
            color: rf
                .color
                .filter(|c| !c.is_automatic())
                .and_then(|c| c.resolve(&indexed)),
        };
        let fill = raw
            .fills
            .get(xf.fill)
            .copied()
            .flatten()
            .and_then(|c| c.resolve(&indexed));
        let mut borders = BordersData::default();
        if let Some(b) = raw.borders.get(xf.border) {
            let line = |slot: &Option<(String, Option<XColor>)>| {
                slot.as_ref().map(|(style, color)| {
                    let (style, width_hmm) = border_line(style);
                    let color = color
                        .filter(|c| !c.is_automatic())
                        .and_then(|c| c.resolve(&indexed))
                        .unwrap_or(0x0000_00ff);
                    BorderLine {
                        style,
                        width_hmm,
                        color,
                    }
                })
            };
            borders.top = line(&b[0]);
            borders.bottom = line(&b[1]);
            borders.left = line(&b[2]);
            borders.right = line(&b[3]);
        }
        let number_format = fmt_cache
            .entry(xf.num_fmt)
            .or_insert_with(|| {
                let code = raw
                    .num_fmts
                    .get(&xf.num_fmt)
                    .map(String::as_str)
                    .or_else(|| builtin_format_code(xf.num_fmt))?;
                excel_to_ooo(code)
            })
            .clone();
        let is_date = number_format.as_deref().is_some_and(is_date_format);
        let style = StyleData {
            protection: xf.protection,
            font,
            fill,
            align: xf.align.clone(),
            borders,
            number_format,
            display: None,
        };
        table.xf_date.push(is_date);
        if style == StyleData::default() {
            table.xf_style.push(None);
            continue;
        }
        let key = format!("{style:?}");
        let idx = *dedup.entry(key).or_insert_with(|| {
            table.styles.push(style);
            u32::try_from(table.styles.len() - 1).unwrap_or(u32::MAX)
        });
        table.xf_style.push(Some(idx));
    }
    for d in &raw.dxfs {
        let resolve = |c: &XColor| c.resolve(&indexed);
        let mut st = CfStyleData::default();
        if let Some(f) = &d.font {
            st.bold = f.bold.then_some(true);
            st.italic = f.italic.then_some(true);
            st.underline = f.underline.then_some(true);
            st.strike = f.strike.then_some(true);
            st.color = f
                .color
                .filter(|c| !c.is_automatic())
                .and_then(|c| resolve(&c));
        }
        st.fill = d.fill.and_then(|c| resolve(&c));
        if let Some(b) = &d.border {
            let line = |slot: &Option<(String, Option<XColor>)>| {
                slot.as_ref().map(|(style, color)| {
                    let (style, width_hmm) = border_line(style);
                    BorderLine {
                        style,
                        width_hmm,
                        color: color.and_then(|c| resolve(&c)).unwrap_or(0x0000_00ff),
                    }
                })
            };
            let bd = BordersData {
                top: line(&b[0]),
                bottom: line(&b[1]),
                left: line(&b[2]),
                right: line(&b[3]),
            };
            st.borders = (bd != BordersData::default()).then_some(bd);
        }
        st.number_format = d.num_fmt.as_deref().and_then(excel_to_ooo);
        table.dxfs.push(st);
    }
    table
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xml::XmlLimits;

    const STYLES: &str = concat!(
        r#"<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">"#,
        r#"<numFmts count="1"><numFmt numFmtId="164" formatCode="yyyy\-mm\-dd"/></numFmts>"#,
        r#"<fonts count="2"><font><sz val="11"/><color theme="1"/><name val="Calibri"/></font>"#,
        r#"<font><b/><i val="1"/><u/><strike val="0"/><sz val="14"/><color rgb="FFFF0000"/><name val="Arial"/></font></fonts>"#,
        r#"<fills count="3"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill>"#,
        r#"<fill><patternFill patternType="solid"><fgColor theme="4" tint="0.79998168889431442"/><bgColor indexed="64"/></patternFill></fill></fills>"#,
        r#"<borders count="2"><border><left/><right/><top/><bottom/><diagonal/></border>"#,
        r#"<border><left style="thin"><color indexed="64"/></left><right style="medium"><color rgb="FF00FF00"/></right><top/><bottom style="double"/></border></borders>"#,
        r#"<cellStyleXfs count="1"><xf numFmtId="0" fontId="1" fillId="2" borderId="1"/></cellStyleXfs>"#,
        r#"<cellXfs count="5"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/>"#,
        r#"<xf numFmtId="164" fontId="0" fillId="0" borderId="0" applyNumberFormat="1"/>"#,
        r#"<xf numFmtId="4" fontId="1" fillId="2" borderId="1" applyFont="1"><alignment horizontal="center" vertical="top" wrapText="1" indent="2"/></xf>"#,
        r#"<xf numFmtId="14" fontId="0" fillId="0" borderId="0"/>"#,
        r#"<xf numFmtId="46" fontId="0" fillId="0" borderId="0"/></cellXfs>"#,
        r#"<cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles>"#,
        r#"<dxfs count="0"/></styleSheet>"#
    );

    fn table() -> StyleTable {
        let mut xr = XmlReader::new(
            STYLES.as_bytes(),
            XmlLimits {
                max_depth: 64,
                max_attrs: 64,
            },
        );
        parse_styles(&mut xr).unwrap_or_default()
    }

    #[test]
    fn cell_xfs_map_to_model_styles() {
        let t = table();
        assert_eq!(t.xf_style.len(), 5);
        assert_eq!(t.style_of(0), None);
        let date = &t.styles[t.style_of(1).unwrap_or(99) as usize];
        assert_eq!(date.number_format.as_deref(), Some("YYYY\\-MM\\-DD"));
        assert!(t.is_date(1) && t.is_date(3) && !t.is_date(4) && !t.is_date(2));
        let s = &t.styles[t.style_of(2).unwrap_or(99) as usize];
        assert!(s.font.bold && s.font.italic && s.font.underline && !s.font.strike);
        assert_eq!(s.font.family.as_deref(), Some("Arial"));
        assert_eq!(s.font.size_pt, Some(14.0));
        assert_eq!(s.font.color, Some(0xff0000ff));
        assert_eq!(s.fill, Some(0xdae3f3ff));
        assert_eq!(s.align.h, HAlign::Center);
        assert_eq!(s.align.v, VAlign::Top);
        assert!(s.align.wrap);
        assert_eq!(s.align.indent, 2);
        assert_eq!(s.number_format.as_deref(), Some("#,##0.00"));
        let left = s
            .borders
            .left
            .as_ref()
            .map(|b| (b.style, b.width_hmm, b.color));
        assert_eq!(left, Some((BorderStyle::Solid, 26, 0x000000ff)));
        let right = s.borders.right.as_ref().map(|b| (b.width_hmm, b.color));
        assert_eq!(right, Some((53, 0x00ff00ff)));
        assert_eq!(
            s.borders.bottom.as_ref().map(|b| b.style),
            Some(BorderStyle::Double)
        );
        assert!(s.borders.top.is_none());
        // Identical xfs share a model style.
        assert_eq!(t.styles.len(), 4);
    }
}
