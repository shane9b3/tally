//! CELL, FORMULA, CURRENT and INFO.

use tally_types::{CellAddr, ErrorKind, Value};

use crate::coerce::top_left;
use crate::registry::{Category, FnFlags, FunctionDef, Param, ParamKind, Registry, ReturnKind};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const CELL_PARAMS: &[Param] = &[
    Param::required("info_type", ParamKind::Text),
    Param::optional("reference", ParamKind::Reference),
];
const FORMULA_PARAMS: &[Param] = &[Param::required("reference", ParamKind::Reference)];
const INFO_PARAMS: &[Param] = &[Param::required("type", ParamKind::Text)];

/// `CELL(info_type [; reference])`.
pub const CELL: FunctionDef = FunctionDef::new("CELL", Category::Information, CELL_PARAMS, cell)
    .returns(ReturnKind::Any)
    .flags(FnFlags::NEEDS_ORIGIN)
    .help("Information about the position, contents or formatting of a cell.");
/// `FORMULA(reference)`.
pub const FORMULA: FunctionDef =
    FunctionDef::new("FORMULA", Category::Information, FORMULA_PARAMS, formula)
        .returns(ReturnKind::Text)
        .flags(FnFlags::REF_ADDRESS_ONLY)
        .help("The formula of a cell as text.");
/// `CURRENT()`.
pub const CURRENT: FunctionDef = FunctionDef::new("CURRENT", Category::Information, &[], current)
    .odf("ORG.OPENOFFICE.CURRENT")
    .help("Current value of the formula so far (not supported: always #N/A).");
/// `INFO(type)`.
pub const INFO: FunctionDef = FunctionDef::new("INFO", Category::Information, INFO_PARAMS, info)
    .returns(ReturnKind::Any)
    .help("Information about the environment.");

/// The first section of a format code split on `;` outside quotes/brackets.
fn sections(code: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    let (mut quote, mut bracket, mut esc) = (false, false, false);
    for c in code.chars() {
        let cur = out.len() - 1;
        if esc {
            esc = false;
        } else if c == '\\' {
            esc = true;
        } else if c == '"' && !bracket {
            quote = !quote;
        } else if c == '[' && !quote {
            bracket = true;
        } else if c == ']' && !quote {
            bracket = false;
        } else if c == ';' && !quote && !bracket {
            out.push(String::new());
            continue;
        }
        out[cur].push(c);
    }
    out
}

/// Removes quoted text, `[...]` modifiers and escaped characters.
fn strip_literals(sec: &str) -> String {
    let mut out = String::new();
    let (mut quote, mut bracket, mut esc) = (false, false, false);
    for c in sec.chars() {
        if esc {
            esc = false;
        } else if c == '\\' {
            esc = true;
        } else if c == '"' && !bracket {
            quote = !quote;
        } else if c == '[' && !quote {
            bracket = true;
        } else if c == ']' && !quote {
            bracket = false;
        } else if !quote && !bracket {
            out.push(c);
        }
    }
    out
}

fn has_color(sec: &str) -> bool {
    let (mut bracket, mut cur) = (false, String::new());
    for c in sec.chars() {
        match c {
            '[' => {
                bracket = true;
                cur.clear();
            }
            ']' if bracket => {
                bracket = false;
                let l = cur.to_ascii_lowercase();
                if l.starts_with("color")
                    || [
                        "red", "blue", "green", "black", "white", "yellow", "cyan", "magenta",
                    ]
                    .contains(&l.as_str())
                {
                    return true;
                }
            }
            _ if bracket => cur.push(c),
            _ => {}
        }
    }
    false
}

/// `true` if the section's date/time letters include a day-of-week name
/// (`NN`, `NNN`, `DDD`, `DDDD`, `AAA`), which only AOO's long built-in dates
/// have.
fn has_weekday(t: &str) -> bool {
    t.contains("nn") || t.contains("ddd") || t.contains("aaa")
}

