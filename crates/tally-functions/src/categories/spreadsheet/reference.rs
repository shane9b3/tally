//! ADDRESS, INDIRECT, OFFSET (reference construction) and AREAS, ROWS,
//! COLUMNS, SHEET, SHEETS (reference inspection).

use tally_types::{col_to_letters, CellRange, ErrorKind, Value, MAX_COLS, MAX_ROWS};

use crate::registry::{Category, FnFlags, FunctionDef, Param, ParamKind, Registry, ReturnKind};
use crate::{run, run_ref, ArgList, Args, EvalContext, EvalValue};

// ---------------------------------------------------------------- ADDRESS

const ADDRESS_PARAMS: &[Param] = &[
    Param::required("row", ParamKind::Integer),
    Param::required("column", ParamKind::Integer),
    Param::optional("abs", ParamKind::Integer),
    Param::optional("a1", ParamKind::Bool),
    Param::optional("sheet", ParamKind::Text),
];

/// `ADDRESS(row; column [; abs [; a1 [; sheet]]])`.
pub const ADDRESS: FunctionDef =
    FunctionDef::new("ADDRESS", Category::Spreadsheet, ADDRESS_PARAMS, address)
        .returns(ReturnKind::Text)
        .help("Cell address as text.");

/// AOO `ScCompiler::CheckTabQuotes`: a sheet name that is not an identifier
/// (letters, digits and `_`; it may start with a digit) or that is purely
/// numeric is quoted, with embedded `'` doubled.
fn quote_sheet(name: &str) -> String {
    let ident = !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_');
    let numeric = !name.is_empty() && name.bytes().all(|c| c.is_ascii_digit());
    if ident && !numeric {
        name.to_string()
    } else {
        format!("'{}'", name.replace('\'', "''"))
    }
}

/// The sheet prefix of AOO `ScAddressFunc`: in Calc A1 syntax a leading
/// `'Doc'#` (or `'Doc'#$`) part is kept as is; the sheet name is quoted
/// with [`quote_sheet`] unless it already starts and ends with `'`.
fn sheet_prefix(name: &str, a1: bool) -> String {
    let (doc, tab) = match a1.then(|| doc_tab_pos(name)).flatten() {
        Some(pos) => {
            let mut pos = pos;
            if name[pos + 1..].starts_with('$') {
                pos += 1;
            }
            name.split_at(pos + 1)
        }
        None => ("", name),
    };
    let quoted = tab.len() >= 2 && tab.starts_with('\'') && tab.ends_with('\'');
    let tab = if quoted || tab == "'" {
        tab.to_owned()
    } else {
        quote_sheet(tab)
    };
    format!("{doc}{tab}")
}

/// AOO `ScCompiler::GetDocTabPos`: the byte index of an unquoted `#` that
/// follows a quoted document name (`'Doc'#Sheet`).
fn doc_tab_pos(s: &str) -> Option<usize> {
    if !s.starts_with('\'') {
        return None;
    }
    let mut quoted = false;
    for (i, c) in s.char_indices() {
        match c {
            '\'' => quoted = !quoted,
            '#' if !quoted => return (s[..i].ends_with('\'')).then_some(i),
            _ => {}
        }
    }
    None
}

