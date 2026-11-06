/**
 * AutoFormat Add / Rename / Delete (MockClient): format a 4 × 4 range, Add it
 * as "Mine" from the selection, apply it to another range and check the
 * styles, rename it, restart the page (the format survives in the prefs) and
 * delete it. The built-in formats cannot be renamed or deleted.
 */
import { expect, test, type Page } from "@playwright/test";

const RED = 0xff0000ff;
const BLUE = 0x0000ffff;
const YELLOW = 0xffff00ff;

async function ready(page: Page) {
  await page.goto("/?template=invoices");
  await expect(page.getByTestId("grid")).toBeVisible();
  await page.waitForFunction(() => window.__tally?.controller.grid != null);
}

const col = (c: number): string => String.fromCharCode(65 + c);

async function select(page: Page, r0: number, c0: number, r1: number, c1: number) {
  const ref = `${col(c0)}${r0 + 1}:${col(c1)}${r1 + 1}`;
  await page.evaluate((ref) => window.__tally!.controller.goTo(ref), ref);
  await expect.poll(() => page.evaluate(() => JSON.stringify(window.__tally!.controller.selection.range))).toBe(JSON.stringify({ r0, c0, r1, c1 }));
}

/** Formats A:D x 0..3 (relative to the origin): red bold header row, blue last column, yellow body. */
async function formatSource(page: Page, r0: number, c0: number) {
  await page.evaluate(
    async ([r0, c0, RED, BLUE, YELLOW]) => {
      const t = window.__tally!;
      const sheet = t.controller.sheet;
      const patch = (over: object) => ({ bold: null, italic: null, underline: null, strike: null, fontFamily: null, fontSizePt: null, color: null, fill: null, hAlign: null, vAlign: null, wrap: null, numberFormat: null, ...over });
      const rg = (a: number, b: number, c: number, d: number) => [{ r0: r0! + a, c0: c0! + b, r1: r0! + c, c1: c0! + d }];
      await t.controller.apply("fmt", [
        { type: "applyStyle", sheet, ranges: rg(0, 0, 3, 3), patch: patch({ fill: YELLOW, numberFormat: "0.00" }) },
        { type: "applyStyle", sheet, ranges: rg(0, 0, 0, 3), patch: patch({ fill: RED, bold: true, hAlign: "center" }) },
        { type: "applyStyle", sheet, ranges: rg(1, 3, 3, 3), patch: patch({ fill: BLUE, italic: true }) },
      ] as never);
    },
    [r0, c0, RED, BLUE, YELLOW] as const,
  );
}

interface Probe {
  fill: number | null;
  bold: boolean;
  italic: boolean;
  hAlign: string;
  numberFormat: string;
}

async function probe(page: Page, r: number, c: number): Promise<Probe> {
  return page.evaluate(
    async ([r, c]) => {
      const t = window.__tally!;
      const doc = t.controller.info!.doc;
      const [block] = await t.client.cells(doc, t.controller.sheet, [{ r0: r!, c0: c!, r1: r!, c1: c! }]);
      const [s] = await t.client.styles(doc, [block?.cells[0]?.style ?? 0]);
      return { fill: s!.fill, bold: s!.bold, italic: s!.italic, hAlign: s!.hAlign, numberFormat: s!.numberFormat };
    },
    [r, c] as const,
  );
}

const open = (page: Page) => page.evaluate(() => window.__tally!.controller.registry.run("format.autoFormat"));

async function addMine(page: Page, name: string) {
  await page.getByTestId("autoformat-add").click();
  await page.getByTestId("autoformat-name").fill(name);
  await page.getByTestId("dialog-ok").click();
  await expect(page.getByTestId(`autoformat-user-${name.toLowerCase()}`)).toBeVisible();
}

