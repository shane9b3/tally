//! Bundled sample workbooks (`new_workbook(Some("invoices"))`, `"budget"`),
//! built in code to match the owner's mockup.

use tally_api::StylePatch;
use tally_numfmt::{builtin_format, QuickFormat, EN_US};
use tally_types::{CellAddr, SheetId};

use crate::units::px_to_hmm;
use crate::workbook::Workbook;

/// Template ids accepted by `new_workbook`.
pub const TEMPLATES: &[&str] = &["invoices", "budget"];

fn put(wb: &mut Workbook, sheet: SheetId, r: u32, c: u32, input: &str) {
    wb.set_input(CellAddr::new(sheet, r, c), input);
}

fn style_cells(
    wb: &mut Workbook,
    sheet: SheetId,
    r0: u32,
    c0: u32,
    r1: u32,
    c1: u32,
    patch: &StylePatch,
) {
    for c in c0..=c1 {
        let ids: Vec<(u32, crate::storage::StyleId)> = (r0..=r1)
            .map(|r| (r, wb.style_at(CellAddr::new(sheet, r, c))))
            .collect();
        for (r, old) in ids {
            let new = wb.styles.patched(old, patch);
            if let Some(col) = wb.sheet_mut(sheet).and_then(|s| s.column_mut(c)) {
                col.styles.set(r, r, new);
            }
        }
    }
}

fn widths(wb: &mut Workbook, sheet: SheetId, px: &[f32]) {
    if let Some(s) = wb.sheet_mut(sheet) {
        for (i, w) in px.iter().enumerate() {
            s.col_dims.set(i as u32, i as u32, px_to_hmm(*w), false);
        }
    }
}

fn bold() -> StylePatch {
    StylePatch {
        bold: Some(true),
        ..StylePatch::default()
    }
}

/// Builds a template workbook, or `None` for an unknown id.
pub fn build(id: &str) -> Option<(Workbook, &'static str)> {
    match id.to_ascii_lowercase().as_str() {
        "invoices" => Some((invoices(), "Invoices")),
        "budget" => Some((budget(), "Budget")),
        _ => None,
    }
}

fn invoices() -> Workbook {
    let mut wb = Workbook::empty();
    let s = wb.add_sheet(Some("Orders"), None);
    let header = [
        "Date", "Customer", "Item", "Qty", "Price", "Total", "Status",
    ];
    for (c, h) in header.iter().enumerate() {
        put(&mut wb, s, 0, c as u32, h);
    }
    let rows: [(&str, &str, &str, u32, &str, &str); 14] = [
        (
            "10/01/2026",
            "Maple & Co. Bakery",
            "Flour (25 kg)",
            12,
            "4.50",
            "Paid",
        ),
        (
            "10/02/2026",
            "Northside Café",
            "Coffee beans",
            30,
            "2.25",
            "Due",
        ),
        ("10/03/2026", "Harbour Deli", "Olive oil", 8, "6.00", "Paid"),
        (
            "10/05/2026",
            "Green Grocer",
            "Paper bags",
            20,
            "3.10",
            "Overdue",
        ),
        (
            "10/06/2026",
            "Blue Door Bistro",
            "Napkins",
            15,
            "1.80",
            "Paid",
        ),
        (
            "10/07/2026",
            "Maple & Co. Bakery",
            "Sugar (10 kg)",
            10,
            "5.25",
            "Due",
        ),
        (
            "10/08/2026",
            "Riverside Market",
            "Paper cups",
            40,
            "0.95",
            "Paid",
        ),
        ("10/09/2026", "Northside Café", "Milk", 24, "1.20", "Paid"),
        ("10/12/2026", "Harbour Deli", "Cheese", 6, "9.75", "Overdue"),
        ("10/13/2026", "Green Grocer", "Crates", 5, "12.00", "Paid"),
        (
            "10/14/2026",
            "Sunrise Diner",
            "Eggs (dozen)",
            18,
            "3.40",
            "Due",
        ),
        (
            "10/15/2026",
            "Blue Door Bistro",
            "Wine glasses",
            12,
            "4.10",
            "Paid",
        ),
        (
            "10/16/2026",
            "Riverside Market",
            "Labels",
            50,
            "0.40",
            "Due",
        ),
        (
            "10/19/2026",
            "Sunrise Diner",
            "Maple syrup",
            9,
            "6.60",
            "Paid",
        ),
    ];
    for (i, (date, cust, item, qty, price, status)) in rows.iter().enumerate() {
        let r = i as u32 + 1;
        put(&mut wb, s, r, 0, date);
        put(&mut wb, s, r, 1, cust);
        put(&mut wb, s, r, 2, item);
        put(&mut wb, s, r, 3, &qty.to_string());
        put(&mut wb, s, r, 4, &format!("${price}"));
        put(&mut wb, s, r, 5, &format!("=D{}*E{}", r + 1, r + 1));
        put(&mut wb, s, r, 6, status);
    }
    let currency = StylePatch {
        number_format: Some(builtin_format(QuickFormat::Currency, &EN_US)),
        ..StylePatch::default()
    };
    style_cells(&mut wb, s, 1, 4, 14, 5, &currency);
    style_cells(&mut wb, s, 0, 0, 0, 6, &bold());
    widths(&mut wb, s, &[100.0, 170.0, 140.0, 60.0, 90.0, 100.0, 90.0]);
    wb
}

fn budget() -> Workbook {
    let mut wb = Workbook::empty();
    let s = wb.add_sheet(Some("Budget"), None);
    for (c, h) in ["Category", "Budget", "Actual", "Difference", "% Used"]
        .iter()
        .enumerate()
    {
        put(&mut wb, s, 0, c as u32, h);
    }
    let rows: [(&str, u32, u32); 8] = [
        ("Rent", 2000, 2000),
        ("Salaries", 8500, 8650),
        ("Utilities", 450, 512),
        ("Marketing", 1200, 980),
        ("Software", 600, 640),
        ("Travel", 800, 455),
        ("Office supplies", 300, 276),
        ("Insurance", 350, 350),
    ];
    for (i, (cat, b, a)) in rows.iter().enumerate() {
        let r = i as u32 + 1;
        let n = r + 1;
        put(&mut wb, s, r, 0, cat);
        put(&mut wb, s, r, 1, &b.to_string());
        put(&mut wb, s, r, 2, &a.to_string());
        put(&mut wb, s, r, 3, &format!("=B{n}-C{n}"));
        put(&mut wb, s, r, 4, &format!("=C{n}/B{n}"));
    }
    let total = rows.len() as u32 + 1;
    put(&mut wb, s, total, 0, "Total");
    for (c, col) in [(1u32, 'B'), (2, 'C'), (3, 'D')] {
        put(&mut wb, s, total, c, &format!("=SUM({col}2:{col}{total})"));
    }
    put(
        &mut wb,
        s,
        total,
        4,
        &format!("=C{}/B{}", total + 1, total + 1),
    );
    let currency = StylePatch {
        number_format: Some(builtin_format(QuickFormat::Currency, &EN_US)),
        ..StylePatch::default()
    };
    let pct = StylePatch {
        number_format: Some(builtin_format(QuickFormat::Percent0, &EN_US)),
        ..StylePatch::default()
    };
    style_cells(&mut wb, s, 1, 1, total, 3, &currency);
    style_cells(&mut wb, s, 1, 4, total, 4, &pct);
    style_cells(&mut wb, s, 0, 0, 0, 4, &bold());
    style_cells(&mut wb, s, total, 0, total, 4, &bold());
    widths(&mut wb, s, &[150.0, 100.0, 100.0, 100.0, 80.0]);
    wb
}
