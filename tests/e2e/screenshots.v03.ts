/**
 * v0.3 screenshots for docs/qa (not part of `pnpm e2e`: the file name does
 * not end in `.spec.ts`). Run against the browser build:
 *
 *   pnpm --filter tally-desktop exec playwright test -c ../../tests/e2e/screenshots.config.ts
 *
 * Writes docs/qa/screenshots/v03-*.png (WebMockClient emulates the
 * conditional formats, validations and notes it shows).
 */
import { expect, test, type Page } from "@playwright/test";

const OUT = process.env.TALLY_SHOTS ?? "../../docs/qa/screenshots";
const ROW_H = 24;
const HEADER_H = 24;

async function ready(page: Page) {
  await page.goto("/?template=invoices");
  await expect(page.getByTestId("grid")).toBeVisible();
  await page.waitForFunction(() => window.__tally?.controller.grid != null);
  await page.evaluate(() => document.fonts.ready);
  await page.waitForTimeout(150);
}

async function cellXY(page: Page, r: number, c: number) {
  const box = await page.getByTestId("grid").boundingBox();
  if (!box) throw new Error("grid not visible");
  const { headerW, xs, w } = await page.evaluate((c) => {
    const ctl = window.__tally!.controller;
    let x = 0;
    for (let i = 0; i < c; i++) x += ctl.layoutSize("col", i);
    return { headerW: (ctl.grid as unknown as { headerW: number }).headerW, xs: x, w: ctl.layoutSize("col", c) };
  }, c);
  return { x: box.x + headerW + xs + w / 2, y: box.y + HEADER_H + r * ROW_H + ROW_H / 2, w, left: box.x + headerW + xs, right: box.x + headerW + xs + w, top: box.y + HEADER_H + r * ROW_H };
}

/** Adds the Total column formats: colour scale, data bar, icon set. */
async function totalFormats(page: Page) {
  await page.evaluate(async () => {
    const v3 = window.__tally!.controller.v3;
    const total = [{ r0: 1, c0: 5, r1: 14, c1: 5 }];
    await v3.addConditionalFormat(total, [
      { kind: { type: "iconSet", set: "threeArrows", thresholds: [{ kind: "percent", value: "0" }, { kind: "percent", value: "33" }, { kind: "percent", value: "67" }], showValue: true, reverse: false }, style: { bold: null, italic: null, underline: null, strike: null, color: null, fill: null, borders: null, numberFormat: null, display: null }, stopIfTrue: false },
      { kind: { type: "dataBar", min: { kind: "autoMin", value: "" }, max: { kind: "autoMax", value: "" }, color: 0x638ec6ff, negativeColor: 0xff0000ff, showValue: true, gradient: true }, style: { bold: null, italic: null, underline: null, strike: null, color: null, fill: null, borders: null, numberFormat: null, display: null }, stopIfTrue: false },
      { kind: { type: "colorScale", stops: [{ threshold: { kind: "min", value: "" }, color: 0xf8696bff }, { threshold: { kind: "percentile", value: "50" }, color: 0xffeb84ff }, { threshold: { kind: "max", value: "" }, color: 0x63be7bff }] }, style: { bold: null, italic: null, underline: null, strike: null, color: null, fill: null, borders: null, numberFormat: null, display: null }, stopIfTrue: false },
    ]);
  });
}

test("CF manager", async ({ page }) => {
  await ready(page);
  await totalFormats(page);
  await page.evaluate(async () => {
    const ctl = window.__tally!.controller;
    await ctl.v3.addConditionalFormat([{ r0: 1, c0: 3, r1: 14, c1: 3 }], [
      { kind: { type: "cellValue", op: "gt", formulas: ["20"] }, style: { bold: true, italic: null, underline: null, strike: null, color: 0x991b1bff, fill: 0xfee2e2ff, borders: null, numberFormat: null, display: null }, stopIfTrue: false },
    ]);
    await ctl.colorByValueOn({ r0: 1, c0: 6, r1: 14, c1: 6 });
  });
  await page.locator('[data-testid="highlight-cells"]').click();
  await page.getByTestId("highlight-menu").locator('[data-item="manage"]').click();
  await expect(page.getByTestId("cf-rules")).toBeVisible();
  await page.waitForTimeout(200);
  await page.screenshot({ path: `${OUT}/v03-cf-manager.png` });
});

test("Total column: colour scale + data bar + icon set", async ({ page }) => {
  await ready(page);
  await totalFormats(page);
  await page.evaluate(() => window.__tally!.controller.colorByValueOn({ r0: 1, c0: 6, r1: 14, c1: 6 }));
  await page.waitForTimeout(300);
  const a = await cellXY(page, 0, 2);
  const b = await cellXY(page, 15, 6);
  await page.screenshot({ path: `${OUT}/v03-total-cf.png`, clip: { x: a.left - 50, y: a.top - 30, width: b.right - a.left + 70, height: b.top - a.top + 40 } });
});

