/**
 * v0.2 UI flows in the browser (MockClient + structural-op shim): fill
 * handle, multi-range stats, font colour, context-menu insert row, dark
 * theme, "At a glance", function autocomplete and point mode.
 */
import { expect, test, type Page } from "@playwright/test";

const ROW_H = 24;
const COL_W = 100;
const HEADER_H = 24;

async function gridBox(page: Page) {
  const box = await page.getByTestId("grid").boundingBox();
  if (!box) throw new Error("grid not visible");
  const headerW = await page.evaluate(() => {
    const g = window.__tally?.controller.grid as unknown as { headerW?: number } | null | undefined;
    return g?.headerW ?? 44;
  });
  return { ...box, headerW };
}

/** Client coordinates of a cell's centre (default sizes). */
async function cellXY(page: Page, r: number, c: number) {
  const b = await gridBox(page);
  return { x: b.x + b.headerW + c * COL_W + COL_W / 2, y: b.y + HEADER_H + r * ROW_H + ROW_H / 2 };
}

async function clickCell(page: Page, r: number, c: number, modifier?: "ControlOrMeta" | "Shift") {
  const p = await cellXY(page, r, c);
  if (modifier) await page.keyboard.down(modifier);
  await page.mouse.click(p.x, p.y);
  if (modifier) await page.keyboard.up(modifier);
}

async function typeInto(page: Page, r: number, c: number, text: string) {
  await clickCell(page, r, c);
  await page.keyboard.type(text);
  await page.keyboard.press("Enter");
}

/** Texts of a range, read through the engine client (row-major, "" for empty). */
async function texts(page: Page, r0: number, c0: number, r1: number, c1: number): Promise<string[]> {
  return page.evaluate(
    async ([r0, c0, r1, c1]) => {
      const t = window.__tally;
      const info = t?.controller.info;
      if (!t || !info) return [];
      const [block] = await t.client.cells(info.doc, t.controller.sheet, [{ r0, c0, r1, c1 }]);
      const out: string[] = [];
      for (let r = r0; r <= r1; r++)
        for (let c = c0; c <= c1; c++) out.push(block?.cells.find((x) => x.r === r && x.c === c)?.text ?? "");
      return out;
    },
    [r0, c0, r1, c1] as const,
  );
}

async function ready(page: Page, url = "/") {
  await page.goto(url);
  await expect(page.getByTestId("grid")).toBeVisible();
  await page.waitForFunction(() => window.__tally?.controller.grid != null);
}

test.beforeEach(async ({ page }) => {
  // A clean theme preference per test (but kept across reloads within it).
  await page.addInitScript(() => {
    if (!sessionStorage.getItem("e2e-cleared")) {
      localStorage.clear();
      sessionStorage.setItem("e2e-cleared", "1");
    }
  });
});

test("fill handle continues a numeric series", async ({ page }) => {
  await ready(page);
  await typeInto(page, 0, 0, "1");
  await typeInto(page, 1, 0, "2");
  await clickCell(page, 0, 0);
  await page.keyboard.press("Shift+ArrowDown");
  await expect(page.getByTestId("name-box")).toHaveValue("A1:A2");
  // The handle is at the bottom-right corner of A2.
  const b = await gridBox(page);
  const hx = b.x + b.headerW + COL_W - 1;
  const hy = b.y + HEADER_H + 2 * ROW_H - 1;
  await page.mouse.move(hx, hy);
  await page.mouse.down();
  const to = await cellXY(page, 4, 0);
  await page.mouse.move(to.x, to.y, { steps: 6 });
  await page.mouse.up();
  await expect.poll(() => texts(page, 0, 0, 4, 0)).toEqual(["1", "2", "3", "4", "5"]);
  await expect(page.getByTestId("name-box")).toHaveValue("A1:A5");
});

test("multi-range selection: stats cover every range", async ({ page }) => {
  await ready(page);
  await typeInto(page, 0, 0, "1");
  await typeInto(page, 1, 0, "2");
  await typeInto(page, 0, 2, "10");
  await clickCell(page, 0, 0);
  await page.keyboard.press("Shift+ArrowDown");
  await clickCell(page, 0, 2, "ControlOrMeta");
  await expect(page.getByTestId("stat-sum")).toContainText("13");
  await expect(page.getByTestId("stat-count")).toContainText("3");
});

