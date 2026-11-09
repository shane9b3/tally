//! v0.3 XLSX export: conditional formats, data validations, notes,
//! hyperlinks, sheet protection, page setup, breaks and document
//! properties, through `rust_xlsxwriter` (password hashes and workbook
//! structure protection, which it cannot write from a stored hash, are
//! patched into the package afterwards, see [`Patches`]).

use rust_xlsxwriter::{
    Color, ConditionalFormat2ColorScale, ConditionalFormat3ColorScale, ConditionalFormatAverage,
    ConditionalFormatAverageRule, ConditionalFormatBlank, ConditionalFormatCell,
    ConditionalFormatCellRule, ConditionalFormatCustomIcon, ConditionalFormatDataBar,
    ConditionalFormatDate, ConditionalFormatDateRule, ConditionalFormatDuplicate,
    ConditionalFormatError, ConditionalFormatFormula, ConditionalFormatIconSet,
    ConditionalFormatIconType, ConditionalFormatText, ConditionalFormatTextRule,
    ConditionalFormatTop, ConditionalFormatTopRule, ConditionalFormatType, DataValidation,
    DataValidationErrorStyle, DataValidationRule, Format, FormatBorder, FormatPattern,
    FormatUnderline, Formula, Note, ProtectionOptions, Url, Worksheet,
};
use tally_model::{
    hash_algorithms, CellRangeLocal, CfDatePeriodData, CfKindData, CfStyleData, CfTextOpData,
    CfThresholdData, CfValueKindData, CmpOpData, DisplayData, HeaderFooterData, IconSetData,
    ListSourceData, OrientationData, PageOrderData, PageScaleData, PageSetupData, PaperData,
    SheetData, SheetProtectionData, ValidationErrorStyleData, ValidationKindData, WorkbookData,
};
use tally_types::{col_to_letters, CellAddr, MAX_COLS, MAX_ROWS};

use crate::colors::model_to_rgb;
use crate::formula::{of_to_excel_at, XlsxResolver};
use crate::XlsxError;

/// Collects warnings (the writer's `Report` lives in `write.rs`).
pub(crate) trait Warn {
    fn warn(&mut self, w: String);
}

fn rgb(c: u32) -> Color {
    Color::RGB(model_to_rgb(c))
}

fn col16(c: u32) -> u16 {
    u16::try_from(c.min(MAX_COLS - 1)).unwrap_or(u16::MAX)
}

fn a1(r: u32, c: u32) -> String {
    format!("{}{}", col_to_letters(c), u64::from(r) + 1)
}

