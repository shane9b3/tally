/**
 * Multi-sheet selection ("grouped sheets", AOO ScMarkData's table marks)
 * in the browser build: ⌘-click on tabs groups sheets, edits on the active
 * sheet repeat on every grouped sheet (WebMockClient mirrors the engine's
 * group.rs: all or nothing, one undo step), a protected sheet in the group
 * refuses the whole edit, Delete removes every selected sheet, Edit ▸ Fill
 * ▸ Sheets… copies the selection, sort is off and Print's "Selected sheets"
 * prints the group.
 */
import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";

const ROW_H = 24;
const HEADER_H = 24;

interface Rec {
  type: string;
  [k: string]: unknown;
}

async function ready(page: Page, url = "/") {
  await page.goto(url);
  await expect(page.getByTestId("grid")).toBeVisible();
  await page.waitForFunction(() => window.__tally?.controller.grid != null);
}

const run = (page: Page, id: string) => page.evaluate((id) => window.__tally!.controller.registry.run(id), id);
const calls = (page: Page): Promise<Rec[]> =>
  page.evaluate(() => JSON.parse(JSON.stringify((window.__tally!.client as unknown as { recordedCalls: unknown[] }).recordedCalls)) as Rec[]);
const tabs = (page: Page) => page.getByRole("tablist", { name: "Sheets" }).getByRole("tab");

/** Three sheets (the first one active); returns their ids in tab order. */
async function threeSheets(page: Page): Promise<number[]> {
  const ids = await page.evaluate(async () => {
    const c = window.__tally!.controller;
    await c.addSheet();
    await c.addSheet();
    return c.info!.sheets.map((s) => s.id);
  });
  expect(ids).toHaveLength(3);
  await tabs(page).nth(0).click();
  await expect.poll(() => page.evaluate(() => window.__tally!.controller.sheet)).toBe(ids[0]);
  return ids;
}

/** ⌘/Ctrl-click on the tab at `i`: adds it to (or removes it from) the selected sheets. */
async function toggleTab(page: Page, i: number) {
  await tabs(page).nth(i).click({ modifiers: ["ControlOrMeta"] });
}

const groupOf = (page: Page) => page.evaluate(() => window.__tally!.controller.selectedSheets);

/** Client coordinates of a cell's centre. */
async function cellXY(page: Page, r: number, c: number) {
  const box = await page.getByTestId("grid").boundingBox();
  if (!box) throw new Error("grid not visible");
  const { headerW, xs, w } = await page.evaluate((c) => {
    const ctl = window.__tally!.controller;
    let x = 0;
    for (let i = 0; i < c; i++) x += ctl.layoutSize("col", i);
    return { headerW: (ctl.grid as unknown as { headerW: number }).headerW, xs: x, w: ctl.layoutSize("col", c) };
  }, c);
  return { x: box.x + headerW + xs + w / 2, y: box.y + HEADER_H + r * ROW_H + ROW_H / 2 };
}

async function clickCell(page: Page, r: number, c: number, modifier?: "Shift") {
  const p = await cellXY(page, r, c);
  if (modifier) await page.keyboard.down(modifier);
  await page.mouse.click(p.x, p.y);
  if (modifier) await page.keyboard.up(modifier);
}

async function typeIn(page: Page, r: number, c: number, text: string) {
  await clickCell(page, r, c);
  await page.keyboard.type(text);
  await page.keyboard.press("Enter");
}

/** Display text of a cell on any sheet. */
const textOn = (page: Page, sheet: number, r: number, c: number) =>
  page.evaluate(
    async ([sheet, r, c]) => {
      const t = window.__tally!;
      const [block] = await t.client.cells(t.controller.info!.doc, sheet, [{ r0: r, c0: c, r1: r, c1: c }]);
      return block?.cells.find((x) => x.r === r && x.c === c)?.text ?? "";
    },
    [sheet, r, c] as const,
  );

/** Whether a cell on any sheet is bold. */
const boldOn = (page: Page, sheet: number, r: number, c: number) =>
  page.evaluate(
    async ([sheet, r, c]) => {
      const t = window.__tally!;
      const doc = t.controller.info!.doc;
      const [block] = await t.client.cells(doc, sheet, [{ r0: r, c0: c, r1: r, c1: c }]);
      const id = block?.cells.find((x) => x.r === r && x.c === c)?.style ?? 0;
      const [style] = await t.client.styles(doc, [id]);
      return style?.bold ?? false;
    },
    [sheet, r, c] as const,
  );

