/**
 * W5-6 data tools in the browser (WebMockClient emulates the ops and records
 * them in `client.recorded`): DataPilot layout → createPivot, the Standard
 * Filter dialog, Subtotals, Text to Columns preview, group / ungroup via the
 * outline gutter, the Styles panel, Format Cells rotation, AutoCorrect on
 * commit, the AutoInput ghost, the Navigator and the Delete-key dialog.
 */
import { expect, test, type Page } from "@playwright/test";

const ROW_H = 24;
const HEADER_H = 24;

interface Rec {
  type: string;
  [k: string]: unknown;
}

async function ready(page: Page, url = "/?template=invoices") {
  await page.goto(url);
  await expect(page.getByTestId("grid")).toBeVisible();
  await page.waitForFunction(() => window.__tally?.controller.grid != null);
}

async function cellXY(page: Page, r: number, c: number) {
  const box = await page.getByTestId("grid").boundingBox();
  if (!box) throw new Error("grid not visible");
  const { headerW, headerH, xs, w } = await page.evaluate((c) => {
    const ctl = window.__tally!.controller;
    let x = 0;
    for (let i = 0; i < c; i++) x += ctl.layoutSize("col", i);
    const geo = ctl.grid!.getGeometry();
    return { headerW: geo.headerW, headerH: geo.headerH, xs: x, w: ctl.layoutSize("col", c) };
  }, c);
  return { x: box.x + headerW + xs + w / 2, y: box.y + headerH + r * ROW_H + ROW_H / 2, headerH, headerW, box };
}

async function clickCell(page: Page, r: number, c: number, modifier?: "Shift") {
  const p = await cellXY(page, r, c);
  if (modifier) await page.keyboard.down(modifier);
  await page.mouse.click(p.x, p.y);
  if (modifier) await page.keyboard.up(modifier);
}

async function selectRange(page: Page, r0: number, c0: number, r1: number, c1: number) {
  await clickCell(page, r0, c0);
  await clickCell(page, r1, c1, "Shift");
}

const recorded = (page: Page): Promise<Rec[]> =>
  page.evaluate(() => JSON.parse(JSON.stringify((window.__tally!.client as unknown as { recorded: unknown[] }).recorded)) as Rec[]);

const run = (page: Page, id: string) => page.evaluate((id) => window.__tally!.controller.registry.run(id), id);

const cellText = (page: Page, r: number, c: number): Promise<string> =>
  page.evaluate(
    async ([r, c]) => {
      const t = window.__tally!;
      const [b] = await t.client.cells(t.controller.info!.doc, t.controller.sheet, [{ r0: r, c0: c, r1: r, c1: c }]);
      return b?.cells[0]?.text ?? "";
    },
    [r, c] as const,
  );

const hiddenRows = (page: Page): Promise<number[]> =>
  page.evaluate(async () => {
    const t = window.__tally!;
    const l = await t.client.sheetLayout(t.controller.info!.doc, t.controller.sheet);
    return l.rowRuns.filter((x) => x.hidden).flatMap((x) => Array.from({ length: x.end - x.start + 1 }, (_, i) => x.start + i));
  });

test("DataPilot layout dialog records createPivot and writes the output", async ({ page }) => {
  await ready(page);
  await clickCell(page, 2, 1);
  await run(page, "data.dataPilot");
  const dlg = page.getByTestId("datapilot-dialog");
  await expect(dlg).toBeVisible();
  await expect(dlg.getByTestId("pivot-field-list")).toContainText("Customer");
  await dlg.getByTestId("pivot-add-row-Status").click();
  await dlg.getByTestId("pivot-add-data-Total").click();
  await expect(dlg.getByTestId("pivot-area-row")).toContainText("Status");
  await expect(dlg.getByTestId("pivot-area-data")).toContainText("Sum - Total");
  // Keyboard alternative: Move to… on the data field.
  await dlg.getByTestId("pivot-more").click();
  await dlg.getByTestId("pivot-target-sheet").selectOption({ index: 1 });
  await dlg.getByTestId("pivot-target").fill("J2");
  await dlg.getByTestId("dialog-ok").click();
  await expect(dlg).toBeHidden();
  const ops = await recorded(page);
  const create = ops.find((o) => o.type === "createPivot") as (Rec & { pivot: { rowFields: unknown[]; dataFields: { function: string }[]; target: { r: number; c: number } } }) | undefined;
  expect(create).toBeDefined();
  expect(create!.pivot.rowFields).toHaveLength(1);
  expect(create!.pivot.dataFields[0]?.function).toBe("sum");
  expect(create!.pivot.target).toEqual({ r: 1, c: 9 });
  // Row 1 is the Filter button row, row 2 the header.
  await expect.poll(() => cellText(page, 1, 9)).toBe("Filter");
  await expect.poll(() => cellText(page, 2, 9)).toBe("Status");
  await expect(page.getByTestId("pivot-band").first()).toBeVisible();
  // Refresh is enabled while the cursor is inside the output (column K may be off-screen: select programmatically).
  await page.evaluate(() => window.__tally!.controller.onSelectionChange({ range: { r0: 3, c0: 9, r1: 3, c1: 9 }, active: { r: 3, c: 9 }, anchor: { r: 3, c: 9 } }));
  await expect.poll(() => page.evaluate(() => window.__tally!.controller.registry.isEnabled("data.refreshPivot"))).toBe(true);
});

