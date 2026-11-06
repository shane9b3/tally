//! Chart objects (wave 5, W5-1): the `Object N/content.xml` of an
//! embedded chart, written from and read into [`ChartData`].
//!
//! The object document is `office:document-content` with its own
//! `office:automatic-styles` (family `chart`) and
//! `office:body/office:chart/chart:chart`. The reader is tolerant of
//! LibreOffice and Apache OpenOffice output: unknown classes become
//! column charts, 3D is kept as a flag, data styles on axes are ignored
//! and a `local-table` fills in values when a series has no range.
//!
//! Foreign attributes (`tally:` namespace) carry what ODF 1.2 cannot:
//! literal series names, the error-bar multiplier, an axis number
//! format and the pie hole fraction. Other readers ignore them.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::BufRead;

use tally_model::{
    AxisCrossData, AxisLabelPosData, AxisMarkPosData, AxisTypeData, CellContentData,
    CellRangeLocal, ChartAxisData, ChartData, ChartKindData, ChartRangeData, ChartSeriesData,
    ChartSourceData, ChartValuesData, DataLabelsData, ErrorBarDirData, ErrorBarKindData,
    ErrorBarsData, LabelPosData, LegendPosData, MarkerShapeData, SeriesNameData, SheetData,
    StackingData, StockVariantData, TimeIntervalData, TimeUnitData, TrendKindData, TrendLineData,
};
use tally_types::{col_to_letters, letters_to_col, Value, MAX_COLS, MAX_ROWS};

use crate::units::{color_hex, fmt_f64, hmm_to_cm, length_to_hmm, parse_color};
use crate::write::odf_sheet_name;
use crate::xml::{escape_attr, escape_text, Elem, Ev, XmlReader};
use crate::OdsError;

/// Most series read per chart.
pub(crate) const MAX_SERIES: usize = 255;
/// Most points taken from a `local-table` per column.
pub(crate) const MAX_LOCAL_ROWS: usize = 32_000;
/// Most points written to the `local-table` per series (larger charts
/// get no cached table; readers plot from the ranges).
const MAX_CACHED_POINTS: usize = 4_000;
/// Longest text kept (titles, names).
const MAX_TEXT: usize = 4_096;

const CHART_NS: &str = concat!(
    r#" xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0""#,
    r#" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0""#,
    r#" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0""#,
    r#" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0""#,
    r#" xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0""#,
    r#" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0""#,
    r#" xmlns:xlink="http://www.w3.org/1999/xlink""#,
    r#" xmlns:number="urn:oasis:names:tc:opendocument:xmlns:datastyle:1.0""#,
    r#" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0""#,
    r#" xmlns:chart="urn:oasis:names:tc:opendocument:xmlns:chart:1.0""#,
    r#" xmlns:ooo="http://openoffice.org/2004/office""#,
    r#" xmlns:chartooo="http://openoffice.org/2010/chart""#,
    r#" xmlns:loext="urn:org:documentfoundation:names:experimental:office:xmlns:loext:1.0""#,
    r#" xmlns:tally="urn:tally:names:experimental:1.0""#,
);

const XML_DECL: &str = r#"<?xml version="1.0" encoding="UTF-8"?>"#;

/// The minimal `Object N/styles.xml`.
pub(crate) fn styles_xml() -> String {
    format!(
        "{XML_DECL}<office:document-styles{CHART_NS} office:version=\"1.2\">\
         <office:styles/><office:automatic-styles/><office:master-styles/>\
         </office:document-styles>"
    )
}

// ---------------------------------------------------------------------------
// Range addresses
// ---------------------------------------------------------------------------

fn cell_address(sheet: &str, r: u32, c: u32) -> String {
    format!("{sheet}.{}{}", col_to_letters(c), u64::from(r) + 1)
}

/// `Sheet1.A1:Sheet1.B5 Sheet2.C1:Sheet2.C9` for the areas of a range
/// (`None` when unset or invalid).
fn range_address(r: &ChartRangeData, sheets: &[SheetData]) -> Option<String> {
    if r.invalid || r.areas.is_empty() {
        return None;
    }
    let parts: Vec<String> = r
        .areas
        .iter()
        .filter_map(|(tab, a)| {
            let name = odf_sheet_name(&sheets.get(*tab as usize)?.name);
            Some(format!(
                "{}:{}",
                cell_address(&name, a.r0, a.c0),
                cell_address(&name, a.r1, a.c1)
            ))
        })
        .collect();
    (!parts.is_empty()).then(|| parts.join(" "))
}

fn split_unquoted(s: &str, sep: char) -> Vec<&str> {
    let mut out = Vec::new();
    let mut quoted = false;
    let mut start = 0;
    for (i, ch) in s.char_indices() {
        if ch == '\'' {
            quoted = !quoted;
        } else if ch == sep && !quoted {
            out.push(&s[start..i]);
            start = i + ch.len_utf8();
        }
    }
    out.push(&s[start..]);
    out
}

/// `Sheet.A1` → (sheet, cell); `.A1`/`A1` → (None, cell).
fn split_sheet_cell(part: &str) -> (Option<&str>, &str) {
    let mut quoted = false;
    let mut dot = None;
    for (i, ch) in part.char_indices() {
        if ch == '\'' {
            quoted = !quoted;
        } else if ch == '.' && !quoted {
            dot = Some(i);
        }
    }
    match dot {
        Some(0) => (None, &part[1..]),
        Some(i) => (Some(&part[..i]), &part[i + 1..]),
        None => (None, part),
    }
}

fn sheet_key(raw: &str) -> String {
    let t = raw.trim().trim_start_matches('$');
    match t.strip_prefix('\'').and_then(|x| x.strip_suffix('\'')) {
        Some(inner) => inner.replace("''", "'"),
        None => t.to_string(),
    }
}

fn parse_cell(s: &str) -> Option<(u32, u32)> {
    let t: String = s.chars().filter(|c| *c != '$').collect();
    let split = t.find(|c: char| c.is_ascii_digit())?;
    let (letters, digits) = t.split_at(split);
    let col = letters_to_col(letters)?;
    let row: u32 = digits.parse().ok()?;
    ((1..=MAX_ROWS).contains(&row) && col < MAX_COLS).then(|| (row - 1, col))
}

/// Resolves sheet names of a cell-range-address against the document.
pub(crate) struct SheetLookup<'a> {
    pub names: &'a [String],
    /// The chart's own sheet (for addresses without a sheet part).
    pub own: u32,
}

impl SheetLookup<'_> {
    fn index(&self, raw: &str) -> Option<u32> {
        let key = sheet_key(raw);
        self.names
            .iter()
            .position(|n| n.eq_ignore_ascii_case(&key))
            .map(|i| i as u32)
    }
}

/// A space-separated cell-range-address list → a range (`invalid` when a
/// part names an unknown sheet or does not parse).
pub(crate) fn parse_range_address(addr: &str, look: &SheetLookup<'_>) -> ChartRangeData {
    let mut out = ChartRangeData::default();
    let t = addr.trim();
    if t.is_empty() {
        return out;
    }
    let mut parts: Vec<String> = Vec::new();
    let mut quoted = false;
    let mut cur = String::new();
    for ch in t.chars() {
        if ch == '\'' {
            quoted = !quoted;
        }
        if ch == ' ' && !quoted {
            if !cur.is_empty() {
                parts.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(ch);
        }
    }
    if !cur.is_empty() {
        parts.push(cur);
    }
    for p in parts.iter().take(64) {
        let ends = split_unquoted(p, ':');
        if ends.is_empty() || ends.len() > 2 {
            out.invalid = true;
            continue;
        }
        let (s1, c1) = split_sheet_cell(ends[0].trim());
        let Some(a) = parse_cell(c1) else {
            out.invalid = true;
            continue;
        };
        let (s2, c2) = match ends.get(1) {
            Some(e) => split_sheet_cell(e.trim()),
            None => (None, c1),
        };
        let Some(b) = parse_cell(c2) else {
            out.invalid = true;
            continue;
        };
        let sheet = match s1.or(s2) {
            Some(s) => match look.index(s) {
                Some(i) => i,
                None => {
                    out.invalid = true;
                    continue;
                }
            },
            None => look.own,
        };
        out.areas.push((
            sheet,
            CellRangeLocal {
                r0: a.0.min(b.0),
                c0: a.1.min(b.1),
                r1: a.0.max(b.0),
                c1: a.1.max(b.1),
            },
        ));
    }
    if !out.areas.is_empty() {
        out.invalid = false;
    }
    out
}

// ---------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------

/// Automatic chart styles collected while writing.
#[derive(Default)]
struct Styles {
    out: String,
    n: usize,
}

impl Styles {
    /// Adds a style and returns its name. Empty property groups are left
    /// out.
    fn add(&mut self, chart: &str, graphic: &str, text: &str) -> String {
        self.n += 1;
        let name = format!("ch{}", self.n);
        let _ = write!(
            self.out,
            r#"<style:style style:name="{name}" style:family="chart">"#
        );
        if !chart.is_empty() {
            let _ = write!(self.out, "<style:chart-properties{chart}/>");
        }
        if !graphic.is_empty() {
            let _ = write!(self.out, "<style:graphic-properties{graphic}/>");
        }
        if !text.is_empty() {
            let _ = write!(self.out, "<style:text-properties{text}/>");
        }
        self.out.push_str("</style:style>");
        name
    }
}

fn attr(out: &mut String, key: &str, value: &str) {
    out.push(' ');
    out.push_str(key);
    out.push_str("=\"");
    escape_attr(value, out);
    out.push('"');
}

fn attr_bool(out: &mut String, key: &str, v: bool) {
    attr(out, key, if v { "true" } else { "false" });
}

fn class_name(kind: ChartKindData, hole: bool, filled: bool) -> &'static str {
    match kind {
        ChartKindData::Column | ChartKindData::Bar => "chart:bar",
        ChartKindData::Line => "chart:line",
        ChartKindData::Area => "chart:area",
        ChartKindData::Pie => {
            if hole {
                "chart:ring"
            } else {
                "chart:circle"
            }
        }
        ChartKindData::Scatter => "chart:scatter",
        ChartKindData::Bubble => "chart:bubble",
        ChartKindData::Radar => {
            if filled {
                "chart:filled-radar"
            } else {
                "chart:radar"
            }
        }
        ChartKindData::Stock => "chart:stock",
    }
}