/// Port of AOO `ScInterpreter::ScAddressFunc` (`interpr1.cxx`):
///
/// - `abs` 1 = `$A$1`, 2 = `A$1`, 3 = `$A1`, 4 = `A1`; 5 to 8 are aliases
///   of 1 to 4; any other value is `#VALUE!` (`PushNoValue`).
/// - `a1` is R1C1 when its floor is 0 (`CONV_XL_R1C1`): `R2C3`, `R2C[3]`,
///   `R[2]C[3]`; a relative offset of 0 is written without brackets (`R`,
///   `C`). As in AOO ("the XL interface actually treats rel R1C1 refs
///   differently"), a relative row/column is an offset from the formula
///   cell, so it may be 0 or negative as long as the target is in the grid.
/// - `sheet` (non-empty) is prefixed with `.` in A1 syntax and `!` in R1C1
///   syntax, without `$` (`Sheet1.$A$1`, `'My Sheet'.A1`, `Sheet1!R1C1`).
/// - A row or column outside the grid is `Err:502`.
fn address(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let floor = |x: f64| crate::coerce::approx_floor(x);
        let row = floor(a.num(0)?);
        let col = floor(a.num(1)?);
        let abs = floor(a.num_opt(2, 1.0)?);
        let a1 = floor(a.num_opt(3, 1.0)?) != 0.0;
        let sheet = if a.is_missing(4) {
            None
        } else {
            Some(a.text(4)?).filter(|s| !s.is_empty())
        };
        let (abs_row, abs_col) = match abs as i64 {
            1 | 5 => (true, true),
            2 | 6 => (true, false),
            3 | 7 => (false, true),
            4 | 8 => (false, false),
            _ => return Err(ErrorKind::Value),
        };
        // The target cell (1-based) that must lie in the grid.
        let origin = a.ctx().origin();
        let (t_row, t_col) = if a1 {
            (row, col)
        } else {
            (
                if abs_row {
                    row
                } else {
                    row + f64::from(origin.row) + 1.0
                },
                if abs_col {
                    col
                } else {
                    col + f64::from(origin.col) + 1.0
                },
            )
        };
        if !(1.0..=f64::from(MAX_ROWS)).contains(&t_row)
            || !(1.0..=f64::from(MAX_COLS)).contains(&t_col)
        {
            return Err(ErrorKind::InvalidArgument);
        }
        let cell = if a1 {
            format!(
                "{}{}{}{}",
                if abs_col { "$" } else { "" },
                col_to_letters((col - 1.0) as u32),
                if abs_row { "$" } else { "" },
                row
            )
        } else {
            let part = |letter: char, v: f64, abs: bool| match (abs, v as i64) {
                (true, n) => format!("{letter}{n}"),
                (false, 0) => letter.to_string(),
                (false, n) => format!("{letter}[{n}]"),
            };
            format!("{}{}", part('R', row, abs_row), part('C', col, abs_col))
        };
        let text = match sheet {
            None => cell,
            Some(s) => format!(
                "{}{}{cell}",
                sheet_prefix(&s, a1),
                if a1 { '.' } else { '!' }
            ),
        };
        Ok(Value::text(text))
    })
}

// --------------------------------------------------------------- INDIRECT

const INDIRECT_PARAMS: &[Param] = &[
    Param::required("ref", ParamKind::Text),
    Param::optional("a1", ParamKind::Bool),
];

/// `INDIRECT(ref [; a1])`.
pub const INDIRECT: FunctionDef =
    FunctionDef::new("INDIRECT", Category::Spreadsheet, INDIRECT_PARAMS, indirect)
        .returns(ReturnKind::Reference)
        .flags(
            FnFlags::VOLATILE
                .union(FnFlags::RETURNS_REF)
                .union(FnFlags::NEEDS_ORIGIN),
        )
        .help("Reference given by text.");

/// The text is parsed by the engine (`EvalContext::parse_reference_text`);
/// text that is no reference (`"foo"`, `""`, `"A0"`, or A1 text under
/// R1C1) is `Err:502` (AOO `ScIndirect` ends with `PushIllegalArgument`
/// after the range-name lookup fails). The resolved range is recorded as a
/// dynamic dependency and returned as a reference.
fn indirect(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run_ref(|| {
        let text = a.text(0)?;
        // AOO ScIndirect: R1C1 (CONV_XL_R1C1) when approxFloor(a1) == 0,
        // so 0, FALSE and 0.5 all select R1C1.
        let a1 = a.is_missing(1) || crate::coerce::approx_floor(a.num(1)?) != 0.0;
        let parsed = a.ctx().parse_reference_text(text.trim(), a1);
        let Some(range) = parsed else {
            // AOO `ConvertDoubleRef` also yields a 3D reference.
            let span = a1
                .then(|| a.ctx().parse_reference_text_3d(text.trim()))
                .flatten()
                .ok_or(ErrorKind::InvalidArgument)?;
            let v = crate::coerce::span_3d_value(a.ctx(), span.0, span.1, span.2);
            a.ctx().record_dynamic_span(span.0, span.1, span.2);
            return Ok(v);
        };
        a.ctx().record_dynamic_dep(&range);
        Ok(EvalValue::Range(range))
    })
}