test("font colour applies to the selection and the toolbar reflects it", async ({ page }) => {
  await ready(page);
  await typeInto(page, 0, 0, "red text");
  await clickCell(page, 0, 0);
  await page.getByTestId("font-color").click();
  await page.locator('[data-testid="color-picker"] [data-color="#dc2626"]').click();
  const color = await page.evaluate(async () => {
    const t = window.__tally;
    const info = t?.controller.info;
    if (!t || !info) return null;
    const [block] = await t.client.cells(info.doc, t.controller.sheet, [{ r0: 0, c0: 0, r1: 0, c1: 0 }]);
    const [style] = await t.client.styles(info.doc, [block?.cells[0]?.style ?? 0]);
    return style?.color ?? null;
  });
  expect(color).toBe(0xdc2626ff);
  await expect(page.getByTestId("font-color").locator("span").last()).toHaveAttribute("style", /rgb\(220, 38, 38\)|#dc2626/);
});

test("context menu on a row header inserts a row", async ({ page }) => {
  await ready(page, "/?template=invoices");
  const b = await gridBox(page);
  // Row 2 (index 1) header.
  await page.mouse.click(b.x + b.headerW / 2, b.y + HEADER_H + ROW_H + ROW_H / 2, { button: "right" });
  const menu = page.getByTestId("context-menu");
  await expect(menu).toBeVisible();
  await expect(menu.locator('[data-item="hide"]')).not.toHaveAttribute("aria-disabled", "true");
  await menu.locator('[data-item="insertAbove"]').click();
  await expect.poll(() => texts(page, 0, 0, 2, 0)).toEqual(["Date", "", "10/01/2026"]);
  // One undo step restores it.
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(() => texts(page, 1, 0, 1, 0)).toEqual(["10/01/2026"]);
});

test("theme switch cycles to dark, persists and repaints the grid", async ({ page }) => {
  await ready(page, "/?template=invoices");
  const toggle = page.getByTestId("theme-toggle");
  await expect(toggle).toHaveAttribute("data-theme-pref", "system");
  await toggle.click(); // light
  await toggle.click(); // dark
  await expect(toggle).toHaveAttribute("data-theme-pref", "dark");
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  expect(await page.evaluate(() => localStorage.getItem("tally.theme"))).toBe("dark");
  // The canvas root takes the dark background token.
  const bg = await page.evaluate(() => getComputedStyle(document.querySelector(".tally-grid") as HTMLElement).backgroundColor);
  expect(bg).toBe("rgb(22, 24, 28)");
  await page.reload();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
});

test("At a glance shows the Sum of the Total column (invoices)", async ({ page }) => {
  await ready(page, "/?template=invoices");
  await clickCell(page, 3, 5);
  await page.keyboard.press("ControlOrMeta+Shift+g");
  const panel = page.getByTestId("glance-panel");
  await expect(panel).toBeVisible();
  await expect(panel.getByTestId("glance-sum-Total")).toContainText("$686.10");
  await expect(panel.getByTestId("glance-rows")).toContainText("14");
  await expect(panel.getByTestId("glance-counts")).toContainText("Paid");
  await expect(panel.getByTestId("glance-chart")).toContainText("Total by Status");
});

test("function autocomplete inserts SUM(", async ({ page }) => {
  await ready(page);
  await clickCell(page, 0, 0);
  await page.keyboard.type("=SU");
  const list = page.getByTestId("fn-autocomplete");
  await expect(list).toBeVisible();
  await expect(list).toContainText("SUM");
  await page.keyboard.press("Tab");
  await expect(page.getByTestId("formula-input")).toHaveValue("=SUM(");
  await expect(list).toHaveCount(0);
  await expect(page.getByTestId("fn-signature")).toContainText("SUM(number1");
});

test("point mode: clicking a cell inserts its reference", async ({ page }) => {
  await ready(page);
  await typeInto(page, 0, 0, "5");
  await typeInto(page, 1, 0, "7");
  await clickCell(page, 0, 1);
  await page.keyboard.type("=");
  await clickCell(page, 0, 0);
  await expect(page.getByTestId("formula-input")).toHaveValue("=A1");
  await page.keyboard.type("+");
  await page.keyboard.press("ArrowDown"); // from B1: B2
  await page.keyboard.press("ArrowLeft"); // A2
  await expect(page.getByTestId("formula-input")).toHaveValue("=A1+A2");
  await page.keyboard.press("Enter");
  await expect.poll(() => texts(page, 0, 1, 0, 1)).toEqual(["12"]);
});
