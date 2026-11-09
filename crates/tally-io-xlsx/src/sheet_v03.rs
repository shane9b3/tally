//! v0.3 worksheet parts: `conditionalFormatting`, `dataValidations`,
//! `hyperlinks`, `sheetProtection`, page setup (`pageMargins`,
//! `pageSetup`, `printOptions`, `headerFooter`, `sheetPr/pageSetUpPr`),
//! `rowBreaks`/`colBreaks`, and the comments part.

use std::io::BufRead;

use tally_model::{
    hash_algorithms, CellRangeLocal, CfColorStopData, CfDatePeriodData, CfKindData, CfRuleData,
    CfStyleData, CfTextOpData, CfThresholdData, CfValueKindData, CmpOpData, ConditionalFormatData,
    HeaderFooterData, HfAreaData, IconSetData, ListSourceData, MarginsData, NoteData,
    OrientationData, PageOrderData, PageScaleData, PageSetupData, PaperData, SheetProtectionData,
    ValidationData, ValidationErrorData, ValidationErrorStyleData, ValidationKindData,
    ValidationMessageData,
};
use tally_types::CellAddr;

use crate::colors::{XColor, INDEXED};
use crate::formula::{excel_to_of, XlsxResolver};
use crate::sheet::parse_range;
use crate::units::parse_bool;
use crate::xml::{Elem, Ev, XmlReader};
use crate::XlsxError;

/// Most rules, validations, links or breaks read per sheet.
const MAX_ITEMS: usize = 100_000;
/// Most conditional formats per sheet (the engine's cap; extras are
/// dropped with a warning).
pub(crate) const MAX_CF_PER_SHEET: usize = 2_048;
/// Most ranges per conditional format.
pub(crate) const MAX_CF_RANGES: usize = 64;
/// Most expanded hyperlink cells per sheet.
pub(crate) const MAX_LINKS_PER_SHEET: usize = 100_000;
/// Longest in-document `location` of a hyperlink.
const MAX_LOCATION: usize = 8 * 1024;
/// Longest comment text kept (the engine's note cap).
pub(crate) const MAX_NOTE_TEXT: usize = 32 * 1024;
/// Most hash rounds honoured for `spinCount` (clamped; MEDIUM-5).
const MAX_SPIN_COUNT: u32 = 100_000;
/// Longest `hashValue` / `saltValue` / `password` kept.
const MAX_HASH_LEN: usize = 1024;
/// Longest attribute-like text kept (messages, header/footer).
const MAX_TEXT: usize = 64 * 1024;

/// Page setup pieces, assembled at the end of the sheet.
#[derive(Debug, Default)]
pub(crate) struct PageParts {
    pub any: bool,
    pub margins: Option<MarginsData>,
    pub orientation: Option<OrientationData>,
    pub paper: Option<PaperData>,
    pub scale: Option<u32>,
    pub fit: Option<(u32, u32)>,
    pub fit_to_page: bool,
    pub first_page: Option<u32>,
    pub over_then_down: bool,
    pub gridlines: bool,
    pub headings: bool,
    pub center_h: bool,
    pub center_v: bool,
    pub header: Option<HeaderFooterData>,
    pub footer: Option<HeaderFooterData>,
}

impl PageParts {
    /// The model page setup (`None` when the sheet had no page parts).
    pub fn build(&self) -> Option<PageSetupData> {
        if !self.any {
            return None;
        }
        let mut p = PageSetupData {
            header: None,
            footer: None,
            ..PageSetupData::default()
        };
        p.header = self.header.clone().filter(|h| !h.is_empty());
        p.footer = self.footer.clone().filter(|h| !h.is_empty());
        p.header_area.on = p.header.is_some();
        p.footer_area.on = p.footer.is_some();
        if let Some(m) = self.margins {
            apply_excel_margins(&mut p, m);
        }
        if let Some(o) = self.orientation {
            p.orientation = o;
        }
        if let Some(pp) = self.paper {
            p.paper = pp;
        }
        p.scale = if self.fit_to_page {
            let (w, h) = self.fit.unwrap_or((1, 1));
            PageScaleData::FitTo {
                width: w,
                height: h,
            }
        } else {
            PageScaleData::Percent(self.scale.unwrap_or(100).clamp(10, 400))
        };
        p.first_page_number = self.first_page;
        p.page_order = if self.over_then_down {
            PageOrderData::LeftToRight
        } else {
            PageOrderData::TopToBottom
        };
        p.print_gridlines = self.gridlines;
        p.print_headers = self.headings;
        p.center_h = self.center_h;
        p.center_v = self.center_v;
        Some(p)
    }
}