// ----------------------------------------------------------------- OFFSET

const OFFSET_PARAMS: &[Param] = &[
    Param::required("reference", ParamKind::Reference),
    Param::required("rows", ParamKind::Integer),
    Param::required("columns", ParamKind::Integer),
    Param::optional("height", ParamKind::Integer),
    Param::optional("width", ParamKind::Integer),
];

/// `OFFSET(reference; rows; columns [; height [; width]])`.
pub const OFFSET: FunctionDef =
    FunctionDef::new("OFFSET", Category::Spreadsheet, OFFSET_PARAMS, offset)
        .returns(ReturnKind::Reference)
        .flags(
            FnFlags::VOLATILE
                .union(FnFlags::RETURNS_REF)
                .union(FnFlags::REF_ADDRESS_ONLY),
        )
        .help("Reference displaced from a start reference.");

/// Shifts the top-left corner by `rows`/`columns`; height and width default
/// to those of `reference` and must be positive (`Err:502`). A result
/// outside the sheet is `#REF!`.
fn offset(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run_ref(|| {
        // AOO `ScOffset`: a 3D base is `PushIllegalArgument` (Err:502).
        if a.any(0).is_3d() {
            return Err(ErrorKind::InvalidArgument);
        }
        let base = a.range(0)?;
        let dr = a.int(1)?;
        let dc = a.int(2)?;
        let h = a.int_opt(3, i64::from(base.rows()))?;
        let w = a.int_opt(4, i64::from(base.cols()))?;
        if h <= 0 || w <= 0 {
            return Err(ErrorKind::InvalidArgument);
        }
        let r0 = i64::from(base.r0) + dr;
        let c0 = i64::from(base.c0) + dc;
        let (r1, c1) = (r0 + h - 1, c0 + w - 1);
        if r0 < 0 || c0 < 0 || r1 >= i64::from(MAX_ROWS) || c1 >= i64::from(MAX_COLS) {
            return Err(ErrorKind::Ref);
        }
        let range = CellRange::new(base.sheet, r0 as u32, c0 as u32, r1 as u32, c1 as u32);
        a.ctx().record_dynamic_dep(&range);
        Ok(EvalValue::Range(range))
    })
}

// ------------------------------------------------------ AREAS, ROWS, COLS

const REF1: &[Param] = &[Param::required("reference", ParamKind::Reference)];

/// `AREAS(reference)`: the number of areas, so 1 for a range and the number
/// of members of a union (AOO `ScAreas`, `svRefList`).
pub const AREAS: FunctionDef = FunctionDef::new("AREAS", Category::Spreadsheet, REF1, areas)
    .returns(ReturnKind::Number)
    .help("Number of areas in a reference.");
/// `ROWS(array)`: a union is `Err:504` (AOO `ScRows`: `errIllegalParameter`
/// for every stack type but a reference or a matrix); a 3D reference counts
/// its rows once per sheet.
pub const ROWS: FunctionDef = FunctionDef::new("ROWS", Category::Spreadsheet, REF1, rows)
    .returns(ReturnKind::Number)
    .help("Number of rows of a reference or array.");
/// `COLUMNS(array)`: a union is `Err:504`, as for [`ROWS`].
pub const COLUMNS: FunctionDef = FunctionDef::new("COLUMNS", Category::Spreadsheet, REF1, columns)
    .returns(ReturnKind::Number)
    .help("Number of columns of a reference or array.");

