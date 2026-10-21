/**
 * v0.2 data features in the browser (WebMockClient emulates the engine
 * ops and records them in `client.recorded`): Sort…, AutoFilter popover,
 * merge warning, Define Name + use, find bar, Paste Special, array
 * formulas (Ctrl+Shift+Enter), hide rows, cut → move.
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

/** Client coordinates of a cell's centre (reads the column widths from the layout). */
async function cellXY(page: Page, r: number, c: number) {
  const box = await page.getByTestId("grid").boundingBox();
  if (!box) throw new Error("grid not visible");
  const { headerW, xs, w } = await page.evaluate((c) => {
    const ctl = window.__tally!.controller;
    let x = 0;
    for (let i = 0; i < c; i++) x += ctl.layoutSize("col", i);
    return { headerW: (ctl.grid as unknown as { headerW: number }).headerW, xs: x, w: ctl.layoutSize("col", c) };
  }, c);
  return { x: box.x + headerW + xs + w / 2, y: box.y + HEADER_H + r * ROW_H + ROW_H / 2, w, left: box.x + headerW + xs };
}

async function clickCell(page: Page, r: number, c: number, modifier?: "Shift") {
  const p = await cellXY(page, r, c);
  if (modifier) await page.keyboard.down(modifier);
  await page.mouse.click(p.x, p.y);
  if (modifier) await page.keyboard.up(modifier);
}

const recorded = (page: Page): Promise<Rec[]> =>
  page.evaluate(() => JSON.parse(JSON.stringify((window.__tally!.client as unknown as { recorded: unknown[] }).recorded)) as Rec[]);

async function texts(page: Page, r0: number, c0: number, r1: number, c1: number): Promise<string[]> {
  return page.evaluate(
    async ([r0, c0, r1, c1]) => {
      const t = window.__tally!;
      const info = t.controller.info!;
      const [block] = await t.client.cells(info.doc, t.controller.sheet, [{ r0, c0, r1, c1 }]);
      const out: string[] = [];
      for (let r = r0; r <= r1; r++) for (let c = c0; c <= c1; c++) out.push(block?.cells.find((x) => x.r === r && x.c === c)?.text ?? "");
      return out;
    },
    [r0, c0, r1, c1] as const,
  );
}

test("Sort… opens with the header detected and sorts by the chosen key", async ({ page }) => {
  await ready(page);
  await clickCell(page, 2, 5); // F3 (Total)
  await page.locator('[data-command="data.sort"]').click();
  await page.getByTestId("sort-menu").locator('[data-item="custom"]').click();
  const dialog = page.getByTestId("sort-dialog");
  await expect(dialog).toBeVisible();
  await expect(dialog.getByTestId("sort-header")).toBeChecked();
  await expect(dialog.getByTestId("sort-key-0").locator("option:checked")).toHaveText("Total");
  await dialog.getByRole("radio", { name: "Z → A" }).first().click();
  await dialog.getByTestId("dialog-ok").click();
  await expect(dialog).toHaveCount(0);
  const ops = await recorded(page);
  expect(ops.at(-1)).toMatchObject({ type: "sortRange", range: { r0: 0, c0: 0, r1: 14, c1: 6 }, keys: [{ col: 5, ascending: false }], hasHeader: true });
  // The mock does not re-anchor formulas, so check a value column: the largest total is the coffee order.
  await expect.poll(() => texts(page, 0, 2, 1, 2)).toEqual(["Item", "Coffee beans"]);
});

test("AutoFilter popover lists the column's values and filters", async ({ page }) => {
  await ready(page);
  await clickCell(page, 3, 1);
  await page.locator('[data-command="data.filter"]').click();
  await expect(page.locator('[data-command="data.filter"]')).toHaveAttribute("aria-pressed", "true");
  // The ▾ at the right of the Status header cell (G1).
  const g1 = await cellXY(page, 0, 6);
  await page.mouse.click(g1.left + g1.w - 11, g1.y);
  const pop = page.getByTestId("filter-popover");
  await expect(pop).toBeVisible();
  const values = pop.getByTestId("filter-values");
  await expect(values.locator("[data-value]")).toHaveText([/Due\s*4/, /Overdue\s*2/, /Paid\s*8/]);
  await pop.getByTestId("filter-search").fill("pa");
  await expect(values.locator("[data-value]")).toHaveCount(1);
  await pop.getByTestId("filter-search").fill("");
  await values.locator('[data-value="Due"] input').click();
  await values.locator('[data-value="Overdue"] input').click();
  await pop.getByTestId("filter-apply").click();
  await expect(pop).toHaveCount(0);
  const ops = await recorded(page);
  expect(ops.at(-1)).toEqual({ type: "setColumnFilter", sheet: 1, col: 6, filter: { values: ["Paid"], condition: null } });
  // Six rows are hidden: the grid now shows row 4 (Harbour Deli, Paid) right below row 2.
  const hidden = await page.evaluate(async () => {
    const t = window.__tally!;
    const l = await t.client.sheetLayout(t.controller.info!.doc, t.controller.sheet);
    return l.rowRuns.filter((r) => r.hidden).reduce((n, r) => n + r.end - r.start + 1, 0);
  });
  expect(hidden).toBe(6);
});