/// Excel's margins → the setup's margins and header/footer areas (for the
/// header and footer `p` holds). Excel's header sits `header` below the
/// page edge and its body starts at `top` whatever the header holds; Tally
/// (like AOO and ODF) puts the header area inside the top margin and
/// pushes the body down. So the top margin becomes Excel's header margin
/// and the header area (spacing included) fills the gap to Excel's top
/// margin, at a fixed height (Excel's gap does not grow with the text).
/// The footer likewise. `m` is kept for an exact write-back.
pub(crate) fn apply_excel_margins(p: &mut PageSetupData, m: MarginsData) {
    let d = MarginsData::default();
    p.margins = MarginsData {
        header: d.header,
        footer: d.footer,
        ..m
    };
    p.header_area = HfAreaData {
        on: p.header.is_some(),
        ..HfAreaData::default()
    };
    p.footer_area = HfAreaData {
        on: p.footer.is_some(),
        ..HfAreaData::default()
    };
    if p.header.is_some() {
        let (top, height, spacing) = hf_from_excel_margins(m.top, m.header);
        p.margins.top = top;
        p.margins.header = height;
        p.header_area.spacing = spacing;
        p.header_area.autofit = false;
    }
    if p.footer.is_some() {
        let (bottom, height, spacing) = hf_from_excel_margins(m.bottom, m.footer);
        p.margins.bottom = bottom;
        p.margins.footer = height;
        p.footer_area.spacing = spacing;
        p.footer_area.autofit = false;
    }
    p.excel_margins = Some(m);
}

/// Excel's `(page margin, header margin)` → Tally's `(page margin, area
/// height, spacing)`: the area, spacing included, starts at Excel's header
/// margin and ends at Excel's page margin; the spacing is AOO's 0.25 cm,
/// at most half the area.
pub(crate) fn hf_from_excel_margins(page: u32, hf: u32) -> (u32, u32, u32) {
    let height = page.saturating_sub(hf);
    (hf, height, HfAreaData::default().spacing.min(height / 2))
}

fn attr<'a>(e: &'a Elem, k: &str) -> Option<&'a str> {
    e.attr_local(k)
}

fn attr_bool(e: &Elem, k: &str, default: bool) -> bool {
    e.attr_local(k).map_or(default, parse_bool)
}

fn attr_f64(e: &Elem, k: &str) -> Option<f64> {
    e.attr_local(k)
        .and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|v| v.is_finite())
}

fn attr_u32(e: &Elem, k: &str) -> Option<u32> {
    e.attr_local(k).and_then(|v| v.trim().parse::<u32>().ok())
}

/// `A1:B2 D4` → ranges.
pub(crate) fn parse_sqref(s: &str) -> Vec<CellRangeLocal> {
    s.split_whitespace()
        .filter_map(parse_range)
        .take(1024)
        .collect()
}