/// The base letter(s) of AOO's CELL("format") code for a date/time section
/// (lower-cased, literals stripped), or `None` if it is not a date/time.
///
/// AOO (`interpr1.cxx` `ScCell`, "FORMAT") maps only its *built-in* date and
/// time formats (`GetIndexTableOffset`) and answers `G` for every other
/// date/time format; Tally has no built-in index table, so the code's shape
/// stands in for it:
///
/// - day, month and year, numeric or with a month name (`NF_DATE_SYSTEM_SHORT`,
///   `NF_DATE_SYS_DDMMYY`, `NF_DATE_SYS_DMMMYY`, `NF_DATE_SYS_DMMMYYYY`, …) → `D1`;
///   year-first (ISO `YYYY-MM-DD`, `NF_DATE_DIN_YYYYMMDD`) and day-of-week
///   (long) dates → `G`;
/// - day and month name (`NF_DATE_SYS_DDMMM`) → `D2`; numeric day and month
///   (`NF_DATE_DIN_MMDD`) → `D5`; month and year (`NF_DATE_SYS_MMYY`) → `D3`;
/// - date and time (`NF_DATETIME_SYSTEM_SHORT_HHMM`,
///   `NF_DATETIME_SYS_DDMMYYYY_HHMMSS`) → `D4`, ISO date-times → `G`;
/// - `HH:MM:SS AM/PM` → `D6`, `HH:MM AM/PM` → `D7`, `HH:MM:SS` → `D8`,
///   `HH:MM` → `D9`; elapsed (`[HH]`), `MM:SS` and fractional seconds → `G`.
fn date_code(sec_lower: &str, plain: &str) -> Option<&'static str> {
    let ampm = plain.contains("am/pm") || plain.contains("a/p");
    let t = plain.replace("am/pm", "").replace("a/p", "");
    let (y, d, h, s, m) = (
        t.contains('y'),
        t.contains('d') || t.contains('n') || t.contains('a'),
        t.contains('h'),
        t.contains('s'),
        t.contains('m'),
    );
    let is_dt = h || y || d || s || (m && !t.contains('0') && !t.contains('#'));
    if !is_dt {
        return None;
    }
    let date = y || d || (m && !h && !s);
    let year_first = t.trim_start().starts_with('y');
    let elapsed = sec_lower.contains("[h") || sec_lower.contains("[m") || sec_lower.contains("[s");
    let frac_sec = s && t.contains(".0");
    if date && (h || s) {
        return Some(if y && d && m && !year_first && !has_weekday(&t) {
            "D4"
        } else {
            "G"
        });
    }
    if date {
        if has_weekday(&t) {
            return Some("G");
        }
        let month_name = t.contains("mmm");
        return Some(match (y, d, m) {
            (true, true, true) if !year_first => "D1",
            (false, true, true) if month_name => "D2",
            (false, true, true) => "D5",
            (true, false, true) => "D3",
            _ => "G",
        });
    }
    // Time only.
    if elapsed || frac_sec || !h {
        return Some("G");
    }
    Some(match (s, ampm) {
        (true, true) => "D6",
        (false, true) => "D7",
        (true, false) => "D8",
        (false, false) => "D9",
    })
}

/// Removes locale-only `[$-409]` tags (no currency symbol) so that their `$`
/// does not read as a currency.
fn strip_locale_tags(raw: &str) -> String {
    let mut out = raw.to_owned();
    while let Some(i) = out.find("[$-") {
        match out[i..].find(']') {
            Some(j) => out.replace_range(i..i + j + 1, ""),
            None => break,
        }
    }
    out
}

/// The AOO CELL("format") code for a number format code (`None` = General).
///
/// Port of `ScCell` "FORMAT" (`interpr1.cxx`): `,`/`F` (number with or
/// without thousands separator), `C`, `S`, `P` followed by the decimals of
/// the first section; date/time codes per [`date_code`]; `G` otherwise
/// (General, text, fractions, booleans). Then `-` when the negative section
/// has a color (`lcl_FormatHasNegColor`) and `()` when the format string
/// contains `(` anywhere (`lcl_FormatHasOpenPar`).
pub(super) fn format_code_info(code: Option<&str>) -> String {
    let Some(code) = code else {
        return "G".into();
    };
    let secs = sections(code);
    let mut out = base_code(&secs[0]);
    if secs.get(1).is_some_and(|s| has_color(s)) {
        out.push('-');
    }
    if code.contains('(') {
        out.push_str("()");
    }
    out
}

