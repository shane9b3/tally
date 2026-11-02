/**
 * Read-only mode in the browser (MockClient, `?readonly=1`: the workbook
 * opens like an ODS file whose settings.xml has `LoadReadonly`): the info
 * bar, disabled editing commands, typing refused, Edit Document, ⇧⌘M
 * (Ctrl+Shift+M elsewhere) toggling Edit Mode, and Save As making an
 * editable copy.
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

const readOnly = (page: Page) => page.evaluate(() => window.__tally?.controller.info?.readOnly === true);
const enabled = (page: Page, id: string) => page.evaluate((cmd) => window.__tally?.controller.registry.isEnabled(cmd) ?? false, id);

test.beforeEach(async ({ page }) => {
  await page.goto("/?template=invoices&readonly=1");
  await expect(page.getByTestId("grid")).toBeVisible();
  await page.waitForFunction(() => window.__tally?.controller.grid != null);
});

test("a document flagged read-only shows the info bar with Edit Document", async ({ page }) => {
  const banner = page.getByTestId("read-only-banner");
  await expect(banner).toBeVisible();
  await expect(banner).toHaveAttribute("role", "status");
  await expect(banner).toContainText("This document is open in read-only mode.");
  await expect(page.getByRole("button", { name: "Edit Document" })).toBeVisible();
  expect(await readOnly(page)).toBe(true);
  // (The " (read-only)" title suffix is the desktop window's: readOnly.test.ts.)
});

test("editing commands are disabled and typing does nothing; viewing still works", async ({ page }) => {
  for (const id of ["file.save", "edit.paste", "edit.cut", "edit.undo", "format.bold", "sheet.insert", "data.sort"]) {
    expect(await enabled(page, id), id).toBe(false);
  }
  for (const id of ["file.saveAs", "edit.copy", "edit.find", "file.print", "file.exportPdf", "view.zoomIn", "edit.editMode"]) {
    expect(await enabled(page, id), id).toBe(true);
  }
  const before = await a1(page);
  await clickCell(page, 0, 0);
  await page.keyboard.type("hello");
  await page.keyboard.press("Enter");
  await expect(page.getByText("Document opened in read-only mode.").first()).toBeVisible();
  expect(await a1(page)).toBe(before);
  // The toolbar's Bold is disabled with AOO's reason; the formatting bar is inert.
  await expect(page.locator('[data-testid="format-bar"]')).toHaveAttribute("aria-disabled", "true");
  // ⌘B does nothing either.
  await clickCell(page, 0, 0);
  await page.keyboard.press("ControlOrMeta+b");
  const bold = await page.evaluate(async () => {
    const t = window.__tally;
    const info = t?.controller.info;
    if (!t || !info) return null;
    const [block] = await t.client.cells(info.doc, t.controller.sheet, [{ r0: 0, c0: 0, r1: 0, c1: 0 }]);
    const id = block?.cells[0]?.style ?? 0;
    const [st] = await t.client.styles(info.doc, [id]);
    return st?.bold ?? false;
  });
  expect(bold).toBe(true); // the invoices header row is bold already: nothing toggled it off
  // Selecting and zooming still work.
  await clickCell(page, 2, 1);
  await expect.poll(() => page.evaluate(() => window.__tally?.controller.selection.active)).toEqual({ r: 2, c: 1 });
});

test("Edit Document makes the document editable", async ({ page }) => {
  await page.getByRole("button", { name: "Edit Document" }).click();
  await expect(page.getByTestId("read-only-banner")).toHaveCount(0);
  expect(await readOnly(page)).toBe(false);
  await clickCell(page, 0, 0);
  await page.keyboard.type("edited");
  await page.keyboard.press("Enter");
  await expect.poll(() => a1(page)).toBe("edited");
  expect(await enabled(page, "file.save")).toBe(true);
});

test("⇧⌘M (Ctrl+Shift+M) toggles Edit Mode", async ({ page }) => {
  await clickCell(page, 1, 1);
  await page.keyboard.press("ControlOrMeta+Shift+M");
  await expect(page.getByTestId("read-only-banner")).toHaveCount(0);
  expect(await readOnly(page)).toBe(false);
  await page.keyboard.press("ControlOrMeta+Shift+M");
  await expect(page.getByTestId("read-only-banner")).toBeVisible();
  expect(await readOnly(page)).toBe(true);
});

test("switching to read-only with unsaved changes asks to save first", async ({ page }) => {
  await page.getByRole("button", { name: "Edit Document" }).click();
  await clickCell(page, 0, 0);
  await page.keyboard.type("draft");
  await page.keyboard.press("Enter");
  await expect.poll(() => a1(page)).toBe("draft");
  await page.keyboard.press("ControlOrMeta+Shift+M");
  const prompt = page.getByTestId("save-prompt");
  await expect(prompt).toBeVisible();
  await page.getByTestId("save-prompt-cancel").click();
  expect(await readOnly(page)).toBe(false);
  await page.keyboard.press("ControlOrMeta+Shift+M");
  await page.getByTestId("save-prompt-save").click();
  await expect(page.getByTestId("read-only-banner")).toBeVisible();
  expect(await page.evaluate(() => window.__tally?.controller.info?.dirty)).toBe(false);
});

test("Save As is offered and the copy is editable", async ({ page }) => {
  expect(await enabled(page, "file.save")).toBe(false);
  expect(await enabled(page, "file.saveAs")).toBe(true);
  await page.evaluate(() => window.__tally?.controller.registry.run("file.saveAs"));
  await expect(page.getByTestId("read-only-banner")).toHaveCount(0);
  expect(await readOnly(page)).toBe(false);
  expect(await enabled(page, "file.save")).toBe(true);
});
