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

/// Quotes a sheet name for Calc reference syntax when it is not a plain identifier.
fn quote_sheet(name: &str) -> String {
    let plain = !name.is_empty()
        && !name.starts_with(|c: char| c.is_ascii_digit())
        && name.chars().all(|c| c.is_alphanumeric() || c == '_');
    if plain {
        name.to_string()
    } else {
        format!("'{}'", name.replace('\'', "''"))
    }
}

/// Calc syntax gives `$Sheet1.$A$1` (the sheet is always absolute, as in AOO
/// and LibreOffice); `abs` 1 = `$A$1`, 2 = `A$1`, 3 = `$A1`, 4 = `A1`.
/// With `a1` FALSE the Excel R1C1 form is built (`R1C1`, `R1C[2]`, `R[1]C2`,
/// `R[1]C[2]`) and the sheet is written `Sheet.` without `$`.
/// Row/column outside the grid or `abs` outside 1..=4 is `Err:502`.
fn address(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let row = a.int(0)?;
        let col = a.int(1)?;
        let abs = a.int_opt(2, 1)?;
        let a1 = a.bool_opt(3, true)?;
        let sheet = if a.is_missing(4) {
            None
        } else {
            Some(a.text(4)?).filter(|s| !s.is_empty())
        };
        if !(1..=i64::from(MAX_ROWS)).contains(&row)
            || !(1..=i64::from(MAX_COLS)).contains(&col)
            || !(1..=4).contains(&abs)
        {
            return Err(ErrorKind::InvalidArgument);
        }
        let (abs_row, abs_col) = match abs {
            1 => (true, true),
            2 => (true, false),
            3 => (false, true),
            _ => (false, false),
        };
        let cell = if a1 {
            format!(
                "{}{}{}{}",
                if abs_col { "$" } else { "" },
                col_to_letters((col - 1) as u32),
                if abs_row { "$" } else { "" },
                row
            )
        } else {
            let r = if abs_row {
                format!("R{row}")
            } else {
                format!("R[{row}]")
            };
            let c = if abs_col {
                format!("C{col}")
            } else {
                format!("C[{col}]")
            };
            format!("{r}{c}")
        };
        let text = match sheet {
            None => cell,
            Some(s) if a1 => format!("${}.{cell}", quote_sheet(&s)),
            Some(s) => format!("{}.{cell}", quote_sheet(&s)),
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
/// failure is `#REF!`. The resolved range is recorded as a dynamic
/// dependency and returned as a reference.
fn indirect(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run_ref(|| {
        let text = a.text(0)?;
        let a1 = a.bool_opt(1, true)?;
        let range = a
            .ctx()
            .parse_reference_text(text.trim(), a1)
            .ok_or(ErrorKind::Ref)?;
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

/// `AREAS(reference)`: unions are not supported, so always 1 for a reference.
pub const AREAS: FunctionDef = FunctionDef::new("AREAS", Category::Spreadsheet, REF1, areas)
    .returns(ReturnKind::Number)
    .help("Number of areas in a reference.");
/// `ROWS(array)`.
pub const ROWS: FunctionDef = FunctionDef::new("ROWS", Category::Spreadsheet, REF1, rows)
    .returns(ReturnKind::Number)
    .help("Number of rows of a reference or array.");
/// `COLUMNS(array)`.
pub const COLUMNS: FunctionDef = FunctionDef::new("COLUMNS", Category::Spreadsheet, REF1, columns)
    .returns(ReturnKind::Number)
    .help("Number of columns of a reference or array.");

fn dims(a: &mut Args<'_>) -> Result<(u32, u32), ErrorKind> {
    match a.any(0) {
        EvalValue::Range(r) => Ok((r.rows(), r.cols())),
        EvalValue::Array(m) | EvalValue::Scalar(Value::Array(m)) => Ok((m.rows, m.cols)),
        EvalValue::Scalar(Value::Error(e)) => Err(e),
        EvalValue::Scalar(_) => Ok((1, 1)),
        EvalValue::Missing => Err(ErrorKind::Value),
    }
}

fn areas(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| match a.any(0) {
        EvalValue::Range(_) => Ok(Value::Number(1.0)),
        EvalValue::Scalar(Value::Error(e)) => Err(e),
        _ => Err(ErrorKind::Value),
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
            _ => return Err(ErrorKind::Value),
        };
        let idx = a.ctx().sheet_index(id).ok_or(ErrorKind::Ref)?;
        Ok(Value::Number(f64::from(idx) + 1.0))
    })
}

/// 3D references are not representable, so a reference always spans one sheet.
fn sheets(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| match a.any(0) {
        EvalValue::Missing => Ok(Value::Number(f64::from(a.ctx().sheet_count()))),
        EvalValue::Range(_) => Ok(Value::Number(1.0)),
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
            Value::text("$Sheet2.$A$1")
        );
        assert_eq!(
            f(vec![
                n(1.0),
                n(1.0),
                n(4.0),
                EvalValue::Missing,
                t("My 'S'")
            ]),
            Value::text("$'My ''S'''.A1")
        );
        assert_eq!(
            f(vec![n(0.0), n(1.0)]),
            Value::Error(ErrorKind::InvalidArgument)
        );
        assert_eq!(
            f(vec![n(1.0), n(16385.0)]),
            Value::Error(ErrorKind::InvalidArgument)
        );
        assert_eq!(
            f(vec![n(1.0), n(1.0), n(5.0)]),
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
        assert_eq!(
            call(&mut c, "INDIRECT", vec![t("nonsense")]),
            Value::Error(ErrorKind::Ref)
        );
        assert_eq!(
            call(&mut c, "INDIRECT", vec![t("")]),
            Value::Error(ErrorKind::Ref)
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
