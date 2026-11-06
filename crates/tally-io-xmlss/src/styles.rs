//! `ss:Styles/ss:Style` → model styles.
//!
//! Each `ss:Style` is read as a set of optional properties ([`RawStyle`]).
//! A style inherits from its `ss:Parent`, and every style without a parent
//! from `Default` (template `ss:Style` of `spreadsheetml2ooo.xsl`: "no
//! parent, but automatic style are automatically inheriting from a style
//! called 'Default'"). Parent chains are resolved iteratively, at most
//! [`MAX_PARENT_DEPTH`] levels, and a cycle stops the walk (with a warning).

use std::collections::{HashMap, HashSet};
use std::io::BufRead;

use tally_model::{
    AlignData, BorderLine, BorderStyle, BordersData, CellProtectionData, FontData, HAlign,
    StyleData, UnderlineStyleData, VAlign, MAX_NUMBER_FORMAT_LEN,
};

use crate::xml::{Elem, Ev, XmlReader};
use crate::XmlssError;

/// Longest `ss:Parent` chain followed.
pub(crate) const MAX_PARENT_DEPTH: usize = 64;

/// Longest style id kept (longer ids are ignored, so the cell is unstyled).
pub(crate) const MAX_STYLE_ID_BYTES: usize = 1024;

/// One border as read: (style, width, colour).
type RawBorder = Option<BorderLine>;

/// The properties one `ss:Style` sets (`None` = inherited).
#[derive(Clone, Debug, Default)]
pub(crate) struct RawStyle {
    pub parent: Option<String>,
    bold: Option<bool>,
    italic: Option<bool>,
    underline: Option<Option<UnderlineStyleData>>,
    strike: Option<bool>,
    family: Option<String>,
    size: Option<f32>,
    color: Option<Option<u32>>,
    /// `Some(None)`: an `ss:Interior` without a fill.
    fill: Option<Option<u32>>,
    /// `ss:Borders` replaces the inherited set: top, bottom, left, right,
    /// diagonal down, diagonal up.
    borders: Option<[RawBorder; 6]>,
    /// `Some(None)`: General.
    number_format: Option<Option<String>>,
    h: Option<HAlign>,
    v: Option<VAlign>,
    wrap: Option<bool>,
    indent: Option<u8>,
    rotation: Option<i16>,
    stacked: Option<bool>,
    shrink: Option<bool>,
    locked: Option<bool>,
    hide_formula: Option<bool>,
}

/// `#RRGGBB` → `0xRRGGBBAA` (opaque); `Automatic` and junk → `None`.
pub(crate) fn parse_color(s: &str) -> Option<u32> {
    let hex = s.trim().strip_prefix('#')?;
    if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(hex, 16).ok().map(|c| (c << 8) | 0xff)
}

/// `1`/`true` (Excel writes `1`).
pub(crate) fn parse_bool(s: &str) -> bool {
    matches!(s.trim(), "1" | "true" | "True" | "TRUE")
}

/// Parses one `ss:Style` (its start tag was just read).
pub(crate) fn parse_style<R: BufRead>(
    xr: &mut XmlReader<R>,
    e: &Elem,
) -> Result<RawStyle, XmlssError> {
    let mut st = RawStyle {
        parent: e
            .attr("Parent")
            .filter(|p| p.len() <= MAX_STYLE_ID_BYTES)
            .map(str::to_string),
        ..RawStyle::default()
    };
    let base = xr.depth();
    loop {
        match xr.next()? {
            Ev::Start(c) => {
                match c.local() {
                    "Font" => font(&c, &mut st),
                    "Interior" => st.fill = Some(interior(&c)),
                    "NumberFormat" => {
                        st.number_format = Some(c.attr("Format").and_then(number_format));
                    }
                    "Alignment" => alignment(&c, &mut st),
                    "Protection" => {
                        if let Some(p) = c.attr("Protected") {
                            st.locked = Some(parse_bool(p));
                        }
                        if let Some(h) = c.attr("HideFormula") {
                            st.hide_formula = Some(parse_bool(h));
                        }
                    }
                    "Borders" => {
                        st.borders = Some(borders(xr)?);
                        continue;
                    }
                    _ => {}
                }
                xr.skip()?;
            }
            Ev::End if xr.depth() < base => return Ok(st),
            Ev::Eof => return Ok(st),
            _ => {}
        }
    }
}

