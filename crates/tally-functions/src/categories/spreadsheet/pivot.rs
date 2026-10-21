//! GETPIVOTDATA.
//!
//! The function only prepares its arguments; the lookup itself is
//! [`EvalContext::pivot_data`], which the engine implements over its
//! DataPilot tables. The default implementation returns `None`, so without
//! an engine every call is `#REF!`, exactly what AOO's `ScGetPivotData`
//! (`sc/source/core/tool/interpr2.cxx`) answers when no table contains the
//! reference.

use tally_types::{ErrorKind, Value};

use crate::coerce::value_to_text;
use crate::registry::{Category, FnFlags, FunctionDef, Param, ParamKind, Registry, ReturnKind};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const PARAMS: &[Param] = &[
    Param::required("data_field", ParamKind::Lazy),
    Param::required("pivot_table", ParamKind::Lazy),
    Param::repeat("field", ParamKind::Lazy),
    Param::repeat("item", ParamKind::Lazy),
];

/// `GETPIVOTDATA(data_field; pivot_table [; field; item ...])`, and the
/// Excel 2000 form `GETPIVOTDATA(pivot_table; "constraints")`.
pub const GETPIVOTDATA: FunctionDef =
    FunctionDef::new("GETPIVOTDATA", Category::Spreadsheet, PARAMS, getpivotdata)
        .arity(2, 30)
        .returns(ReturnKind::Any)
        .flags(FnFlags::LAZY_ARGS)
        .help("Extracts a value from a pivot table (DataPilot).");

/// Both syntaxes end in one [`EvalContext::pivot_data`] call with the
/// top-left cell of the table reference, the data field name and the
/// `(field, item)` filters:
///
/// - AOO form: `data_field` is text, then any number of `field; item`
///   pairs (an odd argument count is `#REF!`, as in AOO). Items may be
///   numbers; they are passed in General format.
/// - Excel 2000 form (exactly two arguments, a reference first and a
///   non-reference second): the string is split into `Field[Item]` groups
///   (the word before `[` is the field, the bracket contents the item,
///   which may contain spaces) and the remaining words, joined by single
///   spaces, are the data field text. AOO (`ScDPObject::ParseFilters`)
///   matches that text greedily against the table's data field names and
///   item names, so the engine resolves it the same way.
///
/// Arguments that evaluate to errors propagate. The hook returning `None`
/// (no pivot table there, unknown field or item) is `#REF!`.
fn getpivotdata(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let n = a.count();
        if n % 2 == 1 {
            return Err(ErrorKind::Ref);
        }
        let excel_form = n == 2
            && matches!(a.any(0), EvalValue::Range(_))
            && !matches!(a.any(1), EvalValue::Range(_));
        let table_arg = usize::from(!excel_form);
        let table = match a.any(table_arg) {
            EvalValue::Range(r) => r,
            EvalValue::Scalar(Value::Error(e)) => return Err(e),
            _ => return Err(ErrorKind::Ref),
        };
        let at = table.start();
        let (field, filters) = if excel_form {
            parse_constraints(&a.text(1)?)?
        } else {
            let field = a.text(0)?.to_string();
            let mut filters = Vec::with_capacity((n - 2) / 2);
            for i in (2..n).step_by(2) {
                let name = a.text(i)?.to_string();
                let item = value_to_text(&a.value(i + 1))?.to_string();
                filters.push((name, item));
            }
            (field, filters)
        };
        let hit = if excel_form {
            a.ctx().pivot_data_excel(at, &field, &filters)
        } else {
            a.ctx().pivot_data(at, &field, &filters)
        };
        match hit {
            Some(Value::Error(e)) => Err(e),
            Some(v) => Ok(v),
            None => Err(ErrorKind::Ref),
        }
    })
}

/// Splits an Excel 2000 constraint string into `Field[Item]` filters and
/// the remaining data-field text. Unbalanced brackets or a bracket without
/// a field name are `#REF!`.
fn parse_constraints(s: &str) -> Result<(String, Vec<(String, String)>), ErrorKind> {
    let mut filters = Vec::new();
    let mut words: Vec<&str> = Vec::new();
    let mut rest = s;
    while let Some(open) = rest.find('[') {
        let close = rest[open + 1..]
            .find(']')
            .map(|i| open + 1 + i)
            .ok_or(ErrorKind::Ref)?;
        let head = rest[..open].trim_end();
        let (before, field) = head.rsplit_once(char::is_whitespace).unwrap_or(("", head));
        if field.is_empty() {
            return Err(ErrorKind::Ref);
        }
        words.extend(before.split_whitespace());
        filters.push((field.to_owned(), rest[open + 1..close].trim().to_owned()));
        rest = &rest[close + 1..];
    }
    if rest.contains(']') {
        return Err(ErrorKind::Ref);
    }
    words.extend(rest.split_whitespace());
    Ok((words.join(" "), filters))
}

pub(super) fn register(r: &mut Registry) {
    r.register(GETPIVOTDATA);
}

#[cfg(test)]
mod tests {
    use super::parse_constraints;
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{CellAddr, ErrorKind, Value};

    fn n(x: f64) -> EvalValue {
        EvalValue::number(x)
    }
    fn t(s: &str) -> EvalValue {
        EvalValue::text(s)
    }

