//! Scenarios (wave 5, W5-2): create, show with copy-back, update, delete,
//! undo and structure changes.
#![allow(clippy::unwrap_used)]

mod common;

use common::{range, Fixture};
use tally_api::{DocumentService, Op, ScenarioDto};
use tally_types::Value;

fn dto(name: &str, copy_back: bool) -> ScenarioDto {
    ScenarioDto {
        id: 0,
        name: name.into(),
        comment: "what if".into(),
        ranges: vec![range("A1:A2")],
        border_color: Some(0x0000_ffff),
        display_border: true,
        copy_back,
        copy_styles: false,
        prevent_changes: false,
        active: false,
    }
}

#[test]
fn scenario_show_copy_back() {
    let mut f = Fixture::new();
    f.set("A1", "10");
    f.set("A2", "20");
    f.set("B1", "=A1+A2");
    let r = f.apply(vec![Op::CreateScenario {
        sheet: f.sheet,
        scenario: dto("Base", true),
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    let base = r.created.unwrap()[0].id;
    f.set("A1", "100");
    let r = f.apply(vec![Op::CreateScenario {
        sheet: f.sheet,
        scenario: dto("High", true),
    }]);
    let high = r.created.unwrap()[0].id;
    let list = f.svc.scenarios(f.doc, f.sheet).unwrap();
    assert_eq!(list.len(), 2);
    assert!(list[1].active && !list[0].active);
    // Showing Base restores 10; the edit made while High was shown is
    // copied back into High.
    f.set("A2", "25");
    let r = f.apply(vec![Op::ShowScenario {
        sheet: f.sheet,
        id: base,
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(f.value("A1"), Value::Number(10.0));
    assert_eq!(f.value("A2"), Value::Number(20.0));
    assert_eq!(f.value("B1"), Value::Number(30.0));
    f.apply(vec![Op::ShowScenario {
        sheet: f.sheet,
        id: high,
    }]);
    assert_eq!(f.value("A1"), Value::Number(100.0));
    assert_eq!(f.value("A2"), Value::Number(25.0));
    assert_eq!(f.value("B1"), Value::Number(125.0));
    f.svc.undo(f.doc).unwrap();
    assert_eq!(f.value("A1"), Value::Number(10.0));
    assert!(f.svc.scenarios(f.doc, f.sheet).unwrap()[0].active);
    f.svc.redo(f.doc).unwrap();
    assert_eq!(f.value("A1"), Value::Number(100.0));
    // Update keeps the values; delete removes.
    let mut d = dto("High2", false);
    d.comment = "renamed".into();
    f.apply(vec![Op::UpdateScenario {
        sheet: f.sheet,
        id: high,
        scenario: d,
    }]);
    let list = f.svc.scenarios(f.doc, f.sheet).unwrap();
    assert_eq!(list[1].name, "High2");
    assert!(!list[1].copy_back);
    f.apply(vec![Op::DeleteScenario {
        sheet: f.sheet,
        id: base,
    }]);
    assert_eq!(f.svc.scenarios(f.doc, f.sheet).unwrap().len(), 1);
    // A duplicate name is refused.
    let r = f.apply(vec![Op::CreateScenario {
        sheet: f.sheet,
        scenario: dto("High2", true),
    }]);
    assert!(!r.errors.is_empty());
}

#[test]
fn scenario_ranges_follow_structure() {
    let mut f = Fixture::new();
    f.set("A1", "1");
    f.set("A2", "2");
    f.apply(vec![Op::CreateScenario {
        sheet: f.sheet,
        scenario: dto("S", true),
    }]);
    f.apply(vec![Op::InsertRows {
        sheet: f.sheet,
        at: 0,
        count: 3,
    }]);
    let s = &f.svc.scenarios(f.doc, f.sheet).unwrap()[0];
    assert_eq!(s.ranges, vec![range("A4:A5")]);
    let data = f.data();
    assert_eq!(data.sheets[0].scenarios[0].cells[0].row, 3);
    // Navigator lists it.
    let nav = f.svc.navigator(f.doc).unwrap();
    assert_eq!(nav.scenarios.len(), 1);
    assert_eq!(nav.scenarios[0].title, "S");
}

#[test]
fn scenario_cap_per_sheet() {
    let mut f = Fixture::new();
    f.set("A1", "1");
    let ops: Vec<Op> = (0..1_001)
        .map(|i| Op::CreateScenario {
            sheet: f.sheet,
            scenario: dto(&format!("S{i}"), false),
        })
        .collect();
    let r = f.apply(ops);
    assert_eq!(r.errors.len(), 1, "the 1,001st is refused");
    assert_eq!(r.errors[0].op_index, 1_000);
    assert_eq!(f.svc.scenarios(f.doc, f.sheet).unwrap().len(), 1_000);
}