/// `A1:B2 D4` for a list of ranges.
pub(crate) fn sqref(ranges: &[CellRangeLocal]) -> String {
    ranges
        .iter()
        .map(|r| {
            if r.r0 == r.r1 && r.c0 == r.c1 {
                a1(r.r0, r.c0)
            } else {
                format!("{}:{}", a1(r.r0, r.c0), a1(r.r1, r.c1))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The differential format of a conditional style.
fn dxf(s: &CfStyleData) -> Format {
    let mut f = Format::new();
    if s.bold == Some(true) {
        f = f.set_bold();
    }
    if s.italic == Some(true) {
        f = f.set_italic();
    }
    if s.underline == Some(true) {
        f = f.set_underline(FormatUnderline::Single);
    }
    if s.strike == Some(true) {
        f = f.set_font_strikethrough();
    }
    let (fill, color) = match &s.display {
        Some(DisplayData::Chip { bg, fg }) => (s.fill.or(Some(*bg)), s.color.or(Some(*fg))),
        None => (s.fill, s.color),
    };
    if let Some(c) = color {
        f = f.set_font_color(rgb(c));
    }
    if let Some(c) = fill {
        // Differential fills use the background colour of a solid pattern.
        f = f
            .set_pattern(FormatPattern::Solid)
            .set_background_color(rgb(c));
    }
    if let Some(b) = &s.borders {
        let line = |l: &tally_model::BorderLine| match l.style {
            tally_model::BorderStyle::Double => FormatBorder::Double,
            tally_model::BorderStyle::Dashed => FormatBorder::Dashed,
            tally_model::BorderStyle::Dotted => FormatBorder::Dotted,
            tally_model::BorderStyle::Solid if l.width_hmm > 35 => FormatBorder::Medium,
            tally_model::BorderStyle::Solid => FormatBorder::Thin,
        };
        if let Some(l) = &b.top {
            f = f.set_border_top(line(l)).set_border_top_color(rgb(l.color));
        }
        if let Some(l) = &b.bottom {
            f = f
                .set_border_bottom(line(l))
                .set_border_bottom_color(rgb(l.color));
        }
        if let Some(l) = &b.left {
            f = f
                .set_border_left(line(l))
                .set_border_left_color(rgb(l.color));
        }
        if let Some(l) = &b.right {
            f = f
                .set_border_right(line(l))
                .set_border_right_color(rgb(l.color));
        }
    }
    if let Some(code) = &s.number_format {
        if let Some(x) = crate::numfmt::ooo_to_excel(code) {
            f = f.set_num_format(x);
        }
    }
    f
}

/// Translates one model formula of a rule (relative to `base`) to Excel
/// text relative to the top-left cell `tl` (Excel's base).
struct Fx<'a> {
    sheet: u32,
    base: (u32, u32),
    tl: (u32, u32),
    resolver: &'a XlsxResolver,
}

impl Fx<'_> {
    fn excel(&self, of: &str) -> Option<String> {
        of_to_excel_at(
            of,
            CellAddr::new(self.sheet, self.base.0, self.base.1),
            CellAddr::new(self.sheet, self.tl.0, self.tl.1),
            self.resolver,
        )
    }

    fn formula(&self, of: &str) -> Option<Formula> {
        self.excel(of).map(|t| Formula::new(format!("={t}")))
    }
}

fn cf_type(t: &CfThresholdData, fx: &Fx<'_>) -> Option<(ConditionalFormatType, Formula)> {
    let num = || {
        t.value
            .trim()
            .parse::<f64>()
            .ok()
            .map(|n| Formula::new(format!("{n}")))
            .or_else(|| fx.formula(&t.value))
    };
    Some(match t.kind {
        CfValueKindData::Min => (ConditionalFormatType::Lowest, Formula::new("0")),
        CfValueKindData::Max => (ConditionalFormatType::Highest, Formula::new("0")),
        CfValueKindData::AutoMin | CfValueKindData::AutoMax => {
            (ConditionalFormatType::Automatic, Formula::new("0"))
        }
        CfValueKindData::Percent => (ConditionalFormatType::Percent, num()?),
        CfValueKindData::Percentile => (ConditionalFormatType::Percentile, num()?),
        CfValueKindData::Number => (ConditionalFormatType::Number, num()?),
        CfValueKindData::Formula => (ConditionalFormatType::Formula, fx.formula(&t.value)?),
    })
}

fn icon_type(s: IconSetData) -> ConditionalFormatIconType {
    use ConditionalFormatIconType as T;
    match s {
        IconSetData::ThreeArrows => T::ThreeArrows,
        IconSetData::ThreeArrowsGray => T::ThreeArrowsGray,
        IconSetData::ThreeFlags => T::ThreeFlags,
        IconSetData::ThreeTrafficLights => T::ThreeTrafficLights,
        IconSetData::ThreeTrafficLightsRimmed => T::ThreeTrafficLightsWithRim,
        IconSetData::ThreeSigns => T::ThreeSigns,
        IconSetData::ThreeSymbols => T::ThreeSymbolsCircled,
        IconSetData::ThreeSymbolsUncircled => T::ThreeSymbols,
        IconSetData::ThreeStars => T::ThreeStars,
        IconSetData::ThreeTriangles => T::ThreeTriangles,
        IconSetData::FourArrows => T::FourArrows,
        IconSetData::FourArrowsGray => T::FourArrowsGray,
        IconSetData::FourRedToBlack => T::FourRedToBlack,
        IconSetData::FourRating => T::FourHistograms,
        IconSetData::FourTrafficLights => T::FourTrafficLights,
        IconSetData::FiveArrows => T::FiveArrows,
        IconSetData::FiveArrowsGray => T::FiveArrowsGray,
        IconSetData::FiveRating => T::FiveHistograms,
        IconSetData::FiveQuarters => T::FiveQuadrants,
        IconSetData::FiveBoxes => T::FiveBoxes,
    }
}

fn cell_rule(op: CmpOpData, a: Formula, b: Option<Formula>) -> ConditionalFormatCellRule<Formula> {
    use ConditionalFormatCellRule as R;
    match (op, b) {
        (CmpOpData::Between, Some(b)) => R::Between(a, b),
        (CmpOpData::NotBetween, Some(b)) => R::NotBetween(a, b),
        (CmpOpData::Eq, _) => R::EqualTo(a),
        (CmpOpData::Ne, _) => R::NotEqualTo(a),
        (CmpOpData::Gt, _) => R::GreaterThan(a),
        (CmpOpData::Ge, _) => R::GreaterThanOrEqualTo(a),
        (CmpOpData::Lt, _) => R::LessThan(a),
        (CmpOpData::Le, _) | (_, None) => R::LessThanOrEqualTo(a),
    }
}

macro_rules! add_cf {
    ($ws:expr, $range:expr, $cf:expr, $multi:expr, $stop:expr) => {{
        let mut cf = $cf;
        if let Some(m) = $multi {
            cf = cf.set_multi_range(m);
        }
        cf = cf.set_stop_if_true($stop);
        let r = $range;
        $ws.add_conditional_format(r.r0, col16(r.c0), r.r1, col16(r.c1), &cf)
            .map(|_| ())
    }};
}

/// Writes the conditional formats of a sheet.
pub(crate) fn write_conditional_formats(
    ws: &mut Worksheet,
    sheet: &SheetData,
    sheet_index: u32,
    resolver: &XlsxResolver,
    report: &mut dyn Warn,
) -> Result<(), XlsxError> {
    for cf in &sheet.conditional_formats {
        let Some(first) = cf.ranges.first().copied() else {
            continue;
        };
        if cf
            .ranges
            .iter()
            .any(|r| r.r1 >= MAX_ROWS || r.c1 >= MAX_COLS)
        {
            continue;
        }
        let multi = (cf.ranges.len() > 1).then(|| sqref(&cf.ranges));
        let fx = Fx {
            sheet: sheet_index,
            base: cf.base,
            tl: (first.r0, first.c0),
            resolver,
        };
        for rule in &cf.rules {
            let fmt = dxf(&rule.style);
            let stop = rule.stop_if_true;
            let res: Result<(), rust_xlsxwriter::XlsxError> = match &rule.kind {
                CfKindData::CellValue { op, formulas } => {
                    let a = formulas.first().and_then(|f| fx.formula(f));
                    let b = formulas.get(1).and_then(|f| fx.formula(f));
                    match a {
                        Some(a) => add_cf!(
                            ws,
                            first,
                            ConditionalFormatCell::new()
                                .set_rule(cell_rule(*op, a, b))
                                .set_format(fmt),
                            multi.clone(),
                            stop
                        ),
                        None => {
                            report.warn(format!(
                                "sheet {:?}: a conditional format value could not be translated",
                                sheet.name
                            ));
                            Ok(())
                        }
                    }
                }
                CfKindData::Formula { formula } => match fx.excel(formula) {
                    Some(t) => add_cf!(
                        ws,
                        first,
                        ConditionalFormatFormula::new()
                            .set_rule(format!("={t}").as_str())
                            .set_format(fmt),
                        multi.clone(),
                        stop
                    ),
                    None => {
                        report.warn(format!(
                            "sheet {:?}: a conditional format formula could not be translated",
                            sheet.name
                        ));
                        Ok(())
                    }
                },
                CfKindData::ColorScale { stops } => {
                    let pts: Option<Vec<_>> = stops
                        .iter()
                        .map(|s| cf_type(&s.threshold, &fx).map(|(t, v)| (t, v, s.color)))
                        .collect();
                    match pts.as_deref() {
                        Some([a, b]) => add_cf!(
                            ws,
                            first,
                            ConditionalFormat2ColorScale::new()
                                .set_minimum(a.0, a.1.clone())
                                .set_maximum(b.0, b.1.clone())
                                .set_minimum_color(rgb(a.2))
                                .set_maximum_color(rgb(b.2)),
                            multi.clone(),
                            false
                        ),
                        Some([a, m, b]) => add_cf!(
                            ws,
                            first,
                            ConditionalFormat3ColorScale::new()
                                .set_minimum(a.0, a.1.clone())
                                .set_midpoint(m.0, m.1.clone())
                                .set_maximum(b.0, b.1.clone())
                                .set_minimum_color(rgb(a.2))
                                .set_midpoint_color(rgb(m.2))
                                .set_maximum_color(rgb(b.2)),
                            multi.clone(),
                            false
                        ),
                        _ => Ok(()),
                    }
                }
                CfKindData::DataBar {
                    min,
                    max,
                    color,
                    negative_color,
                    show_value,
                    gradient,
                } => {
                    let mut bar = ConditionalFormatDataBar::new()
                        .set_fill_color(rgb(*color))
                        .set_solid_fill(!gradient)
                        .set_bar_only(!show_value);
                    if let Some(n) = negative_color {
                        bar = bar.set_negative_fill_color(rgb(*n));
                    }
                    if let Some((t, v)) = cf_type(min, &fx) {
                        bar = bar.set_minimum(t, v);
                    }
                    if let Some((t, v)) = cf_type(max, &fx) {
                        bar = bar.set_maximum(t, v);
                    }
                    add_cf!(ws, first, bar, multi.clone(), false)
                }
                CfKindData::IconSet {
                    set,
                    thresholds,
                    show_value,
                    reverse,
                } => {
                    let it = icon_type(*set);
                    let icons: Vec<ConditionalFormatCustomIcon> = thresholds
                        .iter()
                        .map(|t| match cf_type(t, &fx) {
                            Some((ty, v)) => ConditionalFormatCustomIcon::new().set_rule(ty, v),
                            None => ConditionalFormatCustomIcon::new(),
                        })
                        .collect();
                    let mut is = ConditionalFormatIconSet::new()
                        .set_icon_type(it)
                        .show_icons_only(!show_value)
                        .reverse_icons(*reverse);
                    if icons.len() == set.icon_count() {
                        is = is.set_icons(&icons);
                    }
                    add_cf!(ws, first, is, multi.clone(), false)
                }
                CfKindData::Duplicate | CfKindData::Unique => {
                    let mut d = ConditionalFormatDuplicate::new().set_format(fmt);
                    if matches!(rule.kind, CfKindData::Unique) {
                        d = d.invert();
                    }
                    add_cf!(ws, first, d, multi.clone(), stop)
                }
                CfKindData::Top {
                    rank,
                    percent,
                    bottom,
                } => {
                    let n = u16::try_from(*rank).unwrap_or(1000).clamp(1, 1000);
                    let r = match (percent, bottom) {
                        (false, false) => ConditionalFormatTopRule::Top(n),
                        (false, true) => ConditionalFormatTopRule::Bottom(n),
                        (true, false) => ConditionalFormatTopRule::TopPercent(n.min(100)),
                        (true, true) => ConditionalFormatTopRule::BottomPercent(n.min(100)),
                    };
                    add_cf!(
                        ws,
                        first,
                        ConditionalFormatTop::new().set_rule(r).set_format(fmt),
                        multi.clone(),
                        stop
                    )
                }
                CfKindData::Average { above, equal } => {
                    let r = match (above, equal) {
                        (true, false) => ConditionalFormatAverageRule::AboveAverage,
                        (true, true) => ConditionalFormatAverageRule::EqualOrAboveAverage,
                        (false, false) => ConditionalFormatAverageRule::BelowAverage,
                        (false, true) => ConditionalFormatAverageRule::EqualOrBelowAverage,
                    };
                    add_cf!(
                        ws,
                        first,
                        ConditionalFormatAverage::new().set_rule(r).set_format(fmt),
                        multi.clone(),
                        stop
                    )
                }
                CfKindData::Text { op, text } => {
                    let r = match op {
                        CfTextOpData::Contains => ConditionalFormatTextRule::Contains(text.clone()),
                        CfTextOpData::NotContains => {
                            ConditionalFormatTextRule::DoesNotContain(text.clone())
                        }
                        CfTextOpData::BeginsWith => {
                            ConditionalFormatTextRule::BeginsWith(text.clone())
                        }
                        CfTextOpData::EndsWith => ConditionalFormatTextRule::EndsWith(text.clone()),
                    };
                    add_cf!(
                        ws,
                        first,
                        ConditionalFormatText::new().set_rule(r).set_format(fmt),
                        multi.clone(),
                        stop
                    )
                }
                CfKindData::Blank | CfKindData::NotBlank => {
                    let mut b = ConditionalFormatBlank::new().set_format(fmt);
                    if matches!(rule.kind, CfKindData::NotBlank) {
                        b = b.invert();
                    }
                    add_cf!(ws, first, b, multi.clone(), stop)
                }
                CfKindData::Error | CfKindData::NotError => {
                    let mut e = ConditionalFormatError::new().set_format(fmt);
                    if matches!(rule.kind, CfKindData::NotError) {
                        e = e.invert();
                    }
                    add_cf!(ws, first, e, multi.clone(), stop)
                }
                CfKindData::DateIs { period } => {
                    use ConditionalFormatDateRule as D;
                    let r = match period {
                        CfDatePeriodData::Today => Some(D::Today),
                        CfDatePeriodData::Yesterday => Some(D::Yesterday),
                        CfDatePeriodData::Tomorrow => Some(D::Tomorrow),
                        CfDatePeriodData::Last7Days => Some(D::Last7Days),
                        CfDatePeriodData::ThisWeek => Some(D::ThisWeek),
                        CfDatePeriodData::LastWeek => Some(D::LastWeek),
                        CfDatePeriodData::NextWeek => Some(D::NextWeek),
                        CfDatePeriodData::ThisMonth => Some(D::ThisMonth),
                        CfDatePeriodData::LastMonth => Some(D::LastMonth),
                        CfDatePeriodData::NextMonth => Some(D::NextMonth),
                        _ => None,
                    };
                    match r {
                        Some(r) => add_cf!(
                            ws,
                            first,
                            ConditionalFormatDate::new().set_rule(r).set_format(fmt),
                            multi.clone(),
                            stop
                        ),
                        None => {
                            // Excel has no year periods: an equivalent formula.
                            let delta = match period {
                                CfDatePeriodData::LastYear => "-1",
                                CfDatePeriodData::NextYear => "+1",
                                _ => "",
                            };
                            let cell = a1(first.r0, first.c0);
                            let f = format!("=YEAR({cell})=YEAR(TODAY()){delta}");
                            add_cf!(
                                ws,
                                first,
                                ConditionalFormatFormula::new()
                                    .set_rule(f.as_str())
                                    .set_format(fmt),
                                multi.clone(),
                                stop
                            )
                        }
                    }
                }
            };
            if let Err(e) = res {
                report.warn(format!(
                    "sheet {:?}: a conditional format was skipped ({e})",
                    sheet.name
                ));
            }
        }
    }
    Ok(())
}

/// Writes the data validations of a sheet.
pub(crate) fn write_validations(
    ws: &mut Worksheet,
    sheet: &SheetData,
    sheet_index: u32,
    resolver: &XlsxResolver,
    report: &mut dyn Warn,
) -> Result<(), XlsxError> {
    for v in &sheet.validations {
        let Some(first) = v.ranges.first().copied() else {
            continue;
        };
        let fx = Fx {
            sheet: sheet_index,
            base: v.base,
            tl: (first.r0, first.c0),
            resolver,
        };
        let rule = |op: CmpOpData, f: &[String]| -> Option<DataValidationRule<Formula>> {
            let a = fx.formula(f.first()?)?;
            let b = f.get(1).and_then(|x| fx.formula(x));
            Some(match (op, b) {
                (CmpOpData::Between, Some(b)) => DataValidationRule::Between(a, b),
                (CmpOpData::NotBetween, Some(b)) => DataValidationRule::NotBetween(a, b),
                (CmpOpData::Eq, _) => DataValidationRule::EqualTo(a),
                (CmpOpData::Ne, _) => DataValidationRule::NotEqualTo(a),
                (CmpOpData::Gt, _) => DataValidationRule::GreaterThan(a),
                (CmpOpData::Ge, _) => DataValidationRule::GreaterThanOrEqualTo(a),
                (CmpOpData::Lt, _) => DataValidationRule::LessThan(a),
                (CmpOpData::Le, _) | (_, None) => DataValidationRule::LessThanOrEqualTo(a),
            })
        };
        let mut dv = DataValidation::new();
        let ok = match &v.kind {
            ValidationKindData::Any => {
                dv = dv.allow_any_value();
                true
            }
            ValidationKindData::List { source } => match source {
                ListSourceData::Values(items) => {
                    let refs: Vec<&str> = items.iter().map(String::as_str).collect();
                    match dv.clone().allow_list_strings(&refs) {
                        Ok(d) => {
                            dv = d;
                            true
                        }
                        Err(e) => {
                            report.warn(format!("a list validation was skipped ({e})"));
                            false
                        }
                    }
                }
                ListSourceData::Range(f) => match fx.formula(f) {
                    Some(f) => {
                        dv = dv.allow_list_formula(f);
                        true
                    }
                    None => false,
                },
            },
            ValidationKindData::Whole { op, formulas } => match rule(*op, formulas) {
                Some(r) => {
                    dv = dv.allow_whole_number_formula(r);
                    true
                }
                None => false,
            },
            ValidationKindData::Decimal { op, formulas } => match rule(*op, formulas) {
                Some(r) => {
                    dv = dv.allow_decimal_number_formula(r);
                    true
                }
                None => false,
            },
            ValidationKindData::Date { op, formulas } => match rule(*op, formulas) {
                Some(r) => {
                    dv = dv.allow_date_formula(r);
                    true
                }
                None => false,
            },
            ValidationKindData::Time { op, formulas } => match rule(*op, formulas) {
                Some(r) => {
                    dv = dv.allow_time_formula(r);
                    true
                }
                None => false,
            },
            ValidationKindData::TextLength { op, formulas } => match rule(*op, formulas) {
                Some(r) => {
                    dv = dv.allow_text_length_formula(r);
                    true
                }
                None => false,
            },
            ValidationKindData::Custom { formula } => match fx.formula(formula) {
                Some(f) => {
                    dv = dv.allow_custom(f);
                    true
                }
                None => false,
            },
        };
        if !ok {
            report.warn(format!(
                "sheet {:?}: a validation could not be translated",
                sheet.name
            ));
            continue;
        }
        dv = dv
            .ignore_blank(v.allow_blank)
            .show_dropdown(v.show_dropdown);
        if let Some(h) = &v.input_help {
            dv = dv.show_input_message(true);
            if let Ok(d) = dv
                .clone()
                .set_input_title(h.title.chars().take(32).collect::<String>())
            {
                dv = d;
            }
            if let Ok(d) = dv
                .clone()
                .set_input_message(h.text.chars().take(255).collect::<String>())
            {
                dv = d;
            }
        } else {
            dv = dv.show_input_message(false);
        }
        match &v.error {
            Some(e) => {
                dv = dv.show_error_message(true).set_error_style(match e.style {
                    ValidationErrorStyleData::Stop => DataValidationErrorStyle::Stop,
                    ValidationErrorStyleData::Warning => DataValidationErrorStyle::Warning,
                    ValidationErrorStyleData::Info => DataValidationErrorStyle::Information,
                });
                if !e.title.is_empty() {
                    if let Ok(d) = dv
                        .clone()
                        .set_error_title(e.title.chars().take(32).collect::<String>())
                    {
                        dv = d;
                    }
                }
                if !e.text.is_empty() {
                    if let Ok(d) = dv
                        .clone()
                        .set_error_message(e.text.chars().take(255).collect::<String>())
                    {
                        dv = d;
                    }
                }
            }
            None => dv = dv.show_error_message(false),
        }
        if v.ranges.len() > 1 {
            dv = dv.set_multi_range(sqref(&v.ranges));
        }
        if let Err(e) =
            ws.add_data_validation(first.r0, col16(first.c0), first.r1, col16(first.c1), &dv)
        {
            report.warn(format!(
                "sheet {:?}: a validation was skipped ({e})",
                sheet.name
            ));
        }
    }
    Ok(())
}

/// Writes the notes of a sheet.
pub(crate) fn write_notes(ws: &mut Worksheet, sheet: &SheetData, report: &mut dyn Warn) {
    for n in &sheet.notes {
        if n.row >= MAX_ROWS || n.col >= MAX_COLS {
            continue;
        }
        let mut note = Note::new(n.text.as_str()).set_visible(n.shown);
        if let Some(a) = n.author.as_deref().filter(|a| !a.is_empty()) {
            note = note.set_author(a).add_author_prefix(false);
        }
        if let Err(e) = ws.insert_note(n.row, col16(n.col), &note) {
            report.warn(format!("sheet {:?}: a note was skipped ({e})", sheet.name));
        }
    }
}

/// The `rust_xlsxwriter` URL for a model link (`#Sheet.A1` becomes an
/// internal link).
pub(crate) fn excel_url(url: &str) -> Option<Url> {
    let u = url.trim();
    if let Some(target) = u.strip_prefix('#') {
        // `Sheet1.A1` / `'My Sheet'.A1` → `Sheet1!A1`.
        let t = match target.rfind('.') {
            Some(i) => format!("{}!{}", &target[..i], &target[i + 1..]),
            None => target.to_string(),
        };
        return Some(Url::new(format!("internal:{t}")));
    }
    let lower = u.to_ascii_lowercase();
    let ok = [
        "http://", "https://", "ftp://", "ftps://", "mailto:", "file:",
    ]
    .iter()
    .any(|p| lower.starts_with(p));
    ok.then(|| Url::new(u))
}

/// Writes the sheet protection (the password hash is patched in later).
pub(crate) fn write_protection(ws: &mut Worksheet, p: &SheetProtectionData) {
    let opts = ProtectionOptions {
        select_locked_cells: p.select_locked,
        select_unlocked_cells: p.select_unlocked,
        format_cells: p.format_cells,
        insert_rows: p.insert_rows,
        insert_columns: p.insert_cols,
        delete_rows: p.delete_rows,
        delete_columns: p.delete_cols,
        sort: p.sort,
        use_autofilter: p.auto_filter,
        ..ProtectionOptions::default()
    };
    ws.protect_with_options(&opts);
}

/// Excel's default header/footer margin (0.3"), 1/100 mm.
const EXCEL_HF_MARGIN: u32 = 762;

/// Excel's `(top, bottom, header, footer)` margins of a setup. Imported
/// margins are written back as they were while the setup still matches
/// them. Otherwise Excel's header margin is the header's distance from the
/// page edge (Tally's top margin) and its top margin the body's (plus the
/// printed header area, an empty one switched on included, so the body
/// sits where Tally prints it); a page without a header keeps its top
/// margin and Excel's 0.3" header margin (or less). Footers likewise.
pub(crate) fn excel_margins(p: &PageSetupData) -> (u32, u32, u32, u32) {
    if let Some(raw) = p.excel_margins {
        let mut q = p.clone();
        crate::sheet_v03::apply_excel_margins(&mut q, raw);
        if q.margins == p.margins
            && q.header_area == p.header_area
            && q.footer_area == p.footer_area
        {
            return (raw.top, raw.bottom, raw.header, raw.footer);
        }
    }
    let m = &p.margins;
    let (top, header) = match p.header_shown() {
        Some(h) => (
            m.top
                .saturating_add(p.header_area.printed_height(h, m.header)),
            m.top,
        ),
        None => (m.top, m.top.min(EXCEL_HF_MARGIN)),
    };
    let (bottom, footer) = match p.footer_shown() {
        Some(f) => (
            m.bottom
                .saturating_add(p.footer_area.printed_height(f, m.footer)),
            m.bottom,
        ),
        None => (m.bottom, m.bottom.min(EXCEL_HF_MARGIN)),
    };
    (top, bottom, header, footer)
}

/// Model header/footer → Excel code string (`&L…&C…&R…`).
pub(crate) fn hf_to_excel(h: &HeaderFooterData) -> String {
    let conv = |s: &str| {
        s.replace('&', "&&")
            .replace("&&[PAGES]", "&N")
            .replace("&&[PAGE]", "&P")
            .replace("&&[SHEET]", "&A")
            .replace("&&[FILE]", "&F")
            .replace("&&[DATE]", "&D")
            .replace("&&[TIME]", "&T")
    };
    let mut out = String::new();
    for (code, text) in [("&L", &h.left), ("&C", &h.center), ("&R", &h.right)] {
        if !text.is_empty() {
            out.push_str(code);
            out.push_str(&conv(text));
        }
    }
    out.chars().take(255).collect()
}

fn paper_code(p: PaperData) -> u8 {
    match p {
        PaperData::Letter => 1,
        PaperData::Tabloid => 3,
        PaperData::Legal => 5,
        PaperData::Executive => 7,
        PaperData::A3 => 8,
        PaperData::A4 => 9,
        PaperData::A5 => 11,
        // ISO B5 176 × 250 mm (ECMA-376 §18.3.1.63 code 13); 34 is JIS B5
        // 182 × 257 (review 3, M10).
        PaperData::B5 => 13,
        PaperData::Custom { .. } => 0,
    }
}

/// Writes the page setup, breaks and print titles of a sheet.
pub(crate) fn write_page_setup(
    ws: &mut Worksheet,
    sheet: &SheetData,
    report: &mut dyn Warn,
) -> Result<(), XlsxError> {
    if let Some(p) = &sheet.page_setup {
        write_setup(ws, p, report)?;
    }
    if !sheet.row_breaks.is_empty() {
        let rows: Vec<u32> = sheet
            .row_breaks
            .iter()
            .copied()
            .filter(|r| *r > 0 && *r < MAX_ROWS)
            .take(1023)
            .collect();
        ws.set_page_breaks(&rows)?;
    }
    if !sheet.col_breaks.is_empty() {
        let cols: Vec<u32> = sheet
            .col_breaks
            .iter()
            .copied()
            .filter(|c| *c > 0 && *c < MAX_COLS)
            .take(1023)
            .collect();
        ws.set_vertical_page_breaks(&cols)?;
    }
    Ok(())
}

fn write_setup(
    ws: &mut Worksheet,
    p: &PageSetupData,
    report: &mut dyn Warn,
) -> Result<(), XlsxError> {
    match p.orientation {
        OrientationData::Portrait => ws.set_portrait(),
        OrientationData::Landscape => ws.set_landscape(),
    };
    let code = paper_code(p.paper);
    if code > 0 {
        ws.set_paper_size(code);
    } else {
        report.warn("a custom paper size was written as the default paper".into());
    }
    let inch = |hmm: u32| f64::from(hmm) / 2540.0;
    let (top, bottom, header, footer) = excel_margins(p);
    let m = &p.margins;
    ws.set_margins(
        inch(m.left),
        inch(m.right),
        inch(top),
        inch(bottom),
        inch(header),
        inch(footer),
    );
    match p.scale {
        PageScaleData::Percent(n) => {
            ws.set_print_scale(u16::try_from(n.clamp(10, 400)).unwrap_or(100));
        }
        PageScaleData::FitTo { width, height } => {
            ws.set_print_fit_to_pages(
                u16::try_from(width).unwrap_or(1),
                u16::try_from(height).unwrap_or(0),
            );
        }
        PageScaleData::FitPages(pages) => {
            // Excel has no "N pages in total" mode; 1 wide × N tall is the
            // closest (TODO(W5-3): warn and refine).
            ws.set_print_fit_to_pages(1, u16::try_from(pages).unwrap_or(1));
        }
    }
    if let Some(r) = p.print_range {
        if r.r1 < MAX_ROWS && r.c1 < MAX_COLS {
            ws.set_print_area(r.r0, col16(r.c0), r.r1, col16(r.c1))?;
        }
    }
    if let Some((a, b)) = p.repeat_rows {
        ws.set_repeat_rows(a, b)?;
    }
    if let Some((a, b)) = p.repeat_cols {
        ws.set_repeat_columns(col16(a), col16(b))?;
    }
    // Excel has no "header off": a header switched off is not written.
    if let Some(h) = p.header_shown().filter(|h| !h.is_empty()) {
        ws.set_header(hf_to_excel(h));
    }
    if let Some(h) = p.footer_shown().filter(|h| !h.is_empty()) {
        ws.set_footer(hf_to_excel(h));
    }
    ws.set_print_gridlines(p.print_gridlines);
    ws.set_print_headings(p.print_headers);
    ws.set_print_center_horizontally(p.center_h);
    ws.set_print_center_vertically(p.center_v);
    ws.set_page_order(p.page_order == PageOrderData::TopToBottom);
    // The first page number is patched in (see `Patches::page_setup`):
    // rust_xlsxwriter writes it into `useFirstPageNumber`, a boolean.
    Ok(())
}

/// One relationship to add: `(Id, Type, Target)`.
pub(crate) type RelEntry = (String, String, String);

/// Post-write edits of the package for what `rust_xlsxwriter` cannot
/// express: stored password hashes and workbook structure protection.
#[derive(Default)]
pub(crate) struct Patches {
    /// `xl/worksheets/sheetN.xml` → attributes to add to `<sheetProtection`.
    pub sheet_protection: Vec<(String, String)>,
    /// `xl/worksheets/sheetN.xml` → attributes to add to `<pageSetup`.
    pub page_setup: Vec<(String, String)>,
    /// `<workbookProtection …/>` to insert into `xl/workbook.xml`.
    pub workbook_protection: Option<String>,
    /// Set `date1904="1"`.
    pub date1904: bool,
    /// New parts `(path, xml)` (wave 5: pivot caches and tables).
    pub extra_parts: Vec<(String, String)>,
    /// `[Content_Types].xml` overrides `(part name, content type)`.
    pub content_types: Vec<(String, String)>,
    /// Relationships to add: rels path → `(Id, Type, Target)` (the file is
    /// created when the package has none).
    pub rels: Vec<(String, Vec<RelEntry>)>,
    /// XML inserted into `xl/workbook.xml` before `</workbook>` (wrapped in
    /// `<pivotCaches>` when it holds pivot caches).
    pub workbook_inserts: Vec<String>,
    /// `<definedName>` elements added to `xl/workbook.xml` (AutoFilter
    /// `_xlnm._FilterDatabase` names).
    pub defined_names: Vec<String>,
    /// `xl/worksheets/sheetN.xml` → XML inserted at the `scenarios`
    /// position (after `sheetProtection`).
    pub sheet_inserts: Vec<(String, String)>,
    /// Named cell styles to add to `xl/styles.xml`.
    pub style_names: Vec<String>,
    /// `xl/worksheets/sheetN.xml` → attributes to add to `<sheetView`
    /// (view options rust_xlsxwriter has no setter for).
    pub sheet_view_attrs: Vec<(String, String)>,
    /// `workbookView showSheetTabs="0"` (Tools ▸ Options ▸ View ▸ Sheet tabs off).
    pub hide_sheet_tabs: bool,
    /// Literal chart data (`c:numLit` / `c:strLit`, which rust_xlsxwriter
    /// cannot write): `(marker, xml)`, the marker being the formula of a
    /// placeholder reference in `xl/charts/chartN.xml`.
    pub chart_literals: Vec<String>,
}

impl Patches {
    pub fn is_empty(&self) -> bool {
        self.sheet_protection.is_empty()
            && self.page_setup.is_empty()
            && self.workbook_protection.is_none()
            && !self.date1904
            && self.extra_parts.is_empty()
            && self.content_types.is_empty()
            && self.rels.is_empty()
            && self.workbook_inserts.is_empty()
            && self.defined_names.is_empty()
            && self.sheet_inserts.is_empty()
            && self.style_names.is_empty()
            && self.sheet_view_attrs.is_empty()
            && !self.hide_sheet_tabs
            && self.chart_literals.is_empty()
    }
}

/// The elements that may follow `scenarios` in `CT_Worksheet`, in schema
/// order; a scenarios block is inserted before the first one present.
const AFTER_SCENARIOS: &[&str] = &[
    "<autoFilter",
    "<sortState",
    "<dataConsolidate",
    "<customSheetViews",
    "<mergeCells",
    "<phoneticPr",
    "<conditionalFormatting",
    "<dataValidations",
    "<hyperlinks",
    "<printOptions",
    "<pageMargins",
    "<pageSetup",
    "<headerFooter",
    "<rowBreaks",
    "<colBreaks",
    "<customProperties",
    "<cellWatches",
    "<ignoredErrors",
    "<smartTags",
    "<drawing",
    "<legacyDrawing",
    "<tableParts",
    "<extLst",
    "</worksheet>",
];

fn rels_xml(entries: &[RelEntry]) -> String {
    let mut s = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
    );
    for (id, ty, target) in entries {
        s.push_str(&format!(
            r#"<Relationship Id="{}" Type="{}" Target="{}"/>"#,
            xml_attr(id),
            xml_attr(ty),
            xml_attr(target)
        ));
    }
    s.push_str("</Relationships>");
    s
}

fn xml_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The `sheetProtection` / `workbookProtection` password attributes of a
/// stored hash (`None` when there is no password or no OOXML form).
pub(crate) fn password_attrs(p: &SheetProtectionData, legacy_name: &str) -> Option<String> {
    let hash = p.password_hash.as_deref().filter(|h| !h.is_empty())?;
    match p.algorithm.as_deref() {
        Some(hash_algorithms::XLSX_LEGACY) => {
            Some(format!(r#"{legacy_name}="{}""#, xml_attr(hash)))
        }
        Some(alg) if alg.starts_with("SHA-") || alg.starts_with("SHA") && !alg.contains(':') => {
            let mut s = format!(
                r#"algorithmName="{}" hashValue="{}""#,
                xml_attr(alg),
                xml_attr(hash)
            );
            if let Some(salt) = &p.salt {
                s.push_str(&format!(r#" saltValue="{}""#, xml_attr(salt)));
            }
            if let Some(n) = p.spin_count {
                s.push_str(&format!(r#" spinCount="{n}""#));
            }
            Some(s)
        }
        // ODF hashes (SHA-256 of UTF-8) have no OOXML form; use the
        // legacy hash recorded next to them.
        _ => p
            .xlsx_legacy_hash
            .as_deref()
            .map(|h| format!(r#"{legacy_name}="{}""#, xml_attr(h))),
    }
}

/// The patches for a workbook (protection hashes).
pub(crate) fn patches(data: &WorkbookData, report: &mut dyn Warn) -> Patches {
    let mut out = Patches::default();
    for (i, s) in data.sheets.iter().enumerate() {
        if let Some(n) = s.page_setup.as_ref().and_then(|p| p.first_page_number) {
            out.page_setup.push((
                format!("xl/worksheets/sheet{}.xml", i + 1),
                format!(
                    r#"firstPageNumber="{}" useFirstPageNumber="1""#,
                    n.min(32767)
                ),
            ));
        }
        let Some(p) = &s.protection else { continue };
        match password_attrs(p, "password") {
            Some(a) => out
                .sheet_protection
                .push((format!("xl/worksheets/sheet{}.xml", i + 1), a)),
            None if p.password_hash.is_some() => report.warn(format!(
                "sheet {:?}: its protection password cannot be stored in XLSX; it was removed",
                s.name
            )),
            None => {}
        }
    }
    if let Some(p) = &data.settings.structure_protection {
        let pw = password_attrs(p, "workbookPassword")
            .map(|a| {
                // Workbook protection uses `workbook`-prefixed hash names.
                a.replace("algorithmName=", "workbookAlgorithmName=")
                    .replace("hashValue=", "workbookHashValue=")
                    .replace("saltValue=", "workbookSaltValue=")
                    .replace("spinCount=", "workbookSpinCount=")
            })
            .unwrap_or_default();
        out.workbook_protection = Some(format!(r#"<workbookProtection {pw} lockStructure="1"/>"#));
    }
    out
}

/// Applies `patches` to a finished package.
pub(crate) fn apply_patches(bytes: &[u8], p: &Patches) -> Result<Vec<u8>, XlsxError> {
    use std::io::{Cursor, Read, Write};
    let mut src = zip::ZipArchive::new(Cursor::new(bytes))?;
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::with_capacity(bytes.len())));
    let mut handled_rels: Vec<String> = Vec::new();
    for i in 0..src.len() {
        let mut file = src.by_index(i)?;
        let name = file.name()?.into_owned();
        let sheet_attrs: Vec<&str> = p
            .sheet_protection
            .iter()
            .filter(|(n, _)| *n == name)
            .map(|(_, a)| a.as_str())
            .collect();
        let page_attrs: Vec<&str> = p
            .page_setup
            .iter()
            .filter(|(n, _)| *n == name)
            .map(|(_, a)| a.as_str())
            .collect();
        let is_wb = name == "xl/workbook.xml"
            && (p.date1904
                || p.hide_sheet_tabs
                || p.workbook_protection.is_some()
                || !p.workbook_inserts.is_empty()
                || !p.defined_names.is_empty());
        let sheet_ins: Vec<&str> = p
            .sheet_inserts
            .iter()
            .filter(|(n, _)| *n == name)
            .map(|(_, x)| x.as_str())
            .collect();
        let rel_adds: Vec<&RelEntry> = p
            .rels
            .iter()
            .filter(|(n, _)| *n == name)
            .flat_map(|(_, v)| v.iter())
            .collect();
        let view_attrs: Vec<&str> = p
            .sheet_view_attrs
            .iter()
            .filter(|(n, _)| *n == name)
            .map(|(_, a)| a.as_str())
            .collect();
        let is_ct = name == "[Content_Types].xml" && !p.content_types.is_empty();
        let is_styles = name == "xl/styles.xml" && !p.style_names.is_empty();
        let is_chart = name.starts_with("xl/charts/chart") && !p.chart_literals.is_empty();
        if !rel_adds.is_empty() {
            handled_rels.push(name.clone());
        }
        if !sheet_attrs.is_empty()
            || !page_attrs.is_empty()
            || is_wb
            || !sheet_ins.is_empty()
            || !view_attrs.is_empty()
            || !rel_adds.is_empty()
            || is_ct
            || is_styles
            || is_chart
        {
            let mut xml = String::new();
            file.read_to_string(&mut xml)?;
            drop(file);
            let mut patched = xml;
            for a in sheet_attrs {
                patched =
                    patched.replacen("<sheetProtection ", &format!("<sheetProtection {a} "), 1);
            }
            for a in page_attrs {
                patched = if patched.contains("<pageSetup ") {
                    patched.replacen("<pageSetup ", &format!("<pageSetup {a} "), 1)
                } else {
                    // CT_Worksheet order: … pageMargins, pageSetup, headerFooter …
                    let el = format!("<pageSetup {a}/>");
                    match patched.find("<pageMargins") {
                        Some(i) => {
                            let end = patched[i..].find("/>").map_or(patched.len(), |j| i + j + 2);
                            let mut x = patched.clone();
                            x.insert_str(end, &el);
                            x
                        }
                        None => patched,
                    }
                };
            }
            if is_wb {
                if p.hide_sheet_tabs && !patched.contains("showSheetTabs") {
                    patched =
                        patched.replacen("<workbookView", "<workbookView showSheetTabs=\"0\"", 1);
                }
                if p.date1904 && !patched.contains("date1904") {
                    patched = if patched.contains("<workbookPr") {
                        patched.replacen("<workbookPr", "<workbookPr date1904=\"1\"", 1)
                    } else {
                        patched.replacen("<bookViews", "<workbookPr date1904=\"1\"/><bookViews", 1)
                    };
                }
                if let Some(wp) = &p.workbook_protection {
                    // CT_Workbook order: … workbookPr, workbookProtection, bookViews …
                    patched = patched.replacen("<bookViews", &format!("{wp}<bookViews"), 1);
                }
                if !p.defined_names.is_empty() {
                    // CT_Workbook order: … sheets, …, definedNames, calcPr …
                    let names = p.defined_names.concat();
                    patched = if patched.contains("</definedNames>") {
                        patched.replacen("</definedNames>", &format!("{names}</definedNames>"), 1)
                    } else {
                        let block = format!("<definedNames>{names}</definedNames>");
                        let at = [
                            "<calcPr",
                            "<oleSize",
                            "<customWorkbookViews",
                            "<pivotCaches",
                            "</workbook>",
                        ]
                        .iter()
                        .find_map(|m| patched.find(m))
                        .unwrap_or(patched.len());
                        let mut x = patched;
                        x.insert_str(at, &block);
                        x
                    };
                }
                if !p.workbook_inserts.is_empty() {
                    let block =
                        format!("<pivotCaches>{}</pivotCaches>", p.workbook_inserts.concat());
                    patched = patched.replacen("</workbook>", &format!("{block}</workbook>"), 1);
                }
            }
            for a in &view_attrs {
                patched = patched.replacen("<sheetView ", &format!("<sheetView {a} "), 1);
            }
            for x in &sheet_ins {
                let at = AFTER_SCENARIOS
                    .iter()
                    .filter_map(|m| patched.find(m))
                    .min()
                    .unwrap_or(patched.len());
                patched.insert_str(at, x);
            }
            if !rel_adds.is_empty() {
                let add: String = rel_adds
                    .iter()
                    .map(|(id, ty, target)| {
                        format!(
                            r#"<Relationship Id="{}" Type="{}" Target="{}"/>"#,
                            xml_attr(id),
                            xml_attr(ty),
                            xml_attr(target)
                        )
                    })
                    .collect();
                patched =
                    patched.replacen("</Relationships>", &format!("{add}</Relationships>"), 1);
            }
            if is_ct {
                let add: String = p
                    .content_types
                    .iter()
                    .map(|(part, ct)| {
                        format!(
                            r#"<Override PartName="{}" ContentType="{}"/>"#,
                            xml_attr(part),
                            xml_attr(ct)
                        )
                    })
                    .collect();
                patched = patched.replacen("</Types>", &format!("{add}</Types>"), 1);
            }
            if is_styles {
                patched = crate::tools::patch_styles_xml(&patched, &p.style_names);
            }
            if is_chart {
                patched = crate::chart_write::patch_chart_literals(&patched, &p.chart_literals);
            }
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            out.start_file(name, opts)?;
            out.write_all(patched.as_bytes())?;
        } else {
            out.raw_copy_file(file)?;
        }
    }
    // New parts and relationship files the package did not have.
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (path, xml) in &p.extra_parts {
        out.start_file(path.as_str(), opts)?;
        out.write_all(xml.as_bytes())?;
    }
    let mut created: Vec<String> = Vec::new();
    for (path, _) in &p.rels {
        if handled_rels.contains(path) || created.contains(path) {
            continue;
        }
        let entries: Vec<RelEntry> = p
            .rels
            .iter()
            .filter(|(n, _)| n == path)
            .flat_map(|(_, v)| v.iter().cloned())
            .collect();
        out.start_file(path.as_str(), opts)?;
        out.write_all(rels_xml(&entries).as_bytes())?;
        created.push(path.clone());
    }
    Ok(out.finish()?.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_codes_and_refs() {
        let h = HeaderFooterData {
            left: "&[FILE] & co".into(),
            center: "Page &[PAGE] of &[PAGES]".into(),
            right: String::new(),
        };
        assert_eq!(hf_to_excel(&h), "&L&F && co&CPage &P of &N");
        assert_eq!(
            sqref(&[
                CellRangeLocal {
                    r0: 0,
                    c0: 0,
                    r1: 9,
                    c1: 1
                },
                CellRangeLocal {
                    r0: 3,
                    c0: 3,
                    r1: 3,
                    c1: 3
                }
            ]),
            "A1:B10 D4"
        );
        assert!(excel_url("javascript:x").is_none());
        assert!(excel_url("#Sheet2.B3").is_some());
        let legacy = SheetProtectionData {
            password_hash: Some("83AF".into()),
            algorithm: Some(hash_algorithms::XLSX_LEGACY.into()),
            ..SheetProtectionData::default()
        };
        assert_eq!(
            password_attrs(&legacy, "password").as_deref(),
            Some(r#"password="83AF""#)
        );
        let odf = SheetProtectionData {
            password_hash: Some("x".into()),
            algorithm: Some(hash_algorithms::SHA256.into()),
            ..SheetProtectionData::default()
        };
        assert_eq!(password_attrs(&odf, "password"), None);
    }
}