fn base_code(first: &str) -> String {
    let plain = strip_literals(first).to_ascii_lowercase();
    let p = plain.trim();
    if p.is_empty() || p == "general" || p == "@" {
        return "G".into();
    }
    let decimals = |s: &str| -> usize {
        match s.split_once('.') {
            Some((_, frac)) => frac
                .chars()
                .take_while(|c| matches!(c, '0' | '#' | '?'))
                .count(),
            None => 0,
        }
    };
    if let Some(d) = date_code(&first.to_ascii_lowercase(), &plain) {
        return d.into();
    }
    let raw = strip_locale_tags(&first.to_ascii_lowercase());
    let has_digit = plain.contains('0') || plain.contains('#') || plain.contains('?');
    // Fractions (`# ?/?`) are NUMBERFORMAT_FRACTION: `G` in AOO.
    if !has_digit || plain.contains('/') {
        return "G".into();
    }
    let dec = decimals(&plain);
    if plain.contains('%') {
        format!("P{dec}")
    } else if plain.contains("e+") || plain.contains("e-") {
        let mantissa = plain.split('e').next().unwrap_or("");
        format!("S{}", decimals(mantissa))
    } else if ["$", "€", "£", "¥"].iter().any(|c| raw.contains(c)) {
        format!("C{dec}")
    } else if plain.contains("#,#") || plain.contains(",0") || plain.contains("0,0") {
        format!(",{dec}")
    } else {
        format!("F{dec}")
    }
}

fn target(a: &mut Args<'_>) -> Result<CellAddr, ErrorKind> {
    if a.is_missing(1) {
        Ok(a.ctx().origin())
    } else {
        Ok(a.range(1)?.start())
    }
}

fn cell(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let kind = a.text(0)?.trim().to_ascii_lowercase();
        const KINDS: [&str; 15] = [
            "address",
            "col",
            "color",
            "contents",
            "coord",
            "filename",
            "format",
            "parentheses",
            "prefix",
            "protect",
            "row",
            "sheet",
            "type",
            "width",
            "coord",
        ];
        if !KINDS.contains(&kind.as_str()) {
            return Err(ErrorKind::InvalidArgument);
        }
        let addr = target(&mut a)?;
        let info = a.ctx().cell_format_info(addr);
        let bool_num = |b: bool| Value::Number(if b { 1.0 } else { 0.0 });
        Ok(match kind.as_str() {
            // AOO formats `$Sheet2.$A$1` (SCA_ABS_3D) for a cell on another
            // sheet; the EvalContext exposes no sheet names, so Tally always
            // returns the sheet-less `$A$1`.
            "address" => Value::text(format!(
                "${}${}",
                tally_types::col_to_letters(addr.col),
                addr.row + 1
            )),
            // Lotus form `$TABLE:$COL$ROW`; AOO formats the sheet index as a
            // column ("Yes, passing tab as col is intentional!"), so sheet 1
            // cell A1 is `$A:$A$1`.
            "coord" => {
                let tab = a.ctx().sheet_index(addr.sheet).ok_or(ErrorKind::Ref)?;
                Value::text(format!(
                    "${}:${}${}",
                    tally_types::col_to_letters(tab),
                    tally_types::col_to_letters(addr.col),
                    addr.row + 1
                ))
            }
            "col" => Value::Number(f64::from(addr.col) + 1.0),
            "row" => Value::Number(f64::from(addr.row) + 1.0),
            "sheet" => match a.ctx().sheet_index(addr.sheet) {
                Some(i) => Value::Number(f64::from(i) + 1.0),
                None => return Err(ErrorKind::Ref),
            },
            // The document path is not available to functions.
            "filename" => Value::text(""),
            "contents" => match a.ctx().cell_value(addr) {
                Value::Array(m) => top_left(&m)?,
                v => v,
            },
            "type" => Value::text(match a.ctx().cell_value(addr) {
                Value::Empty => "b",
                Value::Text(_) => "l",
                _ => "v",
            }),
            "prefix" => Value::text(match a.ctx().cell_value(addr) {
                Value::Text(_) => "'",
                _ => "",
            }),
            "format" => Value::text(format_code_info(info.format_code.as_deref())),
            "width" => Value::Number(10.0),
            // ATTR_PROTECTION of the cell (AOO's default attribute is
            // locked, so the engine reports `protected = true` by default).
            "protect" => bool_num(info.protected),
            // lcl_FormatHasOpenPar: `(` anywhere in the format string.
            "parentheses" => bool_num(info.format_code.as_deref().is_some_and(|c| c.contains('('))),
            "color" => bool_num(
                info.format_code
                    .as_deref()
                    .is_some_and(|c| sections(c).get(1).is_some_and(|s| has_color(s))),
            ),
            _ => return Err(ErrorKind::InvalidArgument),
        })
    })
}

