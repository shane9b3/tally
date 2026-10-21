//! Chart parts (wave 5, W5-1): `xl/charts/chartN.xml` (DrawingML
//! charts) → [`ChartData`].
//!
//! The part is read into a small bounded element tree (the XML reader
//! already caps depth and attributes; this caps the node count) and
//! interpreted from there: chart groups (`c:barChart`… including the 3D
//! variants, kept as a flag), series with `c:tx`/`c:cat`/`c:val`/
//! `c:xVal`/`c:yVal`/`c:bubbleSize` in reference or literal form,
//! grouping, titles (rich text flattened), legend, axes (scaling,
//! gridlines, titles, number formats), data labels, trend lines, error
//! bars and series colours. Surface charts are skipped with a warning;
//! unsupported trend line types become linear with a warning.

use std::io::BufRead;

use tally_model::{
    CellRangeLocal, ChartAxisData, ChartData, ChartKindData, ChartRangeData, ChartSeriesData,
    ChartValuesData, DataLabelsData, ErrorBarDirData, ErrorBarKindData, ErrorBarsData,
    LabelPosData, LegendPosData, MarkerShapeData, SeriesNameData, StackingData, StockVariantData,
    TrendKindData, TrendLineData,
};

use crate::colors::{parse_argb, THEME};
use crate::sheet::parse_range;
use crate::xml::{Ev, XmlReader};
use crate::XlsxError;

/// Most elements read from one chart part.
const MAX_NODES: usize = 400_000;
/// Most series per chart.
pub(crate) const MAX_SERIES: usize = 255;
/// Most literal points per series.
const MAX_POINTS: usize = 32_000;
/// Longest text kept.
const MAX_TEXT: usize = 4_096;

/// A bounded element tree node.
#[derive(Debug, Default)]
struct Node {
    name: String,
    attrs: Vec<(String, String)>,
    children: Vec<Node>,
    text: String,
}

impl Node {
    fn attr(&self, key: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| crate::xml::local_name(k) == key && !k.starts_with("xmlns"))
            .map(|(_, v)| v.as_str())
    }

    fn val(&self) -> Option<&str> {
        self.attr("val")
    }

    fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name == name)
    }

    fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }

    fn child_val(&self, name: &str) -> Option<&str> {
        self.child(name).and_then(Node::val)
    }

    fn child_flag(&self, name: &str, default: bool) -> bool {
        match self.child(name).map(|c| c.val().unwrap_or("1")) {
            Some(v) => crate::units::parse_bool(v),
            None => default,
        }
    }

    fn child_f64(&self, name: &str) -> Option<f64> {
        self.child_val(name)
            .and_then(|v| v.trim().parse::<f64>().ok())
            .filter(|v| v.is_finite())
    }

    /// Every `a:t` text under the node, concatenated (rich text).
    fn rich_text(&self, out: &mut String) {
        for c in &self.children {
            if c.name == "t" {
                if out.len() + c.text.len() <= MAX_TEXT {
                    out.push_str(&c.text);
                }
            } else {
                if c.name == "p" && !out.is_empty() {
                    out.push('\n');
                }
                c.rich_text(out);
            }
        }
    }
}

fn read_tree<R: BufRead>(xr: &mut XmlReader<R>) -> Result<Option<Node>, XlsxError> {
    let mut count = 0usize;
    let mut stack: Vec<Node> = Vec::new();
    let mut root: Option<Node> = None;
    loop {
        match xr.next()? {
            Ev::Start(e) => {
                count += 1;
                if count > MAX_NODES {
                    return Err(XlsxError::LimitExceeded {
                        what: "chart XML elements",
                        limit: MAX_NODES as u64,
                    });
                }
                stack.push(Node {
                    name: e.local().to_string(),
                    attrs: e.attrs,
                    children: Vec::new(),
                    text: String::new(),
                });
            }
            Ev::Text(t) => {
                if let Some(n) = stack.last_mut() {
                    if n.text.len() + t.len() <= MAX_TEXT {
                        n.text.push_str(&t);
                    }
                }
            }
            Ev::End => {
                let Some(n) = stack.pop() else {
                    continue;
                };
                match stack.last_mut() {
                    Some(p) => p.children.push(n),
                    None => {
                        if root.is_none() {
                            root = Some(n);
                        }
                    }
                }
            }
            Ev::Eof => break,
        }
    }
    Ok(root)
}

