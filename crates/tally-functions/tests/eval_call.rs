//! `eval_call`: preparation per ParamKind, laziness, implicit intersection
//! and array lifting.

#![allow(clippy::unwrap_used)]

mod common;
use common::*;

use tally_functions::{
    call_def, run, ArgList, Args, Category, EvalContext, FunctionDef, Param, ParamKind,
};

#[test]
fn if_does_not_evaluate_the_other_branch() {
    let mut c = MockEvalContext::new();
    let mut args = MockArgs::new(vec![b(true), n(1.0), e(ErrorKind::Div0)]);
    assert_eq!(call_with(&mut c, "IF", &mut args), num(1.0));
    assert!(args.was_evaluated(0) && args.was_evaluated(1));
    assert!(!args.was_evaluated(2), "else branch must stay unevaluated");

    let mut args = MockArgs::new(vec![b(false), e(ErrorKind::Div0), t("ok")]);
    assert_eq!(call_with(&mut c, "IF", &mut args), txt("ok"));
    assert!(!args.was_evaluated(1));
}

#[test]
fn non_lazy_functions_evaluate_each_arg_once() {
    let mut c = MockEvalContext::new();
    let mut args = MockArgs::new(vec![n(1.0), n(2.0), n(3.0)]);
    assert_eq!(call_with(&mut c, "SUM", &mut args), num(6.0));
    assert_eq!(args.evaluated, vec![0, 1, 2]);
}

#[test]
fn missing_args_are_not_evaluated() {
    let mut c = MockEvalContext::new();
    let mut args = MockArgs::new(vec![n(2.5), missing()]);
    assert_eq!(call_with(&mut c, "ROUND", &mut args), num(3.0));
    assert_eq!(args.evaluated, vec![0]);
}

#[test]
fn scalar_params_get_implicit_intersection() {
    let mut c = MockEvalContext::new();
    c.set(0, "A1", 1.25).set(0, "A2", 2.75).set(0, "A3", 3.5);
    c.set_origin(0, "C2");
    let col = c.range(0, "A1:A3");
    assert_eq!(call(&mut c, "ROUND", vec![col.clone()]), num(3.0));
    c.set_origin(0, "C3");
    assert_eq!(call(&mut c, "ROUND", vec![col.clone()]), num(4.0));
    c.set_origin(0, "C7");
    assert_eq!(
        call(&mut c, "ROUND", vec![col.clone()]),
        err(ErrorKind::Value)
    );
    // NumberSeq params are not intersected.
    assert_eq!(call(&mut c, "SUM", vec![col]), num(7.5));
}

#[test]
fn returned_references_pass_through() {
    let mut c = MockEvalContext::new();
    c.set(0, "A1", 1.0).set(0, "A2", 2.0);
    let r = c.range(0, "A1:A2");
    let mut args = MockArgs::new(vec![b(true), r.clone()]);
    assert_eq!(call_raw(&mut c, "IF", &mut args), r);
}

fn lift_probe(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| Ok(Value::Number(a.num(0)? * 10.0 + a.num(1)?)))
}

const PROBE_PARAMS: &[Param] = &[
    Param::required("a", ParamKind::Number),
    Param::required("b", ParamKind::Number),
];
const PROBE: FunctionDef =
    FunctionDef::new("PROBE", Category::Mathematical, PROBE_PARAMS, lift_probe);

fn probe(c: &mut MockEvalContext, x: EvalValue, y: EvalValue) -> EvalValue {
    call_def(&PROBE, c, &mut MockArgs::new(vec![x, y]))
}

fn matrix(v: &EvalValue) -> (u32, u32, Vec<Value>) {
    match v {
        EvalValue::Array(m) => (m.rows, m.cols, m.data.clone()),
        other => panic!("expected array, got {other:?}"),
    }
}

#[test]
fn array_lifting_only_in_array_context() {
    let mut c = MockEvalContext::new();
    let col = arr(3, 1, vec![num(1.0), num(2.0), num(3.0)]);
    // Normal context: inline array → top-left.
    assert_eq!(probe(&mut c, col.clone(), n(5.0)), n(15.0));
    c.array_context = true;
    let (r, k, data) = matrix(&probe(&mut c, col.clone(), n(5.0)));
    assert_eq!((r, k), (3, 1));
    assert_eq!(data, vec![num(15.0), num(25.0), num(35.0)]);
}