fn formula(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let r = a.range(0)?;
        let info = a.ctx().cell_format_info(r.start());
        match info.formula {
            Some(f) if info.has_formula || !f.is_empty() => {
                if f.starts_with('=') {
                    Ok(Value::Text(f))
                } else {
                    Ok(Value::text(format!("={f}")))
                }
            }
            _ => Err(ErrorKind::NA),
        }
    })
}

/// The running value of the formula is not available through the
/// `EvalContext`, so this is always `#N/A`.
fn current(_ctx: &mut dyn EvalContext, _args: &mut dyn ArgList) -> EvalValue {
    // TODO: needs a running-value hook in the framework.
    EvalValue::error(ErrorKind::NA)
}

fn info(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let key = a.text(0)?.trim().to_ascii_lowercase();
        Ok(match key.as_str() {
            "system" => Value::text(if cfg!(target_os = "windows") {
                "WNT"
            } else if cfg!(target_os = "macos") {
                "MACOSX"
            } else {
                "LINUX"
            }),
            "osversion" => Value::text(if cfg!(target_os = "windows") {
                "Windows (32-bit) NT 10.00"
            } else if cfg!(target_os = "macos") {
                "Macintosh"
            } else {
                "Linux"
            }),
            "release" => Value::text("4.1"),
            "numfile" => Value::Number(1.0),
            "recalc" => Value::text("Automatic"),
            _ => return Err(ErrorKind::InvalidArgument),
        })
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(CELL);
    r.register(FORMULA);
    r.register(CURRENT);
    r.register(INFO);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::CellFormatInfo;
    use crate::mock::{call, MockEvalContext};

    fn fmt(code: Option<&str>) -> String {
        format_code_info(code)
    }

    #[test]
    fn format_codes() {
        assert_eq!(fmt(None), "G");
        assert_eq!(fmt(Some("General")), "G");
        assert_eq!(fmt(Some("0")), "F0");
        assert_eq!(fmt(Some("0.00")), "F2");
        assert_eq!(fmt(Some("#,##0")), ",0");
        assert_eq!(fmt(Some("#,##0.00")), ",2");
        assert_eq!(fmt(Some("0%")), "P0");
        assert_eq!(fmt(Some("0.00%")), "P2");
        assert_eq!(fmt(Some("0.00E+00")), "S2");
        assert_eq!(fmt(Some("\"$\"#,##0.00")), "C2");
        assert_eq!(fmt(Some("[$$-409]#,##0.00;[RED]-[$$-409]#,##0.00")), "C2-");
        assert_eq!(fmt(Some("DD-MMM-YY")), "D1");
        // NF_DATE_SYSTEM_SHORT is D1 in AOO (D4 is Excel's code).
        assert_eq!(fmt(Some("MM/DD/YY")), "D1");
        assert_eq!(fmt(Some("MM/DD/YYYY")), "D1");
        assert_eq!(fmt(Some("DD.MM.YYYY")), "D1");
        assert_eq!(fmt(Some("[$-409]MM/DD/YY")), "D1");
        assert_eq!(fmt(Some("MM/DD/YY HH:MM")), "D4");
        assert_eq!(fmt(Some("YYYY-MM-DD HH:MM")), "G");
        assert_eq!(fmt(Some("YYYY-MM-DD")), "G");
        assert_eq!(fmt(Some("NNNNMMMM DD, YYYY")), "G");
        assert_eq!(fmt(Some("MM-DD")), "D5");
        assert_eq!(fmt(Some("MMMM")), "G");
        assert_eq!(fmt(Some("[HH]:MM:SS")), "G");
        assert_eq!(fmt(Some("MM:SS")), "G");
        assert_eq!(fmt(Some("# ?/?")), "G");
        // lcl_FormatHasOpenPar: "()" when `(` appears anywhere.
        assert_eq!(fmt(Some("#,##0.00;(#,##0.00)")), ",2()");
        assert_eq!(fmt(Some("#,##0.00;[RED](#,##0.00)")), ",2-()");
        assert_eq!(fmt(Some("[$-409]0.00")), "F2");
        assert_eq!(fmt(Some("DD-MMM")), "D2");
        assert_eq!(fmt(Some("MMM-YY")), "D3");
        assert_eq!(fmt(Some("HH:MM:SS AM/PM")), "D6");
        assert_eq!(fmt(Some("HH:MM AM/PM")), "D7");
        assert_eq!(fmt(Some("HH:MM:SS")), "D8");
        assert_eq!(fmt(Some("HH:MM")), "D9");
        assert_eq!(fmt(Some("@")), "G");
    }

    #[test]
    fn coord_and_parentheses() {
        let mut c = MockEvalContext::new();
        c.set_origin(0, "D7");
        assert_eq!(c1(&mut c, "coord", Some("C5")), Value::text("$A:$C$5"));
        assert_eq!(c1(&mut c, "coord", None), Value::text("$A:$D$7"));
        let i = CellFormatInfo {
            format_code: Some("#,##0.00;(#,##0.00)".into()),
            ..Default::default()
        };
        c.set_format(0, "B2", i);
        assert_eq!(c1(&mut c, "parentheses", Some("B2")), Value::Number(1.0));
        assert_eq!(c1(&mut c, "format", Some("B2")), Value::text(",2()"));
        assert_eq!(c1(&mut c, "protect", Some("B2")), Value::Number(0.0));
    }

    fn c1(c: &mut MockEvalContext, kind: &str, cell: Option<&str>) -> Value {
        let mut v = vec![EvalValue::text(kind)];
        if let Some(r) = cell {
            v.push(c.range(0, r));
        }
        call(c, "CELL", v)
    }

    #[test]
    fn cell_infos() {
        let mut c = MockEvalContext::new();
        c.set(0, "B3", 5.0).set(0, "C1", "txt");
        c.set_origin(0, "D7");
        let i = CellFormatInfo {
            format_code: Some("0.00".into()),
            protected: true,
            ..Default::default()
        };
        c.set_format(0, "B3", i);
        assert_eq!(c1(&mut c, "row", Some("B3")), Value::Number(3.0));
        assert_eq!(c1(&mut c, "COL", Some("B3")), Value::Number(2.0));
        assert_eq!(c1(&mut c, "col", None), Value::Number(4.0));
        assert_eq!(c1(&mut c, "address", Some("B3")), Value::text("$B$3"));
        assert_eq!(c1(&mut c, "sheet", Some("B3")), Value::Number(1.0));
        assert_eq!(c1(&mut c, "contents", Some("B3")), Value::Number(5.0));
        assert_eq!(c1(&mut c, "type", Some("B3")), Value::text("v"));
        assert_eq!(c1(&mut c, "type", Some("C1")), Value::text("l"));
        assert_eq!(c1(&mut c, "type", Some("E9")), Value::text("b"));
        assert_eq!(c1(&mut c, "format", Some("B3")), Value::text("F2"));
        assert_eq!(c1(&mut c, "protect", Some("B3")), Value::Number(1.0));
        assert_eq!(c1(&mut c, "prefix", Some("C1")), Value::text("'"));
        assert_eq!(
            c1(&mut c, "bogus", Some("B3")),
            Value::Error(ErrorKind::InvalidArgument)
        );
    }

    #[test]
    fn formula_and_info() {
        let mut c = MockEvalContext::new();
        let i = CellFormatInfo {
            has_formula: true,
            formula: Some("=SUM(1;2)".into()),
            ..Default::default()
        };
        c.set_format(0, "A1", i);
        let r = c.range(0, "A1");
        assert_eq!(call(&mut c, "FORMULA", vec![r]), Value::text("=SUM(1;2)"));
        let r = c.range(0, "A2");
        assert_eq!(
            call(&mut c, "FORMULA", vec![r]),
            Value::Error(ErrorKind::NA)
        );
        assert_eq!(call(&mut c, "CURRENT", vec![]), Value::Error(ErrorKind::NA));
        let sys = call(&mut c, "INFO", vec![EvalValue::text("SYSTEM")]);
        assert!(matches!(sys, Value::Text(_)));
        assert_eq!(
            call(&mut c, "INFO", vec![EvalValue::text("nope")]),
            Value::Error(ErrorKind::InvalidArgument)
        );
        assert_eq!(
            call(&mut c, "INFO", vec![EvalValue::text("recalc")]),
            Value::text("Automatic")
        );
    }
}
