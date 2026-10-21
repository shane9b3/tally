//! Calculation semantics (wave 5, W5-3): iterations, precision as shown,
//! the `SetCalcSettings` op, MULTIPLE.OPERATIONS and reference unions.
#![allow(clippy::unwrap_used)]

mod common;

use common::{input, Fixture};
use tally_api::{
    CalcSettingsDto, CriteriaModeDto, DateEpochDto, DocumentService, IterationsDto, Op, RangeDto,
    StylePatch, TextToNumberDto,
};
use tally_types::{ErrorKind, Value};

fn settings(iterations: Option<(u32, f64)>, precision: bool) -> CalcSettingsDto {
    CalcSettingsDto {
        iterations: iterations.map(|(steps, min_change)| IterationsDto { steps, min_change }),
        date_epoch: DateEpochDto::D1899,
        case_sensitive: false,
        precision_as_shown: precision,
        whole_cell_match: true,
        criteria_mode: CriteriaModeDto::Wildcards,
        text_to_number: TextToNumberDto::ConvertUnambiguous,
    }
}

fn num(v: &Value) -> f64 {
    match v {
        Value::Number(n) => *n,
        other => panic!("not a number: {other:?}"),
    }
}

#[test]
fn iteration_self_ref_converges() {
    let mut f = Fixture::new();
    let s = f.sheet;
    f.set("A1", "=A1/2+1");
    assert_eq!(f.value("A1"), Value::Error(ErrorKind::Circular));
    let res = f.apply(vec![Op::SetCalcSettings {
        settings: settings(Some((100, 0.001)), false),
    }]);
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let v = num(&f.value("A1"));
    assert!((v - 2.0).abs() < 0.001, "{v}");
    // A two-cell cycle converges too, and dependents see the result.
    f.set("B1", "=C1*0.5+1");
    f.set("C1", "=B1*0.5+1");
    f.set("D1", "=B1+C1");
    let b = num(&f.value("B1"));
    assert!((b - 2.0).abs() < 0.01, "{b}");
    assert!((num(&f.value("D1")) - 4.0).abs() < 0.02);
    assert_eq!(
        f.svc
            .calc_settings(f.doc)
            .unwrap()
            .iterations
            .unwrap()
            .steps,
        100
    );
    let _ = s;
}

#[test]
fn iteration_divergent_err523() {
    let mut f = Fixture::new();
    f.apply(vec![Op::SetCalcSettings {
        settings: settings(Some((100, 0.001)), false),
    }]);
    f.set("A1", "=A1+1");
    assert_eq!(f.value("A1"), Value::Error(ErrorKind::NoConvergence));
    // Fewer steps than needed: the counter runs out.
    f.apply(vec![Op::SetCalcSettings {
        settings: settings(Some((3, 0.0)), false),
    }]);
    f.set("B1", "=B1/2+1");
    assert_eq!(f.value("B1"), Value::Error(ErrorKind::NoConvergence));
}

#[test]
fn iteration_off_err522() {
    let mut f = Fixture::new();
    f.apply(vec![Op::SetCalcSettings {
        settings: settings(Some((100, 0.001)), false),
    }]);
    f.set("A1", "=A1/2+1");
    assert!((num(&f.value("A1")) - 2.0).abs() < 0.001);
    f.apply(vec![Op::SetCalcSettings {
        settings: settings(None, false),
    }]);
    assert_eq!(f.value("A1"), Value::Error(ErrorKind::Circular));
}

#[test]
fn precision_as_shown_rounds_sum() {
    let mut f = Fixture::new();
    let s = f.sheet;
    for a1 in ["A1", "A2", "A3"] {
        f.set(a1, "0.333");
    }
    f.apply(vec![Op::ApplyStyle {
        sheet: s,
        ranges: vec![RangeDto {
            r0: 0,
            c0: 0,
            r1: 2,
            c1: 0,
        }],
        patch: StylePatch {
            number_format: Some("0.00".into()),
            ..StylePatch::default()
        },
    }]);
    f.set("B1", "=SUM(A1:A3)");
    f.set("B2", "=A1+A2+A3");
    assert!((num(&f.value("B1")) - 0.999).abs() < 1e-12);
    f.apply(vec![Op::SetCalcSettings {
        settings: settings(None, true),
    }]);
    assert_eq!(f.value("B1"), Value::Number(0.99));
    assert_eq!(f.value("B2"), Value::Number(0.99));
    // A result stored in a formatted cell is rounded too; General is not.
    f.set("C1", "=1/3");
    assert!(
        (num(&f.value("C1")) - 1.0 / 3.0).abs() < 1e-15,
        "General keeps the value"
    );
    f.apply(vec![Op::ApplyStyle {
        sheet: s,
        ranges: vec![RangeDto {
            r0: 0,
            c0: 2,
            r1: 0,
            c1: 2,
        }],
        patch: StylePatch {
            number_format: Some("0.0".into()),
            ..StylePatch::default()
        },
    }]);
    f.set("C1", "=1/3");
    assert_eq!(f.value("C1"), Value::Number(0.3));
}

