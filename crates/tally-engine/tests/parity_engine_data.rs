//! Locks in the engine and data-tool rows of `docs/parity-deviations.md`
//! that were checked against the AOO 4.1 sources on 2026-11-16 (P6, G12,
//! T2, T7, T8; F14 lives in the function conformance files, P7 in
//! `tally-formula/tests/roundtrip.rs`, T6 in `pivot.rs`). Each test
//! names its row and the AOO function it was read from.
#![allow(clippy::unwrap_used)]

mod common;

use common::{range, sel, Fixture};
use tally_api::{
    AggFn, CfRuleDto, CfRuleKind, CfStyleDto, ClearWhat, CmpOp, CsvImportOptionsDto,
    DeleteContentsFlags, DocumentService, ListSource, Op, PasteOptions, PastePayload, ScenarioDto,
    SheetCell, SheetRange, StdCondition, StdConnector, StdFilterOp, StdFilterOptions,
    SubtotalGroup, SubtotalOptions, ValidationDto, ValidationKind,
};
use tally_types::{CellAddr, SheetId, Value};

fn n(x: f64) -> Value {
    Value::Number(x)
}

fn input_of(f: &Fixture, sheet: SheetId, a1: &str) -> String {
    let a = CellAddr::from_a1(sheet, a1).unwrap();
    f.svc
        .cell_edit_info(f.doc, sheet, a.row, a.col)
        .unwrap()
        .input
}

fn rows(f: &mut Fixture, r: u32, c: u32, data: &[&[&str]]) {
    let rows: Vec<Vec<String>> = data
        .iter()
        .map(|r| r.iter().map(|s| s.to_string()).collect())
        .collect();
    let res = f.apply(vec![Op::SetRangeInputs {
        sheet: f.sheet,
        r,
        c,
        rows,
    }]);
    assert!(res.errors.is_empty(), "{:?}", res.errors);
}

fn add_sheet(f: &mut Fixture, name: &str) -> SheetId {
    f.apply(vec![Op::AddSheet {
        name: Some(name.into()),
        index: None,
    }]);
    let sheets = f.svc.info(f.doc).unwrap().sheets;
    sheets.iter().find(|s| s.name == name).unwrap().id
}

