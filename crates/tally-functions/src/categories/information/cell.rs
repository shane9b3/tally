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

/// The AOO CELL("format") code for a number format code (`None` = General).
pub(super) fn format_code_info(code: Option<&str>) -> String {
    let Some(code) = code else {
        return "G".into();
    };
    let secs = sections(code);
    let first = &secs[0];
    let plain = strip_literals(first).to_ascii_lowercase();
    if plain.trim().is_empty() || plain.trim() == "general" || plain.trim() == "@" {
        return "G".into();
    }
    let neg_color = secs.get(1).is_some_and(|s| has_color(s));
    let suffix = if neg_color { "-" } else { "" };
    let decimals = |s: &str| -> usize {
        match s.split_once('.') {
            Some((_, frac)) => frac
                .chars()
                .take_while(|c| matches!(c, '0' | '#' | '?'))
                .count(),
            None => 0,
        }
    };
    let ampm = plain.contains("am/pm") || plain.contains("a/p");
    let t = plain.replace("am/pm", "").replace("a/p", "");
    let (y, d, h, s_, m) = (
        t.contains('y'),
        t.contains('d'),
        t.contains('h'),
        t.contains('s'),
        t.contains('m'),
    );
    if h || y || d || s_ || (m && !t.contains('0') && !t.contains('#')) {
        let code = if h || s_ {
            match (s_, ampm) {
                (true, true) => "D6",
                (false, true) => "D7",
                (true, false) => "D8",
                (false, false) => "D9",
            }
        } else if y && d {
            if t.contains("mmm") {
                "D1"
            } else {
                "D4"
            }
        } else if d && m {
            if t.contains("mmm") {
                "D2"
            } else {
                "D5"
            }
        } else if m && y {
            "D3"
        } else {
            "D4"
        };
        return format!("{code}{suffix}");
    }
    let raw = first.to_ascii_lowercase();
    let has_digit = plain.contains('0') || plain.contains('#') || plain.contains('?');
    if !has_digit {
        return "G".into();
    }
    let dec = decimals(&plain);
    let letter = if plain.contains('%') {
        "P".to_owned()
    } else if plain.contains("e+") || plain.contains("e-") {
        let mantissa = plain.split(['e']).next().unwrap_or("");
        return format!("S{}{suffix}", decimals(mantissa));
    } else if ["$", "€", "£", "¥"].iter().any(|c| raw.contains(c)) {
        "C".to_owned()
    } else if plain.contains("#,#") || plain.contains(",0") || plain.contains("0,0") {
        return format!(",{dec}{suffix}");
    } else {
        "F".to_owned()
    };
    format!("{letter}{dec}{suffix}")
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
            "address" | "coord" => Value::text(format!(
                "${}${}",
                tally_types::col_to_letters(addr.col),
                addr.row + 1
            )),
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
            "protect" => bool_num(info.protected),
            "parentheses" => bool_num(
                info.format_code
                    .as_deref()
                    .is_some_and(|c| strip_literals(&sections(c)[0]).contains('(')),
            ),
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
        assert_eq!(fmt(Some("MM/DD/YY")), "D4");
        assert_eq!(fmt(Some("DD-MMM")), "D2");
        assert_eq!(fmt(Some("MMM-YY")), "D3");
        assert_eq!(fmt(Some("HH:MM:SS AM/PM")), "D6");
        assert_eq!(fmt(Some("HH:MM AM/PM")), "D7");
        assert_eq!(fmt(Some("HH:MM:SS")), "D8");
        assert_eq!(fmt(Some("HH:MM")), "D9");
        assert_eq!(fmt(Some("@")), "G");
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
