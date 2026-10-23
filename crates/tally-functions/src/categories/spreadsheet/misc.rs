//! CHOOSE, LOOKUP, ERRORTYPE, HYPERLINK, DDE and STYLE.

use tally_types::{ErrorKind, Value};

use super::lookup::{make_key, Mode, Table};
use crate::coerce::scalar_value;
use crate::registry::{Category, FnFlags, FunctionDef, Param, ParamKind, Registry, ReturnKind};
use crate::{run, run_ref, ArgList, Args, EvalContext, EvalValue};

// ----------------------------------------------------------------- CHOOSE

const CHOOSE_PARAMS: &[Param] = &[
    Param::required("index", ParamKind::Number),
    Param::repeat("value", ParamKind::Lazy),
];

/// `CHOOSE(index; value1; ...; value30)`.
pub const CHOOSE: FunctionDef =
    FunctionDef::new("CHOOSE", Category::Spreadsheet, CHOOSE_PARAMS, choose)
        .arity(2, 31)
        .returns(ReturnKind::Any)
        .flags(FnFlags::LAZY_ARGS.union(FnFlags::RETURNS_REF))
        .help("Selects a value from up to 30 values by index.");

/// The index is truncated; outside 1..=count it is `Err:502` (AOO
/// `ScChoose`: `PushIllegalArgument`; Excel gives `#VALUE!`). Only the chosen
/// argument is evaluated and references pass through unchanged.
fn choose(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run_ref(|| {
        let idx = a.num(0)?.floor();
        let count = a.count() as f64 - 1.0;
        if !(1.0..=count).contains(&idx) {
            return Err(ErrorKind::InvalidArgument);
        }
        Ok(match a.any(idx as usize) {
            EvalValue::Missing => EvalValue::number(0.0),
            v => v,
        })
    })
}

// ----------------------------------------------------------------- LOOKUP

const LOOKUP_PARAMS: &[Param] = &[
    Param::required("search_criterion", ParamKind::Any),
    Param::required("search_vector", ParamKind::Reference),
    Param::optional("result_vector", ParamKind::Reference),
];

/// `LOOKUP(search; search_vector [; result_vector])`.
pub const LOOKUP: FunctionDef =
    FunctionDef::new("LOOKUP", Category::Spreadsheet, LOOKUP_PARAMS, lookup)
        .returns(ReturnKind::Any)
        .help("Looks up a value in a sorted vector or array.");

/// Sorted-ascending search for the last value `<=` the key (never found:
/// `#N/A`). Vector form: the result comes from the same position in the
/// result vector. Array form (no result vector): a table with more columns
/// than rows is searched along its first row and answers from its last row;
/// otherwise the first column is searched and the last column answers.
fn lookup(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let key = make_key(a.value(0))?;
        let table = Table::from_eval(a.any(1))?;
        let results = if a.is_missing(2) {
            None
        } else {
            Some(Table::from_eval(a.any(2))?)
        };
        let wide = table.cols() > table.rows();
        let line = table.search_line(a.ctx(), !wide, 0);
        let settings = a.ctx().settings().clone();
        let pos = line
            .search(&key, Mode::Ascending, &settings)
            .ok_or(ErrorKind::NA)?;
        let v = match results {
            Some(rt) => {
                let (r, c) = if rt.rows() == 1 { (0, pos) } else { (pos, 0) };
                if r >= rt.rows() || c >= rt.cols() {
                    return Err(ErrorKind::NA);
                }
                rt.get(a.ctx(), r, c)
            }
            None if wide => table.get(a.ctx(), table.rows() - 1, pos),
            None => table.get(a.ctx(), pos, table.cols() - 1),
        };
        Ok(scalar_value(&EvalValue::from_value(v), a.ctx()))
    })
}

// -------------------------------------------------------------- ERRORTYPE

const ANY1: &[Param] = &[Param::required("reference", ParamKind::Any)];

/// `ERRORTYPE(reference)`.
pub const ERRORTYPE: FunctionDef =
    FunctionDef::new("ERRORTYPE", Category::Spreadsheet, ANY1, errortype)
        .odf("ORG.OPENOFFICE.ERRORTYPE")
        .returns(ReturnKind::Number)
        .help("Number of an error value (Err:NNN code); #N/A if there is no error.");

