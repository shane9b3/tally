//! explain() hints and the display-only prettifier.

#![allow(clippy::unwrap_used)] // Test code may unwrap.

mod common;
use common::*;
use std::sync::Arc;
use tally_formula::{explain, pretty, FuncId, LabelSource, NoLabels, Resolver};
use tally_types::{CellAddr, CellRange};

/// Headers in row 1: D "Quantity", E "Price", F "Total", G "Status".
struct Headers;

impl LabelSource for Headers {
    fn label_for(&self, addr: CellAddr) -> Option<String> {
        if addr.row == 0 {
            return None;
        }
        match addr.col {
            3 => Some("Quantity".into()),
            4 => Some("Price".into()),
            5 => Some("Total".into()),
            6 => Some("Status".into()),
            _ => None,
        }
    }
    fn range_label(&self, r: &CellRange) -> Option<String> {
        if r.c0 == r.c1 && r.r0 > 0 {
            self.label_for(r.start())
        } else {
            None
        }
    }
    fn function_name(&self, id: FuncId) -> Option<Arc<str>> {
        resolver().function_name(id)
    }
}

struct NamesOnly;
impl LabelSource for NamesOnly {
    fn label_for(&self, _: CellAddr) -> Option<String> {
        None
    }
    fn range_label(&self, _: &CellRange) -> Option<String> {
        None
    }
    fn function_name(&self, id: FuncId) -> Option<Arc<str>> {
        resolver().function_name(id)
    }
}

fn ex(src: &str, at: &str, labels: &dyn LabelSource) -> Option<String> {
    explain(&p_at(src, at), a1(at), labels)
}

#[test]
fn arithmetic_with_labels() {
    assert_eq!(
        ex("=D6*E6", "F6", &Headers).as_deref(),
        Some("Quantity × Price")
    );
    assert_eq!(
        ex("=D6/E6", "F6", &Headers).as_deref(),
        Some("Quantity ÷ Price")
    );
    assert_eq!(
        ex("=D6+E6-1", "F6", &Headers).as_deref(),
        Some("Quantity + Price − 1")
    );
    assert_eq!(
        ex("=(D6+E6)*2", "F6", &Headers).as_deref(),
        Some("(Quantity + Price) × 2")
    );
}

#[test]
fn arithmetic_without_labels() {
    assert_eq!(ex("=A1*B1", "C1", &NoLabels).as_deref(), Some("A1 × B1"));
    assert_eq!(ex("=A1-B1", "C1", &NoLabels).as_deref(), Some("A1 − B1"));
    assert_eq!(
        ex("=$A$1*2.5", "C1", &NoLabels).as_deref(),
        Some("A1 × 2.5")
    );
}

#[test]
fn aggregates() {
    assert_eq!(
        ex("=SUM(F2:F15)", "F16", &Headers).as_deref(),
        Some("Sum of Total")
    );
    assert_eq!(
        ex("=AVERAGE(E2:E15)", "F16", &Headers).as_deref(),
        Some("Average of Price")
    );
    assert_eq!(
        ex("=COUNT(D2:D15)", "F16", &Headers).as_deref(),
        Some("Count of Quantity")
    );
    assert_eq!(
        ex("=MIN(E2:E15)", "F16", &Headers).as_deref(),
        Some("Smallest Price")
    );
    assert_eq!(
        ex("=MAX(E2:E15)", "F16", &Headers).as_deref(),
        Some("Largest Price")
    );
    assert_eq!(
        ex("=SUM(A1:A3)", "B1", &NamesOnly).as_deref(),
        Some("Sum of A1:A3")
    );
    assert_eq!(
        ex("=SUM(A:A)", "B1", &NamesOnly).as_deref(),
        Some("Sum of A:A")
    );
    assert_eq!(
        ex("=SUM(F2:F15)*2", "F16", &Headers).as_deref(),
        Some("Sum of Total × 2")
    );
}

#[test]
fn sumif() {
    assert_eq!(
        ex("=SUMIF(G2:G15,\"<>Paid\",F2:F15)", "F16", &Headers).as_deref(),
        Some("Sum of Total where Status is not Paid")
    );
    assert_eq!(
        ex("=SUMIF(G2:G15,\"Paid\",F2:F15)", "F16", &Headers).as_deref(),
        Some("Sum of Total where Status is Paid")
    );
    assert_eq!(
        ex("=SUMIF(D2:D15,\">5\")", "F16", &Headers).as_deref(),
        Some("Sum of Quantity where Quantity is greater than 5")
    );
}

#[test]
fn not_explained() {
    for s in [
        "=A1",
        "=1",
        "=IF(A1,1,2)",
        "=SUM(A1,B1)",
        "=\"a\"&\"b\"",
        "=A1^2",
        "=FOO(A1)",
    ] {
        assert_eq!(ex(s, "C5", &Headers), None, "{s}");
    }
    // Known functions need LabelSource::function_name.
    assert_eq!(ex("=SUM(A1:A3)", "B1", &NoLabels), None);
}

#[test]
fn prettifier() {
    assert_eq!(pretty("=D6*E6"), "=D6 × E6");
    assert_eq!(pretty("=A1<>B1"), "=A1 ≠ B1");
    assert_eq!(pretty("=-A1+-2"), "=-A1 + -2");
    assert_eq!(pretty("=SUM(A1:B2,3)*2"), "=SUM(A1:B2,3) × 2");
    assert_eq!(pretty("=  A1  *  B1 "), "=A1 × B1");
    assert_eq!(pretty("=\"a*b\"&\"<>\""), "=\"a*b\" & \"<>\"");
    assert_eq!(pretty("=10%*2"), "=10% × 2");
    assert_eq!(pretty("=IF(A1>=0,-1,1)"), "=IF(A1 >= 0,-1,1)");
}
