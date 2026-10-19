//! Reading ODF styles (cell, column, row, table and data styles) and
//! resolving cell style names into deduplicated [`StyleData`] entries.

use std::collections::HashMap;
use std::io::BufRead;

use tally_model::{BorderLine, CellProtectionData, CfStyleData, HAlign, StyleData, VAlign};

use crate::numfmt::{DataStyle, DataStyles, Kind, Part};
use crate::units::{font_size_pt, length_to_hmm, parse_border, parse_color};
use crate::xml::{Elem, Ev, XmlReader};
use crate::OdsError;

/// Cell style properties as written; `None` means "inherit".
#[derive(Clone, Debug, Default)]
pub(crate) struct RawCellStyle {
    parent: Option<String>,
    data_style: Option<String>,
    bold: Option<bool>,
    italic: Option<bool>,
    underline: Option<bool>,
    strike: Option<bool>,
    font_name: Option<String>,
    font_family: Option<String>,
    size_pt: Option<f32>,
    color: Option<Option<u32>>,
    fill: Option<Option<u32>>,
    h: Option<HAlign>,
    align_fixed: Option<bool>,
    v: Option<VAlign>,
    wrap: Option<bool>,
    border_top: Option<Option<BorderLine>>,
    border_bottom: Option<Option<BorderLine>>,
    border_left: Option<Option<BorderLine>>,
    border_right: Option<Option<BorderLine>>,
    protect: Option<CellProtectionData>,
    hide_print: Option<bool>,
    /// `style:map` conditions (legacy conditional formatting).
    pub(crate) maps: Vec<crate::v03::RawMap>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct RowStyle {
    pub height_hmm: Option<u32>,
    pub optimal: bool,
    /// `fo:break-before="page"`.
    pub break_before: bool,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct TableStyle {
    pub hidden: bool,
    pub tab_color: Option<u32>,
    pub master_page: Option<String>,
}

/// The styles from one place (styles.xml or content.xml).
#[derive(Debug, Default)]
pub(crate) struct Scope {
    cells: HashMap<String, RawCellStyle>,
    data: DataStyles,
    columns: HashMap<String, u32>,
    col_breaks: std::collections::HashSet<String>,
    rows: HashMap<String, RowStyle>,
    tables: HashMap<String, TableStyle>,
    pub(crate) page_layouts: HashMap<String, crate::v03::PageLayout>,
}

/// All styles of a document plus the deduplicated output table.
#[derive(Debug, Default)]
pub(crate) struct StyleSheet {
    /// styles.xml (`office:styles` and its automatic styles).
    pub common: Scope,
    /// content.xml automatic styles.
    pub content: Scope,
    fonts: HashMap<String, String>,
    resolved: HashMap<String, Option<u32>>,
    /// Canonical key ([`style_key`]) → index into `styles`, so deduplication
    /// is O(1) per style instead of a scan of every style so far.
    dedup: HashMap<String, u32>,
    pub styles: Vec<StyleData>,
    /// Resolved style index → the `style:map`s of the (first) style that
    /// resolved to it.
    pub style_maps: HashMap<u32, Vec<crate::v03::RawMap>>,
    /// Master pages (styles.xml).
    pub master_pages: HashMap<String, crate::v03::MasterPage>,
    /// `fo:language`-`fo:country` of the default cell style.
    pub default_locale: Option<String>,
    /// What was dropped to stay within limits.
    pub warnings: Vec<String>,
}

/// A canonical key for a style. `StyleData` holds `f32` sizes, so it cannot
/// derive `Hash`/`Eq`; its `Debug` text is a faithful, deterministic key
/// (equal data gives equal text).
fn style_key(data: &StyleData) -> String {
    format!("{data:?}")
}

/// Which part of the package a style block came from.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Origin {
    Styles,
    Content,
}

impl StyleSheet {
    fn scope_mut(&mut self, origin: Origin) -> &mut Scope {
        match origin {
            Origin::Styles => &mut self.common,
            Origin::Content => &mut self.content,
        }
    }

    pub fn column_width(&self, name: &str) -> Option<u32> {
        self.content
            .columns
            .get(name)
            .or_else(|| self.common.columns.get(name))
            .copied()
    }