test("merging warns that only the top-left value is kept", async ({ page }) => {
  await ready(page);
  await clickCell(page, 0, 0);
  await clickCell(page, 0, 1, "Shift");
  await page.locator('[data-command="format.toggleMerge"]').click();
  // W5-6 (§51): AOO's prompt — "No" keeps only the first cell's content.
  const dialog = page.getByTestId("merge-prompt");
  await expect(dialog).toBeVisible();
  await expect(dialog).toContainText("moved into the first cell");
  await expect(dialog).toContainText("1 cell holds content");
  await dialog.getByTestId("dialog-cancel").click();
  await expect
    .poll(() => page.evaluate(async () => (await window.__tally!.client.sheetLayout(window.__tally!.controller.info!.doc, 1)).merges))
    .toEqual([{ r0: 0, c0: 0, r1: 0, c1: 1 }]);
  await expect.poll(() => texts(page, 0, 0, 0, 1)).toEqual(["Date", ""]);
  // Clicking the covered cell selects the whole merged area; the toggle now unmerges.
  await clickCell(page, 0, 1);
  await expect(page.getByTestId("name-box")).toHaveValue("A1:B1");
  await expect(page.locator('[data-command="format.toggleMerge"]')).toHaveAttribute("aria-pressed", "true");
});

test("Define Name from the menu, then use it in a formula", async ({ page }) => {
  await ready(page);
  const nameBox = page.getByTestId("name-box");
  await nameBox.click();
  await nameBox.fill("F2:F15");
  await nameBox.press("Enter");
  await expect(nameBox).toHaveValue("F2:F15");
  await page.evaluate(() => (window.__tally!.client as unknown as { emitMenuAction(id: string): void }).emitMenuAction("data.defineName"));
  const dialog = page.getByTestId("define-name-dialog");
  await expect(dialog.getByTestId("name-refers")).toHaveValue("$Orders.$F$2:$F$15");
  await dialog.getByTestId("name-name").fill("A1");
  await expect(dialog.getByTestId("dialog-ok")).toBeDisabled();
  await dialog.getByTestId("name-name").fill("Totals");
  await dialog.getByTestId("dialog-ok").click();
  // The selection now shows its name, and the name box lists it.
  await expect(nameBox).toHaveValue("Totals");
  await page.getByTestId("name-box-menu").click();
  await expect(page.getByTestId("name-box-list")).toContainText("Totals");
  await page.keyboard.press("Escape");
  await clickCell(page, 1, 7);
  await page.keyboard.type("=SUM(Totals)");
  await page.keyboard.press("Enter");
  const ops = await recorded(page);
  expect(ops).toContainEqual({ type: "defineName", name: "Totals", scope: null, refersTo: "$Orders.$F$2:$F$15" });
  expect(ops.at(-1)).toEqual({ type: "setCellInput", sheet: 1, r: 1, c: 7, input: "=SUM(Totals)" });
});

test("find bar: next / previous through the matches", async ({ page }) => {
  await ready(page);
  await clickCell(page, 0, 0);
  await page.keyboard.press("ControlOrMeta+f");
  const bar = page.getByTestId("find-bar");
  await expect(bar).toBeVisible();
  await page.getByTestId("find-input").fill("Paid");
  await page.getByTestId("find-input").press("Enter");
  await expect(page.getByTestId("find-status")).toHaveText("1 of 8");
  await expect(page.getByTestId("name-box")).toHaveValue("G2");
  await page.getByTestId("find-input").press("Enter");
  await expect(page.getByTestId("find-status")).toHaveText("2 of 8");
  await expect(page.getByTestId("name-box")).toHaveValue("G4");
  await page.getByTestId("find-prev").click();
  await expect(page.getByTestId("name-box")).toHaveValue("G2");
  await page.getByTestId("find-prev").click();
  await expect(page.getByTestId("find-status")).toHaveText("8 of 8");
  await expect(page.getByTestId("name-box")).toHaveValue("G15");
  await page.getByTestId("find-input").press("Escape");
  await expect(bar).toHaveCount(0);
});