/// `ss:Font` (template `style-style-content`: `ss:Bold`, `ss:Italic`,
/// `ss:Underline`, `ss:StrikeThrough`, `ss:Size`, `ss:FontName`,
/// `ss:Color`).
fn font(c: &Elem, st: &mut RawStyle) {
    if let Some(v) = c.attr("Bold") {
        st.bold = Some(parse_bool(v));
    }
    if let Some(v) = c.attr("Italic") {
        st.italic = Some(parse_bool(v));
    }
    if let Some(v) = c.attr("StrikeThrough") {
        st.strike = Some(parse_bool(v));
    }
    if let Some(v) = c.attr("Underline") {
        st.underline = Some(match v.trim() {
            "Single" => Some(UnderlineStyleData::Single),
            "Double" => Some(UnderlineStyleData::Double),
            "SingleAccounting" => Some(UnderlineStyleData::SingleAccounting),
            "DoubleAccounting" => Some(UnderlineStyleData::DoubleAccounting),
            _ => None,
        });
    }
    if let Some(v) = c.attr("Size").and_then(|s| s.trim().parse::<f32>().ok()) {
        if v.is_finite() && v > 0.0 {
            st.size = Some(v.clamp(1.0, 409.0));
        }
    }
    if let Some(n) = c.attr("FontName").and_then(tally_model::clean_font_name) {
        st.family = Some(n);
    }
    if let Some(v) = c.attr("Color") {
        st.color = Some(parse_color(v));
    }
}

/// `ss:Interior` (template `ss:Interior`): `Solid` fills with `ss:Color`;
/// other patterns mix `ss:Color` and `ss:PatternColor` by the pattern's
/// percentage (`Gray50` → 50 %; template `cell-pattern-color`), `None` is
/// no fill.
fn interior(c: &Elem) -> Option<u32> {
    let color = c.attr("Color").and_then(parse_color);
    match c.attr("Pattern").map(str::trim) {
        None | Some("Solid") => color,
        Some("None") => None,
        Some(p) => {
            let pc = c.attr("PatternColor").and_then(parse_color);
            let pct: Option<f64> = p
                .split_once('y')
                .and_then(|(_, n)| n.parse::<u32>().ok())
                .map(|n| f64::from(n.min(100)) / 100.0);
            match (color, pc, pct) {
                (Some(a), Some(b), Some(t)) => Some(mix(a, b, t)),
                (Some(a), _, _) => Some(a),
                _ => None,
            }
        }
    }
}

fn mix(a: u32, b: u32, t: f64) -> u32 {
    let ch = |c: u32, s: u32| f64::from((c >> s) & 0xff);
    let one = |s: u32| -> u32 {
        let v = (ch(a, s) * (1.0 - t) + ch(b, s) * t).floor();
        (v.clamp(0.0, 255.0) as u32) << s
    };
    one(24) | one(16) | one(8) | 0xff
}