/// Returns the OpenOffice error code (`Err:502` is 502, `#DIV/0!` 532,
/// `#N/A` 32767). LibreOffice instead returns Excel's 1..7 for the
/// standard errors. A non-error is `#N/A`.
fn errortype(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| match a.value(0) {
        Value::Error(e) => Ok(Value::Number(f64::from(e.code()))),
        _ => Err(ErrorKind::NA),
    })
}

// -------------------------------------------------------------- HYPERLINK

const HYPERLINK_PARAMS: &[Param] = &[
    Param::required("url", ParamKind::Text),
    Param::optional("cell_text", ParamKind::Any),
];

/// `HYPERLINK(url [; cell_text])`.
pub const HYPERLINK: FunctionDef = FunctionDef::new(
    "HYPERLINK",
    Category::Spreadsheet,
    HYPERLINK_PARAMS,
    hyperlink,
)
.returns(ReturnKind::Any)
.help("Displays the cell text (or the URL); the UI makes it clickable.");

/// Returns the second argument's value (number or text) if given, else the
/// URL text. No link metadata is attached (the UI layer's job).
fn hyperlink(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let url = a.text(0)?;
        if a.is_missing(1) {
            return Ok(Value::Text(url));
        }
        match a.value(1) {
            Value::Empty => Ok(Value::text("")),
            v => Ok(v),
        }
    })
}

// -------------------------------------------------------------------- DDE

const DDE_PARAMS: &[Param] = &[
    Param::required("server", ParamKind::Text),
    Param::required("file", ParamKind::Text),
    Param::required("range", ParamKind::Text),
    Param::optional("mode", ParamKind::Number),
];

/// `DDE(server; file; range [; mode])`: DDE links are disabled by policy, so
/// the result is `#N/A` (what AOO shows when no DDE server answers).
pub const DDE: FunctionDef = FunctionDef::new("DDE", Category::Spreadsheet, DDE_PARAMS, dde)
    .returns(ReturnKind::Any)
    .help("Dynamic data exchange (disabled; always #N/A).");

fn dde(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        a.text(0)?;
        a.text(1)?;
        a.text(2)?;
        Err(ErrorKind::NA)
    })
}

// ------------------------------------------------------------------ STYLE

const STYLE_PARAMS: &[Param] = &[
    Param::required("style", ParamKind::Text),
    Param::optional("time", ParamKind::Number),
    Param::optional("style2", ParamKind::Text),
];

/// `STYLE(style [; time [; style2]])`: AOO applies a cell style as a side
/// effect and returns 0. A function cannot change formatting here, so only
/// the 0 is returned.
pub const STYLE: FunctionDef =
    FunctionDef::new("STYLE", Category::Spreadsheet, STYLE_PARAMS, style)
        .odf("ORG.OPENOFFICE.STYLE")
        .returns(ReturnKind::Number)
        .help("Applies a cell style (no effect here; returns 0).");

