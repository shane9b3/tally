//! Golden tests for the rule-based assist on the mockup sheet.

use std::collections::HashMap;
use std::time::Instant;

use tally_assist::{
    AssistProvider, AssistRequest, RuleProvider, SheetView, Suggestion, SuggestionKind,
};
use tally_formula::{parse, Dialect, SimpleResolver};
use tally_types::{CellAddr, CellRange, Value};

struct Grid {
    cells: HashMap<(u32, u32), Value>,
    rows: u32,
    cols: u32,
}

impl SheetView for Grid {
    fn value(&self, row: u32, col: u32) -> Value {
        self.cells.get(&(row, col)).cloned().unwrap_or_default()
    }
    fn used(&self) -> Option<CellRange> {
        (self.rows > 0).then(|| CellRange::new(0, 0, 0, self.rows - 1, self.cols - 1))
    }
    fn sheet(&self) -> u32 {
        0
    }
}

/// (date, customer, item, qty, price, status); Total = qty * price.
const ROWS: [(f64, &str, &str, f64, f64, &str); 14] = [
    (46001.0, "Acme Corp", "Widget", 10.0, 12.5, "Paid"),
    (46003.0, "Corner Café", "Gadget", 4.0, 30.0, "Due"),
    (46005.0, "Blue Sky", "Widget", 25.0, 12.5, "Overdue"),
    (46007.0, "Acme Corp", "Gizmo", 2.0, 99.0, "Paid"),
    (46009.0, "Dunder Mill", "Gadget", 20.0, 30.0, "Paid"),
    (46011.0, "Corner Café", "Widget", 8.0, 12.5, "Overdue"),
    (46013.0, "Blue Sky", "Gizmo", 1.0, 99.0, "Due"),
    (46015.0, "Acme Corp", "Gadget", 15.0, 30.0, "Paid"),
    (46017.0, "Evergreen", "Widget", 30.0, 12.5, "Due"),
    (46019.0, "Dunder Mill", "Gizmo", 5.0, 99.0, "Paid"),
    (46021.0, "Corner Café", "Gadget", 12.0, 30.0, "Paid"),
    (46023.0, "Evergreen", "Gizmo", 3.0, 99.0, "Overdue"),
    (46025.0, "Blue Sky", "Widget", 18.0, 12.5, "Paid"),
    (46027.0, "Acme Corp", "Gadget", 6.0, 30.0, "Due"),
];

fn mockup() -> Grid {
    let mut cells = HashMap::new();
    for (c, h) in [
        "Date", "Customer", "Item", "Qty", "Price", "Total", "Status",
    ]
    .iter()
    .enumerate()
    {
        cells.insert((0, c as u32), Value::text(*h));
    }
    for (i, (d, cust, item, qty, price, status)) in ROWS.iter().enumerate() {
        let r = i as u32 + 1;
        cells.insert((r, 0), Value::Number(*d));
        cells.insert((r, 1), Value::text(*cust));
        cells.insert((r, 2), Value::text(*item));
        cells.insert((r, 3), Value::Number(*qty));
        cells.insert((r, 4), Value::Number(*price));
        cells.insert((r, 5), Value::Number(qty * price));
        cells.insert((r, 6), Value::text(*status));
    }
    Grid {
        cells,
        rows: 15,
        cols: 7,
    }
}

fn run(grid: &Grid, query: &str, active: CellAddr) -> Vec<Suggestion> {
    let req = AssistRequest {
        query,
        view: grid,
        active,
        selection: None,
        preview: None,
    };
    RuleProvider::default().suggest(&req)
}

fn ask(query: &str) -> Vec<Suggestion> {
    run(&mockup(), query, CellAddr::new(0, 0, 8))
}

fn first_formula(query: &str) -> Option<String> {
    ask(query).into_iter().find_map(|s| s.formula)
}

fn has_action(query: &str, id: &str) -> bool {
    ask(query).iter().any(|s| s.action.as_deref() == Some(id))
}