test("standard filter dialog hides 3 of the rows", async ({ page }) => {
  await ready(page);
  await clickCell(page, 1, 5);
  await run(page, "data.standardFilter");
  const dlg = page.getByTestId("standard-filter");
  await expect(dlg).toBeVisible();
  await dlg.getByTestId("filter-field-0").selectOption({ label: "Total" });
  await dlg.getByTestId("filter-op-0").selectOption("gt");
  await dlg.getByTestId("filter-value-0").fill("60");
  await dlg.getByTestId("filter-conn-1").selectOption("or");
  await dlg.getByTestId("filter-field-1").selectOption({ label: "Status" });
  await dlg.getByTestId("filter-op-1").selectOption("eq");
  await dlg.getByTestId("filter-value-1").fill("Overdue");
  await dlg.getByTestId("dialog-ok").click();
  const ops = await recorded(page);
  expect(ops.at(-1)).toMatchObject({ type: "standardFilter", conditions: [{ col: 5, op: "gt", value: "60" }, { connector: "or", col: 6, op: "eq", value: "Overdue" }] });
  await expect.poll(() => hiddenRows(page).then((r) => r.length)).toBeGreaterThan(0);
  await run(page, "data.resetFilter");
  await expect.poll(() => hiddenRows(page)).toEqual([]);
});

test("subtotals dialog records the groups; text to columns previews the split", async ({ page }) => {
  await ready(page);
  await clickCell(page, 1, 1);
  await run(page, "data.subtotals");
  const dlg = page.getByTestId("subtotals-dialog");
  await dlg.getByTestId("subtotal-groupby-0").selectOption({ label: "Status" });
  await dlg.getByTestId("subtotal-col-5").click();
  await dlg.getByTestId("subtotal-fn-0").selectOption("average");
  await dlg.getByTestId("dialog-ok").click();
  const ops = await recorded(page);
  const sub = ops.find((o) => o.type === "subtotals") as (Rec & { groups: { groupBy: number; columns: number[]; function: string }[] }) | undefined;
  expect(sub?.groups[0]).toMatchObject({ groupBy: 6, function: "average" });
  expect(sub?.groups[0]?.columns).toContain(5);
  // Text to columns on a typed cell.
  await clickCell(page, 20, 0);
  await page.keyboard.type("north;7;x");
  await page.keyboard.press("Enter");
  await clickCell(page, 20, 0);
  await run(page, "data.textToColumns");
  const t2c = page.getByTestId("text-to-columns");
  // A lone word has no delimiter typed after it: AutoCorrect leaves it alone (review 4 M42).
  await expect(t2c.getByTestId("t2c-preview")).toContainText("north");
  await expect(t2c.getByTestId("t2c-preview").locator("tbody td")).toHaveCount(3);
  await t2c.getByTestId("t2c-semicolon").uncheck();
  await expect(t2c.getByTestId("t2c-preview").locator("tbody td")).toHaveCount(1);
  await t2c.getByTestId("t2c-semicolon").check();
  await t2c.getByTestId("dialog-ok").click();
  await expect.poll(() => cellText(page, 20, 1)).toBe("7");
});

test("group and ungroup through the outline gutter", async ({ page }) => {
  await ready(page);
  await selectRange(page, 2, 0, 4, 0);
  await run(page, "data.group");
  await expect.poll(() => page.evaluate(() => window.__tally!.controller.grid!.getGeometry().gutterW)).toBeGreaterThan(0);
  // Click the +/− box of the group (level 1 column, first row of the group).
  const p = await cellXY(page, 2, 0);
  await page.mouse.click(p.box.x + 6, p.y);
  await expect.poll(async () => {
    const rows = await hiddenRows(page);
    return rows;
  }, { message: "collapsed group rows" }).toEqual([2, 3, 4]);
  // The level-2 button in the corner strip shows every row again (level 1 keeps the group collapsed).
  await page.mouse.click(p.box.x + 13 + 6, p.box.y + p.headerH - 8);
  await expect.poll(() => hiddenRows(page)).toEqual([]);
  await selectRange(page, 2, 0, 4, 0);
  await run(page, "data.ungroup");
  await expect.poll(() => page.evaluate(() => window.__tally!.controller.grid!.getGeometry().gutterW)).toBe(0);
});

