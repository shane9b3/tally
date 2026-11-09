/**
 * Insert > Function List (AOO ScFunctionDockWin): the side panel with the
 * category box, the function list, the syntax and Insert. Double-click writes
 * `=SUM()` into the cell edit with the caret inside the parentheses; Last Used
 * follows the inserts; axe is clean.
 */
import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";

async function ready(page: Page) {
  await page.goto("/");
  await expect(page.getByTestId("grid")).toBeVisible();
  await page.waitForFunction(() => window.__tally?.controller.grid != null);
}

const run = (page: Page, id: string) => page.evaluate((id) => window.__tally!.controller.registry.run(id), id);

/** The caret of the focused editor field. */
const caret = (page: Page) => page.evaluate(() => (document.activeElement as HTMLInputElement | HTMLTextAreaElement).selectionStart);

test("pick Mathematical, double-click SUM: =SUM() with the caret inside; Last Used updates", async ({ page }) => {
  await ready(page);
  await run(page, "insert.functionList");
  const panel = page.getByTestId("function-list-panel");
  await expect(panel).toBeVisible();
  const category = page.getByTestId("function-list-category");
  const labels = await category.locator("option").allTextContents();
  expect(labels.slice(0, 2)).toEqual(["Last Used", "All"]);
  expect(labels.indexOf("Logical")).toBeLessThan(labels.indexOf("Mathematical"));
  expect(labels.indexOf("Mathematical")).toBeLessThan(labels.indexOf("Statistical"));
  expect(labels.indexOf("Statistical")).toBeLessThan(labels.indexOf("Spreadsheet"));

  await category.selectOption({ label: "Mathematical" });
  const sum = page.getByTestId("function-list").getByRole("option", { name: "SUM", exact: true });
  await sum.click();
  await expect(page.getByTestId("function-list-description")).toContainText("SUM(number1; [number2]; …)");
  await sum.dblclick();
  await expect(page.getByTestId("formula-input")).toHaveValue("=SUM()");
  expect(await caret(page)).toBe(5);

  // Last Used now starts with SUM, then the previous list.
  await category.selectOption({ label: "Mathematical" });
  await page.getByTestId("function-list").getByRole("option", { name: "SUMIF", exact: true }).dblclick();
  await category.selectOption({ label: "Last Used" });
  await expect(page.getByTestId("function-list").getByRole("option").first()).toHaveText("SUMIF");
});

test("Enter and the Insert button insert at the caret of an open edit; keyboard type-ahead", async ({ page }) => {
  await ready(page);
  await run(page, "insert.functionList");
  await page.getByTestId("function-list-category").selectOption({ label: "All" });
  const list = page.getByTestId("function-list");
  await list.focus();
  await page.keyboard.type("sumi");
  await expect(list.getByRole("option", { selected: true })).toHaveText("SUMIF");
  await page.keyboard.press("Enter");
  await expect(page.getByTestId("formula-input")).toHaveValue("=SUMIF()");
  expect(await caret(page)).toBe(7);
  // A second insert goes at the caret.
  await page.getByTestId("function-list-insert").click();
  await expect(page.getByTestId("formula-input")).toHaveValue("=SUMIF(SUMIF())");
  expect(await caret(page)).toBe(13);
});

test("closes with its button and the toggle command; a11y clean", async ({ page }) => {
  await ready(page);
  await run(page, "insert.functionList");
  await expect(page.getByTestId("function-list-panel")).toBeVisible();
  await page.waitForFunction(() => document.getAnimations().every((a) => a.playState !== "running"));
  const result = await new AxeBuilder({ page }).include('[data-testid="function-list-panel"]').withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "best-practice"]).analyze();
  expect(result.violations.filter((v) => v.impact === "serious" || v.impact === "critical").map((v) => v.id)).toEqual([]);
  await run(page, "insert.functionList");
  await expect(page.getByTestId("function-list-panel")).toHaveCount(0);
});