/// Calls `f` for each child element of the element just started.
fn each_child<R: BufRead>(
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

fn cmp_op(s: &str) -> CmpOpData {
    match s {
        "notBetween" => CmpOpData::NotBetween,
        "equal" => CmpOpData::Eq,
        "notEqual" => CmpOpData::Ne,
        "greaterThan" => CmpOpData::Gt,
        "greaterThanOrEqual" => CmpOpData::Ge,
        "lessThan" => CmpOpData::Lt,
        "lessThanOrEqual" => CmpOpData::Le,
        _ => CmpOpData::Between,
    }
}

fn cfvo(e: &Elem, origin: CellAddr, r: &XlsxResolver) -> CfThresholdData {
    let val = attr(e, "val").unwrap_or("").to_string();
    let (kind, value) = match attr(e, "type").unwrap_or("num") {
        "min" => (CfValueKindData::Min, String::new()),
        "max" => (CfValueKindData::Max, String::new()),
        "autoMin" => (CfValueKindData::AutoMin, String::new()),
        "autoMax" => (CfValueKindData::AutoMax, String::new()),
        "percent" => (CfValueKindData::Percent, val),
        "percentile" => (CfValueKindData::Percentile, val),
        "formula" => (CfValueKindData::Formula, excel_to_of(&val, origin, r).0),
        _ => (CfValueKindData::Number, val),
    };
    // A non-numeric operand of a number-like kind is a formula.
    let value = match kind {
        CfValueKindData::Number | CfValueKindData::Percent | CfValueKindData::Percentile
            if !value.is_empty() && value.trim().parse::<f64>().is_err() =>
        {
            excel_to_of(&value, origin, r).0
        }
        _ => value,
    };
    CfThresholdData { kind, value }
}

fn color(e: &Elem) -> Option<u32> {
    XColor::from_attrs(|k| e.attr_local(k).map(str::to_string)).resolve(&INDEXED)
}

/// Parses one `conditionalFormatting` element.
pub(crate) fn parse_cf<R: BufRead>(
    xr: &mut XmlReader<R>,
    e: &Elem,
    sheet: u32,
    dxfs: &[CfStyleData],
    r: &XlsxResolver,
) -> Result<Option<ConditionalFormatData>, XlsxError> {
    let mut ranges = parse_sqref(attr(e, "sqref").unwrap_or(""));
    ranges.truncate(MAX_CF_RANGES);
    let base = ranges.first().map_or((0, 0), |r| (r.r0, r.c0));
    let origin = CellAddr::new(sheet, base.0, base.1);
    let mut rules: Vec<(u32, CfRuleData)> = Vec::new();
    each_child(xr, |xr, rule| {
        if rule.local() != "cfRule" || rules.len() >= MAX_ITEMS {
            return xr.skip();
        }
        let ty = attr(&rule, "type").unwrap_or("").to_string();
        let priority = attr_u32(&rule, "priority").unwrap_or(u32::MAX);
        let style = attr_u32(&rule, "dxfId")
            .and_then(|i| dxfs.get(i as usize).cloned())
            .unwrap_or_default();
        let mut formulas: Vec<String> = Vec::new();
        let mut cfvos: Vec<CfThresholdData> = Vec::new();
        let mut colors: Vec<u32> = Vec::new();
        let mut bar_show = true;
        let mut icon: Option<(String, bool, bool)> = None;
        each_child(xr, |xr, c| match c.local() {
            "formula" => {
                let t = xr.text_content(MAX_TEXT)?;
                if formulas.len() < 3 {
                    formulas.push(t);
                }
                Ok(())
            }
            "colorScale" | "dataBar" | "iconSet" => {
                if c.local() == "dataBar" {
                    bar_show = attr_bool(&c, "showValue", true);
                }
                if c.local() == "iconSet" {
                    icon = Some((
                        attr(&c, "iconSet").unwrap_or("3TrafficLights1").to_string(),
                        attr_bool(&c, "showValue", true),
                        attr_bool(&c, "reverse", false),
                    ));
                }
                each_child(xr, |xr, v| {
                    match v.local() {
                        "cfvo" if cfvos.len() < 8 => cfvos.push(cfvo(&v, origin, r)),
                        "color" if colors.len() < 8 => {
                            if let Some(c) = color(&v) {
                                colors.push(c);
                            }
                        }
                        _ => {}
                    }
                    xr.skip()
                })
            }
            _ => xr.skip(),
        })?;
        let of = |i: usize| -> Option<String> {
            formulas.get(i).map(|f| excel_to_of(f.trim(), origin, r).0)
        };
        let text = || attr(&rule, "text").unwrap_or("").to_string();
        let kind = match ty.as_str() {
            "cellIs" => {
                let op = cmp_op(attr(&rule, "operator").unwrap_or("between"));
                let n = if matches!(op, CmpOpData::Between | CmpOpData::NotBetween) {
                    2
                } else {
                    1
                };
                let fs: Vec<String> = (0..n).filter_map(of).collect();
                (fs.len() == n).then_some(CfKindData::CellValue { op, formulas: fs })
            }
            "expression" => of(0).map(|formula| CfKindData::Formula { formula }),
            "colorScale" => {
                let stops: Vec<CfColorStopData> = cfvos
                    .iter()
                    .zip(&colors)
                    .map(|(t, c)| CfColorStopData {
                        threshold: t.clone(),
                        color: *c,
                    })
                    .collect();
                (2..=3)
                    .contains(&stops.len())
                    .then_some(CfKindData::ColorScale { stops })
            }
            "dataBar" => (cfvos.len() >= 2).then(|| CfKindData::DataBar {
                min: cfvos[0].clone(),
                max: cfvos[1].clone(),
                color: colors.first().copied().unwrap_or(0x638e_c6ff),
                negative_color: None,
                show_value: bar_show,
                gradient: true,
            }),
            "iconSet" => icon.as_ref().and_then(|(name, show, rev)| {
                let set = IconSetData::from_file_name(name)?;
                let mut thresholds = cfvos.clone();
                thresholds.truncate(set.icon_count());
                (thresholds.len() == set.icon_count()).then_some(CfKindData::IconSet {
                    set,
                    thresholds,
                    show_value: *show,
                    reverse: *rev,
                })
            }),
            "duplicateValues" => Some(CfKindData::Duplicate),
            "uniqueValues" => Some(CfKindData::Unique),
            "top10" => Some(CfKindData::Top {
                rank: attr_u32(&rule, "rank").unwrap_or(10).max(1),
                percent: attr_bool(&rule, "percent", false),
                bottom: attr_bool(&rule, "bottom", false),
            }),
            "aboveAverage" => Some(CfKindData::Average {
                above: attr_bool(&rule, "aboveAverage", true),
                equal: attr_bool(&rule, "equalAverage", false),
            }),
            "containsText" => Some(CfKindData::Text {
                op: CfTextOpData::Contains,
                text: text(),
            }),
            "notContainsText" => Some(CfKindData::Text {
                op: CfTextOpData::NotContains,
                text: text(),
            }),
            "beginsWith" => Some(CfKindData::Text {
                op: CfTextOpData::BeginsWith,
                text: text(),
            }),
            "endsWith" => Some(CfKindData::Text {
                op: CfTextOpData::EndsWith,
                text: text(),
            }),
            "containsBlanks" => Some(CfKindData::Blank),
            "notContainsBlanks" => Some(CfKindData::NotBlank),
            "containsErrors" => Some(CfKindData::Error),
            "notContainsErrors" => Some(CfKindData::NotError),
            "timePeriod" => {
                let p = match attr(&rule, "timePeriod").unwrap_or("") {
                    "today" => Some(CfDatePeriodData::Today),
                    "yesterday" => Some(CfDatePeriodData::Yesterday),
                    "tomorrow" => Some(CfDatePeriodData::Tomorrow),
                    "last7Days" => Some(CfDatePeriodData::Last7Days),
                    "thisWeek" => Some(CfDatePeriodData::ThisWeek),
                    "lastWeek" => Some(CfDatePeriodData::LastWeek),
                    "nextWeek" => Some(CfDatePeriodData::NextWeek),
                    "thisMonth" => Some(CfDatePeriodData::ThisMonth),
                    "lastMonth" => Some(CfDatePeriodData::LastMonth),
                    "nextMonth" => Some(CfDatePeriodData::NextMonth),
                    _ => None,
                };
                p.map(|period| CfKindData::DateIs { period })
            }
            _ => None,
        };
        if let Some(kind) = kind {
            rules.push((
                priority,
                CfRuleData {
                    kind,
                    style,
                    stop_if_true: attr_bool(&rule, "stopIfTrue", false),
                },
            ));
        }
        Ok(())
    })?;
    if ranges.is_empty() || rules.is_empty() {
        return Ok(None);
    }
    rules.sort_by_key(|(p, _)| *p);
    Ok(Some(ConditionalFormatData {
        ranges,
        base,
        rules: rules.into_iter().map(|(_, r)| r).collect(),
    }))
}

/// Parses one `dataValidation` element.
pub(crate) fn parse_dv<R: BufRead>(
    xr: &mut XmlReader<R>,
    e: &Elem,
    sheet: u32,
    r: &XlsxResolver,
) -> Result<Option<ValidationData>, XlsxError> {
    let mut ranges = parse_sqref(attr(e, "sqref").unwrap_or(""));
    ranges.truncate(MAX_CF_RANGES);
    let base = ranges.first().map_or((0, 0), |r| (r.r0, r.c0));
    let origin = CellAddr::new(sheet, base.0, base.1);
    let mut f1: Option<String> = None;
    let mut f2: Option<String> = None;
    each_child(xr, |xr, c| match c.local() {
        "formula1" => {
            f1 = Some(xr.text_content(MAX_TEXT)?);
            Ok(())
        }
        "formula2" => {
            f2 = Some(xr.text_content(MAX_TEXT)?);
            Ok(())
        }
        _ => xr.skip(),
    })?;
    if ranges.is_empty() {
        return Ok(None);
    }
    let of = |s: &Option<String>| s.as_ref().map(|t| excel_to_of(t.trim(), origin, r).0);
    let op = cmp_op(attr(e, "operator").unwrap_or("between"));
    let operands = || -> Vec<String> {
        let mut v: Vec<String> = of(&f1).into_iter().collect();
        if matches!(op, CmpOpData::Between | CmpOpData::NotBetween) {
            v.extend(of(&f2));
        }
        v
    };
    let kind = match attr(e, "type").unwrap_or("none") {
        "whole" => ValidationKindData::Whole {
            op,
            formulas: operands(),
        },
        "decimal" => ValidationKindData::Decimal {
            op,
            formulas: operands(),
        },
        "date" => ValidationKindData::Date {
            op,
            formulas: operands(),
        },
        "time" => ValidationKindData::Time {
            op,
            formulas: operands(),
        },
        "textLength" => ValidationKindData::TextLength {
            op,
            formulas: operands(),
        },
        "custom" => match of(&f1) {
            Some(formula) => ValidationKindData::Custom { formula },
            None => return Ok(None),
        },
        "list" => {
            let Some(raw) = f1.as_deref().map(str::trim) else {
                return Ok(None);
            };
            match raw.strip_prefix('"').and_then(|x| x.strip_suffix('"')) {
                Some(inner) => ValidationKindData::List {
                    source: ListSourceData::Values(split_list_items(inner)),
                },
                None => ValidationKindData::List {
                    source: ListSourceData::Range(excel_to_of(raw, origin, r).0),
                },
            }
        }
        _ => ValidationKindData::Any,
    };
    let s = |k: &str| attr(e, k).unwrap_or("").to_string();
    let input_help = attr_bool(e, "showInputMessage", false).then(|| ValidationMessageData {
        title: s("promptTitle"),
        text: s("prompt"),
    });
    let error = attr_bool(e, "showErrorMessage", false).then(|| ValidationErrorData {
        style: match attr(e, "errorStyle").unwrap_or("stop") {
            "warning" => ValidationErrorStyleData::Warning,
            "information" => ValidationErrorStyleData::Info,
            _ => ValidationErrorStyleData::Stop,
        },
        title: s("errorTitle"),
        text: s("error"),
    });
    Ok(Some(ValidationData {
        ranges,
        base,
        kind,
        allow_blank: attr_bool(e, "allowBlank", false),
        // OOXML's showDropDown="1" *hides* the in-cell dropdown.
        show_dropdown: !attr_bool(e, "showDropDown", false),
        // Excel dropdowns are never sorted.
        sort_ascending: false,
        input_help,
        error,
    }))
}

/// `sheetProtection` → model (`None` unless `sheet="1"`).
pub(crate) fn parse_protection(e: &Elem) -> Option<SheetProtectionData> {
    if !attr_bool(e, "sheet", false) {
        return None;
    }
    // Over-long hash material is dropped (the sheet stays protected and
    // any password unlocks it, as for an unknown algorithm); the spin
    // count is clamped to Excel's default (MEDIUM-5).
    let a = |k: &str| attr(e, k).map(str::to_string);
    let too_long = ["password", "hashValue", "saltValue", "algorithmName"]
        .iter()
        .any(|k| attr(e, k).is_some_and(|v| v.len() > MAX_HASH_LEN));
    let mut p = SheetProtectionData::default();
    if too_long {
        // Keep the sheet protected without hash material: any password
        // unlocks it (as for an unknown algorithm) instead of none.
    } else if let Some(h) = a("password").filter(|h| !h.is_empty()) {
        p.password_hash = Some(h);
        p.algorithm = Some(hash_algorithms::XLSX_LEGACY.into());
    } else if let Some(h) = a("hashValue") {
        p.password_hash = Some(h);
        p.algorithm = a("algorithmName");
        p.salt = a("saltValue");
        p.spin_count = attr_u32(e, "spinCount").map(|n| n.min(MAX_SPIN_COUNT));
    }
    // Each attribute says the action is *protected* (not allowed).
    let allowed = |k: &str, protected_by_default: bool| !attr_bool(e, k, protected_by_default);
    p.select_locked = allowed("selectLockedCells", false);
    p.select_unlocked = allowed("selectUnlockedCells", false);
    p.format_cells = allowed("formatCells", true);
    p.insert_rows = allowed("insertRows", true);
    p.insert_cols = allowed("insertColumns", true);
    p.delete_rows = allowed("deleteRows", true);
    p.delete_cols = allowed("deleteColumns", true);
    p.sort = allowed("sort", true);
    p.auto_filter = allowed("autoFilter", true);
    Some(p)
}

fn paper_from_code(code: u32) -> Option<PaperData> {
    Some(match code {
        1 | 2 => PaperData::Letter,
        3 => PaperData::Tabloid,
        5 => PaperData::Legal,
        7 => PaperData::Executive,
        8 => PaperData::A3,
        9 | 10 => PaperData::A4,
        11 => PaperData::A5,
        13 | 34 => PaperData::B5,
        _ => return None,
    })
}

/// `pageMargins`, `pageSetup`, `printOptions`, `pageSetUpPr`.
pub(crate) fn page_attrs(e: &Elem, p: &mut PageParts) {
    let inch = |k: &str| attr_f64(e, k).map(|v| (v.max(0.0) * 2540.0).round() as u32);
    match e.local() {
        "pageMargins" => {
            p.any = true;
            let d = MarginsData::default();
            p.margins = Some(MarginsData {
                left: inch("left").unwrap_or(d.left),
                right: inch("right").unwrap_or(d.right),
                top: inch("top").unwrap_or(d.top),
                bottom: inch("bottom").unwrap_or(d.bottom),
                header: inch("header").unwrap_or(d.header),
                footer: inch("footer").unwrap_or(d.footer),
            });
        }
        "pageSetup" => {
            p.any = true;
            p.orientation = match attr(e, "orientation") {
                Some("landscape") => Some(OrientationData::Landscape),
                Some("portrait") => Some(OrientationData::Portrait),
                _ => None,
            };
            p.paper = attr_u32(e, "paperSize").and_then(paper_from_code);
            p.scale = attr_u32(e, "scale");
            p.fit = Some((
                attr_u32(e, "fitToWidth").unwrap_or(1),
                attr_u32(e, "fitToHeight").unwrap_or(1),
            ));
            if attr_bool(e, "useFirstPageNumber", false) {
                p.first_page = attr_u32(e, "firstPageNumber");
            }
            // Some writers put the number into `useFirstPageNumber`.
            if p.first_page.is_none() {
                p.first_page = attr_u32(e, "useFirstPageNumber").filter(|n| *n > 1);
            }
            p.over_then_down = attr(e, "pageOrder") == Some("overThenDown");
        }
        "printOptions" => {
            p.any = true;
            p.gridlines = attr_bool(e, "gridLines", false);
            p.headings = attr_bool(e, "headings", false);
            p.center_h = attr_bool(e, "horizontalCentered", false);
            p.center_v = attr_bool(e, "verticalCentered", false);
        }
        "pageSetUpPr" => p.fit_to_page = attr_bool(e, "fitToPage", false),
        _ => {}
    }
}

/// Excel header/footer codes → model regions.
pub(crate) fn hf_from_excel(s: &str) -> HeaderFooterData {
    let mut out = HeaderFooterData::default();
    let mut cur: u8 = b'C';
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    let push = |out: &mut HeaderFooterData, cur: u8, t: &str| {
        let slot = match cur {
            b'L' => &mut out.left,
            b'R' => &mut out.right,
            _ => &mut out.center,
        };
        slot.push_str(t);
    };
    while i < chars.len() {
        let c = chars[i];
        if c != '&' || i + 1 >= chars.len() {
            let mut b = [0u8; 4];
            push(&mut out, cur, c.encode_utf8(&mut b));
            i += 1;
            continue;
        }
        let n = chars[i + 1];
        i += 2;
        match n {
            'L' | 'C' | 'R' => cur = n as u8,
            '&' => push(&mut out, cur, "&"),
            'P' => push(&mut out, cur, "&[PAGE]"),
            'N' => push(&mut out, cur, "&[PAGES]"),
            'A' => push(&mut out, cur, "&[SHEET]"),
            'F' => push(&mut out, cur, "&[FILE]"),
            'D' => push(&mut out, cur, "&[DATE]"),
            'T' => push(&mut out, cur, "&[TIME]"),
            '"' => {
                // &"Font,Style": skip to the closing quote.
                while i < chars.len() && chars[i] != '"' {
                    i += 1;
                }
                i += 1;
            }
            d if d.is_ascii_digit() => {
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }
            }
            // &B &I &U &S &E &X &Y &K…, &Z (path), &G (picture): dropped.
            'K' => i = (i + 6).min(chars.len()),
            _ => {}
        }
    }
    out
}

