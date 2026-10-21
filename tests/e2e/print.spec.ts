/**
 * W5-5 printing in the browser: Print Preview opens with the mock layout's
 * pages (thumbnails + the rasterized current page), zoom and navigation;
 * Page Break Preview toggles and a break line can be dragged
 * (`setPageBreak` ops); the split window; zoom presets.
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
const calls = (page: Page): Promise<Rec[]> =>
  page.evaluate(() => JSON.parse(JSON.stringify((window.__tally!.client as unknown as { recordedCalls: unknown[] }).recordedCalls)) as Rec[]);

test("print preview shows the mock layout's pages and navigates", async ({ page }) => {
  await ready(page);
  await run(page, "file.printPreview");
  const preview = page.getByTestId("print-preview");
  await expect(preview).toBeVisible();
  await expect(page.getByTestId("print-page-label")).toHaveText(/Page 1 of \d+/);
  await expect(page.getByTestId("print-page")).toBeVisible();
  const n = Number((await page.getByTestId("print-page-label").textContent())!.match(/of (\d+)/)![1]);
  expect(n).toBeGreaterThanOrEqual(1);
  await expect(page.getByTestId("print-thumbs").locator("img")).toHaveCount(Math.min(n, 12));
  const prepared = (await calls(page)).filter((c) => c.type === "preparePrintJob");
  expect(prepared).toHaveLength(1);
  await page.getByRole("button", { name: "Zoom in" }).click();
  await expect(page.getByTestId("print-zoom")).toHaveText("125%");
  if (n > 1) {
    await page.getByRole("button", { name: "Next page" }).click();
    await expect(page.getByTestId("print-page-label")).toHaveText(`Page 2 of ${n}`);
  }
  await page.keyboard.press("Escape");
  await expect(preview).toHaveCount(0);
});

test("page break preview draws the watermark and dragging a break sends setPageBreak", async ({ page }) => {
  await ready(page);
  await run(page, "view.pageBreakPreview");
  await expect(page.getByTestId("grid")).toHaveAttribute("data-page-break-preview", "true");
  const breaks = await page.evaluate(async () => {
    const t = window.__tally!;
    return t.client.pageBreaks(t.controller.info!.doc, t.controller.sheet);
  });
  expect(breaks.area).not.toBeNull();
  // Add a manual break above row 6, then drag it to row 9.
  await page.evaluate(() => {
    const t = window.__tally!;
    const sel = { range: { r0: 5, c0: 0, r1: 5, c1: 0 }, active: { r: 5, c: 0 }, anchor: { r: 5, c: 0 } };
    (t.controller as unknown as { onSelectionChange(s: unknown): void }).onSelectionChange(sel);
  });
  await run(page, "insert.pageBreak");
  await expect.poll(async () => (await recorded(page)).filter((o) => o.type === "setPageBreak").length).toBe(1);
  const box = (await page.getByTestId("grid").boundingBox())!;
  const headerW = await page.evaluate(() => (window.__tally!.controller.grid as unknown as { headerW: number }).headerW);
  const y = box.y + 24 + 5 * 24;
  await page.mouse.move(box.x + headerW + 150, y);
  await page.mouse.down();
  await page.mouse.move(box.x + headerW + 150, y + 3 * 24 + 4, { steps: 5 });
  await page.mouse.up();
  await expect.poll(async () => (await recorded(page)).filter((o) => o.type === "setPageBreak").length).toBe(3);
  const last = (await recorded(page)).slice(-2) as { type: string; at: number; present: boolean }[];
  expect(last.map((o) => [o.at, o.present])).toEqual([
    [5, false],
    [8, true],
  ]);
  await run(page, "view.pageBreakPreview");
  await expect(page.getByTestId("grid")).not.toHaveAttribute("data-page-break-preview", "true");
});

test("split window shows two panes and persists with setSplit; Remove Split clears it", async ({ page }) => {
  await ready(page);
  await page.evaluate(() => {
    const t = window.__tally!;
    const sel = { range: { r0: 6, c0: 1, r1: 6, c1: 1 }, active: { r: 6, c: 1 }, anchor: { r: 6, c: 1 } };
    (t.controller as unknown as { onSelectionChange(s: unknown): void }).onSelectionChange(sel);
  });
  await run(page, "view.split");
  await expect(page.locator(".tally-split")).toHaveCount(1);
  await expect(page.locator(".tally-split .tally-grid")).toHaveCount(2);
  await expect(page.getByTestId("grid")).toHaveAttribute("data-split", "horizontal");
  await expect.poll(async () => (await recorded(page)).filter((o) => o.type === "setSplit").length).toBe(1);
  const op = (await recorded(page)).find((o) => o.type === "setSplit") as { split: { xPx: number; yPx: number } };
  expect(op.split.yPx).toBeGreaterThan(0);
  expect(op.split.xPx).toBe(0);
  await run(page, "view.removeSplit");
  await expect(page.locator(".tally-split")).toHaveCount(0);
  await expect(page.locator(".tally-grid")).toHaveCount(1);
});

test("zoom presets: optimal fits the used area, page width uses the print layout", async ({ page }) => {
  await ready(page);
  await run(page, "view.zoomPageWidth");
  await expect.poll(async () => (await calls(page)).filter((c) => c.type === "printLayout").length).toBe(1);
  await expect(page.getByTestId("zoom")).not.toHaveText("100%");
  await run(page, "view.zoomReset");
  await expect(page.getByTestId("zoom")).toHaveText("100%");
  await run(page, "view.zoomOptimal");
  await expect(page.getByTestId("zoom")).not.toHaveText("100%");
});