fn dims(a: &mut Args<'_>) -> Result<(u32, u32), ErrorKind> {
    let v = a.any(0);
    if v.is_3d() {
        // AOO `ScRows`/`ScColumns`: rows (columns) times the sheets.
        let areas = v.areas().unwrap_or_default();
        let r = areas.first().ok_or(ErrorKind::Ref)?;
        let n = areas.len() as u32;
        return Ok((r.rows().saturating_mul(n), r.cols().saturating_mul(n)));
    }
    if let Some(areas) = v.areas() {
        return match areas {
            [r] => Ok((r.rows(), r.cols())),
            _ => Err(ErrorKind::ParamList),
        };
    }
    match v {
        EvalValue::Array(m) | EvalValue::Scalar(Value::Array(m)) => Ok((m.rows, m.cols)),
        EvalValue::Scalar(Value::Error(e)) => Err(e),
        EvalValue::Scalar(_) => Ok((1, 1)),
        _ => Err(ErrorKind::Value),
    }
}

fn areas(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        // A 3D reference is one area (AOO `ScAreas` counts the tokens).
        let v = a.any(0);
        match v.entries() {
            Some(e) => Ok(Value::Number(e.len() as f64)),
            None => match v {
                EvalValue::Scalar(Value::Error(e)) => Err(e),
                _ => Err(ErrorKind::Value),
            },
        }
    })
}

fn rows(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| Ok(Value::Number(f64::from(dims(&mut a)?.0))))
}

fn columns(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| Ok(Value::Number(f64::from(dims(&mut a)?.1))))
}

// ------------------------------------------------------------ SHEET(S)

const SHEET_PARAMS: &[Param] = &[Param::optional("reference", ParamKind::Reference)];

/// `SHEET([reference])`.
pub const SHEET: FunctionDef =
    FunctionDef::new("SHEET", Category::Spreadsheet, SHEET_PARAMS, sheet)
        .returns(ReturnKind::Number)
        .flags(FnFlags::NEEDS_ORIGIN.union(FnFlags::REF_ADDRESS_ONLY))
        .help("Sheet number of a reference, sheet name or the formula cell.");
/// `SHEETS([reference])`.
pub const SHEETS: FunctionDef =
    FunctionDef::new("SHEETS", Category::Spreadsheet, SHEET_PARAMS, sheets)
        .returns(ReturnKind::Number)
        .flags(FnFlags::REF_ADDRESS_ONLY)
        .help("Number of sheets in a reference or the document.");

/// Resolves a sheet name by asking the engine to parse `<name>.A1`; falls
/// back to treating the text as a full reference.
fn sheet_of_text(ctx: &mut dyn EvalContext, text: &str) -> Option<tally_types::SheetId> {
    let by_name = format!("${}.A1", quote_sheet(text));
    ctx.parse_reference_text(&by_name, true)
        .or_else(|| ctx.parse_reference_text(text, true))
        .map(|r| r.sheet)
}

fn sheet(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let id = match a.any(0) {
            EvalValue::Missing => a.ctx().origin().sheet,
            EvalValue::Range(r) => r.sheet,
            EvalValue::Scalar(Value::Error(e)) => return Err(e),
            EvalValue::Scalar(Value::Text(t)) => sheet_of_text(a.ctx(), &t).ok_or(ErrorKind::NA)?,
            // AOO `ScTable`: the first sheet of a 3D reference.
            v if v.is_3d() => {
                v.areas()
                    .and_then(|x| x.first())
                    .ok_or(ErrorKind::Ref)?
                    .sheet
            }
            _ => return Err(ErrorKind::Value),
        };
        let idx = a.ctx().sheet_index(id).ok_or(ErrorKind::Ref)?;
        Ok(Value::Number(f64::from(idx) + 1.0))
    })
}