/// `ss:Alignment`.
fn alignment(c: &Elem, st: &mut RawStyle) {
    if let Some(h) = c.attr("Horizontal") {
        st.h = Some(match h.trim() {
            "Left" | "Fill" => HAlign::Left,
            "Center" | "CenterAcrossSelection" => HAlign::Center,
            "Right" => HAlign::Right,
            "Justify" | "Distributed" | "JustifyDistributed" => HAlign::Justify,
            _ => HAlign::General,
        });
    }
    if let Some(v) = c.attr("Vertical") {
        // Template `style-style-content`: Automatic → middle.
        st.v = Some(match v.trim() {
            "Top" => VAlign::Top,
            "Center" | "Automatic" | "Justify" | "Distributed" | "JustifyDistributed" => {
                VAlign::Middle
            }
            _ => VAlign::Bottom,
        });
    }
    if let Some(v) = c.attr("WrapText") {
        st.wrap = Some(parse_bool(v));
    }
    if let Some(v) = c.attr("ShrinkToFit") {
        st.shrink = Some(parse_bool(v));
    }
    if let Some(v) = c.attr("VerticalText") {
        st.stacked = Some(parse_bool(v));
    }
    if let Some(i) = c.attr("Indent").and_then(|v| v.trim().parse::<u32>().ok()) {
        st.indent = Some(u8::try_from(i.min(250)).unwrap_or(250));
    }
    if let Some(r) = c.attr("Rotate").and_then(|v| v.trim().parse::<f64>().ok()) {
        if r.is_finite() {
            st.rotation = Some(r.round().clamp(-90.0, 90.0) as i16);
        }
    }
}

/// `ss:Borders/ss:Border` (template `ss:Border`): `ss:Weight` 0 hairline,
/// 1 thin, 2 medium, 3 thick; `Continuous` solid, `Double`, `Dash*` and
/// `Dot` dashed/dotted (AOO 4.1 draws those solid); default colour black.
fn borders<R: BufRead>(xr: &mut XmlReader<R>) -> Result<[RawBorder; 6], XmlssError> {
    let mut out: [RawBorder; 6] = Default::default();
    let base = xr.depth();
    loop {
        match xr.next()? {
            Ev::Start(c) => {
                if c.local() == "Border" {
                    let slot = match c.attr("Position").map(str::trim) {
                        Some("Top") => Some(0),
                        Some("Bottom") => Some(1),
                        Some("Left") => Some(2),
                        Some("Right") => Some(3),
                        Some("DiagonalRight") => Some(4),
                        Some("DiagonalLeft") => Some(5),
                        _ => None,
                    };
                    let line_style = c.attr("LineStyle").map_or("Continuous", str::trim);
                    if let (Some(slot), false) = (slot, line_style == "None") {
                        let weight = c
                            .attr("Weight")
                            .and_then(|w| w.trim().parse::<f64>().ok())
                            .unwrap_or(0.0);
                        let style = match line_style {
                            "Double" => BorderStyle::Double,
                            "Dot" => BorderStyle::Dotted,
                            "Dash" | "DashDot" | "DashDotDot" | "SlantDashDot" => {
                                BorderStyle::Dashed
                            }
                            _ => BorderStyle::Solid,
                        };
                        // Widths of the template (0.002/0.035/0.088/0.141 cm;
                        // double thick 0.105 cm), hairline as XLSX's 9.
                        let width_hmm = match (weight.round() as i64, style) {
                            (3, BorderStyle::Double) => 105,
                            (3, _) => 141,
                            (2, _) => 88,
                            (1, _) => 35,
                            _ => 9,
                        };
                        if let Some(o) = out.get_mut(slot) {
                            *o = Some(BorderLine {
                                style,
                                width_hmm,
                                color: c.attr("Color").and_then(parse_color).unwrap_or(0xff),
                            });
                        }
                    }
                }
                xr.skip()?;
            }
            Ev::End if xr.depth() < base => return Ok(out),
            Ev::Eof => return Ok(out),
            _ => {}
        }
    }
}

