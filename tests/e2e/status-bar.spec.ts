/**
 * The status bar's selection function and selection mode (parity U19), in
 * the browser build: one function at a time (`Sum=…` by default), chosen
 * from the field's menu with AOO's list and kept as an app setting; the
 * STD / EXT / ADD field follows F8 / ⇧F8 and steps on a click.
 */
import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";

const ROW_H = 24;
const COL_W = 100;
const HEADER_H = 24;

async function ready(page: Page) {
  await page.goto("/");
  await expect(page.getByTestId("grid")).toBeVisible();
  await page.waitForFunction(() => window.__tally?.controller.grid != null);
}

async function cellXY(page: Page, r: number, c: number) {
  const box = await page.getByTestId("grid").boundingBox();
  if (!box) throw new Error("grid not visible");
  const headerW = await page.evaluate(() => {
    const g = window.__tally?.controller.grid as unknown as { headerW?: number } | null | undefined;
    return g?.headerW ?? 44;
  });
  return { x: box.x + headerW + c * COL_W + COL_W / 2, y: box.y + HEADER_H + r * ROW_H + ROW_H / 2 };
}

async function clickCell(page: Page, r: number, c: number) {
  const p = await cellXY(page, r, c);
  await page.mouse.click(p.x, p.y);
}

async function typeInto(page: Page, r: number, c: number, text: string) {
  await clickCell(page, r, c);
  await page.keyboard.type(text);
  await page.keyboard.press("Enter");
}

async function fillAndSelect(page: Page) {
  await typeInto(page, 0, 0, "2");
  await typeInto(page, 1, 0, "4");
  await typeInto(page, 2, 0, "x");
  await typeInto(page, 3, 0, "9");
  await clickCell(page, 0, 0);
  for (let i = 0; i < 3; i++) await page.keyboard.press("Shift+ArrowDown");
  await expect(page.getByTestId("name-box")).toHaveValue("A1:A4");
}

test("function menu: AOO's list, one function shown, the choice is kept", async ({ page }) => {
  await ready(page);
  await fillAndSelect(page);
  const func = page.getByTestId("stat-func");
  await expect(func).toHaveText("Sum=15");

  // Right click opens the menu (AOO: context menu of the field).
  await func.click({ button: "right" });
  const menu = page.getByTestId("stat-func-menu");
  await expect(menu.getByRole("menuitemcheckbox")).toHaveText([/^Average$/, /^CountA$/, /^Count$/, /^Maximum$/, /^Minimum$/, /^✓Sum$/, /^None$/]);
  await expect(menu.getByRole("menuitemcheckbox", { checked: true })).toHaveText(/Sum$/);

  const pick = async (id: string) => {
    if (!(await menu.isVisible())) await func.click();
    await menu.locator(`[data-item="${id}"]`).click();
    await expect(menu).toHaveCount(0);
  };
  await pick("average");
  await expect(func).toHaveText("Average=5");
  await pick("countA");
  await expect(func).toHaveText("CountA=4");
  await pick("count");
  await expect(func).toHaveText("Count=3");
  await pick("max");
  await expect(func).toHaveText("Max=9");
  await pick("min");
  await expect(func).toHaveText("Min=2");
  await pick("none");
  await expect(func).toHaveText("");
  await expect(func).toHaveAttribute("aria-label", "Selection function: None");

  // The field stays clickable with None, and the choice survives a reload.
  await pick("countA");
  await expect(func).toHaveText("CountA=4");
  await ready(page);
  await expect(page.getByTestId("stat-func")).toHaveText(/^CountA=/);
});

test("function field: text cells only give Sum=0 and an empty Average", async ({ page }) => {
  await ready(page);
  await typeInto(page, 0, 0, "a");
  await typeInto(page, 1, 0, "b");
  await clickCell(page, 0, 0);
  await page.keyboard.press("Shift+ArrowDown");
  const func = page.getByTestId("stat-func");
  await expect(func).toHaveText("Sum=0");
  await func.click();
  await page.getByTestId("stat-func-menu").locator('[data-item="average"]').click();
  await expect(func).toHaveText("Average=");
});

test("selection mode field: follows F8 / Shift+F8 and steps on a click", async ({ page }) => {
  await ready(page);
  await clickCell(page, 1, 1);
  const mode = page.getByTestId("stat-selmode");
  await expect(mode).toHaveText("STD");
  await page.keyboard.press("F8");
  await expect(mode).toHaveText("EXT");
  await page.keyboard.press("Shift+F8");
  await expect(mode).toHaveText("ADD");
  await page.keyboard.press("Escape");
  await expect(mode).toHaveText("STD");

  await mode.click();
  await expect(mode).toHaveText("EXT");
  await expect(mode).toHaveAttribute("data-mode", "ext");
  // The grid has the focus again: arrows extend the selection.
  await page.keyboard.press("ArrowDown");
  await expect(page.getByTestId("name-box")).toHaveValue("B2:B3");
  await mode.click();
  await expect(mode).toHaveText("ADD");
  await mode.click();
  await expect(mode).toHaveText("STD");
});

test("status bar and its function menu pass axe", async ({ page }) => {
  await ready(page);
  await fillAndSelect(page);
  await page.getByTestId("stat-func").click();
  await expect(page.getByTestId("stat-func-menu")).toBeVisible();
  await page.waitForFunction(() => document.getAnimations().every((a) => a.playState !== "running"));
  for (const sel of ['[data-testid="status-bar"]', '[data-testid="stat-func-menu"]']) {
    const result = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "best-practice"]).include(sel).analyze();
    expect(result.violations.map((v) => `${v.impact} ${v.id}: ${v.nodes.map((n) => n.target.join(" ")).join(", ")}`)).toEqual([]);
  }
});