fn legend_position(l: LegendPosData) -> Option<&'static str> {
    match l {
        LegendPosData::None => None,
        LegendPosData::Left => Some("start"),
        LegendPosData::Right => Some("end"),
        LegendPosData::Top => Some("top"),
        LegendPosData::Bottom => Some("bottom"),
    }
}

fn symbol_name(m: MarkerShapeData) -> Option<&'static str> {
    match m {
        MarkerShapeData::Auto | MarkerShapeData::None => None,
        MarkerShapeData::Circle => Some("circle"),
        MarkerShapeData::Square => Some("square"),
        MarkerShapeData::Diamond => Some("diamond"),
        MarkerShapeData::Triangle => Some("arrow-up"),
        MarkerShapeData::Cross => Some("x"),
        MarkerShapeData::Plus => Some("plus"),
    }
}

fn label_position(p: LabelPosData) -> Option<&'static str> {
    match p {
        LabelPosData::Auto => None,
        LabelPosData::Outside => Some("outside"),
        LabelPosData::Inside => Some("inside"),
        LabelPosData::Center => Some("center"),
        LabelPosData::Above => Some("top"),
        LabelPosData::Below => Some("bottom"),
    }
}

fn axis_style(st: &mut Styles, a: &ChartAxisData) -> String {
    let mut p = String::new();
    attr_bool(&mut p, "chart:display-label", a.visible);
    attr_bool(&mut p, "chart:visible", a.visible);
    attr_bool(&mut p, "chart:logarithmic", a.log);
    attr_bool(&mut p, "chart:reverse-direction", a.reverse);
    if let Some(v) = a.min {
        attr(&mut p, "chart:minimum", &fmt_f64(v));
    }
    if let Some(v) = a.max {
        attr(&mut p, "chart:maximum", &fmt_f64(v));
    }
    if let Some(v) = a.major {
        attr(&mut p, "chart:interval-major", &fmt_f64(v));
    }
    if let Some(v) = a.minor_count {
        attr(&mut p, "chart:interval-minor-divisor", &v.to_string());
    }
    if let Some(f) = &a.number_format {
        attr(&mut p, "tally:number-format", f);
    }
    // ODF 1.2 axis positioning, as AOO 4.1 writes it
    // (XMLAxisPositionPropertyHdl, aXMLChartAxisLabelPositionEnumMap).
    match a.crosses {
        Some(AxisCrossData::Start) => attr(&mut p, "chart:axis-position", "start"),
        Some(AxisCrossData::End) => attr(&mut p, "chart:axis-position", "end"),
        Some(AxisCrossData::Value(v)) => attr(&mut p, "chart:axis-position", &fmt_f64(v)),
        None => {}
    }
    let label_pos = match a.label_position {
        AxisLabelPosData::NearAxis => None,
        AxisLabelPosData::NearAxisOtherSide => Some("near-axis-other-side"),
        AxisLabelPosData::OutsideStart => Some("outside-start"),
        AxisLabelPosData::OutsideEnd => Some("outside-end"),
    };
    if let Some(v) = label_pos {
        attr(&mut p, "chart:axis-label-position", v);
    }
    let mark_pos = match a.mark_position {
        AxisMarkPosData::AtLabelsAndAxis => None,
        AxisMarkPosData::AtLabels => Some("at-labels"),
        AxisMarkPosData::AtAxis => Some("at-axis"),
    };
    if let Some(v) = mark_pos {
        attr(&mut p, "chart:tick-mark-position", v);
    }
    st.add(&p, "", "")
}

fn time_unit_token(u: TimeUnitData) -> &'static str {
    match u {
        TimeUnitData::Days => "days",
        TimeUnitData::Months => "months",
        TimeUnitData::Years => "years",
    }
}

fn time_unit_of(v: &str) -> TimeUnitData {
    // AOO's lcl_getTimeUnit: anything else is days.
    match v {
        "months" => TimeUnitData::Months,
        "years" => TimeUnitData::Years,
        _ => TimeUnitData::Days,
    }
}

/// `chartooo:axis-type` and the `chartooo:date-scale` element of a
/// category axis, as AOO 4.1 writes them (SchXMLExport.cxx
/// `lcl_exportAxisType` / `exportDateScale`): `auto` and `date` carry a
/// date scale with the resolution and intervals that are not automatic.
fn axis_type_attr(a: &ChartAxisData) -> &'static str {
    match a.axis_type {
        AxisTypeData::Auto => "auto",
        AxisTypeData::Text => "text",
        AxisTypeData::Date => "date",
    }
}

fn write_date_scale(s: &mut String, a: &ChartAxisData) {
    if a.axis_type == AxisTypeData::Text {
        return;
    }
    s.push_str("<chartooo:date-scale");
    if let Some(u) = a.time_resolution {
        attr(s, "chart:base-time-unit", time_unit_token(u));
    }
    if let Some(t) = a.major_time {
        attr(s, "chart:major-interval-value", &t.number.to_string());
        attr(s, "chart:major-interval-unit", time_unit_token(t.unit));
    }
    if let Some(t) = a.minor_time {
        attr(s, "chart:minor-interval-value", &t.number.to_string());
        attr(s, "chart:minor-interval-unit", time_unit_token(t.unit));
    }
    s.push_str("/>");
}

