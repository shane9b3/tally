//! Wave 5 (W5-2) ODS pieces: DataPilot tables, outline groups,
//! scenarios, named database ranges with their standard/advanced filters
//! and subtotal rules, named cell styles and the view options. The
//! `v04.rs` placeholders and the `content.rs` arms delegate here.

use std::cell::RefCell;
use std::fmt::Write as _;
use std::io::BufRead;

use tally_model::{
    AggFnData, AutoShowData, BaseItemData, CellRangeLocal, CellStyleData, DataLayoutData,
    DatabaseRangeData, DatePartData, OutlineGroupData, PivotData, PivotDataFieldData,
    PivotFieldData, PivotGroupData, PivotLayoutData, PivotSortData, PivotSourceData,
    PivotSubtotalsData, ScenarioData, SheetData, ShowAsData, ShowAsKindData, StandardFilterData,
    StdConditionData, StdConnectorData, StdFilterOpData, StdFilterOptionsData, SubtotalGroupData,
    SubtotalsData, WorkbookData,
};
use tally_types::{DateEpoch, MAX_COLS, MAX_ROWS};

use crate::units::{color_hex, fmt_f64, parse_color};
use crate::v03::{base_address, parse_base, parse_range_list, range_list};
use crate::write::odf_sheet_name;
use crate::xml::{escape_attr, Elem, Ev, XmlReader};
use crate::OdsError;

thread_local! {
    /// The scenario whose (hidden) sheet is being written, so the
    /// `table:scenario` element can be emitted inside its `table:table`.
    static CURRENT_SCENARIO: RefCell<Option<(String, ScenarioData)>> = const { RefCell::new(None) };
}

/// Sets (or clears) the scenario being written and its base sheet name.
pub(crate) fn set_current_scenario(v: Option<(String, ScenarioData)>) {
    CURRENT_SCENARIO.with(|c| *c.borrow_mut() = v);
}

fn bool_attr(s: &mut String, name: &str, v: bool) {
    let _ = write!(s, r#" {name}="{v}""#);
}

fn attr(s: &mut String, name: &str, v: &str) {
    let _ = write!(s, r#" {name}=""#);
    escape_attr(v, s);
    s.push('"');
}

// ---------------------------------------------------------------------------
// Scenarios
// ---------------------------------------------------------------------------

/// Writes the `table:scenario` element when the sheet being written is
/// a scenario sheet (see [`set_current_scenario`]).
pub(crate) fn write_scenario(s: &mut String, _sheet: &SheetData) {
    CURRENT_SCENARIO.with(|c| {
        let Some((base, sc)) = c.borrow().as_ref().cloned() else {
            return;
        };
        s.push_str("<table:scenario");
        attr(
            s,
            "table:scenario-ranges",
            &range_list(&odf_sheet_name(&base), &sc.ranges),
        );
        bool_attr(s, "table:is-active", sc.active);
        bool_attr(s, "table:display-border", sc.display_border);
        if let Some(c) = sc.border_color {
            attr(s, "table:border-color", &color_hex(c));
        }
        bool_attr(s, "table:copy-back", sc.copy_back);
        bool_attr(s, "table:copy-styles", sc.copy_styles);
        bool_attr(s, "table:copy-formulas", true);
        bool_attr(s, "table:protected", sc.prevent_changes);
        if !sc.comment.is_empty() {
            attr(s, "table:comment", &sc.comment);
        }
        s.push_str("/>");
    });
}

/// The synthetic sheet of a scenario: hidden, holding the scenario's
/// cells, named after the scenario.
pub(crate) fn scenario_sheet(base: &SheetData, sc: &ScenarioData, taken: &[String]) -> SheetData {
    let mut name = sc.name.clone();
    let mut n = 2;
    while taken.iter().any(|t| t.eq_ignore_ascii_case(&name)) {
        name = format!("{}_{n}", sc.name);
        n += 1;
    }
    let mut cells = sc.cells.clone();
    cells.sort_by_key(|c| (c.row, c.col));
    SheetData {
        name,
        cells,
        hidden: true,
        tab_color: base.tab_color,
        ..SheetData::default()
    }
}

/// `table:scenario` attributes → a scenario without its cells (they are
/// the sheet's cells, folded in by [`fold_scenarios`]).
pub(crate) fn scenario_from_attrs(e: &Elem) -> ScenarioData {
    let flag = |k: &str, default: bool| e.attr(k).map_or(default, |v| v == "true");
    ScenarioData {
        name: String::new(),
        comment: e.attr("table:comment").unwrap_or_default().to_string(),
        ranges: e
            .attr("table:scenario-ranges")
            .map(parse_range_list)
            .unwrap_or_default(),
        cells: Vec::new(),
        border_color: e.attr("table:border-color").and_then(parse_color),
        display_border: flag("table:display-border", true),
        copy_back: flag("table:copy-back", true),
        copy_styles: flag("table:copy-styles", false),
        prevent_changes: flag("table:protected", false),
        active: flag("table:is-active", false),
    }
}

// ---------------------------------------------------------------------------
// Outline groups
// ---------------------------------------------------------------------------

fn group_tag(rows: bool) -> &'static str {
    if rows {
        "table:table-row-group"
    } else {
        "table:table-column-group"
    }
}

/// Writes the column or row groups of a sheet in index order: opens
/// the groups starting at an index (outermost first), closes those ending
/// there (innermost first) and reports the next boundary, so repeated
/// runs stop at group starts and ends. Starts and boundaries are sorted
/// once, so the walk is O(indices + groups) (security review 4,
/// HIGH-T5: filtering the whole list per written row was O(rows ×
/// groups)); `last_index` lets the writer extend its range so groups past
/// the data are written and closed (correctness review 4, H1).
pub(crate) struct GroupWalk<'a> {
    /// Groups by start, outermost first.
    order: Vec<&'a OutlineGroupData>,
    /// Next group of `order` to open.
    next: usize,
    /// Ends of the open groups, outermost first.
    open: Vec<u32>,
    /// Starts and `end + 1` of every group, ascending.
    boundaries: Vec<u32>,
    rows: bool,
}

impl<'a> GroupWalk<'a> {
    pub(crate) fn new(groups: &'a [OutlineGroupData], rows: bool) -> Self {
        let limit = if rows { MAX_ROWS } else { MAX_COLS };
        let mut order: Vec<&OutlineGroupData> = groups
            .iter()
            .filter(|g| g.start <= g.end && g.end < limit)
            .collect();
        order.sort_by_key(|g| (g.start, g.level, std::cmp::Reverse(g.end)));
        let mut boundaries: Vec<u32> = order
            .iter()
            .flat_map(|g| [g.start, g.end.saturating_add(1)])
            .collect();
        boundaries.sort_unstable();
        boundaries.dedup();
        Self {
            order,
            next: 0,
            open: Vec::new(),
            boundaries,
            rows,
        }
    }

    /// Opens every group starting at or before `i` that is not open yet.
    pub(crate) fn open(&mut self, s: &mut String, i: u32) {
        while let Some(g) = self.order.get(self.next) {
            if g.start > i {
                break;
            }
            let _ = write!(
                s,
                r#"<{} table:display="{}">"#,
                group_tag(self.rows),
                !g.collapsed
            );
            self.open.push(g.end);
            self.next += 1;
        }
    }

    /// Closes every open group ending at or before `i`, innermost first.
    pub(crate) fn close(&mut self, s: &mut String, i: u32) {
        while self.open.last().is_some_and(|end| *end <= i) {
            self.open.pop();
            let _ = write!(s, "</{}>", group_tag(self.rows));
        }
    }

    /// Closes every group still open.
    pub(crate) fn close_all(&mut self, s: &mut String) {
        while self.open.pop().is_some() {
            let _ = write!(s, "</{}>", group_tag(self.rows));
        }
    }

    /// The first boundary after `i`.
    pub(crate) fn next_boundary(&mut self, i: u32) -> Option<u32> {
        let k = self.boundaries.partition_point(|b| *b <= i);
        self.boundaries.get(k).copied()
    }

    /// The last index any group covers.
    pub(crate) fn last_index(&self) -> Option<u32> {
        self.order.iter().map(|g| g.end).max()
    }
}

/// An open group while reading: axis, first index, element depth, display.
pub(crate) struct OpenGroup {
    pub rows: bool,
    pub start: u32,
    pub depth: u32,
    pub collapsed: bool,
}

