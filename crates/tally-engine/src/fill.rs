//! Fill handle and Fill Series (inventory §4, AOO `ScTable::FillAnalyse`).
//!
//! The target extends the source in exactly one direction. Each lane (a
//! column when filling down/up, a row when filling right/left) is filled
//! on its own from the source cells read *away* from the edge being
//! extended, so filling up or left continues the series backwards
//! (1, 2 filled up gives 0, −1). Styles repeat with the source pattern.
//!
//! `Auto` detection per lane (AOO `ScTable::FillAnalyse` / `FillAuto`).
//! "+1" below is −1 when filling up or left (AOO negates the increment
//! when `!bPositive`), so a single 5 dragged up gives 4, 3, 2:
//! - numbers: one cell steps by +1; several with a constant difference
//!   form a linear series;
//! - date-formatted numbers: one cell steps one day; several step by
//!   months (years) when the day of month is constant and the month
//!   difference is, else by a constant day difference;
//! - entries of an AutoFill list (AOO `ScUserList`: the locale's and
//!   en-US day and month names, abbreviated first, then the Options sort
//!   lists) step through their list, wrapping and keeping the case style
//!   (AOO writes the list's own case: a deliberate difference);
//! - text with a leading or trailing integer ("Item 1", "Q3", "1st",
//!   "001x"; AOO `lcl_DecompValueString`) increments it, zero-padding
//!   when the number has a leading zero; English ordinal suffixes follow
//!   the number ("1st" → "2nd", "11th", "21st"; AOO
//!   `ScGlobal::GetOrdinalSuffix`); several with a constant difference
//!   form a linear series;
//! - anything else repeats the pattern, each repetition adding +1 to
//!   numbers and numbered text (AOO's `FILL_SIMPLE`); plain text is
//!   copied and formulas are relative, so their references follow the
//!   new position.
//!
//! The copy modifier and filtered data (AOO `ScTable::FillAnalyse` /
//! `FillAuto`, `nScFillModeMouseModifier & KEY_MOD1` and i89232's
//! `IsDataFiltered`): a fill-handle drag with ⌘ (`AutoCopy`) copies the
//! source for every content type, single numbers and dates included. On
//! a sheet whose data is filtered, `Auto` and `AutoCopy` both copy and
//! skip the filtered rows: nothing is written to them and the source
//! pattern advances only over the rows that are written (filling right
//! or left, a filtered row's lane is left alone). ⌘D / ⌘R (`Copy`, AOO
//! `FillSeries` with `FILL_SIMPLE`) and the linear, growth and date
//! Fill Series types ignore the filter and write every target cell (the
//! Series AutoFill type is `Auto`, as AOO routes it to `FillAuto`).

use std::sync::Arc;

use tally_api::{ApiError, FillMode};
use tally_numfmt::date::{days_in_month, serial_from_ymd, ymd_from_serial};
use tally_numfmt::EN_US;
use tally_types::{approx_equal, CellAddr, CellRange, DateEpoch, Value};

use crate::ops::{array_block_error, MAX_OP_CELLS};
use crate::storage::{CellContent, StyleId};
use crate::undo::{capture_range, Change};
use crate::workbook::{SnapContent, Workbook};

/// Fill direction.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Dir {
    Down,
    Up,
    Right,
    Left,
}