test("Add from the selection, apply, rename, persist and delete", async ({ page }) => {
  await ready(page);
  await formatSource(page, 20, 0);
  await select(page, 20, 0, 23, 3);
  await open(page);
  const dialog = page.getByTestId("autoformat-dialog");
  await expect(dialog).toBeVisible();

  // The built-in formats cannot be renamed or deleted.
  await page.getByTestId("autoformat-default").click();
  await expect(page.getByTestId("autoformat-delete")).toBeDisabled();
  await expect(page.getByTestId("autoformat-rename")).toBeDisabled();

  await addMine(page, "Mine");
  // The new format is selected, and its preview shows the sampled formats.
  await expect(page.getByTestId("autoformat-user-mine")).toHaveAttribute("aria-selected", "true");
  const preview = page.getByTestId("autoformat-preview");
  await expect(preview.locator("tr").first().locator("td").nth(1)).toHaveCSS("background-color", "rgb(255, 0, 0)");
  await expect(preview.locator("tr").nth(2).locator("td").nth(1)).toHaveCSS("background-color", "rgb(255, 255, 0)");
  // A name already in use is refused.
  await page.getByTestId("autoformat-add").click();
  await page.getByTestId("autoformat-name").fill("mine");
  await expect(page.getByTestId("dialog-ok")).toBeDisabled();
  await page.getByTestId("dialog-cancel").click();
  await expect(page.getByTestId("autoformat-user-mine")).toBeVisible();
  await page.getByTestId("dialog-cancel").click();
  await expect(dialog).toBeHidden();

  // Apply it to an empty range elsewhere.
  await select(page, 30, 5, 34, 8);
  await open(page);
  await page.getByTestId("autoformat-user-mine").click();
  await page.getByTestId("autoformat-opt-autofit").uncheck();
  await page.getByTestId("dialog-ok").click();
  await expect(dialog).toBeHidden();
  expect(await probe(page, 30, 5)).toMatchObject({ fill: RED, bold: true, hAlign: "center" });
  expect(await probe(page, 30, 8)).toMatchObject({ fill: RED, bold: true });
  expect(await probe(page, 31, 5)).toMatchObject({ fill: YELLOW, bold: false, numberFormat: "0.00" });
  expect(await probe(page, 32, 6)).toMatchObject({ fill: YELLOW, bold: false });
  expect(await probe(page, 32, 8)).toMatchObject({ fill: BLUE, italic: true });
  expect(await probe(page, 34, 8)).toMatchObject({ fill: BLUE, italic: true });

  // The options decide what is applied: without Pattern and Font the fill and bold stay.
  await select(page, 40, 5, 43, 8);
  await open(page);
  await page.getByTestId("autoformat-user-mine").click();
  await page.getByTestId("autoformat-opt-pattern").uncheck();
  await page.getByTestId("autoformat-opt-font").uncheck();
  await page.getByTestId("autoformat-opt-autofit").uncheck();
  await page.getByTestId("dialog-ok").click();
  expect(await probe(page, 40, 5)).toMatchObject({ fill: null, bold: false, hAlign: "center" });
  expect(await probe(page, 41, 6)).toMatchObject({ fill: null, numberFormat: "0.00" });

  // Rename.
  await select(page, 30, 5, 34, 8);
  await open(page);
  await page.getByTestId("autoformat-user-mine").click();
  await page.getByTestId("autoformat-rename").click();
  await expect(page.getByTestId("autoformat-name")).toHaveValue("Mine");
  await page.getByTestId("autoformat-name").fill("Renamed");
  await page.getByTestId("dialog-ok").click();
  await expect(page.getByTestId("autoformat-user-renamed")).toBeVisible();
  await expect(page.getByTestId("autoformat-user-mine")).toHaveCount(0);
  await page.getByTestId("dialog-cancel").click();

  // It is stored with the prefs: a fresh page still has it.
  await ready(page);
  await select(page, 30, 5, 34, 8);
  await open(page);
  await expect(page.getByTestId("autoformat-user-renamed")).toBeVisible();

  // Delete (after a confirmation).
  await page.getByTestId("autoformat-user-renamed").click();
  await page.getByTestId("autoformat-delete").click();
  await expect(page.getByTestId("autoformat-name-dialog")).toContainText("Renamed");
  await page.getByTestId("dialog-cancel").click();
  await expect(page.getByTestId("autoformat-user-renamed")).toBeVisible();
  await page.getByTestId("autoformat-delete").click();
  await page.getByTestId("dialog-ok").click();
  await expect(page.getByTestId("autoformat-dialog")).toBeVisible();
  await expect(page.getByTestId("autoformat-user-renamed")).toHaveCount(0);
});

test("AutoFormat needs a selection of at least 3 x 3 cells (AOO)", async ({ page }) => {
  await ready(page);
  await select(page, 50, 0, 52, 1);
  await open(page);
  await expect(page.getByTestId("autoformat-dialog")).toHaveCount(0);
  await expect(page.getByText("Select at least 3 rows and 3 columns for an AutoFormat.")).toBeVisible();
  await select(page, 50, 0, 52, 2);
  await open(page);
  await expect(page.getByTestId("autoformat-add")).toBeEnabled();
});