// ---------------------------------------------------------------------------
// References
// ---------------------------------------------------------------------------

/// `Sheet1!$A$2:$A$10`, `'My Sheet'!$B$1`, `(Sheet1!$A$1:$A$3,Sheet1!$C$1:$C$3)`
/// → a range; unknown sheets and unparsable parts make it `invalid`.
pub(crate) fn parse_formula_ref(text: &str, names: &[String]) -> ChartRangeData {
    let mut out = ChartRangeData::default();
    let t = text.trim();
    let t = t
        .strip_prefix('(')
        .and_then(|x| x.strip_suffix(')'))
        .unwrap_or(t);
    // Split on commas outside quotes.
    let mut parts: Vec<&str> = Vec::new();
    let mut quoted = false;
    let mut start = 0;
    for (i, ch) in t.char_indices() {
        match ch {
            '\'' => quoted = !quoted,
            ',' if !quoted => {
                parts.push(&t[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&t[start..]);
    for p in parts.iter().take(64) {
        let p = p.trim();
        if p.is_empty() {
            continue;
        }
        let (sheet, cells) = match split_sheet(p) {
            Some(x) => x,
            None => {
                out.invalid = true;
                continue;
            }
        };
        let idx = names.iter().position(|n| n.eq_ignore_ascii_case(&sheet));
        let Some(idx) = idx else {
            out.invalid = true;
            continue;
        };
        let cells: String = cells.chars().filter(|c| *c != '$').collect();
        match parse_range(&cells) {
            Some(r) => out.areas.push((idx as u32, r)),
            None => out.invalid = true,
        }
    }
    if !out.areas.is_empty() {
        out.invalid = false;
    }
    out
}

/// `'My Sheet'!A1:B2` → (`My Sheet`, `A1:B2`).
fn split_sheet(p: &str) -> Option<(String, &str)> {
    if let Some(rest) = p.strip_prefix('\'') {
        // Quoted sheet name: find the closing quote (doubled quotes escape).
        let bytes = rest.as_bytes();
        let mut i = 0;
        let mut name = String::new();
        while i < bytes.len() {
            if bytes[i] == b'\'' {
                if bytes.get(i + 1) == Some(&b'\'') {
                    name.push('\'');
                    i += 2;
                    continue;
                }
                let after = &rest[i + 1..];
                let cells = after.strip_prefix('!')?;
                return Some((name, cells));
            }
            let ch = rest[i..].chars().next()?;
            name.push(ch);
            i += ch.len_utf8();
        }
        None
    } else {
        let (s, c) = p.rsplit_once('!')?;
        Some((s.to_string(), c))
    }
}

// ---------------------------------------------------------------------------
// Interpretation
// ---------------------------------------------------------------------------

/// The largest point index + 1 a `numLit`/`strLit` may fill: below
/// `ptCount` when present, below [`MAX_POINTS`], and at most a small
/// multiple of the `pt` elements present, so one `<c:pt idx="31999">`
/// cannot make 32,000 slots.
fn literal_limit(lit: &Node) -> usize {
    let pts = lit.children_named("pt").take(MAX_POINTS).count();
    let count = lit
        .child_val("ptCount")
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(MAX_POINTS);
    MAX_POINTS
        .min(count)
        .min(pts.saturating_mul(4).saturating_add(64))
}

fn data_ref(node: Option<&Node>, names: &[String]) -> (ChartRangeData, Option<ChartValuesData>) {
    let Some(n) = node else {
        return (ChartRangeData::default(), None);
    };
    for c in &n.children {
        match c.name.as_str() {
            "numRef" | "strRef" | "multiLvlStrRef" => {
                if let Some(f) = c.child("f") {
                    return (parse_formula_ref(&f.text, names), None);
                }
            }
            "numLit" => {
                let mut v: Vec<Option<f64>> = Vec::new();
                let limit = literal_limit(c);
                for pt in c.children_named("pt").take(MAX_POINTS) {
                    let idx: usize = pt
                        .attr("idx")
                        .and_then(|i| i.parse().ok())
                        .unwrap_or(v.len());
                    if idx >= limit {
                        continue;
                    }
                    while v.len() <= idx {
                        v.push(None);
                    }
                    v[idx] = pt
                        .child("v")
                        .and_then(|x| x.text.trim().parse::<f64>().ok());
                }
                return (ChartRangeData::default(), Some(ChartValuesData::Literal(v)));
            }
            "strLit" => {
                let mut v: Vec<String> = Vec::new();
                let limit = literal_limit(c);
                for pt in c.children_named("pt").take(MAX_POINTS) {
                    let idx: usize = pt
                        .attr("idx")
                        .and_then(|i| i.parse().ok())
                        .unwrap_or(v.len());
                    if idx >= limit {
                        continue;
                    }
                    while v.len() <= idx {
                        v.push(String::new());
                    }
                    v[idx] = pt
                        .child("v")
                        .map(|x| x.text.chars().take(MAX_TEXT).collect())
                        .unwrap_or_default();
                }
                return (
                    ChartRangeData::default(),
                    Some(ChartValuesData::LiteralText(v)),
                );
            }
            _ => {}
        }
    }
    (ChartRangeData::default(), None)
}

fn fill_color(sp: Option<&Node>) -> Option<u32> {
    let sp = sp?;
    let solid = sp
        .child("solidFill")
        .or_else(|| sp.child("ln").and_then(|l| l.child("solidFill")))?;
    for c in &solid.children {
        match c.name.as_str() {
            "srgbClr" => {
                let v = c.val()?;
                let rgb = u32::from_str_radix(v.trim(), 16).ok()?;
                return Some(((rgb & 0xff_ffff) << 8) | 0xff);
            }
            "schemeClr" => {
                let idx = match c.val()? {
                    "accent1" => 4,
                    "accent2" => 5,
                    "accent3" => 6,
                    "accent4" => 7,
                    "accent5" => 8,
                    "accent6" => 9,
                    "dk1" | "tx1" => 0,
                    "lt1" | "bg1" => 1,
                    "dk2" | "tx2" => 2,
                    "lt2" | "bg2" => 3,
                    _ => return None,
                };
                let rgb = THEME.get(idx).copied()?;
                return Some(((rgb & 0xff_ffff) << 8) | 0xff);
            }
            _ => {}
        }
    }
    None
}

fn title_text(n: Option<&Node>) -> Option<String> {
    let n = n?;
    let mut s = String::new();
    if let Some(tx) = n.child("tx") {
        tx.rich_text(&mut s);
    }
    let s = s.trim().to_string();
    (!s.is_empty()).then_some(s)
}

fn labels_of(dl: Option<&Node>) -> DataLabelsData {
    let Some(d) = dl else {
        return DataLabelsData::default();
    };
    DataLabelsData {
        value: d.child_flag("showVal", false),
        percent: d.child_flag("showPercent", false),
        category: d.child_flag("showCatName", false),
        series_name: d.child_flag("showSerName", false),
        position: match d.child_val("dLblPos") {
            Some("outEnd" | "r") => LabelPosData::Outside,
            Some("inEnd" | "inBase" | "l") => LabelPosData::Inside,
            Some("ctr") => LabelPosData::Center,
            Some("t") => LabelPosData::Above,
            Some("b") => LabelPosData::Below,
            _ => LabelPosData::Auto,
        },
    }
}

fn marker_of(sym: Option<&str>) -> Option<MarkerShapeData> {
    Some(match sym? {
        "none" => MarkerShapeData::None,
        "circle" => MarkerShapeData::Circle,
        "square" => MarkerShapeData::Square,
        "diamond" => MarkerShapeData::Diamond,
        "triangle" => MarkerShapeData::Triangle,
        "x" | "star" => MarkerShapeData::Cross,
        "plus" => MarkerShapeData::Plus,
        _ => MarkerShapeData::Auto,
    })
}

fn axis_of(n: &Node, names_unused: &[String]) -> ChartAxisData {
    let _ = names_unused;
    let scaling = n.child("scaling");
    let mut a = ChartAxisData {
        visible: !n.child_flag("delete", false),
        title: title_text(n.child("title")),
        major_grid: n.child("majorGridlines").is_some(),
        minor_grid: n.child("minorGridlines").is_some(),
        major: n.child_f64("majorUnit").filter(|v| *v > 0.0),
        number_format: n
            .child("numFmt")
            .and_then(|f| f.attr("formatCode"))
            .filter(|f| *f != "General")
            .map(|f| f.chars().take(MAX_TEXT).collect()),
        ..ChartAxisData::default()
    };
    if let Some(s) = scaling {
        a.min = s.child_f64("min");
        a.max = s.child_f64("max");
        a.log = s.child("logBase").is_some();
        a.reverse = s.child_val("orientation") == Some("maxMin");
    }
    if let (Some(major), Some(minor)) = (a.major, n.child_f64("minorUnit")) {
        if minor > 0.0 && major > minor {
            a.minor_count = Some((major / minor).round().clamp(1.0, 1000.0) as u32);
        }
    }
    a
}

/// What a chart group contributes.
struct Group<'a> {
    kind: ChartKindData,
    three_d: bool,
    node: &'a Node,
}

fn group_kind(name: &str) -> Option<(ChartKindData, bool)> {
    Some(match name {
        "barChart" => (ChartKindData::Column, false),
        "bar3DChart" => (ChartKindData::Column, true),
        "lineChart" => (ChartKindData::Line, false),
        "line3DChart" => (ChartKindData::Line, true),
        "areaChart" => (ChartKindData::Area, false),
        "area3DChart" => (ChartKindData::Area, true),
        "pieChart" | "ofPieChart" => (ChartKindData::Pie, false),
        "pie3DChart" => (ChartKindData::Pie, true),
        "doughnutChart" => (ChartKindData::Pie, false),
        "scatterChart" => (ChartKindData::Scatter, false),
        "bubbleChart" => (ChartKindData::Bubble, false),
        "radarChart" => (ChartKindData::Radar, false),
        "stockChart" => (ChartKindData::Stock, false),
        _ => return None,
    })
}

/// Reads a chart part. `names` are the workbook's sheet names (for
/// references); `own` is the sheet the chart sits on (unused by Excel
/// references, which always name a sheet).
pub(crate) fn parse_chart<R: BufRead>(
    xr: &mut XmlReader<R>,
    names: &[String],
    own: u32,
    warnings: &mut Vec<String>,
    literal_budget: &mut usize,
) -> Result<Option<ChartData>, XlsxError> {
    let _ = own;
    let Some(root) = read_tree(xr)? else {
        return Ok(None);
    };
    let space = if root.name == "chartSpace" {
        &root
    } else {
        return Ok(None);
    };
    let Some(chart) = space.child("chart") else {
        return Ok(None);
    };
    let Some(plot) = chart.child("plotArea") else {
        return Ok(None);
    };
    let mut groups: Vec<Group<'_>> = Vec::new();
    let mut surface = false;
    for n in &plot.children {
        match group_kind(&n.name) {
            Some((kind, three_d)) => groups.push(Group {
                kind,
                three_d,
                node: n,
            }),
            None if n.name == "surfaceChart" || n.name == "surface3DChart" => surface = true,
            None => {}
        }
    }
    if groups.is_empty() {
        if surface {
            warnings.push("A surface chart was skipped (not supported).".into());
        }
        return Ok(None);
    }
    if surface {
        warnings.push("A surface chart group was skipped (not supported).".into());
    }
    let main = &groups[0];
    let mut c = ChartData {
        kind: main.kind,
        three_d: groups.iter().any(|g| g.three_d),
        ..ChartData::default()
    };
    let g = main.node;
    if main.kind == ChartKindData::Column && g.child_val("barDir") == Some("bar") {
        c.kind = ChartKindData::Bar;
    }
    c.stacking = match g.child_val("grouping") {
        Some("stacked") => StackingData::Stacked,
        Some("percentStacked") => StackingData::Percent,
        _ => StackingData::None,
    };
    if g.name == "doughnutChart" {
        c.hole = Some(
            g.child_f64("holeSize")
                .map_or(0.5, |h| (h / 100.0).clamp(0.05, 0.95) as f32),
        );
    }
    match main.kind {
        ChartKindData::Line => {
            c.lines = true;
            c.markers = g.child_flag("marker", false);
        }
        ChartKindData::Scatter => {
            let style = g.child_val("scatterStyle").unwrap_or("marker");
            c.lines = !matches!(style, "marker" | "none");
            c.markers = matches!(style, "marker" | "lineMarker" | "smoothMarker" | "none");
            c.smooth = matches!(style, "smooth" | "smoothMarker");
        }
        ChartKindData::Radar => {
            let style = g.child_val("radarStyle").unwrap_or("standard");
            c.filled = style == "filled";
            c.markers = style == "marker";
        }
        ChartKindData::Bubble => {
            c.lines = false;
            c.markers = true;
        }
        _ => {}
    }
    // Series of every group; the first group's kind is the chart's kind.
    let mut hi_low = false;
    let mut up_down = false;
    let mut primary_axes: Vec<String> = g
        .children_named("axId")
        .filter_map(|a| a.val().map(str::to_string))
        .collect();
    primary_axes.truncate(4);
    let mut total = 0usize;
    let mut dropped = 0usize;
    let mut smooth_any = false;
    let mut no_lines: Vec<bool> = Vec::new();
    for grp in &groups {
        let gn = grp.node;
        hi_low |= gn.child("hiLowLines").is_some();
        up_down |= gn.child("upDownBars").is_some();
        let axes: Vec<String> = gn
            .children_named("axId")
            .filter_map(|a| a.val().map(str::to_string))
            .collect();
        let secondary = !axes.is_empty() && axes != primary_axes;
        let group_labels = gn.child("dLbls");
        let mut gkind = grp.kind;
        if gkind == ChartKindData::Column && gn.child_val("barDir") == Some("bar") {
            gkind = ChartKindData::Bar;
        }
        for ser in gn.children_named("ser") {
            total += 1;
            if c.series.len() >= MAX_SERIES {
                dropped += 1;
                continue;
            }
            let mut s = ChartSeriesData::default();
            if let Some(tx) = ser.child("tx") {
                if let Some(f) = tx.child("strRef").and_then(|r| r.child("f")) {
                    s.name = Some(SeriesNameData::Ref(parse_formula_ref(&f.text, names)));
                } else if let Some(v) = tx.child("v").or_else(|| {
                    tx.child("strRef")
                        .and_then(|r| r.child("strCache"))
                        .and_then(|sc| sc.child("pt"))
                        .and_then(|pt| pt.child("v"))
                }) {
                    let t: String = v.text.chars().take(MAX_TEXT).collect();
                    if !t.is_empty() {
                        s.name = Some(SeriesNameData::Text(t));
                    }
                }
            }
            let (cat, cat_lit) = data_ref(ser.child("cat").or_else(|| ser.child("xVal")), names);
            let (val, val_lit) = data_ref(ser.child("val").or_else(|| ser.child("yVal")), names);
            s.values = val;
            s.literal_values = val_lit;
            if matches!(gkind, ChartKindData::Scatter | ChartKindData::Bubble) {
                s.x_values = Some(cat).filter(|r| !r.is_unset());
                s.literal_x = cat_lit;
                let (sz, sz_lit) = data_ref(ser.child("bubbleSize"), names);
                s.sizes = Some(sz).filter(|r| !r.is_unset());
                s.literal_sizes = sz_lit;
            } else if c.categories.is_none() && c.literal_categories.is_none() {
                if !cat.is_unset() || cat.invalid {
                    c.categories = Some(cat);
                } else {
                    c.literal_categories = cat_lit;
                }
            }
            if gkind != c.kind {
                s.kind = Some(gkind);
            }
            s.secondary_axis = secondary;
            s.color = fill_color(ser.child("spPr"));
            // A scatter series drawn without a line (`a:ln/a:noFill`).
            let no_line = ser
                .child("spPr")
                .and_then(|sp| sp.child("ln"))
                .is_some_and(|ln| ln.child("noFill").is_some());
            no_lines.push(no_line);
            s.marker = marker_of(ser.child("marker").and_then(|m| m.child_val("symbol")));
            s.labels = labels_of(ser.child("dLbls").or(group_labels));
            if ser.child_flag("smooth", false) {
                smooth_any = true;
            }
            if ser.child_f64("explosion").is_some_and(|e| e > 0.0) {
                c.exploded = true;
            }
            if let Some(t) = ser.child("trendline") {
                let kind = match t.child_val("trendlineType") {
                    Some("log") => TrendKindData::Logarithmic,
                    Some("exp") => TrendKindData::Exponential,
                    Some("power") => TrendKindData::Power,
                    Some("linear") | None => TrendKindData::Linear,
                    Some(other) => {
                        warnings.push(format!(
                            "A {other} trend line is not supported and was read as linear."
                        ));
                        TrendKindData::Linear
                    }
                };
                s.trend = Some(TrendLineData {
                    kind,
                    show_equation: t.child_flag("dispEq", false),
                    show_r2: t.child_flag("dispRSqr", false),
                    name: t
                        .child("name")
                        .map(|n| n.text.chars().take(MAX_TEXT).collect::<String>())
                        .filter(|n| !n.is_empty()),
                });
            }
            if let Some(e) = ser
                .children_named("errBars")
                .find(|e| e.child_val("errDir").is_none_or(|d| d == "y"))
            {
                let v = e.child_f64("val").unwrap_or(0.0).abs();
                let kind = match e.child_val("errValType") {
                    Some("fixedVal") => Some(ErrorBarKindData::Constant { plus: v, minus: v }),
                    Some("percentage") => Some(ErrorBarKindData::Percent { percent: v }),
                    Some("stdDev") => Some(ErrorBarKindData::StdDev { k: v }),
                    Some("stdErr") => Some(ErrorBarKindData::StdError),
                    Some("cust") => Some(ErrorBarKindData::Range {
                        plus: data_ref(e.child("plus"), names).0,
                        minus: data_ref(e.child("minus"), names).0,
                    }),
                    _ => None,
                };
                if let Some(kind) = kind {
                    s.error_bars = Some(ErrorBarsData {
                        kind,
                        direction: match e.child_val("errBarType") {
                            Some("plus") => ErrorBarDirData::Plus,
                            Some("minus") => ErrorBarDirData::Minus,
                            _ => ErrorBarDirData::Both,
                        },
                    });
                }
            }
            c.series.push(s);
        }
    }
    if smooth_any && c.kind == ChartKindData::Line {
        c.smooth = true;
    }
    if c.kind == ChartKindData::Scatter && !no_lines.is_empty() && no_lines.iter().all(|n| *n) {
        // Points only: a "lineMarker" style whose series all hide the line.
        c.lines = false;
        c.markers = true;
    }
    if dropped > 0 {
        warnings.push(format!(
            "Only the first {MAX_SERIES} of {total} chart series were kept."
        ));
    }
    if c.kind == ChartKindData::Stock {
        let n = c.series.len();
        let volume = c
            .series
            .first()
            .is_some_and(|s| s.kind == Some(ChartKindData::Column))
            || groups.len() > 1 && groups[0].kind == ChartKindData::Column;
        let ohlc = up_down || (n >= 4 && !volume) || (n >= 5 && volume);
        if volume {
            if let Some(f) = c.series.first_mut() {
                f.kind = None;
            }
        }
        let _ = hi_low;
        c.stock = Some(match (volume, ohlc) {
            (true, true) => StockVariantData::VolumeOhlc,
            (true, false) => StockVariantData::VolumeHlc,
            (false, true) => StockVariantData::Ohlc,
            (false, false) => StockVariantData::Hlc,
        });
    }
    // Titles.
    c.title = title_text(chart.child("title"));
    // Legend.
    c.legend = match chart.child("legend") {
        None => LegendPosData::None,
        Some(l) => match l.child_val("legendPos") {
            Some("l") => LegendPosData::Left,
            Some("t") => LegendPosData::Top,
            Some("b") => LegendPosData::Bottom,
            _ => LegendPosData::Right,
        },
    };
    // Absent `plotVisOnly` means hidden cells are plotted (that is how
    // writers switch it on).
    c.include_hidden = match chart.child("plotVisOnly") {
        None => true,
        Some(n) => !crate::units::parse_bool(n.val().unwrap_or("1")),
    };
    // Markers: a "lineMarker"/"marker" style with every series' symbol
    // set to none means no markers (what writers emit for plain lines).
    if matches!(
        c.kind,
        ChartKindData::Line | ChartKindData::Scatter | ChartKindData::Radar
    ) && !c.series.is_empty()
        && c.series
            .iter()
            .all(|s| s.marker == Some(MarkerShapeData::None))
    {
        c.markers = false;
        for s in &mut c.series {
            s.marker = None;
        }
    }
    // Axes: by id, the first group's second axis is the primary value
    // axis; a value axis not referenced by the first group is secondary.
    let mut x_done = false;
    let mut y_done = false;
    for n in &plot.children {
        let is_axis = matches!(n.name.as_str(), "catAx" | "valAx" | "dateAx" | "serAx");
        if !is_axis {
            continue;
        }
        let id = n.child_val("axId").unwrap_or("");
        let pos = n.child_val("axPos").unwrap_or("");
        let a = axis_of(n, names);
        let in_primary = primary_axes.iter().any(|p| p == id);
        let horizontal = matches!(pos, "b" | "t");
        let is_x = if c.kind == ChartKindData::Bar {
            // Bar charts: the category axis is vertical.
            n.name != "valAx"
        } else if n.name == "valAx" && c.kind == ChartKindData::Scatter {
            horizontal
        } else {
            n.name != "valAx"
        };
        if is_x {
            if !x_done && in_primary {
                c.x_axis = a;
                x_done = true;
            }
        } else if in_primary && !y_done {
            c.y_axis = a;
            y_done = true;
        } else if !in_primary && c.y2_axis.is_none() && !is_x {
            c.y2_axis = Some(a);
        }
    }
    if !x_done {
        // No primary x axis found by id: take the first category-like axis.
        if let Some(n) = plot
            .children
            .iter()
            .find(|n| matches!(n.name.as_str(), "catAx" | "dateAx" | "serAx"))
        {
            c.x_axis = axis_of(n, names);
        }
    }
    if !y_done {
        if let Some(n) = plot.children.iter().find(|n| n.name == "valAx") {
            c.y_axis = axis_of(n, names);
        }
    }
    c.background = fill_color(space.child("spPr"));
    c.plot_background = fill_color(plot.child("spPr"));
    let _ = parse_argb;
    let _: Option<CellRangeLocal> = None;
    // The document's literal point budget (see `tally_model::MAX_LITERAL_POINTS`).
    let n = c.literal_points();
    if n > *literal_budget {
        c.drop_literals();
        warnings.push(format!(
            "Chart cached values past {} points per document were not read.",
            tally_model::MAX_LITERAL_POINTS
        ));
    } else {
        *literal_budget -= n;
    }
    Ok(Some(c))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formula_refs() {
        let names = vec!["Sheet1".to_string(), "My Sheet".to_string()];
        let r = parse_formula_ref("Sheet1!$A$2:$A$10", &names);
        assert_eq!(
            r.areas,
            vec![(
                0,
                CellRangeLocal {
                    r0: 1,
                    c0: 0,
                    r1: 9,
                    c1: 0
                }
            )]
        );
        let r = parse_formula_ref("'My Sheet'!$B$1", &names);
        assert_eq!(
            r.areas,
            vec![(
                1,
                CellRangeLocal {
                    r0: 0,
                    c0: 1,
                    r1: 0,
                    c1: 1
                }
            )]
        );
        let r = parse_formula_ref("(Sheet1!$A$1:$A$3,Sheet1!$C$1:$C$3)", &names);
        assert_eq!(r.areas.len(), 2);
        let r = parse_formula_ref("Nope!$A$1", &names);
        assert!(r.invalid && r.areas.is_empty());
        let r = parse_formula_ref("'It''s'!A1", &["It's".to_string()]);
        assert_eq!(r.areas.len(), 1);
    }
}