/// One source cell.
#[derive(Clone, Debug)]
struct Src {
    content: Option<SnapContent>,
    value: Value,
    style: StyleId,
    is_date: bool,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Case {
    Upper,
    Lower,
    Title,
}

fn case_of(s: &str) -> Case {
    if s.chars().all(|c| !c.is_lowercase()) {
        Case::Upper
    } else if s.chars().next().is_some_and(char::is_uppercase) {
        Case::Title
    } else {
        Case::Lower
    }
}

fn apply_case(s: &str, case: Case) -> String {
    match case {
        Case::Upper => s.to_uppercase(),
        Case::Lower => s.to_lowercase(),
        Case::Title => s.to_owned(),
    }
}

/// The AutoFill lists, in AOO `ScUserList` order: the document locale's
/// abbreviated and full day names, abbreviated and full month names, then
/// the Options sort lists (`custom_sort_lists`). The en-US names follow the
/// locale's when the locale differs (Tally keeps English names filling in
/// every locale; AOO only has the UI language's).
fn fill_lists(wb: &Workbook) -> Vec<Vec<String>> {
    let mut out: Vec<Vec<String>> = Vec::new();
    let mut add = |l: Vec<String>| {
        if !l.is_empty() && !out.contains(&l) {
            out.push(l);
        }
    };
    let own = |l: &[&str]| l.iter().map(|x| (*x).to_owned()).collect::<Vec<_>>();
    for loc in [wb.locale, &EN_US] {
        add(own(&loc.short_day_names));
        add(own(&loc.day_names));
        add(own(&loc.short_month_names));
        add(own(&loc.month_names));
    }
    for l in &wb.custom_sort_lists {
        add(l.clone());
    }
    out
}

/// `(list index, position)` of `s` in the first list containing it
/// (case-insensitive, AOO `ScUserList::GetData`).
fn list_pos(lists: &[Vec<String>], s: &str) -> Option<(usize, usize)> {
    let t = s.trim().to_lowercase();
    lists.iter().enumerate().find_map(|(li, list)| {
        list.iter()
            .position(|x| x.to_lowercase() == t)
            .map(|p| (li, p))
    })
}

/// Text with a number at its start (`head`) or end (AOO
/// `lcl_DecompValueString`).
#[derive(Clone, Debug, PartialEq, Eq)]
struct NumText {
    head: bool,
    value: i64,
    /// Zero-padded width: the digit count when the number has a leading
    /// zero, else 0.
    digits: usize,
    /// The text without the number.
    rest: String,
}

/// Decomposes `s` like AOO: a leading number (with an optional `-` sign)
/// wins when it is followed by a space or the end, or when the text does
/// not end in a digit; otherwise a trailing number. Unlike AOO, a `-`
/// before a trailing number is not a sign ("Item-5" fills "Item-6", not
/// "Item-4"; "2024-05" fills "2024-06").
fn numbered(s: &str) -> Option<NumText> {
    const MAX_DIGITS: usize = 15;
    let b = s.as_bytes();
    let last = *b.last()?;
    let neg = usize::from(b[0] == b'-');
    let mut n = neg;
    while n < b.len() && b[n].is_ascii_digit() {
        n += 1;
    }
    let next = b.get(n).copied();
    if n > neg && (next.is_none() || next == Some(b' ') || !last.is_ascii_digit()) {
        let len = n - neg;
        if len > MAX_DIGITS {
            return None;
        }
        return Some(NumText {
            head: true,
            value: s[..n].parse().ok()?,
            digits: if b[neg] == b'0' { len } else { 0 },
            rest: s[n..].to_owned(),
        });
    }
    let mut i = b.len();
    while i > 0 && b[i - 1].is_ascii_digit() {
        i -= 1;
    }
    let len = b.len() - i;
    if len == 0 || len > MAX_DIGITS {
        return None;
    }
    Some(NumText {
        head: false,
        value: s[i..].parse().ok()?,
        digits: if b[i] == b'0' { len } else { 0 },
        rest: s[..i].to_owned(),
    })
}

/// The English ordinal suffix of `n` (`st`, `nd`, `rd`, `th`).
fn ordinal_suffix(n: i64) -> &'static str {
    let a = n.unsigned_abs();
    if (11..=13).contains(&(a % 100)) {
        return "th";
    }
    match a % 10 {
        1 => "st",
        2 => "nd",
        3 => "rd",
        _ => "th",
    }
}

/// Whether `t`'s suffix is the ordinal suffix of its leading number
/// ("1st", "22ND"): `Some(upper case)`, else `None`. AOO compares with
/// the lower-case suffix only; upper case is accepted here too.
fn ordinal_case(t: &NumText) -> Option<bool> {
    if !t.head {
        return None;
    }
    let suf = ordinal_suffix(t.value);
    if t.rest == suf {
        Some(false)
    } else if t.rest == suf.to_ascii_uppercase() {
        Some(true)
    } else {
        None
    }
}

/// `t` with its number replaced by `value` (AOO `lcl_ValueString` /
/// `lcl_getSuffixCell`); an ordinal suffix follows the new number.
fn renumber(t: &NumText, value: i64) -> String {
    let body = if t.digits <= 1 {
        value.to_string()
    } else {
        let mut d = format!("{:0w$}", value.unsigned_abs(), w = t.digits);
        if value < 0 {
            d.insert(0, '-');
        }
        d
    };
    if t.head {
        let rest = match ordinal_case(t) {
            Some(false) => ordinal_suffix(value).to_owned(),
            Some(true) => ordinal_suffix(value).to_ascii_uppercase(),
            None => t.rest.clone(),
        };
        format!("{body}{rest}")
    } else {
        format!("{}{body}", t.rest)
    }
}

