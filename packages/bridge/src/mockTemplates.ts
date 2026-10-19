/**
 * Template contents for `MockClient.newWorkbook(id)`, mirroring the
 * workbooks `tally_engine::service::templates` builds. The mock evaluates
 * only simple formulas, so computed text columns (Stock "Status") are
 * stored as values here; the engine uses `=IF(C2<D2,"Low","OK")`.
 */

export interface MockTemplate {
  /** Document title. */
  title: string;
  /** Sheet name. */
  sheet: string;
  /** Rows of inputs, header first. */
  rows: string[][];
  /** Columns (0-based) shown as currency, body rows only. */
  currencyCols: number[];
  /** Column widths in px. */
  widths: number[];
}

const INVOICES: [string, string, string, number, number, string][] = [
  ["10/01/2026", "Maple & Co. Bakery", "Flour (25 kg)", 12, 4.5, "Paid"],
  ["10/02/2026", "Northside Café", "Coffee beans", 30, 2.25, "Due"],
  ["10/03/2026", "Harbour Deli", "Olive oil", 8, 6.0, "Paid"],
  ["10/05/2026", "Green Grocer", "Paper bags", 20, 3.1, "Overdue"],
  ["10/06/2026", "Blue Door Bistro", "Napkins", 15, 1.8, "Paid"],
  ["10/07/2026", "Maple & Co. Bakery", "Sugar (10 kg)", 10, 5.25, "Due"],
  ["10/08/2026", "Riverside Market", "Paper cups", 40, 0.95, "Paid"],
  ["10/09/2026", "Northside Café", "Milk", 24, 1.2, "Paid"],
  ["10/12/2026", "Harbour Deli", "Cheese", 6, 9.75, "Overdue"],
  ["10/13/2026", "Green Grocer", "Crates", 5, 12.0, "Paid"],
  ["10/14/2026", "Sunrise Diner", "Eggs (dozen)", 18, 3.4, "Due"],
  ["10/15/2026", "Blue Door Bistro", "Wine glasses", 12, 4.1, "Paid"],
  ["10/16/2026", "Riverside Market", "Labels", 50, 0.4, "Due"],
  ["10/19/2026", "Sunrise Diner", "Maple syrup", 9, 6.6, "Paid"],
];

const BUDGET: [string, number, number][] = [
  ["Rent", 2000, 2000],
  ["Salaries", 8500, 8650],
  ["Utilities", 450, 512],
  ["Marketing", 1200, 980],
  ["Software", 600, 640],
  ["Travel", 800, 455],
  ["Office supplies", 300, 276],
  ["Insurance", 350, 350],
];

const STOCK: [string, string, number, number, number][] = [
  ["Flour (kg)", "FL-025", 42, 20, 1.1],
  ["Sugar (kg)", "SU-010", 18, 15, 0.95],
  ["Coffee beans (kg)", "CB-001", 6, 10, 14.5],
  ["Milk (L)", "MK-001", 24, 30, 1.2],
  ["Eggs (dozen)", "EG-012", 15, 12, 3.4],
  ["Butter (kg)", "BT-005", 4, 8, 8.75],
  ["Paper cups", "PC-100", 600, 250, 0.08],
  ["Napkins", "NP-500", 1200, 500, 0.02],
  ["Paper bags", "PB-050", 140, 200, 0.12],
  ["Olive oil (L)", "OO-005", 9, 6, 7.8],
  ["Maple syrup (L)", "MS-001", 3, 5, 11.2],
  ["Labels", "LB-250", 900, 300, 0.03],
];

/** The template with `id`, or `undefined`. */
export function mockTemplate(id: string): MockTemplate | undefined {
  switch (id.toLowerCase()) {
    case "invoices":
      return {
        title: "Invoices",
        sheet: "Orders",
        rows: [
          ["Date", "Customer", "Item", "Qty", "Price", "Total", "Status"],
          ...INVOICES.map(([d, cust, item, qty, price, status], i) => [
            d,
            cust,
            item,
            String(qty),
            String(price),
            `=D${i + 2}*E${i + 2}`,
            status,
          ]),
        ],
        currencyCols: [4, 5],
        widths: [100, 170, 140, 60, 90, 100, 90],
      };
    case "budget": {
      const total = BUDGET.length + 1;
      return {
        title: "Budget",
        sheet: "Budget",
        rows: [
          ["Category", "Budget", "Actual", "Difference", "% Used"],
          ...BUDGET.map(([cat, b, a], i) => [cat, String(b), String(a), `=B${i + 2}-C${i + 2}`, `=C${i + 2}/B${i + 2}`]),
          ["Total", `=SUM(B2:B${total})`, `=SUM(C2:C${total})`, `=SUM(D2:D${total})`, `=C${total + 1}/B${total + 1}`],
        ],
        currencyCols: [1, 2, 3],
        widths: [150, 100, 100, 100, 80],
      };
    }
    case "stock":
      return {
        title: "Stock",
        sheet: "Stock",
        rows: [
          ["Item", "SKU", "On hand", "Reorder at", "Unit cost", "Value", "Status"],
          ...STOCK.map(([item, sku, onHand, reorder, cost], i) => [
            item,
            sku,
            String(onHand),
            String(reorder),
            String(cost),
            `=C${i + 2}*E${i + 2}`,
            onHand < reorder ? "Low" : "OK",
          ]),
        ],
        currencyCols: [4, 5],
        widths: [160, 90, 80, 90, 90, 100, 70],
      };
    default:
      return undefined;
  }
}