/// P6 (deliberate deviation): a sheet reference written without `$`
/// keeps naming its sheet when the formula is copied to another sheet.
/// In AOO it is relative to the formula's tab (`ScCompiler` sets
/// `SetTabRel` unless `SCA_TAB_ABSOLUTE`; a copied cell keeps `nRelTab`,
/// `ScSingleRefData::CalcAbsIfRel`), so the copy two tabs to the right
/// would name the sheet two tabs after `Two`, or `#REF!`.
#[test]
fn p6_sheet_reference_without_dollar_keeps_its_sheet_when_copied() {
    let mut f = Fixture::new();
    let s1 = f.sheet;
    let two = add_sheet(&mut f, "Two");
    let three = add_sheet(&mut f, "Three");
    add_sheet(&mut f, "Four");
    f.apply(vec![common::input(two, "A1", "7")]);
    f.set("B1", "=Two.A1");
    f.set("B2", "=$Two.A1");
    let token = f.svc.copy(f.doc, &sel(s1, "B1:B2")).unwrap().token;
    let r = f.apply(vec![Op::PasteSpecial {
        sheet: three,
        target: range("B1"),
        payload: PastePayload::Internal { token },
        options: PasteOptions::default(),
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(input_of(&f, three, "B1"), "=Two.A1");
    assert_eq!(input_of(&f, three, "B2"), "=$Two.A1");
    assert_eq!(f.value_on(three, "B1"), n(7.0));
}

fn stop_list(a1: &str) -> ValidationDto {
    ValidationDto {
        id: 0,
        ranges: vec![range(a1)],
        kind: ValidationKind::List {
            source: ListSource::Values {
                items: vec!["Paid".into(), "Due".into()],
            },
        },
        allow_blank: true,
        show_dropdown: true,
        sort_ascending: false,
        input_help: None,
        error: None,
    }
}

/// G12: Delete Contents ▸ Formats dissolves the merges that start in
/// the range and removes the conditional format and the validation
/// there, as AOO `ScAttrArray::DeleteArea` (`RemoveAreaMerge`, then the
/// default pattern, which carries `ATTR_CONDITIONAL` / `ATTR_VALIDDATA`).
/// A merge that only reaches into the range stays, and the contents are
/// not touched.
#[test]
fn g12_delete_contents_formats_clears_merges_cf_and_validation() {
    let mut f = Fixture::new();
    let s = f.sheet;
    f.set("A1", "5");
    f.set("B3", "keep");
    let r = f.apply(vec![
        Op::MergeCells {
            sheet: s,
            range: range("B3:C4"),
        },
        // Starts outside the range cleared below (column A).
        Op::MergeCells {
            sheet: s,
            range: range("A6:B6"),
        },
        Op::AddConditionalFormat {
            sheet: s,
            ranges: vec![range("A1:A8")],
            rules: vec![CfRuleDto {
                kind: CfRuleKind::CellValue {
                    op: CmpOp::Gt,
                    formulas: vec!["1".into()],
                },
                style: CfStyleDto {
                    bold: Some(true),
                    ..CfStyleDto::default()
                },
                stop_if_true: false,
            }],
        },
        Op::AddValidation {
            sheet: s,
            validation: stop_list("B1:B8"),
        },
    ]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    let before = f.data();
    let r = f.apply(vec![Op::DeleteContents {
        sheet: s,
        ranges: vec![range("B1:D8")],
        what: DeleteContentsFlags {
            formats: true,
            ..DeleteContentsFlags::default()
        },
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    let d = f.data();
    let sh = &d.sheets[0];
    assert_eq!(sh.merges.len(), 1, "{:?}", sh.merges);
    assert_eq!((sh.merges[0].r0, sh.merges[0].c0), (5, 0), "A6:B6 stays");
    assert!(sh.validations.is_empty(), "{:?}", sh.validations);
    assert_eq!(sh.conditional_formats.len(), 1, "column A is outside");
    assert_eq!(f.value("B3"), Value::text("keep"));
    // Over the conditional format too.
    let r = f.apply(vec![Op::DeleteContents {
        sheet: s,
        ranges: vec![range("A1:A4")],
        what: DeleteContentsFlags {
            formats: true,
            ..DeleteContentsFlags::default()
        },
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    let d = f.data();
    let cf = &d.sheets[0].conditional_formats;
    assert_eq!(cf.len(), 1);
    assert_eq!(cf[0].ranges.len(), 1);
    assert_eq!((cf[0].ranges[0].r0, cf[0].ranges[0].r1), (4, 7), "A5:A8");
    assert_eq!(f.value("A1"), n(5.0));
    // Undo brings everything back.
    f.svc.undo(f.doc).unwrap();
    f.svc.undo(f.doc).unwrap();
    assert_eq!(f.data(), before);
}

fn cond(col: u32, op: StdFilterOp, value: &str) -> StdCondition {
    StdCondition {
        connector: StdConnector::And,
        col,
        op,
        value: value.into(),
    }
}

fn filtered_rows(f: &Fixture) -> Vec<(u32, u32)> {
    f.svc
        .sheet_layout(f.doc, f.sheet)
        .unwrap()
        .row_runs
        .iter()
        .filter(|r| r.hidden && r.filtered == Some(true))
        .map(|r| (r.start, r.end))
        .collect()
}

fn filter(f: &mut Fixture, a1: &str, c: StdCondition, regex: bool) -> Vec<(u32, u32)> {
    let r = f.apply(vec![Op::StandardFilter {
        sheet: f.sheet,
        range: range(a1),
        conditions: vec![c],
        options: StdFilterOptions {
            has_header: true,
            regex,
            ..StdFilterOptions::default()
        },
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    filtered_rows(f)
}

/// T2: "Does not begin with" and "Does not end with" exist and negate
/// "Begins with" / "Ends with" on the cell's input string, plain and as
/// regular expressions (AOO `ScTable::ValidQuery`,
/// `SC_DOES_NOT_BEGIN_WITH` / `SC_DOES_NOT_END_WITH`).
#[test]
fn t2_does_not_begin_or_end_with() {
    let mut f = Fixture::new();
    rows(
        &mut f,
        0,
        0,
        &[&["Name"], &["Anna"], &["Bob"], &["alba"], &["Cab"], &[""]],
    );
    // Rows (0-based) 1 Anna, 2 Bob, 3 alba, 4 Cab, 5 empty.
    use StdFilterOp::{NotBeginsWith, NotEndsWith};
    assert_eq!(
        filter(&mut f, "A1:A6", cond(0, NotBeginsWith, "a"), false),
        vec![(1, 1), (3, 3)]
    );
    assert_eq!(
        filter(&mut f, "A1:A6", cond(0, NotEndsWith, "b"), false),
        vec![(2, 2), (4, 4)]
    );
    assert_eq!(
        filter(&mut f, "A1:A6", cond(0, NotBeginsWith, "[ab]"), true),
        vec![(1, 3)]
    );
    assert_eq!(
        filter(&mut f, "A1:A6", cond(0, NotEndsWith, "a|b"), true),
        vec![(1, 4)]
    );
    // An empty value is never found (AOO `String::Search`): "Begins
    // with" passes no row, "Does not begin with" every row.
    assert_eq!(
        filter(&mut f, "A1:A6", cond(0, StdFilterOp::BeginsWith, ""), false),
        vec![(1, 5)]
    );
    assert_eq!(
        filter(&mut f, "A1:A6", cond(0, NotBeginsWith, ""), false),
        vec![]
    );
}

/// T2 (deviation kept): the text conditions look at a number cell's
/// input string. AOO takes the "by value" branch of `ValidQuery` for a
/// number cell and a query that scans as a number, and that branch has
/// no case for the text operators, so such a cell never passes, the
/// negated conditions included.
#[test]
fn t2_text_conditions_test_the_input_string_of_numbers() {
    let mut f = Fixture::new();
    rows(&mut f, 0, 0, &[&["N"], &["12"], &["21"], &["x1"]]);
    use StdFilterOp::{BeginsWith, NotBeginsWith};
    assert_eq!(
        filter(&mut f, "A1:A4", cond(0, BeginsWith, "1"), false),
        vec![(2, 3)]
    );
    assert_eq!(
        filter(&mut f, "A1:A4", cond(0, NotBeginsWith, "1"), false),
        vec![(1, 1)]
    );
}

fn subtotal_table(f: &mut Fixture) {
    rows(
        f,
        0,
        0,
        &[
            &["Dept", "Name", "Pay", "Bonus"],
            &["Ops", "Ann", "10", "1"],
            &["Ops", "Bob", "30", "2"],
            &["Sales", "Ann", "20", "3"],
        ],
    );
}

/// T7 / review 4 L1: the grand total row totals the first group's
/// columns with the first group's function only (AOO
/// `ScTable::DoSubTotals`: `aRowEntry.nGroupNo = bTotal ? 0 : …`).
#[test]
fn t7_grand_total_uses_the_first_group() {
    let mut f = Fixture::new();
    subtotal_table(&mut f);
    let r = f.apply(vec![Op::Subtotals {
        sheet: f.sheet,
        range: range("A1:D4"),
        groups: vec![
            SubtotalGroup {
                group_by: 0,
                columns: vec![2],
                function: AggFn::Sum,
            },
            SubtotalGroup {
                group_by: 1,
                columns: vec![3],
                function: AggFn::Max,
            },
        ],
        options: SubtotalOptions::default(),
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    let last = (1..40u32)
        .find(|r| f.text(&format!("A{r}")) == "Grand Total")
        .unwrap();
    let s = f.sheet;
    assert_eq!(
        input_of(&f, s, &format!("C{last}")),
        format!("=SUBTOTAL(9,C2:C{})", last - 1)
    );
    assert_eq!(input_of(&f, s, &format!("D{last}")), "");
    assert_eq!(f.value(&format!("C{last}")), n(60.0));
}

fn consolidate(f: &mut Fixture, sources: &[&str], dest: (u32, u32), link: bool) {
    let sheet = f.sheet;
    let r = f.apply(vec![Op::Consolidate {
        function: AggFn::Sum,
        sources: sources
            .iter()
            .map(|a| SheetRange {
                sheet,
                range: range(a),
            })
            .collect(),
        dest: SheetCell {
            sheet,
            r: dest.0,
            c: dest.1,
        },
        by_row_labels: true,
        by_col_labels: true,
        link,
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
}

/// T7 / review 4 L2: a row or column with an empty label is left out of
/// a consolidation by labels, and the shared top-left text is kept (AOO
/// `ScConsData::AddFields` / `AddData` / `OutputToDocument`).
#[test]
fn t7_consolidate_skips_empty_labels_and_keeps_the_corner() {
    let mut f = Fixture::new();
    rows(
        &mut f,
        0,
        0,
        &[
            &["Fruit", "Q1", "", "Q2"],
            &["Apple", "10", "99", "1"],
            &["", "77", "88", "66"],
            &["Pear", "5", "99", "2"],
        ],
    );
    rows(
        &mut f,
        0,
        5,
        &[
            &["Fruit", "Q2", "Q1"],
            &["Pear", "7", "3"],
            &["", "55", "44"],
        ],
    );
    consolidate(&mut f, &["A1:D4", "F1:H3"], (9, 0), false);
    let got: Vec<Vec<String>> = (10..=13)
        .map(|r| {
            ["A", "B", "C", "D"]
                .iter()
                .map(|c| f.text(&format!("{c}{r}")))
                .collect()
        })
        .collect();
    assert_eq!(
        got,
        vec![
            vec!["Fruit", "Q1", "Q2", ""],
            vec!["Apple", "10", "1", ""],
            vec!["Pear", "8", "9", ""],
            vec!["", "", "", ""],
        ]
    );
}

/// T7 / review 4 L3 (deviation kept): "Link to source data" writes one
/// formula per result cell that names every source cell. AOO inserts one
/// row per source cell above each result row, each a single reference,
/// totals them with the function over those rows and groups them in an
/// outline (`ScConsData::OutputToDocument`).
#[test]
fn t7_consolidate_link_writes_one_formula_per_cell() {
    let mut f = Fixture::new();
    rows(
        &mut f,
        0,
        0,
        &[&["", "Q1"], &["Apple", "10"], &["Pear", "5"]],
    );
    rows(&mut f, 0, 3, &[&["", "Q1"], &["Pear", "7"]]);
    consolidate(&mut f, &["A1:B3", "D1:E2"], (9, 0), true);
    let s = f.sheet;
    assert_eq!(f.text("A11"), "Apple");
    assert_eq!(input_of(&f, s, "B11"), "=SUM($Sheet1.B2)");
    assert_eq!(f.text("A12"), "Pear");
    assert_eq!(input_of(&f, s, "B12"), "=SUM($Sheet1.B3,$Sheet1.E2)");
    assert_eq!(f.value("B12"), n(12.0));
    assert!(f.data().sheets[0].row_groups.is_empty());
}

/// T7 / review 4 L4: Text to Columns takes a quote as a string
/// delimiter only at the start of a field (AOO
/// `ScImportExport::ScanNextFieldFromString`).
#[test]
fn t7_text_to_columns_quote_only_at_field_start() {
    let mut f = Fixture::new();
    f.set("A1", "ab\"c,d\"e,f");
    f.set("A2", "\"x,y\"z,w");
    let r = f.apply(vec![Op::TextToColumns {
        sheet: f.sheet,
        range: range("A1:A2"),
        options: CsvImportOptionsDto {
            delimiters: ",".into(),
            ..CsvImportOptionsDto::default()
        },
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(f.text("A1"), "ab\"c");
    assert_eq!(f.text("B1"), "d\"e");
    assert_eq!(f.text("C1"), "f");
    assert_eq!(f.text("A2"), "x,yz");
    assert_eq!(f.text("B2"), "w");
    assert_eq!(f.text("C2"), "");
}

/// T7 / review 4 L5: Remove Duplicates (a Tally extra) compares the
/// displayed text without case, the rule of AOO's "No duplications"
/// filter option (`ScTable::Query`: `GetString` of every column), so 1
/// and 1 shown as 1.0 are different rows.
#[test]
fn t7_remove_duplicates_compares_displayed_text() {
    let mut f = Fixture::new();
    rows(&mut f, 0, 0, &[&["1"], &["1"], &["abc"], &["ABC"], &["1"]]);
    let r = f.apply(vec![Op::SetNumberFormat {
        sheet: f.sheet,
        ranges: vec![range("A2")],
        code: "0.0".into(),
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    let r = f.apply(vec![Op::RemoveDuplicates {
        sheet: f.sheet,
        range: range("A1:A5"),
        cols: vec![],
        has_header: false,
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(r.replaced, Some(2));
    assert_eq!(f.text("A1"), "1");
    assert_eq!(f.text("A2"), "1.0");
    assert_eq!(f.text("A3"), "abc");
    assert_eq!(f.value("A4"), Value::Empty);
}

fn scenario(name: &str, copy_back: bool, prevent_changes: bool) -> ScenarioDto {
    ScenarioDto {
        id: 0,
        name: name.into(),
        comment: String::new(),
        ranges: vec![range("A1:A2")],
        border_color: None,
        display_border: true,
        copy_back,
        copy_styles: false,
        prevent_changes,
        active: false,
    }
}

fn protect(f: &mut Fixture, on: bool) {
    let op = if on {
        Op::ProtectSheet {
            sheet: f.sheet,
            password: None,
            options: None,
        }
    } else {
        Op::UnprotectSheet {
            sheet: f.sheet,
            password: None,
        }
    };
    let res = f.apply(vec![op]);
    assert!(res.errors.is_empty(), "{:?}", res.errors);
}

/// T8 / review 4 L13: on a protected sheet the unlocked cells of the
/// scenario being shown cannot be edited when it has "Prevent changes"
/// and "Copy back" (AOO `ScTable::IsBlockEditable`), and such a scenario
/// cannot be changed or deleted (`ScScenarioListBox`). Showing another
/// scenario only needs unlocked cells (`ScTable::TestCopyScenarioTo`).
#[test]
fn t8_prevent_changes_on_a_protected_sheet() {
    let mut f = Fixture::new();
    let s = f.sheet;
    f.set("A1", "10");
    f.set("A2", "20");
    f.set("C1", "1");
    let r = f.apply(vec![Op::SetCellProtection {
        sheet: s,
        ranges: vec![range("A1:C2")],
        locked: Some(false),
        hidden_formula: None,
        hidden: None,
        hide_print: None,
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    let r = f.apply(vec![Op::CreateScenario {
        sheet: s,
        scenario: scenario("Frozen", true, true),
    }]);
    let frozen = r.created.unwrap()[0].id;
    f.set("A1", "11");
    let r = f.apply(vec![Op::CreateScenario {
        sheet: s,
        scenario: scenario("One way", false, true),
    }]);
    let one_way = r.created.unwrap()[0].id;
    // Not protected: the flag does nothing.
    let r = f.apply(vec![Op::ShowScenario {
        sheet: s,
        id: frozen,
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert!(f.set("A1", "12").errors.is_empty());
    protect(&mut f, true);
    // "Frozen" is shown: its cells are read-only, other unlocked cells
    // are not.
    assert!(!f.set("A1", "13").errors.is_empty());
    assert_eq!(f.value("A1"), n(12.0));
    assert!(f.set("C1", "2").errors.is_empty());
    // Neither its settings nor the scenario itself can go.
    let r = f.apply(vec![Op::UpdateScenario {
        sheet: s,
        id: frozen,
        scenario: scenario("Renamed", true, false),
    }]);
    assert!(!r.errors.is_empty());
    let r = f.apply(vec![Op::DeleteScenario {
        sheet: s,
        id: frozen,
    }]);
    assert!(!r.errors.is_empty());
    assert_eq!(f.svc.scenarios(f.doc, s).unwrap().len(), 2);
    // Showing the other scenario works; without "Copy back" its cells
    // can be edited.
    let r = f.apply(vec![Op::ShowScenario {
        sheet: s,
        id: one_way,
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(f.value("A1"), n(11.0));
    assert!(f.set("A1", "14").errors.is_empty());
    protect(&mut f, false);
    let r = f.apply(vec![Op::DeleteScenario {
        sheet: s,
        id: frozen,
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
}

/// T8 / review 4 L14: copying a sheet does not copy its named database
/// ranges, whose names are unique in the document (AOO
/// `ScDocument::CopyTab` only shifts `pDBCollection`).
#[test]
fn t8_copy_sheet_does_not_duplicate_database_ranges() {
    let mut f = Fixture::new();
    rows(&mut f, 0, 0, &[&["Name"], &["Ann"], &["Bob"]]);
    let r = f.apply(vec![Op::DefineDatabaseRange {
        name: "db1".into(),
        sheet: f.sheet,
        range: range("A1:A3"),
        has_header: true,
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    let r = f.apply(vec![Op::CopySheet {
        sheet: f.sheet,
        name: None,
        index: None,
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    let ranges = f.svc.database_ranges(f.doc).unwrap();
    assert_eq!(ranges.len(), 1, "{ranges:?}");
    assert_eq!(ranges[0].name, "db1");
    let d = f.data();
    assert_eq!(d.sheets[0].database_ranges.len(), 1);
    assert!(d.sheets[1].database_ranges.is_empty());
    assert_eq!(
        f.value_on(r.created.unwrap()[0].id, "A2"),
        Value::text("Ann")
    );
}

/// Review 6 M3: on a protected sheet Delete Contents ▸ Formats does not
/// remove what the direct commands refuse there. Merge / Unmerge and the
/// conditional format and validation commands are refused on a protected
/// sheet whatever it allows, so a Formats delete that would take a merge,
/// a conditional format or a validation with it is refused as a whole
/// (as every protection test: nothing changes); where there is none, the
/// cell formats still go.
#[test]
fn g12_delete_contents_formats_on_a_protected_sheet() {
    let formats = |a1: &str, sheet| Op::DeleteContents {
        sheet,
        ranges: vec![range(a1)],
        what: DeleteContentsFlags {
            formats: true,
            ..DeleteContentsFlags::default()
        },
    };
    let mut f = g12_protected();
    let s = f.sheet;
    let before = f.data();
    // The direct commands are refused.
    let r = f.apply(vec![Op::UnmergeCells {
        sheet: s,
        range: range("B3:C4"),
    }]);
    assert!(!r.errors.is_empty());
    // And so is a Formats delete over the merge, the conditional format
    // or the validation; nothing changes, the cell formats included.
    for a1 in ["B3:C4", "E2", "G4:H5", "A1:H8"] {
        let r = f.apply(vec![formats(a1, s)]);
        assert_eq!(r.errors.len(), 1, "{a1}");
        assert_eq!(r.errors[0].code, "protected", "{a1}");
        assert_eq!(f.data(), before, "{a1}");
    }
    // A merge that only reaches into the range is not removed (G12), so
    // it does not refuse it; neither do cells with none of the three.
    let r = f.apply(vec![formats("C4:D4", s), formats("A6:H8", s)]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert!(!is_bold(&f, "D4") && !is_bold(&f, "A6"));
    assert!(is_bold(&f, "A1"));
    let d = f.data();
    assert_eq!(d.sheets[0].merges, before.sheets[0].merges);
    assert_eq!(
        d.sheets[0].conditional_formats,
        before.sheets[0].conditional_formats
    );
    assert_eq!(d.sheets[0].validations, before.sheets[0].validations);
    // Not protected: everything goes, as before.
    protect(&mut f, false);
    let r = f.apply(vec![formats("A1:H8", s)]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    let d = f.data();
    assert!(d.sheets[0].merges.is_empty());
    assert!(d.sheets[0].conditional_formats.is_empty());
    assert!(d.sheets[0].validations.is_empty());
    assert!(!is_bold(&f, "A1"));
}

fn is_bold(f: &Fixture, a1: &str) -> bool {
    let a = CellAddr::from_a1(f.sheet, a1).unwrap();
    let wb = &f.svc.document(f.doc).unwrap().wb;
    wb.styles.to_dto(wb.style_at(a)).bold
}

/// A protected sheet that allows Format Cells, with A1:H8 bold and
/// unlocked (only the sheet's protection is in the way), a merge at
/// B3:C4, a conditional format on E1:E4 and a validation on G1:G4.
fn g12_protected() -> Fixture {
    let bold = |a1: &str, sheet| Op::ApplyStyle {
        sheet,
        ranges: vec![range(a1)],
        patch: tally_api::StylePatch {
            bold: Some(true),
            ..Default::default()
        },
    };
    let mut f = Fixture::new();
    let s = f.sheet;
    let r = f.apply(vec![
        Op::MergeCells {
            sheet: s,
            range: range("B3:C4"),
        },
        Op::AddConditionalFormat {
            sheet: s,
            ranges: vec![range("E1:E4")],
            rules: vec![CfRuleDto {
                kind: CfRuleKind::CellValue {
                    op: CmpOp::Gt,
                    formulas: vec!["1".into()],
                },
                style: CfStyleDto {
                    bold: Some(true),
                    ..CfStyleDto::default()
                },
                stop_if_true: false,
            }],
        },
        Op::AddValidation {
            sheet: s,
            validation: stop_list("G1:G4"),
        },
        bold("A1:H8", s),
        // Every cell unlocked: only the sheet's protection is in the way.
        Op::SetCellProtection {
            sheet: s,
            ranges: vec![range("A1:H8")],
            locked: Some(false),
            hidden_formula: None,
            hidden: None,
            hide_print: None,
        },
        Op::ProtectSheet {
            sheet: s,
            password: None,
            options: Some(tally_api::ProtectionOptions {
                select_locked: true,
                select_unlocked: true,
                insert_rows: false,
                insert_cols: false,
                delete_rows: false,
                delete_cols: false,
                format_cells: true,
                sort: false,
                auto_filter: false,
            }),
        },
    ]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    f
}

fn clear(a1: &str, sheet: SheetId, what: ClearWhat) -> Op {
    Op::Clear {
        sheet,
        ranges: vec![range(a1)],
        what,
    }
}

/// G12: `Op::Clear` All and Formats follow the same rule as Delete
/// Contents ▸ Formats on a protected sheet: both take the conditional
/// formats and validations of the cells, so both are refused as a whole
/// where there is one (nothing changes, contents included). Neither
/// dissolves a merge, so a merge does not refuse them. Clear Contents is
/// not concerned.
#[test]
fn g12_clear_all_and_formats_on_a_protected_sheet() {
    for what in [ClearWhat::All, ClearWhat::Formats] {
        let mut f = g12_protected();
        let s = f.sheet;
        f.set("E2", "7");
        f.set("G4", "Paid");
        f.set("B3", "merged");
        f.set("A6", "plain");
        let before = f.data();
        for a1 in ["E2", "G4:H5", "A1:H8", "E4:G6"] {
            let r = f.apply(vec![clear(a1, s, what)]);
            assert_eq!(r.errors.len(), 1, "{what:?} {a1}");
            assert_eq!(r.errors[0].code, "protected", "{what:?} {a1}");
            assert_eq!(f.data(), before, "{what:?} {a1}");
        }
        // Several ranges: one with a validation refuses all of them.
        let r = f.apply(vec![Op::Clear {
            sheet: s,
            ranges: vec![range("A6:B6"), range("G1")],
            what,
        }]);
        assert_eq!(r.errors[0].code, "protected", "{what:?}");
        assert_eq!(f.data(), before, "{what:?}");
        // The merge stays, so it is not in the way; nor are plain cells.
        let r = f.apply(vec![clear("B3:C4", s, what), clear("A6:D8", s, what)]);
        assert!(r.errors.is_empty(), "{what:?} {:?}", r.errors);
        assert!(!is_bold(&f, "B3") && !is_bold(&f, "A6"), "{what:?}");
        assert!(is_bold(&f, "A1"), "{what:?}");
        let d = f.data();
        assert_eq!(d.sheets[0].merges, before.sheets[0].merges, "{what:?}");
        assert_eq!(
            d.sheets[0].conditional_formats, before.sheets[0].conditional_formats,
            "{what:?}"
        );
        assert_eq!(
            d.sheets[0].validations, before.sheets[0].validations,
            "{what:?}"
        );
        let gone = what == ClearWhat::All;
        assert_eq!(f.value("B3").is_empty(), gone, "{what:?}");
        assert_eq!(f.value("A6").is_empty(), gone, "{what:?}");
        // Contents alone never touch the three.
        let r = f.apply(vec![clear("E1:G4", s, ClearWhat::Contents)]);
        assert!(r.errors.is_empty(), "{what:?} {:?}", r.errors);
        assert!(f.value("E2").is_empty(), "{what:?}");
        let d = f.data();
        assert_eq!(
            d.sheets[0].conditional_formats, before.sheets[0].conditional_formats,
            "{what:?}"
        );
        assert_eq!(
            d.sheets[0].validations, before.sheets[0].validations,
            "{what:?}"
        );
        // Not protected: the conditional format and the validation go,
        // the merge stays.
        protect(&mut f, false);
        let r = f.apply(vec![clear("A1:H8", s, what)]);
        assert!(r.errors.is_empty(), "{what:?} {:?}", r.errors);
        let d = f.data();
        assert_eq!(d.sheets[0].merges, before.sheets[0].merges, "{what:?}");
        assert!(d.sheets[0].conditional_formats.is_empty(), "{what:?}");
        assert!(d.sheets[0].validations.is_empty(), "{what:?}");
        assert!(!is_bold(&f, "A1"), "{what:?}");
    }
}

/// G12: Clear formats is AOO's Default Formatting (`SID_CELL_FORMAT_RESET`
/// = `DeleteContents(IDF_HARDATTR | IDF_EDITATTR)`, cellsh3.cxx):
/// `ScAttrArray::DeleteHardAttr` clears every pattern item but
/// `ATTR_MERGE` / `ATTR_MERGE_FLAG`, so the conditional format
/// (`ATTR_CONDITIONAL`) and the validation (`ATTR_VALIDDATA`) of the cells
/// go with the formats, the merges and the contents stay; one undo step
/// brings everything back.
#[test]
fn g12_clear_formats_takes_cf_and_validation_and_keeps_merges() {
    let mut f = g12_protected();
    let s = f.sheet;
    protect(&mut f, false);
    f.set("E2", "7");
    f.set("B3", "merged");
    let before = f.data();
    // Part of each: the rest of the conditional format and of the
    // validation stays.
    let r = f.apply(vec![clear("B2:G3", s, ClearWhat::Formats)]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    let d = f.data();
    assert_eq!(d.sheets[0].merges, before.sheets[0].merges);
    assert_eq!(d.sheets[0].conditional_formats.len(), 1);
    assert_eq!(d.sheets[0].validations.len(), 1);
    assert_ne!(
        d.sheets[0].conditional_formats,
        before.sheets[0].conditional_formats
    );
    assert_ne!(d.sheets[0].validations, before.sheets[0].validations);
    assert!(!is_bold(&f, "E2") && is_bold(&f, "E1") && is_bold(&f, "E4"));
    assert_eq!(f.value("E2"), n(7.0));
    assert_eq!(f.value("B3"), Value::text("merged"));
    // The validation no longer applies to G2, still does to G1.
    f.set("G2", "anything");
    assert_eq!(f.value("G2"), Value::text("anything"));
    f.svc.undo(f.doc).unwrap();
    // All of both.
    let r = f.apply(vec![clear("A1:H8", s, ClearWhat::Formats)]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    let d = f.data();
    assert_eq!(d.sheets[0].merges, before.sheets[0].merges);
    assert!(d.sheets[0].conditional_formats.is_empty());
    assert!(d.sheets[0].validations.is_empty());
    assert_eq!(f.value("E2"), n(7.0));
    f.svc.undo(f.doc).unwrap();
    f.svc.undo(f.doc).unwrap();
    assert_eq!(f.data(), before);
}

/// T8 / review 6 L6: with several active scenarios over a cell (only a
/// file can have that), the first one decides, as the loop of AOO
/// `ScTable::IsBlockEditable` over the scenario sheets breaks at the
/// first active one with the range.
#[test]
fn t8_the_first_active_scenario_over_a_cell_decides() {
    let load = |first_frozen: bool| {
        let mut f = Fixture::new();
        let s = f.sheet;
        f.set("A1", "10");
        let r = f.apply(vec![
            Op::SetCellProtection {
                sheet: s,
                ranges: vec![range("A1:C2")],
                locked: Some(false),
                hidden_formula: None,
                hidden: None,
                hide_print: None,
            },
            Op::CreateScenario {
                sheet: s,
                scenario: scenario("One", true, first_frozen),
            },
            Op::CreateScenario {
                sheet: s,
                scenario: scenario("Two", true, !first_frozen),
            },
        ]);
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        protect(&mut f, true);
        let mut data = f.data();
        assert_eq!(data.sheets[0].scenarios.len(), 2);
        for sc in &mut data.sheets[0].scenarios {
            sc.active = true;
        }
        let mut svc = tally_engine::EngineService::new();
        let info = svc.open_workbook(tally_engine::workbook::Workbook::from_data(&data), "t");
        Fixture {
            svc,
            doc: info.doc,
            sheet: info.sheets[0].id,
        }
    };
    // "One" comes first and prevents changes.
    let mut f = load(true);
    assert!(!f.set("A1", "11").errors.is_empty());
    assert!(f.set("C1", "1").errors.is_empty());
    // "One" comes first and does not: "Two" behind it is not asked.
    let mut f = load(false);
    assert!(f.set("A1", "11").errors.is_empty());
}
