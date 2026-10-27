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

test("Page setup… from Print Preview keeps the preview up while its chunk loads; Escape returns into the preview", async ({ page }) => {
  // Hold the dialog chunk back so its first mount really suspends (as on a cold start).
  let release!: () => void;
  const held = new Promise<void>((r) => (release = r));
  await page.route("**/components/Dialogs.tsx*", async (route) => {
    await held;
    await route.continue();
  });
  await ready(page);
  await run(page, "file.printPreview");
  const preview = page.getByTestId("print-preview");
  await expect(preview).toBeVisible();
  await preview.getByRole("button", { name: "Page setup…" }).click();
  // Still loading: the preview stays (a shared Suspense used to hide it, dropping the focus to <body>).
  await page.waitForTimeout(300);
  await expect(preview).toBeVisible();
  expect(await page.evaluate(() => document.activeElement !== document.body)).toBe(true);
  release();
  const dialog = page.getByTestId("page-setup-dialog");
  await expect(dialog).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
  await expect(preview).toBeVisible();
  await expect.poll(() => preview.evaluate((p) => p.contains(document.activeElement))).toBe(true);
});

/** Three sheets: the invoices template, "Sheet2" with 120 rows (several pages) and "Sheet3" with one cell. */
async function threeSheets(page: Page): Promise<number[]> {
  const ids = await page.evaluate(async () => {
    const c = window.__tally!.controller;
    await c.addSheet();
    await c.addSheet();
    const sheets = c.info!.sheets.map((s) => s.id);
    const ops: { type: "setCellInput"; sheet: number; r: number; c: number; input: string }[] = [];
    for (let r = 0; r < 120; r++) ops.push({ type: "setCellInput", sheet: sheets[1]!, r, c: 0, input: `two ${r}` });
    ops.push({ type: "setCellInput", sheet: sheets[2]!, r: 0, c: 0, input: "three" });
    await c.apply("fill", ops);
    return sheets;
  });
  expect(ids).toHaveLength(3);
  await page.getByRole("tablist", { name: "Sheets" }).getByRole("tab").nth(1).click();
  await expect.poll(() => page.evaluate(() => window.__tally!.controller.sheet)).toBe(ids[1]);
  // The engine hears about the switch (view state for save, CSV and Find).
  await expect.poll(async () => (await calls(page)).filter((c) => c.type === "setActiveSheet").at(-1)?.sheet).toBe(ids[1]);
  return ids;
}

/** Pages of one sheet as the engine lays them out. */
const pagesOf = (page: Page, sheet: number) =>
  page.evaluate(async (sheet) => {
    const t = window.__tally!;
    const opts = { sheets: { type: "active" as const }, selection: null, suppressEmptyPages: true, pageFrom: null, pageTo: null, activeSheet: sheet };
    return (await t.client.printLayout(t.controller.info!.doc, opts)).pages.length;
  }, sheet);

const pageCount = async (page: Page) => Number((await page.getByTestId("print-page-label").textContent())!.match(/of (\d+)/)![1]);