/// An `ss:Format` → OOo code (`None` = General). The named formats are
/// those of template `create-number-format-content` (and the currency and
/// boolean cases of `process-number-format-unit`); other codes are Excel
/// syntax and go through [`tally_io_xlsx::excel_to_ooo`].
pub(crate) fn number_format(f: &str) -> Option<String> {
    let named = match f.trim() {
        "General" | "" => return None,
        "General Number" => "0",
        "General Date" => "YYYY-M-D H:MM",
        "Long Date" => "YYYY-MM-DD",
        "Medium Date" => "D-MMM-YY",
        "Short Date" => "D-M-YY",
        "Long Time" => "HH:MM:SS AM/PM",
        "Medium Time" => "H:MM AM/PM",
        "Short Time" => "H:MM",
        "Currency" => "[$$]#,##0.00;[RED][$$]-#,##0.00",
        "Euro Currency" => "[$€]\\ #,##0.00;[RED]([$€]\\ #,##0.00)",
        "Fixed" => "0.00",
        "Standard" => "#,##0.00",
        "Percent" => "0.00%",
        "Scientific" => "0.00E+00",
        "Yes/No" => "\"Yes\";\"Yes\";\"No\"",
        "True/False" => "\"True\";\"True\";\"False\"",
        "On/Off" => "\"On\";\"On\";\"Off\"",
        code => {
            // Template `process-number-format-unit`: a trailing `;@` text
            // section of a multi-section code is dropped.
            let code = match code.strip_suffix(";@") {
                Some(c) if !c.is_empty() => c,
                _ => code,
            };
            return tally_io_xlsx::excel_to_ooo(code);
        }
    };
    Some(named.to_string())
}

/// The resolved styles of a workbook.
#[derive(Debug, Default)]
pub(crate) struct StyleTable {
    /// Raw styles by `ss:ID`.
    pub raw: HashMap<String, RawStyle>,
}

/// The fully inherited properties of one style id.
fn resolve(table: &StyleTable, id: &str, warnings: &mut Vec<String>) -> RawStyle {
    // The chain leaf → root, then Default.
    let mut chain: Vec<&RawStyle> = Vec::new();
    let mut seen: HashSet<&str> = HashSet::new();
    let mut cur = Some(id);
    while let Some(name) = cur {
        if !seen.insert(name) {
            crate::warn(warnings, "A cycle of ss:Parent styles was cut.".into());
            break;
        }
        if chain.len() >= MAX_PARENT_DEPTH {
            crate::warn(
                warnings,
                format!("Style parent chains deeper than {MAX_PARENT_DEPTH} were cut."),
            );
            break;
        }
        let Some(st) = table.raw.get(name) else { break };
        chain.push(st);
        cur = st.parent.as_deref();
    }
    if !seen.contains("Default") {
        if let Some(d) = table.raw.get("Default") {
            chain.push(d);
        }
    }
    let mut out = RawStyle::default();
    for st in chain.iter().rev() {
        macro_rules! take {
            ($($f:ident),*) => {$(if st.$f.is_some() { out.$f.clone_from(&st.$f); })*};
        }
        take!(
            bold,
            italic,
            underline,
            strike,
            family,
            size,
            color,
            fill,
            borders,
            number_format,
            h,
            v,
            wrap,
            indent,
            rotation,
            stacked,
            shrink,
            locked,
            hide_formula
        );
    }
    out
}

