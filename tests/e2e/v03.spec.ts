/**
 * v0.3 flows in the browser (WebMockClient keeps conditional formats,
 * validations, notes, links and protection in its side model and records
 * every op in `client.recorded`, plus `setLocale` / `openUrl` / `goalSeek`
 * calls in `client.recordedCalls`).
 */
import { expect, test, type Page } from "@playwright/test";

const ROW_H = 24;
const HEADER_H = 24;

interface Rec {
  type: string;
  [k: string]: unknown;
}

async function ready(page: Page, url = "/?template=invoices") {
  await page.goto(url);
  await expect(page.getByTestId("grid")).toBeVisible();
  await page.waitForFunction(() => window.__tally?.controller.grid != null);
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
  return { x: box.x + headerW + xs + w / 2, y: box.y + HEADER_H + r * ROW_H + ROW_H / 2, right: box.x + headerW + xs + w };
}

async function clickCell(page: Page, r: number, c: number, modifier?: "Shift" | "Meta") {
  const p = await cellXY(page, r, c);
  if (modifier) await page.keyboard.down(modifier);
  await page.mouse.click(p.x, p.y);
  if (modifier) await page.keyboard.up(modifier);
}

const recorded = (page: Page): Promise<Rec[]> =>
  page.evaluate(() => JSON.parse(JSON.stringify((window.__tally!.client as unknown as { recorded: unknown[] }).recorded)) as Rec[]);

const calls = (page: Page): Promise<Rec[]> =>
  page.evaluate(() => JSON.parse(JSON.stringify((window.__tally!.client as unknown as { recordedCalls: unknown[] }).recordedCalls)) as Rec[]);

const run = (page: Page, id: string) => page.evaluate((id) => window.__tally!.controller.registry.run(id), id);

async function selectRange(page: Page, r0: number, c0: number, r1: number, c1: number) {
  await clickCell(page, r0, c0);
  await clickCell(page, r1, c1, "Shift");
}

async function cellFlags(page: Page, r: number, c: number): Promise<number> {
  return page.evaluate(
    async ([r, c]) => {
      const t = window.__tally!;
      const [b] = await t.client.cells(t.controller.info!.doc, t.controller.sheet, [{ r0: r, c0: c, r1: r, c1: c }]);
      return b?.cells[0]?.flags ?? 0;
    },
    [r, c] as const,
  );
}

test("conditional formatting manager adds a greater-than rule", async ({ page }) => {
  await ready(page);
  await selectRange(page, 1, 5, 14, 5); // F2:F15
  await page.getByTestId("highlight-cells").click();
  await page.getByTestId("highlight-menu").locator('[data-item="manage"]').click();
  const manager = page.getByTestId("cf-manager");
  await expect(manager).toBeVisible();
  await expect(manager).toContainText("No rules on this sheet yet");
  await manager.getByTestId("cf-new").click();
  const editor = page.getByTestId("cf-editor");
  await expect(editor.getByTestId("cf-ranges")).toHaveValue("F2:F15");
  await editor.getByTestId("cf-op").selectOption("gt");
  await editor.getByTestId("cf-value-0").fill("50");
  await editor.getByTestId("dialog-ok").click();
  await expect(page.getByTestId("cf-rules")).toContainText("Cell value > 50");
  const ops = await recorded(page);
  expect(ops.at(-1)).toMatchObject({
    type: "addConditionalFormat",
    ranges: [{ r0: 1, c0: 5, r1: 14, c1: 5 }],
    rules: [{ kind: { type: "cellValue", op: "gt", formulas: ["50"] }, stopIfTrue: false }],
  });
  // The mock evaluates it: F3 (67.5) gets a synthesized style, F4 (48) does not.
  const styled = await page.evaluate(async () => {
    const t = window.__tally!;
    const [b] = await t.client.cells(t.controller.info!.doc, t.controller.sheet, [{ r0: 2, c0: 5, r1: 3, c1: 5 }]);
    return (b?.cells ?? []).map((v) => (v.cfStyle ?? 0) >= 0x40000000);
  });
  expect(styled).toEqual([true, false]);
  // Delete it again.
  await page.getByTestId("cf-delete-0").click();
  await expect(manager).toContainText("No rules on this sheet yet");
  expect((await recorded(page)).at(-1)).toMatchObject({ type: "deleteConditionalFormat" });
});

test("Highlight cells: greater than… preset records a rule", async ({ page }) => {
  await ready(page);
  await selectRange(page, 1, 5, 14, 5);
  await page.getByTestId("highlight-cells").click();
  await page.getByTestId("highlight-menu").locator('[data-item="gt"]').click();
  const dialog = page.getByTestId("cf-preset-dialog");
  await dialog.getByTestId("cf-preset-value").fill("60");
  await dialog.getByTestId("dialog-ok").click();
  expect((await recorded(page)).at(-1)).toMatchObject({ type: "addConditionalFormat", rules: [{ kind: { type: "cellValue", op: "gt", formulas: ["60"] } }] });
});