fn number(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => Some(*n),
        _ => None,
    }
}

fn constant_step(xs: &[f64]) -> Option<f64> {
    let d = xs.get(1)? - xs.first()?;
    xs.windows(2)
        .all(|w| approx_equal(w[1] - w[0], d))
        .then_some(d)
}

/// Adds `months` to a date serial, clamping the day to the month's end.
fn add_months(epoch: DateEpoch, serial: f64, months: i64) -> Option<f64> {
    let (y, m, d) = ymd_from_serial(epoch, serial)?;
    let total = i64::from(y) * 12 + i64::from(m) - 1 + months;
    let ny = total.div_euclid(12);
    let nm = (total.rem_euclid(12) + 1) as u32;
    let nd = d.min(days_in_month(ny, nm));
    let base = serial_from_ymd(epoch, i32::try_from(ny).ok()?, nm, nd)?;
    Some(base + serial.fract())
}

/// What one lane produces at fill position `k` (0-based beyond the source).
enum Plan {
    Copy,
    /// Repeat the pattern, adding `(k / n + 1) * delta` to numbers.
    CopyDelta(f64),
    Linear(f64),
    Growth(f64),
    Months(i64),
    List {
        list: Vec<String>,
        start: usize,
        step: i64,
        case: Case,
    },
    /// Numbered text: `last` (the source cell nearest the fill) renumbered
    /// by `step` per cell.
    Numbered {
        last: NumText,
        step: i64,
    },
}

/// The plan for one lane; `sign` is −1 when filling up or left (the
/// default step of a single cell, and the pattern delta, count down).
fn plan_lane(
    seq: &[Src],
    mode: FillMode,
    epoch: DateEpoch,
    sign: f64,
    lists: &[Vec<String>],
) -> Plan {
    let n = seq.len();
    let all_numbers: Option<Vec<f64>> = seq
        .iter()
        .map(|s| match (&s.content, &s.value) {
            (Some(SnapContent::Value(Value::Number(x))), _) => Some(*x),
            _ => None,
        })
        .collect();
    let all_dates = all_numbers.is_some() && seq.iter().all(|s| s.is_date);
    match mode {
        FillMode::Copy | FillMode::AutoCopy => return Plan::Copy,
        FillMode::LinearSeries => {
            return match &all_numbers {
                Some(xs) if n >= 2 => Plan::Linear(xs[n - 1] - xs[n - 2]),
                Some(_) => Plan::Linear(sign),
                None => Plan::Copy,
            }
        }
        FillMode::GrowthSeries => {
            return match &all_numbers {
                Some(xs) if n >= 2 && xs[n - 2] != 0.0 => Plan::Growth(xs[n - 1] / xs[n - 2]),
                Some(_) => Plan::Growth(1.0),
                None => Plan::Copy,
            }
        }
        FillMode::Date => {
            return match &all_numbers {
                Some(xs) => date_plan(xs, epoch).unwrap_or(Plan::Linear(sign)),
                None => Plan::Copy,
            }
        }
        FillMode::Auto => {}
    }
    if let Some(xs) = &all_numbers {
        if n == 1 {
            return Plan::Linear(sign);
        }
        if all_dates {
            return date_plan(xs, epoch).unwrap_or(Plan::CopyDelta(sign));
        }
        return match constant_step(xs) {
            Some(d) => Plan::Linear(d),
            None => Plan::CopyDelta(sign),
        };
    }
    let texts: Option<Vec<&str>> = seq
        .iter()
        .map(|s| match &s.content {
            Some(SnapContent::Value(Value::Text(t))) => Some(&**t),
            _ => None,
        })
        .collect();
    let Some(texts) = texts else {
        return Plan::CopyDelta(sign);
    };
    // Day / month names and sort lists.
    let pos: Option<Vec<(usize, usize)>> = texts.iter().map(|t| list_pos(lists, t)).collect();
    if let Some(pos) = pos {
        let list = pos[0].0;
        if pos.iter().all(|p| p.0 == list) {
            let len = lists[list].len() as i64;
            let step = if n == 1 {
                sign as i64
            } else {
                let d = (pos[1].1 as i64 - pos[0].1 as i64).rem_euclid(len);
                if pos
                    .windows(2)
                    .all(|w| (w[1].1 as i64 - w[0].1 as i64).rem_euclid(len) == d)
                {
                    d
                } else {
                    return Plan::CopyDelta(sign);
                }
            };
            return Plan::List {
                list: lists[list].clone(),
                start: pos[n - 1].1,
                step,
                case: case_of(texts[n - 1]),
            };
        }
    }
    // Numbered text: AOO only needs the same number position (start or
    // end) and a constant difference; the rest comes from the last cell
    // and the widest zero-padded number sets the width.
    let nums: Option<Vec<NumText>> = texts.iter().map(|t| numbered(t)).collect();
    if let Some(nums) = nums {
        if n == 1 {
            return Plan::Numbered {
                last: nums[0].clone(),
                step: sign as i64,
            };
        }
        let head = nums[0].head;
        let d = nums[1].value - nums[0].value;
        if nums.iter().all(|x| x.head == head)
            && nums.windows(2).all(|w| w[1].value - w[0].value == d)
        {
            let mut last = nums[n - 1].clone();
            last.digits = nums.iter().map(|x| x.digits).max().unwrap_or(0);
            return Plan::Numbered { last, step: d };
        }
    }
    Plan::CopyDelta(sign)
}