#[test]
fn set_calc_settings_undo() {
    let mut f = Fixture::new();
    f.set("A1", "=A1/2+1");
    let before = f.data();
    let res = f.apply(vec![Op::SetCalcSettings {
        settings: settings(Some((50, 0.01)), true),
    }]);
    assert!(res.errors.is_empty());
    assert!(res.undo.can_undo);
    assert!((num(&f.value("A1")) - 2.0).abs() < 0.02);
    let after = f.data();
    assert_ne!(before, after);
    f.svc.undo(f.doc).unwrap();
    assert_eq!(f.data(), before);
    assert_eq!(f.value("A1"), Value::Error(ErrorKind::Circular));
    assert!(f.svc.calc_settings(f.doc).unwrap().iterations.is_none());
    f.svc.redo(f.doc).unwrap();
    assert_eq!(f.data(), after);
    assert!((num(&f.value("A1")) - 2.0).abs() < 0.02);
    // Invalid settings are refused.
    let res = f.apply(vec![Op::SetCalcSettings {
        settings: settings(Some((0, 0.01)), false),
    }]);
    assert_eq!(res.errors.len(), 1);
}

#[test]
fn multiple_operations_one_and_two_variable() {
    let mut f = Fixture::new();
    // Model: price A1, quantity A2, cost A3 = A1*A2 through an intermediate
    // B1 = A1*1.1 (the chain between the inputs and the formula).
    f.set("A1", "10");
    f.set("A2", "3");
    f.set("B1", "=A1*1.1");
    f.set("A3", "=B1*A2");
    assert!((num(&f.value("A3")) - 33.0).abs() < 1e-9);
    // One variable: prices 1..3 down column D, results in E.
    for (i, p) in [1.0, 2.0, 3.0].iter().enumerate() {
        f.set(&format!("D{}", i + 1), &p.to_string());
        f.set(
            &format!("E{}", i + 1),
            &format!("=MULTIPLE.OPERATIONS($A$3,$A$1,D{})", i + 1),
        );
    }
    assert!(
        (num(&f.value("E1")) - 3.3).abs() < 1e-9,
        "{:?}",
        f.value("E1")
    );
    assert!((num(&f.value("E2")) - 6.6).abs() < 1e-9);
    assert!((num(&f.value("E3")) - 9.9).abs() < 1e-9);
    // Two variables: prices down G, quantities across H1:J1.
    f.set("G2", "2");
    f.set("G3", "4");
    f.set("H1", "1");
    f.set("I1", "5");
    f.set("J1", "10");
    for r in 2..=3 {
        for c in ["H", "I", "J"] {
            f.set(
                &format!("{c}{r}"),
                &format!("=MULTIPLE.OPERATIONS($A$3,$A$1,$G{r},$A$2,{c}$1)"),
            );
        }
    }
    assert!((num(&f.value("H2")) - 2.2).abs() < 1e-9);
    assert!(
        (num(&f.value("J3")) - 44.0).abs() < 1e-9,
        "{:?}",
        f.value("J3")
    );
    // The model itself is untouched and dependents follow the inputs.
    assert_eq!(f.value("A1"), Value::Number(10.0));
    f.set("A2", "6");
    assert!(
        (num(&f.value("J3")) - 44.0).abs() < 1e-9,
        "replacement overrides A2"
    );
    f.set("E4", "=MULTIPLE.OPERATIONS(A1,A1,5)");
    assert_eq!(
        f.value("E4"),
        Value::Error(ErrorKind::ParamList),
        "not a formula cell"
    );
}

#[test]
fn reference_unions() {
    let mut f = Fixture::new();
    f.set("A1", "1");
    f.set("A2", "2");
    f.set("B1", "10");
    f.set("B2", "20");
    let cases = [
        ("=SUM(A1:A2~B1:B2)", Value::Number(33.0)),
        ("=SUBTOTAL(9,A1:A2~B1:B2)", Value::Number(33.0)),
        ("=AREAS(A1:A2~B1:B2)", Value::Number(2.0)),
        ("=COUNTIF(A1:A2~B1:B2,\">1\")", Value::Number(3.0)),
        ("=INDEX(A1:A2~B1:B2,2,1,2)", Value::Number(20.0)),
        ("=ROWS(A1:A2~B1:B2)", Value::Error(ErrorKind::ParamList)),
        ("=ISREF(A1~B1)", Value::Bool(true)),
        ("=COUNTBLANK(A1:A2~C1:C2)", Value::Number(2.0)),
        ("=A1:A2~B1:B2", Value::Error(ErrorKind::Value)),
        ("=MAX(A1~B2)", Value::Number(20.0)),
    ];
    for (i, (formula, expected)) in cases.iter().enumerate() {
        let cell = format!("D{}", i + 1);
        f.set(&cell, formula);
        assert_eq!(f.value(&cell), *expected, "{formula}");
    }
    // Every area is a precedent: editing B2 recalculates the union sums.
    f.set("B2", "200");
    assert_eq!(f.value("D1"), Value::Number(213.0));
    assert_eq!(f.value("D2"), Value::Number(213.0));
    let p = f.svc.precedents(f.doc, f.sheet, 0, 3).unwrap();
    assert_eq!(p.len(), 2, "{p:?}");
    // A named union evaluates to a union.
    let s = f.sheet;
    f.apply(vec![Op::DefineName {
        name: "U".into(),
        scope: None,
        refers_to: "$Sheet1.$A$1:$A$2~$Sheet1.$B$1:$B$2".into(),
    }]);
    f.apply(vec![input(s, "E1", "=SUM(U)")]);
    assert_eq!(f.value("E1"), Value::Number(213.0));
}

#[test]
fn locale_tag_reaches_functions() {
    let mut f = Fixture::new();
    f.svc.set_locale(f.doc, "de-DE").unwrap();
    f.set("A1", "1");
    f.set("A2", "2");
    f.set("A3", "1,7");
    f.set("B1", "=COUNTIF(A1:A3;\">1,5\")");
    assert_eq!(f.value("B1"), Value::Number(2.0));
    f.set("B2", "=VALUE(\"1,7\")");
    assert_eq!(f.value("B2"), Value::Number(1.7));
}
