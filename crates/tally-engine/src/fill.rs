//! Fill handle and Fill Series (inventory §4, AOO `ScTable::FillAnalyse`).
//!
//! The target extends the source in exactly one direction. Each lane (a
//! column when filling down/up, a row when filling right/left) is filled
//! on its own from the source cells read *away* from the edge being
//! extended, so filling up or left continues the series backwards
//! (1, 2 filled up gives 0, −1). Styles repeat with the source pattern.
//!
//! `Auto` detection per lane:
//! - numbers: one cell steps by +1; several with a constant difference
//!   form a linear series; otherwise the pattern repeats, each repetition
//!   +1 (AOO's `FILL_SIMPLE` with delta 1);
//! - date-formatted numbers: one cell steps one day; several step by
//!   months (years) when the day of month is constant and the month
//!   difference is, else by a constant day difference; otherwise the
//!   pattern repeats;
//! - day and month names (full or abbreviated, en-US, wrapping) step
//!   through their list keeping the case style;
//! - text with a trailing (else leading) integer ("Item 1", "Q3",
//!   "001x") increments it, keeping leading zeros;
//! - anything else (plain text, formulas, mixed lanes) is copied; formulas
//!   are relative, so their references follow the new position.

use std::sync::Arc;

use tally_api::{ApiError, FillMode};
use tally_numfmt::date::{days_in_month, serial_from_ymd, ymd_from_serial};
use tally_types::{approx_equal, CellAddr, CellRange, DateEpoch, Value};

use crate::ops::{array_block_error, MAX_OP_CELLS};
use crate::storage::{CellContent, StyleId};
use crate::undo::{capture_range, Change};
use crate::workbook::{SnapContent, Workbook};

const DAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
const DAYS_ABBR: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const MONTHS_ABBR: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const LISTS: [&[&str]; 4] = [&DAYS, &DAYS_ABBR, &MONTHS, &MONTHS_ABBR];

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

/// `(list index, position)` of a day/month name.
fn list_pos(s: &str) -> Option<(usize, usize)> {
    let t = s.trim();
    LISTS.iter().enumerate().find_map(|(li, list)| {
        list.iter()
            .position(|x| x.eq_ignore_ascii_case(t))
            .map(|p| (li, p))
    })
}

/// Text with a number: `(prefix, number, digits, suffix)` (trailing number
/// preferred, else leading).
fn numbered(s: &str) -> Option<(String, i64, usize, String)> {
    let bytes = s.as_bytes();
    let end = bytes.len();
    let mut i = end;
    while i > 0 && bytes[i - 1].is_ascii_digit() {
        i -= 1;
    }
    if i < end && end - i <= 15 {
        let n: i64 = s[i..].parse().ok()?;
        return Some((s[..i].to_owned(), n, end - i, String::new()));
    }
    let mut j = 0;
    while j < end && bytes[j].is_ascii_digit() {
        j += 1;
    }
    if j > 0 && j <= 15 && j < end {
        let n: i64 = s[..j].parse().ok()?;
        return Some((String::new(), n, j, s[j..].to_owned()));
    }
    None
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
        list: usize,
        start: usize,
        step: i64,
        case: Case,
    },
    Numbered {
        prefix: String,
        suffix: String,
        last: i64,
        step: i64,
        digits: usize,
    },
}