    pub fn row_style(&self, name: &str) -> Option<&RowStyle> {
        self.content
            .rows
            .get(name)
            .or_else(|| self.common.rows.get(name))
    }

    /// `true` if the column style breaks the page before the column.
    pub fn column_break(&self, name: &str) -> bool {
        self.content.col_breaks.contains(name) || self.common.col_breaks.contains(name)
    }

    /// A page layout by name.
    pub fn page_layout(&self, name: &str) -> Option<&crate::v03::PageLayout> {
        self.common
            .page_layouts
            .get(name)
            .or_else(|| self.content.page_layouts.get(name))
    }

    /// A named style as a conditional (differential) style: only the
    /// properties the style chain sets.
    pub fn cf_style(&self, name: &str) -> CfStyleData {
        let mut chain = Vec::new();
        let mut cur = Some(name.to_string());
        while let Some(n) = cur {
            if n == "Default" || chain.len() >= 16 {
                break;
            }
            let Some(raw) = self.cell(&n) else { break };
            chain.push(raw);
            cur = raw.parent.clone();
        }
        let mut st = CfStyleData::default();
        for raw in chain.iter().rev() {
            if raw.bold.is_some() {
                st.bold = raw.bold;
            }
            if raw.italic.is_some() {
                st.italic = raw.italic;
            }
            if raw.underline.is_some() {
                st.underline = raw.underline;
            }
            if raw.strike.is_some() {
                st.strike = raw.strike;
            }
            if let Some(c) = raw.color {
                st.color = c;
            }
            if let Some(f) = raw.fill {
                st.fill = f;
            }
            let mut b = st.borders.clone().unwrap_or_default();
            let mut any = false;
            for (slot, src) in [
                (&mut b.top, &raw.border_top),
                (&mut b.bottom, &raw.border_bottom),
                (&mut b.left, &raw.border_left),
                (&mut b.right, &raw.border_right),
            ] {
                if let Some(x) = src {
                    slot.clone_from(x);
                    any = true;
                }
            }
            if any {
                st.borders = Some(b);
            }
            if let Some(ds) = raw.data_style.as_deref().and_then(|d| self.data_style(d)) {
                let code = ds.to_code(|n| self.data_style(n));
                if !code.is_empty()
                    && code != "General"
                    && code.chars().count() <= tally_model::MAX_NUMBER_FORMAT_LEN
                {
                    st.number_format = Some(code);
                }
            }
        }
        st
    }

    pub fn table_style(&self, name: &str) -> Option<&TableStyle> {
        self.content
            .tables
            .get(name)
            .or_else(|| self.common.tables.get(name))
    }

    fn cell(&self, name: &str) -> Option<&RawCellStyle> {
        self.content
            .cells
            .get(name)
            .or_else(|| self.common.cells.get(name))
    }

    fn data_style(&self, name: &str) -> Option<&DataStyle> {
        self.content
            .data
            .get(name)
            .or_else(|| self.common.data.get(name))
    }

    /// Resolves a cell style name to an index into [`Self::styles`]
    /// (`None` for the default look). Cached per name.
    pub fn resolve(&mut self, name: &str) -> Option<u32> {
        if let Some(r) = self.resolved.get(name) {
            return *r;
        }
        let data = self.build(name);
        let maps = self.cell(name).map(|r| r.maps.clone()).unwrap_or_default();
        let index = if data == StyleData::default() && maps.is_empty() {
            None
        } else {
            let key = style_key(&data);
            match self.dedup.get(&key) {
                Some(&i) => Some(i),
                None => {
                    let i = self.styles.len() as u32;
                    self.styles.push(data);
                    self.dedup.insert(key, i);
                    Some(i)
                }
            }
        };
        if let (Some(i), false) = (index, maps.is_empty()) {
            self.style_maps.entry(i).or_insert(maps);
        }
        self.resolved.insert(name.to_string(), index);
        index
    }