test("⌘-click groups two tabs; typing, bold and insert row reach both; one ⌘Z undoes it on all", async ({ page }) => {
  await ready(page);
  const [a, b, c] = (await threeSheets(page)) as [number, number, number];
  await toggleTab(page, 1);
  expect(await groupOf(page)).toEqual([a, b]);
  await expect(tabs(page).nth(0)).toHaveAttribute("aria-selected", "true");
  await expect(tabs(page).nth(1)).toHaveAttribute("aria-selected", "true");
  await expect(tabs(page).nth(2)).toHaveAttribute("aria-selected", "false");

  await typeIn(page, 0, 0, "42");
  await expect.poll(() => textOn(page, a, 0, 0)).toBe("42");
  expect(await textOn(page, b, 0, 0)).toBe("42");
  expect(await textOn(page, c, 0, 0)).toBe("");

  await clickCell(page, 0, 0);
  await run(page, "format.bold");
  await expect.poll(() => boldOn(page, b, 0, 0)).toBe(true);
  expect(await boldOn(page, a, 0, 0)).toBe(true);

  await run(page, "sheet.insertRowAbove");
  await expect.poll(() => textOn(page, b, 1, 0)).toBe("42");
  expect(await textOn(page, a, 1, 0)).toBe("42");
  expect(await textOn(page, a, 0, 0)).toBe("");

  // One undo step for the whole grouped insert.
  await page.getByTestId("grid").focus();
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(() => textOn(page, b, 0, 0)).toBe("42");
  expect(await textOn(page, a, 0, 0)).toBe("42");
  expect(await textOn(page, a, 1, 0)).toBe("");
});

test("a protected sheet in the group refuses the edit on every sheet", async ({ page }) => {
  await ready(page);
  const [a, b] = (await threeSheets(page)) as [number, number, number];
  await page.evaluate(async (b) => {
    await window.__tally!.controller.apply("Protect", [{ type: "protectSheet", sheet: b, password: null, options: null }]);
  }, b);
  await toggleTab(page, 1);
  expect(await groupOf(page)).toEqual([a, b]);
  await typeIn(page, 0, 0, "nope");
  await expect(page.getByTestId("toast-error")).toContainText("protected");
  expect(await textOn(page, a, 0, 0)).toBe("");
  expect(await textOn(page, b, 0, 0)).toBe("");
});

test("Delete removes both selected sheets", async ({ page }) => {
  await ready(page);
  const [, b, c] = (await threeSheets(page)) as [number, number, number];
  await tabs(page).nth(1).click();
  await toggleTab(page, 2);
  expect(await groupOf(page)).toEqual([b, c]);
  await tabs(page).nth(1).click({ button: "right" });
  await page.getByRole("menuitem", { name: "Delete 2 sheets" }).click();
  await expect(tabs(page)).toHaveCount(1);
  await expect.poll(() => page.evaluate(() => window.__tally!.controller.info!.sheets.length)).toBe(1);
});

test("Fill Sheet copies values and formats; Skip empty cells keeps the target's", async ({ page }) => {
  await ready(page);
  const [a, b] = (await threeSheets(page)) as [number, number, number];
  // Sources on sheet 1 (A1 bold 10, B1 "hi", C1 empty); sheet 2 has "keep" in C1.
  await page.evaluate(
    async ([a, b]) => {
      const c = window.__tally!.controller;
      await c.apply("setup", [
        { type: "setCellInput", sheet: a, r: 0, c: 0, input: "10" },
        { type: "setCellInput", sheet: a, r: 0, c: 1, input: "hi" },
        { type: "setCellInput", sheet: b, r: 0, c: 2, input: "keep" },
      ]);
    },
    [a, b] as const,
  );
  await clickCell(page, 0, 0);
  await run(page, "format.bold");
  await expect.poll(() => boldOn(page, a, 0, 0)).toBe(true);

  // Without a group the command is off.
  expect(await run(page, "edit.fillSheet")).toBe(false);
  await toggleTab(page, 1);
  await clickCell(page, 0, 0);
  await clickCell(page, 0, 2, "Shift");

  await run(page, "edit.fillSheet");
  const dialog = page.getByTestId("fill-sheet-dialog");
  await expect(dialog).toBeVisible();
  await expect(page.getByRole("dialog", { name: "Fill Sheet" })).toBeVisible();
  await expect(dialog.getByTestId("fillsheet-all")).toBeChecked();
  await expect(dialog.getByTestId("fillsheet-text")).toBeDisabled();
  await expect(dialog.getByTestId("fillsheet-op-none")).toBeChecked();
  // Link, Transpose and Shift cells are not offered.
  await expect(dialog.getByText(/Transpose|Link|Shift cells/)).toHaveCount(0);
  await dialog.getByTestId("fillsheet-skip-empty").check();
  await dialog.getByTestId("dialog-ok").click();
  await expect(dialog).toBeHidden();
  await expect.poll(() => textOn(page, b, 0, 0)).toBe("10");
  expect(await textOn(page, b, 0, 1)).toBe("hi");
  expect(await boldOn(page, b, 0, 0)).toBe(true);
  expect(await textOn(page, b, 0, 2)).toBe("keep");

  // Again without Skip empty cells: the empty C1 clears the target.
  await run(page, "edit.fillSheet");
  await expect(dialog).toBeVisible();
  await dialog.getByTestId("dialog-ok").click();
  await expect.poll(() => textOn(page, b, 0, 2)).toBe("");
  // One undo step brings "keep" back.
  await page.getByTestId("grid").focus();
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(() => textOn(page, b, 0, 2)).toBe("keep");
});