test("print preview opens on the active sheet; All sheets adds the others; a page list narrows it", async ({ page }) => {
  await ready(page);
  const ids = await threeSheets(page);
  const [n1, n2, n3] = [await pagesOf(page, ids[0]!), await pagesOf(page, ids[1]!), await pagesOf(page, ids[2]!)];
  expect(n2).toBeGreaterThan(1);
  await run(page, "file.printPreview");
  const preview = page.getByTestId("print-preview");
  await expect(preview).toBeVisible();
  // The owner's report: the preview showed the first sheet. Now it is sheet 2's pages only.
  await expect(page.getByTestId("print-page")).toHaveAttribute("data-sheet", String(ids[1]));
  await expect(page.getByTestId("print-page-label")).toHaveText(`Page 1 of ${n2}`);
  await expect(page.getByTestId("print-scope")).toHaveValue("selected");
  const prepared = (await calls(page)).filter((c) => c.type === "preparePrintJob").at(-1) as { opts: { sheets: { type: string }; activeSheet: number } };
  expect(prepared.opts.sheets.type).toBe("active");
  expect(prepared.opts.activeSheet).toBe(ids[1]);
  // All sheets: every sheet's pages, still opened at the active sheet's first page.
  await page.getByTestId("print-scope").selectOption("all");
  await expect(page.getByTestId("print-page-label")).toHaveText(`Page ${n1 + 1} of ${n1 + n2 + n3}`);
  await expect(page.getByTestId("print-page")).toHaveAttribute("data-sheet", String(ids[1]));
  expect(await pageCount(page)).toBeGreaterThan(n2);
  // A page list: "1, 3-4" of the whole job.
  await page.getByTestId("print-pages").fill("1, 3-4");
  await page.getByTestId("print-pages").press("Enter");
  await expect(page.getByTestId("print-page-label")).toHaveText(/of 3$/);
  // Invalid input is flagged and does not rebuild the job.
  const before = (await calls(page)).filter((c) => c.type === "preparePrintJob").length;
  await page.getByTestId("print-pages").fill("3-1");
  await page.getByTestId("print-pages").press("Enter");
  await expect(page.getByTestId("print-pages")).toHaveAttribute("aria-invalid", "true");
  expect((await calls(page)).filter((c) => c.type === "preparePrintJob").length).toBe(before);
  // Arrow keys in the field stay in the field.
  const label = await page.getByTestId("print-page-label").textContent();
  await page.getByTestId("print-pages").press("ArrowRight");
  await page.waitForTimeout(100);
  await expect(page.getByTestId("print-page-label")).toHaveText(label!);
  await page.getByTestId("print-pages").fill("");
  await page.getByTestId("print-pages").press("Enter");
  await expect(page.getByTestId("print-page-label")).toHaveText(`Page ${n1 + 1} of ${n1 + n2 + n3}`);
  // The range is remembered for the session.
  await page.getByTestId("print-close").click();
  await expect(preview).toHaveCount(0);
  await run(page, "file.printPreview");
  await expect(page.getByTestId("print-scope")).toHaveValue("all");
  // Selected cells: the selection on the active sheet.
  await page.evaluate(() => {
    const sel = { range: { r0: 4, c0: 0, r1: 9, c1: 0 }, active: { r: 4, c: 0 }, anchor: { r: 4, c: 0 } };
    (window.__tally!.controller as unknown as { onSelectionChange(s: unknown): void }).onSelectionChange(sel);
  });
  await page.getByTestId("print-scope").selectOption("selection");
  await expect(page.getByTestId("print-page-label")).toHaveText("Page 1 of 1");
  const last = (await calls(page)).filter((c) => c.type === "preparePrintJob").at(-1) as { opts: { selection: unknown; activeSheet: number } };
  expect(last.opts).toMatchObject({ selection: [{ r0: 4, c0: 0, r1: 9, c1: 0 }], activeSheet: ids[1] });
  // A multi-range selection prints every range, in order.
  await page.getByTestId("print-close").click();
  await page.evaluate(() => {
    const sel = { range: { r0: 60, c0: 0, r1: 62, c1: 0 }, active: { r: 60, c: 0 }, anchor: { r: 60, c: 0 }, others: [{ r0: 0, c0: 0, r1: 2, c1: 0 }] };
    (window.__tally!.controller as unknown as { onSelectionChange(s: unknown): void }).onSelectionChange(sel);
  });
  await run(page, "file.printPreview");
  await expect(page.getByTestId("print-page-label")).toHaveText("Page 1 of 2");
  const multi = (await calls(page)).filter((c) => c.type === "preparePrintJob").at(-1) as { opts: { selection: unknown } };
  expect(multi.opts.selection).toEqual([
    { r0: 0, c0: 0, r1: 2, c1: 0 },
    { r0: 60, c0: 0, r1: 62, c1: 0 },
  ]);
  // A whole column prints the column's data, not a million rows.
  await page.getByTestId("print-close").click();
  await page.evaluate(() => {
    const sel = { range: { r0: 0, c0: 0, r1: 1_048_575, c1: 0 }, active: { r: 0, c: 0 }, anchor: { r: 0, c: 0 } };
    (window.__tally!.controller as unknown as { onSelectionChange(s: unknown): void }).onSelectionChange(sel);
  });
  await run(page, "file.printPreview");
  await expect(page.getByTestId("print-page-label")).toHaveText(`Page 1 of ${n2}`);
});

test("Export as PDF asks for the range and defaults to the active sheet", async ({ page }) => {
  await ready(page);
  const ids = await threeSheets(page);
  // The web build has no save dialog: stand one in that records the job.
  await page.evaluate(() => {
    const w = window as unknown as { __pdfJobs: number[] };
    w.__pdfJobs = [];
    (window.__tally!.controller.shell as unknown as { exportPdf(doc: number, job: number): Promise<void> }).exportPdf = async (_doc, job) => void w.__pdfJobs.push(job);
  });
  await run(page, "file.exportPdf");
  const dialog = page.getByTestId("print-range-dialog");
  await expect(dialog).toBeVisible();
  await expect(dialog.getByTestId("print-scope-selected")).toBeChecked();
  await expect(dialog.getByTestId("print-pages-all")).toBeChecked();
  await dialog.getByTestId("dialog-ok").click();
  await expect(dialog).toHaveCount(0);
  await expect.poll(() => page.evaluate(() => (window as unknown as { __pdfJobs: number[] }).__pdfJobs.length)).toBe(1);
  const prepared = (await calls(page)).filter((c) => c.type === "preparePrintJob").at(-1) as { opts: { sheets: { type: string }; activeSheet: number; pageRanges: unknown } };
  expect(prepared.opts).toMatchObject({ sheets: { type: "active" }, activeSheet: ids[1], pageRanges: null });
  // Pages "2" of all sheets; a bad page list keeps OK disabled.
  await run(page, "file.exportPdf");
  await dialog.getByTestId("print-scope-all").check();
  await dialog.getByTestId("print-pages-spec").fill("0");
  await expect(dialog.getByTestId("dialog-ok")).toBeDisabled();
  await dialog.getByTestId("print-pages-spec").fill("2");
  await dialog.getByTestId("dialog-ok").click();
  await expect.poll(() => page.evaluate(() => (window as unknown as { __pdfJobs: number[] }).__pdfJobs.length)).toBe(2);
  const second = (await calls(page)).filter((c) => c.type === "preparePrintJob").at(-1) as { opts: unknown };
  expect(second.opts).toMatchObject({ sheets: { type: "all" }, activeSheet: ids[1], pageRanges: [[2, 2]] });
  // Escape cancels.
  await run(page, "file.exportPdf");
  await expect(dialog.getByTestId("print-scope-all")).toBeChecked();
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
});