fn date_plan(xs: &[f64], epoch: DateEpoch) -> Option<Plan> {
    if xs.len() < 2 {
        return Some(Plan::Linear(1.0));
    }
    let ymd: Option<Vec<(i32, u32, u32)>> = xs.iter().map(|x| ymd_from_serial(epoch, *x)).collect();
    if let Some(ymd) = ymd {
        let day = ymd[0].2;
        if ymd.iter().all(|d| d.2 == day) {
            let months: Vec<i64> = ymd
                .iter()
                .map(|(y, m, _)| i64::from(*y) * 12 + i64::from(*m))
                .collect();
            let d = months[1] - months[0];
            if d != 0 && months.windows(2).all(|w| w[1] - w[0] == d) {
                return Some(Plan::Months(d));
            }
        }
    }
    constant_step(xs).map(Plan::Linear)
}

fn produce(plan: &Plan, seq: &[Src], k: usize, epoch: DateEpoch) -> Option<SnapContent> {
    let n = seq.len();
    let last_num = || number(&seq[n - 1].value).unwrap_or(0.0);
    let kk = (k + 1) as f64;
    let num = |x: f64| {
        Some(SnapContent::Value(if x.is_finite() {
            Value::Number(x)
        } else {
            Value::Error(tally_types::ErrorKind::Num)
        }))
    };
    match plan {
        Plan::Copy => seq[k % n].content.clone(),
        Plan::CopyDelta(d) => {
            let src = &seq[k % n];
            let delta = (k / n + 1) as f64 * d;
            match &src.content {
                Some(SnapContent::Value(Value::Number(x))) => num(x + delta),
                Some(SnapContent::Value(Value::Text(t))) => match numbered(t) {
                    Some(nt) => Some(SnapContent::Value(Value::text(renumber(
                        &nt,
                        nt.value.saturating_add(delta as i64),
                    )))),
                    None => src.content.clone(),
                },
                other => other.clone(),
            }
        }
        Plan::Linear(d) => num(last_num() + kk * d),
        Plan::Growth(r) => num(last_num() * r.powf(kk)),
        Plan::Months(m) => match add_months(epoch, last_num(), m * (k as i64 + 1)) {
            Some(x) => num(x),
            None => num(f64::NAN),
        },
        Plan::List {
            list,
            start,
            step,
            case,
        } => {
            let i = (*start as i64 + step * (k as i64 + 1)).rem_euclid(list.len() as i64) as usize;
            Some(SnapContent::Value(Value::text(apply_case(&list[i], *case))))
        }
        Plan::Numbered { last, step } => {
            let v = last.value.saturating_add(step.saturating_mul(k as i64 + 1));
            Some(SnapContent::Value(Value::text(renumber(last, v))))
        }
    }
}

fn direction(source: &CellRange, target: &CellRange) -> Result<Option<Dir>, ApiError> {
    if source == target {
        return Ok(None);
    }
    let same_cols = source.c0 == target.c0 && source.c1 == target.c1;
    let same_rows = source.r0 == target.r0 && source.r1 == target.r1;
    let dir = if same_cols && target.r0 == source.r0 && target.r1 > source.r1 {
        Dir::Down
    } else if same_cols && target.r1 == source.r1 && target.r0 < source.r0 {
        Dir::Up
    } else if same_rows && target.c0 == source.c0 && target.c1 > source.c1 {
        Dir::Right
    } else if same_rows && target.c1 == source.c1 && target.c0 < source.c0 {
        Dir::Left
    } else {
        return Err(ApiError::invalid_argument(
            "the fill target must extend the source in one direction",
        ));
    };
    Ok(Some(dir))
}

