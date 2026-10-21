/**
 * The unsaved-changes guard in the browser (MockClient): editing a cell and
 * then starting from a sidebar template asks Save / Don't Save / Cancel;
 * Cancel (and Escape) keep the edit, Don't Save replaces the workbook.
 * Typing not yet committed with Enter is committed first (as in AOO).
 */
import { expect, test, type Page } from "@playwright/test";

const ROW_H = 24;
const COL_W = 100;

async function clickCell(page: Page, r: number, c: number): Promise<void> {
  const grid = page.getByTestId("grid");
  const box = await grid.boundingBox();
  if (!box) throw new Error("grid not visible");
  const headerW = await page.evaluate(() => {
    const g = window.__tally?.controller.grid as unknown as { headerW?: number } | null | undefined;
    return g?.headerW ?? 44;
  });
  await page.mouse.click(box.x + headerW + c * COL_W + COL_W / 2, box.y + ROW_H + r * ROW_H + ROW_H / 2);
}

async function a1(page: Page): Promise<string | null> {
  return page.evaluate(async () => {
    const t = window.__tally;
    const info = t?.controller.info;
    if (!t || !info) return null;
    const [block] = await t.client.cells(info.doc, t.controller.sheet, [{ r0: 0, c0: 0, r1: 0, c1: 0 }]);
    return block?.cells[0]?.text ?? null;
  });
}

test.beforeEach(async ({ page }) => {
  await page.goto("/");
  await expect(page.getByTestId("grid")).toBeVisible();
  await page.waitForFunction(() => window.__tally?.controller.grid != null);
});

test("editing then picking a sidebar template asks to save; Cancel keeps the edit", async ({ page }) => {
  await clickCell(page, 0, 0);
  await page.keyboard.type("1234");
  await page.keyboard.press("Enter");
  await expect.poll(() => a1(page)).toBe("1234");
  const title = await page.evaluate(() => window.__tally?.controller.info?.title ?? "");

  await page.getByRole("button", { name: "Monthly budget" }).click();
  const prompt = page.getByTestId("save-prompt");
  await expect(prompt).toBeVisible();
  await expect(prompt).toContainText(`Do you want to save the changes you made to “${title}”?`);
  await expect(prompt).toContainText("Your changes will be lost if you don't save them.");
  // Save is the default button.
  await expect(page.getByTestId("save-prompt-save")).toBeFocused();

  await page.getByTestId("save-prompt-cancel").click();
  await expect(prompt).toBeHidden();
  expect(await a1(page)).toBe("1234");
  expect(await page.evaluate(() => window.__tally?.controller.info?.dirty)).toBe(true);

  // Escape is Cancel too.
  await page.getByRole("button", { name: "Monthly budget" }).click();
  await expect(prompt).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(prompt).toBeHidden();
  expect(await a1(page)).toBe("1234");

  // Don't Save replaces the workbook with the template.
  await page.getByRole("button", { name: "Monthly budget" }).click();
  await page.getByTestId("save-prompt-discard").click();
  await expect(prompt).toBeHidden();
  await expect.poll(() => page.evaluate(() => window.__tally?.controller.info?.title)).not.toBe(title);
  expect(await page.evaluate(() => window.__tally?.controller.info?.dirty)).toBe(false);
});

test("typing without Enter, then picking a sidebar template, commits the typing and asks", async ({ page }) => {
  await clickCell(page, 0, 0);
  await page.keyboard.type("5678");
  // Still editing: nothing is in the cell yet.
  expect(await page.evaluate(() => window.__tally?.controller.pendingEdit)).toBe(true);
  expect(await a1(page)).not.toBe("5678");

  // Clicking the sidebar blurs the editor without ending the edit; the guard commits it.
  await page.getByRole("button", { name: "Monthly budget" }).click();
  const prompt = page.getByTestId("save-prompt");
  await expect(prompt).toBeVisible();
  await page.getByTestId("save-prompt-cancel").click();
  await expect(prompt).toBeHidden();
  expect(await a1(page)).toBe("5678");
  expect(await page.evaluate(() => window.__tally?.controller.pendingEdit)).toBe(false);
  expect(await page.evaluate(() => window.__tally?.controller.info?.dirty)).toBe(true);
});

test("a clean workbook is replaced without asking", async ({ page }) => {
  await page.getByRole("button", { name: "Stock count" }).click();
  await expect.poll(() => page.evaluate(() => window.__tally?.controller.info?.title ?? "")).toMatch(/Stock/);
  await expect(page.getByTestId("save-prompt")).toHaveCount(0);
});