    /// A mock pivot anchored at `A1`: data field "Sales", one row field
    /// "Region" with items North/South and a column field "Year".
    fn pivot_ctx() -> MockEvalContext {
        let mut c = MockEvalContext::new();
        const ROWS: [(&str, &str, f64); 4] = [
            ("North", "2025", 10.0),
            ("North", "2026", 20.0),
            ("South", "2025", 5.0),
            ("South", "2026", 7.0),
        ];
        c.set_pivot_data(|at, field, filters| {
            if at != CellAddr::new(0, 0, 0) || field != "Sales" {
                return None;
            }
            if filters.iter().any(|(f, _)| f != "Region" && f != "Year") {
                return None;
            }
            let total: f64 = ROWS
                .iter()
                .filter(|(region, year, _)| {
                    filters
                        .iter()
                        .all(|(f, item)| item == if f == "Region" { region } else { year })
                })
                .map(|(_, _, v)| v)
                .sum();
            Some(Value::Number(total))
        });
        c
    }

    #[test]
    fn aoo_form_total_and_filters() {
        let mut c = pivot_ctx();
        let r = c.range(0, "A1:D5");
        assert_eq!(
            call(&mut c, "GETPIVOTDATA", vec![t("Sales"), r.clone()]),
            Value::Number(42.0)
        );
        assert_eq!(
            call(
                &mut c,
                "GETPIVOTDATA",
                vec![t("Sales"), r.clone(), t("Region"), t("North")]
            ),
            Value::Number(30.0)
        );
        // Numeric items arrive in General format.
        assert_eq!(
            call(
                &mut c,
                "GETPIVOTDATA",
                vec![
                    t("Sales"),
                    r.clone(),
                    t("Region"),
                    t("South"),
                    t("Year"),
                    n(2026.0)
                ]
            ),
            Value::Number(7.0)
        );
        let last = c.pivot_calls.last().cloned();
        assert_eq!(
            last,
            Some((
                CellAddr::new(0, 0, 0),
                "Sales".to_owned(),
                vec![
                    ("Region".to_owned(), "South".to_owned()),
                    ("Year".to_owned(), "2026".to_owned())
                ]
            ))
        );
        // Unknown field: the hook declines.
        assert_eq!(
            call(
                &mut c,
                "GETPIVOTDATA",
                vec![t("Sales"), r, t("Colour"), t("Red")]
            ),
            Value::Error(ErrorKind::Ref)
        );
    }

    #[test]
    fn excel_form() {
        let mut c = pivot_ctx();
        let r = c.range(0, "A1");
        assert_eq!(
            call(&mut c, "GETPIVOTDATA", vec![r.clone(), t("Sales")]),
            Value::Number(42.0)
        );
        assert_eq!(
            call(
                &mut c,
                "GETPIVOTDATA",
                vec![r.clone(), t("Sales Region[North] Year[2025]")]
            ),
            Value::Number(10.0)
        );
        assert_eq!(
            call(&mut c, "GETPIVOTDATA", vec![r, t("Sales [North]")]),
            Value::Error(ErrorKind::Ref)
        );
    }

    #[test]
    fn constraint_parser() {
        assert_eq!(
            parse_constraints("Sum - Sales Region[North West] Year[2026]"),
            Ok((
                "Sum - Sales".to_owned(),
                vec![
                    ("Region".to_owned(), "North West".to_owned()),
                    ("Year".to_owned(), "2026".to_owned())
                ]
            ))
        );
        assert_eq!(parse_constraints("  "), Ok((String::new(), vec![])));
        assert_eq!(parse_constraints("Region[North"), Err(ErrorKind::Ref));
        assert_eq!(parse_constraints("Region North]"), Err(ErrorKind::Ref));
        assert_eq!(parse_constraints("[North]"), Err(ErrorKind::Ref));
    }

    #[test]
    fn no_pivot_is_ref_error() {
        let mut c = MockEvalContext::new();
        let r = c.range(0, "A1:C5");
        assert_eq!(
            call(&mut c, "GETPIVOTDATA", vec![t("Sales"), r.clone()]),
            Value::Error(ErrorKind::Ref)
        );
        // Another anchor than the mock pivot's.
        let mut p = pivot_ctx();
        let elsewhere = p.range(0, "H10:K20");
        assert_eq!(
            call(&mut p, "GETPIVOTDATA", vec![t("Sales"), elsewhere]),
            Value::Error(ErrorKind::Ref)
        );
        // Odd argument count, non-reference table, argument errors.
        assert_eq!(
            call(
                &mut c,
                "GETPIVOTDATA",
                vec![t("Sales"), r.clone(), t("Region")]
            ),
            Value::Error(ErrorKind::Ref)
        );
        assert_eq!(
            call(&mut c, "GETPIVOTDATA", vec![t("Sales"), n(1.0)]),
            Value::Error(ErrorKind::Ref)
        );
        assert_eq!(
            call(
                &mut c,
                "GETPIVOTDATA",
                vec![EvalValue::error(ErrorKind::Div0), r.clone()]
            ),
            Value::Error(ErrorKind::Div0)
        );
        assert_eq!(
            call(
                &mut c,
                "GETPIVOTDATA",
                vec![t("Sales"), r, t("Region"), EvalValue::error(ErrorKind::NA)]
            ),
            Value::Error(ErrorKind::NA)
        );
        assert_eq!(
            call(&mut c, "GETPIVOTDATA", vec![t("Sales")]),
            Value::Error(ErrorKind::MissingVariable2)
        );
    }
}