/// AOO `ScTable::IsDataFiltered`: the sheet's filter has a criterion (an
/// AutoFilter with a filtered column, or a standard / advanced filter).
/// Sheet-wide, like AOO: the fill need not touch the filtered area.
fn data_filtered(wb: &Workbook, sheet: tally_types::SheetId) -> bool {
    wb.sheet(sheet).is_some_and(|s| {
        s.meta
            .auto_filter
            .as_ref()
            .is_some_and(|af| !af.columns.is_empty())
            || s.meta.standard_filter.is_some()
    })
}

/// Fills `target` (which contains `source`) from `source`.
pub fn fill_range(
    wb: &mut Workbook,
    source: CellRange,
    target: CellRange,
    mode: FillMode,
) -> Result<Vec<Change>, ApiError> {
    let Some(dir) = direction(&source, &target)? else {
        return Ok(Vec::new());
    };
    if target.cell_count() > MAX_OP_CELLS {
        return Err(ApiError::invalid_argument("the fill area is too large"));
    }
    let sheet = source.sheet;
    let area = match dir {
        Dir::Down => CellRange::new(sheet, source.r1 + 1, source.c0, target.r1, source.c1),
        Dir::Up => CellRange::new(sheet, target.r0, source.c0, source.r0 - 1, source.c1),
        Dir::Right => CellRange::new(sheet, source.r0, source.c1 + 1, source.r1, target.c1),
        Dir::Left => CellRange::new(sheet, source.r0, target.c0, source.r1, source.c0 - 1),
    };
    if !wb.array_blocks_in(&area).is_empty() || !wb.array_blocks_in(&source).is_empty() {
        return Err(array_block_error());
    }
    // Filtered data: AutoFill copies and leaves filtered rows alone.
    let skip = if matches!(mode, FillMode::Auto | FillMode::AutoCopy) && data_filtered(wb, sheet) {
        wb.sheet(sheet).map(|s| s.filtered_rows.clone())
    } else {
        None
    };
    let mode = if skip.is_some() || mode == FillMode::AutoCopy {
        FillMode::Copy
    } else {
        mode
    };
    let filtered = |row: u32| skip.as_ref().is_some_and(|f| f.contains(row));
    let epoch = wb.settings.date_epoch;
    let lists = fill_lists(wb);
    let inv = capture_range(wb, area, true, true);
    let vertical = matches!(dir, Dir::Down | Dir::Up);
    let lanes = if vertical {
        source.c0..=source.c1
    } else {
        source.r0..=source.r1
    };
    for lane in lanes {
        if !vertical && filtered(lane) {
            continue;
        }
        // Source cells ordered away from the edge being extended.
        let mut idx: Vec<u32> = if vertical {
            (source.r0..=source.r1).collect()
        } else {
            (source.c0..=source.c1).collect()
        };
        if matches!(dir, Dir::Up | Dir::Left) {
            idx.reverse();
        }
        let at = |i: u32| {
            if vertical {
                CellAddr::new(sheet, i, lane)
            } else {
                CellAddr::new(sheet, lane, i)
            }
        };
        let seq: Vec<Src> = idx
            .iter()
            .map(|&i| {
                let a = at(i);
                let style = wb.style_at(a);
                let content = match wb.sheet(sheet).and_then(|s| s.get(a.row, a.col)) {
                    Some(CellContent::ArrayPart(_)) => Some(SnapContent::Value(wb.value(a))),
                    _ => wb.snapshot_content(a),
                };
                Src {
                    content,
                    value: wb.value(a),
                    style,
                    is_date: wb.styles.get(style).format.is_date(),
                }
            })
            .collect();
        let sign = if matches!(dir, Dir::Up | Dir::Left) {
            -1.0
        } else {
            1.0
        };
        let plan = plan_lane(&seq, mode, epoch, sign, &lists);
        let positions: Vec<u32> = match dir {
            Dir::Down => (source.r1 + 1..=target.r1).collect(),
            Dir::Up => (target.r0..source.r0).rev().collect(),
            Dir::Right => (source.c1 + 1..=target.c1).collect(),
            Dir::Left => (target.c0..source.c0).rev().collect(),
        };
        let positions = positions
            .into_iter()
            .filter(|&p| !(vertical && filtered(p)));
        for (k, p) in positions.enumerate() {
            let a = at(p);
            let content = produce(&plan, &seq, k, epoch);
            wb.restore_content(
                a,
                content.map(|c| match c {
                    SnapContent::Formula {
                        formula,
                        invalid_text,
                        array: _,
                    } => SnapContent::Formula {
                        formula: Arc::clone(&formula),
                        invalid_text,
                        array: None,
                    },
                    v => v,
                }),
            );
            let st = seq[k % seq.len()].style;
            if let Some(col) = wb.sheet_mut(sheet).and_then(|s| s.column_mut(a.col)) {
                col.styles.set(a.row, a.row, st);
            }
        }
    }
    wb.pending.ranges.push(area);
    wb.pending.display_only.push(area);
    Ok(vec![Change::Range(inv)])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers() {
        let mut wb = Workbook::new();
        wb.custom_sort_lists = vec![vec!["North".into(), "South".into()]];
        let lists = fill_lists(&wb);
        assert_eq!(list_pos(&lists, "mon"), Some((0, 1)));
        assert_eq!(list_pos(&lists, "Monday"), Some((1, 1)));
        // "May" is in the abbreviated list first (AOO order).
        assert_eq!(list_pos(&lists, "May"), Some((2, 4)));
        assert_eq!(list_pos(&lists, "December"), Some((3, 11)));
        assert_eq!(list_pos(&lists, "south"), Some((4, 1)));
        // A German document fills German names too.
        wb.locale = &tally_numfmt::DE_DE;
        let lists = fill_lists(&wb);
        let (li, p) = list_pos(&lists, tally_numfmt::DE_DE.day_names[1]).unwrap();
        assert_eq!(lists[li][p], tally_numfmt::DE_DE.day_names[1]);
        assert!(list_pos(&lists, "Monday").is_some());
        let nt = |head, value, digits, rest: &str| {
            Some(NumText {
                head,
                value,
                digits,
                rest: rest.into(),
            })
        };
        assert_eq!(numbered("Item 9"), nt(false, 9, 0, "Item "));
        assert_eq!(numbered("007x"), nt(true, 7, 3, "x"));
        assert_eq!(numbered("1st"), nt(true, 1, 0, "st"));
        // A leading number followed by a space wins (AOO #i5550).
        assert_eq!(numbered("1 to 5"), nt(true, 1, 0, " to 5"));
        // Otherwise the trailing one (IP addresses).
        assert_eq!(numbered("10.0.0.1"), nt(false, 1, 0, "10.0.0."));
        assert_eq!(numbered("-3 dB"), nt(true, -3, 0, " dB"));
        assert_eq!(numbered("Item-5"), nt(false, 5, 0, "Item-"));
        assert_eq!(numbered("abc"), None);
        assert_eq!(numbered("-"), None);
        assert_eq!(numbered(""), None);
        let ords: Vec<String> = [1, 2, 3, 4, 11, 12, 13, 21, 22, 23, 101, 111, 112, -1]
            .iter()
            .map(|&v| format!("{v}{}", ordinal_suffix(v)))
            .collect();
        assert_eq!(
            ords,
            [
                "1st", "2nd", "3rd", "4th", "11th", "12th", "13th", "21st", "22nd", "23rd",
                "101st", "111th", "112th", "-1st"
            ]
        );
        let first = numbered("1ST").unwrap();
        assert_eq!(renumber(&first, 2), "2ND");
        // Not an ordinal: the suffix is kept as is (AOO).
        assert_eq!(renumber(&numbered("1th").unwrap(), 2), "2th");
        assert_eq!(renumber(&numbered("1st place").unwrap(), 2), "2st place");
        assert_eq!(renumber(&numbered("Item 09").unwrap(), 10), "Item 10");
        assert_eq!(renumber(&numbered("Item 09").unwrap(), -1), "Item -01");
        assert_eq!(constant_step(&[1.0, 3.0, 5.0]), Some(2.0));
        assert_eq!(constant_step(&[1.0, 3.0, 4.0]), None);
        let e = DateEpoch::default();
        let jan31 = serial_from_ymd(e, 2024, 1, 31).unwrap();
        let feb29 = serial_from_ymd(e, 2024, 2, 29).unwrap();
        assert_eq!(add_months(e, jan31, 1), Some(feb29));
    }
}