test("Paste Special: values, transposed", async ({ page }) => {
  await ready(page);
  await clickCell(page, 1, 3);
  await clickCell(page, 2, 4, "Shift"); // D2:E3
  await page.keyboard.press("ControlOrMeta+c");
  await clickCell(page, 19, 0); // A20
  await page.keyboard.press("ControlOrMeta+Alt+v");
  const dialog = page.getByTestId("paste-special-dialog");
  await expect(dialog).toBeVisible();
  await dialog.getByTestId("paste-values").click();
  await dialog.getByTestId("paste-transpose").check();
  await dialog.getByTestId("dialog-ok").click();
  await expect.poll(async () => (await recorded(page)).at(-1)?.type).toBe("pasteSpecial");
  const ops = await recorded(page);
  expect(ops.at(-1)).toMatchObject({
    type: "pasteSpecial",
    target: { r0: 19, c0: 0 },
    payload: { type: "internal" },
    options: { valuesOnly: true, formatsOnly: false, transpose: true, skipEmpty: false, operation: "none" },
  });
  await expect.poll(() => texts(page, 19, 0, 20, 1)).toEqual(["12", "30", "4.5", "2.25"]);
});

test("Ctrl+Shift+Enter sends SetArrayFormula over the selection", async ({ page }) => {
  await ready(page);
  await clickCell(page, 19, 0);
  await clickCell(page, 20, 1, "Shift"); // A20:B21
  await page.keyboard.type("=D2:E3*2");
  await page.keyboard.press("Control+Shift+Enter");
  const ops = await recorded(page);
  expect(ops.at(-1)).toEqual({ type: "setArrayFormula", sheet: 1, range: { r0: 19, c0: 0, r1: 20, c1: 1 }, input: "=D2:E3*2" });
  // Inside the block: `{=…}` and the hint; a partial edit is refused with a toast.
  await clickCell(page, 20, 1);
  await expect(page.getByTestId("formula-input")).toHaveValue("{=D2:E3*2}");
  await expect(page.getByTestId("formula-hint")).toContainText("Array formula A20:B21");
  await page.keyboard.type("5");
  await page.keyboard.press("Enter");
  await expect(page.getByRole("alert").first()).toContainText(/part of an array/);
});

test("row header menu hides and shows rows", async ({ page }) => {
  await ready(page);
  const box = await page.getByTestId("grid").boundingBox();
  if (!box) throw new Error("no grid");
  const headerW = await page.evaluate(() => (window.__tally!.controller.grid as unknown as { headerW: number }).headerW);
  await page.mouse.click(box.x + headerW / 2, box.y + HEADER_H + 2 * ROW_H + ROW_H / 2, { button: "right" });
  await page.getByTestId("context-menu").locator('[data-item="hide"]').click();
  expect((await recorded(page)).at(-1)).toEqual({ type: "setRowsHidden", sheet: 1, start: 2, end: 2, hidden: true });
  // Row 4's header now sits where row 3's was; select rows 2–4 and show.
  await page.mouse.click(box.x + headerW / 2, box.y + HEADER_H + ROW_H + ROW_H / 2);
  await page.keyboard.down("Shift");
  await page.mouse.click(box.x + headerW / 2, box.y + HEADER_H + 2 * ROW_H + ROW_H / 2);
  await page.keyboard.up("Shift");
  await page.mouse.click(box.x + headerW / 2, box.y + HEADER_H + 2 * ROW_H + ROW_H / 2, { button: "right" });
  await page.getByTestId("context-menu").locator('[data-item="show"]').click();
  expect((await recorded(page)).at(-1)).toMatchObject({ type: "setRowsHidden", start: 1, end: 3, hidden: false });
});

test("cut then paste moves the range (moveRange)", async ({ page }) => {
  await ready(page);
  await clickCell(page, 1, 1);
  await page.keyboard.press("ControlOrMeta+x");
  // Paste from the cell menu (the headless system clipboard is not reliable across tests).
  const to = await cellXY(page, 18, 1);
  await page.mouse.click(to.x, to.y, { button: "right" });
  await page.getByTestId("context-menu").locator('[data-item="paste"]').click();
  await expect.poll(async () => (await recorded(page)).at(-1)).toEqual({ type: "moveRange", sheet: 1, from: { r0: 1, c0: 1, r1: 1, c1: 1 }, to: { r: 18, c: 1 } });
  await expect.poll(() => texts(page, 1, 1, 1, 1)).toEqual([""]);
  await expect.poll(() => texts(page, 18, 1, 18, 1)).toEqual(["Maple & Co. Bakery"]);
});
