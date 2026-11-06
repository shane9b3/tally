/**
 * Format Cells ▸ Cell Protection (AOO ScTabPageProtection, G15): "Hide all"
 * disables "Protected" and "Hide formula"; on a protected sheet a hidden
 * cell shows blank (grid view and input line) and shows again once the
 * sheet is unprotected; "Hide when printing" leaves the cell out of the
 * print preview.
 */
import { expect, test, type Page } from "@playwright/test";

const ROW_H = 24;

interface Rec {
  type: string;
  [k: string]: unknown;
}

async function ready(page: Page, url = "/?template=invoices") {
  await page.goto(url);
  await expect(page.getByTestId("grid")).toBeVisible();
  await page.waitForFunction(() => window.__tally?.controller.grid != null);
}

async function clickCell(page: Page, r: number, c: number) {
  const box = await page.getByTestId("grid").boundingBox();
  if (!box) throw new Error("grid not visible");
  const { headerW, headerH, xs, w } = await page.evaluate((c) => {
    const ctl = window.__tally!.controller;
    let x = 0;
    for (let i = 0; i < c; i++) x += ctl.layoutSize("col", i);
    const geo = ctl.grid!.getGeometry();
    return { headerW: geo.headerW, headerH: geo.headerH, xs: x, w: ctl.layoutSize("col", c) };
  }, c);
  await page.mouse.click(box.x + headerW + xs + w / 2, box.y + headerH + r * ROW_H + ROW_H / 2);
}

const recorded = (page: Page): Promise<Rec[]> =>
  page.evaluate(() => JSON.parse(JSON.stringify((window.__tally!.client as unknown as { recorded: unknown[] }).recorded)) as Rec[]);
const calls = (page: Page): Promise<Rec[]> =>
  page.evaluate(() => JSON.parse(JSON.stringify((window.__tally!.client as unknown as { recordedCalls: unknown[] }).recordedCalls)) as Rec[]);
const run = (page: Page, id: string) => page.evaluate((id) => window.__tally!.controller.registry.run(id), id);

/** The cell's view as the grid gets it: text and flags. */
async function cellView(page: Page, r: number, c: number): Promise<{ text: string; flags: number }> {
  return page.evaluate(
    async ([r, c]) => {
      const t = window.__tally!;
      const [b] = await t.client.cells(t.controller.info!.doc, t.controller.sheet, [{ r0: r, c0: c, r1: r, c1: c }]);
      const v = b?.cells.find((x) => x.r === r && x.c === c);
      return { text: v?.text ?? "", flags: v?.flags ?? 0 };
    },
    [r, c] as const,
  );
}

async function openProtectionTab(page: Page, r: number, c: number) {
  await clickCell(page, r, c);
  await page.keyboard.press("Meta+1");
  const dlg = page.getByTestId("format-cells");
  await expect(dlg).toBeVisible();
  await dlg.getByTestId("tab-protection").click();
  return dlg;
}

async function toggleSheetProtection(page: Page, on: boolean) {
  await run(page, "tools.protectSheet");
  if (on) await page.getByTestId("protect-sheet-dialog").getByTestId("dialog-ok").click();
  await expect.poll(async () => (await recorded(page)).at(-1)?.type).toBe(on ? "protectSheet" : "unprotectSheet");
}

test("Hide all: blank on a protected sheet, shown again when unprotected", async ({ page }) => {
  await ready(page);
  const before = await cellView(page, 1, 1);
  expect(before.text).not.toBe("");
  const dlg = await openProtectionTab(page, 1, 1);
  await expect(dlg.getByTestId("cells-locked")).toBeChecked();
  await expect(dlg.getByTestId("cells-locked")).toBeEnabled();
  await expect(dlg).toContainText("Cell protection is only effective after the current sheet has been protected.");
  await dlg.getByTestId("cells-hide-all").check();
  // AOO UpdateButtons: Hide all disables Protected and Hide formula.
  await expect(dlg.getByTestId("cells-locked")).toBeDisabled();
  await expect(dlg.getByTestId("cells-hide-formula")).toBeDisabled();
  await expect(dlg.getByTestId("cells-hide-print")).toBeEnabled();
  await dlg.getByTestId("dialog-ok").click();
  await expect(dlg).toHaveCount(0);
  // Only the changed box takes a value; the others leave each cell as it is.
  expect((await recorded(page)).at(-1)).toMatchObject({ type: "setCellProtection", locked: null, hiddenFormula: null, hidden: true, hidePrint: null });
  // Not protected yet: still shown.
  expect((await cellView(page, 1, 1)).text).toBe(before.text);

  await toggleSheetProtection(page, true);
  await expect.poll(async () => (await cellView(page, 1, 1)).text).toBe("");
  expect((await cellView(page, 1, 1)).flags & 64).toBe(64);
  await clickCell(page, 1, 2);
  await clickCell(page, 1, 1);
  await expect(page.getByTestId("formula-input")).toHaveValue("");
  // Other cells are unaffected.
  expect((await cellView(page, 1, 0)).flags & 64).toBe(0);

  await toggleSheetProtection(page, false);
  await expect.poll(async () => (await cellView(page, 1, 1)).text).toBe(before.text);
  await clickCell(page, 1, 2);
  await clickCell(page, 1, 1);
  await expect(page.getByTestId("formula-input")).not.toHaveValue("");
});

test("Hide when printing: the print preview leaves the cell out", async ({ page }) => {
  await ready(page);
  const hidden = await cellView(page, 1, 1);
  const kept = await cellView(page, 1, 0);
  expect(hidden.text).not.toBe("");
  expect(kept.text).not.toBe("");
  const dlg = await openProtectionTab(page, 1, 1);
  await dlg.getByTestId("cells-hide-print").check();
  await dlg.getByTestId("dialog-ok").click();
  expect((await recorded(page)).at(-1)).toMatchObject({ type: "setCellProtection", hidePrint: true, hidden: null });
  // Still shown on screen.
  expect((await cellView(page, 1, 1)).text).toBe(hidden.text);

  await run(page, "file.printPreview");
  await expect(page.getByTestId("print-preview")).toBeVisible();
  await expect(page.getByTestId("print-page")).toBeVisible();
  await expect.poll(async () => (await calls(page)).filter((c) => c.type === "printJobPagePng").length).toBeGreaterThan(0);
  const pages = (await calls(page)).filter((c) => c.type === "printJobPagePng" && c.page === 0) as unknown as { texts: { r: number; c: number; text: string }[] }[];
  const texts = pages.at(-1)!.texts;
  expect(texts.some((t) => t.r === 1 && t.c === 0 && t.text === kept.text)).toBe(true);
  expect(texts.some((t) => t.r === 1 && t.c === 1)).toBe(false);
  await page.keyboard.press("Escape");
});