/// `headerFooter` children.
pub(crate) fn parse_header_footer<R: BufRead>(
    xr: &mut XmlReader<R>,
    p: &mut PageParts,
) -> Result<(), XlsxError> {
    p.any = true;
    each_child(xr, |xr, c| match c.local() {
        "oddHeader" => {
            p.header = Some(hf_from_excel(&xr.text_content(MAX_TEXT)?));
            Ok(())
        }
        "oddFooter" => {
            p.footer = Some(hf_from_excel(&xr.text_content(MAX_TEXT)?));
            Ok(())
        }
        _ => xr.skip(),
    })
}

/// `rowBreaks` / `colBreaks` children → manual break positions.
pub(crate) fn parse_breaks<R: BufRead>(
    xr: &mut XmlReader<R>,
    out: &mut Vec<u32>,
) -> Result<(), XlsxError> {
    each_child(xr, |xr, b| {
        if b.local() == "brk" && out.len() < MAX_ITEMS {
            let manual = attr_bool(&b, "man", false);
            if let Some(id) = attr_u32(&b, "id").filter(|id| *id > 0 && manual) {
                out.push(id);
            }
        }
        xr.skip()
    })
}

/// A `hyperlink` element: (row, col, r:id, location).
pub(crate) type RawLink = (CellRangeLocal, Option<String>, Option<String>);

