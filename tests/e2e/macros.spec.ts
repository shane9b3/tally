/**
 * A file with macros (MockClient `?macros=1`: the workbook opens like an
 * `.xlsm` whose VBA project Tally drops): the one-shot notice that macros
 * do not run and are removed on save, in the UI language.
 */
import { expect, test } from "@playwright/test";

const NOTICE = "This document contains macros. Tally does not run macros; they will be removed when you save.";

test("opening a file with macros says they are not run and are removed on save", async ({ page }) => {
  await page.goto("/?macros=1");
  await expect(page.getByTestId("grid")).toBeVisible();
  await expect(page.getByText(NOTICE).first()).toBeVisible();
  expect(await page.evaluate(() => window.__tally?.controller.info?.title)).toBe("Book.xlsm");
});

test("a file without macros shows no macro notice", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByTestId("grid")).toBeVisible();
  await page.waitForFunction(() => window.__tally?.controller.grid != null);
  await expect(page.getByText(NOTICE)).toHaveCount(0);
});
