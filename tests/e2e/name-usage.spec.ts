/**
 * Name range options (AOO Define Names "More >>": Print range, Filter,
 * Repeat column, Repeat row) and the dialogs that offer flagged names:
 * Advanced Filter's criteria list and Page Setup's print range / repeat
 * lists (AOO Format ▸ Print Ranges ▸ Edit).
 */
import { expect, test, type Page } from "@playwright/test";

interface Rec {
  type: string;
  [k: string]: unknown;
}

async function ready(page: Page, url = "/?template=invoices") {
  await page.goto(url);
  await expect(page.getByTestId("grid")).toBeVisible();
  await page.waitForFunction(() => window.__tally?.controller.grid != null);
}

const run = (page: Page, id: string) => page.evaluate((id) => window.__tally!.controller.registry.run(id), id);

const recorded = (page: Page): Promise<Rec[]> =>
  page.evaluate(() => JSON.parse(JSON.stringify((window.__tally!.client as unknown as { recorded: unknown[] }).recorded)) as Rec[]);

const select = (page: Page, r0: number, c0: number) =>
  page.evaluate(([r, c]) => window.__tally!.controller.onSelectionChange({ range: { r0: r, c0: c, r1: r, c1: c }, active: { r, c }, anchor: { r, c } }), [r0, c0] as const);

async function defineName(page: Page, name: string, refersTo: string, flags: string[]) {
  await run(page, "data.defineName");
  const dialog = page.getByTestId("define-name-dialog");
  await expect(dialog).toBeVisible();
  // The options are off until the name has text (AOO ScNameDlg::UpdateChecks).
  await expect(dialog.getByTestId("name-use-print")).toBeDisabled();
  await dialog.getByTestId("name-name").fill(name);
  await dialog.getByTestId("name-refers").fill(refersTo);
  await expect(dialog.getByTestId("name-use-print")).toBeEnabled();
  for (const f of flags) await dialog.getByTestId(f).check();
  await dialog.getByTestId("dialog-ok").click();
  await expect(dialog).toBeHidden();
}

test("define name range options are saved and offered by Advanced Filter and Page Setup", async ({ page }) => {
  await ready(page);
  await defineName(page, "Crit", "$Orders.$J$1:$J$2", ["name-use-filter"]);
  await defineName(page, "Titles", "$Orders.$A$1:$G$1", ["name-use-print", "name-use-repeat-row"]);
  await defineName(page, "Plain", "$Orders.$A$1:$A$3", []);
  const ops = await recorded(page);
  const usage = ops.filter((o) => o.type === "setNameUsage");
  expect(usage).toEqual([
    { type: "setNameUsage", name: "Crit", scope: null, usage: { printRange: false, filter: true, repeatRow: false, repeatColumn: false } },
    { type: "setNameUsage", name: "Titles", scope: null, usage: { printRange: true, filter: false, repeatRow: true, repeatColumn: false } },
  ]);

  // Editing shows the stored options.
  await run(page, "data.manageNames");
  const manage = page.getByTestId("manage-names-dialog");
  await manage.locator('[data-name="Crit"]').getByRole("button", { name: "Edit" }).click();
  const edit = page.getByTestId("define-name-dialog");
  await expect(edit.getByTestId("name-use-filter")).toBeChecked();
  await expect(edit.getByTestId("name-use-print")).not.toBeChecked();
  await page.keyboard.press("Escape");
  await expect(edit).toBeHidden();

  // Advanced Filter lists only the "filter" name; picking it fills the field.
  await select(page, 1, 5);
  await run(page, "data.advancedFilter");
  const filter = page.getByTestId("standard-filter");
  await expect(filter).toBeVisible();
  const crit = filter.getByTestId("filter-criteria-names");
  await expect(crit.locator("option")).toHaveText(["— none —", "Crit"]);
  await crit.selectOption({ label: "Crit" });
  await expect(filter.getByTestId("filter-criteria")).toHaveValue("$Orders.$J$1:$J$2");
  await filter.getByTestId("filter-criteria").fill("J1:J3");
  await expect(crit).toHaveValue(" user");
  await page.keyboard.press("Escape");
  await expect(filter).toBeHidden();

  // Page Setup ▸ Sheet lists the print-range and repeat-row names.
  await run(page, "file.pageSetup");
  const setup = page.getByTestId("page-setup-dialog");
  await expect(setup).toBeVisible();
  await setup.getByTestId("tab-sheet").click();
  await setup.getByTestId("page-print-range-names").selectOption({ label: "Titles" });
  await expect(setup.getByTestId("page-print-range")).toHaveValue("$A$1:$G$1");
  await setup.getByTestId("page-repeat-rows-names").selectOption({ label: "Titles" });
  await expect(setup.getByTestId("page-repeat-rows")).toHaveValue("$1");
  // No name is marked "repeat column": that list is absent.
  await expect(setup.getByTestId("page-repeat-cols-names")).toHaveCount(0);
  await setup.getByTestId("dialog-ok").click();
  expect((await recorded(page)).at(-1)).toMatchObject({ type: "setPageSetup", setup: { printRange: { r0: 0, c0: 0, r1: 0, c1: 6 }, repeatRows: [0, 0] } });
});

test("range options are reachable by keyboard", async ({ page }) => {
  await ready(page);
  await run(page, "data.defineName");
  const dialog = page.getByTestId("define-name-dialog");
  await dialog.getByTestId("name-name").fill("Area");
  await dialog.getByTestId("name-use-print").focus();
  await page.keyboard.press("Space");
  await expect(dialog.getByTestId("name-use-print")).toBeChecked();
  await expect(dialog.getByRole("group", { name: "Range options" })).toBeVisible();
});