test("sort is disabled while sheets are grouped", async ({ page }) => {
  await ready(page);
  await threeSheets(page);
  expect(await page.evaluate(() => window.__tally!.controller.registry.isEnabled("data.sort"))).toBe(true);
  await toggleTab(page, 1);
  expect(await run(page, "data.sort")).toBe(false);
  expect(await run(page, "data.sortAsc")).toBe(false);
  await expect(page.getByTestId("sort-dialog")).toHaveCount(0);
});

test("Print's Selected sheets prints the grouped sheets", async ({ page }) => {
  await ready(page);
  const [a, , c] = (await threeSheets(page)) as [number, number, number];
  await page.evaluate(
    async ([a, c]) => {
      const ops: { type: "setCellInput"; sheet: number; r: number; c: number; input: string }[] = [];
      for (let r = 0; r < 120; r++) ops.push({ type: "setCellInput", sheet: c, r, c: 0, input: `three ${r}` });
      ops.push({ type: "setCellInput", sheet: a, r: 0, c: 0, input: "one" });
      await window.__tally!.controller.apply("fill", ops);
    },
    [a, c] as const,
  );
  const pagesOf = (sheet: number) =>
    page.evaluate(async (sheet) => {
      const t = window.__tally!;
      const opts = { sheets: { type: "active" as const }, selection: null, suppressEmptyPages: true, pageFrom: null, pageTo: null, activeSheet: sheet };
      return (await t.client.printLayout(t.controller.info!.doc, opts)).pages.length;
    }, sheet);
  const [na, nc] = [await pagesOf(a), await pagesOf(c)];
  expect(nc).toBeGreaterThan(1);
  await toggleTab(page, 2);
  expect(await groupOf(page)).toEqual([a, c]);
  await run(page, "file.printPreview");
  await expect(page.getByTestId("print-preview")).toBeVisible();
  await expect(page.getByTestId("print-scope")).toHaveValue("selected");
  await expect(page.getByTestId("print-page-label")).toHaveText(`Page 1 of ${na + nc}`);
  const prepared = (await calls(page)).filter((x) => x.type === "preparePrintJob").at(-1) as { opts: { sheets: { type: string; ids?: number[] } } };
  expect(prepared.opts.sheets).toEqual({ type: "some", ids: [a, c] });
});

test("axe: the grouped tab bar and the Fill Sheet dialog have no serious violations", async ({ page }) => {
  await ready(page);
  await threeSheets(page);
  await toggleTab(page, 1);
  await expect(tabs(page).nth(1)).toHaveAttribute("aria-selected", "true");
  await page.waitForFunction(() => document.getAnimations().every((a) => a.playState !== "running"));
  const result = await new AxeBuilder({ page }).include('[role="tablist"][aria-label="Sheets"]').withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "best-practice"]).analyze();
  const serious = (r: typeof result) => r.violations.filter((v) => v.impact === "serious" || v.impact === "critical").map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ")).join(", ")}`);
  expect(serious(result)).toEqual([]);
  // The Fill Sheet dialog too.
  await run(page, "edit.fillSheet");
  await expect(page.getByTestId("fill-sheet-dialog")).toBeVisible();
  await page.waitForFunction(() => document.getAnimations().every((a) => a.playState !== "running"));
  const dlg = await new AxeBuilder({ page }).include('[data-testid="fill-sheet-dialog"]').withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "best-practice"]).analyze();
  expect(serious(dlg)).toEqual([]);
});