test("Colour by value creates chip rules", async ({ page }) => {
  await ready(page);
  await selectRange(page, 1, 6, 14, 6);
  await page.locator('[data-command="format.colorByValue"]').click();
  const op = (await recorded(page)).at(-1) as Rec & { rules: { kind: { formulas: string[] }; style: { display: { type: string } } }[] };
  expect(op.type).toBe("addConditionalFormat");
  expect(op.rules.map((r) => r.kind.formulas[0])).toEqual(['"Paid"', '"Due"', '"Overdue"']);
  expect(op.rules.every((r) => r.style.display.type === "chip")).toBe(true);
});

test("data validation list shows a dropdown and rejects other values", async ({ page }) => {
  await ready(page);
  await selectRange(page, 1, 6, 14, 6); // G2:G15
  await run(page, "data.validation");
  const dialog = page.getByTestId("validation-dialog");
  await expect(dialog.getByTestId("validation-ranges")).toHaveValue("G2:G15");
  await dialog.getByTestId("validation-items").fill("Paid\nDue\nOverdue");
  await dialog.getByTestId("tab-alert").click();
  await dialog.getByTestId("validation-alert-title").fill("Status");
  await dialog.getByTestId("validation-alert-text").fill("Choose Paid, Due or Overdue.");
  await dialog.getByTestId("dialog-ok").click();
  expect((await recorded(page)).at(-1)).toMatchObject({
    type: "addValidation",
    validation: { ranges: [{ r0: 1, c0: 6, r1: 14, c1: 6 }], kind: { type: "list", source: { type: "values", items: ["Paid", "Due", "Overdue"] } }, showDropdown: true, error: { style: "stop", title: "Status" } },
  });
  await expect.poll(() => cellFlags(page, 3, 6)).toBe(16);
  // Click the cell's ▾ and pick an item.
  const g4 = await cellXY(page, 3, 6);
  await page.mouse.click(g4.x, g4.y);
  await page.mouse.click(g4.right - 10, g4.y);
  const list = page.getByTestId("validation-list");
  await expect(list).toBeVisible();
  await expect(list.locator("[data-item]")).toHaveText([/Paid$/, /Due$/, /Overdue$/]);
  await list.locator('[data-item="Due"]').click();
  expect((await recorded(page)).at(-1)).toMatchObject({ type: "setCellInput", r: 3, c: 6, input: "Due" });
  // Typing something else: the Stop alert, with Retry.
  await page.keyboard.type("Maybe");
  await page.keyboard.press("Enter");
  const alert = page.getByTestId("validation-alert");
  await expect(alert).toBeVisible();
  await expect(alert).toContainText("Status");
  await expect(alert).toContainText("Choose Paid, Due or Overdue.");
  await alert.getByTestId("dialog-ok").click();
  await expect(page.locator(".tally-grid textarea")).toHaveValue("Maybe");
});

test("insert a note, see the indicator and the popover", async ({ page }) => {
  await ready(page);
  await clickCell(page, 2, 1); // B3
  await page.keyboard.press("Shift+F2");
  const editor = page.getByTestId("note-editor");
  await expect(editor).toBeVisible();
  await editor.getByTestId("note-text").fill("Call before shipping");
  await editor.getByTestId("note-save").click();
  expect((await recorded(page)).at(-1)).toMatchObject({ type: "setNote", r: 2, c: 1, note: { text: "Call before shipping", author: "Demo User" } });
  await expect.poll(() => cellFlags(page, 2, 1)).toBe(8);
  await page.mouse.move(10, 10);
  const b3 = await cellXY(page, 2, 1);
  await page.mouse.move(b3.x, b3.y);
  const pop = page.getByTestId("note-popover");
  await expect(pop).toBeVisible();
  await expect(pop).toContainText("Call before shipping");
  await expect(pop).toContainText("Demo User");
});