/// Model style of a resolved style. Font family and size equal to the
/// `Default` style's are left unset (they are the document default, as
/// the XLSX reader does with `fonts[0]`).
fn to_model(r: &RawStyle, default: &RawStyle, long_fmt: &mut bool) -> StyleData {
    let underline = r.underline.flatten();
    let mut number_format = r.number_format.clone().flatten();
    if number_format
        .as_ref()
        .is_some_and(|f| f.chars().count() > MAX_NUMBER_FORMAT_LEN)
    {
        *long_fmt = true;
        number_format = None;
    }
    let b = r.borders.clone().unwrap_or_default();
    let [top, bottom, left, right, diagonal_down, diagonal_up] = b;
    let protection = {
        let p = CellProtectionData {
            locked: r.locked.unwrap_or(true),
            hidden_formula: r.hide_formula.unwrap_or(false),
            ..CellProtectionData::default()
        };
        (p != CellProtectionData::default()).then_some(p)
    };
    StyleData {
        font: FontData {
            bold: r.bold.unwrap_or(false),
            italic: r.italic.unwrap_or(false),
            underline: underline.is_some(),
            strike: r.strike.unwrap_or(false),
            family: r
                .family
                .clone()
                .filter(|f| Some(f) != default.family.as_ref()),
            size_pt: r.size.filter(|s| Some(*s) != default.size),
            color: r.color.flatten(),
            underline_style: underline.unwrap_or_default(),
        },
        fill: r.fill.flatten(),
        align: AlignData {
            h: r.h.unwrap_or_default(),
            v: r.v.unwrap_or_default(),
            wrap: r.wrap.unwrap_or(false),
            indent: r.indent.unwrap_or(0),
            rotation_deg: if r.stacked == Some(true) {
                0
            } else {
                r.rotation.unwrap_or(0)
            },
            stacked: r.stacked.unwrap_or(false),
            shrink_to_fit: r.shrink.unwrap_or(false),
        },
        borders: BordersData {
            top,
            bottom,
            left,
            right,
            diagonal_down,
            diagonal_up,
        },
        number_format,
        display: None,
        protection,
        style_name: None,
    }
}

/// Deduplicating builder of the model style table.
#[derive(Default)]
pub(crate) struct ModelStyles {
    pub styles: Vec<StyleData>,
    dedup: HashMap<String, u32>,
}

impl ModelStyles {
    /// The index of `st` (`None` for the default look).
    pub fn intern(&mut self, st: StyleData) -> Option<u32> {
        if st == StyleData::default() {
            return None;
        }
        let key = format!("{st:?}");
        let styles = &mut self.styles;
        Some(*self.dedup.entry(key).or_insert_with(|| {
            styles.push(st);
            u32::try_from(styles.len() - 1).unwrap_or(u32::MAX)
        }))
    }
}

/// Resolves the style ids `ids` (interned in reference order) → model
/// style indexes, plus the document default font (the `Default` style's
/// family).
pub(crate) fn build(
    table: &StyleTable,
    ids: &[String],
    model: &mut ModelStyles,
    warnings: &mut Vec<String>,
) -> (Vec<Option<u32>>, Option<String>) {
    let default = table
        .raw
        .get("Default")
        .map(|_| resolve(table, "Default", warnings))
        .unwrap_or_default();
    let mut long_fmt = false;
    let map = ids
        .iter()
        .map(|id| {
            let r = resolve(table, id, warnings);
            model.intern(to_model(&r, &default, &mut long_fmt))
        })
        .collect();
    if long_fmt {
        crate::warn(warnings, tally_model::long_number_format_warning());
    }
    (map, default.family)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_formats_parse_as_ooo_codes() {
        for n in [
            "General Number",
            "General Date",
            "Long Date",
            "Medium Date",
            "Short Date",
            "Long Time",
            "Medium Time",
            "Short Time",
            "Currency",
            "Euro Currency",
            "Fixed",
            "Standard",
            "Percent",
            "Scientific",
            "Yes/No",
            "True/False",
            "On/Off",
        ] {
            let code = number_format(n).unwrap_or_else(|| panic!("{n}"));
            assert!(tally_numfmt::parse_format(&code).is_ok(), "{n}: {code}");
        }
        assert_eq!(number_format("General"), None);
        assert_eq!(number_format("0.00%").as_deref(), Some("0.00%"));
        assert_eq!(
            number_format("yyyy\\-mm\\-dd;@").as_deref(),
            Some("YYYY\\-MM\\-DD")
        );
        assert!(tally_io_xlsx::is_date_format(
            &number_format("Short Date").unwrap()
        ));
    }

    #[test]
    fn colors() {
        assert_eq!(parse_color("#FF0000"), Some(0xff00_00ff));
        assert_eq!(parse_color("Automatic"), None);
        assert_eq!(parse_color("#12345"), None);
        assert_eq!(mix(0xffff_ffff, 0x0000_00ff, 0.5), 0x7f7f_7fff);
    }
}