/// A hyperlink's relationship id; an over-long one (never a real id) is
/// dropped, so the link has no target.
fn link_id(e: &Elem) -> Option<String> {
    attr(e, "id")
        .filter(|id| id.len() <= crate::xml::MAX_ID_BYTES)
        .map(str::to_string)
}

/// Reads one `hyperlink`.
pub(crate) fn parse_link(e: &Elem) -> Option<RawLink> {
    let range = parse_range(attr(e, "ref")?)?;
    let location = attr(e, "location")?;
    if location.len() > MAX_LOCATION {
        return None;
    }
    Some((range, link_id(e), Some(location.to_string())))
}

/// Reads one `hyperlink` without a `location`.
pub(crate) fn parse_link_any(e: &Elem) -> Option<RawLink> {
    match attr(e, "location") {
        Some(_) => parse_link(e),
        None => Some((parse_range(attr(e, "ref")?)?, link_id(e), None)),
    }
}

/// `xl/comments*.xml` → notes, with the number of comments truncated to
/// [`MAX_NOTE_TEXT`].
pub(crate) fn parse_comments<R: BufRead>(
    xr: &mut XmlReader<R>,
) -> Result<(Vec<NoteData>, usize), XlsxError> {
    let mut authors: Vec<String> = Vec::new();
    let mut notes: Vec<NoteData> = Vec::new();
    let mut truncated = 0usize;
    loop {
        match xr.next()? {
            Ev::Start(e) => match e.local() {
                "comments" | "authors" | "commentList" => {}
                "author" => {
                    let a = xr.text_content(MAX_TEXT)?;
                    if authors.len() < MAX_ITEMS {
                        authors.push(a);
                    }
                }
                "comment" => {
                    let pos = attr(&e, "ref").and_then(parse_range);
                    let author = attr_u32(&e, "authorId")
                        .and_then(|i| authors.get(i as usize).cloned())
                        .filter(|a| !a.is_empty());
                    let text = xr.text_content(MAX_TEXT)?;
                    if let Some(p) = pos {
                        if notes.len() < MAX_ITEMS {
                            // Excel prefixes the text with "Author:\n".
                            let mut t = match &author {
                                Some(a) => text
                                    .strip_prefix(&format!("{a}:"))
                                    .map(|x| x.trim_start_matches(['\n', '\r']).to_string())
                                    .unwrap_or(text),
                                None => text,
                            };
                            if t.len() > MAX_NOTE_TEXT {
                                let mut end = MAX_NOTE_TEXT;
                                while !t.is_char_boundary(end) {
                                    end -= 1;
                                }
                                t.truncate(end);
                                truncated += 1;
                            }
                            notes.push(NoteData {
                                row: p.r0,
                                col: p.c0,
                                text: t,
                                author,
                                created: None,
                                shown: false,
                            });
                        }
                    }
                }
                _ => xr.skip()?,
            },
            Ev::Eof => break,
            _ => {}
        }
    }
    notes.sort_by_key(|n| (n.row, n.col));
    notes.dedup_by_key(|n| (n.row, n.col));
    Ok((notes, truncated))
}