test("hyperlink dialog sets a link; ⌘-click opens it", async ({ page }) => {
  await ready(page);
  await clickCell(page, 17, 1); // B18
  await page.keyboard.press("Alt+Control+k");
  const dialog = page.getByTestId("hyperlink-dialog");
  await expect(dialog).toBeVisible();
  await dialog.getByTestId("hyperlink-url").fill("javascript:alert(1)");
  await expect(dialog).toContainText("not allowed");
  await expect(dialog.getByTestId("dialog-ok")).toBeDisabled();
  await dialog.getByTestId("hyperlink-url").fill("example.com/orders");
  await dialog.getByTestId("hyperlink-text").fill("Order portal");
  await dialog.getByTestId("dialog-ok").click();
  const ops = await recorded(page);
  expect(ops.slice(-2)).toMatchObject([
    // Literal text, not typed input (a leading +/- stays text; P18).
    { type: "setRangeInputs", r: 17, c: 1, rows: [["Order portal"]] },
    { type: "setLink", r: 17, c: 1, url: "https://example.com/orders" },
  ]);
  await expect.poll(() => cellFlags(page, 17, 1)).toBe(32);
  await clickCell(page, 17, 1, "Meta");
  await expect.poll(() => calls(page)).toContainEqual({ type: "openUrl", url: "https://example.com/orders" });
});

test("protect the sheet, then an edit is refused", async ({ page }) => {
  await ready(page);
  await run(page, "tools.protectSheet");
  const dialog = page.getByTestId("protect-sheet-dialog");
  await expect(dialog).toBeVisible();
  await dialog.getByTestId("dialog-ok").click();
  expect((await recorded(page)).at(-1)).toMatchObject({ type: "protectSheet", password: null });
  await clickCell(page, 1, 1);
  await page.keyboard.type("Changed");
  await page.keyboard.press("Enter");
  await expect(page.getByTestId("toast-error")).toContainText("protected");
  await expect(page.locator(".tally-grid[data-locked]")).toHaveCount(1);
  // Unprotect (no password) and the edit goes through.
  await run(page, "tools.protectSheet");
  expect((await recorded(page)).at(-1)).toMatchObject({ type: "unprotectSheet" });
});

test("goal seek finds the quantity for a total and applies it", async ({ page }) => {
  await ready(page);
  await clickCell(page, 1, 5); // F2 = 12 × 4.5
  await run(page, "tools.goalSeek");
  const dialog = page.getByTestId("goal-seek-dialog");
  await expect(dialog.getByTestId("goal-formula")).toHaveValue("F2");
  await dialog.getByTestId("goal-target").fill("100");
  await dialog.getByTestId("goal-variable").fill("D2");
  await dialog.getByTestId("dialog-ok").click();
  await expect(dialog.getByTestId("goal-result")).toContainText("Found a solution");
  await expect(dialog.getByTestId("goal-result")).toContainText("22.2222222222");
  expect((await calls(page)).at(-1)).toMatchObject({ type: "goalSeek", req: { formulaCell: { r: 1, c: 5 }, target: 100, variableCell: { r: 1, c: 3 } } });
  await dialog.getByTestId("dialog-ok").click();
  await expect(dialog).toHaveCount(0);
  const op = (await recorded(page)).at(-1) as Rec & { input: string };
  expect(op).toMatchObject({ type: "setCellInput", r: 1, c: 3 });
  expect(Number(op.input)).toBeCloseTo(22.2222222, 5);
});

test("page setup saves orientation, header and print range", async ({ page }) => {
  await ready(page);
  await run(page, "file.pageSetup");
  const dialog = page.getByTestId("page-setup-dialog");
  await expect(dialog).toBeVisible();
  await dialog.getByTestId("page-landscape").click();
  await dialog.getByTestId("tab-hf").click();
  // No footer yet: its controls are disabled until "Footer on" (AOO).
  await expect(dialog.getByTestId("footer-center")).toBeDisabled();
  await dialog.getByTestId("footer-on").check();
  await dialog.getByTestId("footer-center").click();
  await dialog.getByTestId("footer-insert-page").click();
  await dialog.getByTestId("tab-sheet").click();
  await dialog.getByTestId("page-print-range").fill("A1:G15");
  await dialog.getByTestId("page-repeat-rows").fill("1");
  await dialog.getByTestId("dialog-ok").click();
  expect((await recorded(page)).at(-1)).toMatchObject({
    type: "setPageSetup",
    setup: { orientation: "landscape", footer: { center: "&[PAGE]" }, footerArea: { on: true }, printRange: { r0: 0, c0: 0, r1: 14, c1: 6 }, repeatRows: [0, 0] },
  });
  // View ▸ Page breaks draws the print range.
  await run(page, "view.pageBreaks");
  await expect.poll(() => page.evaluate(() => (window.__tally!.controller.grid as unknown as { props: { printRange: unknown } }).props.printRange)).toEqual({ r0: 0, c0: 0, r1: 14, c1: 6 });
});

test("locale switch calls set_locale", async ({ page }) => {
  await ready(page);
  await run(page, "view.locale.deDE");
  await expect.poll(() => calls(page)).toContainEqual({ type: "setLocale", locale: "de-DE" });
  await expect(page.getByTestId("toast-info")).toContainText("Deutsch");
});
