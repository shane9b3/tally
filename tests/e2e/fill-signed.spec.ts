/**
 * Typed formulas with a leading `+` / `-` (P18) and AutoFill of a single
 * number dragged up (T16), as AOO 4.1 does them.
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

async function cellInput(page: Page, r: number, c: number): Promise<string> {
  await clickCell(page, r, c);
  return page.getByTestId("formula-input").inputValue();
}

test("typing +B1+B2 stores the formula =+B1+B2", async ({ page }) => {
  await ready(page);
  await typeInto(page, 0, 1, "10");
  await typeInto(page, 1, 1, "20");
  await typeInto(page, 0, 3, "+B1+B2");
  await expect.poll(() => texts(page, 0, 3, 0, 3)).toEqual(["30"]);
  await expect.poll(() => cellInput(page, 0, 3)).toBe("=+B1+B2");
  // Signed numbers and unknown names stay values.
  await typeInto(page, 2, 3, "-5");
  await typeInto(page, 3, 3, "-abc");
  await expect.poll(() => texts(page, 2, 3, 3, 3)).toEqual(["-5", "-abc"]);
  await expect.poll(() => cellInput(page, 3, 3)).toBe("-abc");
});

test("a leading + is formula mode: clicking a cell inserts its reference", async ({ page }) => {
  await ready(page);
  await typeInto(page, 0, 0, "5");
  await clickCell(page, 0, 1);
  await page.keyboard.type("+");
  await clickCell(page, 0, 0);
  await expect(page.getByTestId("formula-input")).toHaveValue("+A1");
  await page.keyboard.press("Enter");
  await expect.poll(() => texts(page, 0, 1, 0, 1)).toEqual(["5"]);
  await expect.poll(() => cellInput(page, 0, 1)).toBe("=+A1");
});

test("a single number dragged up counts down", async ({ page }) => {
  await ready(page);
  await typeInto(page, 4, 0, "5");
  await clickCell(page, 4, 0);
  // Drag the handle (bottom-right corner of A5) up to A2.
  const b = await gridBox(page);
  const hx = b.x + b.headerW + COL_W - 1;
  const hy = b.y + HEADER_H + 5 * ROW_H - 1;
  await page.mouse.move(hx, hy);
  await page.mouse.down();
  const to = await cellXY(page, 1, 0);
  await page.mouse.move(to.x, to.y, { steps: 8 });
  await page.mouse.up();
  await expect.poll(() => texts(page, 1, 0, 4, 0)).toEqual(["2", "3", "4", "5"]);
});

test("⌘D copies instead of continuing a series (AOO Fill Down)", async ({ page }) => {
  await ready(page);
  await typeInto(page, 0, 0, "5");
  await clickCell(page, 0, 0);
  await page.keyboard.press("Shift+ArrowDown");
  await page.keyboard.press("Shift+ArrowDown");
  await page.keyboard.press("ControlOrMeta+d");
  await expect.poll(() => texts(page, 0, 0, 2, 0)).toEqual(["5", "5", "5"]);
});

test("a dash line or a negative time then an arrow key commits, not a reference", async ({ page }) => {
  await ready(page);
  await clickCell(page, 0, 0);
  await page.keyboard.type("-----");
  await page.keyboard.press("ArrowDown");
  await expect.poll(() => texts(page, 0, 0, 0, 0)).toEqual(["-----"]);
  await expect(page.getByTestId("name-box")).toHaveValue("A2");
  await page.keyboard.type("+B1+");
  await page.keyboard.press("ArrowDown"); // after an operator: picks A3
  await expect(page.getByTestId("formula-input")).toHaveValue("+B1+A3");
  await page.keyboard.press("Escape");
  await clickCell(page, 4, 0);
  await page.keyboard.type("-12:30");
  await page.keyboard.press("ArrowDown");
  await expect(page.getByTestId("name-box")).toHaveValue("A6");
  await expect.poll(() => cellInput(page, 4, 0)).not.toMatch(/^=/);
});