fn write_axis(
    s: &mut String,
    st: &mut Styles,
    dim: &str,
    name: &str,
    a: &ChartAxisData,
    categories: Option<&str>,
    category_axis: bool,
) {
    let style = axis_style(st, a);
    let _ = write!(
        s,
        r#"<chart:axis chart:dimension="{dim}" chart:name="{name}" chart:style-name="{style}""#
    );
    if category_axis {
        attr(s, "chartooo:axis-type", axis_type_attr(a));
    }
    s.push('>');
    if category_axis {
        write_date_scale(s, a);
    }
    if let Some(t) = &a.title {
        let ts = st.add("", "", r#" fo:font-size="9pt""#);
        let _ = write!(s, r#"<chart:title chart:style-name="{ts}"><text:p>"#);
        escape_text(t, s);
        s.push_str("</text:p></chart:title>");
    }
    if let Some(c) = categories {
        s.push_str("<chart:categories");
        attr(s, "table:cell-range-address", c);
        s.push_str("/>");
    }
    if a.major_grid {
        s.push_str(r#"<chart:grid chart:class="major"/>"#);
    }
    if a.minor_grid {
        s.push_str(r#"<chart:grid chart:class="minor"/>"#);
    }
    s.push_str("</chart:axis>");
}

fn series_style(st: &mut Styles, c: &ChartData, sv: &ChartSeriesData) -> String {
    let mut p = String::new();
    let kind = sv.kind.unwrap_or(c.kind);
    let line_like = matches!(
        kind,
        ChartKindData::Line | ChartKindData::Scatter | ChartKindData::Radar
    );
    if line_like {
        // The series symbol only when it overrides the chart's `markers`.
        match sv.marker {
            Some(MarkerShapeData::None) => attr(&mut p, "chart:symbol-type", "none"),
            Some(MarkerShapeData::Auto) => attr(&mut p, "chart:symbol-type", "automatic"),
            Some(m) => {
                attr(&mut p, "chart:symbol-type", "named-symbol");
                attr(
                    &mut p,
                    "chart:symbol-name",
                    symbol_name(m).unwrap_or("circle"),
                );
                attr(&mut p, "chart:symbol-width", "0.25cm");
                attr(&mut p, "chart:symbol-height", "0.25cm");
            }
            None => {}
        }
    }
    let labels = &sv.labels;
    let number = match (labels.value, labels.percent) {
        (true, true) => "value-and-percentage",
        (true, false) => "value",
        (false, true) => "percentage",
        (false, false) => "none",
    };
    attr(&mut p, "chart:data-label-number", number);
    attr_bool(&mut p, "chart:data-label-text", labels.category);
    if labels.series_name {
        attr_bool(&mut p, "tally:data-label-series", true);
    }
    if let Some(pos) = label_position(labels.position) {
        attr(&mut p, "chart:label-position", pos);
    }
    if c.exploded && matches!(kind, ChartKindData::Pie) {
        attr(&mut p, "chart:pie-offset", "15");
    }
    let mut g = String::new();
    if let Some(col) = sv.color {
        attr(&mut g, "draw:fill", "solid");
        attr(&mut g, "draw:fill-color", &color_hex(col));
        attr(&mut g, "svg:stroke-color", &color_hex(col));
    }
    if let Some(SeriesNameData::Text(t)) = &sv.name {
        attr(&mut p, "tally:label-text", t);
    }
    st.add(&p, &g, "")
}

fn error_bar_style(st: &mut Styles, e: &ErrorBarsData, sheets: &[SheetData]) -> String {
    let mut p = String::new();
    let category = match &e.kind {
        ErrorBarKindData::Constant { plus, minus } => {
            attr(&mut p, "chart:error-upper-limit", &fmt_f64(*plus));
            attr(&mut p, "chart:error-lower-limit", &fmt_f64(*minus));
            "constant"
        }
        ErrorBarKindData::Percent { percent } => {
            attr(&mut p, "chart:error-percentage", &fmt_f64(*percent));
            "percentage"
        }
        ErrorBarKindData::StdDev { k } => {
            attr(&mut p, "tally:error-k", &fmt_f64(*k));
            "standard-deviation"
        }
        ErrorBarKindData::Variance => "variance",
        ErrorBarKindData::StdError => "standard-error",
        ErrorBarKindData::ErrorMargin { percent } => {
            attr(&mut p, "chart:error-margin", &fmt_f64(*percent));
            "error-margin"
        }
        ErrorBarKindData::Range { plus, minus } => {
            if let Some(a) = range_address(plus, sheets) {
                attr(&mut p, "chart:error-upper-range", &a);
            }
            if let Some(a) = range_address(minus, sheets) {
                attr(&mut p, "chart:error-lower-range", &a);
            }
            "cell-range"
        }
    };
    attr(&mut p, "chart:error-category", category);
    let (up, lo) = match e.direction {
        ErrorBarDirData::Both => (true, true),
        ErrorBarDirData::Plus => (true, false),
        ErrorBarDirData::Minus => (false, true),
    };
    attr_bool(&mut p, "chart:error-upper-indicator", up);
    attr_bool(&mut p, "chart:error-lower-indicator", lo);
    st.add(&p, "", "")
}

fn regression_type(k: TrendKindData) -> &'static str {
    match k {
        TrendKindData::Linear => "linear",
        TrendKindData::Logarithmic => "logarithmic",
        TrendKindData::Exponential => "exponential",
        TrendKindData::Power => "power",
    }
}

/// A cell lookup of one sheet, built on demand for the local table.
pub(crate) struct CellIndex<'a> {
    sheets: &'a [SheetData],
    maps: HashMap<u32, HashMap<(u32, u32), &'a CellContentData>>,
}

impl<'a> CellIndex<'a> {
    pub(crate) fn new(sheets: &'a [SheetData]) -> Self {
        Self {
            sheets,
            maps: HashMap::new(),
        }
    }

    fn value(&mut self, tab: u32, r: u32, c: u32) -> Option<Value> {
        let map = match self.maps.get(&tab) {
            Some(m) => m,
            None => {
                let sheet = self.sheets.get(tab as usize)?;
                let m: HashMap<(u32, u32), &CellContentData> = sheet
                    .cells
                    .iter()
                    .map(|cell| ((cell.row, cell.col), &cell.content))
                    .collect();
                self.maps.entry(tab).or_insert(m)
            }
        };
        let content = map.get(&(r, c))?;
        match content {
            CellContentData::Value(v) => Some(v.clone()),
            CellContentData::Formula { cached, .. }
            | CellContentData::SharedFormula { cached, .. } => cached.clone(),
        }
    }

    /// The cells of a range in reading order (at most `cap`).
    fn cells(&mut self, r: &ChartRangeData, cap: usize) -> Option<Vec<Value>> {
        if r.invalid || r.areas.is_empty() {
            return None;
        }
        let mut out = Vec::new();
        for (tab, a) in &r.areas {
            for row in a.r0..=a.r1 {
                for col in a.c0..=a.c1 {
                    if out.len() >= cap {
                        return None;
                    }
                    out.push(self.value(*tab, row, col).unwrap_or(Value::Empty));
                }
            }
        }
        Some(out)
    }
}

fn value_text(v: &Value) -> String {
    match v {
        Value::Empty => String::new(),
        Value::Number(n) => fmt_f64(*n),
        Value::Text(t) => t.to_string(),
        Value::Bool(b) => if *b { "TRUE" } else { "FALSE" }.into(),
        Value::Error(e) => e.display().into_owned(),
        Value::Array(_) => String::new(),
    }
}

fn local_cell(s: &mut String, v: &Value) {
    match v {
        Value::Number(n) => {
            let _ = write!(
                s,
                r#"<table:table-cell office:value-type="float" office:value="{}"><text:p>{}</text:p></table:table-cell>"#,
                fmt_f64(*n),
                fmt_f64(*n)
            );
        }
        Value::Empty => s.push_str("<table:table-cell/>"),
        other => {
            s.push_str(r#"<table:table-cell office:value-type="string"><text:p>"#);
            escape_text(&value_text(other), s);
            s.push_str("</text:p></table:table-cell>");
        }
    }
}

/// The `local-table` of cached values (header row: series names; first
/// column: categories). Skipped for very large charts.
fn write_local_table(
    s: &mut String,
    c: &ChartData,
    cells: &mut CellIndex<'_>,
    categories: Option<&ChartRangeData>,
) {
    let mut columns: Vec<(String, Vec<Value>)> = Vec::new();
    for (i, sv) in c.series.iter().enumerate() {
        let vals = if !sv.values.is_unset() {
            match cells.cells(&sv.values, MAX_CACHED_POINTS) {
                Some(v) => v,
                None => return,
            }
        } else {
            match &sv.literal_values {
                Some(ChartValuesData::Literal(v)) => v
                    .iter()
                    .take(MAX_CACHED_POINTS)
                    .map(|x| x.map_or(Value::Empty, Value::Number))
                    .collect(),
                Some(ChartValuesData::LiteralText(t)) => t
                    .iter()
                    .take(MAX_CACHED_POINTS)
                    .map(|x| Value::text(x.as_str()))
                    .collect(),
                None => Vec::new(),
            }
        };
        let name = match &sv.name {
            Some(SeriesNameData::Text(t)) => t.clone(),
            Some(SeriesNameData::Ref(r)) => cells
                .cells(r, 1)
                .and_then(|v| v.first().map(value_text))
                .unwrap_or_default(),
            None => format!("Series {}", i + 1),
        };
        columns.push((name, vals));
    }
    let cats: Vec<Value> = match (categories, &c.literal_categories) {
        (Some(r), _) if !r.is_unset() => match cells.cells(r, MAX_CACHED_POINTS) {
            Some(v) => v,
            None => return,
        },
        (_, Some(ChartValuesData::LiteralText(t))) => {
            t.iter().map(|x| Value::text(x.as_str())).collect()
        }
        (_, Some(ChartValuesData::Literal(v))) => v
            .iter()
            .map(|x| x.map_or(Value::Empty, Value::Number))
            .collect(),
        _ => Vec::new(),
    };
    let rows = columns
        .iter()
        .map(|(_, v)| v.len())
        .max()
        .unwrap_or(0)
        .max(cats.len());
    s.push_str(r#"<table:table table:name="local-table"><table:table-header-columns><table:table-column/></table:table-header-columns><table:table-columns>"#);
    let _ = write!(
        s,
        r#"<table:table-column table:number-columns-repeated="{}"/>"#,
        columns.len().max(1)
    );
    s.push_str("</table:table-columns><table:table-header-rows><table:table-row><table:table-cell><text:p/></table:table-cell>");
    for (name, _) in &columns {
        s.push_str(r#"<table:table-cell office:value-type="string"><text:p>"#);
        escape_text(name, s);
        s.push_str("</text:p></table:table-cell>");
    }
    s.push_str("</table:table-row></table:table-header-rows><table:table-rows>");
    for i in 0..rows {
        s.push_str("<table:table-row>");
        match cats.get(i) {
            Some(v) if !matches!(v, Value::Empty) => {
                s.push_str(r#"<table:table-cell office:value-type="string"><text:p>"#);
                escape_text(&value_text(v), s);
                s.push_str("</text:p></table:table-cell>");
            }
            _ => {
                let _ = write!(
                    s,
                    r#"<table:table-cell office:value-type="string"><text:p>{}</text:p></table:table-cell>"#,
                    i + 1
                );
            }
        }
        for (_, vals) in &columns {
            local_cell(s, vals.get(i).unwrap_or(&Value::Empty));
        }
        s.push_str("</table:table-row>");
    }
    s.push_str("</table:table-rows></table:table>");
}

/// Writes `Object N/content.xml` for a chart.
pub(crate) fn content_xml(
    c: &ChartData,
    sheets: &[SheetData],
    cells: &mut CellIndex<'_>,
) -> String {
    let mut st = Styles::default();
    let mut body = String::new();

    // Chart element.
    let mut g = String::new();
    match c.background {
        Some(col) => {
            attr(&mut g, "draw:fill", "solid");
            attr(&mut g, "draw:fill-color", &color_hex(col));
        }
        None => attr(&mut g, "draw:fill", "none"),
    }
    attr(&mut g, "draw:stroke", "none");
    let chart_style = st.add("", &g, "");
    body.push_str("<chart:chart");
    attr(&mut body, "svg:width", &hmm_to_cm(c.anchor.width_hmm));
    attr(&mut body, "svg:height", &hmm_to_cm(c.anchor.height_hmm));
    attr(
        &mut body,
        "chart:class",
        class_name(c.kind, c.hole.is_some(), c.filled),
    );
    attr(&mut body, "chart:style-name", &chart_style);
    attr(&mut body, "xlink:href", "..");
    attr(&mut body, "xlink:type", "simple");
    if let Some(h) = c.hole {
        attr(&mut body, "tally:hole", &fmt_f64(f64::from(h)));
    }
    body.push('>');

    if let Some(t) = &c.title {
        let ts = st.add(
            r#" chart:auto-position="true""#,
            "",
            r#" fo:font-size="13pt""#,
        );
        let _ = write!(body, r#"<chart:title chart:style-name="{ts}"><text:p>"#);
        escape_text(t, &mut body);
        body.push_str("</text:p></chart:title>");
    }
    if let Some(t) = &c.subtitle {
        let ts = st.add(
            r#" chart:auto-position="true""#,
            "",
            r#" fo:font-size="11pt""#,
        );
        let _ = write!(body, r#"<chart:subtitle chart:style-name="{ts}"><text:p>"#);
        escape_text(t, &mut body);
        body.push_str("</text:p></chart:subtitle>");
    }
    if let Some(pos) = legend_position(c.legend) {
        let ls = st.add(
            r#" chart:auto-position="true""#,
            "",
            r#" fo:font-size="10pt""#,
        );
        let _ = write!(
            body,
            r#"<chart:legend chart:legend-position="{pos}" style:legend-expansion="wide" chart:style-name="{ls}"/>"#
        );
    }

    // Plot area.
    let mut p = String::new();
    attr_bool(&mut p, "chart:stacked", c.stacking == StackingData::Stacked);
    attr_bool(
        &mut p,
        "chart:percentage",
        c.stacking == StackingData::Percent,
    );
    attr_bool(&mut p, "chart:three-dimensional", c.three_d);
    if matches!(c.kind, ChartKindData::Column | ChartKindData::Bar) {
        attr_bool(&mut p, "chart:vertical", c.kind == ChartKindData::Bar);
    }
    if matches!(
        c.kind,
        ChartKindData::Line | ChartKindData::Scatter | ChartKindData::Radar
    ) {
        attr(
            &mut p,
            "chart:symbol-type",
            if c.markers { "automatic" } else { "none" },
        );
        attr_bool(&mut p, "chart:lines", c.lines);
        if c.smooth {
            attr(&mut p, "chart:interpolation", "cubic-spline");
        }
    }
    if c.kind == ChartKindData::Stock {
        let v = c.stock.unwrap_or_default();
        attr_bool(
            &mut p,
            "chart:japanese-candle-stick",
            matches!(v, StockVariantData::Ohlc | StockVariantData::VolumeOhlc),
        );
        attr_bool(
            &mut p,
            "chart:stock-with-volume",
            matches!(
                v,
                StockVariantData::VolumeHlc | StockVariantData::VolumeOhlc
            ),
        );
    }
    attr_bool(&mut p, "chart:include-hidden-cells", c.include_hidden);
    let mut g = String::new();
    match c.plot_background {
        Some(col) => {
            attr(&mut g, "draw:fill", "solid");
            attr(&mut g, "draw:fill-color", &color_hex(col));
        }
        None => attr(&mut g, "draw:fill", "none"),
    }
    let plot_style = st.add(&p, &g, "");
    body.push_str("<chart:plot-area");
    attr(&mut body, "chart:style-name", &plot_style);
    if let Some(src) = &c.source {
        if let Some(a) = range_address(&src.range, sheets) {
            attr(&mut body, "table:cell-range-address", &a);
        }
        let labels = match (src.first_row_label, src.first_col_label) {
            (true, true) => "both",
            (true, false) => "row",
            (false, true) => "column",
            (false, false) => "none",
        };
        attr(&mut body, "chart:data-source-has-labels", labels);
        attr_bool(&mut body, "tally:series-in-rows", src.series_in_rows);
    }
    body.push('>');

    let categories = c.categories.as_ref().filter(|r| !r.is_unset());
    let cat_addr = categories.and_then(|r| range_address(r, sheets));
    write_axis(
        &mut body,
        &mut st,
        "x",
        "primary-x",
        &c.x_axis,
        cat_addr.as_deref(),
        !matches!(c.kind, ChartKindData::Scatter | ChartKindData::Bubble),
    );
    write_axis(&mut body, &mut st, "y", "primary-y", &c.y_axis, None, false);
    if let Some(y2) = &c.y2_axis {
        write_axis(&mut body, &mut st, "y", "secondary-y", y2, None, false);
    }

    for sv in c.series.iter().take(MAX_SERIES) {
        let ss = series_style(&mut st, c, sv);
        body.push_str("<chart:series");
        attr(&mut body, "chart:style-name", &ss);
        let kind = sv.kind.unwrap_or(c.kind);
        // Bubble: the series values are the sizes (LibreOffice convention).
        let values = if kind == ChartKindData::Bubble {
            sv.sizes.as_ref().unwrap_or(&sv.values)
        } else {
            &sv.values
        };
        if let Some(a) = range_address(values, sheets) {
            attr(&mut body, "chart:values-cell-range-address", &a);
        }
        if let Some(SeriesNameData::Ref(r)) = &sv.name {
            if let Some(a) = range_address(r, sheets) {
                attr(&mut body, "chart:label-cell-address", &a);
            }
        }
        if let Some(k) = sv.kind {
            attr(&mut body, "chart:class", class_name(k, false, false));
        }
        if sv.secondary_axis {
            attr(&mut body, "chart:attached-axis", "secondary-y");
        }
        body.push('>');
        // Domains: scatter has one (x); bubble has two (y, then x), its
        // values being the sizes (LibreOffice convention).
        if kind == ChartKindData::Bubble {
            if let Some(a) = range_address(&sv.values, sheets) {
                body.push_str("<chart:domain");
                attr(&mut body, "table:cell-range-address", &a);
                body.push_str("/>");
            }
            if let Some(a) = sv.x_values.as_ref().and_then(|r| range_address(r, sheets)) {
                body.push_str("<chart:domain");
                attr(&mut body, "table:cell-range-address", &a);
                body.push_str("/>");
            }
        } else if let Some(a) = sv.x_values.as_ref().and_then(|r| range_address(r, sheets)) {
            body.push_str("<chart:domain");
            attr(&mut body, "table:cell-range-address", &a);
            body.push_str("/>");
        }
        if let Some(t) = &sv.trend {
            let mut p = String::new();
            attr(&mut p, "chart:regression-type", regression_type(t.kind));
            if let Some(n) = &t.name {
                attr(&mut p, "tally:regression-name", n);
            }
            let rs = st.add(&p, "", "");
            let _ = write!(body, r#"<chart:regression-curve chart:style-name="{rs}">"#);
            let _ = write!(
                body,
                r#"<chart:equation chart:display-equation="{}" chart:display-r-square="{}"/>"#,
                t.show_equation, t.show_r2
            );
            body.push_str("</chart:regression-curve>");
        }
        if let Some(e) = &sv.error_bars {
            let es = error_bar_style(&mut st, e, sheets);
            let _ = write!(
                body,
                r#"<chart:error-indicator chart:dimension="y" chart:style-name="{es}"/>"#
            );
        }
        if sv.mean_line {
            let ms = st.add("", r##" svg:stroke-color="#b3b3b3""##, "");
            let _ = write!(body, r#"<chart:mean-value chart:style-name="{ms}"/>"#);
        }
        body.push_str("</chart:series>");
    }
    if c.kind == ChartKindData::Stock {
        body.push_str(
            "<chart:stock-gain-marker/><chart:stock-loss-marker/><chart:stock-range-line/>",
        );
    }
    body.push_str("<chart:wall/><chart:floor/></chart:plot-area>");

    // Bubble sizes live in `values`; the local table writes series values.
    let mut sized = c.clone();
    if c.kind == ChartKindData::Bubble {
        for s in &mut sized.series {
            if let Some(z) = s.sizes.take() {
                s.values = z;
            }
        }
    }
    write_local_table(&mut body, &sized, cells, categories);
    body.push_str("</chart:chart>");

    let mut out = String::with_capacity(body.len() + st.out.len() + 1024);
    out.push_str(XML_DECL);
    let _ = write!(
        out,
        r#"<office:document-content{CHART_NS} office:version="1.2"><office:automatic-styles>{}</office:automatic-styles><office:body><office:chart>{body}</office:chart></office:body></office:document-content>"#,
        st.out
    );
    out
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

/// One automatic chart style: the attributes of its property groups.
#[derive(Default, Debug, Clone)]
struct ChartStyle {
    chart: Vec<(String, String)>,
    graphic: Vec<(String, String)>,
}

impl ChartStyle {
    fn chart_attr(&self, key: &str) -> Option<&str> {
        self.chart
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    fn graphic_attr(&self, key: &str) -> Option<&str> {
        self.graphic
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    fn flag(&self, key: &str) -> Option<bool> {
        self.chart_attr(key).map(|v| v == "true")
    }

    fn number(&self, key: &str) -> Option<f64> {
        self.chart_attr(key)
            .and_then(|v| v.trim().parse::<f64>().ok())
            .filter(|v| v.is_finite())
    }
}

fn parse_styles<R: BufRead>(
    xml: &mut XmlReader<R>,
    out: &mut HashMap<String, ChartStyle>,
) -> Result<(), OdsError> {
    let d = xml.depth();
    let mut current: Option<(String, ChartStyle)> = None;
    loop {
        match xml.next()? {
            Ev::Start(e) => match e.name.as_str() {
                "style:style" => {
                    if let Some((n, s)) = current.take() {
                        out.insert(n, s);
                    }
                    if e.attr("style:family") == Some("chart") {
                        if let Some(n) = e.attr("style:name") {
                            if out.len() < 10_000 {
                                current = Some((n.to_string(), ChartStyle::default()));
                            }
                        }
                    }
                }
                "style:chart-properties" => {
                    if let Some((_, s)) = current.as_mut() {
                        s.chart = e.attrs.clone();
                    }
                    xml.skip()?;
                }
                "style:graphic-properties" => {
                    if let Some((_, s)) = current.as_mut() {
                        s.graphic = e.attrs.clone();
                    }
                    xml.skip()?;
                }
                _ => xml.skip()?,
            },
            Ev::End if xml.depth() < d => break,
            Ev::End => {
                if xml.depth() == d {
                    if let Some((n, s)) = current.take() {
                        out.insert(n, s);
                    }
                }
            }
            Ev::Eof => break,
            _ => {}
        }
    }
    if let Some((n, s)) = current.take() {
        out.insert(n, s);
    }
    Ok(())
}

fn class_of(raw: &str) -> (ChartKindData, bool, bool, bool) {
    // (kind, ring, filled radar, known)
    let local = raw.rsplit(':').next().unwrap_or(raw);
    match local {
        "bar" => (ChartKindData::Column, false, false, true),
        "line" => (ChartKindData::Line, false, false, true),
        "area" => (ChartKindData::Area, false, false, true),
        "circle" => (ChartKindData::Pie, false, false, true),
        "ring" => (ChartKindData::Pie, true, false, true),
        "scatter" => (ChartKindData::Scatter, false, false, true),
        "bubble" => (ChartKindData::Bubble, false, false, true),
        "radar" => (ChartKindData::Radar, false, false, true),
        "filled-radar" => (ChartKindData::Radar, false, true, true),
        "stock" => (ChartKindData::Stock, false, false, true),
        _ => (ChartKindData::Column, false, false, false),
    }
}

fn legend_of(pos: &str) -> LegendPosData {
    match pos {
        "start" | "left" => LegendPosData::Left,
        "top" => LegendPosData::Top,
        "bottom" => LegendPosData::Bottom,
        _ => LegendPosData::Right,
    }
}

fn marker_of(name: &str) -> MarkerShapeData {
    match name {
        "circle" => MarkerShapeData::Circle,
        "square" => MarkerShapeData::Square,
        "diamond" => MarkerShapeData::Diamond,
        "arrow-up" | "arrow-down" | "arrow-left" | "arrow-right" => MarkerShapeData::Triangle,
        "x" => MarkerShapeData::Cross,
        "plus" => MarkerShapeData::Plus,
        _ => MarkerShapeData::Auto,
    }
}

fn label_pos_of(p: &str) -> LabelPosData {
    match p {
        "outside" => LabelPosData::Outside,
        "inside" => LabelPosData::Inside,
        "center" => LabelPosData::Center,
        "top" | "top-left" | "top-right" => LabelPosData::Above,
        "bottom" | "bottom-left" | "bottom-right" => LabelPosData::Below,
        _ => LabelPosData::Auto,
    }
}

fn axis_of(
    style: Option<&ChartStyle>,
    grids: (bool, bool),
    title: Option<String>,
) -> ChartAxisData {
    let mut a = ChartAxisData {
        title,
        major_grid: grids.0,
        minor_grid: grids.1,
        ..ChartAxisData::default()
    };
    if let Some(s) = style {
        a.crosses = match s.chart_attr("chart:axis-position") {
            Some("start") => Some(AxisCrossData::Start),
            Some("end") => Some(AxisCrossData::End),
            // "0" is AOO's default (ChartAxisPosition_ZERO).
            Some(v) => v
                .trim()
                .parse::<f64>()
                .ok()
                .filter(|v| v.is_finite() && *v != 0.0)
                .map(AxisCrossData::Value),
            None => None,
        };
        a.label_position = match s.chart_attr("chart:axis-label-position") {
            Some("near-axis-other-side") => AxisLabelPosData::NearAxisOtherSide,
            Some("outside-start" | "outside-minimum") => AxisLabelPosData::OutsideStart,
            Some("outside-end" | "outside-maximum") => AxisLabelPosData::OutsideEnd,
            _ => AxisLabelPosData::NearAxis,
        };
        a.mark_position = match s.chart_attr("chart:tick-mark-position") {
            Some("at-labels") => AxisMarkPosData::AtLabels,
            Some("at-axis") => AxisMarkPosData::AtAxis,
            _ => AxisMarkPosData::AtLabelsAndAxis,
        };
    }
    if let Some(s) = style {
        a.visible = s.flag("chart:visible").unwrap_or(true)
            && s.flag("chart:display-label").unwrap_or(true);
        a.log = s.flag("chart:logarithmic").unwrap_or(false);
        a.reverse = s.flag("chart:reverse-direction").unwrap_or(false);
        a.min = s.number("chart:minimum");
        a.max = s.number("chart:maximum");
        a.major = s.number("chart:interval-major").filter(|v| *v > 0.0);
        a.minor_count = s
            .chart_attr("chart:interval-minor-divisor")
            .and_then(|v| v.parse::<u32>().ok())
            .filter(|v| *v > 0);
        a.number_format = s
            .chart_attr("tally:number-format")
            .map(|f| f.chars().take(MAX_TEXT).collect());
    }
    a
}

/// The scale type of a category axis as AOO 4.1 reads it
/// (SchXMLAxisContext.cxx): `text` and `date` as written; `auto` (or no
/// attribute) is a date-capable axis only with a `date-scale` element,
/// else text (`AutoDateAxis = false`, "different default for older
/// documents").
fn apply_axis_type(a: &mut ChartAxisData, kind: Option<&str>, scale: Option<&[(String, String)]>) {
    a.axis_type = match kind {
        Some("text") => AxisTypeData::Text,
        Some("date") => AxisTypeData::Date,
        _ if scale.is_some() => AxisTypeData::Auto,
        _ => AxisTypeData::Text,
    };
    let Some(scale) = scale else { return };
    let get = |k: &str| scale.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
    a.time_resolution = get("chart:base-time-unit").map(time_unit_of);
    let interval = |value: &str, unit: &str| -> Option<TimeIntervalData> {
        let v = get(value);
        let u = get(unit);
        if v.is_none() && u.is_none() {
            return None;
        }
        // AOO starts from TimeInterval(1, DAY) and overrides what is set.
        let number = v.map_or(Some(1), |v| v.trim().parse::<u32>().ok())?;
        (1..=10_000).contains(&number).then(|| TimeIntervalData {
            number,
            unit: u.map_or(TimeUnitData::Days, time_unit_of),
        })
    };
    a.major_time = interval("chart:major-interval-value", "chart:major-interval-unit");
    a.minor_time = interval("chart:minor-interval-value", "chart:minor-interval-unit");
}

/// Cached values of a `local-table`.
#[derive(Default, Debug)]
struct LocalTable {
    header: Vec<String>,
    rows: Vec<Vec<Value>>,
}

fn parse_local_cell<R: BufRead>(xml: &mut XmlReader<R>, e: &Elem) -> Result<Value, OdsError> {
    let text = xml.text_content(MAX_TEXT)?;
    Ok(match e.attr("office:value-type") {
        Some("float" | "percentage" | "currency") => e
            .attr("office:value")
            .and_then(|v| v.trim().parse::<f64>().ok())
            .filter(|v| v.is_finite())
            .map_or(Value::Empty, Value::Number),
        Some(_) => {
            if text.is_empty() {
                Value::Empty
            } else {
                Value::text(text.as_str())
            }
        }
        None => Value::Empty,
    })
}

fn parse_local_table<R: BufRead>(xml: &mut XmlReader<R>) -> Result<LocalTable, OdsError> {
    let mut t = LocalTable::default();
    let d = xml.depth();
    let mut in_header = false;
    let mut row: Option<Vec<Value>> = None;
    loop {
        match xml.next()? {
            Ev::Start(e) => match e.name.as_str() {
                "table:table-header-rows" => in_header = true,
                "table:table-rows" => in_header = false,
                "table:table-row" => row = Some(Vec::new()),
                "table:table-cell" => {
                    let v = parse_local_cell(xml, &e)?;
                    if let Some(r) = row.as_mut() {
                        if r.len() < MAX_SERIES + 1 {
                            r.push(v);
                        }
                    }
                }
                "table:table-header-columns" | "table:table-columns" => xml.skip()?,
                _ => {}
            },
            Ev::End if xml.depth() < d => break,
            Ev::End => {
                if let Some(mut r) = row.take_if(|_| xml.depth() == d + 1 || xml.depth() == d + 2) {
                    if in_header && t.header.is_empty() {
                        t.header = r.iter().map(value_text).collect();
                    } else if t.rows.len() < MAX_LOCAL_ROWS {
                        // Only real values are kept: trailing empty cells
                        // are dropped and an empty row holds no allocation.
                        while matches!(r.last(), Some(Value::Empty)) {
                            r.pop();
                        }
                        if r.is_empty() {
                            r = Vec::new();
                        } else {
                            r.shrink_to_fit();
                        }
                        t.rows.push(r);
                    }
                }
            }
            Ev::Eof => break,
            _ => {}
        }
    }
    // Trailing empty rows carry no data.
    while t.rows.last().is_some_and(Vec::is_empty) {
        t.rows.pop();
    }
    Ok(t)
}

/// An axis as read, before its style is applied.
struct RawAxis {
    dim: String,
    name: String,
    style: Option<String>,
    /// (major, minor) grid.
    grids: (bool, bool),
    title: Option<String>,
    categories: Option<String>,
    /// `chartooo:axis-type` (or `chart:axis-type`).
    axis_type: Option<String>,
    /// The `date-scale` element's attributes, when present.
    date_scale: Option<Vec<(String, String)>>,
}

/// What the reader gathers per series before the styles are applied.
#[derive(Default)]
struct RawSeries {
    style: Option<String>,
    values: Option<String>,
    label: Option<String>,
    class: Option<String>,
    attached: Option<String>,
    domains: Vec<String>,
    regression: Option<(Option<String>, bool, bool)>,
    error: Option<String>,
    mean: bool,
    data_points_class_bar: bool,
}

/// Reads an object `content.xml` into a chart. `look` resolves sheet
/// names; `anchor` supplies the placement from the frame.
/// `literal_budget` is what remains of the document's
/// [`tally_model::MAX_LITERAL_POINTS`]: the `local-table` values taken
/// are subtracted, and series past it keep no cached values.
pub(crate) fn parse_content_xml<R: BufRead>(
    xml: &mut XmlReader<R>,
    look: &SheetLookup<'_>,
    warnings: &mut Vec<String>,
    literal_budget: &mut usize,
) -> Result<Option<ChartData>, OdsError> {
    let mut styles: HashMap<String, ChartStyle> = HashMap::new();
    let mut c = ChartData::default();
    let mut found = false;
    let mut known_class = true;
    let mut ring = false;
    let mut chart_style: Option<String> = None;
    let mut plot_style: Option<String> = None;
    let mut series: Vec<RawSeries> = Vec::new();
    let mut dropped_series = 0usize;
    let mut axes: Vec<RawAxis> = Vec::new();
    let mut local: Option<LocalTable> = None;
    let mut legend_seen = false;
    // Current open elements of interest.
    let mut cur_axis: Option<(usize, u32)> = None;
    // (kind, depth): 1 title, 2 subtitle, 3 axis title.
    let mut in_title: Option<(u8, u32)> = None;
    let mut text_buf = String::new();
    loop {
        match xml.next()? {
            Ev::Start(e) => match e.name.as_str() {
                "office:automatic-styles" => parse_styles(xml, &mut styles)?,
                "office:document-content" | "office:body" | "office:chart" => {}
                "chart:chart" => {
                    found = true;
                    let (kind, is_ring, filled, known) =
                        class_of(e.attr("chart:class").unwrap_or(""));
                    c.kind = kind;
                    ring = is_ring;
                    c.filled = filled;
                    known_class = known;
                    chart_style = e.attr("chart:style-name").map(str::to_string);
                    c.hole = e
                        .attr("tally:hole")
                        .and_then(|h| h.parse::<f32>().ok())
                        .filter(|h| h.is_finite() && (0.0..=0.95).contains(h))
                        .or(ring.then_some(0.5));
                    if let Some(w) = e.attr("svg:width").and_then(length_to_hmm) {
                        c.anchor.width_hmm = w.max(100);
                    }
                    if let Some(h) = e.attr("svg:height").and_then(length_to_hmm) {
                        c.anchor.height_hmm = h.max(100);
                    }
                }
                "chart:title" => {
                    in_title = Some((if cur_axis.is_some() { 3 } else { 1 }, xml.depth()));
                    text_buf.clear();
                }
                "chart:subtitle" => {
                    in_title = Some((2, xml.depth()));
                    text_buf.clear();
                }
                "text:p" | "text:span" if in_title.is_some() => {
                    // Accumulate paragraph text (nested spans flatten).
                    if e.name == "text:p" && !text_buf.is_empty() {
                        text_buf.push('\n');
                    }
                }
                "chart:legend" => {
                    legend_seen = true;
                    c.legend = legend_of(e.attr("chart:legend-position").unwrap_or("end"));
                    xml.skip()?;
                }
                "chart:plot-area" => {
                    plot_style = e.attr("chart:style-name").map(str::to_string);
                    if let Some(a) = e.attr("table:cell-range-address") {
                        let range = parse_range_address(a, look);
                        let labels = e.attr("chart:data-source-has-labels").unwrap_or("none");
                        c.source = Some(ChartSourceData {
                            range,
                            series_in_rows: e.attr("tally:series-in-rows") == Some("true"),
                            first_row_label: matches!(labels, "both" | "row"),
                            first_col_label: matches!(labels, "both" | "column"),
                        });
                    }
                }
                "chart:axis" => {
                    if axes.len() < 8 {
                        axes.push(RawAxis {
                            dim: e.attr("chart:dimension").unwrap_or("").to_string(),
                            name: e.attr("chart:name").unwrap_or("").to_string(),
                            style: e.attr("chart:style-name").map(str::to_string),
                            grids: (false, false),
                            title: None,
                            categories: None,
                            axis_type: e
                                .attr("chartooo:axis-type")
                                .or_else(|| e.attr("chart:axis-type"))
                                .map(str::to_string),
                            date_scale: None,
                        });
                        cur_axis = Some((axes.len() - 1, xml.depth()));
                    } else {
                        xml.skip()?;
                    }
                }
                "chartooo:date-scale" | "chart:date-scale" | "loext:date-scale" => {
                    if let Some((i, _)) = cur_axis {
                        axes[i].date_scale = Some(
                            e.attrs
                                .iter()
                                .filter(|(k, _)| k.starts_with("chart:"))
                                .take(8)
                                .cloned()
                                .collect(),
                        );
                    }
                    xml.skip()?;
                }
                "chart:categories" => {
                    if let Some((i, _)) = cur_axis {
                        axes[i].categories = e.attr("table:cell-range-address").map(str::to_string);
                    }
                    xml.skip()?;
                }
                "chart:grid" => {
                    if let Some((i, _)) = cur_axis {
                        match e.attr("chart:class") {
                            Some("minor") => axes[i].grids.1 = true,
                            _ => axes[i].grids.0 = true,
                        }
                    }
                    xml.skip()?;
                }
                "chart:series" => {
                    if series.len() >= MAX_SERIES {
                        dropped_series += 1;
                        xml.skip()?;
                        continue;
                    }
                    series.push(RawSeries {
                        style: e.attr("chart:style-name").map(str::to_string),
                        values: e
                            .attr("chart:values-cell-range-address")
                            .map(str::to_string),
                        label: e.attr("chart:label-cell-address").map(str::to_string),
                        class: e.attr("chart:class").map(str::to_string),
                        attached: e.attr("chart:attached-axis").map(str::to_string),
                        ..RawSeries::default()
                    });
                }
                "chart:domain" => {
                    if let (Some(s), Some(a)) =
                        (series.last_mut(), e.attr("table:cell-range-address"))
                    {
                        if s.domains.len() < 2 {
                            s.domains.push(a.to_string());
                        }
                    }
                    xml.skip()?;
                }
                "chart:regression-curve" => {
                    if let Some(s) = series.last_mut() {
                        s.regression =
                            Some((e.attr("chart:style-name").map(str::to_string), false, false));
                    }
                }
                "chart:equation" => {
                    if let Some(Some(r)) = series.last_mut().map(|s| s.regression.as_mut()) {
                        r.1 = e.attr("chart:display-equation") == Some("true");
                        r.2 = e.attr("chart:display-r-square") == Some("true");
                    }
                    xml.skip()?;
                }
                "chart:error-indicator" => {
                    if let Some(s) = series.last_mut() {
                        if e.attr("chart:dimension").is_none_or(|d| d == "y") {
                            s.error = e.attr("chart:style-name").map(str::to_string);
                        }
                    }
                    xml.skip()?;
                }
                "chart:mean-value" => {
                    if let Some(s) = series.last_mut() {
                        s.mean = true;
                    }
                    xml.skip()?;
                }
                "chart:data-point" => {
                    if let Some(s) = series.last_mut() {
                        s.data_points_class_bar |=
                            e.attr("chart:class").is_some_and(|c| c.ends_with("bar"));
                    }
                    xml.skip()?;
                }
                "table:table" => {
                    if local.is_none() {
                        local = Some(parse_local_table(xml)?);
                    } else {
                        xml.skip()?;
                    }
                }
                "chart:wall"
                | "chart:floor"
                | "chart:stock-gain-marker"
                | "chart:stock-loss-marker"
                | "chart:stock-range-line" => xml.skip()?,
                _ => {
                    if in_title.is_none() {
                        xml.skip()?;
                    }
                }
            },
            Ev::Text(t) => {
                if in_title.is_some() && text_buf.len() + t.len() <= MAX_TEXT {
                    text_buf.push_str(&t);
                }
            }
            Ev::End => {
                if let Some((kind, d)) = in_title {
                    if xml.depth() < d {
                        let text: String = text_buf.trim().chars().take(MAX_TEXT).collect();
                        let text = (!text.is_empty()).then_some(text);
                        match kind {
                            1 => c.title = text,
                            2 => c.subtitle = text,
                            _ => {
                                if let Some((i, _)) = cur_axis {
                                    axes[i].title = text;
                                }
                            }
                        }
                        in_title = None;
                        text_buf.clear();
                    }
                } else if cur_axis.is_some_and(|(_, d)| xml.depth() < d) {
                    cur_axis = None;
                }
            }
            Ev::Eof => break,
        }
    }
    if !found {
        return Ok(None);
    }
    if !known_class {
        warnings.push("A chart of an unknown type was read as a column chart.".into());
    }
    if dropped_series > 0 {
        warnings.push(format!(
            "Only the first {MAX_SERIES} series of a chart were kept ({dropped_series} dropped)."
        ));
    }
    if !legend_seen {
        c.legend = LegendPosData::None;
    }
    // Chart and plot-area styles.
    if let Some(s) = chart_style.as_ref().and_then(|n| styles.get(n)) {
        if s.graphic_attr("draw:fill").is_none_or(|f| f != "none") {
            c.background = s.graphic_attr("draw:fill-color").and_then(parse_color);
        }
    }
    let mut stock_volume = false;
    let mut candle = false;
    if let Some(s) = plot_style.as_ref().and_then(|n| styles.get(n)) {
        c.stacking = if s.flag("chart:percentage").unwrap_or(false) {
            StackingData::Percent
        } else if s.flag("chart:stacked").unwrap_or(false) {
            StackingData::Stacked
        } else {
            StackingData::None
        };
        c.three_d = s.flag("chart:three-dimensional").unwrap_or(false);
        if c.kind == ChartKindData::Column && s.flag("chart:vertical").unwrap_or(false) {
            c.kind = ChartKindData::Bar;
        }
        if let Some(t) = s.chart_attr("chart:symbol-type") {
            c.markers = t != "none";
        } else {
            c.markers = matches!(c.kind, ChartKindData::Scatter);
        }
        c.lines = s.flag("chart:lines").unwrap_or(true);
        c.smooth = s
            .chart_attr("chart:interpolation")
            .is_some_and(|i| i != "none");
        candle = s.flag("chart:japanese-candle-stick").unwrap_or(false);
        stock_volume = s.flag("chart:stock-with-volume").unwrap_or(false);
        c.include_hidden = s.flag("chart:include-hidden-cells").unwrap_or(false);
        if s.graphic_attr("draw:fill").is_none_or(|f| f != "none") {
            c.plot_background = s.graphic_attr("draw:fill-color").and_then(parse_color);
        }
    } else {
        c.markers = matches!(c.kind, ChartKindData::Scatter);
    }
    // Axes.
    let mut categories: Option<ChartRangeData> = None;
    for ax in axes {
        let st = ax.style.as_ref().and_then(|n| styles.get(n));
        let mut a = axis_of(st, ax.grids, ax.title);
        if ax.dim == "x" && !matches!(c.kind, ChartKindData::Scatter | ChartKindData::Bubble) {
            apply_axis_type(&mut a, ax.axis_type.as_deref(), ax.date_scale.as_deref());
        }
        if let Some(addr) = ax.categories {
            categories = Some(parse_range_address(&addr, look));
        }
        match (ax.dim.as_str(), ax.name.as_str()) {
            ("x", _) => c.x_axis = a,
            ("y", "secondary-y") => c.y2_axis = Some(a),
            ("y", _) => c.y_axis = a,
            _ => {}
        }
    }
    c.categories = categories.filter(|r| !r.is_unset() || r.invalid);
    // Series.
    let local = local.unwrap_or_default();
    let mut literal_dropped = false;
    let mut first_bar_volume = false;
    for (i, raw) in series.into_iter().enumerate() {
        let st = raw.style.as_ref().and_then(|n| styles.get(n));
        let mut sv = ChartSeriesData::default();
        if let Some(a) = &raw.values {
            sv.values = parse_range_address(a, look);
        }
        if sv.values.is_unset() {
            // Cached values from the local table (column i + 1), one per
            // row up to the last non-empty row.
            let len = local.rows.len();
            if len > *literal_budget {
                literal_dropped = true;
            } else if len > 0 {
                *literal_budget -= len;
                let col: Vec<Option<f64>> = local.rows[..len]
                    .iter()
                    .map(|r| match r.get(i + 1) {
                        Some(Value::Number(n)) => Some(*n),
                        _ => None,
                    })
                    .collect();
                sv.literal_values = Some(ChartValuesData::Literal(col));
            }
        }
        if let Some(a) = &raw.label {
            sv.name = Some(SeriesNameData::Ref(parse_range_address(a, look)));
        } else if let Some(t) = st.and_then(|s| s.chart_attr("tally:label-text")) {
            sv.name = Some(SeriesNameData::Text(t.chars().take(MAX_TEXT).collect()));
        } else if let Some(h) = local.header.get(i + 1).filter(|h| !h.is_empty()) {
            sv.name = Some(SeriesNameData::Text(h.clone()));
        }
        let class = raw.class.as_deref().map(class_of);
        if let Some((k, _, _, _)) = class {
            if k != c.kind {
                sv.kind = Some(k);
            }
        }
        if c.kind == ChartKindData::Stock
            && i == 0
            && (stock_volume
                || matches!(class, Some((ChartKindData::Column, ..)))
                || raw.data_points_class_bar)
        {
            first_bar_volume = true;
            sv.kind = None;
        }
        sv.secondary_axis = raw.attached.as_deref() == Some("secondary-y");
        // Domains.
        let kind = sv.kind.unwrap_or(c.kind);
        if kind == ChartKindData::Bubble {
            // LibreOffice: values = sizes, domain[0] = y, domain[1] = x.
            let sizes = std::mem::take(&mut sv.values);
            sv.sizes = Some(sizes).filter(|r| !r.is_unset());
            if let Some(y) = raw.domains.first() {
                sv.values = parse_range_address(y, look);
            }
            if let Some(x) = raw.domains.get(1) {
                sv.x_values = Some(parse_range_address(x, look));
            }
        } else if let Some(x) = raw.domains.first() {
            sv.x_values = Some(parse_range_address(x, look));
        }
        if let Some(s) = st {
            sv.color = s
                .graphic_attr("draw:fill-color")
                .or_else(|| s.graphic_attr("svg:stroke-color"))
                .and_then(parse_color);
            // LibreOffice writes a symbol type on every series; only
            // line-like series can show one.
            let line_like = matches!(
                kind,
                ChartKindData::Line | ChartKindData::Scatter | ChartKindData::Radar
            );
            sv.marker = match s.chart_attr("chart:symbol-type") {
                _ if !line_like => None,
                Some("none") => Some(MarkerShapeData::None),
                Some("named-symbol") => {
                    Some(marker_of(s.chart_attr("chart:symbol-name").unwrap_or("")))
                }
                Some("automatic") => Some(MarkerShapeData::Auto),
                _ => None,
            };
            let number = s.chart_attr("chart:data-label-number").unwrap_or("none");
            sv.labels = DataLabelsData {
                value: matches!(number, "value" | "value-and-percentage"),
                percent: matches!(number, "percentage" | "value-and-percentage"),
                category: s.flag("chart:data-label-text").unwrap_or(false),
                series_name: s.flag("tally:data-label-series").unwrap_or(false)
                    || s.flag("chart:data-label-series").unwrap_or(false),
                position: label_pos_of(s.chart_attr("chart:label-position").unwrap_or("")),
            };
            if s.number("chart:pie-offset").is_some_and(|v| v > 0.0) {
                c.exploded = true;
            }
        }
        if let Some((rs, eq, r2)) = raw.regression {
            let rst = rs.as_ref().and_then(|n| styles.get(n));
            let kind = match rst.and_then(|s| s.chart_attr("chart:regression-type")) {
                Some("logarithmic") => TrendKindData::Logarithmic,
                Some("exponential") => TrendKindData::Exponential,
                Some("power") => TrendKindData::Power,
                Some("linear") | None => TrendKindData::Linear,
                Some(other) => {
                    warnings.push(format!(
                        "A {other} trend line is not supported and was read as linear."
                    ));
                    TrendKindData::Linear
                }
            };
            sv.trend = Some(TrendLineData {
                kind,
                show_equation: eq,
                show_r2: r2,
                name: rst
                    .and_then(|s| s.chart_attr("tally:regression-name"))
                    .map(|n| n.chars().take(MAX_TEXT).collect()),
            });
        }
        if let Some(es) = raw.error.as_ref().and_then(|n| styles.get(n)) {
            let kind = match es.chart_attr("chart:error-category") {
                Some("constant") => Some(ErrorBarKindData::Constant {
                    plus: es.number("chart:error-upper-limit").unwrap_or(0.0).abs(),
                    minus: es.number("chart:error-lower-limit").unwrap_or(0.0).abs(),
                }),
                Some("percentage") => Some(ErrorBarKindData::Percent {
                    percent: es.number("chart:error-percentage").unwrap_or(0.0).abs(),
                }),
                Some("standard-deviation") => Some(ErrorBarKindData::StdDev {
                    k: es.number("tally:error-k").unwrap_or(1.0).abs(),
                }),
                Some("variance") => Some(ErrorBarKindData::Variance),
                Some("standard-error") => Some(ErrorBarKindData::StdError),
                Some("error-margin") => Some(ErrorBarKindData::ErrorMargin {
                    percent: es.number("chart:error-margin").unwrap_or(0.0).abs(),
                }),
                Some("cell-range") => Some(ErrorBarKindData::Range {
                    plus: es
                        .chart_attr("chart:error-upper-range")
                        .map(|a| parse_range_address(a, look))
                        .unwrap_or_default(),
                    minus: es
                        .chart_attr("chart:error-lower-range")
                        .map(|a| parse_range_address(a, look))
                        .unwrap_or_default(),
                }),
                _ => None,
            };
            if let Some(kind) = kind {
                let up = es.flag("chart:error-upper-indicator").unwrap_or(true);
                let lo = es.flag("chart:error-lower-indicator").unwrap_or(true);
                sv.error_bars = Some(ErrorBarsData {
                    kind,
                    direction: match (up, lo) {
                        (true, false) => ErrorBarDirData::Plus,
                        (false, true) => ErrorBarDirData::Minus,
                        _ => ErrorBarDirData::Both,
                    },
                });
            }
        }
        sv.mean_line = raw.mean;
        c.series.push(sv);
    }
    if c.kind == ChartKindData::Stock {
        c.stock = Some(match (first_bar_volume, candle) {
            (true, true) => StockVariantData::VolumeOhlc,
            (true, false) => StockVariantData::VolumeHlc,
            (false, true) => StockVariantData::Ohlc,
            (false, false) => StockVariantData::Hlc,
        });
    }
    if c.categories.is_none()
        && !local.rows.is_empty()
        && c.series.iter().any(|s| s.literal_values.is_some())
    {
        let cats: Vec<String> = local
            .rows
            .iter()
            .map(|r| r.first().map(value_text).unwrap_or_default())
            .collect();
        if !cats.iter().any(|t| !t.is_empty()) {
            // No category text: nothing to keep.
        } else if cats.len() > *literal_budget {
            literal_dropped = true;
        } else {
            *literal_budget -= cats.len();
            c.literal_categories = Some(ChartValuesData::LiteralText(cats));
        }
    }
    if literal_dropped {
        warnings.push(format!(
            "Chart cached values past {} points per document were not read.",
            tally_model::MAX_LITERAL_POINTS
        ));
    }
    if ring && c.hole.is_none() {
        c.hole = Some(0.5);
    }
    Ok(Some(c))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_range_addresses() {
        let names = vec!["Sheet1".to_string(), "My Sheet".to_string()];
        let look = SheetLookup {
            names: &names,
            own: 0,
        };
        let r = parse_range_address("Sheet1.B2:Sheet1.B9", &look);
        assert_eq!(
            r.areas,
            vec![(
                0,
                CellRangeLocal {
                    r0: 1,
                    c0: 1,
                    r1: 8,
                    c1: 1
                }
            )]
        );
        let r = parse_range_address("$'My Sheet'.$A$1:.$C$3 Sheet1.D1", &look);
        assert_eq!(r.areas.len(), 2);
        assert_eq!(r.areas[0].0, 1);
        assert_eq!(
            r.areas[1].1,
            CellRangeLocal {
                r0: 0,
                c0: 3,
                r1: 0,
                c1: 3
            }
        );
        let r = parse_range_address("Nope.A1:Nope.A2", &look);
        assert!(r.invalid && r.areas.is_empty());
        assert!(parse_range_address("", &look).is_unset());
    }
}