/// Records a group that ended (its last index is `end`), nested one level
/// deeper than the groups still open on the same axis.
pub(crate) fn close_group(sheet: &mut SheetData, open: &OpenGroup, end: u32, still_open: usize) {
    if open.start > end {
        return;
    }
    let limit = if open.rows { MAX_ROWS } else { MAX_COLS };
    let g = OutlineGroupData {
        start: open.start,
        end: end.min(limit - 1),
        level: u8::try_from(still_open + 1).unwrap_or(u8::MAX),
        collapsed: open.collapsed,
    };
    // AOO keeps at most SC_OL_MAXDEPTH = 7 levels (`sc/inc/olinetab.hxx:31`;
    // `ScOutlineArray::Insert` refuses deeper groups, `olinetab.cxx:240-243`).
    if g.level > MAX_OUTLINE_DEPTH {
        return;
    }
    let list = if open.rows {
        &mut sheet.row_groups
    } else {
        &mut sheet.col_groups
    };
    // Sorted once at the end of the table ([`sort_groups`]): sorting on
    // every push made reading O(G² log G) (security review 4, HIGH-T5).
    if list.len() < 100_000 {
        list.push(g);
    }
}

/// Deepest outline level read (AOO `SC_OL_MAXDEPTH`).
pub(crate) const MAX_OUTLINE_DEPTH: u8 = 7;

/// Sorts the groups read for a sheet (once, after the sheet is read).
pub(crate) fn sort_groups(sheet: &mut SheetData) {
    sheet.row_groups.sort_by_key(|g| (g.level, g.start, g.end));
    sheet.col_groups.sort_by_key(|g| (g.level, g.start, g.end));
}

// ---------------------------------------------------------------------------
// Database ranges, standard filter, subtotal rules
// ---------------------------------------------------------------------------

fn std_op_name(op: StdFilterOpData) -> &'static str {
    match op {
        StdFilterOpData::Eq => "=",
        StdFilterOpData::Ne => "!=",
        StdFilterOpData::Lt => "<",
        StdFilterOpData::Gt => ">",
        StdFilterOpData::Le => "<=",
        StdFilterOpData::Ge => ">=",
        StdFilterOpData::Largest => "top values",
        StdFilterOpData::Smallest => "bottom values",
        StdFilterOpData::LargestPct => "top percent",
        StdFilterOpData::SmallestPct => "bottom percent",
        StdFilterOpData::Contains => "contains",
        StdFilterOpData::NotContains => "does-not-contain",
        StdFilterOpData::BeginsWith => "begins-with",
        StdFilterOpData::NotBeginsWith => "does-not-begin-with",
        StdFilterOpData::EndsWith => "ends-with",
        StdFilterOpData::NotEndsWith => "does-not-end-with",
    }
}

fn std_op_parse(op: &str) -> Option<StdFilterOpData> {
    Some(match op {
        "=" => StdFilterOpData::Eq,
        "!=" => StdFilterOpData::Ne,
        "<" => StdFilterOpData::Lt,
        ">" => StdFilterOpData::Gt,
        "<=" => StdFilterOpData::Le,
        ">=" => StdFilterOpData::Ge,
        "top values" => StdFilterOpData::Largest,
        "bottom values" => StdFilterOpData::Smallest,
        "top percent" => StdFilterOpData::LargestPct,
        "bottom percent" => StdFilterOpData::SmallestPct,
        "contains" => StdFilterOpData::Contains,
        "does-not-contain" => StdFilterOpData::NotContains,
        "begins-with" | "begins" => StdFilterOpData::BeginsWith,
        "does-not-begin-with" => StdFilterOpData::NotBeginsWith,
        "ends-with" | "ends" => StdFilterOpData::EndsWith,
        "does-not-end-with" => StdFilterOpData::NotEndsWith,
        _ => return None,
    })
}

/// `true` if a `table:filter` element (and its subtree) holds something
/// only the standard/advanced filter can express.
pub(crate) fn is_standard_filter(e: &Elem) -> bool {
    e.attr("table:condition-source-range-address").is_some()
        || e.attr("table:display-duplicates") == Some("false")
        || e.attr("table:target-range-address").is_some()
}