#[test]
fn array_lifting_broadcast_and_na_padding() {
    let mut c = MockEvalContext::new();
    c.array_context = true;
    let col = arr(2, 1, vec![num(1.0), num(2.0)]);
    let row = arr(1, 3, vec![num(1.0), num(2.0), num(3.0)]);
    let (r, k, data) = matrix(&probe(&mut c, col, row));
    assert_eq!((r, k), (2, 3));
    assert_eq!(
        data,
        vec![
            num(11.0),
            num(12.0),
            num(13.0),
            num(21.0),
            num(22.0),
            num(23.0)
        ]
    );
    let a = arr(2, 2, vec![num(1.0), num(2.0), num(3.0), num(4.0)]);
    let b3 = arr(3, 1, vec![num(1.0), num(1.0), num(1.0)]);
    let (r, k, data) = matrix(&probe(&mut c, a, b3));
    assert_eq!((r, k), (3, 2));
    assert_eq!(data[4], err(ErrorKind::NA), "outside the 2x2 operand");
    assert_eq!(data[0], num(11.0));
}

#[test]
fn array_lifting_over_ranges_and_errors() {
    let mut c = MockEvalContext::new();
    c.set(0, "A1", 1.0)
        .set(0, "A2", Value::Error(ErrorKind::Div0))
        .set(0, "A3", "x");
    c.array_context = true;
    let r = c.range(0, "A1:A4");
    let (rows, _, data) = matrix(&probe(&mut c, r, n(0.0)));
    assert_eq!(rows, 4);
    assert_eq!(
        data,
        vec![
            num(10.0),
            err(ErrorKind::Div0),
            err(ErrorKind::Value),
            num(0.0)
        ]
    );
}

#[test]
fn lazy_functions_are_not_lifted() {
    let mut c = MockEvalContext::new();
    c.array_context = true;
    let cond = arr(2, 1, vec![Value::Bool(false), Value::Bool(true)]);
    let mut args = MockArgs::new(vec![cond, n(1.0), n(2.0)]);
    assert_eq!(
        call_with(&mut c, "IF", &mut args),
        num(2.0),
        "top-left condition"
    );
}

#[test]
fn non_finite_results_become_num() {
    let mut c = MockEvalContext::new();
    assert_eq!(
        call(&mut c, "SUM", vec![n(f64::MAX), n(f64::MAX)]),
        err(ErrorKind::Num)
    );
}

/// Review 3 M1: a union reaching a scalar parameter is `#VALUE!` (there is
/// no cell to intersect with); sequence parameters flatten it; array
/// lifting cannot materialize it.
#[cfg(feature = "ref-union")]
#[test]
fn unions_per_param_kind() {
    let mut c = MockEvalContext::new();
    c.set(0, "A1", 1.0)
        .set(0, "A2", 2.0)
        .set(0, "B1", 10.0)
        .set(0, "B2", 20.0);
    c.set_origin(0, "D1");
    let u = c.union(0, &["A1:A2", "B1:B2"]);
    assert_eq!(call(&mut c, "SUM", vec![u.clone()]), num(33.0));
    assert_eq!(call(&mut c, "COUNT", vec![u.clone()]), num(4.0));
    assert_eq!(call(&mut c, "MAX", vec![u.clone()]), num(20.0));
    assert_eq!(
        call(&mut c, "ROUND", vec![u.clone()]),
        err(ErrorKind::Value)
    );
    assert_eq!(call(&mut c, "LEN", vec![u.clone()]), err(ErrorKind::Value));
    assert_eq!(call(&mut c, "ISREF", vec![u.clone()]), Value::Bool(true));
    assert_eq!(call(&mut c, "TYPE", vec![u.clone()]), num(64.0));
    assert_eq!(
        call(&mut c, "SUMPRODUCT", vec![u.clone()]),
        err(ErrorKind::Value)
    );
    assert_eq!(
        call(&mut c, "VLOOKUP", vec![n(1.0), u.clone(), n(1.0)]),
        err(ErrorKind::Value)
    );
    assert_eq!(call(&mut c, "MEDIAN", vec![u.clone()]), num(6.0));
    c.array_context = true;
    assert_eq!(call(&mut c, "ROUND", vec![u]), err(ErrorKind::Value));
}

#[test]
fn mock_locale() {
    let mut c = MockEvalContext::new();
    assert_eq!(c.locale_tag(), "en-US");
    c.set_locale("de_de");
    assert_eq!(c.locale_tag(), "de-DE");
    c.set_locale("fr");
    assert_eq!(c.locale_tag(), "fr-FR");
    assert!(
        std::panic::catch_unwind(|| MockEvalContext::new().set_locale("de-CH").locale).is_err()
    );
}