fn style(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        a.text(0)?;
        a.num_opt(1, 0.0)?;
        a.text_opt(2, "")?;
        Ok(Value::Number(0.0))
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(CHOOSE);
    r.register(LOOKUP);
    r.register(ERRORTYPE);
    r.register(HYPERLINK);
    r.register(DDE);
    r.register(STYLE);
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, call_raw, MockArgs, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{ErrorKind, Value};

    fn n(x: f64) -> EvalValue {
        EvalValue::number(x)
    }
    fn t(s: &str) -> EvalValue {
        EvalValue::text(s)
    }

    #[test]
    fn choose_is_lazy() {
        let mut c = MockEvalContext::new();
        let mut args = MockArgs::new(vec![n(2.0), n(10.0), n(20.0), n(30.0)]);
        let v = call_raw(&mut c, "CHOOSE", &mut args);
        assert_eq!(v, n(20.0));
        assert!(args.was_evaluated(2));
        assert!(!args.was_evaluated(1));
        assert!(!args.was_evaluated(3));
    }

    #[test]
    fn choose_ref_and_errors() {
        let mut c = MockEvalContext::new();
        let r = c.range(0, "A1:A3");
        let mut args = MockArgs::new(vec![n(2.7), t("x"), r.clone()]);
        assert_eq!(call_raw(&mut c, "CHOOSE", &mut args), r);
        let bad = Value::Error(ErrorKind::InvalidArgument);
        assert_eq!(call(&mut c, "CHOOSE", vec![n(0.0), n(1.0)]), bad);
        assert_eq!(call(&mut c, "CHOOSE", vec![n(3.0), n(1.0), n(2.0)]), bad);
        assert_eq!(
            call(
                &mut c,
                "CHOOSE",
                vec![EvalValue::error(ErrorKind::NA), n(1.0)]
            ),
            Value::Error(ErrorKind::NA)
        );
        assert_eq!(
            call(&mut c, "CHOOSE", vec![n(1.0)]),
            Value::Error(ErrorKind::MissingVariable2)
        );
    }

    #[test]
    fn lookup_vector() {
        let mut c = MockEvalContext::new();
        for (i, (k, v)) in [(1.0, "a"), (3.0, "b"), (5.0, "c")].iter().enumerate() {
            c.set(0, &format!("A{}", i + 1), *k);
            c.set(0, &format!("C{}", i + 1), *v);
        }
        let s = c.range(0, "A1:A3");
        let r = c.range(0, "C1:C3");
        let f =
            |c: &mut MockEvalContext, k: f64| call(c, "LOOKUP", vec![n(k), s.clone(), r.clone()]);
        assert_eq!(f(&mut c, 4.0), Value::text("b"));
        assert_eq!(f(&mut c, 99.0), Value::text("c"));
        assert_eq!(f(&mut c, 0.0), Value::Error(ErrorKind::NA));
        // Short result vector.
        let short = c.range(0, "C1:C2");
        assert_eq!(
            call(&mut c, "LOOKUP", vec![n(5.0), s, short]),
            Value::Error(ErrorKind::NA)
        );
    }

    #[test]
    fn lookup_array_form() {
        let mut c = MockEvalContext::new();
        c.set(0, "A1", 1.0).set(0, "B1", "one");
        c.set(0, "A2", 2.0).set(0, "B2", "two");
        let tall = c.range(0, "A1:B2");
        assert_eq!(
            call(&mut c, "LOOKUP", vec![n(2.0), tall]),
            Value::text("two")
        );
        c.set(0, "E1", 10.0).set(0, "F1", 20.0).set(0, "G1", 30.0);
        c.set(0, "E2", "x").set(0, "F2", "y").set(0, "G2", "z");
        let wide = c.range(0, "E1:G2");
        assert_eq!(
            call(&mut c, "LOOKUP", vec![n(25.0), wide]),
            Value::text("y")
        );
    }

    #[test]
    fn errortype() {
        let mut c = MockEvalContext::new();
        let e =
            |c: &mut MockEvalContext, k: ErrorKind| call(c, "ERRORTYPE", vec![EvalValue::error(k)]);
        assert_eq!(e(&mut c, ErrorKind::InvalidArgument), Value::Number(502.0));
        assert_eq!(e(&mut c, ErrorKind::Div0), Value::Number(532.0));
        assert_eq!(e(&mut c, ErrorKind::NA), Value::Number(32767.0));
        assert_eq!(
            call(&mut c, "ERRORTYPE", vec![n(1.0)]),
            Value::Error(ErrorKind::NA)
        );
    }

    #[test]
    fn hyperlink() {
        let mut c = MockEvalContext::new();
        assert_eq!(
            call(&mut c, "HYPERLINK", vec![t("http://x")]),
            Value::text("http://x")
        );
        assert_eq!(
            call(&mut c, "HYPERLINK", vec![t("http://x"), t("go")]),
            Value::text("go")
        );
        assert_eq!(
            call(&mut c, "HYPERLINK", vec![t("http://x"), n(5.0)]),
            Value::Number(5.0)
        );
        let r = c.range(0, "A1");
        assert_eq!(call(&mut c, "HYPERLINK", vec![t("u"), r]), Value::text(""));
    }

    #[test]
    fn unsupported() {
        let mut c = MockEvalContext::new();
        assert_eq!(
            call(&mut c, "DDE", vec![t("s"), t("f"), t("r")]),
            Value::Error(ErrorKind::NA)
        );
        assert_eq!(call(&mut c, "STYLE", vec![t("Good")]), Value::Number(0.0));
    }
}