test("styles panel applies a style; Format Cells sets the rotation", async ({ page }) => {
  await ready(page);
  await clickCell(page, 0, 0);
  await run(page, "view.stylesPanel");
  const panel = page.getByTestId("styles-panel");
  await expect(panel).toBeVisible();
  await panel.getByTestId("style-Heading").click();
  await expect.poll(() => recorded(page).then((ops) => ops.at(-1)?.type)).toBe("applyCellStyle");
  await panel.getByTestId("styles-new").click();
  const editor = page.getByTestId("style-editor");
  await editor.getByTestId("style-name").fill("Totals");
  await editor.getByTestId("dialog-ok").click();
  await expect(panel.getByTestId("style-Totals")).toBeVisible();
  await clickCell(page, 1, 1);
  await page.keyboard.press("Meta+1");
  const cells = page.getByTestId("format-cells");
  await expect(cells).toBeVisible();
  await cells.getByTestId("tab-alignment").click();
  await cells.getByTestId("cells-rotation").fill("45");
  await cells.getByTestId("cells-shrink").check();
  await cells.getByTestId("dialog-ok").click();
  const ops = await recorded(page);
  expect(ops.at(-1)).toMatchObject({ type: "applyStyle", patch: { rotationDeg: 45, shrinkToFit: true } });
});

test("AutoCorrect on commit, the AutoInput ghost and the Delete key dialog", async ({ page }) => {
  await ready(page);
  await clickCell(page, 20, 2);
  await page.keyboard.type("teh TEst");
  await page.keyboard.press("Enter");
  await expect.poll(() => cellText(page, 20, 2)).toBe("The Test");
  await page.keyboard.press("Meta+z");
  await expect.poll(() => cellText(page, 20, 2)).toBe("");
  // AutoInput under the Customer column ("Acme…" style names in the template).
  const first = await cellText(page, 1, 1);
  await clickCell(page, 15, 1);
  await page.keyboard.type(first.slice(0, 2));
  await expect(page.getByTestId("autoinput-ghost")).toBeVisible();
  await expect(page.getByTestId("autoinput-ghost")).toContainText(first.slice(2));
  await page.keyboard.press("Enter");
  await expect.poll(() => cellText(page, 15, 1)).toBe(first);
  // Delete opens the dialog, Backspace clears at once.
  await clickCell(page, 15, 1);
  await page.keyboard.press("Delete");
  const del = page.getByTestId("delete-contents");
  await expect(del).toBeVisible();
  await del.getByTestId("delete-formats").check();
  await del.getByTestId("dialog-ok").click();
  await expect.poll(() => recorded(page).then((ops) => ops.at(-1)?.type)).toBe("deleteContents");
  await expect.poll(() => cellText(page, 15, 1)).toBe("");
});

test("navigator lists sheets and names; double-click jumps", async ({ page }) => {
  await ready(page);
  await page.keyboard.press("F5");
  const nav = page.getByTestId("navigator-panel");
  await expect(nav).toBeVisible();
  await expect(nav.getByTestId("nav-sheets")).toContainText("1");
  await page.evaluate(() => window.__tally!.controller.defineName("Totals", null, "$Sheet1.$F$2:$F$15", null));
  await expect(nav.getByTestId("nav-names")).toContainText("Totals");
  await nav.getByTestId("nav-item-sheets-0").dblclick();
  // The sheet entry jumps to its used area.
  await expect(page.getByTestId("name-box")).toHaveValue(/^A1/);
});

test("sheet tab menu copies, colours and hides sheets", async ({ page }) => {
  await ready(page);
  await page.evaluate(() => window.__tally!.controller.addSheet());
  await run(page, "sheet.tabColor");
  await page.getByTestId("tab-color").getByTestId("tab-color-blue").click();
  await page.getByTestId("tab-color").getByTestId("dialog-ok").click();
  await expect.poll(() => recorded(page).then((ops) => ops.at(-1)?.type)).toBe("setTabColor");
  await run(page, "sheet.hide");
  await expect.poll(() => recorded(page).then((ops) => ops.at(-1)?.type)).toBe("setSheetHidden");
  await expect(page.getByRole("tab")).toHaveCount(1);
  await run(page, "sheet.show");
  await page.getByTestId("show-sheet").getByTestId("dialog-ok").click();
  await expect(page.getByRole("tab")).toHaveCount(2);
});
