/**
 * AOO 4.1 parity items: Validity "Sort entries ascending" (G16), Data ▸
 * Select Range / Refresh Range and Edit ▸ Fill ▸ Up / Left, against the
 * browser demo engine.
 */
import { expect, test, type Page } from "@playwright/test";

const ROW_H = 24;
const HEADER_H = 24;

interface Rec {
  type: string;
  [k: string]: unknown;
}

async function ready(page: Page, url = "/") {
  await page.goto(url);
  await expect(page.getByTestId("grid")).toBeVisible();
  await page.waitForFunction(() => window.__tally?.controller.grid != null);
}

async function cellXY(page: Page, r: number, c: number) {
  const box = await page.getByTestId("grid").boundingBox();
  if (!box) throw new Error("grid not visible");
  const { headerW, xs, w } = await page.evaluate((c) => {
    const ctl = window.__tally!.controller;
    let x = 0;
    for (let i = 0; i < c; i++) x += ctl.layoutSize("col", i);
    return { headerW: (ctl.grid as unknown as { headerW: number }).headerW, xs: x, w: ctl.layoutSize("col", c) };
  }, c);
  return { x: box.x + headerW + xs + w / 2, y: box.y + HEADER_H + r * ROW_H + ROW_H / 2, right: box.x + headerW + xs + w };
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

async function typeInto(page: Page, r: number, c: number, text: string) {
  await clickCell(page, r, c);
  await page.keyboard.type(text);
  await page.keyboard.press("Enter");
}

const recorded = (page: Page): Promise<Rec[]> =>
  page.evaluate(() => JSON.parse(JSON.stringify((window.__tally!.client as unknown as { recorded: unknown[] }).recorded)) as Rec[]);

const run = (page: Page, id: string) => page.evaluate((id) => window.__tally!.controller.registry.run(id), id);

/** Texts of a range on the active sheet (row-major, "" for empty). */
async function texts(page: Page, r0: number, c0: number, r1: number, c1: number): Promise<string[]> {
  return page.evaluate(
    async ([r0, c0, r1, c1]) => {
      const t = window.__tally!;
      const [block] = await t.client.cells(t.controller.info!.doc, t.controller.sheet, [{ r0, c0, r1, c1 }]);
      const out: string[] = [];
      for (let r = r0; r <= r1; r++) for (let c = c0; c <= c1; c++) out.push(block?.cells.find((x) => x.r === r && x.c === c)?.text ?? "");
      return out;
    },
    [r0, c0, r1, c1] as const,
  );
}

test("Validity: Sort entries ascending sorts the cell's dropdown", async ({ page }) => {
  await ready(page);
  await clickCell(page, 0, 2); // C1
  await run(page, "data.validation");
  const dialog = page.getByTestId("validation-dialog");
  await dialog.getByTestId("validation-items").fill("pear\n10\nApple\n2\npear");
  const sort = dialog.getByTestId("validation-sort");
  await expect(sort).not.toBeChecked();
  await sort.check();
  await dialog.getByTestId("dialog-ok").click();
  expect((await recorded(page)).at(-1)).toMatchObject({ type: "addValidation", validation: { showDropdown: true, sortAscending: true } });
  const c1 = await cellXY(page, 0, 2);
  await page.mouse.click(c1.x, c1.y);
  await page.mouse.click(c1.right - 10, c1.y);
  const list = page.getByTestId("validation-list");
  await expect(list).toBeVisible();
  // Numbers first by value, then text in code order, each entry once.
  await expect(list.locator("[data-item]")).toHaveText([/2$/, /10$/, /Apple$/, /pear$/]);
});

test("Data ▸ Select Range selects a database range on another sheet", async ({ page }) => {
  await ready(page);
  const ids = await page.evaluate(async () => {
    const ctl = window.__tally!.controller;
    await ctl.apply("add", [{ type: "addSheet", name: "Orders", index: null }]);
    const sheets = ctl.info!.sheets.map((s) => s.id);
    await ctl.apply("define", [
      { type: "defineDatabaseRange", name: "Zeta", sheet: sheets[0]!, range: { r0: 0, c0: 0, r1: 2, c1: 1 }, hasHeader: true },
      { type: "defineDatabaseRange", name: "alpha", sheet: sheets[1]!, range: { r0: 3, c0: 2, r1: 9, c1: 4 }, hasHeader: true },
    ]);
    return sheets;
  });
  await clickCell(page, 0, 0);
  await run(page, "data.selectDatabaseRange");
  const dialog = page.getByTestId("select-db-range");
  await expect(dialog).toBeVisible();
  // The ranges by name, ignoring case (ScDBCollection order).
  await expect(dialog.getByRole("option")).toHaveText(["alpha", "Zeta"]);
  await dialog.getByTestId("select-db-alpha").click();
  await dialog.getByTestId("dialog-ok").click();
  await expect(dialog).toBeHidden();
  await expect.poll(() => page.evaluate(() => window.__tally!.controller.sheet)).toBe(ids[1]);
  await expect.poll(() => page.evaluate(() => window.__tally!.controller.selection.range)).toEqual({ r0: 3, c0: 2, r1: 9, c1: 4 });
});

test("Data ▸ Refresh Range sorts the range again with its kept sort", async ({ page }) => {
  await ready(page);
  await typeInto(page, 0, 0, "Name");
  await typeInto(page, 1, 0, "Cid");
  await typeInto(page, 2, 0, "Ann");
  await typeInto(page, 3, 0, "Bob");
  await page.evaluate(async () => {
    const ctl = window.__tally!.controller;
    await ctl.apply("define", [{ type: "defineDatabaseRange", name: "Names", sheet: ctl.sheet, range: { r0: 0, c0: 0, r1: 3, c1: 0 }, hasHeader: true }]);
    await ctl.apply("sort", [{ type: "sortRange", sheet: ctl.sheet, range: { r0: 0, c0: 0, r1: 3, c1: 0 }, keys: [{ col: 0, ascending: true }], hasHeader: true, caseSensitive: false, options: null }]);
  });
  await expect.poll(() => texts(page, 1, 0, 3, 0)).toEqual(["Ann", "Bob", "Cid"]);
  await typeInto(page, 1, 0, "Zed");
  await clickCell(page, 2, 0);
  await run(page, "data.refreshRange");
  expect((await recorded(page)).some((o) => o.type === "refreshDatabaseRange" && o.name === "Names")).toBe(true);
  await expect.poll(() => texts(page, 1, 0, 3, 0)).toEqual(["Bob", "Cid", "Zed"]);
  // Outside any database range there is nothing to refresh.
  await clickCell(page, 8, 5);
  await run(page, "data.refreshRange");
  await expect(page.getByText("The cursor is not in a database range.")).toBeVisible();
});

test("Edit ▸ Fill ▸ Up and Left copy the last row / column", async ({ page }) => {
  await ready(page);
  await typeInto(page, 1, 2, "1");
  await typeInto(page, 2, 2, "2");
  await typeInto(page, 3, 2, "3");
  await typeInto(page, 3, 0, "5");
  await typeInto(page, 3, 1, "=C4+A4");
  await selectRange(page, 1, 0, 3, 1); // A2:B4
  await run(page, "edit.fillUp");
  // One engine copy of the bottom row over the selection (FILL_TO_TOP); the engine moves the
  // relative references (B2 = C2 + A2, crates/tally-engine/tests/data_small.rs).
  expect((await recorded(page)).at(-1)).toMatchObject({ type: "fillRange", source: { r0: 3, c0: 0, r1: 3, c1: 1 }, target: { r0: 1, c0: 0, r1: 3, c1: 1 }, mode: "copy" });
  await expect.poll(() => texts(page, 1, 0, 3, 0)).toEqual(["5", "5", "5"]);
  await typeInto(page, 0, 6, "x");
  await selectRange(page, 0, 3, 0, 6); // D1:G1
  await run(page, "edit.fillLeft");
  expect((await recorded(page)).at(-1)).toMatchObject({ type: "fillRange", source: { r0: 0, c0: 6, r1: 0, c1: 6 }, target: { r0: 0, c0: 3, r1: 0, c1: 6 }, mode: "copy" });
  await expect.poll(() => texts(page, 0, 3, 0, 6)).toEqual(["x", "x", "x", "x"]);
  // One row: Fill Up does nothing (AOO: nothing to fill).
  const before = (await recorded(page)).length;
  await clickCell(page, 5, 5);
  await run(page, "edit.fillUp");
  expect((await recorded(page)).length).toBe(before);
});