/// A reference spans one sheet, a 3D reference its sheets (AOO `ScTables`).
fn sheets(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| match a.any(0) {
        EvalValue::Missing => Ok(Value::Number(f64::from(a.ctx().sheet_count()))),
        EvalValue::Range(_) => Ok(Value::Number(1.0)),
        v if v.is_3d() => Ok(Value::Number(v.areas().map_or(0, <[_]>::len) as f64)),
        EvalValue::Scalar(Value::Error(e)) => Err(e),
        _ => Err(ErrorKind::Value),
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(ADDRESS);
    r.register(INDIRECT);
    r.register(OFFSET);
    r.register(AREAS);
    r.register(ROWS);
    r.register(COLUMNS);
    r.register(SHEET);
    r.register(SHEETS);
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, call_raw, MockArgs, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{CellRange, ErrorKind, Value};

    fn n(x: f64) -> EvalValue {
        EvalValue::number(x)
    }
    fn t(s: &str) -> EvalValue {
        EvalValue::text(s)
    }

    #[test]
    fn address() {
        let mut c = MockEvalContext::new();
        let mut f = |args: Vec<EvalValue>| call(&mut c, "ADDRESS", args);
        assert_eq!(f(vec![n(1.0), n(1.0)]), Value::text("$A$1"));
        assert_eq!(f(vec![n(3.0), n(28.0), n(4.0)]), Value::text("AB3"));
        assert_eq!(f(vec![n(3.0), n(2.0), n(2.0)]), Value::text("B$3"));
        assert_eq!(f(vec![n(3.0), n(2.0), n(3.0)]), Value::text("$B3"));
        assert_eq!(
            f(vec![n(2.0), n(3.0), n(1.0), EvalValue::bool(false)]),
            Value::text("R2C3")
        );
        assert_eq!(
            f(vec![n(2.0), n(3.0), n(4.0), EvalValue::bool(false)]),
            Value::text("R[2]C[3]")
        );
        assert_eq!(
            f(vec![
                n(1.0),
                n(1.0),
                n(1.0),
                EvalValue::Missing,
                t("Sheet2")
            ]),
            Value::text("Sheet2.$A$1")
        );
        assert_eq!(
            f(vec![
                n(1.0),
                n(1.0),
                n(4.0),
                EvalValue::Missing,
                t("My 'S'")
            ]),
            Value::text("'My ''S'''.A1")
        );
        assert_eq!(
            f(vec![n(0.0), n(1.0)]),
            Value::Error(ErrorKind::InvalidArgument)
        );
        assert_eq!(
            f(vec![n(1.0), n(16385.0)]),
            Value::Error(ErrorKind::InvalidArgument)
        );
        // AOO ScAddressFunc: 5..8 alias 1..4, anything else is #VALUE!.
        assert_eq!(f(vec![n(2.0), n(3.0), n(5.0)]), Value::text("$C$2"));
        assert_eq!(f(vec![n(2.0), n(3.0), n(8.0)]), Value::text("C2"));
        assert_eq!(
            f(vec![n(1.0), n(1.0), n(9.0)]),
            Value::Error(ErrorKind::Value)
        );
        assert_eq!(
            f(vec![n(1.0), n(1.0), n(0.0)]),
            Value::Error(ErrorKind::Value)
        );
        let r1c1 = EvalValue::bool(false);
        assert_eq!(
            f(vec![n(1.0), n(1.0), n(1.0), r1c1.clone(), t("Sheet1")]),
            Value::text("Sheet1!R1C1")
        );
        // A zero relative offset has no brackets.
        assert_eq!(
            f(vec![n(0.0), n(3.0), n(4.0), r1c1.clone()]),
            Value::text("RC[3]")
        );
        assert_eq!(f(vec![n(1.0), n(1.0), n(1.0), n(0.5)]), Value::text("R1C1"));
        // Quoting follows CheckTabQuotes.
        assert_eq!(
            f(vec![
                n(1.0),
                n(1.0),
                n(4.0),
                EvalValue::Missing,
                t("2024Q1")
            ]),
            Value::text("2024Q1.A1")
        );
        assert_eq!(
            f(vec![n(1.0), n(1.0), n(4.0), EvalValue::Missing, t("2024")]),
            Value::text("'2024'.A1")
        );
        assert_eq!(
            f(vec![n(1.0), n(1.0), n(4.0), EvalValue::Missing, t("'Q 1'")]),
            Value::text("'Q 1'.A1")
        );
        assert_eq!(
            f(vec![
                n(1.0),
                n(1.0),
                n(4.0),
                EvalValue::Missing,
                t("'file:///a.ods'#$Sheet 1")
            ]),
            Value::text("'file:///a.ods'#$'Sheet 1'.A1")
        );
    }

    #[test]
    fn address_r1c1_relative_is_offset_from_origin() {
        let mut c = MockEvalContext::new();
        c.set_origin(0, "B5");
        let r1c1 = EvalValue::bool(false);
        let mut f = |args: Vec<EvalValue>| call(&mut c, "ADDRESS", args);
        assert_eq!(
            f(vec![n(-2.0), n(-1.0), n(4.0), r1c1.clone()]),
            Value::text("R[-2]C[-1]")
        );
        // Target row -5 + 5 = 0 is outside the grid.
        assert_eq!(
            f(vec![n(-5.0), n(1.0), n(4.0), r1c1]),
            Value::Error(ErrorKind::InvalidArgument)
        );
    }

    #[test]
    fn indirect() {
        let mut c = MockEvalContext::new();
        c.set(0, "B2", 7.0);
        let mut args = MockArgs::new(vec![t("B2")]);
        assert_eq!(
            call_raw(&mut c, "INDIRECT", &mut args),
            EvalValue::Range(CellRange::from_a1(0, "B2").expect("r"))
        );
        assert_eq!(c.dynamic_deps.len(), 1);
        assert_eq!(call(&mut c, "INDIRECT", vec![t("B2")]), Value::Number(7.0));
        // The mock resolves only A1 text, so these show which syntax the
        // a1 argument selected (AOO: R1C1 when approxFloor(a1) == 0).
        assert_eq!(
            call(&mut c, "INDIRECT", vec![t("B2"), EvalValue::bool(true)]),
            Value::Number(7.0)
        );
        assert_eq!(
            call(&mut c, "INDIRECT", vec![t("B2"), EvalValue::number(2.0)]),
            Value::Number(7.0)
        );
        for r1c1 in [
            EvalValue::bool(false),
            EvalValue::number(0.0),
            EvalValue::number(0.5),
        ] {
            assert_eq!(
                call(&mut c, "INDIRECT", vec![t("B2"), r1c1]),
                Value::Error(ErrorKind::InvalidArgument)
            );
        }
        // Review 3 L: unparsable text is Err:502 (AOO PushIllegalArgument).
        assert_eq!(
            call(&mut c, "INDIRECT", vec![t("nonsense")]),
            Value::Error(ErrorKind::InvalidArgument)
        );
        assert_eq!(
            call(&mut c, "INDIRECT", vec![t("")]),
            Value::Error(ErrorKind::InvalidArgument)
        );
        assert_eq!(
            call(&mut c, "INDIRECT", vec![t("A0")]),
            Value::Error(ErrorKind::InvalidArgument)
        );
    }

    #[test]
    fn offset() {
        let mut c = MockEvalContext::new();
        for (i, a) in ["A1", "A2", "A3", "B2"].iter().enumerate() {
            c.set(0, a, (i + 1) as f64 * 10.0);
        }
        let base = c.range(0, "A1");
        let mut args = MockArgs::new(vec![base.clone(), n(1.0), n(1.0)]);
        assert_eq!(
            call_raw(&mut c, "OFFSET", &mut args),
            EvalValue::Range(CellRange::from_a1(0, "B2").expect("r"))
        );
        let mut args = MockArgs::new(vec![base.clone(), n(0.0), n(0.0), n(3.0), n(1.0)]);
        assert_eq!(
            call_raw(&mut c, "OFFSET", &mut args),
            EvalValue::Range(CellRange::from_a1(0, "A1:A3").expect("r"))
        );
        // Size defaults to the reference's.
        let r = c.range(0, "A1:A2");
        let mut args = MockArgs::new(vec![r, n(1.0), n(0.0)]);
        assert_eq!(
            call_raw(&mut c, "OFFSET", &mut args),
            EvalValue::Range(CellRange::from_a1(0, "A2:A3").expect("r"))
        );
        let f = |c: &mut MockEvalContext, v: Vec<EvalValue>| call(c, "OFFSET", v);
        assert_eq!(
            f(&mut c, vec![base.clone(), n(-1.0), n(0.0)]),
            Value::Error(ErrorKind::Ref)
        );
        assert_eq!(
            f(&mut c, vec![base.clone(), n(0.0), n(0.0), n(0.0)]),
            Value::Error(ErrorKind::InvalidArgument)
        );
        assert_eq!(
            f(&mut c, vec![base.clone(), n(0.0), n(0.0), n(1.0), n(-1.0)]),
            Value::Error(ErrorKind::InvalidArgument)
        );
        assert_eq!(
            f(&mut c, vec![n(1.0), n(0.0), n(0.0)]),
            Value::Error(ErrorKind::Value)
        );
        assert_eq!(f(&mut c, vec![base, n(1.0), n(1.0)]), Value::Number(40.0));
    }

    #[test]
    fn shape_functions() {
        let mut c = MockEvalContext::new();
        let r = c.range(0, "A1:C4");
        assert_eq!(call(&mut c, "ROWS", vec![r.clone()]), Value::Number(4.0));
        assert_eq!(call(&mut c, "COLUMNS", vec![r.clone()]), Value::Number(3.0));
        assert_eq!(call(&mut c, "AREAS", vec![r]), Value::Number(1.0));
        assert_eq!(call(&mut c, "ROWS", vec![n(5.0)]), Value::Number(1.0));
        assert_eq!(
            call(&mut c, "AREAS", vec![n(5.0)]),
            Value::Error(ErrorKind::Value)
        );
        assert_eq!(
            call(&mut c, "ROWS", vec![EvalValue::error(ErrorKind::NA)]),
            Value::Error(ErrorKind::NA)
        );
    }

    /// Review 3 M1: AREAS counts a union's members; ROWS/COLUMNS reject it
    /// with Err:504 as AOO does.
    #[cfg(feature = "ref-union")]
    #[test]
    fn shape_functions_on_unions() {
        let mut c = MockEvalContext::new();
        let u = c.union(0, &["A1:A2", "B1:B2", "D5"]);
        assert_eq!(call(&mut c, "AREAS", vec![u.clone()]), Value::Number(3.0));
        assert_eq!(
            call(&mut c, "ROWS", vec![u.clone()]),
            Value::Error(ErrorKind::ParamList)
        );
        assert_eq!(
            call(&mut c, "COLUMNS", vec![u.clone()]),
            Value::Error(ErrorKind::ParamList)
        );
        // A one-member union is a plain range.
        let one = c.union(0, &["A1:A2"]);
        assert_eq!(one, c.range(0, "A1:A2"));
        assert_eq!(call(&mut c, "AREAS", vec![one]), Value::Number(1.0));
        assert_eq!(
            EvalValue::union(Vec::new()),
            EvalValue::error(ErrorKind::Ref)
        );
        // OFFSET takes a single-area reference only.
        assert_eq!(
            call(&mut c, "OFFSET", vec![u, n(1.0), n(0.0)]),
            Value::Error(ErrorKind::Value)
        );
    }

    #[test]
    fn sheet_functions() {
        let mut c = MockEvalContext::new();
        c.set(2, "A1", 1.0);
        assert_eq!(c.sheets, 3);
        c.set_origin(1, "A1");
        assert_eq!(call(&mut c, "SHEET", vec![]), Value::Number(2.0));
        let r = c.range(2, "A1");
        assert_eq!(call(&mut c, "SHEET", vec![r.clone()]), Value::Number(3.0));
        assert_eq!(call(&mut c, "SHEETS", vec![]), Value::Number(3.0));
        assert_eq!(call(&mut c, "SHEETS", vec![r]), Value::Number(1.0));
        assert_eq!(
            call(&mut c, "SHEET", vec![n(1.0)]),
            Value::Error(ErrorKind::Value)
        );
        // The mock cannot resolve sheet names, so a name is #N/A there.
        assert_eq!(
            call(&mut c, "SHEET", vec![t("Nope")]),
            Value::Error(ErrorKind::NA)
        );
    }
}