/// Splits the inside of an inline list (`"a,b,c"`) into items: a comma
/// inside a `""…""`-quoted item (the doubled quotes are the OOXML escape)
/// does not split it (`"a,""b,c"",d"` → `a`, `b,c`, `d`; review 3, LOW).
pub(crate) fn split_list_items(inner: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut chars = inner.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '"' {
            if chars.peek() == Some(&'"') {
                chars.next();
                in_quotes = !in_quotes;
                continue;
            }
            cur.push('"');
            continue;
        }
        if ch == ',' && !in_quotes {
            items.push(std::mem::take(&mut cur));
            continue;
        }
        cur.push(ch);
    }
    items.push(cur);
    items.into_iter().map(|s| s.trim().to_string()).collect()
}

/// `_xlnm.Print_Titles` text → (rows, cols) to repeat.
#[allow(clippy::type_complexity)]
pub(crate) fn print_titles(text: &str) -> (Option<(u32, u32)>, Option<(u32, u32)>) {
    let mut rows = None;
    let mut cols = None;
    for part in text.split(',') {
        let p = part.trim();
        let p = p.rsplit_once('!').map_or(p, |(_, r)| r);
        let p = p.replace('$', "");
        let Some((a, b)) = p.split_once(':') else {
            continue;
        };
        if let (Ok(x), Ok(y)) = (a.parse::<u32>(), b.parse::<u32>()) {
            if x >= 1 && y >= x {
                rows = Some((x - 1, y - 1));
            }
        } else if let (Some(x), Some(y)) = (
            tally_types::letters_to_col(a),
            tally_types::letters_to_col(b),
        ) {
            if y >= x {
                cols = Some((x, y));
            }
        }
    }
    (rows, cols)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_footer_codes() {
        let h = hf_from_excel("&L&\"Arial,Bold\"&12&F && co&CPage &P of &N&R&D");
        assert_eq!(h.left, "&[FILE] & co");
        assert_eq!(h.center, "Page &[PAGE] of &[PAGES]");
        assert_eq!(h.right, "&[DATE]");
        assert_eq!(hf_from_excel("Plain").center, "Plain");
        assert_eq!(
            print_titles("Sheet1!$1:$2,Sheet1!$A:$B"),
            (Some((0, 1)), Some((0, 1)))
        );
        assert_eq!(print_titles("'My S'!$3:$3"), (Some((2, 2)), None));
    }
}