    fn build(&self, name: &str) -> StyleData {
        // Collect the parent chain (leaf first), stopping at the root
        // "Default" style, which defines the baseline look.
        let mut chain = Vec::new();
        let mut cur = Some(name.to_string());
        while let Some(n) = cur {
            if n == "Default" || chain.len() >= 16 {
                break;
            }
            let Some(raw) = self.cell(&n) else { break };
            chain.push(raw);
            cur = raw.parent.clone();
        }
        let mut m = RawCellStyle::default();
        for raw in chain.iter().rev() {
            macro_rules! inherit {
                ($($f:ident),*) => { $( if raw.$f.is_some() { m.$f = raw.$f.clone(); } )* };
            }
            inherit!(
                data_style,
                bold,
                italic,
                underline,
                strike,
                font_name,
                font_family,
                size_pt,
                color,
                fill,
                h,
                align_fixed,
                v,
                wrap,
                border_top,
                border_bottom,
                border_left,
                border_right,
                protect,
                hide_print
            );
        }
        let mut s = StyleData::default();
        s.font.bold = m.bold.unwrap_or(false);
        s.font.italic = m.italic.unwrap_or(false);
        s.font.underline = m.underline.unwrap_or(false);
        s.font.strike = m.strike.unwrap_or(false);
        s.font.family = m
            .font_name
            .as_ref()
            .map(|n| self.fonts.get(n).cloned().unwrap_or_else(|| n.clone()))
            .or(m.font_family);
        s.font.size_pt = m.size_pt;
        s.font.color = m.color.flatten();
        s.fill = m.fill.flatten();
        s.align.h = if m.align_fixed == Some(false) {
            HAlign::General
        } else {
            m.h.unwrap_or_default()
        };
        s.align.v = m.v.unwrap_or_default();
        s.align.wrap = m.wrap.unwrap_or(false);
        s.borders.top = m.border_top.flatten();
        s.borders.bottom = m.border_bottom.flatten();
        s.borders.left = m.border_left.flatten();
        s.borders.right = m.border_right.flatten();
        let mut prot = m.protect.unwrap_or_default();
        if m.hide_print == Some(true) {
            prot.hide_print = true;
        }
        s.protection = (prot != CellProtectionData::default()).then_some(prot);
        s.number_format = m
            .data_style
            .as_deref()
            .and_then(|d| self.data_style(d))
            .map(|ds| ds.to_code(|n| self.data_style(n)))
            .filter(|code| {
                !code.is_empty()
                    && code != "General"
                    && code.chars().count() <= tally_model::MAX_NUMBER_FORMAT_LEN
            });
        s
    }

    /// Parses `office:font-face-decls`.
    pub fn parse_font_faces<R: BufRead>(&mut self, xml: &mut XmlReader<R>) -> Result<(), OdsError> {
        let d = xml.depth();
        loop {
            match xml.next()? {
                Ev::Start(e) => {
                    if e.name == "style:font-face" {
                        if let (Some(n), Some(f)) =
                            (e.attr("style:name"), e.attr("svg:font-family"))
                        {
                            let fam = f.trim().trim_matches(['\'', '"']).to_string();
                            self.fonts.insert(n.to_string(), fam);
                        }
                    }
                    xml.skip()?;
                }
                Ev::End if xml.depth() < d => return Ok(()),
                Ev::Eof => return Ok(()),
                _ => {}
            }
        }
    }

    /// Parses `office:styles` or `office:automatic-styles`.
    pub fn parse_block<R: BufRead>(
        &mut self,
        xml: &mut XmlReader<R>,
        origin: Origin,
    ) -> Result<(), OdsError> {
        let d = xml.depth();
        loop {
            match xml.next()? {
                Ev::Start(e) => {
                    if e.name == "style:style" {
                        self.parse_style(xml, &e, origin)?;
                    } else if e.name == "style:page-layout" {
                        let name = e.attr("style:name").unwrap_or("").to_string();
                        let l = crate::v03::parse_page_layout(xml)?;
                        self.scope_mut(origin).page_layouts.insert(name, l);
                    } else if e.name == "style:default-style"
                        && e.attr("style:family") == Some("table-cell")
                    {
                        self.parse_default_style(xml)?;
                    } else if let Some(kind) = Kind::from_element(&e.name) {
                        let name = e.attr("style:name").map(str::to_string);
                        let mut ds = parse_data_style(xml, &e, kind)?;
                        // Any number of parts is allowed by the schema; a
                        // code past the limit becomes General (review 2,
                        // HIGH-4). Map targets are checked on their own.
                        if ds.to_code(|_| None).chars().count() > tally_model::MAX_NUMBER_FORMAT_LEN
                        {
                            ds = crate::numfmt::DataStyle::new(kind);
                            let w = tally_model::long_number_format_warning();
                            if !self.warnings.contains(&w) {
                                self.warnings.push(w);
                            }
                        }
                        if let Some(name) = name {
                            self.scope_mut(origin).data.insert(name, ds);
                        }
                    } else {
                        xml.skip()?;
                    }
                }
                Ev::End if xml.depth() < d => return Ok(()),
                Ev::Eof => return Ok(()),
                _ => {}
            }
        }
    }