test("validation dropdown", async ({ page }) => {
  await ready(page);
  await page.evaluate(async () => {
    const ctl = window.__tally!.controller;
    await ctl.v3.applyValidation({ id: 0, ranges: [{ r0: 1, c0: 6, r1: 14, c1: 6 }], kind: { type: "list", source: { type: "values", items: ["Paid", "Due", "Overdue", "Cancelled"] } }, allowBlank: true, showDropdown: true, inputHelp: null, error: { style: "stop", title: "Status", text: "Choose Paid, Due, Overdue or Cancelled." } }, false);
    await ctl.colorByValueOn({ r0: 1, c0: 6, r1: 14, c1: 6 });
  });
  await page.waitForTimeout(250);
  const g = await cellXY(page, 3, 6);
  await page.mouse.click(g.x, g.y);
  await page.mouse.click(g.right - 10, g.y);
  await expect(page.getByTestId("validation-list")).toBeVisible();
  await page.waitForTimeout(300);
  const a = await cellXY(page, 0, 3);
  await page.screenshot({ path: `${OUT}/v03-validation-dropdown.png`, clip: { x: a.left - 50, y: a.top - 30, width: g.right - a.left + 130, height: 300 } });
});

test("note popover", async ({ page }) => {
  await ready(page);
  await page.evaluate(async () => {
    const ctl = window.__tally!.controller;
    await ctl.apply("Note", [{ type: "setNote", sheet: ctl.sheet, r: 2, c: 1, note: { r: 2, c: 1, text: "Call before shipping — the café is closed on Mondays.", author: "Sam", created: "2026-10-12T09:30:00", shown: false } }]);
  });
  await page.waitForTimeout(200);
  const b = await cellXY(page, 2, 1);
  await page.mouse.move(b.x, b.y);
  await expect(page.getByTestId("note-popover")).toBeVisible();
  await page.waitForTimeout(300);
  const a = await cellXY(page, 0, 0);
  await page.screenshot({ path: `${OUT}/v03-note-popover.png`, clip: { x: a.left - 50, y: a.top - 30, width: 620, height: 220 } });
});

test("page setup dialog", async ({ page }) => {
  await ready(page);
  await page.evaluate(async () => {
    const ctl = window.__tally!.controller;
    await ctl.apply("Page setup", [{ type: "setPageSetup", sheet: ctl.sheet, setup: { orientation: "landscape", paper: { type: "a4" }, margins: { topMm: 18, bottomMm: 18, leftMm: 15, rightMm: 15, headerMm: 8, footerMm: 8 }, scale: { type: "fitTo", width: 1, height: 0 }, printRange: { r0: 0, c0: 0, r1: 14, c1: 6 }, repeatRows: [0, 0], repeatCols: null, header: { left: "&[FILE]", center: "Orders", right: "&[DATE]" }, footer: { left: "", center: "Page &[PAGE] of &[PAGES]", right: "" }, printGridlines: true, printHeaders: false, centerH: true, centerV: false, pageOrder: "topToBottom", firstPageNumber: null, rowBreaks: [], colBreaks: [] } }]);
  });
  await page.evaluate(() => window.__tally!.controller.registry.run("file.pageSetup"));
  const dialog = page.getByTestId("page-setup-dialog");
  await expect(dialog).toBeVisible();
  await dialog.getByTestId("tab-hf").click();
  await page.waitForTimeout(200);
  await page.screenshot({ path: `${OUT}/v03-page-setup.png` });
});

test("CF rule editor", async ({ page }) => {
  await ready(page);
  const a = await cellXY(page, 1, 5);
  await page.mouse.click(a.x, a.y);
  await page.keyboard.down("Shift");
  const b = await cellXY(page, 14, 5);
  await page.mouse.click(b.x, b.y);
  await page.keyboard.up("Shift");
  await page.evaluate(() => window.__tally!.controller.registry.run("format.conditional"));
  await page.getByTestId("cf-new").click();
  const editor = page.getByTestId("cf-editor");
  await editor.getByTestId("cf-value-0").fill("60");
  await editor.getByTestId("cf-chip").check();
  await page.waitForTimeout(200);
  await page.screenshot({ path: `${OUT}/v03-cf-editor.png` });
});

test("validation dialog", async ({ page }) => {
  await ready(page);
  await page.evaluate(() => window.__tally!.controller.registry.run("data.validation"));
  const dialog = page.getByTestId("validation-dialog");
  await dialog.getByTestId("validation-items").fill("Paid\nDue\nOverdue");
  await page.waitForTimeout(200);
  await page.screenshot({ path: `${OUT}/v03-validation-dialog.png` });
});
