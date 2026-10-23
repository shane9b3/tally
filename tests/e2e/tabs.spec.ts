/**
 * The sidebar's "Editing" list in the browser build: there is no desktop
 * shell, so `?tabs=<n>` (dev builds only, `@tally/app/dev` mockTabs.ts)
 * fills the tab store with a mock: `n` tabs in this window plus one
 * document in another window. Rows switch and close through the (mock) tab
 * shell; without the mock the section shows the one document as before.
 */
import { expect, test } from "@playwright/test";

test("the Open list shows every document, switches and closes the others", async ({ page }) => {
  await page.goto("/?tabs=3");
  await expect(page.getByTestId("grid")).toBeVisible();
  const section = page.getByTestId("open-documents");
  await expect(section.getByRole("heading")).toHaveText("Editing");
  const list = section.getByRole("tablist", { name: "Open documents" });
  const rows = list.getByRole("tab");
  await expect(rows).toHaveCount(4);
  await expect(rows.nth(0)).toContainText("Untitled");
  await expect(rows.nth(1)).toContainText("Book 2.ods");
  await expect(rows.nth(1)).toContainText("just now");
  await expect(rows.nth(1).getByRole("img", { name: /^Edited/ })).toBeVisible();
  await expect(rows.nth(3)).toContainText("Report.xlsx");
  await expect(rows.nth(3)).toHaveAttribute("data-window", "win-2");
  await expect(rows.nth(0)).toHaveAttribute("aria-selected", "true");

  await rows.nth(2).click();
  await expect(rows.nth(2)).toHaveAttribute("aria-selected", "true");
  await expect(rows.nth(0)).toHaveAttribute("aria-selected", "false");

  // Another window's row: Close Others / Move to New Window are off.
  await rows.nth(3).click({ button: "right" });
  const menu = page.getByTestId("open-doc-menu");
  await expect(menu.getByRole("menuitem", { name: "Close Others" })).toHaveAttribute("aria-disabled", "true");
  await page.keyboard.press("Escape");

  await rows.nth(2).click({ button: "right" });
  await menu.getByRole("menuitem", { name: "Close Others" }).click();
  await expect(rows).toHaveCount(2);
  await expect(rows.nth(0)).toContainText("Book 3.ods");
  await expect(rows.nth(1)).toContainText("Report.xlsx");

  // × on hover closes the other window's document.
  await rows.nth(1).hover();
  await page.getByTestId("open-doc-close-doc-21").click();
  await expect(rows).toHaveCount(1);
});

test("one document: the Open section has one highlighted row", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByTestId("grid")).toBeVisible();
  const rows = page.getByTestId("open-documents").getByRole("tab");
  await expect(rows).toHaveCount(1);
  await expect(rows.nth(0)).toHaveAttribute("aria-selected", "true");
  await expect(page.getByTestId("open-documents").locator('[data-testid^="open-doc-close-"]')).toHaveCount(0);
});
