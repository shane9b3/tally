/**
 * Tools ▸ Options (W5-6C): the Calculate tab round-trips through
 * `setCalcSettings` (mock), the Input tab toggles AutoInput / AutoCorrect,
 * the Sort lists editor records `setCustomSortLists`, and the Load/Save tab
 * changes the autosave interval.
 */
import { expect, test, type Page } from "@playwright/test";

interface Rec {
  type: string;
  [k: string]: unknown;
}

async function ready(page: Page) {
  await page.goto("/?template=invoices");
  await expect(page.getByTestId("grid")).toBeVisible();
  await page.waitForFunction(() => window.__tally?.controller.grid != null);
}

const recorded = (page: Page): Promise<Rec[]> =>
  page.evaluate(() => JSON.parse(JSON.stringify((window.__tally!.client as unknown as { recorded: unknown[] }).recorded)) as Rec[]);

test("Calculate tab round-trips the calc settings", async ({ page }) => {
  await ready(page);
  await page.keyboard.press("Meta+,");
  const dlg = page.getByTestId("options-dialog");
  await expect(dlg).toBeVisible();
  await dlg.getByTestId("opt-iterations").check();
  await dlg.getByTestId("opt-steps").fill("50");
  await dlg.getByTestId("opt-min-change").fill("0.01");
  await dlg.getByTestId("opt-date-epoch").selectOption("d1904");
  await dlg.getByTestId("opt-case").check();
  await dlg.getByTestId("opt-precision").check();
  await dlg.getByTestId("opt-criteria").selectOption("regex");
  await dlg.getByTestId("dialog-ok").click();
  const ops = await recorded(page);
  expect(ops.at(-1)).toMatchObject({ type: "setCalcSettings", settings: { iterations: { steps: 50, minChange: 0.01 }, dateEpoch: "d1904", caseSensitive: true, precisionAsShown: true, criteriaMode: "regex" } });
  // Re-open: the mock reports the stored settings back.
  await page.keyboard.press("Meta+,");
  await expect(dlg.getByTestId("opt-steps")).toHaveValue("50");
  await expect(dlg.getByTestId("opt-date-epoch")).toHaveValue("d1904");
  await expect(dlg.getByTestId("opt-case")).toBeChecked();
  await dlg.getByTestId("dialog-cancel").click();
});

test("Input and Sort lists tabs persist prefs and custom lists", async ({ page }) => {
  await ready(page);
  await page.keyboard.press("Meta+,");
  const dlg = page.getByTestId("options-dialog");
  await dlg.getByTestId("tab-input").click();
  await dlg.getByTestId("opt-autoinput").uncheck();
  await dlg.getByTestId("opt-ac-capitalizeSentence").uncheck();
  await dlg.getByTestId("tab-sortLists").click();
  await dlg.getByTestId("sortlist-new").click();
  await dlg.getByTestId("sortlist-entries").fill("Q1\nQ2\nQ3\nQ4");
  await dlg.getByTestId("tab-loadSave").click();
  await dlg.getByTestId("opt-autosave").fill("3");
  await dlg.getByTestId("dialog-ok").click();
  const ops = await recorded(page);
  expect(ops.at(-1)).toMatchObject({ type: "setCustomSortLists", lists: [["Q1", "Q2", "Q3", "Q4"]] });
  const prefs = await page.evaluate(() => JSON.parse(localStorage.getItem("tally.prefs") ?? "{}") as Record<string, unknown>);
  expect(prefs).toMatchObject({ autoInput: false, autosaveMinutes: 3, autoCorrect: { capitalizeSentence: false } });
  // With AutoCorrect's sentence rule off, lowercase text stays lowercase.
  await page.evaluate(() => window.__tally!.controller.startEdit({ r: 20, c: 0 }, "teh thing", "grid"));
  await page.evaluate(() => window.__tally!.controller.commitEdit("none"));
  await expect.poll(() => recorded(page).then((o) => (o.at(-1) as { input?: string }).input)).toBe("the thing");
  // Reload keeps the prefs (localStorage in the browser build, prefs.json in the desktop app).
  await page.reload();
  await page.waitForFunction(() => window.__tally?.controller.grid != null);
  await page.keyboard.press("Meta+,");
  await dlg.getByTestId("tab-input").click();
  await expect(dlg.getByTestId("opt-autoinput")).not.toBeChecked();
});