/// Writes a `table:filter` element of a standard/advanced filter.
pub(crate) fn write_standard_filter(s: &mut String, f: &StandardFilterData, data: &WorkbookData) {
    s.push_str("<table:filter");
    if let Some((tab, r)) = &f.criteria {
        if let Some(sh) = data.sheets.get(*tab as usize) {
            attr(
                s,
                "table:condition-source-range-address",
                &range_list(&odf_sheet_name(&sh.name), std::slice::from_ref(r)),
            );
        }
    }
    if let Some((tab, r, c)) = f.options.copy_to {
        if let Some(sh) = data.sheets.get(tab as usize) {
            attr(
                s,
                "table:target-range-address",
                &base_address(&odf_sheet_name(&sh.name), r, c),
            );
        }
    }
    if f.options.no_duplicates {
        s.push_str(r#" table:display-duplicates="false""#);
    }
    s.push('>');
    // AND binds tighter than OR: OR groups of AND runs.
    let mut groups: Vec<Vec<&StdConditionData>> = Vec::new();
    for (i, c) in f.conditions.iter().enumerate() {
        if i == 0 || c.connector == StdConnectorData::Or || groups.is_empty() {
            groups.push(vec![c]);
        } else if let Some(g) = groups.last_mut() {
            g.push(c);
        }
    }
    let write_cond = |s: &mut String, c: &StdConditionData| {
        let field = c.col.saturating_sub(f.range.c0);
        let ty = if c.value.trim().parse::<f64>().is_ok() {
            "number"
        } else {
            "text"
        };
        let _ = write!(s, r#"<table:filter-condition table:field-number="{field}""#);
        attr(s, "table:value", &c.value);
        attr(s, "table:operator", std_op_name(c.op));
        let _ = write!(s, r#" table:data-type="{ty}""#);
        if f.options.case_sensitive {
            s.push_str(r#" table:case-sensitive="true""#);
        }
        s.push_str("/>");
    };
    let write_group = |s: &mut String, g: &[&StdConditionData]| {
        if g.len() == 1 {
            write_cond(s, g[0]);
        } else {
            s.push_str("<table:filter-and>");
            for c in g {
                write_cond(s, c);
            }
            s.push_str("</table:filter-and>");
        }
    };
    if groups.len() == 1 {
        write_group(s, &groups[0]);
    } else if !groups.is_empty() {
        s.push_str("<table:filter-or>");
        for g in &groups {
            write_group(s, g);
        }
        s.push_str("</table:filter-or>");
    }
    s.push_str("</table:filter>");
}

/// Parses a `table:filter` element into a standard/advanced filter
/// (`range` is the database range; `sheet_index` resolves sheet names).
pub(crate) fn parse_standard_filter<R: BufRead>(
    xml: &mut XmlReader<R>,
    e: &Elem,
    range: CellRangeLocal,
    sheet_index: &dyn Fn(&str) -> Option<u32>,
) -> Result<StandardFilterData, OdsError> {
    let mut f = StandardFilterData {
        range,
        conditions: Vec::new(),
        options: StdFilterOptionsData {
            has_header: true,
            ..StdFilterOptionsData::default()
        },
        criteria: None,
    };
    f.options.no_duplicates = e.attr("table:display-duplicates") == Some("false");
    if let Some(a) = e.attr("table:condition-source-range-address") {
        let (sheet, cell) = split_sheet(a);
        let ranges = parse_range_list(cell);
        if let (Some(r), Some(tab)) = (ranges.first(), sheet.and_then(sheet_index)) {
            f.criteria = Some((tab, *r));
        }
    }
    if let Some(a) = e.attr("table:target-range-address") {
        let (sheet, cell) = split_sheet(a);
        if let (Some((r, c)), Some(tab)) = (parse_base(cell), sheet.and_then(sheet_index)) {
            f.options.copy_to = Some((tab, r, c));
        }
    }
    // Conditions: a flat list; `filter-or` children start OR groups,
    // `filter-and` children are AND runs.
    let mut case_sensitive = false;
    let d = xml.depth();
    let mut stack: Vec<bool> = Vec::new(); // true = inside an OR
    let mut first_in_group = true;
    loop {
        match xml.next()? {
            Ev::Start(c) => match c.name.as_str() {
                "table:filter-or" => {
                    stack.push(true);
                    first_in_group = true;
                }
                "table:filter-and" => {
                    stack.push(false);
                    first_in_group = true;
                    if stack.len() == 1 {
                        // A top-level AND run: all conditions AND.
                    }
                }
                "table:filter-condition" => {
                    let field = c
                        .attr("table:field-number")
                        .and_then(|v| v.trim().parse::<u32>().ok())
                        .unwrap_or(0);
                    let op = std_op_parse(c.attr("table:operator").unwrap_or("="))
                        .unwrap_or(StdFilterOpData::Eq);
                    case_sensitive |= c.attr("table:case-sensitive") == Some("true");
                    let in_or = stack.last().copied().unwrap_or(false)
                        || (stack.len() >= 2 && stack[stack.len() - 2] && first_in_group);
                    let connector = if f.conditions.is_empty() {
                        StdConnectorData::And
                    } else if in_or {
                        StdConnectorData::Or
                    } else {
                        StdConnectorData::And
                    };
                    first_in_group = false;
                    if f.conditions.len() < 64 {
                        f.conditions.push(StdConditionData {
                            connector,
                            col: range.c0.saturating_add(field).min(MAX_COLS - 1),
                            op,
                            value: c.attr("table:value").unwrap_or_default().to_string(),
                        });
                    }
                    xml.skip()?;
                }
                _ => xml.skip()?,
            },
            Ev::End if xml.depth() < d => break,
            Ev::End => {
                stack.pop();
                first_in_group = true;
            }
            Ev::Eof => break,
            _ => {}
        }
    }
    f.options.case_sensitive = case_sensitive;
    Ok(f)
}

/// `Sheet.A1:Sheet.B2` → (sheet name, the address text).
fn split_sheet(a: &str) -> (Option<&str>, &str) {
    let first = a.split(':').next().unwrap_or(a);
    match first.rfind('.') {
        Some(i) => {
            let name = first[..i].trim_start_matches('$');
            let name = name.trim_matches('\'');
            (Some(name), a)
        }
        None => (None, a),
    }
}

fn agg_name(f: AggFnData) -> &'static str {
    match f {
        AggFnData::Sum => "sum",
        AggFnData::Count => "count",
        AggFnData::Average => "average",
        AggFnData::Max => "max",
        AggFnData::Min => "min",
        AggFnData::Product => "product",
        AggFnData::CountNums => "countnums",
        AggFnData::StDev => "stdev",
        AggFnData::StDevP => "stdevp",
        AggFnData::Var => "var",
        AggFnData::VarP => "varp",
    }
}

fn agg_parse(s: &str) -> Option<AggFnData> {
    Some(match s {
        "sum" => AggFnData::Sum,
        "count" => AggFnData::Count,
        "average" => AggFnData::Average,
        "max" => AggFnData::Max,
        "min" => AggFnData::Min,
        "product" => AggFnData::Product,
        "countnums" => AggFnData::CountNums,
        "stdev" => AggFnData::StDev,
        "stdevp" => AggFnData::StDevP,
        "var" => AggFnData::Var,
        "varp" => AggFnData::VarP,
        _ => return None,
    })
}

fn write_subtotal_rules(s: &mut String, st: &SubtotalsData) {
    s.push_str("<table:subtotal-rules");
    bool_attr(s, "table:case-sensitive", st.case_sensitive);
    bool_attr(s, "table:page-breaks-on-group-change", st.page_breaks);
    bool_attr(s, "table:bind-styles-to-content", st.include_formats);
    s.push('>');
    if st.presort {
        let _ = write!(
            s,
            r#"<table:sort-groups table:data-type="automatic" table:order="{}"/>"#,
            if st.ascending {
                "ascending"
            } else {
                "descending"
            }
        );
    }
    for g in &st.groups {
        let _ = write!(
            s,
            r#"<table:subtotal-rule table:group-by-field-number="{}">"#,
            g.group_by.saturating_sub(st.range.c0)
        );
        for c in &g.columns {
            let _ = write!(
                s,
                r#"<table:subtotal-field table:field-number="{}" table:function="{}"/>"#,
                c.saturating_sub(st.range.c0),
                agg_name(g.function)
            );
        }
        s.push_str("</table:subtotal-rule>");
    }
    s.push_str("</table:subtotal-rules>");
}

/// Parses `table:subtotal-rules`.
pub(crate) fn parse_subtotal_rules<R: BufRead>(
    xml: &mut XmlReader<R>,
    e: &Elem,
    range: CellRangeLocal,
) -> Result<SubtotalsData, OdsError> {
    let flag = |k: &str| e.attr(k) == Some("true");
    let mut st = SubtotalsData {
        range,
        groups: Vec::new(),
        page_breaks: flag("table:page-breaks-on-group-change"),
        case_sensitive: flag("table:case-sensitive"),
        presort: false,
        ascending: true,
        include_formats: flag("table:bind-styles-to-content"),
    };
    let d = xml.depth();
    loop {
        match xml.next()? {
            Ev::Start(c) => match c.name.as_str() {
                "table:sort-groups" => {
                    st.presort = true;
                    st.ascending = c.attr("table:order") != Some("descending");
                    xml.skip()?;
                }
                "table:subtotal-rule" => {
                    let group_by = c
                        .attr("table:group-by-field-number")
                        .and_then(|v| v.trim().parse::<u32>().ok())
                        .unwrap_or(0);
                    let mut g = SubtotalGroupData {
                        group_by: range.c0.saturating_add(group_by).min(MAX_COLS - 1),
                        columns: Vec::new(),
                        function: AggFnData::Sum,
                    };
                    let dd = xml.depth();
                    loop {
                        match xml.next()? {
                            Ev::Start(f) => {
                                if f.name == "table:subtotal-field" {
                                    if let Some(n) = f
                                        .attr("table:field-number")
                                        .and_then(|v| v.trim().parse::<u32>().ok())
                                    {
                                        if g.columns.len() < 1024 {
                                            g.columns
                                                .push(range.c0.saturating_add(n).min(MAX_COLS - 1));
                                        }
                                    }
                                    if let Some(fun) = f.attr("table:function").and_then(agg_parse)
                                    {
                                        g.function = fun;
                                    }
                                }
                                xml.skip()?;
                            }
                            Ev::End if xml.depth() < dd => break,
                            Ev::Eof => break,
                            _ => {}
                        }
                    }
                    if st.groups.len() < 3 {
                        st.groups.push(g);
                    }
                }
                _ => xml.skip()?,
            },
            Ev::End if xml.depth() < d => break,
            Ev::Eof => break,
            _ => {}
        }
    }
    Ok(st)
}

/// `true` when a sheet has named database ranges, a standard filter or
/// subtotal rules to write.
pub(crate) fn has_named_database_ranges(data: &WorkbookData) -> bool {
    data.sheets.iter().any(|s| {
        !s.database_ranges.is_empty()
            || (s.auto_filter.is_none() && (s.standard_filter.is_some() || s.subtotals.is_some()))
    })
}

/// Writes the named database ranges of every sheet, plus an anonymous
/// range for a sheet-level standard filter or subtotal definition when
/// the sheet has no AutoFilter (which would already own the anonymous
/// range).
pub(crate) fn write_named_database_ranges(s: &mut String, data: &WorkbookData) {
    for (tab, sheet) in data.sheets.iter().enumerate() {
        let name = odf_sheet_name(&sheet.name);
        for d in &sheet.database_ranges {
            write_database_range(
                s,
                &d.name,
                &name,
                d.range,
                d.has_header,
                d.filter_buttons,
                d.filter.as_ref(),
                d.subtotals.as_ref(),
                data,
            );
        }
        if sheet.auto_filter.is_none()
            && (sheet.standard_filter.is_some() || sheet.subtotals.is_some())
        {
            let range = sheet
                .standard_filter
                .as_ref()
                .map(|f| f.range)
                .or_else(|| sheet.subtotals.as_ref().map(|st| st.range))
                .unwrap_or_default();
            let has_header = sheet
                .standard_filter
                .as_ref()
                .is_none_or(|f| f.options.has_header);
            write_database_range(
                s,
                &format!("__Anonymous_Sheet_DB__{tab}"),
                &name,
                range,
                has_header,
                false,
                sheet.standard_filter.as_ref(),
                sheet.subtotals.as_ref(),
                data,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn write_database_range(
    s: &mut String,
    name: &str,
    sheet_name: &str,
    range: CellRangeLocal,
    has_header: bool,
    buttons: bool,
    filter: Option<&StandardFilterData>,
    subtotals: Option<&SubtotalsData>,
    data: &WorkbookData,
) {
    s.push_str("<table:database-range");
    attr(s, "table:name", name);
    attr(
        s,
        "table:target-range-address",
        &range_list(sheet_name, std::slice::from_ref(&range)),
    );
    bool_attr(s, "table:contains-header", has_header);
    if buttons {
        s.push_str(r#" table:display-filter-buttons="true""#);
    }
    s.push('>');
    if let Some(f) = filter {
        write_standard_filter(s, f, data);
    }
    if let Some(st) = subtotals {
        write_subtotal_rules(s, st);
    }
    s.push_str("</table:database-range>");
}

/// A parsed named database range with its filter and subtotal rules.
pub(crate) fn database_range_from(
    name: &str,
    range: CellRangeLocal,
    e: &Elem,
    filter: Option<StandardFilterData>,
    subtotals: Option<SubtotalsData>,
) -> DatabaseRangeData {
    DatabaseRangeData {
        name: name.to_string(),
        range,
        has_header: e.attr("table:contains-header") != Some("false"),
        filter_buttons: e.attr("table:display-filter-buttons") == Some("true"),
        filter,
        subtotals,
    }
}

// ---------------------------------------------------------------------------
// DataPilot
// ---------------------------------------------------------------------------

fn orientation_of(orientation: &str) -> Option<&'static str> {
    Some(match orientation {
        "row" => "row",
        "column" => "column",
        "page" => "page",
        "data" => "data",
        _ => return None,
    })
}

fn sort_mode(sort: &PivotSortData) -> (&'static str, bool, Option<u32>) {
    match sort {
        PivotSortData::None => ("none", true, None),
        PivotSortData::Name { ascending } => ("name", *ascending, None),
        PivotSortData::Data {
            data_field,
            ascending,
        } => ("data", *ascending, Some(*data_field)),
        PivotSortData::Manual { .. } => ("manual", true, None),
    }
}

fn layout_mode(l: PivotLayoutData) -> &'static str {
    match l {
        PivotLayoutData::Tabular => "tabular-layout",
        PivotLayoutData::OutlineTop => "outline-subtotals-top",
        PivotLayoutData::OutlineBottom => "outline-subtotals-bottom",
    }
}

fn caption_of(d: &PivotDataFieldData, names: &dyn Fn(u32) -> String) -> String {
    match &d.caption {
        Some(c) if !c.trim().is_empty() => c.clone(),
        _ => format!(
            "{} - {}",
            match d.function {
                AggFnData::Sum => "Sum",
                AggFnData::Count => "Count",
                AggFnData::Average => "Average",
                AggFnData::Max => "Max",
                AggFnData::Min => "Min",
                AggFnData::Product => "Product",
                AggFnData::CountNums => "Count (Numbers only)",
                AggFnData::StDev => "StDev (Sample)",
                AggFnData::StDevP => "StDevP (Population)",
                AggFnData::Var => "Var (Sample)",
                AggFnData::VarP => "VarP (Population)",
            },
            names(d.source_col)
        ),
    }
}

fn write_axis_field(
    s: &mut String,
    f: &PivotFieldData,
    orientation: &str,
    data: &PivotData,
    names: &dyn Fn(u32) -> String,
    epoch: DateEpoch,
) {
    s.push_str("<table:data-pilot-field");
    attr(s, "table:source-field-name", &names(f.source_col));
    attr(s, "table:orientation", orientation);
    s.push_str(r#" table:used-hierarchy="1""#);
    if let Some(sel) = &f.page_item {
        attr(s, "table:selected-page", sel);
    }
    s.push('>');
    s.push_str("<table:data-pilot-level");
    bool_attr(s, "table:show-empty", f.show_empty);
    s.push('>');
    s.push_str("<table:data-pilot-subtotals>");
    match &f.subtotals {
        PivotSubtotalsData::Auto => {
            s.push_str(r#"<table:data-pilot-subtotal table:function="auto"/>"#)
        }
        PivotSubtotalsData::None => {}
        PivotSubtotalsData::Custom(fns) => {
            for fun in fns {
                let _ = write!(
                    s,
                    r#"<table:data-pilot-subtotal table:function="{}"/>"#,
                    agg_name(*fun)
                );
            }
        }
    }
    s.push_str("</table:data-pilot-subtotals>");
    let manual = match &f.sort {
        PivotSortData::Manual { order } => order.clone(),
        _ => Vec::new(),
    };
    if !f.hidden_items.is_empty() || !manual.is_empty() {
        s.push_str("<table:data-pilot-members>");
        for name in manual
            .iter()
            .chain(f.hidden_items.iter().filter(|h| !manual.contains(h)))
        {
            s.push_str("<table:data-pilot-member");
            attr(s, "table:name", name);
            bool_attr(s, "table:display", !f.hidden_items.contains(name));
            s.push_str(r#" table:show-details="true"/>"#);
        }
        s.push_str("</table:data-pilot-members>");
    }
    if let Some(a) = &f.auto_show {
        let _ = write!(
            s,
            r#"<table:data-pilot-display-info table:enabled="true" table:member-count="{}" table:display-member-mode="{}""#,
            a.count,
            if a.top { "from-top" } else { "from-bottom" }
        );
        if let Some(df) = data.data_fields.get(a.data_field as usize) {
            attr(s, "table:data-field", &caption_of(df, names));
        }
        s.push_str("/>");
    }
    let (mode, asc, df) = sort_mode(&f.sort);
    let _ = write!(
        s,
        r#"<table:data-pilot-sort-info table:sort-mode="{mode}" table:order="{}""#,
        if asc { "ascending" } else { "descending" }
    );
    if let Some(d) = df.and_then(|i| data.data_fields.get(i as usize)) {
        attr(s, "table:data-field", &caption_of(d, names));
    }
    s.push_str("/>");
    let _ = write!(
        s,
        r#"<table:data-pilot-layout-info table:add-empty-lines="{}" table:layout-mode="{}"/>"#,
        f.empty_line_after,
        layout_mode(f.layout)
    );
    s.push_str("</table:data-pilot-level>");
    if let Some(g) = &f.group {
        s.push_str("<table:data-pilot-groups");
        attr(s, "table:source-field-name", &names(f.source_col));
        match g {
            PivotGroupData::Numeric { start, end, step } => {
                attr(s, "table:start", &start.map_or("auto".to_string(), fmt_f64));
                attr(s, "table:end", &end.map_or("auto".to_string(), fmt_f64));
                attr(s, "table:step", &fmt_f64(*step));
            }
            PivotGroupData::Date { start, end, parts } => {
                // AOO writes the limits as ISO date-times
                // (`XMLExportDataPilot.cxx:560-581`).
                let iso =
                    |v: &Option<f64>| v.map_or("auto".to_string(), |v| serial_to_iso(v, epoch));
                attr(s, "table:date-start", &iso(start));
                attr(s, "table:date-end", &iso(end));
                match parts.as_slice() {
                    // Days alone with a step: numeric grouping of dates
                    // (no `grouped-by`, AOO `xmldpimp.cxx:1033-1040`).
                    [DatePartData::Days(n)] if *n >= 2 => {
                        let _ = write!(s, r#" table:step="{n}""#);
                    }
                    _ => {
                        let by = parts
                            .first()
                            .map(|p| match p {
                                DatePartData::Seconds => "seconds",
                                DatePartData::Minutes => "minutes",
                                DatePartData::Hours => "hours",
                                DatePartData::Days(_) => "days",
                                DatePartData::Months => "months",
                                DatePartData::Quarters => "quarters",
                                DatePartData::Years => "years",
                            })
                            .unwrap_or("days");
                        attr(s, "table:grouped-by", by);
                    }
                }
                // Extra parts are kept in application data.
                if parts.len() > 1 {
                    let extra: Vec<String> = parts
                        .iter()
                        .skip(1)
                        .map(|p| match p {
                            DatePartData::Seconds => "seconds".into(),
                            DatePartData::Minutes => "minutes".into(),
                            DatePartData::Hours => "hours".into(),
                            DatePartData::Days(n) => format!("days:{n}"),
                            DatePartData::Months => "months".into(),
                            DatePartData::Quarters => "quarters".into(),
                            DatePartData::Years => "years".into(),
                        })
                        .collect();
                    attr(s, "table:application-data", &extra.join(","));
                }
            }
        }
        s.push_str("/>");
    }
    s.push_str("</table:data-pilot-field>");
}

/// Serial date → ISO date-time (`2024-01-01T00:00:00`), as AOO writes
/// pivot date limits.
fn serial_to_iso(v: f64, epoch: DateEpoch) -> String {
    let days = v.floor();
    let secs = ((v - days) * 86_400.0).round() as i64;
    // Howard Hinnant's civil_from_days.
    let z = days as i64 + crate::units::epoch_base(epoch) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}",
        secs / 3600,
        (secs / 60) % 60,
        secs % 60
    )
}

/// Writes `table:data-pilot-tables` for every sheet's pivots.
pub(crate) fn write_pivots(s: &mut String, data: &WorkbookData) {
    let any = data.sheets.iter().any(|sh| !sh.pivots.is_empty());
    if !any {
        return;
    }
    s.push_str("<table:data-pilot-tables>");
    for (tab, sheet) in data.sheets.iter().enumerate() {
        for p in &sheet.pivots {
            write_pivot(s, p, tab, sheet, data);
        }
    }
    s.push_str("</table:data-pilot-tables>");
}

fn write_pivot(s: &mut String, p: &PivotData, tab: usize, sheet: &SheetData, data: &WorkbookData) {
    let (src_sheet, src_range): (Option<&SheetData>, Option<CellRangeLocal>) = match &p.source {
        PivotSourceData::Range { sheet, range } => (data.sheets.get(*sheet as usize), Some(*range)),
        PivotSourceData::DatabaseRange { name } => {
            let found = data.sheets.iter().find_map(|sh| {
                sh.database_ranges
                    .iter()
                    .find(|d| d.name.eq_ignore_ascii_case(name))
                    .map(|d| (sh, d.range))
            });
            match found {
                Some((sh, r)) => (Some(sh), Some(r)),
                None => (None, None),
            }
        }
    };
    // Field names: the header row of the source, else "ColumnN".
    let header: Vec<String> = match (src_sheet, src_range) {
        (Some(sh), Some(r)) => (r.c0..=r.c1)
            .map(|c| {
                sh.cells
                    .iter()
                    .find(|cell| cell.row == r.r0 && cell.col == c)
                    .and_then(|cell| match &cell.content {
                        tally_model::CellContentData::Value(tally_types::Value::Text(t)) => {
                            Some(t.to_string())
                        }
                        tally_model::CellContentData::Value(tally_types::Value::Number(n)) => {
                            Some(fmt_f64(*n))
                        }
                        tally_model::CellContentData::Formula {
                            cached: Some(v), ..
                        }
                        | tally_model::CellContentData::SharedFormula {
                            cached: Some(v), ..
                        } => match v {
                            tally_types::Value::Text(t) => Some(t.to_string()),
                            tally_types::Value::Number(n) => Some(fmt_f64(*n)),
                            _ => None,
                        },
                        _ => None,
                    })
                    .filter(|t| !t.is_empty())
                    .unwrap_or_else(|| format!("Column{}", c - r.c0 + 1))
            })
            .collect(),
        _ => Vec::new(),
    };
    let names = |col: u32| -> String {
        header
            .get(col as usize)
            .cloned()
            .unwrap_or_else(|| format!("Column{}", col + 1))
    };
    let sheet_name = odf_sheet_name(&sheet.name);
    s.push_str("<table:data-pilot-table");
    attr(
        s,
        "table:name",
        if p.name.is_empty() {
            "DataPilot"
        } else {
            &p.name
        },
    );
    let _ = tab;
    let output = p.output.unwrap_or(CellRangeLocal {
        r0: p.target.0,
        c0: p.target.1,
        r1: p.target.0,
        c1: p.target.1,
    });
    attr(
        s,
        "table:target-range-address",
        &range_list(&sheet_name, std::slice::from_ref(&output)),
    );
    attr(
        s,
        "table:grand-total",
        match (p.row_totals, p.col_totals) {
            (true, true) => "both",
            (true, false) => "row",
            (false, true) => "column",
            (false, false) => "none",
        },
    );
    bool_attr(s, "table:ignore-empty-rows", p.ignore_empty_rows);
    bool_attr(s, "table:identify-categories", p.identify_categories);
    bool_attr(s, "table:show-filter-button", p.show_filter_button);
    bool_attr(s, "table:drill-down-on-double-click", p.drill_down);
    s.push('>');
    match (&p.source, src_sheet, src_range) {
        (PivotSourceData::DatabaseRange { name }, _, _) => {
            s.push_str("<table:database-source-table");
            attr(s, "table:database-name", "");
            attr(s, "table:table-name", name);
            s.push_str("/>");
        }
        (PivotSourceData::Range { .. }, Some(sh), Some(r)) => {
            s.push_str("<table:source-cell-range");
            attr(
                s,
                "table:cell-range-address",
                &range_list(&odf_sheet_name(&sh.name), std::slice::from_ref(&r)),
            );
            s.push_str("/>");
        }
        _ => {}
    }
    for f in &p.row_fields {
        write_axis_field(
            s,
            f,
            "row",
            p,
            &names,
            data.settings.calc_settings.date_epoch,
        );
    }
    for f in &p.col_fields {
        write_axis_field(
            s,
            f,
            "column",
            p,
            &names,
            data.settings.calc_settings.date_epoch,
        );
    }
    for f in &p.page_fields {
        write_axis_field(
            s,
            f,
            "page",
            p,
            &names,
            data.settings.calc_settings.date_epoch,
        );
    }
    for d in &p.data_fields {
        s.push_str("<table:data-pilot-field");
        attr(s, "table:source-field-name", &names(d.source_col));
        s.push_str(r#" table:orientation="data" table:used-hierarchy="1""#);
        attr(s, "table:function", agg_name(d.function));
        // The caption is the layout name (`XMLExportDataPilot.cxx:690-696`).
        if let Some(c) = &d.caption {
            attr(s, "tableooo:display-name", c);
        }
        s.push('>');
        if let Some(sa) = &d.show_as {
            s.push_str("<table:data-pilot-field-reference");
            if let Some(bf) = sa.base_field {
                attr(s, "table:field-name", &names(bf));
            }
            attr(
                s,
                "table:type",
                match sa.kind {
                    ShowAsKindData::Difference => "member-difference",
                    ShowAsKindData::PercentOf => "member-percentage",
                    ShowAsKindData::PercentDifference => "member-percentage-difference",
                    ShowAsKindData::RunningTotal => "running-total",
                    ShowAsKindData::PercentOfRow => "row-percentage",
                    ShowAsKindData::PercentOfColumn => "column-percentage",
                    ShowAsKindData::PercentOfTotal => "total-percentage",
                    ShowAsKindData::Index => "index",
                },
            );
            match &sa.base_item {
                Some(BaseItemData::Named(n)) => {
                    attr(s, "table:member-type", "named");
                    attr(s, "table:member-name", n);
                }
                Some(BaseItemData::Previous) => attr(s, "table:member-type", "previous"),
                Some(BaseItemData::Next) => attr(s, "table:member-type", "next"),
                None => {}
            }
            s.push_str("/>");
        }
        s.push_str("</table:data-pilot-field>");
    }
    if p.data_fields.len() > 1 {
        s.push_str(r#"<table:data-pilot-field table:source-field-name="" table:is-data-layout-field="true""#);
        attr(
            s,
            "table:orientation",
            match p.data_layout {
                DataLayoutData::Row => "row",
                DataLayoutData::Column => "column",
            },
        );
        s.push_str(r#" table:used-hierarchy="1"/>"#);
    }
    s.push_str("</table:data-pilot-table>");
}

/// Parses `table:data-pilot-tables` into the target sheets' `pivots`.
pub(crate) fn parse_pivot_tables<R: BufRead>(
    xml: &mut XmlReader<R>,
    sheets: &mut [SheetData],
    epoch: DateEpoch,
) -> Result<(), OdsError> {
    let d = xml.depth();
    let mut count = 0usize;
    loop {
        match xml.next()? {
            Ev::Start(e) if e.name == "table:data-pilot-table" => {
                let parsed = parse_pivot(xml, &e, sheets, epoch)?;
                if let Some((tab, p)) = parsed {
                    if count < 10_000 {
                        if let Some(sh) = sheets.get_mut(tab) {
                            sh.pivots.push(p);
                            count += 1;
                        }
                    }
                }
            }
            Ev::Start(_) => xml.skip()?,
            Ev::End if xml.depth() < d => return Ok(()),
            Ev::Eof => return Ok(()),
            _ => {}
        }
    }
}

fn sheet_tab(sheets: &[SheetData], name: &str) -> Option<usize> {
    let key = name.trim_matches('\'').replace("''", "'");
    sheets.iter().position(|s| s.name == key)
}

fn parse_pivot<R: BufRead>(
    xml: &mut XmlReader<R>,
    e: &Elem,
    sheets: &[SheetData],
    epoch: DateEpoch,
) -> Result<Option<(usize, PivotData)>, OdsError> {
    let mut p = PivotData {
        name: e.attr("table:name").unwrap_or("DataPilot").to_string(),
        ..PivotData::default()
    };
    let (totals_r, totals_c) = match e.attr("table:grand-total").unwrap_or("both") {
        "row" => (true, false),
        "column" => (false, true),
        "none" => (false, false),
        _ => (true, true),
    };
    p.row_totals = totals_r;
    p.col_totals = totals_c;
    p.ignore_empty_rows = e.attr("table:ignore-empty-rows") == Some("true");
    p.identify_categories = e.attr("table:identify-categories") == Some("true");
    p.show_filter_button = e.attr("table:show-filter-button") != Some("false");
    p.drill_down = e.attr("table:drill-down-on-double-click") != Some("false");
    let target = e.attr("table:target-range-address").unwrap_or_default();
    let (tname, _) = split_sheet(target);
    let tab = tname.and_then(|n| sheet_tab(sheets, n));
    let ranges = parse_range_list(target);
    let Some(tab) = tab else {
        xml.skip()?;
        return Ok(None);
    };
    if let Some(r) = ranges.first() {
        p.target = (r.r0, r.c0);
        p.output = Some(*r);
    }
    // Field names come from the source header row; resolved after the
    // source is known.
    let mut header: Vec<String> = Vec::new();
    let mut pending: Vec<PendingField> = Vec::new();
    let d = xml.depth();
    loop {
        match xml.next()? {
            Ev::Start(c) => match c.name.as_str() {
                "table:source-cell-range" => {
                    let a = c.attr("table:cell-range-address").unwrap_or_default();
                    let (sname, _) = split_sheet(a);
                    if let (Some(stab), Some(r)) = (
                        sname.and_then(|n| sheet_tab(sheets, n)),
                        parse_range_list(a).first().copied(),
                    ) {
                        header = header_of(&sheets[stab], r);
                        p.source = PivotSourceData::Range {
                            sheet: stab as u32,
                            range: r,
                        };
                    }
                    xml.skip()?;
                }
                "table:database-source-table" => {
                    let name = c.attr("table:table-name").unwrap_or_default().to_string();
                    let found = sheets.iter().find_map(|sh| {
                        sh.database_ranges
                            .iter()
                            .find(|dr| dr.name.eq_ignore_ascii_case(&name))
                            .map(|dr| (sh, dr.range))
                    });
                    if let Some((sh, r)) = found {
                        header = header_of(sh, r);
                    }
                    p.source = PivotSourceData::DatabaseRange { name };
                    xml.skip()?;
                }
                "table:data-pilot-field" => {
                    // Bounded: past the engine's field cap the pivot is
                    // refused on load anyway (security review 4, H7).
                    if pending.len() >= 1024 {
                        xml.skip()?;
                        continue;
                    }
                    let orientation = c.attr("table:orientation").unwrap_or("row").to_string();
                    let source = c
                        .attr("table:source-field-name")
                        .unwrap_or_default()
                        .to_string();
                    let is_layout = c.attr("table:is-data-layout-field") == Some("true");
                    if is_layout {
                        p.data_layout = if orientation == "column" {
                            DataLayoutData::Column
                        } else {
                            DataLayoutData::Row
                        };
                        xml.skip()?;
                        continue;
                    }
                    if orientation_of(&orientation).is_none() {
                        xml.skip()?;
                        continue;
                    }
                    if orientation == "data" {
                        let (show_as, base_field) = parse_data_field_body(xml)?;
                        let df = PivotDataFieldData {
                            source_col: 0,
                            function: c
                                .attr("table:function")
                                .and_then(agg_parse)
                                .unwrap_or(AggFnData::Sum),
                            // The layout name (`xmldpimp.cxx:1637-1657`);
                            // older Tally files used application data.
                            caption: c
                                .attr("tableooo:display-name")
                                .or_else(|| c.attr("table:application-data"))
                                .filter(|t| !t.is_empty())
                                .map(str::to_string),
                            show_as,
                        };
                        pending.push(PendingField {
                            source,
                            orientation,
                            axis: None,
                            data: Some(df),
                            sort_df: None,
                            auto_df: None,
                            base_field,
                            group_source: None,
                        });
                    } else {
                        let mut f = PivotFieldData {
                            page_item: c.attr("table:selected-page").map(str::to_string),
                            ..PivotFieldData::default()
                        };
                        let (sort_df, auto_df, group_source) =
                            parse_axis_field_body(xml, &mut f, epoch)?;
                        pending.push(PendingField {
                            source,
                            orientation,
                            axis: Some(f),
                            data: None,
                            sort_df,
                            auto_df,
                            base_field: None,
                            group_source,
                        });
                    }
                }
                _ => xml.skip()?,
            },
            Ev::End if xml.depth() < d => break,
            Ev::Eof => break,
            _ => {}
        }
    }
    // Field names resolve against the source header; a group dimension
    // (`Date2`) through its `data-pilot-groups/@source-field-name`;
    // anything else is dropped (AOO `xmldpimp.cxx:1630-1700`).
    let col_of =
        |name: &str| -> Option<u32> { header.iter().position(|h| h == name).map(|i| i as u32) };
    // Data fields first: sort/auto-show refer to them by caption.
    let mut data_fields: Vec<PivotDataFieldData> = Vec::new();
    for pf in &pending {
        if let Some(df) = &pf.data {
            let Some(col) = col_of(&pf.source) else {
                continue;
            };
            let mut df = df.clone();
            df.source_col = col;
            if let (Some(sa), Some(bf)) = (df.show_as.as_mut(), &pf.base_field) {
                // An unknown base field stays unset (an error value, as AOO).
                sa.base_field = col_of(bf);
            }
            data_fields.push(df);
        }
    }
    let captions: Vec<String> = data_fields
        .iter()
        .map(|d| caption_of(d, &|c| header.get(c as usize).cloned().unwrap_or_default()))
        .collect();
    let resolve = |name: &str| -> u32 {
        captions
            .iter()
            .position(|c| c == name)
            .map_or(0, |i| i as u32)
    };
    for pf in pending {
        let Some(mut f) = pf.axis else { continue };
        let col = col_of(&pf.source).or_else(|| pf.group_source.as_deref().and_then(col_of));
        let Some(col) = col else { continue };
        f.source_col = col;
        f.name = pf.source.clone();
        if let (PivotSortData::Data { data_field, .. }, Some(n)) = (&mut f.sort, &pf.sort_df) {
            *data_field = resolve(n);
        }
        if let (Some(a), Some(n)) = (f.auto_show.as_mut(), &pf.auto_df) {
            a.data_field = resolve(n);
        }
        match pf.orientation.as_str() {
            "row" => p.row_fields.push(f),
            "column" => p.col_fields.push(f),
            _ => p.page_fields.push(f),
        }
    }
    p.data_fields = data_fields;
    if p.data_fields.is_empty() {
        return Ok(None);
    }
    Ok(Some((tab, p)))
}

/// A field read before the source header is known.
struct PendingField {
    source: String,
    orientation: String,
    axis: Option<PivotFieldData>,
    data: Option<PivotDataFieldData>,
    sort_df: Option<String>,
    auto_df: Option<String>,
    base_field: Option<String>,
    /// `data-pilot-groups/@source-field-name` of a group dimension.
    group_source: Option<String>,
}

/// The header texts of a source range.
fn header_of(sheet: &SheetData, r: CellRangeLocal) -> Vec<String> {
    (r.c0..=r.c1)
        .map(|c| {
            sheet
                .cells
                .iter()
                .find(|cell| cell.row == r.r0 && cell.col == c)
                .and_then(|cell| match &cell.content {
                    tally_model::CellContentData::Value(tally_types::Value::Text(t)) => {
                        Some(t.to_string())
                    }
                    tally_model::CellContentData::Value(tally_types::Value::Number(n)) => {
                        Some(fmt_f64(*n))
                    }
                    tally_model::CellContentData::Formula {
                        cached: Some(tally_types::Value::Text(t)),
                        ..
                    } => Some(t.to_string()),
                    _ => None,
                })
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| format!("Column{}", c - r.c0 + 1))
        })
        .collect()
}

fn parse_data_field_body<R: BufRead>(
    xml: &mut XmlReader<R>,
) -> Result<(Option<ShowAsData>, Option<String>), OdsError> {
    let d = xml.depth();
    let mut out = None;
    let mut base: Option<String> = None;
    loop {
        match xml.next()? {
            Ev::Start(c) => {
                if c.name == "table:data-pilot-field-reference" {
                    let kind = match c.attr("table:type").unwrap_or("none") {
                        "member-difference" => Some(ShowAsKindData::Difference),
                        "member-percentage" => Some(ShowAsKindData::PercentOf),
                        "member-percentage-difference" => Some(ShowAsKindData::PercentDifference),
                        "running-total" => Some(ShowAsKindData::RunningTotal),
                        "row-percentage" => Some(ShowAsKindData::PercentOfRow),
                        "column-percentage" => Some(ShowAsKindData::PercentOfColumn),
                        "total-percentage" => Some(ShowAsKindData::PercentOfTotal),
                        "index" => Some(ShowAsKindData::Index),
                        _ => None,
                    };
                    if let Some(kind) = kind {
                        let base_item = match c.attr("table:member-type") {
                            Some("named") => c
                                .attr("table:member-name")
                                .map(|n| BaseItemData::Named(n.to_string())),
                            Some("previous") => Some(BaseItemData::Previous),
                            Some("next") => Some(BaseItemData::Next),
                            _ => None,
                        };
                        base = c.attr("table:field-name").map(str::to_string);
                        out = Some(ShowAsData {
                            kind,
                            base_field: None,
                            base_item,
                        });
                    }
                }
                xml.skip()?;
            }
            Ev::End if xml.depth() < d => break,
            Ev::Eof => break,
            _ => {}
        }
    }
    Ok((out, base))
}

/// The sort and auto-show data field names and the group source of an
/// axis field.
type AxisRefs = (Option<String>, Option<String>, Option<String>);

fn parse_axis_field_body<R: BufRead>(
    xml: &mut XmlReader<R>,
    f: &mut PivotFieldData,
    epoch: DateEpoch,
) -> Result<AxisRefs, OdsError> {
    let d = xml.depth();
    let mut manual: Vec<String> = Vec::new();
    let mut sort_is_manual = false;
    let mut sort_df: Option<String> = None;
    let mut auto_df: Option<String> = None;
    let mut group_source: Option<String> = None;
    loop {
        match xml.next()? {
            Ev::Start(c) => match c.name.as_str() {
                "table:data-pilot-level" => {
                    f.show_empty = c.attr("table:show-empty") == Some("true");
                }
                "table:data-pilot-subtotals" => {
                    let mut fns: Vec<AggFnData> = Vec::new();
                    let mut auto = false;
                    let dd = xml.depth();
                    loop {
                        match xml.next()? {
                            Ev::Start(x) => {
                                if x.name == "table:data-pilot-subtotal" {
                                    match x.attr("table:function") {
                                        Some("auto") => auto = true,
                                        Some(other) => {
                                            if let Some(fun) = agg_parse(other) {
                                                fns.push(fun);
                                            }
                                        }
                                        None => {}
                                    }
                                }
                                xml.skip()?;
                            }
                            Ev::End if xml.depth() < dd => break,
                            Ev::Eof => break,
                            _ => {}
                        }
                    }
                    f.subtotals = if auto {
                        PivotSubtotalsData::Auto
                    } else if fns.is_empty() {
                        PivotSubtotalsData::None
                    } else {
                        PivotSubtotalsData::Custom(fns)
                    };
                }
                "table:data-pilot-members" => {
                    let dd = xml.depth();
                    loop {
                        match xml.next()? {
                            Ev::Start(x) => {
                                if x.name == "table:data-pilot-member" {
                                    let name = x.attr("table:name").unwrap_or_default().to_string();
                                    if x.attr("table:display") == Some("false")
                                        && f.hidden_items.len() < 10_000
                                    {
                                        f.hidden_items.push(name.clone());
                                    }
                                    if manual.len() < 10_000 {
                                        manual.push(name);
                                    }
                                }
                                xml.skip()?;
                            }
                            Ev::End if xml.depth() < dd => break,
                            Ev::Eof => break,
                            _ => {}
                        }
                    }
                }
                "table:data-pilot-display-info" => {
                    if c.attr("table:enabled") == Some("true") {
                        f.auto_show = Some(AutoShowData {
                            top: c.attr("table:display-member-mode") != Some("from-bottom"),
                            count: c
                                .attr("table:member-count")
                                .and_then(|v| v.trim().parse().ok())
                                .unwrap_or(1),
                            data_field: 0,
                        });
                        auto_df = c.attr("table:data-field").map(str::to_string);
                    }
                    xml.skip()?;
                }
                "table:data-pilot-sort-info" => {
                    let asc = c.attr("table:order") != Some("descending");
                    f.sort = match c.attr("table:sort-mode").unwrap_or("none") {
                        "name" => PivotSortData::Name { ascending: asc },
                        "data" => {
                            sort_df = c.attr("table:data-field").map(str::to_string);
                            PivotSortData::Data {
                                data_field: 0,
                                ascending: asc,
                            }
                        }
                        "manual" => {
                            sort_is_manual = true;
                            PivotSortData::None
                        }
                        _ => PivotSortData::None,
                    };
                    xml.skip()?;
                }
                "table:data-pilot-layout-info" => {
                    f.empty_line_after = c.attr("table:add-empty-lines") == Some("true");
                    f.layout = match c.attr("table:layout-mode").unwrap_or("tabular-layout") {
                        "outline-subtotals-top" => PivotLayoutData::OutlineTop,
                        "outline-subtotals-bottom" => PivotLayoutData::OutlineBottom,
                        _ => PivotLayoutData::Tabular,
                    };
                    xml.skip()?;
                }
                "table:data-pilot-groups" => {
                    group_source = c
                        .attr("table:source-field-name")
                        .filter(|n| !n.is_empty())
                        .map(str::to_string);
                    let num = |k: &str| c.attr(k).and_then(|v| v.trim().parse::<f64>().ok());
                    // Date limits are ISO date-times (`2024-01-01T00:00:00`);
                    // older Tally files wrote serials.
                    let date = |k: &str| {
                        c.attr(k).and_then(|v| {
                            v.trim()
                                .parse::<f64>()
                                .ok()
                                .or_else(|| crate::units::date_value_to_serial(v, epoch))
                        })
                    };
                    let step_attr = c
                        .attr("table:step")
                        .and_then(|v| v.trim().parse::<f64>().ok());
                    let dated =
                        c.attr("table:date-start").is_some() || c.attr("table:date-end").is_some();
                    f.group = Some(if let Some(by) = c.attr("table:grouped-by") {
                        let step = step_attr
                            .filter(|v| v.is_finite() && *v >= 1.0 && *v <= f64::from(u32::MAX))
                            .map_or(1, |v| v as u32);
                        let part = |name: &str, n: u32| match name {
                            "seconds" => Some(DatePartData::Seconds),
                            "minutes" => Some(DatePartData::Minutes),
                            "hours" => Some(DatePartData::Hours),
                            "days" => Some(DatePartData::Days(n)),
                            "months" => Some(DatePartData::Months),
                            "quarters" => Some(DatePartData::Quarters),
                            "years" => Some(DatePartData::Years),
                            _ => None,
                        };
                        let mut parts: Vec<DatePartData> = part(by, step).into_iter().collect();
                        if let Some(extra) = c.attr("table:application-data") {
                            for x in extra.split(',').take(8) {
                                let (n, st) = match x.split_once(':') {
                                    Some((a, b)) => (a, b.parse::<u32>().unwrap_or(1)),
                                    None => (x, 1),
                                };
                                if let Some(p) = part(n, st) {
                                    parts.push(p);
                                }
                            }
                        }
                        PivotGroupData::Date {
                            start: date("table:date-start"),
                            end: date("table:date-end"),
                            parts,
                        }
                    } else if dated {
                        // Days with a step: numeric grouping of dates.
                        let n = step_attr
                            .filter(|v| v.is_finite() && *v >= 1.0 && *v <= f64::from(u32::MAX))
                            .map_or(1, |v| v as u32);
                        PivotGroupData::Date {
                            start: date("table:date-start"),
                            end: date("table:date-end"),
                            parts: vec![DatePartData::Days(n.max(2))],
                        }
                    } else {
                        PivotGroupData::Numeric {
                            start: num("table:start"),
                            end: num("table:end"),
                            step: num("table:step").unwrap_or(1.0),
                        }
                    });
                    xml.skip()?;
                }
                _ => xml.skip()?,
            },
            Ev::End if xml.depth() < d => break,
            Ev::Eof => break,
            _ => {}
        }
    }
    if sort_is_manual && !manual.is_empty() {
        f.sort = PivotSortData::Manual { order: manual };
    }
    Ok((sort_df, auto_df, group_source))
}

// ---------------------------------------------------------------------------
// Named cell styles and view settings
// ---------------------------------------------------------------------------

/// An ODF style name for a display name: characters outside
/// `[A-Za-z0-9_-]` are hex-escaped as `_XX_`, as LibreOffice does.
pub(crate) fn odf_style_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            out.push(c);
        } else {
            let _ = write!(out, "_{:x}_", u32::from(c));
        }
    }
    out
}

/// The display name of an ODF style name (`Heading_20_1` → `Heading 1`).
pub(crate) fn display_style_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut rest = name;
    while let Some(i) = rest.find('_') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        match after.find('_') {
            Some(j) if j > 0 && j <= 6 && after[..j].chars().all(|c| c.is_ascii_hexdigit()) => {
                if let Some(c) = u32::from_str_radix(&after[..j], 16)
                    .ok()
                    .and_then(char::from_u32)
                {
                    out.push(c);
                    rest = &after[j + 1..];
                    continue;
                }
                out.push('_');
                rest = after;
            }
            _ => {
                out.push('_');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Writes the named cell styles into `office:styles` (after `Default`).
pub(crate) fn write_named_styles(
    s: &mut String,
    data: &WorkbookData,
    cell_xml: &dyn Fn(&mut String, &str, &CellStyleData),
) {
    for st in &data.cell_styles {
        if st.name == "Default" {
            continue;
        }
        let name = odf_style_name(&st.name);
        cell_xml(s, &name, st);
    }
}

/// Writes the per-sheet view options into the settings table entry.
pub(crate) fn write_view_settings(s: &mut String, sheet: &SheetData) {
    let Some(v) = &sheet.view_options else {
        return;
    };
    let item = |s: &mut String, name: &str, b: bool| {
        let _ = write!(
            s,
            r#"<config:config-item config:name="{name}" config:type="boolean">{b}</config:config-item>"#
        );
    };
    item(s, "ShowFormulas", v.show_formulas);
    item(s, "ShowZeroValues", v.show_zero_values);
    item(s, "ShowGrid", v.show_gridlines);
    item(s, "HasColumnRowHeaders", v.show_headers);
    item(s, "IsValueHighlightingEnabled", v.value_highlighting);
}

/// A Window ▸ Split position in 1/100 mm as AOO's `settings.xml` pixels
/// (96 px per inch at 100 %).
pub(crate) fn split_hmm_to_px(hmm: u32) -> u32 {
    ((f64::from(hmm) * 96.0 / 2540.0).round() as u32).min(1_000_000)
}

/// A `settings.xml` split position in pixels as 1/100 mm.
pub(crate) fn split_px_to_hmm(px: u32) -> u32 {
    (f64::from(px.min(1_000_000)) * 2540.0 / 96.0).round() as u32
}

/// A view-settings config item of a sheet (`settings.xml`).
pub(crate) fn apply_view_item(sheet: &mut SheetData, name: &str, value: &str) {
    let b = value.trim() == "true";
    let v = sheet
        .view_options
        .get_or_insert_with(tally_model::ViewOptionsData::default);
    match name {
        "ShowFormulas" => v.show_formulas = b,
        "ShowZeroValues" => v.show_zero_values = b,
        "ShowGrid" => v.show_gridlines = b,
        "HasColumnRowHeaders" => v.show_headers = b,
        "IsValueHighlightingEnabled" => v.value_highlighting = b,
        _ => {}
    }
    if *v == tally_model::ViewOptionsData::default() {
        sheet.view_options = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pivot_sheets() -> Vec<SheetData> {
        let cell = |row, col, t: &str| tally_model::CellData {
            row,
            col,
            content: tally_model::CellContentData::Value(tally_types::Value::text(t)),
            style: None,
        };
        vec![
            SheetData {
                name: "Data".into(),
                cells: vec![cell(0, 0, "Date"), cell(0, 1, "Amount")],
                ..SheetData::default()
            },
            SheetData {
                name: "Pivot".into(),
                ..SheetData::default()
            },
        ]
    }

    fn parse_pivots_xml(xml: &str, sheets: &mut [SheetData]) {
        let mut r = XmlReader::new(
            xml.as_bytes(),
            crate::xml::XmlLimits {
                max_depth: 64,
                max_attrs: 64,
            },
        );
        assert!(matches!(r.next(), Ok(Ev::Start(_))));
        parse_pivot_tables(&mut r, sheets, DateEpoch::D1899).unwrap();
    }

    /// Correctness review 4, H7 and M14: a LibreOffice-shaped table with a
    /// group dimension ("Date2" grouping "Date" by years), an unknown
    /// column field, an ISO date limit and a display name.
    #[test]
    fn pivot_group_dimensions_unknown_fields_and_display_names() {
        let xml = r#"<table:data-pilot-tables><table:data-pilot-table table:name="DataPilot1" table:target-range-address="Pivot.A1:Pivot.C9"><table:source-cell-range table:cell-range-address="Data.A1:Data.B5"/><table:data-pilot-field table:source-field-name="Date2" table:orientation="row"><table:data-pilot-groups table:source-field-name="Date" table:date-start="auto" table:date-end="auto" table:grouped-by="years"/></table:data-pilot-field><table:data-pilot-field table:source-field-name="Date" table:orientation="row"><table:data-pilot-groups table:source-field-name="Date" table:date-start="2024-01-01T00:00:00" table:date-end="auto" table:grouped-by="months"/></table:data-pilot-field><table:data-pilot-field table:source-field-name="Amount" tableooo:display-name="My total" table:orientation="data" table:function="sum"/><table:data-pilot-field table:source-field-name="Nonexistent" table:orientation="column"/><table:data-pilot-field table:source-field-name="Gone" table:orientation="data" table:function="sum"/></table:data-pilot-table></table:data-pilot-tables>"#;
        let mut sheets = pivot_sheets();
        parse_pivots_xml(xml, &mut sheets);
        let p = &sheets[1].pivots[0];
        let rows: Vec<(u32, &str)> = p
            .row_fields
            .iter()
            .map(|f| (f.source_col, f.name.as_str()))
            .collect();
        assert_eq!(rows, [(0, "Date2"), (0, "Date")]);
        assert!(
            p.col_fields.is_empty(),
            "the unknown field is dropped, not column 0"
        );
        assert_eq!(p.data_fields.len(), 1);
        assert_eq!(p.data_fields[0].source_col, 1);
        assert_eq!(p.data_fields[0].caption.as_deref(), Some("My total"));
        // 2024-01-01 is serial 45292.
        assert_eq!(
            p.row_fields[1].group,
            Some(PivotGroupData::Date {
                start: Some(45292.0),
                end: None,
                parts: vec![DatePartData::Months],
            })
        );
    }

    /// M14: date limits are written as ISO date-times, numeric day groups
    /// without `grouped-by`, captions as `tableooo:display-name`; all read
    /// back.
    #[test]
    fn pivot_dates_and_captions_round_trip() {
        let mut sheets = pivot_sheets();
        sheets[1].pivots.push(PivotData {
            name: "P".into(),
            source: PivotSourceData::Range {
                sheet: 0,
                range: CellRangeLocal {
                    r0: 0,
                    c0: 0,
                    r1: 4,
                    c1: 1,
                },
            },
            row_fields: vec![
                PivotFieldData {
                    source_col: 0,
                    group: Some(PivotGroupData::Date {
                        start: Some(45292.0),
                        end: Some(45400.5),
                        parts: vec![DatePartData::Months, DatePartData::Years],
                    }),
                    ..PivotFieldData::default()
                },
                PivotFieldData {
                    source_col: 0,
                    group: Some(PivotGroupData::Date {
                        start: None,
                        end: None,
                        parts: vec![DatePartData::Days(7)],
                    }),
                    ..PivotFieldData::default()
                },
            ],
            data_fields: vec![PivotDataFieldData {
                source_col: 1,
                function: AggFnData::Sum,
                caption: Some("Rev total".into()),
                show_as: None,
            }],
            ..PivotData::default()
        });
        let data = WorkbookData {
            sheets: sheets.clone(),
            ..WorkbookData::default()
        };
        let mut out = String::new();
        write_pivots(&mut out, &data);
        assert!(
            out.contains(r#"table:date-start="2024-01-01T00:00:00""#),
            "{out}"
        );
        assert!(
            out.contains(r#"table:date-end="2024-04-18T12:00:00""#),
            "{out}"
        );
        assert!(
            out.contains(r#"tableooo:display-name="Rev total""#),
            "{out}"
        );
        assert!(!out.contains(r#"table:grouped-by="days""#), "{out}");
        let mut back = pivot_sheets();
        parse_pivots_xml(&out, &mut back);
        let p = &back[1].pivots[0];
        assert_eq!(p.data_fields[0].caption.as_deref(), Some("Rev total"));
        assert_eq!(
            p.row_fields[0].group,
            sheets[1].pivots[0].row_fields[0].group
        );
        assert_eq!(
            p.row_fields[1].group,
            sheets[1].pivots[0].row_fields[1].group
        );
    }

    #[test]
    fn style_names_round_trip() {
        assert_eq!(odf_style_name("Heading 1"), "Heading_20_1");
        assert_eq!(display_style_name("Heading_20_1"), "Heading 1");
        assert_eq!(display_style_name("Result2"), "Result2");
        assert_eq!(display_style_name("a_b"), "a_b");
    }

    #[test]
    fn groups_deeper_than_seven_levels_are_dropped() {
        let mut sheet = SheetData::default();
        for still in 0..9usize {
            let g = OpenGroup {
                rows: true,
                start: still as u32,
                depth: 0,
                collapsed: false,
            };
            close_group(&mut sheet, &g, 40 - still as u32, still);
        }
        sort_groups(&mut sheet);
        assert_eq!(sheet.row_groups.len(), 7);
        assert_eq!(sheet.row_groups.iter().map(|g| g.level).max(), Some(7));
    }

    #[test]
    fn group_tags_nest() {
        let g = [
            OutlineGroupData {
                start: 0,
                end: 5,
                level: 1,
                collapsed: false,
            },
            OutlineGroupData {
                start: 0,
                end: 2,
                level: 2,
                collapsed: true,
            },
        ];
        let mut w = GroupWalk::new(&g, true);
        let mut s = String::new();
        w.open(&mut s, 0);
        assert_eq!(
            s,
            r#"<table:table-row-group table:display="true"><table:table-row-group table:display="false">"#
        );
        assert_eq!(w.next_boundary(0), Some(3));
        let mut e = String::new();
        w.close(&mut e, 2);
        assert_eq!(e, "</table:table-row-group>");
        assert_eq!(w.next_boundary(3), Some(6));
        assert_eq!(w.last_index(), Some(5));
        let mut e = String::new();
        w.close_all(&mut e);
        assert_eq!(e, "</table:table-row-group>");
    }
}