    fn parse_default_style<R: BufRead>(&mut self, xml: &mut XmlReader<R>) -> Result<(), OdsError> {
        let d = xml.depth();
        loop {
            match xml.next()? {
                Ev::Start(p) => {
                    if p.name == "style:text-properties" {
                        if let Some(lang) = p.attr("fo:language").filter(|l| *l != "none") {
                            let tag = match p.attr("fo:country").filter(|c| *c != "none") {
                                Some(c) => format!("{lang}-{c}"),
                                None => lang.to_string(),
                            };
                            self.default_locale = Some(tag);
                        }
                    }
                    xml.skip()?;
                }
                Ev::End if xml.depth() < d => return Ok(()),
                Ev::Eof => return Ok(()),
                _ => {}
            }
        }
    }

    fn parse_style<R: BufRead>(
        &mut self,
        xml: &mut XmlReader<R>,
        e: &Elem,
        origin: Origin,
    ) -> Result<(), OdsError> {
        let name = e.attr("style:name").unwrap_or_default().to_string();
        let family = e.attr("style:family").unwrap_or_default().to_string();
        let mut cell = RawCellStyle {
            parent: e.attr("style:parent-style-name").map(str::to_string),
            data_style: e.attr("style:data-style-name").map(str::to_string),
            ..RawCellStyle::default()
        };
        let mut col_width = None;
        let mut row = RowStyle {
            height_hmm: None,
            optimal: true,
            break_before: false,
        };
        let mut col_break = false;
        let mut table = TableStyle {
            master_page: e.attr("style:master-page-name").map(str::to_string),
            ..TableStyle::default()
        };
        let d = xml.depth();
        loop {
            match xml.next()? {
                Ev::Start(p) => {
                    match p.name.as_str() {
                        "style:text-properties" => apply_text_props(&mut cell, &p),
                        "style:table-cell-properties" => apply_cell_props(&mut cell, &p),
                        "style:paragraph-properties" => apply_para_props(&mut cell, &p),
                        "style:table-column-properties" => {
                            col_width = p.attr("style:column-width").and_then(length_to_hmm);
                            col_break = p.attr("fo:break-before") == Some("page");
                        }
                        "style:table-row-properties" => {
                            row.height_hmm = p.attr("style:row-height").and_then(length_to_hmm);
                            row.optimal = p.attr("style:use-optimal-row-height") != Some("false");
                            row.break_before = p.attr("fo:break-before") == Some("page");
                        }
                        "style:map" if family == "table-cell" => {
                            if let (Some(c), Some(a)) =
                                (p.attr("style:condition"), p.attr("style:apply-style-name"))
                            {
                                if cell.maps.len() < 64 {
                                    cell.maps.push(crate::v03::RawMap {
                                        condition: c.to_string(),
                                        apply_style: a.to_string(),
                                        base: p
                                            .attr("style:base-cell-address")
                                            .and_then(crate::v03::parse_base),
                                    });
                                }
                            }
                        }
                        "style:table-properties" => {
                            table.hidden = p.attr("table:display") == Some("false");
                            table.tab_color = p
                                .attr("tableooo:tab-color")
                                .or_else(|| p.attr("table:tab-color"))
                                .and_then(parse_color);
                        }
                        _ => {}
                    }
                    xml.skip()?;
                }
                Ev::End if xml.depth() < d => break,
                Ev::Eof => break,
                _ => {}
            }
        }
        let scope = self.scope_mut(origin);
        match family.as_str() {
            "table-cell" => {
                scope.cells.insert(name, cell);
            }
            "table-column" => {
                if col_break {
                    scope.col_breaks.insert(name.clone());
                }
                if let Some(w) = col_width {
                    scope.columns.insert(name, w);
                }
            }
            "table-row" => {
                scope.rows.insert(name, row);
            }
            "table" => {
                scope.tables.insert(name, table);
            }
            _ => {}
        }
        Ok(())
    }
}

fn apply_text_props(s: &mut RawCellStyle, p: &Elem) {
    if let Some(w) = p.attr("fo:font-weight") {
        s.bold = Some(match w {
            "bold" => true,
            "normal" => false,
            n => n.parse::<u32>().is_ok_and(|n| n >= 600),
        });
    }
    if let Some(v) = p.attr("fo:font-style") {
        s.italic = Some(matches!(v, "italic" | "oblique"));
    }
    if let Some(v) = p.attr("style:text-underline-style") {
        s.underline = Some(v != "none");
    }
    if let Some(v) = p.attr("style:text-line-through-style") {
        s.strike = Some(v != "none");
    }
    if let Some(v) = p.attr("fo:color") {
        s.color = Some(parse_color(v));
    }
    if let Some(v) = p.attr("fo:font-size").and_then(font_size_pt) {
        s.size_pt = Some(v);
    }
    if let Some(v) = p.attr("style:font-name") {
        s.font_name = Some(v.to_string());
    }
    if let Some(v) = p.attr("fo:font-family") {
        s.font_family = Some(v.trim().trim_matches(['\'', '"']).to_string());
    }
}

fn apply_cell_props(s: &mut RawCellStyle, p: &Elem) {
    if let Some(v) = p.attr("fo:background-color") {
        s.fill = Some(parse_color(v));
    }
    if let Some(v) = p.attr("fo:wrap-option") {
        s.wrap = Some(v == "wrap");
    }
    if let Some(v) = p.attr("style:vertical-align") {
        s.v = Some(match v {
            "top" => VAlign::Top,
            "middle" => VAlign::Middle,
            _ => VAlign::Bottom,
        });
    }
    // `fo:border` sets all four sides; the side attributes override it.
    if let Some(b) = p.attr("fo:border").and_then(parse_border) {
        s.border_top = Some(b.clone());
        s.border_bottom = Some(b.clone());
        s.border_left = Some(b.clone());
        s.border_right = Some(b);
    }
    for (key, slot) in [
        ("fo:border-top", &mut s.border_top),
        ("fo:border-bottom", &mut s.border_bottom),
        ("fo:border-left", &mut s.border_left),
        ("fo:border-right", &mut s.border_right),
    ] {
        if let Some(b) = p.attr(key).and_then(parse_border) {
            *slot = Some(b);
        }
    }
    if let Some(v) = p.attr("style:text-align-source") {
        s.align_fixed = Some(v == "fix");
    }
    if let Some(v) = p.attr("style:cell-protect") {
        let t: Vec<&str> = v.split_whitespace().collect();
        s.protect = Some(CellProtectionData {
            locked: t
                .iter()
                .any(|x| matches!(*x, "protected" | "hidden-and-protected")),
            hidden_formula: t.contains(&"formula-hidden"),
            hidden: t.contains(&"hidden-and-protected"),
            hide_print: false,
        });
    }
    if let Some(v) = p.attr("style:print-content") {
        s.hide_print = Some(v == "false");
    }
}

fn apply_para_props(s: &mut RawCellStyle, p: &Elem) {
    if let Some(v) = p.attr("fo:text-align") {
        s.h = Some(match v {
            "start" | "left" => HAlign::Left,
            "center" => HAlign::Center,
            "end" | "right" => HAlign::Right,
            "justify" => HAlign::Justify,
            _ => HAlign::General,
        });
    }
}

fn num_attr(e: &Elem, key: &str) -> Option<u32> {
    e.attr(key).and_then(|v| v.trim().parse().ok())
}

fn is_long(e: &Elem) -> bool {
    e.attr("number:style") == Some("long")
}

/// Parses one `number:*-style` element into a [`DataStyle`].
pub(crate) fn parse_data_style<R: BufRead>(
    xml: &mut XmlReader<R>,
    e: &Elem,
    kind: Kind,
) -> Result<DataStyle, OdsError> {
    const MAX_LIT: usize = 4096;
    let mut ds = DataStyle::new(kind);
    ds.truncate = e.attr("number:truncate-on-overflow") != Some("false");
    let d = xml.depth();
    loop {
        match xml.next()? {
            Ev::Start(p) => {
                let part = match p.name.as_str() {
                    "number:number" => {
                        let factor = p
                            .attr("number:display-factor")
                            .and_then(|f| f.trim().parse::<f64>().ok())
                            .map_or(0, |f| {
                                let mut k = 0;
                                let mut x = f;
                                while x >= 999.5 && k < 4 {
                                    x /= 1000.0;
                                    k += 1;
                                }
                                k
                            });
                        Some(Part::Number {
                            decimals: num_attr(&p, "number:decimal-places"),
                            min_decimals: num_attr(&p, "number:min-decimal-places"),
                            min_int: num_attr(&p, "number:min-integer-digits").unwrap_or(0),
                            grouping: p.attr("number:grouping") == Some("true"),
                            factor_thousands: factor,
                        })
                    }
                    "number:scientific-number" => Some(Part::Scientific {
                        decimals: num_attr(&p, "number:decimal-places").unwrap_or(0),
                        min_int: num_attr(&p, "number:min-integer-digits").unwrap_or(1),
                        min_exp: num_attr(&p, "number:min-exponent-digits").unwrap_or(2),
                        grouping: p.attr("number:grouping") == Some("true"),
                    }),
                    "number:fraction" => Some(Part::Fraction {
                        min_int: num_attr(&p, "number:min-integer-digits"),
                        min_num: num_attr(&p, "number:min-numerator-digits").unwrap_or(1),
                        min_den: num_attr(&p, "number:min-denominator-digits").unwrap_or(1),
                        den_value: num_attr(&p, "number:denominator-value"),
                    }),
                    "number:text" => {
                        let t = xml.text_content(MAX_LIT)?;
                        if !t.is_empty() {
                            ds.parts.push(Part::Text(t));
                        }
                        continue;
                    }
                    "number:currency-symbol" => {
                        let t = xml.text_content(MAX_LIT)?;
                        ds.parts.push(Part::Currency(t));
                        continue;
                    }
                    "number:fill-character" => {
                        let t = xml.text_content(MAX_LIT)?;
                        if let Some(c) = t.chars().next() {
                            ds.parts.push(Part::Fill(c));
                        }
                        continue;
                    }
                    "number:text-content" => Some(Part::TextContent),
                    "number:boolean" => Some(Part::Boolean),
                    "number:year" => Some(Part::Year { long: is_long(&p) }),
                    "number:month" => Some(Part::Month {
                        long: is_long(&p),
                        textual: p.attr("number:textual") == Some("true"),
                    }),
                    "number:day" => Some(Part::Day { long: is_long(&p) }),
                    "number:day-of-week" => Some(Part::DayOfWeek { long: is_long(&p) }),
                    "number:hours" => Some(Part::Hours { long: is_long(&p) }),
                    "number:minutes" => Some(Part::Minutes { long: is_long(&p) }),
                    "number:seconds" => Some(Part::Seconds {
                        long: is_long(&p),
                        decimals: num_attr(&p, "number:decimal-places").unwrap_or(0),
                    }),
                    "number:am-pm" => Some(Part::AmPm),
                    "number:quarter" => Some(Part::Quarter { long: is_long(&p) }),
                    "number:week-of-year" => Some(Part::Week),
                    "number:era" => Some(Part::Era { long: is_long(&p) }),
                    "style:text-properties" => {
                        ds.color = p.attr("fo:color").and_then(parse_color);
                        None
                    }
                    "style:map" => {
                        if let (Some(c), Some(n)) =
                            (p.attr("style:condition"), p.attr("style:apply-style-name"))
                        {
                            ds.maps.push((c.to_string(), n.to_string()));
                        }
                        None
                    }
                    _ => None,
                };
                if let Some(part) = part {
                    ds.parts.push(part);
                }
                xml.skip()?;
            }
            Ev::End if xml.depth() < d => return Ok(ds),
            Ev::Eof => return Ok(ds),
            _ => {}
        }
    }
}