fn plan_lane(seq: &[Src], mode: FillMode, epoch: DateEpoch) -> Plan {
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
        FillMode::Copy => return Plan::Copy,
        FillMode::LinearSeries => {
            return match &all_numbers {
                Some(xs) if n >= 2 => Plan::Linear(xs[n - 1] - xs[n - 2]),
                Some(_) => Plan::Linear(1.0),
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
                Some(xs) => date_plan(xs, epoch).unwrap_or(Plan::Linear(1.0)),
                None => Plan::Copy,
            }
        }
        FillMode::Auto => {}
    }
    if let Some(xs) = &all_numbers {
        if all_dates {
            if n == 1 {
                return Plan::Linear(1.0);
            }
            return date_plan(xs, epoch).unwrap_or(Plan::Copy);
        }
        if n == 1 {
            return Plan::Linear(1.0);
        }
        return match constant_step(xs) {
            Some(d) => Plan::Linear(d),
            None => Plan::CopyDelta(1.0),
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
        return Plan::Copy;
    };
    // Day / month names.
    let pos: Option<Vec<(usize, usize)>> = texts.iter().map(|t| list_pos(t)).collect();
    if let Some(pos) = pos {
        let list = pos[0].0;
        if pos.iter().all(|p| p.0 == list) {
            let len = LISTS[list].len() as i64;
            let step = if n == 1 {
                1
            } else {
                let d = (pos[1].1 as i64 - pos[0].1 as i64).rem_euclid(len);
                if pos
                    .windows(2)
                    .all(|w| (w[1].1 as i64 - w[0].1 as i64).rem_euclid(len) == d)
                {
                    d
                } else {
                    return Plan::Copy;
                }
            };
            return Plan::List {
                list,
                start: pos[n - 1].1,
                step,
                case: case_of(texts[n - 1]),
            };
        }
    }
    // Numbered text.
    let nums: Option<Vec<(String, i64, usize, String)>> =
        texts.iter().map(|t| numbered(t)).collect();
    if let Some(nums) = nums {
        let (p, s) = (&nums[0].0, &nums[0].3);
        if nums.iter().all(|x| &x.0 == p && &x.3 == s) {
            let step = if n == 1 {
                1
            } else {
                let d = nums[1].1 - nums[0].1;
                if nums.windows(2).all(|w| w[1].1 - w[0].1 == d) {
                    d
                } else {
                    1
                }
            };
            let last = &nums[n - 1];
            return Plan::Numbered {
                prefix: last.0.clone(),
                suffix: last.3.clone(),
                last: last.1,
                step,
                digits: last.2,
            };
        }
    }
    Plan::Copy
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
            match &src.content {
                Some(SnapContent::Value(Value::Number(x))) => num(x + (k / n + 1) as f64 * d),
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
            let l = LISTS[*list];
            let i = (*start as i64 + step * (k as i64 + 1)).rem_euclid(l.len() as i64) as usize;
            Some(SnapContent::Value(Value::text(apply_case(l[i], *case))))
        }
        Plan::Numbered {
            prefix,
            suffix,
            last,
            step,
            digits,
        } => {
            let v = last + step * (k as i64 + 1);
            let body = if v < 0 {
                format!("-{:0w$}", -v, w = *digits)
            } else {
                format!("{v:0w$}", w = *digits)
            };
            Some(SnapContent::Value(Value::text(format!(
                "{prefix}{body}{suffix}"
            ))))
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
    let epoch = wb.settings.date_epoch;
    let inv = capture_range(wb, area, true, true);
    let vertical = matches!(dir, Dir::Down | Dir::Up);
    let lanes = if vertical {
        source.c0..=source.c1
    } else {
        source.r0..=source.r1
    };
    for lane in lanes {
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
        let plan = plan_lane(&seq, mode, epoch);
        let positions: Vec<u32> = match dir {
            Dir::Down => (source.r1 + 1..=target.r1).collect(),
            Dir::Up => (target.r0..source.r0).rev().collect(),
            Dir::Right => (source.c1 + 1..=target.c1).collect(),
            Dir::Left => (target.c0..source.c0).rev().collect(),
        };
        for (k, p) in positions.into_iter().enumerate() {
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
        assert_eq!(list_pos("mon"), Some((1, 1)));
        assert_eq!(list_pos("December"), Some((2, 11)));
        assert_eq!(
            numbered("Item 9"),
            Some(("Item ".into(), 9, 1, String::new()))
        );
        assert_eq!(numbered("007x"), Some((String::new(), 7, 3, "x".into())));
        assert_eq!(numbered("abc"), None);
        assert_eq!(constant_step(&[1.0, 3.0, 5.0]), Some(2.0));
        assert_eq!(constant_step(&[1.0, 3.0, 4.0]), None);
        let e = DateEpoch::default();
        let jan31 = serial_from_ymd(e, 2024, 1, 31).unwrap();
        let feb29 = serial_from_ymd(e, 2024, 2, 29).unwrap();
        assert_eq!(add_months(e, jan31, 1), Some(feb29));
    }
}