#[test]
fn golden_formulas() {
    let cases: &[(&str, &str)] = &[
        (
            "total of unpaid orders",
            r#"=SUMIF(G2:G15,"<>Paid",F2:F15)"#,
        ),
        (
            "sum of total where status is paid",
            r#"=SUMIF(G2:G15,"Paid",F2:F15)"#,
        ),
        ("average price", "=AVERAGE(E2:E15)"),
        ("how many overdue", r#"=COUNTIF(G2:G15,"Overdue")"#),
        ("count of orders", "=COUNTA(A2:A15)"),
        ("max quantity", "=MAX(D2:D15)"),
        ("largest total", "=MAX(F2:F15)"),
        ("smallest price", "=MIN(E2:E15)"),
        ("number of paid orders", r#"=COUNTIF(G2:G15,"Paid")"#),
        (
            "total for Corner Café",
            r#"=SUMIF(B2:B15,"Corner Café",F2:F15)"#,
        ),
        ("sum of qty over 10", r#"=SUMIF(D2:D15,">10")"#),
        ("orders with qty at least 20", r#"=COUNTIF(D2:D15,">=20")"#),
        (
            "average total for due orders",
            r#"=AVERAGEIF(G2:G15,"Due",F2:F15)"#,
        ),
        ("avrage price", "=AVERAGE(E2:E15)"),
        ("totl unpaid", r#"=SUMIF(G2:G15,"<>Paid",F2:F15)"#),
        ("total", "=SUM(F2:F15)"),
        ("sum", "=SUM(F2:F15)"),
        ("total qty", "=SUM(D2:D15)"),
        ("add up the prices", "=SUM(E2:E15)"),
        ("average qty", "=AVERAGE(D2:D15)"),
        ("mean total", "=AVERAGE(F2:F15)"),
        ("median price", "=MEDIAN(E2:E15)"),
        ("highest price", "=MAX(E2:E15)"),
        ("lowest qty", "=MIN(D2:D15)"),
        ("minimum total", "=MIN(F2:F15)"),
        ("how many orders", "=COUNTA(A2:A15)"),
        ("how many are due", r#"=COUNTIF(G2:G15,"Due")"#),
        ("number of unpaid orders", r#"=COUNTIF(G2:G15,"<>Paid")"#),
        ("count overdue", r#"=COUNTIF(G2:G15,"Overdue")"#),
        ("total of overdue", r#"=SUMIF(G2:G15,"Overdue",F2:F15)"#),
        (
            "sum of price where status is not paid",
            r#"=SUMIF(G2:G15,"<>Paid",E2:E15)"#,
        ),
        (
            "sum of total where status isn't due",
            r#"=SUMIF(G2:G15,"<>Due",F2:F15)"#,
        ),
        ("total over 200", r#"=SUMIF(F2:F15,">200")"#),
        ("count qty above 10", r#"=COUNTIF(D2:D15,">10")"#),
        (
            "how many orders have qty below 5",
            r#"=COUNTIF(D2:D15,"<5")"#,
        ),
        ("orders with qty at most 5", r#"=COUNTIF(D2:D15,"<=5")"#),
        (
            "sum of total where qty between 5 and 15",
            r#"=SUMIFS(F2:F15,D2:D15,">=5",D2:D15,"<=15")"#,
        ),
        (
            "total unpaid with qty over 10",
            r#"=SUMIFS(F2:F15,G2:G15,"<>Paid",D2:D15,">10")"#,
        ),
        (
            "total for acme corp",
            r#"=SUMIF(B2:B15,"Acme Corp",F2:F15)"#,
        ),
        (
            "average price for widgets",
            r#"=AVERAGEIF(C2:C15,"Widget",E2:E15)"#,
        ),
        (
            "total of gadget orders",
            r#"=SUMIF(C2:C15,"Gadget",F2:F15)"#,
        ),
        ("count of gizmo", r#"=COUNTIF(C2:C15,"Gizmo")"#),
        (
            "Total of Unpaid Orders",
            r#"=SUMIF(G2:G15,"<>Paid",F2:F15)"#,
        ),
        (
            "TOTAL OF UNPAID ORDERS!!",
            r#"=SUMIF(G2:G15,"<>Paid",F2:F15)"#,
        ),
        (
            "what is the total of unpaid orders?",
            r#"=SUMIF(G2:G15,"<>Paid",F2:F15)"#,
        ),
        ("what's the average price?", "=AVERAGE(E2:E15)"),
        (
            "sum of total where status = Paid",
            r#"=SUMIF(G2:G15,"Paid",F2:F15)"#,
        ),
        (
            "total where status <> Paid",
            r#"=SUMIF(G2:G15,"<>Paid",F2:F15)"#,
        ),
        (
            "total where status != Paid",
            r#"=SUMIF(G2:G15,"<>Paid",F2:F15)"#,
        ),
        ("sum total except paid", r#"=SUMIF(G2:G15,"<>Paid",F2:F15)"#),
        (
            "average qty of paid orders",
            r#"=AVERAGEIF(G2:G15,"Paid",D2:D15)"#,
        ),
        ("sum of qty", "=SUM(D2:D15)"),
        ("sum quantity", "=SUM(D2:D15)"),
        ("avg cost", "=AVERAGE(E2:E15)"),
        ("biggest total", "=MAX(F2:F15)"),
        ("top price", "=MAX(E2:E15)"),
        ("count of customers", "=COUNTA(B2:B15)"),
        ("how many items", "=COUNTA(C2:C15)"),
        ("sum of totals", "=SUM(F2:F15)"),
        ("total sales", "=SUM(F2:F15)"),
        ("unpaid orders", r#"=COUNTIF(G2:G15,"<>Paid")"#),
        (
            "average price of paid",
            r#"=AVERAGEIF(G2:G15,"Paid",E2:E15)"#,
        ),
        (
            "sum of price where qty over 10",
            r#"=SUMIF(D2:D15,">10",E2:E15)"#,
        ),
        ("maximum quantiy", "=MAX(D2:D15)"),
        ("how many overdu", r#"=COUNTIF(G2:G15,"Overdue")"#),
        (
            "sum of totl where status is paid",
            r#"=SUMIF(G2:G15,"Paid",F2:F15)"#,
        ),
        ("median total", "=MEDIAN(F2:F15)"),
    ];
    assert!(cases.len() >= 60);
    let mut failures = Vec::new();
    for (q, want) in cases {
        let got = first_formula(q);
        if got.as_deref() != Some(*want) {
            failures.push(format!("{q:?}: want {want}, got {got:?}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn golden_actions() {
    let cases: &[(&str, &str)] = &[
        ("show only unpaid", "filter.byColumn"),
        ("filter by overdue", "filter.byColumn"),
        ("show only overdue", "filter.byColumn"),
        ("only paid", "filter.byColumn"),
        ("show Corner Café orders", "filter.byColumn"),
        ("chart total by customer", "chart.byColumn"),
        ("plot price by item", "chart.byColumn"),
        ("graph total by status", "chart.byColumn"),
        ("format total as currency", "format.currency"),
        ("format price as currency", "format.currency"),
        ("format total as money", "format.currency"),
    ];
    for (q, id) in cases {
        assert!(has_action(q, id), "{q:?} should offer {id}: {:?}", ask(q));
    }
    // Command-only queries do not insert a formula.
    for q in [
        "show only unpaid",
        "chart total by customer",
        "format total as currency",
    ] {
        assert!(
            first_formula(q).is_none(),
            "{q:?} must not produce a formula"
        );
    }
}

#[test]
fn action_titles_and_args() {
    let s = ask("show only unpaid");
    let f = s
        .iter()
        .find(|s| s.action.as_deref() == Some("filter.byColumn"));
    let f = f.expect("filter suggestion");
    assert_eq!(f.kind, SuggestionKind::Command);
    assert!(f.title.contains("Status is not Paid"), "{}", f.title);
    assert!(f.action_args.contains(&("column".into(), "G".into())));
    let s = ask("show only overdue");
    assert_eq!(s[0].title, "Show only Overdue rows");
    let s = ask("chart total by customer");
    assert_eq!(s[0].title, "Chart Total by Customer");
    let s = ask("format total as currency");
    assert_eq!(s[0].title, "Format Total as currency");
    assert!(s[0].action_args.contains(&("column".into(), "F".into())));
    assert!(s[0]
        .action_args
        .contains(&("range".into(), "F2:F15".into())));
}

#[test]
fn must_return_nothing() {
    let cases = [
        "hello",
        "gibberish asdf qwer",
        "delete everything",
        "asdf",
        "what is the weather today",
        "tell me a joke",
        "",
        "   ",
        "????",
        "12345",
        "foo bar baz",
        "please help me",
        "remove all rows",
        "email this to bob",
        "zzzzzz qqqqq",
    ];
    assert!(cases.len() >= 10);
    for q in cases {
        let s = ask(q);
        assert!(s.is_empty(), "{q:?} should return nothing, got {s:?}");
    }
}

#[test]
fn explanations_and_confidence() {
    let s = ask("total of unpaid orders");
    let top = &s[0];
    assert_eq!(top.kind, SuggestionKind::InsertFormula);
    assert!(top
        .explanation
        .starts_with("Sum of Total where Status is not Paid"));
    assert!(top.confidence > 0.8);
    assert!(top.action.is_none());
    assert!(s.len() <= 5);
    assert!(s.windows(2).all(|w| w[0].confidence >= w[1].confidence));
    // Companion actions are offered after the formula.
    assert!(s
        .iter()
        .any(|s| s.action.as_deref() == Some("filter.byColumn")));
    assert!(s
        .iter()
        .any(|s| s.action.as_deref() == Some("chart.byColumn")));
    assert!(s
        .iter()
        .any(|s| s.action.as_deref() == Some("format.currency")));

    assert!(ask("average price")[0]
        .explanation
        .starts_with("Average of Price"));
    assert!(ask("how many overdue")[0]
        .explanation
        .starts_with("Count of rows where Status is Overdue"));
}

#[test]
fn month_conditions_are_flagged_unsupported() {
    let s = ask("total in october");
    let f = s.iter().find(|s| s.formula.is_some()).expect("formula");
    assert_eq!(f.formula.as_deref(), Some("=SUM(F2:F15)"));
    assert!(f.explanation.contains("not supported"), "{}", f.explanation);
    assert!(f.confidence < 0.7);
}

#[test]
fn conditional_max_is_low_confidence() {
    let s = ask("max total for overdue");
    let f = s.iter().find(|s| s.formula.is_some()).expect("formula");
    assert_eq!(f.formula.as_deref(), Some("=MAX(F2:F15)"));
    assert!(f.confidence < 0.6);
    assert!(f.explanation.contains("not applied"));
    assert_eq!(f.title, "Maximum of Total");
}

#[test]
fn every_formula_parses() {
    let resolver = SimpleResolver::with_default_functions();
    for q in [
        "total of unpaid orders",
        "sum of total where qty between 5 and 15",
        "count of customers",
        "total for Corner Café",
        "median price",
    ] {
        for s in ask(q) {
            if let Some(f) = s.formula {
                assert!(
                    parse(
                        &f,
                        CellAddr::new(0, 0, 8),
                        Dialect::calc_a1_en_us(),
                        &resolver
                    )
                    .is_ok(),
                    "{f}"
                );
                assert!(f.starts_with('='));
            }
        }
    }
}

#[test]
fn preview_callback_is_formatted() {
    let g = mockup();
    let seen = std::cell::RefCell::new(Vec::new());
    let cb = |f: &str| -> Option<Value> {
        seen.borrow_mut().push(f.to_string());
        Some(Value::Number(12480.0))
    };
    let req = AssistRequest {
        query: "total of unpaid orders",
        view: &g,
        active: CellAddr::new(0, 0, 8),
        selection: None,
        preview: Some(&cb),
    };
    let s = RuleProvider::default().suggest(&req);
    assert_eq!(s[0].preview.as_deref(), Some("$12,480"));
    assert_eq!(seen.borrow()[0], r#"=SUMIF(G2:G15,"<>Paid",F2:F15)"#);

    // Counts are not currency; None from the callback leaves no preview.
    let cb2 = |_: &str| Some(Value::Number(3.0));
    let req = AssistRequest {
        query: "how many overdue",
        preview: Some(&cb2),
        ..req
    };
    assert_eq!(
        RuleProvider::default().suggest(&req)[0].preview.as_deref(),
        Some("3")
    );
    let cb3 = |_: &str| None;
    let req = AssistRequest {
        preview: Some(&cb3),
        ..req
    };
    assert_eq!(RuleProvider::default().suggest(&req)[0].preview, None);
}

#[test]
fn active_cell_position_does_not_matter() {
    let g = mockup();
    let want = Some(r#"=SUMIF(G2:G15,"<>Paid",F2:F15)"#.to_string());
    for active in [
        CellAddr::new(0, 0, 0),
        CellAddr::new(0, 7, 3),
        CellAddr::new(0, 15, 0),   // just below the data
        CellAddr::new(0, 0, 7),    // just right of the data
        CellAddr::new(0, 200, 40), // far away: largest region
    ] {
        let s = run(&g, "total of unpaid orders", active);
        assert_eq!(s.into_iter().find_map(|s| s.formula), want, "{active:?}");
    }
    // A selection inside the table expands to the whole table.
    let req = AssistRequest {
        query: "total of unpaid orders",
        view: &g,
        active: CellAddr::new(0, 3, 3),
        selection: Some(CellRange::new(0, 3, 3, 5, 4)),
        preview: None,
    };
    assert_eq!(RuleProvider::default().suggest(&req)[0].formula, want);
}

#[test]
fn table_not_at_origin_and_no_header() {
    // Same data shifted to C5 with no header: ranges start at the first row.
    let mut cells = HashMap::new();
    for r in 0..5u32 {
        cells.insert((4 + r, 2), Value::Number(f64::from(r + 1)));
        cells.insert((4 + r, 3), Value::Number(f64::from((r + 1) * 10)));
    }
    let g = Grid {
        cells,
        rows: 20,
        cols: 6,
    };
    let s = run(&g, "total", CellAddr::new(0, 6, 2));
    assert_eq!(s[0].formula.as_deref(), Some("=SUM(C5:C9)"));
    let s = run(&g, "average", CellAddr::new(0, 6, 3));
    assert_eq!(s[0].formula.as_deref(), Some("=AVERAGE(C5:C9)"));
    // Without headers a named column cannot be matched, but values still can.
    assert!(run(&g, "max qty", CellAddr::new(0, 6, 3))[0].confidence < 0.9);
}

#[test]
fn empty_sheet_gives_nothing() {
    let g = Grid {
        cells: HashMap::new(),
        rows: 0,
        cols: 0,
    };
    assert!(run(&g, "total", CellAddr::new(0, 0, 0)).is_empty());
}

#[test]
fn provider_is_object_safe_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<RuleProvider>();
    let p: Box<dyn AssistProvider> = Box::new(RuleProvider::default());
    assert_eq!(p.id(), "rules");
}

#[test]
fn ten_thousand_rows_is_fast() {
    let mut cells = HashMap::new();
    let statuses = ["Paid", "Due", "Overdue"];
    for (c, h) in ["Id", "Qty", "Total", "Status"].iter().enumerate() {
        cells.insert((0, c as u32), Value::text(*h));
    }
    for r in 1..=10_000u32 {
        cells.insert((r, 0), Value::Number(f64::from(r)));
        cells.insert((r, 1), Value::Number(f64::from(r % 50)));
        cells.insert((r, 2), Value::Number(f64::from(r) * 1.5));
        cells.insert((r, 3), Value::text(statuses[(r % 3) as usize]));
    }
    let g = Grid {
        cells,
        rows: 10_001,
        cols: 4,
    };
    let start = Instant::now();
    let s = run(&g, "total of unpaid orders", CellAddr::new(0, 5, 1));
    let elapsed = start.elapsed();
    assert_eq!(
        s[0].formula.as_deref(),
        Some(r#"=SUMIF(D2:D10001,"<>Paid",C2:C10001)"#)
    );
    let limit = if cfg!(debug_assertions) { 500 } else { 20 };
    assert!(elapsed.as_millis() < limit, "took {elapsed:?}");
}
